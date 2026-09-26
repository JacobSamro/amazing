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
}

impl IaRuntime {
    pub(super) fn new() -> Self {
        let default_environment = amazing_css::MediaEnvironment::default();
        Self {
            viewport_width_px: default_environment.viewport_width_px,
            viewport_height_px: default_environment.viewport_height_px,
            created_at: std::time::Instant::now(),
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
        Ok(())
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
        }
    }
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
}
