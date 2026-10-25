/** A Host LastWindowClosed can predate a new, already-mirrored replacement. */
import assert from "node:assert/strict";
import { mock } from "bun:test";

process.env.KELD_NATIVE_WINDOW_FACADE = "1";
let notifyLastWindow: () => void = () => {
  throw new Error("the facade omitted the one last-window-closed listener");
};
let quitCalls = 0;
let live = true;
mock.module("../../api/src/app.ts", () => ({
  app: {
    isReady: () => true,
    whenReady: async () => {},
    quit: async () => {
      quitCalls += 1;
    },
    onLastWindowClosed: (listener: () => void) => {
      notifyLastWindow = listener;
      return () => {};
    },
  },
  hasObservedHostReady: () => true,
  getWindowSnapshots: () => (live ? [{ host_destroyed: false }] : []),
}));
const { app } = await import("../src/app.ts");
notifyLastWindow();
await Promise.resolve();
assert.equal(
  quitCalls,
  0,
  "queued LastWindowClosed must not kill a new live document",
);
process.env.KELD_NATIVE_WINDOW_FACADE = "0";
await assert.rejects(app.quit(), /KELD-COMPAT-012/);
assert.equal(
  quitCalls,
  0,
  "an app may not disable the boot-scoped Quit safety gate",
);
live = false;
notifyLastWindow();
await Promise.resolve();
assert.equal(
  quitCalls,
  1,
  "real zero-window LastWindowClosed keeps the ordinary quit",
);
console.log("KELD_QUIT_STALE_LAST_WINDOW_SAFE");
