/** Real BootstrapListener/WorkerLink regressions; host half: worker_link.rs. */
import assert from "node:assert/strict";
import { LifecycleLink, type LifecycleHandler } from "./link.ts";
import { isCallError, withIoDeadline } from "../../kipc/src/transport.ts";
import type { WindowEvent } from "./window-codec.ts";

const mode = process.argv[2];
const ref = { id: 7, incarnation: 300n };
const second = { id: 8, incarnation: 301n };
process.env.KELD_NATIVE_WINDOW_FACADE = "1";
const nextJob = (): Promise<void> =>
  new Promise((resolve) => setImmediate(resolve));
const report = (key: string, value: string | number | boolean): void =>
  console.log(`KELD_WL ${key}=${value}`);

async function lifecycleDrain(): Promise<void> {
  const log: string[] = [];
  let session: LifecycleLink | undefined;
  let throws = 0;
  process.on("uncaughtException", (error) => {
    assert.equal(error.message, "terminal callback negative path");
    throws += 1;
  });
  // Ordinary lifecycle callbacks share the handlers object with onWindowFact.
  // Its surviving root must not be mistaken for a retained Source snapshot.
  const handlers: {
    weak: WeakRef<object>;
    value: LifecycleHandler | undefined;
  } = (() => {
    const witness = { label: "ordinary lifecycle callback root" };
    return {
      weak: new WeakRef(witness),
      value: {
        onReady() {
          assert.ok(witness.label);
        },
        onLastWindowClosed() {
          assert.ok(witness.label);
        },
        onConnected(value: LifecycleLink) {
          session = value;
        },
        onLinkDead() {
          throw new Error("explicit close must not notify unexpected death");
        },
        onWindowFact(fact: WindowEvent) {
          log.push(`${fact.variant}:${fact.window.id}`);
          if (fact.variant !== "Destroyed") return;
          const source = session!.windowSource!;
          assert.equal(source.get(fact.window)?.host_destroyed, true);
          if (mode === "source-nested-destroy" && fact.window.id === ref.id) {
            assert.deepEqual(
              session!.callWindowBlocking({
                variant: "Destroy",
                window: second,
              }),
              { variant: "Destroyed" },
            );
            assert.equal(source.get(ref)?.host_destroyed, true);
            assert.equal(source.get(second)?.host_destroyed, true);
          }
          if (
            mode === "source-nested-destroy" &&
            fact.window.id === second.id
          ) {
            assert.equal(source.get(ref), undefined);
            session!.close();
            throw new Error("terminal callback negative path");
          }
        },
      },
    };
  })();
  const connected = await LifecycleLink.connect(
    process.env.KELD_APP_LINK!,
    handlers.value!,
  );
  // Leave the real link's registered ordinary callbacks as the strong owner.
  handlers.value = undefined;
  const source = connected.windowSource!;
  const before = log.length;
  assert.deepEqual(
    connected.callWindowBlocking({ variant: "Destroy", window: ref }),
    { variant: "Destroyed" },
  );
  assert.equal(
    log.length,
    before,
    "blocking return cannot dispatch the terminal fact",
  );
  assert.equal(source.get(ref)?.host_destroyed, true);
  log.push("return");
  if (mode === "source-close-drain") {
    connected.close();
    assert.equal(
      source.get(ref)?.host_destroyed,
      true,
      "close must retain applied terminal evidence until dispatch",
    );
    assert.throws(
      () =>
        connected.callWindowBlocking({ variant: "Destroy", window: second }),
      /KELD-CORE-045/,
    );
    await assert.rejects(
      connected.callWindow({ variant: "Destroy", window: second }),
      /KELD-CORE-045/,
    );
  }
  await withIoDeadline(
    (async () => {
      for (;;) {
        await nextJob();
        try {
          source.all();
        } catch (error) {
          assert.ok(isCallError(error));
          assert.equal(error.code, "KELD-IPC-022");
          break;
        }
      }
    })(),
    10_000,
  );
  log.push("end");
  await nextJob();
  Bun.gc(true);
  report("ordinary-callback-root", handlers.weak.deref() !== undefined);
  report("order", log.join(","));
  report("callback-throws", throws);
  report("done", true);
}

async function earlierAnswer(): Promise<void> {
  const {
    app,
    invokeWindow,
    invokeWindowBlocking,
    getWindowSnapshots,
    onWindowFactApplied,
  } = await import("./app.ts");
  const facts: string[] = [];
  onWindowFactApplied((fact) => facts.push(fact.variant));
  await app.whenReady();
  const answer = invokeWindow({
    variant: "CloseReply",
    window: ref,
    close_seq: 300n,
    verdict: "Veto",
  });
  // The host writes ReplyApplied, a valid earlier fact, then a bad pair and
  // this parked call's reply. The blocking fence applies the bad fact before
  // the earlier async answer's continuation can run, without timing hooks.
  assert.throws(
    () =>
      invokeWindowBlocking({
        variant: "SetMaximized",
        window: ref,
        maximized: true,
      }),
    /KELD-IPC-005/,
  );
  assert.deepEqual(await answer, { variant: "ReplyApplied" });
  assert.throws(getWindowSnapshots, /KELD-IPC-005/);
  await nextJob();
  await nextJob();
  assert.throws(getWindowSnapshots, /KELD-IPC-005/);
  report("answer", "ReplyApplied");
  report("facts", facts.join(","));
  report("sticky-source", "KELD-IPC-005");
  report("done", true);
}

try {
  if (mode === "source-earlier-answer") await earlierAnswer();
  else if (mode === "source-close-drain" || mode === "source-nested-destroy")
    await lifecycleDrain();
  else throw new Error(`unknown Source scenario: ${mode}`);
  process.exit(0);
} catch (error) {
  console.error(error);
  process.exit(1);
}
