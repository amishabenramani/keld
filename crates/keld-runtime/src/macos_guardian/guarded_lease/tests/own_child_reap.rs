//! GH-824 Guardian side: the Guardian is its Bun leader's OS parent and
//! reaps that exact Child only after the group decision. When the host's
//! fail-safe signal lands just before the Guardian's revoke, the leader is
//! the Guardian's own unreaped zombie and keeps the group zombie-only
//! (`kill(-g, 0)` EPERM) until that reap. Each test runs the real lease
//! revoke SEQUENTIALLY on one thread (no concurrent reaper): the revoke must
//! reap its own Child first and only then prove absence, without first
//! burning an in-call wait that cannot succeed.
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Instant;

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;

use super::super::super::GUARDIAN_SHUTDOWN_DEADLINE;
use super::super::super::process_group::{
    LocalGroupWitness, TestGroupSignalPoint, install_test_group_signal_hook,
};
use super::{GenerationLease as _, RevocationCause, locally_enrolled_lease};

/// The test is the leader's OS parent, exactly as the Guardian is.
struct OwnedLeader(Child);

impl Drop for OwnedLeader {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn sleeper(group: i32) -> OwnedLeader {
    OwnedLeader(
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

fn raw(pid: u32) -> i32 {
    i32::try_from(pid).expect("positive native PID")
}

/// Await the kernel's zombie-only group state after a SIGKILL; the poll is
/// the awaited condition, the existing deadline only a kill switch.
fn await_zombie_only(group: i32) {
    let deadline = Instant::now() + GUARDIAN_SHUTDOWN_DEADLINE;
    while kill(Pid::from_raw(-group), None) != Err(Errno::EPERM) {
        assert!(Instant::now() < deadline, "group never became zombie-only");
        thread::yield_now();
    }
}

fn enrolled(leader: &OwnedLeader) -> LocalGroupWitness {
    let witness = LocalGroupWitness::default();
    witness
        .register_child(leader.0.id())
        .expect("kernel proves the owned direct leader");
    witness
}

/// The host's fail-safe `killpg` lands BEFORE the Guardian revoke: the
/// Guardian finds its own leader already a zombie (no inspectable birth,
/// group zombie-only) and must not report a failed revoke.
#[test]
fn guardian_revoke_retires_a_group_the_host_made_zombie_only_after_its_own_reap() {
    let mut leader = sleeper(0);
    let group = raw(leader.0.id());
    let witness = enrolled(&leader);
    killpg(Pid::from_raw(group), Signal::SIGKILL).expect("host fail-safe signal");
    await_zombie_only(group);

    let started = Instant::now();
    let result = locally_enrolled_lease(leader.0.id(), witness)
        .revoke_with_child(RevocationCause::Shutdown, &mut leader.0);
    let elapsed = started.elapsed();

    result.expect("the Guardian's own reap retires the group it parents");
    assert!(
        elapsed < GUARDIAN_SHUTDOWN_DEADLINE / 4,
        "no in-call absence wait before the Guardian's own reap: {elapsed:?}"
    );
    assert_eq!(kill(Pid::from_raw(-group), None), Err(Errno::ESRCH));
    assert_eq!(
        leader
            .0
            .try_wait()
            .expect("reaped status")
            .map(|status| status.signal()),
        Some(Some(9))
    );
}

/// The host's signal lands between the Guardian's birth check and its one
/// signal: the Guardian's own `killpg` is denied (zombie-only EPERM).
#[test]
fn guardian_revoke_reconciles_its_denied_signal_only_after_its_own_reap() {
    let mut leader = sleeper(0);
    let group = raw(leader.0.id());
    let witness = enrolled(&leader);
    let _hook = install_test_group_signal_hook(
        leader.0.id(),
        Box::new(move |point| {
            if point == TestGroupSignalPoint::BeforeSignal {
                killpg(Pid::from_raw(group), Signal::SIGKILL).expect("host fail-safe signal");
                await_zombie_only(group);
            }
        }),
    );

    let started = Instant::now();
    let result = locally_enrolled_lease(leader.0.id(), witness)
        .revoke_with_child(RevocationCause::Shutdown, &mut leader.0);
    let elapsed = started.elapsed();

    result.expect("a denied signal is reconciled after the Guardian's own reap");
    assert!(
        elapsed < GUARDIAN_SHUTDOWN_DEADLINE / 4,
        "no in-call absence wait before the Guardian's own reap: {elapsed:?}"
    );
    assert_eq!(kill(Pid::from_raw(-group), None), Err(Errno::ESRCH));
}

/// Negative control: another member's zombie that the Guardian does NOT
/// parent keeps the group zombie-only after the Guardian's reap. That is
/// not absence: the revoke must fail closed (no CLEAR), and only the real
/// parent's reap makes the group gone.
#[test]
fn guardian_revoke_never_clears_a_group_held_by_a_zombie_it_does_not_reap() {
    let mut leader = sleeper(0);
    let group = raw(leader.0.id());
    let mut other = sleeper(group);
    let witness = enrolled(&leader);
    killpg(Pid::from_raw(group), Signal::SIGKILL).expect("host fail-safe signal");
    await_zombie_only(group);

    let error = locally_enrolled_lease(leader.0.id(), witness)
        .revoke_with_child(RevocationCause::Shutdown, &mut leader.0)
        .expect_err("a zombie this Guardian does not reap is not absence");

    assert!(
        error
            .to_string()
            .contains("registered group identity unavailable"),
        "fail closed with the unproven-identity error: {error}"
    );
    assert_eq!(
        kill(Pid::from_raw(-group), None),
        Err(Errno::EPERM),
        "the group was neither signalled again nor forgotten"
    );
    assert_eq!(
        other.0.wait().expect("reap the other member").signal(),
        Some(9)
    );
    assert_eq!(kill(Pid::from_raw(-group), None), Err(Errno::ESRCH));
}
