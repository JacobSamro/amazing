use super::*;
use crate::host::{AbortHost, BindingHost, CallbackId, DocumentField, DocumentHost, FormAttribute as F, FragmentContext, FragmentParser, HostCallPolicy, ImageLoaderState, Interface, InterfaceBrand, InterfaceDefinition, IterableKind, LegacyPlatformHooks, Operation as O, OptionalHookInstall, PlatformKey, SelectorEngine, UrlReflection};
use amazing_dom::NodeKind;
use amazing_fetch::api::{Blob, BlobPart, File, FormData};
use amazing_net::url::{UrlSearchParams, UrlWithSearchParams};

#[derive(Clone)]
enum WebMock {
    DomParser,
    Url(UrlWithSearchParams, Option<usize>), Params(UrlSearchParams),
    Blob(Blob), File(File), FormData(FormData), FontFaceSet,
}

#[derive(Clone, Debug, PartialEq)]
enum V {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Vec<u16>),
    Node(NodeIdx),
    Platform(PlatformKey, InterfaceBrand),
    Array(Vec<V>),
    Callable(u64),
    Dictionary(Vec<(&'static str, V)>),
    List(NodeIdx),
    Coerce(&'static str),
    Throw,
    Promise(u64),
    /// A callable whose body is a Rust closure run against the live `Host`,
    /// for tests that need a "user callback" to perform real side effects
    /// (e.g. an Observable initializer that calls back into
    /// `subscriber.next()`/`.error()`/`.complete()`/`.addTeardown()` on the
    /// very `Subscriber` value it was invoked with) rather than the fixed
    /// canned response every plain `V::Callable` gets from `Host::call`.
    /// See `Host::script`.
    Script(usize),
}
#[derive(Default)]
struct Host {
    dom: Dom,
    installed: Vec<Interface>,
    class_strings: Vec<(Interface, &'static str)>,
    dom_exception_error_prototype: bool,
    custom_elements_global: bool,
    promises: Vec<(u64, Option<V>, Option<V>)>,
    custom_element_errors: Vec<String>,
    fail_custom_element_constructor: bool,
    skip_optional_hooks: bool,
    fail_class_string: Option<Interface>,
    constants: Vec<(&'static str, u16)>,
    interface_constants: Vec<(Interface, &'static str, u32)>,
    conversions: Vec<&'static str>,
    property_reads: Vec<String>,
    arena_borrowed: bool,
    callbacks: Vec<Option<V>>,
    calls: Vec<u64>,
    call_receivers: Vec<(V, Vec<V>)>,
    reject_lone_surrogate: bool,
    legacy: Vec<(InterfaceBrand, LegacyPlatformHooks)>,
    iterables: Vec<(Interface, IterableKind)>,
    static_methods: Vec<(Interface, &'static str, O, u8)>,
    abort_timers: Vec<(amazing_dom::abort::AbortSignalId, u64)>,
    abort_algorithms: Vec<u64>,
    abort_trace: Vec<u64>,
    active: Option<NodeIdx>,
    focused: bool,
    web: Vec<WebMock>,
    fonts: Option<V>,
    font_ready: Option<V>,
    image_state: Option<ImageLoaderState>,
    image_decode_requested: Vec<NodeIdx>,
    external_targets: Vec<(u64, V)>,
    window: Option<(u64, V)>,
    cancel_events: bool,
    callback_targets: Vec<Option<amazing_dom::events::EventTarget>>,
    #[allow(clippy::type_complexity)]
    scripts: Vec<Option<Box<dyn FnMut(&mut Host, &V, &[V]) -> Result<V, String>>>>,
}
impl Host {
    fn web_push(&mut self, object: WebMock, brand: InterfaceBrand) -> V {
        let slot = self.web.len() as u64;
        self.web.push(object);
        V::Platform(PlatformKey { arena: 1, slot, generation: 1 }, brand)
    }
    fn web_index(&self, value: &V, brand: InterfaceBrand) -> Result<usize, String> {
        self.platform_object(value, brand).map(|key| key.slot as usize)
            .ok_or_else(|| "incompatible web receiver".into())
    }
    /// Wraps a Rust closure as a `V::Script`, callable through the ordinary
    /// `BindingHost::call` path -- see `V::Script`'s doc comment.
    fn script(&mut self, f: impl FnMut(&mut Host, &V, &[V]) -> Result<V, String> + 'static) -> V {
        let id = self.scripts.len();
        self.scripts.push(Some(Box::new(f)));
        V::Script(id)
    }
}
impl BindingHost for Host {
    type Value = V;
    type Error = String;
    fn dom_parser_new(&mut self) -> Result<V, String> {
        Ok(self.web_push(WebMock::DomParser, InterfaceBrand::DOM_PARSER))
    }
    fn create_inert_document(&mut self, bytes: &[u8], content_type: &str,
        url: &str, charset: Option<&str>) -> Result<NodeIdx, DomError> {
        if bytes == b"<bad" { return Err(DomError::SyntaxError); }
        let mut parsed = Dom::new();
        let html = content_type == "text/html";
        let name = if bytes.starts_with(b"<parsererror") { "parsererror" }
            else if html { "html" } else { "root" };
        parsed.create_element_ns(NodeIdx(0),
            Some(if html { "http://www.w3.org/1999/xhtml" } else { "urn:test" }), name)?;
        self.dom.import_inert_document(&parsed, url, content_type, charset.unwrap_or("UTF-8"))
    }
    fn global_object(&mut self) -> Result<V, String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn create_object(&mut self, _prototype: Option<&V>) -> Result<V, String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn create_native_function(
        &mut self,
        _name: &'static str,
        _length: u8,
        _operation: O,
        _policy: HostCallPolicy,
    ) -> Result<V, String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn define_data_property(
        &mut self,
        _object: &V,
        _name: &'static str,
        _value: V,
        _writable: bool,
        _enumerable: bool,
        _configurable: bool,
    ) -> Result<(), String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn define_accessor_property(
        &mut self,
        _object: &V,
        _name: &'static str,
        _getter: Option<V>,
        _setter: Option<V>,
        _enumerable: bool,
        _configurable: bool,
    ) -> Result<(), String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn define_legacy_platform_object(&mut self, brand: InterfaceBrand, hooks: LegacyPlatformHooks) -> Result<(), String> {
        self.legacy.push((brand, hooks)); Ok(())
    }
    fn define_iterable(&mut self, interface: Interface, kind: IterableKind) -> Result<(), String> {
        self.iterables.push((interface, kind)); Ok(())
    }
    fn define_pair_iterable(&mut self, _interface: Interface) -> Result<OptionalHookInstall, String> {
        Ok(if self.skip_optional_hooks { OptionalHookInstall::NotSupported } else { OptionalHookInstall::Installed })
    }
    fn get_property(&mut self, object: &V, name: &str) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        self.property_reads.push(name.to_owned());
        match object {
            V::Dictionary(entries) => Ok(entries.iter().find(|(key, _)| *key == name)
                .map_or(V::Undefined, |(_, value)| value.clone())),
            V::Array(values) if name == "length" => Ok(V::Number(values.len() as f64)),
            V::Array(values) => Ok(name.parse::<usize>().ok().and_then(|i| values.get(i).cloned()).unwrap_or(V::Undefined)),
            _ => Err("property access on non-object".into()),
        }
    }
    fn set_property(&mut self, _object: &V, _name: &str, _value: V) -> Result<(), String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn wrap_platform_object(&mut self, key: PlatformKey, brand: InterfaceBrand) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        Ok(if brand == InterfaceBrand::NODE {
            V::Node(self.node_from_platform_key(key).ok_or("stale node key")?)
        } else if brand == InterfaceBrand::NODE_LIST {
            V::List(self.node_from_platform_key(key).ok_or("stale parent key")?)
        } else {
            V::Platform(key, brand)
        })
    }
    fn platform_object(&self, value: &V, brand: InterfaceBrand) -> Option<PlatformKey> {
        match value {
            V::Node(node) if brand == InterfaceBrand::NODE => Some(self.node_platform_key(*node)),
            V::List(node) if brand == InterfaceBrand::NODE_LIST => Some(self.node_platform_key(*node)),
            V::Platform(key, actual) if *actual == brand => Some(*key),
            _ => None,
        }
    }
    fn node_platform_key(&self, node: NodeIdx) -> PlatformKey {
        PlatformKey { arena: 1, slot: u64::from(node.0), generation: 1 }
    }
    fn node_from_platform_key(&self, key: PlatformKey) -> Option<NodeIdx> {
        (key.arena == 1 && key.generation == 1 && key.slot < self.dom.node_count() as u64)
            .then(|| NodeIdx(key.slot as u32))
    }
    fn is_callable(&self, value: &V) -> bool { matches!(value, V::Callable(_) | V::Script(_)) }
    fn call(&mut self, function: &V, this: &V, arguments: &[V]) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        self.call_receivers.push((this.clone(), arguments.to_vec()));
        if let Some(V::Platform(key, InterfaceBrand::EVENT)) = arguments.first() {
            let event = self.dom.events_mut().event_mut(amazing_dom::events::EventId::from_slot(key.slot as usize));
            self.callback_targets.push(event.current_target());
            if self.cancel_events { event.prevent_default(); }
        }
        if let V::Callable(id) = function { self.calls.push(*id); self.abort_trace.push(*id); }
        if let V::Script(id) = function {
            let mut script = self.scripts[*id].take().expect("script re-entered (no reentry support in this mock)");
            let result = script(self, this, arguments);
            self.scripts[*id] = Some(script);
            return result;
        }
        match function { V::Callable(1) => Ok(V::Number(1.)), V::Callable(_) => Ok(V::Undefined), _ => Err("not callable".into()) }
    }
    fn same_value(&self, left: &V, right: &V) -> bool { left == right }
    fn retain_value(&mut self, value: V) -> CallbackId {
        assert!(!self.arena_borrowed);
        let id = CallbackId(self.callbacks.len() as u64);
        self.callbacks.push(Some(value));
        id
    }
    fn root_value(&self, callback: CallbackId) -> Option<V> {
        self.callbacks.get(callback.0 as usize).and_then(Clone::clone)
    }
    fn release_value(&mut self, callback: CallbackId) {
        if let Some(slot) = self.callbacks.get_mut(callback.0 as usize) { *slot = None; }
    }
    fn window_event_target_slot(&self, value: &V) -> Option<u64> {
        self.window.as_ref().filter(|(_, window)| window == value).map(|(slot, _)| *slot)
    }
    fn window_event_target_value(&self, slot: u64) -> Option<V> {
        self.window.as_ref().filter(|(target, _)| *target == slot).map(|(_, window)| window.clone())
    }
    fn external_event_target_slot(&self, value: &V) -> Option<u64> {
        self.external_targets.iter().find(|(_, wrapper)| wrapper == value).map(|(slot, _)| *slot)
    }
    fn external_event_target_value(&self, slot: u64) -> Option<V> {
        self.external_targets.iter().find(|(target, _)| *target == slot).map(|(_, value)| value.clone())
    }
    fn is_message_port(&self, value: &V) -> bool { self.external_event_target_slot(value).is_some() }
    fn sequence_message_ports(&mut self, value: &V) -> Result<Vec<V>, String> {
        match value { V::Array(values) if values.iter().all(|value| self.is_message_port(value)) => Ok(values.clone()),
            _ => Err("TypeError: Expected MessagePort sequence".into()) }
    }
    fn create_frozen_array(&mut self, values: Vec<V>) -> Result<V, String> { self.create_array(values) }
    fn create_array(&mut self, values: Vec<V>) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        Ok(V::Array(values))
    }
    fn to_number(&mut self, value: &V) -> Result<f64, String> {
        assert!(!self.arena_borrowed);
        match value { V::Number(n) => Ok(*n), V::Null => Ok(0.), V::Undefined => Ok(f64::NAN), V::Bool(b) => Ok(if *b { 1. } else { 0. }), V::Throw => Err("coercion sentinel".into()), _ => Ok(f64::NAN) }
    }
    fn throw_value(&mut self, value: V) -> String { format!("throw: {value:?}") }
    fn install_interface(&mut self, d: &'static InterfaceDefinition) -> Result<(), String> {
        self.installed.push(d.interface);
        Ok(())
    }
    fn install_interface_class_string(&mut self, interface: Interface, name: &'static str) -> Result<OptionalHookInstall, String> {
        if self.fail_class_string == Some(interface) { return Err("class string install failed".into()); }
        if self.skip_optional_hooks { return Ok(OptionalHookInstall::NotSupported); }
        self.class_strings.push((interface, name));
        Ok(OptionalHookInstall::Installed)
    }
    fn install_dom_exception_error_prototype(&mut self) -> Result<OptionalHookInstall, String> {
        if self.skip_optional_hooks { return Ok(OptionalHookInstall::NotSupported); }
        self.dom_exception_error_prototype = true;
        Ok(OptionalHookInstall::Installed)
    }
    fn install_custom_elements_global(&mut self) -> Result<OptionalHookInstall, String> {
        if self.skip_optional_hooks { return Ok(OptionalHookInstall::NotSupported); }
        self.custom_elements_global = true;
        Ok(OptionalHookInstall::Installed)
    }
    fn install_cssom(&mut self) -> Result<OptionalHookInstall, String> {
        Ok(if self.skip_optional_hooks { OptionalHookInstall::NotSupported } else { OptionalHookInstall::Installed })
    }
    fn is_constructor(&mut self, value: &V) -> Result<bool, String> {
        Ok(matches!(value, V::Dictionary(entries) if entries.iter().any(|(name, value)|
            *name == "__constructor" && *value == V::Bool(true))))
    }
    fn is_object_value(&mut self, value: &V) -> Result<bool, String> {
        Ok(!matches!(value, V::Undefined | V::Null | V::Bool(_) | V::Number(_) | V::String(_)))
    }
    fn require_object(&mut self, value: &V) -> Result<(), String> {
        if matches!(value, V::Dictionary(_) | V::Array(_)) { Ok(()) }
        else { Err("TypeError: Expected an object".into()) }
    }
    fn sequence_dom_strings(&mut self, value: &V) -> Result<Vec<Vec<u16>>, String> {
        let V::Array(values) = value else { return Err("TypeError: Expected a sequence".into()) };
        values.iter().map(|value| self.to_dom_string(value)).collect()
    }
    fn sequence_shadow_roots(&mut self, value: &V) -> Result<Vec<NodeIdx>, String> {
        let V::Array(values) = value else { return Err("TypeError: Expected a sequence".into()) };
        values.iter().map(|value| match value {
            V::Node(node) if self.dom.is_shadow_root(*node) => Ok(*node),
            _ => Err("TypeError: Expected a ShadowRoot".into()),
        }).collect()
    }
    fn construct_custom_element(&mut self, _constructor: &V, definition: usize, element: NodeIdx)
        -> Result<V, String> {
        if self.fail_custom_element_constructor { return Err("constructor failed".into()); }
        assert_eq!(self.with_dom(|dom| dom.custom_element_super(definition))?, element);
        Ok(V::Node(element))
    }
    fn new_promise(&mut self) -> Result<V, String> {
        let id = self.promises.len() as u64;
        self.promises.push((id, None, None));
        Ok(V::Promise(id))
    }
    fn resolve_promise(&mut self, promise: &V, value: V) -> Result<(), String> {
        let V::Promise(id) = promise else { return Err("TypeError: Expected a Promise".into()) };
        self.promises[*id as usize].1 = Some(value);
        Ok(())
    }
    fn reject_promise(&mut self, promise: &V, reason: V) -> Result<(), String> {
        let V::Promise(id) = promise else { return Err("TypeError: Expected a Promise".into()) };
        self.promises[*id as usize].2 = Some(reason);
        Ok(())
    }
    fn dom_exception_value(&mut self, error: DomError) -> Result<V, String> {
        Ok(str_value(&error.to_string()))
    }
    fn image_loader_state(&mut self, _node: NodeIdx) -> Result<Option<ImageLoaderState>, String> {
        Ok(self.image_state.clone())
    }
    fn image_decode(&mut self, node: NodeIdx, promise: &V) -> Result<OptionalHookInstall, String> {
        self.image_decode_requested.push(node);
        if self.image_state.is_some() {
            self.resolve_promise(promise, V::Undefined)?;
            Ok(OptionalHookInstall::Installed)
        } else { Ok(OptionalHookInstall::NotSupported) }
    }
    fn report_custom_element_exception(&mut self, error: String) -> Result<(), String> {
        self.custom_element_errors.push(error);
        Ok(())
    }
    fn form_pattern_matches(&mut self, pattern: &[u16], value: &[u16]) -> Result<Option<bool>, String> {
        assert!(!self.arena_borrowed);
        if pattern == "[A-Z]+".encode_utf16().collect::<Vec<_>>() {
            return Ok(Some(value.iter().all(|unit| (65..=90).contains(unit))));
        }
        Err("NotSupportedError".into())
    }
    fn form_date_from_epoch_ms(&mut self, milliseconds: f64) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        Ok(V::Number(milliseconds))
    }
    fn form_date_to_epoch_ms(&mut self, value: &V) -> Result<Option<f64>, String> {
        assert!(!self.arena_borrowed);
        match value { V::Number(n) => Ok(Some(*n)), _ => Err("TypeError: Expected Date".into()) }
    }
    fn url_new(&mut self, url: UrlWithSearchParams) -> Result<V, String> {
        Ok(self.web_push(WebMock::Url(url, None), InterfaceBrand::URL))
    }
    fn url_record(&mut self, receiver: &V) -> Result<UrlWithSearchParams, String> {
        let index = self.web_index(receiver, InterfaceBrand::URL)?;
        match &self.web[index] { WebMock::Url(url, _) => Ok(url.clone()), _ => Err("not URL".into()) }
    }
    fn url_search_params(&mut self, receiver: &V) -> Result<V, String> {
        let index = self.web_index(receiver, InterfaceBrand::URL)?;
        let params = match &self.web[index] {
            WebMock::Url(_, Some(params)) => *params,
            WebMock::Url(url, None) => {
                let value = url.search_params();
                let next = self.web.len();
                self.web.push(WebMock::Params(value));
                if let WebMock::Url(_, cache) = &mut self.web[index] { *cache = Some(next); }
                next
            }
            _ => return Err("not URL".into()),
        };
        Ok(V::Platform(PlatformKey { arena: 1, slot: params as u64, generation: 1 }, InterfaceBrand::URL_SEARCH_PARAMS))
    }
    fn url_params_new(&mut self, params: UrlSearchParams) -> Result<V, String> {
        Ok(self.web_push(WebMock::Params(params), InterfaceBrand::URL_SEARCH_PARAMS))
    }
    fn url_params_record(&mut self, receiver: &V) -> Result<UrlSearchParams, String> {
        let index = self.web_index(receiver, InterfaceBrand::URL_SEARCH_PARAMS)?;
        match &self.web[index] { WebMock::Params(params) => Ok(params.clone()), _ => Err("not params".into()) }
    }
    fn url_params_store(&mut self, receiver: &V, params: UrlSearchParams) -> Result<(), String> {
        let index = self.web_index(receiver, InterfaceBrand::URL_SEARCH_PARAMS)?;
        self.web[index] = WebMock::Params(params);
        Ok(())
    }
    fn url_params_pairs(&mut self, value: &V) -> Result<Option<Vec<(String, String)>>, String> {
        match value {
            V::Array(rows) => rows.iter().map(|row| match row {
                V::Array(pair) if pair.len() == 2 => Ok((String::from_utf16_lossy(&self.to_dom_string(&pair[0])?), String::from_utf16_lossy(&self.to_dom_string(&pair[1])?))),
                _ => Err("invalid pair".into()),
            }).collect::<Result<Vec<_>,_>>().map(Some),
            V::Dictionary(entries) => entries.iter().map(|(key, value)| Ok(((*key).into(), String::from_utf16_lossy(&self.to_dom_string(value)?)))).collect::<Result<Vec<_>,_>>().map(Some),
            _ => Ok(None),
        }
    }
    fn blob_parts(&mut self, value: &V) -> Result<Vec<BlobPart>, String> {
        let V::Array(parts) = value else { return Err("Blob parts must be array".into()) };
        parts.iter().map(|part| match part {
            V::Platform(_, InterfaceBrand::BLOB) => self.blob_record(part).map(BlobPart::Blob),
            V::Platform(_, InterfaceBrand::FILE) => self.file_record(part).map(|file| BlobPart::Blob(file.blob)),
            _ => Ok(BlobPart::Text(String::from_utf16_lossy(&self.to_dom_string(part)?))),
        }).collect()
    }
    fn blob_new(&mut self, blob: Blob) -> Result<V, String> { Ok(self.web_push(WebMock::Blob(blob), InterfaceBrand::BLOB)) }
    fn blob_record(&mut self, receiver: &V) -> Result<Blob, String> {
        let index = self.web_index(receiver, InterfaceBrand::BLOB)?;
        match &self.web[index] { WebMock::Blob(blob) => Ok(blob.clone()), _ => Err("not blob".into()) }
    }
    fn file_new(&mut self, file: File) -> Result<V, String> { Ok(self.web_push(WebMock::File(file), InterfaceBrand::FILE)) }
    fn file_record(&mut self, receiver: &V) -> Result<File, String> {
        let index = self.web_index(receiver, InterfaceBrand::FILE)?;
        match &self.web[index] { WebMock::File(file) => Ok(file.clone()), _ => Err("not file".into()) }
    }
    fn form_data_new(&mut self, data: FormData) -> Result<V, String> { Ok(self.web_push(WebMock::FormData(data), InterfaceBrand::FORM_DATA)) }
    fn form_data_record(&mut self, receiver: &V) -> Result<FormData, String> {
        let index = self.web_index(receiver, InterfaceBrand::FORM_DATA)?;
        match &self.web[index] { WebMock::FormData(data) => Ok(data.clone()), _ => Err("not form data".into()) }
    }
    fn form_data_store(&mut self, receiver: &V, data: FormData) -> Result<(), String> {
        let index = self.web_index(receiver, InterfaceBrand::FORM_DATA)?;
        self.web[index] = WebMock::FormData(data);
        Ok(())
    }
    fn blob_promise_bytes(&mut self, bytes: Vec<u8>, as_text: bool, as_array_buffer: bool) -> Result<V, String> {
        let value = if as_text { str_value(&String::from_utf8_lossy(&bytes)) }
            else if as_array_buffer { V::Array(bytes.iter().map(|b| V::Number(f64::from(*b))).collect()) }
            else { V::Array(bytes.iter().map(|b| V::Number(f64::from(*b))).collect()) };
        let id = self.promises.len() as u64;
        self.promises.push((id, Some(value), None));
        Ok(V::Promise(id))
    }
    fn font_face_set(&mut self, _document: NodeIdx) -> Result<V, String> {
        if let Some(value) = &self.fonts { return Ok(value.clone()) }
        let value = self.web_push(WebMock::FontFaceSet, InterfaceBrand::FONT_FACE_SET);
        self.fonts = Some(value.clone());
        Ok(value)
    }
    fn font_face_set_ready(&mut self, receiver: &V) -> Result<V, String> {
        self.web_index(receiver, InterfaceBrand::FONT_FACE_SET)?;
        if let Some(value) = &self.font_ready { return Ok(value.clone()) }
        let id = self.promises.len() as u64;
        self.promises.push((id, Some(receiver.clone()), None));
        let value = V::Promise(id);
        self.font_ready = Some(value.clone());
        Ok(value)
    }
    fn install_node_constant(&mut self, n: &'static str, v: u16) -> Result<(), String> {
        self.constants.push((n, v));
        Ok(())
    }
    fn install_interface_constant(&mut self, interface: Interface, name: &'static str, value: u32) -> Result<(), String> {
        self.interface_constants.push((interface, name, value)); Ok(())
    }
    fn install_static_method(&mut self, interface: Interface, name: &'static str,
        operation: O, length: u8) -> Result<(), String> {
        self.static_methods.push((interface, name, operation, length)); Ok(())
    }
    fn current_document(&self) -> NodeIdx {
        NodeIdx(0)
    }
    fn with_dom<R>(
        &mut self,
        callback: impl FnOnce(&mut Dom) -> Result<R, DomError>,
    ) -> Result<R, String> {
        assert!(!self.arena_borrowed);
        self.arena_borrowed = true;
        let result = callback(&mut self.dom).map_err(|e| e.to_string());
        self.arena_borrowed = false;
        result
    }
    fn is_html_document(&self, n: NodeIdx) -> bool {
        n == NodeIdx(0) || self.dom.document_metadata(n).ok().flatten()
            .is_some_and(|data| data.content_type == "text/html")
    }
    fn undefined(&self) -> V {
        V::Undefined
    }
    fn null(&self) -> V {
        V::Null
    }
    fn boolean(&self, v: bool) -> V {
        V::Bool(v)
    }
    fn number(&self, v: f64) -> V {
        V::Number(v)
    }
    fn string(&self, v: &[u16]) -> Result<V, String> {
        if self.reject_lone_surrogate && char::decode_utf16(v.iter().copied()).any(|c| c.is_err()) {
            return Err("TypeError: cannot represent lone surrogate".into());
        }
        Ok(V::String(v.to_vec()))
    }
    fn is_undefined(&self, v: &V) -> bool {
        *v == V::Undefined
    }
    fn is_nullish(&self, v: &V) -> bool {
        matches!(v, V::Undefined | V::Null)
    }
    fn to_boolean(&self, v: &V) -> bool {
        match v { V::Undefined | V::Null | V::Bool(false) => false,
            V::Number(n) => *n != 0. && !n.is_nan(),
            V::String(s) => !s.is_empty(), _ => true }
    }
    fn string_units(&self, v: &V) -> Option<Vec<u16>> {
        if let V::String(units) = v { Some(units.clone()) } else { None }
    }
    fn to_dom_string(&mut self, value: &V) -> Result<Vec<u16>, String> {
        assert!(!self.arena_borrowed);
        let text = match value {
            V::String(v) => return Ok(v.clone()),
            V::Coerce(v) => {
                self.conversions.push(v);
                *v
            }
            V::Throw => return Err("coercion sentinel".into()),
            V::Null => "null",
            V::Undefined => "undefined",
            _ => "other",
        };
        Ok(text.encode_utf16().collect())
    }
    fn type_error(&mut self, message: &'static str) -> String {
        format!("TypeError: {message}")
    }
}
impl AbortHost for Host {
    fn schedule_abort_timeout(&mut self, signal: amazing_dom::abort::AbortSignalId,
        milliseconds: u64) -> Result<(), String> {
        assert!(!self.arena_borrowed);
        self.abort_timers.push((signal, milliseconds)); Ok(())
    }
    fn run_abort_algorithm(&mut self, algorithm: u64, _reason: &V) -> Result<(), String> {
        assert!(!self.arena_borrowed);
        self.abort_algorithms.push(algorithm);
        self.abort_trace.push(1000 + algorithm);
        Ok(())
    }
}
impl DocumentHost for Host {
    fn document_default_view(&mut self, document: NodeIdx) -> Result<Option<V>, String> {
        Ok(self.window.as_ref().filter(|_| document == NodeIdx(0)).map(|(_, view)| view.clone()))
    }
    fn document_field(&mut self, document: NodeIdx, field: DocumentField) -> Result<Vec<u16>, String> {
        assert!(!self.arena_borrowed);
        if let Some(data) = self.dom.document_metadata(document).map_err(|error| error.to_string())? {
            let value = match field {
                DocumentField::URL | DocumentField::DocumentURI => data.url.as_str(),
                DocumentField::CharacterSet => data.character_set.as_str(),
                DocumentField::ContentType => data.content_type.as_str(),
                DocumentField::CompatMode => "CSS1Compat",
                DocumentField::Referrer => "",
            };
            return Ok(value.encode_utf16().collect());
        }
        let value = match field {
            DocumentField::URL | DocumentField::DocumentURI => "https://example.test/page",
            DocumentField::CharacterSet => "UTF-8",
            DocumentField::ContentType => "text/html",
            DocumentField::CompatMode => "CSS1Compat",
            DocumentField::Referrer => "https://referrer.test/",
        };
        Ok(value.encode_utf16().collect())
    }
    fn active_element(&mut self, _document: NodeIdx) -> Result<Option<NodeIdx>, String> {
        assert!(!self.arena_borrowed); Ok(self.active)
    }
    fn has_focus(&mut self, _document: NodeIdx) -> Result<bool, String> {
        assert!(!self.arena_borrowed); Ok(self.focused)
    }
}
impl UrlReflection for Host {
    fn resolve_url(&mut self, input: &[u16]) -> Result<Vec<u16>, String> {
        assert!(!self.arena_borrowed);
        let raw = String::from_utf16(input).map_err(|_| "TypeError: invalid URL units".to_owned())?;
        Ok(if raw.starts_with("https://") { raw } else { format!("https://example.test/{raw}") }
            .encode_utf16().collect())
    }
}
impl FragmentParser for Host {
    fn parse_fragment(&mut self, context: FragmentContext, markup: &[u16]) -> Result<Dom, DomError> {
        assert!(!self.arena_borrowed);
        assert!(!context.local_name.is_empty());
        let source = String::from_utf16(markup).map_err(|_| DomError::SyntaxError)?;
        let mut parsed = Dom::new();
        if source == "<b>new</b>" || source == "<em>x</em>" {
            let name = if source.starts_with("<b>") { "b" } else { "em" };
            let element = parsed.create_element(NodeIdx(0), name);
            parsed.append_child(element, NodeKind::Text { data: if name == "b" { "new" } else { "x" }.into() });
        } else if !source.is_empty() {
            parsed.append_child(NodeIdx(0), NodeKind::Text { data: source });
        }
        Ok(parsed)
    }
    /// Crate-level coverage for `markup_bindings.rs`'s top-level
    /// declarative-shadow-root handling (`tests_set_html_unsafe.rs`) needs
    /// a `Dom` shaped exactly like `amazing_html::parse_fragment_with_
    /// shadow_roots` would produce for a top-level `<template
    /// shadowrootmode="open">` -- not a real HTML parser (that shape's real
    /// parsing is covered by `amazing-html`'s own tests and the
    /// `shadow-dom/declarative` WPT gate). Recognizes two sentinel markup
    /// strings; anything else falls back to the ordinary mock parser above.
    fn parse_fragment_with_shadow_roots(&mut self, context: FragmentContext, markup: &[u16])
        -> Result<Dom, DomError> {
        assert!(!self.arena_borrowed);
        let source = String::from_utf16(markup).map_err(|_| DomError::SyntaxError)?;
        if source == "<template-shadow-open>" || source == "<template-shadow-open-with-sibling>" {
            let mut parsed = Dom::new();
            let template = parsed.create_element(NodeIdx(0), "template");
            parsed.set_attribute(template, "shadowrootmode", "open").expect("set attribute");
            let contents = parsed.ensure_template_contents(template).expect("template contents");
            let child = parsed.create_element(contents, "b");
            parsed.append_child(child, NodeKind::Text { data: "shadow-child".into() });
            if source == "<template-shadow-open-with-sibling>" {
                let sibling = parsed.create_element(NodeIdx(0), "i");
                parsed.append_child(sibling, NodeKind::Text { data: "sibling".into() });
            }
            return Ok(parsed);
        }
        self.parse_fragment(context, markup)
    }
}
impl SelectorEngine for Host {
    type SelectorHandle = String;
    fn parse_selector(&mut self, selector: &[u16]) -> Result<String, DomError> {
        assert!(!self.arena_borrowed);
        let text = String::from_utf16(selector).map_err(|_| DomError::SyntaxError)?;
        if text.is_empty() || text == "?" { return Err(DomError::SyntaxError); }
        Ok(text)
    }
    fn matches_selector(&mut self, element: NodeIdx, selector: &String) -> Result<bool, DomError> {
        assert!(!self.arena_borrowed);
        Ok(self.dom.name(element) == Some(selector.as_str()))
    }
}
fn str_value(s: &str) -> V {
    V::String(s.encode_utf16().collect())
}
fn call(h: &mut Host, op: O, receiver: V, args: &[V]) -> V {
    invoke(h, op, receiver, args).unwrap()
}
fn mock_legacy_get(h: &mut Host, receiver: V, key: &str) -> V {
    let hooks = h.legacy.iter().find(|(brand, _)| h.platform_object(&receiver, *brand).is_some()).unwrap().1;
    if let Ok(index) = key.parse::<u32>() {
        if index < crate::supported_index_count(h, hooks.supported_index_count, &receiver).unwrap() {
            return call(h, hooks.indexed_getter, receiver, &[V::Number(index as f64)]);
        }
    }
    if let Some(names_op) = hooks.supported_names {
        let supported = crate::supported_names(h, names_op, &receiver).unwrap();
        if supported.contains(&key.encode_utf16().collect()) {
            return call(h, hooks.named_getter.unwrap(), receiver, &[str_value(key)]);
        }
    }
    V::Undefined
}
fn mock_legacy_keys(h: &mut Host, receiver: V) -> Vec<String> {
    let hooks = h.legacy.iter().find(|(brand, _)| h.platform_object(&receiver, *brand).is_some()).unwrap().1;
    let mut keys = Vec::new();
    let count = crate::supported_index_count(h, hooks.supported_index_count, &receiver).unwrap();
    keys.extend((0..count).map(|index| index.to_string()));
    if let Some(names) = hooks.supported_names {
        let values = crate::supported_names(h, names, &receiver).unwrap();
        for units in values { keys.push(String::from_utf16(&units).unwrap()); }
    }
    keys
}
fn node(v: V) -> NodeIdx {
    if let V::Node(n) = v {
        n
    } else {
        panic!("expected node")
    }
}
fn element(h: &mut Host) -> NodeIdx {
    h.dom
        .create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "div")
        .unwrap()
}

#[test]
fn generic_platform_identity_is_private_and_callbacks_run_after_dom_borrow() {
    let mut h = Host::default();
    let key = PlatformKey { arena: 1, slot: 9, generation: 3 };
    let wrapper = h.wrap_platform_object(key, InterfaceBrand::RANGE).unwrap();
    assert_eq!(h.platform_object(&wrapper, InterfaceBrand::RANGE), Some(key));
    assert_eq!(h.platform_object(&wrapper, InterfaceBrand::EVENT), None);
    assert_eq!(h.node_brand(&wrapper), None);
    assert_eq!(h.node_brand(&V::Node(NodeIdx(0))), Some(NodeIdx(0)));
    let callback = V::Callable(7);
    assert!(h.is_callable(&callback));
    assert!(h.same_value(&callback, &V::Callable(7)));
    let callback_id = h.retain_value(callback.clone());
    assert_eq!(h.root_value(callback_id), Some(callback.clone()));
    h.with_dom(|dom| { dom.create_node_in(NodeIdx(0), NodeKind::Comment { data: "x".into() })?; Ok(()) }).unwrap();
    assert_eq!(h.call(&callback, &V::Undefined, &[]), Ok(V::Undefined));
    h.release_value(callback_id);
    assert_eq!(h.root_value(callback_id), None);
    assert_eq!(h.create_array(vec![wrapper.clone()]), Ok(V::Array(vec![wrapper])));
}

#[test]
fn range_installs_inheritance_constants_and_dispatches_live_boundaries() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    assert!(h.interface_constants.contains(&(Interface::Range, "END_TO_START", 3)));
    let range = call(&mut h, O::CreateRange, V::Node(NodeIdx(0)), &[]);
    assert_eq!(call(&mut h, O::RangeCollapsed, range.clone(), &[]), V::Bool(true));
    let text = node(call(&mut h, O::CreateText, V::Node(NodeIdx(0)), &[str_value("abcd")]));
    call(&mut h, O::RangeSetStart, range.clone(), &[V::Node(text), V::Number(2.)]);
    assert_eq!(call(&mut h, O::RangeStartContainer, range.clone(), &[]), V::Node(text));
    assert_eq!(call(&mut h, O::RangeStartOffset, range.clone(), &[]), V::Number(2.));
    assert_eq!(call(&mut h, O::RangeEndOffset, range.clone(), &[]), V::Number(2.));
    let clone = call(&mut h, O::RangeCloneRange, range.clone(), &[]);
    assert_ne!(clone, range);
    assert_eq!(call(&mut h, O::RangeStartOffset, clone, &[]), V::Number(2.));
    assert!(invoke(&mut h, O::RangeSetEnd, range.clone(), &[V::Node(text), V::Throw]).is_err());
    assert_eq!(call(&mut h, O::RangeEndOffset, range, &[]), V::Number(2.));
}

#[test]
fn document_implementation_is_same_object_and_builds_document_fixture() {
    let mut h = Host::default();
    let implementation = call(&mut h, O::DocumentImplementation, V::Node(NodeIdx(0)), &[]);
    assert_eq!(implementation, call(&mut h, O::DocumentImplementation, V::Node(NodeIdx(0)), &[]));
    assert_eq!(call(&mut h, O::ImplementationHasFeature, implementation.clone(), &[]), V::Bool(true));
    let doc = node(call(&mut h, O::ImplementationCreateHTMLDocument, implementation.clone(), &[str_value("title")]));
    let children: Vec<_> = h.dom.children(doc).collect();
    assert_eq!(children.len(), 2);
    assert!(matches!(h.dom.get(children[0]).kind, NodeKind::DocumentType { .. }));
    assert_eq!(h.dom.name(children[1]), Some("html"));
    let no_title = node(call(&mut h, O::ImplementationCreateHTMLDocument, implementation.clone(), &[V::Undefined]));
    let html = h.dom.document_element(no_title).unwrap().unwrap();
    let head = h.dom.children(html).next().unwrap();
    assert_eq!(h.dom.children(head).count(), 0);
    let custom = node(call(&mut h, O::ImplementationCreateDocument, implementation, &[str_value("urn:test"), str_value("p:root"), V::Null]));
    let root = h.dom.children(custom).next().unwrap();
    assert_eq!(h.dom.qualified_name(root).as_deref(), Some("p:root"));
    assert_eq!(h.dom.namespace_uri(root), Some("urn:test"));
}

#[test]
fn create_document_null_name_omits_root_and_namespace_sets_mime_type() {
    let mut h = Host::default();
    let implementation = call(&mut h, O::DocumentImplementation, V::Node(NodeIdx(0)), &[]);
    for (uri, mime) in [
        ("http://www.w3.org/1999/xhtml", "application/xhtml+xml"),
        ("http://www.w3.org/2000/svg", "image/svg+xml"),
        ("urn:other", "application/xml"),
    ] {
        let doc = node(call(&mut h, O::ImplementationCreateDocument, implementation.clone(),
            &[str_value(uri), V::Null]));
        assert_eq!(h.dom.document_element(doc).unwrap(), None);
        assert_eq!(h.dom.document_metadata(doc).unwrap().unwrap().content_type, mime);
    }
    let undefined = node(call(&mut h, O::ImplementationCreateDocument, implementation.clone(),
        &[V::Null, V::Undefined]));
    assert_eq!(h.dom.name(h.dom.document_element(undefined).unwrap().unwrap()), Some("undefined"));
    assert!(invoke(&mut h, O::ImplementationCreateDocument, implementation, &[]).is_err());
}

#[test]
fn xhtml_document_create_element_uses_html_namespace_without_lowercasing() {
    let mut host = Host::default();
    let implementation = call(&mut host, O::DocumentImplementation, V::Node(NodeIdx(0)), &[]);
    let document = node(call(&mut host, O::ImplementationCreateDocument, implementation,
        &[str_value("http://www.w3.org/1999/xhtml"), str_value("html")]));
    let element = node(call(&mut host, O::CreateElement, V::Node(document), &[str_value("HeAd")]));
    assert_eq!(host.dom.namespace_uri(element), Some("http://www.w3.org/1999/xhtml"));
    assert_eq!(host.dom.local_name(element), Some("HeAd"));
}

#[test]
fn static_range_is_immutable_and_uses_inherited_boundary_getters() {
    let mut h = Host::default();
    let text = node(call(&mut h, O::CreateText, V::Node(NodeIdx(0)), &[str_value("abcd")]));
    let init = V::Dictionary(vec![
        ("startContainer", V::Node(text)), ("startOffset", V::Number(1.)),
        ("endContainer", V::Node(text)), ("endOffset", V::Number(3.)),
    ]);
    let static_range = call(&mut h, O::StaticRangeConstructor, V::Undefined, &[init]);
    assert_eq!(call(&mut h, O::RangeStartOffset, static_range.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::RangeEndOffset, static_range.clone(), &[]), V::Number(3.));
    assert_eq!(call(&mut h, O::RangeCollapsed, static_range.clone(), &[]), V::Bool(false));
    assert!(invoke(&mut h, O::RangeSetStart, static_range, &[V::Node(text), V::Number(2.)]).is_err());
}

#[test]
fn node_filter_and_dom_exception_constants_match_legacy_values() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    assert!(h.interface_constants.contains(&(Interface::NodeFilter, "SHOW_ALL", u32::MAX)));
    assert!(h.interface_constants.contains(&(Interface::NodeFilter, "FILTER_SKIP", 3)));
    assert_eq!(crate::dom_exception::from_dom_error(DomError::IndexSizeError), Some(("IndexSizeError", 1)));
    assert_eq!(crate::dom_exception::from_dom_error(DomError::TypeError), None);
    assert!(crate::dom_exception::LEGACY_CODES.contains(&("DATA_CLONE_ERR", 25)));
}

#[test]
fn child_nodes_is_canonical_and_live_through_nodelist_dispatch() {
    let mut h = Host::default();
    let parent = element(&mut h);
    let list = call(&mut h, O::ChildNodes, V::Node(parent), &[]);
    assert_eq!(list, call(&mut h, O::ChildNodes, V::Node(parent), &[]));
    assert_eq!(call(&mut h, O::NodeListLength, list.clone(), &[]), V::Number(0.));
    let child = element(&mut h);
    h.dom.pre_insert(child, parent, None).unwrap();
    assert_eq!(call(&mut h, O::NodeListLength, list.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::NodeListItem, list.clone(), &[V::Number(0.)]), V::Node(child));
    assert_eq!(call(&mut h, O::NodeListItem, list, &[V::Number(1.)]), V::Null);
}

#[test]
fn traversal_filter_calls_after_borrow_and_retains_callback_identity() {
    let mut h = Host::default();
    let root = element(&mut h);
    let child = element(&mut h);
    h.dom.pre_insert(child, root, None).unwrap();
    let filter = V::Dictionary(vec![("acceptNode", V::Callable(1))]);
    let iterator = call(&mut h, O::CreateNodeIterator, V::Node(NodeIdx(0)),
        &[V::Node(root), V::Number(u32::MAX as f64), filter.clone()]);
    assert_eq!(call(&mut h, O::TraversalFilter, iterator.clone(), &[]), filter);
    assert_eq!(call(&mut h, O::IteratorNextNode, iterator.clone(), &[]), V::Node(root));
    assert_eq!(call(&mut h, O::IteratorNextNode, iterator.clone(), &[]), V::Node(child));
    assert_eq!(call(&mut h, O::IteratorNextNode, iterator.clone(), &[]), V::Null);
    assert_eq!(call(&mut h, O::IteratorReferenceNode, iterator, &[]), V::Node(child));
    let walker = call(&mut h, O::CreateTreeWalker, V::Node(NodeIdx(0)),
        &[V::Node(root), V::Number(u32::MAX as f64), V::Callable(1)]);
    assert_eq!(call(&mut h, O::WalkerFirstChild, walker.clone(), &[]), V::Node(child));
    assert_eq!(call(&mut h, O::WalkerCurrentNode, walker.clone(), &[]), V::Node(child));
    assert_eq!(call(&mut h, O::WalkerParentNode, walker, &[]), V::Node(root));
}

#[test]
fn traversal_undefined_what_to_show_uses_show_all_default() {
    let mut host = Host::default();
    let root = element(&mut host);
    for op in [O::CreateNodeIterator, O::CreateTreeWalker] {
        let traversal = call(&mut host, op, V::Node(NodeIdx(0)),
            &[V::Node(root), V::Undefined, V::Undefined]);
        assert_eq!(call(&mut host, O::TraversalWhatToShow, traversal, &[]),
            V::Number(u32::MAX as f64));
    }
}

#[test]
fn parent_node_move_before_is_exposed_on_document_element_and_fragment() {
    for interface in [Interface::Document, Interface::Element, Interface::DocumentFragment] {
        let definition = INTERFACES.iter().find(|definition| definition.interface == interface).unwrap();
        assert!(definition.members.iter().any(|member| member.name == "moveBefore"
            && member.operation == O::MoveBefore));
    }
    let mut host = Host::default();
    let parent = element(&mut host);
    let first = host.dom.create_element(parent, "a");
    let second = host.dom.create_element(parent, "b");
    call(&mut host, O::MoveBefore, V::Node(parent), &[V::Node(second), V::Node(first)]);
    assert_eq!(host.dom.children(parent).collect::<Vec<_>>(), vec![second, first]);
}

#[test]
fn adjacent_element_and_text_insert_at_each_tree_position() {
    let mut host = Host::default();
    let parent = element(&mut host);
    let target = host.dom.create_element(parent, "target");
    let before = host.dom.create_element(parent, "before");
    assert_eq!(call(&mut host, O::ElementInsertAdjacentElement, V::Node(target),
        &[str_value("BEFOREBEGIN"), V::Node(before)]), V::Node(before));
    assert_eq!(host.dom.get(target).prev_sibling, Some(before));
    let after = host.dom.create_element(parent, "after");
    call(&mut host, O::ElementInsertAdjacentElement, V::Node(target),
        &[str_value("afterend"), V::Node(after)]);
    assert_eq!(host.dom.get(target).next_sibling, Some(after));
    call(&mut host, O::ElementInsertAdjacentText, V::Node(target),
        &[str_value("afterbegin"), str_value("prefix")]);
    call(&mut host, O::ElementInsertAdjacentText, V::Node(target),
        &[str_value("beforeend"), str_value("suffix")]);
    assert_eq!(host.dom.children(target).map(|node| host.dom.character_data_utf16(node).unwrap())
        .collect::<Vec<_>>(), vec!["prefix".encode_utf16().collect::<Vec<_>>(),
        "suffix".encode_utf16().collect::<Vec<_>>()]);
    assert_eq!(invoke(&mut host, O::ElementInsertAdjacentElement, V::Node(target),
        &[str_value("invalid"), V::Node(before)]), Err("SyntaxError".into()));
    let detached = element(&mut host);
    assert_eq!(call(&mut host, O::ElementInsertAdjacentElement, V::Node(detached),
        &[str_value("beforebegin"), V::Node(before)]), V::Null);
}

#[test]
fn character_data_data_setter_uses_legacy_null_to_empty_string() {
    let mut host = Host::default();
    let text = host.dom.create_node_in(NodeIdx(0), amazing_dom::NodeKind::Text { data: String::new() }).unwrap();
    call(&mut host, O::SetData, V::Node(text), &[V::Null]);
    assert_eq!(call(&mut host, O::Data, V::Node(text), &[]), str_value(""));
    call(&mut host, O::SetData, V::Node(text), &[V::Undefined]);
    assert_eq!(call(&mut host, O::Data, V::Node(text), &[]), str_value("undefined"));
}

#[test]
fn event_target_dispatches_outside_borrow_and_releases_once_listener() {
    let mut h = Host::default();
    let target = call(&mut h, O::EventTargetConstructor, V::Undefined, &[]);
    let event = call(&mut h, O::EventConstructor, V::Undefined,
        &[str_value("ping"), V::Dictionary(vec![("cancelable", V::Bool(true))])]);
    call(&mut h, O::AddEventListener, target.clone(), &[
        str_value("ping"), V::Callable(7), V::Dictionary(vec![("once", V::Bool(true))])]);
    assert_eq!(h.callbacks.iter().filter(|v| v.is_some()).count(), 1);
    assert_eq!(call(&mut h, O::DispatchEvent, target.clone(), &[event.clone()]), V::Bool(true));
    assert_eq!(h.calls, [7]);
    assert_eq!(h.callbacks.iter().filter(|v| v.is_some()).count(), 0);
    assert_eq!(call(&mut h, O::EventTargetValue, event.clone(), &[]), target);
    assert_eq!(call(&mut h, O::EventCurrentTarget, event.clone(), &[]), V::Null);
    call(&mut h, O::EventPreventDefault, event.clone(), &[]);
    assert_eq!(call(&mut h, O::EventDefaultPrevented, event.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::DispatchEvent, target, &[event]), V::Bool(false));
    assert_eq!(h.calls, [7]);
}

#[test]
fn passive_listener_option_preserves_absent_versus_explicit_false() {
    let mut h = Host::default();
    let omitted = V::Dictionary(vec![("passive", V::Undefined)]);
    let explicit = V::Dictionary(vec![("passive", V::Bool(false))]);
    assert_eq!(crate::event_bindings::passive_option(&mut h, Some(&omitted)).unwrap(), None);
    assert_eq!(crate::event_bindings::passive_option(&mut h, Some(&explicit)).unwrap(), Some(false));
}

#[test]
fn image_and_source_reflection_uses_loader_state_and_decode_promise() {
    let mut h = Host::default();
    let root = element(&mut h);
    let image = h.dom.create_element(root, "img");
    let source = h.dom.create_element(root, "source");
    assert_eq!(call(&mut h, O::ImageComplete, V::Node(image), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::ImageDecoding, V::Node(image), &[]), str_value("auto"));
    call(&mut h, O::ImageSetDecoding, V::Node(image), &[str_value("ASYNC")]);
    assert_eq!(call(&mut h, O::ImageDecoding, V::Node(image), &[]), str_value("async"));
    call(&mut h, O::ImageSetSrcset, V::Node(image), &[str_value("a.png 1x, b.png 2x")]);
    assert_eq!(call(&mut h, O::ImageSrcset, V::Node(image), &[]), str_value("a.png 1x, b.png 2x"));
    assert_eq!(call(&mut h, O::ImageComplete, V::Node(image), &[]), V::Bool(false));
    call(&mut h, O::SourceSetMedia, V::Node(source), &[str_value("(min-width: 40em)")]);
    call(&mut h, O::SourceSetSrcset, V::Node(source), &[str_value("wide.png")]);
    assert_eq!(call(&mut h, O::SourceMedia, V::Node(source), &[]), str_value("(min-width: 40em)"));
    assert_eq!(call(&mut h, O::SourceSrcset, V::Node(source), &[]), str_value("wide.png"));
    h.image_state = Some(ImageLoaderState {
        current_src: "https://example.test/b.png".encode_utf16().collect(),
        complete: true, natural_width: 640, natural_height: 480,
    });
    assert_eq!(call(&mut h, O::ImageCurrentSrc, V::Node(image), &[]), str_value("https://example.test/b.png"));
    assert_eq!(call(&mut h, O::ImageComplete, V::Node(image), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::ImageNaturalWidth, V::Node(image), &[]), V::Number(640.));
    assert_eq!(call(&mut h, O::ImageNaturalHeight, V::Node(image), &[]), V::Number(480.));
    let promise = call(&mut h, O::ImageDecode, V::Node(image), &[]);
    assert_eq!(promise, V::Promise(0));
    assert_eq!(h.promises[0].1, Some(V::Undefined));
    assert_eq!(h.image_decode_requested, [image]);
}

#[test]
fn mutation_observer_records_are_snapshots_and_callback_survives_disconnect() {
    let mut h = Host::default();
    let root = element(&mut h);
    let observer = call(&mut h, O::MutationObserverConstructor, V::Undefined, &[V::Callable(7)]);
    let options = V::Dictionary(vec![("childList", V::Bool(true))]);
    call(&mut h, O::MutationObserve, observer.clone(), &[V::Node(root), options.clone()]);
    let child = element(&mut h);
    h.dom.pre_insert(child, root, None).unwrap();
    let records = call(&mut h, O::MutationTakeRecords, observer.clone(), &[]);
    let V::Array(records) = records else { panic!("expected records array") };
    assert_eq!(records.len(), 1);
    assert_eq!(call(&mut h, O::MutationRecordType, records[0].clone(), &[]), str_value("childList"));
    let added = call(&mut h, O::MutationRecordAddedNodes, records[0].clone(), &[]);
    assert_eq!(added, call(&mut h, O::MutationRecordAddedNodes, records[0].clone(), &[]));
    assert_eq!(call(&mut h, O::NodeListLength, added.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::NodeListItem, added, &[V::Number(0.)]), V::Node(child));
    call(&mut h, O::MutationDisconnect, observer.clone(), &[]);
    assert_eq!(h.callbacks.iter().filter(|v| v.is_some()).count(), 1);
    call(&mut h, O::MutationObserve, observer.clone(), &[V::Node(root), options]);
    let second = element(&mut h);
    h.dom.pre_insert(second, root, None).unwrap();
    crate::deliver_mutation_checkpoint(&mut h).unwrap();
    assert_eq!(h.calls, [7]);
    assert_eq!(h.call_receivers.last().unwrap().0, observer);
    assert_eq!(h.call_receivers.last().unwrap().1.len(), 2);
    assert_eq!(call(&mut h, O::MutationTakeRecords, observer, &[]), V::Array(vec![]));
}

#[test]
fn take_records_retains_each_attribute_child_list_and_character_data_mutation() {
    let mut h = Host::default();
    let target = element(&mut h);
    call(&mut h, O::ElementSetId, V::Node(target), &[str_value("n00")]);
    let observer = call(&mut h, O::MutationObserverConstructor, V::Undefined, &[V::Callable(7)]);
    let options = V::Dictionary(vec![
        ("subtree", V::Bool(true)), ("childList", V::Bool(true)),
        ("attributes", V::Bool(true)), ("characterData", V::Bool(true)),
        ("attributeOldValue", V::Bool(true)), ("characterDataOldValue", V::Bool(true)),
    ]);
    call(&mut h, O::MutationObserve, observer.clone(), &[V::Node(target), options]);
    for id in ["foo", "bar"] {
        call(&mut h, O::ElementSetId, V::Node(target), &[str_value(id)]);
    }
    call(&mut h, O::ElementSetClassName, V::Node(target), &[str_value("bar")]);
    call(&mut h, O::SetTextContent, V::Node(target), &[str_value("old data")]);
    let text = h.dom.get(target).first_child.unwrap();
    call(&mut h, O::SetData, V::Node(text), &[str_value("new data")]);
    let V::Array(records) = call(&mut h, O::MutationTakeRecords, observer.clone(), &[]) else {
        panic!("expected records array")
    };
    let types: Vec<_> = records.iter().map(|record|
        call(&mut h, O::MutationRecordType, record.clone(), &[])).collect();
    let names: Vec<_> = records.iter().map(|record|
        call(&mut h, O::MutationRecordAttributeName, record.clone(), &[])).collect();
    assert_eq!(records.len(), 5);
    assert_eq!(types, vec![str_value("attributes"), str_value("attributes"),
        str_value("attributes"), str_value("childList"), str_value("characterData")]);
    assert_eq!(names, vec![str_value("id"), str_value("id"), str_value("class"),
        V::Null, V::Null]);
    assert_eq!(call(&mut h, O::MutationRecordOldValue, records[0].clone(), &[]), str_value("n00"));
    assert_eq!(call(&mut h, O::MutationRecordOldValue, records[1].clone(), &[]), str_value("foo"));
    assert_eq!(call(&mut h, O::MutationRecordOldValue, records[4].clone(), &[]), str_value("old data"));
    assert_eq!(call(&mut h, O::MutationTakeRecords, observer, &[]), V::Array(vec![]));
}

#[test]
fn dom_exception_preserves_code_units_and_maps_native_error_names() {
    let mut h = Host::default();
    let message = V::String(vec![0xD800, 0x61]);
    let exception = call(&mut h, O::DOMExceptionConstructor, V::Undefined,
        &[message.clone(), str_value("NotFoundError")]);
    assert_eq!(call(&mut h, O::DOMExceptionMessage, exception.clone(), &[]), message);
    assert_eq!(call(&mut h, O::DOMExceptionName, exception.clone(), &[]), str_value("NotFoundError"));
    assert_eq!(call(&mut h, O::DOMExceptionCode, exception, &[]), V::Number(8.));
    assert_eq!(crate::dom_exception::legacy_code(&[0xD800]), 0);
    let thrown = crate::dom_exception::throw_dom_error(&mut h, DomError::IndexSizeError);
    assert!(thrown.starts_with("throw: Platform("));
}

#[test]
fn custom_event_detail_is_rooted_and_replaced_without_dom_borrow() {
    let mut h = Host::default();
    let detail = V::Array(vec![V::Number(3.)]);
    let event = call(&mut h, O::CustomEventConstructor, V::Undefined,
        &[str_value("ready"), V::Dictionary(vec![("detail", detail.clone()), ("bubbles", V::Bool(true))])]);
    assert_eq!(call(&mut h, O::CustomEventDetail, event.clone(), &[]), detail);
    assert_eq!(call(&mut h, O::EventBubbles, event.clone(), &[]), V::Bool(true));
    assert_eq!(h.callbacks.iter().filter(|v| v.is_some()).count(), 1);
    let second = V::Array(vec![V::Number(4.)]);
    call(&mut h, O::CustomEventInit, event.clone(), &[str_value("again"), V::Bool(false), V::Bool(true), second.clone()]);
    assert_eq!(call(&mut h, O::CustomEventDetail, event.clone(), &[]), second);
    assert_eq!(call(&mut h, O::EventType, event.clone(), &[]), str_value("again"));
    assert_eq!(h.callbacks.iter().filter(|v| v.is_some()).count(), 1);
    let plain = call(&mut h, O::EventConstructor, V::Undefined, &[str_value("plain")]);
    assert_eq!(invoke(&mut h, O::CustomEventDetail, plain, &[]),
        Err("TypeError: Expected a CustomEvent".into()));
    let legacy = call(&mut h, O::CreateEvent, V::Node(NodeIdx(0)), &[str_value("CustomEvent")]);
    assert_eq!(call(&mut h, O::CustomEventDetail, legacy.clone(), &[]), V::Null);
    let target = call(&mut h, O::EventTargetConstructor, V::Undefined, &[]);
    assert_eq!(invoke(&mut h, O::DispatchEvent, target.clone(), &[legacy.clone()]),
        Err("InvalidStateError".into()));
    call(&mut h, O::CustomEventInit, legacy.clone(), &[str_value("ready")]);
    assert_eq!(call(&mut h, O::DispatchEvent, target, &[legacy]), V::Bool(true));
}

#[test]
fn event_subclass_constructors_preserve_dictionary_fields_and_defaults() {
    let mut h = Host::default();
    let mouse = call(&mut h, O::MouseEventConstructor, V::Undefined, &[
        str_value("click"), V::Dictionary(vec![
            ("bubbles", V::Bool(true)), ("clientX", V::Number(40.)),
            ("ctrlKey", V::Bool(true)), ("button", V::Number(1.)),
        ]),
    ]);
    assert_eq!(call(&mut h, O::EventBubbles, mouse.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::EventExtra("clientX"), mouse.clone(), &[]), V::Number(40.));
    assert_eq!(call(&mut h, O::EventExtra("ctrlKey"), mouse.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::EventExtra("button"), mouse.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::EventExtra("relatedTarget"), mouse, &[]), V::Null);
    let keyboard = call(&mut h, O::KeyboardEventConstructor, V::Undefined, &[str_value("keydown")]);
    assert_eq!(call(&mut h, O::EventExtra("key"), keyboard.clone(), &[]), str_value(""));
    assert_eq!(call(&mut h, O::EventExtra("repeat"), keyboard, &[]), V::Bool(false));
}

#[test]
fn cdata_inherits_text_and_preserves_utf16_only_in_xml_document() {
    let mut h = Host::default();
    let xml = node(call(&mut h, O::DocumentConstructor, V::Undefined, &[]));
    let units = V::String(vec![0xD800, 0x61]);
    let cdata = call(&mut h, O::CreateCDATASection, V::Node(xml), &[units.clone()]);
    assert_eq!(call(&mut h, O::NodeType, cdata.clone(), &[]), V::Number(4.));
    assert_eq!(call(&mut h, O::NodeName, cdata.clone(), &[]), str_value("#cdata-section"));
    assert_eq!(call(&mut h, O::Data, cdata.clone(), &[]), units);
    assert_eq!(call(&mut h, O::Length, cdata, &[]), V::Number(2.));
    assert_eq!(invoke(&mut h, O::CreateCDATASection, V::Node(NodeIdx(0)), &[str_value("x")]),
        Err("NotSupportedError".into()));
}

#[test]
fn fallible_host_string_propagates_lone_surrogate_error() {
    let mut h = Host::default();
    let text = node(call(&mut h, O::CreateText, V::Node(NodeIdx(0)),
        &[V::String(vec![0xD800])]));
    h.reject_lone_surrogate = true;
    assert_eq!(invoke(&mut h, O::NodeValue, V::Node(text), &[]),
        Err("TypeError: cannot represent lone surrogate".into()));
    assert_eq!(h.dom.character_data_utf16(text), Some(vec![0xD800]));
}

#[test]
fn attr_and_named_node_map_keep_identity_and_utf16_value() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    let xml = node(call(&mut h, O::DocumentConstructor, V::Undefined, &[]));
    let element = call(&mut h, O::CreateElementNS, V::Node(xml),
        &[str_value("urn:test"), str_value("root")]);
    let attr = call(&mut h, O::CreateAttributeNS, V::Node(xml),
        &[str_value("urn:attr"), str_value("p:name")]);
    let units = V::String(vec![0xD800, 0x61]);
    call(&mut h, O::AttrSetValue, attr.clone(), &[units.clone()]);
    let map = call(&mut h, O::ElementAttributes, element.clone(), &[]);
    assert_eq!(map, call(&mut h, O::ElementAttributes, element.clone(), &[]));
    assert_eq!(call(&mut h, O::NamedNodeMapSetNamedItemNS, map.clone(), &[attr.clone()]), V::Null);
    assert_eq!(call(&mut h, O::NamedNodeMapLength, map.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::NamedNodeMapItem, map.clone(), &[V::Number(0.)]), attr);
    assert_eq!(mock_legacy_keys(&mut h, map.clone()), vec!["0", "p:name"]);
    assert_eq!(mock_legacy_get(&mut h, map.clone(), "0"), attr);
    assert_eq!(mock_legacy_get(&mut h, map.clone(), "p:name"), attr);
    assert_eq!(call(&mut h, O::ElementGetAttributeNS, element.clone(),
        &[str_value("urn:attr"), str_value("name")]), units);
    assert_eq!(call(&mut h, O::AttrOwnerElement, attr.clone(), &[]), element);
    assert_eq!(call(&mut h, O::NamedNodeMapRemoveNamedItemNS, map,
        &[str_value("urn:attr"), str_value("name")]), attr.clone());
    assert_eq!(call(&mut h, O::AttrOwnerElement, attr.clone(), &[]), V::Null);
    assert_eq!(call(&mut h, O::AttrValue, attr, &[]), V::String(vec![0xD800, 0x61]));
}

// dom/nodes/Element-hasAttribute.html and Element-hasAttributes.html at
// c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
#[test]
fn element_attribute_queries_toggle_and_name_validation_follow_dom_rules() {
    let mut h = Host::default();
    let element = h.dom.create_element(NodeIdx(0), "div");
    let value = V::Node(element);
    assert_eq!(call(&mut h, O::ElementHasAttributes, value.clone(), &[]), V::Bool(false));
    assert_eq!(call(&mut h, O::ElementHasAttribute, value.clone(), &[str_value("X")]), V::Bool(false));
    assert_eq!(call(&mut h, O::ElementToggleAttribute, value.clone(), &[str_value("DATA-X")]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementHasAttribute, value.clone(), &[str_value("data-x")]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementHasAttributes, value.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementGetAttributeNames, value.clone(), &[]),
        V::Array(vec![str_value("data-x")]));
    assert_eq!(call(&mut h, O::ElementToggleAttribute, value.clone(),
        &[str_value("data-x"), V::Bool(true)]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementToggleAttribute, value.clone(),
        &[str_value("data-x"), V::Bool(false)]), V::Bool(false));
    assert_eq!(call(&mut h, O::ElementHasAttributes, value.clone(), &[]), V::Bool(false));
    assert_eq!(invoke(&mut h, O::ElementToggleAttribute, value.clone(), &[str_value("bad name")]),
        Err("InvalidCharacterError".into()));
    assert_eq!(invoke(&mut h, O::ElementSetAttribute, value.clone(),
        &[str_value("bad name"), str_value("x")]), Err("InvalidCharacterError".into()));
    call(&mut h, O::ElementSetAttributeNS, value.clone(),
        &[str_value("urn:test"), str_value("p:name"), str_value("value")]);
    assert_eq!(call(&mut h, O::ElementHasAttribute, value.clone(), &[str_value("p:name")]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementHasAttributeNS, value.clone(),
        &[str_value("urn:test"), str_value("name")]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementHasAttributeNS, value.clone(),
        &[V::Null, str_value("name")]), V::Bool(false));
    assert_eq!(call(&mut h, O::ElementGetAttributeNames, value.clone(), &[]),
        V::Array(vec![str_value("p:name")]));
    assert_eq!(call(&mut h, O::ElementToggleAttribute, value.clone(), &[str_value("p:name")]),
        V::Bool(false));
    assert_eq!(call(&mut h, O::ElementHasAttributes, value, &[]), V::Bool(false));
    let ns_element = h.dom.create_element(NodeIdx(0), "p");
    let ns_value = V::Node(ns_element);
    call(&mut h, O::ElementSetAttributeNS, ns_value.clone(),
        &[str_value("foo"), str_value("x"), str_value("first")]);
    call(&mut h, O::ElementSetAttributeNS, ns_value.clone(),
        &[str_value("foo2"), str_value("x"), str_value("second")]);
    call(&mut h, O::ElementSetAttribute, ns_value.clone(),
        &[str_value("x"), str_value("changed")]);
    assert_eq!(call(&mut h, O::ElementGetAttribute, ns_value.clone(), &[str_value("x")]), str_value("changed"));
    assert_eq!(call(&mut h, O::ElementGetAttributeNS, ns_value.clone(),
        &[str_value("foo"), str_value("x")]), str_value("changed"));
    assert_eq!(call(&mut h, O::ElementGetAttributeNS, ns_value.clone(),
        &[str_value("foo2"), str_value("x")]), str_value("second"));
    let map = call(&mut h, O::ElementAttributes, ns_value.clone(), &[]);
    assert_eq!(call(&mut h, O::NamedNodeMapLength, map, &[]), V::Number(2.0));
    assert_eq!(call(&mut h, O::ElementHasAttributes, ns_value, &[]), V::Bool(true));
    let svg = call(&mut h, O::CreateElementNS, V::Node(NodeIdx(0)),
        &[str_value("http://www.w3.org/2000/svg"), str_value("svg")]);
    call(&mut h, O::ElementSetAttribute, svg.clone(), &[str_value("DATA-X"), str_value("yes")]);
    assert_eq!(call(&mut h, O::ElementHasAttribute, svg.clone(), &[str_value("DATA-X")]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementHasAttribute, svg, &[str_value("data-x")]), V::Bool(false));
}

#[test]
fn attribute_names_and_named_node_map_share_cross_namespace_insertion_order() {
    let mut h = Host::default();
    let element = V::Node(h.dom.create_element(NodeIdx(0), "div"));
    call(&mut h, O::ElementSetAttributeNS, element.clone(),
        &[str_value("urn:a"), str_value("a:first"), str_value("1")]);
    call(&mut h, O::ElementSetAttribute, element.clone(),
        &[str_value("plain"), str_value("2")]);
    call(&mut h, O::ElementSetAttributeNS, element.clone(),
        &[str_value("urn:b"), str_value("b:last"), str_value("3")]);
    assert_eq!(call(&mut h, O::ElementGetAttributeNames, element.clone(), &[]),
        V::Array(vec![str_value("a:first"), str_value("plain"), str_value("b:last")]));
    let map = call(&mut h, O::ElementAttributes, element, &[]);
    for (index, name) in ["a:first", "plain", "b:last"].iter().enumerate() {
        let attr = call(&mut h, O::NamedNodeMapItem, map.clone(), &[V::Number(index as f64)]);
        assert_eq!(call(&mut h, O::AttrName, attr, &[]), str_value(name));
    }
}

#[test]
fn dom_parser_creates_inert_documents_and_xml_error_document() {
    let mut h = Host::default();
    assert!(INTERFACES.iter().any(|definition| definition.interface == Interface::XMLDocument
        && definition.parent == Some(Interface::Document)));
    let parser = call(&mut h, O::DOMParserConstructor, V::Undefined, &[]);
    let html = call(&mut h, O::DOMParserParseFromString, parser.clone(),
        &[str_value("<p>hello</p>"), str_value("text/html")]);
    let document = node(html);
    let metadata = h.dom.document_metadata(document).unwrap().unwrap();
    assert_eq!(metadata.url, "https://example.test/page");
    assert_eq!(metadata.content_type, "text/html");
    assert_eq!(metadata.character_set, "UTF-8");
    assert_eq!(call(&mut h, O::DocumentURL, V::Node(document), &[]), str_value("https://example.test/page"));
    assert_eq!(call(&mut h, O::DocumentContentType, V::Node(document), &[]), str_value("text/html"));
    assert_eq!(h.dom.document_ready_state(document).unwrap().as_str(), "complete");
    assert_eq!(h.dom.local_name(h.dom.document_element(document).unwrap().unwrap()), Some("html"));
    let xml_error = node(call(&mut h, O::DOMParserParseFromString, parser.clone(),
        &[str_value("<bad"), str_value("application/xml")]));
    assert_eq!(h.dom.local_name(h.dom.document_element(xml_error).unwrap().unwrap()), Some("parsererror"));
    assert_eq!(invoke(&mut h, O::DOMParserParseFromString, parser,
        &[str_value("x"), str_value("text/plain")]), Err("TypeError: Unsupported DOMParser content type".into()));
}

/// `Document.parseHTMLUnsafe` is static and routes through the same
/// `create_inert_document("text/html", ...)` primitive `DOMParser.
/// parseFromString` already uses -- see `parser_bindings.rs`'s doc comment
/// for why this needed no new parser support, unlike `ElementSetHTMLUnsafe`.
#[test]
fn document_parse_html_unsafe_creates_an_inert_html_document() {
    let mut h = Host::default();
    let document = node(call(&mut h, O::DocumentParseHTMLUnsafe, V::Undefined,
        &[str_value("<template shadowrootmode=open><p>hi</p></template>")]));
    let metadata = h.dom.document_metadata(document).unwrap().unwrap();
    assert_eq!(metadata.content_type, "text/html");
    assert_eq!(h.dom.local_name(h.dom.document_element(document).unwrap().unwrap()), Some("html"));
    assert_eq!(invoke(&mut h, O::DocumentParseHTMLUnsafe, V::Undefined, &[]),
        Err("TypeError: Not enough arguments".into()));
}

#[test]
fn processing_instruction_and_range_point_queries_preserve_units() {
    let mut h = Host::default();
    let xml = node(call(&mut h, O::DocumentConstructor, V::Undefined, &[]));
    let pi = call(&mut h, O::CreateProcessingInstruction, V::Node(xml),
        &[str_value("p"), V::String(vec![0xD800])]);
    assert_eq!(call(&mut h, O::NodeType, pi.clone(), &[]), V::Number(7.));
    assert_eq!(call(&mut h, O::Data, pi.clone(), &[]), V::String(vec![0xD800]));
    assert_eq!(call(&mut h, O::ProcessingInstructionTarget, pi, &[]), str_value("p"));
    let text = node(call(&mut h, O::CreateText, V::Node(xml), &[str_value("abcd")]));
    let range = call(&mut h, O::RangeConstructor, V::Undefined, &[]);
    call(&mut h, O::RangeSetStart, range.clone(), &[V::Node(text), V::Number(1.)]);
    call(&mut h, O::RangeSetEnd, range.clone(), &[V::Node(text), V::Number(3.)]);
    assert_eq!(call(&mut h, O::RangeIsPointInRange, range.clone(), &[V::Node(text), V::Number(2.)]), V::Bool(true));
    assert_eq!(call(&mut h, O::RangeComparePoint, range.clone(), &[V::Node(text), V::Number(0.)]), V::Number(-1.));
    assert_eq!(call(&mut h, O::RangeStringifier, range, &[]), str_value("bc"));
}

#[test]
fn installation_orders_parent_interfaces_and_preserves_constant_values() {
    let mut h = Host::default();
    let report = install(&mut h).unwrap();
    assert!(report.not_installed.is_empty());
    assert!(h.custom_elements_global);
    assert_eq!(h.class_strings,
        INTERFACES.iter().map(|definition| (definition.interface, definition.name)).collect::<Vec<_>>());
    assert!(h.dom_exception_error_prototype);
    for d in INTERFACES {
        if let Some(parent) = d.parent {
            assert!(
                h.installed.iter().position(|i| *i == parent)
                    < h.installed.iter().position(|i| *i == d.interface)
            );
        }
    }
    assert!(h.constants.contains(&("DOCUMENT_FRAGMENT_NODE", 11)));
    assert!(h
        .constants
        .contains(&("DOCUMENT_POSITION_CONTAINED_BY", 16)));
}

#[test]
fn unsupported_optional_hooks_are_recorded_without_aborting_installation() {
    let mut h = Host { skip_optional_hooks: true, ..Host::default() };
    let report = install(&mut h).unwrap();
    assert_eq!(h.installed.len(), INTERFACES.len());
    assert!(h.class_strings.is_empty());
    assert!(!h.dom_exception_error_prototype);
    assert_eq!(report.not_installed.len(), INTERFACES.len() + 5);
    assert_eq!(report.not_installed[0], crate::OptionalCapability::InterfaceClassString(INTERFACES[0].interface));
    assert_eq!(report.not_installed[INTERFACES.len()], crate::OptionalCapability::DomExceptionErrorPrototype);
    assert_eq!(report.not_installed[INTERFACES.len() + 1], crate::OptionalCapability::CustomElementsGlobal);
    assert_eq!(report.not_installed[INTERFACES.len() + 2], crate::OptionalCapability::PairIterable(Interface::URLSearchParams));
    assert_eq!(report.not_installed[INTERFACES.len() + 3], crate::OptionalCapability::PairIterable(Interface::FormData));
    assert_eq!(report.not_installed.last(), Some(&crate::OptionalCapability::CssomGlobal));
    assert!(h.constants.contains(&("DOCUMENT_FRAGMENT_NODE", 11)));
}

#[test]
fn optional_hook_installation_errors_remain_fatal() {
    let mut h = Host { fail_class_string: Some(INTERFACES[0].interface), ..Host::default() };
    assert_eq!(install(&mut h), Err("class string install failed".into()));
}

#[test]
fn webidl_stringifiers_are_installed_on_declaring_prototypes() {
    for (interface, operation) in [
        (Interface::HTMLAnchorElement, O::ElementHref),
        (Interface::HTMLAreaElement, O::ElementHref),
        (Interface::DOMTokenList, O::TokenListValue),
        (Interface::Range, O::RangeStringifier),
    ] {
        let definition = INTERFACES.iter().find(|d| d.interface == interface).unwrap();
        assert!(definition.members.iter().any(|member|
            member.name == "toString" && member.operation == operation &&
            matches!(member.kind, crate::host::MemberKind::Method { length: 0 })));
    }
    let exception = INTERFACES.iter().find(|d| d.interface == Interface::DOMException).unwrap();
    assert!(!exception.members.iter().any(|member| member.name == "toString"));
}

#[test]
fn shadow_root_and_slot_bindings_keep_identity_and_hide_closed_mode() {
    let mut h = Host::default();
    let host = h.dom.create_element(NodeIdx(0), "div");
    let light = h.dom.create_element(host, "span");
    let init = V::Dictionary(vec![("mode", str_value("open")),
        ("delegatesFocus", V::Bool(true)), ("slotAssignment", str_value("named")),
        ("clonable", V::Bool(true)), ("serializable", V::Bool(true))]);
    let root = call(&mut h, O::ElementAttachShadow, V::Node(host), &[init]);
    assert_eq!(call(&mut h, O::ElementShadowRoot, V::Node(host), &[]), root);
    assert_eq!(call(&mut h, O::ShadowHost, root.clone(), &[]), V::Node(host));
    assert_eq!(call(&mut h, O::ShadowMode, root.clone(), &[]), str_value("open"));
    assert_eq!(call(&mut h, O::ShadowDelegatesFocus, root.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::ShadowSlotAssignment, root.clone(), &[]), str_value("named"));
    assert_eq!(call(&mut h, O::ShadowClonable, root.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::ShadowSerializable, root.clone(), &[]), V::Bool(true));
    let slot = h.dom.create_element(node(root.clone()), "slot");
    assert_eq!(call(&mut h, O::SlotAssignedNodes, V::Node(slot), &[]), V::Array(vec![V::Node(light)]));
    assert_eq!(call(&mut h, O::ElementAssignedSlot, V::Node(light), &[]), V::Node(slot));
    assert_eq!(call(&mut h, O::GetRootNode, V::Node(slot), &[]), root.clone());
    assert_eq!(call(&mut h, O::GetRootNode, V::Node(slot),
        &[V::Dictionary(vec![("composed", V::Bool(true))])]), V::Node(NodeIdx(0)));
    call(&mut h, O::ElementSetInnerHTML, root.clone(), &[str_value("<b>new</b>")]);
    assert_eq!(call(&mut h, O::ElementInnerHTML, root, &[]), str_value("<b>new</b>"));
    let closed_host = h.dom.create_element(NodeIdx(0), "div");
    let closed_child = h.dom.create_element(closed_host, "i");
    let closed = call(&mut h, O::ElementAttachShadow, V::Node(closed_host),
        &[V::Dictionary(vec![("mode", str_value("closed"))])]);
    let closed_slot = h.dom.create_element(node(closed), "slot");
    assert_eq!(call(&mut h, O::ElementShadowRoot, V::Node(closed_host), &[]), V::Null);
    assert_eq!(call(&mut h, O::ElementAssignedSlot, V::Node(closed_child), &[]), V::Null);
    assert_eq!(call(&mut h, O::SlotAssignedNodes, V::Node(closed_slot), &[]),
        V::Array(vec![V::Node(closed_child)]));
}

#[test]
fn slotchange_uses_mutation_checkpoint_and_runs_listener_after_dom_borrow() {
    let mut h = Host::default();
    let host = h.dom.create_element(NodeIdx(0), "div");
    let root = h.dom.attach_shadow(host, amazing_dom::shadow::ShadowOptions {
        mode: amazing_dom::shadow::ShadowMode::Open, delegates_focus: false,
        slot_assignment: amazing_dom::shadow::SlotAssignment::Named,
        clonable: false, serializable: false,
    }).unwrap();
    let slot = h.dom.create_element(root, "slot");
    call(&mut h, O::AddEventListener, V::Node(slot),
        &[str_value("slotchange"), V::Callable(27)]);
    h.dom.create_element(host, "span");
    assert!(h.dom.mutation_microtask_pending());
    deliver_mutation_checkpoint(&mut h).unwrap();
    assert_eq!(h.calls, vec![27]);
    assert!(!h.dom.mutation_microtask_pending());
}

#[test]
fn constructor_defaults_differ_from_required_factory_arguments_and_preserve_units() {
    let mut h = Host::default();
    let a = node(call(
        &mut h,
        O::CommentConstructor,
        V::Undefined,
        &[V::Undefined],
    ));
    assert_eq!(h.dom.character_data_utf16(a), Some(vec![]));
    let b = node(call(
        &mut h,
        O::CommentConstructor,
        V::Undefined,
        &[V::Null],
    ));
    assert_eq!(call(&mut h, O::Data, V::Node(b), &[]), str_value("null"));
    assert!(invoke(&mut h, O::CreateText, V::Node(NodeIdx(0)), &[]).is_err());
    let text = V::String(vec![0xD800, 0xD83D, 0xDE00]);
    let c = call(&mut h, O::CreateText, V::Node(NodeIdx(0)), &[text.clone()]);
    assert_eq!(call(&mut h, O::NodeValue, c.clone(), &[]), text);
    assert_eq!(call(&mut h, O::Length, c.clone(), &[]), V::Number(3.));
    assert_eq!(call(&mut h, O::OwnerDocument, c, &[]), V::Node(NodeIdx(0)));
}

#[test]
fn argument_coercion_finishes_before_any_tree_mutation_and_propagates_throw() {
    let mut h = Host::default();
    let parent = element(&mut h);
    let child = element(&mut h);
    h.dom.pre_insert(child, parent, None).unwrap();
    let error = invoke(
        &mut h,
        O::Append,
        V::Node(parent),
        &[V::Node(child), V::Coerce("first"), V::Throw],
    );
    assert_eq!(error, Err("coercion sentinel".into()));
    assert_eq!(h.conversions, ["first"]);
    assert_eq!(h.dom.children(parent).collect::<Vec<_>>(), [child]);
    assert_eq!(h.dom.node_count(), 3);
}

#[test]
fn wrong_receiver_is_rejected_before_user_coercion() {
    let mut h = Host::default();
    let text = call(&mut h, O::TextConstructor, V::Undefined, &[]);
    assert_eq!(
        invoke(&mut h, O::Append, text, &[V::Throw]),
        Err("TypeError".into())
    );
    assert!(invoke(&mut h, O::CreateComment, V::Null, &[V::Throw])
        .unwrap_err()
        .starts_with("TypeError:"));
    assert!(h.conversions.is_empty());
}

#[test]
fn detached_child_still_coerces_arguments_but_does_not_allocate_text() {
    let mut h = Host::default();
    let target = element(&mut h);
    let count = h.dom.node_count();
    call(
        &mut h,
        O::Before,
        V::Node(target),
        &[V::Coerce("a"), V::Coerce("b")],
    );
    assert_eq!(h.conversions, ["a", "b"]);
    assert_eq!(h.dom.node_count(), count);
}

#[test]
fn mutation_return_values_and_links_keep_node_identity() {
    let mut h = Host::default();
    let parent = element(&mut h);
    let child = call(
        &mut h,
        O::CommentConstructor,
        V::Undefined,
        &[str_value("x")],
    );
    assert_eq!(
        call(&mut h, O::AppendChild, V::Node(parent), &[child.clone()]),
        child
    );
    assert_eq!(call(&mut h, O::FirstChild, V::Node(parent), &[]), child);
    assert_eq!(
        call(&mut h, O::ParentElement, child.clone(), &[]),
        V::Node(parent)
    );
    assert_eq!(
        call(&mut h, O::RemoveChild, V::Node(parent), &[child.clone()]),
        child
    );
    assert_eq!(call(&mut h, O::ParentNode, child, &[]), V::Null);
}

#[test]
fn text_content_excludes_comments_and_setter_replaces_children() {
    let mut h = Host::default();
    let parent = element(&mut h);
    let comment = call(
        &mut h,
        O::CommentConstructor,
        V::Undefined,
        &[str_value("excluded")],
    );
    call(
        &mut h,
        O::Append,
        V::Node(parent),
        &[str_value("a"), comment, str_value("b")],
    );
    assert_eq!(
        call(&mut h, O::TextContent, V::Node(parent), &[]),
        str_value("ab")
    );
    let units = V::String(vec![0xDFFF]);
    call(&mut h, O::SetTextContent, V::Node(parent), &[units.clone()]);
    assert_eq!(call(&mut h, O::TextContent, V::Node(parent), &[]), units);
    call(&mut h, O::SetTextContent, V::Node(parent), &[V::Null]);
    assert_eq!(
        call(&mut h, O::HasChildNodes, V::Node(parent), &[]),
        V::Bool(false)
    );
}

#[test]
fn whole_text_joins_adjacent_text_and_cdata_until_a_non_text_sibling() {
    let mut h = Host::default();
    let parent = element(&mut h);
    let first = node(call(&mut h, O::CreateText, V::Node(NodeIdx(0)), &[str_value("one")]));
    let cdata = h.dom.create_node_in(NodeIdx(0), NodeKind::CDATASection { data: "two".into() }).unwrap();
    let third = node(call(&mut h, O::CreateText, V::Node(NodeIdx(0)), &[str_value("three")]));
    h.dom.pre_insert(first, parent, None).unwrap();
    h.dom.pre_insert(cdata, parent, None).unwrap();
    h.dom.pre_insert(third, parent, None).unwrap();
    assert_eq!(call(&mut h, O::TextWholeText, V::Node(third), &[]), str_value("onetwothree"));
    let comment = h.dom.create_node_in(NodeIdx(0), NodeKind::Comment { data: String::new() }).unwrap();
    h.dom.pre_insert(comment, parent, Some(third)).unwrap();
    assert_eq!(call(&mut h, O::TextWholeText, V::Node(third), &[]), str_value("three"));
}

#[test]
fn is_equal_node_compares_data_attributes_and_descendants() {
    let mut h = Host::default();
    let first = element(&mut h);
    h.dom.set_attribute(first, "id", "same").unwrap();
    h.dom.create_text(first, "child");
    let second = h.dom.clone_node(first, true).unwrap();
    assert_eq!(call(&mut h, O::NodeIsEqualNode, V::Node(first), &[V::Node(second)]), V::Bool(true));
    assert_eq!(call(&mut h, O::NodeIsEqualNode, V::Node(first), &[V::Null]), V::Bool(false));
    h.dom.set_attribute(second, "id", "different").unwrap();
    assert_eq!(call(&mut h, O::NodeIsEqualNode, V::Node(first), &[V::Node(second)]), V::Bool(false));
}

#[test]
fn compare_document_position_exposes_dom_tree_order() {
    let mut h = Host::default();
    let parent = element(&mut h);
    let first = h.dom.create_element(parent, "a");
    let second = h.dom.create_element(parent, "b");
    assert_eq!(call(&mut h, O::NodeCompareDocumentPosition, V::Node(first), &[V::Node(second)]),
        V::Number(4.));
    assert_eq!(call(&mut h, O::NodeCompareDocumentPosition, V::Node(parent), &[V::Node(first)]),
        V::Number(20.));
}

#[test]
fn selection_uses_the_same_live_range_after_text_mutation() {
    let mut h = Host::default();
    let root = element(&mut h);
    let text = h.dom.create_text(root, "abc");
    let range = call(&mut h, O::CreateRange, V::Node(NodeIdx(0)), &[]);
    call(&mut h, O::RangeSetStart, range.clone(), &[V::Node(text), V::Number(1.)]);
    call(&mut h, O::RangeSetEnd, range.clone(), &[V::Node(text), V::Number(3.)]);
    let selection = call(&mut h, O::GetSelection, V::Node(NodeIdx(0)), &[]);
    assert_eq!(call(&mut h, O::SelectionRangeCount, selection.clone(), &[]), V::Number(0.));
    call(&mut h, O::SelectionAddRange, selection.clone(), &[range.clone()]);
    assert_eq!(call(&mut h, O::SelectionGetRangeAt, selection.clone(), &[V::Number(0.)]), range);
    h.dom.replace_character_data(text, 0, 0, &['x' as u16]).unwrap();
    assert_eq!(call(&mut h, O::SelectionAnchorOffset, selection.clone(), &[]), V::Number(2.));
    assert_eq!(call(&mut h, O::SelectionFocusOffset, selection.clone(), &[]), V::Number(4.));
    call(&mut h, O::SelectionRemoveAllRanges, selection.clone(), &[]);
    assert_eq!(call(&mut h, O::SelectionRangeCount, selection, &[]), V::Number(0.));
}

#[test]
fn node_name_casing_depends_on_namespace_and_owner_document() {
    let mut h = Host::default();
    let html = element(&mut h);
    assert_eq!(
        call(&mut h, O::NodeName, V::Node(html), &[]),
        str_value("DIV")
    );
    let xml = node(call(&mut h, O::DocumentConstructor, V::Undefined, &[]));
    h.dom.adopt(html, xml).unwrap();
    assert_eq!(
        call(&mut h, O::NodeName, V::Node(html), &[]),
        str_value("div")
    );
    let svg = h
        .dom
        .create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "svg:path")
        .unwrap();
    assert_eq!(
        call(&mut h, O::NodeName, V::Node(svg), &[]),
        str_value("svg:path")
    );
    assert_eq!(call(&mut h, O::NodeValue, V::Node(svg), &[]), V::Null);
    assert_eq!(call(&mut h, O::TextContent, V::Node(xml), &[]), V::Null);
    assert!(matches!(h.dom.get(xml).kind, NodeKind::Document));
}

#[test]
fn template_content_binding_returns_the_persistent_inert_fragment() {
    let mut h = Host::default();
    let template = h.dom.create_element(NodeIdx(0), "template");
    let receiver = V::Node(template);
    let first = call(&mut h, O::TemplateContent, receiver.clone(), &[]);
    assert_eq!(call(&mut h, O::TemplateContent, receiver, &[]), first);
    let contents = node(first);
    assert_ne!(h.dom.owner_document(contents), Some(NodeIdx(0)));
    assert_eq!(h.dom.children(template).count(), 0);
}

#[test]
fn wpt_element_names_and_document_type_fields_are_installed() {
    let mut h = Host::default();
    let html = h.dom.create_element_ns_in(NodeIdx(0),
        Some("http://www.w3.org/1999/xhtml"), "p:span").unwrap();
    let receiver = V::Node(html);
    assert_eq!(call(&mut h, O::ElementNamespaceURI, receiver.clone(), &[]),
        str_value("http://www.w3.org/1999/xhtml"));
    assert_eq!(call(&mut h, O::ElementPrefix, receiver.clone(), &[]), str_value("p"));
    assert_eq!(call(&mut h, O::ElementLocalName, receiver.clone(), &[]), str_value("span"));
    assert_eq!(call(&mut h, O::ElementTagName, receiver, &[]), str_value("P:SPAN"));
    let null = h.dom.create_element_ns_in(NodeIdx(0), None, "span").unwrap();
    assert_eq!(call(&mut h, O::ElementNamespaceURI, V::Node(null), &[]), V::Null);
    assert_eq!(call(&mut h, O::ElementPrefix, V::Node(null), &[]), V::Null);
    assert_eq!(call(&mut h, O::ElementTagName, V::Node(null), &[]), str_value("span"));
    let doctype = h.dom.create_document_type_in(NodeIdx(0), "html", "public", "system").unwrap();
    assert_eq!(call(&mut h, O::DocumentTypeName, V::Node(doctype), &[]), str_value("html"));
    assert_eq!(call(&mut h, O::DocumentTypePublicId, V::Node(doctype), &[]), str_value("public"));
    assert_eq!(call(&mut h, O::DocumentTypeSystemId, V::Node(doctype), &[]), str_value("system"));
    assert!(invoke(&mut h, O::ElementLocalName, V::Node(doctype), &[]).is_err());
}

#[test]
fn wpt_create_element_uses_local_name_validation() {
    let mut h = Host::default();
    let uppercase = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)),
        &[str_value("DIV")]));
    assert_eq!(call(&mut h, O::ElementLocalName, V::Node(uppercase), &[]), str_value("div"));
    let element = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)),
        &[str_value(":good:times:")]));
    assert_eq!(call(&mut h, O::ElementLocalName, V::Node(element), &[]),
        str_value(":good:times:"));
    assert_eq!(call(&mut h, O::ElementPrefix, V::Node(element), &[]), V::Null);
    assert_eq!(invoke(&mut h, O::CreateElementNS, V::Node(NodeIdx(0)),
        &[str_value("http://www.w3.org/1999/xhtml"), str_value(":good:times:")]),
        Err("InvalidCharacterError".into()));
}

#[test]
fn wpt_empty_attribute_namespace_matches_null_namespace() {
    let mut h = Host::default();
    let element = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)),
        &[str_value("p")]));
    call(&mut h, O::ElementSetAttribute, V::Node(element),
        &[str_value("FOO"), str_value("bar")]);
    assert_eq!(call(&mut h, O::ElementGetAttributeNS, V::Node(element),
        &[str_value(""), str_value("foo")]), str_value("bar"));
    assert_eq!(call(&mut h, O::ElementGetAttributeNS, V::Node(element),
        &[str_value(""), str_value("FOO")]), V::Null);
}

#[test]
fn wpt_character_data_methods_use_utf16_offsets_and_validate_bounds() {
    let mut h = Host::default();
    let parent = h.dom.create_element(NodeIdx(0), "div");
    let text = h.dom.create_text(parent, "a😀b");
    let receiver = V::Node(text);
    assert_eq!(call(&mut h, O::CharacterDataSubstring, receiver.clone(),
        &[V::Number(1.), V::Number(2.)]), V::String(vec![0xD83D, 0xDE00]));
    assert!(invoke(&mut h, O::CharacterDataSubstring, receiver.clone(),
        &[V::Number(5.), V::Number(1.)]).is_err());
    call(&mut h, O::CharacterDataInsert, receiver.clone(),
        &[V::Number(1.), str_value("x")]);
    call(&mut h, O::CharacterDataDelete, receiver.clone(),
        &[V::Number(2.), V::Number(2.)]);
    call(&mut h, O::CharacterDataReplace, receiver.clone(),
        &[V::Number(1.), V::Number(1.), str_value("y")]);
    call(&mut h, O::CharacterDataAppend, receiver.clone(), &[str_value("z")]);
    assert_eq!(call(&mut h, O::Data, receiver.clone(), &[]), str_value("aybz"));
    let split = node(call(&mut h, O::TextSplit, receiver.clone(), &[V::Number(2.)]));
    assert_eq!(call(&mut h, O::Data, receiver, &[]), str_value("ay"));
    assert_eq!(call(&mut h, O::Data, V::Node(split), &[]), str_value("bz"));
}

#[test]
fn wpt_node_namespace_and_identity_methods_are_installed() {
    let mut h = Host::default();
    let root = h.dom.create_element_ns(NodeIdx(0), Some("urn:own"), "p:root").unwrap();
    let receiver = V::Node(root);
    assert_eq!(call(&mut h, O::NodeLookupNamespaceURI, receiver.clone(), &[str_value("p")]),
        str_value("urn:own"));
    assert_eq!(call(&mut h, O::NodeLookupNamespaceURI, receiver.clone(), &[V::Null]), V::Null);
    assert_eq!(call(&mut h, O::NodeIsDefaultNamespace, receiver.clone(), &[V::Null]), V::Bool(true));
    assert_eq!(call(&mut h, O::NodeIsSameNode, receiver.clone(), &[receiver.clone()]), V::Bool(true));
    assert_eq!(call(&mut h, O::NodeIsSameNode, receiver.clone(), &[V::Null]), V::Bool(false));
    let first = h.dom.create_text(root, "a");
    let second = h.dom.create_text(root, "b");
    call(&mut h, O::NodeNormalize, receiver, &[]);
    assert_eq!(h.dom.character_data(first).as_deref(), Some("ab"));
    assert_eq!(h.dom.get(second).parent, None);
}

#[test]
fn class_list_is_live_canonical_and_validates_tokens() {
    let mut h = Host::default();
    let element = V::Node(element(&mut h));
    let list = call(&mut h, O::ElementClassList, element.clone(), &[]);
    assert_eq!(call(&mut h, O::ElementClassList, element.clone(), &[]), list);
    let definition = INTERFACES.iter().find(|definition| definition.interface == Interface::Element).unwrap();
    assert!(definition.members.iter().any(|member| member.name == "classList"
        && member.operation == O::ElementSetClassName
        && member.kind == crate::host::MemberKind::Setter));
    call(&mut h, O::ElementSetClassName, element.clone(), &[str_value("x")]);
    assert_eq!(call(&mut h, O::TokenListValue, list.clone(), &[]), str_value("x"));
    call(&mut h, O::ElementSetClassName, element.clone(), &[str_value(" a  b a ")]);
    assert_eq!(call(&mut h, O::TokenListLength, list.clone(), &[]), V::Number(2.));
    assert_eq!(call(&mut h, O::TokenListItem, list.clone(), &[V::Number(1.)]), str_value("b"));
    call(&mut h, O::TokenListAdd, list.clone(), &[str_value("c")]);
    assert_eq!(call(&mut h, O::ElementClassName, element.clone(), &[]), str_value("a b c"));
    assert_eq!(call(&mut h, O::TokenListToggle, list.clone(), &[str_value("b")]), V::Bool(false));
    assert_eq!(call(&mut h, O::TokenListContains, list.clone(), &[str_value("b")]), V::Bool(false));
    assert_eq!(call(&mut h, O::TokenListReplace, list.clone(), &[str_value("a"), str_value("d")]), V::Bool(true));
    call(&mut h, O::TokenListRemove, list.clone(), &[]);
    assert_eq!(call(&mut h, O::TokenListValue, list.clone(), &[]), str_value("d c"));
    assert_eq!(invoke(&mut h, O::TokenListAdd, list.clone(), &[str_value("")]), Err("SyntaxError".into()));
    assert_eq!(invoke(&mut h, O::TokenListAdd, list.clone(), &[str_value("a b")]), Err("InvalidCharacterError".into()));
    call(&mut h, O::TokenListSetValue, list.clone(), &[V::String(vec![0xD800])]);
    assert_eq!(call(&mut h, O::ElementClassName, element, &[]), V::String(vec![0xD800]));
    h.reject_lone_surrogate = true;
    assert_eq!(invoke(&mut h, O::TokenListValue, list, &[]), Err("TypeError: cannot represent lone surrogate".into()));
}

#[test]
fn forced_token_toggle_without_a_change_queues_no_attribute_mutation() {
    let mut h = Host::default();
    let node = element(&mut h);
    h.dom.set_attribute(node, "class", "present").unwrap();
    let list = call(&mut h, O::ElementClassList, V::Node(node), &[]);
    let observer = h.dom.create_mutation_observer(1);
    h.dom.observe(observer, node, amazing_dom::mutation::ObserveOptions {
        attributes: Some(true),
        ..Default::default()
    }).unwrap();

    assert_eq!(call(&mut h, O::TokenListToggle, list.clone(), &[str_value("present"), V::Bool(true)]), V::Bool(true));
    assert_eq!(call(&mut h, O::TokenListToggle, list, &[str_value("absent"), V::Bool(false)]), V::Bool(false));
    assert!(h.dom.take_mutation_records(observer).is_empty());
    assert_eq!(h.dom.get_attribute_ns(node, None, "class"), Some("present"));
}

#[test]
fn html_collection_binding_and_legacy_hooks_are_live() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    // 11: the merge of HTML-26's SELECTED_OPTIONS legacy hook with dev's
    // DOM_STRING_MAP legacy hook (dataset) adds one each over the prior 9.
    assert_eq!(h.legacy.len(), 11);
    assert!(h.legacy.iter().any(|(brand, hooks)| *brand == InterfaceBrand::NAMED_NODE_MAP
        && hooks.indexed_getter == O::NamedNodeMapItem
        && hooks.named_getter == Some(O::NamedNodeMapGetNamedItem)
        && hooks.unenumerable_names));
    assert_eq!(h.legacy[0].0, InterfaceBrand::COLLECTION);
    assert_eq!(h.legacy[0].1.named_getter, Some(O::HTMLCollectionNamedGetter));
    assert!(h.legacy[0].1.unenumerable_names);
    assert!(h.legacy.iter().filter(|(brand, _)| *brand != InterfaceBrand::DOM_STRING_MAP)
        .all(|(brand, hooks)| !hooks.override_builtins
        && (hooks.indexed_setter.is_none() || *brand == InterfaceBrand::OPTIONS)
        && hooks.named_setter.is_none()
        && hooks.named_deleter.is_none()));
    assert!(h.iterables.contains(&(Interface::NodeList, IterableKind::Values)));
    assert!(h.iterables.contains(&(Interface::DOMTokenList, IterableKind::Values)));
    let root = h.dom.create_element(NodeIdx(0), "div");
    let document_children = call(&mut h, O::DocumentChildren, V::Node(NodeIdx(0)), &[]);
    assert_eq!(call(&mut h, O::HTMLCollectionItem, document_children, &[V::Number(0.)]), V::Node(root));
    let fragment = h.dom.create_document_fragment(NodeIdx(0)).unwrap();
    let fragment_child = h.dom.create_element(fragment, "i");
    let fragment_children = call(&mut h, O::FragmentChildren, V::Node(fragment), &[]);
    assert_eq!(call(&mut h, O::HTMLCollectionItem, fragment_children, &[V::Number(0.)]), V::Node(fragment_child));
    let first = h.dom.create_element(root, "span");
    h.dom.set_attribute(first, "id", "first").unwrap();
    let receiver = V::Node(root);
    let child_nodes = call(&mut h, O::ChildNodes, receiver.clone(), &[]);
    assert_eq!(mock_legacy_get(&mut h, child_nodes, "0"), V::Node(first));
    call(&mut h, O::ElementSetClassName, V::Node(first), &[str_value("a b")]);
    let class_list = call(&mut h, O::ElementClassList, V::Node(first), &[]);
    assert_eq!(mock_legacy_get(&mut h, class_list, "1"), str_value("b"));
    let collection = call(&mut h, O::ElementChildren, receiver.clone(), &[]);
    assert_eq!(call(&mut h, O::ElementChildren, receiver.clone(), &[]), collection);
    assert_eq!(call(&mut h, O::HTMLCollectionLength, collection.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::HTMLCollectionItem, collection.clone(), &[V::Number(0.)]), V::Node(first));
    assert_eq!(call(&mut h, O::HTMLCollectionNamedGetter, collection.clone(), &[str_value("first")]), V::Node(first));
    assert_eq!(mock_legacy_get(&mut h, collection.clone(), "0"), V::Node(first));
    assert_eq!(mock_legacy_get(&mut h, collection.clone(), "first"), V::Node(first));
    assert_eq!(mock_legacy_keys(&mut h, collection.clone()), vec!["0", "first"]);
    assert_eq!(crate::supported_names(&mut h, O::HTMLCollectionSupportedNames, &collection).unwrap(), vec!["first".encode_utf16().collect::<Vec<_>>()]);
    let second = h.dom.create_element(root, "b");
    h.dom.set_attribute(second, "name", "named").unwrap();
    assert_eq!(crate::supported_index_count(&mut h, O::HTMLCollectionSupportedIndexCount, &collection).unwrap(), 2);
    assert_eq!(call(&mut h, O::HTMLCollectionNamedItem, collection.clone(), &[str_value("named")]), V::Node(second));
    h.dom.detach(first);
    assert_eq!(call(&mut h, O::HTMLCollectionItem, collection.clone(), &[V::Number(0.)]), V::Node(second));
    assert_eq!(mock_legacy_get(&mut h, collection.clone(), "first"), V::Undefined);
    assert_eq!(mock_legacy_keys(&mut h, collection.clone()), vec!["0", "named"]);
    assert_eq!(mock_legacy_get(&mut h, collection, "0"), V::Node(second));
    let tags = call(&mut h, O::ElementGetElementsByTagName, receiver.clone(), &[str_value("b")]);
    assert_eq!(call(&mut h, O::HTMLCollectionLength, tags, &[]), V::Number(1.));
    let surrogate_query = call(&mut h, O::ElementGetElementsByTagName, receiver.clone(), &[V::String(vec![0xD800])]);
    assert_eq!(call(&mut h, O::HTMLCollectionLength, surrogate_query, &[]), V::Number(0.));
    let classes = call(&mut h, O::ElementGetElementsByClassName, receiver, &[str_value("x")]);
    assert_eq!(call(&mut h, O::HTMLCollectionLength, classes, &[]), V::Number(0.));
}

#[test]
fn parent_and_sibling_element_navigation_bindings_are_live() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    let root = h.dom.create_element(NodeIdx(0), "div");
    let text = h.dom.create_text(root, "x");
    let a = h.dom.create_element(root, "a");
    let b = h.dom.create_element(root, "b");
    assert_eq!(call(&mut h, O::FirstElementChild, V::Node(root), &[]), V::Node(a));
    assert_eq!(call(&mut h, O::LastElementChild, V::Node(root), &[]), V::Node(b));
    assert_eq!(call(&mut h, O::ChildElementCount, V::Node(root), &[]), V::Number(2.));
    assert_eq!(call(&mut h, O::NextElementSibling, V::Node(text), &[]), V::Node(a));
    assert_eq!(call(&mut h, O::PreviousElementSibling, V::Node(b), &[]), V::Node(a));
    h.dom.detach(a);
    assert_eq!(call(&mut h, O::ChildElementCount, V::Node(root), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::PreviousElementSibling, V::Node(b), &[]), V::Null);
    assert_eq!(call(&mut h, O::FirstElementChild, V::Node(NodeIdx(0)), &[]), V::Node(root));
}

#[test]
fn html_reflection_handles_strings_keywords_booleans_and_tabindex() {
    let mut h = Host::default();
    let node = h.dom.create_element(NodeIdx(0), "button");
    let value = V::Node(node);
    assert_eq!(call(&mut h, O::HTMLTabIndex, value.clone(), &[]), V::Number(0.));
    call(&mut h, O::ElementSetId, value.clone(), &[str_value("x")]);
    assert_eq!(call(&mut h, O::ElementId, value.clone(), &[]), str_value("x"));
    call(&mut h, O::HTMLSetTitle, value.clone(), &[V::String(vec![0xD800])]);
    assert_eq!(call(&mut h, O::HTMLTitle, value.clone(), &[]), V::String(vec![0xD800]));
    h.dom.set_attribute(node, "dir", "RTL").unwrap();
    assert_eq!(call(&mut h, O::HTMLDir, value.clone(), &[]), str_value("rtl"));
    h.dom.set_attribute(node, "dir", "bogus").unwrap();
    assert_eq!(call(&mut h, O::HTMLDir, value.clone(), &[]), str_value(""));
    call(&mut h, O::HTMLSetHidden, value.clone(), &[V::Bool(true)]);
    assert_eq!(call(&mut h, O::HTMLHidden, value.clone(), &[]), V::Bool(true));
    call(&mut h, O::HTMLSetHidden, value.clone(), &[V::Bool(false)]);
    assert_eq!(call(&mut h, O::HTMLHidden, value.clone(), &[]), V::Bool(false));
    call(&mut h, O::HTMLSetHidden, value.clone(), &[str_value("UNTIL-FOUND")]);
    assert_eq!(call(&mut h, O::HTMLHidden, value.clone(), &[]), str_value("until-found"));
    call(&mut h, O::HTMLSetTabIndex, value.clone(), &[V::Number(-3.)]);
    assert_eq!(call(&mut h, O::HTMLTabIndex, value.clone(), &[]), V::Number(-3.));
    call(&mut h, O::HTMLSetTranslate, value.clone(), &[V::Bool(false)]);
    assert_eq!(call(&mut h, O::HTMLTranslate, value, &[]), V::Bool(false));
}

#[test]
fn html_element_interface_lookup_has_specialized_and_unknown_fallbacks() {
    assert_eq!(html_element_interface("button"), Interface::HTMLButtonElement);
    assert_eq!(html_element_interface("h3"), Interface::HTMLHeadingElement);
    assert_eq!(html_element_interface("span"), Interface::HTMLSpanElement);
    assert_eq!(html_element_interface("section"), Interface::HTMLElement);
    assert_eq!(html_element_interface("my-widget"), Interface::HTMLElement);
    assert_eq!(html_element_interface("someunknown"), Interface::HTMLUnknownElement);
    let video = INTERFACES.iter().find(|definition| definition.interface == Interface::HTMLVideoElement).unwrap();
    assert_eq!(video.parent, Some(Interface::HTMLMediaElement));
}

#[test]
fn shared_html_integer_reflection_parses_leading_digits_and_range() {
    let units = |s: &str| s.encode_utf16().collect::<Vec<_>>();
    assert_eq!(crate::html_reflection::signed_long(&units("  -12px")), Some(-12));
    assert_eq!(crate::html_reflection::signed_long(&units("+2")), Some(2));
    assert_eq!(crate::html_reflection::signed_long(&units("2147483648")), None);
    assert_eq!(crate::html_reflection::unsigned_long(&units("4294967295rest")), Some(u32::MAX));
    assert_eq!(crate::html_reflection::unsigned_long(&units("-1")), None);
}

#[test]
fn markup_and_selector_seams_are_live_and_run_after_dom_borrow() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    let root = h.dom.create_element(NodeIdx(0), "div");
    let old = h.dom.create_element(root, "span");
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(root), &[]), str_value("<span></span>"));
    call(&mut h, O::ElementSetInnerHTML, V::Node(root), &[str_value("<b>new</b>")]);
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(root), &[]), str_value("<b>new</b>"));
    assert_eq!(h.dom.get(old).parent, None);
    let b = h.dom.get(root).first_child.unwrap();
    assert_eq!(call(&mut h, O::ParentQuerySelector, V::Node(root), &[str_value("b")]), V::Node(b));
    let list = call(&mut h, O::ParentQuerySelectorAll, V::Node(root), &[str_value("b")]);
    assert_eq!(call(&mut h, O::NodeListLength, list.clone(), &[]), V::Number(1.));
    assert_eq!(call(&mut h, O::NodeListItem, list.clone(), &[V::Number(0.)]), V::Node(b));
    assert_eq!(call(&mut h, O::ElementMatches, V::Node(b), &[str_value("b")]), V::Bool(true));
    assert_eq!(call(&mut h, O::ElementWebkitMatchesSelector, V::Node(b), &[str_value("div")]), V::Bool(false));
    assert_eq!(call(&mut h, O::ElementClosest, V::Node(b), &[str_value("div")]), V::Node(root));
    assert_eq!(invoke(&mut h, O::ParentQuerySelector, V::Node(root), &[str_value("?")]), Err("SyntaxError".into()));
    assert_eq!(invoke(&mut h, O::ParentQuerySelector, V::Node(root), &[V::String(vec![0xD800])]), Err("SyntaxError".into()));
    call(&mut h, O::ElementInsertAdjacentHTML, V::Node(b), &[str_value("afterend"), str_value("<em>x</em>")]);
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(root), &[]), str_value("<b>new</b><em>x</em>"));
    assert_eq!(call(&mut h, O::ElementInnerText, V::Node(root), &[]), str_value("newx"));
    call(&mut h, O::ElementSetOuterHTML, V::Node(b), &[str_value("<em>x</em>")]);
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(root), &[]), str_value("<em>x</em><em>x</em>"));
    assert_eq!(call(&mut h, O::NodeListLength, list, &[]), V::Number(1.));
    assert_eq!(invoke(&mut h, O::ElementInsertAdjacentHTML, V::Node(b), &[str_value("beside"), str_value("x")]), Err("SyntaxError".into()));
    assert_eq!(invoke(&mut h, O::ElementSetOuterHTML, V::Node(old), &[str_value("x")]), Err("NoModificationAllowedError".into()));
    assert_eq!(invoke(&mut h, O::ElementInsertAdjacentHTML, V::Node(old), &[str_value("beforebegin"), str_value("x")]), Err("NoModificationAllowedError".into()));
}

#[test]
fn wpt_closest_walks_inclusive_ancestors_through_selector_host() {
    // dom/nodes/Element-closest.html at WPT c2a2fce: the select,
    // fieldset, div, and absent-ancestor result sites, using tag selectors
    // supported by this deliberately small mock SelectorEngine.
    let mut host = Host::default();
    let body = host.dom.create_element(NodeIdx(0), "body");
    let div = host.dom.create_element(body, "div");
    let fieldset = host.dom.create_element(div, "fieldset");
    let select = host.dom.create_element(fieldset, "select");
    let option = host.dom.create_element(select, "option");
    for (selector, expected) in [("select", Some(select)), ("fieldset", Some(fieldset)),
        ("div", Some(div)), ("body", Some(body)), ("form", None)] {
        let actual = call(&mut host, O::ElementClosest, V::Node(option), &[str_value(selector)]);
        assert_eq!(actual, expected.map(V::Node).unwrap_or(V::Null));
    }
    assert_eq!(call(&mut host, O::ElementMatches, V::Node(option), &[str_value("option")]), V::Bool(true));
    assert_eq!(call(&mut host, O::ElementMatches, V::Node(option), &[str_value("select")]), V::Bool(false));
    assert_eq!(invoke(&mut host, O::ElementMatches, V::Node(option), &[str_value("?")]), Err("SyntaxError".into()));
}

#[test]
fn adjacent_html_positions_and_inner_text_approximation() {
    let mut h = Host::default();
    let root = h.dom.create_element(NodeIdx(0), "div");
    let target = h.dom.create_element(root, "p");
    for position in ["beforebegin", "afterbegin", "beforeend", "afterend"] {
        call(&mut h, O::ElementInsertAdjacentHTML, V::Node(target),
            &[str_value(position), str_value("<em>x</em>")]);
    }
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(root), &[]),
        str_value("<em>x</em><p><em>x</em><em>x</em></p><em>x</em>"));
    call(&mut h, O::ElementSetInnerText, V::Node(target), &[str_value("a\nb")]);
    assert_eq!(call(&mut h, O::ElementInnerText, V::Node(target), &[]), str_value("a\nb"));
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(target), &[]), str_value("a\nb"));
}

#[test]
fn abort_controller_delivers_algorithms_and_events_before_any_dependents() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    assert!(h.static_methods.contains(&(Interface::AbortSignal, "any", O::AbortSignalAny, 1)));
    let controller = call(&mut h, O::AbortControllerConstructor, V::Undefined, &[]);
    let source = call(&mut h, O::AbortControllerSignal, controller.clone(), &[]);
    let dependent = call(&mut h, O::AbortSignalAny, V::Undefined, &[V::Array(vec![source.clone()])]);
    let source_id = amazing_dom::abort::AbortSignalId(h.platform_object(&source, InterfaceBrand::ABORT_SIGNAL).unwrap().slot as usize);
    let dependent_id = amazing_dom::abort::AbortSignalId(h.platform_object(&dependent, InterfaceBrand::ABORT_SIGNAL).unwrap().slot as usize);
    h.dom.add_abort_algorithm(source_id, 1).unwrap();
    h.dom.add_abort_algorithm(dependent_id, 2).unwrap();
    call(&mut h, O::AddEventListener, source.clone(), &[str_value("abort"), V::Callable(11)]);
    call(&mut h, O::AddEventListener, dependent.clone(), &[str_value("abort"), V::Callable(12)]);
    call(&mut h, O::AbortControllerAbort, controller.clone(), &[str_value("why")]);
    assert_eq!(h.abort_trace, vec![1001, 11, 1002, 12]);
    assert_eq!(call(&mut h, O::AbortSignalAborted, dependent.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::AbortSignalReason, dependent.clone(), &[]), str_value("why"));
    assert_eq!(invoke(&mut h, O::AbortSignalThrowIfAborted, source.clone(), &[]), Err("throw: String([119, 104, 121])".into()));
    call(&mut h, O::AbortControllerAbort, controller, &[str_value("later")]);
    assert_eq!(h.abort_trace, vec![1001, 11, 1002, 12]);
    let already = call(&mut h, O::AbortSignalAny, V::Undefined, &[V::Array(vec![source])]);
    assert_eq!(call(&mut h, O::AbortSignalReason, already, &[]), str_value("why"));
}

#[test]
fn timeout_signal_uses_host_timer_and_fires_on_expiry() {
    let mut h = Host::default();
    assert_eq!(invoke(&mut h, O::AbortSignalTimeout, V::Undefined, &[V::Number(-1.)]),
        Err("TypeError: AbortSignal.timeout requires an unsigned long long".into()));
    let signal = call(&mut h, O::AbortSignalTimeout, V::Undefined, &[V::Number(5.)]);
    let id = amazing_dom::abort::AbortSignalId(h.platform_object(&signal, InterfaceBrand::ABORT_SIGNAL).unwrap().slot as usize);
    assert_eq!(h.abort_timers, vec![(id, 5)]);
    assert_eq!(call(&mut h, O::AbortSignalAborted, signal.clone(), &[]), V::Bool(false));
    call(&mut h, O::AddEventListener, signal.clone(), &[str_value("abort"), V::Callable(13)]);
    call(&mut h, O::AbortSignalSetOnAbort, signal.clone(), &[V::Callable(14)]);
    assert_eq!(call(&mut h, O::AbortSignalOnAbort, signal.clone(), &[]), V::Callable(14));
    crate::fire_abort_timeout(&mut h, id).unwrap();
    assert_eq!(call(&mut h, O::AbortSignalAborted, signal, &[]), V::Bool(true));
    assert_eq!(h.abort_trace, vec![13, 14]);
    crate::fire_abort_timeout(&mut h, id).unwrap();
    assert_eq!(h.abort_trace, vec![13, 14]);
}

#[test]
fn document_surface_uses_tree_and_host_metadata() {
    let mut h = Host::default();
    let doc = V::Node(NodeIdx(0));
    assert_eq!(call(&mut h, O::DocumentDoctype, doc.clone(), &[]), V::Null);
    let doctype = h.dom.create_document_type_in(NodeIdx(0), "html", "", "").unwrap();
    h.dom.pre_insert(doctype, NodeIdx(0), None).unwrap();
    assert_eq!(call(&mut h, O::DocumentDoctype, doc.clone(), &[]), V::Node(doctype));
    let html = h.dom.create_element(NodeIdx(0), "html");
    let head = h.dom.create_element(html, "head");
    let body = h.dom.create_element(html, "body");
    let image = h.dom.create_element(body, "img");
    h.dom.set_attribute(image, "id", "hero").unwrap();
    h.dom.set_attribute(image, "name", "photo").unwrap();
    let a = h.dom.create_element(body, "a");
    h.dom.set_attribute(a, "href", "x").unwrap();
    assert_eq!(call(&mut h, O::DocumentHead, doc.clone(), &[]), V::Node(head));
    assert_eq!(call(&mut h, O::DocumentBody, doc.clone(), &[]), V::Node(body));
    assert_eq!(call(&mut h, O::DocumentElement, doc.clone(), &[]), V::Node(html));
    call(&mut h, O::DocumentSetTitle, doc.clone(), &[str_value(" New  title ")]);
    assert_eq!(call(&mut h, O::DocumentTitle, doc.clone(), &[]), str_value("New title"));
    assert_eq!(call(&mut h, O::DocumentGetElementById, doc.clone(), &[str_value("hero")]), V::Node(image));
    for (op, expected) in [(O::DocumentImages, 1.), (O::DocumentLinks, 1.),
        (O::DocumentForms, 0.), (O::DocumentScripts, 0.)] {
        let list = call(&mut h, op, doc.clone(), &[]);
        assert_eq!(call(&mut h, O::HTMLCollectionLength, list, &[]), V::Number(expected));
    }
    let named = call(&mut h, O::DocumentGetElementsByName, doc.clone(), &[str_value("photo")]);
    assert_eq!(call(&mut h, O::NodeListItem, named.clone(), &[V::Number(0.)]), V::Node(image));
    h.dom.remove_attribute(image, "name").unwrap();
    assert_eq!(call(&mut h, O::NodeListLength, named, &[]), V::Number(0.));
    assert_eq!(call(&mut h, O::DocumentReadyState, doc.clone(), &[]), str_value("loading"));
    h.dom.set_document_ready_state(NodeIdx(0), amazing_dom::document_surface::ReadyState::Interactive).unwrap();
    assert_eq!(call(&mut h, O::DocumentReadyState, doc.clone(), &[]), str_value("interactive"));
    assert_eq!(call(&mut h, O::DocumentURL, doc.clone(), &[]), str_value("https://example.test/page"));
    assert_eq!(call(&mut h, O::DocumentCharacterSet, doc.clone(), &[]), str_value("UTF-8"));
    assert_eq!(call(&mut h, O::DocumentActiveElement, doc.clone(), &[]), V::Node(body));
    h.active = Some(a); h.focused = true;
    assert_eq!(call(&mut h, O::DocumentActiveElement, doc.clone(), &[]), V::Node(a));
    assert_eq!(call(&mut h, O::DocumentHasFocus, doc.clone(), &[]), V::Bool(true));
    let fragment = h.dom.create_document_fragment(NodeIdx(0)).unwrap();
    let child = h.dom.create_element(fragment, "span");
    h.dom.set_attribute(child, "id", "fragment-id").unwrap();
    assert_eq!(call(&mut h, O::FragmentGetElementById, V::Node(fragment), &[str_value("fragment-id")]), V::Node(child));
    let replacement = h.dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "body").unwrap();
    call(&mut h, O::DocumentSetBody, doc.clone(), &[V::Node(replacement)]);
    assert_eq!(call(&mut h, O::DocumentBody, doc.clone(), &[]), V::Node(replacement));
    assert_eq!(invoke(&mut h, O::DocumentSetBody, doc, &[V::Node(head)]), Err("HierarchyRequestError".into()));
}

#[test]
fn node_base_uri_uses_first_document_base_and_document_url_fallback() {
    let mut h = Host::default();
    let doc = NodeIdx(0);
    let html = h.dom.create_element(doc, "html");
    let head = h.dom.create_element(html, "head");
    let body = h.dom.create_element(html, "body");
    let element = h.dom.create_element(body, "p");
    assert_eq!(call(&mut h, O::NodeBaseURI, V::Node(element), &[]), str_value("https://example.test/page"));
    // Pinned WPT dom/nodes/Node-baseURI.html: detached, fragment, and Attr nodes.
    let detached = h.dom.create_element_ns_in(doc, Some("http://www.w3.org/1999/xhtml"), "div").unwrap();
    let fragment = h.dom.create_document_fragment(doc).unwrap();
    let in_fragment = h.dom.create_element(fragment, "div");
    let detached_attr = node(call(&mut h, O::CreateAttribute, V::Node(doc), &[str_value("class")]));
    call(&mut h, O::ElementSetAttribute, V::Node(element), &[str_value("class"), str_value("abc")]);
    let attached_attr = node(call(&mut h, O::ElementGetAttributeNode, V::Node(element), &[str_value("class")]));
    for candidate in [detached, in_fragment, detached_attr, attached_attr] {
        assert_eq!(call(&mut h, O::NodeBaseURI, V::Node(candidate), &[]), str_value("https://example.test/page"));
    }
    let first = h.dom.create_element(head, "base");
    h.dom.set_attribute(first, "href", "https://cdn.test/assets/").unwrap();
    let second = h.dom.create_element(body, "base");
    h.dom.set_attribute(second, "href", "https://ignored.test/").unwrap();
    assert_eq!(call(&mut h, O::NodeBaseURI, V::Node(element), &[]), str_value("https://cdn.test/assets/"));
    assert_eq!(call(&mut h, O::NodeBaseURI, V::Node(doc), &[]), str_value("https://cdn.test/assets/"));
    h.dom.remove_attribute(first, "href").unwrap();
    assert_eq!(call(&mut h, O::NodeBaseURI, V::Node(element), &[]), str_value("https://ignored.test/"));
    h.dom.detach(second);
    assert_eq!(call(&mut h, O::NodeBaseURI, V::Node(element), &[]), str_value("https://example.test/page"));
}

#[test]
fn hyperlink_components_reinitialize_and_write_back_live_href() {
    let mut h = Host::default();
    let root = h.dom.create_element(NodeIdx(0), "div");
    for tag in ["a", "area"] {
        let link = h.dom.create_element(root, tag);
        assert_eq!(call(&mut h, O::HyperlinkProtocol, V::Node(link), &[]), str_value(":"));
        call(&mut h, O::HyperlinkSetSearch, V::Node(link), &[str_value("?ignored")]);
        assert!(h.dom.attribute_utf16(link, "href").is_none());
        call(&mut h, O::ElementSetHref, V::Node(link), &[str_value("https://user:secret@example.test:8443/path?old#mark")]);
        for (op, expected) in [(O::HyperlinkOrigin, "https://example.test:8443"),
            (O::HyperlinkProtocol, "https:"), (O::HyperlinkUsername, "user"),
            (O::HyperlinkPassword, "secret"), (O::HyperlinkHost, "example.test:8443"),
            (O::HyperlinkHostname, "example.test"), (O::HyperlinkPort, "8443"),
            (O::HyperlinkPathname, "/path"), (O::HyperlinkSearch, "?old"),
            (O::HyperlinkHash, "#mark")] {
            assert_eq!(call(&mut h, op, V::Node(link), &[]), str_value(expected));
        }
        call(&mut h, O::HyperlinkSetSearch, V::Node(link), &[str_value("?€#")]);
        assert_eq!(call(&mut h, O::HyperlinkSearch, V::Node(link), &[]), str_value("?%E2%82%AC%23"));
        assert_eq!(call(&mut h, O::HyperlinkHash, V::Node(link), &[]), str_value("#mark"));
        call(&mut h, O::HyperlinkSetHash, V::Node(link), &[str_value("#")]);
        assert_eq!(call(&mut h, O::HyperlinkHash, V::Node(link), &[]), str_value(""));
        call(&mut h, O::HyperlinkSetSearch, V::Node(link), &[str_value("?")]);
        assert_eq!(call(&mut h, O::HyperlinkSearch, V::Node(link), &[]), str_value(""));
        assert!(h.dom.attribute_utf16(link, "href").unwrap().ends_with(&[63, 35]));
    }
    assert!(invoke(&mut h, O::HyperlinkSearch, V::Node(root), &[]).is_err());
}

#[test]
fn element_specific_reflection_handles_urls_enums_numbers_and_rel_list() {
    let mut h = Host::default();
    let root = h.dom.create_element(NodeIdx(0), "div");
    let anchor = h.dom.create_element(root, "a");
    call(&mut h, O::ElementSetHref, V::Node(anchor), &[str_value("next")]);
    assert_eq!(call(&mut h, O::ElementHref, V::Node(anchor), &[]), str_value("https://example.test/next"));
    assert_eq!(h.dom.attribute_utf16(anchor, "href"), Some("next".encode_utf16().collect()));
    let rel = call(&mut h, O::ElementRelList, V::Node(anchor), &[]);
    assert_eq!(rel, call(&mut h, O::ElementRelList, V::Node(anchor), &[]));
    assert_eq!(call(&mut h, O::TokenListSupports, rel.clone(), &[str_value("NOOPENER")]), V::Bool(true));
    assert_eq!(call(&mut h, O::TokenListSupports, rel.clone(), &[str_value("stylesheet")]), V::Bool(false));
    call(&mut h, O::TokenListAdd, rel, &[str_value("noopener")]);
    assert_eq!(call(&mut h, O::ElementRel, V::Node(anchor), &[]), str_value("noopener"));
    let image = h.dom.create_element(root, "img");
    call(&mut h, O::ImageSetWidth, V::Node(image), &[V::Number(-1.)]);
    assert_eq!(call(&mut h, O::ImageWidth, V::Node(image), &[]), V::Number(u32::MAX as f64));
    h.dom.set_attribute(image, "loading", "unknown").unwrap();
    assert_eq!(call(&mut h, O::ImageLoading, V::Node(image), &[]), str_value("eager"));
    let input = h.dom.create_element(root, "input");
    h.dom.set_attribute(input, "type", "UNSUPPORTED").unwrap();
    assert_eq!(call(&mut h, O::InputType, V::Node(input), &[]), str_value("text"));
    call(&mut h, O::InputSetType, V::Node(input), &[str_value("EMAIL")]);
    assert_eq!(call(&mut h, O::InputType, V::Node(input), &[]), str_value("email"));
    call(&mut h, O::ElementSetDisabled, V::Node(input), &[V::Bool(true)]);
    assert_eq!(call(&mut h, O::ElementDisabled, V::Node(input), &[]), V::Bool(true));
    let cell = h.dom.create_element(root, "td");
    call(&mut h, O::CellSetColSpan, V::Node(cell), &[V::Number(5000.)]);
    assert_eq!(call(&mut h, O::CellColSpan, V::Node(cell), &[]), V::Number(1000.));
    call(&mut h, O::CellSetRowSpan, V::Node(cell), &[V::Number(0.)]);
    assert_eq!(call(&mut h, O::CellRowSpan, V::Node(cell), &[]), V::Number(0.));
    let list = h.dom.create_element(root, "ol");
    call(&mut h, O::OListSetStart, V::Node(list), &[V::Number(-3.)]);
    assert_eq!(call(&mut h, O::OListStart, V::Node(list), &[]), V::Number(-3.));
    let item = h.dom.create_element(list, "li");
    call(&mut h, O::LISetValue, V::Node(item), &[V::Number(7.)]);
    assert_eq!(call(&mut h, O::LIValue, V::Node(item), &[]), V::Number(7.));
    let link = h.dom.create_element(root, "link");
    let link_rel = call(&mut h, O::ElementRelList, V::Node(link), &[]);
    assert_eq!(call(&mut h, O::TokenListSupports, link_rel, &[str_value("STYLESHEET")]), V::Bool(true));
    call(&mut h, O::ResourceSetType, V::Node(link), &[str_value("text/css")]);
    assert_eq!(call(&mut h, O::ResourceType, V::Node(link), &[]), str_value("text/css"));
    let meta = h.dom.create_element(root, "meta");
    call(&mut h, O::MetaSetHttpEquiv, V::Node(meta), &[str_value("refresh")]);
    assert_eq!(call(&mut h, O::MetaHttpEquiv, V::Node(meta), &[]), str_value("refresh"));
    let script = h.dom.create_element(root, "script");
    call(&mut h, O::ScriptSetAsync, V::Node(script), &[V::Bool(true)]);
    assert_eq!(call(&mut h, O::ScriptAsync, V::Node(script), &[]), V::Bool(true));
}

/// HTML-35: `HTMLScriptElement.async` reflects the "force async" flag, not a
/// plain boolean content attribute (HTML SS4.5.6). `Document.createElement`
/// starts the flag `true`; the setter always clears it first, so once
/// cleared only the content attribute controls the getter. A script element
/// created the way the HTML parser creates one (the plain, low-level
/// `Dom::create_element` that `amazing-html`'s tree builder uses, never
/// `create_element_local_in`/`create_element_ns_in`) never gets the flag set
/// and defaults to `false`.
#[test]
fn html_35_script_async_reflects_force_async_flag() {
    let mut h = Host::default();
    let script = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)), &[str_value("script")]));
    assert_eq!(call(&mut h, O::ScriptAsync, V::Node(script), &[]), V::Bool(true));
    call(&mut h, O::ScriptSetAsync, V::Node(script), &[V::Bool(false)]);
    assert_eq!(call(&mut h, O::ScriptAsync, V::Node(script), &[]), V::Bool(false));
    assert!(h.dom.attribute_utf16(script, "async").is_none());
    // The flag is now cleared; only the content attribute drives the getter.
    h.dom.set_attribute(script, "async", "").unwrap();
    assert_eq!(call(&mut h, O::ScriptAsync, V::Node(script), &[]), V::Bool(true));
    call(&mut h, O::ScriptSetAsync, V::Node(script), &[V::Bool(false)]);
    assert_eq!(call(&mut h, O::ScriptAsync, V::Node(script), &[]), V::Bool(false));

    let parser_script = h.dom.create_element(NodeIdx(0), "script");
    assert_eq!(call(&mut h, O::ScriptAsync, V::Node(parser_script), &[]), V::Bool(false));
}

#[test]
fn wpt_html_token_list_attributes_are_live_and_canonical() {
    let mut h = Host::default();
    for (tag, operation, attribute) in [
        ("output", O::OutputHtmlFor, "for"),
        ("iframe", O::IFrameSandbox, "sandbox"),
        ("link", O::LinkSizes, "sizes"),
    ] {
        let element = h.dom.create_element(NodeIdx(0), tag);
        let receiver = V::Node(element);
        let list = call(&mut h, operation, receiver.clone(), &[]);
        assert_eq!(call(&mut h, operation, receiver, &[]), list);
        call(&mut h, O::TokenListAdd, list.clone(), &[str_value("sample")]);
        assert_eq!(h.dom.attribute_utf16(element, attribute),
            Some("sample".encode_utf16().collect()));
        assert_eq!(call(&mut h, O::TokenListValue, list, &[]), str_value("sample"));
    }
}

#[test]
fn put_forwards_token_list_assignments_update_the_live_value() {
    let mut h = Host::default();
    for (tag, interface, getter, setter, attribute) in [
        ("a", Interface::HTMLAnchorElement, O::ElementRelList, O::ElementSetRelList, "rel"),
        ("link", Interface::HTMLLinkElement, O::ElementRelList, O::ElementSetRelList, "rel"),
        ("link", Interface::HTMLLinkElement, O::LinkSizes, O::LinkSetSizes, "sizes"),
        ("form", Interface::HTMLFormElement, O::FormAttributeGet(crate::host::FormAttribute::RelList),
            O::FormAttributeSet(crate::host::FormAttribute::RelList), "rel"),
        ("output", Interface::HTMLOutputElement, O::OutputHtmlFor, O::OutputSetHtmlFor, "for"),
        ("iframe", Interface::HTMLIFrameElement, O::IFrameSandbox, O::IFrameSetSandbox, "sandbox"),
    ] {
        let member = INTERFACES.iter().find(|definition| definition.interface == interface).unwrap()
            .members.iter().find(|member| member.operation == setter).unwrap();
        assert_eq!(member.kind, crate::host::MemberKind::Setter);
        let element = h.dom.create_element(NodeIdx(0), tag);
        let receiver = V::Node(element);
        let list = call(&mut h, getter, receiver.clone(), &[]);
        call(&mut h, setter, receiver.clone(), &[str_value("alpha  beta")]);
        assert_eq!(call(&mut h, O::TokenListValue, list.clone(), &[]), str_value("alpha  beta"));
        assert_eq!(h.dom.attribute_utf16(element, attribute), Some("alpha  beta".encode_utf16().collect()));
        assert_eq!(call(&mut h, getter, receiver, &[]), list);
    }
}

// Pinned WPT custom-elements/CustomElementRegistry.html,
// CustomElementRegistry-getName.html, attribute-changed-callback.html,
// connected-callbacks.html, and custom-element-reaction-queue.html ports.
#[test]
fn custom_element_registry_validates_defines_upgrades_and_resolves_when_defined() {
    let mut h = Host::default();
    let registry = crate::registry_for_document(&mut h, NodeIdx(0)).unwrap();
    let html = h.dom.create_element(NodeIdx(0), "html");
    let element = h.dom.create_element(html, "x-card");
    h.dom.set_attribute(element, "title", "ready").unwrap();
    let constructor = V::Dictionary(vec![
        ("__constructor", V::Bool(true)), ("__id", V::Number(1.)),
        ("prototype", V::Dictionary(vec![
            ("attributeChangedCallback", V::Callable(11)),
            ("connectedCallback", V::Callable(12)),
        ])),
        ("observedAttributes", V::Array(vec![str_value("title")])),
    ]);
    let pending = call(&mut h, O::CustomElementRegistryWhenDefined, registry.clone(), &[str_value("x-card")]);
    assert_eq!(call(&mut h, O::CustomElementRegistryWhenDefined, registry.clone(), &[str_value("x-card")]), pending);
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("badname"), V::Number(1.)]), Err("TypeError: Expected a constructor".into()));
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("badname"), constructor.clone()]), Err("SyntaxError".into()));
    assert_eq!(call(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("x-card"), constructor.clone()]), V::Undefined);
    assert_eq!(h.promises[0].1, Some(constructor.clone()));
    assert_eq!(call(&mut h, O::CustomElementRegistryGet, registry.clone(), &[str_value("x-card")]), constructor);
    assert_eq!(call(&mut h, O::CustomElementRegistryGetName, registry.clone(), &[constructor.clone()]), str_value("x-card"));
    assert_eq!(call(&mut h, O::CustomElementRegistryGetName, registry.clone(),
        &[V::Dictionary(vec![("__constructor", V::Bool(true)), ("__id", V::Number(999.))])]), V::Null);
    assert_eq!(call(&mut h, O::CustomElementRegistryGet, registry.clone(), &[str_value("not-defined")]), V::Undefined);
    let resolved = call(&mut h, O::CustomElementRegistryWhenDefined, registry.clone(), &[str_value("x-card")]);
    assert_ne!(resolved, pending);
    let V::Promise(resolved_id) = resolved else { panic!("expected resolved promise") };
    assert_eq!(h.promises[resolved_id as usize].1, Some(constructor.clone()));
    assert_eq!(h.dom.custom_element_state(element), amazing_dom::custom_elements::CustomElementState::Custom);
    assert_eq!(h.calls, [11, 12]);
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("x-card"), constructor.clone()]), Err("NotSupportedError".into()));
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("x-other"), constructor.clone()]), Err("NotSupportedError".into()));
    let invalid = call(&mut h, O::CustomElementRegistryWhenDefined, registry.clone(), &[str_value("plain")]);
    let V::Promise(id) = invalid else { panic!("expected promise") };
    assert_eq!(h.promises[id as usize].2, Some(str_value("SyntaxError")));
    let detached = h.dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "x-card").unwrap();
    call(&mut h, O::CustomElementRegistryUpgrade, registry, &[V::Node(detached)]);
    assert_eq!(h.dom.custom_element_state(detached), amazing_dom::custom_elements::CustomElementState::Custom);
}

// Pinned WPT custom-elements/attribute-changed-callback.html and
// disconnected-callbacks.html mutation boundary ports.
#[test]
fn custom_element_callbacks_run_after_dom_borrow_at_mutation_boundary() {
    let mut h = Host::default();
    let registry = crate::registry_for_document(&mut h, NodeIdx(0)).unwrap();
    let html = h.dom.create_element(NodeIdx(0), "html");
    let element = h.dom.create_element(html, "x-live");
    let constructor = V::Dictionary(vec![
        ("__constructor", V::Bool(true)),
        ("prototype", V::Dictionary(vec![
            ("attributeChangedCallback", V::Callable(21)),
            ("connectedCallback", V::Callable(22)),
            ("disconnectedCallback", V::Callable(23)),
        ])),
        ("observedAttributes", V::Array(vec![str_value("title")])),
    ]);
    call(&mut h, O::CustomElementRegistryDefine, registry, &[str_value("x-live"), constructor]);
    assert_eq!(h.calls, [22]);
    call(&mut h, O::ElementSetAttribute, V::Node(element), &[str_value("title"), str_value("foo")]);
    assert_eq!(h.calls, [22, 21]);
    call(&mut h, O::Remove, V::Node(element), &[]);
    assert_eq!(h.calls, [22, 21, 23]);
    assert!(h.custom_element_errors.is_empty());
}

// Pinned WPT custom-elements/CustomElementRegistry.html and
// Document-createElement-customized-builtins.html assertion ports.
#[test]
fn custom_element_creation_handles_is_and_disable_shadow() {
    let mut h = Host::default();
    let registry = crate::registry_for_document(&mut h, NodeIdx(0)).unwrap();
    let built_in = V::Dictionary(vec![("__constructor", V::Bool(true)),
        ("__id", V::Number(2.)), ("prototype", V::Dictionary(vec![]))]);
    call(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("x-button"), built_in, V::Dictionary(vec![("extends", str_value("button"))])]);
    let plain = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)), &[str_value("button")]));
    assert_eq!(h.dom.custom_element_state(plain), amazing_dom::custom_elements::CustomElementState::Undefined);
    let customized = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)),
        &[str_value("button"), V::Dictionary(vec![("is", str_value("x-button"))])]));
    assert_eq!(h.dom.custom_element_state(customized), amazing_dom::custom_elements::CustomElementState::Custom);
    let customized_ns = node(call(&mut h, O::CreateElementNS, V::Node(NodeIdx(0)),
        &[str_value("http://www.w3.org/1999/xhtml"), str_value("button"),
            V::Dictionary(vec![("is", str_value("x-button"))])]));
    assert_eq!(h.dom.custom_element_state(customized_ns), amazing_dom::custom_elements::CustomElementState::Custom);
    let shadow_disabled = V::Dictionary(vec![("__constructor", V::Bool(true)),
        ("__id", V::Number(3.)), ("prototype", V::Dictionary(vec![])),
        ("disabledFeatures", V::Array(vec![str_value("shadow")]))]);
    call(&mut h, O::CustomElementRegistryDefine, registry,
        &[str_value("x-no-shadow"), shadow_disabled]);
    let host = node(call(&mut h, O::CreateElement, V::Node(NodeIdx(0)), &[str_value("x-no-shadow")]));
    assert_eq!(invoke(&mut h, O::ElementAttachShadow, V::Node(host),
        &[V::Dictionary(vec![("mode", str_value("open"))])]), Err("NotSupportedError".into()));
}

// Pinned WPT custom-elements/customized-built-in-constructor-exceptions.html
// and Document-createElement-customized-builtins.html constructor error port.
#[test]
fn synchronous_custom_element_creation_propagates_constructor_failure() {
    let mut h = Host::default();
    let registry = crate::registry_for_document(&mut h, NodeIdx(0)).unwrap();
    let constructor = V::Dictionary(vec![("__constructor", V::Bool(true)),
        ("prototype", V::Dictionary(vec![]))]);
    call(&mut h, O::CustomElementRegistryDefine, registry,
        &[str_value("x-fail"), constructor]);
    h.fail_custom_element_constructor = true;
    assert_eq!(invoke(&mut h, O::CreateElement, V::Node(NodeIdx(0)), &[str_value("x-fail")]),
        Err("constructor failed".into()));
}

// Pinned WPT custom-elements/CustomElementRegistry.html validation order and
// observedAttributes conditional extraction ports.
#[test]
fn custom_element_definition_running_and_callback_extraction_order() {
    let mut h = Host::default();
    let registry = crate::registry_for_document(&mut h, NodeIdx(0)).unwrap();
    h.dom.custom_element_begin_definition(NodeIdx(0)).unwrap();
    let constructor = V::Dictionary(vec![("__constructor", V::Bool(true)),
        ("prototype", V::Dictionary(vec![])),
        ("observedAttributes", V::Number(3.))]);
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("x-running"), V::Number(1.)]), Err("TypeError: Expected a constructor".into()));
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("invalid"), constructor.clone()]), Err("SyntaxError".into()));
    assert_eq!(invoke(&mut h, O::CustomElementRegistryDefine, registry.clone(),
        &[str_value("x-running"), constructor.clone()]), Err("NotSupportedError".into()));
    h.dom.custom_element_end_definition(NodeIdx(0));
    // No attributeChangedCallback means observedAttributes is not converted.
    assert_eq!(call(&mut h, O::CustomElementRegistryDefine, registry,
        &[str_value("x-running"), constructor]), V::Undefined);
}

// Pinned WPT custom-elements/custom-element-reaction-queue.html: each
// element's reactions run after its upgrade before the next element's.
#[test]
fn custom_element_upgrade_drains_each_element_reactions_in_order() {
    let mut h = Host::default();
    let registry = crate::registry_for_document(&mut h, NodeIdx(0)).unwrap();
    let html = h.dom.create_element(NodeIdx(0), "html");
    let first = h.dom.create_element(html, "x-ordered");
    let second = h.dom.create_element(html, "x-ordered");
    h.dom.set_attribute(first, "title", "a").unwrap();
    h.dom.set_attribute(second, "title", "b").unwrap();
    let constructor = V::Dictionary(vec![("__constructor", V::Bool(true)),
        ("prototype", V::Dictionary(vec![("attributeChangedCallback", V::Callable(31)),
            ("connectedCallback", V::Callable(32))])),
        ("observedAttributes", V::Array(vec![str_value("title")]))]);
    call(&mut h, O::CustomElementRegistryDefine, registry,
        &[str_value("x-ordered"), constructor]);
    assert_eq!(h.calls, [31, 32, 31, 32]);
}

// WPT pin 53bc812da2ed81179ebd1ba76b2d197cc4747d12:
// html/semantics/forms/the-input-element/valueMode.html and radio.html.
#[test]
fn form_input_value_modes_radio_group_and_reset_bindings() {
    let mut h = Host::default();
    let form = h.dom.create_element(NodeIdx(0), "form");
    let a = h.dom.create_element(form, "input");
    let b = h.dom.create_element(form, "input");
    for node in [a, b] {
        h.dom.set_attribute(node, "type", "radio").unwrap();
        h.dom.set_attribute(node, "name", "answer").unwrap();
    }
    assert_eq!(call(&mut h, O::InputValue, V::Node(a), &[]), str_value("on"));
    call(&mut h, O::InputSetValue, V::Node(a), &[str_value("yes")]);
    assert_eq!(call(&mut h, O::InputDefaultValue, V::Node(a), &[]), str_value("yes"));
    call(&mut h, O::InputSetChecked, V::Node(a), &[V::Bool(true)]);
    call(&mut h, O::InputSetChecked, V::Node(b), &[V::Bool(true)]);
    assert_eq!(call(&mut h, O::InputChecked, V::Node(a), &[]), V::Bool(false));
    assert_eq!(call(&mut h, O::InputChecked, V::Node(b), &[]), V::Bool(true));
    let controls = call(&mut h, O::FormElements, V::Node(form), &[]);
    assert_eq!(crate::supported_index_count(&mut h, O::FormCollectionSupportedIndexCount, &controls).unwrap(), 2);
    assert_eq!(call(&mut h, O::FormCollectionNamedItem, controls.clone(), &[str_value("answer")]),
        call(&mut h, O::FormCollectionNamedGetter, controls, &[str_value("answer")]));
    call(&mut h, O::FormReset, V::Node(form), &[]);
    assert_eq!(call(&mut h, O::InputChecked, V::Node(b), &[]), V::Bool(false));
}

// WPT pin 53bc812da2ed81179ebd1ba76b2d197cc4747d12:
// html/semantics/forms/the-select-element/selected-index.html and
// html/semantics/forms/constraints/form-validation-validity-valueMissing.html.
#[test]
fn select_and_validity_bindings_follow_live_dom() {
    let mut h = Host::default();
    let form = h.dom.create_element(NodeIdx(0), "form");
    let select = h.dom.create_element(form, "select");
    let first = h.dom.create_element(select, "option");
    h.dom.set_attribute(first, "value", "first").unwrap();
    let second = h.dom.create_element(select, "option");
    h.dom.set_attribute(second, "value", "second").unwrap();
    h.dom.set_attribute(second, "selected", "").unwrap();
    assert_eq!(call(&mut h, O::SelectSelectedIndex, V::Node(select), &[]), V::Number(1.));
    let options = call(&mut h, O::SelectOptions, V::Node(select), &[]);
    assert_eq!(crate::supported_index_count(&mut h, O::OptionsSupportedIndexCount, &options).unwrap(), 2);
    assert_eq!(call(&mut h, O::OptionsItem, options, &[V::Number(0.)]), V::Node(first));
    call(&mut h, O::SelectSetSelectedIndex, V::Node(select), &[V::Number(0.)]);
    assert_eq!(call(&mut h, O::SelectValue, V::Node(select), &[]), str_value("first"));
    call(&mut h, O::FormReset, V::Node(form), &[]);
    assert_eq!(call(&mut h, O::SelectSelectedIndex, V::Node(select), &[]), V::Number(1.));
    let input = h.dom.create_element(form, "input");
    h.dom.set_attribute(input, "required", "").unwrap();
    let validity = call(&mut h, O::ControlValidity, V::Node(input), &[]);
    assert_eq!(call(&mut h, O::ValidityValueMissing, validity.clone(), &[]), V::Bool(true));
    assert_eq!(call(&mut h, O::ValidityValid, validity, &[]), V::Bool(false));
    assert_eq!(call(&mut h, O::ControlCheckValidity, V::Node(input), &[]), V::Bool(false));
    call(&mut h, O::InputSetValue, V::Node(input), &[str_value("filled")]);
    assert_eq!(call(&mut h, O::ControlCheckValidity, V::Node(input), &[]), V::Bool(true));
}

#[test]
fn reflected_form_attributes_feed_validation_and_preserve_webidl_rules() {
    let mut h = Host::default();
    let form = h.dom.create_element(NodeIdx(0), "form");
    let input = h.dom.create_element(form, "input");
    call(&mut h, O::FormAttributeSet(F::Required), V::Node(input), &[V::Bool(true)]);
    assert_eq!(call(&mut h, O::ControlCheckValidity, V::Node(input), &[]), V::Bool(false));
    call(&mut h, O::FormAttributeSet(F::Required), V::Node(input), &[V::Bool(false)]);
    assert_eq!(call(&mut h, O::ControlCheckValidity, V::Node(input), &[]), V::Bool(true));
    call(&mut h, O::FormAttributeSet(F::Pattern), V::Node(input), &[str_value("[A-Z]+")]);
    assert_eq!(h.dom.get_attribute_ns(input, None, "pattern"), Some("[A-Z]+"));
    assert_eq!(call(&mut h, O::FormAttributeGet(F::MaxLength), V::Node(input), &[]), V::Number(-1.));
    assert!(crate::invoke(&mut h, O::FormAttributeSet(F::MaxLength), V::Node(input), &[V::Number(-1.)]).is_err());
    call(&mut h, O::FormAttributeSet(F::MaxLength), V::Node(input), &[V::Number(7.)]);
    assert_eq!(call(&mut h, O::FormAttributeGet(F::MaxLength), V::Node(input), &[]), V::Number(7.));
    call(&mut h, O::FormAttributeSet(F::Method), V::Node(form), &[str_value("POST")]);
    assert_eq!(call(&mut h, O::FormAttributeGet(F::Method), V::Node(form), &[]), str_value("post"));
    let option = h.dom.create_element(form, "option");
    let text = h.dom.create_text(option, "  Example  label ");
    assert_ne!(option, text);
    assert_eq!(call(&mut h, O::FormAttributeGet(F::Label), V::Node(option), &[]), str_value("Example label"));
}

#[test]
fn get_html_options_include_serializable_and_explicit_shadow_roots() {
    use amazing_dom::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
    let mut h = Host::default();
    let container = h.dom.create_element(NodeIdx(0), "main");
    let host = h.dom.create_element(container, "div");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: false, serializable: false };
    let root = h.dom.attach_declarative_shadow(host, options, false).unwrap();
    h.dom.create_text(root, "hidden");
    let empty = call(&mut h, O::ElementGetHTML, V::Node(container), &[]);
    assert_eq!(empty, str_value("<div></div>"));
    let explicit = V::Dictionary(vec![("shadowRoots", V::Array(vec![V::Node(root)]))]);
    assert_eq!(call(&mut h, O::ElementGetHTML, V::Node(container), &[explicit]),
        str_value("<div><template shadowrootmode=\"closed\">hidden</template></div>"));
    let own_root = V::Dictionary(vec![("shadowRoots", V::Array(vec![V::Node(root)]))]);
    assert_eq!(call(&mut h, O::ElementGetHTML, V::Node(host), &[own_root]),
        str_value("<template shadowrootmode=\"closed\">hidden</template>"));
    let serializable = V::Dictionary(vec![("serializableShadowRoots", V::Bool(true))]);
    assert_eq!(call(&mut h, O::ElementGetHTML, V::Node(container), &[serializable]), empty);
}

// dom/nodes/Document-adoptNode.html and Document-importNode.html at
// c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
#[test]
fn document_adopt_and_import_preserve_identity_and_target_ownership() {
    let mut h = Host::default();
    let other = h.dom.create_document();
    let parent = h.dom.create_element(NodeIdx(0), "main");
    let element = h.dom.create_element(parent, "div");
    let child = h.dom.create_element(element, "span");
    h.dom.set_attribute(element, "id", "adopt-me").unwrap();
    let attr = h.dom.get_attribute_node(element, None, "id").unwrap();
    let shallow = node(call(&mut h, O::DocumentImportNode, V::Node(other), &[V::Node(element)]));
    assert_ne!(shallow, element);
    assert_eq!(h.dom.owner_document(shallow), Some(other));
    assert_eq!(h.dom.children(shallow).count(), 0);
    assert_eq!(h.dom.owner_document(element), Some(NodeIdx(0)));
    let deep = node(call(&mut h, O::DocumentImportNode, V::Node(other),
        &[V::Node(element), V::Bool(true)]));
    assert_eq!(h.dom.owner_document(h.dom.children(deep).next().unwrap()), Some(other));
    let null_options = node(call(&mut h, O::DocumentImportNode, V::Node(other),
        &[V::Node(element), V::Null]));
    assert_eq!(h.dom.children(null_options).count(), 1);
    let self_only = V::Dictionary(vec![("selfOnly", V::Bool(true))]);
    let shallow_dict = node(call(&mut h, O::DocumentImportNode, V::Node(other),
        &[V::Node(element), self_only]));
    assert_eq!(h.dom.children(shallow_dict).count(), 0);
    assert_eq!(call(&mut h, O::DocumentAdoptNode, V::Node(other), &[V::Node(element)]), V::Node(element));
    assert_eq!(h.dom.get(element).parent, None);
    assert_eq!(h.dom.owner_document(element), Some(other));
    assert_eq!(h.dom.owner_document(child), Some(other));
    assert_eq!(h.dom.owner_document(attr), Some(other));
    assert_eq!(invoke(&mut h, O::DocumentAdoptNode, V::Node(other), &[V::Node(NodeIdx(0))]),
        Err("NotSupportedError".into()));
    assert_eq!(invoke(&mut h, O::DocumentImportNode, V::Node(other), &[V::Node(NodeIdx(0))]),
        Err("NotSupportedError".into()));
}

// WPT pin 53bc812da2ed81179ebd1ba76b2d197cc4747d12:
// html/semantics/forms/constraints/form-validation-validity-patternMismatch.html and
// html/semantics/forms/the-input-element/input-valueasdate.html.
#[test]
fn pattern_host_seam_and_date_wrapping_run_outside_dom_borrow() {
    let mut h = Host::default();
    let input = h.dom.create_element(NodeIdx(0), "input");
    h.dom.set_attribute(input, "pattern", "[A-Z]+").unwrap();
    call(&mut h, O::InputSetValue, V::Node(input), &[str_value("abc")]);
    let validity = call(&mut h, O::ControlValidity, V::Node(input), &[]);
    assert_eq!(call(&mut h, O::ValidityPatternMismatch, validity.clone(), &[]), V::Bool(true));
    call(&mut h, O::InputSetValue, V::Node(input), &[str_value("ABC")]);
    assert_eq!(call(&mut h, O::ValidityPatternMismatch, validity, &[]), V::Bool(false));
    h.dom.remove_attribute(input, "pattern").unwrap();
    h.dom.set_attribute(input, "type", "date").unwrap();
    call(&mut h, O::InputSetValue, V::Node(input), &[str_value("1970-01-02")]);
    assert_eq!(call(&mut h, O::InputValueAsDate, V::Node(input), &[]), V::Number(86_400_000.));
    call(&mut h, O::InputSetValueAsDate, V::Node(input), &[V::Number(0.)]);
    assert_eq!(call(&mut h, O::InputValue, V::Node(input), &[]), str_value("1970-01-01"));
}

#[test]
fn url_bindings_keep_search_params_live_and_same_object() {
    let mut h = Host::default();
    assert_eq!(call(&mut h, O::URLCanParse, V::Undefined, &[str_value("/a"), str_value("https://example.test/")]), V::Bool(true));
    assert_eq!(call(&mut h, O::URLCanParse, V::Undefined, &[str_value("bad"), str_value("invalid base")]), V::Bool(false));
    assert_eq!(call(&mut h, O::URLParse, V::Undefined, &[str_value("bad")]), V::Null);
    let url = call(&mut h, O::URLConstructor, V::Undefined, &[str_value("/a?x=1"), str_value("https://example.test/base")]);
    assert_eq!(call(&mut h, O::URLHref, url.clone(), &[]), str_value("https://example.test/a?x=1"));
    assert_eq!(call(&mut h, O::URLOrigin, url.clone(), &[]), str_value("https://example.test"));
    let params = call(&mut h, O::URLSearchParams, url.clone(), &[]);
    assert_eq!(call(&mut h, O::URLSearchParams, url.clone(), &[]), params);
    call(&mut h, O::URLSearchParamsAppend, params.clone(), &[str_value("y"), str_value("a b")]);
    assert_eq!(call(&mut h, O::URLSearch, url.clone(), &[]), str_value("?x=1&y=a+b"));
    call(&mut h, O::URLSetSearch, url.clone(), &[str_value("?z=3")]);
    assert_eq!(call(&mut h, O::URLSearchParamsGet, params.clone(), &[str_value("z")]), str_value("3"));
    call(&mut h, O::URLSetHostname, url.clone(), &[str_value("other.test")]);
    assert_eq!(call(&mut h, O::URLHostname, url, &[]), str_value("other.test"));
}

#[test]
fn url_search_params_constructor_overloads_and_mutation_preserve_order() {
    let mut h = Host::default();
    let sequence = V::Array(vec![V::Array(vec![str_value("a"), str_value("1")]), V::Array(vec![str_value("a"), str_value("2")])]);
    let params = call(&mut h, O::URLSearchParamsConstructor, V::Undefined, &[sequence]);
    assert_eq!(call(&mut h, O::URLSearchParamsGetAll, params.clone(), &[str_value("a")]), V::Array(vec![str_value("1"), str_value("2")]));
    call(&mut h, O::URLSearchParamsDelete, params.clone(), &[str_value("a"), str_value("1")]);
    assert_eq!(call(&mut h, O::URLSearchParamsToString, params, &[]), str_value("a=2"));
    let record = V::Dictionary(vec![("b", str_value("x y")), ("a", str_value("z"))]);
    let params = call(&mut h, O::URLSearchParamsConstructor, V::Undefined, &[record]);
    call(&mut h, O::URLSearchParamsSort, params.clone(), &[]);
    assert_eq!(call(&mut h, O::URLSearchParamsToString, params, &[]), str_value("a=z&b=x+y"));
}

#[test]
fn blob_file_form_data_and_font_set_bindings_use_native_models() {
    let mut h = Host::default();
    let blob = call(&mut h, O::BlobConstructor, V::Undefined, &[
        V::Array(vec![str_value("abc"), str_value("def")]),
        V::Dictionary(vec![("type", str_value("TEXT/PLAIN"))]),
    ]);
    assert_eq!(call(&mut h, O::BlobSize, blob.clone(), &[]), V::Number(6.));
    assert_eq!(call(&mut h, O::BlobType, blob.clone(), &[]), str_value("text/plain"));
    let slice = call(&mut h, O::BlobSlice, blob.clone(), &[V::Number(1.), V::Number(4.)]);
    assert_eq!(call(&mut h, O::BlobSize, slice.clone(), &[]), V::Number(3.));
    let text = call(&mut h, O::BlobText, slice, &[]);
    assert!(matches!(text, V::Promise(_)));
    let file = call(&mut h, O::FileConstructor, V::Undefined, &[
        V::Array(vec![blob]), str_value("a/b.txt"),
        V::Dictionary(vec![("type", str_value("Text/Plain")), ("lastModified", V::Number(42.))]),
    ]);
    assert_eq!(call(&mut h, O::FileName, file.clone(), &[]), str_value("a:b.txt"));
    assert_eq!(call(&mut h, O::FileLastModified, file.clone(), &[]), V::Number(42.));
    let form_data = call(&mut h, O::FormDataConstructor, V::Undefined, &[]);
    call(&mut h, O::FormDataAppend, form_data.clone(), &[str_value("name"), str_value("one")]);
    call(&mut h, O::FormDataAppend, form_data.clone(), &[str_value("name"), str_value("two")]);
    assert_eq!(call(&mut h, O::FormDataGetAll, form_data.clone(), &[str_value("name")]), V::Array(vec![str_value("one"), str_value("two")]));
    call(&mut h, O::FormDataSet, form_data.clone(), &[str_value("name"), str_value("last")]);
    assert_eq!(call(&mut h, O::FormDataGet, form_data.clone(), &[str_value("name")]), str_value("last"));
    call(&mut h, O::FormDataAppend, form_data.clone(), &[str_value("upload"), file]);
    assert_eq!(call(&mut h, O::FormDataHas, form_data, &[str_value("upload")]), V::Bool(true));
    let fonts = call(&mut h, O::DocumentFonts, V::Node(NodeIdx(0)), &[]);
    assert_eq!(call(&mut h, O::DocumentFonts, V::Node(NodeIdx(0)), &[]), fonts);
    assert_eq!(call(&mut h, O::FontFaceSetStatus, fonts.clone(), &[]), str_value("loaded"));
    let ready = call(&mut h, O::FontFaceSetReady, fonts.clone(), &[]);
    assert_eq!(call(&mut h, O::FontFaceSetReady, fonts, &[]), ready);
}


#[test]
fn static_range_requires_all_members_and_converts_in_dictionary_order() {
    let mut h = Host::default();
    let entries = vec![
        ("startContainer", V::Node(NodeIdx(0))), ("startOffset", V::Number(0.)),
        ("endContainer", V::Node(NodeIdx(0))), ("endOffset", V::Number(0.)),
    ];
    for missing in 0..entries.len() {
        for explicit_undefined in [false, true] {
            let mut init = entries.clone();
            if explicit_undefined { init[missing].1 = V::Undefined; }
            else { init.remove(missing); }
            assert!(invoke(&mut h, O::StaticRangeConstructor, V::Undefined,
                &[V::Dictionary(init)]).is_err(), "{}", entries[missing].0);
        }
    }
    for init in [V::Null, V::Undefined] {
        assert!(invoke(&mut h, O::StaticRangeConstructor, V::Undefined, &[init]).is_err());
    }
    h.property_reads.clear();
    call(&mut h, O::StaticRangeConstructor, V::Undefined, &[V::Dictionary(entries.clone())]);
    assert_eq!(h.property_reads, ["endContainer", "endOffset", "startContainer", "startOffset"]);
    // null is a present numeric member and converts to zero, unlike undefined.
    let mut init = entries;
    init[1].1 = V::Null;
    init[3].1 = V::Null;
    call(&mut h, O::StaticRangeConstructor, V::Undefined, &[V::Dictionary(init)]);
}

#[path = "tests_message_events.rs"]
mod message_events;

#[path = "tests_event_dictionaries.rs"]
mod event_dictionaries;

#[path = "tests_legacy_events.rs"]
mod legacy_events;

#[path = "tests_dom_exception.rs"]
mod dom_exception;

#[path = "tests_dataset.rs"]
mod tests_dataset;

#[path = "tests_legacy_document.rs"]
mod tests_legacy_document;

#[path = "tests_listener_registration.rs"]
mod tests_listener_registration;

#[path = "tests_tokenlist_semantics.rs"]
mod tests_tokenlist_semantics;

#[test]
fn document_default_view_uses_host_association_and_enforces_document_brand() {
    let mut h = Host::default();
    let window = V::Dictionary(vec![]);
    h.window = Some((0, window.clone()));
    assert_eq!(call(&mut h, O::DocumentDefaultView, V::Node(NodeIdx(0)), &[]), window);
    let inert = call(&mut h, O::DocumentConstructor, V::Undefined, &[]);
    assert_eq!(call(&mut h, O::DocumentDefaultView, inert, &[]), V::Null);
    let node = element(&mut h);
    assert!(invoke(&mut h, O::DocumentDefaultView, V::Node(node), &[]).is_err());
}

#[path = "tests_event_modifiers.rs"]
mod tests_event_modifiers;

#[path = "tests_navigation_events.rs"]
mod tests_navigation_events;

#[path = "tests_event_types.rs"]
mod tests_event_types;

#[path = "tests_error_events.rs"]
mod error_events;

#[path = "tests_dom_b_reflect.rs"]
mod tests_dom_b_reflect;

#[path = "tests_observable.rs"]
mod tests_observable;
#[path = "tests_set_html_unsafe.rs"]
mod tests_set_html_unsafe;

#[test]
fn native_ua_payload_cancellation_and_activation_share_one_dispatch() {
    use amazing_dom::events::{Event, EventClass, EventExtra, EventInit, EventTarget};
    use std::time::Duration;
    let mut host=Host::default();
    let input=host.dom.create_element_ns(NodeIdx(0),Some("http://www.w3.org/1999/xhtml"),"input").unwrap();
    host.dom.set_attribute(input,"type","checkbox").unwrap();
    host.dom.set_input_indeterminate(input,true).unwrap();
    let listener=host.script(move |host,receiver,args| {
        assert_eq!(receiver,&V::Node(input));
        assert_eq!(call(host,O::EventIsTrusted,args[0].clone(),&[]),V::Bool(true));
        assert_eq!(call(host,O::EventTimeStamp,args[0].clone(),&[]),V::Number(12.));
        assert_eq!(call(host,O::EventExtra("clientX"),args[0].clone(),&[]),V::Number(31.5));
        assert_eq!(call(host,O::EventExtra("shiftKey"),args[0].clone(),&[]),V::Bool(true));
        assert!(host.dom.input_checked(input).unwrap());
        assert!(!host.dom.input_indeterminate(input).unwrap());
        Ok(V::Undefined)
    });
    call(&mut host,O::AddEventListener,V::Node(input),&[str_value("click"),listener]);
    for (kind,id) in [("input",41),("change",42)] {
        call(&mut host,O::AddEventListener,V::Node(input),&[str_value(kind),V::Callable(id)]);
    }
    let click=|| {
        let mut event=Event::new("click",EventInit {bubbles:true,cancelable:true,composed:true},Duration::from_millis(12));
        event.set_class(EventClass::Mouse);
        event.set_extra("clientX",EventExtra::Number(31.5));
        event.set_extra("shiftKey",EventExtra::Boolean(true));
        event
    };
    host.cancel_events=true;
    assert!(!dispatch_user_agent_event_with_payload(&mut host,EventTarget::Node(input),click()).unwrap());
    assert!(!host.dom.input_checked(input).unwrap());
    assert!(host.dom.input_indeterminate(input).unwrap());
    assert!(host.calls.is_empty());
    host.cancel_events=false;
    assert!(dispatch_user_agent_event_with_payload(&mut host,EventTarget::Node(input),click()).unwrap());
    assert!(host.dom.input_checked(input).unwrap());
    assert!(!host.dom.input_indeterminate(input).unwrap());
    assert_eq!(host.calls,vec![41,42]);
}
