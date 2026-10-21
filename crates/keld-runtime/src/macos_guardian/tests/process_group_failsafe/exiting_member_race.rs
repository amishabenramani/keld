//! GH-824 host CLEAR trigger: the last live member of an attested group
//! finishes exiting between the zero-signal probe and the one group SIGKILL.
//!
//! XNU `killpg1` counts an exiting member as found but skips `SZOMB` members
//! and returns EPERM when it found none. The group is then zombie-only: the
//! same ambiguous state the read-only absence probe already handles. These
//! native tests put the member's exit exactly in that window through a
//! test-only fault point, instead of relying on scheduler timing.
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use super::super::super::GUARDIAN_SHUTDOWN_DEADLINE;
use super::super::super::process_group::{
    GroupIdentity, install_test_ambiguous_probe_receipt, install_test_before_group_signal,
    observe_group_identity, terminate_attested_group,
};
use super::TestOwnedGroupChild;

fn sleeper(group: i32) -> TestOwnedGroupChild {
    TestOwnedGroupChild(
        Command::new("/bin/sleep")
            .arg("30")
            .process_group(group)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("test-owned native group process"),
    )
}

/// A reaped original leader plus one live, test-owned member of its group.
fn reaped_leader_with_live_member() -> (GroupIdentity, i32, TestOwnedGroupChild) {
    let mut leader = sleeper(0);
    let group = leader.0.id();
    let original = observe_group_identity(group).expect("original kernel group birth");
    let raw = i32::try_from(group).expect("positive native group PID");
    let member = sleeper(raw);
    let member_pid = Pid::from_raw(i32::try_from(member.0.id()).expect("member PID"));
    assert_eq!(
        nix::unistd::getpgid(Some(member_pid)),
        Ok(Pid::from_raw(raw))
    );
    leader.0.kill().expect("kill exact test-owned leader");
    assert_eq!(leader.0.wait().expect("reap leader").signal(), Some(9));
    assert_eq!(
        nix::unistd::getpgid(Some(Pid::from_raw(raw))),
        Err(Errno::ESRCH)
    );
    assert_eq!(
        kill(Pid::from_raw(-raw), None),
        Ok(()),
        "live member keeps the group signallable"
    );
    (original, raw, member)
}

/// Fault point body: kill the still-unreaped (so never recycled) test-owned
/// member and await the kernel's zombie-only group state. The poll is the
/// awaited condition; the deadline is only a kill switch.
fn make_group_zombie_only(raw: i32, member: Pid) -> Box<dyn FnOnce()> {
    Box::new(move || {
        kill(member, Signal::SIGKILL).expect("kill exact test-owned member");
        let deadline = Instant::now() + GUARDIAN_SHUTDOWN_DEADLINE;
        while kill(Pid::from_raw(-raw), None) != Err(Errno::EPERM) {
            assert!(
                Instant::now() < deadline,
                "member never became a zombie-only group"
            );
            thread::yield_now();
        }
    })
}

#[test]
fn member_exit_between_probe_and_group_signal_retires_after_reap_without_host_fault() {
    let (original, raw, mut member) = reaped_leader_with_live_member();
    let member_pid = Pid::from_raw(i32::try_from(member.0.id()).expect("member PID"));
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, member_pid));
    // The member's parent (this test) reaps it only AFTER the shipping owner
    // has itself observed the real (leader ESRCH, group EPERM) state that its
    // own denied signal left behind.
    let (observed_tx, observed_rx) = mpsc::channel();
    let _receipt = install_test_ambiguous_probe_receipt(original.pid, observed_tx);
    let reaper = thread::spawn(move || {
        let observed = observed_rx.recv_timeout(GUARDIAN_SHUTDOWN_DEADLINE).is_ok();
        let status = member.0.wait().expect("reap killed group member");
        (observed, status)
    });

    let result = terminate_attested_group(original);
    let (observed, status) = reaper.join().expect("member reaper joins");

    result.expect("a group made zombie-only by its last exit is retired once reaped");
    assert!(observed, "owner awaited the reap instead of guessing");
    assert_eq!(status.signal(), Some(9));
    assert_eq!(kill(Pid::from_raw(-raw), None), Err(Errno::ESRCH));
}

#[test]
fn member_exit_between_probe_and_group_signal_never_clears_an_unreaped_group() {
    let (original, raw, mut member) = reaped_leader_with_live_member();
    let member_pid = Pid::from_raw(i32::try_from(member.0.id()).expect("member PID"));
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, member_pid));

    let started = Instant::now();
    let error = terminate_attested_group(original)
        .expect_err("EPERM alone is never proof that the group is gone");
    let elapsed = started.elapsed();

    assert!(
        error
            .to_string()
            .contains("registered group identity unavailable"),
        "a persistent zombie-only group must end in the bounded absence probe: {error}"
    );
    assert!(
        elapsed < GUARDIAN_SHUTDOWN_DEADLINE,
        "the absence probe stays inside the existing shutdown budget: {elapsed:?}"
    );
    assert_eq!(
        kill(Pid::from_raw(-raw), None),
        Err(Errno::EPERM),
        "the owner neither forgot nor re-signalled the unproven group"
    );
    assert_eq!(member.0.wait().expect("reap member").signal(), Some(9));
    assert_eq!(kill(Pid::from_raw(-raw), None), Err(Errno::ESRCH));
}
