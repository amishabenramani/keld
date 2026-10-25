/** Boundary regressions: independent Bun process keeps transport mocks isolated. */
import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "bun:test";
import {
  APP_LINK_IO_DEADLINE_MS,
  withIoDeadline,
} from "../../kipc/src/transport.ts";

for (const scenario of [
  "recovery-gap",
  "replay-order",
  "replay-link-death",
  "adopted-without-replay-wrapper",
  "unmaterialized-close",
  "mutable-id",
]) {
  test(`GH531 P1 ${scenario} dispatch and ownership`, async () => {
    const child = Bun.spawn(
      [
        process.execPath,
        join(import.meta.dir, "browser-window-recovery.test-role.ts"),
        scenario,
      ],
      { stdin: "ignore", stdout: "pipe", stderr: "pipe" },
    );
    try {
      const [exit, stdout, stderr] = await withIoDeadline(
        Promise.all([
          child.exited,
          new Response(child.stdout).text(),
          new Response(child.stderr).text(),
        ]),
        APP_LINK_IO_DEADLINE_MS,
      );
      assert.equal(exit, 0, `${scenario}: ${stdout}\n${stderr}`);
      assert.equal(stdout.trim(), `PASS ${scenario}`);
      assert.equal(stderr, "");
    } finally {
      if (child.exitCode === null) child.kill();
      await child.exited;
    }
  });
}
