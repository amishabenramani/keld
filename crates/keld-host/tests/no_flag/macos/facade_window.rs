//! Actual no-flag schema-two Host, authenticated window transport and `AppKit`
//! window creation. This is a native protocol acceptance, NOT yet a public
//! Electron `BrowserWindow` constructor or draw.io compatibility claim.

use crate::support::EVENT_DEADLINE;
use crate::support::control::{accept_before, parse_pid, read_control_line, wait_child_output};
use crate::support::dev_cycle::ShippingLaunchCleanup;
use crate::support::native_window::native_window_rows;
use crate::support::process::{await_process_gone, process_group};
use crate::support::product::ProductFixture;
use std::fs;
use std::io::{BufReader, Write as _};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

const ROLE: &str = r#"
import { createConnection } from "node:net";
import { createInterface } from "node:readline";

const path = process.env.KELD_T1B_CONTROL;
const endpoint = process.env.KELD_APP_LINK;
if (!path || !endpoint) throw new Error("missing Host-owned endpoint or test control");
const link = await WorkerLink.open({
  link: endpoint,
  receive: { eventChannels: [LIFECYCLE_CHANNEL, WINDOW_CHANNEL], callReceivers: [] },
});
let readyResolve: () => void = () => {};
const ready = new Promise<void>((resolve) => { readyResolve = resolve; });
let created: WindowEvent | undefined;
let createdCount = 0;
link.setStateApplier(LIFECYCLE_CHANNEL, (payload) => {
  if (payload.length !== 1 || payload[0] !== 0) throw new Error("invalid Ready payload");
  readyResolve();
});
link.setStateApplier(WINDOW_CHANNEL, (payload) => {
  const fact = decodeWindowEvent(payload);
  if (fact.variant === "Created") { created = fact; createdCount += 1; }
});

const control = createConnection({ path });
await new Promise<void>((resolve, reject) => {
  control.once("connect", resolve);
  control.once("error", reject);
});
function report(message: string): void { control.write(`${message}\n`); }
report(`BOUND ${process.pid}`);
let window: WindowRef | undefined;
for await (const command of createInterface({ input: control })) {
  if (command === "PRE_SUB") {
    let code = "returned";
    try {
      await link.call(WINDOW_CHANNEL,
        encodeWindowRequest({ variant: "Create", width: 640, height: 480, x: null, y: null }), 5000);
    } catch (error) {
      code = isCallError(error) ? error.code : "untyped";
    }
    report(`PRE_SUB ${code}`);
  } else if (command === "SUBSCRIBE") {
    const response = await link.call(LIFECYCLE_CHANNEL, Uint8Array.of(1, WINDOW_CHANNEL), 5000);
    if (response.length !== 1 || response[0] !== 1) throw new Error("Subscribe not accepted");
    report(`SUBSCRIBED ${createdCount}`);
  } else if (command === "READY") {
    await ready;
    report("READY");
  } else if (command === "CREATE") {
    const response = decodeWindowResponse(await link.call(WINDOW_CHANNEL,
      encodeWindowRequest({ variant: "Create", width: 640, height: 480, x: null, y: null }), 5000));
    if (response.variant !== "Created") throw new Error(`unexpected ${response.variant}`);
    window = response.window;
    if (created?.variant !== "Created" || created.window.id !== window.id ||
        created.window.incarnation !== window.incarnation || createdCount !== 1) {
      throw new Error("host Created reply did not follow its matching native fact");
    }
    report(`CREATED ${window.id}:${window.incarnation} ${created.state.bounds.width}x${created.state.bounds.height}`);
  } else if (command === "STALE") {
    if (!window) throw new Error("missing host-created window");
    let code = "returned";
    try {
      await link.call(WINDOW_CHANNEL, encodeWindowRequest({
        variant: "SetMaximized", window: { id: window.id, incarnation: window.incarnation + 1n },
        maximized: true,
      }), 5000);
    } catch (error) {
      code = isCallError(error) ? error.code : "untyped";
    }
    report(`STALE ${code}`);
  } else if (command === "QUIT") {
    await quitAndCloseLink(link, APP_LINK_IO_DEADLINE_MS);
    report("QUIT_REPLY");
    break;
  } else {
    throw new Error(`unknown control request ${command}`);
  }
}
control.end();
"#;

fn census(pid: u32) -> Vec<serde_json::Value> {
    let rows = native_window_rows(pid);
    let value: serde_json::Value = serde_json::from_str(&rows).expect("CoreGraphics JSON census");
    assert_eq!(
        value["capture_preflight"], true,
        "Screen Recording unavailable: {rows}"
    );
    assert_eq!(value["pid"], pid, "wrong CoreGraphics process: {rows}");
    value["rows"]
        .as_array()
        .expect("window rows")
        .iter()
        .filter(|window| window["layer"] == 0)
        .cloned()
        .collect()
}

fn visible_app_windows(pid: u32) -> Vec<serde_json::Value> {
    census(pid)
        .into_iter()
        .filter(|window| window["on_screen"] == true)
        .collect()
}

fn stage(fixture: &ProductFixture) -> keld_cli::boot::DevBootStage {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let transport = serde_json::to_string(
        &repo
            .join("packages/@keld/kipc/src/transport.ts")
            .to_string_lossy(),
    )
    .expect("absolute transport import");
    let codec = serde_json::to_string(
        &repo
            .join("packages/@keld/api/src/window-codec.ts")
            .to_string_lossy(),
    )
    .expect("absolute window codec import");
    let source = format!(
        "import {{ WorkerLink, LIFECYCLE_CHANNEL, WINDOW_CHANNEL, isCallError, quitAndCloseLink, APP_LINK_IO_DEADLINE_MS }} from {transport};\n\
         import {{ encodeWindowRequest, decodeWindowResponse, decodeWindowEvent }} from {codec};\n\
         import type {{ WindowRef, WindowEvent }} from {codec};\n{ROLE}"
    );
    crate::renderer_bridge::bundle_public_api_entry(
        &fixture.project,
        repo,
        "gh449-native-window",
        &source,
    );
    fs::write(
        fixture.project.join("keld.config.ts"),
        "export default { name: \"KELD native Window acceptance\", entry: \"src/main.ts\", initialWindow: \"none\" } as const;\n",
    )
    .expect("strict native schema-two config");
    keld_cli::boot::stage_dev_boot(&fixture.project, Path::new(env!("CARGO_BIN_EXE_keld-host")))
        .expect("compile real Host stage")
}

#[test]
fn native_schema_two_subscribe_creates_one_real_app_window_without_authority_leak() {
    let fixture = ProductFixture::new("facade-native-create");
    let staged = stage(&fixture);
    let listener_path = fixture.root.path().join("native-create.sock");
    let listener = UnixListener::bind(&listener_path).expect("private control listener");
    listener.set_nonblocking(true).expect("bounded accept");
    let child = Command::new(staged.host())
        .process_group(0)
        .current_dir(staged.root())
        .env("KELD_T1B_CONTROL", &listener_path)
        .env("KELD_DEV_LEASE", "stdin-v1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch actual macOS shipping Host");
    let mut cleanup = ShippingLaunchCleanup::new(child);
    let host = cleanup.cli.as_mut().expect("live Host");
    let host_pid = host.id();
    let _lease = host.stdin.take().expect("keep Host dev lease");
    let stream = accept_before(&listener, Instant::now() + EVENT_DEADLINE);
    stream
        .set_read_timeout(Some(EVENT_DEADLINE))
        .expect("bounded control reader");
    let mut reader = BufReader::new(stream.try_clone().expect("control clone"));
    let mut writer = stream;
    let bound = read_control_line(&mut reader);
    let parts: Vec<_> = bound.split_whitespace().collect();
    assert_eq!(parts.first().copied(), Some("BOUND"), "{bound}");
    let bun_pid = parse_pid(parts.get(1).copied(), &bound);
    cleanup.record_authenticated_groups(process_group(host_pid), process_group(bun_pid));

    writer.write_all(b"PRE_SUB\n").expect("unsubscribed create");
    assert_eq!(read_control_line(&mut reader), "PRE_SUB KELD-CORE-045");
    assert!(
        census(host_pid).is_empty(),
        "refused Create allocated a window"
    );
    writer
        .write_all(b"SUBSCRIBE\n")
        .expect("subscribe to window events");
    assert_eq!(read_control_line(&mut reader), "SUBSCRIBED 0");
    writer.write_all(b"READY\n").expect("raw Init Ready");
    assert_eq!(read_control_line(&mut reader), "READY");
    assert!(
        census(host_pid).is_empty(),
        "Init Ready allocated a native anchor"
    );

    writer.write_all(b"CREATE\n").expect("authenticated Create");
    assert_eq!(read_control_line(&mut reader), "CREATED 1:1 640x480");
    let created_rows = visible_app_windows(host_pid);
    assert_eq!(
        created_rows.len(),
        1,
        "Create must allocate exactly one visible app window: {created_rows:?}"
    );
    assert_eq!(created_rows[0]["title"], "Keld", "actual native app title");
    writer
        .write_all(b"STALE\n")
        .expect("forged window generation");
    assert_eq!(read_control_line(&mut reader), "STALE KELD-CORE-038");
    let stale_rows = visible_app_windows(host_pid);
    assert_eq!(
        stale_rows, created_rows,
        "stale caller mutated native census: before {created_rows:?}, after {stale_rows:?}"
    );

    writer
        .write_all(b"QUIT\n")
        .expect("orderly authenticated Quit");
    assert_eq!(read_control_line(&mut reader), "QUIT_REPLY");
    drop(writer);
    let output = wait_child_output(cleanup.release(), EVENT_DEADLINE);
    assert!(
        output.status.success(),
        "native Host shutdown failed: {output:?}"
    );
    await_process_gone(bun_pid);
}
