//! XHR, XHR upload and FileReader as real DOM EventTargets, plus FileReader.
//!
//! Each such object owns an external event-target slot in the DOM event system;
//! events are genuine `Event` / `ProgressEvent` objects dispatched through the
//! ordinary DOM event path (EventHandler attributes included).
use super::*;
use amazing_dom::events::{Event, EventClass, EventExtra, EventInit, EventTarget};

/// FileReader states and pending read, keyed by Fetch object slot.
pub(super) struct FileReaderState {
    ready_state: u8,
    generation: u64,
    result: Option<JsValue>,
    error: Option<JsValue>,
    job: Option<ReadJob>,
}
struct ReadJob { blob: amazing_fetch::api::Blob, kind: ReadKind, label: Option<String> }
#[derive(Clone, Copy, PartialEq)]
enum ReadKind { ArrayBuffer, BinaryString, Text, DataUrl }

#[derive(Default)]
pub(super) struct EventTargetRuntime {
    /// Fetch object slot -> DOM external event-target slot.
    pub(super) slots: HashMap<u64, u64>,
    /// DOM external slot -> the wrapper object.
    values: HashMap<u64, JsValue>,
    pub(super) readers: HashMap<u64, FileReaderState>,
}

impl RuntimeBindingHost<'_> {
    pub(crate) fn fetch_event_slot_of(&self, value: &JsValue) -> Option<u64> {
        for interface in [FetchInterface::XMLHttpRequest, FetchInterface::XMLHttpRequestUpload, FetchInterface::FileReader] {
            if let Some(key) = self.platform_object(value, interface.brand()) {
                return self.state.borrow().net.events.slots.get(&key.slot).copied();
            }
        }
        None
    }
    pub(crate) fn fetch_event_value_of(&self, slot: u64) -> Option<JsValue> {
        self.state.borrow().net.events.values.get(&slot).cloned()
    }
    /// Called by `fetch_allocate` for objects that are event targets.
    pub(super) fn fetch_register_event_target(&mut self, fetch_slot: u64, value: &JsValue) -> Result<(), Throw> {
        if self.state.borrow().net.events.slots.contains_key(&fetch_slot) { return Ok(()); }
        let slot = self.with_dom(|dom| Ok(dom.create_event_target()))?;
        self.messaging_register_generic_target(slot)?;
        let mut state = self.state.borrow_mut();
        state.net.events.slots.insert(fetch_slot, slot);
        state.net.events.values.insert(slot, value.clone());
        Ok(())
    }
    pub(super) fn fetch_event_handler_impl(&mut self, receiver: &JsValue, name: &'static str,
        new: Option<&JsValue>) -> Result<JsValue, Throw> {
        let slot = self.fetch_event_slot_of(receiver)
            .ok_or_else(|| Throw::type_error("Illegal invocation"))?;
        let target = EventTarget::External(slot);
        let kind = name.strip_prefix("on").unwrap_or(name);
        let Some(new) = new else {
            let root = self.with_dom(|dom| Ok(dom.events().event_handler(target, kind)))?;
            return Ok(root.and_then(|r| self.root_value(r)).unwrap_or(JsValue::Null));
        };
        let root = matches!(new, JsValue::Object(_)).then(|| self.retain_value(new.clone()));
        match self.with_dom(|dom| Ok(dom.events_mut().set_event_handler(target, kind, root))) {
            Ok(old) => { if let Some(old) = old { self.release_value(old); } }
            Err(error) => { if let Some(root) = root { self.release_value(root); } return Err(error); }
        }
        Ok(JsValue::Undefined)
    }
    pub(super) fn fetch_has_listeners(&mut self, value: &JsValue, kinds: &[&str]) -> Result<bool, Throw> {
        let Some(slot) = self.fetch_event_slot_of(value) else { return Ok(false) };
        let target = EventTarget::External(slot);
        self.with_dom(|dom| Ok(kinds.iter().any(|kind| {
            dom.events().event_handler(target, kind).is_some()
                || !dom.events().listener_callbacks(target, kind, false).is_empty()
                || !dom.events().listener_callbacks(target, kind, true).is_empty()
        })))
    }
    /// Fire a trusted, non-bubbling, non-cancelable event at an event-target slot.
    pub(super) fn fetch_fire(&mut self, slot: u64, name: &str, progress: Option<(u64, u64, bool)>) -> Result<(), Throw> {
        let mut event = Event::new(name, EventInit { bubbles: false, cancelable: false, composed: false }, Duration::ZERO);
        if let Some((loaded, total, computable)) = progress {
            event.set_class(EventClass::Progress);
            event.set_extra("loaded", EventExtra::Number(loaded as f64));
            event.set_extra("total", EventExtra::Number(total as f64));
            event.set_extra("lengthComputable", EventExtra::Boolean(computable));
        }
        self.state.borrow_mut().net.report_listener_errors += 1;
        let result = amazing_bindings::dispatch_user_agent_event_with_payload(self, EventTarget::External(slot), event);
        self.state.borrow_mut().net.report_listener_errors -= 1;
        result.map(|_| ())
    }

    // ---------------------------------------------------------------- FileReader

    fn reader_dom_exception(&mut self, name: &str) -> Throw {
        match invoke(self, Operation::DOMExceptionConstructor, JsValue::Undefined,
            &[JsValue::str(name), JsValue::str(name)]) {
            Ok(value) => self.throw_value(value),
            Err(failure) => failure,
        }
    }
    fn reader_slot(&self, receiver: &JsValue) -> Result<u64, Throw> {
        self.platform_object(receiver, FetchInterface::FileReader.brand()).map(|key| key.slot)
            .filter(|slot| self.state.borrow().net.events.readers.contains_key(slot))
            .ok_or_else(|| Throw::type_error("Illegal invocation"))
    }
    fn reader_schedule(&mut self, slot: u64, generation: u64, stage: u8) -> Result<(), Throw> {
        let callback = self.native("FileReader task", 0,
            NativeEntry::Net(NetNativeEntry::FileReaderTask { slot, generation, stage }), HostCallPolicy::CallOnly);
        let global = self.js.binding_global_object();
        let timer = self.js.binding_get_property(&global, "setTimeout")?;
        binding_call(self.js, &timer, &global, &[callback, JsValue::num(0.0)])?;
        Ok(())
    }
    pub(super) fn fetch_file_reader_impl(&mut self, op: FetchOperation, receiver: &JsValue, args: &[JsValue])
        -> Result<JsValue, Throw> {
        use FetchOperation as O;
        if op == O::FileReaderConstructor {
            let value = self.fetch_allocate(FetchInterface::FileReader, FetchObject::FileReader)?;
            let slot = self.platform_object(&value, FetchInterface::FileReader.brand())
                .ok_or_else(|| Throw::type_error("FileReader allocation"))?.slot;
            self.state.borrow_mut().net.events.readers.insert(slot, FileReaderState {
                ready_state: 0, generation: 0, result: None, error: None, job: None });
            return Ok(value);
        }
        let slot = self.reader_slot(receiver)?;
        match op {
            O::FileReaderReadyState => Ok(JsValue::num(self.state.borrow().net.events.readers[&slot].ready_state as f64)),
            O::FileReaderResult => {
                let state = self.state.borrow();
                let reader = &state.net.events.readers[&slot];
                Ok(if reader.ready_state == 2 { reader.result.clone().unwrap_or(JsValue::Null) } else { JsValue::Null })
            }
            O::FileReaderError => Ok(self.state.borrow().net.events.readers[&slot].error.clone().unwrap_or(JsValue::Null)),
            O::FileReaderAbort => {
                let state = self.state.borrow().net.events.readers[&slot].ready_state;
                if state != 1 {
                    if state == 2 { self.state.borrow_mut().net.events.readers.get_mut(&slot).unwrap().result = None; }
                    return Ok(JsValue::Undefined);
                }
                let error = {
                    match invoke(self, Operation::DOMExceptionConstructor, JsValue::Undefined,
                        &[JsValue::str("The read was aborted"), JsValue::str("AbortError")]) {
                        Ok(value) => value, Err(failure) => return Err(failure),
                    }
                };
                {
                    let mut st = self.state.borrow_mut();
                    let reader = st.net.events.readers.get_mut(&slot).unwrap();
                    reader.ready_state = 2; reader.result = None; reader.error = Some(error);
                    reader.generation += 1; reader.job = None;
                }
                let target = self.state.borrow().net.events.slots[&slot];
                self.fetch_fire(target, "abort", Some((0, 0, false)))?;
                self.fetch_fire(target, "loadend", Some((0, 0, false)))?;
                Ok(JsValue::Undefined)
            }
            _ => {
                let kind = match op {
                    O::FileReaderReadAsArrayBuffer => ReadKind::ArrayBuffer,
                    O::FileReaderReadAsBinaryString => ReadKind::BinaryString,
                    O::FileReaderReadAsText => ReadKind::Text,
                    _ => ReadKind::DataUrl,
                };
                let blob_value = args.first().cloned()
                    .ok_or_else(|| Throw::type_error("Not enough arguments"))?;
                let blob = if self.platform_object(&blob_value, InterfaceBrand::FILE).is_some() {
                    self.file_record(&blob_value)?.blob
                } else if self.platform_object(&blob_value, InterfaceBrand::BLOB).is_some() {
                    self.blob_record(&blob_value)?
                } else { return Err(Throw::type_error("Argument is not a Blob")); };
                let label = match args.get(1) {
                    Some(v) if kind == ReadKind::Text && !matches!(v, JsValue::Undefined) =>
                        Some(String::from_utf16_lossy(&self.js.binding_to_dom_string(v)?)),
                    _ => None,
                };
                if self.state.borrow().net.events.readers[&slot].ready_state == 1 {
                    return Err(self.reader_dom_exception("InvalidStateError"));
                }
                let generation = {
                    let mut st = self.state.borrow_mut();
                    let reader = st.net.events.readers.get_mut(&slot).unwrap();
                    reader.ready_state = 1; reader.result = None; reader.error = None;
                    reader.generation += 1; reader.job = Some(ReadJob { blob, kind, label });
                    reader.generation
                };
                self.reader_schedule(slot, generation, 0)?;
                Ok(JsValue::Undefined)
            }
        }
    }
    /// One queued FileReader task: stage 0 loadstart, 1 progress, 2 load and loadend.
    pub(super) fn file_reader_task(&mut self, slot: u64, generation: u64, stage: u8) -> Result<JsValue, Throw> {
        let (current, size, target) = {
            let st = self.state.borrow();
            let Some(reader) = st.net.events.readers.get(&slot) else { return Ok(JsValue::Undefined) };
            (reader.generation == generation && reader.ready_state == 1,
             reader.job.as_ref().map_or(0, |job| job.blob.bytes.len() as u64),
             st.net.events.slots.get(&slot).copied())
        };
        let Some(target) = target.filter(|_| current) else { return Ok(JsValue::Undefined) };
        match stage {
            0 => { self.fetch_fire(target, "loadstart", Some((0, size, true)))?; }
            1 => { self.fetch_fire(target, "progress", Some((size, size, true)))?; }
            _ => {
                let job = self.state.borrow_mut().net.events.readers.get_mut(&slot).and_then(|r| r.job.take());
                let Some(job) = job else { return Ok(JsValue::Undefined) };
                let result = self.reader_result(&job)?;
                {
                    let mut st = self.state.borrow_mut();
                    let reader = st.net.events.readers.get_mut(&slot).unwrap();
                    reader.ready_state = 2; reader.result = Some(result);
                }
                self.fetch_fire(target, "load", Some((size, size, true)))?;
                if self.state.borrow().net.events.readers[&slot].ready_state != 1 {
                    self.fetch_fire(target, "loadend", Some((size, size, true)))?;
                }
                return Ok(JsValue::Undefined);
            }
        }
        // A handler may have aborted or restarted the reader; the generation check covers that.
        let still = self.state.borrow().net.events.readers.get(&slot)
            .is_some_and(|r| r.generation == generation && r.ready_state == 1);
        if still { self.reader_schedule(slot, generation, stage + 1)?; }
        Ok(JsValue::Undefined)
    }
    fn reader_result(&mut self, job: &ReadJob) -> Result<JsValue, Throw> {
        let bytes = &job.blob.bytes;
        Ok(match job.kind {
            ReadKind::ArrayBuffer => self.js.binding_create_array_buffer(bytes.clone())?,
            ReadKind::BinaryString => JsValue::str(bytes.iter().map(|b| *b as char).collect::<String>()),
            ReadKind::Text => {
                let from_type = job.blob.content_type.split(';').skip(1).find_map(|p| {
                    let (k, v) = p.split_once('=')?;
                    k.trim().eq_ignore_ascii_case("charset").then(|| v.trim().trim_matches('"').to_owned())
                });
                let label = job.label.clone().or(from_type);
                JsValue::str(decode_xhr_text(bytes, label.as_deref()))
            }
            ReadKind::DataUrl => {
                let mime = if job.blob.content_type.is_empty() { "application/octet-stream" } else { &job.blob.content_type };
                JsValue::str(format!("data:{mime};base64,{}", base64_encode_bytes(bytes)))
            }
        })
    }
}

fn base64_encode_bytes(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

impl RuntimeBindingHost<'_> {
    /// XMLHttpRequest -> XMLHttpRequestEventTarget -> EventTarget, FileReader -> EventTarget.
    pub(super) fn link_event_target_prototypes(&mut self) -> Result<(), Throw> {
        let interfaces = self.state.borrow().net.fetch_interfaces.clone();
        let find = |wanted: FetchInterface| interfaces.iter().find(|(kind, _, _)| *kind == wanted)
            .map(|(_, p, c)| (p.clone(), c.clone()));
        let (Some(event_proto), Some(event_ctor)) = (self.prototype(Interface::EventTarget), self.constructor(Interface::EventTarget))
            else { return Err(Throw::type_error("EventTarget interface missing")) };
        let (Some(xet), Some(xhr), Some(upload), Some(reader)) = (
            find(FetchInterface::XMLHttpRequestEventTarget), find(FetchInterface::XMLHttpRequest),
            find(FetchInterface::XMLHttpRequestUpload), find(FetchInterface::FileReader))
            else { return Err(Throw::type_error("Fetch event-target interfaces missing")) };
        let set = |child: &JsValue, parent: &JsValue| {
            if let (JsValue::Object(child), JsValue::Object(parent)) = (child, parent) {
                child.borrow_mut().proto = Some(parent.clone());
            }
        };
        set(&xet.0, &event_proto); set(&xet.1, &event_ctor);
        set(&xhr.0, &xet.0); set(&xhr.1, &xet.1);
        set(&upload.0, &xet.0); set(&upload.1, &xet.1);
        set(&reader.0, &event_proto); set(&reader.1, &event_ctor);
        for (name, (proto, ctor)) in [("XMLHttpRequestEventTarget", &xet), ("XMLHttpRequest", &xhr),
            ("XMLHttpRequestUpload", &upload), ("FileReader", &reader)] {
            self.js.binding_define_symbol_data_property(proto,
                amazing_js::value::WellKnownSymbol::ToStringTag.symbol(), JsValue::str(name),
                BindingPropertyFlags { writable: false, enumerable: false, configurable: true })?;
            let _ = ctor;
        }
        Ok(())
    }
}
