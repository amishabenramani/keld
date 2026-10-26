import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const workflow = Bun.YAML.parse(
  readFileSync(".github/workflows/ci-provenance.yml", "utf8"),
) as Record<string, any>;
const job = workflow.jobs.provenance;
const reviewedAction =
  "actions/github-script@3a2844b7e9c422d3c10d287c895573f7108da1b3";
const code = job.steps[0].with.script as string;
const AsyncFunction = Object.getPrototypeOf(async function () {})
  .constructor as new (
  ...args: string[]
) => (...args: unknown[]) => Promise<void>;
const evaluate = new AsyncFunction("github", "context", "core", code);

type ChangedFile = {
  filename: string;
  status: string;
  previous_filename?: string;
};

function fixture(
  files: ChangedFile[],
  overrides: {
    head?: string;
    base?: string;
    apiHead?: string;
    apiBase?: string;
    baseRef?: string;
    repo?: string;
    state?: string;
    changedFiles?: number;
    changedOnSecondFetch?: boolean;
    apiFailure?: boolean;
    badEventNumber?: boolean;
  } = {},
) {
  const expectedHead = overrides.head ?? "head-commit-1";
  const expectedBase = overrides.base ?? "base-commit-1";
  const current = {
    number: 123,
    state: overrides.state ?? "open",
    changed_files: overrides.changedFiles ?? files.length,
    base: {
      ref: overrides.baseRef ?? "main",
      sha: overrides.apiBase ?? expectedBase,
      repo: { full_name: "gyldlab/keld" },
    },
    head: { sha: overrides.apiHead ?? expectedHead },
  };
  let reads = 0;
  let paginationCalls = 0;
  const github = {
    rest: {
      pulls: {
        get: async () => {
          if (overrides.apiFailure) throw Error("GitHub API unavailable");
          reads += 1;
          if (reads === 2 && overrides.changedOnSecondFetch) {
            return { data: { ...current, head: { sha: "newer-head" } } };
          }
          return { data: current };
        },
        listFiles: () => {
          throw Error("GitHub paginate must be used");
        },
      },
    },
    paginate: async (_unused: unknown, args: Record<string, unknown>) => {
      paginationCalls += 1;
      if (overrides.apiFailure) throw Error("GitHub API unavailable");
      expect(args).toEqual({
        owner: "gyldlab",
        repo: "keld",
        pull_number: 123,
        per_page: 100,
      });
      return files;
    },
  };
  const context = {
    repo: { owner: "gyldlab", repo: overrides.repo ?? "keld" },
    payload: {
      pull_request: {
        number: overrides.badEventNumber ? -123 : 123,
        head: { sha: expectedHead },
        base: { sha: expectedBase },
      },
    },
  };
  const messages: string[] = [];
  const core = { info: (message: string) => messages.push(message) };
  return {
    run: () => evaluate(github, context, core),
    getReads: () => reads,
    getPages: () => paginationCalls,
    messages,
  };
}

test("base-owned workflow is immutable/read-only and never executes PR code", () => {
  expect(workflow.name).toBe("CI provenance");
  expect(workflow.on.pull_request_target.types).toEqual([
    "opened",
    "reopened",
    "synchronize",
    "edited",
    "ready_for_review",
  ]);
  expect(workflow.permissions).toEqual({
    contents: "read",
    "pull-requests": "read",
  });
  expect(Object.keys(workflow.jobs)).toEqual(["provenance"]);
  expect(job.name).toBe("CI provenance");
  expect(job["runs-on"]).toBe("ubuntu-latest");
  expect(job["timeout-minutes"]).toBe(5);
  expect(job.steps).toHaveLength(1);
  expect(job.steps[0].uses).toBe(reviewedAction);
  expect(Object.keys(job.steps[0])).toEqual(["name", "uses", "with"]);
  expect(Object.keys(job.steps[0].with)).toEqual(["script"]);
  expect(job.steps[0].run).toBeUndefined();
  expect(job.if).toBeUndefined();
});

test("benign documentation and non-authority lockfile changes are admitted", async () => {
  const f = fixture([
    { filename: "docs/notes.md", status: "modified" },
    { filename: "Cargo.lock", status: "modified" },
  ]);
  await expect(f.run()).resolves.toBeUndefined();
  expect(f.getReads()).toBe(2);
  expect(f.getPages()).toBe(1);
  expect(f.messages[0]).toContain("2 PR files inspected");
});

test.each([
  [".github/workflows/evil.yml", "added"],
  [".github/workflows/ci.yml", "modified"],
  [".github/workflows/keldbot.yml", "removed"],
  [".github/workflows/CI.YML", "modified"],
  [".github/CODEOWNERS", "modified"],
  ["tools/ci-inputs.json", "modified"],
  ["tools/dependency_review_metadata.sh", "modified"],
  ["tools/repo_path_contract.rs", "modified"],
  ["tools/windows_signed_installed_acceptance.ps1", "modified"],
  [".config/nextest.toml", "modified"],
  [".cargo/config.toml", "added"],
  [".github/actions/untrusted/action.yml", "added"],
  ["crates/keld-updater-helper/deny.toml", "modified"],
  [".gitleaks.toml", "modified"],
  ["tools/ci_changes.sh", "modified"],
  ["tools/ci_workflow_security.ts", "modified"],
  ["tools/ci_hygiene.rs", "modified"],
  ["tools/test_ci_local.py", "modified"],
  ["justfile", "modified"],
  ["rust-toolchain.toml", "modified"],
] as const)(
  "protected CI authority change %s (%s) is refused even if legacy CI required is green",
  async (filename, status) => {
    const f = fixture([{ filename, status }]);
    await expect(f.run()).rejects.toThrow(
      "protected CI authority require a separately reviewed",
    );
  },
);

test("renaming protected CI workflow to innocuous docs path is refused", async () => {
  const f = fixture([
    {
      filename: "docs/old-ci-workflow.txt",
      previous_filename: ".github/workflows/ci.yml",
      status: "renamed",
    },
  ]);
  await expect(f.run()).rejects.toThrow("protected CI authority");
});

test("renames missing original path and unknown statuses fail closed", async () => {
  await expect(
    fixture([{ filename: "x", status: "renamed" }]).run(),
  ).rejects.toThrow("rename with missing source path");
  await expect(
    fixture([{ filename: "x", status: "surprise" }]).run(),
  ).rejects.toThrow("unknown changed-file status");
});

test("mismatched or changing head/base, missing files, and wrong base fail closed", async () => {
  await expect(
    fixture([{ filename: "docs/ok.md", status: "modified" }], {
      apiHead: "unexpected-head",
    }).run(),
  ).rejects.toThrow("stale pull-request head/base");
  await expect(
    fixture([{ filename: "docs/ok.md", status: "modified" }], {
      apiBase: "different-base",
    }).run(),
  ).rejects.toThrow("stale pull-request head/base");
  await expect(
    fixture([{ filename: "docs/ok.md", status: "modified" }], {
      changedOnSecondFetch: true,
    }).run(),
  ).rejects.toThrow("stale pull-request head/base");
  await expect(
    fixture([{ filename: "docs/ok.md", status: "modified" }], {
      changedFiles: 2,
    }).run(),
  ).rejects.toThrow("incomplete or truncated file list");
  await expect(fixture([], { baseRef: "dev" }).run()).rejects.toThrow(
    "accepts only gyldlab/keld main",
  );
  await expect(fixture([], { state: "closed" }).run()).rejects.toThrow(
    "absent, closed or changed",
  );
});

test("GitHub API problems, invalid PR/repo and pagination cap refuse admission", async () => {
  await expect(fixture([], { apiFailure: true }).run()).rejects.toThrow(
    "GitHub API unavailable",
  );
  await expect(fixture([], { repo: "evil" }).run()).rejects.toThrow(
    "expected base repository",
  );
  await expect(fixture([], { badEventNumber: true }).run()).rejects.toThrow(
    "valid pull request number",
  );
  await expect(fixture([], { changedFiles: 3000 }).run()).rejects.toThrow(
    "cannot safely enumerate all changed files",
  );
});

test("backslash and NUL in GitHub paths are refused, not normalized", async () => {
  for (const filename of [".github\\workflows\\evil.yml", "x\0y"]) {
    await expect(
      fixture([{ filename, status: "added" }]).run(),
    ).rejects.toThrow("malformed Git paths");
  }
});
