# IPC-11c follow-up: real JS realm for cross-process postMessage/MessagePort

Base: local `dev` at `5b923bf7` (after IPC-11e/11f merged at `da9508f3`).
Commit: `41b0bdb4`.

## What landed

`ipc-demo-child` (the renderer test-double binary) now hosts a real
`amazing_js::Interpreter`, opt-in via the `__js_realm` capability instead of
the production byte-frame renderer path (`crates/amazing-process/src/bin/ipc-demo-child.rs`).
The realm runs on one dedicated worker thread (`amazing_process::demo_realm`),
bridged from the IPC dispatch threads (each incoming call gets its own thread
per `amazing_ipc::Connection`) via an `mpsc::channel<Job>` with a per-call
reply channel, since `Interpreter` is `Rc`-based and not `Send`.

A new IPC interface, `DemoPage` (number 20), lets a test drive the realm:
`eval(source) -> String`, `deliver_window_message(payload)`,
`deliver_port_message(PortMessage { port, payload })`,
`take_window_outbox(()) -> Vec<ByteBuf>`, `take_port_outbox(()) -> Vec<PortMessage>`.
The realm installs `postMessage`/`portPostMessage` globals and a
`portHandlers` object via `set_host_native_dispatcher`; outbound calls queue
into an outbox the test drains and hands to the "browser" side.

`structured_clone_wire.rs` duplicates `amazing::navigator::state_wire`'s
encode/decode of `amazing_js::SerializedValue`, since `amazing-process`
cannot depend on `amazing` (wrong dependency direction) and cannot `impl`
the foreign `codec::Wire` trait for the foreign `SerializedValue` type
(orphan rule). Both copies must move in lock-step if the node/atom set
changes.

## Bug found and fixed

`MessagingEndpoint::post_port` (`crates/amazing-process/src/messaging.rs`)
computed the port-transfer target as `router.owner_of(request.port)` — but
`post()` requires the caller to already own `request.port`, so this always
resolved to the *sender's own* client id. The bundled `transfer` list in
`PortPost` was therefore a silent no-op self-transfer instead of handing
ports to the message recipient. Fixed by adding `MessageRouter::peer_of()`
and computing the target as `peer_of(request.port).and_then(|peer| owner_of(peer))`.
Regression test:
`post_port_transfers_bundled_ports_to_the_message_recipient_not_the_sender`.

## End-to-end proof

`crates/amazing-process/tests/demo_realm_two_process.rs`, two real spawned
OS processes (`ProcessHost::spawn` with `CARGO_BIN_EXE_ipc-demo-child`):

- `two_real_processes_exchange_window_messages_both_ways`: renderer2's page
  runs `postMessage({from: 'renderer2', n: 1})`; the payload is drained from
  its outbox and delivered into renderer1's realm, which runs a real
  structured-clone deserialize and reads `e.data.from`/`e.data.n` back out.
  Then the same round trip renderer1 -> renderer2.
- `a_messageport_transferred_across_processes_stays_live`: a real
  `MessageRouter` (standing in for the browser process) creates a port pair
  owned by client 1, then `transfer()`s the far end to client 2 — the port
  crosses the process boundary here, exactly as a real browser process would
  mediate a `MessageChannel` handoff. Renderer2's page never created the
  port; it only learns the id and installs a handler
  (`portHandlers['<id>'] = ...`). Renderer1 posts through `portPostMessage`,
  the outbox is drained, `router.post()` looks up the entangled peer's
  *current* owner (renderer2, post-transfer) and the delivery is routed
  there — proving the fixed `post_port` semantics end to end, not just in
  the unit test.

No js-runtime gap was found: `HostNativeDispatch`, `structured_serialize`/
`deserialize`, and the low-level `binding_*` embedding API already exposed
everything the realm needed. No `questions.md` entry was required.

## Gate

Linux workspace gate, Boat sandbox (`bx_cxad649k`, pool-sys, kernel 7.0 pool
at the time of this run — see IPC-12 below for the kernel-6.8 EPERM finding
that surfaced immediately after):

| Command | Result |
| --- | --- |
| `cargo nextest run -p amazing-process --locked` | 65/65 passed, 3 skipped |
| `cargo test -p amazing-process --locked --doc` | 0 doctests (crate has none), ok |
| `cargo nextest run --workspace --locked --no-fail-fast` | **2901/2901 passed**, 10 skipped, exit 0 |
| `cargo test --workspace --locked --doc` | all doctests ok (amazing-html 1, amazing-js 1, all other crates 0) |

`Cargo.lock` gained one additive edge (`amazing-process` -> `amazing-js`),
verified byte-identical to a lockfile a real `cargo build` produced before
being applied by hand (the sandbox rsync in `linux-run.sh` overwrites any
lockfile changed only on the sandbox, so the fix had to land in the
worktree's own `Cargo.lock`).
