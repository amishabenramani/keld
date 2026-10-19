//! Host process output is real native data, not a diagnostic authority.
use super::replacement_control_failure_summary;
use std::process::Command;

#[test]
fn real_child_private_output_is_not_printed_on_recovery_admission_failure() {
    const PRIVATE: &str = "KELD_TEST_PRIVATE_RECOVERY_OUTPUT_TOKEN_SENTINEL";
    // Real subprocess stdout+stderr enter an owned Output, as in the
    // recovery error path. No app/Guardian process or raw PID is involved.
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            "printf '%s\\n' '{PRIVATE}'; printf '%s\\n' '{PRIVATE}' >&2"
        ))
        .output()
        .expect("owned synthetic native producer");
    assert!(output.status.success(), "synthetic native producer");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(PRIVATE)
            && String::from_utf8_lossy(&output.stderr).contains(PRIVATE),
        "native producer must deliver private bytes on both real pipes"
    );

    let result = replacement_control_failure_summary(
        &format!("replacement: HELLO control failure at {PRIVATE}"),
        &output,
    );
    assert!(
        !result.contains(PRIVATE),
        "recovery diagnostics leaked private native process bytes"
    );
    assert!(
        result.contains("error_category=hello")
            && result.contains("host_success=true")
            && result.contains("stdout_bytes=")
            && result.contains("stderr_bytes="),
        "recovery failure must retain fixed category and count-only diagnostic evidence"
    );
}

#[test]
fn recovery_control_failure_callsite_uses_only_safe_summary() {
    // Source-binding catches a regression where a safe helper still exists
    // but the actual panic silently starts formatting the raw Output again.
    let source = include_str!("../recovery_cycle.rs");
    assert!(
        source.contains("panic!(\"{}\", replacement_control_failure_summary(&error, &output));"),
        "replacement failure panic must consume only the sanitized summary"
    );
    assert!(
        !source.contains("panic!(\"{error}; host output: {output:?}\")"),
        "replacement failure cannot inline private stdout or stderr"
    );
}
