//! IPC lane: BroadcastChannel, MessageChannel/MessagePort and Window.postMessage.
//!
//! This is the lane-owned surface while dom-bindings installs the shared
//! `MessageEvent` class and makes the ports real `EventTarget`s. Message
//! payloads are structured-clone graphs encoded by the versioned history wire
//! codec; delivery crosses documents through a thread-local browser router and
//! each document drains its own inbox at the scheduler checkpoint.
use super::*;
use crate::navigator::state_wire;
use amazing_process::messaging::{
    MessageRouter, MessagingError, ChannelKey, ClientId, PortId, MAX_MESSAGE_BYTES,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::{Rc, Weak};

pub(crate) const MESSAGING_BRAND_BASE: u64 = 200;
pub(crate) const KIND_PORT: u64 = 0;
pub(crate) const KIND_CHANNEL: u64 = 1;
pub(crate) const KIND_MESSAGE_EVENT: u64 = 2;
pub(crate) const KIND_MESSAGE_CHANNEL: u64 = 3;

const MAX_INBOX: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) enum MessagingOp {
    MessageEventConstructor,
    MessageChannelConstructor,
    BroadcastChannelConstructor,
    ChannelPort1,
    ChannelPort2,
    ChannelName,
    PortPost,
    PortStart,
    PortClose,
    AddEventListener,
    RemoveEventListener,
    DispatchEvent,
    OnMessageGet,
    OnMessageSet,
    OnMessageErrorGet,
    OnMessageErrorSet,
    WindowPostMessage,
}

#[derive(Clone)]
pub(crate) struct Listener {
    kind: String,
    callback: JsValue,
    once: bool,
}

enum MsgObject {
    Port {
        id: PortId,
        started: bool,
        closed: bool,
        handlers: HashMap<String, JsValue>,
        listeners: Vec<Listener>,
    },
    Channel {
        name: String,
        router_client: u64,
        closed: bool,
        handlers: HashMap<String, JsValue>,
        listeners: Vec<Listener>,
    },
}

#[derive(Clone)]
enum DeliveryTarget {
    Port(PortId),
    ChannelSlot(usize),
    Window,
}

pub(crate) struct QueuedMessage {
    target: DeliveryTarget,
    payload: Vec<u8>,
    origin: String,
    ports: Vec<PortId>,
}

pub(crate) struct MessagingRealm {
    client: ClientId,
    objects: Vec<MsgObject>,
    ports: HashMap<PortId, usize>,
    classes: HashMap<u64, (JsValue, JsValue)>,
    inbox: VecDeque<QueuedMessage>,
    globals: HashMap<String, JsValue>,
}

#[derive(Default)]
struct BrowserMessaging {
    router: MessageRouter,
    next_client: ClientId,
    next_channel: u64,
    realms: BTreeMap<ClientId, Weak<RefCell<BindingState>>>,
    channels: BTreeMap<u64, (ClientId, usize)>,
}

thread_local! {
    static BROWSER: RefCell<BrowserMessaging> = RefCell::new(BrowserMessaging::default());
}

fn messaging_error(error: MessagingError) -> Throw {
    Throw::type_error(format!("messaging: {error:?}"))
}

impl RuntimeBindings {
    /// Drain queued cross-document deliveries for this realm. Called from the
    /// scheduler checkpoint next to `schedule_storage_events`.
    pub(crate) fn schedule_messages(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost {
            js,
            state: self.state.clone(),
        };
        adapter.messaging_drain()
    }
}

#[cfg(test)]
impl RuntimeBindings {
    /// Test hook: enqueue a raw wire payload as if a peer had sent it.
    pub(crate) fn queue_raw_message(&self, target: DeliveryTarget, payload: Vec<u8>) {
        let mut state = self.state.borrow_mut();
        if let Some(realm) = state.messaging.as_mut() {
            realm.inbox.push_back(QueuedMessage {
                target,
                payload,
                origin: String::new(),
                ports: Vec::new(),
            });
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn messaging_ensure(&mut self) -> Result<(), Throw> {
        if self.state.borrow().messaging.is_some() {
            return Ok(());
        }
        let client = BROWSER.with(|browser| {
            let mut browser = browser.borrow_mut();
            let client = browser.next_client.max(1);
            browser.next_client = client + 1;
            client
        });
        BROWSER.with(|browser| {
            browser
                .borrow_mut()
                .realms
                .insert(client, Rc::downgrade(&self.state));
        });
        self.state.borrow_mut().messaging = Some(MessagingRealm {
            client,
            objects: Vec::new(),
            ports: HashMap::new(),
            classes: HashMap::new(),
            inbox: VecDeque::new(),
            globals: HashMap::new(),
        });
        Ok(())
    }

    fn messaging_client(&self) -> ClientId {
        self.state.borrow().messaging.as_ref().map_or(0, |r| r.client)
    }

    fn messaging_key(&self, name: &str) -> ChannelKey {
        let url = self.state.borrow().document_url.clone().unwrap_or_default();
        let (origin, site) = match amazing_fetch::HttpUrl::parse(&url) {
            Ok(parsed) => {
                let origin = parsed.origin().serialize();
                let site = match parsed
                    .origin()
                    .site(&amazing_fetch::psl::PublicSuffixList)
                {
                    amazing_fetch::Site::Tuple { scheme, domain } => format!("{scheme}://{domain}"),
                    amazing_fetch::Site::Opaque(_) => origin.clone(),
                };
                (origin, site)
            }
            Err(_) => {
                let origin = if url.is_empty() {
                    "null".to_owned()
                } else {
                    url
                };
                (origin.clone(), origin)
            }
        };
        ChannelKey::new(origin, site, name)
    }

    fn messaging_origin(&self) -> String {
        let url = self.state.borrow().document_url.clone().unwrap_or_default();
        amazing_fetch::HttpUrl::parse(&url)
            .map(|parsed| parsed.origin().serialize())
            .unwrap_or(url)
    }

    fn messaging_set_class(&self, kind: u64, prototype: JsValue, constructor: JsValue) {
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            realm.classes.insert(kind, (prototype, constructor));
        }
    }

    fn messaging_class(&self, kind: u64) -> Option<JsValue> {
        self.state
            .borrow()
            .messaging
            .as_ref()
            .and_then(|realm| realm.classes.get(&kind))
            .map(|pair| pair.0.clone())
    }

    fn messaging_allocate(&mut self, kind: u64, object: MsgObject) -> Result<JsValue, Throw> {
        self.messaging_ensure()?;
        let prototype = self
            .messaging_class(kind)
            .unwrap_or_else(|| self.js.binding_object_prototype());
        let slot = {
            let mut state = self.state.borrow_mut();
            let realm = state.messaging.as_mut().unwrap();
            let slot = realm.objects.len();
            realm.objects.push(object);
            if let MsgObject::Port { id, .. } = &realm.objects[slot] {
                realm.ports.insert(*id, slot);
            }
            slot
        };
        let arena = self.state.borrow().arena;
        let value = self.js.binding_wrap_host(
            HostHandle {
                arena,
                slot: slot as u64,
                generation: 1,
                brand: MESSAGING_BRAND_BASE + kind,
            },
            &prototype,
        )?;
        self.state
            .borrow_mut()
            .messaging
            .as_mut()
            .unwrap()
            .globals
            .insert(format!("{kind}:{slot}"), value.clone());
        Ok(value)
    }

    fn messaging_receiver(&self, value: &JsValue, kind: u64) -> Result<usize, Throw> {
        let handle = self
            .js
            .binding_host_handle(value, MESSAGING_BRAND_BASE + kind)
            .ok_or_else(|| Throw::type_error("Incompatible messaging receiver"))?;
        if handle.arena != self.state.borrow().arena || handle.generation != 1 {
            return Err(Throw::type_error("Stale messaging receiver"));
        }
        let slot = handle.slot as usize;
        if self
            .state
            .borrow()
            .messaging
            .as_ref()
            .is_none_or(|realm| slot >= realm.objects.len())
        {
            return Err(Throw::type_error("Invalid messaging slot"));
        }
        Ok(slot)
    }

    /// Split a transfer list into MessagePorts the router must move and the
    /// remaining transferables that structured clone detaches.
    fn messaging_transfer(
        &mut self,
        transfer: Option<&JsValue>,
    ) -> Result<(Vec<JsValue>, Vec<PortId>), Throw> {
        let Some(transfer) = transfer.cloned() else {
            return Ok((Vec::new(), Vec::new()));
        };
        if self.js.binding_is_nullish(&transfer) {
            return Ok((Vec::new(), Vec::new()));
        }
        // WebIDL `sequence<object>` conversion: only a real sequence is
        // accepted. A primitive or a plain array-like object (the pre-WebIDL
        // postMessage convention) is a TypeError, not an empty transfer list.
        if !self.messaging_is_array(&transfer)? {
            return Err(Throw::type_error("transfer must be a sequence of objects"));
        }
        let length = match self.js.binding_get_property(&transfer, "length")? {
            JsValue::Num(n) if n.is_finite() && n > 0.0 => n as usize,
            _ => 0,
        };
        let mut transferables = Vec::new();
        let mut ports = Vec::new();
        for index in 0..length.min(1024) {
            let value = self.js.binding_get_property(&transfer, &index.to_string())?;
            if let Some(port) = self.messaging_port_id(&value) {
                ports.push(port);
            } else {
                transferables.push(value);
            }
        }
        Ok((transferables, ports))
    }

    /// `Array.isArray(value)`, used for the WebIDL `sequence<object>` transfer
    /// conversion. A full iterable check needs an exposed `@@iterator`; this
    /// accepts every actual array (including subclasses) and rejects the
    /// array-like objects the old postMessage convention allowed.
    fn messaging_is_array(&mut self, value: &JsValue) -> Result<bool, Throw> {
        if !matches!(value, JsValue::Object(_)) {
            return Ok(false);
        }
        let global = self.js.binding_global_object();
        let array = self.js.binding_get_property(&global, "Array")?;
        let is_array = self.js.binding_get_property(&array, "isArray")?;
        if !self.js.binding_is_callable(&is_array) {
            return Ok(false);
        }
        let result = self.js.binding_call(&is_array, array, vec![value.clone()])?;
        Ok(matches!(result, JsValue::Bool(true)))
    }

    fn messaging_port_id(&self, value: &JsValue) -> Option<PortId> {
        let handle = self
            .js
            .binding_host_handle(value, MESSAGING_BRAND_BASE + KIND_PORT)?;
        let slot = handle.slot as usize;
        let state = self.state.borrow();
        let realm = state.messaging.as_ref()?;
        match realm.objects.get(slot) {
            Some(MsgObject::Port { id, .. }) => Some(*id),
            _ => None,
        }
    }

    fn messaging_serialize(
        &mut self,
        data: &JsValue,
        transfer: Option<&JsValue>,
    ) -> Result<(Vec<u8>, Vec<PortId>), Throw> {
        let (transferables, ports) = self.messaging_transfer(transfer)?;
        let serialized = self
            .js
            .structured_serialize_with_transfer(data, &transferables)?;
        let payload = state_wire::encode(&serialized).map_err(Throw::type_error)?;
        if payload.len() > MAX_MESSAGE_BYTES {
            return Err(Throw::type_error("message too large"));
        }
        Ok((payload, ports))
    }

    fn messaging_queue(
        &mut self,
        client: ClientId,
        target: DeliveryTarget,
        payload: Vec<u8>,
        ports: Vec<PortId>,
    ) {
        let origin = self.messaging_origin();
        let realm = BROWSER.with(|browser| {
            browser
                .borrow()
                .realms
                .get(&client)
                .and_then(Weak::upgrade)
        });
        if let Some(realm) = realm {
            if let Some(messaging) = realm.borrow_mut().messaging.as_mut() {
                if messaging.inbox.len() < MAX_INBOX {
                    messaging.inbox.push_back(QueuedMessage {
                        target,
                        payload,
                        origin,
                        ports,
                    });
                }
            }
        }
    }

    fn messaging_peer_owner(&self, port: PortId) -> Option<ClientId> {
        BROWSER.with(|browser| {
            let _ = port;
            browser.borrow().router.owner_of(port)
        })
    }

    fn messaging_drain(&mut self) -> Result<(), Throw> {
        // Deliver only the messages already queued when the checkpoint began.
        // A handler that posts again queues the delivery for the next
        // checkpoint rather than extending this one, so a self-posting loop
        // (for example `onmessage` replying to itself) cannot starve timers or
        // the rest of the event loop.
        let ready = {
            let state = self.state.borrow();
            state
                .messaging
                .as_ref()
                .map_or(0, |realm| realm.inbox.len())
        };
        for _ in 0..ready {
            let message = {
                let mut state = self.state.borrow_mut();
                match state.messaging.as_mut() {
                    Some(realm) => realm.inbox.pop_front(),
                    None => break,
                }
            };
            let Some(message) = message else {
                break;
            };
            self.messaging_deliver(message)?;
        }
        Ok(())
    }

    fn messaging_deliver(&mut self, message: QueuedMessage) -> Result<(), Throw> {
        let origin = message.origin.clone();
        let target = message.target.clone();
        let data = match state_wire::decode(&message.payload) {
            Ok(serialized) => self.js.structured_deserialize(&serialized).ok(),
            Err(_) => None,
        };
        match data {
            Some(data) => {
                let ports = self.messaging_wrap_ports(&target, &message.ports);
                self.messaging_dispatch(target, data, &origin, ports)
            }
            // A malformed or unserializable payload must not abort the
            // event-loop turn: a peer process can send arbitrary bytes. Per
            // HTML, fire `messageerror` at the target and keep going.
            None => self.messaging_dispatch_error(target, &origin),
        }
    }

    fn messaging_dispatch(
        &mut self,
        target: DeliveryTarget,
        data: JsValue,
        origin: &str,
        ports: JsValue,
    ) -> Result<(), Throw> {
        match target {
            DeliveryTarget::Port(id) => {
                let event = self.messaging_make_event("message", data, origin, ports)?;
                self.messaging_dispatch_port("message", id, event)
            }
            DeliveryTarget::ChannelSlot(slot) => {
                let event = self.messaging_make_event("message", data, origin, ports)?;
                self.messaging_dispatch_channel("message", slot, event)
            }
            DeliveryTarget::Window => {
                self.messaging_dispatch_window("message", data, origin, ports)
            }
        }
    }

    fn messaging_dispatch_error(
        &mut self,
        target: DeliveryTarget,
        origin: &str,
    ) -> Result<(), Throw> {
        let ports = self.js.binding_create_array(Vec::new())?;
        match target {
            DeliveryTarget::Port(id) => {
                let event =
                    self.messaging_make_event("messageerror", JsValue::Null, origin, ports)?;
                self.messaging_dispatch_port("messageerror", id, event)
            }
            DeliveryTarget::ChannelSlot(slot) => {
                let event =
                    self.messaging_make_event("messageerror", JsValue::Null, origin, ports)?;
                self.messaging_dispatch_channel("messageerror", slot, event)
            }
            DeliveryTarget::Window => {
                self.messaging_dispatch_window("messageerror", JsValue::Null, origin, ports)
            }
        }
    }

    fn messaging_wrap_ports(&mut self, _target: &DeliveryTarget, ports: &[PortId]) -> JsValue {
        if ports.is_empty() {
            return JsValue::Undefined;
        }
        let mut values = Vec::new();
        for id in ports {
            let slot = {
                let state = self.state.borrow();
                state
                    .messaging
                    .as_ref()
                    .and_then(|realm| realm.ports.get(id))
                    .copied()
            };
            let Some(slot) = slot else { continue };
            let wrapper = {
                let state = self.state.borrow();
                state
                    .messaging
                    .as_ref()
                    .and_then(|realm| realm.globals.get(&format!("{KIND_PORT}:{slot}")).cloned())
            };
            if let Some(wrapper) = wrapper {
                values.push(wrapper);
            }
        }
        self.js
            .binding_create_array(values)
            .unwrap_or(JsValue::Undefined)
    }

    fn messaging_make_event(
        &mut self,
        kind: &str,
        data: JsValue,
        origin: &str,
        ports: JsValue,
    ) -> Result<JsValue, Throw> {
        let prototype = self
            .messaging_class(KIND_MESSAGE_EVENT)
            .unwrap_or_else(|| self.js.binding_object_prototype());
        let event = self.js.binding_create_object(Some(&prototype))?;
        let flags = BindingPropertyFlags::METHOD;
        self.js
            .binding_define_data_property(&event, "type", JsValue::str(kind), flags)?;
        self.js
            .binding_define_data_property(&event, "data", data, flags)?;
        self.js
            .binding_define_data_property(&event, "origin", JsValue::str(origin), flags)?;
        self.js
            .binding_define_data_property(&event, "lastEventId", JsValue::str(""), flags)?;
        self.js
            .binding_define_data_property(&event, "source", JsValue::Null, flags)?;
        self.js
            .binding_define_data_property(&event, "ports", ports, flags)?;
        self.js
            .binding_define_data_property(&event, "bubbles", JsValue::Bool(false), flags)?;
        self.js
            .binding_define_data_property(&event, "cancelable", JsValue::Bool(false), flags)?;
        self.js
            .binding_define_data_property(&event, "isTrusted", JsValue::Bool(true), flags)?;
        Ok(event)
    }

    fn messaging_dispatch_port(
        &mut self,
        kind: &str,
        id: PortId,
        event: JsValue,
    ) -> Result<(), Throw> {
        let (slot, this) = {
            let state = self.state.borrow();
            let Some(realm) = state.messaging.as_ref() else {
                return Ok(());
            };
            let Some(slot) = realm.ports.get(&id).copied() else {
                return Ok(());
            };
            let this = realm.globals.get(&format!("{KIND_PORT}:{slot}")).cloned();
            (slot, this)
        };
        let Some(this) = this else { return Ok(()) };
        self.messaging_fire(slot, kind, this, event)
    }

    fn messaging_dispatch_channel(
        &mut self,
        kind: &str,
        slot: usize,
        event: JsValue,
    ) -> Result<(), Throw> {
        let this = {
            let state = self.state.borrow();
            let Some(realm) = state.messaging.as_ref() else {
                return Ok(());
            };
            realm.globals.get(&format!("{KIND_CHANNEL}:{slot}")).cloned()
        };
        let Some(this) = this else { return Ok(()) };
        self.messaging_fire(slot, kind, this, event)
    }

    fn messaging_dispatch_window(
        &mut self,
        kind: &str,
        data: JsValue,
        origin: &str,
        ports: JsValue,
    ) -> Result<(), Throw> {
        let window = self.js.binding_global_object();
        let flags = BindingPropertyFlags::METHOD;
        let event = invoke(
            self,
            Operation::EventConstructor,
            JsValue::Undefined,
            &[JsValue::str(kind)],
        )?;
        // `MessageEvent.ports` is always a sequence, never `undefined`.
        let ports = if matches!(ports, JsValue::Undefined) {
            self.js.binding_create_array(Vec::new())?
        } else {
            ports
        };
        self.js
            .binding_define_data_property(&event, "data", data, flags)?;
        self.js.binding_define_data_property(
            &event,
            "origin",
            JsValue::str(origin),
            flags,
        )?;
        self.js
            .binding_define_data_property(&event, "ports", ports, flags)?;
        invoke(self, Operation::DispatchEvent, window.clone(), &[event.clone()])?;
        let handler = self.messaging_window_handler(kind);
        if self.js.binding_is_callable(&handler) {
            let _ = self.js.binding_call(&handler, window, vec![event]);
        }
        Ok(())
    }

    fn messaging_fire(
        &mut self,
        slot: usize,
        kind: &str,
        this: JsValue,
        event: JsValue,
    ) -> Result<(), Throw> {
        let (listeners, handler) = {
            let state = self.state.borrow();
            let Some(realm) = state.messaging.as_ref() else {
                return Ok(());
            };
            let Some(object) = realm.objects.get(slot) else {
                return Ok(());
            };
            let (listeners, handlers) = match object {
                MsgObject::Port {
                    listeners, handlers, ..
                }
                | MsgObject::Channel {
                    listeners, handlers, ..
                } => (listeners.clone(), handlers.clone()),
            };
            (listeners, handlers.get(kind).cloned())
        };
        let mut remaining = Vec::new();
        for listener in listeners {
            if listener.kind == kind && self.js.binding_is_callable(&listener.callback) {
                let _ = self
                    .js
                    .binding_call(&listener.callback, this.clone(), vec![event.clone()]);
            }
            if !listener.once {
                remaining.push(listener);
            }
        }
        if !remaining.is_empty() {
            let mut state = self.state.borrow_mut();
            if let Some(realm) = state.messaging.as_mut() {
                if let Some(object) = realm.objects.get_mut(slot) {
                    let target = match object {
                        MsgObject::Port { listeners, .. } | MsgObject::Channel { listeners, .. } => {
                            listeners
                        }
                    };
                    *target = remaining;
                }
            }
        }
        if let Some(handler) = handler {
            if self.js.binding_is_callable(&handler) {
                let _ = self.js.binding_call(&handler, this, vec![event]);
            }
        }
        Ok(())
    }

    fn messaging_handler(&self, slot: usize, key: &str) -> JsValue {
        self.state
            .borrow()
            .messaging
            .as_ref()
            .and_then(|realm| realm.objects.get(slot))
            .and_then(|object| match object {
                MsgObject::Port { handlers, .. } | MsgObject::Channel { handlers, .. } => {
                    handlers.get(key).cloned()
                }
            })
            .unwrap_or(JsValue::Null)
    }

    fn messaging_window_handler(&self, key: &str) -> JsValue {
        self.state
            .borrow()
            .messaging
            .as_ref()
            .and_then(|realm| realm.globals.get(&format!("window:{key}")).cloned())
            .unwrap_or(JsValue::Null)
    }

    fn messaging_set_handler(&self, slot: usize, key: &str, value: JsValue) {
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            if let Some(object) = realm.objects.get_mut(slot) {
                let handlers = match object {
                    MsgObject::Port { handlers, .. } | MsgObject::Channel { handlers, .. } => handlers,
                };
                if value.is_undefined() {
                    handlers.remove(key);
                } else {
                    handlers.insert(key.to_owned(), value);
                }
            }
        }
    }

    fn messaging_store_global(&self, key: &str, value: JsValue) {
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            realm.globals.insert(key.to_owned(), value);
        }
    }

    fn messaging_add_listener(
        &mut self,
        slot: usize,
        args: &[JsValue],
    ) -> Result<JsValue, Throw> {
        let kind = self
            .js
            .binding_to_dom_string(args.first().unwrap_or(&JsValue::Undefined))
            .map(|units| String::from_utf16_lossy(&units))
            .unwrap_or_default();
        let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        let once = args
            .get(2)
            .map(|options| {
                if self.js.binding_to_boolean(options) {
                    true
                } else {
                    self.js
                        .binding_get_property(options, "once")
                        .map(|value| self.js.binding_to_boolean(&value))
                        .unwrap_or(false)
                }
            })
            .unwrap_or(false);
        if !self.js.binding_is_callable(&callback) {
            return Ok(JsValue::Undefined);
        }
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            if let Some(object) = realm.objects.get_mut(slot) {
                let listeners = match object {
                    MsgObject::Port { listeners, .. } | MsgObject::Channel { listeners, .. } => {
                        listeners
                    }
                };
                listeners.push(Listener {
                    kind,
                    callback,
                    once,
                });
            }
        }
        Ok(JsValue::Undefined)
    }

    fn messaging_remove_listener(&mut self, slot: usize, args: &[JsValue]) -> JsValue {
        let kind = self
            .js
            .binding_to_dom_string(args.first().unwrap_or(&JsValue::Undefined))
            .map(|units| String::from_utf16_lossy(&units))
            .unwrap_or_default();
        let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            if let Some(object) = realm.objects.get_mut(slot) {
                let listeners = match object {
                    MsgObject::Port { listeners, .. } | MsgObject::Channel { listeners, .. } => {
                        listeners
                    }
                };
                listeners.retain(|listener| {
                    !(listener.kind == kind && self.js.binding_same_value(&listener.callback, &callback))
                });
            }
        }
        JsValue::Undefined
    }

    fn messaging_dispatch_event(&mut self, slot: usize, args: &[JsValue]) -> Result<JsValue, Throw> {
        let event = args.first().cloned().unwrap_or(JsValue::Undefined);
        let kind = self
            .js
            .binding_get_property(&event, "type")
            .map(|value| self.js.binding_to_dom_string(&value).ok())
            .ok()
            .flatten()
            .map(|units| String::from_utf16_lossy(&units))
            .unwrap_or_default();
        let this = {
            let state = self.state.borrow();
            let kind_brand = if state
                .messaging
                .as_ref()
                .and_then(|realm| realm.objects.get(slot))
                .is_some_and(|object| matches!(object, MsgObject::Port { .. }))
            {
                KIND_PORT
            } else {
                KIND_CHANNEL
            };
            state
                .messaging
                .as_ref()
                .and_then(|realm| realm.globals.get(&format!("{kind_brand}:{slot}")).cloned())
        };
        if let Some(this) = this {
            self.messaging_fire(slot, &kind, this, event)?;
        }
        Ok(JsValue::Bool(true))
    }

    /// Native entry point for every messaging op.
    pub(super) fn messaging_invoke(
        &mut self,
        op: MessagingOp,
        this: JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, Throw> {
        match op {
            MessagingOp::MessageEventConstructor => {
                let prototype = self
                    .messaging_class(KIND_MESSAGE_EVENT)
                    .unwrap_or_else(|| self.js.binding_object_prototype());
                let event = self.js.binding_create_object(Some(&prototype))?;
                let init = args.first().cloned().unwrap_or(JsValue::Undefined);
                let flags = BindingPropertyFlags::METHOD;
                let read = |adapter: &mut Self, name: &str| -> Result<JsValue, Throw> {
                    if adapter.js.binding_is_nullish(&init) {
                        return Ok(JsValue::Undefined);
                    }
                    adapter.js.binding_get_property(&init, name)
                };
                let data = read(self, "data")?;
                let origin = read(self, "origin")?;
                let last_event_id = read(self, "lastEventId")?;
                let source = read(self, "source")?;
                let ports = read(self, "ports")?;
                self.js
                    .binding_define_data_property(&event, "type", JsValue::str("message"), flags)?;
                self.js
                    .binding_define_data_property(&event, "data", data, flags)?;
                self.js.binding_define_data_property(
                    &event,
                    "origin",
                    if origin.is_undefined() {
                        JsValue::str("")
                    } else {
                        origin
                    },
                    flags,
                )?;
                self.js.binding_define_data_property(
                    &event,
                    "lastEventId",
                    if last_event_id.is_undefined() {
                        JsValue::str("")
                    } else {
                        last_event_id
                    },
                    flags,
                )?;
                self.js.binding_define_data_property(
                    &event,
                    "source",
                    if source.is_undefined() {
                        JsValue::Null
                    } else {
                        source
                    },
                    flags,
                )?;
                let ports_value = if ports.is_undefined() {
                    self.js.binding_create_array(Vec::new())?
                } else {
                    ports
                };
                self.js
                    .binding_define_data_property(&event, "ports", ports_value, flags)?;
                self.js
                    .binding_define_data_property(&event, "isTrusted", JsValue::Bool(false), flags)?;
                Ok(event)
            }
            MessagingOp::MessageChannelConstructor => {
                let client = self.messaging_client();
                let (a, b) = BROWSER.with(|browser| {
                    browser
                        .borrow_mut()
                        .router
                        .create_pair(client)
                        .map_err(messaging_error)
                })?;
                let port_a = self.messaging_allocate(
                    KIND_PORT,
                    MsgObject::Port {
                        id: a,
                        started: false,
                        closed: false,
                        handlers: HashMap::new(),
                        listeners: Vec::new(),
                    },
                )?;
                let port_b = self.messaging_allocate(
                    KIND_PORT,
                    MsgObject::Port {
                        id: b,
                        started: false,
                        closed: false,
                        handlers: HashMap::new(),
                        listeners: Vec::new(),
                    },
                )?;
                let prototype = self
                    .messaging_class(KIND_MESSAGE_CHANNEL)
                    .unwrap_or_else(|| self.js.binding_object_prototype());
                let channel = self.js.binding_create_object(Some(&prototype))?;
                let flags = BindingPropertyFlags::METHOD;
                self.js
                    .binding_define_data_property(&channel, "port1", port_a, flags)?;
                self.js
                    .binding_define_data_property(&channel, "port2", port_b, flags)?;
                Ok(channel)
            }
            MessagingOp::BroadcastChannelConstructor => {
                let name = args
                    .first()
                    .map(|value| self.js.binding_to_dom_string(value))
                    .transpose()?
                    .map(|units| String::from_utf16_lossy(&units))
                    .unwrap_or_default();
                let key = self.messaging_key(&name);
                let client = self.messaging_client();
                let value = self.messaging_allocate(
                    KIND_CHANNEL,
                    MsgObject::Channel {
                        name: name.clone(),
                        router_client: 0,
                        closed: false,
                        handlers: HashMap::new(),
                        listeners: Vec::new(),
                    },
                )?;
                let slot = self.messaging_receiver(&value, KIND_CHANNEL)?;
                let channel_client = BROWSER.with(|browser| {
                    let mut browser = browser.borrow_mut();
                    let client = browser.next_channel.max(1);
                    browser.next_channel = client + 1;
                    client
                });
                if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
                    if let Some(MsgObject::Channel { router_client, .. }) =
                        realm.objects.get_mut(slot)
                    {
                        *router_client = channel_client;
                    }
                }
                BROWSER.with(|browser| {
                    browser
                        .borrow_mut()
                        .channels
                        .insert(channel_client, (client, slot));
                });
                BROWSER.with(|browser| {
                    browser
                        .borrow_mut()
                        .router
                        .subscribe(key, channel_client)
                        .map_err(messaging_error)
                })?;
                Ok(value)
            }
            MessagingOp::ChannelPort1 | MessagingOp::ChannelPort2 => {
                let name = if matches!(op, MessagingOp::ChannelPort1) {
                    "port1"
                } else {
                    "port2"
                };
                self.js.binding_get_property(&this, name)
            }
            MessagingOp::ChannelName => {
                let slot = self.messaging_receiver(&this, KIND_CHANNEL)?;
                let state = self.state.borrow();
                let name = state
                    .messaging
                    .as_ref()
                    .and_then(|realm| realm.objects.get(slot))
                    .map(|object| match object {
                        MsgObject::Channel { name, .. } => name.clone(),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                Ok(JsValue::str(&name))
            }
            MessagingOp::PortPost => {
                if self.messaging_receiver(&this, KIND_PORT).is_ok() {
                    return self.messaging_port_post(&this, args);
                }
                let _ = self.messaging_receiver(&this, KIND_CHANNEL)?;
                self.messaging_channel_post(&this, args)
            }
            MessagingOp::PortStart => {
                let slot = self.messaging_receiver(&this, KIND_PORT)?;
                self.messaging_start_port(slot)?;
                Ok(JsValue::Undefined)
            }
            MessagingOp::PortClose => {
                if let Ok(slot) = self.messaging_receiver(&this, KIND_PORT) {
                    self.messaging_close_port(slot);
                } else if let Ok(slot) = self.messaging_receiver(&this, KIND_CHANNEL) {
                    self.messaging_close_channel(slot);
                }
                Ok(JsValue::Undefined)
            }
            MessagingOp::AddEventListener => {
                let slot = self
                    .messaging_receiver(&this, KIND_PORT)
                    .or_else(|_| self.messaging_receiver(&this, KIND_CHANNEL))?;
                self.messaging_add_listener(slot, args)
            }
            MessagingOp::RemoveEventListener => {
                let slot = self
                    .messaging_receiver(&this, KIND_PORT)
                    .or_else(|_| self.messaging_receiver(&this, KIND_CHANNEL))?;
                Ok(self.messaging_remove_listener(slot, args))
            }
            MessagingOp::DispatchEvent => {
                let slot = self
                    .messaging_receiver(&this, KIND_PORT)
                    .or_else(|_| self.messaging_receiver(&this, KIND_CHANNEL))?;
                self.messaging_dispatch_event(slot, args)
            }
            MessagingOp::OnMessageGet | MessagingOp::OnMessageErrorGet => {
                let key = if matches!(op, MessagingOp::OnMessageGet) {
                    "message"
                } else {
                    "messageerror"
                };
                if let Ok(slot) = self
                    .messaging_receiver(&this, KIND_PORT)
                    .or_else(|_| self.messaging_receiver(&this, KIND_CHANNEL))
                {
                    return Ok(self.messaging_handler(slot, key));
                }
                Ok(self.messaging_window_handler(key))
            }
            MessagingOp::OnMessageSet | MessagingOp::OnMessageErrorSet => {
                let key = if matches!(op, MessagingOp::OnMessageSet) {
                    "message"
                } else {
                    "messageerror"
                };
                let value = args.first().cloned().unwrap_or(JsValue::Undefined);
                if let Ok(slot) = self
                    .messaging_receiver(&this, KIND_PORT)
                    .or_else(|_| self.messaging_receiver(&this, KIND_CHANNEL))
                {
                    self.messaging_set_handler(slot, key, value);
                    // Setting `onmessage` on a port implicitly starts it.
                    if key == "message"
                        && self.messaging_receiver(&this, KIND_PORT).is_ok()
                    {
                        self.messaging_start_port(slot)?;
                    }
                } else {
                    self.messaging_store_global(&format!("window:{key}"), value);
                }
                Ok(JsValue::Undefined)
            }
            MessagingOp::WindowPostMessage => self.messaging_window_post(args),
        }
    }

    fn messaging_object_port_id(&self, slot: usize) -> Option<PortId> {
        let state = self.state.borrow();
        let realm = state.messaging.as_ref()?;
        match realm.objects.get(slot) {
            Some(MsgObject::Port { id, .. }) => Some(*id),
            _ => None,
        }
    }

    fn messaging_channel_post(
        &mut self,
        this: &JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, Throw> {
        let slot = self.messaging_receiver(this, KIND_CHANNEL)?;
        let name = {
            let state = self.state.borrow();
            state
                .messaging
                .as_ref()
                .and_then(|realm| realm.objects.get(slot))
                .map(|object| match object {
                    MsgObject::Channel { name, .. } => name.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default()
        };
        let data = args.first().cloned().unwrap_or(JsValue::Undefined);
        let (payload, _ports) = self.messaging_serialize(&data, None)?;
        let key = self.messaging_key(&name);
        let router_client = {
            let state = self.state.borrow();
            state
                .messaging
                .as_ref()
                .and_then(|realm| realm.objects.get(slot))
                .map(|object| match object {
                    MsgObject::Channel { router_client, .. } => *router_client,
                    _ => 0,
                })
                .unwrap_or(0)
        };
        if router_client == 0 {
            return Ok(JsValue::Undefined);
        }
        let deliveries = BROWSER.with(|browser| {
            browser
                .borrow()
                .router
                .broadcast(&key, router_client, payload)
                .map_err(messaging_error)
        })?;
        for delivery in deliveries {
            let target = BROWSER.with(|browser| {
                browser
                    .borrow()
                    .channels
                    .get(&delivery.client)
                    .copied()
            });
            if let Some((doc_client, target_slot)) = target {
                self.messaging_queue(
                    doc_client,
                    DeliveryTarget::ChannelSlot(target_slot),
                    delivery.payload,
                    Vec::new(),
                );
            }
        }
        Ok(JsValue::Undefined)
    }

    fn messaging_port_post(
        &mut self,
        this: &JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, Throw> {        let slot = self.messaging_receiver(this, KIND_PORT)?;
        let id = self
            .messaging_object_port_id(slot)
            .ok_or_else(|| Throw::type_error("postMessage on a non-port"))?;
        let client = self.messaging_client();
        let data = args.first().cloned().unwrap_or(JsValue::Undefined);
        let (payload, transfer_ports) = self.messaging_serialize(&data, args.get(1))?;
        let (delivery, peer_owner) = BROWSER.with(|browser| {
            let mut browser = browser.borrow_mut();
            let peer_owner = browser.router.owner_of(id);
            let delivery = browser
                .router
                .post(id, client, payload.clone())
                .map_err(messaging_error)?;
            Ok::<_, Throw>((delivery, peer_owner))
        })?;
        if let Some(target) = peer_owner {
            for port in transfer_ports {
                BROWSER.with(|browser| {
                    let _ = browser.borrow_mut().router.transfer(port, client, target);
                });
            }
        }
        match delivery {
            Some(delivery) => {
                self.messaging_queue(
                    delivery.client,
                    DeliveryTarget::Port(delivery.port),
                    delivery.payload,
                    Vec::new(),
                );
            }
            None => {
                // Queued at the peer: nothing to deliver until start().
            }
        }
        Ok(JsValue::Undefined)
    }

    fn messaging_start_port(&mut self, slot: usize) -> Result<(), Throw> {
        let id = self
            .messaging_object_port_id(slot)
            .ok_or_else(|| Throw::type_error("start on a non-port"))?;
        let client = self.messaging_client();
        let deliveries = BROWSER.with(|browser| {
            browser
                .borrow_mut()
                .router
                .start(id, client)
                .map_err(messaging_error)
        })?;
        for delivery in deliveries {
            self.messaging_queue(
                delivery.client,
                DeliveryTarget::Port(delivery.port),
                delivery.payload,
                Vec::new(),
            );
        }
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            if let Some(MsgObject::Port { started, .. }) = realm.objects.get_mut(slot) {
                *started = true;
            }
        }
        Ok(())
    }

    fn messaging_close_port(&mut self, slot: usize) {
        let id = self.messaging_object_port_id(slot);
        let client = self.messaging_client();
        if let Some(id) = id {
            BROWSER.with(|browser| {
                let _ = browser.borrow_mut().router.close(id, client);
            });
        }
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            if let Some(object) = realm.objects.get_mut(slot) {
                if let MsgObject::Port { closed, .. } = object {
                    *closed = true;
                }
            }
        }
    }

    fn messaging_close_channel(&mut self, slot: usize) {
        let mut name = String::new();
        let mut router_client: u64 = 0;
        {
            let state = self.state.borrow();
            if let Some(realm) = state.messaging.as_ref() {
                if let Some(MsgObject::Channel {
                    name: channel_name,
                    router_client: channel_client,
                    ..
                }) = realm.objects.get(slot)
                {
                    name = channel_name.clone();
                    router_client = *channel_client;
                }
            }
        }
        let key = self.messaging_key(&name);
        if router_client != 0 {
            BROWSER.with(|browser| {
                let mut browser = browser.borrow_mut();
                browser.router.unsubscribe(&key, router_client);
                browser.channels.remove(&router_client);
            });
        }
        if let Some(realm) = self.state.borrow_mut().messaging.as_mut() {
            if let Some(object) = realm.objects.get_mut(slot) {
                if let MsgObject::Channel { closed, .. } = object {
                    *closed = true;
                }
            }
        }
    }

    fn messaging_window_post(&mut self, args: &[JsValue]) -> Result<JsValue, Throw> {
        if args.is_empty() {
            return Err(Throw::type_error(
                "postMessage requires a message argument",
            ));
        }
        let data = args[0].clone();
        let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        let (target_origin, transfer) =
            if matches!(options, JsValue::Object(_)) && !self.js.binding_is_nullish(&options) {
                let target = self.js.binding_get_property(&options, "targetOrigin")?;
                let transfer = self.js.binding_get_property(&options, "transfer")?;
                (target, transfer)
            } else {
                (
                    options,
                    args.get(2).cloned().unwrap_or(JsValue::Undefined),
                )
            };
        if matches!(transfer, JsValue::Null) {
            return Err(Throw::type_error("transfer must be a sequence, not null"));
        }
        if !self.js.binding_is_nullish(&target_origin) {
            let Some(units) = self.js.binding_string_units(&target_origin) else {
                return Err(Throw::type_error("targetOrigin must be a string"));
            };
            let origin = String::from_utf16_lossy(&units);
            if origin != "*" && origin != "/" {
                let parsed = amazing_fetch::HttpUrl::parse(&origin)
                    .map_err(|_| Throw::syntax_error("invalid target origin"))?;
                if parsed.origin().serialize() != self.messaging_origin() {
                    // A different origin never receives the message.
                    return Ok(JsValue::Undefined);
                }
            }
        }
        let (payload, transfer_ports) = self.messaging_serialize(&data, Some(&transfer))?;
        let client = self.messaging_client();
        self.messaging_queue(
            client,
            DeliveryTarget::Window,
            payload,
            transfer_ports,
        );
        Ok(JsValue::Undefined)
    }

    pub(super) fn install_messaging(&mut self) -> Result<(), Throw> {
        self.messaging_ensure()?;
        let global = self.js.binding_global_object();
        let object_proto = self.js.binding_object_prototype();
        let flags = BindingPropertyFlags::METHOD;

        // MessageEvent
        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        let constructor = self.native(
            "MessageEvent",
            0,
            NativeEntry::Messaging(MessagingOp::MessageEventConstructor),
            HostCallPolicy::ConstructOnly,
        );
        self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
        self.define_data_property(&global, "MessageEvent", constructor.clone(), true, false, true)?;
        self.messaging_set_class(KIND_MESSAGE_EVENT, prototype, constructor);

        // MessageChannel
        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        let constructor = self.native(
            "MessageChannel",
            0,
            NativeEntry::Messaging(MessagingOp::MessageChannelConstructor),
            HostCallPolicy::ConstructOnly,
        );
        self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
        for (name, op) in [
            ("port1", MessagingOp::ChannelPort1),
            ("port2", MessagingOp::ChannelPort2),
        ] {
            let getter = self.native(name, 0, NativeEntry::Messaging(op), HostCallPolicy::CallOnly);
            self.define_accessor_property(&prototype, name, Some(getter), None, true, true)?;
        }
        self.define_data_property(&global, "MessageChannel", constructor.clone(), true, false, true)?;
        self.messaging_set_class(KIND_MESSAGE_CHANNEL, prototype, constructor);

        // MessagePort (no public constructor)
        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        self.install_messaging_ops(&prototype)?;
        let port_constructor = self.native(
            "MessagePort",
            0,
            NativeEntry::IllegalConstructor,
            HostCallPolicy::ConstructOnly,
        );
        self.define_data_property(&port_constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", port_constructor.clone(), true, false, true)?;
        self.define_data_property(
            &global,
            "MessagePort",
            port_constructor.clone(),
            true,
            false,
            true,
        )?;
        self.messaging_set_class(KIND_PORT, prototype, port_constructor);

        // BroadcastChannel
        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        let constructor = self.native(
            "BroadcastChannel",
            1,
            NativeEntry::Messaging(MessagingOp::BroadcastChannelConstructor),
            HostCallPolicy::ConstructOnly,
        );
        self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
        let getter = self.native(
            "name",
            0,
            NativeEntry::Messaging(MessagingOp::ChannelName),
            HostCallPolicy::CallOnly,
        );
        self.define_accessor_property(&prototype, "name", Some(getter), None, true, true)?;
        self.install_messaging_ops(&prototype)?;
        self.define_data_property(
            &global,
            "BroadcastChannel",
            constructor.clone(),
            true,
            false,
            true,
        )?;
        self.messaging_set_class(KIND_CHANNEL, prototype, constructor);

        // window.postMessage and the two message handler attributes.
        let post = self.native(
            "postMessage",
            1,
            NativeEntry::Messaging(MessagingOp::WindowPostMessage),
            HostCallPolicy::CallOnly,
        );
        self.define_data_property(&global, "postMessage", post, true, false, true)?;
        for (name, get_op, set_op) in [
            (
                "onmessage",
                MessagingOp::OnMessageGet,
                MessagingOp::OnMessageSet,
            ),
            (
                "onmessageerror",
                MessagingOp::OnMessageErrorGet,
                MessagingOp::OnMessageErrorSet,
            ),
        ] {
            let getter = self.native(name, 0, NativeEntry::Messaging(get_op), HostCallPolicy::CallOnly);
            let setter = self.native(name, 1, NativeEntry::Messaging(set_op), HostCallPolicy::CallOnly);
            self.define_accessor_property(&global, name, Some(getter), Some(setter), true, true)?;
        }
        let _ = flags;
        Ok(())
    }

    fn install_messaging_ops(&mut self, prototype: &JsValue) -> Result<(), Throw> {
        for (name, length, op) in [
            ("postMessage", 2u8, MessagingOp::PortPost),
            ("start", 0, MessagingOp::PortStart),
            ("close", 0, MessagingOp::PortClose),
            ("addEventListener", 2, MessagingOp::AddEventListener),
            ("removeEventListener", 2, MessagingOp::RemoveEventListener),
            ("dispatchEvent", 1, MessagingOp::DispatchEvent),
        ] {
            let function = self.native(
                name,
                length,
                NativeEntry::Messaging(op),
                HostCallPolicy::CallOnly,
            );
            self.define_data_property(prototype, name, function, true, false, true)?;
        }
        for (name, get_op, set_op) in [
            (
                "onmessage",
                MessagingOp::OnMessageGet,
                MessagingOp::OnMessageSet,
            ),
            (
                "onmessageerror",
                MessagingOp::OnMessageErrorGet,
                MessagingOp::OnMessageErrorSet,
            ),
        ] {
            let getter = self.native(name, 0, NativeEntry::Messaging(get_op), HostCallPolicy::CallOnly);
            let setter = self.native(name, 1, NativeEntry::Messaging(set_op), HostCallPolicy::CallOnly);
            self.define_accessor_property(prototype, name, Some(getter), Some(setter), true, true)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler() -> crate::js_scheduler::DriverScheduler {
        let host = Rc::new(RefCell::new(crate::dom_host::BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap()
    }

    fn run(scheduler: &mut crate::js_scheduler::DriverScheduler, script: &str) {
        if let Err(error) = scheduler.run_script(script) {
            panic!("script error: {}", error.describe());
        }
    }

    #[test]
    fn entangled_ports_deliver_structured_clone_at_checkpoint() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var got = null;
                var mc = new MessageChannel();
                mc.port1.onmessage = function(e) { got = e.data; };
                mc.port1.start();
                mc.port2.postMessage({n: 41, a: [1, 2], s: "hi"});
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("got !== null && got.n === 41 && got.a[1] === 2 && got.s === 'hi'")
                .unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn window_post_message_reaches_the_onmessage_handler() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var got = null;
                onmessage = function(e) { got = e.data; };
                postMessage({x: 7}, "*");
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler.run_script("got !== null && got.x === 7").unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn broadcast_channel_is_named_and_skips_its_own_sender() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var bc = new BroadcastChannel("updates");
                var self_delivered = false;
                bc.addEventListener("message", function() { self_delivered = true; });
                bc.postMessage({k: 1});
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("bc.name === 'updates' && self_delivered === false")
                .unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn post_message_validates_target_origin_transfer_and_arity() {
        let mut scheduler = scheduler();
        scheduler.set_location_url("https://example.test/page.html");
        run(
            &mut scheduler,
            r#"
                var thrown = [];
                function attempt(fn) {
                    try { fn(); thrown.push("no"); } catch (e) { thrown.push(e.name); }
                }
                attempt(function() { postMessage({}, "not a url"); });
                attempt(function() { postMessage({}, "http://[bad"); });
                attempt(function() { postMessage({}, "/", null); });
                attempt(function() { postMessage(); });
            "#,
        );
        assert_eq!(
            scheduler.run_script("thrown.join(',')").unwrap(),
            JsValue::str("SyntaxError,SyntaxError,TypeError,TypeError")
        );
    }

    #[test]
    fn message_port_is_an_instance_and_onmessage_starts_it() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var mc = new MessageChannel();
                var got = null;
                var isPort = mc.port1 instanceof MessagePort;
                mc.port1.onmessage = function(e) { got = e.data; };
                mc.port2.postMessage(5);
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("isPort === true && got === 5")
                .unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn two_channels_in_one_document_receive_each_other() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var a = new BroadcastChannel("room");
                var b = new BroadcastChannel("room");
                var got = null;
                b.onmessage = function(e) { got = e.data; };
                a.postMessage({v: 9});
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("got !== null && got.v === 9")
                .unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn window_post_message_carries_transferred_ports() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var mc = new MessageChannel();
                var got = -1;
                var allPorts = false;
                onmessage = function(e) {
                    got = e.ports.length;
                    allPorts = e.ports[0] instanceof MessagePort
                        && e.ports[1] instanceof MessagePort;
                };
                postMessage("payload", "*", [mc.port1, mc.port2]);
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("got === 2 && allPorts === true")
                .unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn post_message_rejects_non_sequence_transfer() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var thrown = [];
                function attempt(value) {
                    try { postMessage("", "*", value); thrown.push("no"); }
                    catch (e) { thrown.push(e.name); }
                }
                attempt(1);
                attempt({length: 1});
                attempt("ab");
            "#,
        );
        assert_eq!(
            scheduler.run_script("thrown.join(',')").unwrap(),
            JsValue::str("TypeError,TypeError,TypeError")
        );
    }

    #[test]
    fn malformed_payload_fires_messageerror_not_message() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var got = 0, errored = 0;
                onmessage = function() { got++; };
                onmessageerror = function(e) { errored += (e.type === "messageerror"); };
            "#,
        );
        scheduler
            .test_bindings()
            .queue_raw_message(DeliveryTarget::Window, vec![0xff, 0x00, 0x01]);
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("got === 0 && errored === 1")
                .unwrap(),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn a_self_posting_handler_does_not_starve_the_checkpoint() {
        let mut scheduler = scheduler();
        run(
            &mut scheduler,
            r#"
                var count = 0;
                var mc = new MessageChannel();
                mc.port1.addEventListener("message", function() { count++; });
                mc.port1.addEventListener("message", function() {
                    mc.port2.postMessage(0);
                });
                mc.port1.start();
                mc.port2.postMessage(0);
            "#,
        );
        scheduler.pump_now(true).unwrap();
        assert_eq!(
            scheduler
                .run_script("count >= 1 && count < 100")
                .unwrap(),
            JsValue::Bool(true)
        );
    }
}
