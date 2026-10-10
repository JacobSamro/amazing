//! Original-object receiving custody for XHR; no incumbent/global URL fallback.
use super::*;
use super::super::html::receiving_context::{ReceivingDocumentIdentity,receiving_document_client_from_state};
use amazing_fetch::{ReceivingDocumentClient,xhr::XhrUrlContext};

pub(super) struct XhrPrepared {
    pub record:Rc<RefCell<FetchObject>>,
    pub generation:u64,
    pub started:Instant,
    pub receiving:ReceivingDocumentClient<ReceivingDocumentIdentity>,
}
impl RuntimeBindingHost<'_> {
    pub(super) fn xhr_receiving_client(&mut self,receiver:&JsValue)
        ->Result<ReceivingDocumentClient<ReceivingDocumentIdentity>,Throw> {
        let realm=self.js.binding_object_creation_realm(receiver)?;
        let capture=(|| {
            let state=self.state.borrow();let host=state.host.borrow();
            let context=state.frames.document_context_for_realm(&host.dom,realm)
                .ok_or(DomError::InvalidStateError)?;
            let active=state.frames.history_context(&host.dom,context.document)
                .filter(|context|context.realm==realm).ok_or(DomError::InvalidStateError)?;
            let identity=ReceivingDocumentIdentity{realm,document:context.document,generation:context.document_generation,
                window:host.dom.document_window_event_target(active.document.document).ok_or(DomError::InvalidStateError)?};
            receiving_document_client_from_state(&state,&identity)
        })();
        capture.map_err(|error|throw_dom_error(self,error))
    }
    pub(super) fn xhr_prepare_received(&mut self,record:Rc<RefCell<FetchObject>>,receiver:JsValue)->Result<(),Throw> {
        let receiving=self.xhr_receiving_client(&receiver)?;
        let preparation={
            let mut object=record.borrow_mut();let FetchObject::Xhr(xhr)=&mut *object else {
                return Err(Throw::type_error("Expected XMLHttpRequest"));};
            xhr.prepare_service_received(&receiving).map(|_|xhr.generation())
        };
        let generation=preparation.map_err(|error|self.fetch_xhr_error(error))?;
        let mut state=self.state.borrow_mut();
        state.net.xhr_prepared.retain(|prepared|!Rc::ptr_eq(&prepared.record,&record)||prepared.generation!=generation);
        state.net.xhr_prepared.push(XhrPrepared{record,generation,started:Instant::now(),receiving});
        Ok(())
    }
    pub(super) fn xhr_discard_received(&mut self,record:&Rc<RefCell<FetchObject>>,generation:u64) {
        let mut state=self.state.borrow_mut();
        state.net.xhr_prepared.retain(|prepared|!Rc::ptr_eq(&prepared.record,record)||prepared.generation!=generation);
        let mut tasks=std::mem::take(&mut state.net.xhr_pending);
        for mut task in tasks.drain(..) {
            if Rc::ptr_eq(&task.record,record) && task.generation==generation {
                match &mut task.transport {
                    XhrTransport::Stream(pending)=>{pending.cancel(&mut state.net.fetch_network);}
                    XhrTransport::Dispatch(pending)=>{pending.cancel(&mut state.net.fetch_network.dispatcher);}
                    XhrTransport::Data|XhrTransport::Terminal(_)=>{}
                }
            } else {state.net.xhr_pending.push(task);}
        }
    }
    pub(super) fn xhr_stream_event(&mut self,event:&ServiceEvent)->Result<bool,Throw> {
        let id=match event {ServiceEvent::RequestBodyProgress{id,..}|ServiceEvent::ResponseHead{id,..}
            |ServiceEvent::BodyChunk{id,..}|ServiceEvent::StreamFinished{id}|ServiceEvent::StreamFailed{id,..}=>*id,
            _=>return Ok(false)};
        let index=self.state.borrow().net.xhr_pending.iter().position(|task|
            matches!(&task.transport,XhrTransport::Stream(pending) if pending.id()==id));
        let Some(index)=index else {return Ok(false);};
        let mut task=self.state.borrow_mut().net.xhr_pending.remove(index);
        if !self.xhr_receiving_alive(&task) {
            if let XhrTransport::Stream(pending)=&mut task.transport {
                pending.cancel(&mut self.state.borrow_mut().net.fetch_network);
            }
            if let FetchObject::Xhr(xhr)=&mut *task.record.borrow_mut() {xhr.abort();}
            return Ok(true);
        }
        let events={
            let mut state=self.state.borrow_mut();let mut object=task.record.borrow_mut();
            let FetchObject::Xhr(xhr)=&mut *object else {return Ok(true);};
            let XhrTransport::Stream(pending)=&mut task.transport else {unreachable!()};
            xhr.on_service_event(pending,&mut state.net.fetch_network,event,task.started.elapsed().as_millis() as u64)
        }.map_err(|error|self.fetch_xhr_error(error));
        let result=events.and_then(|events|self.xhr_events(&mut task,events));
        let keep=matches!(&*task.record.borrow(),FetchObject::Xhr(xhr)
            if xhr.generation()==task.generation && (xhr.send_invoked()||!task.queued_events.is_empty()));
        if keep && self.xhr_receiving_alive(&task) {self.state.borrow_mut().net.xhr_pending.push(task);}
        else if let XhrTransport::Stream(pending)=&mut task.transport {
            if keep {if let FetchObject::Xhr(xhr)=&mut *task.record.borrow_mut(){xhr.abort();}}
            pending.cancel(&mut self.state.borrow_mut().net.fetch_network);
        }
        result?;Ok(true)
    }
    pub(super) fn xhr_receiving_alive(&self,task:&XhrPending)->bool {
        receiving_document_client_from_state(&self.state.borrow(),&task.receiving.identity).is_ok()
    }
    pub(super) fn xhr_url_context_received(&mut self,receiver:&JsValue)->Result<XhrUrlContext,Throw> {
        let receiving=self.xhr_receiving_client(receiver)?;
        Ok(XhrUrlContext{base_url:receiving.base_url,origin:receiving.origin})
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn xhr_dispatch_event_received(&mut self,receiver:&JsValue,event:XhrEvent)->Result<(),Throw> {
        let record=self.fetch_record(receiver,FetchInterface::XMLHttpRequest)?;
        if !matches!(&*record.borrow(),FetchObject::Xhr(xhr) if xhr.is_current_event(&event)) {return Ok(());}
        let realm=self.js.binding_object_creation_realm(receiver)?;
        let identity={let state=self.state.borrow();let host=state.host.borrow();
            let Some(context)=state.frames.document_context_for_realm(&host.dom,realm) else {return Ok(());};
            let Some(active)=state.frames.history_context(&host.dom,context.document).filter(|active|active.realm==realm) else {return Ok(());};
            ReceivingDocumentIdentity{realm,document:context.document,generation:context.document_generation,
                window:host.dom.document_window_event_target(active.document.document).ok_or_else(||Throw::type_error("Missing original XHR Window"))?}
        };
        let source_url=receiving_document_client_from_state(&self.state.borrow(),&identity)
            .map(|client|client.creation_url.serialize()).map_err(|_|Throw::type_error("Missing original XHR event receiving admission"))?;
        let state=self.state.clone();let receiver=receiver.clone();
        self.js.binding_with_realm(realm,|js|RuntimeBindingHost{js,state}
            .xhr_dispatch_event_inner(&receiver,event,identity,&source_url))
    }
    fn xhr_report_listener_exception(&mut self,identity:ReceivingDocumentIdentity,url:&str,error:&Throw)->Result<(),Throw> {
        // A callback may have retired its original Window. The original
        // exception is then an inactive-task disposition, never relabelled to
        // the current/fresh global. Otherwise use the actual Window error path.
        if receiving_document_client_from_state(&self.state.borrow(),&identity).is_err() {return Ok(());}
        let bindings=RuntimeBindings{state:self.state.clone()};
        self.js.binding_with_realm(identity.realm,|js|bindings.report_module_exception(js,error,url))
    }
    fn xhr_dispatch_event_inner(&mut self, receiver: &JsValue, event: XhrEvent, identity:ReceivingDocumentIdentity, source_url:&str) -> Result<(), Throw> {
        let key = self.platform_object(receiver, FetchInterface::XMLHttpRequest.brand())
            .ok_or_else(|| Throw::type_error("Stale XHR receiver"))?;
        let previous = self.state.borrow_mut().net.xhr_ready_state_override.replace((key.slot, event.ready_state));
        let _override_guard = XhrStateOverrideGuard(self.state.clone(), previous);
        let name = match event.kind {
            XhrEventKind::ReadyStateChange => "readystatechange",
            XhrEventKind::LoadStart => "loadstart", XhrEventKind::Progress => "progress",
            XhrEventKind::Load => "load", XhrEventKind::Error => "error",
            XhrEventKind::Abort => "abort", XhrEventKind::Timeout => "timeout",
            XhrEventKind::LoadEnd => "loadend",
        };
        let target = if event.target == XhrEventTarget::Upload {
            self.js.binding_get_property(receiver, "upload")?
        } else { receiver.clone() };
        let value = self.fetch_allocate(FetchInterface::ProgressEvent,
            FetchObject::ProgressEvent { kind: name.into(), loaded: event.loaded,
                total: event.total, length_computable: event.length_computable })?;
        self.js.binding_define_data_property(&value, "target", target.clone(), BindingPropertyFlags::METHOD)?;
        self.js.binding_define_data_property(&value, "currentTarget", target.clone(), BindingPropertyFlags::METHOD)?;
        let callback = self.js.binding_get_property(&target, &format!("on{name}"))?;
        if self.js.binding_is_callable(&callback) {
            if let Err(error)=binding_call(self.js,&callback,&target,&[value.clone()]) {
                self.xhr_report_listener_exception(identity,source_url,&error)?;
            }
        }
        let brand = if event.target == XhrEventTarget::Upload {
            FetchInterface::XMLHttpRequestUpload.brand()
        } else { FetchInterface::XMLHttpRequest.brand() };
        let key = self.platform_object(&target, brand).ok_or_else(|| Throw::type_error("Stale XHR target"))?;
        let listeners = self.state.borrow().net.fetch_listeners.iter()
            .filter(|(slot, family, kind, _)| *slot == key.slot && *family == brand.0 && kind == name)
            .map(|(_, _, _, callback)| callback.clone()).collect::<Vec<_>>();
        for callback in listeners {
            if let Err(error)=binding_call(self.js,&callback,&target,&[value.clone()]) {
                self.xhr_report_listener_exception(identity,source_url,&error)?;
            }
        }
        Ok(())
    }
}
