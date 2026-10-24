/** Pure source/alias contracts, not link attachment, heap bytes or native proof. */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "bun:test";
import { APP_LINK_IO_DEADLINE_MS, WINDOW_CHANNEL, withIoDeadline } from "../../kipc/src/transport.ts";
import { encodeWindowEvent, type WindowEvent, type WindowRef } from "./window-codec.ts";
import { WindowSource } from "./window-source.ts";

const WINDOW: WindowRef = { id: 7, incarnation: 300n };
const STATE = { bounds: { x: -3, y: 4, width: 640, height: 480 }, focused: true, maximized: false, full_screen: true };
const literals = new Map<string, string>();
for (const row of readFileSync(join(import.meta.dir, "../../../../crates/keld-ipc/tests/fixtures/window-v0.tsv"), "utf8").trimEnd().split("\n")) {
  if (row.startsWith("#")) continue;
  const [name, family, hex] = row.split("\t");
  if (family === "event") literals.set(name, hex);
}
function literal(name: string): Uint8Array {
  const hex = literals.get(name);
  if (!hex) throw new Error(`Missing Rust-owned event vector ${name}`);
  return Uint8Array.from(Buffer.from(hex, "hex"));
}
function accept(source: WindowSource, bytes: Uint8Array): WindowEvent {
  const fact = source.decode(bytes);
  source.apply(fact);
  return fact;
}
function rejected(action: () => void): Error {
  let error: unknown;
  try { action(); } catch (caught) { error = caught; }
  expect(error).toBeInstanceOf(Error);
  expect(error).toHaveProperty("code", "KELD-IPC-005");
  return error as Error;
}

test("Created literal supplies the complete host snapshot on the generated channel", () => {
  const source = new WindowSource(() => { throw new Error("apply must not dispatch"); });
  expect(source.channel).toBe(WINDOW_CHANNEL);
  expect(source.all()).toEqual([]);
  accept(source, literal("event-created"));
  expect(source.get(WINDOW)).toEqual({ window: WINDOW, state: STATE, pending_close: null, host_destroyed: false });
  expect(source.all()).toEqual([source.get(WINDOW)!]);
  expect(source.get({ ...WINDOW, incarnation: 301n })).toBeUndefined();
  expect(source.get({ id: 8, incarnation: 300n })).toBeUndefined();
  expect(source.get(WINDOW)?.state).toEqual(STATE);
});

test("every state fact changes only its own host-confirmed field", () => {
  const source = new WindowSource(() => {});
  accept(source, literal("event-created"));
  const initial = source.get(WINDOW)!;
  // Literal postcard fields: Bounds, pair(7,300), zigzag(1,-2), size(33,44).
  accept(source, Uint8Array.of(1, 7, 0xac, 2, 2, 3, 33, 44));
  expect(source.get(WINDOW)?.state).toEqual({ ...STATE, bounds: { x: 1, y: -2, width: 33, height: 44 } });
  accept(source, Uint8Array.of(2, 7, 0xac, 2, 0));
  expect(source.get(WINDOW)?.state).toEqual({ bounds: { x: 1, y: -2, width: 33, height: 44 }, focused: false, maximized: false, full_screen: true });
  accept(source, Uint8Array.of(3, 7, 0xac, 2, 1));
  expect(source.get(WINDOW)?.state).toEqual({ bounds: { x: 1, y: -2, width: 33, height: 44 }, focused: false, maximized: true, full_screen: true });
  accept(source, Uint8Array.of(4, 7, 0xac, 2, 0));
  expect(source.get(WINDOW)?.state).toEqual({ bounds: { x: 1, y: -2, width: 33, height: 44 }, focused: false, maximized: true, full_screen: false });
  expect(initial).toEqual({ window: WINDOW, state: STATE, pending_close: null, host_destroyed: false });
  expect(source.get(WINDOW)).not.toBe(initial);
});

test("all eight Rust-owned event vectors are accepted in their specified order", () => {
  expect(literals.size).toBe(8);
  for (const created of ["event-created", "event-created-pending-close"]) {
    const source = new WindowSource(() => {});
    accept(source, literal(created));
    expect(source.get(WINDOW)?.pending_close).toBe(created.endsWith("pending-close") ? 300n : null);
    for (const name of ["event-bounds", "event-focus", "event-maximized", "event-full-screen", "event-close-requested"]) {
      accept(source, literal(name));
      expect(source.get(WINDOW)?.host_destroyed).toBe(false);
    }
    expect(source.get(WINDOW)?.pending_close).toBe(300n);
    expect(source.get(WINDOW)?.state).toEqual(STATE);
    accept(source, literal("event-destroyed"));
    expect(source.get(WINDOW)).toEqual({ window: WINDOW, state: STATE, pending_close: null, host_destroyed: true });
    expect(source.all()).toHaveLength(1);
  }
});

test("facts require Created first and reject unknown ids or stale identity halves", () => {
  for (const name of ["event-bounds", "event-focus", "event-maximized", "event-full-screen", "event-close-requested", "event-destroyed"]) {
    const source = new WindowSource(() => {});
    rejected(() => accept(source, literal(name)));
    expect(() => source.all()).toThrow("KELD-IPC-005");
  }
  for (const window of [{ id: 8, incarnation: 300n }, { id: 7, incarnation: 301n }]) {
    const source = new WindowSource(() => {});
    accept(source, literal("event-created"));
    const error = rejected(() => accept(source, encodeWindowEvent({ variant: "Focus", window, focused: false })));
    expect(() => source.get(WINDOW)).toThrow(error);
    expect(() => source.all()).toThrow(error);
  }
});

test("nonzero host pair is required without restricting valid dimensions or close sequences", () => {
  for (const window of [{ id: 0, incarnation: 300n }, { id: 7, incarnation: 0n }]) {
    const source = new WindowSource(() => {});
    rejected(() => accept(source, encodeWindowEvent({ variant: "Created", window, state: STATE, pending_close: null })));
  }
  for (const pending_close of [0n, 0xffff_ffff_ffff_ffffn]) {
    const source = new WindowSource(() => {});
    const window = { id: 0xffff_ffff, incarnation: 0xffff_ffff_ffff_ffffn };
    const state = { ...STATE, bounds: { x: -0x8000_0000, y: 0x7fff_ffff, width: 0, height: 0xffff_ffff } };
    accept(source, encodeWindowEvent({ variant: "Created", window, state, pending_close }));
    expect(source.get(window)).toEqual({ window, state, pending_close, host_destroyed: false });
    accept(source, encodeWindowEvent({ variant: "CloseRequested", window, close_seq: pending_close }));
    expect(source.get(window)?.pending_close).toBe(pending_close);
  }
});

test("duplicate Created and every fact after Destroyed invalidate rather than resurrect", () => {
  const duplicate = new WindowSource(() => {});
  accept(duplicate, literal("event-created"));
  rejected(() => accept(duplicate, literal("event-created")));
  for (const name of literals.keys()) {
    const source = new WindowSource(() => {});
    accept(source, literal("event-created"));
    accept(source, literal("event-destroyed"));
    rejected(() => accept(source, literal(name)));
    expect(() => source.get(WINDOW)).toThrow("KELD-IPC-005");
  }
});

test("all Created truncations, trailing bytes, unknown tags and invalid booleans invalidate reads", () => {
  const created = literal("event-created");
  const invalid = [Uint8Array.of(7), Uint8Array.of(2, 7, 0xac, 2, 2), new Uint8Array([...created, 0])];
  for (let end = 0; end < created.length; end += 1) invalid.push(created.slice(0, end));
  for (const bytes of invalid) {
    const source = new WindowSource(() => {});
    accept(source, created);
    const error = rejected(() => source.decode(bytes));
    expect(error.message).not.toContain("KELD-IPC-003");
    expect(() => source.get(WINDOW)).toThrow(error);
    expect(() => source.all()).toThrow(error);
    expect(() => source.decode(created)).toThrow(error);
  }
});

test("payload mutation after decode cannot affect the decoded or applied graph", () => {
  const source = new WindowSource(() => {});
  const bytes = literal("event-created-pending-close");
  const fact = source.decode(bytes);
  bytes.fill(0xff);
  expect(fact).toEqual({ variant: "Created", window: WINDOW, state: STATE, pending_close: 300n });
  source.apply(fact);
  expect(source.get(WINDOW)).toEqual({ window: WINDOW, state: STATE, pending_close: 300n, host_destroyed: false });
});

test("fact and snapshot graphs are deeply frozen and separately owned", () => {
  const source = new WindowSource(() => {});
  const fact = accept(source, literal("event-created"));
  if (fact.variant !== "Created") throw new Error("wrong literal event fixture");
  const value = source.get(WINDOW)!;
  for (const object of [fact, fact.window, fact.state, fact.state.bounds, value, value.window, value.state, value.state.bounds]) {
    expect(Object.isFrozen(object)).toBe(true);
    expect(Reflect.set(object, "injected", true)).toBe(false);
  }
  expect(value).not.toBe(fact);
  expect(value.window).not.toBe(fact.window);
  expect(value.state).not.toBe(fact.state);
  expect(value.state.bounds).not.toBe(fact.state.bounds);
  expect(Reflect.set(fact.state.bounds, "width", 1)).toBe(false);
  expect(Reflect.set(value.state, "maximized", true)).toBe(false);
  expect(Reflect.set(value.window, "incarnation", 301n)).toBe(false);
  const all = source.all();
  expect(Object.isFrozen(all)).toBe(true);
  expect(Reflect.set(all, "length", 0)).toBe(false);
  expect(source.get(WINDOW)?.state).toEqual(STATE);
  const boundsFact = accept(source, literal("event-bounds"));
  if (boundsFact.variant !== "Bounds") throw new Error("wrong bounds literal fixture");
  expect(Object.isFrozen(boundsFact.bounds)).toBe(true);
  expect(source.get(WINDOW)?.state.bounds).not.toBe(boundsFact.bounds);
});

test("dispatch gets the exact applied fact and listener exceptions do not invalidate state", () => {
  const observed: WindowEvent[] = [];
  const listenerFailure = new Error("application listener failure");
  const source = new WindowSource((fact) => { observed.push(fact); throw listenerFailure; });
  const fact = accept(source, literal("event-created"));
  expect(observed).toEqual([]);
  expect(() => source.dispatch(fact)).toThrow(listenerFailure);
  expect(observed[0]).toBe(fact);
  expect(source.get(WINDOW)?.state).toEqual(STATE);
  accept(source, literal("event-close-requested"));
  expect(source.get(WINDOW)?.pending_close).toBe(300n);
});

test("source failure denies reads immediately while earlier applied facts remain dispatchable", () => {
  const observed: WindowEvent[] = [];
  const source = new WindowSource((fact) => observed.push(fact));
  const earlier = accept(source, literal("event-created"));
  const error = rejected(() => source.decode(Uint8Array.of(7)));
  expect(() => source.get(WINDOW)).toThrow(error);
  source.dispatch(earlier);
  expect(observed).toHaveLength(1);
  expect(observed[0]).toBe(earlier);
  expect(() => source.apply(earlier)).toThrow(error);
  // This unit test proves invalidation and cause priority, not Map heap release.
  source.end(new Error("later transport completion"));
  expect(() => source.get(WINDOW)).toThrow(error);
});

test("terminal end is permanent and preserves its first existing transport error", () => {
  const source = new WindowSource(() => {});
  const fact = accept(source, literal("event-created"));
  const error = new Error("KELD-IPC-023: peer ended after the retained-record drain");
  source.end(error);
  source.end(new Error("later terminal error"));
  expect(() => source.get(WINDOW)).toThrow(error);
  expect(() => source.all()).toThrow(error);
  expect(() => source.decode(literal("event-created"))).toThrow(error);
  expect(() => source.apply(fact)).toThrow(error);
});

test("multiple host windows keep independent focus facts and stable host-id ordering", () => {
  const source = new WindowSource(() => {});
  const other = { id: 3, incarnation: 900n };
  accept(source, literal("event-created"));
  accept(source, encodeWindowEvent({ variant: "Created", window: other, state: STATE, pending_close: null }));
  expect(source.all().map((value) => value.window.id)).toEqual([3, 7]);
  accept(source, Uint8Array.of(2, 7, 0xac, 2, 0));
  expect(source.get(WINDOW)?.state.focused).toBe(false);
  expect(source.get(other)?.state.focused).toBe(true);
  accept(source, literal("event-destroyed"));
  expect(source.get(other)).toEqual({ window: other, state: STATE, pending_close: null, host_destroyed: false });
});

test("standalone source releases replaced and ended snapshot graphs using the same Bun executable", async () => {
  const runtime = { version: Bun.version, revision: Bun.revision, platform: process.platform, arch: process.arch };
  const child = Bun.spawn([process.execPath, join(import.meta.dir, "window-source-release.test-role.ts"), Buffer.from(literal("event-created")).toString("hex"), runtime.version, runtime.revision, runtime.platform, runtime.arch], {
    stdin: "ignore", stdout: "pipe", stderr: "pipe",
  });
  try {
    const [exit, stdout, stderr] = await withIoDeadline(Promise.all([
      child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]), APP_LINK_IO_DEADLINE_MS);
    expect({ exit, stderr }).toEqual({ exit: 0, stderr: "" });
    expect(JSON.parse(stdout)).toEqual({
      runtime,
      replacement: { trackedGraphs: 33, oldObjectsLive: 0, currentObjectsLive: 4, sourceAlive: true, externalSnapshotAlive: true, census: 2 },
      end: { oldObjectsLive: 0, currentObjectsLive: 0, sourceAlive: true, externalSnapshotAlive: true, readsDenied: true },
    });
  } finally {
    if (child.exitCode === null) child.kill();
    await child.exited;
  }
}, APP_LINK_IO_DEADLINE_MS * 2);
