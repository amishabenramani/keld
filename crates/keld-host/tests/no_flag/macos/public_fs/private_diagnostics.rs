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

/// Assert an untrusted report's `phase` field. A failure names only the fixed
/// marker and the expected literal, never the report or the observed value.
pub(super) fn assert_report_phase(report: &Value, expected: &str, phase: Phase) {
    assert!(
        report["phase"] == expected,
        "{} expected phase {expected}",
        observed_marker(report, phase)
    );
}

/// Success and byte counts of a captured child `Output`, never its bytes.
pub(super) fn output_summary(output: &Output) -> String {
    format!(
        "success={} stdout_bytes={} stderr_bytes={}",
        output.status.success(),
        output.stdout.len(),
        output.stderr.len()
    )
}

pub(super) fn host_exit_summary(output: &Output, report_count: usize) -> String {
    // The exit status is already asserted by the original test. Retain only
    // non-secret counts and success, not any stdout/stderr bytes or payloads.
    format!(
        "KELD_KEL140_PUBLIC_DIAGNOSTICS host_{} reports={report_count}",
        output_summary(output)
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
    fn report_phase_failure_names_only_the_fixed_marker_and_expected_phase() {
        const PRIVATE: &str = "KELD_TEST_PRIVATE_SENTINEL_phase_token";
        assert_report_phase(
            &serde_json::json!({ "phase": "app-ready", "token": PRIVATE }),
            "app-ready",
            Phase::RecoveryBoot,
        );
        let leaked = serde_json::json!({ "phase": PRIVATE, "token": PRIVATE });
        let failure = std::panic::catch_unwind(|| {
            assert_report_phase(&leaked, "app-ready", Phase::RecoveryBoot);
        })
        .expect_err("a wrong phase still fails the original predicate");
        let message = failure
            .downcast_ref::<String>()
            .expect("formatted assertion message");
        assert!(message.contains("KELD_KEL140_RECOVERY_BOOT"), "{message}");
        assert!(message.contains("app-ready"), "{message}");
        assert!(!message.contains(PRIVATE));
    }

    #[test]
    fn known_untrusted_record_formatters_are_not_used_in_fixture_assertions() {
        // Structural regression guard for the private-record leak fixed in
        // the adjacent test owner. It complements the real sentinel formatter
        // test; it is not a general Rust log parser or comprehensive secret scan.
        let source = include_str!("../public_fs.rs");
        assert!(
            untrusted_interpolations(source).is_empty(),
            "an untrusted fixture record, Output or Host stream was reintroduced in a \
             test diagnostic: {:?}",
            untrusted_interpolations(source)
        );
    }

    #[test]
    fn untrusted_interpolation_guard_sees_every_known_binding_shape() {
        // Negative control for the structural guard: each shape that once
        // leaked (named record, Debug Output, raw Host stderr, positional
        // Debug) must be reported, and fixed markers must not be.
        for leaked in [
            r#"assert_eq!(a, b, "{started}");"#,
            r#"assert_eq!(a, b, "{admitted}");"#,
            r#"assert_eq!(a, b, "{ready}");"#,
            r#"assert!(ok, "{stderr}");"#,
            r#"panic!("late: {late_report:?}");"#,
            r#"assert!(ok, "{:?}", evidence.click);"#,
            r#"eprintln!("host_output={output:?}");"#,
            r#"eprintln!("{report}");"#,
        ] {
            assert!(
                !untrusted_interpolations(leaked).is_empty(),
                "guard missed {leaked}"
            );
        }
        assert!(untrusted_interpolations(r#"eprintln!("{}", marker); "{phase} {key}""#).is_empty());
    }

    // Untrusted bindings in public_fs.rs: app/observer reports, captured
    // child Output and Host stdout/stderr. Positional Debug formatting is
    // rejected outright because its argument cannot be checked textually.
    fn untrusted_interpolations(source: &str) -> Vec<String> {
        const BINDINGS: [&str; 12] = [
            "report",
            "result",
            "started",
            "admitted",
            "ready",
            "late_report",
            "output",
            "click",
            "stderr",
            "stdout",
            "executable",
            "ps_output",
        ];
        let mut found = Vec::new();
        for binding in BINDINGS {
            for shape in [format!("{{{binding}}}"), format!("{{{binding}:")] {
                if source.contains(&shape) {
                    found.push(shape);
                }
            }
        }
        for positional in ["{:?}", "{:#?}"] {
            if source.contains(positional) {
                found.push(positional.to_owned());
            }
        }
        found
    }
}
