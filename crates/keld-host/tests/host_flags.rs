//! Cross-platform host argument boundary.
#![allow(clippy::expect_used)] // integration-test process/output assertions

use std::process::Command;

#[cfg(windows)]
#[test]
fn malformed_windows_utf16_argv_is_cli_044_not_a_panic() {
    use std::os::windows::ffi::OsStringExt as _;

    // An unpaired surrogate is representable in the Windows native argv but
    // cannot be converted by std::env::args(). The host must refuse this
    // unsupported CLI argument before any boot or WebView2 authority exists.
    let malformed = std::ffi::OsString::from_wide(&[0xd800]);
    let output = Command::new(env!("CARGO_BIN_EXE_keld-host"))
        .arg(malformed)
        .output()
        .expect("launch host with a test-owned malformed Unicode argument");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("fixed error uses UTF-8");
    assert!(stderr.contains("KELD-CLI-044"), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
    assert!(!stderr.contains("KELD-CORE-034"), "{stderr}");
}

#[cfg(windows)]
#[test]
fn hello_flag_after_unknown_argument_never_selects_hello_window() {
    // A flag in argv[2] is not the host command. The old any-position
    // scan incorrectly selected the hello-mode parser and could admit a
    // GUI when the earlier token happened to be a valid title option.
    let output = Command::new(env!("CARGO_BIN_EXE_keld-host"))
        .args(["--not-a-keld-host-flag", "--hello"])
        .output()
        .expect("run host with an out-of-position hello flag");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("fixed host refusal is UTF-8");
    assert!(
        stderr.contains("KELD-CLI-044: unknown host argument"),
        "{stderr}"
    );
    assert!(!stderr.contains("unknown hello flag"), "{stderr}");
    assert!(!stderr.contains("KELD-WV-008"), "{stderr}");
}

#[cfg(windows)]
#[test]
fn repeated_hello_command_is_refused_before_webview_setup() {
    // The original parser accepted a second --hello as an ordinary hello
    // option. The trailing unknown argument makes that buggy path terminate
    // without opening a GUI, so the failure-first test is bounded.
    let output = Command::new(env!("CARGO_BIN_EXE_keld-host"))
        .args(["--hello", "--hello", "--not-a-keld-option"])
        .output()
        .expect("run host with repeated diagnostic role command");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("fixed diagnostic refusal is UTF-8");
    assert!(
        stderr.contains("KELD-CLI-044: unknown host argument"),
        "{stderr}"
    );
    assert!(!stderr.contains("unknown hello flag"), "{stderr}");
    assert!(
        !stderr.contains("--not-a-keld-option"),
        "the second role command must be refused before reading later options: {stderr}"
    );
}

#[test]
fn unknown_argument_is_cli_044_before_boot_selection() {
    let output = Command::new(env!("CARGO_BIN_EXE_keld-host"))
        .arg("--not-a-keld-host-flag")
        .output()
        .expect("run host with unknown argument");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("host error is UTF-8");
    assert!(stderr.contains("KELD-CLI-044"), "{stderr}");
    assert!(!stderr.contains("KELD-CORE-034"), "{stderr}");
    assert!(!stderr.contains("keld.boot.json"), "{stderr}");
}
