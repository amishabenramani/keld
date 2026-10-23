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
    git("config", "maintenance.auto", "false");
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

test("native formatter stays within the JS/TS family and honors native ignore files", () => {
  const root = mkdtempSync(join(tmpdir(), "keld-js-format-scope-"));
  try {
    const original = "export const value={answer:42};\n";
    const sources = ["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"].map(
      (ext) => `source.${ext}`,
    );
    const untouched = [
      "source.rs",
      "document.md",
      "page.html",
      "data.json",
      "data.yaml",
    ];
    for (const name of [...sources, ...untouched, "ignored.ts"]) {
      writeFileSync(join(root, name), original);
    }
    expect(
      Bun.spawnSync(["git", "init", "--quiet"], { cwd: root }).exitCode,
    ).toBe(0);
    expect(
      Bun.spawnSync(["git", "config", "maintenance.auto", "false"], {
        cwd: root,
      }).exitCode,
    ).toBe(0);
    expect(Bun.spawnSync(["git", "add", "."], { cwd: root }).exitCode).toBe(0);
    writeFileSync(join(root, ".gitignore"), "ignored.ts\n");
    expect(trackedSources(root).sort()).toEqual(
      [...sources, "ignored.ts"].sort(),
    );
    expect(formatTrackedSources(root, true)).toBe(1);
    for (const name of [...sources, ...untouched, "ignored.ts"]) {
      expect(readFileSync(join(root, name), "utf8")).toBe(original);
    }
    expect(formatTrackedSources(root, false)).toBe(0);
    for (const name of sources) {
      expect(readFileSync(join(root, name), "utf8")).not.toBe(original);
    }
    for (const name of [...untouched, "ignored.ts"]) {
      expect(readFileSync(join(root, name), "utf8")).toBe(original);
    }
    expect(formatTrackedSources(root, true)).toBe(0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("tracked filenames are literal paths rather than native formatter options", () => {
  const root = mkdtempSync(join(tmpdir(), "keld-js-format-options-"));
  try {
    const files = ["--ignore-pattern=target.ts", "target.ts"];
    const original = "export const value={answer:42};\n";
    for (const file of files) writeFileSync(join(root, file), original);
    for (const args of [
      ["init", "--quiet"],
      ["config", "maintenance.auto", "false"],
      ["add", "--", ...files],
    ]) {
      expect(Bun.spawnSync(["git", ...args], { cwd: root }).exitCode).toBe(0);
    }
    expect(trackedSources(root)).toEqual(files);
    expect(formatTrackedSources(root, true)).toBe(1);
    for (const file of files) {
      expect(readFileSync(join(root, file), "utf8")).toBe(original);
    }
    expect(formatTrackedSources(root, false)).toBe(0);
    for (const file of files) {
      expect(readFileSync(join(root, file), "utf8")).not.toBe(original);
    }
    expect(formatTrackedSources(root, true)).toBe(0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
