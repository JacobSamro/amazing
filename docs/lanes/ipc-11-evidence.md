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
| `cargo nextest run -p amazing -p amazing-process` (final, `9744fd00`) | 249/249, 7 skipped |
| `cargo nextest run -p amazing` | 191/191, 3 skipped |
| `cargo nextest run -p amazing-process` | 56/56, 3 skipped |
| driver messaging tests | 3/3 (`MessageChannel` structured clone, `Window.postMessage`, `BroadcastChannel` sender exclusion) |

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

38 `web-locks/` files (`.any.js`/`.https.any.js`) through the same harness:
**0 pass, 19 fail, 19 error** — `navigator`/`navigator.locks` does not exist
yet, so the tests stop before lock arbitration. Raw JSON:
[ipc-11-web-locks/](ipc-11-web-locks/).

The process seam is not yet wired: `amazing_storage::locks::LockCoordinator`
(storage commit `6e190b07`) is not in dev, so the `StorageService` lock
operations cannot compile. When storage merges its coordinator and
`navigator.locks` adapter, IPC will own one coordinator in the storage process,
expose request/release/release_client/query, notify the requesting renderer
when a queued id is granted, and release clients on disconnect/crash. See
`questions.md`.

## Cross-lane

- dom-bindings owns the real `MessageEvent` class and making the ports
  `EventTarget`s; IPC's classes are stubs until then.
- Storage owns `amazing_storage::locks` and the `navigator.locks` adapter; IPC
  owns the storage-process coordinator seam.
