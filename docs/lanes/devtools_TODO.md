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

Source `783a1464`. Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 80/80
nextest (0 skipped) and 1 doctest; workspace 2749 passed, 0 failed, 10 skipped,
doctests 4 passed.

`Overlay.enable`/`disable`, `setInspectMode`, `highlightNode`,
`highlightRect`, `highlightQuad`, `hideHighlight` and the `setShow*` toggles.
New seam types `Rgba` and `HighlightConfig`; `highlight_*` default to
`Unsupported` while the enable/mode/hide methods are advisory. The `setShow*`
flags are validated and accepted as engine hints.

## CDP-7 — Performance and Security domains — done

Source `20fe9331`. Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 83/83
nextest (0 skipped) and 1 doctest; workspace 2761 passed, 0 failed, 10 skipped,
doctests 4 passed.

- `Performance`: `enable`/`disable`/`setTimeDomain` and `getMetrics` over
  `DevToolsSession::performance_metrics`.
- `Security`: `enable`/`disable` (enable emits the current state),
  `setIgnoreCertificateErrors`, `handleCertificateError`,
  `setOverrideCertificateErrors`, and `Security.securityStateChanged` changes
  drained from the engine. New seam type `SecurityState` and
  `security_state`/`drain_security_states`/`set_ignore_certificate_errors`/
  `handle_certificate_error`.

## CDP-8 — Runtime depth — done

Linux gate on Boat `bx_xbhqabax` (per-lane target), measured together with
CDP-9 in the same bulk pass: `amazing-devtools` 89/89 nextest (0 skipped) and
1 doctest; workspace 2808 passed, 0 failed, 10 skipped, doctests 4 passed.

`Runtime.compileScript`, `runScript`, `getIsolateId`, `getHeapUsage`,
`globalLexicalScopeNames`, `queryObjects`, `setAsyncCallStackDepth` and
`discardConsoleEntries`. New seam type `HeapUsage` plus `compile_script`,
`run_script`, `get_isolate_id`, `get_heap_usage`, `global_lexical_scope_names`,
`query_objects`, `set_async_call_stack_depth` and `discard_console_entries`.

## CDP-9 — Accessibility domain — done

Linux gate on Boat `bx_xbhqabax` (per-lane target), measured together with
CDP-8 in the same bulk pass: `amazing-devtools` 89/89 nextest (0 skipped) and
1 doctest; workspace 2808 passed, 0 failed, 10 skipped, doctests 4 passed.

`Accessibility.enable`/`disable`, `getFullAXTree`, `getPartialAXTree`,
`getChildAXNodes` and `queryAXTree`. New seam type `AxNode` (CDP
`Accessibility.AXNode` shape: `ax_node_id`, `ignored`, `role`, `name`,
`description`, `value`, the originating `EngineNodeId` and `child_ids`) plus
`accessibility_enabled`/`get_full_ax_tree`/`get_partial_ax_tree`/
`get_child_ax_nodes`/`query_ax_tree`. `getPartialAXTree` and `queryAXTree`
resolve `nodeId`/`backendNodeId` through the same node-identity path as `DOM`
and `Overlay`.

This crate does not depend on `amazing-a11y`; the seam is CDP-shaped only, so
the driver converts its own `amazing_a11y::AxTree` (role, computed name/
description, DOM node, children) into `AxNode`s when it adopts this trait —
same pattern as every other domain here.

## CDP-10 — Debugger domain — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 95/95
nextest (0 skipped) and 1 doctest; workspace 2871 passed, 0 failed, 10
skipped, doctests 4 passed.

`Debugger.enable`/`disable`, `setBreakpointByUrl`, `removeBreakpoint`,
`pause`/`resume` (emits `Debugger.resumed`), `stepInto`/`stepOver`/`stepOut`,
`evaluateOnCallFrame` (reuses `Runtime`'s `evaluate_result` serializer) and
`getScriptSource`. New seam types `ScriptLocation`, `ScriptInfo`, `CallFrame`
and `PausedEvent`, kept to the fields a front end needs to show a stack and
evaluate in a frame — Chrome's full `CallFrame`/`Scope` schema carries more
optional detail (a scope chain, `this`, generator state) the driver can add
later without breaking this seam. `Debugger.scriptParsed`/`Debugger.paused`
drain like every other domain's push events (`drain_scripts_parsed`/
`drain_debugger_paused`), gated by a new `EnabledDomains::debugger` flag.

The seam trait methods for the whole CDP-10..CDP-15 batch (`Debugger`,
`Profiler`, `HeapProfiler`, `DOMSnapshot`, `Storage`/`DOMStorage`/
`IndexedDB`, `Tracing`, `Accessibility.nodesUpdated`) were designed and added
to `host.rs` together in this ticket, each defaulting to
`HostError::Unsupported` or an advisory no-op like every other domain here,
so later tickets implement their domain layer against an already-stable
seam. Recorded the `Debugger`/`Profiler`/`HeapProfiler` seam for the JS
engine in `.agents/js-runtime/questions.md` (the engine implements
`DevToolsSession` for these methods, not this crate).

## CDP-11 — Profiler and HeapProfiler domains — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools` 99/99
nextest (0 skipped) and 1 doctest; workspace 2875 passed, 0 failed, 10
skipped, doctests 4 passed.

`Profiler.enable`/`disable`, `setSamplingInterval`, `start`, `stop` (returns
`Profiler.Profile`, serialized from the `CpuProfile`/`CpuProfileNode` seam
defined in CDP-10). `HeapProfiler.enable`/`disable`, `takeHeapSnapshot`
(streams `HeapProfiler.addHeapSnapshotChunk` per chunk the engine returns,
then one `reportHeapSnapshotProgress {finished:true}`), `collectGarbage`,
`getHeapObjectId`, `getObjectByHeapObjectId`. No new seam types beyond
CDP-10's; both domains are advisory-enable with no continuous push events
(no `EnabledDomains` flag added). `MockHost` scripts a one-node CPU profile,
a two-chunk heap snapshot and a heap-object round trip. Real profiling and
heap data need the JS engine (`.agents/js-runtime/questions.md`, CDP-10).

## CDP-12 — DOMSnapshot domain — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools`
101/101 nextest (0 skipped) and 1 doctest; workspace 2877 passed, 0 failed,
10 skipped, doctests 4 passed.

`DOMSnapshot.captureSnapshot`. The `DomSnapshotDocument`/`DomSnapshotNode`
seam (CDP-10 batch) stays in plain strings and builds `children` top-down,
the natural order for a tree walk; this domain performs the wire format's
two CDP-specific transforms when serializing: string interning (a
deduplicating `StringTable`, referenced by index from `documentURL`/
`baseURL`/`nodeName`/`nodeValue`/`attributes`) and the parent-index
inversion (`nodes.parentIndex`, computed by inverting each node's
`children`). `MockHost` scripts a 4-node document over the same fixed
`html`/`body`/`div#main`/text fixture other domains use.

## CDP-13 — Storage, DOMStorage and IndexedDB domains — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools`
108/108 nextest (0 skipped) and 1 doctest; workspace 2884 passed, 0 failed,
10 skipped, doctests 4 passed.

Three real CDP domains, grouped under one ticket per the brief's `Storage
(cookies/localStorage/IndexedDB list/clear via seam)` scope:

- `Storage`: `getCookies`/`setCookies`/`clearCookies`. New seam type
  `Cookie` and `get_cookies`/`set_cookies`/`clear_cookies`.
- `DOMStorage`: `enable`/`disable`, `getDOMStorageItems`,
  `setDOMStorageItem`, `removeDOMStorageItem`, `clear`. New seam type
  `DomStorageId` (security origin plus local-vs-session flag) and
  `get_dom_storage_items`/`set_dom_storage_item`/`remove_dom_storage_item`/
  `clear_dom_storage`. New `EnabledDomains::dom_storage` flag (tracked for
  parity with real CDP; not yet gating any push event).
- `IndexedDB`: `enable`/`disable`, `requestDatabaseNames`,
  `clearObjectStore`, `deleteDatabase` — the ticket's `list`/`clear` scope;
  `requestDatabase`/`requestData` row-level browsing is a later ticket. New
  `EnabledDomains::indexed_db` flag (same parity purpose).

`MockHost` scripts a cookie jar, a `localStorage` map keyed by security
origin, and an IndexedDB database-name map keyed by security origin.

## CDP-14 — Tracing domain — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools`
112/112 nextest (0 skipped) and 1 doctest; workspace 2888 passed, 0 failed,
10 skipped, doctests 4 passed.

`Tracing.getCategories`, `start` (parses the comma-separated `categories`
string CDP actually sends, not an array), `end`. `Tracing.end` collects
synchronously: it calls `stop_tracing` then drains whatever the engine
buffered via the CDP-10 batch's `drain_trace_events() -> Vec<Json>` (each
entry a free-form Trace Event Format object, so no new struct type — the
format is engine-defined, not this crate's to shape), emitting at most one
`Tracing.dataCollected` (skipped when nothing was collected) followed
always by one `Tracing.tracingComplete`. No `Tracing.enable`/`disable` in
real CDP either, so no `EnabledDomains` flag. `MockHost` scripts two
categories and one trace event.

## CDP-15 — Accessibility.nodesUpdated push events — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing-devtools`
113/113 nextest (0 skipped) and 1 doctest; workspace 2889 passed, 0 failed,
10 skipped, doctests 4 passed.

`Accessibility.nodesUpdated`, drained like every other domain's push events
(`DevToolsSession::drain_ax_nodes_updated`, added to the CDP-10 batch),
gated by the `EnabledDomains::accessibility` flag CDP-9 already added — no
new flag needed since `Accessibility.enable` already sets it. Closes the
one gap CDP-9's report noted. `MockHost` gains `MockScript::push_ax_update`
alongside its other `push_*` scripting helpers.

This closes out the CDP-10..15 boundary batch from the brief (Debugger,
Profiler/HeapProfiler, DOMSnapshot, Storage/DOMStorage/IndexedDB, Tracing,
Accessibility push events). Remaining candidates for a future ticket:
`Fetch`'s `permessage-deflate`, `IndexedDB.requestDatabase`/`requestData`
row-level browsing, `Tracing`'s streamed (rather than synchronous) chunking,
`Animation`, `ServiceWorker`, `Page.printToPDF`, and multi-process renderer
target routing.
