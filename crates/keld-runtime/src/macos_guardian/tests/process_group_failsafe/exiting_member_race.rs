//! GH-824 host CLEAR trigger: the last live member of an attested group
//! finishes exiting between the cleanup's checks and its one group SIGKILL.
//!
//! XNU `killpg1` counts an exiting member as found but skips `SZOMB` members
//! and returns EPERM when it found none. The group is then zombie-only: the
//! same ambiguous state the read-only absence owner already handles. These
//! native tests put the member's exit exactly in that window through a
//! test-only fault point, instead of relying on scheduler timing. The tests
//! and fault point follow #1077 (main); here the denied signal is classified
//! by the one signal-phase EPERM classifier and decided by the one absence
//! owner, never by a second signal.
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
use super::super::test_support::await_held_zombie_group;
use super::TestOwnedGroupChild;

const DENIED_SIGNAL: &str = "macOS guardian process-group signal";

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

fn pid_of(child: &TestOwnedGroupChild) -> Pid {
    Pid::from_raw(i32::try_from(child.0.id()).expect("positive native PID"))
}

/// A reaped original leader plus one live, test-owned member of its group.
fn reaped_leader_with_live_member() -> (GroupIdentity, i32, TestOwnedGroupChild) {
    let mut leader = sleeper(0);
    let group = leader.0.id();
    let original = observe_group_identity(group).expect("original kernel group birth");
    let raw = i32::try_from(group).expect("positive native group PID");
    let member = sleeper(raw);
    assert_eq!(
        nix::unistd::getpgid(Some(pid_of(&member))),
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
/// process and await the kernel's zombie-only group state through the shared
/// fixture owner. The poll is the awaited condition; its bound is a kill switch.
fn make_group_zombie_only(raw: i32, last_live: Pid) -> Box<dyn FnOnce()> {
    Box::new(move || {
        kill(last_live, Signal::SIGKILL).expect("kill exact test-owned process");
        await_held_zombie_group(Pid::from_raw(raw));
    })
}

/// The test reaps `held` only AFTER the shipping owner has itself observed the
/// real (leader ESRCH, group EPERM) state its own denied signal left behind.
fn reap_after_owner_observes(
    group: u32,
    mut held: TestOwnedGroupChild,
) -> (
    super::super::super::process_group::TestAmbiguousProbeReceipt,
    thread::JoinHandle<(bool, std::process::ExitStatus)>,
) {
    let (observed_tx, observed_rx) = mpsc::channel();
    let receipt = install_test_ambiguous_probe_receipt(group, observed_tx);
    let reaper = thread::spawn(move || {
        let observed = observed_rx.recv_timeout(GUARDIAN_SHUTDOWN_DEADLINE).is_ok();
        let status = held.0.wait().expect("reap killed group process");
        (observed, status)
    });
    (receipt, reaper)
}

fn assert_denied_signal(error: &super::super::super::RuntimeError) {
    assert!(
        matches!(
            error,
            super::super::super::RuntimeError::Lifecycle { phase, source }
                if *phase == DENIED_SIGNAL && source.raw_os_error() == Some(nix::libc::EPERM)
        ),
        "an unproven group keeps the typed signal-phase EPERM: {error}"
    );
}

#[test]
fn member_exit_between_probe_and_group_signal_retires_after_reap_without_host_fault() {
    let (original, raw, member) = reaped_leader_with_live_member();
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, pid_of(&member)));
    let (_receipt, reaper) = reap_after_owner_observes(original.pid, member);

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
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, pid_of(&member)));

    let started = Instant::now();
    let error = terminate_attested_group(original)
        .expect_err("EPERM alone is never proof that the group is gone");
    let elapsed = started.elapsed();

    assert_denied_signal(&error);
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

// After its one denied signal, the call never signals again, even when a
// process joins the group and makes it signallable while the owner polls.
#[test]
fn denied_group_signal_is_never_repeated_when_the_group_becomes_signallable() {
    let (original, raw, mut member) = reaped_leader_with_live_member();
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, pid_of(&member)));
    let (observed_tx, observed_rx) = mpsc::channel();
    let _receipt = install_test_ambiguous_probe_receipt(original.pid, observed_tx);
    let joiner = thread::spawn(move || {
        observed_rx
            .recv_timeout(GUARDIAN_SHUTDOWN_DEADLINE)
            .expect("owner observed the zombie-only group after its denied signal");
        let joiner = sleeper(raw);
        assert_eq!(kill(Pid::from_raw(-raw), None), Ok(()));
        joiner
    });

    let error = terminate_attested_group(original)
        .expect_err("a group signallable again after the denial is not absent");
    let mut joiner = joiner.join().expect("joiner thread");

    assert_denied_signal(&error);
    assert!(
        joiner.0.try_wait().expect("inspect joiner").is_none(),
        "the call must not signal the group a second time"
    );
    kill(pid_of(&joiner), Signal::SIGTERM).expect("signal the test-owned joiner");
    assert_eq!(
        joiner.0.wait().expect("reap joiner").signal(),
        Some(Signal::SIGTERM as i32),
        "only the test's SIGTERM ended the joiner"
    );
    assert_eq!(member.0.wait().expect("reap member").signal(), Some(9));
}

/// A live, birth-matched original leader alone in its group.
fn attested_live_leader() -> (GroupIdentity, i32, TestOwnedGroupChild) {
    let leader = sleeper(0);
    let group = leader.0.id();
    let original = observe_group_identity(group).expect("original kernel group birth");
    let raw = i32::try_from(group).expect("positive native group PID");
    assert_eq!(kill(Pid::from_raw(-raw), None), Ok(()));
    (original, raw, leader)
}

// Birth-matched arm: the leader passed the identity check, then became a
// zombie before the one signal. The denied signal goes to the same owner.
#[test]
fn attested_leader_exit_before_group_signal_retires_after_reap_without_host_fault() {
    let (original, raw, leader) = attested_live_leader();
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, pid_of(&leader)));
    let (_receipt, reaper) = reap_after_owner_observes(original.pid, leader);

    let result = terminate_attested_group(original);
    let (observed, status) = reaper.join().expect("leader reaper joins");

    result.expect("a leader that exited into a zombie-only group is retired once reaped");
    assert!(observed, "owner awaited the reap instead of guessing");
    assert_eq!(status.signal(), Some(9));
    assert_eq!(kill(Pid::from_raw(-raw), None), Err(Errno::ESRCH));
}

#[test]
fn attested_leader_exit_before_group_signal_never_clears_an_unreaped_group() {
    let (original, raw, mut leader) = attested_live_leader();
    install_test_before_group_signal(original.pid, make_group_zombie_only(raw, pid_of(&leader)));

    let error = terminate_attested_group(original)
        .expect_err("a held zombie leader is never proof that the group is gone");

    assert_denied_signal(&error);
    assert_eq!(kill(Pid::from_raw(-raw), None), Err(Errno::EPERM));
    assert_eq!(leader.0.wait().expect("reap leader").signal(), Some(9));
    assert_eq!(kill(Pid::from_raw(-raw), None), Err(Errno::ESRCH));
}
