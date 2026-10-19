//! Privacy-safe, non-authoritative public-FS test diagnostics.
//!
//! These markers are not protocol records or process authority. Their exact
//! literal contents must not depend on an untrusted observation, path, PID,
//! token, process image, or captured Host stdout/stderr.

use serde_json::Value;
use std::process::Output;

#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    PublicWitness,
    PublicCleanup,
    RecoveryBoot,
    RecoveryConsumer,
    RecoveryPage,
    Observation,
    ProcessDisplay,
    ProcessImageMatch,
    StagedFixtureObservation,
}

impl Phase {
    pub(super) const fn marker(self) -> &'static str {
        match self {
            Self::PublicWitness => "KELD_KEL140_PUBLIC_WITNESS",
            Self::PublicCleanup => "KELD_KEL140_PUBLIC_CLEANUP",
            Self::RecoveryBoot => "KELD_KEL140_RECOVERY_BOOT",
            Self::RecoveryConsumer => "KELD_KEL140_RECOVERY_CONSUMER",
            Self::RecoveryPage => "KELD_KEL140_RECOVERY_PAGE",
            Self::Observation => "KELD_KEL140_OBSERVATION",
            Self::ProcessDisplay => "KELD_KEL140_PROCESS_DISPLAY",
            Self::ProcessImageMatch => "KELD_KEL140_PROCESS_IMAGE_MATCH",
            Self::StagedFixtureObservation => "KELD_KEL143_STAGED_FIXTURE_OBSERVATION",
        }
    }
}

pub(super) fn observed_marker(_untrusted_report: &Value, phase: Phase) -> &'static str {
    // Deliberately never interpolate the report, including any optional token,
    // endpoint, embedded private root or executable image.
    phase.marker()
}

pub(super) fn host_exit_summary(output: &Output, report_count: usize) -> String {
    // The exit status is already asserted by the original test. Retain only
    // non-secret counts and success, not any stdout/stderr bytes or payloads.
    format!(
        "KELD_KEL140_PUBLIC_DIAGNOSTICS host_success={} stdout_bytes={} stderr_bytes={} reports={report_count}",
        output.status.success(),
        output.stdout.len(),
        output.stderr.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;

    #[test]
    fn private_host_and_observation_bytes_cannot_reach_fixed_diagnostics() {
        const PRIVATE: &str = "KELD_TEST_PRIVATE_SENTINEL_token_endpoint_root";
        let raw_report = serde_json::json!({
            "phase": "app-ready",
            "token": PRIVATE,
            "endpoint": PRIVATE,
            "executable": PRIVATE
        });
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: PRIVATE.as_bytes().to_vec(),
            stderr: PRIVATE.as_bytes().to_vec(),
        };
        let safe_host = host_exit_summary(&output, 3);
        assert!(safe_host.contains("host_success=true"));
        assert!(safe_host.contains("reports=3"));
        assert!(safe_host.contains("stderr_bytes="));
        assert!(!safe_host.contains(PRIVATE));
        assert!(!safe_host.contains("token"));
        assert!(!safe_host.contains("endpoint"));
        for phase in [
            Phase::PublicWitness,
            Phase::PublicCleanup,
            Phase::RecoveryBoot,
            Phase::RecoveryConsumer,
            Phase::RecoveryPage,
            Phase::Observation,
            Phase::ProcessDisplay,
            Phase::ProcessImageMatch,
            Phase::StagedFixtureObservation,
        ] {
            let marker = observed_marker(&raw_report, phase);
            assert!(marker.starts_with("KELD_KEL"));
            assert!(!marker.contains(PRIVATE));
            assert!(!marker.contains("token"));
            assert!(!marker.contains("endpoint"));
        }
    }

    #[test]
    fn known_untrusted_record_formatters_are_not_used_in_fixture_assertions() {
        // Structural regression guard for the private-record leak fixed in
        // the adjacent test owner. It complements the real sentinel formatter
        // test; it is not a general Rust log parser or comprehensive secret scan.
        let source = include_str!("../public_fs.rs");
        for raw in [
            "{report}",
            "{report:?}",
            "{output:?}",
            "host_output={output:?}",
            "ps_output={executable:?}",
        ] {
            assert!(
                !source.contains(raw),
                "an entire untrusted fixture record was reintroduced in a test diagnostic"
            );
        }
    }
}
