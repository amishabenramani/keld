//! Test-owned, kernel-reported exit phase of the source-attested original Guardian.
//!
//! The watcher is armed before releasing the selected public Quit. An absent
//! event is never evidence of liveness; a refused nonchild subscription never
//! becomes a successful kernel-exit observation.

use crate::support::process::ProcessExitObserver;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitPhase {
    NoEvent,
    Observed,
    Refused,
    Unproved,
}

pub(super) struct GuardianExitWitness {
    observer: Option<ProcessExitObserver>,
    phase: ExitPhase,
}

impl GuardianExitWitness {
    pub(super) fn arm(original_guardian: u32) -> Self {
        match ProcessExitObserver::try_arm(original_guardian) {
            Ok(mut observer) => match observer.try_poll_exit() {
                Ok(None) => Self {
                    observer: Some(observer),
                    phase: ExitPhase::NoEvent,
                },
                // An event already present before Quit is not a post-Quit witness.
                Ok(Some(_)) | Err(_) => Self {
                    observer: None,
                    phase: ExitPhase::Unproved,
                },
            },
            Err(_) => Self {
                observer: None,
                phase: ExitPhase::Refused,
            },
        }
    }

    /// Called only after the selected Bun has completed its real HTTP body
    /// receipt. The Host Child wait invokes this only while that Child is live.
    pub(super) fn poll(&mut self, deadline: Instant) {
        if self.phase != ExitPhase::NoEvent {
            return;
        }
        if Instant::now() >= deadline {
            self.phase = ExitPhase::Unproved;
            return;
        }
        let event = self
            .observer
            .as_mut()
            .expect("the armed original Guardian exit watcher")
            .try_poll_exit();
        // A zero-time kernel read can still be scheduled past the Host clock.
        if Instant::now() >= deadline {
            self.phase = ExitPhase::Unproved;
            return;
        }
        self.phase = match event {
            Ok(Some(_)) => ExitPhase::Observed,
            Ok(None) => ExitPhase::NoEvent,
            Err(_) => ExitPhase::Unproved,
        };
    }

    pub(super) const fn stage(&self) -> &'static str {
        match self.phase {
            ExitPhase::NoEvent => "guardian_exit_event_not_observed",
            ExitPhase::Observed => "guardian_exit_event_observed_before_host_exit",
            ExitPhase::Refused => "guardian_exit_event_registration_refused",
            ExitPhase::Unproved => "guardian_exit_event_unproved",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExitPhase, GuardianExitWitness};
    use crate::support::PROCESS_DEADLINE;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::Instant;

    struct OwnedShell(Child);

    impl Drop for OwnedShell {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Real kernel `NOTE_EXITSTATUS` of an owned blocked shell, not an injected
    /// event or a PID probe. The sole shell waits for test-owned stdin EOF.
    #[test]
    fn native_original_exit_event_requires_real_status23_before_deadline() {
        let mut child = OwnedShell(
            Command::new("/bin/sh")
                .args(["-c", "IFS= read -r line; exit 23"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start owned native status23 witness"),
        );
        let mut witness = GuardianExitWitness::arm(child.0.id());
        assert_eq!(
            witness.phase,
            ExitPhase::NoEvent,
            "a live owned shell cannot report an exit before release"
        );
        drop(
            child
                .0
                .stdin
                .take()
                .expect("release only owned shell stdin"),
        );
        let deadline = Instant::now() + PROCESS_DEADLINE;
        loop {
            witness.poll(deadline);
            if witness.phase == ExitPhase::Observed {
                break;
            }
            assert_eq!(
                witness.phase,
                ExitPhase::NoEvent,
                "a registered native watcher must not fabricate/refuse exit"
            );
            assert!(
                Instant::now() < deadline,
                "native status23 event missing within the original process deadline"
            );
            thread::yield_now();
        }
        assert_eq!(
            child.0.wait().expect("reap exact owned shell").code(),
            Some(23),
            "kernel event must correspond to the original exiting process"
        );
    }
}
