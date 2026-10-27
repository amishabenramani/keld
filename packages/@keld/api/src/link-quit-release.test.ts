import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "bun:test";
import {
  APP_LINK_IO_DEADLINE_MS,
  withIoDeadline,
} from "../../kipc/src/transport.ts";

for (const scenario of ["success", "refusal"]) {
  test(`local Quit releases retained window source after ${scenario}`, async () => {
    const child = Bun.spawn(
      [
        process.execPath,
        join(import.meta.dir, "link-quit-release.test-role.ts"),
        scenario,
      ],
      { stdin: "ignore", stdout: "pipe", stderr: "pipe" },
    );
    try {
      const [code, stdout, stderr] = await withIoDeadline(
        Promise.all([
          child.exited,
          new Response(child.stdout).text(),
          new Response(child.stderr).text(),
        ]),
        APP_LINK_IO_DEADLINE_MS,
      );
      assert.equal(code, 0, `${stdout}\n${stderr}`);
      assert.equal(stdout.trim(), `PASS ${scenario}`);
      assert.equal(stderr, "");
    } finally {
      if (child.exitCode === null) child.kill();
      await child.exited;
    }
  });
}
