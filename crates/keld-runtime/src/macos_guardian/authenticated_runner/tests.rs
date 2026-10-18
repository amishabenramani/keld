//! Real KGR1 terminal failure must report denied native group cleanup without
//! waiting indefinitely for a still-executing, independently test-owned Child.
use super::run_live_group;
use crate::macos_guardian::process_group::{observe_group_identity, terminate_attested_group};
use nix::sys::signal::kill;
use nix::unistd::{Pid, getpgid, getpgrp, pipe};
use std::fs::File;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn denied_attested_kgr1_group_signal_never_blocks_on_live_direct_child_wait() {
    let child = Command::new("/bin/sleep")
        .arg("30")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("own original native KGR1-style group leader");
    let pid = child.id();
    let raw = i32::try_from(pid).expect("original native group PID");
    assert_ne!(
        Pid::from_raw(raw),
        getpgrp(),
        "never use the test runner's PGID"
    );
    let original = observe_group_identity(pid).expect("retain live original kernel birth");
    assert_eq!(getpgid(Some(Pid::from_raw(raw))), Ok(Pid::from_raw(raw)));

    let (reader, writer) = pipe().expect("owned true host-liveness pipe");
    drop(writer); // Actual host EOF is what starts terminal group cleanup.
    let stale_birth = original.altered_birth_for_test();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = run_live_group(child, stale_birth, File::from(reader), || Ok(()));
        let _ = tx.send(result);
    });

    // Sample the product result BEFORE independent cleanup. An intentional
    // wrong-birth group signal must not kill the executing original group.
    // The test subsequently rescues with the TRUE retained kernel birth even
    // in the deliberate failing old-code version: no leaked native process.
    let early = rx.recv_timeout(Duration::from_millis(400));
    let still_alive = kill(Pid::from_raw(raw), None).is_ok()
        && getpgid(Some(Pid::from_raw(raw))) == Ok(Pid::from_raw(raw));
    let rescue = terminate_attested_group(original);
    worker
        .join()
        .expect("terminal error worker joins after owned rescue");
    let reaped = rustix::process::waitpid(
        rustix::process::Pid::from_raw(raw),
        rustix::process::WaitOptions::empty(),
    );
    rescue.expect("original test-owned birth must permit native cleanup");
    assert!(
        early.is_ok(),
        "failed attested group cleanup may not block indefinitely in Child::wait"
    );
    let observed =
        reaped.expect("product returned without reaping, so test must own exact native wait");
    assert!(
        observed.is_some(),
        "original test-owned child was not externally reaped"
    );
    assert!(
        still_alive,
        "a false birth must not signal the original live group"
    );
    let error = early
        .expect("captured bounded result")
        .expect_err("refused group signal cannot be reported as clean KGR1 success");
    assert!(
        error
            .to_string()
            .contains("process-group leader PID was reused"),
        "retain the original native birth-mismatch failure: {error}"
    );
}
