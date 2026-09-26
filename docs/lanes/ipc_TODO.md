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

Move the keyed lock coordinator into the storage process and expose it over IPC.
Wires `amazing_storage::locks::LockCoordinator` (storage lane, committed
`6e190b07`, **not yet in dev** — see questions).

- Protocol: `LockMode`, `LockOptions`, `LockRequest`, `LockOutcome`,
  `LockQuery`, `LockSnapshot`; `StorageService` ops `lock_request`,
  `lock_release`, `lock_release_client`, `lock_query` (ids 5-8).
- Storage process owns one `LockCoordinator`, keyed by browser-minted
  partition + client id; FIFO pending queue; grant notifications.
- Release on renderer disconnect and crash (`release_client`), bounded queues.
- Browser-side `LockSession` proxy in the driver adapter.
- Tests: process-level grant/queue/steal/if_available/disconnect/crash, plus
  fuzz targets. Gate WPT `web-locks/` once storage's `navigator.locks` binding
  is in dev.
- Blocked on `amazing_storage::locks` in dev; IPC will swap the import when it
  lands.

### 11b. Cross-document/process messaging

- `amazing-process/src/messaging.rs`: browser-owned `MessageRouter`
  - BroadcastChannel registry keyed by (partition, name), FIFO delivery,
    sender excluded, bounded subscribers/messages.
  - MessagePort entanglement pairs with FIFO queues, ports start paused until
    `start()`, transfer and close, bounded queues.
  - Client disconnect/crash cleanup; bounded totals.
- Wire: `MessagingService(15)` (browser<->renderer), structured-clone payloads
  as bounded byte graphs (`SerializedValue` wire), port transfer ids.
- Driver: a shared browser-level router so documents in one browser (and the
  WPT harness's page + iframes) share BroadcastChannel/MessagePort state.
- WebIDL (lane-owned adapter, minimal shared hook lines): `BroadcastChannel`,
  `MessageChannel`, `MessagePort`, `MessageEvent`, `Window.postMessage`,
  `onmessage`/`onmessageerror`, `addEventListener('message')`.
- Structured clone through `SerializedValue`; driver `StructuredCloneHost` for
  host-transferable values where required.
- Tests: router unit tests, driver JS tests (`postMessage`, entangled ports,
  broadcast to two documents), WPT `webmessaging/` before/after.

### 11c. Follow-ups

- Cross-process JS endpoint: the `ipc-demo-child` renderer is a navigation
  token stub and runs no page JS, so real renderer-to-renderer delivery needs a
  JS realm in the renderer child (separate ticket).
- MessageEvent conformance (`instanceof MessageEvent`, `source`/`ports`
  semantics) needs `EventClass::Message` in `amazing-dom`; IPC raises it as a
  cross-lane request if the lane-owned prototype proves insufficient for WPT.
