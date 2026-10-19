//! KEL-118 independent product descendant observations; no process authority.
use super::ProcessFamily;
use crate::support::PROCESS_DEADLINE;
use crate::support::control::accept_before;
use crate::support::lease_descriptors::pipe_identity;
use crate::support::process::{ProcessExitObserver, parent_process, process_group};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

pub(super) struct DescendantQuit {
    _owned: tempfile::TempDir,
    listener: UnixListener,
    control: PathBuf,
    helper: PathBuf,
    python: PathBuf,
    token: String,
    peer: Option<BufReader<UnixStream>>,
    writer: Option<UnixStream>,
    bun_exit: Option<ProcessExitObserver>,
    child_exit: Option<ProcessExitObserver>,
    child_pid: Option<u32>,
    require_post_parent_ack: bool,
}

// Native records can contain a private token, executable path, PID or nonce.
// A diagnostic is only a fixed phase receipt, never the record or authority.
#[derive(Clone, Copy)]
enum DiagnosticPhase {
    ChildReady,
    ChildAliveAfterBunExit,
}

fn safe_control_marker(_record: &Value, phase: DiagnosticPhase) -> String {
    match phase {
        DiagnosticPhase::ChildReady => "KEL118_PRODUCT_ARM child_ready",
        DiagnosticPhase::ChildAliveAfterBunExit => "KEL118_PRODUCT_AFTER_EXIT0 alive=true",
    }
    .to_owned()
}

impl DescendantQuit {
    pub(super) fn bind() -> Self {
        // This changes only the harness oracle for the production-removal run.
        // The product receives the same public Quit and descendant fixture.
        let require_post_parent_ack = match std::env::var("KEL118_REQUIRE_POST_PARENT_ACK") {
            Ok(value) => {
                assert!(
                    value == "1",
                    "negative-control oracle accepts only 1 or unset"
                );
                true
            }
            Err(std::env::VarError::NotPresent) => false,
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("negative-control oracle configuration was not valid Unicode")
            }
        };
        let owned = tempfile::Builder::new()
            .prefix("keld-118-")
            .tempdir_in("/tmp")
            .expect("short owned descendant control directory");
        let control = owned.path().join("control.sock");
        let listener = UnixListener::bind(&control).expect("owned descendant control");
        listener
            .set_nonblocking(true)
            .expect("bounded descendant accept");
        let helper = owned.path().join("held-child.py");
        fs::write(
            &helper,
            include_str!("../../../fixtures/public_quit_descendant.py"),
        )
        .expect("owned Python peer fixture");
        let output = Command::new("python3")
            .args([
                "-c",
                "import os,sys; print(os.path.realpath(sys.executable))",
            ])
            .output()
            .expect("select existing installed Python fixture interpreter");
        assert!(output.status.success(), "Python interpreter probe failed");
        let python = PathBuf::from(
            std::str::from_utf8(&output.stdout)
                .expect("Python path UTF-8")
                .trim(),
        );
        assert!(
            python.is_absolute() && python.is_file(),
            "Python fixture interpreter must be an existing absolute file"
        );
        let token = fresh_nonce();
        Self {
            _owned: owned,
            listener,
            control,
            helper,
            python,
            token,
            peer: None,
            writer: None,
            bun_exit: None,
            child_exit: None,
            child_pid: None,
            require_post_parent_ack,
        }
    }

    pub(super) fn configuration(&self) -> Value {
        json!({"python":self.python,"helper":self.helper,
            "control":self.control,"token":self.token})
    }

    pub(super) fn arm(&mut self, family: ProcessFamily) {
        let stream = accept_before(&self.listener, Instant::now() + PROCESS_DEADLINE);
        stream
            .set_read_timeout(Some(PROCESS_DEADLINE))
            .expect("control deadline");
        stream
            .set_write_timeout(Some(PROCESS_DEADLINE))
            .expect("control write deadline");
        self.writer = Some(stream.try_clone().expect("fixture control writer"));
        let mut reader = BufReader::new(stream);
        let ready = read_record(&mut reader).expect("actual descendant ready record");
        assert!(ready["phase"] == "child-ready", "child-ready phase missing");
        assert!(ready["token"] == self.token, "private child token mismatch");
        assert!(ready["pid"] == family.descendant, "child identity mismatch");
        assert!(ready["ppid"] == family.bun, "child parent mismatch");
        assert!(ready["pgid"] == family.bun, "child group mismatch");
        assert!(
            parent_process(family.descendant) == family.bun,
            "child's kernel parent does not match Bun"
        );
        assert!(
            process_group(family.descendant) == family.bun,
            "child's kernel process group does not match Bun"
        );
        assert!(
            fs::canonicalize(ready["python"].as_str().expect("actual Python image"))
                .expect("canonical Python peer image")
                == self.python,
            "child interpreter identity mismatch"
        );
        let bun_pipe = pipe_identity(family.bun, "1");
        let child_pipe = pipe_identity(family.descendant, "1");
        assert!(
            bun_pipe == child_pipe,
            "descendant must inherit exact Bun stdout endpoint"
        );
        self.bun_exit = Some(ProcessExitObserver::arm(family.bun));
        self.child_exit = Some(ProcessExitObserver::arm(family.descendant));
        self.child_pid = Some(family.descendant);
        self.peer = Some(reader);
        eprintln!(
            "{}",
            safe_control_marker(&ready, DiagnosticPhase::ChildReady)
        );
    }

    pub(super) fn observe_cooperative_exit_and_guardian_reap(&mut self) {
        let parent = self
            .bun_exit
            .take()
            .expect("prearmed actual Bun exit")
            .wait();
        assert_eq!(
            parent.code(),
            Some(0),
            "public Quit must end in actual Bun exit0: {parent:?}"
        );
        assert_eq!(
            parent.signal(),
            None,
            "cooperative exit cannot be guardian-induced SIGKILL"
        );
        // Mint only after independent kernel Bun exit0. A missing PONG may be
        // normal rapid group kill, but then exact child kernel SIGKILL9 is required.
        let nonce = fresh_nonce();
        let command = json!({"command":"ping","token":self.token,"nonce":nonce});
        let mut encoded = serde_json::to_vec(&command).expect("bounded private command");
        encoded.push(b'\n');
        let writer = self.writer.as_mut().expect("retained control writer");
        writer
            .set_write_timeout(Some(PROCESS_DEADLINE))
            .expect("post-parent write bound");
        let peer = self.peer.as_mut().expect("retained descendant peer");
        let acknowledgment_deadline = if self.require_post_parent_ack {
            PROCESS_DEADLINE
        } else {
            std::time::Duration::from_millis(250)
        };
        peer.get_ref()
            .set_read_timeout(Some(acknowledgment_deadline))
            .expect("bounded post-parent acknowledgment");
        let observed = writer.write_all(&encoded).and_then(|()| read_record(peer));
        let alive_after_parent = match observed {
            Ok(reply) => {
                assert!(reply["phase"] == "child-alive", "child-alive phase missing");
                assert!(
                    reply["pid"] == self.child_pid.expect("exact descendant"),
                    "child identity changed after Bun exit"
                );
                assert!(reply["nonce"] == nonce, "post-Bun child nonce mismatch");
                assert!(reply["stdout"] == "written" || reply["stdout"] == "broken-pipe");
                eprintln!(
                    "{}",
                    safe_control_marker(&reply, DiagnosticPhase::ChildAliveAfterBunExit)
                );
                true
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                ) =>
            {
                false
            }
            Err(error) => panic!("post-parent descendant observation failed: {error}"),
        };
        assert!(
            alive_after_parent || !self.require_post_parent_ack,
            "production group-kill-removal control requires a fresh surviving-child ACK after actual Bun exit0; no attribution proof before fallback",
        );
        eprintln!(
            "KEL118_PRODUCT_REAP_OBSERVATION actual_bun_exit=0 alive_after_parent={alive_after_parent} required_fresh_ack={} pre_fallback=true",
            self.require_post_parent_ack,
        );
        let child = self
            .child_exit
            .take()
            .expect("prearmed descendant exit")
            .wait();
        assert_eq!(
            child.signal(),
            Some(9),
            "guardian must terminate inherited writer; EOF/EPIPE/watchdog cannot qualify: {child:?}"
        );
        assert_eq!(child.code(), None);
        eprintln!(
            "KEL118_PRODUCT_FORK_QUIT actual_bun_exit=0 alive_after_parent={alive_after_parent} descendant_signal=9 product=true"
        );
    }
}

// Drop only closes observation resources. The existing shipping launch cleanup
// owner retains the independently admitted Bun group until all assertions pass.
// This observer never signals/releases the product child or reaps Bun.

fn fresh_nonce() -> String {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .expect("fixture nonce source")
        .read_exact(&mut bytes)
        .expect("complete fixture nonce");
    let mut rendered = String::with_capacity(32);
    for byte in bytes {
        write!(&mut rendered, "{byte:02x}").expect("bounded fixture nonce formatting");
    }
    rendered
}

fn read_record(reader: &mut BufReader<UnixStream>) -> std::io::Result<Value> {
    let mut bytes = Vec::new();
    let read = reader.take(4097).read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "fixture peer closed",
        ));
    }
    if bytes.len() > 4096 || !bytes.ends_with(b"\n") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bounded fixture record",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_descendant_control_values_never_reach_phase_diagnostics() {
        const PRIVATE: &str = "KELD_TEST_PRIVATE_SENTINEL_TOKEN_NONCE_ROOT_PATH";
        let record = json!({
            "phase": "child-alive", "token": PRIVATE, "nonce": PRIVATE,
            "python": PRIVATE, "pid": PRIVATE, "stdout": PRIVATE
        });
        for (phase, expected) in [
            (
                DiagnosticPhase::ChildReady,
                "KEL118_PRODUCT_ARM child_ready",
            ),
            (
                DiagnosticPhase::ChildAliveAfterBunExit,
                "KEL118_PRODUCT_AFTER_EXIT0 alive=true",
            ),
        ] {
            let diagnostic = safe_control_marker(&record, phase);
            assert!(!diagnostic.contains(PRIVATE));
            assert!(!diagnostic.contains("nonce"));
            assert!(!diagnostic.contains("token"));
            assert_eq!(diagnostic, expected);
        }
    }

    #[test]
    fn descendant_oracle_diagnostics_do_not_interpolate_raw_control_records() {
        let source = include_str!("descendant_quit.rs");
        for name in ["ready", "reply"] {
            for interpolation in [
                format!("{{{name}}}"),
                format!("{{{name}:?}}"),
                format!("eprintln!(\"{{:?}}\", {name})"),
                format!("eprintln!(\"{{}}\", {name})"),
                format!("assert_eq!({name}["),
            ] {
                assert!(
                    !source.contains(&interpolation),
                    "raw private descendant record formatting was reintroduced"
                );
            }
        }
    }
}
