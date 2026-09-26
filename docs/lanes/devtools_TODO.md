# devtools lane decision map

Owns `crates/amazing-devtools/` (new, std-only). The lane implements the Chrome
DevTools Protocol server; the driver adopts the engine seam in a later
integrator-assist ticket.

## CDP-1 — std-only CDP server (bulk) — done

Brief: a CDP server over WebSocket (RFC 6455 server side, minimal HTTP
`/json/version` and `/json/list`) with the domains `Target`, `Page`
(navigate, reload, lifecycle events, captureScreenshot hook), `Runtime`
(evaluate, callFunctionOn, getProperties, consoleAPICalled), `DOM`
(getDocument, querySelector, getOuterHTML, describeNode), `CSS`
(getComputedStyleForNode), `Network` (requestWillBeSent, responseReceived,
loadingFinished, via a hook trait) and `Log`; engine-facing traits defined in
this crate; scripted CDP client tests.

Source `bcc3679a`; base `c6817825` (`origin/dev`).

### Shape and layering

- `json` is a dependency-free DOM/serializer with ordered object members,
  bounded parse depth/size and stable integer rendering.
- `ws` is the RFC 6455 message layer: masked client frames, fragmentation with
  interleaved control frames, ping/pong, close codes, UTF-8 validation, and
  frame/message limits. It carries both the server role and a client-role
  masking path the tests use.
- `http` reads one HTTP/1.1 head with a header cap and writes fixed responses;
  `server` upgrades to WebSocket only after validating
  `Sec-WebSocket-Version: 13` and a base64 16-byte `Sec-WebSocket-Key`.
- `protocol` frames JSON-RPC requests/responses/events with the CDP error codes
  (`-32700` parse, `-32600`/`-32601`/`-32602`, `-32000` server).
- `session` owns per-connection state: the attached `DevToolsSession`, the
  CDP node-id map and the enabled-domain set; events are filtered by enabled
  domains, matching Chrome.

### Engine seam (the contract the driver implements)

- `DevToolsHost`: `info()` (product/user-agent/revision/JS version/protocol
  version), `list_targets()`, `attach(target_id) -> Box<dyn DevToolsSession>`.
- `DevToolsSession`: one attached page target. Every method has a default
  returning `HostError::Unsupported` (surfaced as `-32601`), so domains are
  adopted one at a time. Page, Runtime, DOM, CSS and the event `drain_*`
  families are all here. `drain_*` must not block; the connection's own thread
  polls after every command and on socket idle.
- Identity rule: CDP `nodeId` is a dense per-session integer assigned by this
  crate; `backendNodeId` is the engine's stable `EngineNodeId(u64)`, so the
  driver never mints ids.
- `mock::MockHost` is a public in-memory host over a fixed DOM and scriptable
  event queues for tests and early driver integration.

### Server behaviour

- `CdpServer::bind` on an arbitrary address; one thread per connection;
  connection cap; non-blocking accept loop with a shutdown flag.
- `/json/version` and `/json/list` advertise absolute `ws://` URLs built from
  the request `Host` authority (falling back to the local address).
- `/devtools/page/<id>` binds directly to one session; `/devtools/browser`
  supports `Target.getTargets`, `Target.attachToTarget` (flattened) and
  `Browser.getVersion`. Page-domain commands without a session on the browser
  endpoint are refused.
- Page commands answer on the default session; flattened sessions tag every
  response and event with `sessionId`.

### Coverage and gaps

Implemented domains: `Target` (getTargets, getTargetInfo, attachToTarget,
activateTarget, closeTarget, setDiscoverTargets, getBrowserContexts,
setAutoAttach), `Browser` (getVersion, getBrowserCommandLine), `Page` (enable,
disable, navigate, reload, getFrameTree, lifecycleEvent, frameNavigated,
captureScreenshot), `Runtime` (enable, disable, evaluate, callFunctionOn,
getProperties, releaseObject, releaseObjectGroup, runIfWaitingForDebugger,
consoleAPICalled, executionContextCreated), `DOM` (getDocument,
querySelector, querySelectorAll, getOuterHTML, describeNode,
setAttributeValue, resolveNode), `CSS` (getComputedStyleForNode,
getMatchedStylesForNode), `Network` (enable, events), `Log` (enable, events).

Not implemented (future tickets, no conformance credit implied): `Fetch`
interception (`Fetch.requestPaused`/`continueRequest`), `Debugger`/`Profiler`/
`HeapProfiler`/`Runtime` call frames and `getHeapUsage`, `Input`, `Emulation`,
`Tracing`/`Performance`, `DOMSnapshot`, `DOM.setFileInputFiles`,
`Animation`, `Overlay`, `Security`, `ServiceWorker`, `Page.printToPDF`,
`Browser` events (`Target.targetCreated`/`targetDestroyed`/`attachedToTarget`
push), `Target.setAutoAttach` child targeting, and server-side
`permessage-deflate`. Multi-process renderer target routing is deferred until
the ADR 0005 path is available to the driver.

- [x] JSON, base64, SHA-1, WebSocket (RFC 6455), HTTP head and discovery.
- [x] Protocol framing, per-session node ids, enabled-domain event filtering.
- [x] Target/Page/Runtime/DOM/CSS/Network/Log domains behind the seam.
- [x] MockHost and a scripted CDP client that speaks JSON-RPC over a real
  WebSocket (discovery, page round trip across every domain, flattened browser
  sessions, error reporting).
- [x] Linux crate gate and full Linux workspace gate.
- [ ] CDP-2 candidates: `Fetch` interception + `Debugger`, push
  target-lifecycle events, `Page.printToPDF`. Driver adoption (integrator-assist
  `runtime_host_devtools.rs`) is a separate ticket.
