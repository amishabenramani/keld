use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write as _;
use std::net::TcpListener;
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::process::Child;
use std::process::Output;
use std::thread;
use std::time::Duration;
use std::time::Instant;

/// A fixture listener whose peers are accepted by polling a NONBLOCKING
/// socket. One owner for every no-flag macOS fixture accept: a completed
/// client `connect` does not mean the server side is already dequeued, so a
/// single immediate `accept` may return `WouldBlock` (Darwin errno 35).
pub(crate) trait FixtureListener {
    type Stream;

    /// One native `accept`; the accepted stream is normalized to blocking
    /// because BSD `accept` inherits the listener's `O_NONBLOCK`.
    fn accept_blocking(&self) -> std::io::Result<Self::Stream>;
}

impl FixtureListener for UnixListener {
    type Stream = UnixStream;

    fn accept_blocking(&self) -> std::io::Result<UnixStream> {
        let (stream, _) = self.accept()?;
        stream
            .set_nonblocking(false)
            .expect("normalize accepted fixture stream");
        Ok(stream)
    }
}

impl FixtureListener for TcpListener {
    type Stream = TcpStream;

    fn accept_blocking(&self) -> std::io::Result<TcpStream> {
        let (stream, _) = self.accept()?;
        stream
            .set_nonblocking(false)
            .expect("normalize accepted fixture stream");
        Ok(stream)
    }
}

pub(crate) fn accept_before<L: FixtureListener>(listener: &L, deadline: Instant) -> L::Stream {
    accept_before_observing(listener, deadline, || {})
}

/// Accept the next queued peer, retrying only an empty queue until the
/// caller's ORIGINAL deadline. The deadline is checked before every native
/// attempt, so a peer dequeued after it never counts as timely. `on_empty`
/// observes each real `WouldBlock`; it cannot substitute an accept result.
pub(crate) fn accept_before_observing<L: FixtureListener>(
    listener: &L,
    deadline: Instant,
    mut on_empty: impl FnMut(),
) -> L::Stream {
    loop {
        assert!(
            Instant::now() < deadline,
            "fixture peer was not accepted before its deadline"
        );
        match listener.accept_blocking() {
            Ok(stream) => return stream,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                on_empty();
                thread::yield_now();
            }
            Err(error) => panic!("accept fixture peer: {error}"),
        }
    }
}

#[test]
fn accept_before_waits_for_a_peer_that_connects_after_real_would_block() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("own native listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking fixture accept mode");
    let address = listener.local_addr().expect("native listener address");
    let (release, released) = std::sync::mpsc::channel::<()>();
    // The peer connects only after the helper has observed a real empty
    // queue. If the helper panics first, dropping `release` ends the peer.
    let peer = thread::spawn(move || {
        if released.recv().is_err() {
            return;
        }
        let mut stream = TcpStream::connect(address).expect("connect native peer");
        stream.write_all(b"W").expect("native peer byte");
    });
    let mut empty_polls = 0_u32;
    let mut release = Some(release);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        accept_before_observing(&listener, Instant::now() + super::EVENT_DEADLINE, || {
            empty_polls += 1;
            if let Some(release) = release.take() {
                release.send(()).expect("release native peer");
            }
        })
    }));
    drop(release);
    peer.join().expect("native peer joins");
    let mut accepted = outcome.expect("WouldBlock before the peer is not an accept failure");
    assert!(empty_polls >= 1, "the helper never saw a real empty queue");
    let mut byte = [0_u8; 1];
    accepted
        .set_read_timeout(Some(super::EVENT_DEADLINE))
        .expect("bounded accepted read");
    accepted
        .read_exact(&mut byte)
        .expect("accepted native peer byte");
    assert_eq!(byte, *b"W");
}

#[test]
fn accept_before_refuses_a_queued_peer_after_the_original_deadline() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("own native listener");
    let deadline = Instant::now();
    let peer = TcpStream::connect(listener.local_addr().expect("native listener address"))
        .expect("queued native peer");
    // A blocking listener with a completed connect has a dequeuable peer, so
    // only the deadline can refuse it.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        accept_before(&listener, deadline)
    }));
    let panic = outcome.expect_err("a peer accepted after the deadline is not timely");
    assert_eq!(
        panic.downcast_ref::<&str>().copied(),
        Some("fixture peer was not accepted before its deadline")
    );
    let late = listener
        .accept()
        .expect("the queued peer was left unaccepted");
    assert_eq!(
        late.1,
        peer.local_addr().expect("queued native peer address")
    );
}

pub(crate) fn read_control_line(reader: &mut BufReader<UnixStream>) -> String {
    let mut line = String::new();
    let read = reader.read_line(&mut line).expect("read control line");
    assert_ne!(read, 0, "control EOF before observation");
    line.trim_end().to_owned()
}

pub(crate) fn wait_child_output(child: Child, timeout: Duration) -> Output {
    wait_child_output_observing(child, timeout, || {})
}

pub(crate) fn wait_child_output_observing(
    child: Child,
    timeout: Duration,
    mut observe: impl FnMut(),
) -> Output {
    // Install custody before pipe setup or callbacks can fail. Rust's Child
    // alone does not kill/reap on Drop.
    let mut child = ReapOnDrop(child);
    let mut stdout = PipeCapture::new(child.0.stdout.take());
    let mut stderr = PipeCapture::new(child.0.stderr.take());
    let deadline = Instant::now() + timeout;
    let mut status = None;
    loop {
        // Bound work per stream so continuous output cannot starve observation
        // or the deadline. No reader thread can outlive this wait on unwind.
        stdout.read_once();
        stderr.read_once();
        if status.is_none() {
            status = child.0.try_wait().expect("inspect child exit");
        }
        if let Some(status) = status
            && stdout.pipe.is_none()
            && stderr.pipe.is_none()
        {
            return Output {
                status,
                stdout: stdout.bytes,
                stderr: stderr.bytes,
            };
        }
        if status.is_none() {
            observe();
        }
        assert!(
            Instant::now() < deadline,
            "child exceeded exit/output deadline (status={status:?}, stdout_bytes={}, stderr_bytes={}, stderr_markers={:?})",
            stdout.bytes.len(),
            stderr.bytes.len(),
            redacted_stderr_markers(&stderr.bytes),
        );
        thread::yield_now();
    }
}

/// Reveal only fixed marker names on failure. Raw host stderr may contain
/// temporary file paths or runtime bootstrap details; never print its body.
fn redacted_stderr_markers(stderr: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .take(24)
        .map(|line| {
            line.split_whitespace()
                .next()
                .filter(|word| {
                    matches!(
                        *word,
                        "KELD_KEL142_BIND"
                            | "KELD_KEL140_PUBLIC_APP_COMPONENT"
                            | "KELD_KEL140_PUBLIC_QUIT_REQUEST"
                            | "KELD_KEL140_PUBLIC_FS_RETIRED"
                    )
                })
                .unwrap_or("[redacted]")
                .to_owned()
        })
        .collect()
}

#[test]
fn bounded_child_timeout_stderr_diagnostics_do_not_echo_untrusted_fields() {
    let markers = redacted_stderr_markers(
        b"KELD_KEL140_PUBLIC_QUIT_REQUEST calls=0 secret=value\nuntrusted /tmp/secret-material\nKELD_KEL142_BIND nonce=private\nKELD_KEL_BROKEN/path should-not-print\nKELD_KELSECRET_ABC123PRIVATE\n",
    );
    assert_eq!(
        markers,
        [
            "KELD_KEL140_PUBLIC_QUIT_REQUEST",
            "[redacted]",
            "KELD_KEL142_BIND",
            "[redacted]",
            "[redacted]"
        ]
    );
}

struct ReapOnDrop(Child);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        // Child caches an observed exit, so these do not signal a reused PID.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct PipeCapture<P> {
    pipe: Option<P>,
    bytes: Vec<u8>,
}

unsafe extern "C" {
    fn fcntl(fd: std::ffi::c_int, command: std::ffi::c_int, ...) -> std::ffi::c_int;
}

impl<P: Read + AsRawFd> PipeCapture<P> {
    fn new(pipe: Option<P>) -> Self {
        if let Some(pipe) = &pipe {
            // Darwin SDK sys/fcntl.h: retain every existing status flag while
            // enabling nonblocking reads for bounded fixture cleanup.
            const F_GETFL: std::ffi::c_int = 3;
            const F_SETFL: std::ffi::c_int = 4;
            const O_NONBLOCK: std::ffi::c_int = 4;
            // SAFETY: pipe owns this live descriptor for the entire borrow;
            // F_GETFL has no variadic argument and transfers no ownership.
            let flags = unsafe { fcntl(pipe.as_raw_fd(), F_GETFL) };
            assert!(
                flags >= 0,
                "read capture flags: {}",
                std::io::Error::last_os_error()
            );
            // SAFETY: same owned descriptor, integer F_SETFL argument from its
            // flags plus Darwin O_NONBLOCK. No pointers or lifetime extension.
            let result = unsafe { fcntl(pipe.as_raw_fd(), F_SETFL, flags | O_NONBLOCK) };
            assert_eq!(
                result,
                0,
                "nonblocking capture: {}",
                std::io::Error::last_os_error()
            );
        }
        Self {
            pipe,
            bytes: Vec::new(),
        }
    }

    fn read_once(&mut self) {
        let Some(pipe) = &mut self.pipe else { return };
        let mut buffer = [0; 8192];
        match pipe.read(&mut buffer) {
            Ok(0) => {
                self.pipe.take();
            }
            Ok(count) => self.bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => panic!("drain child pipe: {error}"),
        }
    }
}

pub(crate) fn parse_pid(field: Option<&str>, line: &str) -> u32 {
    field
        .unwrap_or_else(|| panic!("missing pid: {line}"))
        .parse()
        .unwrap_or_else(|error| panic!("invalid pid ({error}): {line}"))
}
