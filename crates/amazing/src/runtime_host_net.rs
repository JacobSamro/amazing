//! Fetch and XHR runtime adapter owned by the net lane.
use super::*;
use amazing_bindings::fetch_interfaces::{FetchHeaders, FetchHost, FetchInterface, FetchObject, FetchOperation, HeaderIterationKind};
use amazing_fetch::{api::{ApiError, ApiRequest, ApiResponse, PendingFetchApi}, service::{ClientContext, NetworkService, ServiceEvent, ServiceId}, xhr::{Xhr, XhrError, XhrEvent, XhrEventKind, XhrEventTarget, XhrState}, HttpUrl};
use amazing_net::dispatch::{DispatchLimits, NetEvent};
use std::time::{Duration, Instant};

enum XhrTransport {
    Data,
    Dispatch(PendingFetchApi),
}
struct XhrPending {
    record: Rc<RefCell<FetchObject>>,
    receiver: JsValue,
    generation: u64,
    started: Instant,
    transport: XhrTransport,
}

pub(super) struct NetRuntime {
    fetch_objects: Vec<Rc<RefCell<FetchObject>>>,
    fetch_interfaces: Vec<(FetchInterface, JsValue, JsValue)>,
    fetch_network: NetworkService,
    legacy_fetch: Option<JsValue>,
    fetch_listeners: Vec<(u64, u64, String, JsValue)>,
    fetch_header_iterator_prototype: Option<JsValue>,
    pub(super) fetch_resolver_capture: Option<(JsValue, JsValue)>,
    fetch_pending: Vec<(ServiceId, JsValue, JsValue, AbortSignalId)>,
    fetch_pump_scheduled: bool,
    xhr_pending: Vec<XhrPending>,
    xhr_ready_state_override: Option<(u64, XhrState)>,
    xhr_document_nodes: HashMap<u64, (u64, NodeIdx)>,
}

impl NetRuntime {
    pub(super) fn new() -> Self {
        Self {
            fetch_objects: Vec::new(),
            fetch_interfaces: Vec::new(),
            fetch_network: NetworkService::new(DispatchLimits::default()),
            legacy_fetch: None,
            fetch_listeners: Vec::new(),
            fetch_header_iterator_prototype: None,
            fetch_resolver_capture: None,
            fetch_pending: Vec::new(),
            fetch_pump_scheduled: false,
            xhr_pending: Vec::new(),
            xhr_ready_state_override: None,
            xhr_document_nodes: HashMap::new(),
        }
    }
}

struct XhrStateOverrideGuard(Rc<RefCell<BindingState>>, Option<(u64, XhrState)>);
impl Drop for XhrStateOverrideGuard {
    fn drop(&mut self) { self.0.borrow_mut().net.xhr_ready_state_override = self.1; }
}

impl RuntimeBindingHost<'_> {
    pub(crate) fn fetch_request_record(&self, value: &JsValue) -> Result<ApiRequest, Throw> {
        let record = self.fetch_record(value, FetchInterface::Request)?;
        let record = record.borrow();
        let FetchObject::Request(request, _) = &*record else {
            return Err(Throw::type_error("Expected Request"));
        };
        request.clone_request().map_err(|error| Throw::type_error(format!("{error:?}")))
    }
    pub(crate) fn fetch_response_record(&self, value: &JsValue) -> Result<ApiResponse, Throw> {
        let record = self.fetch_record(value, FetchInterface::Response)?;
        let record = record.borrow();
        let FetchObject::Response(response) = &*record else {
            return Err(Throw::type_error("Expected Response"));
        };
        response.clone_response().map_err(|error| Throw::type_error(format!("{error:?}")))
    }
    pub(crate) fn fetch_response_new(&mut self, response: ApiResponse) -> Result<JsValue, Throw> {
        self.fetch_allocate(FetchInterface::Response, FetchObject::Response(response))
    }
    fn schedule_fetch_pump(&mut self) -> Result<(), Throw> {
        if self.state.borrow().net.fetch_pump_scheduled { return Ok(()); }
        self.state.borrow_mut().net.fetch_pump_scheduled = true;
        let result = (|| {
            let callback = self.native("fetch network step", 0, NativeEntry::FetchPump, HostCallPolicy::CallOnly);
            let global = self.js.binding_global_object();
            let timer = self.js.binding_get_property(&global, "setTimeout")?;
            binding_call(self.js, &timer, &global, &[callback, JsValue::num(1.0)])?;
            Ok(())
        })();
        if result.is_err() { self.state.borrow_mut().net.fetch_pump_scheduled = false; }
        result
    }
    pub(super) fn fetch_pump(&mut self) -> Result<JsValue, Throw> {
        self.state.borrow_mut().net.fetch_pump_scheduled = false;
        let aborted = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            state.net.fetch_pending.iter().filter_map(|(id, _, _, signal)|
                host.dom.abort_signal_reason(*signal).ok().flatten().map(|_| *id))
                .collect::<Vec<_>>()
        };
        for id in aborted { self.state.borrow_mut().net.fetch_network.abort(id); }
        let events = self.state.borrow_mut().net.fetch_network.step(8192, Instant::now());
        for event in events {
            let (id, result) = match event {
                ServiceEvent::Finished { id, result } => (id, result),
                ServiceEvent::ExternalNetwork(event) => {
                    self.xhr_network_event(event)?;
                    continue;
                }
                _ => continue,
            };
            let pending = {
                let mut state = self.state.borrow_mut();
                state.net.fetch_pending.iter().position(|(found, _, _, _)| *found == id)
                    .map(|index| state.net.fetch_pending.remove(index))
            };
            let Some((_, resolve, reject, signal)) = pending else { continue; };
            let result = if self.fetch_signal_reason(signal).is_some() {
                Err(ApiError::Aborted)
            } else { result };
            match result {
                Ok(response) => {
                    let value = self.fetch_allocate(FetchInterface::Response, FetchObject::Response(response))?;
                    binding_call(self.js, &resolve, &JsValue::Undefined, &[value])?;
                }
                Err(error) => {
                    let reason = if error == ApiError::Aborted {
                        match self.fetch_signal_reason(signal) {
                            Some(reason) => reason,
                            None => self.fetch_error_value(&error)?,
                        }
                    } else { self.fetch_error_value(&error)? };
                    binding_call(self.js, &reject, &JsValue::Undefined, &[reason])?;
                }
            }
        }
        self.xhr_drive()?;
        if !self.state.borrow().net.fetch_pending.is_empty() || !self.state.borrow().net.xhr_pending.is_empty() {
            self.schedule_fetch_pump()?;
        }
        Ok(JsValue::Undefined)
    }
    fn fetch_error_value(&mut self, error: &ApiError) -> Result<JsValue, Throw> {
        if *error == ApiError::Aborted {
            return invoke(self, Operation::DOMExceptionConstructor, JsValue::Undefined,
                &[JsValue::str("The operation was aborted"), JsValue::str("AbortError")]);
        }
        let kind = self.js.binding_get_property(&self.js.binding_global_object(), "TypeError")?;
        self.js.binding_construct(&kind, vec![JsValue::str(format!("{error:?}"))])
    }
    fn fetch_signal_reason(&self, signal: AbortSignalId) -> Option<JsValue> {
        let root = self.state.borrow().host.borrow().dom.abort_signal_reason(signal).ok().flatten()?;
        self.root_value(root)
    }
    fn xhr_events(&mut self, task: &XhrPending, events: Vec<XhrEvent>) -> Result<(), Throw> {
        for event in events {
            let current = match &*task.record.borrow() {
                FetchObject::Xhr(xhr) => xhr.is_current_event(&event),
                _ => false,
            };
            if !current { break; }
            self.fetch_xhr_event(&task.receiver, event)?;
        }
        Ok(())
    }
    fn xhr_network_event(&mut self, event: NetEvent) -> Result<(), Throw> {
        let wire = match &event {
            NetEvent::Head { id, .. } | NetEvent::Body { id, .. } |
            NetEvent::Done { id } | NetEvent::Error { id, .. } | NetEvent::Aborted { id } => *id,
        };
        let found = self.state.borrow().net.xhr_pending.iter().position(|task|
            matches!(&task.transport, XhrTransport::Dispatch(pending) if pending.id() == wire));
        let Some(index) = found else { return Ok(()); };
        let mut task = self.state.borrow_mut().net.xhr_pending.remove(index);
        let now = Instant::now();
        let elapsed = now.duration_since(task.started).as_millis() as u64;
        let events = {
            let mut state = self.state.borrow_mut();
            let mut record = task.record.borrow_mut();
            let FetchObject::Xhr(xhr) = &mut *record else { return Ok(()); };
            let XhrTransport::Dispatch(pending) = &mut task.transport else { unreachable!() };
            xhr.on_dispatch_event(pending, &event, &mut state.net.fetch_network.dispatcher, elapsed, now)
        }.map_err(|e| self.fetch_xhr_error(e))?;
        self.xhr_events(&task, events)?;
        let keep = matches!(&*task.record.borrow(), FetchObject::Xhr(xhr)
            if xhr.generation() == task.generation && xhr.send_invoked());
        if keep { self.state.borrow_mut().net.xhr_pending.push(task); }
        Ok(())
    }
    fn xhr_drive(&mut self) -> Result<(), Throw> {
        let tasks = std::mem::take(&mut self.state.borrow_mut().net.xhr_pending);
        let mut survivors = Vec::new();
        for task in tasks {
            let now_ms = task.started.elapsed().as_millis() as u64;
            let events = {
                let mut record = task.record.borrow_mut();
                let FetchObject::Xhr(xhr) = &mut *record else { continue; };
                if xhr.generation() != task.generation || !xhr.send_invoked() { continue; }
                match (&task.transport, xhr.state()) {
                    (XhrTransport::Data, XhrState::Opened) =>
                        xhr.advance(&mut FetchDataOnly, &amazing_fetch::Site::Opaque(0), now_ms),
                    (_, XhrState::HeadersReceived) =>
                        xhr.pull_response(1, now_ms),
                    (_, XhrState::Loading) =>
                        xhr.pull_response(8192, now_ms),
                    (XhrTransport::Dispatch(pending), XhrState::Opened) => {
                        let mut state = self.state.borrow_mut();
                        xhr.check_dispatch_timeout(pending, &mut state.net.fetch_network.dispatcher, now_ms)
                            .map(|events| events.unwrap_or_default())
                    }
                    _ => Ok(Vec::new()),
                }
            }.map_err(|e| self.fetch_xhr_error(e))?;
            self.xhr_events(&task, events)?;
            let keep = matches!(&*task.record.borrow(), FetchObject::Xhr(xhr)
                if xhr.generation() == task.generation && xhr.send_invoked());
            if keep { survivors.push(task); }
        }
        self.state.borrow_mut().net.xhr_pending.extend(survivors);
        Ok(())
    }
}

struct FetchDataOnly;
impl amazing_fetch::NetworkOrCache for FetchDataOnly {
    fn exchange(&mut self, _: &amazing_fetch::Request, _: &amazing_fetch::FetchPlan,
        _: &amazing_fetch::Site) -> Result<amazing_fetch::InternalResponse, amazing_fetch::FetchError> {
        Err(amazing_fetch::FetchError::Transport)
    }
}

impl FetchHost for RuntimeBindingHost<'_> {
    fn fetch_native(
        &mut self,
        name: &'static str,
        length: u8,
        operation: FetchOperation,
        policy: HostCallPolicy,
    ) -> JsValue {
        if operation == FetchOperation::Fetch {
            let global = self.js.binding_global_object();
            self.state.borrow_mut().net.legacy_fetch = self.js.binding_get_property(&global, "fetch").ok();
        }
        self.native(name, length, NativeEntry::Fetch(operation), policy)
    }
    fn fetch_register(
        &mut self,
        interface: FetchInterface,
        prototype: JsValue,
        constructor: JsValue,
    ) {
        self.state
            .borrow_mut()
            .net.fetch_interfaces
            .push((interface, prototype, constructor));
    }
    fn fetch_prototype(&self, interface: FetchInterface) -> Option<JsValue> {
        if interface == FetchInterface::HeadersIterator {
            return self.state.borrow().net.fetch_header_iterator_prototype.clone();
        }
        self.state
            .borrow()
            .net.fetch_interfaces
            .iter()
            .find(|(kind, _, _)| *kind == interface)
            .map(|(_, prototype, _)| prototype.clone())
    }
    fn fetch_allocate(
        &mut self,
        interface: FetchInterface,
        object: FetchObject,
    ) -> Result<JsValue, Throw> {
        let slot = {
            let mut state = self.state.borrow_mut();
            let existing = state.net.fetch_objects.iter().position(|entry| match (&*entry.borrow(), &object) {
                (FetchObject::Headers(FetchHeaders::Request(a)), FetchObject::Headers(FetchHeaders::Request(b))) => Rc::ptr_eq(a, b),
                (FetchObject::Headers(FetchHeaders::Response(a)), FetchObject::Headers(FetchHeaders::Response(b))) => Rc::ptr_eq(a, b),
                (FetchObject::XhrUpload(a), FetchObject::XhrUpload(b)) => Rc::ptr_eq(a, b),
                _ => false,
            });
            if let Some(slot) = existing { slot as u64 } else {
                let slot = state.net.fetch_objects.len() as u64;
                state.net.fetch_objects.push(Rc::new(RefCell::new(object)));
                slot
            }
        };
        let prototype = self
            .fetch_prototype(interface)
            .ok_or_else(|| Throw::type_error("Missing Fetch prototype"))?;
        let value = self.js.binding_wrap_host(
            HostHandle {
                arena: self.state.borrow().arena,
                slot,
                generation: 1,
                brand: interface.brand().0,
            },
            &prototype,
        )?;
        self.materialize_accessors(&value);
        Ok(value)
    }
    fn fetch_record(
        &self,
        value: &JsValue,
        interface: FetchInterface,
    ) -> Result<Rc<RefCell<FetchObject>>, Throw> {
        let key = self
            .platform_object(value, interface.brand())
            .ok_or_else(|| Throw::type_error("Incompatible Fetch receiver"))?;
        self.state
            .borrow()
            .net.fetch_objects
            .get(key.slot as usize)
            .cloned()
            .ok_or_else(|| Throw::type_error("Stale Fetch receiver"))
    }
    fn fetch_own_keys(&mut self, value: &JsValue) -> Result<Vec<String>, Throw> {
        if !matches!(value, JsValue::Object(_)) {
            return Err(self.fetch_type_error("Headers init must be an object"));
        }
        self.js.binding_own_property_keys(value)
    }
    fn fetch_base(&self) -> Result<HttpUrl, Throw> {
        let url = self
            .state
            .borrow()
            .document_url
            .clone()
            .unwrap_or_else(|| "http://localhost/".into());
        HttpUrl::parse(&url).map_err(|_| Throw::type_error("Invalid document URL"))
    }
    fn fetch_local_file(&mut self, args: &[JsValue]) -> Result<Option<JsValue>, Throw> {
        let base = self.state.borrow().document_url.clone().unwrap_or_default();
        if !base.starts_with("file:") && !std::path::Path::new(&base).is_file() {
            return Ok(None);
        }
        let Some(input) = args.first() else { return Ok(None); };
        let input = String::from_utf16_lossy(&self.js.binding_to_dom_string(input)?);
        let target = crate::cmd::js::resolve_script_url(&base, &input);
        if target.starts_with("http:") || target.starts_with("https:")
            || target.starts_with("data:") || target.starts_with("about:") {
            return Ok(None);
        }
        let Some(fetch) = self.state.borrow().net.legacy_fetch.clone() else {
            return Ok(None);
        };
        let global = self.js.binding_global_object();
        binding_call(self.js, &fetch, &global, args).map(Some)
    }
    fn fetch_start(&mut self, request: ApiRequest, signal: AbortSignalId) -> Result<JsValue, Throw> {
        let started = self.state.borrow_mut().net.fetch_network.start(request, ClientContext::default());
        let id = match started {
            Ok(id) => id,
            Err(ApiError::Aborted) => {
                if let Some(reason) = self.fetch_signal_reason(signal) {
                    let promise = self.js.binding_get_property(&self.js.binding_global_object(), "Promise")?;
                    let reject = self.js.binding_get_property(&promise, "reject")?;
                    return binding_call(self.js, &reject, &promise, &[reason]);
                }
                return self.fetch_promise(Err(ApiError::Aborted));
            }
            Err(error) => return self.fetch_promise(Err(error)),
        };
        let executor = self.native("fetch promise executor", 2,
            NativeEntry::FetchPromiseExecutor, HostCallPolicy::CallOnly);
        let promise_class = self.js.binding_get_property(&self.js.binding_global_object(), "Promise")?;
        let promise = self.js.binding_construct(&promise_class, vec![executor])?;
        let (resolve, reject) = self.state.borrow_mut().net.fetch_resolver_capture.take()
            .ok_or_else(|| Throw::type_error("Promise did not supply resolvers"))?;
        self.state.borrow_mut().net.fetch_pending.push((id, resolve, reject, signal));
        self.schedule_fetch_pump()?;
        Ok(promise)
    }
    fn fetch_execute(&mut self, request: &mut ApiRequest) -> Result<ApiResponse, ApiError> {
        let owned = request.clone_request()?;
        let id = self
            .state
            .borrow_mut()
            .net.fetch_network
            .start(owned, ClientContext::default())?;
        let start = Instant::now();
        for tick in 0..15_000 {
            let events = self
                .state
                .borrow_mut()
                .net.fetch_network
                .step(8192, start + Duration::from_millis(tick));
            for event in events {
                if let ServiceEvent::Finished { id: found, result } = event {
                    if found == id {
                        return result;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(ApiError::Network(amazing_fetch::FetchError::Transport))
    }
    fn fetch_promise(&mut self, result: Result<JsValue, ApiError>) -> Result<JsValue, Throw> {
        match result {
            Ok(value) => self.fetch_promise_value(value),
            Err(error) => {
                let reason = self.fetch_error_value(&error)?;
                let global = self.js.binding_global_object();
                let promise = self.js.binding_get_property(&global, "Promise")?;
                let reject = self.js.binding_get_property(&promise, "reject")?;
                binding_call(self.js, &reject, &promise, &[reason])
            }
        }
    }
    fn fetch_promise_value(&mut self, value: JsValue) -> Result<JsValue, Throw> {
        let promise = self
            .js
            .binding_get_property(&self.js.binding_global_object(), "Promise")?;
        let resolve = self.js.binding_get_property(&promise, "resolve")?;
        binding_call(self.js, &resolve, &promise, &[value])
    }
    fn fetch_array_buffer(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        self.js.binding_create_array_buffer(bytes)
    }
    fn fetch_uint8_array(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        self.js.binding_create_uint8_array(bytes)
    }
    fn fetch_buffer_source_bytes(&mut self, value: &JsValue) -> Result<Option<Vec<u8>>, Throw> {
        self.js.binding_buffer_source_bytes(value)
    }
    fn fetch_array_iterator(&mut self, values: Vec<JsValue>) -> Result<JsValue, Throw> {
        let array = binding_create_array(self.js, values)?;
        let method = self.js.binding_get_property(&array, "values")?;
        binding_call(self.js, &method, &array, &[])
    }
    fn fetch_symbol_iterator(&mut self, prototype: &JsValue, method: JsValue) -> Result<(), Throw> {
        self.js.binding_define_symbol_data_property(prototype,
            amazing_js::value::WellKnownSymbol::Iterator.symbol(), method,
            BindingPropertyFlags { writable: true, enumerable: false, configurable: true })
    }
    fn fetch_json_parse(&mut self, text: &str) -> Result<JsValue, Throw> {
        let json = self
            .js
            .binding_get_property(&self.js.binding_global_object(), "JSON")?;
        let parse = self.js.binding_get_property(&json, "parse")?;
        binding_call(self.js, &parse, &json, &[JsValue::str(text)])
    }
    fn fetch_range_error(&mut self, message: &str) -> Throw {
        let global = self.js.binding_global_object();
        match self.js.binding_get_property(&global, "RangeError")
            .and_then(|kind| self.js.binding_construct(&kind, vec![JsValue::str(message)])) {
            Ok(value) => Throw(value), Err(_) => Throw::range_error(message),
        }
    }
    fn fetch_type_error(&mut self, message: &str) -> Throw {
        let global = self.js.binding_global_object();
        match self.js.binding_get_property(&global, "TypeError")
            .and_then(|kind| self.js.binding_construct(&kind, vec![JsValue::str(message)])) {
            Ok(value) => Throw(value), Err(_) => Throw::type_error(message),
        }
    }
    fn fetch_xhr_execute(&mut self, xhr: &mut Xhr) -> Result<Vec<XhrEvent>, XhrError> {
        if xhr.is_data_request() {
            let mut events = xhr.advance(&mut FetchDataOnly, &amazing_fetch::Site::Opaque(0), 0)?;
            while matches!(xhr.state(), XhrState::HeadersReceived | XhrState::Loading) {
                events.extend(xhr.pull_response(8192, 0)?);
            }
            return Ok(events);
        }
        let start = Instant::now();
        let mut state = self.state.borrow_mut();
        let mut pending = xhr.start_dispatch(&mut state.net.fetch_network.dispatcher, 0, start)?;
        let mut events = Vec::new();
        for tick in 0..15_000 {
            let now = start + Duration::from_millis(tick);
            for event in state.net.fetch_network.dispatcher.step(8192, now) {
                events.extend(xhr.on_dispatch_event(&mut pending, &event,
                    &mut state.net.fetch_network.dispatcher, tick, now)?);
            }
            if matches!(xhr.state(), XhrState::HeadersReceived | XhrState::Loading) {
                while matches!(xhr.state(), XhrState::HeadersReceived | XhrState::Loading) {
                    events.extend(xhr.pull_response(8192, tick)?);
                }
                return Ok(events);
            }
            if xhr.state() == XhrState::Done { return Ok(events); }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(XhrError::Timeout)
    }
    fn fetch_xhr_start(&mut self, record: Rc<RefCell<FetchObject>>,
        receiver: JsValue) -> Result<(), Throw> {
        let started = Instant::now();
        let (generation, data) = match &*record.borrow() {
            FetchObject::Xhr(xhr) => (xhr.generation(), xhr.is_data_request()),
            _ => return Err(self.fetch_type_error("Expected XMLHttpRequest")),
        };
        let transport = if data { XhrTransport::Data } else {
            let result = {
                let mut state = self.state.borrow_mut();
                let mut object = record.borrow_mut();
                let FetchObject::Xhr(xhr) = &mut *object else { unreachable!() };
                xhr.start_dispatch(&mut state.net.fetch_network.dispatcher, 0, started)
            };
            XhrTransport::Dispatch(result.map_err(|error| self.fetch_xhr_error(error))?)
        };
        self.state.borrow_mut().net.xhr_pending.push(XhrPending {
            record, receiver, generation, started, transport,
        });
        self.schedule_fetch_pump()
    }
    fn fetch_upload_has_listeners(&mut self, receiver: &JsValue) -> Result<bool, Throw> {
        let upload = self.js.binding_get_property(receiver, "upload")?;
        let key = self.platform_object(&upload, FetchInterface::XMLHttpRequestUpload.brand())
            .ok_or_else(|| Throw::type_error("Invalid XHR upload target"))?;
        if self.state.borrow().net.fetch_listeners.iter().any(|(slot, brand, _, _)|
            *slot == key.slot && *brand == FetchInterface::XMLHttpRequestUpload.brand().0) {
            return Ok(true);
        }
        for name in ["onloadstart", "onprogress", "onabort", "onerror", "onload", "ontimeout", "onloadend"] {
            let handler = self.js.binding_get_property(&upload, name)?;
            if self.js.binding_is_callable(&handler) { return Ok(true); }
        }
        Ok(false)
    }
    fn fetch_xhr_ready_state(&self, receiver: &JsValue, current: XhrState) -> XhrState {
        let Some(key) = self.platform_object(receiver, FetchInterface::XMLHttpRequest.brand())
            else { return current; };
        self.state.borrow().net.xhr_ready_state_override
            .filter(|(slot, _)| *slot == key.slot)
            .map_or(current, |(_, state)| state)
    }
    fn fetch_xhr_event(&mut self, receiver: &JsValue, event: XhrEvent) -> Result<(), Throw> {
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
            binding_call(self.js, &callback, &target, &[value.clone()])?;
        }
        let brand = if event.target == XhrEventTarget::Upload {
            FetchInterface::XMLHttpRequestUpload.brand()
        } else { FetchInterface::XMLHttpRequest.brand() };
        let key = self.platform_object(&target, brand).ok_or_else(|| Throw::type_error("Stale XHR target"))?;
        let listeners = self.state.borrow().net.fetch_listeners.iter()
            .filter(|(slot, family, kind, _)| *slot == key.slot && *family == brand.0 && kind == name)
            .map(|(_, _, _, callback)| callback.clone()).collect::<Vec<_>>();
        for callback in listeners {
            binding_call(self.js, &callback, &target, &[value.clone()])?;
        }
        Ok(())
    }
    fn fetch_xhr_error(&mut self, error: XhrError) -> Throw {
        let name = error.dom_exception_name();
        if name == "TypeError" { return Throw::type_error(format!("{error:?}")); }
        match invoke(self, Operation::DOMExceptionConstructor, JsValue::Undefined,
            &[JsValue::str(format!("{error:?}")), JsValue::str(name)]) {
            Ok(value) => self.throw_value(value), Err(failure) => failure,
        }
    }
    fn fetch_xhr_document(&mut self, receiver: &JsValue, xhr: &Xhr) -> Result<JsValue, Throw> {
        if xhr.state() != XhrState::Done { return Ok(JsValue::Null); }
        let key = self.platform_object(receiver, FetchInterface::XMLHttpRequest.brand())
            .ok_or_else(|| Throw::type_error("Stale XHR receiver"))?;
        let cached = self.state.borrow().net.xhr_document_nodes.get(&key.slot).copied();
        if let Some((generation, document)) = cached {
            if generation == xhr.generation() { return self.wrap_node(document); }
        }
        let Some(source) = xhr.document_source() else { return Ok(JsValue::Null); };
        let document = match self.create_inert_document(&source.bytes, &source.content_type,
            &source.url, source.charset.as_deref()) {
            Ok(document) => document,
            Err(DomError::SyntaxError) => return Ok(JsValue::Null),
            Err(error) => return Err(throw_dom_error(self, error)),
        };
        self.state.borrow_mut().net.xhr_document_nodes.insert(key.slot, (xhr.generation(), document));
        self.wrap_node(document)
    }
    fn fetch_reject_throw(&mut self, error: Throw) -> Result<JsValue, Throw> {
        let promise = self.js.binding_get_property(&self.js.binding_global_object(), "Promise")?;
        let reject = self.js.binding_get_property(&promise, "reject")?;
        binding_call(self.js, &reject, &promise, &[error.0])
    }
    fn fetch_add_listener(&mut self, target: &JsValue, name: String, callback: JsValue) -> Result<(), Throw> {
        if !self.js.binding_is_callable(&callback) { return Ok(()); }
        let (key, brand) = if let Some(key) = self.platform_object(target, FetchInterface::XMLHttpRequest.brand()) {
            (key, FetchInterface::XMLHttpRequest.brand())
        } else {
            let brand = FetchInterface::XMLHttpRequestUpload.brand();
            (self.platform_object(target, brand).ok_or_else(|| Throw::type_error("Invalid XHR target"))?, brand)
        };
        let mut state = self.state.borrow_mut();
        if !state.net.fetch_listeners.iter().any(|(slot, family, kind, value)|
            *slot == key.slot && *family == brand.0 && kind == &name && binding_same_value(self.js, value, &callback)) {
            state.net.fetch_listeners.push((key.slot, brand.0, name, callback));
        }
        Ok(())
    }
    fn fetch_remove_listener(&mut self, target: &JsValue, name: &str, callback: &JsValue) -> Result<(), Throw> {
        let (key, brand) = if let Some(key) = self.platform_object(target, FetchInterface::XMLHttpRequest.brand()) {
            (key, FetchInterface::XMLHttpRequest.brand())
        } else {
            let brand = FetchInterface::XMLHttpRequestUpload.brand();
            (self.platform_object(target, brand).ok_or_else(|| Throw::type_error("Invalid XHR target"))?, brand)
        };
        self.state.borrow_mut().net.fetch_listeners.retain(|(slot, family, kind, value)|
            !(*slot == key.slot && *family == brand.0 && kind == name && binding_same_value(self.js, value, callback)));
        Ok(())
    }
    fn fetch_header_iterator(&mut self, source: Rc<RefCell<FetchObject>>,
        kind: HeaderIterationKind) -> Result<JsValue, Throw> {
        if self.state.borrow().net.fetch_header_iterator_prototype.is_none() {
            let prototype = self.js.binding_create_object(None)?;
            let next = self.native("next", 0, NativeEntry::Fetch(FetchOperation::HeaderIteratorNext), HostCallPolicy::CallOnly);
            self.js.binding_define_data_property(&prototype, "next", next,
                BindingPropertyFlags { writable: true, enumerable: true, configurable: true })?;
            let self_method = self.native("[Symbol.iterator]", 0,
                NativeEntry::Fetch(FetchOperation::HeaderIteratorSelf), HostCallPolicy::CallOnly);
            self.js.binding_define_symbol_data_property(&prototype,
                amazing_js::value::WellKnownSymbol::Iterator.symbol(), self_method,
                BindingPropertyFlags { writable: true, enumerable: false, configurable: true })?;
            self.state.borrow_mut().net.fetch_header_iterator_prototype = Some(prototype);
        }
        self.fetch_allocate(FetchInterface::HeadersIterator,
            FetchObject::HeaderIterator { source, index: 0, kind })
    }
    fn fetch_iterator_result(&mut self, value: JsValue, done: bool) -> Result<JsValue, Throw> {
        let result = self.js.binding_create_object(None)?;
        self.js.binding_define_data_property(&result, "value", value, BindingPropertyFlags::METHOD)?;
        self.js.binding_define_data_property(&result, "done", JsValue::Bool(done), BindingPropertyFlags::METHOD)?;
        Ok(result)
    }
    fn fetch_sequence(&mut self, value: &JsValue) -> Result<Option<Vec<Vec<JsValue>>>, Throw> {
        let JsValue::Object(object) = value else { return Ok(None); };
        let symbol = amazing_js::value::WellKnownSymbol::Iterator.symbol();
        let mut current = Some(object.clone());
        let mut method = None;
        while let Some(object) = current {
            let object = object.borrow();
            if let Some(property) = object.own_prop(&symbol) {
                method = Some(property.value.clone());
                break;
            }
            current = object.proto.clone();
        }
        let Some(method) = method else { return Ok(None); };
        if !self.js.binding_is_callable(&method) { return Err(self.fetch_type_error("Headers iterator is not callable")); }
        let iterator = binding_call(self.js, &method, value, &[])?;
        let next = self.js.binding_get_property(&iterator, "next")?;
        if !self.js.binding_is_callable(&next) { return Err(self.fetch_type_error("Iterator next is not callable")); }
        let mut rows = Vec::new();
        for _ in 0..100_000 {
            let item = binding_call(self.js, &next, &iterator, &[])?;
            let done = self.js.binding_get_property(&item, "done")?;
            if self.js.binding_to_boolean(&done) { return Ok(Some(rows)); }
            let row = self.js.binding_get_property(&item, "value")?;
            let length = self.js.binding_get_property(&row, "length")?;
            let JsValue::Num(length) = length else { return Err(self.fetch_type_error("Header tuple is not a sequence")); };
            if length != 2.0 { return Err(self.fetch_type_error("Header tuple must contain two items")); }
            rows.push(vec![self.js.binding_get_property(&row, "0")?,
                self.js.binding_get_property(&row, "1")?]);
        }
        Err(self.fetch_type_error("Headers sequence too long"))
    }
    fn fetch_number_value(&self, value: &JsValue) -> Option<f64> {
        if let JsValue::Num(n) = value {
            Some(*n)
        } else {
            None
        }
    }
}
