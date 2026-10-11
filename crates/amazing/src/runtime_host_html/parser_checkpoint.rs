//! Parser microtask checkpoints borrow the actual installed scheduler registry.
use super::*;
use amazing_dom::event_loop::{Action, TaskSource};
use amazing_js::DomHost;
impl DriverFifo {
    pub(crate) fn new(loop_: EventLoop) -> Self {
        Self { loop_, pending: VecDeque::new(), payloads: BTreeMap::new(), timers: BTreeMap::new(),
            frames: BTreeMap::new(), next_callback: 1, trace: Vec::new(), trace_entry_depth: 0 }
    }
    pub(crate) fn own(&mut self, payload: DriverPayload) -> Result<DriverCallbackId, Throw> {
        let id = self.next_callback;
        self.next_callback = id.checked_add(1)
            .ok_or_else(|| Throw::range_error("driver callback IDs exhausted"))?;
        self.payloads.insert(id, payload);
        Ok(DriverCallbackId(id))
    }

    pub(crate) fn admit(&mut self, request: SchedulingRequest, bindings: &RuntimeBindings) -> Result<(), Throw> {
        match request {
            SchedulingRequest::Timer { registration, callback_token, delay_ms, repeat } => {
                let callback = self.own(DriverPayload::Callback(callback_token))?;
                let core = match self.loop_.set_timer(crate::js_scheduler::GLOBAL, callback, delay_ms, repeat) {
                    Ok(id) => id,
                    Err(error) => { self.payloads.remove(&callback.0); return Err(crate::js_scheduler::loop_error(error)); }
                };
                self.timers.insert(registration, DriverTimerRegistration { core, callback, repeat });
            }
            SchedulingRequest::CancelTimer { registration } => {
                if let Some(entry) = self.timers.remove(&registration) {
                    self.loop_.clear_timer(entry.core);
                    self.payloads.remove(&entry.callback.0);
                }
            }
            SchedulingRequest::AnimationFrame { registration, callback_token } => {
                let callback = self.own(DriverPayload::Callback(callback_token))?;
                let core = match self.loop_.request_animation_frame(crate::js_scheduler::GLOBAL, callback) {
                    Ok(id) => id,
                    Err(error) => { self.payloads.remove(&callback.0); return Err(crate::js_scheduler::loop_error(error)); }
                };
                self.frames.insert(registration, (core, callback));
            }
            SchedulingRequest::CancelAnimationFrame { registration } => {
                if let Some((core, callback)) = self.frames.get(&registration).copied() {
                    self.loop_.cancel_animation_frame(crate::js_scheduler::GLOBAL, core).map_err(crate::js_scheduler::loop_error)?;
                    self.frames.remove(&registration);
                    self.payloads.remove(&callback.0);
                }
            }
            SchedulingRequest::Microtask { job_token } => {
                let callback = self.own(DriverPayload::Job(job_token))?;
                if let Err(error) = self.loop_.queue_microtask(crate::js_scheduler::GLOBAL, callback) {
                    self.payloads.remove(&callback.0);
                    return Err(crate::js_scheduler::loop_error(error));
                }
            }
            SchedulingRequest::NativeTask { token } => {
                if bindings.native_selection_task(token)?.is_none() {
                    bindings.discard_native_selection_task(token)?;
                } else {
                    let callback = self.own(DriverPayload::NativeSelection(token))?;
                    if let Err(error) = self.loop_.queue_task(TaskSource::UserInteraction, Some(crate::js_scheduler::GLOBAL), callback) {
                        self.payloads.remove(&callback.0);
                        return Err(crate::js_scheduler::loop_error(error));
                    }
                    // Existing root scheduling-global policy remains explicit;
                    // target Document generation and creation realm are independent.
                    bindings.native_selection_admitted(token);
                }
            }
            SchedulingRequest::NativeMicrotask { global, kind } => {
                if global != crate::js_scheduler::GLOBAL.0 || kind != NativeMicrotaskKind::MutationObserver {
                    return Err(Throw::type_error("unknown native microtask request"));
                }
                let callback = self.own(DriverPayload::Mutation)?;
                if let Err(error) = self.loop_.queue_microtask(crate::js_scheduler::GLOBAL, callback) {
                    self.payloads.remove(&callback.0);
                    return Err(crate::js_scheduler::loop_error(error));
                }
            }
        }
        Ok(())
    }

}
impl RuntimeBindings {
    pub(crate) fn attach_parser_checkpoint_fifo(&self, fifo: &Rc<RefCell<DriverFifo>>) -> Result<(), Throw> {
        let mut state = self.state.borrow_mut();
        if state.frames.parser_checkpoint_fifo.as_ref().and_then(std::rc::Weak::upgrade).is_some() {
            return Err(Throw::type_error("Driver checkpoint FIFO already installed"));
        }
        state.frames.parser_checkpoint_fifo = Some(Rc::downgrade(fifo));
        Ok(())
    }
    fn admit_parser_checkpoint_outbox(&self, js: &mut Interpreter, fifo: &Rc<RefCell<DriverFifo>>) -> Result<(), Throw> {
        // The same actual native mutation producer and ordered Interpreter
        // outbox used by DriverScheduler::enter. No queue is fabricated.
        let host = self.state.borrow().host.clone();
        let native = host.borrow().pending_native_microtasks();
        for request in native {
            js.enqueue_native_microtask(request.global, request.kind)?;
            host.borrow_mut().acknowledge_native_microtask(request);
        }
        self.emit_native_selection_tasks(js)?;
        fifo.borrow_mut().pending.extend(js.take_scheduling_requests());
        loop {
            let request = { fifo.borrow().pending.front().copied() };
            let Some(request) = request else { break; };
            fifo.borrow_mut().admit(request, self)?;
            fifo.borrow_mut().pending.pop_front();
        }
        Ok(())
    }
    pub(crate) fn parser_microtask_checkpoint(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let fifo = self.state.borrow().frames.parser_checkpoint_fifo.as_ref()
            .and_then(std::rc::Weak::upgrade).ok_or_else(|| Throw::type_error("Original driver checkpoint FIFO retired"))?;
        self.admit_parser_checkpoint_outbox(js, &fifo)?;
        let enclosing = fifo.borrow().loop_.pending_step();
        let requested = {
            let mut owner = fifo.borrow_mut();
            match enclosing {
                Some(step) => owner.loop_.checkpoint_after_callback(step),
                None => owner.loop_.request_microtask_checkpoint(),
            }.map_err(crate::js_scheduler::loop_error)?
        };
        if !requested { return Ok(()); }
        let mut first_error = None;
        loop {
            if !fifo.borrow().loop_.performing_microtask_checkpoint() { break; }
            let step = fifo.borrow_mut().loop_.step().map_err(crate::js_scheduler::loop_error)?
                .ok_or_else(|| Throw::type_error("Actual checkpoint lost its continuation"))?;
            let result = (|| -> Result<(), Throw> { match step.action {
                Action::Microtask { callback, .. } => {
                    let payload = fifo.borrow_mut().payloads.remove(&callback.0)
                        .ok_or_else(|| Throw::type_error("Original microtask payload missing"))?;
                    // Popped once; no scheduler or Document borrow is retained.
                    let mutation = matches!(&payload, DriverPayload::Mutation);
                    let before = js.output.borrow().len();
                    let result = match payload {
                        DriverPayload::Job(token) => {
                            // The actual queueMicrotask object is inspected
                            // BEFORE the Interpreter consumes its one-shot job.
                            // Promise/module jobs retain their rejection paths.
                            let callback = js.binding_scheduled_microtask_callback(token)?;
                            let context = match callback {
                                Some(callback) => {
                                    let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
                                    adapter.html_capture_microtask_exception(&callback).map(Some)
                                }
                                None => Ok(None),
                            };
                            // Consume the real owned job even if this platform
                            // lacks its reporting global; never leak its token
                            // or substitute a caller/global for that failure.
                            let invoked = js.invoke_one_job(token);
                            match (invoked, context) {
                                (Err(error), Ok(Some(context))) => {
                                    let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
                                    adapter.html_report_constructor_exception(context, error)
                                }
                                (_, Err(error)) => Err(error),
                                (result, _) => result,
                            }
                        },
                        DriverPayload::Mutation => self.deliver_mutations(js),
                        _ => Err(Throw::type_error("Non-microtask payload in checkpoint")),
                    };
                    let reactions = self.deliver_reactions(js);
                    if fifo.borrow().trace_entry_depth == 0 {
                        fifo.borrow_mut().trace.extend(js.output.borrow()[before..].iter().cloned());
                    }
                    if mutation {
                        self.state.borrow().host.borrow_mut().native_forwarded = false;
                        fifo.borrow_mut().trace.push("M".into());
                    }
                    result.and(reactions)
                }
                Action::CheckpointCleanup => { self.storage_deactivate_transactions(); Ok(()) },
                _ => Err(Throw::type_error("Checkpoint yielded an unrelated action")),
            } })();
            if let Err(error) = result { first_error.get_or_insert(error); }
            // Complete the actual step even on author error/admission failure.
            let admitted = self.admit_parser_checkpoint_outbox(js, &fifo);
            fifo.borrow_mut().loop_.complete(step.id).map_err(crate::js_scheduler::loop_error)?;
            if let Err(error) = admitted { first_error.get_or_insert(error); }
        }
        if fifo.borrow().loop_.pending_step() != enclosing {
            return Err(Throw::type_error("Checkpoint did not restore original callback"));
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler,JS_CAPACITY,GLOBAL};
    use amazing_dom::event_loop::TaskSource;
    #[test]
    fn checkpoint_restores_actual_inflight_callback_and_does_not_run_later_tasks() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(),native_forwarded:false }));
        let mut driver = DriverScheduler::new(host,JS_CAPACITY).unwrap();
        driver.run_script("var checkpointOrder=[];queueMicrotask(()=>checkpointOrder.push('job'));setTimeout(()=>checkpointOrder.push('timer'),0)").unwrap();
        // Use the already installed FIFO; no substitute event loop is created.
        let bindings = driver.bindings_for_parser_checkpoint();
        let fifo = bindings.state.borrow().frames.parser_checkpoint_fifo.as_ref().unwrap().upgrade().unwrap();
        bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
        let callback = fifo.borrow_mut().own(DriverPayload::FormSelection(0)).unwrap();
        fifo.borrow_mut().loop_.queue_task(TaskSource::DomManipulation,Some(GLOBAL),callback).unwrap();
        // The earlier zero-delay timer is genuinely first in this SAME queue.
        let step = fifo.borrow_mut().loop_.step().unwrap().unwrap();
        driver.js.run_script_utf16("queueMicrotask(()=>checkpointOrder.push('nested'))".encode_utf16().collect::<Vec<_>>().as_slice()).unwrap();
        bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
        assert_eq!(fifo.borrow().loop_.pending_step(),Some(step.id));
        assert_eq!(driver.js.run_script_utf16("checkpointOrder.join('|')".encode_utf16().collect::<Vec<_>>().as_slice()).unwrap(),JsValue::str("job|nested"));
        fifo.borrow_mut().loop_.complete(step.id).unwrap();
    }
    #[test]
    fn recursive_checkpoint_keeps_guard_and_owned_fifo_and_retirement_is_explicit() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom:Dom::new(),native_forwarded:false }));
        let mut driver = DriverScheduler::new(host,JS_CAPACITY).unwrap();
        let bindings = driver.bindings_for_parser_checkpoint();
        let fifo = bindings.state.borrow().frames.parser_checkpoint_fifo.as_ref().unwrap().upgrade().unwrap();
        assert!(fifo.borrow_mut().loop_.request_microtask_checkpoint().unwrap());
        driver.run_script("var guarded=0;queueMicrotask(()=>guarded++)").unwrap();
        bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
        assert!(fifo.borrow().loop_.performing_microtask_checkpoint());
        let step=fifo.borrow_mut().loop_.step().unwrap().unwrap();
        let Action::Microtask{callback,..}=step.action else{panic!("actual admitted microtask")};
        let DriverPayload::Job(token)=fifo.borrow_mut().payloads.remove(&callback.0).unwrap() else{panic!("actual job")};
        driver.js.invoke_one_job(token).unwrap();fifo.borrow_mut().loop_.complete(step.id).unwrap();
        let cleanup=fifo.borrow_mut().loop_.step().unwrap().unwrap();fifo.borrow_mut().loop_.complete(cleanup.id).unwrap();
        drop(fifo);drop(driver);
        let mut js=Interpreter::new();assert!(bindings.parser_microtask_checkpoint(&mut js).is_err());
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler,JS_CAPACITY};
    fn driver()->DriverScheduler {
        DriverScheduler::new(Rc::new(RefCell::new(BrowserDomHost{dom:Dom::new(),native_forwarded:false})),JS_CAPACITY).unwrap()
    }
    #[test]
    fn checkpoint_author_error_completes_step_and_does_not_starve_later_owned_jobs() {
        let mut driver=driver();let bindings=driver.bindings_for_parser_checkpoint();
        driver.run_script("var checkpointAfterError=0, jobMarker={}, jobErrors=[];window.addEventListener('error',e=>{jobErrors.push([e.error,e.isTrusted]);e.preventDefault();});queueMicrotask(()=>{throw jobMarker});queueMicrotask(()=>checkpointAfterError++)").unwrap();
        bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
        assert_eq!(driver.run_script("jobErrors.length===1 && jobErrors[0][0]===jobMarker && jobErrors[0][1]===true").unwrap(),JsValue::Bool(true));
        let fifo=bindings.state.borrow().frames.parser_checkpoint_fifo.as_ref().unwrap().upgrade().unwrap();
        assert!(!fifo.borrow().loop_.performing_microtask_checkpoint());assert_eq!(fifo.borrow().loop_.pending_step(),None);
        assert_eq!(driver.run_script("checkpointAfterError").unwrap(),JsValue::num(1.0));
        bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
    }
    #[test]
    fn downstream_admission_failure_retains_actual_job_token_and_fifo_front_for_retry() {
        let mut driver=driver();let bindings=driver.bindings_for_parser_checkpoint();
        let fifo=bindings.state.borrow().frames.parser_checkpoint_fifo.as_ref().unwrap().upgrade().unwrap();
        let before=fifo.borrow().next_callback;fifo.borrow_mut().next_callback=u64::MAX;
        driver.js.run_script_utf16("var retriedJobs=0;queueMicrotask(()=>retriedJobs++)".encode_utf16().collect::<Vec<_>>().as_slice()).unwrap();
        assert!(bindings.parser_microtask_checkpoint(&mut driver.js).is_err());
        assert_eq!(fifo.borrow().pending.len(),1);assert!(fifo.borrow().payloads.is_empty());
        let retained=fifo.borrow().pending.front().copied().unwrap();fifo.borrow_mut().next_callback=before;
        bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
        assert!(fifo.borrow().pending.is_empty());assert!(fifo.borrow().payloads.is_empty());
        assert!(matches!(retained,SchedulingRequest::Microtask{..}));
        assert_eq!(driver.run_script("retriedJobs").unwrap(),JsValue::num(1.0));
    }
}
