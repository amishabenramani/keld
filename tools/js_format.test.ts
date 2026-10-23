import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { formatTrackedSources, trackedSources } from "./js_format.ts";

test("native tracked formatter checks without writing, repairs, and is idempotent", () => {
  const root = mkdtempSync(join(tmpdir(), "keld-js-format-"));
  try {
    const git = (...args: string[]) => {
      expect(Bun.spawnSync(["git", ...args], { cwd: root }).exitCode).toBe(0);
    };
    git("init", "--quiet");
    const path = join(root, "source with spaces.ts");
    const original = "export const value={answer:42};\n";
    writeFileSync(path, original);
    writeFileSync(join(root, "untracked.ts"), original);
    writeFileSync(join(root, "document.md"), "untouched document");
    git("add", "source with spaces.ts", "document.md");
    expect(trackedSources(root)).toEqual(["source with spaces.ts"]);
    expect(formatTrackedSources(root, true)).toBe(1);
    expect(readFileSync(path, "utf8")).toBe(original);
    expect(formatTrackedSources(root, false)).toBe(0);
    const formatted = readFileSync(path, "utf8");
    expect(formatted).not.toBe(original);
    expect(formatTrackedSources(root, true)).toBe(0);
    expect(formatTrackedSources(root, false)).toBe(0);
    expect(readFileSync(path, "utf8")).toBe(formatted);
    expect(readFileSync(join(root, "untracked.ts"), "utf8")).toBe(original);
    expect(readFileSync(join(root, "document.md"), "utf8")).toBe(
      "untouched document",
    );
    git("add", "source with spaces.ts");
    git("rm", "--cached", "source with spaces.ts");
    expect(formatTrackedSources(root, false)).toBe(0);
    expect(readFileSync(join(root, "untracked.ts"), "utf8")).toBe(original);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
