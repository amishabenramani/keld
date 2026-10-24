# Spec: first-proof `keld migrate` output, ownership and honest report (GH-497)

Status: draft
Linear: GH-497 (#517 GitHub tracker of record; KEL-127 sequencing reference) · Owner: @0monish · Updated: 2026-10-24
Disposition: **OPEN / DRAFT / DO NOT MERGE**. The CLI verb remains reserved (`KELD-CLI-045`); this document and its architecture corrections do not implement or execute migration.

## 1. Goal & non-goals

Define one auditable, reversible, fail-closed output contract for the first macOS draw.io migration. Default `keld migrate` only inspects source and prints a **static estimate, not a compatibility score**. `keld migrate --write` may modify only the approved, owned output set after explicit write approval, and may request Legacy only after **separate affirmative acceptance of its reduced security guarantees**. The specification is the common contract consumed by GH-499's hand-authored fixture, not a hidden second resolver, profile source or permissions evaluator.

Non-goals: implementing the CLI or analyzer scanner (X02-T7/X02-T8); building, installing or executing draw.io (#589); schema inventions for #452/#453; changing #586/#588 profile admission, #541/#552 channel evaluation or #502 package resolution; automatic source patching or dependency installation; asserting strict sandbox security under Legacy; publishing a KEL-74 product score, install PASS, percentage or native evidence from static source inspection.

## 2. Spec refs

- [Architecture 04](../architecture/04-electron-compat.md) §§1–4,6 and [06](../architecture/06-runtime-and-tooling.md) §2 own developer UX and reserved-verb distinction; [07](../architecture/07-agent-experience.md) §§2,4 owns the live error/finding shapes.
- [Architecture 03](../architecture/03-security.md) §3, [KEL-78/#586/#588](https://github.com/gyldlab/keld/issues/588) own explicit root `keld.permissions.jsonc.profile`, admission and its exact security-forfeit text; no implicit Legacy.
- [GH #541](https://github.com/gyldlab/keld/issues/541), #552/#542 own **exact per-window** `el:<channel>` permission parsing, evaluation and navigation generation. [GH #499](https://github.com/gyldlab/keld/pull/1170) owns the pinned draw.io artifact, not a generic analyzer.
- #452 owns role-env/session facts under KEL-75; #453 owns the single-instance declaration; #502 owns the one installed `@keld/electron` ESM/CJS resolver; #491 owns `electron-updater` adapter; #478 owns host-dialog grants. #587/#500/#589 own test corpus, authentic receipts and scoring.
- Existing consumers: `crates/keld-core/src/hello.rs` reads root `name` and `entry`; `crates/keld-cli/src/doctor.rs::DoctorFinding` serializes `label,ok,detail,error?`, with `KeldErrorObject {code,message,cause?,fix,docs}`. `keld_permissions_explain` currently evaluates the live manifest without a recorder; channel explanation is not yet supported.

## 3. Acceptance criteria (binary; each must have an independent falsifier)

| ID | Given / when | Observable and single mutation that must fail |
|---|---|---|
| M01 | No `--write` / analyze an existing project | Zero filesystem mutations, child processes or package installations; adding a write before opt-in fails |
| M02 | `--write` with a diff | An immutable preimage, planned file list and reversible before/after file hashes; one unlisted file or irrevertible write fails |
| M03 | `--write` requests `profile: "legacy"` | Separate affirmative security-forfeit consent, not implied by `--write`, Electron sandbox flags, or env; removing separate consent fails |
| M04 | A proposed writable key is reported | Its file, grammar, exact owner and consuming test ID are mapped in §4; deleting owner or using a guessed unimplemented key fails |
| M05 | Profile declaration is produced | Only unique root `keld.permissions.jsonc.profile` strict/legacy from #586/#588; moving it to config, duplicating/nulling it fails |
| M06 | Declared role environment is proposed | #452-approved exact field only; `KELD_`-prefixed names, loader variables or environment authority fail |
| M07 | Channel grants are proposed | Each literal `el:<channel>` is an **active** main-side `ipcMain.on/once/handle` literal intersecting an observed operation; then additionally bound to a Host-owned window/document identity for actual authority. `el:*`, comment-only, computed, renderer-derived or unioned channels fail |
| M08 | System grants are proposed | Only actual approved `app.system` dialog/menu literals; no `fs` grants under Legacy, `$VAR` expansion, hostless globs or updater network grant; one wildcard or guessed grant fails |
| M09 | Package dependencies/scripts are proposed | One `electron` package key resolves to #502 `@keld/electron`, `electron-updater` to #491 adapter, and Electron-binary/builder scripts are removed/replaced; any second resolver or surviving binary script fails |
| M10 | Migration outputs are enumerated | `keld.build.ts`, `keld.compat.ts`, Bun alias and tsconfig `paths.electron` are not written, each with printed reason; reinserting any fails |
| M11 | Static report is printed as text/JSON | ✔/▲/✘/unclassified counts and every dropped input with reason, owner and next action; `compat score: 91%`, `expected to run` or unqualified percentage fails |
| M12 | Product claim is requested | KEL-74 claim line only for a documented **committed** corpus ID with complete current manifest/evidence; synthetic/source-only green or unknown installation fails |
| M13 | Existing source inputs are ignored | `disableBlinkFeatures`, unsupported Chromium command-line switches and updater `setFeedURL` values are reported with reason, not silently re-created as grants; hiding one dropped input fails |
| M14 | Report error is serialized | Reuse actual `DoctorFinding` and nested `KeldErrorObject` field shapes and appropriate existing error registry owner; inventing success for rejected inputs or a fake live CLI result fails |

## 4. Design

**Atomic owners and trust.** The analyzer is untrusted static input: neither application source nor a renderer can mint Host authority. The CLI is the proposed report/write coordinator only. Root Keld config, permissions guard, package resolver, Host role-spawn, profile admission and product evidence remain their listed owners. Scanning source is not an OS receipt. Filesystem changes require explicit `--write`, an immutable preimage manifest, conflict detection before edits, bounded atomic writes, a complete per-file diff and a reversible rollback plan; a partial write must be reported as failure, never success.

**Normative output mapping (planned, not automatically valid on current main).** Each row names one existing or future parser/admission owner and the earliest relevant consumer/negative-test identifier. A missing implementation means **BLOCKED**, not permission to make up a key.

| Output/key or operation | Sole owner and reader | Consumer test/gate | First-proof status |
|---|---|---|---|
| `keld.config.ts` root `name` | existing `keld-core::hello` reader | M04 + GH-499 A09, hello config tests | Existing root syntax; use actual compiler |
| `keld.config.ts` root `entry` | existing hello-config / CLI boot compiler | M04 + GH-499 A09, boot-stage entry test | Existing relative path; no `defineConfig({app:{...}})` sketch |
| `keld.config.ts` root `initialWindow: "none"` for Electron facade boots | #446/F01-T2, approved schema-2 CLI compiler and pending Host admission | M04 + GH-446 facade no-renderer boot test; GH-499 A09 | CLI producer exists in DRAFT #1171, Host still rejects schema 2; macOS-only until qualified |
| `keld.config.ts` single-instance **declaration** | #453/F01-T5, Host lock owner | M04 + GH-499 A09, native lock verdict | **BLOCKED until #453 approves/implements key** |
| `keld.config.ts` role environment, exactly `DRAWIO_DISABLE_UPDATE=true` | #452/F01-T4 under KEL-75 amendment | M06 + GH-499 A08, rejected loader/env test | **BLOCKED until #452 approves/implements key**; no `KELD_*` |
| `keld.permissions.jsonc` root `profile: "legacy"` | #586/X03-T5 spec; #588/X03-T7 live admission; parent KEL-78 | M03/M05 + GH-499 A03, host profile receipt | Selected root `/profile`; consent and runtime admission separate |
| `keld.permissions.jsonc` `windows.<w>.channels` literal `el:<channel>` | #541/F04-T7 spec; #552 guard evaluator; #542 navigation principal | M07 + GH-499 A04/A06, deny-before-Bun | **BLOCKED** until actual parser, Host window binding and evidence |
| `keld.permissions.jsonc` exact `app.system` dialog/menu | #478/F06-D3 + guard manifest owner | M08 + GH-499 A07, denied unauthorized system operation | **BLOCKED** where absent in active parser; no guessed token |
| `package.json` dependency key `electron` → `@keld/electron` | #502/X03-T1 single package resolver | M09 + GH-499 A10, ESM/default/CJS require | **BLOCKED** until resolver consumes installed tree |
| `package.json` `electron-updater` → reviewed adapter | #491/F09-T1 | M09 + GH-499 A10, import-time inert adapter | **BLOCKED** until adapter exists |
| `package.json` dev/build scripts | #497 migrate output; live CLI verb owner for allowed replacements | M09 + GH-499 A11, no `electron .`/builder binary | Never substitute `keld build` as if currently implemented |
| Report summary, reversibility, dropped inputs | #497/X02-T1 contract; future X02-T8 writer | M01/M02/M10–M14 | Proposed static report only; no corpus claim by default |

**Forbidden or removed outputs, each printed verbatim with a reason:** `keld.build.ts` — `not generated: keld build is reserved (KELD-CLI-045)`; `keld.compat.ts` — `not generated: no schema or live reader`; Bun `bunfig.toml` electron alias — `not generated: package dependency is the sole #502 resolver`; `tsconfig.json` `compilerOptions.paths.electron` — `removed if present: second resolver forbidden` (report previous value and reversible diff). No `fs` grant from a dialog selection; no update-feed network grant from `autoUpdater`. `disableBlinkFeatures`, Chromium switches and `setFeedURL` values appear under dropped inputs with their owner-specific reason.

**Report contracts (future, not live).** A human report starts `Keld migrate static analysis — <project>`, then an explicit `static estimate, not a compatibility score` label, four count rows `✔`, `▲`, `✘`, `unclassified`, granted and denied proposals, security profile/forfeiture, omitted files/reasons, dropped inputs and planned diff/rollback receipt. On read-only analysis, `planned` must not say `written`. A JSON result wraps the same estimate and *actual* `DoctorFinding {label,ok,detail,error?}` values; every error uses the existing `KeldErrorObject {code,message,cause?,fix,docs}` shape. Do not invent a stable CLI error code without its error-registry owner or turn a hypothetical output into a live result. Only an independently committed KEL-74 corpus can add its separate qualified product-claim line.

**Existing abstractions and alternatives.** Reuse the current exported-root config scanner and guard manifest parser; do not execute arbitrary `keld.config.ts`. A second `electron` resolver through a Bun alias, TS paths, plugin or duplicated CommonJS entry conflicts with #502; use package-manager dependency resolution. Rejected: app-env authority, wildcard/`el:*` grants, automatic FS scope from a dialog, unsupported percentage claims and `keld build` output while the verb is reserved. No new Rust wire, FFI, principal, dependency or privileged channel in this spec.

**Platform/lifecycle.** The report grammar is platform-neutral; the initial draw.io artifact is macOS-only under explicit Legacy, and it forfeits Keld's strict zero-ambient-authority claim. #446's approved facade selection requires `initialWindow: "none"` and no `renderer` key; the current #1171 compiler produces schema 2 but the Host does not yet accept it. The ordinary non-facade example in architecture 04 remains schema 1. Linux/Windows native acceptance is unknown. The proposed CLI owns atomic file writes and rollback artifacts only; Host still owns spawned role facts, root permission verification, authenticated window/document generations and application lifecycle. A developer's static report cannot override those live outcomes.

## 5. Boundaries

- **This PR only:** `docs/specs/gh497-migrate-output-contract.md` and the listed normative errata in `docs/architecture/03-security.md`, `04-electron-compat.md`, `06-runtime-and-tooling.md`, `07-agent-experience.md`; generated docs only if the actual generator requires them.
- **Do not touch:** production CLI/Host, `keld-guard`, `keld-core`, `@keld/electron`, draw.io app source, #449 `app_session.rs`, #446 branch, #499 hand-authored spec, shared CI/wire/agent policies or permission defaults. No actual artifact or product claim.

## 6. Tasks (ordered, one concern per implementation PR)

- [ ] T0 — Publish this draft spec + same-PR architecture errata; validate structural, doc/generator and negative-control checks; obtain exact-content human approval. **STOP before implementation.**
- [ ] T1 — Future X02-T8 writer implements read-only analysis/report with independent zero-write and invalid-input tests. No unsupported keys are written.
- [ ] T2 — Future X02-T8 write path implements exact owned outputs only after #452/#453/#491/#502/#541/#588 schema/permission admission gates, separate Legacy consent, rollback receipt and conflict tests.
- [ ] T3 — #499 consumes the approved contract to author and validate pinned draw.io output; #589 independently runs the real Mac app/OS gates.

## 7. Test plan

| Acceptance / fault | Independent oracle and planned negative control |
|---|---|
| M01–M03 | Temporary real directory byte/file census before/after default analysis; mutate to write unapproved file, suppress explicit-forfeit consent, or leave a failed partial write |
| M04–M06 | Exact config/guard-reader validation; replace one owner cell with parent KEL-78, move root `profile`, inject `KELD_APP_LINK` role env or output unimplemented #453 key |
| M07–M08 | Real pinned-source comment-aware main-registration census plus guard parser, Host window/generation source; add inactive `ipcMain.on('log')`, `el:*` or `$HOME/**` and require refusal |
| M09–M10 | Resolved installed-package tree + reversible manifest/scripts diff; reintroduce `electron .`, bunfig alias, TS path, `keld.build.ts` or omit any not-generated reason |
| M11–M14 | CLI/JSON structured report comparison; inject `compat score: 91%`, uncommitted corpus ID, omitted discarded `setFeedURL` or fabricated error/finding fields |
| T0 docs gates | Required section and mapping-field census; grep architecture samples/rows for stale percentage/score, `just llms-test`, `just llms-check`, `just hygiene`, final CI router/docs checks |

Tests that need native Mac/Host or a live CLI must remain **planned/unrun**, not PASS from this docs-only PR. Every test above has a one-fault falsifier, and no unchanged app fixture or synthetic log is admitted as product proof.

## 8. Review gates triggered

- **Public API:** yes (future CLI/report and JSON consumer contract; source-only now).
- **Permission model:** yes (explicit Legacy consent, exact per-window literals and host-owned source rule). Independent exact-head security review before implementation.
- **Unsafe:** none. **Dependency addition:** none. **Wire protocol:** none.
- Independent exact-head documentation/architecture review and hosted required CI; DRAFT/DO NOT MERGE until spec approval and guards.

## 9. Perf impact

None measured or claimed by this documentation-only change. Future analyzer and write path must record actual project-size work, time, file/IO allocation and baseline; no static compatibility counts imply runtime performance.

## 10. Open questions

**Implementation blockers rather than discretionary new keys:** #452 role-env grammar/session admission, #453 single-instance schema, #491 updater adapter, #502 sole resolver, #541/#552 grants and #542 real document generation, #588 effective Legacy admission, #446/#449 Host Ready/window, and #589 authentic Mac receipt. The spec can be approved without fabricating these implementations, but T2/T3 must fail closed until the respective owners produce and verify them. Exact-content approval of this draft remains outstanding; it cannot be inferred from the approval of #500/#589 or a green docs-only CI run.
