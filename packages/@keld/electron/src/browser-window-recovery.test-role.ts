/** Isolated Bun role: real WindowSource codec/mirror with a controlled Host-call seam. */
import assert from "node:assert/strict";
import { mock } from "bun:test";
import { linkError } from "../../kipc/src/transport.ts";
import {
  encodeWindowEvent,
  type WindowEvent,
  type WindowRef,
  type WindowRequest,
  type WindowResponse,
} from "../../api/src/window-codec.ts";
import { WindowSource } from "../../api/src/window-source.ts";

let ready = false;
let signalReady: () => void = () => {};
const readyPromise = new Promise<void>((resolve) => {
  signalReady = resolve;
});
let factListener: (fact: WindowEvent) => void = () => {
  throw new Error("window event listener was not installed");
};
const source = new WindowSource((fact) => factListener(fact));
const calls: WindowRequest[] = [];
let lastId = 0;
let adoptNext: WindowRef | undefined;
const state = {
  bounds: { x: 15, y: 30, width: 640, height: 480 },
  focused: false,
  maximized: false,
  full_screen: false,
};

function fact(event: WindowEvent, dispatch = true): WindowEvent {
  const decoded = source.decode(encodeWindowEvent(event));
  source.apply(decoded);
  if (dispatch) source.dispatch(decoded);
  return decoded;
}
function createHostWindow(pending_close: bigint | null = null): WindowRef {
  const window = { id: ++lastId, incarnation: BigInt(100 + lastId) };
  fact({ variant: "Created", window, state, pending_close });
  return window;
}
function becomeReady(): void {
  ready = true;
  signalReady();
}

// The mock is only the host transport; WindowSource's wire decode, apply and
// dispatch remain real. The subprocess prevents mocks leaking into other tests.
mock.module("../../api/src/app.ts", () => ({
  app: { whenReady: () => readyPromise, isReady: () => ready },
  getWindowSnapshot: (window: WindowRef) =>
    ready ? source.get(window) : undefined,
  getWindowSnapshots: () => (ready ? source.all() : []),
  onWindowFactApplied: (listener: (event: WindowEvent) => void) => {
    factListener = listener;
    return () => {};
  },
  invokeWindow: async (request: WindowRequest): Promise<WindowResponse> => {
    calls.push(request);
    return { variant: "ReplyApplied" };
  },
  invokeWindowBlocking: (request: WindowRequest): WindowResponse => {
    calls.push(request);
    if (request.variant === "Create") {
      if (adoptNext) {
        const window = adoptNext;
        adoptNext = undefined;
        return { variant: "Adopted", window };
      }
      return { variant: "Created", window: createHostWindow() };
    }
    if (request.variant === "Destroy") {
      // A blocking response sees the applied Host tombstone; notification
      // dispatch happens after the caller resumes, as on the real WorkerLink.
      fact({ variant: "Destroyed", window: request.window }, false);
      return { variant: "Destroyed" };
    }
    throw new Error(`unexpected blocking call ${request.variant}`);
  },
}));
mock.module("./app.ts", () => ({ emitNativeAppEvent: () => {} }));

const { BrowserWindow } = await import("./browser-window.ts");
const mode = process.argv[2];
if (mode === "recovery-gap") {
  // Host replays a gap-native close before the successor's Ready barrier.
  const ref = createHostWindow(7n);
  becomeReady();
  await Promise.resolve();
  const win = BrowserWindow.getAllWindows()[0];
  assert.equal(win.id, ref.id);
  let closeEvents = 0;
  win.on("close", (event: { preventDefault(): void }) => {
    closeEvents += 1;
    event.preventDefault();
  });
  // The successor gets its Ready turn to install existing application policy
  // before the held replay close is dispatched.
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(
    closeEvents,
    1,
    "Created.pending_close must reach the successor",
  );
  assert.deepEqual(
    calls.filter((call) => call.variant === "CloseReply"),
    [{ variant: "CloseReply", window: ref, close_seq: 7n, verdict: "Veto" }],
  );
  assert.equal(BrowserWindow.getAllWindows()[0], win);
  assert.equal(win.isDestroyed(), false);
} else if (mode === "replay-order") {
  const ref = createHostWindow(7n);
  becomeReady();
  const win = BrowserWindow.getAllWindows()[0];
  const observed: string[] = [];
  win.on("close", (event: { preventDefault(): void }) => {
    observed.push("close");
    event.preventDefault();
  });
  win.on("resize", () => observed.push("resize"));
  // The Host wrote Created(pending) before this later Bounds. Both source
  // facts are real decoded/applied bytes; the listener order is the oracle.
  fact({
    variant: "Bounds",
    window: ref,
    bounds: { x: 15, y: 30, width: 777, height: 480 },
  });
  assert.deepEqual(observed, ["close", "resize"]);
  assert.deepEqual(win.getSize(), [777, 480]);
  assert.deepEqual(
    calls.filter((call) => call.variant === "CloseReply"),
    [{ variant: "CloseReply", window: ref, close_seq: 7n, verdict: "Veto" }],
  );
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(observed, ["close", "resize"], "no duplicate replay close");
} else if (mode === "pre-ready-close-order") {
  const ref = createHostWindow();
  // A real CloseRequested can arrive after Subscribe but before the facade
  // emits Ready. It must be ordered ahead of the next Bounds fact, not only
  // when the first close was attached to a replayed Created frame.
  fact({ variant: "CloseRequested", window: ref, close_seq: 11n });
  becomeReady();
  const win = BrowserWindow.getAllWindows()[0];
  const observed: string[] = [];
  win.on("close", (event: { preventDefault(): void }) => {
    observed.push("close");
    event.preventDefault();
  });
  win.on("resize", () => observed.push("resize"));
  fact({
    variant: "Bounds",
    window: ref,
    bounds: { x: 15, y: 30, width: 888, height: 480 },
  });
  assert.deepEqual(observed, ["close", "resize"]);
  assert.deepEqual(
    calls.filter((call) => call.variant === "CloseReply"),
    [{ variant: "CloseReply", window: ref, close_seq: 11n, verdict: "Veto" }],
  );
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(observed, ["close", "resize"]);
} else if (mode === "move-only-does-not-resize") {
  becomeReady();
  const win = new BrowserWindow();
  const ref = source.all()[0]?.window;
  assert.ok(ref);
  const resized: string[] = [];
  win.on("resize", () => {
    resized.push(win.getSize().join("x"));
  });
  fact({
    variant: "Bounds",
    window: ref,
    bounds: { x: 25, y: 48, width: 640, height: 480 },
  });
  assert.deepEqual(resized, [], "a pure native move cannot emit resize");
  assert.deepEqual(win.getPosition(), [25, 48]);
  fact({
    variant: "Bounds",
    window: ref,
    bounds: { x: 25, y: 48, width: 777, height: 480 },
  });
  assert.deepEqual(
    resized,
    ["777x480"],
    "resize listener reads applied Host fact",
  );
  fact({
    variant: "Bounds",
    window: ref,
    bounds: { x: 25, y: 48, width: 777, height: 480 },
  });
  assert.deepEqual(resized, ["777x480"], "no duplicate resize for same bounds");
} else if (mode === "throwing-contents-destroyed") {
  becomeReady();
  const window = new BrowserWindow();
  const ref = source.all()[0]?.window;
  assert.ok(ref);
  const observed: string[] = [];
  const errors: unknown[][] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => {
    errors.push(args);
  };
  try {
    window.webContents.on("destroyed", () => {
      observed.push("webContents destroyed");
      throw new Error("untrusted user listener");
    });
    window.on("closed", () => {
      assert.equal(window.isDestroyed(), true);
      observed.push("window closed");
    });
    // The Host has physically released the real window: its event cannot be
    // lost because a JS callback throws from the contents tombstone.
    fact({ variant: "Destroyed", window: ref });
    assert.deepEqual(observed, ["webContents destroyed", "window closed"]);
    assert.equal(window.isDestroyed(), true);
    assert.equal(BrowserWindow.getAllWindows().length, 0);
    assert.equal(errors.length, 1, "one bounded callback diagnostic");

    const second = new BrowserWindow();
    const secondContents = second.webContents;
    secondContents.on("destroyed", () => {
      observed.push("second contents destroyed");
      throw new Error("deferred user listener");
    });
    second.on("closed", () => observed.push("second closed"));
    second.destroy();
    await Promise.resolve();
    assert.equal(secondContents.isDestroyed(), true);
    assert.deepEqual(observed.slice(-2), [
      "second closed",
      "second contents destroyed",
    ]);
    assert.equal(
      errors.length,
      2,
      "deferred Destroy reports one diagnostic too",
    );
  } finally {
    console.error = originalError;
  }
} else if (mode === "no-public-contents-dispose") {
  becomeReady();
  const win = new BrowserWindow();
  const contents = win.webContents;
  let destroyed = 0;
  contents.on("destroyed", () => {
    destroyed += 1;
  });
  // A hostile app can reflect on a returned WebContents and invoke its
  // prototype methods. A public dispose() must never forge the Host tombstone.
  const forged = Reflect.get(contents, "dispose") as unknown;
  if (typeof forged === "function") Reflect.apply(forged, contents, []);
  assert.equal(contents.isDestroyed(), false, "Host never sent Destroyed");
  assert.equal(win.isDestroyed(), false);
  assert.equal(destroyed, 0, "caller must not fabricate native completion");
  assert.equal(BrowserWindow.getAllWindows()[0], win);
} else if (mode === "throwing-close-and-logger") {
  becomeReady();
  const win = new BrowserWindow();
  const ref = source.all()[0]?.window;
  assert.ok(ref);
  let closeEvents = 0;
  win.on("close", () => {
    closeEvents += 1;
    throw new Error("untrusted application close listener");
  });
  const originalError = console.error;
  console.error = () => {
    throw new Error("untrusted logger");
  };
  try {
    fact({ variant: "CloseRequested", window: ref, close_seq: 21n });
  } finally {
    console.error = originalError;
  }
  assert.equal(closeEvents, 1);
  assert.deepEqual(
    calls.filter((call) => call.variant === "CloseReply"),
    [{ variant: "CloseReply", window: ref, close_seq: 21n, verdict: "Veto" }],
    "a logging failure must never suppress the one terminal close verdict",
  );
  assert.equal(win.isDestroyed(), false);
} else if (mode === "replay-link-death") {
  createHostWindow(7n);
  becomeReady();
  const errors: unknown[][] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]) => {
    errors.push(args);
  };
  try {
    source.end(linkError("KELD-IPC-022", "peer closed"));
    // A deferred callback must not throw out of setImmediate after the
    // authenticated link's mirror was invalidated.
    await Promise.resolve();
    await new Promise<void>((resolve) => setImmediate(resolve));
    assert.equal(
      calls.filter((call) => call.variant === "CloseReply").length,
      0,
    );
    assert.equal(errors.length, 1, "record one terminal diagnostic");
    assert.match(String(errors[0]?.[0]), /Host mirror failure/);
  } finally {
    console.error = originalError;
  }
} else if (mode === "adopted-without-replay-wrapper") {
  const ref = createHostWindow();
  adoptNext = ref;
  becomeReady();
  // The app constructs before calling getAllWindows(), so Host adopts a
  // retained physical window directly. This is not a brand-new safe document.
  const win = new BrowserWindow();
  assert.equal(win.id, ref.id);
  assert.equal(BrowserWindow.getAllWindows()[0], win);
  fact({ variant: "CloseRequested", window: ref, close_seq: 14n });
  assert.deepEqual(
    calls.filter((call) => call.variant === "CloseReply"),
    [{ variant: "CloseReply", window: ref, close_seq: 14n, verdict: "Veto" }],
    "host-adopted window must not automatically close before user veto setup",
  );
} else if (mode === "unmaterialized-close") {
  becomeReady();
  const ref = createHostWindow();
  // No caller has requested getAllWindows(), but Host already owns the window.
  const closeEvents: number[] = [];
  const emit = BrowserWindow.prototype.emit;
  BrowserWindow.prototype.emit = function (
    event: string | symbol,
    ...args: unknown[]
  ) {
    if (event === "close") closeEvents.push(this.id);
    return Reflect.apply(emit, this, [event, ...args]) as boolean;
  };
  const request: WindowEvent = {
    variant: "CloseRequested",
    window: ref,
    close_seq: 8n,
  };
  const applied = fact(request);
  source.dispatch(applied); // duplicate dispatch must not send a second verdict
  const win = BrowserWindow.getAllWindows()[0];
  assert.equal(win.id, ref.id);
  assert.deepEqual(
    closeEvents,
    [ref.id],
    "one close event for the pending sequence",
  );
  assert.deepEqual(
    calls.filter((call) => call.variant === "CloseReply"),
    [{ variant: "CloseReply", window: ref, close_seq: 8n, verdict: "Veto" }],
    "an unclaimed recovered wrapper must not silently Allow unsaved-close",
  );
  let laterCalls = 0;
  win.on("close", (event: { preventDefault(): void }) => {
    laterCalls += 1;
    event.preventDefault();
  });
  fact({ variant: "CloseRequested", window: ref, close_seq: 9n });
  assert.equal(laterCalls, 1, "reuse the wrapper and its installed listeners");
  assert.equal(calls.filter((call) => call.variant === "CloseReply").length, 2);
} else if (mode === "mutable-id") {
  becomeReady();
  const a = new BrowserWindow();
  const b = new BrowserWindow();
  const aId = a.id;
  const bId = b.id;
  assert.notEqual(aId, bId);
  assert.equal(Reflect.set(a, "id", bId), false, "id must not be writable");
  assert.throws(() => Object.defineProperty(a, "id", { value: bId }));
  assert.equal(a.id, aId);
  const descriptors = Object.getOwnPropertyDescriptor(a, "id");
  assert.equal(descriptors?.writable, false);
  assert.equal(descriptors?.configurable, false);
  a.destroy();
  assert.equal(BrowserWindow.getAllWindows()[0], b);
  let closeEvents = 0;
  b.on("close", (event: { preventDefault(): void }) => {
    closeEvents += 1;
    event.preventDefault();
  });
  fact({
    variant: "CloseRequested",
    window: { id: bId, incarnation: BigInt(100 + bId) },
    close_seq: 1n,
  });
  assert.equal(
    closeEvents,
    1,
    "A.destroy must not steal B's dispatch identity",
  );
  assert.equal(
    calls.filter((call) => call.variant === "CloseReply").at(-1)?.variant,
    "CloseReply",
  );
} else {
  throw new Error(`unknown recovery role ${mode}`);
}
console.log(`PASS ${mode}`);
