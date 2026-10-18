//! Owned temporary resources, process probes, and bounded test-control events.
use super::{
    Child, Command, Instant, NEXT_PATH_ID, Ordering, PROCESS_GONE_DEADLINE, Path, PathBuf,
    PermissionsExt, Read, Stdio, SystemTime, UNIX_EPOCH, UnixListener, UnixStream, Write, fs,
    thread,
};

pub(super) fn send_event(control: &Path, event: &str) {
    try_send_event(control, event).expect("send controller event");
}

pub(super) fn try_send_event(control: &Path, event: &str) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(control)?;
    stream.write_all(event.as_bytes())?;
    stream.write_all(b"\n")
}

pub(super) fn revoke_registered_link(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)?;
    let owner_dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "registered link has no owner directory",
        )
    })?;
    fs::remove_dir(owner_dir)
}

pub(super) fn accept_line_before(listener: &UnixListener, deadline: Instant) -> String {
    loop {
        assert!(
            Instant::now() < deadline,
            "controller event deadline expired before a complete line"
        );
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_nonblocking(true)
                    .expect("keep accepted controller stream nonblocking");
                let mut line = Vec::new();
                let mut chunk = [0_u8; 512];
                loop {
                    assert!(
                        Instant::now() < deadline,
                        "controller event deadline expired with incomplete line"
                    );
                    match stream.read(&mut chunk) {
                        Ok(0) => panic!("controller event EOF before newline"),
                        Ok(read) => {
                            let end = chunk[..read]
                                .iter()
                                .position(|byte| *byte == b'\n')
                                .map_or(read, |newline| newline + 1);
                            assert!(
                                line.len() + end <= 16 * 1024,
                                "controller event exceeds bounded fixture line"
                            );
                            line.extend_from_slice(&chunk[..end]);
                            if chunk[end - 1] == b'\n' {
                                assert!(
                                    Instant::now() < deadline,
                                    "controller event deadline expired before newline"
                                );
                                return String::from_utf8(line)
                                    .expect("controller event must be UTF-8")
                                    .trim_end()
                                    .to_owned();
                            }
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            thread::yield_now();
                        }
                        Err(error) => panic!("read controller event: {error}"),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::yield_now();
            }
            Err(error) => panic!("accept controller event: {error}"),
        }
    }
}

pub(super) fn parse_pid(field: Option<&str>, line: &str) -> u32 {
    field
        .unwrap_or_else(|| panic!("event omitted pid: {line}"))
        .parse()
        .unwrap_or_else(|error| panic!("event pid is invalid ({error}): {line}"))
}

fn process_exists(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn kill_pid(pid: u32) {
    let status = Command::new("/bin/kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .expect("invoke kill for one process");
    assert!(status.success(), "kill process {pid}: {status:?}");
}

pub(super) fn process_group(pid: u32) -> u32 {
    let output = Command::new("/bin/ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .expect("inspect process group");
    assert!(output.status.success(), "inspect process group for {pid}");
    String::from_utf8(output.stdout)
        .expect("process group output is UTF-8")
        .trim()
        .parse()
        .expect("process group output is numeric")
}

pub(super) fn parent_process(pid: u32) -> u32 {
    let output = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .expect("inspect parent process");
    assert!(output.status.success(), "inspect parent process for {pid}");
    String::from_utf8(output.stdout)
        .expect("parent process output is UTF-8")
        .trim()
        .parse()
        .expect("parent process output is numeric")
}

pub(super) fn process_rss_kib(pid: u32) -> u64 {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("inspect process RSS");
    assert!(output.status.success(), "inspect process RSS for {pid}");
    String::from_utf8(output.stdout)
        .expect("process RSS output is UTF-8")
        .trim()
        .parse()
        .expect("process RSS output is numeric")
}

pub(super) fn await_process_gone(pid: u32) {
    let deadline = Instant::now() + PROCESS_GONE_DEADLINE;
    while process_exists(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} survived the guardian's bounded reap deadline"
        );
        thread::yield_now();
    }
}

pub(super) struct CycleCleanup {
    pub(super) host: Option<Child>,
    pub(super) peer: Option<Child>,
    pub(super) group: Option<u32>,
    pub(super) leaker: Option<u32>,
    pub(super) group_gone: bool,
}

impl CycleCleanup {
    pub(super) fn new(host: Child) -> Self {
        Self {
            host: Some(host),
            peer: None,
            group: None,
            leaker: None,
            group_gone: false,
        }
    }
}

impl Drop for CycleCleanup {
    fn drop(&mut self) {
        if !self.group_gone
            && let Some(group) = self.group
        {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &format!("-{group}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if let Some(host) = self.host.as_mut() {
            let _ = host.kill();
            let _ = host.wait();
        }
        if let Some(peer) = self.peer.as_mut() {
            let _ = peer.kill();
            let _ = peer.wait();
        }
        if let Some(leaker) = self.leaker {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &leaker.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) control: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after Unix epoch")
            .as_nanos();
        let root = PathBuf::from(format!(
            "/tmp/k78-{}-{epoch_nanos:x}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create owner fixture directory");
        let mut permissions = fs::metadata(&root).expect("fixture metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&root, permissions).expect("set fixture permissions");
        let control = root.join("control.sock");
        Self { root, control }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.control);
        let _ = fs::remove_dir(&self.root);
    }
}

#[cfg(test)]
mod deadline_regressions {
    use super::{Fixture, UnixListener, UnixStream, accept_line_before};
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn split_control_event_finishing_before_deadline_remains_valid() {
        let fixture = Fixture::new();
        let listener = UnixListener::bind(&fixture.control).expect("bind private control fixture");
        listener.set_nonblocking(true).expect("nonblocking accept");
        let mut peer = UnixStream::connect(&fixture.control).expect("connect private peer");
        peer.write_all(b"GUARDIAN 123 ")
            .expect("first event fragment");
        let completion = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            peer.write_all(b"456\n")
                .expect("finish event before deadline");
        });
        assert_eq!(
            accept_line_before(&listener, Instant::now() + Duration::from_secs(1)),
            "GUARDIAN 123 456"
        );
        completion.join().expect("join owned peer");
    }

    #[test]
    fn partial_control_event_cannot_outlive_original_deadline() {
        let fixture = Fixture::new();
        let listener = UnixListener::bind(&fixture.control).expect("bind private control fixture");
        listener.set_nonblocking(true).expect("nonblocking accept");
        let mut peer = UnixStream::connect(&fixture.control).expect("connect private peer");
        peer.write_all(b"GUARDIAN 123")
            .expect("send incomplete control event");
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let held_peer = thread::spawn(move || {
            let _ = release_rx.recv_timeout(Duration::from_millis(750));
            drop(peer);
        });

        let started = Instant::now();
        let outcome = std::panic::catch_unwind(|| {
            accept_line_before(&listener, started + Duration::from_millis(100))
        });
        let elapsed = started.elapsed();
        // The sender can arrive after the peer's bounded auto-release in the
        // buggy implementation; that must not mask the product oracle.
        let _ = release_tx.send(());
        held_peer.join().expect("join owned peer");
        let failure = outcome.expect_err("incomplete event must never become accepted on EOF");
        let reason = failure
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| failure.downcast_ref::<&str>().copied())
            .expect("text deadline failure");
        assert!(
            reason.contains("deadline"),
            "failure must identify deadline expiration"
        );
        assert!(
            elapsed < Duration::from_millis(600),
            "incomplete event blocked longer than original deadline: {elapsed:?}"
        );
    }
}
