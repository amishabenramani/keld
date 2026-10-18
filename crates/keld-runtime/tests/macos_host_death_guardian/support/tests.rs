//! Native controller-event and original-birth test-ownership negative controls.
use super::{CycleCleanup, Fixture, accept_line_before, retain_parented_birth};
use nix::unistd::{Pid, getpgid, getpgrp};
use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct OwnedSentinel(Child);

impl Drop for OwnedSentinel {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn native_sentinel() -> OwnedSentinel {
    let sentinel = OwnedSentinel(
        Command::new("/bin/cat")
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("own an independent live native group"),
    );
    let raw = i32::try_from(sentinel.0.id()).expect("valid PID");
    assert_ne!(getpgrp(), Pid::from_raw(raw));
    assert_eq!(getpgid(Some(Pid::from_raw(raw))), Ok(Pid::from_raw(raw)));
    sentinel
}

fn assert_sentinel_echo(sentinel: &mut OwnedSentinel) {
    let written = sentinel
        .0
        .stdin
        .as_mut()
        .expect("test owns sentinel stdin")
        .write_all(b"still-here\n");
    let mut stdout = sentinel.0.stdout.take().expect("test owns stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut received = [0_u8; 11];
        let read = stdout.read_exact(&mut received);
        let _ = tx.send(read.map(|()| received));
    });
    let outcome = rx.recv_timeout(Duration::from_secs(2));
    if outcome.is_err() {
        let _ = sentinel.0.kill();
    }
    reader
        .join()
        .expect("bounded native sentinel read terminates");
    assert!(written.is_ok(), "cleanup killed independent sentinel stdin");
    assert!(
        matches!(outcome, Ok(Ok(bytes)) if bytes == *b"still-here\n"),
        "unverified fixture cleanup must leave independently owned native sentinel live"
    );
}

#[test]
fn unverified_cleanup_group_marker_cannot_signal_unrelated_native_group() {
    let mut sentinel = native_sentinel();
    let host = Command::new("/usr/bin/true")
        .spawn()
        .expect("own completed harmless host");
    let mut cleanup = CycleCleanup::new(host);
    cleanup.group = Some(sentinel.0.id());
    drop(cleanup);
    assert_sentinel_echo(&mut sentinel);
}

#[test]
fn unverified_cleanup_leaker_marker_cannot_signal_unrelated_native_process() {
    let mut sentinel = native_sentinel();
    let host = Command::new("/usr/bin/true")
        .spawn()
        .expect("own completed harmless host");
    let mut cleanup = CycleCleanup::new(host);
    cleanup.leaker = Some(sentinel.0.id());
    drop(cleanup);
    assert_sentinel_echo(&mut sentinel);
}

#[test]
fn altered_original_birth_cannot_signal_same_numeric_native_group() {
    let mut sentinel = native_sentinel();
    let original = retain_parented_birth(sentinel.0.id(), std::process::id());
    let mut stale = original;
    stale.started_usec = stale.started_usec.wrapping_add(1);
    let host = Command::new("/usr/bin/true")
        .spawn()
        .expect("own completed harmless host");
    let mut cleanup = CycleCleanup::new(host);
    cleanup.group = Some(sentinel.0.id());
    cleanup.group_birth = Some(stale);
    drop(cleanup);
    assert_sentinel_echo(&mut sentinel);
}

#[test]
fn original_kernel_birth_allows_cleanup_of_exact_test_owned_native_group() {
    use std::os::unix::process::ExitStatusExt as _;

    let mut sentinel = native_sentinel();
    let original = retain_parented_birth(sentinel.0.id(), std::process::id());
    let host = Command::new("/usr/bin/true")
        .spawn()
        .expect("own completed harmless host");
    let mut cleanup = CycleCleanup::new(host);
    cleanup.group = Some(original.pid);
    cleanup.group_birth = Some(original);
    drop(cleanup);
    assert_eq!(
        sentinel
            .0
            .wait()
            .expect("reap test-owned group leader after birth-attested cleanup")
            .signal(),
        Some(9),
        "valid original test-owned kernel birth must retain safe cleanup"
    );
}

#[test]
fn original_kernel_birth_allows_cleanup_of_exact_test_owned_leaker() {
    use std::os::unix::process::ExitStatusExt as _;

    let mut sentinel = native_sentinel();
    let original = retain_parented_birth(sentinel.0.id(), std::process::id());
    let host = Command::new("/usr/bin/true")
        .spawn()
        .expect("own completed harmless host");
    let mut cleanup = CycleCleanup::new(host);
    cleanup.leaker = Some(original.pid);
    cleanup.leaker_birth = Some(original);
    drop(cleanup);
    assert_eq!(
        sentinel
            .0
            .wait()
            .expect("reap original native leaker after birth-attested cleanup")
            .signal(),
        Some(9),
        "valid original kernel birth must retain safe leaker cleanup"
    );
}

#[test]
fn accepted_partial_controller_record_eof_is_not_a_valid_event() {
    let fixture = Fixture::new();
    let listener = UnixListener::bind(&fixture.control).expect("bind private event socket");
    listener
        .set_nonblocking(true)
        .expect("bounded controller accept");
    let mut sender = UnixStream::connect(&fixture.control).expect("own event sender");
    sender
        .write_all(b"GUARDIAN 123")
        .expect("send incomplete event");
    drop(sender);
    let outcome = std::panic::catch_unwind(|| {
        accept_line_before(&listener, Instant::now() + Duration::from_millis(200))
    });
    assert!(
        outcome.is_err(),
        "controller must refuse EOF before a newline-terminated record"
    );
}

#[test]
fn accepted_partial_controller_record_respects_original_deadline() {
    let fixture = Fixture::new();
    let listener = UnixListener::bind(&fixture.control).expect("bind private event socket");
    listener
        .set_nonblocking(true)
        .expect("bounded controller accept");
    let mut sender = UnixStream::connect(&fixture.control).expect("own event sender");
    sender
        .write_all(b"GUARDIAN 123")
        .expect("send incomplete event");
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(150);
        let outcome = std::panic::catch_unwind(|| accept_line_before(&listener, deadline));
        let _ = tx.send(outcome.is_err());
    });
    // A broken blocking read must always be released and the thread joined
    // before asserting RED: never strand the native test process.
    let timely = rx.recv_timeout(Duration::from_millis(400));
    drop(sender);
    worker
        .join()
        .expect("owned controller reader joins after sender EOF");
    assert_eq!(
        timely,
        Ok(true),
        "accepted partial record must fail by the original deadline before sender EOF"
    );
}
