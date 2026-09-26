# IPC lane TODO

Crate ownership: `amazing-ipc`, `amazing-process` (ADR 0005), plus the lane's
driver adapter files under `crates/amazing/src/` and, by IPC-11 grant, the
WebIDL surface for BroadcastChannel/MessageChannel/MessagePort/postMessage.

## Done

- IPC-1..IPC-6: IPC framing/versioning/limits, renderer+network broker,
  demand-driven shared frames (ADR 0007), sandbox policy (ADR 0008), network
  driver adoption (ADR 0009).
- IPC-7: compositor frame snapshot wire + GPU parity.
- IPC-8: sandboxed utility decoder process (image/audio/font), storage service
  process, ADR 0012 draft.
- IPC-9: browser-owned session history and bfcache lifecycle (`history.rs`).
- IPC-10: default-off per-tab Navigator + process history adapter (ADOPTED into
  dev `0dd4b272`). WPT `html/browsers/history` on/off parity exact.

## IPC-11 (active) — Web Locks cross-tab operations and messaging

### 11a. Web Locks cross-tab operations (StorageService)

Landed `f330dcf8` (process seam). Storage independently landed an in-process
`navigator.locks` binding on dev `838361c5`.

- Protocol: `LockMode`, `LockOptions`, `LockRequest`, `LockOutcome`,
  `LockRelease`, `LockClient`, `LockQuery`, `LockInfo`, `LockSnapshot`;
  `StorageService` ops 5-8. **Done.**
- Storage process owns one `LockCoordinator`; FIFO pending queue; grant
  results returned to the browser. **Done**; grant *notifications* to a
  renderer are the browser adapter's follow-up when storage reroutes its
  in-process coordinator through this seam.
- Release on disconnect/crash via `lock_release_client`. **Done** (browser
  calls it; `runtime_host_storage.rs` currently releases in-process on Drop).
- Tests: process-level grant/queue/steal/if_available/query/release_client. Gate
  WPT `web-locks/` re-measured after the storage merge.
- Swap of the storage binding from the in-process coordinator to this process
  proxy is storage-owned; IPC's protocol and endpoint are ready.

### 11b. Cross-document/process messaging

Landed `0196bdb0` (transport) and `9744fd00` (driver bindings):

- `amazing-process/src/messaging.rs`: browser-owned `MessageRouter`
  - BroadcastChannel registry keyed by (partition, name), FIFO delivery,
    sender excluded, bounded subscribers/messages. **Done.**
  - MessagePort entanglement pairs with FIFO queues, ports start paused until
    `start()`, transfer and close, bounded queues. **Done.**
  - Client disconnect/crash cleanup; bounded totals. **Done.**
- Wire: `MessagingService(15)` (browser<->renderer), structured-clone payloads
  as bounded byte graphs (`SerializedValue` wire), port transfer ids. **Done.**
- Driver: thread-local browser router shared across documents; each realm
  drains its inbox at the scheduler checkpoint. **Done**
  (`runtime_host_messaging.rs`, minimal shared hook in `runtime_binding_host.rs`
  and `js_scheduler.rs`).
- WebIDL (lane-owned): `BroadcastChannel`, `MessageChannel`, `MessagePort`,
  `MessageEvent`, `Window.postMessage`, `onmessage`/`onmessageerror`,
  `addEventListener('message')`. **Done as stubs**; dom-bindings will replace
  `MessageEvent` and make the ports real `EventTarget`s.
- Linux: `amazing-process` 56/56 (3 skipped); `amazing` 191/191 (3 skipped,
  including 3 messaging driver tests). WPT `webmessaging/` gate in progress.

### 11d. WPT conformance hardening (`4aa97770`, merged `aef9d5be`)

- Per-channel BroadcastChannel router clients (two channels on one name in one
  document receive each other); dead realm name→slot map removed.
- `Window.postMessage` options form, targetOrigin URL validation, null-transfer
  and missing-message TypeErrors.
- `MessagePort` global `instanceof`, implicit start on `onmessage`, shared start
  path; `onmessage`/`onmessageerror` default `null`.
- WPT `webmessaging/`: files 27→33 pass, subtests 51→58 pass, zero
  regressions. Linux workspace 2530/2530 + 2/2 doctests.
- Evidence `docs/lanes/ipc-11-evidence.md`; raw
  `docs/lanes/ipc-11d-webmessaging/`. Remaining gaps are cross-lane
  (`questions.md`).

### 11e. Transfer ports, sequence conversion, bounded drain (`53884517`, head `7eb81e02`)

- `Window.postMessage` delivers the wrapped transfer ports on the message event.
- Transfer lists use WebIDL `sequence<object>` conversion: primitive/array-like
  transfer is a TypeError.
- Message delivery is bounded per checkpoint so a self-posting handler cannot
  starve the event loop.
- WPT `webmessaging/`: files 33→38 pass (from 27 before 11d), subtests 58→64
  pass; zero regressions. Linux workspace 2704/2704 + 4/4 doctests.
- Raw `docs/lanes/ipc-11e-webmessaging/`; cumulative table in
  `docs/lanes/ipc-11-evidence.md`.

### 11c. Follow-ups

- Cross-process JS endpoint: the `ipc-demo-child` renderer is a navigation
  token stub and runs no page JS, so real renderer-to-renderer delivery needs a
  JS realm in the renderer child (separate ticket).
- MessageEvent conformance (`instanceof MessageEvent`, `source`/`ports`
  semantics) needs `EventClass::Message` in `amazing-dom`; IPC raises it as a
  cross-lane request if the lane-owned prototype proves insufficient for WPT.
