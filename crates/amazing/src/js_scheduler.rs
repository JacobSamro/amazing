//! Driver-owned JS-4 scheduling adapter. Every interpreter entry returns through
//! `enter`, which transfers the complete ordered outbox before another entry.
use std::{cell::RefCell, collections::{BTreeMap, VecDeque}, rc::Rc, time::{Duration, Instant}};

use amazing_dom::event_loop::{Action, AnimationFrameId, CallbackId, EventLoop, GlobalId, RenderPhase, TaskSource, TimerId};
use amazing_dom::abort::AbortSignalId;
use amazing_js::{DomHost, interp::{Interpreter, NativeMicrotaskKind, NativeTaskToken, SchedulingRequest, SchedulingToken}, value::{JsValue, Throw}};

use crate::dom_host::BrowserDomHost;

pub(crate) const GLOBAL: GlobalId = GlobalId(1);
pub(crate) const JS_CAPACITY: usize = 4096;

#[derive(Clone, Copy)]
enum Payload {
    Job(SchedulingToken),
    Callback(SchedulingToken),
    Mutation,
    AbortTimeout(AbortSignalId, u64),
    FormSelection(u32),
    NativeSelection(NativeTaskToken),
}
struct TimerRegistration {
    core: TimerId,
    callback: CallbackId,
    repeat: bool,
}

pub(crate) struct DriverScheduler {
    pub(crate) js: Interpreter,
    host: Rc<RefCell<BrowserDomHost>>,
    bindings: crate::runtime_binding_host::RuntimeBindings,
    loop_: EventLoop,
    pending: VecDeque<SchedulingRequest>,
    payloads: BTreeMap<u64, Payload>,
    timers: BTreeMap<u32, TimerRegistration>,
    frames: BTreeMap<u32, (AnimationFrameId, CallbackId)>,
    next_callback: u64,
    started: Instant,
    now: Duration,
    trace: Vec<String>,
    pending_resize: bool,
    pending_scroll: Vec<u32>,
    render_hook: Option<Rc<dyn Fn()>>,
}

impl DriverScheduler {
    pub(crate) fn new(host: Rc<RefCell<BrowserDomHost>>, js_capacity: usize) -> Result<Self, Throw> {
        let mut js = Interpreter::new_with_external_scheduler(js_capacity)?;
        js.set_dom_host(host.clone());
        let bindings = crate::runtime_binding_host::install(&mut js, host.clone())?;
        let mut loop_ = EventLoop::new();
        loop_.register_global(GLOBAL).map_err(loop_error)?;
        Ok(Self {
            js, host, bindings, loop_, pending: VecDeque::new(), payloads: BTreeMap::new(),
            timers: BTreeMap::new(), frames: BTreeMap::new(), next_callback: 1,
            started: Instant::now(), now: Duration::ZERO, trace: Vec::new(),
            pending_resize: false, pending_scroll: Vec::new(), render_hook: None,
        })
    }

    pub(crate) fn run_script(&mut self, source: &str) -> Result<JsValue, Throw> {
        self.enter(|js| js.run_script(source))
    }
    pub(crate) fn run_page_script(&mut self, script: &crate::cmd::js::PageScript,
        source: &str, url: &str) -> Result<JsValue, Throw> {
        let bindings = self.bindings.clone();
        self.enter(|js| bindings.execute_page_script(js, script, source, url))
    }
    pub(crate) fn load_classic_page_script(&mut self, script: &crate::cmd::js::PageScript,
        base: &str) -> Result<(String, String), Throw> {
        self.bindings.load_classic_page_script(&mut self.js, script, base)
    }
    pub(crate) fn set_module_source_loader(&mut self,
        loader: Rc<crate::runtime_binding_host::modules::ModuleSourceLoader>) {
        self.bindings.set_module_source_loader(loader);
    }
    pub(crate) fn set_module_resource_loader(&mut self,
        loader: Rc<crate::runtime_binding_host::modules::ModuleResourceLoader>) {
        self.bindings.set_module_resource_loader(loader);
    }
    pub(crate) fn load_module_source(&self, url: &str) -> Result<String, String> {
        self.bindings.load_module_source(&self.js, url)
    }
    pub(crate) fn page_script_url(&self, page: &str, script: &crate::cmd::js::PageScript) -> String {
        self.bindings.page_script_url(&self.js, script, page)
    }
    pub(crate) fn finish_module_script(&mut self, script: &crate::cmd::js::PageScript,
        error: Option<&Throw>, url: &str, fetched: bool) -> Result<(), Throw> {
        let bindings = self.bindings.clone();
        self.enter(|js| bindings.finish_module_script(js, script, error, url, fetched))
    }
    pub(crate) fn set_location_url(&mut self, url: impl Into<String>) {
        let url = url.into();
        self.js.set_location_url(&url);
        self.bindings.set_document_url(&mut self.js, &url);
        // Top realm origin, so same-origin iframe WindowProxy access (JSRT-9)
        // compares against a real value rather than the "" default.
        if let Ok(parsed) = amazing_net::url::ParsedUrl::parse(&url) {
            let realm = self.js.binding_current_realm();
            let _ = self.js.binding_set_realm_origin(realm, &parsed.origin().serialize());
        }
    }
    /// IA-LIVE-3: the driver's real viewport, for `window.matchMedia` to
    /// evaluate size-based media features against (`runtime_host_
    /// integrator_assist.rs`). Same shape as `set_location_url` above — both
    /// tell the interpreter something only the driver knows.
    pub(crate) fn set_viewport(&mut self, width: f32, height: f32) {
        self.bindings.set_viewport(width, height);
    }
    pub(crate) fn set_frame_resource_resolver(&mut self,
        resolver: Rc<dyn Fn(&str) -> Option<String>>) {
        self.bindings.set_frame_resource_resolver(resolver);
    }
    pub(crate) fn set_frame_resource_loader(&mut self,
        loader: crate::runtime_binding_host::html::HtmlFrameResourceLoader) {
        self.bindings.set_frame_resource_loader(loader);
    }
    pub(crate) fn set_child_navigation_hook(&mut self,
        hook: crate::runtime_binding_host::html::ChildNavigationHook) {
        self.bindings.set_child_navigation_hook(hook);
    }
    pub(crate) fn set_form_navigation_hook(&mut self,
        hook: crate::runtime_binding_host::html::FormNavigationHook) {
        self.bindings.set_form_navigation_hook(hook);
    }
    pub(crate) fn queue_resize_event(&mut self) { self.pending_resize = true; }
    pub(crate) fn queue_scroll_event(&mut self, node: u32) {
        if !self.pending_scroll.contains(&node) { self.pending_scroll.push(node); }
    }
    pub(crate) fn set_render_hook(&mut self, hook: Rc<dyn Fn()>) {
        self.render_hook = Some(hook);
    }
    pub(crate) fn dispatch_pending_iframe_loads(&mut self) -> Result<(), Throw> {
        self.dispatch_iframe_loads(None)
    }
    /// Only current eager child navigations delay this actual Document load.
    /// Lazy navigation completion remains queued for the normal task turn.
    pub(crate) fn dispatch_load_blocking_iframe_loads(&mut self, document: amazing_dom::NodeIdx) -> Result<(), Throw> {
        self.dispatch_iframe_loads(Some(document))
    }
    fn dispatch_iframe_loads(&mut self, owner: Option<amazing_dom::NodeIdx>) -> Result<(), Throw> {
        while let Some(load) = self.bindings.take_next_iframe_load(owner) {
            let frame = load.frame;
            let document = load.document;
            let fire_load_events = load.fire_load_events;
            if let Some(document) = document {
                // HTML-29: brackets the child document's own top-level
                // script phase (`setup_iframe_window` narrows the point per
                // script) so `document.write` from one of its scripts
                // inserts at the parser's insertion point instead of always
                // implicitly reopening the document; ended once its load
                // event has fired, matching a real parser.
                self.bindings.begin_document_parsing(document);
                let bindings = self.bindings.clone();
                self.enter(|js| bindings.setup_iframe_window(js, frame, document))?;
                if fire_load_events && self.bindings.iframe_load_is_current(&load) {
                    self.dispatch_dom_event(document.0, "DOMContentLoaded")?;
                    // A child load waits for newly discovered dependent frames.
                    // Their own Window load precedes this Document's Window load.
                    self.dispatch_load_blocking_iframe_loads(document)?;
                    if self.bindings.iframe_load_is_current(&load) {
                        self.dispatch_document_load(document)?;
                    }
                }
                self.bindings.end_document_parsing(document);
            }
            if fire_load_events && self.bindings.iframe_load_is_current(&load) {
                // HTML fires the embedding element's load separately, with
                // default flags; it must not bubble into its owner Document.
                self.dispatch_user_agent_payload(amazing_dom::events::EventTarget::Node(frame),
                    amazing_dom::events::Event::new("load", amazing_dom::events::EventInit::default(),
                        std::time::Duration::ZERO))?;
            }
        }
        Ok(())
    }
    /// HTML-29: see `RuntimeBindings::begin_document_parsing`.
    pub(crate) fn begin_document_parsing(&mut self, document: u32) {
        self.bindings.begin_document_parsing(amazing_dom::NodeIdx(document));
    }
    /// HTML-29: see `RuntimeBindings::narrow_write_insertion_point`.
    pub(crate) fn narrow_write_insertion_point(&mut self, document: u32, script_node: u32) {
        self.bindings.narrow_write_insertion_point(amazing_dom::NodeIdx(document), amazing_dom::NodeIdx(script_node));
    }
    /// HTML-29: see `RuntimeBindings::end_document_parsing`.
    pub(crate) fn end_document_parsing(&mut self, document: u32) {
        self.bindings.end_document_parsing(amazing_dom::NodeIdx(document));
    }
    /// HTML-31: see `RuntimeBindings::register_active_parser`.
    pub(crate) fn register_active_parser(&mut self, document: u32, parser: Rc<RefCell<amazing_html::ResumableParser>>) {
        self.bindings.register_active_parser(amazing_dom::NodeIdx(document), parser);
    }
    /// HTML-31: see `RuntimeBindings::unregister_active_parser`.
    pub(crate) fn unregister_active_parser(&mut self, document: u32) {
        self.bindings.unregister_active_parser(amazing_dom::NodeIdx(document));
    }
    /// Allocate connected parser-created child contexts without dispatching
    /// their queued load events. The caller has exposed the actual parsed DOM.
    pub(crate) fn refresh_parser_browsing_contexts(&mut self) -> Result<(), Throw> {
        let bindings = self.bindings.clone();
        // Context setup is not an author-script/reaction checkpoint. In particular,
        // do not materialize named globals from a partially parsed document.
        self.drain_outbox()?;
        bindings.navigate_iframes(&mut self.js);
        self.collect_native_mutations()?;
        self.drain_outbox()
    }
    pub(crate) fn set_document_metadata(&mut self, character_set: &str, content_type: &str, referrer: &str) {
        self.bindings.set_document_metadata(character_set, content_type, referrer);
    }
    pub(crate) fn set_document_content_type(&mut self, content_type: &str) {
        self.bindings.set_document_content_type(content_type);
    }
    pub(crate) fn set_image_loader_states(&mut self, media: &crate::page::PageMedia) -> Result<(), Throw> {
        self.bindings.set_image_loader_states(&mut self.js, media)?;
        self.drain_outbox()
    }
    /// HTML's Window load uses the associated Document as its legacy target.
    /// Public dispatchEvent and Document DOMContentLoaded keep their usual paths.
    pub(crate) fn dispatch_document_load(&mut self, document: amazing_dom::NodeIdx) -> Result<(), Throw> {
        let bindings = self.bindings.clone();
        self.enter(|js| bindings.dispatch_document_load(js, document))
    }

    pub(crate) fn dispatch_dom_event(&mut self, node: u32, event: &str) -> Result<(), Throw> {
        let bindings = self.bindings.clone();
        self.enter(|js| {
            js.dispatch_dom_event(node, event)?;
            bindings.dispatch_event(js, node, event)
        })
    }
    pub(crate) fn dispatch_animation_event(&mut self, event: &amazing_compositor::CssAnimationEvent) -> Result<(), Throw> {
        let bindings = self.bindings.clone();
        self.enter(|js| bindings.dispatch_animation_event(js, event))
    }
    pub(crate) fn dispatch_user_agent_payload(&mut self,target:amazing_dom::events::EventTarget,
        event:amazing_dom::events::Event)->Result<bool,Throw> {
        let bindings=self.bindings.clone();
        self.enter(|js|bindings.dispatch_user_agent_payload(js,target,event))
    }
    pub(crate) fn install_inline_handler(&mut self, node: u32, event: &str, body: &str) -> Result<(), Throw> {
        let bindings = self.bindings.clone();
        self.enter(|js| bindings.install_canonical_inline_handler(js, node, event, body))
    }

    fn enter<T>(&mut self, f: impl FnOnce(&mut Interpreter) -> Result<T, Throw>) -> Result<T, Throw> {
        // A prior downstream admission failure leaves its taken requests here.
        // Retry them before allowing another JS entry; never silently discard one.
        self.drain_outbox()?;
        let before = self.js.output.borrow().len();
        let result = f(&mut self.js);
        let bindings = self.bindings.clone();
        let reactions = bindings.deliver_reactions(&mut self.js);
        self.trace.extend(self.js.output.borrow()[before..].iter().cloned());
        let collected = self.collect_native_mutations();
        let drained = self.drain_outbox();
        reactions?;
        collected?;
        drained?;
        result
    }

    fn collect_native_mutations(&mut self) -> Result<(), Throw> {
        let pending = self.host.borrow().pending_native_microtasks();
        for request in pending {
            self.js.enqueue_native_microtask(request.global, request.kind)?;
            self.host.borrow_mut().acknowledge_native_microtask(request);
        }
        Ok(())
    }

    fn own(&mut self, payload: Payload) -> Result<CallbackId, Throw> {
        let id = self.next_callback;
        self.next_callback = id.checked_add(1)
            .ok_or_else(|| Throw::range_error("driver callback IDs exhausted"))?;
        self.payloads.insert(id, payload);
        Ok(CallbackId(id))
    }

    fn drain_outbox(&mut self) -> Result<(), Throw> {
        self.bindings.emit_native_selection_tasks(&mut self.js)?;
        self.pending.extend(self.js.take_scheduling_requests());
        while let Some(request) = self.pending.front().copied() {
            self.admit(request)?;
            self.pending.pop_front();
        }
        // Cross-document storage notifications enter this document as tasks,
        // after requests already emitted by its own script execution.
        self.bindings.schedule_storage_events(&mut self.js)?;
        // Cross-document messages (postMessage/MessageChannel/BroadcastChannel)
        // enter this document as tasks at the same checkpoint.
        self.bindings.schedule_messages(&mut self.js)?;
        self.pending.extend(self.js.take_scheduling_requests());
        while let Some(request) = self.pending.front().copied() {
            self.admit(request)?;
            self.pending.pop_front();
        }
        for (signal, delay_ms) in self.bindings.take_abort_timeouts() {
            self.register_abort_timeout(signal, delay_ms)?;
        }
        for node in self.bindings.take_form_selection_events() {
            let callback = self.own(Payload::FormSelection(node.0))?;
            self.loop_.queue_task(TaskSource::UserInteraction, Some(GLOBAL), callback)
                .map_err(loop_error)?;
        }
        Ok(())
    }

    fn register_abort_timeout(&mut self, signal: AbortSignalId, delay_ms: u64) -> Result<(), Throw> {
        let chunk = delay_ms.min(i32::MAX as u64);
        let callback = self.own(Payload::AbortTimeout(signal, delay_ms - chunk))?;
        if let Err(error) = self.loop_.set_timer(GLOBAL, callback, chunk as i32, false) {
            self.payloads.remove(&callback.0);
            return Err(loop_error(error));
        }
        Ok(())
    }

    fn admit(&mut self, request: SchedulingRequest) -> Result<(), Throw> {
        match request {
            SchedulingRequest::Timer { registration, callback_token, delay_ms, repeat } => {
                let callback = self.own(Payload::Callback(callback_token))?;
                let core = match self.loop_.set_timer(GLOBAL, callback, delay_ms, repeat) {
                    Ok(id) => id,
                    Err(error) => { self.payloads.remove(&callback.0); return Err(loop_error(error)); }
                };
                self.timers.insert(registration, TimerRegistration { core, callback, repeat });
            }
            SchedulingRequest::CancelTimer { registration } => {
                if let Some(entry) = self.timers.remove(&registration) {
                    self.loop_.clear_timer(entry.core);
                    self.payloads.remove(&entry.callback.0);
                }
            }
            SchedulingRequest::AnimationFrame { registration, callback_token } => {
                let callback = self.own(Payload::Callback(callback_token))?;
                let core = match self.loop_.request_animation_frame(GLOBAL, callback) {
                    Ok(id) => id,
                    Err(error) => { self.payloads.remove(&callback.0); return Err(loop_error(error)); }
                };
                self.frames.insert(registration, (core, callback));
            }
            SchedulingRequest::CancelAnimationFrame { registration } => {
                if let Some((core, callback)) = self.frames.get(&registration).copied() {
                    self.loop_.cancel_animation_frame(GLOBAL, core).map_err(loop_error)?;
                    self.frames.remove(&registration);
                    self.payloads.remove(&callback.0);
                }
            }
            SchedulingRequest::Microtask { job_token } => {
                let callback = self.own(Payload::Job(job_token))?;
                if let Err(error) = self.loop_.queue_microtask(GLOBAL, callback) {
                    self.payloads.remove(&callback.0);
                    return Err(loop_error(error));
                }
            }
            SchedulingRequest::NativeTask { token } => {
                if self.bindings.native_selection_task(token)?.is_none() {
                    self.bindings.discard_native_selection_task(token)?;
                } else {
                    let callback = self.own(Payload::NativeSelection(token))?;
                    if let Err(error) = self.loop_.queue_task(TaskSource::UserInteraction, Some(GLOBAL), callback) {
                        self.payloads.remove(&callback.0);
                        return Err(loop_error(error));
                    }
                    // Existing root scheduling-global policy remains explicit;
                    // target Document generation and creation realm are independent.
                    self.bindings.native_selection_admitted(token);
                }
            }
            SchedulingRequest::NativeMicrotask { global, kind } => {
                if global != GLOBAL.0 || kind != NativeMicrotaskKind::MutationObserver {
                    return Err(Throw::type_error("unknown native microtask request"));
                }
                let callback = self.own(Payload::Mutation)?;
                if let Err(error) = self.loop_.queue_microtask(GLOBAL, callback) {
                    self.payloads.remove(&callback.0);
                    return Err(loop_error(error));
                }
            }
        }
        Ok(())
    }

    fn invoke(&mut self, callback: CallbackId, timestamp: Option<Duration>) -> Result<(), Throw> {
        let payload = self.payloads.get(&callback.0).copied()
            .ok_or_else(|| Throw::type_error("unknown driver callback"))?;
        match payload {
            Payload::Job(token) => {
                self.payloads.remove(&callback.0);
                self.enter(|js| js.invoke_one_job(token))
            }
            Payload::Callback(token) => {
                // A repeating timer retains its token and payload. A one-shot
                // timer or frame is removed after invocation, including throws.
                let repeating = self.timers.values().any(|timer| timer.callback == callback && timer.repeat);
                if !repeating {
                    self.payloads.remove(&callback.0);
                    self.timers.retain(|_, timer| timer.callback != callback);
                    self.frames.retain(|_, (_, id)| *id != callback);
                }
                let receiver = JsValue::Object(self.js.global_obj.clone());
                self.enter(|js| js.invoke_one_callback(token, receiver, timestamp))
            }
            Payload::Mutation => {
                self.payloads.remove(&callback.0);
                let bindings = self.bindings.clone();
                let result = self.enter(|js| bindings.deliver_mutations(js));
                self.host.borrow_mut().native_forwarded = false;
                self.trace.push("M".into());
                result
            }
            Payload::AbortTimeout(signal, remaining) => {
                self.payloads.remove(&callback.0);
                if remaining > 0 { return self.register_abort_timeout(signal, remaining); }
                let bindings = self.bindings.clone();
                self.enter(|js| bindings.fire_abort_timeout(js, signal))
            }
            Payload::NativeSelection(token) => {
                self.payloads.remove(&callback.0);
                let bindings = self.bindings.clone();
                let task = bindings.release_native_selection_task(token)?;
                self.enter(|js| bindings.fire_selection_change(js, task))
            }
            Payload::FormSelection(node) => {
                self.payloads.remove(&callback.0);
                self.dispatch_dom_event(node, "select")
            }
        }
    }

    fn update_rendering(&mut self, phase: RenderPhase) -> Result<(), Throw> {
        match phase {
            RenderPhase::BeforeAnimationFrames => {
                if self.pending_resize {
                    self.pending_resize = false;
                    self.dispatch_dom_event(0, "resize")?;
                }
                for node in std::mem::take(&mut self.pending_scroll) {
                    self.dispatch_dom_event(node, "scroll")?;
                }
            }
            RenderPhase::AfterAnimationFrames => {
                if let Some(hook) = &self.render_hook { hook(); }
            }
        }
        Ok(())
    }

    /// Pump one explicit driver clock/render opportunity until the loop is idle.
    /// Callback exceptions are returned after the step has been completed and
    /// all newly emitted requests have been admitted.
    pub(crate) fn pump_at(&mut self, now: Duration, render: bool) -> Result<bool, Throw> {
        self.pump_at_with_render(now, render, |_, _| {})
    }

    pub(crate) fn has_pending_network_io(&self) -> bool {
        self.bindings.has_pending_network_io()
    }

    /// Like [`Self::pump_at`], but invokes `on_render` for each `Action::Render`
    /// the event loop yields. The loop orders resize/scroll, then rAF callbacks,
    /// then the `AfterAnimationFrames` render step, so a compositor can tick at
    /// the frame boundary without a second clock. Drivers that do not render in
    /// the loop keep using `pump_at`, which ignores the actions.
    pub(crate) fn pump_at_with_render(
        &mut self,
        now: Duration,
        render: bool,
        mut on_render: impl FnMut(RenderPhase, Duration),
    ) -> Result<bool, Throw> {
        self.drain_outbox()?;
        self.dispatch_pending_iframe_loads()?;
        self.now = self.now.max(now);
        self.loop_.advance_to(self.now).map_err(loop_error)?;
        // Update rendering follows the task and microtask checkpoint for this
        // turn. In particular, a timer may register a frame callback before
        // this opportunity begins.
        let mut did_work = false;
        let mut first_error = None;
        let mut rendering_queued = !render;
        for _ in 0..10_000 {
            let step = match self.loop_.step().map_err(loop_error)? {
                Some(step) => step,
                None if !rendering_queued => {
                    rendering_queued = true;
                    self.loop_.rendering_opportunity(GLOBAL, self.now).map_err(loop_error)?;
                    continue;
                }
                None => return first_error.map_or(Ok(did_work), Err),
            };
            let result = match step.action {
                Action::Task { callback, .. } | Action::Microtask { callback, .. } => {
                    did_work = true;
                    self.invoke(callback, None)
                }
                Action::AnimationFrame { callback, timestamp, .. } => {
                    did_work = true;
                    self.invoke(callback, Some(timestamp))
                }
                Action::CheckpointCleanup => { self.bindings.storage_deactivate_transactions(); Ok(()) },
                Action::Render { phase, timestamp, .. } => {
                    let result = self.update_rendering(phase);
                    on_render(phase, timestamp);
                    result
                }
            };
            if let Err(error) = result { first_error.get_or_insert(error); }
            // Even a throw or admission failure must release the in-flight step.
            let drained = self.drain_outbox();
            self.loop_.complete(step.id).map_err(loop_error)?;
            if let Err(error) = drained { return Err(error); }
        }
        Err(Throw::range_error("driver event loop step limit reached"))
    }

    pub(crate) fn pump_now(&mut self, render: bool) -> Result<bool, Throw> {
        self.pump_at(self.started.elapsed(), render)
    }

    /// Test hook for lane-owned bindings tests (see `runtime_host_messaging`).
    #[cfg(test)]
    pub(crate) fn test_bindings(&self) -> &crate::runtime_binding_host::RuntimeBindings {
        &self.bindings
    }

    /// [`Self::pump_now`] variant that ticks a host render hook at the loop's
    /// frame boundary (see [`Self::pump_at_with_render`]).
    pub(crate) fn pump_now_with_render(
        &mut self,
        render: bool,
        on_render: impl FnMut(RenderPhase, Duration),
    ) -> Result<bool, Throw> {
        self.pump_at_with_render(self.started.elapsed(), render, on_render)
    }
}

impl Drop for DriverScheduler {
    fn drop(&mut self) {
        // Do not rely on Rc teardown: retained platform wrappers may keep
        // bindings alive after queued callbacks and the global are gone.
        self.bindings.retire_native_selection_tasks();
    }
}

fn loop_error(error: amazing_dom::event_loop::LoopError) -> Throw {
    Throw::type_error(format!("EventLoop: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_dom::{mutation::ObserveOptions, Dom, NodeIdx};
    use amazing_js::DomHost;

    fn adapter(capacity: usize, observe: bool) -> (DriverScheduler, Rc<RefCell<BrowserDomHost>>) {
        let mut dom = Dom::new();
        let element = dom.create_element(NodeIdx(0), "x");
        if observe {
            let observer = dom.create_mutation_observer(1);
            dom.observe(observer, element, ObserveOptions {
                attributes: Some(true), ..Default::default()
            }).unwrap();
        }
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), capacity).unwrap();
        if observe {
            let callback = scheduler.run_script("(()=>{})").unwrap();
            scheduler.bindings.seed_test_root(amazing_dom::events::CallbackId(1), callback);
        }
        (scheduler, host)
    }

    #[test]
    fn abort_signal_timeout_fires_from_driver_timer_queue() {
        let (mut driver, _) = adapter(16, false);
        driver.run_script("globalThis.signal = AbortSignal.timeout(0)").unwrap();
        assert!(matches!(driver.run_script("signal.aborted").unwrap(), JsValue::Bool(false)));
        driver.pump_at(Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("signal.aborted").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn nested_animation_frames_then_timeout_settle_a_promise() {
        let (mut driver, _) = adapter(64, false);
        driver.run_script("globalThis.settled = false; new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 0)))).then(() => { settled = true; });").unwrap();
        for frame in 0..10 {
            driver.pump_at(Duration::from_millis(frame * 16), true).unwrap();
        }
        assert!(matches!(driver.run_script("settled").unwrap(), JsValue::Bool(true)));
    }

    #[test]
    fn promise_then_async_await_settles_across_nested_frames() {
        let (mut driver, _) = adapter(64, false);
        driver.run_script("globalThis.done = false; Promise.resolve().then(async () => { await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 0)))); done = true; });").unwrap();
        for frame in 0..10 {
            driver.pump_at(Duration::from_millis(frame * 16), true).unwrap();
        }
        assert!(matches!(driver.run_script("done").unwrap(), JsValue::Bool(true)));
    }

    #[test]
    fn detached_input_select_queues_select_event() {
        let (mut driver, _) = adapter(64, false);
        driver.run_script("globalThis.hit = 0; var i = document.createElement('input'); i.type = 'text'; i.value = 'foobar'; i.addEventListener('select', () => { hit++; }); i.select();").unwrap();
        driver.pump_at(Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("hit").unwrap(), JsValue::Num(n) if n == 1.0));
    }

    #[test]
    fn detached_clone_select_queues_select_event() {
        let (mut driver, _) = adapter(64, false);
        // The source control's live selection must not be cloned: the clone
        // starts at the default end selection, so select() changes it and fires.
        driver.run_script("globalThis.hit = 0; var i = document.createElement('input'); i.type = 'text'; i.value = 'foobar'; i.select(); var c = i.cloneNode(true); c.addEventListener('select', () => { hit++; }); c.select();").unwrap();
        driver.pump_at(Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("hit").unwrap(), JsValue::Num(n) if n == 1.0));
    }

    #[test]
    #[ignore = "blocked on js-lane: tree-walking `await` on a pending promise returns immediately instead of suspending (interp.rs:1980)"]
    fn await_suspends_until_timer_fires() {
        let (mut driver, _) = adapter(64, false);
        driver.run_script("globalThis.order = ''; (async () => { order += 'A'; await new Promise(resolve => setTimeout(resolve, 1000)); order += 'B'; })(); order += 'C';").unwrap();
        assert_eq!(driver.run_script("order").unwrap(), JsValue::str("AC"));
        driver.pump_at(Duration::from_millis(1200), false).unwrap();
        assert_eq!(driver.run_script("order").unwrap(), JsValue::str("ACB"));
    }

    #[test]
    fn validity_flags_are_computed_for_disabled_and_readonly_controls() {
        let (mut driver, _) = adapter(64, false);
        // valueMissing is mutability-gated: a disabled/readonly required empty
        // control is not missing. Size/pattern/range/steps are independent.
        driver.run_script("var i = document.createElement('input'); i.required = true; globalThis.cv = i.validity.valueMissing; globalThis.cc = i.validity.valid; var c = i.cloneNode(true); globalThis.kv = c.validity.valueMissing; globalThis.kc = c.validity.valid;").unwrap();
        assert!(matches!(driver.run_script("cv").unwrap(), JsValue::Bool(true)), "connected valueMissing");
        assert!(matches!(driver.run_script("cc").unwrap(), JsValue::Bool(false)), "connected valid");
        assert!(matches!(driver.run_script("kv").unwrap(), JsValue::Bool(true)), "cloned valueMissing");
        assert!(matches!(driver.run_script("kc").unwrap(), JsValue::Bool(false)), "cloned valid");
        // A disabled or readonly control with a pattern mismatch is still invalid.
        driver.run_script("var p = document.createElement('input'); p.pattern = '[A-Z]'; p.value = 'abc'; globalThis.pm = p.validity.patternMismatch; globalThis.pv = p.validity.valid; var d = p.cloneNode(true); d.disabled = true; globalThis.dm = d.validity.patternMismatch; globalThis.dv = d.validity.valid; var r = p.cloneNode(true); r.readOnly = true; globalThis.rv = r.validity.valid; var n = document.createElement('input'); n.type = 'number'; n.max = '5'; n.value = '6'; globalThis.rm = n.validity.rangeOverflow; globalThis.rvm = n.validity.valid; var dn = n.cloneNode(true); dn.disabled = true; globalThis.drm = dn.validity.rangeOverflow; globalThis.drv = dn.validity.valid;").unwrap();
        for (name, expected) in [("pm", true), ("pv", false), ("dm", true), ("dv", false), ("rv", false), ("rm", true), ("rvm", false), ("drm", true), ("drv", false)] {
            let value = driver.run_script(name).unwrap();
            assert!(matches!(value, JsValue::Bool(v) if v == expected), "validity {name}");
        }
    }

    #[test]
    fn input_type_change_preserves_undirtied_default_value() {
        let (mut driver, _) = adapter(16, false);
        driver.run_script("var i = document.createElement('input'); i.type = 'color'; globalThis.v1 = i.value; i.type = 'text'; globalThis.v2 = i.value; i.type = 'color'; globalThis.v3 = i.value; i.value = '#ffffff'; globalThis.v4 = i.value; i.type = 'text'; globalThis.v5 = i.value;").unwrap();
        for (name, expected) in [
            ("v1", "#000000"), ("v2", ""), ("v3", "#000000"),
            ("v4", "#ffffff"), ("v5", "#ffffff"),
        ] {
            assert_eq!(driver.run_script(name).unwrap(), JsValue::str(expected), "color {name}");
        }
    }

    #[test]
    fn rendering_dispatches_resize_before_frames_and_paints_after() {
        let (mut driver, _) = adapter(64, false);
        let painted = Rc::new(RefCell::new(false));
        let painted_for_hook = painted.clone();
        driver.set_render_hook(Rc::new(move || *painted_for_hook.borrow_mut() = true));
        driver.run_script("globalThis.order = ''; window.addEventListener('resize', () => { order += 'R'; }); requestAnimationFrame(() => { order += 'F'; });").unwrap();
        driver.queue_resize_event();
        driver.pump_at(Duration::from_micros(16_667), true).unwrap();
        assert_eq!(driver.run_script("order").unwrap(), JsValue::str("RF"));
        assert!(*painted.borrow());
    }

    #[test]
    fn driver_orders_timer_promise_microtask_and_frame_cancellation() {
        let (mut driver, _) = adapter(16, false);
        driver.run_script("setTimeout(()=>{console.log('A'); Promise.resolve().then(()=>{console.log('P'); queueMicrotask(()=>console.log('R'))}); queueMicrotask(()=>console.log('Q'))},0); setTimeout(()=>console.log('B'),0)").unwrap();
        driver.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(driver.trace, ["A", "P", "Q", "R", "B"]);
        println!("TRACE timer-promise-microtask: {}", driver.trace.join(","));

        let (mut frame, _) = adapter(16, false);
        frame.run_script("let second; requestAnimationFrame(()=>{console.log('F1'); Promise.resolve().then(()=>cancelAnimationFrame(second))}); second=requestAnimationFrame(()=>console.log('F2'))").unwrap();
        frame.pump_at(Duration::from_millis(16), true).unwrap();
        assert_eq!(frame.trace, ["F1"]);
        println!("TRACE frame-cancel: {}", frame.trace.join(","));
    }

    #[test]
    fn driver_render_hook_brackets_animation_frames() {
        let (mut driver, _) = adapter(16, false);
        driver.run_script("requestAnimationFrame(()=>console.log('F'))").unwrap();
        let phases = Rc::new(RefCell::new(Vec::<&'static str>::new()));
        let record = phases.clone();
        driver.pump_at_with_render(Duration::from_millis(16), true, move |phase, timestamp| {
            let label = match phase {
                RenderPhase::BeforeAnimationFrames => "before",
                RenderPhase::AfterAnimationFrames => "after",
            };
            assert_eq!(timestamp, Duration::from_millis(16));
            record.borrow_mut().push(label);
        }).unwrap();
        // The loop yields the pre-rAF render step, then the rAF callback, then the
        // post-rAF render step, so a compositor ticks once per frame in order.
        assert_eq!(&*phases.borrow(), &["before", "after"]);
        assert_eq!(driver.trace, ["F"]);
        println!("TRACE render-hook-phases: before,F,after");
    }

    #[test]
    fn driver_native_mutation_preserves_capacity_and_rearms() {
        let script = "Promise.resolve().then(()=>console.log('P')); document.querySelector('x').setAttribute('watched','1'); try {queueMicrotask(()=>console.log('Q'))} catch(e) {console.log(e.name)}";
        let (mut one, host_one) = adapter(1, true);
        one.run_script(script).unwrap();
        assert!(host_one.borrow().dom.mutation_microtask_pending());
        assert!(host_one.borrow().native_forwarded);
        one.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(one.trace, ["TypeError", "P", "M"]);
        assert!(!host_one.borrow().dom.mutation_microtask_pending());
        println!("TRACE native-capacity-one: {}", one.trace.join(","));

        let (mut two, host_two) = adapter(2, true);
        two.run_script(script).unwrap();
        two.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(two.trace, ["P", "M", "Q"]);
        assert!(!host_two.borrow().native_forwarded);
        two.run_script("document.querySelector('x').setAttribute('watched','2')").unwrap();
        assert!(host_two.borrow().native_forwarded);
        two.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(two.trace, ["P", "M", "Q", "M"]);
        println!("TRACE native-rearm: {}", two.trace.join(","));
    }

    #[test]
    fn driver_drains_after_thrown_script_and_callback() {
        let (mut script, _) = adapter(8, false);
        let error = script.run_script("Promise.resolve().then(()=>console.log('P')); throw new Error('script sentinel')").unwrap_err();
        assert!(error.describe().contains("script sentinel"));
        script.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(script.trace, ["P"]);
        println!("TRACE thrown-script: {}", script.trace.join(","));

        let (mut callback, _) = adapter(8, false);
        callback.run_script("setTimeout(()=>{console.log('A'); Promise.resolve().then(()=>console.log('P')); throw new Error('callback sentinel')},0); setTimeout(()=>console.log('B'),0)").unwrap();
        let error = callback.pump_at(Duration::ZERO, false).unwrap_err();
        assert!(error.describe().contains("callback sentinel"));
        assert_eq!(callback.trace, ["A", "P", "B"]);
        println!("TRACE thrown-callback: {}", callback.trace.join(","));
    }

    #[test]
    fn driver_retains_taken_request_after_downstream_failure() {
        let (mut driver, host) = adapter(8, false);
        // Invalid global simulates a downstream admission failure. The request
        // has left the interpreter, so the driver must retain it and fail closed.
        driver.js.enqueue_native_microtask(999, NativeMicrotaskKind::MutationObserver).unwrap();
        assert!(driver.run_script("console.log('must not run')").is_err());
        assert_eq!(driver.pending.len(), 1);
        assert!(driver.run_script("console.log('still blocked')").is_err());
        assert_eq!(driver.pending.len(), 1);
        assert!(driver.js.output.borrow().is_empty());
        assert!(!host.borrow().native_forwarded);
        println!("TRACE downstream-fail-closed: retained=1 executed=0");
    }

    #[test]
    fn driver_mutation_flag_batches_until_checkpoint() {
        let (mut driver, host) = adapter(8, true);
        assert_eq!(host.borrow_mut().query_selector("x"), Some(1));
        driver.run_script("let x=document.querySelector('x'); x.setAttribute('watched','1'); x.setAttribute('watched','2')").unwrap();
        assert!(host.borrow().native_forwarded);
        assert_eq!(driver.payloads.values().filter(|p| matches!(p, Payload::Mutation)).count(), 1);
        driver.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(driver.trace, ["M"]);
        assert!(!host.borrow().native_forwarded);
        println!("TRACE mutation-batch: {}", driver.trace.join(","));
    }
}

#[cfg(test)]
#[path = "runtime_native_selection_task_tests.rs"]
mod native_selection_task_tests;
