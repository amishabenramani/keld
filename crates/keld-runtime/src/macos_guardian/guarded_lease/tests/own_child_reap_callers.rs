//! GH-824 D1, one contract per Guardian-side parent caller of
//! `retire_own_child_group`: the guarded lease, the supervised lease, and the
//! legacy one-group runner (`run_live_group` and `reject_spawn`). Each caller
//! is its Bun leader's OS parent, so it must decide the group (at most one
//! signal, no in-call wait) and prove absence only after reaping its exact
//! Child. A caller that signals or waits before its own reap burns an in-call
//! absence wait that cannot succeed while its own zombie holds the group.
//!
//! Each caller runs twice: once with the host's fail-safe signal landing
//! before the call (no signal is authorized) and once with it landing between
//! the call's birth check and its one signal (that signal is denied). The
//! test hook counts every group signal the product prepares.
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt as _;
use std::process::{Child, ExitStatus};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;

use super::super::super::authenticated_runner::{reject_spawn, run_live_group};
use super::super::super::process_group::{
    GroupIdentity, LocalGroupWitness, TestGroupSignalPoint, install_test_group_signal_hook,
    validate_guardian_child_group,
};
use super::super::super::supervised_lease::SupervisedGuardianLease;
use super::super::super::{GUARDIAN_SHUTDOWN_DEADLINE, RuntimeError, io, lifecycle_error};
use super::own_child_reap::{await_zombie_only, raw, sleeper_child};
use super::{GenerationLease as _, RevocationCause, locally_enrolled_lease};

/// Where the host's fail-safe `killpg` lands relative to the caller's call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostSignal {
    /// Before the call: the leader is already the caller's own unreaped
    /// zombie, so the decision is ambiguous and sends no signal.
    BeforeCall,
    /// Between the call's birth check and its one signal, which the kernel
    /// then denies (zombie-only EPERM).
    BeforeCallerSignal,
}

/// The caller's result and, when it exposes one, its reaped leader status.
type CallerOutcome = (Result<(), RuntimeError>, Option<ExitStatus>);

fn assert_caller_decides_only_after_its_own_reap(
    host: HostSignal,
    caller: impl FnOnce(Child, GroupIdentity, LocalGroupWitness) -> CallerOutcome,
) {
    let leader = sleeper_child(0);
    let pid = leader.id();
    let group = raw(pid);
    let identity = validate_guardian_child_group(pid, std::process::id())
        .expect("kernel birth of the owned leader");
    let witness = LocalGroupWitness::default();
    witness
        .register_child(pid)
        .expect("kernel proves the owned direct leader");
    let signals: Arc<Mutex<u32>> = Arc::default();
    let count = Arc::clone(&signals);
    let _hook = install_test_group_signal_hook(
        pid,
        Box::new(move |point| {
            if point == TestGroupSignalPoint::BeforeSignal {
                let mut prepared = count.lock().expect("signal count");
                *prepared += 1;
                if host == HostSignal::BeforeCallerSignal && *prepared == 1 {
                    killpg(Pid::from_raw(group), Signal::SIGKILL).expect("host fail-safe signal");
                    await_zombie_only(group);
                }
            }
        }),
    );
    if host == HostSignal::BeforeCall {
        killpg(Pid::from_raw(group), Signal::SIGKILL).expect("host fail-safe signal");
        await_zombie_only(group);
    }

    let started = Instant::now();
    let (result, status) = caller(leader, identity, witness);
    let elapsed = started.elapsed();

    result.expect("the caller's own reap retires the group it parents");
    assert!(
        elapsed < GUARDIAN_SHUTDOWN_DEADLINE / 4,
        "no in-call absence wait before the caller's own reap: {elapsed:?}"
    );
    assert_eq!(
        *signals.lock().expect("signal count"),
        u32::from(host == HostSignal::BeforeCallerSignal),
        "exactly the decision's signals, at most one per call"
    );
    assert_eq!(
        kill(Pid::from_raw(group), None),
        Err(Errno::ESRCH),
        "the caller reaped its own leader"
    );
    assert_eq!(kill(Pid::from_raw(-group), None), Err(Errno::ESRCH));
    if let Some(status) = status {
        assert_eq!(status.signal(), Some(9), "the host's SIGKILL ended it");
    }
}

fn guarded_lease(
    mut child: Child,
    _identity: GroupIdentity,
    witness: LocalGroupWitness,
) -> CallerOutcome {
    let result = locally_enrolled_lease(child.id(), witness)
        .revoke_with_child(RevocationCause::Shutdown, &mut child);
    (result, child.try_wait().expect("cached reap status"))
}

fn supervised_lease(
    mut child: Child,
    _identity: GroupIdentity,
    witness: LocalGroupWitness,
) -> CallerOutcome {
    let result = SupervisedGuardianLease::locally_enrolled_for_test(child.id(), witness)
        .revoke_with_child(RevocationCause::Shutdown, &mut child);
    (result, child.try_wait().expect("cached reap status"))
}

fn live_group_runner(
    child: Child,
    identity: GroupIdentity,
    _witness: LocalGroupWitness,
) -> CallerOutcome {
    let (liveness, host) = UnixStream::pair().expect("host-liveness pair");
    drop(host);
    match run_live_group(child, identity, liveness, || Ok(())) {
        Ok(report) => (Ok(()), Some(report.leader_status())),
        Err(error) => (Err(error), None),
    }
}

fn rejected_spawn_runner(
    child: Child,
    identity: GroupIdentity,
    _witness: LocalGroupWitness,
) -> CallerOutcome {
    let primary = || {
        lifecycle_error(
            "macOS guardian pre-exec registration",
            io::Error::other("test-rejected registration"),
        )
    };
    let error = reject_spawn(primary(), None, Some(identity), Some(child), || Ok(()));
    // The rejection is the primary error alone: any cleanup failure would be
    // collapsed into it.
    let result = if error.to_string() == primary().to_string() {
        Ok(())
    } else {
        Err(error)
    };
    (result, None)
}

#[test]
fn guarded_lease_reaps_before_deciding_a_group_the_host_made_zombie_only() {
    assert_caller_decides_only_after_its_own_reap(HostSignal::BeforeCall, guarded_lease);
}

#[test]
fn guarded_lease_reconciles_its_denied_signal_only_after_its_own_reap() {
    assert_caller_decides_only_after_its_own_reap(HostSignal::BeforeCallerSignal, guarded_lease);
}

#[test]
fn supervised_lease_reaps_before_deciding_a_group_the_host_made_zombie_only() {
    assert_caller_decides_only_after_its_own_reap(HostSignal::BeforeCall, supervised_lease);
}

#[test]
fn supervised_lease_reconciles_its_denied_signal_only_after_its_own_reap() {
    assert_caller_decides_only_after_its_own_reap(HostSignal::BeforeCallerSignal, supervised_lease);
}

#[test]
fn live_group_runner_reaps_before_deciding_a_group_the_host_made_zombie_only() {
    assert_caller_decides_only_after_its_own_reap(HostSignal::BeforeCall, live_group_runner);
}

#[test]
fn live_group_runner_reconciles_its_denied_signal_only_after_its_own_reap() {
    assert_caller_decides_only_after_its_own_reap(
        HostSignal::BeforeCallerSignal,
        live_group_runner,
    );
}

#[test]
fn rejected_spawn_reaps_before_deciding_a_group_the_host_made_zombie_only() {
    assert_caller_decides_only_after_its_own_reap(HostSignal::BeforeCall, rejected_spawn_runner);
}

#[test]
fn rejected_spawn_reconciles_its_denied_signal_only_after_its_own_reap() {
    assert_caller_decides_only_after_its_own_reap(
        HostSignal::BeforeCallerSignal,
        rejected_spawn_runner,
    );
}
