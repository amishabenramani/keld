# Spec: request error codes split by dispatch proof (KEL-142 §7 revision)

Status: draft
Linear: GH-1095 (#517; tracker of record; linked from KEL-143) · Revises: KEL-142 §7 ·
Owner: repository owner · Updated: 2026-10-22

Code citations are at `origin/main` `516644b0`. Evidence for the current
behaviour comes from the never-merge experiment PR #1082 (issue #1081). This
draft authorizes no production change. Implementation starts only after the
exact content is approved.

## 1. Goal & non-goals

A rejected `window.keld.invoke` call or Bun application call has to tell the
app one thing: whether it is safe to send the request again. Today it cannot.

- `KELD-WV-011` is registered as "rejected before application dispatch". The
  host also returns it after the CALL frame was written, when the app link ends
  or the page navigates while a call is pending.
- `KELD-API-001` covers both "no handler is registered", where no app code ran,
  and "the handler threw", where the handler may have run.

The app therefore cannot tell "never sent" from "may have run".

This revision splits the codes by what Keld can prove:

| Code | Class | Meaning |
|---|---|---|
| `KELD-WV-011` (narrowed) | Not sent, not retryable as is | the renderer request was refused before the host attempted any write: invalid, busy, stale, undeclared, or the app is quitting |
| `KELD-WV-012` (new) | **Definitely not dispatched** | no admitted application generation existed, so the host attempted no write |
| `KELD-WV-013` (new) | **Outcome unknown** | the host attempted the CALL write, or the page navigated while the call was pending; the handler may have run |
| `KELD-API-002` (new) | **Not executed** (asserted by `@keld/api`) | the call reached Bun, but no handler was registered, so no app code ran |
| `KELD-API-001` (narrowed) | Outcome unknown | the call reached the application stage and failed: decode, handler throw, encode, or oversize reply |

Observable outcome: each renderer invoke settles exactly once with one of these
codes or a reply. No layer of Keld ever replays a request. Only a WV-012 or
API-002 result permits one resend, and the app makes it.

Non-goals:

- navigation policy, the trusted-page predicate, and which documents may hold
  the bridge (separate work);
- the restart notification's design, delivery and ordering (separate spec; this
  spec only consumes it as the WV-012 resend trigger, §4.4);
- the secure app origin (#1090, KEL-79);
- a host-side dedup store, a "last dispatched id", or a Keld status-query API
  (all rejected, §4.1);
- a reply deadline for a hung Bun (§4.7; liveness, not classification);
- Windows or Linux renderer bridges (none exist; §4.6);
- new KIPC codes for Bun-side outbound calls (§4.5).

## 2. Spec refs

- `docs/specs/kel142-macos-renderer-bridge-api.md` §4.1 (no automatic replay),
  §4.2 (terminal outcomes), §6 (ordering, one terminal), §7 (errors). This
  revision replaces §7 and refines §4.2's "missing handler" bullet. Every other
  KEL-142 rule is unchanged.
- `docs/specs/gh527-worker-owned-blocking-call-transport.md` §4.4 (`WorkerLink`
  terminal codes) and §4.6 (`@keld/api` ERR answers). Consumed, not changed;
  see §4.5 and Open question 3.
- `docs/architecture/02-ipc.md`: KIPC frames and the `CallError` payload. Not
  changed.
- `docs/architecture/07-agent-experience.md` §2 and
  `docs/engineering/keld-error-codes.md`: registry rules (a heading lands only
  with its first emitter).
- KEL-143 acceptance: "prove recovery never silently replays an operation whose
  outcome is ambiguous".

The design deviates from no architecture section.

## 3. Acceptance criteria (binary, each becomes a test; §7 maps them)

1. **WV-012 only without a write.** The renderer call arrives while the router
   has no admitted generation (`current` is `None`). The host returns
   `KELD-WV-012`. `write_frame` is never invoked for that call, and the handler
   census for that call stays 0 in every generation.
2. **Failed or partial write is Unknown.** `write_frame` fails after k bytes,
   for any k including 0. The host returns `KELD-WV-013`, never `KELD-WV-012`.
   It clears the slot, retires the generation and writes no further byte to that
   stream.
3. **After the dispatch point is Unknown.** The CALL write completed, and then
   the link ends, the reader fails, the generation is revoked, or a lifecycle
   write fails. The host returns `KELD-WV-013` in every case, whether or not the
   handler ran.
4. **Navigation while pending is Unknown.** The page facade has a pending
   invoke when `pagehide` fires. The promise rejects with `KELD-WV-013`.
5. **No handler is API-002.** The Echo CALL reaches `@keld/api` while no
   handler is registered. Bun answers one ERR `KELD-API-002` and runs no app
   code. A handler that throws, a decode failure, an encode failure and an
   oversize reply answer `KELD-API-001`, never `KELD-API-002`.
6. **WV-011 is narrowed and still pre-write.** Every remaining `KELD-WV-011`
   emitter returns before `begin_echo_call` invokes `write_frame`. These are:
   - page facade validation;
   - the isolated-world busy check;
   - native admission;
   - the undeclared channel check;
   - quiescing;
   - lock poisoning;
   - the session-wide busy slot.
7. **Exactly one terminal, no replay.** Each invoke whose document is alive
   when the outcome is decided gets exactly one terminal. A later reply or
   retirement for the same call is discarded. No Keld layer re-sends a CALL
   frame.
8. **One resend after WV-012 or API-002 runs once.** After the successor
   generation is ready, the page resends once, as a new invoke. The handler
   census for the logical action is exactly 1.
9. **Bun outbound codes are unchanged and documented as Unknown.**
   - `WorkerLink.call` returns `KELD-IPC-022` both when the link had already
     ended and when the call was still pending at close. Neither case claims
     "not sent".
   - The receiver-side refusals `KELD-IPC-024` and `KELD-IPC-005` (capacity)
     keep "not executed".
10. **Error shape is unchanged.**
    - A page rejection is still an `Error` whose message starts with
      `"<code>: "`.
    - A Bun ERR still carries the code in `CallError.code`.
    - The wv-link result still carries it in `code`.

## 4. Design

### 4.1 First principles, reuse and rejected alternatives

Facts (each one is an atom from the GH-1095 claim, verified at `516644b0`):

- **F1. The dispatch point.** The renderer CALL's dispatch point is the
  instant `write_frame` returns `Ok` inside `begin_echo_call`
  (`crates/keld-core/src/app_session.rs:5853`).
  - `write_frame` writes the header with `write_all`, then the payload with
    `write_all`, then flushes (`crates/keld-ipc/src/link.rs:835`).
  - The host can observe this point: `KELD_KEL142_KIPC_CALL` is printed only
    after it (`app_session.rs:4245-4252`).
- **F2. Pre-write refusals.** The refusals in `begin_echo_call` all return
  before the slot is armed and before `write_frame` is invoked
  (`app_session.rs:5812-5840`). They are:
  - quiescing;
  - generation lock poisoned;
  - no admitted generation;
  - pending lock poisoned;
  - slot busy.
- **F3. What a failed write does not tell the host.** `write_all` reports `Err`
  without reporting how many bytes reached the kernel.
  - Bun's `FrameReader` dispatches only complete frames
    (`packages/@keld/kipc/src/transport.ts:664-665`).
  - So "a failed write is never dispatched" holds only if no later writer
    completes the truncated frame on that stream.
  - A census of every writer to that stream has not been done. Until fault
    injection and that census exist, a failed write is **Unknown**. This is the
    #1081 final review's caveat on the failed-write case.
- **F4. Nothing is observable after the dispatch point.** Once the write has
  returned, nothing the host can see separates these cases:
  - Bun never read the frame;
  - Bun read it but did not run it;
  - the handler ran;
  - the handler ran and replied.

  A successor generation is a new principal with no memory of its predecessor,
  so it cannot attest either. Only a live receiver can say "not processed"
  (RFC 9113 §8.7: a server "MUST NOT indicate that a stream has not been
  processed unless it can guarantee that fact").
- **F5. The no-handler check runs before any app code.**
  `dispatchApplicationCall` throws "Echo channel has no registered handler"
  before it calls `decodeRequest` or the handler
  (`packages/@keld/api/src/channels.ts:95-97`). `resolveEchoCall` then maps
  every throw to one `KELD-API-001` ERR (`packages/@keld/api/src/echo-call.ts:30-37`).
- **F6. The link opens lazily.** It opens only on the first `app.whenReady()`,
  `app.quit()`, `fs` or `auth` call (`packages/@keld/api/src/app.ts:74-97`,
  `120-121`, `162-171`). `channels.handle` is synchronous and never touches the
  link. A handler registered before the first `whenReady()` therefore exists
  before any CALL can arrive.
- **F7. The page facade cannot see dispatch.**
  - On `pagehide` it rejects its pending call
    (`crates/keld-wv/src/wkwebview/macos_bridge.rs:63-68`).
  - It has no signal that tells it whether the isolated world queued the call
    before bind or the host already wrote it.
- **F8. Bun outbound codes are shared between Not sent and Unknown.**
  - `WorkerLink.call` throws the recorded terminal code from `#throwIfTerminal`
    before it posts to the Worker (`transport.ts:2011-2017`, `2177-2180`).
  - It rejects a still-pending call with the same code in `#finalize`
    (`transport.ts:2506-2514`).

Reuse:

- The codes travel in fields that already exist:
  - `CallError.code` inside a KIPC ERR;
  - the wv-link result `code`;
  - the page `Error` message prefix.
- Settlement reuses the slots that already exist:
  - the page facade `pending`;
  - isolated-world `inflight`;
  - native `BridgeState.pending`, matched on navigation and request;
  - host `pending_echo`, matched on correlation and attempt
    (`app_session.rs:5885-5918`).
- Nothing is rewritten.

Rejected alternatives:

- **A single "retryable" flag on WV-011.** It still hides three different
  policies: no generation, busy and quitting.
- **WV-012 for every pre-dispatch failure, including a failed write.** F3: this
  is unproven.
- **A host-side dedup store or result cache.** The host sees the dispatch point,
  not the commit of the effect. A cache would fabricate results for Unknown
  calls and would carry a predecessor generation's results to a new principal.
- **"Last dispatched id" in the restart notification.** "Dispatched" is not
  "processed". The owner has also decided that the notification carries no ids
  or counters.
- **A Keld status-query API.** Only the app's durable store knows whether the
  effect happened.
- **Reclassifying `KELD-IPC-022` for the `#throwIfTerminal` case.** §4.5.

Compatibility fallback: none required. The codes are additive. §4.3 states what
still returns the old codes, and the safe default (§4.4: an unrecognized code
means Unknown) keeps old callers correct.

### 4.2 Failure stages and codes (renderer → host → Bun, macOS)

The stages run in request order. "DP" is the dispatch point (F1).

| # | Stage | Owner / site at `516644b0` | Today | After this revision |
|---|---|---|---|---|
| S1 | Page facade validation: `opts`, channel, payload type, size > 4096, page `pending` | `macos_bridge.rs:70-79` | WV-011 | **WV-011** (unchanged) |
| S2 | Isolated-world busy (`queued` / `inflight`) | `macos_bridge.rs:157-166` | WV-011 | **WV-011** |
| S3 | Native admission: envelope decode, stale or forged nonce, wrong frame/view/world, destroyed bridge, busy | `macos_bridge.rs:393-431`, `reply_immediate_error` `:801-820` | WV-011 | **WV-011** |
| S4 | Host dispatch: undeclared channel | `app_session.rs:4213-4226` | WV-011 | **WV-011** |
| S5 | `begin_echo_call`: session quiescing | `app_session.rs:5813-5819` | WV-011 "unavailable or already pending" | **WV-011**, detail "application is quitting" |
| S6 | `begin_echo_call`: a lock is poisoned | `:5820-5823`, `:5831-5834` | WV-011 (same text) | **WV-011**, detail "host call state is unavailable" |
| S7 | `begin_echo_call`: session-wide slot busy | `:5835-5840` | WV-011 (same text) | **WV-011**, detail "another application call is already pending" |
| S8 | `begin_echo_call`: no admitted generation (`current` is `None`) | `:5824-5829` | WV-011 (same text) | **WV-012** |
| S9 | `write_frame` returns `Err` (any byte count, k ≥ 0) | `:5853-5876` | WV-011 (same text) | **WV-013** |
| DP | `write_frame` returns `Ok` | `:5878` | (none) | (none) |
| S10 | After DP: link EOF/reset, reader error, `Revoke` retirement, failed lifecycle write, reply channel disconnected | retirement details at `:5869`, `:6436`, `:7002`, `:7044`, `:7718`; mapped at `:4278-4285` | WV-011 "application link ended before the renderer reply" | **WV-013** |
| S11 | Bun: no handler registered at dispatch time (never registered, unsubscribed, or registered after this call arrived) | `channels.ts:95-97` | API-001 | **API-002** |
| S12 | Bun: `decodeRequest` throws | `channels.ts:99` | API-001 | **API-001** (Open question 2) |
| S13 | Bun: handler throws or rejects | `channels.ts:100` | API-001 | **API-001** |
| S14 | Bun: `encodeResponse` throws, or reply > `MAX_FRAME_LEN` | `channels.ts:101`, `echo-call.ts:26-28` | API-001 | **API-001** (the effect is committed) |
| S15 | Reply in flight when Bun dies | host reader vs `Revoke` race | reply or WV-011 | reply or **WV-013**, never WV-012 |
| S16 | Page `pagehide` while an invoke is pending | `macos_bridge.rs:63-68` | WV-011 "renderer document navigated before reply" | **WV-013** |
| S17 | Bun alive but never answers | dispatch thread waits with no deadline (`:4253-4266`) | no terminal | unchanged (§4.7) |

How each crash point maps to a code:

| When it happens | Code |
|---|---|
| Crash before dispatch (generation already revoked) | S8 → WV-012 |
| Crash during the write | S9 → WV-013 |
| Crash after the write | S10 → WV-013 |
| Connection loss | S9 or S10 → WV-013 |
| Restart window | S8 → WV-012 |
| Navigation | S3 (stale nonce, before dispatch) → WV-011; S16 → WV-013 |
| Timeouts | none on the renderer path; S17 is out of scope |

The S5–S9 split requires one implementation change. `begin_echo_call` must
return a typed refusal that says which stage it failed at, and the dispatch
thread must map that stage to a code. It MUST NOT infer the stage from a
`HostAppError` detail string. Only the S8 variant may become WV-012.

Promotion rule for S9: a later revision MAY move k-byte write failures to
WV-012, but only if both of these pass:

- a fault-injection test with k ∈ {0, 1, HEADER_LEN−1, HEADER_LEN,
  HEADER_LEN+len−1};
- a complete census showing that no writer can touch the generation stream
  after a failed write.

Until then, S9 is WV-013.

Trust note for API-002: the code is asserted by the app's own Bun process and
passed through by the host. Keld guarantees only `@keld/api`'s emission site
(S11). App code that builds its own ERR frames can claim any code, with the same
trust as its handler.

### 4.3 Backward compatibility

- **Still `KELD-WV-011`:** S1–S7. Every one of these returns before the host
  attempts a write. The registry meaning "rejected before application dispatch"
  becomes true of every emitter again.
- **No longer `KELD-WV-011`:**
  - S8 moves to WV-012;
  - S9, S10 and S16 move to WV-013.
- **Still `KELD-API-001`:**
  - S12–S14;
  - every registration-time error that `channels.handle` throws synchronously
    to app code: reserved lifecycle channel, undeclared descriptor, a handler
    that is not a function, and a duplicate handler. These errors are not call
    outcomes.
- **No longer `KELD-API-001`:** S11 moves to API-002.
- **Shape is unchanged** (§3 criterion 10). Detail strings are diagnostic, not
  part of the contract; only the code prefix is.
- **Effect on existing callers:**
  - A caller that treats every rejection as "handle the error" sees no
    difference.
  - A caller that matched WV-011 as "safe to resend" was wrong for S9, S10 and
    S16. After this revision, those cases carry WV-013, which such a caller does
    not match, so it stops resending them. The change makes callers safer, not
    less safe.
  - `@keld/electron` has no `ipcRenderer`/`ipcMain` today, so no Electron
    caller exists.
- **No dual emission and no adapter.** Each call gets exactly one code.
- **Migration unit.** Each implementing task updates, in the same PR, the
  in-repo assertions on the old strings:
  - `crates/keld-host/tests/no_flag/macos/public_fs.rs:755-768`;
  - `crates/keld-host/tests/no_flag/macos/public_fs/outcomes.rs:88`;
  - `crates/keld-core/src/app_session/tests/native_renderer_main_entry.rs:162-167`;
  - `packages/@keld/api/src/echo-call.test.ts`;
  - `packages/@keld/api/src/channels.test.ts`;
  - the `macos_bridge.rs` unit tests.

  The `KELD-WV-011` registry `fix` and the `WvError::RendererBridge` `Display`
  text stop recommending a document reload.

### 4.4 Retry rules (normative for apps; Keld itself never resends)

**Default:** a code the app does not recognize means Outcome Unknown.

| Code | Rule |
|---|---|
| WV-011 | **Do not resend automatically.** If the detail says the request is invalid, fix it. If the detail is "already pending", resend only after your own pending call settles. If the detail is "application is quitting", never resend. |
| WV-012 | **One resend** after the successor Bun generation is ready (see the rules below). |
| WV-013 | **Never resend automatically**, unless it is made safe in one of the two ways below. |
| API-002 | **No app code ran.** Fix the registration order so the error cannot recur, then the page MAY resend once. |
| API-001 | Same as WV-013: the call may have run. |

WV-012 resend rules:

- The resend is a new `invoke`, with a new local request id and a new host
  correlation. It is never a replay of the old frame.
- The trigger is the trusted restart notification (separate spec). Until that
  ships, the only allowed trigger is a later user action or an app-level
  signal. Never use a timer or a backoff loop.
- A second WV-012, or an API-002, on the resend is surfaced to the user and is
  not resent again. The limit is at most one automatic resend per logical
  action.

WV-013 can be made safe to resend in one of two ways:

- **(a) An app-supplied idempotency key.** The page puts the key (for example
  `crypto.randomUUID()` per user action) in its own payload and keeps it across
  the resend. The Bun handler records `(key, result)` in the same durable
  transaction as the effect, returns the stored result for a repeated key, and
  passes the key on to external APIs.
- **(b) An app-defined status check.** The app's own handler reads that durable
  record and confirms the effect did not happen.

An in-memory map dies with the crashed generation and makes neither safe. Keld
ships no key store, dedup helper or status API (§4.1). Keld never reads, stores
or forwards the key.

API-002 guidance: register every `channels.handle` handler synchronously at
module top level, **before** the first `app.whenReady()` and before any `fs` or
`auth` call. F6 proves this ordering closes the window.

```ts
import { app, channels, echoChannel } from "@keld/api";

channels.handle(echoChannel, handleEcho); // first: no link exists yet
await app.whenReady();                    // then: opens the link
```

A handler registered inside `whenReady().then(...)` leaves a window after every
start and restart in which calls get API-002.

### 4.5 Bun-side outbound calls (role → host)

No new code is reserved or emitted; this section only records each code's class.

| Code | Class | Note |
|---|---|---|
| `KELD-IPC-004`, `KELD-IPC-005` local (deadline, frame bound, blocking call in flight) | Not sent, deterministic | thrown before any post |
| `KELD-IPC-022`, `025`, `026`, `027` | **Unknown** | the same code is thrown before posting (`#throwIfTerminal`) and for a call pending at close (F8); callers MUST NOT infer "not sent" from it |
| `KELD-IPC-023` | Unknown | already registered so |
| `KELD-IPC-006` | Unknown | the host may still run the call |
| `KELD-IPC-024` | Not executed | receiver-side: the host ran nothing behind an accepted Quit |
| `KELD-IPC-005` capacity | Not executed | receiver-side admission refusal |

A Bun crash kills the Bun caller, so splitting `KELD-IPC-022` would help only
with Worker death and host retirement while Bun is alive. That is not a Phase 2
need (YAGNI).

GH-527 and the comment at `transport.ts:2571-2572` say that a call pending at a
Quit-close "is treated as `KELD-IPC-024` (neither ran)". That is unproven for a
call written before the Quit. This revision governs callers: such a call is
Unknown until test AT-B3 decides it (Open question 3).

### 4.6 Platform notes

- **macOS:** the only renderer bridge (`#[cfg(target_os = "macos")]` on
  `begin_echo_call`, `pending_echo` and `start_renderer_dispatch`). Every
  renderer criterion is macOS-only.
- **Windows and Linux:** no renderer bridge. The `@keld/api` (S11–S14) and
  `@keld/kipc` (§4.5) rules apply on all three OSes.
- **A future Windows named-pipe Echo path:** it MUST re-prove F1 and F3
  (overlapped `WriteFile` can complete despite a cancel) before it emits WV-012.

### 4.7 Exactly-once settlement and no replay

- Each layer holds at most one pending entry and empties it **before** it
  delivers a terminal. A second terminal finds the slot empty and is dropped.
  The layers are:
  - page facade `pending`;
  - isolated-world `inflight`;
  - native `BridgeState.pending`, matched on `(navigation, request)`;
  - host `pending_echo`, matched on `(attempt, correlation)`.
- The dispatch thread sends exactly one outcome per renderer request.
- A reply that arrives after a retirement is refused by the correlation and
  attempt check, and the reader that would deliver it is already inert.
- Each stage's code is assigned at exactly one site.
- **No replay:**
  - no layer keeps the payload after the write;
  - no layer re-sends a CALL frame;
  - a successor generation never receives a predecessor's call;
  - Keld's only re-delivery is the app's own new invoke under §4.4.
- **Exception (existing behaviour, kept):** at session shutdown, the dispatch
  thread can stop without an outcome while the document is being destroyed.
  "Exactly once" therefore applies to documents still alive when the outcome is
  decided.
- **Hung Bun:** a call to a live Bun that never answers has no terminal (S17).
  This is a liveness defect, outside this classification spec (Open
  question 5).

### 4.8 Remaining design items

- **New or changed types:**
  - a private `begin_echo_call` refusal enum in keld-core, with variants
    `Quitting`, `StateUnavailable`, `Busy`, `NoGeneration` and
    `WriteFailed(HostAppError)`;
  - no public Rust type changes.
- **TypeScript:** `dispatchApplicationCall` throws a typed no-handler error,
  and `resolveEchoCall` maps it to `KELD-API-002`. Every other throw still maps
  to `KELD-API-001`.
- **Capabilities and manifest:** none.
- **Wire and protocol:** no frame, envelope, field or version change. New
  *values* travel in existing string fields (§8).
- **Runtime seam:** none. The execution owner, crash domain and ordering are
  unchanged.
- **Persisted state:** none.

## 5. Boundaries

- **Implement in:**
  - `crates/keld-core/src/app_session.rs` (`begin_echo_call` refusal type,
    renderer dispatch mapping) and its tests;
  - `crates/keld-wv/src/wkwebview/macos_bridge.rs` (`pagehide` code) and
    `crates/keld-wv/src/error.rs` (`fix` text);
  - `packages/@keld/api/src/{channels,echo-call}.ts` and their tests;
  - `docs/engineering/keld-error-codes.md`;
  - the KEL-142 spec §7;
  - the host macOS acceptance tests named in §7.
- **Must not touch:**
  - KIPC frame layout, `CallError` schema, `keld.wv-link/v1` shape;
  - `@keld/kipc` transport behaviour;
  - guard, permissions, workspace `Cargo.toml`;
  - navigation policy;
  - restart-notification code.

## 6. Tasks (each ≈ one PR; ordered)

- [ ] T0: this spec, the KEL-142 §7 current-behaviour correction, and the
  interim `KELD-WV-011` registry text. Docs only.
- [ ] T1: characterization tests that pin today's codes at S8–S16, with negative
  controls (AT-12a/13a/13b/13c arms written against today's strings). Test-only.
- [ ] T2: keld-core typed refusal, the S8 → WV-012 and S9/S10 → WV-013 mapping,
  the WV-012/WV-013 registry headings, and T1's assertions flipped to the target
  codes.
- [ ] T3: keld-wv `pagehide` → WV-013, the `WvError::RendererBridge` fix text,
  and the narrowed WV-011 registry text.
- [ ] T4: `@keld/api` S11 → API-002, its registry heading, and the unit and
  hosted API-002 tests.
- [ ] T5: developer guidance for §4.4: the idempotency pattern and registering
  before `whenReady`. The X12-style example app comes only when a consumer
  needs it.

T2–T4 are independent once T1 lands. The WV-012 resend trigger depends on the
restart-notification spec (Open question 1).

## 7. Test plan

Shared oracles:

- **Handler census:** every test handler POSTs `{gen, pid, nonce, phase}` to the
  test observer. The phases are `entered`, `effect-committed` and `returning`.
- **Host markers:** `RENDERER_ADMIT`, and `KELD_KEL142_KIPC_CALL` (DP).
- **Terminal count:** the number of page settlements per nonce.
- **"Census 0 forever":** re-checked after the successor generation serves three
  more calls.

Every hosted arm prints its raw census with `--success-output immediate`.

| ID | Criterion | Stage | Platform | N | Observable | Negative control / mutation |
|---|---|---|---|---|---|---|
| AT-12a | 1, 7 | S8 (G2 held before HELLO) | hosted macOS | 10 × arms {handler before `whenReady`, handler inside `whenReady`} | page code `KELD-WV-012`; `KIPC_CALL` = admits − 1; census 0 for G1 and G2 forever; 1 terminal | G2 released before the invoke → reply, census 1 (the oracle can count) |
| AT-12b | 1 | S8 | macOS unit (keld-core) | deterministic | WV-012 variant returned; the counting writer saw **0 write calls** | admitted generation → 1 write call and no WV-012; mutation: map `WriteFailed` to WV-012 → AT-13a fails |
| AT-12c | 8 | S8 then resend | hosted macOS | 10 | after host Ready for G2, one resend → reply; census for the logical action = 1 | resend before G2 is admitted → WV-012 again (readiness is load-bearing) |
| AT-13a | 2 | S9, k ∈ {0, 1, HEADER_LEN−1, HEADER_LEN, HEADER_LEN+len−1} | macOS unit (keld-core; extends `failed_primary_writes_retire_the_generation`) | deterministic per k | WV-013; slot cleared; generation retired; 0 bytes written to the stream after the failure | k = full length → DP reached, no S9; characterization: an injected completing write after failure makes the Bun `FrameReader` dispatch (proves why S9 cannot be WV-012 yet) |
| AT-13b | 3, 7 | S10 | hosted macOS | 10 per arm | arms: (i) SIGSTOP Bun → invoke → SIGKILL: census 0; (ii) kill at `entered`: census 1; (iii) kill after `effect-committed`: effect present. Every arm: `KELD-WV-013`, 1 terminal, G2 census for the nonce 0 forever | arm (ii) census 1 under the same code proves WV-013 cannot mean "not sent"; mutation: S10 → WV-012 fails arm (ii) |
| AT-13c | 4 | S16 | hosted macOS | 10 | handler held at `entered`; page `location.reload()`; the `pagehide` rejection beacon reports `KELD-WV-013`; census 1; no settlement in the successor document | navigate while queued before bind → census 0 with the same WV-013 (the facade cannot tell) |
| AT-13d | 7 | S15 | hosted macOS | 50 | kill Bun on the `returning` POST; outcome ∈ {reply, WV-013}; never WV-012; 1 terminal | none beyond AT-13b; the success:Unknown ratio is reported, not asserted |
| AT-API2a | 5 | S11–S14 | `bun test` in `@keld/api`, macOS + Linux + Windows CI | deterministic | missing → API-002, census 0; late (call, then register, then call) → API-002 then reply, census 1; unsubscribed → API-002; handler throws after effect → API-001, census 1; decode failure → API-001, census 0; oversize → API-001, census 1 | mutation: move the no-handler check after `handler(request)` → the census for the API-002 case becomes 1 and the test fails; a throwing handler never yields API-002 |
| AT-API2b | 5, 8 | S11 across restart | hosted macOS | 10 × arms {inside `whenReady`, before `whenReady`} | inside: at least one post-restart API-002 with G2 census 0, then one resend → census 1; before: 0 API-002 across all N | the inside arm reproduces the window (round-1 E8 hit it 10/10 under API-001) |
| AT-11 | 6 | S1–S7 | macOS unit (keld-wv + keld-core) | deterministic | each S1–S7 reason → WV-011 and 0 write calls | mutation: emit WV-011 from S9 → fails |
| AT-B1 | 9 | `#throwIfTerminal` | `@keld/kipc` unit, 3 OSes | deterministic | link already ended → `call()` rejects IPC-022; host fixture read 0 frames | remove `#throwIfTerminal` → frame posted (the check is the proof, but the code stays Unknown) |
| AT-B2 | 9 | pending at close | `@keld/kipc` unit, 3 OSes | deterministic | pending, then host close → IPC-022; host read 1 frame; same code as AT-B1 | none: the code identity *is* the assertion |
| AT-B3 | 9 | written before Quit, pending at close | keld-core unit | deterministic | host census for that CALL recorded; decides Open question 3 | written after Quit → IPC-024, census 0 (existing `guarded_router_and_quit.rs`) |
| AT-S | 7 | settlement | macOS unit (keld-wv, keld-core) | deterministic | late reply after retirement refused; facade result after `pagehide` ignored; native result for a stale `(navigation, request)` dropped | remove the corr/attempt check → a second settlement is observed |

Anti-flake:

- No sleeps. Holds and releases use the existing test-only observer gates.
- Kills are driven from census POSTs, not timers.
- Hosted arms use the PR #1082 fixtures, promoted into landing tests by T1.

## 8. Review gates triggered

| Gate | Applies | Reason |
|---|---|---|
| Public API | **Yes** | three new page- and app-visible codes; narrowed meanings for `KELD-WV-011` and `KELD-API-001`; normative retry guidance; KEL-142 §7 revised |
| Wire protocol | **No** (flag for the reviewer) | no KIPC frame, `CallError` schema, `keld.wv-link/v1` envelope field or version change. The new codes are new *values* in existing string fields (`CallError.code`, wv-link result `code`). If the reviewer counts registered code values as wire vocabulary, this becomes Yes for those values only, with no version bump, because receivers treat an unknown code as Unknown |
| Permission model | No | no grant, capability or guard path |
| Dependency addition | No | none |
| Unsafe | No | no FFI change |

## 9. Perf impact

None. `begin_echo_call` returns an enum instead of an error built from strings,
on a cold refusal path. The hot reply path is unchanged. No budget in
architecture 01 §5 can move.

## 10. Open questions (owner decisions; recommendation first)

1. **When the WV-012 resend trigger ships.** Recommend shipping WV-012 in T2,
   before the restart notification. Until then, the guidance allows the one
   resend only on a later user action or an app-level signal. The alternative
   is to gate T2 on the notification spec. That delays a correct
   classification without making anything safer.
2. **Decode failure (S12).** Recommend keeping it on `KELD-API-001`: it is
   conservative and the request is deterministic, so a resend of the same bytes
   cannot help. The alternative is a fourth code ("not executed, invalid
   request"), which the owner's reservation did not include.
3. **The GH-527 claim that "022 at a Quit-close = 024".** Recommend correcting
   the GH-527 text and the `transport.ts` comment to "Unknown" unless AT-B3
   proves that the host runs no call written before an accepted Quit. Until
   then, §4.5 governs callers.
4. **Promoting S9.** Recommend keeping failed writes as WV-013 permanently
   unless the §4.2 promotion rule passes in a later reviewed revision. The
   value is small, because a write failure is rare and already retires the
   generation.
5. **Hung Bun (S17).** Recommend a separate liveness issue: no reply deadline,
   and a session-wide slot that makes every other view "busy". It is not part
   of this split.
