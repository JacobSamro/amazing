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
- [x] CDP-2 candidates: `Fetch` interception + `Debugger`, push
  target-lifecycle events, `Page.printToPDF`. Driver adoption (integrator-assist
  `runtime_host_devtools.rs`) is a separate ticket.

## CDP-2 — inspector read paths and target lifecycle push — done

Source commit `4519210c`; base `dev` `0c117657` merged as `f1de15ad`. Linux gate on
Boat `bx_xbhqabax`: `amazing-devtools` 69/69 nextest (0 skipped) and 1 doctest;
workspace 2641 passed, 0 failed, 10 skipped, doctests 3 passed.

Adds the read paths a DevTools front end uses after connect, plus the
`Target.*` push events, all behind the same seam (every new method defaults to
`HostError::Unsupported`):

- DOM: `getBoxModel` (`BoxModel`), `getContentQuads`, `getAttributes` (already
  present), `performSearch`/`getSearchResults`/`discardSearchResults` with a
  per-session search cache, `pushNodesByBackendIdsToFrontend`, `setNodeValue`.
- CSS: `getStyleSheetText`/`setStyleSheetText`, `getMediaQueries`,
  `getBackgroundColors`.
- Page: `getLayoutMetrics` (`LayoutMetrics`/`Viewport`),
  `getResourceTree` (`FrameResourceTree`/`ResourceInfo`), `getResourceContent`
  (text or base64), and `Page.domContentEventFired`/`loadEventFired` builders.
- Target: the host can queue `TargetEvent`s; the browser endpoint emits
  `Target.targetCreated`/`targetInfoChanged`/`targetDestroyed` after
  `Target.setDiscoverTargets(true)` and auto-attaches a flattened session
  (`Target.attachedToTarget`, then `Target.detachedFromTarget`) when
  `Target.setAutoAttach(true)` is set on a new target.

`mock::MockHost` implements all of it over the fixed document and gains
scriptable target events (`push_target_created`/`push_target_destroyed`).

## CDP-3 — Input domain and DOM hit testing — done

Source `f4c33c86`. Linux gate on Boat `bx_xbhqabax`: `amazing-devtools` 72/72
nextest (0 skipped) and 1 doctest; workspace 2706 passed, 0 failed, 10 skipped,
doctests 4 passed.

- `Input`: `setIgnoreInputEvents`, `dispatchMouseEvent`, `dispatchKeyEvent`,
  `dispatchTouchEvent`, `insertText`. New seam types `MouseEvent`, `KeyEvent`,
  `TouchPoint`/`TouchEvent` and the matching `dispatch_*`/`insert_text`/
  `set_ignore_input_events` methods.
- `DOM.getNodeForLocation`: hit-tests the document via
  `DevToolsSession::node_for_location` and returns the node id, backend node id
  and frame id.
- `MockHost` records every input event and returns a configurable `hit_test`.

## CDP-4 — Fetch interception and Network response bodies — done

Source `11e58a00`. Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 74/74
nextest (0 skipped) and 1 doctest; workspace 2736 passed, 0 failed, 10 skipped,
doctests 4 passed.

- `Network.getResponseBody` (text or base64) and `Network.getRequestPostData`.
- `Fetch` domain: `enable` (with `RequestPattern` parsing), `disable`,
  `continueRequest`, `failRequest`, `fulfillRequest` (base64 body) and
  `getResponseBody`; `Fetch.requestPaused` events drained from the engine.
- Seam: `FetchPattern`, `InterceptedRequest`, `FetchContinue`/`FetchFulfill`/
  `FetchDecision`, and `set_fetch_enabled`/`drain_intercepted_requests`/
  `resolve_intercepted_request` plus the two body methods.

## CDP-5 — Emulation domain — done

Source `85e6f5c9`. Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 77/77
nextest (0 skipped) and 1 doctest; workspace 2746 passed, 0 failed, 10 skipped,
doctests 4 passed.

`Emulation.setDeviceMetricsOverride`/`clearDeviceMetricsOverride`,
`setUserAgentOverride`, `setGeolocationOverride`/`clearGeolocationOverride`,
`setTimezoneOverride`, `setLocaleOverride`, `setEmulatedMedia`,
`setScrollbarsHidden`, `setTouchEmulationEnabled`, `setCPUThrottlingRate` and
`setScriptExecutionDisabled`. New seam types `DeviceMetrics` and `MediaFeature`;
the seam methods are advisory and default to success. New protocol helpers
`u64_param`/`bool_param`.

## CDP-6 — Overlay domain — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 80/80
nextest (0 skipped) and 1 doctest; workspace 2749 passed, 0 failed, 10 skipped,
doctests 4 passed.

`Overlay.enable`/`disable`, `setInspectMode`, `highlightNode`,
`highlightRect`, `highlightQuad`, `hideHighlight` and the `setShow*` toggles.
New seam types `Rgba` and `HighlightConfig`; `highlight_*` default to
`Unsupported` while the enable/mode/hide methods are advisory. The `setShow*`
flags are validated and accepted as engine hints.
