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
