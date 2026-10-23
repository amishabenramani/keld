# Spec: Bun 1.4.4 and native JavaScript tooling

Status: draft
Linear: GH-1091 (user-requested combined PR; internal reference reconciliation pending) · Owner: GYLDLAB · Updated: 2026-10-23

## 1. Goal & non-goals

Use stable Bun 1.4.4 consistently for active repository runtime pins, CI and current
setup guidance. Give contributors and newly created Keld applications native Bun
formatting commands with a non-writing check suitable for CI. The user also requests
native linting; this remains blocked because the verified stable binary has no native
`bun lint`, and upstream documents lint as canary-only.

Non-goals: relabel historical receipts, replace TypeScript's strict checker, change
application runtime ownership or permissions, claim faster builds, add a third-party
formatter/linter, or silently upgrade the application runtime to canary.

## 2. Spec refs

No boundary change. Architecture 06 §1 owns the supervised, pinned Bun runtime;
architecture 07 owns developer/agent tooling. Existing KEL-77 differential tests keep
their oracles. Public claims remain independently owned by CONTRIBUTING.md.

## 3. Acceptance criteria (binary, each becomes a test)

1. Every active setup-bun step, packageManager field and strict runtime assertion
   selects 1.4.4; a stale 1.4.2, canary, latest or prefix-lookalike fails pin admission.
2. The existing package tests, strict TypeScript checks and Rust workspace gates pass
   with the selected stable binary. Each unrun platform is explicitly unverified.
3. Repository JavaScript/TypeScript formatting uses native `bun fmt --no-script`;
   its check variant returns nonzero for deliberately unformatted input without
   modifying it, formatting repairs it, and a second check leaves it unchanged.
4. `keld create` emits a dependency-free package with `format` and `format:check`
   scripts running the same native commands. A real generated app passes both;
   intentionally unformatted app code fails its check before formatting fixes it.
5. Generated JavaScript/TypeScript artifacts are updated only through their owning
   generators. Formatting a generated output and regenerating must agree; an
   incompatible generator is corrected at its owner before enabling the gate.
6. CI and `just ci` execute the formatter check for affected JavaScript inputs,
   including scaffold and generator inputs. Removing the command or its required
   admission dependency makes a negative control fail.
7. No successful lint check, native lint script or shipped-lint claim exists unless
   the selected, exact upstream tool version actually supports native lint. On the
   stable choice, the missing capability remains explicitly tracked on GH-1091.

## 4. Design

Reuse the upstream Bun binary and existing `just`/CI inventory, input-fingerprint
generator, package manifests and CLI template owner. Do not add a second formatter
implementation or a formatter runtime dependency. Keep `tsc --noEmit` and existing
semantic tests; formatting and linting are separate contracts from type checking.

Repository formatting selects the tracked JavaScript/TypeScript family, including
tests and tooling, through one owning recipe. Do not run an unrestricted formatter
over instruction Markdown, historical evidence, nested research or vendor files.
Use Bun defaults unless an existing project contract requires an explicit setting;
disable import sorting and other optional transforms by leaving their defaults off.
Generated files must reach the same formatting through their existing owner, never
through manual edits. Preserve intentional malformed parser fixtures as fixtures,
with a named path and regression oracle rather than a broad source exclusion.

Generated applications expose `format = bun fmt --no-script` and
`format:check = bun fmt --no-script --check`. These commands run in the application
checkout with the contributor's normal tooling authority, not in the sandboxed
application process. Native commands must not recursively call identically named
package scripts. CLI/template tests execute both commands in a temporary app and
inspect exit codes, bytes and a second successful check.

New Rust types/channels, capabilities, manifest grants and wire changes: none.
Runtime seam: none. Tool processes are cold developer/CI subprocesses; existing
supervised application lifetime and host handles remain owned by their current
owners. Platform notes: prove Windows locally; require actual Linux/macOS CI for
their corresponding tests. No inference from one OS to another.

Migration unit: active Bun CI pins, strict runtime expectations, packageManager
metadata, setup docs, JS formatting recipes/CI admission, scaffold scripts and any
formatter-incompatible generated source owner. Historical evidence stays historical.
Compatibility fallback: existing semantic tests and type checker remain required;
there is no false-green fallback for unavailable native lint.

## 5. Boundaries

Implement in `.github/workflows/ci.yml`, existing `tools/ci_*` owners, `justfile`,
`packages/@keld/*/package.json`, CLI template/create tests and relevant current setup
documentation. Correct a generator only if its output violates the selected formatter
contract, after loading its governing spec and nearest crate rules.

Must not touch Cargo dependency selection, unsafe production paths, permissions,
wire parsing, historical compatibility receipts or unrelated public readiness work.

## 6. Tasks (each ≈ one PR; ordered; no placeholders — vertical slices only)

The user explicitly requests these slices in the same PR as the public-claims fix.

- [ ] T1: update active stable pins and current docs; prove existing pin rejection
  controls accept only 1.4.4 and execute existing semantic gates with that binary.
- [ ] T2, after human spec approval: integrate native formatting into the repository
  and generated apps, with failure-first check/mutation tests and CI admission.
- [ ] T3: resolve upstream lint capability choice before implementing lint. Stable
  native lint is unavailable. A separately pinned canary developer toolchain needs
  its own exact-version, installation, checksum and CI design added here for approval.

## 7. Test plan

AC1: existing CI-hygiene tests plus stale-pin/canary/latest/prefix negative controls.
AC2: frozen package installs, strict package typechecks/tests, `just ci`, format,
warning-denied workspace clippy and full workspace nextest; hosted OS-specific gates.
AC3–5: real native formatter subprocesses, exact file hashes before/after check,
idempotent output and real generated app tests; regenerate using current generators.
AC6: existing CI router/hygiene suites with command-removal negative control.
AC7: exact stable binary `bun lint --help` exit1 and `Script not found "lint"`;
recheck upstream capability before any future claim. Tests use isolated temporary
directories, awaited subprocess exit, no sleep synchronization or external network.

## 8. Review gates triggered

Dependency (Bun runtime bump); independent architecture/dependency review of exact
diff. Unsafe, public Rust/TS API, permission model and wire protocol: none.
CI/shared input owners also require independent review under existing policy.

## 9. Perf impact

None claimed. Formatting adds a developer/CI check; no runtime performance change
is claimed from the language or upstream benchmark numbers.

## 10. Open questions

Human approval required before T2: approve the stable 1.4.4 native-formatting design
above. For lint, recommend keeping the application/runtime and tooling stable and
tracking native lint until an exact stable release includes it. Alternatively,
authorize designing a separately pinned canary developer-only lint toolchain; that
is a distinct version and support contract, not a native lint feature of 1.4.4.

Upstream evidence (retrieved 2026-10-23):
[release](https://github.com/oven-sh/bun/releases/tag/bun-v1.4.4),
[formatter release notes](https://bun.sh/blog/bun-v1.4.4),
[lint availability](https://bun.sh/docs/lint).
Local checksum-verified Windows binary: `1.4.4+38afee6c8`; `bun fmt --help` exits0,
`bun lint --help` exits1 with `error: Script not found "lint"`.
