//! Real Interpreter + DOM + EventLoop adapter proof for the JS-4 outbox.
//! This is a driver integration test, not a product scheduling adapter.

use amazing_dom::event_loop::{Action, AnimationFrameId, CallbackId, EventLoop, GlobalId, TimerId};
use amazing_dom::mutation::ObserveOptions;
use amazing_dom::{Dom, NodeIdx};
use amazing_js::interp::{
    DomHost, HostResponse, Interpreter, NativeMicrotaskKind, NativeMicrotaskRequest,
    SchedulingRequest, SchedulingToken,
};
use amazing_js::value::JsValue;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

const GLOBAL: GlobalId = GlobalId(7);

struct NativeState {
    dom: Dom,
    element: NodeIdx,
    forwarded: bool,
}
impl NativeState {
    fn new() -> Self {
        let mut dom = Dom::new();
        let element = dom.create_element(NodeIdx(0), "x");
        let observer = dom.create_mutation_observer(1);
        dom.observe(
            observer,
            element,
            ObserveOptions {
                attributes: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        Self {
            dom,
            element,
            forwarded: false,
        }
    }
}
struct NativeHost(Rc<RefCell<NativeState>>);
impl DomHost for NativeHost {
    fn query_selector(&mut self, selector: &str) -> Option<u32> {
        (selector == "x").then(|| self.0.borrow().element.0)
    }
    fn query_selector_all(&mut self, _: &str) -> Vec<u32> {
        Vec::new()
    }
    fn matches_selector(&self, _: u32, _: &str) -> bool {
        false
    }
    fn parent_node(&self, _: u32) -> Option<u32> {
        None
    }
    fn get_text_content(&self, _: u32) -> Option<String> {
        None
    }
    fn set_text_content(&mut self, _: u32, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn get_document_title(&self) -> String {
        String::new()
    }
    fn set_document_title(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn fetch(&mut self, _: &str, _: &str) -> Result<HostResponse, String> {
        Err("fetch is unused in this proof".into())
    }
    fn get_inner_html(&self, _: u32) -> Option<String> {
        None
    }
    fn set_inner_html(&mut self, _: u32, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>> {
        let state = self.0.borrow();
        Some(
            state
                .dom
                .get_attribute_ns(NodeIdx(node), None, name)
                .map(str::to_owned),
        )
    }
    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String> {
        self.0
            .borrow_mut()
            .dom
            .set_attribute(NodeIdx(node), name, value)
            .map_err(|e| e.to_string())
    }
    fn remove_attribute(&mut self, node: u32, name: &str) -> Result<(), String> {
        self.0
            .borrow_mut()
            .dom
            .remove_attribute(NodeIdx(node), name)
            .map_err(|e| e.to_string())
    }
    fn create_element(&mut self, name: &str) -> Result<u32, String> {
        Ok(self.0.borrow_mut().dom.create_element(NodeIdx(0), name).0)
    }
    fn create_text_node(&mut self, _: &str) -> Result<u32, String> {
        Err("unused".into())
    }
    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String> {
        self.0
            .borrow_mut()
            .dom
            .pre_insert(NodeIdx(child), NodeIdx(parent), None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn append_text(&mut self, _: u32, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn remove_node(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn get_style(&self, _: u32, _: &str) -> Option<String> {
        None
    }
    fn set_style(&mut self, _: u32, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn pending_native_microtasks(&self) -> Vec<NativeMicrotaskRequest> {
        let state = self.0.borrow();
        if state.dom.mutation_microtask_pending() && !state.forwarded {
            vec![NativeMicrotaskRequest {
                global: GLOBAL.0,
                kind: NativeMicrotaskKind::MutationObserver,
            }]
        } else {
            Vec::new()
        }
    }
    fn acknowledge_native_microtask(&mut self, _: NativeMicrotaskRequest) {
        let mut state = self.0.borrow_mut();
        assert!(state.dom.mutation_microtask_pending());
        assert!(!state.forwarded);
        state.forwarded = true;
    }
}

enum Payload {
    Job(SchedulingToken),
    Callback(SchedulingToken),
    Mutation,
}
struct Adapter {
    loop_: EventLoop,
    js: Interpreter,
    native: Rc<RefCell<NativeState>>,
    payloads: BTreeMap<u64, Payload>,
    timers: BTreeMap<u32, TimerId>,
    frames: BTreeMap<u32, AnimationFrameId>,
    next_callback: u64,
    trace: Vec<String>,
    errors: Vec<String>,
}
impl Adapter {
    fn new(js_capacity: usize) -> Self {
        let native = Rc::new(RefCell::new(NativeState::new()));
        let mut js = Interpreter::new_with_external_scheduler(js_capacity).unwrap();
        let host: Rc<RefCell<dyn DomHost>> = Rc::new(RefCell::new(NativeHost(native.clone())));
        js.set_dom_host(host);
        let mut loop_ = EventLoop::new();
        loop_.register_global(GLOBAL).unwrap();
        Self {
            loop_,
            js,
            native,
            payloads: BTreeMap::new(),
            timers: BTreeMap::new(),
            frames: BTreeMap::new(),
            next_callback: 1,
            trace: Vec::new(),
            errors: Vec::new(),
        }
    }
    fn own(&mut self, payload: Payload) -> CallbackId {
        let id = self.next_callback;
        self.next_callback += 1;
        self.payloads.insert(id, payload);
        CallbackId(id)
    }
    fn drain_outbox(&mut self) {
        for request in self.js.take_scheduling_requests() {
            match request {
                SchedulingRequest::Timer {
                    registration,
                    callback_token,
                    delay_ms,
                    repeat,
                } => {
                    let callback = self.own(Payload::Callback(callback_token));
                    let id = self
                        .loop_
                        .set_timer(GLOBAL, callback, delay_ms, repeat)
                        .unwrap();
                    self.timers.insert(registration, id);
                }
                SchedulingRequest::CancelTimer { registration } => {
                    if let Some(id) = self.timers.remove(&registration) {
                        self.loop_.clear_timer(id);
                    }
                }
                SchedulingRequest::AnimationFrame {
                    registration,
                    callback_token,
                } => {
                    let callback = self.own(Payload::Callback(callback_token));
                    let id = self
                        .loop_
                        .request_animation_frame(GLOBAL, callback)
                        .unwrap();
                    self.frames.insert(registration, id);
                }
                SchedulingRequest::CancelAnimationFrame { registration } => {
                    if let Some(id) = self.frames.remove(&registration) {
                        self.loop_.cancel_animation_frame(GLOBAL, id).unwrap();
                    }
                }
                SchedulingRequest::Microtask { job_token } => {
                    let callback = self.own(Payload::Job(job_token));
                    self.loop_.queue_microtask(GLOBAL, callback).unwrap();
                }
                SchedulingRequest::NativeMicrotask { global, kind } => {
                    assert_eq!(global, GLOBAL.0);
                    assert_eq!(kind, NativeMicrotaskKind::MutationObserver);
                    let callback = self.own(Payload::Mutation);
                    self.loop_.queue_microtask(GLOBAL, callback).unwrap();
                }
            }
        }
    }
    fn run(&mut self, script: &str) {
        if let Err(error) = self.js.run_script(script) {
            self.errors.push(error.describe());
        }
        self.drain_outbox();
        self.loop_.advance_to(Duration::ZERO).unwrap();
    }
    fn pump(&mut self) {
        for _ in 0..200 {
            let Some(step) = self.loop_.step().unwrap() else {
                return;
            };
            match step.action {
                Action::Task { callback, .. } | Action::Microtask { callback, .. } => {
                    self.invoke(callback, None)
                }
                Action::AnimationFrame {
                    callback,
                    timestamp,
                    ..
                } => self.invoke(callback, Some(timestamp)),
                Action::CheckpointCleanup | Action::Render { .. } => (),
            }
            self.drain_outbox();
            self.loop_.complete(step.id).unwrap();
        }
        panic!("adapter failed to become idle");
    }
    fn invoke(&mut self, callback: CallbackId, timestamp: Option<Duration>) {
        let before = self.js.output.borrow().len();
        let result = match self.payloads.remove(&callback.0).unwrap() {
            Payload::Job(token) => self.js.invoke_one_job(token),
            Payload::Callback(token) => self.js.invoke_one_callback(
                token,
                JsValue::Object(self.js.global_obj.clone()),
                timestamp,
            ),
            Payload::Mutation => {
                let mut state = self.native.borrow_mut();
                let mut checkpoint = state.dom.begin_mutation_checkpoint();
                state.forwarded = false;
                while state.dom.next_mutation_delivery(&mut checkpoint).is_some() {
                    self.trace.push("M".into());
                }
                Ok(())
            }
        };
        self.trace
            .extend(self.js.output.borrow()[before..].iter().cloned());
        if let Err(error) = result {
            self.errors.push(error.describe());
        }
    }
}

#[test]
fn real_loop_orders_script_p_q_chained_r_before_timer_b_and_cancels_frame() {
    let mut a = Adapter::new(16);
    a.run("setTimeout(()=>{console.log('A'); Promise.resolve().then(()=>{console.log('P'); queueMicrotask(()=>console.log('R'))}); queueMicrotask(()=>console.log('Q'))},0); setTimeout(()=>console.log('B'),0)");
    a.pump();
    assert_eq!(a.trace, ["A", "P", "Q", "R", "B"]);

    let mut frame = Adapter::new(16);
    frame.run("let second; requestAnimationFrame(()=>{console.log('F1'); Promise.resolve().then(()=>cancelAnimationFrame(second))}); second=requestAnimationFrame(()=>console.log('F2'))");
    frame
        .loop_
        .rendering_opportunity(GLOBAL, Duration::from_millis(16))
        .unwrap();
    frame.pump();
    assert_eq!(frame.trace, ["F1"]);
}

#[test]
fn real_loop_preserves_native_interleaving_with_reserved_capacity() {
    let script = "Promise.resolve().then(()=>console.log('P')); document.querySelector('x').setAttribute('watched','1'); try {queueMicrotask(()=>console.log('Q'))} catch(e) {console.log(e.name)}";
    let mut one = Adapter::new(1);
    one.run(script);
    assert!(one.native.borrow().dom.mutation_microtask_pending());
    assert!(one.native.borrow().forwarded);
    one.pump();
    assert_eq!(one.trace, ["P", "M"]);
    assert!(one.js.output.borrow().iter().any(|s| s == "TypeError"));
    assert!(!one.native.borrow().dom.mutation_microtask_pending());

    let mut two = Adapter::new(2);
    two.run(script);
    two.pump();
    assert_eq!(two.trace, ["P", "M", "Q"]);
    assert!(!two.js.output.borrow().iter().any(|s| s == "TypeError"));
}

#[test]
fn native_forwarding_waits_for_admission_and_rearms_after_delivery() {
    let mut adapter = Adapter::new(8);
    adapter.run("let x=document.querySelector('x'); x.setAttribute('watched','1'); x.setAttribute('watched','2')");
    assert!(adapter.native.borrow().dom.mutation_microtask_pending());
    assert!(adapter.native.borrow().forwarded);
    assert_eq!(
        adapter
            .payloads
            .values()
            .filter(|p| matches!(p, Payload::Mutation))
            .count(),
        1
    );
    adapter.pump();
    assert_eq!(adapter.trace, ["M"]);
    assert!(!adapter.native.borrow().dom.mutation_microtask_pending());
    assert!(!adapter.native.borrow().forwarded);

    // A new mutation after the checkpoint must create a new native slot.
    adapter.run("document.querySelector('x').setAttribute('watched','3')");
    assert!(adapter.native.borrow().forwarded);
    adapter.pump();
    assert_eq!(adapter.trace, ["M", "M"]);

    // Force a duplicate native request before a host mutation. Admission
    // rejects it, so the host must leave its forwarded bit clear.
    adapter
        .js
        .enqueue_native_microtask(GLOBAL.0, NativeMicrotaskKind::MutationObserver)
        .unwrap();
    let error = adapter
        .js
        .run_script("document.querySelector('x').setAttribute('watched','4')")
        .unwrap_err();
    assert!(error.describe().starts_with("TypeError:"));
    assert!(adapter.native.borrow().dom.mutation_microtask_pending());
    assert!(!adapter.native.borrow().forwarded);
}

#[test]
fn thrown_callback_still_completes_its_checkpoint_and_runs_later_timer() {
    let mut adapter = Adapter::new(8);
    adapter.run("setTimeout(()=>{console.log('A'); Promise.resolve().then(()=>console.log('P')); throw new Error('sentinel')},0); setTimeout(()=>console.log('B'),0)");
    adapter.pump();
    assert_eq!(adapter.trace, ["A", "P", "B"]);
    assert_eq!(adapter.errors.len(), 1);
    assert!(adapter.errors[0].contains("sentinel"));
}

#[test]
fn thrown_script_still_forwards_earlier_scheduling_requests() {
    let mut adapter = Adapter::new(8);
    adapter.run("Promise.resolve().then(()=>console.log('P')); throw new Error('script sentinel')");
    adapter.pump();
    assert_eq!(adapter.trace, ["P"]);
    assert_eq!(adapter.errors.len(), 1);
    assert!(adapter.errors[0].contains("script sentinel"));
}
