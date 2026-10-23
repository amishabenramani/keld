// GH-1095 hosted app body (promoted from the GH-1081 round-3 app). Imports
// come from the public API prelude the test prepends. Every hold is an
// ordinary fixture HTTP wait the test owns; nothing here is a host or runtime
// hook. The handler is registered at module top, before the first lifecycle
// call (spec gh1095 §4.4), and records every entry: that record is the
// handler census.
const base = "http://127.0.0.1:__PORT__";
let handlerCalls = 0;
async function post(value: unknown): Promise<void> {
  const response = await fetch(base + "/observe", { method: "POST",
    headers: { "Content-Type": "text/plain" }, body: JSON.stringify(value) });
  if (!response.ok) throw new Error("gh1095 app observation refused");
}
function holdId(): string {
  return Array.from(crypto.getRandomValues(new Uint32Array(4)),
    value => value.toString(16).padStart(8, "0")).join("");
}
channels.handle(echoChannel, async request => {
  handlerCalls += 1;
  const label: string = JSON.parse(request.message).label;
  const record = { phase: "handler", pid: process.pid, call: handlerCalls,
    message: request.message, count: request.count };
  console.error("KELD_GH1095_HANDLER " + JSON.stringify(record));
  await post(record);
  if (label.startsWith("held")) {
    // Held inside the handler until the test kills Bun or drops this wait;
    // a released hold then answers like any other call.
    await fetch(base + "/recovery-pending/" + process.pid + "/" + holdId()).then(
      () => undefined, () => undefined);
    await post({ phase: "handler-released", pid: process.pid, message: request.message });
  }
  return { message: JSON.stringify({ pid: process.pid, call: handlerCalls }), count: request.count };
});
const bootstrap = process.env.KELD_APP_LINK;
if (!bootstrap) throw new Error("gh1095 app has no host-minted link");
// Hold before the first lifecycle call (which opens the app link), so the
// test can act while the host has no admitted generation.
await post({ phase: "recovery-app-boot", pid: process.pid, appLink: bootstrap });
const admission = await fetch(base + "/recovery-admit/" + process.pid);
if (!admission.ok || await admission.text() !== "admit") {
  throw new Error("invalid gh1095 admission command");
}
await app.whenReady();
await post({ phase: "app-ready", pid: process.pid,
  endpoint: bootstrap.slice(0, bootstrap.lastIndexOf("#")),
  executable: process.execPath, bunVersion: Bun.version });
const quit = await fetch(base + "/recovery-quit/" + process.pid);
if (!quit.ok || await quit.text() !== "quit") throw new Error("invalid gh1095 quit command");
console.error("KELD_GH1095_QUIT_REQUEST calls=" + handlerCalls);
await app.quit();
process.exit(0);
