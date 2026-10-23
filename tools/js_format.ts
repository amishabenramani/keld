/** Repository JS/TS formatting; native Bun owns syntax and formatting decisions. */
import { resolve } from "node:path";

/** Tracked JS/TS sources only: scratch, dependencies and nested research stay out. */
export function trackedSources(root: string): string[] {
  const result = Bun.spawnSync(["git", "ls-files", "-z"], { cwd: root });
  if (result.exitCode !== 0) {
    throw new Error(`JS formatting: git inventory failed: ${result.stderr}`);
  }
  return result.stdout
    .toString()
    .split("\0")
    .filter((path) => /\.(?:[cm]?[jt]s|[jt]sx)$/.test(path));
}

/** Run the real native formatter; a check never requests writes. */
export function formatTrackedSources(root: string, check: boolean): number {
  const files = trackedSources(root);
  let batch: string[] = [];
  let size = 0;
  const flush = (): number => {
    if (batch.length === 0) return 0;
    const result = Bun.spawnSync(
      [
        process.execPath,
        "fmt",
        "--no-script",
        ...(check ? ["--check"] : []),
        ...batch,
      ],
      { cwd: root, stdout: "inherit", stderr: "inherit" },
    );
    batch = [];
    size = 0;
    return result.exitCode;
  };
  for (const file of files) {
    // Bound the escaped argv below Windows' command-line limit, including quotes.
    const cost = file.length * 2 + 3;
    if (size + cost > 16_000 && batch.length > 0) {
      const code = flush();
      if (code !== 0) return code;
    }
    batch.push(file);
    size += cost;
  }
  return flush();
}

if (import.meta.main) {
  const [mode, ...extra] = process.argv.slice(2);
  if (extra.length || (mode !== "--check" && mode !== "--write")) {
    console.error("usage: bun tools/js_format.ts <--check|--write>");
    process.exitCode = 2;
  } else {
    const root = resolve(import.meta.dir, "..");
    const generator = resolve(
      root,
      "packages/@keld/kipc/scripts/echo-codegen.ts",
    );
    const generate = (command: string): number =>
      Bun.spawnSync([process.execPath, generator, command], {
        cwd: root,
        stdout: "inherit",
        stderr: "inherit",
      }).exitCode;
    try {
      // Generated artifacts and byte stamps are always updated by their owner.
      let code = generate(mode === "--check" ? "check" : "generate");
      if (code === 0) code = formatTrackedSources(root, mode === "--check");
      if (code === 0 && mode === "--write") code = generate("check");
      process.exitCode = code;
    } catch (error) {
      console.error(error instanceof Error ? error.message : String(error));
      process.exitCode = 1;
    }
  }
}
