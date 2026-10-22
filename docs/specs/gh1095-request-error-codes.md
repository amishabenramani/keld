# Spec: request error codes split by dispatch proof (KEL-142 §7 revision)

Status: draft
Linear: GH-1095 (#517; tracker of record; linked from KEL-143) · Revises: KEL-142 §7 ·
Owner: repository owner · Updated: 2026-10-23

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
codes or a reply. No layer of Keld ever replays a request. The rules for an
app resend are in §4.4:

- only WV-012 or API-002 permits one *automatic* resend of an action that is
  not idempotent;
- WV-011 is never resent automatically, whatever its detail says;
- WV-013 and API-001 may be resent only under §4.4 (a) or (b).

The app makes every resend; Keld makes none.

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
  terminal codes) and §4.6 (`@keld/api` ERR answers). Consumed. T0 adds only
  interim notes where it says that a call pending at a Quit-close "neither ran"
  (§4.5, Open question 3).
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
   has no admitted generation (`current` is `None`). This covers both app
   startup, before the first generation is admitted, and the gap after a crash.
   The host returns
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
   - native admission (S3b);
   - the undeclared channel check;
   - quiescing;
   - lock poisoning;
   - the session-wide busy slot.

   Hostile envelopes rejected at S3a get no terminal and are never dispatched.
7. **Exactly one terminal, no replay.** Each facade-formed invoke whose
   document is alive when the outcome is decided gets exactly one terminal.
   A hostile envelope that skips the facade can get none (§4.2, S3a). A later
   reply or retirement for the same call is discarded. No Keld layer re-sends a
   CALL frame.
   - This includes a document whose bridge state was reset by a host
     `navigate()` whose load never committed. That document stays live, and
     its admitted call settles once with `KELD-WV-013` (§4.7).
8. **One resend after WV-012 or API-002 runs once.** The page resends once,
   as a new invoke, and the handler census for the logical action is exactly 1.
   The trigger depends on the code:
   - after WV-012, the trigger is the first generation (at startup) or the
     successor generation becoming ready;
   - after API-002, the trigger is the handler being registered, which may
     happen in the same generation (late registration).
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
    - The wv-link result still carries it in `code`. The only addition is the
      private `document` field of criterion 12, which never reaches the page.
11. **The host never trusts a Bun-supplied `KELD-WV-*` code.** Bun answers an
    Echo CALL with an ERR whose `CallError.code` starts with `KELD-WV-`. The
    host forwards it to the renderer as `KELD-WV-013`.
    - A Bun ERR with any other code (`KELD-API-*`, `KELD-IPC-*`) is forwarded
      unchanged.
    - Only the host can mint `KELD-WV-011` and `KELD-WV-012` for a renderer
      call.
12. **A result settles only the document and request that sent it.**
    - Each native→isolated result carries the document nonce of the request it
      answers. The nonce stays inside keld-wv end to end (§4.7). The structs
      that cross into keld-core (`RendererBridgeRequest`,
      `RendererBridgeOutcome`) carry no nonce.
    - The isolated world settles a result only if both of these hold:
      `document` equals its current nonce, and `request` equals its in-flight
      request id. It drops any other result without changing `inflight`.
    - A stale invoke from a predecessor document therefore never settles the
      successor's pending call.

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
  `app.quit()`, `fs` or `auth` call. `ensureLink` is at
  `packages/@keld/api/src/app.ts:74-97`, and its only callers are at `108`
  (quit), `122` (whenReady), `164` (fs) and `171` (auth). `channels.handle` is synchronous and never touches the
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

- **F9. The host forwards Bun's ERR code verbatim.** The reader decodes the
  Echo ERR's `CallError` (`app_session.rs:7766-7770`), and the dispatch thread
  copies `error.code` into the renderer outcome (`:4272-4278`). Today, Bun can
  therefore make the page see any code, including `KELD-WV-012`, for a call
  that Bun received.
- **F10. A native result reaches whichever document is current.**
  - `reply_immediate_error` (`macos_bridge.rs:801-822`) and the dispatch outcome
    path evaluate `__keldBridgeNativeResult` in the WebView's current isolated
    world.
  - That function (`:137-147`) clears `inflight` and relays the result without
    checking its request id, and every document starts request ids at 1.
  - So a refusal of a stale invoke from a predecessor document, handled after
    the successor binds, can settle the successor's pending call.

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
| S3a | Native: wrong content controller/world, subframe, no or wrong WebView, envelope decode failure (only a hostile envelope that skips the facade reaches these) | `macos_bridge.rs:699-745` (`report_detail_rejection`, then `return`) | no terminal | unchanged: **no terminal**, zero dispatch |
| S3b | Native admission: stale or forged nonce, busy; host dispatch queue unavailable | `admit_invoke` `macos_bridge.rs:393-431`; `:776-795`; `reply_immediate_error` `:801-822` | WV-011 | **WV-011** |
| S4 | Host dispatch: undeclared channel | `app_session.rs:4213-4226` | WV-011 | **WV-011** |
| S5 | `begin_echo_call`: session quiescing | `app_session.rs:5813-5819` | WV-011 "unavailable or already pending" | **WV-011**, detail "application is quitting" |
| S6 | `begin_echo_call`: a lock is poisoned | `:5820-5823`, `:5831-5834` | WV-011 (same text) | **WV-011**, detail "host call state is unavailable" |
| S7 | `begin_echo_call`: session-wide slot busy | `:5835-5840` | WV-011 (same text) | **WV-011**, detail "another application call is already pending" |
| S8 | `begin_echo_call`: no admitted generation (`current` is `None`) | `:5824-5829` | WV-011 (same text) | **WV-012** |
| S9 | `write_frame` returns `Err` (any byte count, k ≥ 0) | `:5853-5876` | WV-011 (same text) | **WV-013** |
| DP | `write_frame` returns `Ok` | `:5878` | (none) | (none) |
| S10 | After DP: link EOF/reset, reader error, `Revoke` retirement, failed lifecycle or filesystem write, accepted Quit, profile handover or lease loss, router shutdown, reply channel disconnected | every retirement that answers the pending slot, e.g. `:6343`, `:6436`, `:6558`, `:6719`, `:7002`, `:7044`, `:7396`, `:7718`; all mapped generically at `:4278-4285` | WV-011 "application link ended before the renderer reply" | **WV-013** |
| S10b | After DP: Bun answers ERR with a `KELD-WV-*` code | forwarded at `:4272-4278` | that code, verbatim | **WV-013** (host rewrite, criterion 11) |
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
| Navigation | S3b (stale nonce, before dispatch) → WV-011; S16 → WV-013 |
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

Trust rule for codes from Bun. Every Bun ERR arrives after DP, so it can never
prove "not sent".

- The host rewrites any `KELD-WV-*` code from Bun to `KELD-WV-013` (S10b). This
  keeps WV-011 and WV-012 host-minted only.
- `KELD-API-002` is passed through. It is asserted by the app's own Bun process:
  Keld guarantees only `@keld/api`'s emission site (S11). App code that builds
  its own ERR frames can claim API-002 with the same trust as its handler. No
  host-side proof is possible, and none is claimed.

### 4.3 Backward compatibility

- **Still `KELD-WV-011`:** S1, S2, S3b and S4–S7. Every one of these returns
  before the host attempts a write. The registry meaning "rejected before
  application dispatch" becomes true again of every outcome the renderer can
  see.
  - `WvError::RendererBridge` (Rust side) also uses `KELD-WV-011` for
    bridge-install and reply-encode failures (`macos_bridge.rs:511-618`, `:843`).
    Those are construction or host faults, not call outcomes, and this
    revision leaves them unchanged.
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
  part of the contract; only the code prefix is. No rule in this spec, the
  KEL-142 note or the registry branches on a detail string.
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
  text stop recommending a document reload. The `Display` pin at
  `crates/keld-wv/src/error.rs:196` changes with it.
- **Interim mismatch (blocker owned by T3).** T0 already removes the reload
  advice from the registry `fix`. `error.rs:109-110` still says "Reload the
  current main-frame document" until T3 lands, because changing it is
  production code.

### 4.4 Retry rules (normative for apps; Keld itself never resends)

**Default:** a code the app does not recognize means Outcome Unknown.

| Code | Rule |
|---|---|
| WV-011 | **Never resend automatically**, whatever the detail. A new invoke made from a later user action is a new request, not a resend. The detail helps a developer diagnose the refusal; code MUST NOT branch on it. Until the §4.7 document-scoped result lands (T3), WV-011 proves "never sent" only for the document that issued the call (F10). |
| WV-012 | **One resend** after the first (startup) or successor Bun generation is ready (see the rules below). |
| WV-013 | **Never resend automatically**, unless it is made safe in one of the two ways below. |
| API-002 | **No app code ran.** Fix the registration order so the error cannot recur, then the page MAY resend once. |
| API-001 | Same as WV-013: the call may have run. |

WV-012 resend rules:

- The resend is a new `invoke`, with a new local request id and a new host
  correlation. It is never a replay of the old frame.
- The trigger is the trusted restart notification after a restart (separate
  spec). Startup has no Keld readiness signal, and neither does a restart
  until that notification ships. In those cases the only allowed trigger is a
  later user action or an app-level signal. Never use a timer or a backoff
  loop.
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

- T0 adds an interim note to GH-527 at each such passage.
- The `transport.ts` comment is production source, so T0 leaves it unchanged.
  The follow-up is recorded under T4: correct that comment to match the AT-B3
  verdict.

### 4.6 Platform notes

- **macOS:** the only renderer bridge (`#[cfg(target_os = "macos")]` on
  `begin_echo_call`, `pending_echo` and `start_renderer_dispatch`). Every
  renderer criterion is macOS-only.
- **Windows and Linux:** no renderer bridge. The `@keld/api` (S11–S14) and
  `@keld/kipc` (§4.5) rules apply on all three OSes.
- **A future Windows named-pipe Echo path:** it MUST re-prove F1 and F3
  (overlapped `WriteFile` can complete despite a cancel) before it emits WV-012.

### 4.7 Exactly-once settlement and no replay

- Three layers hold at most one pending entry and empty it **before** they
  deliver a terminal. A second terminal finds the slot empty and is dropped.
  The three layers are:
  - page facade `pending`, matched on request id (`macos_bridge.rs:49-53`);
  - native `BridgeState.pending`, matched on `(navigation, request)`
    (`:425-436`);
  - host `pending_echo`, matched on `(attempt, correlation)`.
- **Today**, the isolated world's `inflight` flag is not a settlement guard.
  `__keldBridgeNativeResult` (`:137-147`) clears it and relays every result
  without checking the request id. Duplicates within one document are dropped
  by the native layer and the facade, but a result can cross documents (F10).
- **Settlement fix (T3), document-scoped results:**
  - The isolated world records the in-flight request id, not a boolean.
  - Every native→isolated result carries `document`. The field is a new
    `document: Option<String>` on the private `PageOutcome` struct
    (`macos_bridge.rs:653`), serialized into the JSON passed to
    `__keldBridgeNativeResult`. It is filled in one of three ways:
    - **Immediate refusals** (`reply_immediate_error`, `:801-822`) copy it from
      the `document` field of the envelope being refused. An S3b stale nonce
      therefore carries the old nonce, and the successor drops it.
    - **Dispatch outcomes.** `PendingRequest` (keld-wv `BridgeState`) gains
      `document: String`, set by `admit_invoke` (`:393-431`) to the nonce the
      request was admitted under. `BridgeState::settle` (`:425`) returns that
      nonce (`Option<String>`, not `bool`) when it removes the matching
      `(navigation, request)` entry. `deliver` (`:633`) writes it into
      `PageOutcome.document`.
    - The host round trip is unchanged. `RendererBridgeRequest` and
      `RendererBridgeOutcome` still identify the call by `(webview,
      navigation, request)`, and keld-core never sees the nonce.
  - `__keldBridgeNativeResult` settles only when `document` equals the current
    `documentNonce` and `request` equals the in-flight id. Otherwise it drops
    the result and leaves `inflight` unchanged.
  - The relay to the page world never includes `document`.
  - The nonce already lives in the isolated world, so echoing it back exposes
    nothing new. The page world cannot read it, which keeps KEL-142 §5.1.
- **The resulting gap:** a stale invoke whose document is gone gets no
  terminal. That is correct, because its own `pagehide` already settled it
  with WV-013 (S16).
- **Bridge reset without a commit (host `navigate()` whose load never
  commits):**
  - **When the reset happens:**
    - In wry 0.56.1, `Started` fires only when a navigation commits
      (wry `src/wkwebview/navigation.rs:17-25`, `did_commit_navigation`), so a navigation the page cancels never resets
      the bridge.
    - The only reset that comes before a commit is the eager
      `navigation_started()` in host `navigate()` (`wkwebview/mod.rs:687-690`).
      It runs before `load_html`/`load_url`.
    - Main has no navigation-policy handler.
  - **Today:** that reset (`BridgeState::navigation_started`, `:357-361`)
    clears the native `pending` entry. The host outcome that arrives later is
    then dropped by `settle`. If the load never commits (for example a URL
    that never responds, or a load that fails before it commits), the original
    document stays live and its call never settles. That breaks criterion 7.
  - **Fix (T3):** `navigation_started` returns the `PendingRequest` it clears.
    The bridge wrapper (`:443`) then delivers one `PageOutcome { request,
    document, ok: false, code: "KELD-WV-013", detail: "renderer navigation
    started before reply" }` into the WebView's isolated world. The code is
    WV-013 because the call may already have been dispatched.
  - **If the navigation commits,** the result reaches either the old document,
    which settles once (its later `pagehide` finds `pending` empty), or the
    successor, which drops it on the `document` check.
  - **If the load never commits,** the original document settles once with
    WV-013. The later host outcome finds no native entry and is dropped, as
    today.
  - `destroy()` needs nothing, because the document dies with the view.
  - **Not covered:** the uncommitted document keeps a stale nonce, so its later
    invokes settle once with WV-011 (S3b) until a new navigation binds. Making
    it rebind is a usability matter for the navigation-boundary work, not a
    settlement gap.
  - T1 pins today's hang (AT-S arm d); T3 flips it.
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
  - in keld-wv (all private):
    - `PendingRequest.document: String`;
    - `PageOutcome.document: Option<String>`;
    - `BridgeState::settle` returns `Option<String>`;
    - `BridgeState::navigation_started` returns `Option<PendingRequest>`;
  - no public Rust type changes. `RendererBridgeRequest` and
    `RendererBridgeOutcome` are unchanged.
- **TypeScript:** `dispatchApplicationCall` throws a typed no-handler error,
  and `resolveEchoCall` maps it to `KELD-API-002`. Every other throw still maps
  to `KELD-API-001`.
- **Capabilities and manifest:** none.
- **Wire and protocol:**
  - KIPC: no frame, field or version change. New *values* travel in existing
    string fields, and the host now rewrites one value class (S10b).
  - `keld.wv-link/v1`: the private native→isolated result gains one field,
    `document` (§4.7). The isolated→native envelope is unchanged. See §8.
- **Runtime seam:** none. The execution owner, crash domain and ordering are
  unchanged.
- **Persisted state:** none.

## 5. Boundaries

- **Implement in:**
  - `crates/keld-core/src/app_session.rs` (`begin_echo_call` refusal type,
    renderer dispatch mapping) and its tests;
  - `crates/keld-wv/src/wkwebview/macos_bridge.rs` (`pagehide` code,
    document-scoped result) and `crates/keld-wv/src/error.rs` (`fix` text);
  - `packages/@keld/api/src/{channels,echo-call}.ts` and their tests;
  - `docs/engineering/keld-error-codes.md`;
  - the KEL-142 spec §7;
  - the host macOS acceptance tests named in §7.
- **Must not touch:**
  - KIPC frame layout, `CallError` schema, the isolated→native
    `keld.wv-link/v1` envelope;
  - `@keld/kipc` transport behaviour;
  - guard, permissions, workspace `Cargo.toml`;
  - navigation policy;
  - restart-notification code.

## 6. Tasks (each ≈ one PR; ordered)

- [ ] T0: docs only. It includes:
  - this spec;
  - the KEL-142 §7 current-behaviour correction;
  - the interim `KELD-WV-011` registry text;
  - the GH-527 interim notes.
- [ ] T1: test-only characterisation. It pins today's codes at S8–S16 and
  S10b with negative controls, writing the AT-12a/12d/13a/13b/13c arms against
  today's strings. It also pins the cross-document settlement today (AT-S arm
  c) and the hang after a host `navigate()` that never commits (AT-S arm d).
- [ ] T2: keld-core. It includes:
  - the typed refusal;
  - the S8 → WV-012, S9/S10 → WV-013 and S10b rewrite;
  - the WV-012/WV-013 registry headings;
  - T1's assertions flipped to the target codes.
- [ ] T3: keld-wv. It includes:
  - `pagehide` → WV-013;
  - the document-scoped result, and settlement after an uncommitted host
    `navigate()` (§4.7), which flip T1's AT-S arms c and d;
  - the `WvError::RendererBridge` fix text;
  - the narrowed WV-011 registry text.
- [ ] T4: `@keld/api` and `@keld/kipc`. It includes:
  - S11 → API-002, its registry heading, and the unit and hosted API-002 tests;
  - the follow-up fix to the `transport.ts:2571-2572` comment, and to the GH-527
    text, matching the AT-B3 verdict.
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
| AT-12b | 1 | S8 | macOS unit (keld-core) | deterministic | WV-012 variant returned; the counting writer saw **0 write calls** | admitted generation → 1 write call and no WV-012; **own mutation:** move the `current is None` refusal after `write_frame` → the counting writer sees 1 call and AT-12b fails; cross-check: map `WriteFailed` to WV-012 → AT-13a fails |
| AT-12d | 1, 8 | S8 at startup (G1 held before HELLO, page already loaded) | hosted macOS | 10 | page invokes before the first admission → `KELD-WV-012`, census 0; after G1 Ready, one resend → reply, census 1; `KIPC_CALL` = admits − 1 | G1 released before the invoke → reply, census 1 (the startup gap is load-bearing) |
| AT-12c | 8 | S8 then resend | hosted macOS | 10 | after host Ready for G2, one resend → reply; census for the logical action = 1 | resend before G2 is admitted → WV-012 again (readiness is load-bearing) |
| AT-13a | 2 | S9, k ∈ {0, 1, HEADER_LEN−1, HEADER_LEN, HEADER_LEN+len−1} | macOS unit (keld-core; extends `failed_primary_writes_retire_the_generation`) | deterministic per k | WV-013; slot cleared; generation retired; 0 bytes written to the stream after the failure | k = full length → DP reached, no S9; characterization: an injected completing write after failure makes the Bun `FrameReader` dispatch (proves why S9 cannot be WV-012 yet) |
| AT-13b | 3, 7 | S10 | hosted macOS | 10 per arm | arms: (i) SIGSTOP Bun → invoke → SIGKILL: census 0; (ii) kill at `entered`: census 1; (iii) kill after `effect-committed`: effect present. Every arm: `KELD-WV-013`, 1 terminal, G2 census for the nonce 0 forever | arm (ii) census 1 under the same code proves WV-013 cannot mean "not sent"; mutation: S10 → WV-012 fails arm (ii) |
| AT-13c | 4 | S16 | hosted macOS | 10 | handler held at `entered`; page `location.reload()`; the `pagehide` rejection beacon reports `KELD-WV-013`; census 1; no settlement in the successor document | navigate while queued before bind → census 0 with the same WV-013 (the facade cannot tell) |
| AT-13d | 7 | S15 | hosted macOS | 50 | kill Bun on the `returning` POST; outcome ∈ {reply, WV-013}; never WV-012; 1 terminal; the success:Unknown ratio is reported, not asserted | **kill disabled**, same harness → 50/50 reply and 0 WV-013 (the Unknown outcomes come from the kill, not from the harness); mutation: S15 retirement → WV-012 fails |
| AT-API2a | 5 | S11–S14 | `bun test` in `@keld/api`, macOS + Linux + Windows CI | deterministic | missing → API-002, census 0; late (call, then register, then call) → API-002 then reply, census 1; unsubscribed → API-002; handler throws after effect → API-001, census 1; decode failure → API-001, census 0; oversize → API-001, census 1 | mutation: move the no-handler check after `handler(request)` → the census for the API-002 case becomes 1 and the test fails; a throwing handler never yields API-002 |
| AT-API2b | 5, 8 | S11 across restart | hosted macOS | 10 × arms {inside `whenReady`, before `whenReady`} | inside: at least one post-restart API-002 with G2 census 0, then one resend → census 1; before: 0 API-002 across all N | the inside arm reproduces the window (round-1 E8 hit it 10/10 under API-001) |
| AT-11 | 6 | S1, S2, S3b, S4–S7 (and S3a: no terminal) | macOS unit (keld-wv + keld-core) | deterministic | each reason → WV-011 and 0 write calls; each S3a reason → no terminal and no dispatch | mutation: emit WV-011 from S9 → fails |
| AT-B1 | 9 | `#throwIfTerminal` | `@keld/kipc` unit, 3 OSes | deterministic | link already ended → `call()` rejects IPC-022; host fixture read 0 frames | remove `#throwIfTerminal` → frame posted (the check is the proof, but the code stays Unknown) |
| AT-B2 | 9 | pending at close | `@keld/kipc` unit, 3 OSes | deterministic | pending, then host close → IPC-022; host read 1 frame; same code as AT-B1 | host writes the REPLY before closing → the call resolves with the reply, not IPC-022 (the close produces 022, not the harness) |
| AT-B3 | 9 | written before Quit, pending at close | keld-core unit | deterministic | host census for that CALL recorded; decides Open question 3 | written after Quit → IPC-024, census 0 (existing `guarded_router_and_quit.rs`) |
| AT-S | 7, 12 | settlement | macOS unit (keld-wv, keld-core) + hosted macOS | deterministic (unit); 10 (hosted) | (a) late reply after retirement refused; (b) facade result after `pagehide` ignored, and native result for a stale `(navigation, request)` dropped; (c) a stale-document invoke refused after the successor binds, while the successor has a pending call with the same request id → the successor's call settles only from its own result; (d) handler held at `entered`; the test calls host `navigate()` (`WebEngine::navigate`) to a URL whose load never commits (the observer accepts the connection and never responds); then the handler is released → the **original document** receives exactly one settlement, `KELD-WV-013` (counted by a page-side beacon that carries the original document's own nonce); census 1; the later host outcome is dropped. T1 pins today's behaviour (c may cross; d: 0 settlements); T3 flips both to the target | (a) remove the corr/attempt check → second settlement observed; (c) remove the `document` check in `__keldBridgeNativeResult` → the successor settles with the stale WV-011; (d) the same host `navigate()` to a URL that loads and commits → the original document settles once with WV-013 (by `pagehide` or the delivered result, never both), and the successor document records 0 settlements |
| AT-SP | 11 | S10b | macOS unit (keld-core; fake Bun peer on `socketpair`) | deterministic | Bun ERR `code = "KELD-WV-012"` (and `"KELD-WV-011"`) → renderer outcome `KELD-WV-013`; the frame was written (`KIPC_CALL` emitted) | Bun ERR `code = "KELD-API-002"` → forwarded unchanged (the rewrite is scoped to `KELD-WV-*`); mutation: remove the rewrite → the page sees `KELD-WV-012` and the test fails |

Anti-flake:

- No sleeps. Holds and releases use the existing test-only observer gates.
- Kills are driven from census POSTs, not timers.
- Hosted arms use the PR #1082 fixtures, promoted into landing tests by T1.

## 8. Review gates triggered

| Gate | Applies | Reason |
|---|---|---|
| Public API | **Yes** | three new page- and app-visible codes; narrowed meanings for `KELD-WV-011` and `KELD-API-001`; normative retry guidance; KEL-142 §7 revised |
| Wire protocol | **Yes** (values, plus one private field; no version bump) | KIPC: `KELD-API-002` is a new value crossing the Bun→host link in `CallError.code`, and the host now rewrites `KELD-WV-*` values arriving there (S10b). The frame and the `CallError` schema are unchanged, and receivers treat an unknown code as Unknown, so no version bump. `keld.wv-link/v1`: the native→isolated result gains `document` (§4.7). Both ends ship in one keld-wv build, and the isolated→native envelope and its decoder are unchanged, so no version bump; the reviewer confirms this |
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

Cross-document settlement, formerly Open question 6, is now in scope: see F10,
§4.7, criterion 12 and AT-S arm c.
