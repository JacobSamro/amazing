# Pending native resolution and connection

Recorded source base: `5efe03ecd6ebf0cbeacca0a4dc24aa55a06f8f58`.
This ticket adds an opt-in producer to the existing Dispatcher/NetworkService.
The shell actor and font loader installation remain product-owned dependencies.
Source-only until the whole native/workspace/docs/CSP paired gate is terminal.

## Contract

`Dispatcher::new_pending_io(limits)` and
`Dispatcher::with_tls_policy_pending_io(limits, policy)` are fallible constructors.
`Dispatcher::with_io_pending(limits, resolver, connector)` preserves custom
provider compatibility: default trait methods call the existing synchronous
methods. A custom provider must override `begin_resolve`/`begin_connect` to obtain
pending behavior. Non-Send provider state is supported; only a submitted native
callback crosses a worker boundary. `ResolveStep`/`ConnectStep` distinguish Ready,
Pending, and Busy. The dispatcher supplies private service/request/generation
contexts; operations are move-only and validated on delivery.

The standard opt-in uses the existing ToSocketAddrs/connect_timeout operations,
with four resolution and four connection workers shared per process. At most64
queued/running/ready-unread operations are physically owned across dispatchers.
Busy waits in the existing request state; ordinary request/per-origin/deadline
limits still apply. Queued cancellation removes actual queued work. Running
cancellation suppresses delivery immediately but retains its operation allocation
until the provider returns. Late real sockets close without sending any HTTP or
WebSocket request bytes. Recreating an actor cannot recover slots still owned by
running callbacks. No claim of physical interruption of std OS DNS/connect is made.
`pending_io_snapshot()` observes process-wide provider counters, not per-actor
memory or proof that a particular syscall is active. A running callback can be
waiting in a provider fixture before/after a genuine native operation.

Resolved addresses retain original order/fallback. Connect execution recalculates
actual remaining time before invoking std connect_timeout. No TLS stream is
adopted/preauthenticated: the original PendingTls verification, ALPN, per-instance
trust policy, bounded encrypted budgets and output drainage remain on the actor.
The default normal roots are unchanged. Explicit test fixture CA trust still
performs the ordinary handshake/chain/hostname/time/signature verification.

`NetworkService::new_pending_io` and `with_tls_policy_pending_io` configure that
same dispatcher while retaining existing Fetch, CORS, cookies/cache/Reporting
state. The existing font service can adopt this constructor directly.

## Native controls and installed boundary

Four slow controlled resolution callbacks coexist with a real replacement HTTP
request completing on the same actor. Abort/supersede removes each original
request once. Real connected TCP streams held before delivery exercise HTTP/WS
cancel, timeout and dispatcher destruction, retained running allocation, and peer
EOF without request bytes. HTTP/HTTPS/WS/WSS success uses real requests and genuine
fixture TLS handshakes; default roots reject the fixture in HTTPS and WSS. Ordered
address fallback preserves Host/origin and non-Send synchronous resolver state.
Separate isolated worker controls cover64-operation saturation, queued removal,
cancelled-running ownership, late/unread sockets, wrong owner/request/generation/
HTTP-vs-WS delivery, panic/error completion, replay, expired callback admission and reentrant queued
provider disposal. Queued callbacks and discarded sockets are destroyed outside
the pool mutex; physical ownership remains until disposal finishes. The late-socket counter is
published only after actual socket disposal, and never implies peer acknowledgement.
Real font-service peers exercise CORS allow/deny, response-size limits and abort
before any socket admission. Existing original controls remain unchanged.

Controlled delays are provider gates around genuine std operations. They do not
prove an interrupted OS lookup or blocked SYN. Product still needs to install and
qualify the opt-in actor/font constructors, its real per-tab stop/supersede/close
controls, and UI progress. No installed browser responsiveness claim is made here.
No source was copied from another engine; no new dependency or second stack.

Qualification complete on `71503026e615aa534dec49cca63b52b8c22b6b42` against recorded `5efe03ecd6ebf0cbeacca0a4dc24aa55a06f8f58`: actual BEFORE/AFTER/controller/compact-fetch0. Workspace5263→5274, docs38→38, native835→846, CSP33files/82subtests unchanged, every paired lost list empty. All11 new grouped controls pass.1111 ELF files per side, source/generated/raw retained native/tmp; AFTER manifest SHAf46b328c9eb08fc30b90ab10a3f3444268ccec9661f063dc30fad18d879c2469. Original32/81 CSP subset remains retained. Failed preallocation21 and compiler100 evidence remains explicit; incomplete native/workspace execution is UNMEASURED. Root merged-current workspace/full16 and product installation remain separate.
