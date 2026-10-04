//! Driver adoption of the CDP host seam (`amazing-devtools::host`), CDP-16.
//!
//! This is the devtools lane's own driver-adapter file (see AGENTS.md "Driver
//! adapter per lane"). integrator-assist landed the first slice
//! (`Page.navigate`/`getFrameTree`/`getLayoutMetrics`, `Runtime.evaluate`) with
//! a documented gap: `evaluate` built a *fresh* JS realm per call, because the
//! `amazing-js` heap is `Rc`-based (single-threaded execution is a locked
//! decision) while `DevToolsSession: Send` and a CDP connection owns its
//! session on its own thread.
//!
//! CDP-16 fixes that by giving the page a single owning thread instead of
//! trying to make the engine itself `Send`. [`PageOwner`] holds the live DOM
//! (`Rc<RefCell<BrowserDomHost>>`) and a persistent [`DriverScheduler`] (one JS
//! realm, kept alive across calls) and never leaves the thread
//! [`spawn_page_thread`] starts for it. [`DriverDevToolsHost`] and
//! [`DriverPageSession`] are thin, `Send`/`Sync` handles that talk to that
//! thread over an `std::sync::mpsc` channel of [`PageCommand`]s, each carrying
//! its own one-shot reply channel — the same request/response shape a
//! multi-process browser uses between its DevTools and renderer processes,
//! done here with threads and channels because Amazing is still
//! single-process (ADR "no-sandbox/no-multi-process non-goal", superseded by
//! ADR 0005 but not yet implemented).
//!
//! `Runtime.evaluate`, `Page.navigate`, `DOM.getDocument`/`querySelector(All)`
//! and `Accessibility.getFullAXTree` all now go through that one channel to
//! the one page thread, so they see the same live DOM and the same JS realm
//! (globals, closures) across calls — the regression test
//! `evaluate_persists_full_js_realm_state_across_calls` is the proof.
//!
//! CDP-17 makes the session usable by a real DevTools frontend: `DOM`
//! mutation (`describeNode`, `getOuterHTML`, `setAttributeValue`,
//! `setOuterHTML`, `removeNode`, `requestChildNodes`), comma-separated
//! selector lists, `CSS` computed/matched styles and stylesheet text,
//! `Input.dispatchMouseEvent`/`dispatchKeyEvent` into the page's real event
//! dispatch, and `Page.captureScreenshot` as PNG through the existing
//! `amazing render` raster path (`amazing_canvas::encode_png`, reused rather
//! than duplicated — see its doc comment for the stored-DEFLATE zlib it
//! writes).
//!
//! ## Known limitations (follow-ups, not silently claimed as done)
//!
//! * `Page.captureScreenshot` only supports `format: "png"`; `jpeg`/`webp`
//!   stay `Unsupported` (no encoder for either in the workspace). `clip` crops
//!   the rendered surface; `quality`/`captureBeyondViewport`/`fromSurface`
//!   are accepted but not honored beyond the existing raster path's own
//!   viewport sizing.
//! * `Input.dispatchMouseEvent` hit-tests the *normal-flow and positioned*
//!   layout tree (`amazing_layout::Flow::Blocks/Flex/Grid` plus
//!   `LayoutBox::positioned`, topmost-child-wins) to resolve a target node
//!   from `(x, y)`; it does not descend into `Flow::Inlines` line boxes, so a
//!   click inside inline text resolves to the nearest block/inline-container
//!   ancestor, not the exact inline run. The dispatched DOM event carries no
//!   coordinate/button detail properties — no binding-host API accepts a rich
//!   init dict at the `dispatch_dom_event` layer this driver already uses for
//!   scripted event dispatch.
//! * `Input.dispatchKeyEvent` dispatches to `Dom::active_element`, falling
//!   back to the document root when nothing is focused; likewise carries no
//!   key/text detail properties on the constructed event object.
//! * `CSS.getMatchedStylesForNode` walks the page's own author stylesheets
//!   (`amazing_css::selectors::matches` against each rule's
//!   `selector_srcs`) and reports every match as `StyleOrigin::Author`; the
//!   user-agent/built-in sheet is not separately exposed, so UA-only rules
//!   are not listed.
//! * `CSS.getStyleSheetText` uses the 0-based index into the page's collected
//!   author stylesheets as an ad-hoc `styleSheetId` (this driver does not
//!   otherwise mint or track stylesheet ids) and reconstructs CSS text from
//!   parsed `selector_srcs`/`declarations` — an honest re-serialization, not
//!   the original source bytes (the parser does not retain those).
//! * `DOM.setOuterHTML` re-parses the replacement HTML with
//!   `amazing_html::parse_fragment` in the *parent* element's tag-name
//!   context (or `"body"` if the parent isn't an element) and splices the
//!   result in at the original position; it does not attempt the spec's
//!   exact context-element algorithm for every edge case (e.g. `<template>`
//!   contents, foreign-namespace contexts).
//! * `Accessibility.getFullAXTree` (like `amazing axtree`) lays out with
//!   `amazing_text::FallbackTextSystem` and `amazing_layout::NoReplacedElements`,
//!   so replaced content and real glyph metrics are not reflected — the same
//!   documented gap as the a11y lane's own CLI entry point. The same fallback
//!   layout is used for hit-testing and for computed style's box geometry.
//! * `depth` on `Accessibility.getFullAXTree` is ignored (the whole tree is
//!   always returned), matching the CDP `-1` convention this crate already
//!   normalizes `depth` to at the domain layer.
//!
//! CDP-18 wires `Network`, `Runtime` console/exceptions, `Log`, and
//! `Runtime.getProperties`/`callFunctionOn` (with an objectId registry) to
//! the live page, and switches page load from strict (`prepare_page_scripts`)
//! to lenient: a thrown script, inline handler or event-loop step no longer
//! fails the whole load, it becomes a `Runtime.exceptionThrown` +
//! `Log.entryAdded` pair instead (`PageOwner::load_scripts_leniently`,
//! replacing the old `prepare_page_scripts` call). `Input.dispatchMouseEvent`/
//! `dispatchKeyEvent` now catch a listener's own throw the same way rather
//! than failing the DevTools command itself.
//!
//! * Console capture reads `amazing_js::interp::Interpreter.output`
//!   directly — the field the driver already used for its own diagnostics.
//!   `native_log` is the *only* console method the interpreter implements
//!   (no `warn`/`error`/`info`/`debug`), and it already joins every argument
//!   into one string before appending, so every `Runtime.consoleAPICalled`
//!   this driver emits has `kind: "log"` and a single string `RemoteObject`
//!   argument — individual argument values and the real console level are
//!   both lost before this driver ever sees the line.
//! * Network instrumentation covers only the top-level document fetch
//!   (`Page.navigate`/initial load) and `<script src>` fetches; stylesheet
//!   and image loads are not instrumented (those loading paths belong to the
//!   css/layout/replaced-content lanes, not this driver). `getResponseBody`
//!   only has a body for a request this driver itself fetched and recorded.
//! * `Runtime.getProperties`' `ownProperties=false` (include inherited) is
//!   not distinguished from `true`: every call uses
//!   `binding_own_property_keys`/`binding_get_own_property` (own properties
//!   only), and every descriptor reports `isOwn: true`.
//! * `Runtime.callFunctionOn`'s `arguments[].value` only round-trips
//!   JSON primitives (null/bool/number/string, via [`json_to_js_value`]);
//!   an array or object literal argument becomes `undefined` — the JSON
//!   model has no existing bridge into a *nested* JS object/array outside
//!   `amazing-js`'s own `JSON.parse` built-in.
//! * `objectId`s are minted per `PageOwner` (one realm) and never expire on
//!   their own; `Runtime.releaseObject` is the only way to drop one, and
//!   navigating the page (which replaces the whole `PageOwner`) drops all of
//!   them at once, matching a real frame navigation invalidating every
//!   `objectId` from the previous document.
//! * `Debugger.*` (stepping, breakpoints) stays `Unsupported`: CDP-18's
//!   ticket explicitly keeps that seam for the js-runtime lane, since it
//!   needs VM-level hooks this driver does not implement.
//!
//! CDP-19 implements `DOM.getBoxModel`/`getContentQuads` and
//! `Overlay.highlightNode`/`highlightRect`/`highlightQuad` on the live page
//! (`DOM.getDocument`/`querySelector`/`getOuterHTML`/`setAttributeValue` and
//! `CSS.getComputedStyleForNode` were already implemented from CDP-16/17;
//! this ticket's own delta is the box model and overlay).
//!
//! * `find_box_for_node` walks the same fallback layout tree `hit_test_node`/
//!   `full_ax_tree` use, matching by node identity instead of by point, to
//!   build a `DOM.BoxModel` (content/padding/border/margin quads) from a
//!   `LayoutBox`'s already-resolved `border_box`/`content_box` plus its
//!   `border`/`margin` side widths. `getContentQuads` (whose dispatch layer
//!   already extracted just `model.content` from an earlier ticket) does
//!   not split inline content into one quad per line box — every node gets
//!   the single content quad `getBoxModel`'s `content` field would also
//!   report (the same `Flow::Inlines` gap `hit_test`/`find_box_for_node`
//!   already document).
//! * **No compositor overlay layer exists in this engine**, so
//!   `Overlay.highlightNode`/`highlightRect`/`highlightQuad` never paint
//!   anything — there is no pixel difference to prove over
//!   `Page.captureScreenshot`. `highlightNode` still does real work: it
//!   validates the node resolves to actual box geometry (the same
//!   computation `getBoxModel` uses), reporting `HostError::NotFound` for an
//!   unknown or box-less node exactly as a real highlight request would
//!   fail. `highlightRect`/`highlightQuad` accept any coordinates (matching
//!   real Chrome, which does too) and answer directly from
//!   `DriverPageSession` without a page-thread round trip, since there is no
//!   engine state to check them against. `overlay_enabled`/
//!   `set_inspect_mode`/`hideHighlight` are left at the trait's own `Ok(())`
//!   defaults for the same reason. Proof is `DOM.getBoxModel`'s real
//!   geometry, not a rendered overlay.
//!
//! CDP-20 implements the `Debugger` domain: `enable`/`disable`,
//! `scriptParsed` (classic and module scripts), `getScriptSource`,
//! `setBreakpointByUrl`/`removeBreakpoint` at line granularity, and
//! `pause`/`resume`, plus a minimal per-statement hook added to
//! `amazing-js` itself (`amazing_js::debug_hook::DebugHook`,
//! `Interpreter::set_debug_hook`/`set_current_script`) so this driver can
//! see which line is about to run without the interpreter knowing what a
//! breakpoint is.
//!
//! * **This driver cannot suspend JS execution mid-statement.** The engine
//!   is a single-threaded tree-walking interpreter with no coroutine/fiber
//!   mechanism, and this driver's own page thread processes one
//!   `PageCommand` at a time (see the module's top doc comment) — nothing
//!   can service a `Debugger.resume` command while the same thread is stuck
//!   inside a synchronous `run_script` call. So `Debugger.pause` cannot mean
//!   "halt execution and wait": it arms a one-shot flag that the *next*
//!   statement to actually run (the next event-loop tick, timer, or handler
//!   on an otherwise idle page) reports as a `Debugger.paused` "other" event
//!   — real, but after the fact, not a live suspend. `Debugger.resume`
//!   correspondingly just clears that flag if it had not fired yet; nothing
//!   is ever truly blocked for it to resume. CDP-23 makes `stepInto`/
//!   `stepOver`/`stepOut` real in this same after-the-fact sense (see that
//!   section below) by comparing the interpreter's own call-stack depth
//!   against the depth at the last pause, and `evaluateOnCallFrame` real by
//!   delegating to the page's global realm — but none of the three make
//!   execution actually stop mid-statement. Real synchronous pause/resume/
//!   step needs the interpreter itself to expose a
//!   resumable/steppable execution API (fibers, or a bytecode VM with
//!   explicit continuation state); that is a genuine `amazing-js` change,
//!   not a driver one, and is filed to the js-runtime lane in
//!   `.agents/js-runtime/questions.md`.
//! * **A breakpoint hit is still real**, just reported post-hoc: the hook
//!   fires on every statement (`Interpreter::exec_stmt`'s choke-point, plus
//!   the top-level program loop's own fast path for a bare `expr;`
//!   statement, which bypasses `exec_stmt` for performance and needed its
//!   own hook call), checks the statement's `(url, line)` against every
//!   `setBreakpointByUrl` breakpoint, and queues a `Debugger.paused` event
//!   with `hitBreakpoints` set and `reason: "Breakpoint"` the moment one
//!   matches — it just cannot stop the statement from finishing first.
//! * **Script identity is coarse.** `Interpreter::set_current_script` tags
//!   every subsequent statement with whichever script the driver most
//!   recently told it about; only [`PageOwner::load_scripts_leniently`]'s
//!   per-top-level-script loop calls it. An inline event handler, a timer
//!   callback, or a `Runtime.evaluate`/`Runtime.callFunctionOn` call all run
//!   under whatever the *last top-level script* set, not their own real
//!   identity — an honest simplification, not real Chrome's per-execution-
//!   context script attribution.
//! * **Module dependency instrumentation is incomplete.** Root module scripts
//!   execute and report their source; dependency fetches use the browser module
//!   transport without recording the PageOwner's Network/Debugger events.
//! * **Script line/column extents are relative to the script's own text**,
//!   not to its position in the containing HTML document — this driver has
//!   no HTML-node source-position tracking to add that offset, so
//!   `startLine`/`startColumn` are always `0`/`0` and `endLine`/`endColumn`
//!   come from a plain line/character count of the script's own source
//!   ([`script_extent`]). `hash` is a `std::hash::DefaultHasher` (SipHash)
//!   digest of the source text ([`script_hash`]) — a real, stable value a
//!   client can compare across edits, not V8's own script-hash algorithm.
//! * **`setBreakpointByUrl`'s `url` is matched literally**, never as a
//!   regex. Its `condition` parameter was accepted-but-ignored through
//!   CDP-22; CDP-23 evaluates it for real (see that section below).
//!
//! CDP-21 implements `Network.setCacheDisabled`/`setExtraHTTPHeaders` (new
//! `amazing-devtools` protocol-layer trait methods, since neither existed
//! there before this ticket — unlike CDP-19/20 the dispatch layer needed
//! extending, not just the driver), `Emulation.setDeviceMetricsOverride`/
//! `clearDeviceMetricsOverride`, and `Input.insertText`.
//!
//! * **`Network.setCacheDisabled` has no actual HTTP cache to disable** —
//!   `amazing-net` has no cache subsystem at all. The honest, wire-observable
//!   substitute: while disabled, every future request (including the very
//!   next document fetch a `Page.navigate` performs) carries real
//!   `Cache-Control: no-cache`/`Pragma: no-cache` request headers, matching
//!   what a real browser sends over the wire in addition to bypassing its
//!   cache.
//! * **`Network.setExtraHTTPHeaders` is a full replace, not a merge**,
//!   matching CDP, and applies to every future request the same way.
//! * **Both survive a `Page.navigate` reload.** A reload replaces the whole
//!   [`PageOwner`] (`navigate()`'s `*self = reloaded`), and the very first
//!   document fetch of the *new* `PageOwner` must already see the headers —
//!   so `extra_headers`/`cache_disabled` are threaded as explicit parameters
//!   into [`PageOwner::fetch_and_load`] itself, with `navigate()` passing its
//!   own current state forward, rather than trying to inject them after
//!   construction.
//! * **`Emulation.setDeviceMetricsOverride`'s `width`/`height` are read
//!   through [`PageOwner::effective_width`]/[`PageOwner::effective_fixed_height`]
//!   at every layout/render entry point** (`getLayoutMetrics`, hit-testing,
//!   `getBoxModel`, `getComputedStyleForNode`'s media environment,
//!   `captureScreenshot`, `getFullAXTree`), so the override is consistently
//!   observable everywhere, not just in one command. `height: 0` means "don't
//!   override height" (auto height from content), matching real Chrome's own
//!   documented convention for that field — mapped directly onto this
//!   engine's pre-existing `paint_host_surface(..., fixed_height: Option<u32>,
//!   ...)` convention where `None` already meant the same thing.
//! * **The override does not ride along with a reload the way headers do** —
//!   there is no "base height" field to bake it into (unlike width, which has
//!   a real base field), so `navigate()` instead captures
//!   `override_width`/`override_fixed_height`/`device_scale_factor` before
//!   fetching and explicitly restores them onto the replacement `PageOwner`
//!   afterward.
//! * **`deviceScaleFactor`/`mobile`/`screenWidth`/`screenHeight`/
//!   `positionX`/`positionY`/`dontSetVisibleSize`/orientation are accepted
//!   but not applied** beyond storing `deviceScaleFactor` — this driver's
//!   raster path has no concept of a separate screen size, window position,
//!   or orientation to honor them against.
//! * **`Input.insertText` has no caret/selection model.** It appends `text`
//!   to the focused element's own `value` (`document.activeElement`) and
//!   dispatches a real, bubbling `input` event — through a small synthesized
//!   JS snippet run via the existing `scheduler.run_script`, reusing the
//!   engine's own real `activeElement`/`value` setter/`Event`/`dispatchEvent`
//!   bindings rather than reimplementing DOM value-setting or event dispatch
//!   in the driver. Scope is `<input>`/`<textarea>`-only (anything with a
//!   `value` property); a `contenteditable` element or nothing focused
//!   returns `HostError::Unsupported`, since there is no rich-text insertion
//!   point to append to.
//!
//! CDP-22 adds `Debugger.stepOver` and real `scopeChain` on paused frames,
//! plus two `Runtime.callFunctionOn`/`getProperties` completeness fixes
//! (`returnByValue`, inherited properties) — all still built entirely on the
//! exec_stmt debug hook and object-registry machinery CDP-20/CDP-18 already
//! added, with no further `amazing-js` changes.
//!
//! * **`Debugger.stepOver` (CDP-22) shared `Debugger.pause`'s exact
//!   one-shot-flag mechanism**, because [`DebugHook::on_statement`] reported
//!   only `(script_id, url, line)`, with no call-stack depth. CDP-23 (below)
//!   replaces this with real depth-based stepping.
//! * **`Debugger.paused`'s call frames now carry a real `scopeChain`.**
//!   [`DriverDebugHook::on_statement`] itself still cannot build one (it only
//!   has plain `Rc<RefCell<_>>` cells, no interpreter/object-registry
//!   access — see that struct's own doc comment); [`PageOwner::
//!   drain_debugger_paused`] fills it in afterward, since it runs on
//!   `PageOwner` and has both. The only scope this driver can honestly offer
//!   is `"global"` — this engine's tree-walking interpreter has no reified
//!   per-call local/closure environment record reachable from outside a
//!   running call, so there is no real per-frame "local"/"closure" scope
//!   object to hand back. The global object itself is real and live, not a
//!   snapshot: it is minted through the same [`PageOwner::remote_object`]
//!   registration `Runtime.evaluate` uses, so `Runtime.getProperties` on it
//!   reflects genuine `globalThis` state at the moment of the pause.
//! * **`Runtime.evaluate`/`callFunctionOn`'s `returnByValue: true` is now
//!   honored.** It was accepted (the field exists on both param structs)
//!   but silently ignored before this ticket — every object result got an
//!   `objectId` reference regardless of the flag.
//!   [`PageOwner::remote_object_by_value`] serializes a plain object/array
//!   structurally instead (own *enumerable* properties through getters,
//!   `Array.isArray`-detected element order for arrays), depth-capped at 5
//!   rather than cycle-tracked (no pointer-identity check for `JsValue::
//!   Object`'s `Rc` exists in this driver) — an honest safety net for a
//!   self-referential object, not a full structured-clone. `BigInt`/`Symbol`
//!   values inside a by-value result are nulled, matching the existing
//!   "can't carry either verbatim" limitation elsewhere in this file.
//! * **`Runtime.getProperties`'s `ownProperties: false` now walks the
//!   prototype chain** instead of silently behaving like `true` always did.
//!   There is no `[[Prototype]]` accessor in `amazing-js`'s own binding
//!   surface for an arbitrary object (`interp_bindings.rs` only has
//!   `binding_object_prototype`/`binding_set_global_prototype`, both for the
//!   *global* object's own prototype specifically) — [`PageOwner::
//!   get_properties`] instead reuses real JS (`Object.getPrototypeOf`)
//!   through the same `run_script` + `binding_call` idiom `callFunctionOn`
//!   already uses, walking up to 20 levels (a safety net against a
//!   corrupted/cyclic chain, not a real spec limit) and tagging each
//!   inherited property `is_own: false`.
//!
//! CDP-23 makes `Debugger.stepOver`/`stepInto`/`stepOut` real (depth-based,
//! not the CDP-22 one-shot-flag approximation), evaluates
//! `setBreakpointByUrl`'s `condition`, and implements
//! `Debugger.evaluateOnCallFrame` — the last new `Debugger` methods this
//! driver had left at the trait's `Unsupported` default. The only
//! `amazing-js` change is [`DebugHook::on_statement`] gaining a 4th `depth:
//! u32` parameter — `Interpreter`'s own existing `MAX_CALL_DEPTH`
//! recursion-guard counter, reported as-is; see `debug_hook.rs`'s own CDP-23
//! doc-comment addition. Everything else here is driver-side.
//!
//! * **Stepping compares depth, not identity.** [`StepMode`] holds what the
//!   *next* qualifying statement's `depth` must satisfy: `Any` (stepInto,
//!   and `Debugger.pause`) takes the very next statement regardless of
//!   depth; `Over(reference)` (stepOver) takes the next statement no deeper
//!   than `reference`; `Out(reference)` (stepOut) takes the next statement
//!   strictly shallower than `reference`. `reference` is
//!   [`PageOwner::step_reference_depth`] — the depth recorded at the *most
//!   recent* pause/breakpoint hit ([`PageOwner::last_paused_depth`]) — since
//!   this driver still never truly suspends anything (unchanged from
//!   CDP-20/22: see the bullets above), so a step command arriving has no
//!   live "current frame" of its own to read a depth from; the last pause is
//!   the best available stand-in, and matches the common case of a step
//!   issued right after a pause/breakpoint/previous step landed. Before any
//!   pause has ever happened, the reference defaults to `0`, so an initial
//!   `stepOver`/`stepOut` behaves as "stop at the next top-level statement".
//! * **Conditional breakpoints are evaluated at drain time, not by the
//!   hook.** [`DriverDebugHook::on_statement`] still only has plain
//!   `Rc<RefCell<_>>` cells (see that struct's own doc comment on why); the
//!   condition itself is stored on the [`BreakpointRecord`] and evaluated in
//!   [`PageOwner::drain_debugger_paused`] via the same `scheduler.run_script`
//!   + `binding_to_boolean` path other real-evaluation features in this
//!   driver already use. An erroring condition counts as false (matching
//!   real Chrome, which never surfaces a bad condition's exception to the
//!   frontend as if it were a thrown breakpoint). A breakpoint hit whose
//!   condition fails is filtered out of that event's `hit_breakpoints`; if
//!   that empties it and the pause was not *also* independently forced (a
//!   `Debugger.pause`/step landing on the same statement), the whole event
//!   is dropped, exactly as if the breakpoint had never matched.
//! * **`Debugger.evaluateOnCallFrame` has exactly one real frame to evaluate
//!   in.** This driver never truly suspends a call frame, and the only call
//!   frame id it ever mints is the literal `"cf0"`
//!   ([`DriverDebugHook::on_statement`]'s synthesized [`CallFrame`]), so
//!   [`PageOwner::evaluate_on_call_frame`] validates `call_frame_id == "cf0"`
//!   (else a genuine `HostError::NotFound`, matching real Chrome for a
//!   stale/unknown frame id) and otherwise delegates to the exact same
//!   global-realm evaluation `Runtime.evaluate` uses.
//! * **Local/closure scopes remain out of reach.** [`PageOwner::
//!   drain_debugger_paused`]'s `scopeChain` is still `"global"`-only (CDP-22
//!   limitation, unchanged) — `amazing-js`'s `Env` has no public accessor to
//!   enumerate a frame's own bindings. Filed to js-runtime in
//!   `.agents/js-runtime/questions.md` (a new, separate entry from the
//!   existing true-pause request).
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use amazing_devtools::host::{
    AxNode, BoxModel, CallArgument, CallFrame, CallFunctionParams, ConsoleEvent, DeviceMetrics,
    DevToolsHost, DevToolsSession, EngineNodeId, EvaluateParams, EvaluateResult, ExceptionDetails,
    ExceptionEvent, FrameInfo, FrameTree, HighlightConfig, HostError, KeyEvent, LayoutMetrics,
    LogEntry, MatchedRule, MouseEvent, NetworkFailedEvent, NetworkFinishedEvent,
    NetworkRequestEvent, NetworkResponseEvent, NodeDescription, ObjectPreview, PausedEvent,
    PropertyDescriptor, PropertyPreview, Quad, RemoteObject, ResourceContent, Scope, Screenshot,
    ScreenshotFormat, ScreenshotParams, ScriptInfo, ScriptLocation, StyleOrigin, TargetInfo,
    Viewport,
};
use amazing_devtools::server::CdpServer;
use amazing_js::debug_hook::DebugHook;
use amazing_js::DomHost;

use crate::dom_host::BrowserDomHost;
use crate::js_scheduler::DriverScheduler;

/// Reply channel for one [`PageCommand`].
type Reply<T> = Sender<Result<T, HostError>>;

/// One request the session/host side sends to the page thread. Every variant
/// is `Send` (plain owned data plus a `Sender`, itself `Send` when its payload
/// is), which is what lets [`DriverPageSession`] satisfy `DevToolsSession: Send`
/// without the engine itself being `Send`.
enum PageCommand {
    Navigate(String, Reply<FrameInfo>),
    FrameTree(Reply<FrameTree>),
    LayoutMetrics(Reply<LayoutMetrics>),
    Evaluate(EvaluateParams, Reply<EvaluateResult>),
    GetDocument(u32, Reply<NodeDescription>),
    QuerySelector(EngineNodeId, String, Reply<Option<EngineNodeId>>),
    QuerySelectorAll(EngineNodeId, String, Reply<Vec<EngineNodeId>>),
    FullAxTree(Reply<Vec<AxNode>>),
    AccessibilityEnabled(bool, Reply<()>),
    PartialAxTree(EngineNodeId, bool, Reply<Vec<AxNode>>),
    ChildAxNodes(String, Reply<Vec<AxNode>>),
    QueryAxTree(Option<EngineNodeId>, Option<String>, Option<String>, Reply<Vec<AxNode>>),
    DrainAxNodesUpdated(Reply<Vec<AxNode>>),
    DrainAxLoadComplete(Reply<Option<AxNode>>),
    DescribeNode(EngineNodeId, u32, Reply<NodeDescription>),
    GetOuterHtml(EngineNodeId, Reply<String>),
    SetAttributeValue(EngineNodeId, String, Option<String>, Reply<()>),
    SetOuterHtml(EngineNodeId, String, Reply<()>),
    RemoveNode(EngineNodeId, Reply<()>),
    ComputedStyleForNode(EngineNodeId, Reply<Vec<(String, String)>>),
    MatchedStylesForNode(EngineNodeId, Reply<Vec<MatchedRule>>),
    StyleSheetText(String, Reply<Option<String>>),
    DispatchMouseEvent(MouseEvent, Reply<()>),
    DispatchKeyEvent(KeyEvent, Reply<()>),
    CaptureScreenshot(ScreenshotParams, Reply<Screenshot>),
    GetProperties(String, bool, Reply<Vec<PropertyDescriptor>>),
    CallFunctionOn(CallFunctionParams, Reply<EvaluateResult>),
    ReleaseObject(String, Reply<()>),
    ReleaseObjectGroup(String, Reply<()>),
    GetResponseBody(String, Reply<Option<ResourceContent>>),
    DrainConsole(Reply<Vec<ConsoleEvent>>),
    DrainNetworkRequests(Reply<Vec<NetworkRequestEvent>>),
    DrainNetworkResponses(Reply<Vec<NetworkResponseEvent>>),
    DrainNetworkFinished(Reply<Vec<NetworkFinishedEvent>>),
    DrainNetworkFailed(Reply<Vec<NetworkFailedEvent>>),
    DrainExceptions(Reply<Vec<ExceptionEvent>>),
    DrainLog(Reply<Vec<LogEntry>>),
    GetBoxModel(EngineNodeId, Reply<BoxModel>),
    GetContentQuads(EngineNodeId, Reply<Vec<BoxModel>>),
    HighlightNode(EngineNodeId, Reply<()>),
    DebuggerEnabled(bool, Reply<()>),
    SetBreakpointByUrl(String, u32, Option<u32>, Option<String>, Reply<(String, Vec<ScriptLocation>)>),
    RemoveBreakpoint(String, Reply<()>),
    DebuggerPause(Reply<()>),
    DebuggerResume(Reply<()>),
    DebuggerStepOver(Reply<()>),
    DebuggerStepInto(Reply<()>),
    DebuggerStepOut(Reply<()>),
    EvaluateOnCallFrame(String, String, Reply<EvaluateResult>),
    GetScriptSource(String, Reply<String>),
    DrainScriptsParsed(Reply<Vec<ScriptInfo>>),
    DrainDebuggerPaused(Reply<Vec<PausedEvent>>),
    SetCacheDisabled(bool, Reply<()>),
    SetExtraHttpHeaders(Vec<(String, String)>, Reply<()>),
    SetDeviceMetricsOverride(DeviceMetrics, Reply<()>),
    ClearDeviceMetricsOverride(Reply<()>),
    InsertText(String, Reply<()>),
    Shutdown,
}

/// Sends `make(reply_tx)` to the page thread and blocks for its answer. The
/// only failure mode besides the command's own `HostError` is the page thread
/// having already exited (e.g. the host was dropped mid-call).
fn request<T>(sender: &Sender<PageCommand>, make: impl FnOnce(Reply<T>) -> PageCommand) -> Result<T, HostError> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    sender
        .send(make(reply_tx))
        .map_err(|_| HostError::Internal("devtools page thread is gone".into()))?;
    reply_rx
        .recv()
        .map_err(|_| HostError::Internal("devtools page thread is gone".into()))?
}

fn origin_of(url: &str) -> String {
    amazing_net::Url::parse(url)
        .map(|parsed| format!("{}://{}", parsed.scheme, parsed.host))
        .unwrap_or_else(|_| url.to_owned())
}

fn frame_info(target_id: &str, url: &str) -> FrameInfo {
    FrameInfo {
        id: target_id.to_owned(),
        parent_id: None,
        url: url.to_owned(),
        name: String::new(),
        security_origin: origin_of(url),
        mime_type: "text/html".into(),
    }
}

/// Seconds since this process started, monotonic — what every CDP event's
/// `timestamp` field means (`Network.*`, `Runtime.consoleAPICalled`,
/// `Log.entryAdded`). No existing helper in the codebase produces this (the
/// only prior use was ad hoc `Instant::now()` in test code), so CDP-18 adds
/// one process-wide monotonic clock here.
fn monotonic_seconds() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// Seconds since the Unix epoch — `Network.requestWillBeSent`'s `wallTime`.
fn wall_time_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_secs_f64())
        .unwrap_or(0.0)
}

/// Wire headers as `(name, value)` pairs, in server order, for
/// `Network.requestWillBeSent`/`responseReceived`.
fn headers_from(headers: &amazing_net::Headers) -> Vec<(String, String)> {
    headers.iter().map(|(name, value)| (name.to_owned(), value.to_owned())).collect()
}

/// The base media type (no `; charset=...` parameter), or `"text/plain"` when
/// the server sent no `Content-Type` — `Network.responseReceived`'s
/// `mimeType` is never absent on the wire even though the header can be.
fn mime_type_of(headers: &amazing_net::Headers) -> String {
    headers
        .get("content-type")
        .map(|raw| raw.split(';').next().unwrap_or(raw).trim().to_owned())
        .filter(|mime| !mime.is_empty())
        .unwrap_or_else(|| "text/plain".to_owned())
}

/// Converts a `Runtime.CallArgument.value` JSON literal into a `JsValue` for
/// `callFunctionOn`. Arrays and objects become `undefined`: the JSON model
/// only carries a value that round-trips through `JSON.parse`-shaped data,
/// and no existing binding builds a *nested* JS object/array from `Json`
/// outside `amazing-js`'s own JSON built-in — see the module's "Known
/// limitations".
fn json_to_js_value(value: &amazing_devtools::json::Json) -> amazing_js::value::JsValue {
    use amazing_devtools::json::Json;
    use amazing_js::value::JsValue;
    match value {
        Json::Null => JsValue::Null,
        Json::Bool(b) => JsValue::Bool(*b),
        Json::Number(n) => JsValue::Num(*n),
        Json::String(s) => JsValue::str(s.clone()),
        Json::Array(_) | Json::Object(_) => JsValue::Undefined,
    }
}

fn base_description(node: EngineNodeId, node_type: u16, name: &str, value: &str) -> NodeDescription {
    NodeDescription {
        node,
        parent: None,
        backend_node_id: node.0,
        node_type,
        node_name: name.to_owned(),
        local_name: String::new(),
        node_value: value.to_owned(),
        attributes: Vec::new(),
        child_node_count: 0,
        children: Vec::new(),
    }
}

/// Builds a CDP `DOM.Node` tree from the live arena DOM, expanding `depth`
/// levels of children (0 = the node itself only). `node_type` numbering
/// follows the DOM spec (`Node.ELEMENT_NODE` etc), matching what
/// `amazing-devtools`' own `NodeDescription` helpers assume.
fn describe_node(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, depth: u32) -> NodeDescription {
    use amazing_dom::NodeKind;
    let engine = EngineNodeId(node.0 as u64);
    let children: Vec<amazing_dom::NodeIdx> = dom.children(node).collect();
    let mut desc = match &dom.get(node).kind {
        NodeKind::Document => base_description(engine, 9, "#document", ""),
        NodeKind::DocumentFragment => base_description(engine, 11, "#document-fragment", ""),
        NodeKind::DocumentType { name, .. } => base_description(engine, 10, name, ""),
        NodeKind::Element { name, attrs, .. } => {
            NodeDescription::element(engine, name.clone()).with_attributes(attrs.clone())
        }
        NodeKind::Text { data } => NodeDescription::text(engine, data.clone()),
        NodeKind::Comment { data } => base_description(engine, 8, "#comment", data),
        NodeKind::CDATASection { data } => base_description(engine, 4, "#cdata-section", data),
        NodeKind::ProcessingInstruction { target, data } => base_description(engine, 7, target, data),
        NodeKind::Attr { .. } => base_description(engine, 2, "", ""),
    };
    desc.child_node_count = children.len();
    if depth > 0 {
        desc.children = children
            .into_iter()
            .map(|child| describe_node(dom, child, depth - 1))
            .collect();
    }
    desc
}

/// Splits a CDP/CSS selector list on top-level commas, respecting
/// parentheses, brackets and quoted strings (`:is(a, b)`, `[data-x="a,b"]`),
/// so `DOM.querySelector(All)` accepts the same comma-separated lists a page's
/// own CSS does.
fn split_selector_list(selector: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0usize;
    for (index, ch) in selector.char_indices() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(selector[start..index].trim());
                    start = index + 1;
                }
                _ => {}
            },
        }
    }
    parts.push(selector[start..].trim());
    parts
}

fn parse_selector_list(selector: &str) -> Result<Vec<amazing_css::selectors::Selector>, HostError> {
    split_selector_list(selector)
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| {
            amazing_css::selectors::parse_selector(part)
                .ok_or_else(|| HostError::InvalidParams(format!("bad selector: {part}")))
        })
        .collect()
}

fn matches_any(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selectors: &[amazing_css::selectors::Selector],
) -> bool {
    selectors.iter().any(|selector| amazing_css::selectors::matches(dom, node, selector))
}

/// Copies `source_node`'s subtree (from a standalone fragment `Dom`, as
/// `amazing_html::parse_fragment` returns) into `target`, appended under
/// `parent`. Mirrors `amazing-html`'s own `parse_fragment::clone_subtree`
/// (private to that crate), since there is no cross-`Dom` "adopt" primitive.
fn clone_subtree_into(
    source: &amazing_dom::Dom,
    source_node: amazing_dom::NodeIdx,
    target: &mut amazing_dom::Dom,
    parent: amazing_dom::NodeIdx,
) -> amazing_dom::NodeIdx {
    let created = target.append_child(parent, source.get(source_node).kind.clone());
    for child in source.children(source_node) {
        clone_subtree_into(source, child, target, created);
    }
    created
}

fn find_first(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selectors: &[amazing_css::selectors::Selector],
) -> Option<EngineNodeId> {
    if matches_any(dom, node, selectors) {
        return Some(EngineNodeId(node.0 as u64));
    }
    for child in dom.children(node) {
        if let Some(found) = find_first(dom, child, selectors) {
            return Some(found);
        }
    }
    None
}

fn find_all(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selectors: &[amazing_css::selectors::Selector],
    out: &mut Vec<EngineNodeId>,
) {
    if matches_any(dom, node, selectors) {
        out.push(EngineNodeId(node.0 as u64));
    }
    for child in dom.children(node) {
        find_all(dom, child, selectors, out);
    }
}

/// Serializes `node`'s outer HTML, reusing the same spec markup serializer
/// (`Dom::serialize_node`) the JS `outerHTML` getter binding uses
/// (`amazing-bindings/src/markup_bindings.rs`), so DevTools and scripted
/// access agree.
fn outer_html(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx) -> Result<String, HostError> {
    dom.serialize_node(node)
        .map(|units| String::from_utf16_lossy(&units))
        .map_err(|error| HostError::Internal(format!("{error:?}")))
}

/// Crops a rendered RGBA8 surface to `clip` (`Page.captureScreenshot`'s
/// viewport-relative clip rect), clamped to the surface's own bounds.
fn crop_surface(surface: &amazing_paint::Surface, clip: &amazing_devtools::host::Rect) -> amazing_paint::Surface {
    let x = (clip.x.max(0.0) as u32).min(surface.width);
    let y = (clip.y.max(0.0) as u32).min(surface.height);
    let w = (clip.width.max(0.0) as u32).min(surface.width.saturating_sub(x));
    let h = (clip.height.max(0.0) as u32).min(surface.height.saturating_sub(y));
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = (row as usize * surface.width as usize + x as usize) * 4;
        let end = start + w as usize * 4;
        pixels.extend_from_slice(&surface.pixels[start..end]);
    }
    amazing_paint::Surface { width: w, height: h, pixels }
}

/// Depth-first, topmost-child-wins point-in-box hit test over a laid-out
/// page. Checks absolutely/fixed-positioned descendants (`positioned`, which
/// paint above normal flow) before normal-flow children, and walks
/// `Flow::Blocks`/`Flex`/`Grid` children in reverse (later paint order wins).
/// Does not descend into `Flow::Inlines` line boxes — see the module's
/// "Known limitations".
fn hit_test<'a>(box_: &'a amazing_layout::LayoutBox, x: f32, y: f32) -> Option<&'a amazing_layout::LayoutBox> {
    // Positioned descendants are checked unconditionally, before this box's
    // own bounds: an absolutely/fixed-positioned box commonly extends beyond
    // the shrink-to-fit box of the ancestor that hoisted it (e.g. a
    // `position: absolute` child of an otherwise-empty `<body>`), so gating
    // on the ancestor's own `border_box` first would reject points that are
    // only inside the positioned descendant.
    for child in box_.positioned.iter().rev() {
        if let Some(hit) = hit_test(child, x, y) {
            return Some(hit);
        }
    }
    let b = &box_.border_box;
    if x < b.x || y < b.y || x > b.x + b.w || y > b.y + b.h {
        return None;
    }
    match &box_.flow {
        amazing_layout::Flow::Blocks(children)
        | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => {
            for child in children.iter().rev() {
                if let Some(hit) = hit_test(child, x, y) {
                    return Some(hit);
                }
            }
        }
        amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => {}
    }
    Some(box_)
}

/// Depth-first search for the laid-out box whose `node` is `target`,
/// checking `positioned` descendants first (mirrors [`hit_test`]'s own
/// traversal order, though identity search doesn't care about paint order).
/// Does not descend into `Flow::Inlines` line boxes — same documented gap as
/// `hit_test`: a node that only contributes to an inline line box (a `<span>`
/// with no box of its own, say) is not found. Used by `DOM.getBoxModel`/
/// `getContentQuads` and `Overlay.highlightNode`'s validation.
fn find_box_for_node<'a>(
    box_: &'a amazing_layout::LayoutBox,
    target: amazing_dom::NodeIdx,
) -> Option<&'a amazing_layout::LayoutBox> {
    if box_.node == Some(target) {
        return Some(box_);
    }
    for child in &box_.positioned {
        if let Some(found) = find_box_for_node(child, target) {
            return Some(found);
        }
    }
    match &box_.flow {
        amazing_layout::Flow::Blocks(children)
        | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => {
            for child in children {
                if let Some(found) = find_box_for_node(child, target) {
                    return Some(found);
                }
            }
        }
        amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => {}
    }
    None
}

/// An axis-aligned CDP `Quad` for a layout `Rect`.
fn quad_from_rect(rect: &amazing_layout::Rect) -> Quad {
    Quad::rect(rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64)
}

/// Builds a CDP `DOM.BoxModel` (content/padding/border/margin quads) from a
/// laid-out box: `border_box`/`content_box` are already resolved by layout;
/// the padding box is the border box inset by `border`'s resolved
/// thicknesses, and the margin box is the border box outset by `margin`
/// (auto margins that layout left unresolved report as `0`, the same
/// simplification `SidesM::unwrap_or` invites elsewhere in this crate).
fn box_model_from(box_: &amazing_layout::LayoutBox) -> BoxModel {
    let border = box_.border_box;
    let content = box_.content_box;
    let margin_sides = box_.margin.unwrap_or(0.0);
    let padding = amazing_layout::Rect {
        x: border.x + box_.border.left,
        y: border.y + box_.border.top,
        w: (border.w - box_.border.left - box_.border.right).max(0.0),
        h: (border.h - box_.border.top - box_.border.bottom).max(0.0),
    };
    let margin = amazing_layout::Rect {
        x: border.x - margin_sides.left,
        y: border.y - margin_sides.top,
        w: border.w + margin_sides.left + margin_sides.right,
        h: border.h + margin_sides.top + margin_sides.bottom,
    };
    BoxModel {
        content: quad_from_rect(&content),
        padding: quad_from_rect(&padding),
        border: quad_from_rect(&border),
        margin: quad_from_rect(&margin),
        width: border.w.round() as i64,
        height: border.h.round() as i64,
    }
}

/// Converts one a11y-lane node into the CDP-shaped seam, matching the
/// pattern `amazing-devtools::domains::accessibility` documents: this crate
/// stays independent of `amazing-a11y`, so the conversion lives entirely on
/// the driver side. Shared by [`convert_ax_tree`] (the whole-tree case) and
/// CDP-25's `getPartialAXTree`/`getChildAXNodes`/`queryAXTree`/
/// `Accessibility.loadComplete`, which each convert a handful of individual
/// nodes rather than a whole tree.
fn convert_ax_node(node: &amazing_a11y::AxNode) -> AxNode {
    AxNode {
        ax_node_id: node.id.0.to_string(),
        ignored: false,
        role: Some(node.role.as_str().to_owned()),
        name: (!node.name.is_empty()).then(|| node.name.clone()),
        description: (!node.description.is_empty()).then(|| node.description.clone()),
        value: None,
        node: node.dom_node.map(|idx| EngineNodeId(idx.0 as u64)),
        child_ids: node.children.iter().map(|id| id.0.to_string()).collect(),
    }
}

/// Converts the a11y lane's own tree into the CDP-shaped seam, root first
/// (`amazing_a11y::AxTree::nodes()`'s own preorder, per its doc comment).
fn convert_ax_tree(tree: &amazing_a11y::AxTree) -> Vec<AxNode> {
    tree.nodes().iter().map(convert_ax_node).collect()
}

/// The one piece of state that must live on a single thread: the live DOM and
/// a persistent JS realm bound to it. Not `Send` (the JS heap is `Rc`-based),
/// so it never leaves the thread [`spawn_page_thread`] gives it.
struct PageOwner {
    dom_host: Rc<RefCell<BrowserDomHost>>,
    scheduler: DriverScheduler,
    target_id: String,
    url: String,
    width: f32,
    /// How many lines of `scheduler.js.output` `drain_console` has already
    /// reported, so repeated polls only emit new `console.log` lines.
    console_read: usize,
    next_request_id: u64,
    next_object_id: u64,
    /// `Runtime.RemoteObject.objectId` registry for this realm
    /// (`Runtime.getProperties`/`callFunctionOn` handles), cleared on
    /// navigation along with the rest of `PageOwner`.
    objects: HashMap<String, amazing_js::value::JsValue>,
    /// `Runtime.releaseObjectGroup` (CDP-24): the `objectGroup` (or `""` for
    /// CDP's ungrouped default) each top-level `evaluate`/`callFunctionOn`
    /// result's `objectId` was minted under, so a whole group can be dropped
    /// at once. Only top-level results are tagged — `Runtime.getProperties`
    /// has no `objectGroup` parameter in real CDP either, so a property
    /// value's own handle stays ungrouped, same as real Chrome.
    object_groups: HashMap<String, String>,
    /// Response bodies by request id, for `Network.getResponseBody`. Only
    /// the document and `<script src>` fetches this driver instruments are
    /// present — see the module's "Known limitations".
    bodies: HashMap<String, ResourceContent>,
    network_requests: VecDeque<NetworkRequestEvent>,
    network_responses: VecDeque<NetworkResponseEvent>,
    network_finished: VecDeque<NetworkFinishedEvent>,
    network_failed: VecDeque<NetworkFailedEvent>,
    exceptions: VecDeque<ExceptionEvent>,
    logs: VecDeque<LogEntry>,
    /// Every script this page has parsed (classic, executed; or module,
    /// collected but not executed — see [`Self::load_scripts_leniently`]'s
    /// doc comment), keyed by the minted `script_id` in [`ScriptRecord`].
    scripts: Vec<ScriptRecord>,
    /// `Debugger.scriptParsed` events not yet drained.
    scripts_parsed: VecDeque<ScriptInfo>,
    next_script_id: u64,
    debugger_enabled: bool,
    /// Breakpoints by URL pattern, shared with `debug_hook` so the
    /// interpreter's per-statement callback can match against them without
    /// re-entering `PageOwner` (see [`DriverDebugHook`]'s doc comment).
    breakpoints: Rc<RefCell<Vec<BreakpointRecord>>>,
    next_breakpoint_id: u64,
    /// One-shot "pause at a statement matching this depth condition" request
    /// (CDP-23), also shared with the hook. `None` means no pause/step is
    /// armed. See [`StepMode`].
    step_request: Rc<RefCell<Option<StepMode>>>,
    /// The call-stack `depth` (see `debug_hook.rs`'s CDP-23 addition)
    /// recorded at the most recent pause/breakpoint hit — the reference
    /// point [`Self::debugger_step_over`]/[`Self::debugger_step_out`] compare
    /// against, since this driver never truly suspends and so has no live
    /// "current frame" to read a depth from when a step command arrives.
    last_paused_depth: Rc<RefCell<u32>>,
    /// `Debugger.paused` events the hook has queued, shared for the same
    /// reason as `breakpoints`. The `bool` is "this pause was forced by a
    /// `Debugger.pause`/step command" (as opposed to a breakpoint hit only),
    /// which [`Self::drain_debugger_paused`] uses to decide whether a
    /// breakpoint whose condition evaluates false still surfaces the pause
    /// (with `hit_breakpoints` cleared and `reason` downgraded to `"other"`)
    /// rather than being dropped outright.
    debugger_paused: Rc<RefCell<VecDeque<(bool, PausedEvent)>>>,
    /// `Network.setExtraHTTPHeaders`' current header set (CDP-21), applied
    /// to every future fetch this driver makes — the initial document fetch
    /// on a `Page.navigate` reload and every `<script src>` fetch
    /// (`fetch_and_record`). A full replace each call, matching CDP; empty
    /// by default. Not applied to the very first page load, since no CDP
    /// session exists yet to have set anything.
    extra_headers: Vec<(String, String)>,
    /// `Network.setCacheDisabled` (CDP-21). This driver has no HTTP cache of
    /// its own to bypass, so the real, observable effect is the same one a
    /// disabled cache produces over the wire in a real browser: every future
    /// fetch also carries `Cache-Control: no-cache` and `Pragma: no-cache`.
    cache_disabled: bool,
    /// `Emulation.setDeviceMetricsOverride`'s `width`/`height` (CDP-21),
    /// read through [`Self::effective_width`]/[`Self::effective_fixed_height`]
    /// everywhere this driver currently reads `self.width`/passes a fixed
    /// height into `paint_host_surface`. `None` (the common case) means "use
    /// the page's own base `width`"/"auto height from content" exactly as
    /// before this ticket.
    override_width: Option<f32>,
    override_fixed_height: Option<u32>,
    /// `deviceScaleFactor`/`mobile` are accepted and stored but not applied
    /// to anything — this driver's raster path has no DPI-scaling concept
    /// and no separate mobile layout mode to switch into. See the module's
    /// "Known limitations".
    device_scale_factor: f64,
    /// CDP-25: the a11y lane's own stateful watcher (A11Y-14), holding the
    /// tree from the most recent [`Self::refresh_ax_tree`] checkpoint plus a
    /// queue of the diff events every checkpoint since the last
    /// [`Self::drain_ax_nodes_updated`] has produced. A checkpoint is
    /// recorded on every `Accessibility.*` read (`enable`, `getFullAXTree`,
    /// `getPartialAXTree`, `getChildAXNodes`, `queryAXTree`) — this driver
    /// has no continuous mutation-observer hook of its own to key off
    /// instead (see the module's "Known limitations"), so "did the tree
    /// change" is only actually checked when a client asks to look at it,
    /// same as every other on-demand-recomputed piece of state in this file
    /// (style, layout, matched rules, …).
    ax_watcher: crate::runtime_host_a11y::AxTreeWatcher,
    /// One-shot `Accessibility.loadComplete` payload, queued by
    /// [`Self::accessibility_enabled`] and cleared by
    /// [`Self::drain_ax_load_complete`]. `None` when nothing is queued.
    ax_load_complete: Option<AxNode>,
}

/// One script this driver has seen, classic or module (see
/// [`PageOwner::load_scripts_leniently`]).
struct ScriptRecord {
    script_id: String,
    url: String,
    source: String,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
    hash: String,
}

/// A breakpoint set by `Debugger.setBreakpointByUrl`, matched against the
/// *URL* of whatever script is currently running (not a specific
/// `script_id`), so it also matches a script loaded after the breakpoint was
/// set, the same as real Chrome's persistent-by-url breakpoints.
///
/// `line` is stored in `amazing_js`'s own 1-based line numbering
/// (`ast::SourceLocation::line`, what [`DebugHook::on_statement`] reports),
/// not CDP's 0-based `lineNumber` — converted once at
/// [`PageOwner::set_breakpoint_by_url`], so the hot per-statement match in
/// [`DriverDebugHook::on_statement`] is a plain integer comparison.
struct BreakpointRecord {
    id: String,
    url: String,
    line: u32,
    column: Option<u32>,
    /// `Debugger.setBreakpointByUrl`'s `condition` (CDP-23), a JS expression
    /// evaluated in the page's global realm at
    /// [`PageOwner::drain_debugger_paused`] time (not by the hook itself —
    /// see [`DriverDebugHook`]'s doc comment on why it cannot reach the
    /// interpreter). `None` (the common case) means unconditional.
    condition: Option<String>,
}

/// What the next `on_statement` call should treat as "the step landed",
/// compared against that statement's own `depth` (`debug_hook.rs`'s CDP-23
/// addition — `Interpreter`'s own recursion-depth counter, incremented per
/// nested function call, decremented on return). Since this driver never
/// truly suspends anything (see the module's "Known limitations"), a step
/// command has no live call frame to read "the current depth" from; instead
/// it compares against `last_paused_depth`, the depth recorded at the most
/// recent pause/breakpoint hit — the natural reference point for a step
/// issued right after one.
#[derive(Clone, Copy)]
enum StepMode {
    /// `Debugger.pause` and `stepInto`: the very next statement, at any
    /// depth, qualifies.
    Any,
    /// `stepOver`: a statement *deeper* than the reference depth is "inside
    /// a call this step is stepping over", so it does not qualify — only
    /// `depth <= reference` does (this frame continuing, or having just
    /// returned to a shallower one).
    Over(u32),
    /// `stepOut`: only `depth < reference` qualifies — the frame at
    /// `reference` must have actually returned.
    Out(u32),
}

impl StepMode {
    /// Whether a statement at `depth` satisfies this step request.
    fn matches(self, depth: u32) -> bool {
        match self {
            StepMode::Any => true,
            StepMode::Over(reference) => depth <= reference,
            StepMode::Out(reference) => depth < reference,
        }
    }
}

/// CDP-20's driver-side [`DebugHook`] implementation. Lives on the
/// `amazing-js` `Interpreter` (via `Rc<RefCell<dyn DebugHook>>`) but shares
/// its actual state with [`PageOwner`] through plain `Rc<RefCell<_>>` cells
/// rather than by calling back into `PageOwner` methods — the interpreter
/// calls this hook from deep inside a `PageOwner` method that already holds
/// `&mut self`, so a callback that tried to re-enter `PageOwner` would need
/// a second `&mut` borrow the borrow checker (rightly) will not allow. Only
/// plain data flows through these cells; no engine or `PageOwner` state is
/// reachable from here.
struct DriverDebugHook {
    breakpoints: Rc<RefCell<Vec<BreakpointRecord>>>,
    step_request: Rc<RefCell<Option<StepMode>>>,
    last_paused_depth: Rc<RefCell<u32>>,
    paused: Rc<RefCell<VecDeque<(bool, PausedEvent)>>>,
}

impl DebugHook for DriverDebugHook {
    fn on_statement(&mut self, script_id: &str, url: &str, line: u32, depth: u32) {
        let hit_ids: Vec<String> = self
            .breakpoints
            .borrow()
            .iter()
            .filter(|bp| bp.url == url && bp.line == line)
            .map(|bp| bp.id.clone())
            .collect();
        let step_hit = {
            let mut step = self.step_request.borrow_mut();
            let hit = step.is_some_and(|mode| mode.matches(depth));
            if hit {
                *step = None;
            }
            hit
        };
        if hit_ids.is_empty() && !step_hit {
            return;
        }
        *self.last_paused_depth.borrow_mut() = depth;
        let frame = CallFrame {
            call_frame_id: "cf0".to_owned(),
            function_name: String::new(),
            // `line` is `amazing_js`'s 1-based line; CDP's `Location.lineNumber` is 0-based.
            location: ScriptLocation { script_id: script_id.to_owned(), line_number: line.saturating_sub(1), column_number: None },
            url: url.to_owned(),
            // Filled in by `PageOwner::drain_debugger_paused` (CDP-22), which
            // has the interpreter/object-registry access this hook does not
            // (see this struct's own doc comment on why state only flows
            // through plain data here).
            scope_chain: Vec::new(),
        };
        let reason = if hit_ids.is_empty() { "other" } else { "Breakpoint" };
        self.paused.borrow_mut().push_back((
            step_hit,
            PausedEvent { call_frames: vec![frame], reason: reason.to_owned(), hit_breakpoints: hit_ids },
        ));
    }
}

/// A double-quoted JS string literal for `text`, for `Input.insertText`
/// (CDP-21) to splice into a small synthesized script. Escapes only what a
/// typed string realistically needs (backslash, double quote, and the line
/// terminators JS disallows unescaped in a string literal) — not a general
/// JSON/JS-literal encoder.
fn js_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Builds the request-header list CDP-21's `Network.setExtraHTTPHeaders`/
/// `setCacheDisabled` add to every future fetch: `extra` first (a client's
/// own headers take precedence in position, matching how a real browser
/// merges them), then `Cache-Control: no-cache`/`Pragma: no-cache` if the
/// cache is disabled — the same two headers a real browser adds over the
/// wire for "disable cache", used here since this driver has no HTTP cache
/// of its own to actually bypass.
fn request_headers_for(extra: &[(String, String)], cache_disabled: bool) -> Vec<(String, String)> {
    let mut headers = extra.to_vec();
    if cache_disabled {
        headers.push(("Cache-Control".to_owned(), "no-cache".to_owned()));
        headers.push(("Pragma".to_owned(), "no-cache".to_owned()));
    }
    headers
}

/// A deterministic content hash for `Debugger.scriptParsed`'s `hash` field.
/// Uses `std`'s own `SipHash` (`DefaultHasher`) rather than a real
/// cryptographic digest — this workspace's engine crates take zero external
/// dependencies, and CDP only needs *a* stable value a client can compare
/// across edits, not V8's own hash algorithm.
fn script_hash(source: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// `(end_line, end_column)` of `source`, 0-based and relative to the
/// script's own text (this driver has no HTML-node source-position tracking
/// to offset it into its containing document — see the module's "Known
/// limitations").
fn script_extent(source: &str) -> (u32, u32) {
    let lines: Vec<&str> = source.split('\n').collect();
    let end_line = lines.len().saturating_sub(1) as u32;
    let end_column = lines.last().map(|line| line.chars().count()).unwrap_or(0) as u32;
    (end_line, end_column)
}

impl PageOwner {
    /// Builds a page owner around `dom` with a fresh, empty JS realm and no
    /// accumulated events — the common tail of [`Self::from_parsed`] and
    /// [`Self::fetch_and_load`], factored out so both can seed network
    /// events (for `fetch_and_load`'s document fetch) before scripts run.
    fn empty(target_id: &str, url: &str, width: f32, dom: amazing_dom::Dom) -> Result<Self, HostError> {
        let dom_host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(dom_host.clone(), crate::js_scheduler::JS_CAPACITY)
            .map_err(|error| HostError::Internal(error.describe()))?;
        let breakpoints = Rc::new(RefCell::new(Vec::new()));
        let step_request = Rc::new(RefCell::new(None));
        let last_paused_depth = Rc::new(RefCell::new(0));
        let debugger_paused = Rc::new(RefCell::new(VecDeque::new()));
        let hook: amazing_js::debug_hook::SharedDebugHook = Rc::new(RefCell::new(DriverDebugHook {
            breakpoints: breakpoints.clone(),
            step_request: step_request.clone(),
            last_paused_depth: last_paused_depth.clone(),
            paused: debugger_paused.clone(),
        }));
        scheduler.js.set_debug_hook(Some(hook));
        Ok(Self {
            dom_host,
            scheduler,
            target_id: target_id.to_owned(),
            url: url.to_owned(),
            width,
            console_read: 0,
            next_request_id: 0,
            next_object_id: 0,
            objects: HashMap::new(),
            object_groups: HashMap::new(),
            bodies: HashMap::new(),
            network_requests: VecDeque::new(),
            network_responses: VecDeque::new(),
            network_finished: VecDeque::new(),
            network_failed: VecDeque::new(),
            exceptions: VecDeque::new(),
            logs: VecDeque::new(),
            scripts: Vec::new(),
            scripts_parsed: VecDeque::new(),
            next_script_id: 0,
            debugger_enabled: false,
            breakpoints,
            next_breakpoint_id: 0,
            step_request,
            last_paused_depth,
            debugger_paused,
            extra_headers: Vec::new(),
            cache_disabled: false,
            override_width: None,
            override_fixed_height: None,
            device_scale_factor: 1.0,
            ax_watcher: crate::runtime_host_a11y::AxTreeWatcher::new(),
            ax_load_complete: None,
        })
    }

    /// Builds a page owner from an already-parsed document: installs the JS
    /// realm, then runs every `<script>`/inline handler and dispatches
    /// `DOMContentLoaded`/`load` *leniently* (CDP-18): a thrown script,
    /// handler or event-loop step is reported as `Runtime.exceptionThrown`
    /// plus a `Log.entryAdded` instead of failing the whole page load, so a
    /// DevTools frontend can see the page's own bugs rather than losing the
    /// realm. `<script src>` fetches are instrumented as real `Network.*`
    /// events (document + script fetches; see the module's "Known
    /// limitations" for the stylesheet/image gap).
    fn from_parsed(target_id: &str, url: &str, dom: amazing_dom::Dom, width: f32) -> Result<Self, HostError> {
        let mut page = Self::empty(target_id, url, width, dom)?;
        page.scheduler.set_location_url(url);
        page.load_scripts_leniently();
        Ok(page)
    }

    /// Fetches `url` as the top-level document (instrumented as a real
    /// `Network.requestWillBeSent`/`responseReceived`/`loadingFinished` or
    /// `loadingFailed` triple/pair), parses it, then runs its scripts exactly
    /// as [`Self::from_parsed`] does. `extra_headers`/`cache_disabled` (CDP-21)
    /// carry a prior `Network.setExtraHTTPHeaders`/`setCacheDisabled` state
    /// across a `Page.navigate` reload into both this document fetch and
    /// every `<script src>` fetch [`Self::load_scripts_leniently`] makes for
    /// the freshly loaded page (`request_headers_for`) — the very first page
    /// load passes `&[]`/`false`, since no CDP session exists yet to have
    /// set anything.
    fn fetch_and_load(target_id: &str, url: &str, width: f32, extra_headers: &[(String, String)], cache_disabled: bool) -> Result<Self, HostError> {
        let mut network_requests = VecDeque::new();
        let mut network_responses = VecDeque::new();
        let mut network_finished = VecDeque::new();
        let mut network_failed = VecDeque::new();
        let mut bodies = HashMap::new();

        let request_id = "req-1".to_owned();
        let request_headers = request_headers_for(extra_headers, cache_disabled);
        network_requests.push_back(NetworkRequestEvent {
            request_id: request_id.clone(),
            url: url.to_owned(),
            method: "GET".into(),
            headers: request_headers.clone(),
            post_data: None,
            resource_type: "Document".into(),
            frame_id: target_id.to_owned(),
            loader_id: target_id.to_owned(),
            timestamp: monotonic_seconds(),
            wall_time: wall_time_seconds(),
        });

        let options = amazing_net::FetchOptions {
            max_response_bytes: 16 * 1024 * 1024,
            timeout: std::time::Duration::from_secs(15),
            request_headers,
            ..amazing_net::FetchOptions::default()
        };
        let response = match amazing_net::fetch(url, &options) {
            Ok(response) if response.is_success() => response,
            Ok(response) => {
                let message = format!("HTTP {} {}", response.status, response.reason);
                network_failed.push_back(NetworkFailedEvent {
                    request_id,
                    resource_type: "Document".into(),
                    error_text: message.clone(),
                    canceled: false,
                    timestamp: monotonic_seconds(),
                });
                return Err(HostError::Internal(message));
            }
            Err(error) => {
                let message = error.to_string();
                network_failed.push_back(NetworkFailedEvent {
                    request_id,
                    resource_type: "Document".into(),
                    error_text: message.clone(),
                    canceled: false,
                    timestamp: monotonic_seconds(),
                });
                return Err(HostError::Internal(message));
            }
        };
        let mime_type = mime_type_of(&response.headers);
        network_responses.push_back(NetworkResponseEvent {
            request_id: request_id.clone(),
            url: response.url.clone(),
            status: response.status,
            status_text: response.reason.clone(),
            mime_type: mime_type.clone(),
            headers: headers_from(&response.headers),
            encoded_data_length: response.body.len() as u64,
            from_disk_cache: false,
            resource_type: "Document".into(),
            timestamp: monotonic_seconds(),
        });
        network_finished.push_back(NetworkFinishedEvent {
            request_id: request_id.clone(),
            encoded_data_length: response.body.len() as u64,
            decoded_body_length: response.body.len() as u64,
            timestamp: monotonic_seconds(),
        });
        bodies.insert(
            request_id,
            ResourceContent { mime_type: mime_type.clone(), bytes: response.body.clone() },
        );

        let content_type = (!mime_type.is_empty()).then_some(mime_type.as_str());
        let document = crate::document::parse_document_bytes(&response.body, &response.url, content_type)
            .map_err(|error| HostError::Internal(error.to_string()))?;

        let mut page = Self::empty(target_id, &response.url, width, document.dom)?;
        page.next_request_id = 1;
        page.network_requests = network_requests;
        page.network_responses = network_responses;
        page.network_finished = network_finished;
        page.network_failed = network_failed;
        page.bodies = bodies;
        page.extra_headers = extra_headers.to_vec();
        page.cache_disabled = cache_disabled;
        page.scheduler.set_location_url(&page.url);
        page.load_scripts_leniently();
        Ok(page)
    }

    /// Runs every `<script>`/inline handler, dispatches
    /// `DOMContentLoaded`/`load` and pumps the bounded event loop, collecting
    /// failures as exceptions/log entries instead of aborting — mirrors
    /// `cmd::js::execute_page_scripts_lenient`'s logic but keeps this page's
    /// own persistent `scheduler` rather than building (and dropping) its
    /// own, since CDP needs the realm to survive past page load.
    fn load_scripts_leniently(&mut self) {
        let (mut scripts, handlers) = {
            let host = self.dom_host.borrow();
            let mut scripts = Vec::new();
            crate::cmd::js::collect_script_text(&host.dom, amazing_dom::NodeIdx(0), &mut scripts);
            let mut handlers = Vec::new();
            crate::cmd::js::collect_inline_handlers(&host.dom, amazing_dom::NodeIdx(0), &mut handlers);
            (scripts, handlers)
        };
        for (node, event, body) in handlers {
            if let Err(thrown) = self.scheduler.install_inline_handler(node, &event, &body) {
                self.record_load_error(format!("inline {event} handler on node {node}: {}", thrown.describe()));
            }
        }
        let parser_session = match self.scheduler.begin_parser_session(amazing_dom::NodeIdx(0)) {
            Ok(session) => session,
            Err(thrown) => {
                self.record_load_error(format!("parser session: {}", thrown.describe()));
                return;
            }
        };
        if let Err(thrown) = self.scheduler.parser_eof(amazing_dom::NodeIdx(0)) {
            self.record_load_error(format!("parser EOF: {}", thrown.describe()));
        }
        scripts.sort_by_key(|script| script.kind == crate::cmd::js::ScriptKind::Module);
        for script in scripts {
            if !self.scheduler.parser_session_current(parser_session) { break; }
            let (source, url) = match &script.src {
                Some(src) => {
                    let script_url = crate::cmd::js::resolve_script_url(&self.url, &src);
                    match self.fetch_and_record(&script_url, "Script") {
                        Ok(text) => (text, script_url),
                        Err(()) => continue, // already recorded as a network failure
                    }
                }
                None => (script.body.clone(), self.url.clone()),
            };
            let script_id = self.register_script(&url, &source);
            self.scheduler.js.set_current_script(script_id, url.clone());
            if let Err(thrown) = self.scheduler.run_page_script(&script, &source, &url) {
                self.record_load_error(format!("page script: {}", thrown.describe()));
            }
        }
        if self.scheduler.parser_session_current(parser_session)
            && let Err(thrown) = self.scheduler.finish_parser_document(amazing_dom::NodeIdx(0)) {
            self.record_load_error(format!("parser completion: {}", thrown.describe()));
        }
        for _ in 0..16 {
            match self.scheduler.pump_at(std::time::Duration::from_millis(16), true) {
                Ok(true) => {}
                Ok(false) => break,
                Err(thrown) => {
                    self.record_load_error(format!("page event loop: {}", thrown.describe()));
                    break;
                }
            }
        }
    }

    /// Fetches `url` as a subresource of the page (currently only
    /// `<script src>`), recording `Network.requestWillBeSent` plus either a
    /// `responseReceived`/`loadingFinished` pair or a `loadingFailed` (with a
    /// matching `Log.entryAdded`). Returns the body decoded as text on
    /// success.
    fn fetch_and_record(&mut self, url: &str, resource_type: &str) -> Result<String, ()> {
        let request_id = self.record_request(url, "GET", resource_type);
        let options = amazing_net::FetchOptions {
            request_headers: request_headers_for(&self.extra_headers, self.cache_disabled),
            ..amazing_net::FetchOptions::default()
        };
        match amazing_net::fetch(url, &options) {
            Ok(response) if response.is_success() => {
                let text = response.text().unwrap_or_default();
                self.record_response(&request_id, &response, resource_type);
                Ok(text)
            }
            Ok(response) => {
                let message = format!("HTTP {} {}", response.status, response.reason);
                self.record_failure(&request_id, resource_type, &message);
                self.record_load_error(format!("{resource_type} fetch {url}: {message}"));
                Err(())
            }
            Err(error) => {
                let message = error.to_string();
                self.record_failure(&request_id, resource_type, &message);
                self.record_load_error(format!("{resource_type} fetch {url}: {message}"));
                Err(())
            }
        }
    }

    fn record_request(&mut self, url: &str, method: &str, resource_type: &str) -> String {
        self.next_request_id += 1;
        let request_id = format!("req-{}", self.next_request_id);
        self.network_requests.push_back(NetworkRequestEvent {
            request_id: request_id.clone(),
            url: url.to_owned(),
            method: method.to_owned(),
            headers: Vec::new(),
            post_data: None,
            resource_type: resource_type.to_owned(),
            frame_id: self.target_id.clone(),
            loader_id: self.target_id.clone(),
            timestamp: monotonic_seconds(),
            wall_time: wall_time_seconds(),
        });
        request_id
    }

    fn record_response(&mut self, request_id: &str, response: &amazing_net::Response, resource_type: &str) {
        let mime_type = mime_type_of(&response.headers);
        self.network_responses.push_back(NetworkResponseEvent {
            request_id: request_id.to_owned(),
            url: response.url.clone(),
            status: response.status,
            status_text: response.reason.clone(),
            mime_type: mime_type.clone(),
            headers: headers_from(&response.headers),
            encoded_data_length: response.body.len() as u64,
            from_disk_cache: false,
            resource_type: resource_type.to_owned(),
            timestamp: monotonic_seconds(),
        });
        self.network_finished.push_back(NetworkFinishedEvent {
            request_id: request_id.to_owned(),
            encoded_data_length: response.body.len() as u64,
            decoded_body_length: response.body.len() as u64,
            timestamp: monotonic_seconds(),
        });
        self.bodies.insert(request_id.to_owned(), ResourceContent { mime_type, bytes: response.body.clone() });
    }

    fn record_failure(&mut self, request_id: &str, resource_type: &str, error_text: &str) {
        self.network_failed.push_back(NetworkFailedEvent {
            request_id: request_id.to_owned(),
            resource_type: resource_type.to_owned(),
            error_text: error_text.to_owned(),
            canceled: false,
            timestamp: monotonic_seconds(),
        });
        self.logs.push_back(LogEntry {
            source: "network".into(),
            level: "error".into(),
            text: format!("Failed to load resource: {error_text}"),
            timestamp: monotonic_seconds(),
            url: None,
            line_number: None,
            stack_trace: None,
        });
    }

    /// Records a page-load-time failure (a script/handler/event-loop error
    /// that isn't itself a live JS exception value) as both
    /// `Runtime.exceptionThrown` and `Log.entryAdded`, matching a real
    /// browser surfacing an uncaught error through both the console and the
    /// Issues-adjacent Log domain.
    fn record_load_error(&mut self, text: String) {
        self.exceptions.push_back(ExceptionEvent {
            details: ExceptionDetails {
                text: text.clone(),
                line_number: 0,
                column_number: 0,
                url: Some(self.url.clone()),
                exception: None,
                stack_trace: None,
            },
            timestamp: monotonic_seconds(),
        });
        self.logs.push_back(LogEntry {
            source: "javascript".into(),
            level: "error".into(),
            text,
            timestamp: monotonic_seconds(),
            url: Some(self.url.clone()),
            line_number: None,
            stack_trace: None,
        });
    }

    /// Records a live `Throw`n JS value (from a mouse/key event listener) as
    /// `Runtime.exceptionThrown` plus `Log.entryAdded`, carrying the thrown
    /// value itself as the exception's `RemoteObject` (unlike
    /// `record_load_error`, which has no live value to attach).
    fn record_exception(&mut self, thrown: &amazing_js::value::Throw) {
        let text = thrown.describe();
        let exception_object = self.remote_object(&thrown.0);
        self.exceptions.push_back(ExceptionEvent {
            details: ExceptionDetails {
                text: text.clone(),
                line_number: 0,
                column_number: 0,
                url: None,
                exception: Some(exception_object),
                stack_trace: None,
            },
            timestamp: monotonic_seconds(),
        });
        self.logs.push_back(LogEntry {
            source: "javascript".into(),
            level: "error".into(),
            text,
            timestamp: monotonic_seconds(),
            url: None,
            line_number: None,
            stack_trace: None,
        });
    }

    /// Converts an engine value into a CDP `RemoteObject`, minting and
    /// registering a real `objectId` for `JsValue::Object` so
    /// `Runtime.getProperties`/`callFunctionOn` can resolve it back later.
    /// `BigInt`/`Symbol` still only get a debug description (the JSON model
    /// can't carry either verbatim) — see the module's "Known limitations".
    fn remote_object(&mut self, value: &amazing_js::value::JsValue) -> RemoteObject {
        use amazing_js::value::JsValue;
        match value {
            JsValue::Undefined => RemoteObject::undefined(),
            JsValue::Null => RemoteObject::null(),
            JsValue::Bool(b) => RemoteObject::boolean(*b),
            JsValue::Num(n) => RemoteObject::number(*n),
            JsValue::Str(s) => RemoteObject::string(s.to_utf8_lossy()),
            JsValue::BigInt(_) | JsValue::Symbol(_) => RemoteObject {
                object_type: if matches!(value, JsValue::BigInt(_)) { "bigint" } else { "symbol" }.to_string(),
                description: Some(format!("{value:?}")),
                ..RemoteObject::undefined()
            },
            JsValue::Object(_) => {
                self.next_object_id += 1;
                let object_id = format!("object:{}", self.next_object_id);
                self.objects.insert(object_id.clone(), value.clone());
                RemoteObject {
                    object_type: if self.scheduler.js.binding_is_callable(value) { "function" } else { "object" }.to_string(),
                    description: Some(format!("{value:?}")),
                    object_id: Some(object_id),
                    ..RemoteObject::undefined()
                }
            }
        }
    }

    /// CDP-22 completeness fix: `Runtime.evaluate`/`callFunctionOn`'s
    /// `returnByValue: true` was accepted (`EvaluateParams`/
    /// `CallFunctionParams` both carry the field) but never honored — every
    /// object result got an `objectId` reference regardless. This serializes
    /// a plain object/array structurally into `RemoteObject::value` instead,
    /// matching CDP's "serialize instead of a handle" semantics; no
    /// `objectId` is minted, since real Chrome does not hand out a reference
    /// for a by-value result either. Primitives already carry `value`
    /// unconditionally (see [`Self::remote_object`]), so they pass through
    /// unchanged.
    fn remote_object_by_value(&mut self, value: &amazing_js::value::JsValue) -> RemoteObject {
        use amazing_js::value::JsValue;
        match value {
            JsValue::Object(_) => {
                let is_array = self.js_is_array(value);
                let json = self.js_value_to_json(value, 0);
                RemoteObject {
                    object_type: "object".to_owned(),
                    subtype: if is_array { Some("array".to_owned()) } else { None },
                    description: Some(format!("{value:?}")),
                    value: Some(json),
                    ..RemoteObject::undefined()
                }
            }
            other => self.remote_object(other),
        }
    }

    /// `Array.isArray(value)`, run through the exact same `run_script` +
    /// `binding_call` idiom `Runtime.callFunctionOn` already uses to invoke
    /// a function against a live value — there is no `binding_is_array` in
    /// `amazing-js`'s own binding surface, so this reuses real JS instead of
    /// asking js-runtime for a new one.
    fn js_is_array(&mut self, value: &amazing_js::value::JsValue) -> bool {
        use amazing_js::value::JsValue;
        match self.scheduler.run_script("Array.isArray") {
            Ok(is_array_fn) => matches!(
                self.scheduler.js.binding_call(&is_array_fn, JsValue::Undefined, vec![value.clone()]),
                Ok(JsValue::Bool(true))
            ),
            Err(_) => false,
        }
    }

    /// Structurally serializes `value` for [`Self::remote_object_by_value`].
    /// Depth-capped at 5 rather than cycle-tracked — an honest safety net
    /// against a self-referential object (this driver has no pointer-
    /// identity check for `JsValue::Object`'s `Rc`), not a full structured-
    /// clone. Arrays walk `length`/indices (real element order); plain
    /// objects walk own *enumerable* string-keyed properties through
    /// getters, matching what `JSON.stringify` would produce — non-
    /// enumerable properties and `BigInt`/`Symbol` values are dropped/
    /// nulled the same way `JSON.stringify` drops or throws on them (nulled
    /// here rather than erroring, an intentionally lenient choice for a
    /// debugging tool).
    fn js_value_to_json(&mut self, value: &amazing_js::value::JsValue, depth: u32) -> amazing_devtools::json::Json {
        use amazing_devtools::json::Json;
        use amazing_js::value::JsValue;
        match value {
            JsValue::Undefined | JsValue::Null => Json::Null,
            JsValue::Bool(b) => Json::Bool(*b),
            JsValue::Num(n) => Json::number(*n),
            JsValue::Str(s) => Json::String(s.to_utf8_lossy()),
            JsValue::BigInt(_) | JsValue::Symbol(_) => Json::Null,
            JsValue::Object(_) => {
                if depth >= 5 {
                    return Json::Null;
                }
                if self.js_is_array(value) {
                    let length = match self.scheduler.js.binding_get_property(value, "length") {
                        Ok(JsValue::Num(n)) if n.is_finite() && n >= 0.0 => n as usize,
                        _ => 0,
                    };
                    let mut items = Vec::with_capacity(length.min(10_000));
                    for index in 0..length {
                        let item = self
                            .scheduler
                            .js
                            .binding_get_property(value, &index.to_string())
                            .unwrap_or(JsValue::Undefined);
                        items.push(self.js_value_to_json(&item, depth + 1));
                    }
                    Json::Array(items)
                } else {
                    let keys = self.scheduler.js.binding_own_property_keys(value).unwrap_or_default();
                    let mut fields = Vec::with_capacity(keys.len());
                    for key in keys {
                        let property = match self.scheduler.js.binding_get_own_property(value, &key) {
                            Ok(Some(property)) if property.enumerable => property,
                            _ => continue,
                        };
                        let field_value = match &property.getter {
                            Some(getter) => self
                                .scheduler
                                .js
                                .binding_call(getter, value.clone(), Vec::new())
                                .unwrap_or(JsValue::Undefined),
                            None if property.is_accessor() => JsValue::Undefined,
                            None => property.value.clone(),
                        };
                        fields.push((key, self.js_value_to_json(&field_value, depth + 1)));
                    }
                    Json::Object(fields)
                }
            }
        }
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let override_width = self.override_width;
        let override_fixed_height = self.override_fixed_height;
        let device_scale_factor = self.device_scale_factor;
        let mut reloaded = Self::fetch_and_load(&self.target_id, url, self.width, &self.extra_headers, self.cache_disabled)?;
        // CDP-21: `Network.setExtraHTTPHeaders`/`setCacheDisabled` already
        // carried into the fresh document/script fetches above (and land on
        // `reloaded` itself via `fetch_and_load`'s own assignment); a device
        // metrics override has no fetch to ride along with, so it is simply
        // restored here — real Chrome persists it across navigation the
        // same way.
        reloaded.override_width = override_width;
        reloaded.override_fixed_height = override_fixed_height;
        reloaded.device_scale_factor = device_scale_factor;
        *self = reloaded;
        Ok(frame_info(&self.target_id, &self.url))
    }

    fn frame_tree(&self) -> FrameTree {
        FrameTree { frame: frame_info(&self.target_id, &self.url), child_frames: Vec::new() }
    }

    fn layout_metrics(&self) -> Result<LayoutMetrics, HostError> {
        let width = self.effective_width();
        let surface = crate::cmd::render::paint_host_surface(&self.dom_host, &self.url, width, self.effective_fixed_height(), "")
            .map_err(|error| HostError::Internal(error.to_string()))?;
        let viewport = Viewport {
            page_x: 0.0,
            page_y: 0.0,
            client_width: width as f64,
            client_height: surface.height as f64,
        };
        Ok(LayoutMetrics {
            layout_viewport: viewport,
            visual_viewport: viewport,
            content_width: surface.width as f64,
            content_height: surface.height as f64,
        })
    }

    /// Runs `expression` in this page's persistent realm. Unlike the first
    /// slice, this is the *same* `DriverScheduler` (and so the same globals
    /// and closures) across every call for this page's lifetime.
    ///
    /// CDP-24: `generatePreview` attaches an [`ObjectPreview`] to the result
    /// (see [`Self::object_preview`]); `objectGroup` tags any minted
    /// `objectId` so [`Self::release_object_group`] can drop it later (see
    /// [`Self::tag_object_group`]). Neither applies to a `returnByValue`
    /// result — it has no `objectId` to tag or preview to attach beyond its
    /// own serialized `value`.
    fn evaluate(&mut self, params: &EvaluateParams) -> EvaluateResult {
        match self.scheduler.run_script(&params.expression) {
            Ok(value) => {
                let mut object =
                    if params.return_by_value { self.remote_object_by_value(&value) } else { self.remote_object(&value) };
                if params.generate_preview {
                    object.preview = self.object_preview(&value);
                }
                self.tag_object_group(&object, params.object_group.as_deref());
                EvaluateResult::value(object)
            }
            Err(thrown) => {
                let exception_object = self.remote_object(&thrown.0);
                self.tag_object_group(&exception_object, params.object_group.as_deref());
                EvaluateResult {
                    result: exception_object.clone(),
                    exception: Some(ExceptionDetails {
                        text: thrown.describe(),
                        line_number: 0,
                        column_number: 0,
                        url: None,
                        exception: Some(exception_object),
                        stack_trace: None,
                    }),
                }
            }
        }
    }

    /// Tags `object`'s `objectId` (if any) with `group` — `""` for CDP's
    /// ungrouped default when no `objectGroup` was given — so
    /// [`Self::release_object_group`] can find it later. A primitive or
    /// by-value result has no `objectId` and this is a no-op for it.
    fn tag_object_group(&mut self, object: &RemoteObject, group: Option<&str>) {
        if let Some(id) = &object.object_id {
            self.object_groups.insert(id.clone(), group.unwrap_or("").to_owned());
        }
    }

    /// Builds a shallow `Runtime.ObjectPreview` for `value` (CDP-24): `None`
    /// for a non-object — primitives don't carry a `.preview` in real CDP
    /// either. Caps at 5 own enumerable properties (matching real Chrome's
    /// own default), setting `overflow: true` when more exist. Each
    /// property's value is summarized by [`Self::property_preview`] as a
    /// primitive type/short string only — a nested object/function property
    /// gets no rendered value (an honest one-level-deep limit: a recursive
    /// preview risks the same self-referential-object problem
    /// `remote_object_by_value`'s depth cap already works around, and this
    /// driver has no cycle/pointer-identity check for `JsValue::Object`'s
    /// `Rc` to lean on instead).
    fn object_preview(&mut self, value: &amazing_js::value::JsValue) -> Option<ObjectPreview> {
        use amazing_js::value::JsValue;
        if !matches!(value, JsValue::Object(_)) {
            return None;
        }
        const PREVIEW_LIMIT: usize = 5;
        let object_type = if self.scheduler.js.binding_is_callable(value) { "function" } else { "object" }.to_owned();
        let subtype = if self.js_is_array(value) { Some("array".to_owned()) } else { None };
        let description = Some(format!("{value:?}"));
        let keys = match self.scheduler.js.binding_own_property_keys(value) {
            Ok(keys) => keys,
            Err(_) => return Some(ObjectPreview { object_type, subtype, description, overflow: false, properties: Vec::new() }),
        };
        let overflow = keys.len() > PREVIEW_LIMIT;
        let mut properties = Vec::with_capacity(PREVIEW_LIMIT.min(keys.len()));
        for key in keys.into_iter().take(PREVIEW_LIMIT) {
            let property = match self.scheduler.js.binding_get_own_property(value, &key) {
                Ok(Some(property)) if property.enumerable => property,
                _ => continue,
            };
            let property_value = match &property.getter {
                Some(getter) => {
                    self.scheduler.js.binding_call(getter, value.clone(), Vec::new()).unwrap_or(JsValue::Undefined)
                }
                None if property.is_accessor() => JsValue::Undefined,
                None => property.value.clone(),
            };
            properties.push(self.property_preview(key, &property_value));
        }
        Some(ObjectPreview { object_type, subtype, description, overflow, properties })
    }

    /// One [`ObjectPreview::properties`] entry for `value` under `name` —
    /// see [`Self::object_preview`]'s doc comment on why a nested object
    /// gets no rendered value.
    fn property_preview(&mut self, name: String, value: &amazing_js::value::JsValue) -> PropertyPreview {
        use amazing_js::value::JsValue;
        let (object_type, rendered) = match value {
            JsValue::Undefined => ("undefined".to_owned(), Some("undefined".to_owned())),
            JsValue::Null => ("object".to_owned(), Some("null".to_owned())),
            JsValue::Bool(b) => ("boolean".to_owned(), Some(b.to_string())),
            JsValue::Num(n) => ("number".to_owned(), Some(amazing_devtools::json::Json::Number(*n).to_string())),
            JsValue::Str(s) => ("string".to_owned(), Some(s.to_utf8_lossy())),
            JsValue::BigInt(_) => ("bigint".to_owned(), Some(format!("{value:?}"))),
            JsValue::Symbol(_) => ("symbol".to_owned(), Some(format!("{value:?}"))),
            JsValue::Object(_) => {
                (if self.scheduler.js.binding_is_callable(value) { "function" } else { "object" }.to_owned(), None)
            }
        };
        PropertyPreview { name, object_type, value: rendered }
    }

    /// The properties [`Self::get_properties`] reports for one level of an
    /// object's own-property table (either `object_id`'s own properties, or
    /// one prototype's own properties during the chain walk below), each
    /// tagged `is_own` for the caller. Accessor properties are read through
    /// their getter, matching what a DevTools Properties panel shows.
    ///
    /// CDP-24: an object-typed value always gets a [`Self::object_preview`]
    /// attached — real `Runtime.getProperties` has no `generatePreview`
    /// parameter of its own to gate this on, unlike `evaluate`/
    /// `callFunctionOn`.
    fn own_properties(&mut self, object: &amazing_js::value::JsValue, is_own: bool) -> Result<Vec<PropertyDescriptor>, HostError> {
        let keys = self
            .scheduler
            .js
            .binding_own_property_keys(object)
            .map_err(|thrown| HostError::Internal(thrown.describe()))?;
        let mut properties = Vec::with_capacity(keys.len());
        for key in keys {
            let property = match self.scheduler.js.binding_get_own_property(object, &key) {
                Ok(Some(property)) => property,
                Ok(None) => continue,
                Err(thrown) => return Err(HostError::Internal(thrown.describe())),
            };
            let value = match &property.getter {
                Some(getter) => self
                    .scheduler
                    .js
                    .binding_call(getter, object.clone(), Vec::new())
                    .unwrap_or(amazing_js::value::JsValue::Undefined),
                None if property.is_accessor() => amazing_js::value::JsValue::Undefined,
                None => property.value.clone(),
            };
            let mut remote = self.remote_object(&value);
            remote.preview = self.object_preview(&value);
            properties.push(PropertyDescriptor {
                name: key,
                value: remote,
                writable: property.writable,
                configurable: property.configurable,
                enumerable: property.enumerable,
                is_own,
            });
        }
        Ok(properties)
    }

    /// `Runtime.getProperties`. `own_only=true` (CDP's `ownProperties: true`,
    /// the common "expand this object" case) lists just `object_id`'s own
    /// properties (`is_own: true`). `own_only=false` (CDP-22 completeness
    /// fix) also walks the prototype chain, one level at a time
    /// (`is_own: false` on each), stopping at `null`/a non-object or after
    /// 20 levels — a safety net against a corrupted/cyclic chain, not a real
    /// spec limit. There is no `[[Prototype]]` accessor in `amazing-js`'s own
    /// binding surface for an arbitrary object (only
    /// `binding_object_prototype`/`binding_set_global_prototype`, both for
    /// the *global* prototype specifically — see `interp_bindings.rs`), so
    /// the walk reuses real JS (`Object.getPrototypeOf`) through the same
    /// `run_script` + `binding_call` idiom `call_function_on` already uses,
    /// rather than asking js-runtime for a new binding.
    fn get_properties(&mut self, object_id: &str, own_only: bool) -> Result<Vec<PropertyDescriptor>, HostError> {
        let object = self.objects.get(object_id).cloned().ok_or(HostError::NotFound)?;
        let mut properties = self.own_properties(&object, true)?;
        if !own_only {
            let get_prototype = self
                .scheduler
                .run_script("(function() { return Object.getPrototypeOf(this); })")
                .map_err(|thrown| HostError::Internal(thrown.describe()))?;
            let mut current = object;
            for _ in 0..20 {
                let prototype = match self.scheduler.js.binding_call(&get_prototype, current.clone(), Vec::new()) {
                    Ok(value @ amazing_js::value::JsValue::Object(_)) => value,
                    _ => break,
                };
                properties.extend(self.own_properties(&prototype, false)?);
                current = prototype;
            }
        }
        Ok(properties)
    }

    /// `Runtime.callFunctionOn`: evaluates `function_declaration` (a function
    /// expression's source, wrapped in parens so it evaluates to the
    /// function itself rather than being parsed as a statement), resolves
    /// `this` from `object_id` (or the global object), converts each
    /// argument (`object_id` handles resolve to the live value; JSON
    /// `value`s go through [`json_to_js_value`]) and invokes it through the
    /// same `binding_call` embedding entry point `get_properties` uses for
    /// accessors. `returnByValue` (CDP-22 completeness fix) is honored the
    /// same way `evaluate` honors it — see [`Self::remote_object_by_value`].
    /// CDP-24: `Runtime.callFunctionOn` has no `generatePreview` parameter
    /// in real CDP (only `evaluate` does, matching `CallFunctionParams`'s
    /// own fields — no `generate_preview` there), so this driver never
    /// attaches a preview to its result. `objectGroup` still tags any
    /// minted `objectId`, same as `evaluate`.
    fn call_function_on(&mut self, params: &CallFunctionParams) -> Result<EvaluateResult, HostError> {
        let this = match &params.object_id {
            Some(id) => self.objects.get(id).cloned().unwrap_or(amazing_js::value::JsValue::Undefined),
            None => self.scheduler.js.binding_global_object(),
        };
        let function = match self.scheduler.run_script(&format!("({})", params.function_declaration)) {
            Ok(value) => value,
            Err(thrown) => return Ok(self.evaluate_exception(thrown, params.object_group.as_deref())),
        };
        let mut args = Vec::with_capacity(params.arguments.len());
        for argument in &params.arguments {
            args.push(self.call_argument_value(argument));
        }
        match self.scheduler.js.binding_call(&function, this, args) {
            Ok(value) => {
                let object =
                    if params.return_by_value { self.remote_object_by_value(&value) } else { self.remote_object(&value) };
                self.tag_object_group(&object, params.object_group.as_deref());
                Ok(EvaluateResult::value(object))
            }
            Err(thrown) => Ok(self.evaluate_exception(thrown, params.object_group.as_deref())),
        }
    }

    fn call_argument_value(&mut self, argument: &CallArgument) -> amazing_js::value::JsValue {
        if let Some(object_id) = &argument.object_id {
            return self.objects.get(object_id).cloned().unwrap_or(amazing_js::value::JsValue::Undefined);
        }
        match &argument.value {
            Some(value) => json_to_js_value(value),
            None => amazing_js::value::JsValue::Undefined,
        }
    }

    fn evaluate_exception(&mut self, thrown: amazing_js::value::Throw, object_group: Option<&str>) -> EvaluateResult {
        let exception_object = self.remote_object(&thrown.0);
        self.tag_object_group(&exception_object, object_group);
        EvaluateResult {
            result: exception_object.clone(),
            exception: Some(ExceptionDetails {
                text: thrown.describe(),
                line_number: 0,
                column_number: 0,
                url: None,
                exception: Some(exception_object),
                stack_trace: None,
            }),
        }
    }

    fn release_object(&mut self, object_id: &str) {
        self.objects.remove(object_id);
        self.object_groups.remove(object_id);
    }

    /// `Runtime.releaseObjectGroup` (CDP-24): drops every `objectId` minted
    /// under `group` by a top-level `evaluate`/`callFunctionOn` result (see
    /// [`Self::tag_object_group`]). `group == ""` releases CDP's ungrouped
    /// default bucket — every result whose call omitted `objectGroup`.
    fn release_object_group(&mut self, group: &str) {
        let ids: Vec<String> =
            self.object_groups.iter().filter(|(_, g)| g.as_str() == group).map(|(id, _)| id.clone()).collect();
        for id in ids {
            self.objects.remove(&id);
            self.object_groups.remove(&id);
        }
    }

    fn get_response_body(&self, request_id: &str) -> Option<ResourceContent> {
        self.bodies.get(request_id).cloned()
    }

    fn drain_console(&mut self) -> Vec<ConsoleEvent> {
        // `native_log` (the only console method the engine builtin-implements
        // — no `warn`/`error`/`info`/`debug`) already joins every argument
        // into one line, so `kind` is always `"log"` and each event carries a
        // single string `RemoteObject` rather than per-argument objects; see
        // the module's "Known limitations".
        let total = self.scheduler.js.output.borrow().len();
        if self.console_read >= total {
            return Vec::new();
        }
        let events: Vec<ConsoleEvent> = self.scheduler.js.output.borrow()[self.console_read..total]
            .iter()
            .map(|line| ConsoleEvent {
                kind: "log".into(),
                args: vec![RemoteObject::string(line.clone())],
                execution_context_id: 1,
                timestamp: monotonic_seconds(),
            })
            .collect();
        self.console_read = total;
        events
    }

    fn drain_network_requests(&mut self) -> Vec<NetworkRequestEvent> {
        self.network_requests.drain(..).collect()
    }

    fn drain_network_responses(&mut self) -> Vec<NetworkResponseEvent> {
        self.network_responses.drain(..).collect()
    }

    fn drain_network_finished(&mut self) -> Vec<NetworkFinishedEvent> {
        self.network_finished.drain(..).collect()
    }

    fn drain_network_failed(&mut self) -> Vec<NetworkFailedEvent> {
        self.network_failed.drain(..).collect()
    }

    fn drain_exceptions(&mut self) -> Vec<ExceptionEvent> {
        self.exceptions.drain(..).collect()
    }

    fn drain_log(&mut self) -> Vec<LogEntry> {
        self.logs.drain(..).collect()
    }

    fn get_document(&self, depth: u32) -> NodeDescription {
        let host = self.dom_host.borrow();
        describe_node(&host.dom, amazing_dom::NodeIdx(0), depth)
    }

    fn query_selector(&self, node: EngineNodeId, selector: &str) -> Result<Option<EngineNodeId>, HostError> {
        let parsed = parse_selector_list(selector)?;
        let host = self.dom_host.borrow();
        Ok(find_first(&host.dom, amazing_dom::NodeIdx(node.0 as u32), &parsed))
    }

    fn query_selector_all(&self, node: EngineNodeId, selector: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let parsed = parse_selector_list(selector)?;
        let host = self.dom_host.borrow();
        let mut out = Vec::new();
        find_all(&host.dom, amazing_dom::NodeIdx(node.0 as u32), &parsed, &mut out);
        Ok(out)
    }

    fn describe_node(&self, node: EngineNodeId, depth: u32) -> NodeDescription {
        let host = self.dom_host.borrow();
        describe_node(&host.dom, amazing_dom::NodeIdx(node.0 as u32), depth)
    }

    fn get_outer_html(&self, node: EngineNodeId) -> Result<String, HostError> {
        let host = self.dom_host.borrow();
        outer_html(&host.dom, amazing_dom::NodeIdx(node.0 as u32))
    }

    fn set_attribute_value(&mut self, node: EngineNodeId, name: &str, value: Option<&str>) -> Result<(), HostError> {
        let mut host = self.dom_host.borrow_mut();
        let idx = amazing_dom::NodeIdx(node.0 as u32);
        let result = match value {
            Some(value) => host.dom.set_attribute(idx, name, value),
            None => host.dom.remove_attribute(idx, name),
        };
        result.map_err(|error| HostError::Internal(format!("{error:?}")))
    }

    /// Reparses `html` in the parent element's tag-name context (falling
    /// back to `"body"` when the parent isn't an element, e.g. the document
    /// root), splices the result in at `node`'s original position, then
    /// detaches `node`. See the module doc's "Known limitations".
    fn set_outer_html(&mut self, node: EngineNodeId, html: &str) -> Result<(), HostError> {
        let mut host = self.dom_host.borrow_mut();
        let idx = amazing_dom::NodeIdx(node.0 as u32);
        let entry = host.dom.get(idx);
        let parent = entry
            .parent
            .ok_or_else(|| HostError::InvalidParams("node has no parent to splice into".into()))?;
        let next_sibling = entry.next_sibling;
        let context = match &host.dom.get(parent).kind {
            amazing_dom::NodeKind::Element { name, .. } => name.clone(),
            _ => "body".to_owned(),
        };
        let fragment = amazing_html::parse_fragment(&context, html)
            .map_err(HostError::Internal)?;
        let children: Vec<amazing_dom::NodeIdx> = fragment.children(amazing_dom::NodeIdx(0)).collect();
        for child in children {
            let created = clone_subtree_into(&fragment, child, &mut host.dom, parent);
            host.dom
                .detach(created);
            host.dom
                .pre_insert(created, parent, next_sibling)
                .map_err(|error| HostError::Internal(format!("{error:?}")))?;
        }
        host.dom.detach(idx);
        Ok(())
    }

    fn remove_node(&mut self, node: EngineNodeId) -> Result<(), HostError> {
        let mut host = self.dom_host.borrow_mut();
        host.dom.detach(amazing_dom::NodeIdx(node.0 as u32));
        Ok(())
    }

    fn media_environment(&self) -> amazing_css::MediaEnvironment {
        amazing_css::MediaEnvironment { viewport_width_px: self.effective_width().max(1.0), ..amazing_css::MediaEnvironment::default() }
    }

    /// The width this page's layout/paint/media-query pipeline should
    /// actually use: `Emulation.setDeviceMetricsOverride`'s `width`
    /// (CDP-21) when active, else the page's own base `width`.
    fn effective_width(&self) -> f32 {
        self.override_width.unwrap_or(self.width)
    }

    /// The fixed viewport height `paint_host_surface` should render at
    /// (CDP-21's device metrics override), or `None` for the pre-existing
    /// "auto height from content" behavior every call site already used.
    fn effective_fixed_height(&self) -> Option<u32> {
        self.override_fixed_height
    }

    /// `Emulation.setDeviceMetricsOverride`. `height: 0` (a real, documented
    /// CDP value) keeps auto height rather than fixing it at `0`, matching
    /// real Chrome's own "don't override" convention for that field.
    /// `mobile`/`screen_*`/`position_*`/`dont_set_visible_size`/orientation
    /// are accepted but not applied — see the module's "Known limitations".
    fn set_device_metrics_override(&mut self, metrics: &DeviceMetrics) -> Result<(), HostError> {
        self.override_width = Some(metrics.width as f32);
        self.override_fixed_height = if metrics.height == 0 { None } else { Some(metrics.height) };
        self.device_scale_factor = metrics.device_scale_factor;
        Ok(())
    }

    fn clear_device_metrics_override(&mut self) -> Result<(), HostError> {
        self.override_width = None;
        self.override_fixed_height = None;
        self.device_scale_factor = 1.0;
        Ok(())
    }

    /// `Network.setCacheDisabled`. See the `cache_disabled` field's doc
    /// comment for what this driver can actually do about it.
    fn set_cache_disabled(&mut self, disabled: bool) -> Result<(), HostError> {
        self.cache_disabled = disabled;
        Ok(())
    }

    /// `Network.setExtraHTTPHeaders`: a full replace, matching CDP.
    fn set_extra_http_headers(&mut self, headers: &[(String, String)]) -> Result<(), HostError> {
        self.extra_headers = headers.to_vec();
        Ok(())
    }

    /// `Input.insertText`: appends `text` to the focused input/textarea's
    /// own `value` (there is no caret/selection model to insert at a
    /// specific position, so appending at the end is this driver's honest
    /// simplification) and dispatches a real `input` event, through the
    /// engine's own JS bindings rather than reimplementing value-setting
    /// and event dispatch here — `document.activeElement`, the `value`
    /// setter, `new Event`, and `dispatchEvent` are all real, spec-backed
    /// bindings this reuses as-is. Returns `HostError::Unsupported` when the
    /// focused element has no `value` (e.g. nothing is focused, or a
    /// `contenteditable` element — real rich-text insertion is out of scope;
    /// see the module's "Known limitations").
    fn insert_text(&mut self, text: &str) -> Result<(), HostError> {
        use amazing_js::value::JsValue;
        let script = format!(
            "(() => {{ const el = document.activeElement; if (!el || !('value' in el)) return false; \
             el.value = (el.value || '') + {literal}; \
             el.dispatchEvent(new Event('input', {{ bubbles: true }})); return true; }})()",
            literal = js_string_literal(text),
        );
        match self.scheduler.run_script(&script) {
            Ok(JsValue::Bool(false)) => Err(HostError::Unsupported),
            Ok(_) => Ok(()),
            Err(thrown) => {
                self.record_exception(&thrown);
                Err(HostError::Internal(thrown.describe()))
            }
        }
    }

    /// The page's own `<style>`/`<link rel=stylesheet>` sheets, in document
    /// order — the same collection `full_ax_tree` and `layout_metrics` (via
    /// `paint_host_surface`) use.
    fn author_sheets(&self, dom: &amazing_dom::Dom) -> Vec<amazing_css::Stylesheet> {
        let mut sources = Vec::new();
        let _ = crate::page::collect_author_css(dom, amazing_dom::NodeIdx(0), &self.url, &mut sources);
        sources.into_iter().map(|(_, _, sheet)| sheet).collect()
    }

    fn computed_style_for_node(&self, node: EngineNodeId) -> Result<Vec<(String, String)>, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let (sheets, page_text) = crate::font_metrics::load(dom, &self.url);
        let styled = crate::font_metrics::compute(dom, &sheets, &self.media_environment(), None, &page_text.provider);
        let style = styled
            .get(amazing_dom::NodeIdx(node.0 as u32))
            .ok_or(HostError::NotFound)?;
        Ok(style.iter().map(|(name, value)| (name.to_owned(), value.to_owned())).collect())
    }

    /// Matched rules from the page's own author stylesheets. Every match is
    /// reported as `StyleOrigin::Author` — see the module's "Known
    /// limitations" on the missing user-agent sheet.
    fn matched_styles_for_node(&self, node: EngineNodeId) -> Result<Vec<MatchedRule>, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let idx = amazing_dom::NodeIdx(node.0 as u32);
        let sheets = self.author_sheets(dom);
        let mut matched = Vec::new();
        for sheet in &sheets {
            for rule in &sheet.rules {
                for selector_src in &rule.selector_srcs {
                    let Some(selector) = amazing_css::selectors::parse_selector(selector_src) else { continue };
                    if amazing_css::selectors::matches(dom, idx, &selector) {
                        matched.push(MatchedRule {
                            selector: selector_src.clone(),
                            origin: StyleOrigin::Author,
                            properties: rule
                                .declarations
                                .iter()
                                .map(|decl| (decl.property.clone(), decl.value.clone()))
                                .collect(),
                        });
                    }
                }
            }
        }
        Ok(matched)
    }

    /// `style_sheet_id` is the 0-based index into this page's own collected
    /// author stylesheets (see the module's "Known limitations"); the text is
    /// re-serialized from parsed rules, not the original source bytes.
    fn style_sheet_text(&self, style_sheet_id: &str) -> Result<Option<String>, HostError> {
        let index: usize = style_sheet_id
            .parse()
            .map_err(|_| HostError::InvalidParams(format!("bad styleSheetId: {style_sheet_id}")))?;
        let host = self.dom_host.borrow();
        let sheets = self.author_sheets(&host.dom);
        let Some(sheet) = sheets.get(index) else { return Ok(None) };
        let mut text = String::new();
        for rule in &sheet.rules {
            if rule.selector_srcs.is_empty() {
                continue;
            }
            text.push_str(&rule.selector_srcs.join(", "));
            text.push_str(" {\n");
            for decl in &rule.declarations {
                text.push_str("  ");
                text.push_str(&decl.property);
                text.push_str(": ");
                text.push_str(&decl.value);
                if decl.important {
                    text.push_str(" !important");
                }
                text.push_str(";\n");
            }
            text.push_str("}\n");
        }
        Ok(Some(text))
    }

    /// Computes style and layout using the registered page font provider, then
    /// hit-tests it. Returns `HostError::NotFound` when `(x, y)` misses every
    /// box (e.g. outside the page).
    fn hit_test_node(&self, x: f64, y: f64) -> Result<EngineNodeId, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let (sheets, page_text) = crate::font_metrics::load(dom, &self.url);
        let styled = crate::font_metrics::compute(dom, &sheets, &self.media_environment(), None, &page_text.provider);
        let text = &page_text.provider;
        let layout = amazing_layout::layout_styled_document(
            dom,
            &styled,
            text,
            &amazing_layout::NoReplacedElements,
            self.effective_width(),
            self.effective_fixed_height().map(|h| h as f32),
        );
        hit_test(&layout, x as f32, y as f32)
            .and_then(|box_| box_.node)
            .map(|node| EngineNodeId(node.0 as u64))
            .ok_or(HostError::NotFound)
    }

    /// Computes style and the same page-font layout `hit_test_node`/
    /// `full_ax_tree` use, then finds `node`'s own laid-out box
    /// (`find_box_for_node`) and converts it to a CDP box model
    /// (`DOM.getBoxModel`). `HostError::NotFound` covers both an unknown
    /// node and a node with no box of its own (`display: none`, or one that
    /// only contributes to an inline line box — see the module's "Known
    /// limitations").
    fn get_box_model(&self, node: EngineNodeId) -> Result<BoxModel, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let (sheets, page_text) = crate::font_metrics::load(dom, &self.url);
        let styled = crate::font_metrics::compute(dom, &sheets, &self.media_environment(), None, &page_text.provider);
        let text = &page_text.provider;
        let layout = amazing_layout::layout_styled_document(
            dom,
            &styled,
            text,
            &amazing_layout::NoReplacedElements,
            self.effective_width(),
            self.effective_fixed_height().map(|h| h as f32),
        );
        let target = amazing_dom::NodeIdx(node.0 as u32);
        let box_ = find_box_for_node(&layout, target).ok_or(HostError::NotFound)?;
        Ok(box_model_from(box_))
    }

    /// `DOM.getContentQuads`: this driver does not split inline content into
    /// one quad per line box (the same `Flow::Inlines` gap `get_box_model`
    /// documents), so every node gets exactly the one border-box quad
    /// `getBoxModel` would also report.
    fn get_content_quads(&self, node: EngineNodeId) -> Result<Vec<BoxModel>, HostError> {
        Ok(vec![self.get_box_model(node)?])
    }

    /// `Overlay.highlightNode`: no compositor overlay layer exists in this
    /// engine yet (see the module's "Known limitations"), so there is
    /// nothing to paint — but the command still validates that `node`
    /// resolves to real box geometry (the same computation `getBoxModel`
    /// uses), reporting `HostError::NotFound` for an unknown or box-less
    /// node exactly as a real highlight request would fail.
    fn highlight_node(&self, node: EngineNodeId) -> Result<(), HostError> {
        self.get_box_model(node).map(|_| ())
    }

    /// Mints a `script_id` and records a [`ScriptRecord`] (source kept for
    /// `get_script_source`). Only queues a live `Debugger.scriptParsed` if
    /// the domain is *already* enabled — a script parsed before the client
    /// ever calls `Debugger.enable` (i.e. every script on this driver's
    /// synchronous initial page load) is reported once, by that enable
    /// call's own backfill, not twice.
    fn register_script(&mut self, url: &str, source: &str) -> String {
        self.next_script_id += 1;
        let script_id = format!("script-{}", self.next_script_id);
        let (end_line, end_column) = script_extent(source);
        let hash = script_hash(source);
        self.scripts.push(ScriptRecord {
            script_id: script_id.clone(),
            url: url.to_owned(),
            source: source.to_owned(),
            start_line: 0,
            start_column: 0,
            end_line,
            end_column,
            hash: hash.clone(),
        });
        if self.debugger_enabled {
            self.scripts_parsed.push_back(ScriptInfo {
                script_id: script_id.clone(),
                url: url.to_owned(),
                start_line: 0,
                start_column: 0,
                end_line,
                end_column,
                hash,
            });
        }
        script_id
    }

    /// `Debugger.enable`: backfills a `scriptParsed` for every script parsed
    /// before the client enabled the domain, matching real Chrome (a client
    /// that enables `Debugger` after `Page.navigate` still sees every
    /// script). Re-enabling is a no-op; disabling just stops the flag —
    /// breakpoints and their bookkeeping are left in place, since real
    /// Chrome only deactivates them for the *session*, and this driver has
    /// only one session per page anyway.
    fn debugger_enabled(&mut self, enabled: bool) -> Result<(), HostError> {
        if enabled && !self.debugger_enabled {
            for script in &self.scripts {
                self.scripts_parsed.push_back(ScriptInfo {
                    script_id: script.script_id.clone(),
                    url: script.url.clone(),
                    start_line: script.start_line,
                    start_column: script.start_column,
                    end_line: script.end_line,
                    end_column: script.end_column,
                    hash: script.hash.clone(),
                });
            }
        }
        self.debugger_enabled = enabled;
        Ok(())
    }

    /// `Debugger.setBreakpointByUrl` at line granularity. `url` is matched
    /// literally (no `urlRegex` support) against every script's own
    /// `ScriptRecord::url`; a script that has not loaded yet can still match
    /// once it does, since the debug hook re-checks `breakpoints` by URL on
    /// every statement rather than resolving once at set time. `locations`
    /// only reports scripts that exist *right now* and whose source actually
    /// has that many lines — matching real Chrome, which can return an empty
    /// `locations` array for an unresolved breakpoint without that being an
    /// error. `line_number` arrives 0-based (CDP convention); stored as
    /// `amazing_js`'s 1-based line so [`DriverDebugHook::on_statement`] can
    /// compare it directly against `ast::SourceLocation::line`. `condition`
    /// (CDP-23) is stored as-is and evaluated later, at
    /// [`Self::drain_debugger_paused`] time — see [`BreakpointRecord`]'s doc
    /// comment for why the hook itself cannot evaluate it.
    fn set_breakpoint_by_url(
        &mut self,
        url: &str,
        line_number: u32,
        column_number: Option<u32>,
        condition: Option<&str>,
    ) -> Result<(String, Vec<ScriptLocation>), HostError> {
        self.next_breakpoint_id += 1;
        let id = format!("bp-{}", self.next_breakpoint_id);
        self.breakpoints.borrow_mut().push(BreakpointRecord {
            id: id.clone(),
            url: url.to_owned(),
            line: line_number + 1,
            column: column_number,
            condition: condition.map(str::to_owned),
        });
        let locations = self
            .scripts
            .iter()
            .filter(|script| script.url == url && line_number <= script.end_line)
            .map(|script| ScriptLocation { script_id: script.script_id.clone(), line_number, column_number })
            .collect();
        Ok((id, locations))
    }

    fn remove_breakpoint(&mut self, breakpoint_id: &str) {
        self.breakpoints.borrow_mut().retain(|bp| bp.id != breakpoint_id);
    }

    /// `Debugger.pause`: see `DriverPageSession::debugger_pause`'s doc
    /// comment for why this is "pause at the next statement that actually
    /// runs" rather than a true synchronous halt.
    fn debugger_pause(&mut self) -> Result<(), HostError> {
        *self.step_request.borrow_mut() = Some(StepMode::Any);
        Ok(())
    }

    fn debugger_resume(&mut self) -> Result<(), HostError> {
        *self.step_request.borrow_mut() = None;
        Ok(())
    }

    /// The depth [`StepMode::Over`]/[`StepMode::Out`] compare against: the
    /// depth recorded at the most recent pause/breakpoint hit (CDP-23). See
    /// [`StepMode`]'s own doc comment for why this stand-in, rather than a
    /// live "current frame", is what a step command has to work with.
    fn step_reference_depth(&self) -> u32 {
        *self.last_paused_depth.borrow()
    }

    /// `Debugger.stepInto` (CDP-23): pause at the very next statement, at
    /// any depth — the same one-shot request `debugger_pause` arms.
    fn debugger_step_into(&mut self) -> Result<(), HostError> {
        *self.step_request.borrow_mut() = Some(StepMode::Any);
        Ok(())
    }

    /// `Debugger.stepOver` (CDP-23, now real): pauses at the next statement
    /// whose call-stack `depth` is no deeper than the reference depth — see
    /// [`StepMode::Over`].
    fn debugger_step_over(&mut self) -> Result<(), HostError> {
        *self.step_request.borrow_mut() = Some(StepMode::Over(self.step_reference_depth()));
        Ok(())
    }

    /// `Debugger.stepOut` (CDP-23): pauses once the frame at the reference
    /// depth has actually returned — see [`StepMode::Out`].
    fn debugger_step_out(&mut self) -> Result<(), HostError> {
        *self.step_request.borrow_mut() = Some(StepMode::Out(self.step_reference_depth()));
        Ok(())
    }

    /// `Debugger.evaluateOnCallFrame` (CDP-23). This driver never truly
    /// suspends a call frame — see the module's "Known limitations" — and
    /// the only call frame id it ever mints is the literal `"cf0"`
    /// ([`DriverDebugHook::on_statement`]'s synthesized [`CallFrame`]), so
    /// there is exactly one frame to evaluate "in": the page's global realm,
    /// via the same path `Runtime.evaluate` uses. Any other id is a genuine
    /// `NotFound`, matching real Chrome for a stale/unknown frame id.
    fn evaluate_on_call_frame(&mut self, call_frame_id: &str, expression: &str) -> Result<EvaluateResult, HostError> {
        if call_frame_id != "cf0" {
            return Err(HostError::NotFound);
        }
        Ok(self.evaluate(&EvaluateParams { expression: expression.to_owned(), ..EvaluateParams::default() }))
    }

    fn get_script_source(&self, script_id: &str) -> Result<String, HostError> {
        self.scripts
            .iter()
            .find(|script| script.script_id == script_id)
            .map(|script| script.source.clone())
            .ok_or(HostError::NotFound)
    }

    fn drain_scripts_parsed(&mut self) -> Vec<ScriptInfo> {
        self.scripts_parsed.drain(..).collect()
    }

    /// Drains queued `Debugger.paused` events. Two enrichment passes happen
    /// here rather than in [`DriverDebugHook::on_statement`], because both
    /// need interpreter/scheduler access the hook does not have (see that
    /// struct's doc comment):
    ///
    /// 1. **Condition filtering (CDP-23).** For an event whose
    ///    `hit_breakpoints` are non-empty, each hit breakpoint's stored
    ///    `condition` (if any) is evaluated in the page's global realm via
    ///    the same `scheduler.run_script` path `Runtime.evaluate` uses; an
    ///    unconditional breakpoint always passes, and an erroring condition
    ///    counts as false (matching real Chrome, which never lets a bad
    ///    condition itself throw to the frontend). `hit_breakpoints` is
    ///    narrowed to only the ids that passed. If none passed: a purely
    ///    breakpoint-driven pause (not also a forced `Debugger.pause`/step)
    ///    is dropped entirely, exactly as if the breakpoint had not fired;
    ///    one that *was* also forced is kept but downgraded to
    ///    `reason: "other"` with an empty `hit_breakpoints`, since the step
    ///    itself still legitimately landed.
    /// 2. **`scope_chain` (CDP-22).** The only scope this driver can
    ///    honestly offer is `"global"`: this engine's tree-walking
    ///    interpreter has no reified per-call local/closure environment
    ///    records reachable from outside a running call, so there is no
    ///    real "local"/"closure" scope object to hand back yet — see the
    ///    module's "Known limitations" and the open `.agents/js-runtime/
    ///    questions.md` request. The global object itself is real and live:
    ///    it goes through the same `remote_object` registration
    ///    `Runtime.evaluate` uses, so `Runtime.getProperties` on it reflects
    ///    genuine `globalThis` state, not a snapshot.
    fn drain_debugger_paused(&mut self) -> Vec<PausedEvent> {
        let raw: Vec<(bool, PausedEvent)> = self.debugger_paused.borrow_mut().drain(..).collect();
        if raw.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::with_capacity(raw.len());
        for (forced, mut event) in raw {
            if !event.hit_breakpoints.is_empty() {
                let conditions: Vec<(String, Option<String>)> = {
                    let breakpoints = self.breakpoints.borrow();
                    event
                        .hit_breakpoints
                        .iter()
                        .filter_map(|id| breakpoints.iter().find(|bp| &bp.id == id).map(|bp| (id.clone(), bp.condition.clone())))
                        .collect()
                };
                let passing: Vec<String> = conditions
                    .into_iter()
                    .filter(|(_, condition)| match condition {
                        None => true,
                        Some(expr) => match self.scheduler.run_script(expr) {
                            Ok(value) => self.scheduler.js.binding_to_boolean(&value),
                            Err(_) => false,
                        },
                    })
                    .map(|(id, _)| id)
                    .collect();
                if passing.is_empty() && !forced {
                    continue;
                }
                event.hit_breakpoints = passing;
                if event.hit_breakpoints.is_empty() {
                    event.reason = "other".to_owned();
                }
            }
            events.push(event);
        }
        if events.is_empty() {
            return events;
        }
        let global_scope = match self.scheduler.run_script("globalThis") {
            Ok(value) => Some(Scope { scope_type: "global".to_owned(), object: self.remote_object(&value) }),
            Err(_) => None,
        };
        if let Some(scope) = global_scope {
            for event in &mut events {
                for frame in &mut event.call_frames {
                    frame.scope_chain = vec![scope.clone()];
                }
            }
        }
        events
    }

    fn dispatch_mouse_event(&mut self, event: &MouseEvent) -> Result<(), HostError> {
        let dom_event = match event.kind.as_str() {
            "mousePressed" => "mousedown",
            "mouseReleased" => "mouseup",
            "mouseMoved" => "mousemove",
            "mouseWheel" => "wheel",
            other => other,
        };
        let target = self.hit_test_node(event.x, event.y)?;
        // A listener's own bug does not fail the DevTools command itself,
        // matching real browser semantics: it is reported through
        // `Runtime.exceptionThrown`/the console instead (CDP-18).
        if let Err(thrown) = self.scheduler.dispatch_dom_event(target.0 as u32, dom_event) {
            self.record_exception(&thrown);
        }
        Ok(())
    }

    fn dispatch_key_event(&mut self, event: &KeyEvent) -> Result<(), HostError> {
        let dom_event = match event.kind.as_str() {
            "keyDown" | "rawKeyDown" => "keydown",
            "keyUp" => "keyup",
            "char" => "keypress",
            other => other,
        };
        let target = {
            let host = self.dom_host.borrow();
            host.dom
                .active_element(amazing_dom::NodeIdx(0))
                .unwrap_or(amazing_dom::NodeIdx(0))
        };
        if let Err(thrown) = self.scheduler.dispatch_dom_event(target.0, dom_event) {
            self.record_exception(&thrown);
        }
        Ok(())
    }

    /// Renders through the same `paint_host_surface` path `amazing
    /// render`/`render-page` use, then encodes the RGBA8 surface as PNG with
    /// `amazing_canvas::encode_png` (reused, not duplicated).
    fn capture_screenshot(&self, params: &ScreenshotParams) -> Result<Screenshot, HostError> {
        if !matches!(params.format, ScreenshotFormat::Png) {
            return Err(HostError::Unsupported);
        }
        let mut surface = crate::cmd::render::paint_host_surface(
            &self.dom_host,
            &self.url,
            self.effective_width(),
            self.effective_fixed_height(),
            "",
        )
        .map_err(|error| HostError::Internal(error.to_string()))?;
        if let Some(clip) = &params.clip {
            surface = crop_surface(&surface, clip);
        }
        let image = amazing_canvas::ImageData::from_rgba(surface.width, surface.height, surface.pixels)
            .map_err(|error| HostError::Internal(format!("{error:?}")))?;
        Ok(Screenshot { data: amazing_canvas::encode_png(&image), format: ScreenshotFormat::Png })
    }

    /// Computes style and layout with actual page fonts and
    /// `NoReplacedElements` on the live DOM and
    /// records a fresh checkpoint on [`Self::ax_watcher`] — building the
    /// tree fresh on the first call, or diffing it against whatever
    /// checkpoint came before on every later one (CDP-25's
    /// `AxTreeWatcher`, A11Y-14). Every `Accessibility.*` read funnels
    /// through this, so a client that calls, say, `getFullAXTree` twice
    /// with a DOM mutation in between sees that mutation reflected in the
    /// second call *and* gets a real `Accessibility.nodesUpdated` queued
    /// for [`Self::drain_ax_nodes_updated`] to report — this driver has no
    /// continuous mutation-observer hook of its own to key checkpoints off
    /// instead (see the module's "Known limitations"; filed originally as
    /// `.agents/devtools/questions.md`'s A11Y-8 hookup note, closed by this
    /// ticket at this granularity).
    fn refresh_ax_tree(&mut self) -> amazing_a11y::AxTree {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let (sheets, page_text) = crate::font_metrics::load(dom, &self.url);
        let styled = crate::font_metrics::compute(dom, &sheets, &self.media_environment(), None, &page_text.provider);
        let text = &page_text.provider;
        let layout = amazing_layout::layout_styled_document(
            dom,
            &styled,
            text,
            &amazing_layout::NoReplacedElements,
            self.effective_width(),
            self.effective_fixed_height().map(|h| h as f32),
        );
        self.ax_watcher.record_checkpoint(dom, &styled, &layout);
        drop(host);
        self.ax_watcher.current_tree().cloned().expect("record_checkpoint always leaves a current tree")
    }

    /// `Accessibility.getFullAXTree`. `depth` is accepted but not applied —
    /// the whole tree is always returned; see the module's "Known
    /// limitations" (unchanged behavior from before this ticket).
    fn full_ax_tree(&mut self) -> Result<Vec<AxNode>, HostError> {
        let tree = self.refresh_ax_tree();
        Ok(convert_ax_tree(&tree))
    }

    /// `Accessibility.enable`/`disable`. Enabling immediately builds (or
    /// rebuilds) the tree and queues its root for
    /// [`Self::drain_ax_load_complete`] — this driver's stand-in for real
    /// Chrome's page-load-keyed `Accessibility.loadComplete`, since it has
    /// no separate "page finished loading" signal to fire that from
    /// instead (see [`Self::ax_load_complete`]'s doc comment). Disabling
    /// clears anything still queued, matching a client that stopped
    /// listening.
    fn accessibility_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.ax_load_complete = None;
            return;
        }
        let tree = self.refresh_ax_tree();
        self.ax_load_complete = tree.root().and_then(|id| tree.node(id)).map(convert_ax_node);
    }

    /// `Accessibility.getPartialAXTree`: the subtree rooted at `node`
    /// (`AxTree::walk`'s preorder), plus — when `fetch_relatives` is set —
    /// every ancestor up to the root first, matching real CDP's "the node,
    /// its subtree, and (optionally) its ancestor chain" shape.
    fn partial_ax_tree(&mut self, node: EngineNodeId, fetch_relatives: bool) -> Result<Vec<AxNode>, HostError> {
        let tree = self.refresh_ax_tree();
        let ax_id = tree.find_dom(amazing_dom::NodeIdx(node.0 as u32)).ok_or(HostError::NotFound)?;
        let mut ids = Vec::new();
        if fetch_relatives {
            let mut ancestors = Vec::new();
            let mut current = tree.node(ax_id).and_then(|entry| entry.parent);
            while let Some(id) = current {
                ancestors.push(id);
                current = tree.node(id).and_then(|entry| entry.parent);
            }
            ancestors.reverse();
            ids.extend(ancestors);
        }
        ids.extend(tree.walk(Some(ax_id)));
        Ok(ids.into_iter().filter_map(|id| tree.node(id)).map(convert_ax_node).collect())
    }

    /// `Accessibility.getChildAXNodes`: the direct children of the node
    /// named by its string `AXNodeId`, each with its own `childIds` already
    /// filled in (so a client can keep expanding without another round
    /// trip per grandchild). `HostError::NotFound` covers both a malformed
    /// id and one from a stale/earlier tree.
    fn child_ax_nodes(&mut self, ax_node_id: &str) -> Result<Vec<AxNode>, HostError> {
        let tree = self.refresh_ax_tree();
        let raw: u32 = ax_node_id
            .parse()
            .map_err(|_| HostError::InvalidParams(format!("bad AXNodeId: {ax_node_id}")))?;
        let id = amazing_a11y::AxNodeId(raw);
        if tree.node(id).is_none() {
            return Err(HostError::NotFound);
        }
        Ok(tree.children(id).iter().filter_map(|child| tree.node(*child)).map(convert_ax_node).collect())
    }

    /// `Accessibility.queryAXTree`: nodes under `node` (`AxTree::walk`
    /// conveniently already defaults to the whole tree when its start is
    /// `None`, matching this method's own "the whole tree when node is
    /// `None`" contract for free) whose accessible name equals
    /// `accessible_name` (when given) and whose role matches `role`
    /// case-insensitively (when given) — real Chrome's own CDP role
    /// strings are already lowercase, so this only makes the match forgiving
    /// of a client that sends different casing, not a behavior change.
    fn query_ax_tree(
        &mut self,
        node: Option<EngineNodeId>,
        accessible_name: Option<&str>,
        role: Option<&str>,
    ) -> Result<Vec<AxNode>, HostError> {
        let tree = self.refresh_ax_tree();
        let start = match node {
            Some(node) => Some(tree.find_dom(amazing_dom::NodeIdx(node.0 as u32)).ok_or(HostError::NotFound)?),
            None => None,
        };
        Ok(tree
            .walk(start)
            .into_iter()
            .filter_map(|id| tree.node(id))
            .filter(|entry| accessible_name.is_none_or(|want| entry.name == want))
            .filter(|entry| role.is_none_or(|want| entry.role.as_str().eq_ignore_ascii_case(want)))
            .map(convert_ax_node)
            .collect())
    }

    /// `Accessibility.nodesUpdated`: every event [`Self::ax_watcher`] has
    /// queued since the last drain, minus `Removed` ones (real CDP's
    /// `nodesUpdated` has no removal concept of its own — DOM removal is
    /// signalled through `DOM.*` separately, matching the a11y lane's own
    /// `.agents/devtools/questions.md` hookup note), deduplicated by
    /// `AxNodeId` (a node whose name *and* role both changed in one
    /// checkpoint gets one `AxEvent` per aspect, but CDP wants one updated
    /// snapshot per node, not one per aspect) and resolved against the
    /// *current* tree rather than carried on the event itself, so a node
    /// that changed twice before ever being drained is reported with its
    /// latest values, not a stale intermediate one.
    fn drain_ax_nodes_updated(&mut self) -> Vec<AxNode> {
        let events = self.ax_watcher.drain_events();
        if events.is_empty() {
            return Vec::new();
        }
        let Some(tree) = self.ax_watcher.current_tree() else { return Vec::new() };
        let mut seen = std::collections::HashSet::new();
        events
            .iter()
            .filter(|event| event.kind != amazing_a11y::AxEventKind::Removed)
            .filter(|event| seen.insert(event.node))
            .filter_map(|event| tree.node(event.node))
            .map(convert_ax_node)
            .collect()
    }

    /// `Accessibility.loadComplete`: takes whatever [`Self::accessibility_enabled`]
    /// queued, once.
    fn drain_ax_load_complete(&mut self) -> Option<AxNode> {
        self.ax_load_complete.take()
    }
}

/// What [`spawn_page_thread`] needs to build the page's [`PageOwner`] — kept
/// separate from `PageOwner` itself so the *construction inputs* (plain
/// `Send` data: strings, an `amazing_dom::Dom`, a width) cross the thread
/// boundary in the `spawn` closure, while the engine state `PageOwner` builds
/// (the `Rc`-based JS heap) never does. `std::thread::spawn` requires the
/// closure to be `Send`, which only constrains what it *captures*, not what
/// it constructs after starting — so `PageOwner::from_parsed`/`fetch_and_load`
/// run inside the spawned thread, not before it.
enum PageInit {
    Parsed { target_id: String, url: String, dom: amazing_dom::Dom, width: f32 },
    Fetch { target_id: String, url: String, width: f32 },
}

impl PageInit {
    fn load(self) -> Result<PageOwner, HostError> {
        match self {
            PageInit::Parsed { target_id, url, dom, width } => PageOwner::from_parsed(&target_id, &url, dom, width),
            PageInit::Fetch { target_id, url, width } => PageOwner::fetch_and_load(&target_id, &url, width, &[], false),
        }
    }
}

fn run_page_thread(mut page: PageOwner, rx: Receiver<PageCommand>) {
    while let Ok(command) = rx.recv() {
        match command {
            PageCommand::Navigate(url, reply) => {
                let _ = reply.send(page.navigate(&url));
            }
            PageCommand::FrameTree(reply) => {
                let _ = reply.send(Ok(page.frame_tree()));
            }
            PageCommand::LayoutMetrics(reply) => {
                let _ = reply.send(page.layout_metrics());
            }
            PageCommand::Evaluate(params, reply) => {
                let _ = reply.send(Ok(page.evaluate(&params)));
            }
            PageCommand::GetDocument(depth, reply) => {
                let _ = reply.send(Ok(page.get_document(depth)));
            }
            PageCommand::QuerySelector(node, selector, reply) => {
                let _ = reply.send(page.query_selector(node, &selector));
            }
            PageCommand::QuerySelectorAll(node, selector, reply) => {
                let _ = reply.send(page.query_selector_all(node, &selector));
            }
            PageCommand::FullAxTree(reply) => {
                let _ = reply.send(page.full_ax_tree());
            }
            PageCommand::AccessibilityEnabled(enabled, reply) => {
                page.accessibility_enabled(enabled);
                let _ = reply.send(Ok(()));
            }
            PageCommand::PartialAxTree(node, fetch_relatives, reply) => {
                let _ = reply.send(page.partial_ax_tree(node, fetch_relatives));
            }
            PageCommand::ChildAxNodes(id, reply) => {
                let _ = reply.send(page.child_ax_nodes(&id));
            }
            PageCommand::QueryAxTree(node, name, role, reply) => {
                let _ = reply.send(page.query_ax_tree(node, name.as_deref(), role.as_deref()));
            }
            PageCommand::DrainAxNodesUpdated(reply) => {
                let _ = reply.send(Ok(page.drain_ax_nodes_updated()));
            }
            PageCommand::DrainAxLoadComplete(reply) => {
                let _ = reply.send(Ok(page.drain_ax_load_complete()));
            }
            PageCommand::DescribeNode(node, depth, reply) => {
                let _ = reply.send(Ok(page.describe_node(node, depth)));
            }
            PageCommand::GetOuterHtml(node, reply) => {
                let _ = reply.send(page.get_outer_html(node));
            }
            PageCommand::SetAttributeValue(node, name, value, reply) => {
                let _ = reply.send(page.set_attribute_value(node, &name, value.as_deref()));
            }
            PageCommand::SetOuterHtml(node, html, reply) => {
                let _ = reply.send(page.set_outer_html(node, &html));
            }
            PageCommand::RemoveNode(node, reply) => {
                let _ = reply.send(page.remove_node(node));
            }
            PageCommand::ComputedStyleForNode(node, reply) => {
                let _ = reply.send(page.computed_style_for_node(node));
            }
            PageCommand::MatchedStylesForNode(node, reply) => {
                let _ = reply.send(page.matched_styles_for_node(node));
            }
            PageCommand::StyleSheetText(style_sheet_id, reply) => {
                let _ = reply.send(page.style_sheet_text(&style_sheet_id));
            }
            PageCommand::DispatchMouseEvent(event, reply) => {
                let _ = reply.send(page.dispatch_mouse_event(&event));
            }
            PageCommand::DispatchKeyEvent(event, reply) => {
                let _ = reply.send(page.dispatch_key_event(&event));
            }
            PageCommand::CaptureScreenshot(params, reply) => {
                let _ = reply.send(page.capture_screenshot(&params));
            }
            PageCommand::GetProperties(object_id, own_only, reply) => {
                let _ = reply.send(page.get_properties(&object_id, own_only));
            }
            PageCommand::CallFunctionOn(params, reply) => {
                let _ = reply.send(page.call_function_on(&params));
            }
            PageCommand::ReleaseObject(object_id, reply) => {
                page.release_object(&object_id);
                let _ = reply.send(Ok(()));
            }
            PageCommand::ReleaseObjectGroup(group, reply) => {
                page.release_object_group(&group);
                let _ = reply.send(Ok(()));
            }
            PageCommand::GetResponseBody(request_id, reply) => {
                let _ = reply.send(Ok(page.get_response_body(&request_id)));
            }
            PageCommand::DrainConsole(reply) => {
                let _ = reply.send(Ok(page.drain_console()));
            }
            PageCommand::DrainNetworkRequests(reply) => {
                let _ = reply.send(Ok(page.drain_network_requests()));
            }
            PageCommand::DrainNetworkResponses(reply) => {
                let _ = reply.send(Ok(page.drain_network_responses()));
            }
            PageCommand::DrainNetworkFinished(reply) => {
                let _ = reply.send(Ok(page.drain_network_finished()));
            }
            PageCommand::DrainNetworkFailed(reply) => {
                let _ = reply.send(Ok(page.drain_network_failed()));
            }
            PageCommand::DrainExceptions(reply) => {
                let _ = reply.send(Ok(page.drain_exceptions()));
            }
            PageCommand::DrainLog(reply) => {
                let _ = reply.send(Ok(page.drain_log()));
            }
            PageCommand::GetBoxModel(node, reply) => {
                let _ = reply.send(page.get_box_model(node));
            }
            PageCommand::GetContentQuads(node, reply) => {
                let _ = reply.send(page.get_content_quads(node));
            }
            PageCommand::HighlightNode(node, reply) => {
                let _ = reply.send(page.highlight_node(node));
            }
            PageCommand::DebuggerEnabled(enabled, reply) => {
                let _ = reply.send(page.debugger_enabled(enabled));
            }
            PageCommand::SetBreakpointByUrl(url, line, column, condition, reply) => {
                let _ = reply.send(page.set_breakpoint_by_url(&url, line, column, condition.as_deref()));
            }
            PageCommand::RemoveBreakpoint(id, reply) => {
                page.remove_breakpoint(&id);
                let _ = reply.send(Ok(()));
            }
            PageCommand::DebuggerPause(reply) => {
                let _ = reply.send(page.debugger_pause());
            }
            PageCommand::DebuggerResume(reply) => {
                let _ = reply.send(page.debugger_resume());
            }
            PageCommand::DebuggerStepOver(reply) => {
                let _ = reply.send(page.debugger_step_over());
            }
            PageCommand::DebuggerStepInto(reply) => {
                let _ = reply.send(page.debugger_step_into());
            }
            PageCommand::DebuggerStepOut(reply) => {
                let _ = reply.send(page.debugger_step_out());
            }
            PageCommand::EvaluateOnCallFrame(call_frame_id, expression, reply) => {
                let _ = reply.send(page.evaluate_on_call_frame(&call_frame_id, &expression));
            }
            PageCommand::GetScriptSource(script_id, reply) => {
                let _ = reply.send(page.get_script_source(&script_id));
            }
            PageCommand::DrainScriptsParsed(reply) => {
                let _ = reply.send(Ok(page.drain_scripts_parsed()));
            }
            PageCommand::DrainDebuggerPaused(reply) => {
                let _ = reply.send(Ok(page.drain_debugger_paused()));
            }
            PageCommand::SetCacheDisabled(disabled, reply) => {
                let _ = reply.send(page.set_cache_disabled(disabled));
            }
            PageCommand::SetExtraHttpHeaders(headers, reply) => {
                let _ = reply.send(page.set_extra_http_headers(&headers));
            }
            PageCommand::SetDeviceMetricsOverride(metrics, reply) => {
                let _ = reply.send(page.set_device_metrics_override(&metrics));
            }
            PageCommand::ClearDeviceMetricsOverride(reply) => {
                let _ = reply.send(page.clear_device_metrics_override());
            }
            PageCommand::InsertText(text, reply) => {
                let _ = reply.send(page.insert_text(&text));
            }
            PageCommand::Shutdown => break,
        }
    }
}

/// Starts the dedicated page thread and returns the channel to talk to it,
/// plus the loaded page's final URL. Blocks until the initial
/// parse/script/load finishes (or fails), so a bad URL is reported to the
/// caller synchronously rather than silently wedging every later command. The
/// thread runs until it receives [`PageCommand::Shutdown`] (sent by
/// [`DriverDevToolsHost`]'s `Drop`), so it never outlives its host.
fn spawn_page_thread(init: PageInit) -> Result<(Sender<PageCommand>, String), HostError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<String, HostError>>();
    std::thread::Builder::new()
        .name("devtools-page".into())
        .spawn(move || match init.load() {
            Ok(page) => {
                let _ = ready_tx.send(Ok(page.url.clone()));
                run_page_thread(page, rx);
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })
        .expect("spawn devtools page thread");
    let url = ready_rx
        .recv()
        .map_err(|_| HostError::Internal("devtools page thread died before starting".into()))??;
    Ok((tx, url))
}

/// One attached page target: a `Send` handle to the page thread. It holds no
/// engine state itself, so it trivially satisfies `DevToolsSession: Send`
/// regardless of how many worker threads a `CdpServer` connection spawns.
pub(crate) struct DriverPageSession {
    target_id: String,
    sender: Sender<PageCommand>,
    current_url: Arc<Mutex<String>>,
}

impl DevToolsSession for DriverPageSession {
    fn target_id(&self) -> &str {
        &self.target_id
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let frame = request(&self.sender, |reply| PageCommand::Navigate(url.to_owned(), reply))?;
        if let Ok(mut current) = self.current_url.lock() {
            *current = frame.url.clone();
        }
        Ok(frame)
    }

    fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
        request(&self.sender, PageCommand::FrameTree)
    }

    fn get_layout_metrics(&mut self) -> Result<LayoutMetrics, HostError> {
        request(&self.sender, PageCommand::LayoutMetrics)
    }

    fn evaluate(&mut self, params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
        let params = params.clone();
        request(&self.sender, |reply| PageCommand::Evaluate(params, reply))
    }

    fn get_document(&mut self, depth: u32) -> Result<NodeDescription, HostError> {
        request(&self.sender, |reply| PageCommand::GetDocument(depth, reply))
    }

    fn query_selector(&mut self, node: EngineNodeId, selector: &str) -> Result<Option<EngineNodeId>, HostError> {
        let selector = selector.to_owned();
        request(&self.sender, |reply| PageCommand::QuerySelector(node, selector, reply))
    }

    fn query_selector_all(&mut self, node: EngineNodeId, selector: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let selector = selector.to_owned();
        request(&self.sender, |reply| PageCommand::QuerySelectorAll(node, selector, reply))
    }

    fn get_full_ax_tree(&mut self, _depth: Option<u32>) -> Result<Vec<AxNode>, HostError> {
        request(&self.sender, PageCommand::FullAxTree)
    }

    fn accessibility_enabled(&mut self, enabled: bool) -> Result<(), HostError> {
        request(&self.sender, |reply| PageCommand::AccessibilityEnabled(enabled, reply))
    }

    fn get_partial_ax_tree(&mut self, node: EngineNodeId, fetch_relatives: bool) -> Result<Vec<AxNode>, HostError> {
        request(&self.sender, |reply| PageCommand::PartialAxTree(node, fetch_relatives, reply))
    }

    fn get_child_ax_nodes(&mut self, ax_node_id: &str) -> Result<Vec<AxNode>, HostError> {
        let ax_node_id = ax_node_id.to_owned();
        request(&self.sender, |reply| PageCommand::ChildAxNodes(ax_node_id, reply))
    }

    fn query_ax_tree(
        &mut self,
        node: Option<EngineNodeId>,
        accessible_name: Option<&str>,
        role: Option<&str>,
    ) -> Result<Vec<AxNode>, HostError> {
        let accessible_name = accessible_name.map(str::to_owned);
        let role = role.map(str::to_owned);
        request(&self.sender, |reply| PageCommand::QueryAxTree(node, accessible_name, role, reply))
    }

    fn drain_ax_nodes_updated(&mut self) -> Vec<AxNode> {
        request(&self.sender, PageCommand::DrainAxNodesUpdated).unwrap_or_default()
    }

    fn drain_ax_load_complete(&mut self) -> Option<AxNode> {
        request(&self.sender, PageCommand::DrainAxLoadComplete).unwrap_or_default()
    }

    fn describe_node(&mut self, node: EngineNodeId, depth: u32) -> Result<NodeDescription, HostError> {
        request(&self.sender, |reply| PageCommand::DescribeNode(node, depth, reply))
    }

    fn get_outer_html(&mut self, node: EngineNodeId) -> Result<String, HostError> {
        request(&self.sender, |reply| PageCommand::GetOuterHtml(node, reply))
    }

    fn set_attribute_value(&mut self, node: EngineNodeId, name: &str, value: Option<&str>) -> Result<(), HostError> {
        let name = name.to_owned();
        let value = value.map(str::to_owned);
        request(&self.sender, |reply| PageCommand::SetAttributeValue(node, name, value, reply))
    }

    fn set_outer_html(&mut self, node: EngineNodeId, html: &str) -> Result<(), HostError> {
        let html = html.to_owned();
        request(&self.sender, |reply| PageCommand::SetOuterHtml(node, html, reply))
    }

    fn remove_node(&mut self, node: EngineNodeId) -> Result<(), HostError> {
        request(&self.sender, |reply| PageCommand::RemoveNode(node, reply))
    }

    fn get_computed_style_for_node(&mut self, node: EngineNodeId) -> Result<Vec<(String, String)>, HostError> {
        request(&self.sender, |reply| PageCommand::ComputedStyleForNode(node, reply))
    }

    fn get_matched_styles_for_node(&mut self, node: EngineNodeId) -> Result<Vec<MatchedRule>, HostError> {
        request(&self.sender, |reply| PageCommand::MatchedStylesForNode(node, reply))
    }

    fn get_style_sheet_text(&mut self, style_sheet_id: &str) -> Result<Option<String>, HostError> {
        let style_sheet_id = style_sheet_id.to_owned();
        request(&self.sender, |reply| PageCommand::StyleSheetText(style_sheet_id, reply))
    }

    fn dispatch_mouse_event(&mut self, event: &MouseEvent) -> Result<(), HostError> {
        let event = event.clone();
        request(&self.sender, |reply| PageCommand::DispatchMouseEvent(event, reply))
    }

    fn dispatch_key_event(&mut self, event: &KeyEvent) -> Result<(), HostError> {
        let event = event.clone();
        request(&self.sender, |reply| PageCommand::DispatchKeyEvent(event, reply))
    }

    fn capture_screenshot(&mut self, params: &ScreenshotParams) -> Result<Screenshot, HostError> {
        let params = params.clone();
        request(&self.sender, |reply| PageCommand::CaptureScreenshot(params, reply))
    }

    fn get_properties(&mut self, object_id: &str, own_only: bool) -> Result<Vec<PropertyDescriptor>, HostError> {
        let object_id = object_id.to_owned();
        request(&self.sender, |reply| PageCommand::GetProperties(object_id, own_only, reply))
    }

    fn call_function_on(&mut self, params: &CallFunctionParams) -> Result<EvaluateResult, HostError> {
        let params = params.clone();
        request(&self.sender, |reply| PageCommand::CallFunctionOn(params, reply))
    }

    fn release_object(&mut self, object_id: &str) -> Result<(), HostError> {
        let object_id = object_id.to_owned();
        request(&self.sender, |reply| PageCommand::ReleaseObject(object_id, reply))
    }

    fn release_object_group(&mut self, object_group: &str) -> Result<(), HostError> {
        let object_group = object_group.to_owned();
        request(&self.sender, |reply| PageCommand::ReleaseObjectGroup(object_group, reply))
    }

    fn get_response_body(&mut self, request_id: &str) -> Result<Option<ResourceContent>, HostError> {
        let request_id = request_id.to_owned();
        request(&self.sender, |reply| PageCommand::GetResponseBody(request_id, reply))
    }

    fn drain_console(&mut self) -> Vec<ConsoleEvent> {
        request(&self.sender, PageCommand::DrainConsole).unwrap_or_default()
    }

    fn drain_network_requests(&mut self) -> Vec<NetworkRequestEvent> {
        request(&self.sender, PageCommand::DrainNetworkRequests).unwrap_or_default()
    }

    fn drain_network_responses(&mut self) -> Vec<NetworkResponseEvent> {
        request(&self.sender, PageCommand::DrainNetworkResponses).unwrap_or_default()
    }

    fn drain_network_finished(&mut self) -> Vec<NetworkFinishedEvent> {
        request(&self.sender, PageCommand::DrainNetworkFinished).unwrap_or_default()
    }

    fn drain_network_failed(&mut self) -> Vec<NetworkFailedEvent> {
        request(&self.sender, PageCommand::DrainNetworkFailed).unwrap_or_default()
    }

    fn drain_exceptions(&mut self) -> Vec<ExceptionEvent> {
        request(&self.sender, PageCommand::DrainExceptions).unwrap_or_default()
    }

    fn drain_log(&mut self) -> Vec<LogEntry> {
        request(&self.sender, PageCommand::DrainLog).unwrap_or_default()
    }

    fn get_box_model(&mut self, node: EngineNodeId) -> Result<BoxModel, HostError> {
        request(&self.sender, |reply| PageCommand::GetBoxModel(node, reply))
    }

    fn get_content_quads(&mut self, node: EngineNodeId) -> Result<Vec<BoxModel>, HostError> {
        request(&self.sender, |reply| PageCommand::GetContentQuads(node, reply))
    }

    /// `Overlay.highlightNode`: routed to the page thread since it validates
    /// against real layout (`PageOwner::highlight_node`, which reuses
    /// `get_box_model`'s computation). `highlightRect`/`highlightQuad` take
    /// arbitrary coordinates with nothing engine-specific to validate (real
    /// Chrome accepts any rect/quad too), so they answer directly without a
    /// round trip; `overlay_enabled`/`set_inspect_mode`/`hide_highlight` are
    /// left at the trait's own `Ok(())` defaults — bookkeeping this driver
    /// has nothing further to do for, since no overlay is actually painted
    /// (see the module's "Known limitations").
    fn highlight_node(&mut self, node: EngineNodeId, _config: &HighlightConfig) -> Result<(), HostError> {
        request(&self.sender, |reply| PageCommand::HighlightNode(node, reply))
    }

    fn highlight_rect(&mut self, _rect: amazing_devtools::host::Rect, _config: &HighlightConfig) -> Result<(), HostError> {
        Ok(())
    }

    fn highlight_quad(&mut self, _quad: Quad, _config: &HighlightConfig) -> Result<(), HostError> {
        Ok(())
    }

    /// CDP-20: `Debugger.enable` backfills `scriptParsed` for every script
    /// already parsed (`PageOwner::debugger_enabled`), matching real Chrome.
    fn debugger_enabled(&mut self, enabled: bool) -> Result<(), HostError> {
        request(&self.sender, |reply| PageCommand::DebuggerEnabled(enabled, reply))
    }

    /// `condition` (CDP-23) is a JS expression evaluated at
    /// `PageOwner::drain_debugger_paused` time — see that method's doc
    /// comment.
    fn set_breakpoint_by_url(
        &mut self,
        url: &str,
        line_number: u32,
        column_number: Option<u32>,
        condition: Option<&str>,
    ) -> Result<(String, Vec<ScriptLocation>), HostError> {
        let url = url.to_owned();
        let condition = condition.map(str::to_owned);
        request(&self.sender, |reply| PageCommand::SetBreakpointByUrl(url, line_number, column_number, condition, reply))
    }

    fn remove_breakpoint(&mut self, breakpoint_id: &str) -> Result<(), HostError> {
        let breakpoint_id = breakpoint_id.to_owned();
        request(&self.sender, |reply| PageCommand::RemoveBreakpoint(breakpoint_id, reply))
    }

    /// `Debugger.pause`: since nothing can be blocked mid-statement on this
    /// driver's single page thread (see the module's "Known limitations"),
    /// this arms a one-shot flag the debug hook consumes on the *next*
    /// statement that actually runs (the next event-loop tick, timer, or
    /// handler for an otherwise idle page) rather than halting anything
    /// already executing.
    fn debugger_pause(&mut self) -> Result<(), HostError> {
        request(&self.sender, PageCommand::DebuggerPause)
    }

    /// `Debugger.resume`: clears that same flag if it had not fired yet.
    /// Nothing is ever truly suspended by this driver, so there is no
    /// execution to actually resume — see "Known limitations".
    fn debugger_resume(&mut self) -> Result<(), HostError> {
        request(&self.sender, PageCommand::DebuggerResume)
    }

    /// `Debugger.stepOver` (CDP-23, now real depth-based stepping): see
    /// `PageOwner::debugger_step_over`'s doc comment.
    fn debugger_step_over(&mut self) -> Result<(), HostError> {
        request(&self.sender, PageCommand::DebuggerStepOver)
    }

    /// `Debugger.stepInto` (CDP-23).
    fn debugger_step_into(&mut self) -> Result<(), HostError> {
        request(&self.sender, PageCommand::DebuggerStepInto)
    }

    /// `Debugger.stepOut` (CDP-23).
    fn debugger_step_out(&mut self) -> Result<(), HostError> {
        request(&self.sender, PageCommand::DebuggerStepOut)
    }

    /// `Debugger.evaluateOnCallFrame` (CDP-23): see
    /// `PageOwner::evaluate_on_call_frame`'s doc comment.
    fn evaluate_on_call_frame(&mut self, call_frame_id: &str, expression: &str) -> Result<EvaluateResult, HostError> {
        let call_frame_id = call_frame_id.to_owned();
        let expression = expression.to_owned();
        request(&self.sender, |reply| PageCommand::EvaluateOnCallFrame(call_frame_id, expression, reply))
    }

    fn get_script_source(&mut self, script_id: &str) -> Result<String, HostError> {
        let script_id = script_id.to_owned();
        request(&self.sender, |reply| PageCommand::GetScriptSource(script_id, reply))
    }

    fn drain_scripts_parsed(&mut self) -> Vec<ScriptInfo> {
        request(&self.sender, PageCommand::DrainScriptsParsed).unwrap_or_default()
    }

    fn drain_debugger_paused(&mut self) -> Vec<PausedEvent> {
        request(&self.sender, PageCommand::DrainDebuggerPaused).unwrap_or_default()
    }

    fn set_cache_disabled(&mut self, disabled: bool) -> Result<(), HostError> {
        request(&self.sender, |reply| PageCommand::SetCacheDisabled(disabled, reply))
    }

    fn set_extra_http_headers(&mut self, headers: &[(String, String)]) -> Result<(), HostError> {
        let headers = headers.to_vec();
        request(&self.sender, |reply| PageCommand::SetExtraHttpHeaders(headers, reply))
    }

    fn set_device_metrics_override(&mut self, metrics: &DeviceMetrics) -> Result<(), HostError> {
        let metrics = metrics.clone();
        request(&self.sender, |reply| PageCommand::SetDeviceMetricsOverride(metrics, reply))
    }

    fn clear_device_metrics_override(&mut self) -> Result<(), HostError> {
        request(&self.sender, PageCommand::ClearDeviceMetricsOverride)
    }

    fn insert_text(&mut self, text: &str) -> Result<(), HostError> {
        let text = text.to_owned();
        request(&self.sender, |reply| PageCommand::InsertText(text, reply))
    }
}

/// The browser-level seam: one page target, backed by a real page (fetched or,
/// for tests, parsed directly) running on its own thread.
pub(crate) struct DriverDevToolsHost {
    target_id: String,
    sender: Mutex<Sender<PageCommand>>,
    current_url: Arc<Mutex<String>>,
}

impl DriverDevToolsHost {
    fn spawn(target_id: impl Into<String>, init: PageInit) -> Result<Self, HostError> {
        let (sender, url) = spawn_page_thread(init)?;
        Ok(Self { target_id: target_id.into(), sender: Mutex::new(sender), current_url: Arc::new(Mutex::new(url)) })
    }
}

impl Drop for DriverDevToolsHost {
    fn drop(&mut self) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(PageCommand::Shutdown);
        }
    }
}

impl DevToolsHost for DriverDevToolsHost {
    fn list_targets(&self) -> Vec<TargetInfo> {
        let url = self.current_url.lock().expect("devtools url poisoned").clone();
        vec![TargetInfo::page(self.target_id.clone(), url.clone(), url)]
    }

    fn attach(&self, target_id: &str) -> Result<Box<dyn DevToolsSession>, HostError> {
        if target_id != self.target_id {
            return Err(HostError::NotFound);
        }
        let sender = self.sender.lock().expect("devtools sender poisoned").clone();
        Ok(Box::new(DriverPageSession {
            target_id: self.target_id.clone(),
            sender,
            current_url: self.current_url.clone(),
        }))
    }
}

/// A running CDP server on its own thread, stopped on drop.
pub(crate) struct DevToolsHandle {
    pub(crate) addr: SocketAddr,
    pub(crate) browser_id: String,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl Drop for DevToolsHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl DevToolsHandle {
    /// Blocks the calling thread until the server thread exits. Nothing in
    /// this handle sets the shutdown flag itself, so this is for a foreground
    /// CLI command that should serve until the process is killed, not for
    /// code that wants to stop the server (drop the handle for that).
    pub(crate) fn wait(mut self) -> io::Result<()> {
        match self.join.take() {
            Some(join) => join.join().unwrap_or(Ok(())),
            None => Ok(()),
        }
    }
}

/// Fetches and parses `initial_url`, starts its dedicated page thread, then
/// binds and starts a CDP server for it on `addr` (`"127.0.0.1:0"` for an
/// ephemeral port) on a background thread.
pub(crate) fn spawn_devtools_server(initial_url: &str, addr: &str) -> Result<DevToolsHandle, String> {
    let init = PageInit::Fetch { target_id: "page-1".into(), url: initial_url.to_owned(), width: 1024.0 };
    let host = Arc::new(DriverDevToolsHost::spawn("page-1", init).map_err(|error| error.to_string())?);
    let server = CdpServer::bind(host, addr).map_err(|error| format!("bind {addr}: {error}"))?;
    let resolved_addr = server.local_addr().map_err(|error| error.to_string())?;
    let browser_id = server.browser_id().to_owned();
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));
    Ok(DevToolsHandle { addr: resolved_addr, browser_id, shutdown, join: Some(join) })
}

#[cfg(test)]
#[path = "runtime_host_devtools_tests.rs"]
mod tests;
