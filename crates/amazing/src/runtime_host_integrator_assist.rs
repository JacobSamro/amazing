//! Window/navigator-level HOST APIs whose *value* comes from the driver
//! itself, as opposed to WebIDL interface scaffolding (dom-bindings' job).
//! IA-LIVE-3: `matchMedia` (evaluated against the driver's real viewport via
//! `amazing_css::MediaQueryList`/`MediaEnvironment`, the same types CSS uses
//! for `@media`), `requestIdleCallback`/`cancelIdleCallback` (the existing
//! `Interpreter::schedule_timer`/`cancel_timer` driver timer queue, the same
//! mechanism `setTimeout`/`clearTimeout` already use), `performance.now()`/
//! `performance.timeOrigin` (the host wall clock) and plain `navigator.*`
//! identity data. `.agents/dom-bindings/questions.md` (2026-09-26T14:56:00Z,
//! the IA-LIVE-PAGES 17-API survey) had not picked any of these up as of this
//! ticket.
//!
//! Per `AGENTS.md`'s driver-adapter-per-lane rule, this file is this ticket's
//! entire footprint in the shared driver beyond a handful of one-line hooks
//! in `runtime_binding_host.rs` (one `mod` line, one `NativeEntry` variant
//! plus its dispatch arm, one `BindingState` field plus its initializer, one
//! installer call).
//!
//! Deliberately **not** attempted here — left to dom-bindings as interface-
//! level work, per the ticket's own split:
//! - `matchMedia` returns a plain, static snapshot object (`matches`,
//!   `media`, and no-op `addListener`/`removeListener`/`addEventListener`/
//!   `removeEventListener`), not a live `MediaQueryList` `EventTarget` that
//!   fires `change` when the driver's viewport later changes.
//! - The object passed to a `requestIdleCallback` callback is a plain object
//!   shaped like `IdleDeadline` (`timeRemaining()`, `didTimeout`), not the
//!   real interface; `timeRemaining()` always answers `0` (spec-legal: "no
//!   idle time available" is always a safe answer), since this driver has no
//!   genuine idle-vs-busy detection to report.
//! - No `PerformanceObserver`/marks/measures/entries — only the two clock
//!   primitives real pages call constantly (`now()`, `timeOrigin`).

use super::*;

/// This lane's whole state footprint in `BindingState`.
pub(super) struct IaRuntime {
    /// Set once per page load (`execute_page_scripts_lenient` via
    /// `DriverScheduler::set_viewport`); read on every `matchMedia()` call.
    /// Defaults match `amazing_css::MediaEnvironment::default()` so a script
    /// that calls `matchMedia` before the driver knows a real viewport (e.g.
    /// a bare `amazing run script.js`, no page) still gets a sane
    /// environment instead of a fabricated zero-size one.
    viewport_width_px: f32,
    viewport_height_px: f32,
    created_at: std::time::Instant,
    /// IA-TESTDRIVER: `test_driver.set_permission` results, keyed by
    /// permission name. Nothing reads this map yet -- there is no
    /// `navigator.permissions` implementation anywhere in this engine (a
    /// clean `grep` for it came back empty) -- so this is forward-compatible
    /// scaffolding for whichever lane ships the real Permissions API, not a
    /// working permission gate today. Storing it here (rather than throwing)
    /// still lets a test's *own* assertions after the `set_permission` call
    /// run instead of failing on the setup step alone.
    permissions: std::collections::HashMap<String, String>,
}

impl IaRuntime {
    pub(super) fn new() -> Self {
        let default_environment = amazing_css::MediaEnvironment::default();
        Self {
            viewport_width_px: default_environment.viewport_width_px,
            viewport_height_px: default_environment.viewport_height_px,
            created_at: std::time::Instant::now(),
            permissions: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum IaNativeEntry {
    MatchMedia,
    NoOp,
    RequestIdleCallback,
    CancelIdleCallback,
    IdleDeadlineTimeRemaining,
    PerformanceNow,
    // IA-TESTDRIVER: native backends for `testdriver-vendor.js`
    // (`crate::cmd::wpt_harness`'s embedded `TESTDRIVER_VENDOR_JS`), called
    // from that script's global `__amazing_td_*` functions.
    TdClick,
    TdSendKeys,
    TdActionSequence,
    TdSetPermission,
    TdDeleteAllCookies,
    TdGetComputedLabel,
    TdGetComputedRole,
    TdGetClientRects,
    TdElementsFromPoint,
}

impl RuntimeBindings {
    /// The driver's real per-page viewport, for `matchMedia` to evaluate
    /// size-based media features against. Mirrors `set_document_url`'s shape
    /// exactly (both are "tell the interpreter something the driver alone
    /// knows"), and is called from the same place (`DriverScheduler`).
    pub(crate) fn set_viewport(&self, width: f32, height: f32) {
        let mut state = self.state.borrow_mut();
        state.integrator_assist.viewport_width_px = width;
        state.integrator_assist.viewport_height_px = height;
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_integrator_assist_globals(&mut self) -> Result<(), Throw> {
        let global = self.js.binding_global_object();

        // navigator.*: get-or-create the same way `runtime_host_storage.rs`
        // already does for `navigator.storage`/`navigator.locks` — additive,
        // no single owner, matching this codebase's existing convention for
        // this particular object.
        let navigator = self.js.binding_get_property(&global, "navigator")?;
        let navigator = if matches!(navigator, JsValue::Object(_)) {
            navigator
        } else {
            let value = self.js.binding_create_object(None)?;
            self.define_data_property(&global, "navigator", value.clone(), true, false, true)?;
            value
        };
        let languages = self.js.binding_create_array(vec![JsValue::str("en-US")])?;
        let hardware_concurrency = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(1.0);
        for (name, value) in [
            ("userAgent", JsValue::str("Amazing/0.1 (from-scratch)")),
            ("appVersion", JsValue::str("0.1 (from-scratch; Amazing)")),
            ("platform", JsValue::str(Self::navigator_platform())),
            ("language", JsValue::str("en-US")),
            ("onLine", JsValue::Bool(true)),
            ("cookieEnabled", JsValue::Bool(true)),
            ("hardwareConcurrency", JsValue::num(hardware_concurrency)),
        ] {
            self.define_data_property(&navigator, name, value, true, true, true)?;
        }
        self.define_data_property(&navigator, "languages", languages, true, true, true)?;

        // performance.now()/timeOrigin: the two clock primitives real pages
        // call constantly; PerformanceObserver/marks/measures/entries are
        // interface-level work left to dom-bindings.
        let performance = self.js.binding_create_object(None)?;
        let now = self.native("now", 0, NativeEntry::IntegratorAssist(IaNativeEntry::PerformanceNow), HostCallPolicy::CallOnly);
        self.define_data_property(&performance, "now", now, true, true, true)?;
        let time_origin_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        self.define_data_property(&performance, "timeOrigin", JsValue::num(time_origin_ms), true, true, true)?;
        self.define_data_property(&global, "performance", performance, true, false, true)?;

        // matchMedia: a static snapshot evaluated against the driver's real
        // viewport (see `IaRuntime::viewport_width_px`/`_height_px`).
        let match_media = self.native("matchMedia", 1, NativeEntry::IntegratorAssist(IaNativeEntry::MatchMedia), HostCallPolicy::CallOnly);
        self.define_data_property(&global, "matchMedia", match_media, true, false, true)?;

        // requestIdleCallback/cancelIdleCallback: the existing driver timer
        // queue (`Interpreter::schedule_timer`/`cancel_timer`, same mechanism
        // `setTimeout`/`clearTimeout` already use), so a page that only uses
        // this as a "run soon" hint (the overwhelmingly common real-world
        // pattern: `requestIdleCallback(cb)` with no timeout, or falling back
        // to `setTimeout` when it's absent) behaves correctly end to end.
        let ric = self.native("requestIdleCallback", 1,
            NativeEntry::IntegratorAssist(IaNativeEntry::RequestIdleCallback), HostCallPolicy::CallOnly);
        self.define_data_property(&global, "requestIdleCallback", ric, true, false, true)?;
        let cic = self.native("cancelIdleCallback", 1,
            NativeEntry::IntegratorAssist(IaNativeEntry::CancelIdleCallback), HostCallPolicy::CallOnly);
        self.define_data_property(&global, "cancelIdleCallback", cic, true, false, true)?;

        // IA-TESTDRIVER: the `__amazing_td_*` native entry points that
        // `crate::cmd::wpt_harness::TESTDRIVER_VENDOR_JS` calls into. Global
        // functions rather than methods on some driver object, matching this
        // file's `install_callback` sibling in `wpt_harness.rs`
        // (`__amazing_result`/`__amazing_complete`) -- the vendor script
        // itself is the only caller, so there is no interface surface to
        // keep clean here.
        for (name, entry) in [
            ("__amazing_td_click", IaNativeEntry::TdClick),
            ("__amazing_td_send_keys", IaNativeEntry::TdSendKeys),
            ("__amazing_td_action_sequence", IaNativeEntry::TdActionSequence),
            ("__amazing_td_set_permission", IaNativeEntry::TdSetPermission),
            ("__amazing_td_delete_all_cookies", IaNativeEntry::TdDeleteAllCookies),
            ("__amazing_td_get_computed_label", IaNativeEntry::TdGetComputedLabel),
            ("__amazing_td_get_computed_role", IaNativeEntry::TdGetComputedRole),
            ("__amazing_td_get_client_rects", IaNativeEntry::TdGetClientRects),
            ("__amazing_td_elements_from_point", IaNativeEntry::TdElementsFromPoint),
        ] {
            let function = self.native(name, 0, NativeEntry::IntegratorAssist(entry), HostCallPolicy::CallOnly);
            self.define_data_property(&global, name, function, true, false, true)?;
        }
        Ok(())
    }

    // --- IA-TESTDRIVER geometry: a from-scratch layout pass exists
    // (`amazing_layout`), but no CSSOM View geometry (`getClientRects`,
    // `elementFromPoint`/`elementsFromPoint`) is wired up to script anywhere
    // in the engine (a clean grep across `crates/amazing*/src` came back
    // empty) -- real WPT `testdriver.js` itself calls both from its own
    // `click()`/`send_keys()` wrappers before ever reaching
    // `test_driver_internal`, so without *some* implementation those two
    // actions fail before this ticket's own backend is ever invoked. Rather
    // than adding general, always-available CSSOM View support (interface-
    // level work that belongs to `layout`/`dom-bindings`), this computes a
    // fresh, approximate layout tree (no author stylesheets, the
    // `FallbackTextSystem`/`NoReplacedElements` no-op providers -- text
    // metrics and replaced content are approximate, not paint-accurate) on
    // demand, purely for this harness's own testdriver polyfill
    // (`TESTDRIVER_VENDOR_JS` only defines `getClientRects`/
    // `elementsFromPoint` etc. when they are not already present -- see that
    // script -- so a real implementation from `layout`/`dom-bindings` later
    // automatically takes over without this file changing).
    fn td_layout(&self) -> (amazing_css::StyledDom, amazing_layout::LayoutBox) {
        let (width, height) = {
            let state = self.state.borrow();
            (state.integrator_assist.viewport_width_px, state.integrator_assist.viewport_height_px)
        };
        let host = self.state.borrow().host.clone();
        let dom_ref = host.borrow();
        let styled = amazing_css::compute(&dom_ref.dom, &[]);
        let layout = amazing_layout::layout_styled_document(
            &dom_ref.dom, &styled,
            &amazing_text::FallbackTextSystem::new(), &amazing_layout::NoReplacedElements,
            width, Some(height),
        );
        (styled, layout)
    }

    /// `amazing_a11y::build` needs `&Dom` again alongside the styled/layout
    /// pair `td_layout` already computed (mirrors `runtime_host_a11y.rs`'s
    /// own `build_ax_tree`, not reusable here since it lives in a different
    /// lane's file); this exists so `get_computed_label`/`get_computed_role`
    /// share one call site.
    fn td_ax_tree(&self) -> amazing_a11y::AxTree {
        let (styled, layout) = self.td_layout();
        let host = self.state.borrow().host.clone();
        let dom_ref = host.borrow();
        amazing_a11y::build(&dom_ref.dom, &styled, &layout)
    }

    /// Trusted, bubbling, cancelable, composed -- the shape every native UI
    /// event (`mousedown`/`mouseup`/`click`/`mousemove`/`wheel`/`keydown`/
    /// `keyup`) this file dispatches uses. `input` events use their own
    /// (non-cancelable) shape at their one call site below.
    fn td_ui_event_init() -> amazing_dom::events::EventInit {
        amazing_dom::events::EventInit { bubbles: true, cancelable: true, composed: true }
    }

    /// `amazing_bindings::dispatch_user_agent_event` is the exact path a real
    /// user click already takes in the shipped GUI shell (`gui.rs`'s
    /// `WM_LBUTTONUP` handler calls the equivalent
    /// `DriverScheduler::dispatch_dom_event`, which bottoms out here too), so
    /// routing through it is what makes these testdriver-originated events
    /// genuinely trusted (`Event.isTrusted`) and gives them the engine's real
    /// default actions (checkbox/radio toggling, label click forwarding,
    /// submit-button activation, ...) rather than a synthetic,
    /// script-`dispatchEvent`-shaped stand-in.
    fn td_dispatch(&mut self, node: amazing_dom::NodeIdx, kind: &str,
        init: amazing_dom::events::EventInit) -> Result<(), Throw> {
        amazing_bindings::dispatch_user_agent_event(
            self, amazing_dom::events::EventTarget::Node(node), kind, init)
    }

    /// Only `INPUT` text-entry types and `TEXTAREA` get their `.value`
    /// mutated by `td_send_char` below; a checkbox/radio/button/etc.
    /// `INPUT.value` is a different, non-text-entry concept (the control's
    /// submitted value, defaulting to `"on"`) that typing must not touch.
    fn td_is_text_entry(&mut self, element: &JsValue) -> Result<bool, Throw> {
        let tag = self.js.binding_get_property(element, "tagName")?;
        let tag = tag.as_str().map(ToString::to_string).unwrap_or_default().to_ascii_uppercase();
        if tag == "TEXTAREA" {
            return Ok(true);
        }
        if tag != "INPUT" {
            return Ok(false);
        }
        let kind = self.js.binding_get_property(element, "type")?;
        let kind = kind.as_str().map(ToString::to_string).unwrap_or_else(|| "text".to_string())
            .to_ascii_lowercase();
        Ok(matches!(kind.as_str(), "text" | "search" | "email" | "password" | "tel" | "url" | "number"))
    }

    /// One WebDriver-normalized key value (`keys` is iterated by `char` in
    /// `TdSendKeys`/`TdActionSequence` below): dispatches `keydown`, mutates
    /// `.value` for a text-entry element (a real `input`/`keypress` default
    /// action for text insertion does not exist in this engine yet -- see
    /// this method's own gap note below -- so this stands in for it), fires
    /// `input`, then dispatches `keyup`.
    ///
    /// **Known gap, filed to dom-bindings/html (not attempted here):** the
    /// dispatched `keydown`/`keyup` carry no `key`/`code`/`keyCode` (a
    /// generic `Event`, since `amazing_bindings::dispatch_user_agent_event`'s
    /// public signature has no way to attach `EventExtra` data or select
    /// `EventClass::Keyboard` -- both are internal to `event_bindings.rs`,
    /// reached only through crate-private helpers this adapter cannot call
    /// from outside `amazing-bindings`). A subtest asserting
    /// `event.key === "a"` still fails; one asserting the field's resulting
    /// `.value` and that an `input`/`keydown`/`keyup` sequence fired at all
    /// passes.
    fn td_send_char(&mut self, node: amazing_dom::NodeIdx, element: &JsValue, ch: char) -> Result<(), Throw> {
        let ui = Self::td_ui_event_init();
        self.td_dispatch(node, "keydown", ui)?;
        let is_backspace = ch == '\u{E003}';
        let is_special = matches!(ch,
            '\u{E000}'..='\u{E040}' /* WebDriver normalized-key PUA block */);
        if (is_backspace || !is_special) && self.td_is_text_entry(element)? {
            let current = self.js.binding_get_property(element, "value")?;
            let units = self.to_dom_string(&current)?;
            let mut text = String::from_utf16_lossy(&units);
            if is_backspace {
                text.pop();
            } else {
                text.push(ch);
            }
            self.js.binding_set_property(element, "value", JsValue::str(&text))?;
            self.td_dispatch(node, "input",
                amazing_dom::events::EventInit { bubbles: true, cancelable: false, composed: true })?;
        }
        self.td_dispatch(node, "keyup", ui)?;
        Ok(())
    }

    fn td_array_len(&mut self, value: &JsValue) -> Result<u32, Throw> {
        let length = self.js.binding_get_property(value, "length")?;
        let length = self.js.binding_to_number(&length)?;
        Ok(if length.is_finite() && length > 0.0 { length.trunc() as u32 } else { 0 })
    }
    fn td_array_get(&mut self, value: &JsValue, index: u32) -> Result<JsValue, Throw> {
        self.js.binding_get_property(value, &index.to_string())
    }

    fn navigator_platform() -> &'static str {
        match std::env::consts::OS {
            "windows" => "Win32",
            "macos" => "MacIntel",
            _ => "Linux x86_64",
        }
    }

    pub(super) fn invoke_integrator_assist(&mut self, entry: IaNativeEntry, _this: JsValue,
        args: &[JsValue]) -> Result<JsValue, Throw> {
        match entry {
            IaNativeEntry::MatchMedia => {
                let query = args.first()
                    .and_then(JsValue::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let (width, height) = {
                    let state = self.state.borrow();
                    (state.integrator_assist.viewport_width_px, state.integrator_assist.viewport_height_px)
                };
                let environment = amazing_css::MediaEnvironment {
                    viewport_width_px: width,
                    viewport_height_px: height,
                    ..amazing_css::MediaEnvironment::default()
                };
                let matches = amazing_css::MediaQueryList::parse(&query).evaluate(&environment);
                let result = self.js.binding_create_object(None)?;
                self.define_data_property(&result, "matches", JsValue::Bool(matches), true, true, true)?;
                self.define_data_property(&result, "media", JsValue::str(query), true, true, true)?;
                let noop = self.native("noop", 0, NativeEntry::IntegratorAssist(IaNativeEntry::NoOp), HostCallPolicy::CallOnly);
                for name in ["addListener", "removeListener", "addEventListener", "removeEventListener"] {
                    self.define_data_property(&result, name, noop.clone(), true, true, true)?;
                }
                Ok(result)
            }
            IaNativeEntry::NoOp => Ok(JsValue::Undefined),
            IaNativeEntry::RequestIdleCallback => {
                let callback = args.first().cloned()
                    .filter(JsValue::is_object)
                    .ok_or_else(|| Throw::type_error("requestIdleCallback requires a callable"))?;
                let timeout = args.get(1)
                    .filter(|value| matches!(value, JsValue::Object(_)))
                    .and_then(|options| self.js.binding_get_property(options, "timeout").ok())
                    .and_then(|value| value.to_number().ok())
                    .filter(|n| n.is_finite() && *n >= 0.0);
                // Timer due-times are real `Instant`s (`amazing_js::interp`),
                // not a simulated clock, so a nonzero default here would make
                // firing depend on genuine wall-clock progress between the
                // call and the next event-loop pump -- exactly the kind of
                // flake `AbortSignal.timeout(0)` avoids elsewhere in this
                // codebase by using a 0ms delay. Absent an explicit `timeout`
                // option, fire as soon as the loop is next pumped instead of
                // waiting on a real (and here, spec-irrelevant) millisecond.
                let delay = timeout.map(|t| t.min(f64::from(i32::MAX)) as i32).unwrap_or(0);
                let time_remaining = self.native("timeRemaining", 0,
                    NativeEntry::IntegratorAssist(IaNativeEntry::IdleDeadlineTimeRemaining), HostCallPolicy::CallOnly);
                let deadline = self.js.binding_create_object(None)?;
                self.define_data_property(&deadline, "timeRemaining", time_remaining, true, true, true)?;
                self.define_data_property(&deadline, "didTimeout", JsValue::Bool(false), true, true, true)?;
                let id = self.js.schedule_timer(callback, delay, vec![deadline])?;
                Ok(JsValue::num(f64::from(id)))
            }
            IaNativeEntry::CancelIdleCallback => {
                let id = args.first().and_then(|value| value.to_number().ok()).unwrap_or(0.0);
                self.js.cancel_timer(id as u32)?;
                Ok(JsValue::Undefined)
            }
            IaNativeEntry::IdleDeadlineTimeRemaining => Ok(JsValue::num(0.0)),
            IaNativeEntry::PerformanceNow => {
                let elapsed = self.state.borrow().integrator_assist.created_at.elapsed().as_secs_f64() * 1000.0;
                Ok(JsValue::num(elapsed))
            }
            IaNativeEntry::TdClick => {
                let element = args.first().cloned().unwrap_or(JsValue::Undefined);
                let node = self.node_brand(&element)
                    .ok_or_else(|| Throw::type_error("test_driver_internal.click: expected an element"))?;
                let ui = Self::td_ui_event_init();
                self.td_dispatch(node, "mousedown", ui)?;
                self.td_dispatch(node, "mouseup", ui)?;
                // `Operation::HTMLElementClick` -- the exact native operation
                // a real `element.click()` JS call routes to
                // (`input_activation_bindings::invoke`) -- runs the DOM's own
                // pre-activation/completion state machine
                // (`begin_html_click`/`finish_input_activation`) around the
                // "click" event dispatch. A hand-rolled `dispatch_user_agent_
                // event(.., "click", ..)` (this method's first draft) fires
                // listeners but never enters that state machine, so a
                // checkbox/radio never toggles and a `<label>` never forwards
                // to its control -- both are driven by activation completion,
                // not by the event type name alone.
                invoke(self, Operation::HTMLElementClick, element, &[])?;
                Ok(JsValue::Undefined)
            }
            IaNativeEntry::TdSendKeys => {
                let element = args.first().cloned().unwrap_or(JsValue::Undefined);
                let node = self.node_brand(&element)
                    .ok_or_else(|| Throw::type_error("test_driver_internal.send_keys: expected an element"))?;
                let keys = args.get(1).cloned().unwrap_or(JsValue::Undefined);
                let units = self.to_dom_string(&keys)?;
                let text = String::from_utf16_lossy(&units);
                for ch in text.chars() {
                    self.td_send_char(node, &element, ch)?;
                }
                Ok(JsValue::Undefined)
            }
            IaNativeEntry::TdActionSequence => {
                let actions = args.first().cloned().unwrap_or(JsValue::Undefined);
                let ui = Self::td_ui_event_init();
                let source_count = self.td_array_len(&actions)?;
                for source_index in 0..source_count {
                    let source = self.td_array_get(&actions, source_index)?;
                    let source_kind = self.js.binding_get_property(&source, "type")?
                        .as_str().map(ToString::to_string).unwrap_or_default();
                    let acts = self.js.binding_get_property(&source, "actions")?;
                    let act_count = self.td_array_len(&acts)?;
                    // WebDriver actions are per-tick across every input
                    // source; this runner processes one source's ticks in
                    // full before the next, which only differs from a real
                    // interleaved dispatch for tests combining two active
                    // sources in the same tick (rare in this ticket's target
                    // directories).
                    let mut pointer_target: Option<amazing_dom::NodeIdx> = None;
                    for act_index in 0..act_count {
                        let act = self.td_array_get(&acts, act_index)?;
                        let act_kind = self.js.binding_get_property(&act, "type")?
                            .as_str().map(ToString::to_string).unwrap_or_default();
                        match source_kind.as_str() {
                            "pointer" => {
                                // The overwhelmingly common WPT idiom is
                                // `pointerMove(0, 0, {origin: element})` --
                                // an element-relative move to (0,0), i.e.
                                // "target this element" -- so an element
                                // `origin` is used directly as the dispatch
                                // target without resolving its pixel offset;
                                // only an absolute (viewport/pointer-origin)
                                // move needs the layout hit-test below.
                                let origin = self.js.binding_get_property(&act, "origin")?;
                                if let Some(node) = self.node_brand(&origin) {
                                    pointer_target = Some(node);
                                } else if act_kind == "pointerMove" {
                                    let x = self.js.binding_get_property(&act, "x")
                                        .ok().and_then(|v| v.to_number().ok()).unwrap_or(0.0) as f32;
                                    let y = self.js.binding_get_property(&act, "y")
                                        .ok().and_then(|v| v.to_number().ok()).unwrap_or(0.0) as f32;
                                    let (_, layout) = self.td_layout();
                                    pointer_target = td_hit_test(&layout, x, y);
                                }
                                match act_kind.as_str() {
                                    "pointerMove" => {
                                        if let Some(node) = pointer_target {
                                            self.td_dispatch(node, "mousemove", ui)?;
                                        }
                                    }
                                    "pointerDown" => {
                                        if let Some(node) = pointer_target {
                                            self.td_dispatch(node, "mousedown", ui)?;
                                        }
                                    }
                                    "pointerUp" => {
                                        if let Some(node) = pointer_target {
                                            self.td_dispatch(node, "mouseup", ui)?;
                                            // See `TdClick`'s doc comment: a
                                            // real click's activation
                                            // completion (checkbox/radio
                                            // toggle, label forwarding) needs
                                            // the actual click operation, not
                                            // just a "click"-named event.
                                            let target = self.wrap_node(node)?;
                                            invoke(self, Operation::HTMLElementClick, target, &[])?;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            "key" => {
                                if act_kind == "keyDown" {
                                    let value = self.js.binding_get_property(&act, "value")?;
                                    let ch = value.as_str().and_then(|s| s.to_utf8_lossy().chars().next());
                                    if let Some(ch) = ch {
                                        let global = self.js.binding_global_object();
                                        let document = self.js.binding_get_property(&global, "document")?;
                                        let active = self.js.binding_get_property(&document, "activeElement")?;
                                        if let Some(node) = self.node_brand(&active) {
                                            // Dispatches its own keydown+keyup
                                            // (see `td_send_char`'s doc
                                            // comment); the paired `keyUp`
                                            // tick for the same key is a
                                            // deliberate no-op below rather
                                            // than tracking "held" keys.
                                            self.td_send_char(node, &active, ch)?;
                                        }
                                    }
                                }
                            }
                            "wheel" => {
                                if act_kind == "scroll" {
                                    let x = self.js.binding_get_property(&act, "x")
                                        .ok().and_then(|v| v.to_number().ok()).unwrap_or(0.0) as f32;
                                    let y = self.js.binding_get_property(&act, "y")
                                        .ok().and_then(|v| v.to_number().ok()).unwrap_or(0.0) as f32;
                                    let (_, layout) = self.td_layout();
                                    if let Some(node) = td_hit_test(&layout, x, y) {
                                        self.td_dispatch(node, "wheel", ui)?;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(JsValue::Undefined)
            }
            IaNativeEntry::TdSetPermission => {
                let params = args.first().cloned().unwrap_or(JsValue::Undefined);
                let descriptor = self.js.binding_get_property(&params, "descriptor")?;
                let name = self.js.binding_get_property(&descriptor, "name")?
                    .as_str().map(ToString::to_string).unwrap_or_default();
                let state = self.js.binding_get_property(&params, "state")?
                    .as_str().map(ToString::to_string).unwrap_or_default();
                if !name.is_empty() {
                    self.state.borrow_mut().integrator_assist.permissions.insert(name, state);
                }
                Ok(JsValue::Undefined)
            }
            // No cookie store exists anywhere in this engine yet (`document.
            // cookie` is unimplemented; a clean grep for a cookie jar in
            // `crates/amazing*/src` came back empty) -- deleting all cookies
            // from an always-empty jar is trivially, honestly successful,
            // matching WebDriver's own "no error if there are no cookies"
            // semantics, rather than throwing "not implemented" for a
            // feature this ticket did not add.
            IaNativeEntry::TdDeleteAllCookies => Ok(JsValue::Undefined),
            IaNativeEntry::TdGetComputedLabel => {
                let element = args.first().cloned().unwrap_or(JsValue::Undefined);
                let node = self.node_brand(&element)
                    .ok_or_else(|| Throw::type_error("get_computed_label: expected an element"))?;
                let tree = self.td_ax_tree();
                let name = tree.find_dom(node).and_then(|id| tree.node(id))
                    .map(|ax_node| ax_node.name.clone()).unwrap_or_default();
                Ok(JsValue::str(&name))
            }
            IaNativeEntry::TdGetComputedRole => {
                let element = args.first().cloned().unwrap_or(JsValue::Undefined);
                let node = self.node_brand(&element)
                    .ok_or_else(|| Throw::type_error("get_computed_role: expected an element"))?;
                let tree = self.td_ax_tree();
                let role = tree.find_dom(node).and_then(|id| tree.node(id))
                    .map(|ax_node| ax_node.role.as_str()).unwrap_or("");
                Ok(JsValue::str(role))
            }
            IaNativeEntry::TdGetClientRects => {
                let element = args.first().cloned().unwrap_or(JsValue::Undefined);
                let Some(node) = self.node_brand(&element) else {
                    return self.js.binding_create_array(vec![]);
                };
                let (_, layout) = self.td_layout();
                let Some(rect) = td_find_rect(&layout, node) else {
                    return self.js.binding_create_array(vec![]);
                };
                let object = self.js.binding_create_object(None)?;
                for (name, value) in [
                    ("x", rect.x as f64), ("left", rect.x as f64),
                    ("y", rect.y as f64), ("top", rect.y as f64),
                    ("width", rect.w as f64), ("height", rect.h as f64),
                    ("right", (rect.x + rect.w) as f64), ("bottom", (rect.y + rect.h) as f64),
                ] {
                    self.define_data_property(&object, name, JsValue::num(value), true, true, true)?;
                }
                self.js.binding_create_array(vec![object])
            }
            IaNativeEntry::TdElementsFromPoint => {
                let x = args.first().and_then(|v| v.to_number().ok()).unwrap_or(0.0) as f32;
                let y = args.get(1).and_then(|v| v.to_number().ok()).unwrap_or(0.0) as f32;
                let (_, layout) = self.td_layout();
                // Approximate: the single topmost hit, not the full z-order
                // stack a real `elementsFromPoint` returns -- see this
                // impl's module-level gap note on CSSOM View geometry.
                match td_hit_test(&layout, x, y) {
                    Some(node) => { let wrapped = self.wrap_node(node)?; self.js.binding_create_array(vec![wrapped]) }
                    None => self.js.binding_create_array(vec![]),
                }
            }
        }
    }
}

/// A rectangle in the shape `td_find_rect`/`td_hit_test` need, built from a
/// `ReplacedFrag` (see `td_find_rect`'s doc comment on why that case matters:
/// `<input>`/`<label>`/etc default to `display: inline`, so their geometry
/// lives in their inline-formatting-context ancestor's `LineBox.replaced`,
/// not as their own child `LayoutBox`).
fn td_replaced_frag_rect(frag: &amazing_layout::ReplacedFrag) -> amazing_layout::Rect {
    amazing_layout::Rect { x: frag.x, y: frag.y, w: frag.width, h: frag.height }
}

/// Same top-down, z-order-by-source-index algorithm as `gui.rs`'s private
/// `hit_test` (that copy is `#[cfg(windows)]`-gated and not reusable here);
/// kept in lockstep intentionally rather than factored into a shared helper,
/// since this one is deliberately approximate (see `td_layout`'s doc
/// comment) while the shell's is the real, paint-driving hit test. Extended
/// (IA-TESTDRIVER) to also hit-test `LineBox.replaced` fragments -- see
/// `td_find_rect`'s doc comment.
fn td_hit_test(layout: &amazing_layout::LayoutBox, x: f32, y: f32) -> Option<amazing_dom::NodeIdx> {
    let mut children: Vec<&amazing_layout::LayoutBox> = match &layout.flow {
        amazing_layout::Flow::Blocks(children)
        | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => children.iter().collect(),
        amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => Vec::new(),
    };
    children.extend(layout.positioned.iter());
    children.sort_by_key(|child| child.node.map(|node| node.0).unwrap_or(0));
    for child in children.iter().rev() {
        if let Some(node) = td_hit_test(child, x, y) {
            return Some(node);
        }
    }
    let contains = |rect: amazing_layout::Rect| {
        x >= rect.x && y >= rect.y && x < rect.x + rect.w && y < rect.y + rect.h
    };
    for line in layout.lines.iter().rev() {
        for frag in line.replaced.iter().rev() {
            if contains(td_replaced_frag_rect(frag)) {
                return Some(frag.node);
            }
        }
    }
    contains(layout.border_box).then_some(layout.node).flatten()
}

/// `<input>`/`<label>`/etc default to `display: inline` (the CSS UA
/// stylesheet), so a form control's geometry is not its own child
/// `LayoutBox` in `Flow::Blocks`/`.positioned` (which is where a `display:
/// block`/absolutely-positioned descendant would live) -- it is an atomic
/// fragment placed *inside a line* of its nearest block/inline-formatting-
/// context ancestor, recorded in that ancestor's own `LayoutBox.lines[].
/// replaced` (`amazing_layout::ReplacedFrag`). `gui.rs`'s original
/// `find_layout_box` (and this file's first IA-TESTDRIVER draft, copied from
/// it) never looked there, so every `getClientRects()`/`elementFromPoint`
/// call on an inline-level element like a checkbox or its `<label>` came back
/// empty. Returns a plain `Rect` rather than `&LayoutBox` since a
/// `ReplacedFrag` match has no backing `LayoutBox` to borrow.
fn td_find_rect(layout: &amazing_layout::LayoutBox, node: amazing_dom::NodeIdx) -> Option<amazing_layout::Rect> {
    if layout.node == Some(node) {
        return Some(layout.border_box);
    }
    for line in &layout.lines {
        if let Some(frag) = line.replaced.iter().find(|frag| frag.node == node) {
            return Some(td_replaced_frag_rect(frag));
        }
    }
    let children: &[amazing_layout::LayoutBox] = match &layout.flow {
        amazing_layout::Flow::Blocks(children)
        | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => children,
        amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => &[],
    };
    children.iter().find_map(|child| td_find_rect(child, node))
        .or_else(|| layout.positioned.iter().find_map(|child| td_find_rect(child, node)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom_host::BrowserDomHost;
    use std::{cell::RefCell, rc::Rc};

    fn scheduler() -> crate::js_scheduler::DriverScheduler {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap()
    }

    #[test]
    fn match_media_reflects_the_drivers_real_viewport() {
        let mut scheduler = scheduler();
        scheduler.set_viewport(1024.0, 768.0);
        assert_eq!(scheduler.run_script("matchMedia('(min-width: 800px)').matches").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("matchMedia('(min-width: 2000px)').matches").unwrap(), JsValue::Bool(false));
        scheduler.set_viewport(300.0, 768.0);
        assert_eq!(scheduler.run_script("matchMedia('(min-width: 800px)').matches").unwrap(), JsValue::Bool(false));
    }

    #[test]
    fn match_media_object_has_the_media_string_and_noop_listener_methods() {
        let mut scheduler = scheduler();
        assert_eq!(
            scheduler.run_script("matchMedia('(min-width: 1px)').media").unwrap(),
            JsValue::str("(min-width: 1px)"),
        );
        // Defensive-registration code should not throw even though nothing
        // will ever fire; see this file's module doc comment.
        scheduler.run_script(
            "var mq = matchMedia('(min-width: 1px)'); \
             mq.addListener(function(){}); mq.addEventListener('change', function(){}); \
             mq.removeListener(function(){}); mq.removeEventListener('change', function(){});"
        ).unwrap();
    }

    #[test]
    fn navigator_and_performance_report_real_driver_values() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script("typeof navigator.userAgent").unwrap(), JsValue::str("string"));
        assert_eq!(scheduler.run_script("navigator.onLine").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("navigator.languages.length").unwrap(), JsValue::num(1.0));
        assert_eq!(scheduler.run_script("typeof performance.now()").unwrap(), JsValue::str("number"));
        assert_eq!(scheduler.run_script("performance.now() >= 0").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("typeof performance.timeOrigin").unwrap(), JsValue::str("number"));
    }

    #[test]
    fn request_idle_callback_runs_the_callback_with_an_idle_deadline_shape() {
        let mut scheduler = scheduler();
        scheduler.run_script(
            "var seen = null; \
             requestIdleCallback(function(deadline) { seen = typeof deadline.timeRemaining() + ' ' + deadline.didTimeout; });"
        ).unwrap();
        for _ in 0..8 {
            scheduler.pump_now(false).unwrap();
        }
        assert_eq!(scheduler.run_script("seen").unwrap(), JsValue::str("number false"));
    }

    #[test]
    fn cancel_idle_callback_prevents_the_callback_from_running() {
        let mut scheduler = scheduler();
        scheduler.run_script(
            "var ran = false; \
             var id = requestIdleCallback(function() { ran = true; }); \
             cancelIdleCallback(id);"
        ).unwrap();
        for _ in 0..8 {
            scheduler.pump_now(false).unwrap();
        }
        assert_eq!(scheduler.run_script("ran").unwrap(), JsValue::Bool(false));
    }

    fn checkbox_scheduler() -> crate::js_scheduler::DriverScheduler {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(
                "<body><input type=checkbox id=box><input type=text id=field>\
                 <label for=box id=lbl>label</label></body>"),
            native_forwarded: false,
        }));
        crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap()
    }

    // NOTE (IA-TESTDRIVER): `__amazing_td_click` runs the real click-activation
    // path (`Operation::HTMLElementClick`), which toggles the checkbox as a
    // genuine default action -- but that path's own event dispatch always goes
    // through `event_bindings::invoke(host, O::DispatchEvent, ...)`, which is
    // the *script*-triggered dispatch operation and therefore marks the event
    // untrusted (`DispatchOrigin::Script`, see amazing-dom/src/events.rs:617).
    // A driver-generated click should be trusted (WebDriver's own "Element
    // Click" is a real input action, not a script call to `.click()`). Filed
    // to dom-bindings/questions.md: expose a trusted variant of the
    // click-activation sequence. Tracked here so a future fix flips this
    // assertion from `false` to `true` without anyone having to rediscover it.
    #[test]
    fn td_click_toggles_a_checkbox_via_the_real_click_default_action() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script(
            "var box = document.getElementById('box'); var clicks = 0; var trusted_clicks = 0; \
             box.addEventListener('click', function(e) { clicks += 1; trusted_clicks += (e.isTrusted ? 1 : 0); });"
        ).unwrap();
        scheduler.run_script("__amazing_td_click(document.getElementById('box'))").unwrap();
        assert_eq!(scheduler.run_script("box.checked").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("clicks").unwrap(), JsValue::num(1.0));
        // Known gap (see comment above): should be 1, currently 0.
        assert_eq!(scheduler.run_script("trusted_clicks").unwrap(), JsValue::num(0.0));
    }

    // NOTE (IA-TESTDRIVER): clicking a `<label>` does not currently forward
    // activation to its associated control. `Dom::begin_event_dispatch` only
    // ever calls `prepare_input_activation` on the click's activation target
    // (amazing-dom/src/events.rs:790-794), and `prepare_input_activation`
    // (amazing-dom/src/input_activation.rs) returns `None` for anything that
    // is not an `<input>`, even though `has_html_activation_behavior` already
    // lists "label" as activation-bearing. `Dom::label_control` exists
    // (amazing-dom/src/forms.rs:901) but nothing wires it into activation.
    // Filed to dom-bindings/questions.md. Flip this assertion to `true` once
    // that lands.
    #[test]
    fn td_click_on_a_label_does_not_yet_forward_to_its_associated_checkbox() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script("__amazing_td_click(document.getElementById('lbl'))").unwrap();
        assert_eq!(scheduler.run_script("document.getElementById('box').checked").unwrap(), JsValue::Bool(false));
    }

    #[test]
    fn td_send_keys_types_into_a_text_field_and_fires_input() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script(
            "var field = document.getElementById('field'); var inputs = 0; \
             field.addEventListener('input', function() { inputs += 1; });"
        ).unwrap();
        scheduler.run_script("__amazing_td_send_keys(document.getElementById('field'), 'hi')").unwrap();
        assert_eq!(scheduler.run_script("field.value").unwrap(), JsValue::str("hi"));
        assert_eq!(scheduler.run_script("inputs").unwrap(), JsValue::num(2.0));
    }

    #[test]
    fn td_send_keys_backspace_removes_the_last_character() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script("__amazing_td_send_keys(document.getElementById('field'), 'ab\u{E003}')").unwrap();
        assert_eq!(scheduler.run_script("document.getElementById('field').value").unwrap(), JsValue::str("a"));
    }

    #[test]
    fn td_send_keys_does_not_mutate_a_checkboxs_value() {
        let mut scheduler = checkbox_scheduler();
        let before = scheduler.run_script("document.getElementById('box').value").unwrap();
        scheduler.run_script("__amazing_td_send_keys(document.getElementById('box'), 'x')").unwrap();
        assert_eq!(scheduler.run_script("document.getElementById('box').value").unwrap(), before);
    }

    #[test]
    fn td_action_sequence_pointer_down_up_on_an_element_origin_clicks_it() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script(
            "var seq = [{type: 'pointer', id: 'p', actions: [\
                {type: 'pointerMove', x: 0, y: 0, origin: document.getElementById('box')}, \
                {type: 'pointerDown', button: 0}, \
                {type: 'pointerUp', button: 0}]}]; \
             __amazing_td_action_sequence(seq, null);"
        ).unwrap();
        assert_eq!(scheduler.run_script("document.getElementById('box').checked").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn td_action_sequence_key_source_types_into_the_active_element() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script("document.getElementById('field').focus();").unwrap();
        scheduler.run_script(
            "var seq = [{type: 'key', id: 'k', actions: [\
                {type: 'keyDown', value: 'z'}, {type: 'keyUp', value: 'z'}]}]; \
             __amazing_td_action_sequence(seq, null);"
        ).unwrap();
        assert_eq!(scheduler.run_script("document.getElementById('field').value").unwrap(), JsValue::str("z"));
    }

    #[test]
    fn td_get_computed_role_reports_a_checkbox() {
        let mut scheduler = checkbox_scheduler();
        assert_eq!(
            scheduler.run_script("__amazing_td_get_computed_role(document.getElementById('box'))").unwrap(),
            JsValue::str("checkbox"),
        );
    }

    #[test]
    fn td_get_computed_label_reports_the_associated_labels_text() {
        // HTML-AAM: a `<label for>` supplies its target control's accessible
        // name -- the checkbox's own computed label, not the `<label>`
        // element's (a `<label>`'s own role/name are not what a real
        // WPT test queries this for).
        let mut scheduler = checkbox_scheduler();
        assert_eq!(
            scheduler.run_script("__amazing_td_get_computed_label(document.getElementById('box'))").unwrap(),
            JsValue::str("label"),
        );
    }

    #[test]
    fn td_delete_all_cookies_and_set_permission_resolve_without_throwing() {
        let mut scheduler = checkbox_scheduler();
        scheduler.run_script("__amazing_td_delete_all_cookies(null)").unwrap();
        scheduler.run_script(
            "__amazing_td_set_permission({descriptor: {name: 'geolocation'}, state: 'granted'}, null)"
        ).unwrap();
    }

    #[test]
    fn td_get_client_rects_and_elements_from_point_are_defined_and_consistent() {
        let mut scheduler = checkbox_scheduler();
        let rects = scheduler.run_script(
            "var r = __amazing_td_get_client_rects(document.getElementById('box'))[0]; \
             [typeof r.x, typeof r.y, typeof r.width, typeof r.height].join(',')"
        ).unwrap();
        assert_eq!(rects, JsValue::str("number,number,number,number"));
    }
}
