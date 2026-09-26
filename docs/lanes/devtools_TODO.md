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

## CDP-16 — real `DevToolsSession` host, single-threaded page owner — done

Linux gate on Boat `bx_xbhqabax` (per-lane target): `amazing` crate nextest
259/259 (0 skipped) and 1 doctest; workspace nextest 3023 passed, 0 failed,
12 skipped, doctests 4 passed.

integrator-assist landed the first driver adoption directly (`crates/amazing/
src/runtime_host_devtools.rs`, granted for this ticket like a11y's A11Y-6,
merged to `dev` at `4bab124d`/`2e8f60c9`): `DriverDevToolsHost`/
`DriverPageSession` over a real fetched page, `Page.navigate`/
`getFrameTree`/`getLayoutMetrics`, `Runtime.evaluate`, and CDP discovery on a
background thread via `amazing devtools <url> [listen-addr]`. Its own doc
comment named the gap: `evaluate` built a **fresh JS realm per call**, because
`amazing-js`'s `Rc`-based heap can't be `Send` while `DevToolsSession: Send`
requires the session to be usable from whichever thread a CDP connection
spawns. DOM mutations survived (plain owned data moved through a throwaway
scheduler each call); `globalThis` state, closures and anything else living
in the JS heap did not.

This ticket is the file's per its grant ("the file is yours from now on");
fixed the gap with a dedicated single-threaded page owner rather than any
attempt to make the engine `Send`:

- `PageOwner` holds the live `Rc<RefCell<BrowserDomHost>>` DOM and one
  persistent `DriverScheduler` (one JS realm, kept alive for the page's whole
  lifetime) and never leaves the thread it is built on.
- `spawn_page_thread` starts that thread. Its closure captures a `PageInit`
  (`Parsed { dom: amazing_dom::Dom, .. }` for tests, or `Fetch { url, .. }`
  for the real CLI path) — plain `Send` data — and calls
  `PageOwner::from_parsed`/`fetch_and_load` **inside** the spawned closure, so
  the non-`Send` `PageOwner` is constructed on its own thread and never
  crosses the `std::thread::spawn` boundary that requires the closure itself
  to be `Send`. A one-shot ready channel reports the initial load's success
  (with the resolved URL) or failure back to the caller synchronously, so a
  bad URL fails `spawn_devtools_server`/`DriverDevToolsHost::spawn` the same
  way it did before.
- `DriverDevToolsHost` (needs `Send + Sync`) and `DriverPageSession` (needs
  `Send`) hold nothing but a `Mutex<Sender<PageCommand>>`/`Sender<PageCommand>`
  and an `Arc<Mutex<String>>` for the display URL — no engine state, so both
  trivially satisfy their bounds. Every `DevToolsSession` method sends a
  `PageCommand` (its own one-shot reply channel) and blocks for the answer;
  the page thread's `run_page_thread` loop is the only code that ever calls
  into `PageOwner`. `DriverDevToolsHost::drop` sends `PageCommand::Shutdown`,
  so the page thread never outlives its host.
- Wired up on that one live page/realm: `Runtime.evaluate` (now the *same*
  scheduler across calls), `Page.navigate` (reloads `PageOwner` in place, on
  the page thread, no new thread spawned), `DOM.getDocument` (recursive
  `NodeDescription` builder over the live arena DOM, depth-limited per the
  domain layer's existing `-1`→64 convention) and `DOM.querySelector`/
  `querySelectorAll` (single selector via `amazing_css::selectors::parse_selector`/
  `matches`, no comma-list support yet) and `Accessibility.getFullAXTree`
  (style+layout via the same minimal `FallbackTextSystem`/`NoReplacedElements`
  path `amazing axtree` uses, then `runtime_host_a11y::build_ax_tree` per the
  ticket, converted to the CDP-shaped `host::AxNode` seam).
- Proof: `evaluate_persists_full_js_realm_state_across_calls` increments a
  `globalThis` counter across two `evaluate()` calls and asserts `2`, not `1`
  twice — the exact regression the fresh-realm-per-call bug would produce.
  `end_to_end_cdp_client_evaluates_queries_dom_and_reads_the_ax_tree` is a raw
  WebSocket CDP client (the same framing `amazing-devtools`' own
  `tests/cdp.rs` uses) driving `Runtime.evaluate`, `DOM.getDocument`,
  `DOM.querySelector`/`querySelectorAll` and `Accessibility.getFullAXTree`
  against the real page thread over a real socket.
- `navigate_fetches_a_real_page_and_evaluate_reads_the_new_dom` exercises the
  real `navigate()` fetch path against a one-shot local HTTP fixture server
  (everything on `127.0.0.1`), since `fetch_document` is HTTP(S)-only.

Known limitations, unchanged or extended from the previous report:
`Page.captureScreenshot` still `Unsupported` (no PNG/JPEG/WebP encoder, only
BMP); `DOM.describeNode`/`getOuterHTML`/`setAttributeValue` and the CSS/Input
domains stay on the crate default; `querySelector(All)` takes one selector,
no comma lists; `Accessibility.getFullAXTree`'s `depth` parameter is ignored
(whole tree always returned, matching the domain layer's own `-1`
convention); layout for the ax tree uses the same fallback text/no-replaced-
content path as `amazing axtree`, not the driver's real providers.

## CDP-17 — DOM mutation, CSS styles, Input dispatch, PNG screenshots — done

Closed most of CDP-16's boundary list, making the driver usable by a real
DevTools frontend rather than just `Runtime.evaluate`/basic DOM reads.

- Surveying `domains/dom.rs`/`css.rs`/`input.rs`/`page.rs` first showed almost
  every CDP method the ticket asked for was **already dispatch-wired** from
  earlier tickets (CDP-2..15), calling `DevToolsSession` methods that default
  to `HostError::Unsupported` — so this ticket was almost entirely a
  driver-side (`runtime_host_devtools.rs`) implementation task, not new
  domain wiring. The two exceptions, confirmed missing by grep: `DOM.setOuterHTML`
  and `DOM.removeNode` needed new `set_outer_html`/`remove_node` trait
  methods on `DevToolsSession` (`amazing-devtools/src/host.rs`) plus dispatch
  arms (`domains/dom.rs`).
- **DOM**: `describeNode`/`getOuterHTML` reuse `Dom::serialize_node` (the same
  serializer the JS `outerHTML` binding uses, per
  `amazing-bindings/src/markup_bindings.rs` — DevTools and scripted access
  now agree). `setAttributeValue` calls `Dom::set_attribute`/
  `remove_attribute`. `setOuterHTML` re-parses the replacement HTML with
  `amazing_html::parse_fragment` in the parent element's tag-name context
  (falling back to `"body"` when the parent isn't an element), clones the
  fragment into the live `Dom` with a new `clone_subtree_into` helper
  (mirroring `amazing-html`'s own private `clone_subtree` — there is no
  cross-`Dom` "adopt" primitive), then `detach`+`pre_insert`s the clone at
  the original position before detaching the original node. `removeNode` is
  a one-line `Dom::detach`. `querySelector`/`querySelectorAll` now accept
  comma-separated selector lists via a new `split_selector_list` (splits on
  top-level commas, tracking quote/paren/bracket depth so `:is(a, b)` and
  `[data-x="a,b"]` are not split).
- **CSS**: `getComputedStyleForNode` reads `StyledDom::get`/
  `ComputedStyle::iter` after computing style with the page's own author
  sheets. `getMatchedStylesForNode` walks those same `Stylesheet.rules`
  against `amazing_css::selectors::matches` (using each rule's pre-split
  `selector_srcs`, so no selector-list splitter was needed there), reporting
  every match as `StyleOrigin::Author` — there is no separately exposed
  user-agent/built-in sheet, a documented limitation rather than a silent
  gap. `getStyleSheetText` uses the 0-based index into the page's collected
  author sheets as an ad-hoc `styleSheetId` (this driver mints no other id
  for stylesheets) and reconstructs CSS text from parsed `selector_srcs`/
  `declarations` — an honest re-serialization, not the original source
  bytes, since the parser doesn't retain those.
- **Input**: `dispatchMouseEvent`/`dispatchKeyEvent` reach real
  `addEventListener` handlers via the existing `DriverScheduler::
  dispatch_dom_event` (the same call scripted event dispatch already uses,
  so DevTools-originated and script-originated events share one path).
  Mouse events resolve a target node through a **new** `hit_test`: a
  depth-first, topmost-child-wins point-in-box walk of the
  `amazing_layout::LayoutBox` tree returned by `layout_styled_document`
  (`Flow::Blocks`/`Flex`/`Grid` for normal-flow children, `LayoutBox::
  positioned` for absolutely/fixed-positioned descendants). No point-to-node
  helper existed anywhere in the workspace before this (confirmed by grep
  across `amazing-layout`, `crates/amazing`, `amazing-paint`); everything
  needed to write one was already public on `LayoutBox`, so no cross-lane
  seam to `layout` was needed. Key events target `Dom::active_element`,
  falling back to the document root when nothing is focused.
- **Page**: `captureScreenshot` renders through the existing
  `paint_host_surface` raster path (`amazing render`'s own function) and
  encodes PNG with `amazing_canvas::encode_png` — the ticket said to write a
  small encoder "if no encoder exists in the workspace yet"; one already did
  (`crates/amazing-canvas/src/png.rs`, a stored-DEFLATE zlib writer), so it
  was reused rather than duplicated. `clip` crops the rendered `Surface`
  before encoding; only `format: "png"` is supported (`jpeg`/`webp` stay
  `Unsupported` — no encoder for either in the workspace).
- **A real bug the tests caught**: `hit_test`'s first version checked a
  box's own point-in-box bounds *before* checking its `positioned` children,
  so a `position: absolute` descendant that extends past the shrink-to-fit
  box of the ancestor that hoisted it — the common case, e.g. an
  otherwise-empty `<body>` with one absolutely-positioned child taller than
  the body's own auto height — was unreachable by any click landing inside
  it but outside the ancestor's shrunk box. The end-to-end Input test failed
  with `HostError::NotFound` on exactly this shape before the fix
  (`positioned` descendants are now checked unconditionally, ahead of the
  box's own bounds check).
- Proof — four new end-to-end tests, each a real WebSocket CDP client (same
  framing as `amazing-devtools`'s own `tests/cdp.rs`) against the real page
  thread, never `MockHost`:
  `end_to_end_dom_mutation_and_selector_lists_work_through_the_real_websocket_client`
  (describeNode/getOuterHTML/setAttributeValue/setOuterHTML/removeNode plus a
  comma-separated `querySelectorAll`), `end_to_end_css_computed_matched_
  styles_and_stylesheet_text`, `end_to_end_input_dispatch_reaches_real_
  dom_event_listeners` (a real `addEventListener` handler observably runs,
  for both a hit-tested mouse event and a focused-element key event), and
  `end_to_end_capture_screenshot_returns_a_valid_png` (base64-decodes the
  response and checks the real PNG magic bytes).

Known limitations, unchanged or newly documented: `hit_test` does not
descend into `Flow::Inlines` line boxes, so a click on inline text resolves
to the nearest block/inline-container ancestor, not the exact inline run;
dispatched DOM events carry no coordinate/key/button detail properties (no
binding-host API at the `dispatch_dom_event` layer accepts a rich init
dict); `getMatchedStylesForNode` omits the user-agent sheet;
`getStyleSheetText`'s id scheme and reconstructed text are driver-side
approximations; layout for hit-testing (like the ax tree) still uses
`FallbackTextSystem`/`NoReplacedElements`, not the driver's real providers.

Linux gate (Boat `bx_xbhqabax`, per-lane target): `amazing` crate nextest
265/265 (0 skipped, +4 vs. CDP-16's 261) + 1 doctest; `amazing-devtools`
crate nextest 113/113 + 1 doctest; workspace nextest 3029 passed, 0 failed,
12 skipped; all 4 workspace doctests passed. Committed as `3d06357e`. No
cross-lane seam needed (`questions.md` for js-runtime/dom-bindings/layout
untouched this ticket).

Boundary candidates for a future ticket: `DOM.getNodeForLocation` (trivial
reuse of the new `hit_test`, still `Unsupported`), inline-run hit-testing, a
real text/replaced-content provider for layout (shared by hit-testing and
the ax tree), and Network/Fetch domain wiring on the live page.

## CDP-18 — Network/Runtime console/exceptions, Log, object inspection — done

Wired the remaining "live page" gaps CDP-16/17 left `Unsupported`:

- **Network**: `requestWillBeSent`/`responseReceived`/`loadingFinished`/
  `loadingFailed` plus `getResponseBody`, for the top-level document fetch
  (`Page.navigate` and the initial load) and every `<script src>` fetch.
  Bypassed `cmd::real_pages::fetch_document` (which discards status/headers
  and fails hard on non-2xx) in favor of calling `amazing_net::fetch`
  directly from the driver, so real status codes, headers and bodies are
  available (`amazing-devtools/src/host.rs` adds `NetworkFailedEvent` +
  `drain_network_failed`). Response bodies are cached per request id in
  `PageOwner::bodies` for `getResponseBody`. Scope is document + script only
  — stylesheet/image loads belong to css/layout/replaced-content, not this
  driver, and are not instrumented.
- **Runtime console/exceptions**: `consoleAPICalled` reads
  `amazing_js::interp::Interpreter.output` directly (the field the driver
  already read for its own diagnostics) rather than needing any new
  engine-side hook. `native_log` is the only console method the interpreter
  implements and already joins every argument into one string, so every
  event has `kind: "log"` and one string `RemoteObject` argument.
  `exceptionThrown` covers two sources: page-load failures (a thrown script,
  inline handler, or bounded event-loop step) and a mouse/key event
  listener's own throw — the latter used to propagate as `HostError::
  Internal`, failing the whole `Input.dispatch*Event` command; it now goes
  through `PageOwner::record_exception` and the command still returns `Ok`,
  matching a real browser (a page's own bug doesn't fail the DevTools
  command, only gets reported through the console/exception channel).
  `Log.entryAdded` mirrors both exception sources plus network failures,
  not ordinary `console.log` calls — matching Chrome's own split between
  the Log domain (browser-internal) and the console API.
- **Page load: strict → lenient**: `PageOwner::from_parsed`/`fetch_and_load`
  no longer call `cmd::js::prepare_page_scripts` (which fails the whole
  load — and drops the realm — on the first script/handler error). A new
  `PageOwner::load_scripts_leniently` mirrors `cmd::js::
  execute_page_scripts_lenient`'s collect-don't-abort logic but keeps this
  page's own persistent `scheduler` (that sibling function builds and drops
  its own scheduler, which doesn't fit CDP-16's persistent-realm design);
  every failure becomes an exception+log entry instead of an `Err`.
- **Runtime.getProperties/callFunctionOn**: a per-realm `objectId` registry
  (`PageOwner::objects: HashMap<String, JsValue>`) minted by
  `PageOwner::remote_object`, which every `evaluate`/`getProperties`/
  `callFunctionOn` result now goes through instead of the old free function
  `js_value_to_remote_object` (removed). Built entirely on `amazing-js`'s
  already-public embedding API — no engine or shared-file change needed:
  `binding_own_property_keys`/`binding_get_own_property` (own properties,
  reading through a getter via `binding_call` when present) for
  `getProperties`, and `binding_call` (with `this` resolved from `objectId`
  or `binding_global_object`, arguments from `objectId` handles or
  `json_to_js_value` for JSON literals) for `callFunctionOn`. `objectId`s
  never expire individually except via `Runtime.releaseObject`; navigating
  the page replaces the whole `PageOwner`, dropping all of them at once —
  matching a real navigation invalidating the previous document's ids.
- **Debugger.\* stays `Unsupported`** — the ticket explicitly keeps
  stepping/breakpoints for js-runtime, since that needs VM-level hooks this
  driver does not implement. No `questions.md` entry was needed: nothing
  else in this ticket required another lane's internals.
- Proof — two new end-to-end tests, both a real WebSocket CDP client
  against the real page thread:
  `end_to_end_network_console_and_object_inspection_from_a_real_fetched_page`
  drives a real fixture HTTP server (`serve_multi`, new — serves N routes by
  path, generalizing the existing single-route `serve_once`) with a document
  that `<script src>`s an external script which `console.log`s and sets a
  global object, through `PageInit::Fetch` (the real `amazing_net::fetch`
  path, not the in-memory `spawn_test_host` shortcut), and checks
  `Network.requestWillBeSent`/`responseReceived`/`loadingFinished` for both
  the document and the script, `Runtime.consoleAPICalled` for the script's
  log line, `Runtime.getProperties` and `callFunctionOn` against the
  script's global object, and `Network.getResponseBody` for the script.
  `end_to_end_uncaught_script_error_is_reported_not_fatal` proves a page
  with a top-level throw still loads (`document.title` still evaluates) and
  produces both `Runtime.exceptionThrown` and a `source: "javascript"`
  `Log.entryAdded`.

Known limitations, newly documented: console events are always `kind:
"log"` with one string argument (no `warn`/`error`/`info`/`debug`, no
per-argument `RemoteObject`s — `native_log` already collapsed everything
before this driver ever sees it); `getProperties`' `ownProperties=false`
(include inherited) is not distinguished from `true` — every call is own
properties only, every descriptor reports `isOwn: true`; `callFunctionOn`
arguments only round-trip JSON primitives, an array/object literal argument
becomes `undefined`; network instrumentation covers only document + script
fetches.

Linux gate (Boat `bx_xbhqabax`, per-lane target): `amazing` + `amazing-
devtools` crate nextest combined 383/383 (0 failed) + both crates' doctests
green; workspace nextest 3051 passed, 0 failed, 13 skipped; all workspace
doctests passed (amazing-js, amazing-html, amazing-a11y, amazing-devtools).
Committed as `528d2e03`. No cross-lane seam needed.

Boundary candidates for a future ticket: per-argument `RemoteObject`s for
console calls (needs an engine-side change to `native_log` to stop joining
arguments before the driver sees them — a js-runtime-owned seam);
`ownProperties=false` vs `true`; stylesheet/image `Network.*` events;
`Debugger.*` stepping/breakpoints (js-runtime).

## CDP-19 — DOM.getBoxModel/getContentQuads and Overlay highlight — done

Coordinator's ticket named `DOM.getDocument`, `querySelector`,
`getOuterHTML`, `DOM.setAttributeValue` and `CSS.getComputedStyleForNode` —
all five were already implemented in CDP-16/17. This ticket's actual delta,
confirmed by grepping the driver for zero hits on "highlight"/"Overlay"
before starting, was `DOM.getBoxModel`/`getContentQuads` and every
`Overlay.*` highlight method — both already fully dispatch-wired in
`amazing-devtools` from an earlier generic-protocol ticket (`domains/dom.rs`,
`domains/overlay.rs`), defaulting to `Unsupported`/`Ok(())` since the driver
never implemented them. Pure driver-side work; no `amazing-devtools` change
needed.

- **`DOM.getBoxModel`**: a new `find_box_for_node` walks the same fallback
  layout tree `hit_test_node`/`full_ax_tree` already use (style compute +
  `amazing_layout::layout_styled_document` with `FallbackTextSystem`/
  `NoReplacedElements`), matching by node identity (`LayoutBox::node ==
  Some(target)`) instead of by point — mirrors `hit_test`'s traversal order
  (checks `positioned` descendants first) but doesn't need paint-order
  tie-breaking since only one node can match. A new `box_model_from`
  converts the found box into a CDP `BoxModel`: `border_box`/`content_box`
  are already resolved by layout; the padding box is the border box inset
  by the box's own resolved `border` (thickness) `Sides`; the margin box is
  the border box outset by `margin` (`SidesM::unwrap_or(0.0)` — an
  unresolved auto margin reports as `0`, the same simplification this file
  already uses elsewhere).
- **`DOM.getContentQuads`** wraps `get_box_model` in a one-element `Vec` —
  the dispatch layer already extracts just `model.content` per node
  (confirmed by reading `domains/dom.rs`'s existing `"DOM.getContentQuads"`
  arm before writing this — it was already spec-correct: "content quads",
  not border quads, matching real Chrome), so no per-line-box splitting was
  needed to make the numbers agree; both share the same documented
  `Flow::Inlines` gap `hit_test`/`find_box_for_node` already have.
- **`Overlay.highlightNode`** validates the node resolves to real box
  geometry by calling `get_box_model` and discarding the result, so an
  unknown or box-less node (a `display: none` element, which produces no
  `LayoutBox` at all) reports `HostError::NotFound` exactly as a real
  highlight request against a non-existent target would fail.
  `highlightRect`/`highlightQuad` now answer `Ok(())` directly from
  `DriverPageSession` (no page-thread round trip — there's no engine state
  to check arbitrary coordinates against, and real Chrome accepts any rect/
  quad too, so this is not a shortcut around real validation, there simply
  isn't any to do). `overlay_enabled`/`setInspectMode`/`hideHighlight` were
  *already* answering `Ok(())` from the trait's own defaults, unnoticed
  until this ticket read the actual default bodies closely — no override
  needed. **No compositor overlay layer exists in this engine**, so none of
  the four highlight methods paint anything; there is no pixel difference a
  screenshot could prove, which is why `DOM.getBoxModel`'s real geometry is
  the proof instead.
- Proof — one end-to-end WebSocket CDP client test chaining the "already
  done" DOM/CSS methods onto the new box-model/Overlay ones against a
  single live node (`end_to_end_box_model_and_overlay_highlight_on_the_live_
  page`): `getDocument` → `querySelector` → `getOuterHTML` (contains the
  text) → `setAttributeValue` → `getOuterHTML` again (contains the new
  attribute) → `getComputedStyleForNode` → `getBoxModel` (asserts the real
  box-model arithmetic: a 100×50 content box with 2px padding + 5px border
  on every side reports a 114×64 border box, and a further 3px margin
  reports a 120-wide margin box — pure box-model math, independent of where
  the absolutely-positioned element actually landed on the page) →
  `getContentQuads` (one 100-wide content quad) → `highlightNode` (succeeds)
  → `hideHighlight` → `highlightNode` on a `display: none` sibling (a real
  CDP error, checked with a new `Client::raw` helper that returns the whole
  reply instead of assuming success, since `result()`/`result_and_events()`
  both panic on an `"error"` reply by design).
- A real bug the test caught mid-write: the first draft of the test asserted
  `getContentQuads` returned the *border*-box quad (114 wide); it actually
  returns the *content*-box quad (100 wide) — not a driver bug, a
  misreading of `domains/dom.rs`'s already-existing, already-correct
  dispatch code (which literally is named "content quads"). Fixed the test,
  not the driver.

Known limitations, newly documented: no compositor overlay layer exists, so
`Overlay.*` highlighting is validate-only with no visual effect;
`getBoxModel`/`getContentQuads`/`highlightNode` share `hit_test`'s
`Flow::Inlines` gap (a node that only contributes to an inline line box is
not found); an unresolved auto margin reports as `0` in the margin box.

Linux gate (Boat `bx_xbhqabax`, per-lane target): `amazing` + `amazing-
devtools` crate nextest combined 387/387 (0 failed) + both crates' doctests
green; workspace nextest 3113 passed, 0 failed, 13 skipped; all workspace
doctests passed. Committed as `4f01de21`. No cross-lane seam needed.

Boundary candidates for a future ticket: a real compositor overlay layer to
actually paint highlights (would need the `compositor` lane); per-line-box
`getContentQuads` for inline content; per-argument `RemoteObject`s for
console calls; `ownProperties=false` vs `true`; stylesheet/image
`Network.*` events; `Debugger.*` stepping/breakpoints (js-runtime).
