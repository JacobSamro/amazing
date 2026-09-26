# IPC-11 — Web Locks cross-tab operations and cross-document messaging

Commits: `0196bdb0` (browser message router + `MessagingService(15)`),
`9744fd00` (driver bindings). Base: dev `f5f97c39` / origin/dev `d6ee28b3`.
Pinned WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

## What landed

**Transport (`amazing-process/src/messaging.rs`).** A browser-owned
`MessageRouter`: a BroadcastChannel registry keyed by (origin, top-level site,
name) with FIFO delivery and the sender excluded; entangled MessagePort pairs
with bounded FIFO queues, ports paused until `start()`, transfer and close; and
`disconnect` cleanup that closes peer ports. `MessagingService(15)` carries
subscribe/unsubscribe/broadcast/create_ports/start_port/post_port/close_port/
disconnect as bounded wire requests; `MessagingEndpoint` owns one router. The
`ipc-fuzz` decoder and dispatcher sets now cover every messaging message.

**Driver (`crates/amazing/src/runtime_host_messaging.rs`).** A lane-owned
`BroadcastChannel`, `MessageChannel`, `MessagePort`, `MessageEvent` and
`Window.postMessage`/`onmessage`/`onmessageerror` surface. Payloads are
JSS-SC `SerializedValue` graphs encoded by the versioned history wire codec.
A thread-local router is shared by every document on the thread; a postMessage
computes deliveries and enqueues them into the target realm's inbox, which the
document drains at its scheduler checkpoint (the granted `schedule_messages`
hook next to storage). The shared-file footprint is one `NativeEntry` variant
+ arm, one installer call, one `BindingState` field, and one scheduler line.

## Linux gates (venus-001, exact source `9744fd00`)

| Command | Result |
| --- | --- |
| `cargo nextest run -p amazing -p amazing-process` (merged head `cad48c43`) | 257/257, 7 skipped |
| `cargo nextest run -p amazing -p amazing-process` (merged head `f330dcf8`) | 256/256, 7 skipped |
| `cargo nextest run -p amazing -p amazing-process` (messaging `9744fd00`) | 249/249, 7 skipped |
| `cargo nextest run -p amazing` | 191/191, 3 skipped |
| `cargo nextest run -p amazing-process` | 58/58, 3 skipped |
| driver messaging tests | 3/3 (`MessageChannel` structured clone, `Window.postMessage`, `BroadcastChannel` sender exclusion) |
| storage lock-seam tests | 2/2 (FIFO grant/queue, `if_available`, `steal`, query, release-on-disconnect) |

Fuzz (`ipc-fuzz`, 90 s each; 58 decoder targets and 12 dispatcher targets now
include every messaging message): decoder **8,382,072** cases, dispatcher
**11,972,412** cases, zero panics.

## WPT `webmessaging/` selection
95 `.html` files under `webmessaging/`, run through `amazing wpt-harness`
against the shared sparse checkout on venus-001 (now carrying `webmessaging/`).
Raw per-file JSON: [ipc-11-webmessaging/](ipc-11-webmessaging/).

| Status | Files |
| --- | --- |
| pass | 27 |
| fail | 29 |
| error | 28 |
| timeout | 11 |

The passing set is the single-document `window.postMessage` family
(`with-options_*`, most `without-ports_*` / `with-ports_*`), which exercises
`postMessage`, structured clone and `onmessage`. The non-passing set is
dominated by cases outside this ticket's reach today: iframes/cross-origin
(the harness pins `window.parent = window`, and no child JS realm exists),
Workers/service workers, `blobs/` and `CryptoKey`, `.https.`/`.sub.`
secure-context variants, and the multi-global `current`/`incumbent` harness
features. `BroadcastChannel` cross-document cases (`ordering`,
`cross-origin`, `detached-iframe`) error for the same iframe/realm reason.

Before this ticket the API surface did not exist at all, so the same selection
was error-only; the 27 passes are new.

## Web Locks

Process seam `f330dcf8`: `StorageService(13)` ops 5-8 (`lock_request`,
`lock_release`, `lock_release_client`, `lock_query`) over one browser-owned
`LockCoordinator` in the storage process, with wire tests for FIFO
grant/queue, `if_available`, `steal`, query and release-on-disconnect. Raw
JSON: [ipc-11-web-locks-merged/](ipc-11-web-locks-merged/).

`web-locks/` (38 `.any.js`/`.https.any.js` files) through the harness:

| Tree | pass | fail | error | timeout |
| --- | --- | --- | --- | --- |
| before STORE-5 (no `navigator.locks`) | 0 | 19 | 19 | 0 |
| merged head `f330dcf8` | 1 | 15 | 21 | 1 |

The single pass is `lock-attributes.https.any.js` (2 subtests), from storage's
in-process `navigator.locks` binding. Storage's binding still owns the
coordinator in-process; IPC's process seam and protocol are ready for storage
to reroute through it (then the browser calls `lock_release_client` on
renderer disconnect/crash). The `.https.` secure-context tests and the
steal/if-available/abort/signal families remain engine and binding work.

## Cross-lane

- dom-bindings owns the real `MessageEvent` class and making the ports
  `EventTarget`s; IPC's classes are stubs until then.
- Storage owns `amazing_storage::locks` and the `navigator.locks` adapter; IPC
  owns the storage-process coordinator seam.

## Windows gate

`scripts/win-run.sh ipc -- 'cargo nextest run --workspace --locked --no-fail-fast; cargo test --workspace --locked --doc'`
on win-001 (win-002 offline), gate source `2ae31e97` (code `f330dcf8`), base
`838361c5`:

- workspace nextest: **2503/2503 passed**, 9 skipped, **0 failures**, exit 0.
- workspace doctests: **2/2 passed** (`amazing-html`, `amazing-js`).

An earlier attempt failed during extraction with `No space left on device`;
after the orchestrator purged caches the same gate passed. The final Windows
run predates the last dev merge to `cad48c43` (siblings only; the lane's own
crates are unchanged), whose Linux crate gate is 257/257.

## IPC-11d — messaging conformance hardening (code `4aa97770`)

Merged with `dev` at `aef9d5be` (`Linux-first: Windows gates deferred`,
local dev `8ba1ca55` + IA-REAL-PAGES) so the workspace gate is current.
Windows gates are deferred; the gate below is Linux on the lane's Boat
sandbox.

**Landed.** BroadcastChannel router clients are per channel object rather than
per document, so two channels on one name in one document receive each other
and the dead realm name→slot map is gone. `Window.postMessage` accepts the
`WindowPostMessageOptions` form, validates `targetOrigin` (URL parse failure),
rejects a null transfer and a missing message. `MessagePort` is bound to its
global constructor (`instanceof`), starts implicitly when `onmessage` is set,
and shares one start path. `onmessage`/`onmessageerror` default to `null`.

**Linux workspace gate** (Boat sandbox, `--ref aef9d5be`):

| Command | Result |
| --- | --- |
| `cargo nextest run --workspace --locked --no-fail-fast` | **2530/2530 passed**, 10 skipped, exit 0 |
| `cargo test --workspace --locked --doc` | **2/2** passed, exit 0 |

**WPT `webmessaging/`** (95 `.html`, pinned `c2a2fce6`) through
`amazing wpt-harness`, before `9744fd00` → after `4aa97770`:

| Level | pass | fail | timeout | error |
| --- | --- | --- | --- | --- |
| files before | 27 | 29 | 11 | 28 |
| files after | 33 | 24 | 10 | 28 |
| subtests before | 51 | 54 | 0 | 0 |
| subtests after | 58 | 48 | 0 | 0 |

Zero regressions. Fixed files: `with-options/null-transfer`,
`{with,without}-ports/015` (different origin), `{with,without}-ports/023`,
`without-ports/009` (zero arguments). Raw JSON:
[ipc-11d-webmessaging/](ipc-11d-webmessaging/). The remaining in-lane
conformance gaps are logged as cross-lane requests in `questions.md` (Date
`valueOf`/`getTime`, DOMException, element event-handler content attributes,
`__defineGetter__`), plus an `@@iterator` helper request.

## IPC-11e — transfer ports, sequence conversion, bounded drain (code `53884517`)

Base `aef9d5be`; original gate ran on a pooled Boat sandbox at merged head
`7eb81e02`, **before per-lane `CARGO_TARGET_DIR`s landed** (`86c44031`), so
that run could have linked another lane's stale artifacts. Re-gated below on
the corrected per-lane target dir.

**Landed.** `Window.postMessage` delivers the wrapped transfer ports on the
`MessageEvent` instead of an always-empty array. Transfer lists use WebIDL
`sequence<object>` conversion (`Array.isArray`): a primitive or a plain
array-like object is a `TypeError`. `messaging_drain` delivers only the
messages queued when the checkpoint began, so a handler that posts again cannot
starve timers or the event loop.

**Re-gate (2026-09-26), merged head `1c481bf0`**, Boat `pool-sys` sandbox
`bx_cxad649k`, per-lane `CARGO_TARGET_DIR=/home/user/amazing/targets/ipc`:

| Command | Result |
| --- | --- |
| `cargo nextest run --workspace --locked --no-fail-fast` | **2806/2806 passed**, 10 skipped, exit 0 |
| `cargo test --workspace --locked --doc` | all doctests passed, exit 0 |

Confirmed green on the corrected per-lane target dir; state set to done with
this SHA (see IPC-11f below for the two extra tests that bring the workspace
total to 2808 after that ticket).

**WPT `webmessaging/`** (95 `.html`, `c2a2fce6`), `9744fd00` → `4aa97770`
(IPC-11d) → `53884517` (IPC-11e):

| Level | pass | fail | timeout | error |
| --- | --- | --- | --- | --- |
| files before | 27 | 29 | 11 | 28 |
| files after 11d | 33 | 24 | 10 | 28 |
| files after 11e | 38 | 19 | 10 | 28 |
| subtests before | 51 | 54 | 0 | 0 |
| subtests after 11d | 58 | 48 | 0 | 0 |
| subtests after 11e | 64 | 42 | 0 | 0 |

11e fixed `MessageEvent/onmessage/postMessage` infinite loop, `with-options/
message-channel-transferable`, `with-ports/025` (`1 as ports`), `/026`
(`{length:1}`), `/027` (both subtests). Zero regressions. Raw JSON:
[ipc-11e-webmessaging/](ipc-11e-webmessaging/). Cumulative 11d+11e: files
27→38 pass, subtests 51→64 pass.

Re-measured on the pinned selection at the IPC-11f head (below) with a
committed runner (`docs/lanes/ipc-webmessaging-run.py`, since the pooled
sandbox's WPT sparse-checkout had gone stale under concurrent lane use — see
IPC-11f): files 38 pass / 19 fail / 13 timeout / 25 error (same 38 passes, same
64 passing subtests; the 3-file shift among fail/timeout/error tracks
`messageerror.html`'s harness path, which now runs to completion instead of
one file previously misclassified). Zero lost passes either way.

## IPC-11f — malformed messaging payload fires `messageerror`, structured-clone decode fuzzing

**Landed.** `messaging_deliver` no longer propagates `Throw` when a peer's
wire payload fails `state_wire::decode` or `structured_deserialize`; per HTML,
a malformed message now fires `messageerror` at the target instead of
aborting the scheduler's checkpoint turn (a cross-process peer can send
arbitrary bytes, so one bad message must not stop every other queued message
or timer). `messaging_dispatch`/`messaging_dispatch_error` share the
port/channel/window delivery paths, parameterised by event `kind`
(`"message"` vs `"messageerror"`). New test:
`malformed_payload_fires_messageerror_not_message` (`runtime_host_messaging.rs`),
using a `#[cfg(test)]` `RuntimeBindings::queue_raw_message` hook and a matching
`DriverScheduler::test_bindings()` accessor (`js_scheduler.rs`, both
test-only).

**Structured-clone wire fuzzing.** `state_wire::decode` (the `SerializedValue`
wire codec `navigator.rs` uses for both `history.state` and messaging
payloads) had no fuzz coverage of its own — `ipc-fuzz` lives in
`amazing-process` and cannot depend on this driver-owned module. Added two
tests in `navigator.rs`: a deterministic xorshift mutation fuzzer seeded from
five valid encodings (empty root, a self-referencing cycle, a BigInt-keyed
`Map`, a typed-array view, degenerate empty/garbage seeds) run for 100,000
mutated cases, plus 2,000 pure-random byte strings of increasing length up to
2,000 bytes. Both assert `decode` returns `Ok`/`Err` and never panics.

**Linux workspace gate**, same sandbox/target dir as the IPC-11e re-gate,
merged head `1c481bf0` plus this commit:

| Command | Result |
| --- | --- |
| `cargo nextest run --workspace --locked --no-fail-fast` | **2808/2808 passed**, 10 skipped, exit 0 |
| `cargo test --workspace --locked --doc` | all doctests passed, exit 0 |

Both new tests run in well under a second (`structured_clone_decode_never_panics_under_mutation`,
`structured_clone_decode_never_panics_on_random_bytes`); zero panics, zero
regressions elsewhere.

WPT `webmessaging/` re-measured at this head with `docs/lanes/ipc-webmessaging-run.py`
(no pre-filtering; every file is handed to `amazing wpt-harness` and classified
by its own verdict): **38 pass / 19 fail / 13 timeout / 25 error** of 95, same
38 file passes and 64 subtest passes as IPC-11e. Raw per-file JSON on the
sandbox under `~/amazing/out/ipc/webmessaging/` (not copied back — ephemeral
sandbox scratch per AGENTS.md's disk-hygiene note; the counts above are the
durable evidence).

**Cross-lane note.** The pooled Boat sandbox's `~/corpora/wpt` sparse
checkout lost `common/` (56 files reduced to 3, 3,477 lines of `git status`
deletions) partway through this session, almost certainly another lane's
concurrent WPT run rewriting the sparse-checkout list on the same `pool-sys`
sandbox (the documented drift). Recovered with `git sparse-checkout reapply &&
git checkout -- .`; logged nothing further since a working fix was in hand,
but any lane hitting inexplicable `os error 2` / "resource escapes root" on a
pooled sandbox should check `git -C ~/corpora/wpt status --short` first.
