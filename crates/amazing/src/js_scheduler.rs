//! Driver-owned JS-4 scheduling adapter. Every interpreter entry returns through
//! `enter`, which transfers the complete ordered outbox before another entry.
use std::{cell::RefCell, collections::{BTreeMap, VecDeque}, rc::Rc, time::{Duration, Instant}};

use amazing_dom::event_loop::{Action, AnimationFrameId, CallbackId, EventLoop, GlobalId, TimerId};
use amazing_js::{DomHost, interp::{Interpreter, NativeMicrotaskKind, SchedulingRequest, SchedulingToken}, value::{JsValue, Throw}};

use crate::dom_host::BrowserDomHost;

pub(crate) const GLOBAL: GlobalId = GlobalId(1);
pub(crate) const JS_CAPACITY: usize = 4096;

#[derive(Clone, Copy)]
enum Payload {
    Job(SchedulingToken),
    Callback(SchedulingToken),
    Mutation,
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
        })
    }

    pub(crate) fn run_script(&mut self, source: &str) -> Result<JsValue, Throw> {
        self.enter(|js| js.run_script(source))
    }
    pub(crate) fn dispatch_dom_event(&mut self, node: u32, event: &str) -> Result<(), Throw> {
        self.enter(|js| js.dispatch_dom_event(node, event))
    }
    pub(crate) fn install_inline_handler(&mut self, node: u32, event: &str, body: &str) -> Result<(), Throw> {
        self.enter(|js| js.install_inline_handler(node, event, body))
    }

    fn enter<T>(&mut self, f: impl FnOnce(&mut Interpreter) -> Result<T, Throw>) -> Result<T, Throw> {
        // A prior downstream admission failure leaves its taken requests here.
        // Retry them before allowing another JS entry; never silently discard one.
        self.drain_outbox()?;
        let before = self.js.output.borrow().len();
        let result = f(&mut self.js);
        self.trace.extend(self.js.output.borrow()[before..].iter().cloned());
        let collected = self.collect_native_mutations();
        let drained = self.drain_outbox();
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
        self.pending.extend(self.js.take_scheduling_requests());
        while let Some(request) = self.pending.front().copied() {
            self.admit(request)?;
            self.pending.pop_front();
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
        }
    }

    /// Pump one explicit driver clock/render opportunity until the loop is idle.
    /// Callback exceptions are returned after the step has been completed and
    /// all newly emitted requests have been admitted.
    pub(crate) fn pump_at(&mut self, now: Duration, render: bool) -> Result<bool, Throw> {
        self.drain_outbox()?;
        self.now = self.now.max(now);
        self.loop_.advance_to(self.now).map_err(loop_error)?;
        if render { self.loop_.rendering_opportunity(GLOBAL, self.now).map_err(loop_error)?; }
        let mut did_work = false;
        let mut first_error = None;
        for _ in 0..10_000 {
            let Some(step) = self.loop_.step().map_err(loop_error)? else {
                return first_error.map_or(Ok(did_work), Err);
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
                Action::CheckpointCleanup | Action::Render { .. } => Ok(()),
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
            scheduler.bindings.seed_test_root(CallbackId(1), callback);
        }
        (scheduler, host)
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
