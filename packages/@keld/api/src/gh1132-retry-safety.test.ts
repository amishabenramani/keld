import { Database } from "bun:sqlite";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "bun:test";

import { ECHO_CHANNEL, FrameKind, decodeCallError } from "../../kipc/src/transport.ts";
import { resolveEchoCall, type EchoCallHandler } from "./echo-call.ts";

const PAYLOAD = Uint8Array.from([0, 0]);
const LOGICAL_ACTION = "one-logical-action";

function latch(): { wait: Promise<void>; release: () => void } {
  let release = () => {};
  const wait = new Promise<void>((resolve) => { release = resolve; });
  return { wait, release };
}

function withSqlite(
  run: (db: Database) => Promise<void>,
): () => Promise<void> {
  return async () => {
    const directory = mkdtempSync(join(tmpdir(), "keld-gh1132-"));
    const db = new Database(join(directory, "actions.sqlite"));
    try {
      await run(db);
    } finally {
      db.close();
      rmSync(directory, { recursive: true, force: true });
    }
  };
}

describe("GH-1132: a status snapshot is not a replay fence", () => {
  test(
    "counterexample: API-001 can return before a deferred effect, so status-only resend commits twice",
    withSqlite(async (db) => {
      db.exec("CREATE TABLE effects (id INTEGER PRIMARY KEY, logical_key TEXT NOT NULL)");
      const insert = db.query("INSERT INTO effects (logical_key) VALUES (?1)");
      const count = db.query("SELECT COUNT(*) AS n FROM effects WHERE logical_key = ?1");
      const committed = (): number =>
        (count.get(LOGICAL_ACTION) as { n: number }).n;

      const delayed = latch();
      const outstanding: Promise<unknown>[] = [];
      const first: EchoCallHandler = async () => {
        outstanding.push(delayed.wait.then(() => insert.run(LOGICAL_ACTION)));
        throw new Error("handler failed but did not cancel its child task");
      };

      try {
        const firstAnswer = await resolveEchoCall(ECHO_CHANNEL, PAYLOAD, first);
        expect(firstAnswer.kind).toBe(FrameKind.Err);
        expect(decodeCallError(firstAnswer.payload).code).toBe("KELD-API-001");
        expect(committed()).toBe(0); // A real durable status query sees "absent".

        // This is the *old, unsafe app policy* — never recommended by GH-1132.
        const statusOnlyAllowsResend = committed() === 0;
        expect(statusOnlyAllowsResend).toBe(true);
        const retry = await resolveEchoCall(ECHO_CHANNEL, PAYLOAD, async () => {
          insert.run(LOGICAL_ACTION);
          return Uint8Array.of(0x41);
        });
        expect(retry.kind).toBe(FrameKind.Reply);

        delayed.release(); // Earlier work commits *after* the status query.
        await Promise.all(outstanding);
        expect(committed()).toBe(2); // Why the old spec rule is unsafe.
      } finally {
        delayed.release();
        await Promise.allSettled(outstanding);
      }
    }),
  );

  test(
    "safe app control: both attempts share one atomic durable idempotency key",
    withSqlite(async (db) => {
      db.exec("CREATE TABLE effects (logical_key TEXT PRIMARY KEY, result INTEGER NOT NULL)");
      const commit = db.query(
        "INSERT INTO effects (logical_key, result) VALUES (?1, 65) ON CONFLICT(logical_key) DO NOTHING",
      );
      const count = db.query("SELECT COUNT(*) AS n FROM effects WHERE logical_key = ?1");
      const committed = (): number =>
        (count.get(LOGICAL_ACTION) as { n: number }).n;

      const delayed = latch();
      const outstanding: Promise<unknown>[] = [];
      const first: EchoCallHandler = async () => {
        outstanding.push(delayed.wait.then(() => commit.run(LOGICAL_ACTION)));
        throw new Error("first handler did not await its deferred effect");
      };

      try {
        const firstAnswer = await resolveEchoCall(ECHO_CHANNEL, PAYLOAD, first);
        expect(firstAnswer.kind).toBe(FrameKind.Err);
        expect(decodeCallError(firstAnswer.payload).code).toBe("KELD-API-001");
        expect(committed()).toBe(0);

        // The retry is safe only because it shares the same durable uniqueness
        // constraint with the original. This is APP logic, never a Keld retry.
        const retry = await resolveEchoCall(ECHO_CHANNEL, PAYLOAD, async () => {
          commit.run(LOGICAL_ACTION);
          return Uint8Array.of(0x41);
        });
        expect(retry.kind).toBe(FrameKind.Reply);
        expect(retry.payload).toEqual(Uint8Array.of(0x41));

        delayed.release();
        await Promise.all(outstanding);
        expect(committed()).toBe(1); // At most one durable effect for one key.
      } finally {
        delayed.release();
        await Promise.allSettled(outstanding);
      }
    }),
  );
});
