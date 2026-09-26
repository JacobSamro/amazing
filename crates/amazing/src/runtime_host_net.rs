//! Fetch, XHR and WebSocket runtime adapter owned by the net lane.
use super::*;
use amazing_bindings::fetch_interfaces::{FetchHeaders, FetchHost, FetchInterface, FetchObject, FetchOperation, HeaderIterationKind};
use amazing_fetch::{api::{ApiError, ApiRequest, ApiResponse, PendingFetchApi}, service::{ClientContext, NetworkService, ServiceEvent, ServiceId, WebSocketClient}, xhr::{Xhr, XhrError, XhrEvent, XhrEventKind, XhrEventTarget, XhrState}, HttpUrl};
use amazing_net::dispatch::{DispatchLimits, Dispatcher, NetEvent, WebSocketEvent};
use amazing_net::websocket::{Close as WsClose, Event as WsEvent};
use std::time::{Duration, Instant};

/// Own brand namespace for WebSocket instances, disjoint from Fetch's
/// InterfaceBrand(200..206) and messaging's MESSAGING_BRAND_BASE (200).
const WEBSOCKET_BRAND_BASE: u64 = 300;

#[derive(Clone, Copy)]
pub(super) enum NetNativeEntry {
    Operation(FetchOperation),
    PromiseExecutor,
    Pump,
    WebSocket(WebSocketOp),
}

#[derive(Clone, Copy)]
pub(super) enum WebSocketOp {
    Constructor,
    UrlGet,
    ProtocolGet,
    ExtensionsGet,
    ReadyStateGet,
    BufferedAmountGet,
    BinaryTypeGet,
    BinaryTypeSet,
    Send,
    Close,
    AddEventListener,
    RemoveEventListener,
    DispatchEvent,
    OnOpenGet,
    OnOpenSet,
    OnMessageGet,
    OnMessageSet,
    OnErrorGet,
    OnErrorSet,
    OnCloseGet,
    OnCloseSet,
    CloseEventConstructor,
}

#[derive(Clone)]
struct WsListener {
    kind: String,
    callback: JsValue,
    once: bool,
}

/// One `WebSocket` instance. Kept for the object's whole lifetime once
/// allocated (mirrors `fetch_objects`/`xhr_pending`'s own retention), since
/// its `HostHandle` slot must stay valid for as long as the JS wrapper lives.
struct WsObject {
    wrapper: JsValue,
    service_id: Option<ServiceId>,
    url: String,
    ready_state: u16,
    protocol: String,
    binary_type_arraybuffer: bool,
    /// Bytes handed to `send()` that have not yet actually been written to
    /// the transport (WHATWG `bufferedAmount`): increased synchronously by
    /// every `send()` call, decreased as `WebSocketEvent::Sent` reports the
    /// dispatcher's write progress, and never decreased for a CLOSING/
    /// CLOSED socket since nothing is ever transmitted.
    buffered_amount: u64,
    listeners: Vec<WsListener>,
    handlers: HashMap<&'static str, JsValue>,
    /// Set when the connection must fail (CSP/mixed-content/transport
    /// rejection at construction time, or `close()` called while
    /// CONNECTING). Consumed on the next pump tick, which fires `error`
    /// then a synthesized abnormal `close`.
    deferred_failure: bool,
}
impl WsObject {
    fn new(url: String) -> Self {
        Self {
            wrapper: JsValue::Undefined,
            service_id: None,
            url,
            ready_state: 0,
            protocol: String::new(),
            binary_type_arraybuffer: false,
            buffered_amount: 0,
            listeners: Vec::new(),
            handlers: HashMap::new(),
            deferred_failure: false,
        }
    }
}

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
    websockets: Vec<WsObject>,
    ws_by_service: HashMap<ServiceId, usize>,
    websocket_prototype: Option<JsValue>,
    close_event_prototype: Option<JsValue>,
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
            websockets: Vec::new(),
            ws_by_service: HashMap::new(),
            websocket_prototype: None,
            close_event_prototype: None,
        }
    }
}

struct XhrStateOverrideGuard(Rc<RefCell<BindingState>>, Option<(u64, XhrState)>);
impl Drop for XhrStateOverrideGuard {
    fn drop(&mut self) { self.0.borrow_mut().net.xhr_ready_state_override = self.1; }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_net(&mut self) -> Result<(), Throw> {
        amazing_bindings::fetch_interfaces::install(self)?;
        let fetch = self.js.binding_get_property(&self.js.binding_global_object(), "fetch")?;
        let env = EnvHandle::new(EnvKind::Global, Some(self.js.global_env.clone()));
        env.declare_lexical("fetch", fetch, false)?;
        self.js.global_env = env;
        self.install_websocket()?;
        Ok(())
    }

    pub(super) fn invoke_net(&mut self, entry: NetNativeEntry, this: JsValue,
        args: &[JsValue]) -> Result<JsValue, Throw> {
        match entry {
            NetNativeEntry::Operation(operation) =>
                amazing_bindings::fetch_interfaces::invoke(self, operation, this, args),
            NetNativeEntry::PromiseExecutor => {
                let resolve = args.first().cloned().ok_or_else(|| Throw::type_error("Missing promise resolver"))?;
                let reject = args.get(1).cloned().ok_or_else(|| Throw::type_error("Missing promise rejector"))?;
                self.state.borrow_mut().net.fetch_resolver_capture = Some((resolve, reject));
                Ok(JsValue::Undefined)
            }
            NetNativeEntry::Pump => self.fetch_pump(),
            NetNativeEntry::WebSocket(op) => self.websocket_invoke(op, this, args),
        }
    }

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
        let FetchObject::Response(response, _) = &*record else {
            return Err(Throw::type_error("Expected Response"));
        };
        response.clone_response().map_err(|error| Throw::type_error(format!("{error:?}")))
    }
    pub(crate) fn fetch_response_new(&mut self, response: ApiResponse) -> Result<JsValue, Throw> {
        self.fetch_allocate(FetchInterface::Response, FetchObject::Response(response, None))
    }
    fn schedule_fetch_pump(&mut self) -> Result<(), Throw> {
        if self.state.borrow().net.fetch_pump_scheduled { return Ok(()); }
        self.state.borrow_mut().net.fetch_pump_scheduled = true;
        let result = (|| {
            let callback = self.native("fetch network step", 0, NativeEntry::Net(NetNativeEntry::Pump), HostCallPolicy::CallOnly);
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
                ServiceEvent::WebSocket { id, event } => {
                    self.websocket_event(id, event)?;
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
                    let value = self.fetch_allocate(FetchInterface::Response, FetchObject::Response(response, Some(signal)))?;
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
        self.websocket_pump_deferred()?;
        if !self.state.borrow().net.fetch_pending.is_empty() || !self.state.borrow().net.xhr_pending.is_empty()
            || !self.state.borrow().net.ws_by_service.is_empty() {
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

fn is_ws_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
        })
}

fn clamp_u16(n: f64) -> u16 {
    if n.is_nan() { 0 } else { n.clamp(0.0, 65535.0).round() as u16 }
}

/// WebSocket (RFC6455 client transport already lives in `amazing-net`/
/// `amazing-fetch`; this is the JS interface, installed the same way IPC
/// installs MessageChannel/BroadcastChannel in `runtime_host_messaging.rs`:
/// its own local host-object family (brand `WEBSOCKET_BRAND_BASE`), its own
/// hand-rolled addEventListener/removeEventListener/dispatchEvent and on*
/// handler storage (mirroring this same file's XHR listener plumbing above),
/// not the shared `amazing_dom::events` EventTarget registry. `MessageEvent`
/// reuses the global constructor messaging already installs; `CloseEvent`
/// (code/reason/wasClean) has no existing shared binding, so a minimal one is
/// installed here, in the same style as messaging's own `MessageEvent`.
impl RuntimeBindingHost<'_> {
    fn dom_string(&mut self, value: &JsValue) -> Result<String, Throw> {
        self.js
            .binding_to_dom_string(value)
            .map(|units| String::from_utf16_lossy(&units))
    }
    fn ws_dom_exception(&mut self, message: &str, name: &str) -> Throw {
        match invoke(self, Operation::DOMExceptionConstructor, JsValue::Undefined,
            &[JsValue::str(message), JsValue::str(name)]) {
            Ok(value) => self.throw_value(value),
            Err(failure) => failure,
        }
    }
    fn ws_syntax_error(&mut self, message: &str) -> Throw {
        self.ws_dom_exception(message, "SyntaxError")
    }
    fn ws_invalid_access_error(&mut self, message: &str) -> Throw {
        self.ws_dom_exception(message, "InvalidAccessError")
    }
    fn ws_invalid_state_error(&mut self, message: &str) -> Throw {
        self.ws_dom_exception(message, "InvalidStateError")
    }

    fn websocket_slot(&self, value: &JsValue) -> Result<usize, Throw> {
        let handle = self
            .js
            .binding_host_handle(value, WEBSOCKET_BRAND_BASE)
            .ok_or_else(|| Throw::type_error("Incompatible WebSocket receiver"))?;
        if handle.arena != self.state.borrow().arena || handle.generation != 1 {
            return Err(Throw::type_error("Stale WebSocket receiver"));
        }
        let slot = handle.slot as usize;
        if slot >= self.state.borrow().net.websockets.len() {
            return Err(Throw::type_error("Invalid WebSocket receiver"));
        }
        Ok(slot)
    }
    fn websocket_wrapper(&self, slot: usize) -> JsValue {
        self.state.borrow().net.websockets[slot].wrapper.clone()
    }

    /// `(DOMString or sequence<DOMString>)`: a string is one protocol; an
    /// object is walked through its own `Symbol.iterator`, falling back to a
    /// single-item DOMString conversion when neither applies.
    fn websocket_read_protocols(&mut self, value: &JsValue) -> Result<Vec<String>, Throw> {
        if value.is_undefined() {
            return Ok(Vec::new());
        }
        if matches!(value, JsValue::Str(_)) {
            return Ok(vec![self.dom_string(value)?]);
        }
        let JsValue::Object(object) = value else {
            return Ok(vec![self.dom_string(value)?]);
        };
        let symbol = amazing_js::value::WellKnownSymbol::Iterator.symbol();
        let mut current = Some(object.clone());
        let mut method = None;
        while let Some(object) = current {
            let borrowed = object.borrow();
            if let Some(property) = borrowed.own_prop(&symbol) {
                method = Some(property.value.clone());
                break;
            }
            current = borrowed.proto.clone();
        }
        let Some(method) = method else {
            return Ok(vec![self.dom_string(value)?]);
        };
        if !self.js.binding_is_callable(&method) {
            return Ok(vec![self.dom_string(value)?]);
        }
        let iterator = binding_call(self.js, &method, value, &[])?;
        let next = self.js.binding_get_property(&iterator, "next")?;
        let mut items = Vec::new();
        for _ in 0..1024 {
            let item = binding_call(self.js, &next, &iterator, &[])?;
            let done = self.js.binding_get_property(&item, "done")?;
            if self.js.binding_to_boolean(&done) {
                return Ok(items);
            }
            let entry = self.js.binding_get_property(&item, "value")?;
            items.push(self.dom_string(&entry)?);
        }
        Err(self.fetch_type_error("WebSocket protocols sequence too long"))
    }

    fn websocket_construct(&mut self, args: &[JsValue]) -> Result<JsValue, Throw> {
        let url_value = args.first().cloned().unwrap_or(JsValue::Undefined);
        let url_str = self.dom_string(&url_value)?;
        let base_str = self.state.borrow().document_url.clone().unwrap_or_default();
        let base = amazing_net::ParsedUrl::parse(&base_str).ok();
        let mut resolved = amazing_net::ParsedUrl::parse_with_base(&url_str, base.as_ref())
            .map_err(|_| self.ws_syntax_error("Invalid WebSocket URL"))?;
        // WebSockets living standard, constructor steps: after parsing
        // (relative to the entry settings object's API base URL), an
        // `http`/`https` scheme is rewritten to `ws`/`wss` before the
        // scheme is validated -- this is what lets a relative `url` (e.g.
        // `""`, `"test"`) resolve against the document's own http(s)
        // location and still construct successfully.
        match resolved.scheme() {
            "http" => resolved.set_protocol("ws"),
            "https" => resolved.set_protocol("wss"),
            _ => {}
        }
        if !matches!(resolved.scheme(), "ws" | "wss") {
            return Err(self.ws_syntax_error("WebSocket URL must use the ws: or wss: scheme"));
        }
        if resolved.fragment().is_some() {
            return Err(self.ws_syntax_error("WebSocket URL must not include a fragment"));
        }
        let protocols_value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        let protocols = self.websocket_read_protocols(&protocols_value)?;
        // WebSockets living standard constructor steps: duplicate detection
        // among the requested subprotocols is ASCII-case-insensitive (two
        // values differing only by case are still "the same" protocol).
        let mut seen = HashSet::new();
        for protocol in &protocols {
            if !is_ws_token(protocol) || !seen.insert(protocol.to_ascii_lowercase()) {
                return Err(self.ws_syntax_error("Invalid or duplicate WebSocket subprotocol"));
            }
        }
        let url_string = resolved.serialize();

        let slot = {
            let mut state = self.state.borrow_mut();
            let slot = state.net.websockets.len();
            state.net.websockets.push(WsObject::new(url_string.clone()));
            slot
        };
        let prototype = self
            .state
            .borrow()
            .net
            .websocket_prototype
            .clone()
            .ok_or_else(|| Throw::type_error("WebSocket interface not installed"))?;
        let arena = self.state.borrow().arena;
        let wrapper = self.js.binding_wrap_host(
            HostHandle { arena, slot: slot as u64, generation: 1, brand: WEBSOCKET_BRAND_BASE },
            &prototype,
        )?;
        self.state.borrow_mut().net.websockets[slot].wrapper = wrapper.clone();

        let base = self.fetch_base()?;
        let origin = base.origin();
        // WPT `websockets/mixed-content.https.any.js`: a secure top-level
        // document must not be able to open an insecure `ws://` connection,
        // the same "client restricts mixed content" gate `fetch()` and
        // subresource requests already go through.
        let mut context = ClientContext::default();
        context.security.client_restricts_mixed_content = base.as_parsed().scheme() == "https";
        let client = WebSocketClient { origin: origin.clone(), context };
        let started = self
            .state
            .borrow_mut()
            .net
            .fetch_network
            .open_websocket(&url_string, protocols, client);
        match started {
            Ok(id) => {
                let mut state = self.state.borrow_mut();
                state.net.websockets[slot].service_id = Some(id);
                state.net.ws_by_service.insert(id, slot);
            }
            // "Establish a WebSocket connection" fails for CSP/mixed-content/
            // transport reasons: per spec this is an async failure, observed
            // as readyState CONNECTING followed by error+close, never a
            // synchronous constructor exception.
            Err(_) => self.state.borrow_mut().net.websockets[slot].deferred_failure = true,
        }
        self.schedule_fetch_pump()?;
        Ok(wrapper)
    }

    fn websocket_send(&mut self, this: &JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let slot = self.websocket_slot(this)?;
        let ready_state = self.state.borrow().net.websockets[slot].ready_state;
        if ready_state == 0 {
            return Err(self.ws_invalid_state_error("WebSocket is still CONNECTING"));
        }
        let data = args.first().cloned().unwrap_or(JsValue::Undefined);
        // WebIDL `(USVString or Blob or ArrayBuffer or ArrayBufferView) data`:
        // the interface-typed branches are checked structurally; anything
        // else (including a plain string, `null`, `undefined`, a number, or
        // an object with its own `toString`) falls through to the USVString
        // conversion via `ToString`, per the union-type conversion algorithm
        // -- `send(null)` is required to transmit the text "null", not throw.
        let (message, len) = if let Some(bytes) = self.fetch_buffer_source_bytes(&data)? {
            let len = bytes.len() as u64;
            (WsEvent::Binary(bytes), len)
        } else if let Ok(blob) = self.blob_record(&data) {
            let len = blob.bytes.len() as u64;
            (WsEvent::Binary(blob.bytes), len)
        } else {
            let text = self.dom_string(&data)?;
            let len = text.len() as u64;
            (WsEvent::Text(text), len)
        };
        // WHATWG "send a WebSocket message": bufferedAmount increases by the
        // message length synchronously, before any of the data is actually
        // written to the network. If the connection is CLOSING/CLOSED (or
        // the queue attempt is rejected outright), nothing is ever
        // transmitted and this increase is permanent; otherwise it comes
        // back down as `WebSocketEvent::Sent` reports bytes actually
        // flushed to the transport (see `websocket_event`).
        self.state.borrow_mut().net.websockets[slot].buffered_amount += len;
        if ready_state >= 2 {
            return Ok(JsValue::Undefined);
        }
        let service_id = self.state.borrow().net.websockets[slot].service_id;
        if let Some(id) = service_id {
            let _ = self.state.borrow_mut().net.fetch_network.send_websocket(id, message);
        }
        Ok(JsValue::Undefined)
    }

    fn websocket_close(&mut self, this: &JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let slot = self.websocket_slot(this)?;
        let code = match args.first() {
            None | Some(JsValue::Undefined) => None,
            Some(value) => {
                let n = self.js.binding_to_number(value)?;
                let code = clamp_u16(n);
                if code != 1000 && !(3000..=4999).contains(&code) {
                    return Err(self.ws_invalid_access_error(
                        "WebSocket close code must be 1000 or in the range 3000-4999",
                    ));
                }
                Some(code)
            }
        };
        let reason = match args.get(1) {
            None | Some(JsValue::Undefined) => String::new(),
            Some(value) => self.dom_string(value)?,
        };
        if reason.len() > 123 {
            return Err(self.ws_syntax_error("WebSocket close reason must be at most 123 UTF-8 bytes"));
        }
        let (ready_state, service_id) = {
            let state = self.state.borrow();
            let ws = &state.net.websockets[slot];
            (ws.ready_state, ws.service_id)
        };
        match ready_state {
            2 | 3 => {} // CLOSING or CLOSED: no-op.
            0 => {
                // CONNECTING: "fail the WebSocket connection" -- there is no
                // handshake to close, so drop the pending connection and
                // synthesize the failure on the next pump tick.
                if let Some(id) = service_id {
                    self.state.borrow_mut().net.fetch_network.forget_websocket(id);
                    self.state.borrow_mut().net.ws_by_service.remove(&id);
                }
                let mut state = self.state.borrow_mut();
                let ws = &mut state.net.websockets[slot];
                ws.ready_state = 2;
                ws.deferred_failure = true;
                drop(state);
                self.schedule_fetch_pump()?;
            }
            _ => {
                self.state.borrow_mut().net.websockets[slot].ready_state = 2;
                if let Some(id) = service_id {
                    let close = code.map(|code| WsClose { code, reason });
                    let _ = self.state.borrow_mut().net.fetch_network.close_websocket(id, close);
                }
            }
        }
        Ok(JsValue::Undefined)
    }

    fn websocket_binary_type_set(&mut self, slot: usize, value: &JsValue) -> Result<(), Throw> {
        let text = self.dom_string(value)?;
        match text.as_str() {
            "arraybuffer" => self.state.borrow_mut().net.websockets[slot].binary_type_arraybuffer = true,
            "blob" => self.state.borrow_mut().net.websockets[slot].binary_type_arraybuffer = false,
            _ => {} // WHATWG binaryType setter: ignore any other value.
        }
        Ok(())
    }

    fn websocket_add_listener(&mut self, slot: usize, args: &[JsValue]) -> Result<JsValue, Throw> {
        let kind = self.dom_string(args.first().unwrap_or(&JsValue::Undefined))?;
        let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        if !self.js.binding_is_callable(&callback) {
            return Ok(JsValue::Undefined);
        }
        let once = args.get(2).is_some_and(|options| {
            if self.js.binding_to_boolean(options) {
                return true;
            }
            self.js
                .binding_get_property(options, "once")
                .map(|value| self.js.binding_to_boolean(&value))
                .unwrap_or(false)
        });
        let duplicate = self.state.borrow().net.websockets[slot]
            .listeners
            .iter()
            .any(|l| l.kind == kind && binding_same_value(self.js, &l.callback, &callback));
        if !duplicate {
            self.state.borrow_mut().net.websockets[slot]
                .listeners
                .push(WsListener { kind, callback, once });
        }
        Ok(JsValue::Undefined)
    }
    fn websocket_remove_listener(&mut self, slot: usize, args: &[JsValue]) -> Result<JsValue, Throw> {
        let kind = self.dom_string(args.first().unwrap_or(&JsValue::Undefined))?;
        let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        self.state.borrow_mut().net.websockets[slot]
            .listeners
            .retain(|l| !(l.kind == kind && binding_same_value(self.js, &l.callback, &callback)));
        Ok(JsValue::Undefined)
    }
    fn websocket_dispatch_event(&mut self, slot: usize, args: &[JsValue]) -> Result<JsValue, Throw> {
        let event = args.first().cloned().unwrap_or(JsValue::Undefined);
        let kind = self
            .js
            .binding_get_property(&event, "type")
            .and_then(|value| self.dom_string(&value))
            .unwrap_or_default();
        self.websocket_dispatch(slot, &kind, event)?;
        Ok(JsValue::Bool(true))
    }
    fn websocket_handler_get(&self, slot: usize, key: &str) -> JsValue {
        self.state
            .borrow()
            .net
            .websockets
            .get(slot)
            .and_then(|ws| ws.handlers.get(key).cloned())
            .unwrap_or(JsValue::Null)
    }
    fn websocket_handler_set(&mut self, slot: usize, key: &'static str, value: JsValue) {
        // WebHTML "event handler IDL attributes" setter, `EventHandler`
        // being [TreatNonCallableAsNull]: a value that is not callable
        // (including null/undefined, but also e.g. a plain number or a
        // non-callable object) converts to null, not a stored non-function
        // value -- the getter must read back null, never the raw input.
        let callable = self.js.binding_is_callable(&value);
        let mut state = self.state.borrow_mut();
        let handlers = &mut state.net.websockets[slot].handlers;
        if callable {
            handlers.insert(key, value);
        } else {
            handlers.remove(key);
        }
    }

    /// Fire `kind` at `slot`'s on-handler and its listeners, in that order,
    /// matching this file's XHR event dispatch (`fetch_xhr_event`) rather
    /// than the shared `amazing_dom::events` registry: WebSocket is its own
    /// local host-object family, like MessagePort/BroadcastChannel.
    fn websocket_dispatch(&mut self, slot: usize, kind: &str, event: JsValue) -> Result<(), Throw> {
        let wrapper = self.websocket_wrapper(slot);
        let flags = BindingPropertyFlags::METHOD;
        self.js.binding_define_data_property(&event, "target", wrapper.clone(), flags)?;
        self.js.binding_define_data_property(&event, "currentTarget", wrapper.clone(), flags)?;
        let handler = self.state.borrow().net.websockets[slot].handlers.get(kind).cloned();
        if let Some(handler) = handler {
            if self.js.binding_is_callable(&handler) {
                binding_call(self.js, &handler, &wrapper, &[event.clone()])?;
            }
        }
        let listeners = self.state.borrow().net.websockets[slot].listeners.clone();
        let mut remaining = Vec::with_capacity(listeners.len());
        for listener in listeners {
            let matched = listener.kind == kind;
            if matched {
                binding_call(self.js, &listener.callback, &wrapper, &[event.clone()])?;
            }
            if !(matched && listener.once) {
                remaining.push(listener);
            }
        }
        self.state.borrow_mut().net.websockets[slot].listeners = remaining;
        Ok(())
    }
    fn websocket_fire_simple(&mut self, slot: usize, kind: &str) -> Result<(), Throw> {
        let event = invoke(self, Operation::EventConstructor, JsValue::Undefined, &[JsValue::str(kind)])?;
        self.websocket_dispatch(slot, kind, event)
    }
    fn websocket_fire_message(&mut self, slot: usize, message: WsEvent) -> Result<(), Throw> {
        let arraybuffer = self.state.borrow().net.websockets[slot].binary_type_arraybuffer;
        let data = match message {
            WsEvent::Text(text) => JsValue::str(text),
            WsEvent::Binary(bytes) => {
                if arraybuffer {
                    self.fetch_array_buffer(bytes)?
                } else {
                    let blob = Blob::from_bytes(bytes, "");
                    self.blob_new(blob)?
                }
            }
            WsEvent::Close(_) => return Ok(()), // never surfaced through Message.
        };
        // The installed global `MessageEvent` constructor is messaging's own
        // hand-rolled `MessagingOp::MessageEventConstructor` (see
        // runtime_host_messaging.rs), which takes a single init-dictionary
        // argument (no separate `type`) and hardcodes `type: "message"`.
        let ctor = self.js.binding_get_property(&self.js.binding_global_object(), "MessageEvent")?;
        let init = self.js.binding_create_object(None)?;
        self.js.binding_define_data_property(&init, "data", data, BindingPropertyFlags::METHOD)?;
        let event = self.js.binding_construct(&ctor, vec![init])?;
        self.websocket_dispatch(slot, "message", event)
    }
    fn websocket_make_close_event(&mut self, code: u16, reason: &str, was_clean: bool) -> Result<JsValue, Throw> {
        let prototype = self
            .state
            .borrow()
            .net
            .close_event_prototype
            .clone()
            .unwrap_or_else(|| self.js.binding_object_prototype());
        let event = self.js.binding_create_object(Some(&prototype))?;
        let flags = BindingPropertyFlags::METHOD;
        self.js.binding_define_data_property(&event, "type", JsValue::str("close"), flags)?;
        self.js.binding_define_data_property(&event, "code", JsValue::num(code as f64), flags)?;
        self.js.binding_define_data_property(&event, "reason", JsValue::str(reason), flags)?;
        self.js.binding_define_data_property(&event, "wasClean", JsValue::Bool(was_clean), flags)?;
        self.js.binding_define_data_property(&event, "bubbles", JsValue::Bool(false), flags)?;
        self.js.binding_define_data_property(&event, "cancelable", JsValue::Bool(false), flags)?;
        self.js.binding_define_data_property(&event, "isTrusted", JsValue::Bool(true), flags)?;
        Ok(event)
    }
    fn websocket_fire_close(&mut self, slot: usize, code: u16, reason: String, was_clean: bool) -> Result<(), Throw> {
        let event = self.websocket_make_close_event(code, &reason, was_clean)?;
        self.websocket_dispatch(slot, "close", event)
    }
    fn websocket_close_event_construct(&mut self, args: &[JsValue]) -> Result<JsValue, Throw> {
        let prototype = self
            .state
            .borrow()
            .net
            .close_event_prototype
            .clone()
            .unwrap_or_else(|| self.js.binding_object_prototype());
        let event = self.js.binding_create_object(Some(&prototype))?;
        let ty = args.first().cloned().unwrap_or(JsValue::Undefined);
        let ty = self.dom_string(&ty)?;
        let init = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        let read = |host: &mut Self, name: &str| -> Result<JsValue, Throw> {
            if host.js.binding_is_nullish(&init) {
                return Ok(JsValue::Undefined);
            }
            host.js.binding_get_property(&init, name)
        };
        let code = read(self, "code")?;
        let reason = read(self, "reason")?;
        let was_clean = read(self, "wasClean")?;
        let flags = BindingPropertyFlags::METHOD;
        self.js.binding_define_data_property(&event, "type", JsValue::str(ty), flags)?;
        self.js.binding_define_data_property(
            &event, "code", if code.is_undefined() { JsValue::num(0.0) } else { code }, flags,
        )?;
        self.js.binding_define_data_property(
            &event, "reason", if reason.is_undefined() { JsValue::str("") } else { reason }, flags,
        )?;
        let was_clean = if was_clean.is_undefined() { false } else { self.js.binding_to_boolean(&was_clean) };
        self.js.binding_define_data_property(&event, "wasClean", JsValue::Bool(was_clean), flags)?;
        self.js.binding_define_data_property(&event, "bubbles", JsValue::Bool(false), flags)?;
        self.js.binding_define_data_property(&event, "cancelable", JsValue::Bool(false), flags)?;
        self.js.binding_define_data_property(&event, "isTrusted", JsValue::Bool(false), flags)?;
        Ok(event)
    }

    /// Route one `ServiceEvent::WebSocket` to its object. Exactly one of
    /// `Close`/`Error` ever arrives per socket; both remove it from
    /// `ws_by_service` so the pump loop stops driving it.
    fn websocket_event(&mut self, id: ServiceId, event: WebSocketEvent) -> Result<(), Throw> {
        let Some(slot) = self.state.borrow().net.ws_by_service.get(&id).copied() else {
            return Ok(());
        };
        match event {
            WebSocketEvent::Open { protocol, .. } => {
                {
                    let mut state = self.state.borrow_mut();
                    let ws = &mut state.net.websockets[slot];
                    ws.ready_state = 1;
                    ws.protocol = protocol.unwrap_or_default();
                }
                self.websocket_fire_simple(slot, "open")
            }
            WebSocketEvent::Message { message, .. } => self.websocket_fire_message(slot, message),
            WebSocketEvent::Close { close, .. } => {
                let (code, reason) = match close {
                    Some(close) => (close.code, close.reason),
                    // WHATWG default when no status code was received.
                    None => (1005, String::new()),
                };
                {
                    let mut state = self.state.borrow_mut();
                    state.net.websockets[slot].ready_state = 3;
                    state.net.ws_by_service.remove(&id);
                }
                self.websocket_fire_close(slot, code, reason, true)
            }
            WebSocketEvent::Error { .. } => {
                {
                    let mut state = self.state.borrow_mut();
                    state.net.websockets[slot].ready_state = 3;
                    state.net.ws_by_service.remove(&id);
                }
                self.websocket_fire_simple(slot, "error")?;
                self.websocket_fire_close(slot, 1006, String::new(), false)
            }
            WebSocketEvent::Sent { bytes, .. } => {
                // WHATWG: `bufferedAmount` decreases as previously-queued
                // `send()` payload bytes are actually transmitted; it never
                // fires a script-observable event by itself.
                let mut state = self.state.borrow_mut();
                let buffered = &mut state.net.websockets[slot].buffered_amount;
                *buffered = buffered.saturating_sub(bytes);
                Ok(())
            }
        }
    }

    /// Fire the deferred error+close pair for sockets whose connection
    /// attempt was rejected synchronously (CSP/mixed-content/transport) or
    /// whose `close()` arrived while still CONNECTING. Called every pump
    /// tick so the failure surfaces as a real task, not inline with the
    /// constructor or `close()` call that requested it.
    fn websocket_pump_deferred(&mut self) -> Result<(), Throw> {
        let slots: Vec<usize> = {
            let mut state = self.state.borrow_mut();
            let mut found = Vec::new();
            for (index, ws) in state.net.websockets.iter_mut().enumerate() {
                if ws.deferred_failure && ws.ready_state != 3 {
                    ws.deferred_failure = false;
                    found.push(index);
                }
            }
            found
        };
        for slot in slots {
            self.state.borrow_mut().net.websockets[slot].ready_state = 3;
            self.websocket_fire_simple(slot, "error")?;
            self.websocket_fire_close(slot, 1006, String::new(), false)?;
        }
        Ok(())
    }

    pub(super) fn websocket_invoke(&mut self, op: WebSocketOp, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        match op {
            WebSocketOp::Constructor => self.websocket_construct(args),
            WebSocketOp::UrlGet => {
                let slot = self.websocket_slot(&this)?;
                Ok(JsValue::str(self.state.borrow().net.websockets[slot].url.clone()))
            }
            WebSocketOp::ProtocolGet => {
                let slot = self.websocket_slot(&this)?;
                Ok(JsValue::str(self.state.borrow().net.websockets[slot].protocol.clone()))
            }
            WebSocketOp::ExtensionsGet => {
                self.websocket_slot(&this)?;
                Ok(JsValue::str(""))
            }
            WebSocketOp::ReadyStateGet => {
                let slot = self.websocket_slot(&this)?;
                Ok(JsValue::num(self.state.borrow().net.websockets[slot].ready_state as f64))
            }
            WebSocketOp::BufferedAmountGet => {
                let slot = self.websocket_slot(&this)?;
                Ok(JsValue::num(self.state.borrow().net.websockets[slot].buffered_amount as f64))
            }
            WebSocketOp::BinaryTypeGet => {
                let slot = self.websocket_slot(&this)?;
                let arraybuffer = self.state.borrow().net.websockets[slot].binary_type_arraybuffer;
                Ok(JsValue::str(if arraybuffer { "arraybuffer" } else { "blob" }))
            }
            WebSocketOp::BinaryTypeSet => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_binary_type_set(slot, args.first().unwrap_or(&JsValue::Undefined))?;
                Ok(JsValue::Undefined)
            }
            WebSocketOp::Send => self.websocket_send(&this, args),
            WebSocketOp::Close => self.websocket_close(&this, args),
            WebSocketOp::AddEventListener => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_add_listener(slot, args)
            }
            WebSocketOp::RemoveEventListener => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_remove_listener(slot, args)
            }
            WebSocketOp::DispatchEvent => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_dispatch_event(slot, args)
            }
            WebSocketOp::OnOpenGet => { let slot = self.websocket_slot(&this)?; Ok(self.websocket_handler_get(slot, "open")) }
            WebSocketOp::OnOpenSet => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_handler_set(slot, "open", args.first().cloned().unwrap_or(JsValue::Undefined));
                Ok(JsValue::Undefined)
            }
            WebSocketOp::OnMessageGet => { let slot = self.websocket_slot(&this)?; Ok(self.websocket_handler_get(slot, "message")) }
            WebSocketOp::OnMessageSet => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_handler_set(slot, "message", args.first().cloned().unwrap_or(JsValue::Undefined));
                Ok(JsValue::Undefined)
            }
            WebSocketOp::OnErrorGet => { let slot = self.websocket_slot(&this)?; Ok(self.websocket_handler_get(slot, "error")) }
            WebSocketOp::OnErrorSet => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_handler_set(slot, "error", args.first().cloned().unwrap_or(JsValue::Undefined));
                Ok(JsValue::Undefined)
            }
            WebSocketOp::OnCloseGet => { let slot = self.websocket_slot(&this)?; Ok(self.websocket_handler_get(slot, "close")) }
            WebSocketOp::OnCloseSet => {
                let slot = self.websocket_slot(&this)?;
                self.websocket_handler_set(slot, "close", args.first().cloned().unwrap_or(JsValue::Undefined));
                Ok(JsValue::Undefined)
            }
            WebSocketOp::CloseEventConstructor => self.websocket_close_event_construct(args),
        }
    }

    fn install_websocket(&mut self) -> Result<(), Throw> {
        let global = self.js.binding_global_object();
        let object_proto = self.js.binding_object_prototype();

        // CloseEvent: no shared binding exists yet (see .agents/dom-bindings/
        // questions.md), so install a minimal constructible one, in the same
        // driver-local style as messaging's MessageEvent.
        let close_prototype = self.js.binding_create_object(Some(&object_proto))?;
        let close_ctor = self.native(
            "CloseEvent", 1,
            NativeEntry::Net(NetNativeEntry::WebSocket(WebSocketOp::CloseEventConstructor)),
            HostCallPolicy::ConstructOnly,
        );
        self.define_data_property(&close_ctor, "prototype", close_prototype.clone(), false, false, false)?;
        self.define_data_property(&close_prototype, "constructor", close_ctor.clone(), true, false, true)?;
        self.define_data_property(&global, "CloseEvent", close_ctor, true, false, true)?;
        self.state.borrow_mut().net.close_event_prototype = Some(close_prototype);

        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        let constructor = self.native(
            "WebSocket", 1,
            NativeEntry::Net(NetNativeEntry::WebSocket(WebSocketOp::Constructor)),
            HostCallPolicy::ConstructOnly,
        );
        self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;

        for (name, value) in [("CONNECTING", 0.0), ("OPEN", 1.0), ("CLOSING", 2.0), ("CLOSED", 3.0)] {
            self.define_data_property(&constructor, name, JsValue::num(value), false, false, false)?;
            self.define_data_property(&prototype, name, JsValue::num(value), false, false, false)?;
        }

        for (name, op) in [
            ("url", WebSocketOp::UrlGet),
            ("readyState", WebSocketOp::ReadyStateGet),
            ("bufferedAmount", WebSocketOp::BufferedAmountGet),
            ("extensions", WebSocketOp::ExtensionsGet),
            ("protocol", WebSocketOp::ProtocolGet),
        ] {
            let getter = self.native(name, 0, NativeEntry::Net(NetNativeEntry::WebSocket(op)), HostCallPolicy::CallOnly);
            self.define_accessor_property(&prototype, name, Some(getter), None, true, true)?;
        }
        {
            let getter = self.native("binaryType", 0,
                NativeEntry::Net(NetNativeEntry::WebSocket(WebSocketOp::BinaryTypeGet)), HostCallPolicy::CallOnly);
            let setter = self.native("binaryType", 1,
                NativeEntry::Net(NetNativeEntry::WebSocket(WebSocketOp::BinaryTypeSet)), HostCallPolicy::CallOnly);
            self.define_accessor_property(&prototype, "binaryType", Some(getter), Some(setter), true, true)?;
        }

        for (name, length, op) in [
            ("send", 1u8, WebSocketOp::Send),
            ("close", 0, WebSocketOp::Close),
            ("addEventListener", 2, WebSocketOp::AddEventListener),
            ("removeEventListener", 2, WebSocketOp::RemoveEventListener),
            ("dispatchEvent", 1, WebSocketOp::DispatchEvent),
        ] {
            let function = self.native(name, length, NativeEntry::Net(NetNativeEntry::WebSocket(op)), HostCallPolicy::CallOnly);
            self.define_data_property(&prototype, name, function, true, false, true)?;
        }

        for (name, get_op, set_op) in [
            ("onopen", WebSocketOp::OnOpenGet, WebSocketOp::OnOpenSet),
            ("onmessage", WebSocketOp::OnMessageGet, WebSocketOp::OnMessageSet),
            ("onerror", WebSocketOp::OnErrorGet, WebSocketOp::OnErrorSet),
            ("onclose", WebSocketOp::OnCloseGet, WebSocketOp::OnCloseSet),
        ] {
            let getter = self.native(name, 0, NativeEntry::Net(NetNativeEntry::WebSocket(get_op)), HostCallPolicy::CallOnly);
            let setter = self.native(name, 1, NativeEntry::Net(NetNativeEntry::WebSocket(set_op)), HostCallPolicy::CallOnly);
            self.define_accessor_property(&prototype, name, Some(getter), Some(setter), true, true)?;
        }

        self.define_data_property(&global, "WebSocket", constructor, true, false, true)?;
        self.state.borrow_mut().net.websocket_prototype = Some(prototype);
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
        self.native(name, length, NativeEntry::Net(NetNativeEntry::Operation(operation)), policy)
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
    fn fetch_body_abort_rejection(&mut self, signal: AbortSignalId) -> Result<Option<JsValue>, Throw> {
        let Some(reason) = self.fetch_signal_reason(signal) else { return Ok(None); };
        let global = self.js.binding_global_object();
        let promise = self.js.binding_get_property(&global, "Promise")?;
        let reject = self.js.binding_get_property(&promise, "reject")?;
        binding_call(self.js, &reject, &promise, &[reason]).map(Some)
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
            NativeEntry::Net(NetNativeEntry::PromiseExecutor), HostCallPolicy::CallOnly);
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
        // A synchronous XHR blocks the event loop, but it must not drive the
        // shared NetworkService dispatcher: `Dispatcher::step` advances every
        // in-flight connection, so consuming its events here would silently drop
        // the replies of pending asynchronous XHRs and fetches, leaving them
        // hung. Give the blocking request its own dispatcher (same StdResolver,
        // so the WPT shim host routing still applies) so asynchronous traffic
        // keeps its events.
        let start = Instant::now();
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        let mut pending = xhr.start_dispatch(&mut dispatcher, 0, start)?;
        let mut events = Vec::new();
        for tick in 0..15_000 {
            let now = start + Duration::from_millis(tick);
            for event in dispatcher.step(8192, now) {
                events.extend(xhr.on_dispatch_event(&mut pending, &event,
                    &mut dispatcher, tick, now)?);
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
            // The iterator prototype must chain to %IteratorPrototype%, which
            // WPT's `checkIteratorProperties` verifies through
            // Object.getPrototypeOf(Object.getPrototypeOf(iterator)).
            let iterator_base = self.js.run_script(
                "Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()))",
            )?;
            let prototype = self.js.binding_create_object(Some(&iterator_base))?;
            let next = self.native("next", 0, NativeEntry::Net(NetNativeEntry::Operation(FetchOperation::HeaderIteratorNext)), HostCallPolicy::CallOnly);
            self.js.binding_define_data_property(&prototype, "next", next,
                BindingPropertyFlags { writable: true, enumerable: true, configurable: true })?;
            let self_method = self.native("[Symbol.iterator]", 0,
                NativeEntry::Net(NetNativeEntry::Operation(FetchOperation::HeaderIteratorSelf)), HostCallPolicy::CallOnly);
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    use amazing_js::value::JsValue;

    use crate::{dom_host::BrowserDomHost, js_scheduler::DriverScheduler};

    // Minimal SHA-1, used only to compute the WebSocket handshake's
    // Sec-WebSocket-Accept header for the local echo server below.
    // `amazing_net::websocket::handshake::{base64, accept}` are `pub(crate)`
    // to amazing-net and intentionally not widened for this driver-level test.
    fn sha1(data: &[u8]) -> [u8; 20] {
        let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
        let ml = (data.len() as u64) * 8;
        let mut msg = data.to_vec();
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&ml.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for (i, word) in w.iter_mut().take(16).enumerate() {
                *word = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
            for (i, word) in w.iter().enumerate() {
                let (f, k) = if i < 20 {
                    ((b & c) | ((!b) & d), 0x5A82_7999u32)
                } else if i < 40 {
                    (b ^ c ^ d, 0x6ED9_EBA1u32)
                } else if i < 60 {
                    ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32)
                } else {
                    (b ^ c ^ d, 0xCA62_C1D6u32)
                };
                let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = temp;
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
        }
        let mut out = [0u8; 20];
        for (i, word) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
    fn base64_encode(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b0 = chunk[0];
            let b1 = *chunk.get(1).unwrap_or(&0);
            let b2 = *chunk.get(2).unwrap_or(&0);
            let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
            out.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
            out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 0x3F) as usize] as char } else { '=' });
            out.push(if chunk.len() > 2 { CHARS[(n & 0x3F) as usize] as char } else { '=' });
        }
        out
    }
    fn read_ws_frame(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).ok()?;
        let opcode = head[0] & 0x0F;
        let masked = head[1] & 0x80 != 0;
        let mut len = (head[1] & 0x7F) as u64;
        if len == 126 {
            let mut ext = [0u8; 2];
            stream.read_exact(&mut ext).ok()?;
            len = u16::from_be_bytes(ext) as u64;
        } else if len == 127 {
            let mut ext = [0u8; 8];
            stream.read_exact(&mut ext).ok()?;
            len = u64::from_be_bytes(ext);
        }
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask).ok()?;
        }
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload).ok()?;
        if masked {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
        }
        Some((opcode, payload))
    }
    fn write_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) {
        let mut frame = vec![0x80 | opcode];
        let len = payload.len();
        if len < 126 {
            frame.push(len as u8);
        } else if len < 65536 {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        let _ = stream.write_all(&frame);
        let _ = stream.flush();
    }
    /// A one-shot RFC6455 echo server: accepts a single connection, completes
    /// the handshake, then echoes back every text/binary frame and answers a
    /// close frame with its own close frame before dropping the socket.
    fn spawn_ws_echo_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return; };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&buf);
            let key = text
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key:"))
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let accept_input = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            let accept = base64_encode(&sha1(accept_input.as_bytes()));
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
            let _ = stream.flush();
            loop {
                let Some((opcode, payload)) = read_ws_frame(&mut stream) else { return; };
                match opcode {
                    0x8 => {
                        write_ws_frame(&mut stream, 0x8, &payload);
                        return;
                    }
                    0x1 | 0x2 => write_ws_frame(&mut stream, opcode, &payload),
                    _ => {}
                }
            }
        });
        port
    }

    /// A loopback server requiring `Authorization: Basic Zm9vOmJhcg==`
    /// (`foo:bar`) before completing the opening handshake, matching WPT's
    /// `websockets/basic_auth` shim handler -- for NET-39's URL-userinfo
    /// (`ws://foo:bar@host/...`) -> `Authorization` header fix.
    fn spawn_ws_basic_auth_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return; };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&buf);
            let authorized = text
                .lines()
                .any(|line| line.trim() == "Authorization: Basic Zm9vOmJhcg==");
            if !authorized {
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Basic realm=\"x\"\r\n\r\n",
                );
                return;
            }
            let key = text
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key:"))
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let accept_input = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            let accept = base64_encode(&sha1(accept_input.as_bytes()));
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
        });
        port
    }

    /// Full round trip against a real local server: constructor connects,
    /// `onopen` sends a text message, the echo server bounces it back through
    /// `onmessage`, and a client-initiated `close(1000, ...)` completes with a
    /// clean `onclose` (code 1000, `wasClean === true`).
    #[test]
    fn websocket_echo_open_message_close() {
        let port = spawn_ws_echo_server();

        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/");

        scheduler
            .run_script(&format!(
                "var opened=false, received=null, closed=false, closeCode=null, closeClean=null; \
                 var ws = new WebSocket('ws://127.0.0.1:{port}/'); \
                 ws.onopen = function() {{ opened = true; ws.send('hello'); }}; \
                 ws.onmessage = function(e) {{ received = e.data; ws.close(1000, 'bye'); }}; \
                 ws.onclose = function(e) {{ closed = true; closeCode = e.code; closeClean = e.wasClean; }};"
            ))
            .unwrap();

        let mut closed = false;
        for _ in 0..400 {
            if scheduler.run_script("closed").unwrap() == JsValue::Bool(true) {
                closed = true;
                break;
            }
            scheduler.pump_now(false).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(closed, "WebSocket echo round-trip did not complete: no close event fired");
        assert_eq!(scheduler.run_script("opened").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("received === 'hello'").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("closeCode === 1000").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("closeClean === true").unwrap(), JsValue::Bool(true));
    }

    /// A large (65000-byte) single-frame text message round-trips the same
    /// way `hello` does: `bufferedAmount` reflects the queued length right
    /// after `send()` (WHATWG "send a WebSocket message"), the echo server's
    /// reply still arrives, and `bufferedAmount` decreases via
    /// `WebSocketEvent::Sent` once the frame is actually flushed.
    #[test]
    fn websocket_large_message_round_trip_tracks_buffered_amount() {
        let port = spawn_ws_echo_server();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/");
        scheduler
            .run_script(&format!(
                "var opened=false, received=null, closed=false, bufAfterSend=null; \
                 var ws = new WebSocket('ws://127.0.0.1:{port}/'); \
                 ws.onopen = function() {{ \
                     opened = true; \
                     var data = ''; for (var i = 0; i < 65000; i++) {{ data += 'c'; }} \
                     ws.send(data); \
                     bufAfterSend = ws.bufferedAmount; \
                 }}; \
                 ws.onmessage = function(e) {{ received = e.data.length; ws.close(1000, 'bye'); }}; \
                 ws.onclose = function(e) {{ closed = true; }};"
            ))
            .unwrap();
        let mut closed = false;
        for _ in 0..2000 {
            if scheduler.run_script("closed").unwrap() == JsValue::Bool(true) {
                closed = true;
                break;
            }
            scheduler.pump_now(false).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(closed, "large message round-trip did not complete");
        assert_eq!(scheduler.run_script("opened").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("bufAfterSend === 65000").unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("received === 65000").unwrap(), JsValue::Bool(true));
    }

    /// NET-39 (WPT `websockets/basic-auth.any.js`): a `ws://user:pass@host/`
    /// URL authenticates via an `Authorization: Basic` header derived from
    /// the userinfo, the same as `fetch()`/XHR, rather than being rejected.
    #[test]
    fn websocket_url_userinfo_sends_basic_auth_header() {
        let port = spawn_ws_basic_auth_server();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/");
        scheduler
            .run_script(&format!(
                "var opened=false, errored=false; \
                 var ws = new WebSocket('ws://foo:bar@127.0.0.1:{port}/'); \
                 ws.onopen = function() {{ opened = true; ws.close(); }}; \
                 ws.onerror = function() {{ errored = true; }};"
            ))
            .unwrap();
        let mut opened = false;
        for _ in 0..400 {
            if scheduler.run_script("opened || errored").unwrap() == JsValue::Bool(true) {
                opened = scheduler.run_script("opened").unwrap() == JsValue::Bool(true);
                break;
            }
            scheduler.pump_now(false).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(opened, "WebSocket with URL userinfo should authenticate and open");
    }

    /// Constructor-time URL validation: relative-to-base resolution, scheme
    /// restricted to ws/wss, and no fragment -- all synchronous SyntaxError
    /// per the WHATWG WebSocket constructor steps, so no server is needed.
    #[test]
    fn websocket_constructor_rejects_bad_urls() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/");

        let cases = [
            "ftp://example.test/",        // scheme not http(s)/ws(s), and not rewritten
            "ws://example.test/#frag",    // fragment not allowed
            "ws://",                      // special scheme requires a non-empty host
        ];
        for case in cases {
            let script = format!(
                "(function() {{ try {{ new WebSocket('{case}'); return false; }} \
                 catch (e) {{ return e.name === 'SyntaxError'; }} }})()"
            );
            let result = scheduler.run_script(&script).unwrap();
            assert_eq!(result, JsValue::Bool(true), "expected SyntaxError for {case:?}");
        }

        // A valid construction starts CONNECTING and exposes the resolved URL.
        let ok = scheduler
            .run_script("var w = new WebSocket('ws://example.test/socket'); w.readyState === 0 && w.url === 'ws://example.test/socket'")
            .unwrap();
        assert_eq!(ok, JsValue::Bool(true));

        // WebSockets living standard constructor steps 4-5: an `http`/`https`
        // scheme (typically from a relative URL resolved against the
        // document's own location) is rewritten to `ws`/`wss` before the
        // scheme is validated, so an absolute `http://` URL constructs
        // successfully rather than throwing.
        let rewritten = scheduler
            .run_script("var w2 = new WebSocket('http://example.test/socket'); w2.url === 'ws://example.test/socket'")
            .unwrap();
        assert_eq!(rewritten, JsValue::Bool(true), "http scheme should rewrite to ws");
    }

    #[test]
    fn fetch_wrappers_do_not_alias_storage_wrappers() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/");
        let result = scheduler.run_script("var r = new Request('/'); var h = new Headers(); h.append('X-Probe', 'yes'); r.method === 'GET' && !!r.signal && h.has('x-probe') && h.get('x-probe') === 'yes'")
            .unwrap_or_else(|error| panic!("Fetch wrapper probe: {}", error.describe()));
        assert_eq!(result, JsValue::Bool(true));
    }

    /// A synchronous XHR blocks the event loop, but it must not advance the
    /// shared NetworkService dispatcher: doing so consumes and discards the
    /// replies of pending asynchronous requests, leaving them hung. The async
    /// `/slow` reply lands during the blocking `/fast` sync request.
    #[test]
    fn synchronous_xhr_keeps_pending_async_xhr_reply() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // One thread per connection, detached; the process exits with the test.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break; };
                std::thread::spawn(move || {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    let request = String::from_utf8_lossy(&buf);
                    let delay = if request.starts_with("GET /slow") { 60 } else { 600 };
                    std::thread::sleep(Duration::from_millis(delay));
                    let body = b"ok";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                        body.len());
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                    let _ = stream.flush();
                });
            }
        });

        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url(&format!("http://127.0.0.1:{port}/"));

        scheduler.run_script(
            "var asyncDone=false; var a=new XMLHttpRequest(); a.onload=function(){asyncDone=true;}; \
             a.open('GET','/slow'); a.send();"
        ).unwrap();
        // Drive the asynchronous request into its in-flight read state.
        for _ in 0..30 {
            scheduler.pump_now(false).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        // The blocking request runs for 600ms; the 60ms async reply arrives while
        // it is blocked. Before the isolation fix the sync loop ate that reply.
        scheduler.run_script(
            "var s=new XMLHttpRequest(); s.open('GET','/fast',false); s.send(); s.status"
        ).unwrap();

        let mut done = false;
        for _ in 0..800 {
            if scheduler.run_script("asyncDone").unwrap() == JsValue::Bool(true) {
                done = true;
                break;
            }
            scheduler.pump_now(false).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(done, "pending async XHR lost its reply behind a synchronous XHR");
    }
}
