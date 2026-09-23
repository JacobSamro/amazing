//! Driver-owned WebIDL installation over the same arena used for page parsing.
//! DOMString values cross the runtime boundary as exact UTF-16 code units.
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use std::collections::HashSet;
use std::collections::{BTreeMap, VecDeque};
use std::{path::PathBuf, rc::Weak};

use amazing_bindings::{
    deliver_custom_element_reactions, deliver_mutation_checkpoint, fire_abort_timeout,
    html_element_interface, registry_for_document,
    dom_exception::throw_dom_error,
    host::{
        BindingHost, CallbackId, HostCallPolicy, Interface, InterfaceBrand, InterfaceDefinition,
        MemberKind, Operation, OptionalHookInstall, PlatformKey, RootId, LegacyPlatformHooks, IterableKind,
        FragmentParser, FragmentContext, SelectorEngine, AbortHost, DocumentHost, DocumentField, UrlReflection,
    },
    install as install_interfaces, invoke,
};
use amazing_dom::{Dom, DomError, NodeIdx, NodeKind, Namespace, abort::AbortSignalId};
use amazing_bindings::{Blob, BlobPart, File, FormData};
use amazing_bindings::storage_interfaces::{self, StorageInterface as SI, MemberKind as SM};
use amazing_storage::{core::{BucketKey, DiskStore, StorageKey}, web::{PersistentWebStorage, StorageChange}, idb::{self, IndexedDb, StructuredValue, Key, KeyRange, Direction, Mode, Durability, Transaction as IdbTransaction, Cursor as IdbCursor}, cache::{self, PersistentCacheStorage}};
use amazing_net::url::{UrlSearchParams, UrlWithSearchParams};
use amazing_js::value::Exotic;
use amazing_js::{
    DomHost,
    env::{EnvHandle, EnvKind},
    interp::{BindingPropertyFlags, HostNativeDispatch, Interpreter, LegacyPlatformHooks as RuntimeLegacyHooks},
    value::{
        FunctionBody, HostCallPolicy as JsCallPolicy, HostFunctionId, HostHandle, JsValue, Throw,
    },
};

use crate::dom_host::BrowserDomHost;
use amazing_bindings::canvas_interfaces::{self, CanvasSlot, Member, MemberKind as CanvasMemberKind};
use amazing_canvas::{self as canvas, FillRule, ImageData as CanvasImageData, PaintStyle, Path2D, Transform};

#[derive(Clone, Copy)]
enum NativeEntry {
    Canvas(CanvasTarget, Member),
    Operation(Operation),
    IllegalConstructor,
    CapturePromiseResolvers,
    Storage(SI, &'static str, SM),
    StorageTick,
}

struct StorageRuntime {
    web: Rc<RefCell<PersistentWebStorage>>,
    idb: Rc<RefCell<IndexedDb>>,
    cache: PersistentCacheStorage,
    objects: Vec<StorageObject>,
    wrappers: HashMap<usize, JsValue>,
    pending: VecDeque<StorageJob>,
    blocked_opens: VecDeque<(StorageKey,String,usize)>,
    blocked_deletes: VecDeque<(StorageKey,String,usize)>,
    tick_scheduled: bool,
}

thread_local! {
    static SHARED_WEB_STORAGE: RefCell<HashMap<PathBuf,Weak<RefCell<PersistentWebStorage>>>>=RefCell::new(HashMap::new());
    static SHARED_INDEXED_DB: RefCell<HashMap<PathBuf,Weak<RefCell<IndexedDb>>>>=RefCell::new(HashMap::new());
}

enum StorageObject {
    Area { session: bool },
    Factory,
    Request { result: JsValue, error: JsValue, ready: bool, source: JsValue, transaction: JsValue, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Database { connection: u64, name: String, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Transaction { connection: u64, id: u64, value: Option<IdbTransaction>, mode: Mode, scope: Vec<String>, queued: bool, active_for_requests: bool, handlers: HashMap<String, JsValue>, listeners: Vec<(String, JsValue)> },
    Store { transaction: usize, name: String },
    Index { transaction: usize, store: String, name: String },
    Cursor { cursor: IdbCursor, transaction: usize, source: usize, request: usize, with_value: bool },
    Range(KeyRange),
    CacheStorage,
    Cache { name: String },
}

enum StorageJob {
    RequestSuccess { request: usize, result: JsValue },
    RequestError { request: usize, error: JsValue },
    DeleteSuccess { request: usize, old: u64 },
    OpenUpgrade { request: usize, connection: u64, name: String, old: u64, new: u64 },
    OpenBlocked { request: usize, old: u64, new: Option<u64> },
    DatabaseVersionChange { connection: u64, old: u64, new: Option<u64> },
    DispatchListener { slot: usize, event: JsValue, callbacks: VecDeque<JsValue> },
    TransactionComplete { transaction: usize },
    StorageEvent(StorageChange),
}

enum StoreResult {
    None,
    Count(usize),
    Key(Key),
    OptionalKey(Option<Key>),
    Keys(Vec<Key>),
    Value(Option<StructuredValue>),
    Values(Vec<StructuredValue>),
}

#[derive(Clone, Copy)]
enum CanvasTarget { Element, Context, Path, Gradient, Pattern, ImageData, TextMetrics }

#[derive(Clone, Copy)]
enum CanvasGradientKind { Linear, Radial, Conic }

enum CanvasObject {
    Context(NodeIdx, JsValue),
    Path(Path2D),
    Gradient(canvas::Gradient, CanvasGradientKind),
    Pattern(canvas::Pattern, bool),
    ImageData(u32, u32, JsValue),
    TextMetrics(canvas::TextMetrics),
}

enum WebObject {
    DomParser,
    Url(UrlWithSearchParams, Option<u64>),
    Params(UrlSearchParams),
    Blob(Blob),
    File(File),
    FormData(FormData),
    FontFaceSet(NodeIdx, Option<JsValue>),
}

struct BindingState {
    canvas_text_system: Option<Rc<dyn amazing_text::TextSystem>>,
    canvas_dirty: HashSet<NodeIdx>,
    canvas_style_values: HashMap<(NodeIdx, &'static str), JsValue>,
    canvas_slots: HashMap<NodeIdx, CanvasSlot>,
    canvas_objects: Vec<(JsValue, CanvasObject)>,
    host: Rc<RefCell<BrowserDomHost>>,
    arena: u64,
    functions: Vec<NativeEntry>,
    interfaces: Vec<(Interface, JsValue, JsValue)>,
    roots: Vec<Option<JsValue>>,
    captured_resolvers: Option<(JsValue, JsValue)>,
    promise_resolvers: HashMap<usize, (JsValue, JsValue)>,
    legacy_document: JsValue,
    legacy_elements: HashMap<u32, JsValue>,
    abort_timeouts: Vec<(AbortSignalId, u64)>,
    document_url: Option<String>,
    document_character_set: Option<String>,
    document_content_type: Option<String>,
    document_referrer: Option<String>,
    web_objects: Vec<WebObject>,
    font_sets: HashMap<NodeIdx, u64>,
    storage: Option<StorageRuntime>,
    storage_classes: HashMap<SI, (JsValue, JsValue)>,
}

/// A short-lived adapter. JS calls never hold a borrow of BindingState or Dom.
pub(crate) struct RuntimeBindingHost<'a> {
    js: &'a mut Interpreter,
    state: Rc<RefCell<BindingState>>,
}

#[derive(Clone)]
pub(crate) struct RuntimeBindings {
    state: Rc<RefCell<BindingState>>,
}

impl RuntimeBindings {
    pub(crate) fn set_canvas_text_system(&self, system: Rc<dyn amazing_text::TextSystem>) {
        self.state.borrow_mut().canvas_text_system = Some(system);
    }
    /// Rendering-update seam: drain changed bitmaps after script jobs and
    /// before building the compositor frame.
    pub(crate) fn take_canvas_damage(&self) -> Vec<(NodeIdx, amazing_paint::Surface)> {
        let mut state = self.state.borrow_mut();
        let changed = std::mem::take(&mut state.canvas_dirty);
        changed.into_iter().filter_map(|node| state.canvas_slots.get(&node)
            .map(|slot| (node, slot.context.surface.clone()))).collect()
    }
    pub(crate) fn create_inert_document(&self, js: &mut Interpreter, bytes: &[u8],
        content_type: &str, url: &str, charset: Option<&str>) -> Result<NodeIdx, DomError> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        adapter.create_inert_document(bytes, content_type, url, charset)
    }

    /// IndexedDB transactions stop accepting new requests after the task's
    /// microtask checkpoint. Request event dispatch reactivates its transaction.
    pub(crate) fn storage_deactivate_transactions(&self) {
        let mut state=self.state.borrow_mut();
        if let Some(storage)=state.storage.as_mut(){
            for object in &mut storage.objects {
                if let StorageObject::Transaction{active_for_requests,..}=object {*active_for_requests=false;}
            }
        }
    }
    pub(crate) fn set_document_url(&self, url: &str) {
        self.state.borrow_mut().document_url = Some(url.to_owned());
    }

    pub(crate) fn set_document_metadata(&self, character_set: &str, content_type: &str, referrer: &str) {
        let mut state = self.state.borrow_mut();
        state.document_character_set = Some(character_set.to_owned());
        state.document_content_type = Some(content_type.to_owned());
        state.document_referrer = Some(referrer.to_owned());
    }

    pub(crate) fn set_document_content_type(&self, content_type: &str) {
        self.state.borrow_mut().document_content_type = Some(content_type.to_owned());
    }

    pub(crate) fn take_abort_timeouts(&self) -> Vec<(AbortSignalId, u64)> {
        std::mem::take(&mut self.state.borrow_mut().abort_timeouts)
    }

    pub(crate) fn fire_abort_timeout(&self, js: &mut Interpreter, id: AbortSignalId) -> Result<(), Throw> {
        fire_abort_timeout(&mut RuntimeBindingHost { js, state: self.state.clone() }, id)
    }
    pub(crate) fn dispatch_event(&self, js: &mut Interpreter, node: u32, name: &str) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        let prototype = adapter.js.binding_object_prototype();
        let options = adapter.create_object(Some(&prototype))?;
        adapter.js.binding_define_data_property(&options, "bubbles", JsValue::Bool(true),
            BindingPropertyFlags::METHOD)?;
        let event = invoke(&mut adapter, Operation::EventConstructor, JsValue::Undefined,
            &[JsValue::str(name), options])?;
        let target = adapter.wrap_node(NodeIdx(node))?;
        invoke(&mut adapter, Operation::DispatchEvent, target, &[event])?;
        Ok(())
    }

    pub(crate) fn dispatch_animation_event(&self, js: &mut Interpreter, animation: &amazing_compositor::CssAnimationEvent) -> Result<(), Throw> {
        use amazing_compositor::CssAnimationEventKind as Kind;
        let (name, property) = match animation.kind {
            Kind::AnimationStart => ("animationstart", "animationName"),
            Kind::AnimationIteration => ("animationiteration", "animationName"),
            Kind::AnimationEnd => ("animationend", "animationName"),
            Kind::AnimationCancel => ("animationcancel", "animationName"),
            Kind::TransitionRun => ("transitionrun", "propertyName"),
            Kind::TransitionStart => ("transitionstart", "propertyName"),
            Kind::TransitionEnd => ("transitionend", "propertyName"),
            Kind::TransitionCancel => ("transitioncancel", "propertyName"),
        };
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        let prototype = adapter.js.binding_object_prototype();
        let options = adapter.create_object(Some(&prototype))?;
        adapter.js.binding_define_data_property(&options, "bubbles", JsValue::Bool(true), BindingPropertyFlags::METHOD)?;
        let event = invoke(&mut adapter, Operation::EventConstructor, JsValue::Undefined,
            &[JsValue::str(name), options])?;
        adapter.js.binding_define_data_property(&event, property, JsValue::str(&animation.name), BindingPropertyFlags::METHOD)?;
        adapter.js.binding_define_data_property(&event, "elapsedTime", JsValue::num(animation.elapsed_time_ms / 1000.), BindingPropertyFlags::METHOD)?;
        adapter.js.binding_define_data_property(&event, "pseudoElement", JsValue::str(""), BindingPropertyFlags::METHOD)?;
        let target = adapter.wrap_node(NodeIdx(animation.element as u32))?;
        invoke(&mut adapter, Operation::DispatchEvent, target, &[event])?;
        Ok(())
    }

    pub(crate) fn deliver_mutations(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost {
            js,
            state: self.state.clone(),
        };
        deliver_mutation_checkpoint(&mut adapter)
    }

    pub(crate) fn deliver_reactions(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        deliver_custom_element_reactions(&mut adapter)
    }

    #[cfg(test)]
    pub(crate) fn seed_test_root(&self, id: RootId, value: JsValue) {
        let mut state = self.state.borrow_mut();
        state.roots.resize(id.0 as usize + 1, None);
        state.roots[id.0 as usize] = Some(value);
    }
}

impl RuntimeBindingHost<'_> {
    fn materialize_location_unforgeables(&self, wrapper: &JsValue) -> Result<(), Throw> {
        let JsValue::Object(object) = wrapper else { return Err(Throw::type_error("Location wrapper")); };
        let prototype = self.prototype(Interface::Location)
            .ok_or_else(|| Throw::type_error("Location prototype missing"))?;
        let JsValue::Object(prototype) = prototype else { return Err(Throw::type_error("Location prototype")); };
        for name in ["href", "origin", "protocol", "host", "hostname", "port",
            "pathname", "search", "hash", "assign", "replace", "reload",
            "ancestorOrigins", "toString"] {
            let Some(mut property) = prototype.borrow().own_prop(name).cloned() else { continue };
            property.configurable = false;
            object.borrow_mut().define(name, property)
                .map_err(|_| Throw::type_error("Cannot install Location member"))?;
        }
        Ok(())
    }

    fn materialize_document_location_unforgeable(&self, wrapper: &JsValue) -> Result<(), Throw> {
        let JsValue::Object(object) = wrapper else { return Ok(()); };
        let location_property = { object.borrow().own_prop("location").cloned() };
        let Some(mut property) = location_property else { return Ok(()); };
        property.configurable = false;
        object.borrow_mut().define("location", property)
            .map_err(|_| Throw::type_error("Cannot install Document.location"))
    }

    fn hook_receiver(&mut self, handle: HostHandle) -> Result<JsValue, Throw> {
        self.wrap_platform_object(PlatformKey { arena: handle.arena, slot: handle.slot,
            generation: handle.generation }, InterfaceBrand(handle.brand))
    }
    fn materialize_accessors(&self, wrapper: &JsValue) {
        let JsValue::Object(object) = wrapper else { return; };
        let mut prototype = object.borrow().proto.clone();
        while let Some(current) = prototype {
            let (next, accessors) = {
                let current = current.borrow();
                let accessors = current.own_keys_enumerable().into_iter()
                    .filter_map(|name| current.own_prop(&name).cloned()
                        .filter(|property| property.getter.is_some() || property.setter.is_some())
                        .map(|property| (name, property)))
                    .collect::<Vec<_>>();
                (current.proto.clone(), accessors)
            };
            for (name, property) in accessors {
                let mut object = object.borrow_mut();
                if object.own_prop(&name).is_none() {
                    let _ = object.define(&name, property);
                }
            }
            prototype = next;
        }
    }

    fn preserve_legacy_element_extensions(&mut self, node: NodeIdx, wrapper: &JsValue) -> Result<(), Throw> {
        let host = self.state.borrow().host.clone();
        let connected = host.borrow().dom.is_connected(node);
        let old = self.state.borrow().legacy_elements.get(&node.0).cloned();
        let old = if let Some(old) = old { old } else if !connected {
            let value = self.js.binding_make_legacy_dom_element(node.0);
            self.state.borrow_mut().legacy_elements.insert(node.0, value.clone());
            value
        } else {
            let document = self.state.borrow().legacy_document.clone();
            let query = self.js.binding_get_property(&document, "querySelectorAll")?;
            let all = binding_call(self.js, &query, &document, &[JsValue::str("*")])?;
            let length = self.js.binding_get_property(&all, "length")?;
            let count = match length { JsValue::Num(n) if n.is_finite() && n >= 0.0 => n as usize, _ => 0 };
            let mut found = None;
            for index in 0..count {
                let value = self.js.binding_get_property(&all, &index.to_string())?;
                let Some(slot) = (match &value {
                    JsValue::Object(object) => object.borrow().get_own("[[DomNode]]"),
                    _ => None,
                }) else { continue };
                if let JsValue::Num(slot) = slot {
                    let slot = slot as u32;
                    self.state.borrow_mut().legacy_elements.insert(slot, value.clone());
                    if slot == node.0 { found = Some(value); }
                }
            }
            let Some(old) = found else { return Ok(()); };
            old
        };
        let (JsValue::Object(old), JsValue::Object(new)) = (old, wrapper) else {
            return Ok(());
        };
        let extensions = ["dataset", "style", "classList", "matches", "closest",
            "hasAttribute", "removeAttribute", "innerHTML", "innerText", "onclick", "click"];
        let properties = {
            let old = old.borrow();
            extensions.iter().filter_map(|name| old.own_prop(*name).cloned().map(|prop| (*name, prop)))
                .collect::<Vec<_>>()
        };
        // The legacy extensions still use the old node marker internally. The
        // binding brand remains a separate private runtime slot.
        new.borrow_mut().set_own("[[DomNode]]", JsValue::num(f64::from(node.0)));
        for (name, property) in properties {
            // Keep the installed WebIDL member when both layers expose a
            // name. The fallback only fills gaps in the DOM-7 surface.
            let mut object = Some(new.clone());
            let mut present = false;
            while let Some(current) = object {
                let current = current.borrow();
                if current.own_prop(name).is_some() { present = true; break; }
                object = current.proto.clone();
            }
            if !present {
                new.borrow_mut().define(name, property)
                    .map_err(|_| Throw::type_error("Cannot install legacy extension"))?;
            }
        }
        Ok(())
    }

    fn prototype(&self, interface: Interface) -> Option<JsValue> {
        self.state
            .borrow()
            .interfaces
            .iter()
            .find(|(kind, _, _)| *kind == interface)
            .map(|(_, prototype, _)| prototype.clone())
    }

    fn constructor(&self, interface: Interface) -> Option<JsValue> {
        self.state
            .borrow()
            .interfaces
            .iter()
            .find(|(kind, _, _)| *kind == interface)
            .map(|(_, _, constructor)| constructor.clone())
    }

    fn native(
        &mut self,
        name: &'static str,
        length: u8,
        entry: NativeEntry,
        policy: HostCallPolicy,
    ) -> JsValue {
        let mut state = self.state.borrow_mut();
        let id = HostFunctionId(state.functions.len() as u64);
        state.functions.push(entry);
        drop(state);
        let policy = match policy {
            HostCallPolicy::CallOnly => JsCallPolicy::CallOnly,
            HostCallPolicy::ConstructOnly => JsCallPolicy::ConstructOnly,
            HostCallPolicy::CallAndConstruct => JsCallPolicy::CallAndConstruct,
        };
        self.js
            .binding_create_native_function(id, name, length, policy)
    }

    fn interface_for(&self, key: PlatformKey, brand: InterfaceBrand) -> Result<Interface, Throw> {
        if brand == InterfaceBrand::NODE {
            let node = self
                .node_from_platform_key(key)
                .ok_or_else(|| Throw::type_error("Invalid node wrapper"))?;
            let host = self.state.borrow().host.clone();
            let host = host.borrow();
            return Ok(match &host.dom.get(node).kind {
                NodeKind::Document if host.dom.document_metadata(node).ok().flatten()
                    .is_some_and(|metadata| metadata.content_type != "text/html") => Interface::XMLDocument,
                NodeKind::Document => Interface::Document,
                NodeKind::DocumentFragment if host.dom.is_shadow_root(node) => Interface::ShadowRoot,
                NodeKind::DocumentFragment => Interface::DocumentFragment,
                NodeKind::DocumentType { .. } => Interface::DocumentType,
                NodeKind::Element { .. } => {
                    if host.dom.namespace(node) == Some(Namespace::Html) {
                        html_element_interface(host.dom.name(node).unwrap_or(""))
                    } else { Interface::Element }
                },
                NodeKind::Text { .. } => Interface::Text,
                NodeKind::CDATASection { .. } => Interface::CDATASection,
                NodeKind::Attr { .. } => Interface::Attr,
                NodeKind::Comment { .. } => Interface::Comment,
                NodeKind::ProcessingInstruction { .. } => Interface::ProcessingInstruction,
            });
        }
        Ok(match brand {
            InterfaceBrand::RANGE => Interface::Range,
            InterfaceBrand::STATIC_RANGE => Interface::StaticRange,
            InterfaceBrand::TRAVERSAL => Interface::NodeIterator,
            InterfaceBrand::NODE_ITERATOR => Interface::NodeIterator,
            InterfaceBrand::TREE_WALKER => Interface::TreeWalker,
            InterfaceBrand::EVENT => Interface::Event,
            InterfaceBrand::EVENT_TARGET => Interface::EventTarget,
            InterfaceBrand::MUTATION_OBSERVER => Interface::MutationObserver,
            InterfaceBrand::MUTATION_RECORD => Interface::MutationRecord,
            InterfaceBrand::ATTRIBUTE => Interface::Attr,
            InterfaceBrand::COLLECTION => Interface::HTMLCollection,
            InterfaceBrand::FORM_CONTROLS => Interface::HTMLFormControlsCollection,
            InterfaceBrand::OPTIONS => Interface::HTMLOptionsCollection,
            InterfaceBrand::RADIO_NODE_LIST => Interface::RadioNodeList,
            InterfaceBrand::VALIDITY_STATE => Interface::ValidityState,
            InterfaceBrand::CUSTOM_ELEMENT_REGISTRY => Interface::CustomElementRegistry,
            InterfaceBrand::NODE_LIST => Interface::NodeList,
            InterfaceBrand::STATIC_NODE_LIST => Interface::NodeList,
            InterfaceBrand::FILTERED_NODE_LIST => Interface::NodeList,
            InterfaceBrand::NAMED_NODE_MAP => Interface::NamedNodeMap,
            InterfaceBrand::DOM_TOKEN_LIST => Interface::DOMTokenList,
            InterfaceBrand::LOCATION => Interface::Location,
            InterfaceBrand::DOM_STRING_LIST => Interface::DOMStringList,
            InterfaceBrand::DOM_EXCEPTION => Interface::DOMException,
            InterfaceBrand::DOM_IMPLEMENTATION => Interface::DOMImplementation,
            InterfaceBrand::ABORT_CONTROLLER => Interface::AbortController,
            InterfaceBrand::ABORT_SIGNAL => Interface::AbortSignal,
            InterfaceBrand::URL => Interface::URL,
            InterfaceBrand::URL_SEARCH_PARAMS => Interface::URLSearchParams,
            InterfaceBrand::BLOB => Interface::Blob,
            InterfaceBrand::FILE => Interface::File,
            InterfaceBrand::FORM_DATA => Interface::FormData,
            InterfaceBrand::FONT_FACE_SET => Interface::FontFaceSet,
            InterfaceBrand::DOM_PARSER => Interface::DOMParser,
            _ => return Err(Throw::type_error("Unknown platform brand")),
        })
    }


}

/// Install once before the first page script. The existing legacy host remains
/// available for browser features that DOM-7 does not yet define.
pub(crate) fn install(
    js: &mut Interpreter,
    host: Rc<RefCell<BrowserDomHost>>,
) -> Result<RuntimeBindings, Throw> {
    let arena = Rc::as_ptr(&host) as u64;
    let state = Rc::new(RefCell::new(BindingState {
        canvas_text_system: None,
        canvas_dirty: HashSet::new(),
        canvas_style_values: HashMap::new(),
        canvas_slots: HashMap::new(),
        canvas_objects: Vec::new(),
        host,
        arena,
        functions: Vec::new(),
        interfaces: Vec::new(),
        roots: Vec::new(),
        captured_resolvers: None,
        promise_resolvers: HashMap::new(),
        legacy_document: js.binding_get_property(&js.binding_global_object(), "document")?,
        legacy_elements: HashMap::new(),
        abort_timeouts: Vec::new(),
        document_url: None,
        document_character_set: None,
        document_content_type: None,
        document_referrer: None,
        web_objects: Vec::new(),
        font_sets: HashMap::new(),
        storage: None,
        storage_classes: HashMap::new(),
    }));
    let for_dispatch = state.clone();
    let dispatch: Rc<HostNativeDispatch> = Rc::new(move |js, id, this, args, _new_target| {
        let entry = for_dispatch
            .borrow()
            .functions
            .get(id.0 as usize)
            .copied()
            .ok_or_else(|| Throw::type_error("Unknown native binding"))?;
        let mut adapter = RuntimeBindingHost {
            js,
            state: for_dispatch.clone(),
        };
        match entry {
            NativeEntry::Canvas(target, member) => adapter.invoke_canvas(target, member, this, &args),
            NativeEntry::Operation(operation) if matches!(operation,
                Operation::ElementSetAttribute | Operation::ElementRemoveAttribute) => {
                let canvas_node = adapter.canvas_element(&this).ok();
                let dimension = args.first().and_then(|value| adapter.js.binding_string_units(value))
                    .map(|units| String::from_utf16_lossy(&units).to_ascii_lowercase())
                    .is_some_and(|name| name == "width" || name == "height");
                let result = invoke(&mut adapter, operation, this, &args);
                if result.is_ok() && dimension {
                    if let Some(node) = canvas_node { adapter.canvas_reset(node); }
                }
                result
            }
            NativeEntry::Operation(operation) => invoke(&mut adapter, operation, this, &args),
            NativeEntry::IllegalConstructor => Err(Throw::type_error("Illegal constructor")),
            NativeEntry::CapturePromiseResolvers => {
                let (Some(resolve), Some(reject)) = (args.first(), args.get(1)) else {
                    return Err(Throw::type_error("Promise executor missing resolvers"));
                };
                adapter.state.borrow_mut().captured_resolvers = Some((resolve.clone(), reject.clone()));
                Ok(JsValue::Undefined)
            }
            NativeEntry::Storage(interface, member, kind) => adapter.storage_invoke(interface, member, kind, this, &args),
            NativeEntry::StorageTick => adapter.storage_tick(),
        }
    });
    js.set_host_native_dispatcher(dispatch);
    let mut adapter = RuntimeBindingHost {
        js,
        state: state.clone(),
    };
    install_interfaces(&mut adapter)?;
    adapter.install_storage_interfaces()?;
    adapter.install_canvas()?;
    let location = adapter.wrap_platform_object(PlatformKey {
        arena, slot: 0, generation: 1,
    }, InterfaceBrand::LOCATION)?;
    adapter.js.binding_replace_location_object(location)?;
    if let Some(element_prototype) = adapter.prototype(Interface::Element) {
        adapter.js.binding_install_put_forwards(&element_prototype, "style", "cssText")?;
    }
    if let Some(document_prototype) = adapter.prototype(Interface::Document) {
        adapter.js.binding_install_put_forwards(&document_prototype, "location", "href")?;
    }
    let window = adapter.js.binding_global_object();
    adapter.js.binding_install_put_forwards(&window, "location", "href")?;
    let document = adapter.wrap_node(NodeIdx(0))?;
    // set_dom_host installs an immutable lexical legacy document. Shadow it in
    // a new global environment so every new page script sees the canonical
    // private wrapper while older browser globals remain available upstream.
    let fresh = EnvHandle::new(EnvKind::Global, Some(adapter.js.global_env.clone()));
    fresh.declare_lexical("document", document.clone(), false)?;
    let local_storage=adapter.storage_allocate(SI::Storage,StorageObject::Area{session:false})?;
    let session_storage=adapter.storage_allocate(SI::Storage,StorageObject::Area{session:true})?;
    fresh.declare_lexical("localStorage",local_storage,false)?;
    fresh.declare_lexical("sessionStorage",session_storage,false)?;
    adapter.js.global_env = fresh;
    adapter
        .js
        .global_obj
        .borrow_mut()
        .set_own("document", document);
    Ok(RuntimeBindings { state })
}

impl RuntimeBindingHost<'_> {
    fn web_allocate(&mut self, object: WebObject, brand: InterfaceBrand) -> Result<JsValue, Throw> {
        let slot = {
            let mut state = self.state.borrow_mut();
            let slot = state.web_objects.len() as u64;
            state.web_objects.push(object);
            slot
        };
        let arena = self.state.borrow().arena;
        self.wrap_platform_object(PlatformKey { arena, slot, generation: 1 }, brand)
    }
    fn web_slot(&self, value: &JsValue, brand: InterfaceBrand) -> Result<usize, Throw> {
        let key = self.platform_object(value, brand)
            .ok_or_else(|| Throw::type_error("Incompatible web object receiver"))?;
        let slot = key.slot as usize;
        if slot >= self.state.borrow().web_objects.len() {
            return Err(Throw::type_error("Stale web object receiver"));
        }
        Ok(slot)
    }
}

impl BindingHost for RuntimeBindingHost<'_> {
    type Value = JsValue;
    type Error = Throw;

    fn dom_parser_new(&mut self) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::DomParser, InterfaceBrand::DOM_PARSER)
    }
    fn create_inert_document(&mut self, bytes: &[u8], content_type: &str,
        url: &str, charset: Option<&str>) -> Result<NodeIdx, DomError> {
        let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let (parsed, detected_encoding) = if mime == "text/html" {
            let decoded = amazing_html::encoding::decode(bytes, charset);
            (amazing_html::parse_bytes(bytes, charset), decoded.encoding)
        } else if matches!(mime.as_str(), "text/xml" | "application/xml" | "application/xhtml+xml" | "image/svg+xml")
            || mime.ends_with("+xml") {
            let (source, encoding) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                (String::from_utf8(bytes[3..].to_vec()).map_err(|_| DomError::SyntaxError)?,
                    amazing_html::encoding::Encoding::Utf8)
            } else if !bytes.starts_with(&[0xFE, 0xFF]) && !bytes.starts_with(&[0xFF, 0xFE])
                && charset.is_none_or(|label| label.eq_ignore_ascii_case("UTF-8")) {
                (String::from_utf8(bytes.to_vec()).map_err(|_| DomError::SyntaxError)?,
                    amazing_html::encoding::Encoding::Utf8)
            } else {
                let decoded = amazing_html::encoding::decode(bytes, charset);
                if charset.is_some() && !matches!(decoded.source,
                    amazing_html::encoding::Source::Transport | amazing_html::encoding::Source::Bom) {
                    return Err(DomError::SyntaxError);
                }
                (decoded.text, decoded.encoding)
            };
            (amazing_html::parse_xml(&source).map_err(|_| DomError::SyntaxError)?, encoding)
        } else { return Err(DomError::NotSupportedError); };
        let character_set = if detected_encoding == amazing_html::encoding::Encoding::Utf8 {
            "UTF-8"
        } else { detected_encoding.name() };
        let host = self.state.borrow().host.clone();
        let result = host.borrow_mut().dom.import_inert_document(&parsed, url, &mime,
            character_set);
        result
    }

    fn global_object(&mut self) -> Result<JsValue, Throw> {
        Ok(self.js.binding_global_object())
    }
    fn create_object(&mut self, prototype: Option<&JsValue>) -> Result<JsValue, Throw> {
        self.js.binding_create_object(prototype)
    }
    fn create_native_function(
        &mut self,
        name: &'static str,
        length: u8,
        operation: Operation,
        policy: HostCallPolicy,
    ) -> Result<JsValue, Throw> {
        Ok(self.native(name, length, NativeEntry::Operation(operation), policy))
    }
    fn define_data_property(
        &mut self,
        object: &JsValue,
        name: &'static str,
        value: JsValue,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> Result<(), Throw> {
        self.js.binding_define_data_property(
            object,
            name,
            value,
            BindingPropertyFlags {
                writable,
                enumerable,
                configurable,
            },
        )
    }
    fn define_accessor_property(
        &mut self,
        object: &JsValue,
        name: &'static str,
        getter: Option<JsValue>,
        setter: Option<JsValue>,
        enumerable: bool,
        configurable: bool,
    ) -> Result<(), Throw> {
        self.js.binding_define_accessor_property(
            object,
            name,
            getter.as_ref(),
            setter.as_ref(),
            enumerable,
            configurable,
        )
    }
    fn define_legacy_platform_object(&mut self, brand: InterfaceBrand, hooks: LegacyPlatformHooks) -> Result<(), Throw> {
        let state = self.state.clone();
        let count_op = hooks.supported_index_count;
        let count = Rc::new(move |handle: HostHandle| {
            let borrowed = state.borrow();
            if handle.arena != borrowed.arena || handle.generation != 1 || handle.brand != brand.0 {
                return Err(Throw::type_error("Invalid legacy platform object"));
            }
            let mut host = borrowed.host.borrow_mut();
            let dom = &mut host.dom;
            let length = match count_op {
                Operation::HTMLCollectionSupportedIndexCount => dom.collection_length(handle.slot as usize),
                Operation::FormCollectionSupportedIndexCount => dom.form_controls(NodeIdx(handle.slot as u32)).map(|items| items.len()),
                Operation::OptionsSupportedIndexCount => dom.select_options(NodeIdx(handle.slot as u32)).map(|items| items.len()),
                Operation::RadioNodeListSupportedIndexCount => dom.radio_list_items(handle.slot as usize).map(|items| items.len()),
                Operation::NodeListSupportedIndexCount => match brand {
                    InterfaceBrand::NODE_LIST => Ok(dom.children(NodeIdx(handle.slot as u32)).count()),
                    InterfaceBrand::STATIC_NODE_LIST => dom.static_node_list(handle.slot as usize)
                        .map(|nodes| nodes.len()).ok_or(DomError::InvalidStateError),
                    InterfaceBrand::FILTERED_NODE_LIST => dom.collection_length(handle.slot as usize),
                    _ => Err(DomError::InvalidStateError),
                },
                Operation::TokenListSupportedIndexCount => {
                    if let Some((node, attribute)) = dom.token_list_info(handle.slot as usize) {
                        let value = dom.attrs_of(node).and_then(|attrs| attrs.iter()
                            .find(|(name, _)| name == attribute).map(|(_, value)| value.as_str())).unwrap_or("");
                        let mut seen = std::collections::HashSet::new();
                        Ok(value.split(|ch| matches!(ch, '\t' | '\n' | '\u{000C}' | '\r' | ' '))
                            .filter(|token| !token.is_empty() && seen.insert(*token)).count())
                    } else { Err(DomError::InvalidStateError) }
                },
                Operation::NamedNodeMapSupportedIndexCount => {
                    dom.attribute_nodes(NodeIdx(handle.slot as u32)).map(|attrs| attrs.len())
                },
                _ => Err(DomError::NotSupportedError),
            }.map_err(|error| Throw::type_error(error.to_string()))?;
            u32::try_from(length).map_err(|_| Throw::range_error("collection length exceeds u32"))
        });
        let names = hooks.supported_names.map(|name_op| {
            let state = self.state.clone();
            Rc::new(move |handle: HostHandle| {
                let borrowed = state.borrow();
                if handle.arena != borrowed.arena || handle.generation != 1 || handle.brand != brand.0 {
                    return Err(Throw::type_error("Invalid legacy platform object"));
                }
                if brand == InterfaceBrand::NAMED_NODE_MAP {
                    let mut host = borrowed.host.borrow_mut();
                    let dom = &mut host.dom;
                    let element = NodeIdx(handle.slot as u32);
                    let attrs = dom.attribute_nodes(element)
                        .map_err(|error| Throw::type_error(error.to_string()))?;
                    let html = dom.owner_document(element) == Some(NodeIdx(0))
                        && dom.namespace_uri(element) == Some("http://www.w3.org/1999/xhtml");
                    let mut seen = std::collections::HashSet::new();
                    return Ok(attrs.into_iter().filter_map(|attr| dom.attr_snapshot(attr).ok())
                        .map(|attr| attr.name())
                        .filter(|name| !html || name.to_ascii_lowercase() == *name)
                        .map(|name| name.encode_utf16().collect::<Vec<_>>())
                        .filter(|name| seen.insert(name.clone())).collect());
                }
                let mut host = borrowed.host.borrow_mut();
                let dom = &mut host.dom;
                let result = match name_op {
                    Operation::FormCollectionSupportedNames => dom.form_controls(NodeIdx(handle.slot as u32))
                        .map(|items| native_form_names(dom, &items)),
                    Operation::OptionsSupportedNames => dom.select_options(NodeIdx(handle.slot as u32))
                        .map(|items| native_form_names(dom, &items)),
                    _ => dom.collection_names(handle.slot as usize),
                };
                result.map_err(|error| Throw::type_error(error.to_string()))
            }) as Rc<dyn Fn(HostHandle) -> Result<Vec<Vec<u16>>, Throw>>
        });
        let indexed_op = hooks.indexed_getter;
        let state = self.state.clone();
        let indexed_getter = Rc::new(move |js: &mut Interpreter, handle: HostHandle, index: u32| {
            let mut adapter = RuntimeBindingHost { js, state: state.clone() };
            let receiver = adapter.hook_receiver(handle)?;
            invoke(&mut adapter, indexed_op, receiver, &[JsValue::num(f64::from(index))])
        });
        let named_getter = hooks.named_getter.map(|op| {
            let state = self.state.clone();
            Rc::new(move |js: &mut Interpreter, handle: HostHandle, name: &[u16]| {
                let mut adapter = RuntimeBindingHost { js, state: state.clone() };
                let receiver = adapter.hook_receiver(handle)?;
                let arg = adapter.string(name)?;
                invoke(&mut adapter, op, receiver, &[arg])
            }) as Rc<dyn Fn(&mut Interpreter, HostHandle, &[u16]) -> Result<JsValue, Throw>>
        });
        let indexed_setter = hooks.indexed_setter.map(|op| {
            let state = self.state.clone();
            Rc::new(move |js: &mut Interpreter, handle: HostHandle, index: u32, value: JsValue| {
                let mut adapter = RuntimeBindingHost { js, state: state.clone() };
                let receiver = adapter.hook_receiver(handle)?;
                invoke(&mut adapter, op, receiver, &[JsValue::num(f64::from(index)), value])?;
                Ok(())
            }) as Rc<dyn Fn(&mut Interpreter, HostHandle, u32, JsValue) -> Result<(), Throw>>
        });
        self.js.binding_define_legacy_platform_object(brand.0, RuntimeLegacyHooks {
            supported_index_count: count, supported_names: names, indexed_getter, named_getter,
            indexed_setter, named_setter: None, named_deleter: None,
            unenumerable_names: hooks.unenumerable_names, override_builtins: hooks.override_builtins,
        });
        Ok(())
    }
    fn define_iterable(&mut self, interface: Interface, _kind: IterableKind) -> Result<(), Throw> {
        let brand = match interface {
            Interface::NodeList => InterfaceBrand::NODE_LIST,
            Interface::DOMTokenList => InterfaceBrand::DOM_TOKEN_LIST,
            Interface::Location => InterfaceBrand::LOCATION,
            Interface::DOMStringList => InterfaceBrand::DOM_STRING_LIST,
            _ => return Err(Throw::type_error("Unsupported iterable interface")),
        };
        let prototype = self.prototype(interface).ok_or_else(|| Throw::type_error("Missing iterable prototype"))?;
        self.js.binding_define_iterable(brand.0, &prototype)
    }
    fn get_property(&mut self, object: &JsValue, name: &str) -> Result<JsValue, Throw> {
        self.js.binding_get_property(object, name)
    }
    fn set_property(&mut self, object: &JsValue, name: &str, value: JsValue) -> Result<(), Throw> {
        self.js.binding_set_property(object, name, value)
    }
    fn wrap_platform_object(
        &mut self,
        key: PlatformKey,
        brand: InterfaceBrand,
    ) -> Result<JsValue, Throw> {
        let interface = self.interface_for(key, brand)?;
        let prototype = self
            .prototype(interface)
            .ok_or_else(|| Throw::type_error("Interface prototype not installed"))?;
        let value = self.js.binding_wrap_host(
            HostHandle {
                arena: key.arena,
                slot: key.slot,
                generation: key.generation,
                brand: brand.0,
            },
            &prototype,
        )?;
        if brand == InterfaceBrand::LOCATION {
            self.materialize_location_unforgeables(&value)?;
        } else {
            self.materialize_accessors(&value);
        }
        if brand == InterfaceBrand::NODE && key.slot == 0 {
            self.materialize_document_location_unforgeable(&value)?;
        }
        if brand == InterfaceBrand::NODE {
            if let Some(node) = self.node_from_platform_key(key) {
                let is_element = {
                    let host = self.state.borrow().host.clone();
                    matches!(&host.borrow().dom.get(node).kind, NodeKind::Element { .. })
                };
                if is_element { self.preserve_legacy_element_extensions(node, &value)?; }
            }
        }
        Ok(value)
    }
    fn platform_object(&self, value: &JsValue, brand: InterfaceBrand) -> Option<PlatformKey> {
        let handle = self.js.binding_host_handle(value, brand.0)?;
        let key = PlatformKey {
            arena: handle.arena,
            slot: handle.slot,
            generation: handle.generation,
        };
        (key.arena == self.state.borrow().arena && key.generation == 1).then_some(key)
    }
    fn node_platform_key(&self, node: NodeIdx) -> PlatformKey {
        PlatformKey {
            arena: self.state.borrow().arena,
            slot: u64::from(node.0),
            generation: 1,
        }
    }
    fn node_from_platform_key(&self, key: PlatformKey) -> Option<NodeIdx> {
        let state = self.state.borrow();
        (key.arena == state.arena
            && key.generation == 1
            && key.slot < state.host.borrow().dom.node_count() as u64)
            .then(|| NodeIdx(key.slot as u32))
    }
    fn is_callable(&self, value: &JsValue) -> bool {
        self.js.binding_is_callable(value)
    }
    fn call(
        &mut self,
        function: &JsValue,
        this: &JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, Throw> {
        binding_call(self.js, function, this, args)
    }
    fn same_value(&self, left: &JsValue, right: &JsValue) -> bool {
        binding_same_value(self.js, left, right)
    }
    fn retain_value(&mut self, value: JsValue) -> RootId {
        let mut state = self.state.borrow_mut();
        let id = CallbackId(state.roots.len() as u64);
        state.roots.push(Some(value));
        id
    }
    fn root_value(&self, root: RootId) -> Option<JsValue> {
        self.state
            .borrow()
            .roots
            .get(root.0 as usize)
            .and_then(Clone::clone)
    }
    fn release_value(&mut self, root: RootId) {
        if let Some(value) = self.state.borrow_mut().roots.get_mut(root.0 as usize) {
            *value = None;
        }
    }
    fn create_array(&mut self, values: Vec<JsValue>) -> Result<JsValue, Throw> {
        binding_create_array(self.js, values)
    }
    fn to_number(&mut self, value: &JsValue) -> Result<f64, Throw> {
        self.js.binding_to_number(value)
    }
    fn throw_value(&mut self, value: JsValue) -> Throw {
        self.js.binding_throw(value)
    }
    fn install_interface(&mut self, definition: &'static InterfaceDefinition) -> Result<(), Throw> {
        let parent = definition
            .parent
            .and_then(|kind| self.prototype(kind))
            .unwrap_or_else(|| self.js.binding_object_prototype());
        let prototype = self.js.binding_create_object(Some(&parent))?;
        let constructor = match definition.constructor {
            Some(operation) => {
                let length = if matches!(
                    operation,
                    Operation::EventConstructor
                        | Operation::CustomEventConstructor
                        | Operation::MutationObserverConstructor
                        | Operation::StaticRangeConstructor
                ) {
                    1
                } else {
                    0
                };
                self.native(
                    definition.name,
                    length,
                    NativeEntry::Operation(operation),
                    HostCallPolicy::ConstructOnly,
                )
            }
            None => self.native(
                definition.name,
                0,
                NativeEntry::IllegalConstructor,
                HostCallPolicy::ConstructOnly,
            ),
        };
        self.define_data_property(
            &constructor,
            "prototype",
            prototype.clone(),
            false,
            false,
            false,
        )?;
        self.define_data_property(
            &prototype,
            "constructor",
            constructor.clone(),
            true,
            false,
            true,
        )?;
        let mut getters = Vec::<(&'static str, JsValue)>::new();
        for member in definition.members {
            match member.kind {
                MemberKind::Method { length } => {
                    let function = self.native(
                        member.name,
                        length,
                        NativeEntry::Operation(member.operation),
                        HostCallPolicy::CallOnly,
                    );
                    self.define_data_property(
                        &prototype,
                        member.name,
                        function,
                        true,
                        false,
                        true,
                    )?;
                }
                MemberKind::Getter => {
                    let function = self.native(
                        member.name,
                        0,
                        NativeEntry::Operation(member.operation),
                        HostCallPolicy::CallOnly,
                    );
                    getters.push((member.name, function.clone()));
                    self.define_accessor_property(
                        &prototype,
                        member.name,
                        Some(function),
                        None,
                        true,
                        true,
                    )?;
                }
                MemberKind::Setter => {
                    let function = self.native(
                        member.name,
                        0,
                        NativeEntry::Operation(member.operation),
                        HostCallPolicy::CallOnly,
                    );
                    let getter = getters
                        .iter()
                        .find(|(name, _)| *name == member.name)
                        .map(|(_, function)| function.clone());
                    self.define_accessor_property(
                        &prototype,
                        member.name,
                        getter,
                        Some(function),
                        true,
                        true,
                    )?;
                }
            }
        }
        self.state.borrow_mut().interfaces.push((
            definition.interface,
            prototype,
            constructor.clone(),
        ));
        let global = self.js.binding_global_object();
        self.define_data_property(&global, definition.name, constructor, true, false, true)
    }
    fn install_interface_class_string(&mut self, interface: Interface, name: &'static str)
        -> Result<OptionalHookInstall, Throw> {
        let prototype = self.prototype(interface)
            .ok_or_else(|| Throw::type_error("interface prototype missing"))?;
        self.js.binding_define_symbol_data_property(
            &prototype,
            amazing_js::value::WellKnownSymbol::ToStringTag.symbol(),
            JsValue::str(name),
            BindingPropertyFlags { writable: false, enumerable: false, configurable: true },
        )?;
        Ok(OptionalHookInstall::Installed)
    }
    fn install_dom_exception_error_prototype(&mut self) -> Result<OptionalHookInstall, Throw> {
        let global = self.js.binding_global_object();
        let error = self.js.binding_get_property(&global, "Error")?;
        let error_prototype = self.js.binding_get_property(&error, "prototype")?;
        let dom_exception_prototype = self.prototype(Interface::DOMException)
            .ok_or_else(|| Throw::type_error("DOMException prototype missing"))?;
        let (JsValue::Object(dom_exception), JsValue::Object(error)) =
            (dom_exception_prototype, error_prototype) else {
                return Err(Throw::type_error("Error prototype missing"));
            };
        dom_exception.borrow_mut().proto = Some(error);
        Ok(OptionalHookInstall::Installed)
    }
    fn install_custom_elements_global(&mut self) -> Result<OptionalHookInstall, Throw> {
        let registry = registry_for_document(self, NodeIdx(0))?;
        let global = self.js.binding_global_object();
        self.define_data_property(&global, "customElements", registry, true, false, true)?;
        Ok(OptionalHookInstall::Installed)
    }
    fn is_constructor(&mut self, value: &JsValue) -> Result<bool, Throw> {
        let JsValue::Object(object) = value else { return Ok(false) };
        let object = object.borrow();
        Ok(object.func.as_ref().is_some_and(|function| {
            !function.is_arrow && !function.is_async && !function.is_generator
                && !matches!(function.body, FunctionBody::HostNative(_, JsCallPolicy::CallOnly, _))
        }))
    }
    fn require_object(&mut self, value: &JsValue) -> Result<(), Throw> {
        if value.is_object() { Ok(()) } else { Err(Throw::type_error("Expected an object")) }
    }
    fn sequence_dom_strings(&mut self, value: &JsValue) -> Result<Vec<Vec<u16>>, Throw> {
        self.require_object(value)?;
        let length = self.js.binding_get_property(value, "length")?;
        let length = self.js.binding_to_number(&length)?;
        if !length.is_finite() || length < 0.0 || length > u32::MAX as f64 {
            return Err(Throw::type_error("Invalid sequence length"));
        }
        let mut strings = Vec::new();
        for index in 0..length.trunc() as u32 {
            let item = self.js.binding_get_property(value, &index.to_string())?;
            strings.push(self.to_dom_string(&item)?);
        }
        Ok(strings)
    }
    fn construct_custom_element(&mut self, constructor: &JsValue,
        _definition: usize, _element: NodeIdx) -> Result<JsValue, Throw> {
        // JSRT-7 must make derived `super()` return this element's canonical
        // wrapper. Until then the native finish check rejects ordinary objects.
        self.js.binding_construct(constructor, Vec::new())
    }
    fn new_promise(&mut self) -> Result<JsValue, Throw> {
        let global = self.js.binding_global_object();
        let constructor = self.js.binding_get_property(&global, "Promise")?;
        let executor = self.native("customElements executor", 2,
            NativeEntry::CapturePromiseResolvers, HostCallPolicy::CallOnly);
        let promise = self.js.binding_construct(&constructor, vec![executor])?;
        let JsValue::Object(object) = &promise else {
            return Err(Throw::type_error("Promise constructor returned a non-object"));
        };
        let resolvers = self.state.borrow_mut().captured_resolvers.take()
            .ok_or_else(|| Throw::type_error("Promise constructor did not call executor"))?;
        self.state.borrow_mut().promise_resolvers.insert(Rc::as_ptr(&object.0) as usize, resolvers);
        Ok(promise)
    }
    fn resolve_promise(&mut self, promise: &JsValue, value: JsValue) -> Result<(), Throw> {
        let JsValue::Object(object) = promise else { return Err(Throw::type_error("Expected a Promise")) };
        let pair = self.state.borrow_mut().promise_resolvers.remove(&(Rc::as_ptr(&object.0) as usize))
            .ok_or_else(|| Throw::type_error("Unknown customElements Promise"))?;
        binding_call(self.js, &pair.0, &JsValue::Undefined, &[value])?;
        Ok(())
    }
    fn reject_promise(&mut self, promise: &JsValue, reason: JsValue) -> Result<(), Throw> {
        let JsValue::Object(object) = promise else { return Err(Throw::type_error("Expected a Promise")) };
        let pair = self.state.borrow_mut().promise_resolvers.remove(&(Rc::as_ptr(&object.0) as usize))
            .ok_or_else(|| Throw::type_error("Unknown customElements Promise"))?;
        binding_call(self.js, &pair.1, &JsValue::Undefined, &[reason])?;
        Ok(())
    }
    fn dom_exception_value(&mut self, error: DomError) -> Result<JsValue, Throw> {
        Ok(throw_dom_error(self, error).0)
    }
    fn report_custom_element_exception(&mut self, error: Throw) -> Result<(), Throw> {
        eprintln!("Uncaught custom element reaction: {}", error.describe());
        Ok(())
    }
    fn form_pattern_matches(&mut self, pattern: &[u16], value: &[u16]) -> Result<Option<bool>, Throw> {
        use amazing_regex::{Error as RegexError, Regex};

        // HTML first compiles the source on its own. Anchoring must not turn
        // an invalid pattern into a valid one.
        match Regex::new_utf16(pattern, "v") {
            Ok(_) => {}
            Err(RegexError::Syntax(_)) => return Ok(None),
            Err(error) => return Err(Throw::range_error(error.to_string())),
        }
        let mut anchored = "^(?:".encode_utf16().collect::<Vec<_>>();
        anchored.extend_from_slice(pattern);
        anchored.extend(")$".encode_utf16());
        let regex = match Regex::new_utf16(&anchored, "v") {
            Ok(regex) => regex,
            Err(RegexError::Syntax(_)) => return Ok(None),
            Err(error) => return Err(Throw::range_error(error.to_string())),
        };
        regex.exec_utf16(value, 0)
            .map(|result| Some(result.is_some()))
            .map_err(|error| Throw::range_error(error.to_string()))
    }
    fn form_date_from_epoch_ms(&mut self, milliseconds: f64) -> Result<JsValue, Throw> {
        let global = self.js.binding_global_object();
        let date = self.js.binding_get_property(&global, "Date")?;
        self.js.binding_construct(&date, vec![JsValue::num(milliseconds)])
    }
    fn form_date_to_epoch_ms(&mut self, value: &JsValue) -> Result<Option<f64>, Throw> {
        let JsValue::Object(object) = value else { return Err(Throw::type_error("Expected a Date")) };
        let global = self.js.binding_global_object();
        let date = self.js.binding_get_property(&global, "Date")?;
        let JsValue::Object(date_prototype) = self.js.binding_get_property(&date, "prototype")? else {
            return Err(Throw::type_error("Date prototype missing"));
        };
        let mut prototype = object.borrow().proto.clone();
        let mut is_date = false;
        while let Some(current) = prototype {
            if Rc::ptr_eq(&current.0, &date_prototype.0) { is_date = true; break; }
            prototype = current.borrow().proto.clone();
        }
        if !is_date { return Err(Throw::type_error("Expected a Date")); }
        match object.borrow().get_own("[[DateValue]]") {
            Some(JsValue::Num(milliseconds)) if milliseconds.is_finite() => Ok(Some(milliseconds)),
            Some(JsValue::Num(_)) => Ok(None),
            _ => Err(Throw::type_error("Invalid Date value")),
        }
    }

    fn document_location(&mut self, document: NodeIdx) -> Result<Option<JsValue>, Throw> {
        if document != NodeIdx(0) { return Ok(None); }
        let arena = self.state.borrow().arena;
        self.wrap_platform_object(PlatformKey { arena, slot: 0, generation: 1 },
            InterfaceBrand::LOCATION).map(Some)
    }
    fn location_href(&mut self) -> Result<String, Throw> {
        Ok(self.js.binding_location_href().to_owned())
    }
    fn location_navigate(&mut self, url: &str, replace: bool) -> Result<(), Throw> {
        self.js.binding_location_navigate(url, replace);
        self.state.borrow_mut().document_url = Some(url.to_owned());
        Ok(())
    }
    fn location_reload(&mut self) -> Result<(), Throw> {
        self.js.binding_location_reload();
        Ok(())
    }
    fn location_ancestor_origins(&mut self) -> Result<JsValue, Throw> {
        let arena = self.state.borrow().arena;
        self.wrap_platform_object(PlatformKey { arena, slot: 0, generation: 1 },
            InterfaceBrand::DOM_STRING_LIST)
    }
    fn url_new(&mut self, url: UrlWithSearchParams) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::Url(url, None), InterfaceBrand::URL)
    }
    fn url_record(&mut self, receiver: &JsValue) -> Result<UrlWithSearchParams, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::URL)?;
        match &self.state.borrow().web_objects[slot] {
            WebObject::Url(url, _) => Ok(url.clone()),
            _ => Err(Throw::type_error("Expected URL")),
        }
    }
    fn url_search_params(&mut self, receiver: &JsValue) -> Result<JsValue, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::URL)?;
        let existing = match &self.state.borrow().web_objects[slot] {
            WebObject::Url(_, params) => *params,
            _ => return Err(Throw::type_error("Expected URL")),
        };
        let params_slot = if let Some(existing) = existing { existing } else {
            let url = self.url_record(receiver)?;
            let mut state = self.state.borrow_mut();
            let next = state.web_objects.len() as u64;
            state.web_objects.push(WebObject::Params(url.search_params()));
            if let WebObject::Url(_, params) = &mut state.web_objects[slot] { *params = Some(next); }
            next
        };
        let arena = self.state.borrow().arena;
        self.wrap_platform_object(PlatformKey { arena,
            slot: params_slot, generation: 1 }, InterfaceBrand::URL_SEARCH_PARAMS)
    }
    fn url_params_new(&mut self, params: UrlSearchParams) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::Params(params), InterfaceBrand::URL_SEARCH_PARAMS)
    }
    fn url_params_record(&mut self, receiver: &JsValue) -> Result<UrlSearchParams, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::URL_SEARCH_PARAMS)?;
        match &self.state.borrow().web_objects[slot] {
            WebObject::Params(params) => Ok(params.clone()),
            _ => Err(Throw::type_error("Expected URLSearchParams")),
        }
    }
    fn url_params_store(&mut self, receiver: &JsValue, params: UrlSearchParams) -> Result<(), Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::URL_SEARCH_PARAMS)?;
        self.state.borrow_mut().web_objects[slot] = WebObject::Params(params);
        Ok(())
    }
    fn url_params_pairs(&mut self, value: &JsValue) -> Result<Option<Vec<(String, String)>>, Throw> {
        let JsValue::Object(object) = value else { return Ok(None); };
        if self.platform_object(value, InterfaceBrand::URL_SEARCH_PARAMS).is_some() {
            return Ok(Some(self.url_params_record(value)?.pairs()));
        }
        let mut pairs = Vec::new();
        if matches!(object.borrow().exotic, Exotic::Array) {
            let length = self.js.binding_get_property(value, "length")?;
            let JsValue::Num(length) = length else { return Err(Throw::type_error("Invalid sequence length")); };
            for index in 0..length as usize {
                let pair = self.js.binding_get_property(value, &index.to_string())?;
                let JsValue::Object(inner) = &pair else { return Err(Throw::type_error("Expected pair sequence")); };
                if !matches!(inner.borrow().exotic, Exotic::Array) {
                    return Err(Throw::type_error("Expected pair sequence"));
                }
                if !matches!(self.js.binding_get_property(&pair, "length")?, JsValue::Num(2.0)) {
                    return Err(Throw::type_error("Each pair must have two items"));
                }
                let name = self.js.binding_get_property(&pair, "0")?;
                let value = self.js.binding_get_property(&pair, "1")?;
                pairs.push((String::from_utf16_lossy(&self.to_dom_string(&name)?),
                    String::from_utf16_lossy(&self.to_dom_string(&value)?)));
            }
        } else {
            let names = object.borrow().own_keys_enumerable();
            for name in names {
                let key = name.to_string();
                let value = self.js.binding_get_property(value, &key)?;
                pairs.push((String::from_utf16_lossy(name.units()),
                    String::from_utf16_lossy(&self.to_dom_string(&value)?)));
            }
        }
        Ok(Some(pairs))
    }
    fn blob_parts(&mut self, value: &JsValue) -> Result<Vec<BlobPart>, Throw> {
        let JsValue::Object(object) = value else { return Err(Throw::type_error("Blob parts must be a sequence")); };
        if !matches!(object.borrow().exotic, Exotic::Array) {
            return Err(Throw::type_error("Blob parts must be a sequence"));
        }
        let JsValue::Num(length) = self.js.binding_get_property(value, "length")? else {
            return Err(Throw::type_error("Invalid Blob parts length"));
        };
        let mut parts = Vec::new();
        for index in 0..length as usize {
            let part = self.js.binding_get_property(value, &index.to_string())?;
            if self.platform_object(&part, InterfaceBrand::FILE).is_some() {
                parts.push(BlobPart::Blob(self.file_record(&part)?.blob));
            } else if self.platform_object(&part, InterfaceBrand::BLOB).is_some() {
                parts.push(BlobPart::Blob(self.blob_record(&part)?));
            } else {
                parts.push(BlobPart::Text(String::from_utf16_lossy(&self.to_dom_string(&part)?)));
            }
        }
        Ok(parts)
    }
    fn blob_new(&mut self, blob: Blob) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::Blob(blob), InterfaceBrand::BLOB)
    }
    fn blob_record(&mut self, receiver: &JsValue) -> Result<Blob, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::BLOB)?;
        match &self.state.borrow().web_objects[slot] {
            WebObject::Blob(blob) => Ok(blob.clone()),
            _ => Err(Throw::type_error("Expected Blob")),
        }
    }
    fn file_new(&mut self, file: File) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::File(file), InterfaceBrand::FILE)
    }
    fn file_record(&mut self, receiver: &JsValue) -> Result<File, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::FILE)?;
        match &self.state.borrow().web_objects[slot] {
            WebObject::File(file) => Ok(file.clone()),
            _ => Err(Throw::type_error("Expected File")),
        }
    }
    fn form_data_new(&mut self, data: FormData) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::FormData(data), InterfaceBrand::FORM_DATA)
    }
    fn form_data_record(&mut self, receiver: &JsValue) -> Result<FormData, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::FORM_DATA)?;
        match &self.state.borrow().web_objects[slot] {
            WebObject::FormData(data) => Ok(data.clone()),
            _ => Err(Throw::type_error("Expected FormData")),
        }
    }
    fn form_data_store(&mut self, receiver: &JsValue, data: FormData) -> Result<(), Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::FORM_DATA)?;
        self.state.borrow_mut().web_objects[slot] = WebObject::FormData(data);
        Ok(())
    }
    fn blob_promise_bytes(&mut self, bytes: Vec<u8>, as_text: bool, _as_array_buffer: bool) -> Result<JsValue, Throw> {
        if !as_text { return self.with_dom::<JsValue>(|_| Err(DomError::NotSupportedError)); }
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let value = JsValue::str(String::from_utf8_lossy(bytes).as_ref());
        let promise = self.js.binding_get_property(&self.js.binding_global_object(), "Promise")?;
        let resolve = self.js.binding_get_property(&promise, "resolve")?;
        binding_call(self.js, &resolve, &promise, &[value])
    }
    fn font_face_set(&mut self, document: NodeIdx) -> Result<JsValue, Throw> {
        let existing = self.state.borrow().font_sets.get(&document).copied();
        let slot = if let Some(slot) = existing { slot } else {
            let mut state = self.state.borrow_mut();
            let slot = state.web_objects.len() as u64;
            state.web_objects.push(WebObject::FontFaceSet(document, None));
            state.font_sets.insert(document, slot);
            slot
        };
        let arena = self.state.borrow().arena;
        self.wrap_platform_object(PlatformKey { arena,
            slot, generation: 1 }, InterfaceBrand::FONT_FACE_SET)
    }
    fn font_face_set_ready(&mut self, receiver: &JsValue) -> Result<JsValue, Throw> {
        let slot = self.web_slot(receiver, InterfaceBrand::FONT_FACE_SET)?;
        if let WebObject::FontFaceSet(_, Some(promise)) = &self.state.borrow().web_objects[slot] {
            return Ok(promise.clone());
        }
        let promise_class = self.js.binding_get_property(&self.js.binding_global_object(), "Promise")?;
        let resolve = self.js.binding_get_property(&promise_class, "resolve")?;
        let promise = binding_call(self.js, &resolve, &promise_class, &[receiver.clone()])?;
        if let WebObject::FontFaceSet(_, ready) = &mut self.state.borrow_mut().web_objects[slot] {
            *ready = Some(promise.clone());
        }
        Ok(promise)
    }
    fn install_node_constant(&mut self, name: &'static str, value: u16) -> Result<(), Throw> {
        let prototype = self
            .prototype(Interface::Node)
            .ok_or_else(|| Throw::type_error("Node missing"))?;
        let constructor = self
            .constructor(Interface::Node)
            .ok_or_else(|| Throw::type_error("Node missing"))?;
        for owner in [&prototype, &constructor] {
            self.define_data_property(
                owner,
                name,
                JsValue::num(f64::from(value)),
                false,
                true,
                false,
            )?;
        }
        Ok(())
    }
    fn install_interface_constant(
        &mut self,
        interface: Interface,
        name: &'static str,
        value: u32,
    ) -> Result<(), Throw> {
        let prototype = self
            .prototype(interface)
            .ok_or_else(|| Throw::type_error("Interface missing"))?;
        let constructor = self
            .constructor(interface)
            .ok_or_else(|| Throw::type_error("Interface missing"))?;
        for owner in [&prototype, &constructor] {
            self.define_data_property(owner, name, JsValue::num(value as f64), false, true, false)?;
        }
        Ok(())
    }
    fn install_static_method(&mut self, interface: Interface, name: &'static str,
        operation: Operation, length: u8) -> Result<(), Throw> {
        let constructor = self.constructor(interface).ok_or_else(|| Throw::type_error("Missing interface constructor"))?;
        let method = self.native(name, length, NativeEntry::Operation(operation), HostCallPolicy::CallOnly);
        self.define_data_property(&constructor, name, method, true, false, true)
    }
    fn current_document(&self) -> NodeIdx {
        NodeIdx(0)
    }
    fn with_dom<R>(
        &mut self,
        callback: impl FnOnce(&mut Dom) -> Result<R, DomError>,
    ) -> Result<R, Throw> {
        let host = self.state.borrow().host.clone();
        let result = callback(&mut host.borrow_mut().dom);
        // Preserve the order in which script queues JS jobs and native DOM
        // mutations. Collecting only at the end of the script would move the
        // mutation checkpoint behind jobs queued after this operation.
        let pending = host.borrow().pending_native_microtasks();
        for request in pending {
            self.js.enqueue_native_microtask(request.global, request.kind)?;
            host.borrow_mut().acknowledge_native_microtask(request);
        }
        result.map_err(|error| throw_dom_error(self, error))
    }
    fn is_html_document(&self, document: NodeIdx) -> bool {
        if document != NodeIdx(0) {
            let host = self.state.borrow().host.clone();
            return host.borrow().dom.document_metadata(document).ok().flatten()
                .is_some_and(|metadata| metadata.content_type == "text/html");
        }
        document == NodeIdx(0)
    }
    fn undefined(&self) -> JsValue {
        JsValue::Undefined
    }
    fn null(&self) -> JsValue {
        JsValue::Null
    }
    fn boolean(&self, value: bool) -> JsValue {
        JsValue::Bool(value)
    }
    fn number(&self, value: f64) -> JsValue {
        JsValue::Num(value)
    }
    fn string(&self, value: &[u16]) -> Result<JsValue, Throw> {
        Ok(self.js.binding_string(value))
    }
    fn is_undefined(&self, value: &JsValue) -> bool {
        value.is_undefined()
    }
    fn is_nullish(&self, value: &JsValue) -> bool {
        self.js.binding_is_nullish(value)
    }
    fn to_boolean(&self, value: &JsValue) -> bool {
        self.js.binding_to_boolean(value)
    }
    fn string_units(&self, value: &JsValue) -> Option<Vec<u16>> {
        self.js.binding_string_units(value)
    }
    fn to_dom_string(&mut self, value: &JsValue) -> Result<Vec<u16>, Throw> {
        self.js.binding_to_dom_string(value)
    }
    fn type_error(&mut self, message: &'static str) -> Throw {
        Throw::type_error(message)
    }
}

impl FragmentParser for RuntimeBindingHost<'_> {
    fn parse_fragment(&mut self, context: FragmentContext, markup: &[u16]) -> Result<Dom, DomError> {
        if context.namespace_uri.as_deref() != Some("http://www.w3.org/1999/xhtml") {
            return Err(DomError::NotSupportedError);
        }
        let markup = String::from_utf16(markup).map_err(|_| DomError::TypeError)?;
        // A paragraph uses the ordinary in-body fragment insertion mode.
        // The parser's `div` context has the same tokenizer and insertion
        // state for this context; special contexts remain explicit above.
        let parser_context = if context.local_name == "p" { "div" } else { &context.local_name };
        amazing_html::parse_fragment(parser_context, &markup)
            .map_err(|_| DomError::NotSupportedError)
    }
}

impl SelectorEngine for RuntimeBindingHost<'_> {
    type SelectorHandle = Vec<amazing_css::selectors::Selector>;
    fn parse_selector(&mut self, selector: &[u16]) -> Result<Self::SelectorHandle, DomError> {
        let selector = String::from_utf16(selector).map_err(|_| DomError::TypeError)?;
        let mut selectors = Vec::new();
        let mut start = 0;
        let mut depth = 0u32;
        let mut quote = None;
        let mut escaped = false;
        for (index, ch) in selector.char_indices() {
            if escaped { escaped = false; continue; }
            if ch == '\\' { escaped = true; continue; }
            if let Some(active) = quote {
                if ch == active { quote = None; }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' | '[' => depth += 1,
                ')' | ']' => depth = depth.checked_sub(1).ok_or(DomError::SyntaxError)?,
                ',' if depth == 0 => {
                    selectors.push(amazing_css::selectors::parse_selector(&selector[start..index])
                        .ok_or(DomError::SyntaxError)?);
                    start = index + 1;
                }
                _ => {}
            }
        }
        if quote.is_some() || depth != 0 { return Err(DomError::SyntaxError); }
        selectors.push(amazing_css::selectors::parse_selector(&selector[start..])
            .ok_or(DomError::SyntaxError)?);
        Ok(selectors)
    }
    fn matches_selector(&mut self, element: NodeIdx, selector: &Self::SelectorHandle) -> Result<bool, DomError> {
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        if element.0 as usize >= host.dom.node_count() { return Err(DomError::InvalidNode); }
        Ok(selector.iter().any(|part| amazing_css::selectors::matches(&host.dom, element, part)))
    }
}

impl AbortHost for RuntimeBindingHost<'_> {
    fn schedule_abort_timeout(&mut self, signal: AbortSignalId, milliseconds: u64) -> Result<(), Throw> {
        self.state.borrow_mut().abort_timeouts.push((signal, milliseconds));
        Ok(())
    }
    fn run_abort_algorithm(&mut self, _algorithm: u64, _reason: &JsValue) -> Result<(), Throw> {
        Err(throw_dom_error(self, DomError::NotSupportedError))
    }
}

impl DocumentHost for RuntimeBindingHost<'_> {
    fn document_field(&mut self, document: NodeIdx, field: DocumentField) -> Result<Vec<u16>, Throw> {
        if document != NodeIdx(0) {
            let host = self.state.borrow().host.clone();
            let metadata = { host.borrow().dom.document_metadata(document).ok().flatten().cloned() };
            let Some(metadata) = metadata else {
                return Err(throw_dom_error(self, DomError::NotSupportedError));
            };
            let value = match field {
                DocumentField::URL | DocumentField::DocumentURI => metadata.url,
                DocumentField::CharacterSet => metadata.character_set,
                DocumentField::ContentType => metadata.content_type,
                DocumentField::CompatMode => "CSS1Compat".into(),
                DocumentField::Referrer => String::new(),
            };
            return Ok(value.encode_utf16().collect());
        }
        let value = match field {
            DocumentField::URL | DocumentField::DocumentURI => self.state.borrow().document_url.clone(),
            DocumentField::CharacterSet => self.state.borrow().document_character_set.clone(),
            DocumentField::ContentType => self.state.borrow().document_content_type.clone(),
            DocumentField::Referrer => self.state.borrow().document_referrer.clone(),
            DocumentField::CompatMode => None,
        };
        let Some(value) = value else {
            return Err(throw_dom_error(self, DomError::NotSupportedError));
        };
        Ok(value.encode_utf16().collect())
    }
    fn active_element(&mut self, _document: NodeIdx) -> Result<Option<NodeIdx>, Throw> {
        if _document != NodeIdx(0) { return Ok(None); }
        Err(throw_dom_error(self, DomError::NotSupportedError))
    }
    fn has_focus(&mut self, _document: NodeIdx) -> Result<bool, Throw> {
        if _document != NodeIdx(0) { return Ok(false); }
        Err(throw_dom_error(self, DomError::NotSupportedError))
    }
}

impl UrlReflection for RuntimeBindingHost<'_> {
    fn resolve_url(&mut self, input: &[u16]) -> Result<Vec<u16>, Throw> {
        let input = String::from_utf16(input)
            .map_err(|_| Throw::type_error("lone surrogate not yet representable"))?;
        let base_url = self.state.borrow().document_url.clone();
        let base = base_url
            .ok_or_else(|| throw_dom_error(self, DomError::NotSupportedError))?;
        let base = amazing_net::url::ParsedUrl::parse(&base)
            .map_err(|_| throw_dom_error(self, DomError::NotSupportedError))?;
        let resolved = base.resolve(&input)
            .map_err(|_| Throw::type_error("Invalid URL"))?;
        Ok(resolved.serialize().encode_utf16().collect())
    }
}

// Keep the three JSRT-3 facade calls in one place. Native callbacks must use
// the supplied receiver, and arrays must carry this interpreter's prototype.
fn binding_call(
    js: &mut Interpreter,
    function: &JsValue,
    this: &JsValue,
    args: &[JsValue],
) -> Result<JsValue, Throw> {
    js.binding_call(function, this.clone(), args.to_vec())
}
fn binding_create_array(js: &mut Interpreter, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    js.binding_create_array(values)
}
fn binding_same_value(js: &Interpreter, left: &JsValue, right: &JsValue) -> bool {
    js.binding_same_value(left, right)
}

fn native_form_names(dom: &Dom, items: &[NodeIdx]) -> Vec<Vec<u16>> {
    let mut names = Vec::new();
    for &item in items {
        for value in [dom.attribute_utf16(item, "id"), dom.attribute_utf16(item, "name")] {
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                if !names.contains(&value) { names.push(value); }
            }
        }
    }
    names
}

const STORAGE_BRAND_BASE: u64 = 100;
const STORAGE_QUOTA: u64 = 50 * 1024 * 1024;

impl RuntimeBindingHost<'_> {
    fn install_storage_interfaces(&mut self) -> Result<(), Throw> {
        for definition in storage_interfaces::INTERFACES {
            let parent = definition.parent
                .and_then(|kind| self.state.borrow().storage_classes.get(&kind).map(|pair|pair.0.clone()))
                .unwrap_or_else(||self.js.binding_object_prototype());
            let prototype=self.js.binding_create_object(Some(&parent))?;
            let entry=if definition.kind==SI::IDBVersionChangeEvent{
                NativeEntry::Storage(SI::IDBVersionChangeEvent,"constructor",SM::StaticMethod)
            }else{NativeEntry::IllegalConstructor};
            let constructor=self.native(definition.name,if definition.kind==SI::IDBVersionChangeEvent{1}else{0},entry,HostCallPolicy::ConstructOnly);
            self.define_data_property(&constructor,"prototype",prototype.clone(),false,false,false)?;
            self.define_data_property(&prototype,"constructor",constructor.clone(),true,false,true)?;
            let mut getters:HashMap<&'static str,JsValue>=HashMap::new();
            for member in definition.members {
                let function=self.native(member.name,member.length,
                    NativeEntry::Storage(definition.kind,member.name,member.kind),HostCallPolicy::CallOnly);
                match member.kind {
                    SM::Method=>self.define_data_property(&prototype,member.name,function,true,false,true)?,
                    SM::StaticMethod=>self.define_data_property(&constructor,member.name,function,true,false,true)?,
                    SM::Getter=>{getters.insert(member.name,function.clone());
                        self.define_accessor_property(&prototype,member.name,Some(function),None,true,true)?;},
                    SM::Setter=>self.define_accessor_property(&prototype,member.name,getters.get(member.name).cloned(),Some(function),true,true)?,
                }
            }
            self.state.borrow_mut().storage_classes.insert(definition.kind,(prototype,constructor.clone()));
            let global=self.js.binding_global_object();
            self.define_data_property(&global,definition.name,constructor,true,false,true)?;
        }
        let global=self.js.binding_global_object();
        for name in ["localStorage","sessionStorage"] {
            let getter=self.native(name,0,NativeEntry::Storage(SI::Storage,name,SM::Getter),HostCallPolicy::CallOnly);
            self.define_accessor_property(&global,name,Some(getter),None,true,true)?;
        }
        for (name,interface,object) in [
            ("indexedDB",SI::IDBFactory,StorageObject::Factory),
            ("caches",SI::CacheStorage,StorageObject::CacheStorage),
        ] {
            let wrapper=self.storage_allocate(interface,object)?;
            self.define_data_property(&global,name,wrapper,true,true,true)?;
        }
        self.install_storage_legacy();
        Ok(())
    }
    fn storage_profile_path() -> std::path::PathBuf {
        if let Some(path)=std::env::var_os("AMAZING_STORAGE_DIR") {return path.into()}
        if let Some(path)=std::env::var_os("LOCALAPPDATA") {return std::path::PathBuf::from(path).join("Amazing").join("Storage")}
        if let Some(path)=std::env::var_os("XDG_DATA_HOME") {return std::path::PathBuf::from(path).join("amazing").join("storage")}
        std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir)
            .join(".local").join("share").join("amazing").join("storage")
    }
    fn storage_ensure(&mut self) -> Result<(), Throw> {
        if self.state.borrow().storage.is_some(){return Ok(())}
        let path=Self::storage_profile_path();
        let web=SHARED_WEB_STORAGE.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let value=Rc::new(RefCell::new(PersistentWebStorage::open(DiskStore::open(path.join("web"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("storage open: {e:?}")))?,10*1024*1024)
                .map_err(|e|Throw::type_error(format!("Web Storage open: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let idb=SHARED_INDEXED_DB.with(|shared|->Result<_,Throw>{
            if let Some(value)=shared.borrow().get(&path).and_then(Weak::upgrade){return Ok(value)}
            let value=Rc::new(RefCell::new(IndexedDb::with_disk(DiskStore::open(path.join("idb"),STORAGE_QUOTA)
                .map_err(|e|Throw::type_error(format!("IndexedDB open: {e:?}")))?)
                .map_err(|e|Throw::type_error(format!("IndexedDB decode: {e:?}")))?));
            shared.borrow_mut().insert(path.clone(),Rc::downgrade(&value));Ok(value)
        })?;
        let cache=PersistentCacheStorage::open(DiskStore::open(path.join("cache"),STORAGE_QUOTA)
            .map_err(|e|Throw::type_error(format!("Cache open: {e:?}")))?)
            .map_err(|e|Throw::type_error(format!("Cache decode: {e:?}")))?;
        self.state.borrow_mut().storage=Some(StorageRuntime {web,idb,cache,objects:Vec::new(),wrappers:HashMap::new(),pending:VecDeque::new(),blocked_opens:VecDeque::new(),blocked_deletes:VecDeque::new(),tick_scheduled:false});
        Ok(())
    }
    fn storage_key(&self) -> Result<StorageKey, Throw> {
        use amazing_fetch::{HttpUrl,Site,psl::PublicSuffixList};
        let url=self.state.borrow().document_url.clone().ok_or_else(||Throw::type_error("Storage requires a document URL"))?;
        let parsed=HttpUrl::parse(&url).map_err(|_|Throw::type_error("Storage requires an HTTP(S) origin"))?;
        let origin=parsed.origin();
        let site=match origin.site(&PublicSuffixList) {
            Site::Tuple {scheme,domain}=>format!("{scheme}://{domain}"),
            Site::Opaque(_)=>return Err(Throw::type_error("Storage is unavailable for opaque origins")),
        };
        Ok(StorageKey::new(origin.serialize(),site))
    }
    fn storage_bucket(&self) -> Result<BucketKey, Throw> {Ok(BucketKey::new(self.storage_key()?,"default"))}
    fn storage_allocate(&mut self,kind:SI,object:StorageObject) -> Result<JsValue, Throw> {
        self.storage_ensure()?;
        let (slot,prototype)={
            let mut state=self.state.borrow_mut();
            let storage=state.storage.as_mut().expect("storage initialized");
            let slot=storage.objects.len();storage.objects.push(object);
            let prototype=state.storage_classes.get(&kind).ok_or_else(||Throw::type_error("Storage class not installed"))?.0.clone();
            (slot,prototype)
        };
        let arena=self.state.borrow().arena;
        let value=self.js.binding_wrap_host(HostHandle {arena,slot:slot as u64,generation:1,brand:STORAGE_BRAND_BASE+kind as u64},&prototype)?;
        self.state.borrow_mut().storage.as_mut().expect("storage initialized").wrappers.insert(slot,value.clone());
        self.materialize_accessors(&value);
        Ok(value)
    }
    fn storage_slot(&self,value:&JsValue,kind:SI)->Result<usize,Throw>{
        let handle=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+kind as u64)
            .or_else(||(kind==SI::IDBRequest).then(||self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBOpenDBRequest as u64)).flatten())
            .or_else(||(kind==SI::IDBCursor).then(||self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBCursorWithValue as u64)).flatten())
            .ok_or_else(||Throw::type_error("Incompatible storage receiver"))?;
        if handle.arena!=self.state.borrow().arena||handle.generation!=1{return Err(Throw::type_error("Stale storage receiver"))}
        let slot=handle.slot as usize;
        if self.state.borrow().storage.as_ref().is_none_or(|storage|slot>=storage.objects.len()){
            return Err(Throw::type_error("Invalid storage slot"));
        }
        Ok(slot)
    }
    fn storage_wrapper(&self,slot:usize)->Result<JsValue,Throw>{
        self.state.borrow().storage.as_ref().and_then(|s|s.wrappers.get(&slot)).cloned()
            .ok_or_else(||Throw::type_error("Storage wrapper missing"))
    }
    fn storage_dom_error(&mut self,name:&str,message:&str)->Throw {
        let result=(||{
            let global=self.js.binding_global_object();
            let constructor=self.js.binding_get_property(&global,"DOMException")?;
            let value=self.js.binding_construct(&constructor,vec![JsValue::str(message),JsValue::str(name)])?;
            Ok::<Throw,Throw>(self.js.binding_throw(value))
        })();
        result.unwrap_or_else(|_|Throw::type_error(format!("{name}: {message}")))
    }
    fn storage_string(&mut self,value:&JsValue)->Result<String,Throw>{
        Ok(String::from_utf16_lossy(&self.js.binding_to_dom_string(value)?))
    }
    fn storage_arg<'a>(&self,args:&'a[JsValue],index:usize)->Result<&'a JsValue,Throw>{
        args.get(index).ok_or_else(||Throw::type_error("Missing required argument"))
    }
    fn storage_schedule(&mut self,job:StorageJob)->Result<(),Throw>{
        let needs_tick={
            let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().expect("storage initialized");
            storage.pending.push_back(job);
            if storage.tick_scheduled {false}else{storage.tick_scheduled=true;true}
        };
        if needs_tick {
            let callback=self.native("storage task",0,NativeEntry::StorageTick,HostCallPolicy::CallOnly);
            self.js.schedule_timer(callback,0,Vec::new())?;
        }
        Ok(())
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_key_for_url(url:&str)->Result<StorageKey,Throw>{
        use amazing_fetch::{HttpUrl,Site,psl::PublicSuffixList};
        let parsed=HttpUrl::parse(url).map_err(|_|Throw::type_error("Storage requires an HTTP(S) origin"))?;
        let origin=parsed.origin();
        let site=match origin.site(&PublicSuffixList) {
            Site::Tuple {scheme,domain}=>format!("{scheme}://{domain}"),
            Site::Opaque(_)=>return Err(Throw::type_error("Storage is unavailable for opaque origins")),
        };
        Ok(StorageKey::new(origin.serialize(),site))
    }
    fn storage_area_session(&self,slot:usize)->Result<bool,Throw>{
        match self.state.borrow().storage.as_ref().and_then(|s|s.objects.get(slot)) {
            Some(StorageObject::Area {session})=>Ok(*session),
            _=>Err(Throw::type_error("Expected Storage")),
        }
    }
    fn storage_area_pairs(&mut self,slot:usize)->Result<Vec<(Vec<u16>,Vec<u16>)>,Throw>{
        let session=self.storage_area_session(slot)?;
        let key=self.storage_key()?;let bucket=BucketKey::new(key.clone(),"default");
        let id=self.state.borrow().arena;
        let web=self.state.borrow().storage.as_ref().expect("storage initialized").web.clone();
        let mut web=web.borrow_mut();
        let area=if session {Some(&*web.session(id,key))} else {web.local(&bucket)};
        let mut pairs=Vec::new();
        if let Some(area)=area {for index in 0..area.len(){let name=area.key(index).unwrap().to_vec();let value=area.get(&name).unwrap().to_vec();pairs.push((name,value));}}
        Ok(pairs)
    }
    fn storage_area_get(&mut self,slot:usize,name:&[u16])->Result<Option<Vec<u16>>,Throw>{
        Ok(self.storage_area_pairs(slot)?.into_iter().find(|(key,_)|key==name).map(|(_,value)|value))
    }
    fn storage_area_change(&mut self,slot:usize,name:Option<Vec<u16>>,value:Option<Vec<u16>>)->Result<(),Throw>{
        let session=self.storage_area_session(slot)?;
        let key=self.storage_key()?;let bucket=BucketKey::new(key.clone(),"default");
        let id=self.state.borrow().arena;
        let result={
            let web=self.state.borrow().storage.as_ref().expect("storage initialized").web.clone();
            let mut web=web.borrow_mut();
            if session {
                let area=web.session(id,key);
                match (name,value){(Some(name),Some(value))=>area.set(name,value,id).map_err(|e|format!("{e:?}")),
                    (Some(name),None)=>Ok(area.remove(&name,id)),(None,_)=>Ok(area.clear(id))}
            } else {
                web.change_local(bucket,|area|match (name,value){
                    (Some(name),Some(value))=>area.set(name,value,id),
                    (Some(name),None)=>Ok(area.remove(&name,id)),(None,_)=>Ok(area.clear(id)),
                }).map_err(|e|format!("{e:?}"))
            }
        };
        match result {
            Ok(Some(_change))=>{},
            Ok(None)=>{},
            Err(error) if error.contains("QuotaExceeded")=>return Err(self.storage_dom_error("QuotaExceededError","Storage quota exceeded")),
            Err(error)=>return Err(Throw::type_error(format!("Storage failure: {error}"))),
        }
        Ok(())
    }
    fn install_storage_legacy(&mut self){
        let brand=STORAGE_BRAND_BASE+SI::Storage as u64;
        let state=self.state.clone();
        let names=Rc::new(move |handle:HostHandle|{
            let borrowed=state.borrow();
            if handle.arena!=borrowed.arena||handle.brand!=brand{return Err(Throw::type_error("Invalid Storage handle"))}
            let url=borrowed.document_url.clone().ok_or_else(||Throw::type_error("Storage requires a document URL"))?;
            let key=Self::storage_key_for_url(&url)?;let bucket=BucketKey::new(key.clone(),"default");
            let storage=borrowed.storage.as_ref().ok_or_else(||Throw::type_error("Storage unavailable"))?;
            let session=matches!(storage.objects.get(handle.slot as usize),Some(StorageObject::Area {session:true}));
            let web=storage.web.clone();
            drop(borrowed);
            let mut web=web.borrow_mut();
            let area=if session {&*web.session(handle.arena,key)} else {match web.local(&bucket) {Some(area)=>area,None=>return Ok(Vec::new())}};
            Ok((0..area.len()).map(|i|area.key(i).unwrap().to_vec()).collect())
        });
        let state=self.state.clone();
        let get=Rc::new(move |js:&mut Interpreter,handle:HostHandle,name:&[u16]|{
            let mut adapter=RuntimeBindingHost {js,state:state.clone()};
            let value=adapter.storage_area_get(handle.slot as usize,name)?;
            Ok(value.map_or(JsValue::Null,JsValue::str_units))
        });
        let state=self.state.clone();
        let set=Rc::new(move |js:&mut Interpreter,handle:HostHandle,name:&[u16],value:JsValue|{
            let mut adapter=RuntimeBindingHost {js,state:state.clone()};
            let value=adapter.js.binding_to_dom_string(&value)?;
            adapter.storage_area_change(handle.slot as usize,Some(name.to_vec()),Some(value))
        });
        let state=self.state.clone();
        let delete=Rc::new(move |js:&mut Interpreter,handle:HostHandle,name:&[u16]|{
            let mut adapter=RuntimeBindingHost {js,state:state.clone()};
            adapter.storage_area_change(handle.slot as usize,Some(name.to_vec()),None)?;Ok(true)
        });
        self.js.binding_define_legacy_platform_object(brand,RuntimeLegacyHooks {
            supported_index_count:Rc::new(|_|Ok(0)),supported_names:Some(names),
            indexed_getter:Rc::new(|_,_,_|Ok(JsValue::Undefined)),named_getter:Some(get),
            indexed_setter:None,named_setter:Some(set),named_deleter:Some(delete),
            unenumerable_names:false,override_builtins:false,
        });
    }
    fn storage_invoke(&mut self,interface:SI,member:&'static str,kind:SM,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        match interface {
            SI::Storage=>self.storage_area_invoke(member,kind,this,args),
            SI::IDBFactory=>self.storage_factory_invoke(member,this,args),
            SI::IDBKeyRange=>self.storage_range_invoke(member,kind,this,args),
            SI::IDBVersionChangeEvent=>{
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let options=args.get(1).cloned().unwrap_or(JsValue::Undefined);
                let old=if matches!(options,JsValue::Object(_)){
                    let value=self.js.binding_get_property(&options,"oldVersion")?;
                    if matches!(value,JsValue::Undefined){0}else{self.js.binding_to_number(&value)? as u64}
                }else{0};
                let new=if matches!(options,JsValue::Object(_)){
                    let value=self.js.binding_get_property(&options,"newVersion")?;
                    if matches!(value,JsValue::Undefined|JsValue::Null){None}else{Some(self.js.binding_to_number(&value)? as u64)}
                }else{None};
                self.storage_version_event(&name,old,new)
            },
            SI::CacheStorage|SI::Cache=>self.storage_cache_invoke(interface,member,this,args),
            _=>self.storage_idb_invoke(interface,member,kind,this,args),
        }
    }
    fn storage_area_invoke(&mut self,member:&str,_kind:SM,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        if member=="localStorage"||member=="sessionStorage" {
            let _=self.storage_key()?;self.storage_ensure()?;
            let session=member=="sessionStorage";
            let existing={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                storage.objects.iter().position(|o|matches!(o,StorageObject::Area{session:s} if *s==session))};
            if let Some(slot)=existing{return self.storage_wrapper(slot)}
            return self.storage_allocate(SI::Storage,StorageObject::Area{session});
        }
        let slot=self.storage_slot(&this,SI::Storage)?;
        match member {
            "length"=>Ok(JsValue::num(self.storage_area_pairs(slot)?.len() as f64)),
            "key"=>{
                let number=self.js.binding_to_number(self.storage_arg(args,0)?)?;
                let index=if number.is_finite()&&number>=0.0{number as usize}else{usize::MAX};
                Ok(self.storage_area_pairs(slot)?.get(index).map_or(JsValue::Null,|(name,_)|JsValue::str_units(name.clone())))
            },
            "getItem"=>{let key=self.js.binding_to_dom_string(self.storage_arg(args,0)?)?;
                Ok(self.storage_area_get(slot,&key)?.map_or(JsValue::Null,JsValue::str_units))},
            "setItem"=>{let key=self.js.binding_to_dom_string(self.storage_arg(args,0)?)?;
                let value=self.js.binding_to_dom_string(self.storage_arg(args,1)?)?;
                self.storage_area_change(slot,Some(key),Some(value))?;Ok(JsValue::Undefined)},
            "removeItem"=>{let key=self.js.binding_to_dom_string(self.storage_arg(args,0)?)?;
                self.storage_area_change(slot,Some(key),None)?;Ok(JsValue::Undefined)},
            "clear"=>{self.storage_area_change(slot,None,None)?;Ok(JsValue::Undefined)},
            _=>Err(Throw::type_error("Unknown Storage member")),
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn storage_idb_error(&mut self, error: idb::Error) -> Throw {
        let name = match error {
            idb::Error::Data => "DataError", idb::Error::Constraint => "ConstraintError",
            idb::Error::NotFound => "NotFoundError", idb::Error::InvalidState => "InvalidStateError",
            idb::Error::ReadOnly => "ReadOnlyError", idb::Error::TransactionInactive => "TransactionInactiveError",
            idb::Error::Abort => "AbortError", idb::Error::Version => "VersionError",
            idb::Error::Blocked => "InvalidStateError", idb::Error::QuotaExceeded => "QuotaExceededError",
        };
        self.storage_dom_error(name, name)
    }
    fn storage_request(&mut self, open: bool, source: JsValue, transaction: JsValue) -> Result<(usize, JsValue), Throw> {
        let kind=if open {SI::IDBOpenDBRequest}else{SI::IDBRequest};
        let value=self.storage_allocate(kind,StorageObject::Request {result:JsValue::Undefined,error:JsValue::Null,
            ready:false,source,transaction,handlers:HashMap::new(),listeners:Vec::new()})?;
        let slot=self.storage_slot(&value,kind)?;
        Ok((slot,value))
    }
    fn storage_factory_invoke(&mut self, member: &str, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let _=self.storage_slot(&this,SI::IDBFactory)?;
        let key=self.storage_key()?;
        match member {
            "cmp"=>{
                if args.len()<2 {
                    let global=self.js.binding_global_object();
                    let constructor=self.js.binding_get_property(&global,"TypeError")?;
                    let error=self.js.binding_construct(&constructor,vec![JsValue::str("Missing required argument")])?;
                    return Err(self.js.binding_throw(error));
                }
                let a=self.storage_to_key(self.storage_arg(args,0)?)?;
                let b=self.storage_to_key(self.storage_arg(args,1)?)?;
                Ok(JsValue::num(a.cmp(&b) as i32 as f64))
            },
            "databases"=>{
                let names=self.state.borrow().storage.as_ref().unwrap().idb.borrow().databases(&key);
                let mut entries=Vec::new();
                for (name,version) in names {
                    let entry=self.js.binding_create_object(None)?;
                    self.define_data_property(&entry,"name",JsValue::str(name),true,true,true)?;
                    self.define_data_property(&entry,"version",JsValue::num(version as f64),true,true,true)?;
                    entries.push(entry);
                }
                let array=self.js.binding_create_array(entries)?;
                let promise=self.new_promise()?;
                self.resolve_promise(&promise,array)?;
                Ok(promise)
            },
            "open"=>{
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let version=if let Some(value)=args.get(1) {Some(self.js.binding_to_number(value)? as u64)}else{None};
                if version==Some(0){return Err(Throw::type_error("Database version must be positive"))}
                let (request,wrapper)=self.storage_request(true,JsValue::Null,JsValue::Null)?;
                let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().open(key.clone(),name.clone(),version);
                match result {
                    Ok(open)=>{
                        if open.blocked {self.state.borrow_mut().storage.as_mut().unwrap().blocked_opens.push_back((key,name.clone(),request));}
                        for event in open.events {
                            match event {
                                idb::OpenEvent::UpgradeNeeded{old,new}=>{
                                    self.storage_schedule(StorageJob::OpenUpgrade{request,connection:open.connection.unwrap(),name:name.clone(),old,new})?;
                                },
                                idb::OpenEvent::Blocked{old,new}=>{
                                    self.storage_schedule(StorageJob::OpenBlocked{request,old,new})?;
                                },
                                idb::OpenEvent::Success{connection,..}=>{
                                    let database=self.storage_allocate(SI::IDBDatabase,StorageObject::Database{connection,name:name.clone(),handlers:HashMap::new(),listeners:Vec::new()})?;
                                    self.storage_schedule(StorageJob::RequestSuccess{request,result:database})?;
                                },
                                idb::OpenEvent::VersionChange{connection,old,new}=>self.storage_schedule(StorageJob::DatabaseVersionChange{connection,old,new})?,
                            }
                        }
                    },
                    Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;},
                }
                Ok(wrapper)
            },
            "deleteDatabase"=>{
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let (request,wrapper)=self.storage_request(true,JsValue::Null,JsValue::Null)?;
                let old=self.state.borrow().storage.as_ref().unwrap().idb.borrow().database(&key,&name).map_or(0,|db|db.version);
                let connections={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();storage.objects.iter().filter_map(|object|match object{
                    StorageObject::Database{connection,name:db_name,..} if db_name==&name=>Some(*connection),_=>None}).collect::<Vec<_>>()};
                let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().delete_database(&key,&name);
                match result {
                    Ok(_)=>self.storage_schedule(StorageJob::DeleteSuccess{request,old})?,
                    Err(idb::Error::Blocked)=>{
                        self.state.borrow_mut().storage.as_mut().unwrap().blocked_deletes.push_back((key,name,request));
                        for connection in connections {self.storage_schedule(StorageJob::DatabaseVersionChange{connection,old,new:None})?;}
                        self.storage_schedule(StorageJob::OpenBlocked{request,old,new:None})?;
                    },
                    Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;},
                }
                Ok(wrapper)
            },
            _=>Err(Throw::type_error("Unknown IDBFactory member")),
        }
    }
    fn storage_resume_blocked(&mut self)->Result<(),Throw>{
        let idb=self.state.borrow().storage.as_ref().unwrap().idb.clone();
        let resumed=idb.borrow_mut().resume_pending();
        for open in resumed {
            let Some(connection)=open.connection else{continue};
            let (key,name)={let idb=idb.borrow();let Some(info)=idb.connection(connection) else{continue};(info.key.clone(),info.name.clone())};
            let request={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();
                storage.blocked_opens.iter().position(|(pending_key,pending_name,_)|*pending_key==key&&*pending_name==name)
                    .and_then(|position|storage.blocked_opens.remove(position)).map(|(_,_,request)|request)};
            if let Some(request)=request {
                for event in open.events {
                    if let idb::OpenEvent::UpgradeNeeded{old,new}=event {
                        self.storage_schedule(StorageJob::OpenUpgrade{request,connection,name:name.clone(),old,new})?;
                    }
                }
            }
        }
        let pending={let mut state=self.state.borrow_mut();std::mem::take(&mut state.storage.as_mut().unwrap().blocked_deletes)};
        for (key,name,request) in pending {
            match idb.borrow_mut().delete_database(&key,&name) {
                Ok(old)=>self.storage_schedule(StorageJob::DeleteSuccess{request,old})?,
                Err(idb::Error::Blocked)=>self.state.borrow_mut().storage.as_mut().unwrap().blocked_deletes.push_back((key,name,request)),
                Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;},
            }
        }
        Ok(())
    }
    fn storage_range_invoke(&mut self, member: &str, kind: SM, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if kind==SM::StaticMethod {
            let a=self.storage_to_key(self.storage_arg(args,0)?)?;
            let range=match member {
                "only"=>KeyRange::only(a),
                "lowerBound"=>KeyRange{lower:Some(a),upper:None,lower_open:args.get(1).is_some_and(|v|self.js.binding_to_boolean(v)),upper_open:false},
                "upperBound"=>KeyRange{lower:None,upper:Some(a),lower_open:false,upper_open:args.get(1).is_some_and(|v|self.js.binding_to_boolean(v))},
                "bound"=>{let b=self.storage_to_key(self.storage_arg(args,1)?)?;
                    KeyRange::bound(a,b,args.get(2).is_some_and(|v|self.js.binding_to_boolean(v)),
                        args.get(3).is_some_and(|v|self.js.binding_to_boolean(v))).map_err(|e|self.storage_idb_error(e))?},
                _=>return Err(Throw::type_error("Unknown IDBKeyRange member")),
            };
            return self.storage_allocate(SI::IDBKeyRange,StorageObject::Range(range));
        }
        let slot=self.storage_slot(&this,SI::IDBKeyRange)?;
        let range=match self.state.borrow().storage.as_ref().unwrap().objects.get(slot){Some(StorageObject::Range(range))=>range.clone(),_=>unreachable!()};
        match member {
            "lower"=>range.lower.as_ref().map_or(Ok(JsValue::Undefined),|v|self.storage_from_key(v)),
            "upper"=>range.upper.as_ref().map_or(Ok(JsValue::Undefined),|v|self.storage_from_key(v)),
            "lowerOpen"=>Ok(JsValue::Bool(range.lower_open)),"upperOpen"=>Ok(JsValue::Bool(range.upper_open)),
            "includes"=>Ok(JsValue::Bool(range.includes(&self.storage_to_key(self.storage_arg(args,0)?)?))),
            _=>Err(Throw::type_error("Unknown IDBKeyRange member")),
        }
    }
    fn storage_cache_invoke(&mut self, _interface: SI, _member: &str, _this: JsValue, _args: &[JsValue]) -> Result<JsValue, Throw> {
        Err(Throw::type_error("Cache API awaits Fetch wrappers"))
    }
    fn storage_idb_invoke(&mut self, interface: SI, member: &str, kind: SM, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if matches!(member,"addEventListener"|"removeEventListener") {
            let slot=self.storage_slot(&this,interface)?;
            let name=self.storage_string(self.storage_arg(args,0)?)?;
            let callback=self.storage_arg(args,1)?.clone();
            let mut state=self.state.borrow_mut();
            let listeners=match &mut state.storage.as_mut().unwrap().objects[slot] {
                StorageObject::Request{listeners,..}|StorageObject::Transaction{listeners,..}|StorageObject::Database{listeners,..}=>listeners,
                _=>return Err(Throw::type_error("Not an event target")),
            };
            if member=="addEventListener" {listeners.push((name,callback));}
            else {listeners.retain(|(event,listener)|event!=&name||!self.js.binding_same_value(listener,&callback));}
            return Ok(JsValue::Undefined)
        }
        match interface {
            SI::IDBRequest|SI::IDBOpenDBRequest=>{
                let slot=self.storage_slot(&this,interface)?;
                let (result,error,ready,source,transaction,handler)={let state=self.state.borrow();
                    match &state.storage.as_ref().unwrap().objects[slot] {
                        StorageObject::Request{result,error,ready,source,transaction,handlers,..}=>(result.clone(),error.clone(),*ready,source.clone(),transaction.clone(),handlers.get(member.trim_start_matches("on")).cloned()),
                        _=>unreachable!(),
                    }};
                if member.starts_with("on") {
                    if kind==SM::Setter {let callback=args.first().cloned().unwrap_or(JsValue::Null);
                        if let StorageObject::Request{handlers,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{handlers.insert(member[2..].to_owned(),callback);}
                        return Ok(JsValue::Undefined)}
                    return Ok(handler.unwrap_or(JsValue::Null))
                }
                match member {"result"=>if ready{Ok(result)}else{Err(self.storage_dom_error("InvalidStateError","Request is pending"))},
                    "error"=>if ready {Ok(error)}else{Err(self.storage_dom_error("InvalidStateError","Request is pending"))},
                    "source"=>Ok(source),"transaction"=>Ok(transaction),
                    "readyState"=>Ok(JsValue::str(if ready{"done"}else{"pending"})),
                    _=>Err(Throw::type_error("Unknown IDBRequest member")),}
            },
            SI::IDBDatabase=>{
                let slot=self.storage_slot(&this,interface)?;
                if member.starts_with("on") {
                    if kind==SM::Setter {let callback=args.first().cloned().unwrap_or(JsValue::Null);
                        if let StorageObject::Database{handlers,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{handlers.insert(member[2..].to_owned(),callback);}
                        return Ok(JsValue::Undefined)}
                    let state=self.state.borrow();if let StorageObject::Database{handlers,..}=&state.storage.as_ref().unwrap().objects[slot]{return Ok(handlers.get(&member[2..]).cloned().unwrap_or(JsValue::Null))}
                }
                let (connection,name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{StorageObject::Database{connection,name,..}=>(*connection,name.clone()),_=>unreachable!()}};
                let key=self.storage_key()?;
                match member {
                    "name"=>Ok(JsValue::str(name)),
                    "version"=>{let state=self.state.borrow();let version=state.storage.as_ref().unwrap().idb.borrow().connection(connection).map_or(0,|connection|connection.version);
                        Ok(JsValue::num(version as f64))},
                    "objectStoreNames"=>{let names={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                        storage.objects.iter().find_map(|object|match object {StorageObject::Transaction{connection:id,mode:Mode::VersionChange,value:Some(tx),..} if *id==connection=>
                            Some(tx.database().store_names().into_iter().map(str::to_owned).collect::<Vec<_>>()),_=>None})
                            .or_else(||storage.idb.borrow().database(&key,&name).map(|db|db.store_names().into_iter().map(str::to_owned).collect::<Vec<_>>())).unwrap_or_default()};
                        self.js.binding_create_array(names.into_iter().map(JsValue::str).collect())},
                    "close"=>{let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().close(connection);
                        if let Err(error)=result {if error!=idb::Error::NotFound{return Err(self.storage_idb_error(error))}}
                        self.storage_resume_blocked()?;Ok(JsValue::Undefined)},
                    "transaction"=>{
                        let names_value=self.storage_arg(args,0)?.clone();
                        let names=if let JsValue::Object(object)=&names_value {if matches!(object.borrow().exotic,Exotic::Array){
                            let length=self.js.binding_get_property(&names_value,"length")?;let length=self.js.binding_to_number(&length)? as usize;
                            let mut names=Vec::new();for index in 0..length {let value=self.js.binding_get_property(&names_value,&index.to_string())?;names.push(self.storage_string(&value)?);}names
                        }else{vec![self.storage_string(&names_value)?]}}else{vec![self.storage_string(&names_value)?]};
                        let mode=if args.get(1).is_some_and(|v|matches!(v,JsValue::Str(s) if s.to_utf8_lossy()=="readwrite")){Mode::ReadWrite}else{Mode::ReadOnly};
                        self.storage_new_transaction(connection,names,mode)
                    },
                    "createObjectStore"=>{
                        let name=self.storage_string(self.storage_arg(args,0)?)?;
                        let options=args.get(1).cloned().unwrap_or(JsValue::Undefined);
                        let key_path=if matches!(options,JsValue::Object(_)) {let path=self.js.binding_get_property(&options,"keyPath")?;
                            self.storage_parse_key_path(&path)?}else{None};
                        let auto=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"autoIncrement")?;self.js.binding_to_boolean(&value)}else{false};
                        let transaction=self.storage_upgrade_transaction(connection)?;
                        {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{unreachable!()};
                            tx.database_mut().and_then(|db|db.create_store(name.clone(),key_path,auto)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
                        self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction,name})
                    },
                    "deleteObjectStore"=>{let name=self.storage_string(self.storage_arg(args,0)?)?;let transaction=self.storage_upgrade_transaction(connection)?;
                        {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{unreachable!()};
                            tx.database_mut().and_then(|db|db.delete_store(&name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}Ok(JsValue::Undefined)},
                    _=>Ok(JsValue::Null),
                }
            },
            SI::IDBTransaction=>self.storage_transaction_invoke(member,kind,this,args),
            SI::IDBObjectStore=>self.storage_store_invoke(member,this,args),
            SI::IDBIndex=>self.storage_index_invoke(member,this,args),
            SI::IDBCursor|SI::IDBCursorWithValue=>self.storage_cursor_invoke(interface,member,this,args),
            _=>Err(Throw::type_error("IndexedDB interface method unavailable")),
        }
    }
    fn storage_new_transaction(&mut self,connection:u64,names:Vec<String>,mode:Mode)->Result<JsValue,Throw>{
        let begun={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,names.clone(),mode,Durability::Default)};
        let (id,value,queued)=match begun {
            Ok((id,value))=>(id,Some(value),false),
            Err(idb::Error::Blocked)=>{
                let valid={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                    storage.idb.borrow().connection(connection).and_then(|c|storage.idb.borrow().database(&c.key,&c.name).map(|db|
                        names.iter().all(|name|db.store_names().contains(&name.as_str())))).unwrap_or(false)};
                if !valid {return Err(self.storage_dom_error("NotFoundError","Object store not found"))}
                (0,None,true)
            },
            Err(error)=>return Err(self.storage_idb_error(error)),
        };
        let wrapper=self.storage_allocate(SI::IDBTransaction,StorageObject::Transaction{connection,id,value,mode,scope:names,queued,active_for_requests:true,handlers:HashMap::new(),listeners:Vec::new()})?;
        let slot=self.storage_slot(&wrapper,SI::IDBTransaction)?;
        self.storage_schedule(StorageJob::TransactionComplete{transaction:slot})?;
        Ok(wrapper)
    }
    fn storage_upgrade_transaction(&mut self,connection:u64)->Result<usize,Throw>{
        let slot=self.state.borrow().storage.as_ref().unwrap().objects.iter().position(|object|
            matches!(object,StorageObject::Transaction{connection:id,mode:Mode::VersionChange,value:Some(_),..} if *id==connection));
        slot.ok_or_else(||self.storage_dom_error("InvalidStateError","No active upgrade transaction"))
    }
    fn storage_transaction_accepts_request(&mut self,slot:usize)->Result<(),Throw>{
        let active={let state=self.state.borrow();matches!(state.storage.as_ref().unwrap().objects.get(slot),
            Some(StorageObject::Transaction{value:Some(tx),active_for_requests:true,..}) if tx.is_active())};
        if active {Ok(())} else {Err(self.storage_dom_error("TransactionInactiveError","Transaction is inactive"))}
    }
    fn storage_transaction_invoke(&mut self,member:&str,kind:SM,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBTransaction)?;
        if member.starts_with("on") {
            if kind==SM::Setter {let callback=args.first().cloned().unwrap_or(JsValue::Null);
                if let StorageObject::Transaction{handlers,..}=&mut self.state.borrow_mut().storage.as_mut().unwrap().objects[slot]{handlers.insert(member[2..].to_owned(),callback);}
                return Ok(JsValue::Undefined)}
            let state=self.state.borrow();if let StorageObject::Transaction{handlers,..}=&state.storage.as_ref().unwrap().objects[slot]{return Ok(handlers.get(&member[2..]).cloned().unwrap_or(JsValue::Null))}
        }
        let (connection,mode,names,active)={let state=self.state.borrow();let StorageObject::Transaction{connection,mode,scope,value,queued,..}=&state.storage.as_ref().unwrap().objects[slot] else{unreachable!()};
            (*connection,*mode,scope.clone(),value.is_some()||*queued)};
        match member {
            "mode"=>Ok(JsValue::str(match mode{Mode::ReadOnly=>"readonly",Mode::ReadWrite=>"readwrite",Mode::VersionChange=>"versionchange"})),
            "durability"=>Ok(JsValue::str("default")),"error"=>Ok(JsValue::Null),
            "objectStoreNames"=>self.js.binding_create_array(names.into_iter().map(JsValue::str).collect()),
            "db"=>{let state=self.state.borrow();Ok(state.storage.as_ref().unwrap().objects.iter().enumerate().find_map(|(index,object)|
                matches!(object,StorageObject::Database{connection:id,..} if *id==connection).then(||state.storage.as_ref().unwrap().wrappers.get(&index).cloned()).flatten()).unwrap_or(JsValue::Null))},
            "objectStore"=>{if !active{return Err(self.storage_dom_error("TransactionInactiveError","Transaction has finished"))}
                let name=self.storage_string(self.storage_arg(args,0)?)?;
                let found={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                    match &storage.objects[slot] {
                        StorageObject::Transaction{value:Some(tx),..}=>tx.store(&name).is_ok(),
                        StorageObject::Transaction{queued:true,..}=>{
                            let idb=storage.idb.borrow();
                            idb.connection(connection).and_then(|connection|idb.database(&connection.key,&connection.name))
                                .is_some_and(|db|db.store_names().contains(&name.as_str()))
                        },
                        _=>false,
                    }
                };
                if !found {return Err(self.storage_dom_error("NotFoundError","Object store not found"))}
                self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction:slot,name})},
            "abort"=>{let mut state=self.state.borrow_mut();if let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[slot]{tx.abort();}Ok(JsValue::Undefined)},
            "commit"=>Ok(JsValue::Undefined),
            _=>Err(Throw::type_error("Unknown IDBTransaction member")),
        }
    }
    fn storage_store_invoke(&mut self,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBObjectStore)?;
        let (transaction,name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{StorageObject::Store{transaction,name}=>(*transaction,name.clone()),_=>unreachable!()}};
        match member {
            "name"=>return Ok(JsValue::str(name)),"transaction"=>return self.storage_wrapper(transaction),
            "keyPath"|"autoIncrement"|"indexNames"=>{
                let (key_path,auto,names)={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                    let store=tx.store(&name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
                    (store.key_path.clone(),store.auto_increment,store.index_names().into_iter().map(str::to_owned).collect::<Vec<_>>())};
                return match member {"keyPath"=>key_path.as_ref().map_or(Ok(JsValue::Null),|path|self.storage_key_path_value(path)),
                    "autoIncrement"=>Ok(JsValue::Bool(auto)),_=>self.js.binding_create_array(names.into_iter().map(JsValue::str).collect())};
            },
            _=>{},
        }
        if matches!(member,"createIndex"|"index"|"deleteIndex") {
            if member!="index" {self.storage_transaction_accepts_request(transaction)?;}
            let index_name=self.storage_string(self.storage_arg(args,0)?)?;
            if member=="index" {
                {let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                    tx.store(&name).and_then(|store|store.index(&index_name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
                return self.storage_allocate(SI::IDBIndex,StorageObject::Index{transaction,store:name,name:index_name});
            }
            if member=="createIndex" {
                let path=self.storage_parse_key_path(self.storage_arg(args,1)?)?
                    .ok_or_else(||self.storage_dom_error("SyntaxError","Index key path is required"))?;
                let options=args.get(2).cloned().unwrap_or(JsValue::Undefined);
                let unique=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"unique")?;self.js.binding_to_boolean(&value)}else{false};
                let multi=if matches!(options,JsValue::Object(_)){let value=self.js.binding_get_property(&options,"multiEntry")?;self.js.binding_to_boolean(&value)}else{false};
                {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),mode:Mode::VersionChange,..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{return Err(Throw::type_error("No active upgrade transaction"))};
                    tx.store_mut(&name).and_then(|store|store.create_index(index_name.clone(),path,unique,multi)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
                return self.storage_allocate(SI::IDBIndex,StorageObject::Index{transaction,store:name,name:index_name});
            }
            {let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),mode:Mode::VersionChange,..}=&mut state.storage.as_mut().unwrap().objects[transaction] else{return Err(Throw::type_error("No active upgrade transaction"))};
                tx.store_mut(&name).and_then(|store|store.delete_index(&index_name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;}
            return Ok(JsValue::Undefined);
        }
        if member=="openCursor"||member=="openKeyCursor" {
            self.storage_transaction_accepts_request(transaction)?;
            let range=self.storage_query_range(args.first())?;
            let direction=self.storage_direction(args.get(1))?;
            let cursor={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction] else{return Err(Throw::type_error("Inactive transaction"))};
                tx.store(&name).map_err(|error|Throw::type_error(format!("{error:?}")))?.cursor(range.as_ref(),direction)};
            return self.storage_open_cursor(cursor,transaction,slot,this,member=="openCursor");
        }
        self.storage_transaction_accepts_request(transaction)?;
        let (key,range)=if matches!(member,"add"|"put") {
            let key=args.get(1).map(|value|self.storage_to_key(value)).transpose()?;(key,None)
        }else if let Some(value)=args.first(){
            if let Some(handle)=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBKeyRange as u64){
                let state=self.state.borrow();let range=match &state.storage.as_ref().unwrap().objects[handle.slot as usize]{StorageObject::Range(range)=>range.clone(),_=>unreachable!()};(None,Some(range))
            }else{let key=self.storage_to_key(value)?;(Some(key.clone()),Some(KeyRange::only(key)))}
        }else{(None,None)};
        let value=if matches!(member,"add"|"put"){Some(self.storage_clone_value(self.storage_arg(args,0)?,&mut Vec::new())?)}else{None};
        let limit=if matches!(member,"getAll"|"getAllKeys") {args.get(1).filter(|v|!matches!(v,JsValue::Undefined)).map(|v|self.js.binding_to_number(v).map(|n|n.max(0.0) as usize)).transpose()?}else{None};
        let outcome={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();
            let StorageObject::Transaction{value:Some(tx),..}=&mut storage.objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
            match member {
                "add"|"put"=>tx.store_mut(&name).and_then(|store|store.put(value.unwrap(),key,member=="put")).map(StoreResult::Key),
                "get"|"getKey"|"getAll"|"getAllKeys"|"count"=>tx.store(&name).map(|store|{
                    let records=store.records(range.as_ref(),limit);
                    match member {"get"=>StoreResult::Value(records.first().map(|(_,value)|value.clone())),
                        "getKey"=>StoreResult::OptionalKey(records.first().map(|(key,_)|key.clone())),
                        "getAll"=>StoreResult::Values(records.into_iter().map(|(_,value)|value).collect()),
                        "getAllKeys"=>StoreResult::Keys(records.into_iter().map(|(key,_)|key).collect()),
                        _=>StoreResult::Count(store.count(range.as_ref()))}
                }),
                "delete"=>tx.store_mut(&name).map(|store|{if let Some(key)=key.as_ref(){store.delete(key);}StoreResult::None}),
                "clear"=>tx.store_mut(&name).map(|store|{store.clear();StoreResult::None}),
                _=>Err(idb::Error::InvalidState),
            }
        };
        let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
        let result=match outcome {
            Ok(StoreResult::None)=>JsValue::Undefined,Ok(StoreResult::Count(count))=>JsValue::num(count as f64),
            Ok(StoreResult::Key(key))=>self.storage_from_key(&key)?,
            Ok(StoreResult::OptionalKey(key))=>key.as_ref().map_or(Ok(JsValue::Undefined),|key|self.storage_from_key(key))?,
            Ok(StoreResult::Keys(keys))=>{let mut items=Vec::new();for key in keys{items.push(self.storage_from_key(&key)?)}self.js.binding_create_array(items)?},
            Ok(StoreResult::Value(value))=>value.as_ref().map_or(Ok(JsValue::Undefined),|value|self.storage_from_value(value))?,
            Ok(StoreResult::Values(values))=>{let mut items=Vec::new();for value in values{items.push(self.storage_from_value(&value)?)}self.js.binding_create_array(items)?},
            Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;return Ok(wrapper)},
        };
        self.storage_schedule(StorageJob::RequestSuccess{request,result})?;
        Ok(wrapper)
    }
    fn storage_query_range(&mut self,value:Option<&JsValue>)->Result<Option<KeyRange>,Throw>{
        let Some(value)=value else{return Ok(None)};
        if matches!(value,JsValue::Undefined|JsValue::Null){return Ok(None)}
        if let Some(handle)=self.js.binding_host_handle(value,STORAGE_BRAND_BASE+SI::IDBKeyRange as u64){
            let state=self.state.borrow();return match &state.storage.as_ref().unwrap().objects[handle.slot as usize]{StorageObject::Range(range)=>Ok(Some(range.clone())),_=>Err(Throw::type_error("Invalid key range"))}
        }
        Ok(Some(KeyRange::only(self.storage_to_key(value)?)))
    }
    fn storage_parse_key_path(&mut self,value:&JsValue)->Result<Option<idb::KeyPath>,Throw>{
        if matches!(value,JsValue::Undefined|JsValue::Null){return Ok(None)}
        let path=if let JsValue::Object(object)=value {
            if matches!(object.borrow().exotic,Exotic::Array){
                let length=self.js.binding_get_property(value,"length")?;
                let count=self.js.binding_to_number(&length)? as usize;
                if count>100_000{return Err(self.storage_dom_error("SyntaxError","Key path is too long"))}
                let mut parts=Vec::with_capacity(count);
                for index in 0..count {
                    let item=self.js.binding_get_property(value,&index.to_string())?;
                    parts.push(self.storage_string(&item)?);
                }
                idb::KeyPath::Compound(parts)
            }else{idb::KeyPath::Single(self.storage_string(value)?)}
        }else{idb::KeyPath::Single(self.storage_string(value)?)};
        let valid=match &path {idb::KeyPath::Single(path)=>Self::storage_valid_key_path(path),
            idb::KeyPath::Compound(paths)=>paths.iter().all(|path|Self::storage_valid_key_path(path))};
        if !valid{return Err(self.storage_dom_error("SyntaxError","Invalid key path"))}
        Ok(Some(path))
    }
    fn storage_valid_key_path(path:&str)->bool {
        if path.is_empty(){return true}
        path.split('.').all(|part|{
            let mut chars=part.chars();
            chars.next().is_some_and(|c|c=='_'||c=='$'||c.is_alphabetic())
                &&chars.all(|c|c=='_'||c=='$'||c=='\u{200c}'||c=='\u{200d}'||c.is_alphanumeric())
        })
    }
    fn storage_key_path_value(&mut self,path:&idb::KeyPath)->Result<JsValue,Throw>{
        match path {idb::KeyPath::Single(path)=>Ok(JsValue::str(path)),
            idb::KeyPath::Compound(paths)=>self.js.binding_create_array(paths.iter().map(JsValue::str).collect())}
    }
    fn storage_direction(&mut self,value:Option<&JsValue>)->Result<Direction,Throw>{
        let Some(value)=value else{return Ok(Direction::Next)};
        match self.storage_string(value)?.as_str(){"next"=>Ok(Direction::Next),"nextunique"=>Ok(Direction::NextUnique),
            "prev"=>Ok(Direction::Prev),"prevunique"=>Ok(Direction::PrevUnique),_=>Err(Throw::type_error("Invalid cursor direction"))}
    }
    fn storage_open_cursor(&mut self,mut cursor:IdbCursor,transaction:usize,source:usize,source_value:JsValue,with_value:bool)->Result<JsValue,Throw>{
        let present=cursor.advance(1).map_err(|error|self.storage_idb_error(error))?.is_some();
        let (request,wrapper)=self.storage_request(false,source_value,self.storage_wrapper(transaction)?)?;
        let result=if present {let kind=if with_value{SI::IDBCursorWithValue}else{SI::IDBCursor};
            self.storage_allocate(kind,StorageObject::Cursor{cursor,transaction,source,request,with_value})?}else{JsValue::Null};
        self.storage_schedule(StorageJob::RequestSuccess{request,result})?;
        Ok(wrapper)
    }
    fn storage_index_invoke(&mut self,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,SI::IDBIndex)?;
        let (transaction,store_name,index_name)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[slot]{
            StorageObject::Index{transaction,store,name}=>(*transaction,store.clone(),name.clone()),_=>unreachable!()}};
        if member=="name" {return Ok(JsValue::str(index_name))}
        if member=="objectStore" {return self.storage_allocate(SI::IDBObjectStore,StorageObject::Store{transaction,name:store_name})}
        if matches!(member,"keyPath"|"unique"|"multiEntry") {
            let (path,unique,multi)={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                let index=tx.store(&store_name).and_then(|store|store.index(&index_name)).map_err(|error|Throw::type_error(format!("{error:?}")))?;
                (index.key_path.clone(),index.unique,index.multi_entry)};
            return match member {"keyPath"=>self.storage_key_path_value(&path),
                "unique"=>Ok(JsValue::Bool(unique)),_=>Ok(JsValue::Bool(multi))}
        }
        let range=self.storage_query_range(args.first())?;
        self.storage_transaction_accepts_request(transaction)?;
        let limit=if matches!(member,"getAll"|"getAllKeys") {args.get(1).filter(|v|!matches!(v,JsValue::Undefined)).map(|v|self.js.binding_to_number(v).map(|n|n.max(0.0) as usize)).transpose()?}else{None};
        if member=="openCursor"||member=="openKeyCursor" {
            let direction=self.storage_direction(args.get(1))?;
            let cursor={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                let store=tx.store(&store_name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
                store.index(&index_name).map_err(|error|Throw::type_error(format!("{error:?}")))?.cursor(store,range.as_ref(),direction)};
            return self.storage_open_cursor(cursor,transaction,slot,this,member=="openCursor");
        }
        let outcome={let state=self.state.borrow();let StorageObject::Transaction{value:Some(tx),..}=&state.storage.as_ref().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
            let store=tx.store(&store_name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
            let index=store.index(&index_name).map_err(|error|Throw::type_error(format!("{error:?}")))?;
            let records=index.records(store,range.as_ref(),limit);
            match member {"get"=>StoreResult::Value(records.first().map(|(_,_,value)|value.clone())),
                "getKey"=>StoreResult::OptionalKey(records.first().map(|(_,key,_)|key.clone())),
                "getAll"=>StoreResult::Values(records.into_iter().map(|(_,_,value)|value).collect()),
                "getAllKeys"=>StoreResult::Keys(records.into_iter().map(|(_,key,_)|key).collect()),
                "count"=>StoreResult::Count(index.count(range.as_ref())),
                _=>return Err(Throw::type_error("Unknown IDBIndex member")),
            }};
        let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
        let result=match outcome {StoreResult::Value(value)=>value.as_ref().map_or(Ok(JsValue::Undefined),|value|self.storage_from_value(value))?,
            StoreResult::OptionalKey(key)=>key.as_ref().map_or(Ok(JsValue::Undefined),|key|self.storage_from_key(key))?,
            StoreResult::Keys(keys)=>{let mut items=Vec::new();for key in keys{items.push(self.storage_from_key(&key)?)}self.js.binding_create_array(items)?},
            StoreResult::Values(values)=>{let mut items=Vec::new();for value in values{items.push(self.storage_from_value(&value)?)}self.js.binding_create_array(items)?},
            StoreResult::Count(count)=>JsValue::num(count as f64),_=>JsValue::Undefined};
        self.storage_schedule(StorageJob::RequestSuccess{request,result})?;Ok(wrapper)
    }
    fn storage_cursor_invoke(&mut self,interface:SI,member:&str,this:JsValue,args:&[JsValue])->Result<JsValue,Throw>{
        let slot=self.storage_slot(&this,interface)?;
        let (record,direction,source,request,transaction,with_value)={let state=self.state.borrow();let StorageObject::Cursor{cursor,source,request,transaction,with_value}= &state.storage.as_ref().unwrap().objects[slot]else{unreachable!()};
            (cursor.current().cloned(),cursor.direction(),*source,*request,*transaction,*with_value)};
        match member {
            "source"=>self.storage_wrapper(source),"request"=>self.storage_wrapper(request),
            "direction"=>Ok(JsValue::str(match direction{Direction::Next=>"next",Direction::NextUnique=>"nextunique",Direction::Prev=>"prev",Direction::PrevUnique=>"prevunique"})),
            "key"=>record.as_ref().map_or(Ok(JsValue::Undefined),|record|self.storage_from_key(&record.key)),
            "primaryKey"=>record.as_ref().map_or(Ok(JsValue::Undefined),|record|self.storage_from_key(&record.primary_key)),
            "value" if with_value=>record.as_ref().map_or(Ok(JsValue::Undefined),|record|self.storage_from_value(&record.value)),
            "advance"|"continue"|"continuePrimaryKey"=>{
                self.storage_transaction_accepts_request(transaction)?;
                let key=if member=="continue"&&args.first().is_some(){Some(self.storage_to_key(&args[0])?)}else{None};
                let pair=if member=="continuePrimaryKey"{Some((self.storage_to_key(self.storage_arg(args,0)?)?,self.storage_to_key(self.storage_arg(args,1)?)?))}else{None};
                let count=if member=="advance"{self.js.binding_to_number(self.storage_arg(args,0)?)? as usize}else{1};
                let result={let mut state=self.state.borrow_mut();let StorageObject::Cursor{cursor,..}=&mut state.storage.as_mut().unwrap().objects[slot]else{unreachable!()};
                    match member {"advance"=>cursor.advance(count),"continue"=>cursor.continue_to(key.as_ref()),
                        _=>{let (key,primary)=pair.as_ref().unwrap();cursor.continue_primary_key(key,primary)}}.map(|record|record.is_some())};
                let present=result.map_err(|error|self.storage_idb_error(error))?;
                if let Some(StorageObject::Request{ready,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){*ready=false;}
                self.storage_schedule(StorageJob::RequestSuccess{request,result:if present{this}else{JsValue::Null}})?;
                Ok(JsValue::Undefined)
            },
            "update"|"delete"=>{
                self.storage_transaction_accepts_request(transaction)?;
                let record=record.ok_or_else(||self.storage_dom_error("InvalidStateError","Cursor exhausted"))?;
                let value=if member=="update"{Some(self.storage_clone_value(self.storage_arg(args,0)?,&mut Vec::new())?)}else{None};
                let (store_name,_)={let state=self.state.borrow();match &state.storage.as_ref().unwrap().objects[source]{
                    StorageObject::Store{name,..}=>(name.clone(),false),StorageObject::Index{store,..}=>(store.clone(),true),_=>return Err(Throw::type_error("Invalid cursor source"))}};
                let outcome={let mut state=self.state.borrow_mut();let StorageObject::Transaction{value:Some(tx),..}=&mut state.storage.as_mut().unwrap().objects[transaction]else{return Err(Throw::type_error("Inactive transaction"))};
                    tx.store_mut(&store_name).and_then(|store|if let Some(value)=value{store.put(value,Some(record.primary_key.clone()),true).map(StoreResult::Key)}else{store.delete(&record.primary_key);Ok(StoreResult::None)})};
                let (request,wrapper)=self.storage_request(false,this,self.storage_wrapper(transaction)?)?;
                match outcome {Ok(StoreResult::Key(key))=>{let result=self.storage_from_key(&key)?;self.storage_schedule(StorageJob::RequestSuccess{request,result})?},
                    Ok(_)=>self.storage_schedule(StorageJob::RequestSuccess{request,result:JsValue::Undefined})?,
                    Err(error)=>{let error=self.storage_idb_error(error).0;self.storage_schedule(StorageJob::RequestError{request,error})?;}};
                Ok(wrapper)
            },
            _=>Err(Throw::type_error("Unknown IDBCursor member")),
        }
    }
    fn storage_tick(&mut self) -> Result<JsValue, Throw> {
        let job={let mut state=self.state.borrow_mut();let storage=state.storage.as_mut().unwrap();storage.tick_scheduled=false;storage.pending.pop_front()};
        if let Some(job)=job {
            match job {
                StorageJob::RequestSuccess{request,result}=>{
                    if let Some(StorageObject::Request{result:stored,ready,error,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *stored=result;*ready=true;*error=JsValue::Null;
                    }
                    self.storage_activate_request_transaction(request);
                    self.storage_dispatch(request,"success",None,None)?;
                },
                StorageJob::RequestError{request,error}=>{
                    if let Some(StorageObject::Request{error:stored,ready,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *stored=error;*ready=true;
                    }
                    self.storage_activate_request_transaction(request);
                    self.storage_dispatch(request,"error",None,None)?;
                },
                StorageJob::DeleteSuccess{request,old}=>{
                    if let Some(StorageObject::Request{result,error,ready,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *result=JsValue::Undefined;*error=JsValue::Null;*ready=true;
                    }
                    self.storage_dispatch(request,"success",Some(old),None)?;
                },
                StorageJob::OpenBlocked{request,old,new}=>{
                    let still_blocked={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();
                        storage.blocked_opens.iter().any(|(_,_,slot)|*slot==request)
                            ||storage.blocked_deletes.iter().any(|(_,_,slot)|*slot==request)};
                    if still_blocked {self.storage_dispatch(request,"blocked",Some(old),new)?;}
                },
                StorageJob::DatabaseVersionChange{connection,old,new}=>{
                    let slots={let state=self.state.borrow();state.storage.as_ref().unwrap().objects.iter().enumerate()
                        .filter_map(|(slot,object)|matches!(object,StorageObject::Database{connection:id,..} if *id==connection).then_some(slot)).collect::<Vec<_>>()};
                    for slot in slots {self.storage_dispatch(slot,"versionchange",Some(old),new)?;}
                },
                StorageJob::DispatchListener{slot,event,mut callbacks}=>{
                    self.storage_activate_request_transaction(slot);
                    if let Some(callback)=callbacks.pop_front(){
                        let target=self.storage_wrapper(slot)?;
                        binding_call(self.js,&callback,&target,&[event.clone()])?;
                    }
                    if !callbacks.is_empty(){self.state.borrow_mut().storage.as_mut().unwrap().pending.push_front(
                        StorageJob::DispatchListener{slot,event,callbacks});}
                },
                StorageJob::OpenUpgrade{request,connection,name,old,new}=>{
                    let database=self.storage_allocate(SI::IDBDatabase,StorageObject::Database{connection,name,handlers:HashMap::new(),listeners:Vec::new()})?;
                    let (txid,tx)={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,Vec::<String>::new(),Mode::VersionChange,Durability::Default)}
                        .map_err(|error|self.storage_idb_error(error))?;
                    let transaction=self.storage_allocate(SI::IDBTransaction,StorageObject::Transaction{connection,id:txid,value:Some(tx),mode:Mode::VersionChange,scope:Vec::new(),queued:false,active_for_requests:true,handlers:HashMap::new(),listeners:Vec::new()})?;
                    let transaction_slot=self.storage_slot(&transaction,SI::IDBTransaction)?;
                    if let Some(StorageObject::Request{result,transaction:stored,ready,error,..})=self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(request){
                        *result=database.clone();*stored=transaction;*ready=true;*error=JsValue::Null;
                    }
                    self.storage_dispatch(request,"upgradeneeded",Some(old),Some(new))?;
                    self.storage_schedule(StorageJob::TransactionComplete{transaction:transaction_slot})?;
                    self.storage_schedule(StorageJob::RequestSuccess{request,result:database})?;
                },
                StorageJob::TransactionComplete{transaction}=>{
                    let queued={let state=self.state.borrow();match state.storage.as_ref().unwrap().objects.get(transaction){
                        Some(StorageObject::Transaction{connection,mode,scope,queued:true,..})=>Some((*connection,*mode,scope.clone())),_=>None}};
                    if let Some((connection,mode,scope))=queued {
                        let begun={let state=self.state.borrow();state.storage.as_ref().unwrap().idb.borrow_mut().begin(connection,scope,mode,Durability::Default)};
                        match begun {
                            Ok((id,value))=>{let mut state=self.state.borrow_mut();if let StorageObject::Transaction{id:stored,value:stored_value,queued,..}=
                                &mut state.storage.as_mut().unwrap().objects[transaction]{*stored=id;*stored_value=Some(value);*queued=false;}},
                            Err(idb::Error::Blocked)=>{self.storage_schedule(StorageJob::TransactionComplete{transaction})?;return Ok(JsValue::Undefined)},
                            Err(error)=>return Err(self.storage_idb_error(error)),
                        }
                    }
                    let waiting={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();let wrapper=storage.wrappers.get(&transaction);
                        storage.pending.iter().any(|job|match job {StorageJob::RequestSuccess{request,..}|StorageJob::RequestError{request,..}=>{
                            matches!(storage.objects.get(*request),Some(StorageObject::Request{transaction:value,source,..}) if !matches!(source,JsValue::Null) && wrapper.is_some_and(|wrapper|self.js.binding_same_value(wrapper,value)))},_=>false})};
                    if waiting {self.storage_schedule(StorageJob::TransactionComplete{transaction})?;
                        return Ok(JsValue::Undefined)}
                    let (connection,id,value)={let mut state=self.state.borrow_mut();
                        match state.storage.as_mut().unwrap().objects.get_mut(transaction){Some(StorageObject::Transaction{connection,id,value,..})=>(*connection,*id,value.take()),_=>return Ok(JsValue::Undefined)}};
                    if let Some(value)=value {
                        let result=self.state.borrow().storage.as_ref().unwrap().idb.borrow_mut().finish(connection,id,value,true);
                        match result {Ok(())=>self.storage_dispatch(transaction,"complete",None,None)?,Err(_)=>self.storage_dispatch(transaction,"abort",None,None)?}
                    }
                },
                StorageJob::StorageEvent(_)=>{},
            }
        }
        let more={let state=self.state.borrow();let storage=state.storage.as_ref().unwrap();!storage.pending.is_empty()&&!storage.tick_scheduled};
        if more {
            let callback=self.native("storage task",0,NativeEntry::StorageTick,HostCallPolicy::CallOnly);
            self.state.borrow_mut().storage.as_mut().unwrap().tick_scheduled=true;
            self.js.schedule_timer(callback,0,Vec::new())?;
        }
        Ok(JsValue::Undefined)
    }
    fn storage_activate_request_transaction(&mut self,request:usize){
        let value={let state=self.state.borrow();match state.storage.as_ref().unwrap().objects.get(request){
            Some(StorageObject::Request{transaction,..})=>transaction.clone(),_=>return}};
        let Ok(slot)=self.storage_slot(&value,SI::IDBTransaction) else{return};
        if let Some(StorageObject::Transaction{value:Some(_),active_for_requests,..})=
            self.state.borrow_mut().storage.as_mut().unwrap().objects.get_mut(slot){*active_for_requests=true;}
    }
    fn storage_dispatch(&mut self,slot:usize,name:&str,old:Option<u64>,new:Option<u64>)->Result<(),Throw>{
        let (handler,listeners)={let state=self.state.borrow();let object=&state.storage.as_ref().unwrap().objects[slot];
            let (handlers,listeners)=match object {
                StorageObject::Request{handlers,listeners,..}|StorageObject::Transaction{handlers,listeners,..}|StorageObject::Database{handlers,listeners,..}=>(handlers,listeners),
                _=>return Ok(()),
            };
            (handlers.get(name).cloned(),listeners.iter().filter(|(kind,_)|kind==name).map(|(_,callback)|callback.clone()).collect::<Vec<_>>())
        };
        let event=if let Some(old)=old {self.storage_version_event(name,old,new)?} else {let event=self.js.binding_create_object(None)?;
            self.define_data_property(&event,"type",JsValue::str(name),true,true,true)?;event};
        let target=self.storage_wrapper(slot)?;
        self.define_data_property(&event,"target",target.clone(),true,true,true)?;
        let mut callbacks=handler.into_iter().chain(listeners).filter(|callback|self.js.binding_is_callable(callback)).collect::<VecDeque<_>>();
        if let Some(callback)=callbacks.pop_front(){
            binding_call(self.js,&callback,&target,&[event.clone()])?;
        }
        if !callbacks.is_empty(){
            self.state.borrow_mut().storage.as_mut().unwrap().pending.push_front(
                StorageJob::DispatchListener{slot,event,callbacks});
        }
        Ok(())
    }
    fn storage_version_event(&mut self,name:&str,old:u64,new:Option<u64>)->Result<JsValue,Throw>{
        let prototype=self.state.borrow().storage_classes.get(&SI::IDBVersionChangeEvent)
            .ok_or_else(||Throw::type_error("IDBVersionChangeEvent is not installed"))?.0.clone();
        let event=self.js.binding_create_object(Some(&prototype))?;
        self.define_data_property(&event,"type",JsValue::str(name),true,true,true)?;
        self.define_data_property(&event,"oldVersion",JsValue::num(old as f64),true,true,true)?;
        self.define_data_property(&event,"newVersion",new.map_or(JsValue::Null,|version|JsValue::num(version as f64)),true,true,true)?;
        Ok(event)
    }
    fn storage_to_key(&mut self,value:&JsValue)->Result<Key,Throw>{
        match value {
            JsValue::Num(number)=>Key::number(*number).map_err(|error|self.storage_idb_error(error)),
            JsValue::Str(string)=>Ok(Key::String(string.to_utf8_lossy())),
            JsValue::Object(object) if matches!(object.borrow().get_own("[[DateValue]]"),Some(JsValue::Num(_)))=>{
                let Some(JsValue::Num(value))=object.borrow().get_own("[[DateValue]]") else{unreachable!()};
                Key::date(value).map_err(|error|self.storage_idb_error(error))
            },
            JsValue::Object(object) if matches!(object.borrow().exotic,Exotic::Array)=>{
                let length=self.js.binding_get_property(value,"length")?;
                let count=self.js.binding_to_number(&length)? as usize;
                if count>100_000{return Err(self.storage_dom_error("DataError","Key is too long"))}
                let mut items=Vec::new();for index in 0..count {let item=self.js.binding_get_property(value,&index.to_string())?;items.push(self.storage_to_key(&item)?)}
                Ok(Key::Array(items))
            },
            JsValue::Object(_) if self.storage_binary_bytes(value)?.is_some()=>Ok(Key::Binary(self.storage_binary_bytes(value)?.unwrap())),
            _=>Err(self.storage_dom_error("DataError","Invalid IndexedDB key")),
        }
    }
    fn storage_binary_bytes(&mut self,value:&JsValue)->Result<Option<Vec<u8>>,Throw>{
        let JsValue::Object(_)=value else{return Ok(None)};
        let constructor=self.js.binding_get_property(value,"constructor")?;
        let name=self.js.binding_get_property(&constructor,"name")?;
        let JsValue::Str(name)=name else{return Ok(None)};
        let name=name.to_utf8_lossy();
        if name!="ArrayBuffer"&&!name.ends_with("Array")&&name!="DataView"{return Ok(None)}
        let input=if name=="ArrayBuffer"{value.clone()}else{self.js.binding_get_property(value,"buffer")?};
        let offset=if name=="ArrayBuffer"{0}else{let value=self.js.binding_get_property(value,"byteOffset")?;self.js.binding_to_number(&value)? as usize};
        let length=self.js.binding_get_property(value,"byteLength")?;
        let length=self.js.binding_to_number(&length)? as usize;
        if length>10_000_000{return Err(self.storage_dom_error("DataError","Binary key is too long"))}
        let global=self.js.binding_global_object();let constructor=self.js.binding_get_property(&global,"Uint8Array")?;
        let view=self.js.binding_construct(&constructor,vec![input,JsValue::num(offset as f64),JsValue::num(length as f64)])?;
        let mut bytes=Vec::with_capacity(length);
        for index in 0..length {let item=self.js.binding_get_property(&view,&index.to_string())?;bytes.push(self.js.binding_to_number(&item)? as u8)}
        Ok(Some(bytes))
    }
    fn storage_from_key(&mut self,key:&Key)->Result<JsValue,Throw>{
        match key {Key::Number(value)|Key::Date(value)=>Ok(JsValue::num(*value)),
            Key::String(value)=>Ok(JsValue::str(value)),Key::Binary(value)=>self.js.binding_create_array_buffer(value.clone()),
            Key::Array(items)=>{let mut values=Vec::new();for item in items {values.push(self.storage_from_key(item)?)}self.js.binding_create_array(values)}}
    }
    fn storage_clone_value(&mut self,value:&JsValue,seen:&mut Vec<usize>)->Result<StructuredValue,Throw>{
        match value {
            JsValue::Undefined=>Ok(StructuredValue::Undefined),
            JsValue::Null=>Ok(StructuredValue::Null),
            JsValue::Bool(value)=>Ok(StructuredValue::Bool(*value)),JsValue::Num(value)=>Ok(StructuredValue::Number(*value)),
            JsValue::Str(value)=>Ok(StructuredValue::String(value.to_utf8_lossy())),
            JsValue::Object(object) if matches!(object.borrow().get_own("[[DateValue]]"),Some(JsValue::Num(_)))=>{
                let Some(JsValue::Num(value))=object.borrow().get_own("[[DateValue]]") else{unreachable!()};Ok(StructuredValue::Date(value))
            },
            JsValue::Object(_) if self.storage_binary_bytes(value)?.is_some()=>Ok(StructuredValue::Binary(self.storage_binary_bytes(value)?.unwrap())),
            JsValue::Object(object)=>{
                let pointer=Rc::as_ptr(&object.0) as usize;
                if object.borrow().func.is_some()||!matches!(object.borrow().exotic,Exotic::Ordinary|Exotic::Array)||seen.contains(&pointer){
                    return Err(self.storage_dom_error("DataCloneError","Value cannot be cloned"));
                }
                seen.push(pointer);
                let result=if matches!(object.borrow().exotic,Exotic::Array) {
                    let length=self.js.binding_get_property(value,"length")?;let count=self.js.binding_to_number(&length)? as usize;
                    if count>100_000{return Err(self.storage_dom_error("DataCloneError","Array is too long"))}
                    let mut items=Vec::new();for index in 0..count {let item=self.js.binding_get_property(value,&index.to_string())?;items.push(self.storage_clone_value(&item,seen)?)}
                    StructuredValue::Array(items)
                }else{
                    let names=self.js.binding_own_property_keys(value)?;let mut map=BTreeMap::new();
                    for name in names {let item=self.js.binding_get_property(value,&name)?;map.insert(name,self.storage_clone_value(&item,seen)?);}
                    StructuredValue::Object(map)
                };seen.pop();Ok(result)
            },
            _=>Err(self.storage_dom_error("DataCloneError","Value cannot be cloned")),
        }
    }
    fn storage_from_value(&mut self,value:&StructuredValue)->Result<JsValue,Throw>{
        match value {StructuredValue::Undefined=>Ok(JsValue::Undefined),StructuredValue::Null=>Ok(JsValue::Null),StructuredValue::Bool(value)=>Ok(JsValue::Bool(*value)),
            StructuredValue::Number(value)=>Ok(JsValue::num(*value)),
            StructuredValue::Date(value)=>{let global=self.js.binding_global_object();let constructor=self.js.binding_get_property(&global,"Date")?;self.js.binding_construct(&constructor,vec![JsValue::num(*value)])},
            StructuredValue::String(value)=>Ok(JsValue::str(value)),StructuredValue::Binary(value)=>self.js.binding_create_array_buffer(value.clone()),
            StructuredValue::Array(items)=>{let mut values=Vec::new();for item in items{values.push(self.storage_from_value(item)?)}self.js.binding_create_array(values)},
            StructuredValue::Object(items)=>{let object=self.js.binding_create_object(None)?;
                for (name,value) in items {let item=self.storage_from_value(value)?;self.js.binding_set_property(&object,name,item)?;}Ok(object)},
        }
    }
}

#[cfg(test)]
mod storage_binding_tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};
    use amazing_js::value::JsValue;
    use crate::{dom_host::BrowserDomHost, js_scheduler::DriverScheduler};

    fn scheduler() -> DriverScheduler {
        let host=Rc::new(RefCell::new(BrowserDomHost{dom:amazing_html::parse("<body></body>"),native_forwarded:false}));
        let mut scheduler=DriverScheduler::new(host,crate::js_scheduler::JS_CAPACITY).unwrap();
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        scheduler.set_location_url(format!("https://storage-{nonce}.example.test/"));
        scheduler
    }

    #[test]
    fn web_storage_named_properties_and_methods_share_ordered_area() {
        let mut scheduler=scheduler();
        for script in ["localStorage.clear()","localStorage.setItem('first','one')","localStorage.second='two'"] {
            scheduler.run_script(script).unwrap_or_else(|error|panic!("{script}: {}",error.describe()));
        }
        let value=scheduler.run_script("localStorage.first==='one' && localStorage.getItem('second')==='two' && localStorage.length===2 && localStorage.key(0)==='first' && localStorage.key(1)==='second'").unwrap_or_else(|error|panic!("read: {}",error.describe()));
        assert_eq!(value,JsValue::Bool(true));
    }

    #[test]
    fn local_storage_changes_are_visible_in_another_document() {
        let mut first=scheduler();let mut second=scheduler();
        let origin=format!("https://shared-{}.example.test/",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        first.set_location_url(&origin);second.set_location_url(&origin);
        first.run_script("localStorage.setItem('shared','value')").unwrap();
        assert_eq!(second.run_script("localStorage.getItem('shared')").unwrap(),JsValue::str("value"));
    }

    #[test]
    fn indexeddb_upgrade_request_commits_then_reads_record() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var phase='start'; var request=indexedDB.open('integration',1); request.onupgradeneeded=function(){ phase='upgrade'; var db=request.result; db.createObjectStore('items'); }; request.onsuccess=function(){ phase='success'; try { var tx=request.result.transaction('items','readwrite'); phase='transaction'; var store=tx.objectStore('items'); phase='store'; var put=store.put({label:'stored'},7); phase='put-request'; put.onsuccess=function(){ result='put'; }; tx.oncomplete=function(){ phase='complete'; var read=request.result.transaction('items'); var get=read.objectStore('items').get(7); get.onsuccess=function(){ result=get.result.label; }; }; } catch(e) {result=e.name+':'+e.message;} }; request.onerror=function(){result='error';};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap_or_else(|error|{
                let phase=scheduler.run_script("phase+':'+result").unwrap();
                let details=match error.0 {JsValue::Object(object)=>{let object=object.borrow();format!("{:?} {:?}",object.get_own("name"),object.get_own("message"))},other=>format!("{other:?}")};
                panic!("pump at {phase:?}: {details}")
            });
            if scheduler.run_script("result").unwrap()==JsValue::str("stored") {break}
        }
        assert_eq!(scheduler.run_script("result+':'+phase+':'+request.readyState").unwrap(),JsValue::str("stored:complete:done"));
    }

    #[test]
    fn indexeddb_compares_binary_date_and_array_keys() {
        let mut scheduler=scheduler();
        for (script, expected) in [
            ("indexedDB.cmp([0], new Uint8Array([0]))", 1.0),
            ("indexedDB.cmp(new Uint8Array([0]), '0')", 1.0),
            ("indexedDB.cmp('', new Date(0))", 1.0),
            ("indexedDB.cmp(new Date(0), 0)", 1.0),
            ("indexedDB.cmp(new Int8Array([-1]), new Uint8Array([0]))", 1.0),
        ] {
            assert_eq!(scheduler.run_script(script).unwrap_or_else(|e| panic!("{script}: {}",e.describe())),JsValue::num(expected));
        }
    }

    #[test]
    fn indexeddb_preserves_undefined_through_commit() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var rq=indexedDB.open('undefined-value',1); rq.onupgradeneeded=function(){rq.result.createObjectStore('items')}; rq.onsuccess=function(){var tx=rq.result.transaction('items','readwrite'); tx.objectStore('items').put(undefined,1); tx.oncomplete=function(){var read=rq.result.transaction('items'); var get=read.objectStore('items').get(1); get.onsuccess=function(){result=get.result===undefined?'undefined':'wrong'};};};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()==JsValue::str("undefined") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("undefined"));
    }

    #[test]
    fn indexeddb_compound_key_path_round_trips() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var rq=indexedDB.open('compound',1); rq.onupgradeneeded=function(){var store=rq.result.createObjectStore('items',{keyPath:['group','id']}); result=store.keyPath.join(','); store.put({group:'a',id:2,value:'found'});}; rq.onsuccess=function(){var get=rq.result.transaction('items').objectStore('items').get(['a',2]); get.onsuccess=function(){result+=':'+get.result.value};};").unwrap();
        for step in 1..20 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()==JsValue::str("group,id:found") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("group,id:found"));
    }

    #[test]
    fn indexeddb_blocked_upgrade_resumes_after_close() {
        let mut scheduler=scheduler();
        scheduler.run_script("var result='pending'; var first=indexedDB.open('upgrade',1); first.onupgradeneeded=function(){first.result.createObjectStore('old')}; first.onsuccess=function(){var db=first.result; db.onversionchange=function(e){result='version:'+e.oldVersion+':'+e.newVersion;db.close()}; var second=indexedDB.open('upgrade',2); second.onupgradeneeded=function(){second.result.createObjectStore('new')}; second.onsuccess=function(){result+=':ready:'+second.result.version};};").unwrap();
        for step in 1..30 {
            scheduler.pump_at(Duration::from_millis(step*10),false).unwrap();
            if scheduler.run_script("result").unwrap()==JsValue::str("version:1:2:ready:2") {break}
        }
        assert_eq!(scheduler.run_script("result").unwrap(),JsValue::str("version:1:2:ready:2"));
    }
}

// Canvas registration is deliberately local to this adapter: the shared
// interface tables remain owned by dom-bindings.
impl RuntimeBindingHost<'_> {
    fn canvas_reset(&mut self, node: NodeIdx) {
        let (width, height) = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
        let mut state = self.state.borrow_mut();
        if let Some(slot) = state.canvas_slots.get_mut(&node) {
            if width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216 {
                slot.reset(width, height);
            } else { slot.reset(0, 0); }
        }
        state.canvas_style_values.retain(|(key, _), _| *key != node);
        state.canvas_dirty.insert(node);
    }
    fn install_canvas(&mut self) -> Result<(), Throw> {
        let element = self.prototype(Interface::HTMLCanvasElement)
            .ok_or_else(|| Throw::type_error("HTMLCanvasElement prototype missing"))?;
        self.install_canvas_members(&element, CanvasTarget::Element, canvas_interfaces::ELEMENT)?;
        for (name, target, members, length) in [
            ("CanvasRenderingContext2D", CanvasTarget::Context, canvas_interfaces::CONTEXT, 0),
            ("Path2D", CanvasTarget::Path, canvas_interfaces::PATH, 0),
            ("CanvasGradient", CanvasTarget::Gradient, canvas_interfaces::GRADIENT, 0),
            ("CanvasPattern", CanvasTarget::Pattern, canvas_interfaces::PATTERN, 0),
            ("ImageData", CanvasTarget::ImageData, canvas_interfaces::IMAGE_DATA, 2),
            ("TextMetrics", CanvasTarget::TextMetrics, canvas_interfaces::TEXT_METRICS, 0),
        ] {
            let prototype = self.js.binding_create_object(Some(&self.js.binding_object_prototype()))?;
            self.install_canvas_members(&prototype, target, members)?;
            let constructor = self.native(name, length, NativeEntry::Canvas(target,
                Member { name, kind: CanvasMemberKind::Constructor(length) }), HostCallPolicy::ConstructOnly);
            self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
            self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
            let global = self.js.binding_global_object();
            self.define_data_property(&global, name, constructor, true, false, true)?;
        }
        Ok(())
    }

    fn install_canvas_members(&mut self, prototype: &JsValue, target: CanvasTarget, members: &[Member]) -> Result<(), Throw> {
        let mut getters = HashMap::<&'static str, JsValue>::new();
        for member in members {
            let kind = member.kind;
            let name = member.name;
            let length = match kind { CanvasMemberKind::Method(n) | CanvasMemberKind::Constructor(n) => n, _ => 0 };
            let function = self.native(name, length, NativeEntry::Canvas(target, *member), HostCallPolicy::CallOnly);
            match kind {
                CanvasMemberKind::Method(_) => self.define_data_property(prototype, name, function, true, false, true)?,
                CanvasMemberKind::Getter => {
                    self.define_accessor_property(prototype, name, Some(function.clone()), None, true, true)?;
                    getters.insert(name, function);
                }
                CanvasMemberKind::Setter => {
                    let getter = getters.get(name).cloned();
                    self.define_accessor_property(prototype, name, getter, Some(function), true, true)?;
                }
                CanvasMemberKind::Constructor(_) => unreachable!(),
            }
        }
        Ok(())
    }

    fn canvas_object_index(&self, value: &JsValue) -> Option<usize> {
        self.state.borrow().canvas_objects.iter().position(|(candidate, _)|
            self.js.binding_same_value(candidate, value))
    }
    fn canvas_new_object(&mut self, name: &str, payload: CanvasObject) -> Result<JsValue, Throw> {
        let global = self.js.binding_global_object();
        let constructor = self.js.binding_get_property(&global, name)?;
        let prototype = self.js.binding_get_property(&constructor, "prototype")?;
        let object = self.js.binding_create_object(Some(&prototype))?;
        self.materialize_accessors(&object);
        self.state.borrow_mut().canvas_objects.push((object.clone(), payload));
        Ok(object)
    }
    fn canvas_numbers(&mut self, args: &[JsValue], count: usize) -> Result<Option<Vec<f64>>, Throw> {
        if args.len() < count { return Err(Throw::type_error("Not enough canvas arguments")); }
        let mut values = Vec::with_capacity(count);
        for arg in &args[..count] { values.push(self.js.binding_to_number(arg)?); }
        Ok(values.iter().all(|v| v.is_finite()).then_some(values))
    }
    fn canvas_string(&mut self, arg: Option<&JsValue>) -> Result<String, Throw> {
        let value = arg.cloned().unwrap_or(JsValue::Undefined);
        let units = self.js.binding_to_dom_string(&value)?;
        Ok(String::from_utf16_lossy(&units))
    }
    fn canvas_dimension(&self, node: NodeIdx, name: &str) -> u32 {
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        let default = if name == "width" { 300 } else { 150 };
        host.dom.get_attribute_ns(node, None, name)
            .map(|s| canvas_interfaces::parse_dimension(s, default)).unwrap_or(default)
    }
    fn canvas_element(&self, value: &JsValue) -> Result<NodeIdx, Throw> {
        let node = self.node_brand(value).ok_or_else(|| Throw::type_error("Expected HTMLCanvasElement"))?;
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        if host.dom.namespace(node) != Some(Namespace::Html) || host.dom.name(node) != Some("canvas") {
            return Err(Throw::type_error("Expected HTMLCanvasElement"));
        }
        Ok(node)
    }
    fn canvas_context_node(&self, value: &JsValue) -> Result<NodeIdx, Throw> {
        let index = self.canvas_object_index(value).ok_or_else(|| Throw::type_error("Expected CanvasRenderingContext2D"))?;
        match &self.state.borrow().canvas_objects[index].1 {
            CanvasObject::Context(node, _) => Ok(*node),
            _ => Err(Throw::type_error("Expected CanvasRenderingContext2D")),
        }
    }
    fn canvas_exception(&mut self, name: &str) -> Throw {
        let global = self.js.binding_global_object();
        let constructor = self.js.binding_get_property(&global, "DOMException");
        if let Ok(constructor) = constructor {
            if let Ok(value) = self.js.binding_construct(&constructor,
                vec![JsValue::str(name), JsValue::str(name)]) { return self.js.binding_throw(value); }
        }
        Throw::type_error(name)
    }
    fn canvas_image_data_object(&mut self, image: CanvasImageData) -> Result<JsValue, Throw> {
        let global = self.js.binding_global_object();
        let ctor = self.js.binding_get_property(&global, "Uint8ClampedArray")?;
        let data = self.js.binding_construct(&ctor, vec![JsValue::num(image.data.len() as f64)])?;
        for (i, byte) in image.data.iter().enumerate() {
            self.js.binding_set_property(&data, &i.to_string(), JsValue::num(f64::from(*byte)))?;
        }
        self.canvas_new_object("ImageData", CanvasObject::ImageData(image.width, image.height, data))
    }
    fn canvas_read_image_data(&mut self, value: &JsValue) -> Result<CanvasImageData, Throw> {
        let index = self.canvas_object_index(value).ok_or_else(|| Throw::type_error("Expected ImageData"))?;
        let (width, height, data) = match &self.state.borrow().canvas_objects[index].1 {
            CanvasObject::ImageData(w, h, data) => (*w, *h, data.clone()),
            _ => return Err(Throw::type_error("Expected ImageData")),
        };
        let length = width as usize * height as usize * 4;
        let mut pixels = Vec::with_capacity(length);
        for i in 0..length {
            let byte = self.js.binding_get_property(&data, &i.to_string())?;
            pixels.push(self.js.binding_to_number(&byte)? as u8);
        }
        CanvasImageData::from_rgba(width, height, pixels).map_err(|_| self.canvas_exception("InvalidStateError"))
    }
    fn invoke_canvas(&mut self, target: CanvasTarget, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if matches!(member.kind, CanvasMemberKind::Constructor(_)) {
            return self.canvas_construct(target, args);
        }
        match target {
            CanvasTarget::Element => self.canvas_element_call(member, receiver, args),
            CanvasTarget::Context => self.canvas_context_call(member, receiver, args),
            CanvasTarget::Path => self.canvas_path_call(member, receiver, args),
            CanvasTarget::Gradient => self.canvas_gradient_call(member, receiver, args),
            CanvasTarget::Pattern => self.canvas_pattern_call(member, receiver, args),
            CanvasTarget::ImageData => self.canvas_image_data_call(member, receiver),
            CanvasTarget::TextMetrics => self.canvas_metrics_call(member, receiver),
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_construct(&mut self, target: CanvasTarget, args: &[JsValue]) -> Result<JsValue, Throw> {
        match target {
            CanvasTarget::Path => {
                let path = if let Some(first) = args.first() {
                    if let Some(index) = self.canvas_object_index(first) {
                        if let CanvasObject::Path(path) = &self.state.borrow().canvas_objects[index].1 {
                            path.clone()
                        } else { Path2D::new() }
                    } else {
                        let data = self.canvas_string(Some(first))?;
                        Path2D::from_svg(&data).map_err(|_| self.canvas_exception("SyntaxError"))?
                    }
                } else { Path2D::new() };
                self.canvas_new_object("Path2D", CanvasObject::Path(path))
            }
            CanvasTarget::ImageData => {
                if let Some(JsValue::Object(object)) = args.first() {
                    if !matches!(object.borrow().exotic, Exotic::TypedArray) {
                        return Err(Throw::type_error("ImageData needs Uint8ClampedArray"));
                    }
                    let global = self.js.binding_global_object();
                    let expected = self.js.binding_get_property(&global, "Uint8ClampedArray")?;
                    let actual = self.js.binding_get_property(&args[0], "constructor")?;
                    if !self.js.binding_same_value(&expected, &actual) {
                        return Err(Throw::type_error("ImageData needs Uint8ClampedArray"));
                    }
                    let width_arg = args.get(1).ok_or_else(|| Throw::type_error("Missing ImageData width"))?;
                    let width = self.js.binding_to_number(width_arg)? as u32;
                    let data = args[0].clone();
                    let length_value = self.js.binding_get_property(&data, "length")?;
                    let length = self.js.binding_to_number(&length_value)? as usize;
                    let height = if let Some(value) = args.get(2) {
                        self.js.binding_to_number(value)? as u32
                    } else if width > 0 { (length / 4 / width as usize) as u32 } else { 0 };
                    if width == 0 || height == 0 || width as u64 * height as u64 > 16_777_216 ||
                        length != width as usize * height as usize * 4 {
                        return Err(self.canvas_exception("IndexSizeError"));
                    }
                    return self.canvas_new_object("ImageData", CanvasObject::ImageData(width, height, data));
                }
                let Some(values) = self.canvas_numbers(args, 2)? else { return Err(Throw::type_error("Non-finite ImageData dimensions")); };
                let (width, height) = (values[0] as u32, values[1] as u32);
                if width == 0 || height == 0 || width > 8192 || height > 8192 || width as u64 * height as u64 > 16_777_216 {
                    return Err(self.canvas_exception("IndexSizeError"));
                }
                let image = CanvasImageData::new(width, height).map_err(|_| self.canvas_exception("IndexSizeError"))?;
                self.canvas_image_data_object(image)
            }
            _ => Err(Throw::type_error("Illegal constructor")),
        }
    }
    fn canvas_element_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let node = self.canvas_element(&receiver)?;
        let name = member.name;
        match member.kind {
            CanvasMemberKind::Getter if name == "width" || name == "height" =>
                Ok(JsValue::num(self.canvas_dimension(node, name) as f64)),
            CanvasMemberKind::Setter if name == "width" || name == "height" => {
                let value = self.js.binding_to_number(args.first().unwrap_or(&JsValue::Undefined))?;
                let width = if value.is_finite() { value.rem_euclid(4_294_967_296.) as u32 } else { 0 };
                let host = self.state.borrow().host.clone();
                host.borrow_mut().dom.set_attribute(node, name, &width.to_string())
                    .map_err(|_| self.canvas_exception("InvalidStateError"))?;
                let dimensions = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
                if let Some(slot) = self.state.borrow_mut().canvas_slots.get_mut(&node) {
                    if dimensions.0 <= 8192 && dimensions.1 <= 8192 &&
                        u64::from(dimensions.0) * u64::from(dimensions.1) <= 16_777_216 {
                        slot.reset(dimensions.0, dimensions.1);
                    } else { slot.reset(0, 0); }
                }
                self.state.borrow_mut().canvas_style_values.retain(|(key, _), _| *key != node);
                self.state.borrow_mut().canvas_dirty.insert(node);
                Ok(JsValue::Undefined)
            }
            CanvasMemberKind::Method(_) if name == "getContext" => {
                if args.is_empty() { return Err(Throw::type_error("getContext requires a context id")); }
                let id = self.canvas_string(args.first())?;
                if id != "2d" { return Ok(JsValue::Null); }
                if let Some((object, _)) = self.state.borrow().canvas_objects.iter().find(|(_, payload)|
                    matches!(payload, CanvasObject::Context(key, _) if *key == node)) {
                    return Ok(object.clone());
                }
                let width = self.canvas_dimension(node, "width");
                let height = self.canvas_dimension(node, "height");
                if width > 8192 || height > 8192 || width as u64 * height as u64 > 16_777_216 { return Ok(JsValue::Null); }
                self.state.borrow_mut().canvas_slots.insert(node, CanvasSlot::new(node, width, height));
                self.state.borrow_mut().canvas_dirty.insert(node);
                self.canvas_new_object("CanvasRenderingContext2D", CanvasObject::Context(node, receiver))
            }
            CanvasMemberKind::Method(_) if name == "toDataURL" => {
                let media_type = if args.is_empty() { "image/png".to_string() } else { self.canvas_string(args.first())?.to_ascii_lowercase() };
                if !self.state.borrow().canvas_slots.get(&node).is_none_or(|slot| slot.origin_clean) {
                    return Err(self.canvas_exception("SecurityError"));
                }
                let dimensions = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
                if dimensions.0 == 0 || dimensions.1 == 0 || dimensions.0 > 8192 || dimensions.1 > 8192 ||
                    u64::from(dimensions.0) * u64::from(dimensions.1) > 16_777_216 {
                    return Ok(JsValue::str("data:,"));
                }
                let pixels = self.state.borrow().canvas_slots.get(&node)
                    .map(|slot| slot.context.surface.pixels.clone());
                let image = if let Some(pixels) = pixels {
                    CanvasImageData::from_rgba(dimensions.0, dimensions.1, pixels)
                } else { CanvasImageData::new(dimensions.0, dimensions.1) }
                    .map_err(|_| self.canvas_exception("InvalidStateError"))?;
                let _ = media_type; // Unrecognized encodings fall back to PNG per HTML.
                Ok(JsValue::str(format!("data:image/png;base64,{}", canvas_base64(&canvas::encode_png(&image)))))
            }
            CanvasMemberKind::Method(_) if name == "toBlob" => {
                let callback = args.first().ok_or_else(|| Throw::type_error("toBlob needs a callback"))?;
                if !self.js.binding_is_callable(callback) { return Err(Throw::type_error("toBlob callback is not callable")); }
                if !self.state.borrow().canvas_slots.get(&node).is_none_or(|slot| slot.origin_clean) {
                    return Err(self.canvas_exception("SecurityError"));
                }
                let (width, height) = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
                let argument = if width == 0 || height == 0 || width > 8192 || height > 8192 ||
                    u64::from(width) * u64::from(height) > 16_777_216 { JsValue::Null } else {
                    let pixels = self.state.borrow().canvas_slots.get(&node)
                        .map(|slot| slot.context.surface.pixels.clone())
                        .unwrap_or_else(|| vec![0; width as usize * height as usize * 4]);
                    let image = CanvasImageData::from_rgba(width, height, pixels)
                        .map_err(|_| self.canvas_exception("InvalidStateError"))?;
                    self.blob_new(Blob::from_bytes(canvas::encode_png(&image), "image/png"))?
                };
                self.js.binding_call(callback, JsValue::Undefined, vec![argument])?;
                Ok(JsValue::Undefined)
            }
            _ => Err(Throw::type_error("Unknown canvas element member")),
        }
    }
}

fn canvas_base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(DIGITS[(a >> 2) as usize] as char);
        out.push(DIGITS[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 { DIGITS[(((b & 15) << 2) | (c >> 6)) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { DIGITS[(c & 63) as usize] as char } else { '=' });
    }
    out
}

impl RuntimeBindingHost<'_> {
    fn canvas_refresh_styles(&mut self, node: NodeIdx) {
        for name in ["fillStyle", "strokeStyle"] {
            let value = self.state.borrow().canvas_style_values.get(&(node, name)).cloned();
            let Some(value) = value else { continue };
            let Some(index) = self.canvas_object_index(&value) else { continue };
            let (style, tainted) = match &self.state.borrow().canvas_objects[index].1 {
                CanvasObject::Gradient(g, kind) => (Some(match kind {
                    CanvasGradientKind::Linear => PaintStyle::LinearGradient(g.clone()),
                    CanvasGradientKind::Radial => PaintStyle::RadialGradient(g.clone()),
                    CanvasGradientKind::Conic => PaintStyle::ConicGradient(g.clone()),
                }), false),
                CanvasObject::Pattern(p, tainted) => (Some(PaintStyle::Pattern(p.clone())), *tainted),
                _ => (None, false),
            };
            let mut state = self.state.borrow_mut();
            if let Some(slot) = state.canvas_slots.get_mut(&node) {
                if let Some(style) = style {
                    if name == "fillStyle" { slot.context.state.fill_style = style; }
                    else { slot.context.state.stroke_style = style; }
                    if tainted { slot.origin_clean = false; }
                }
            }
        }
    }
    fn canvas_context_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let node = self.canvas_context_node(&receiver)?;
        let name = member.name;
        if name == "canvas" && member.kind == CanvasMemberKind::Setter { return Ok(JsValue::Undefined); }
        if name == "canvas" { return match &self.state.borrow().canvas_objects[self.canvas_object_index(&receiver).unwrap()].1 {
            CanvasObject::Context(_, element) => Ok(element.clone()), _ => unreachable!()
        }; }
        if matches!(member.kind, CanvasMemberKind::Getter | CanvasMemberKind::Setter) {
            return self.canvas_context_attribute(node, member, args);
        }
        if name == "isContextLost" { return Ok(JsValue::Bool(false)); }
        if name == "drawFocusIfNeeded" {
            if args.is_empty() { return Err(Throw::type_error("Missing focus target")); }
            return Ok(JsValue::Undefined);
        }
        if name == "getContextAttributes" {
            let object = self.js.binding_create_object(Some(&self.js.binding_object_prototype()))?;
            for (key, value) in [("alpha", JsValue::Bool(true)),
                ("desynchronized", JsValue::Bool(false)),
                ("colorSpace", JsValue::str("srgb")),
                ("willReadFrequently", JsValue::Bool(false))] {
                self.js.binding_define_data_property(&object, key, value, BindingPropertyFlags::METHOD)?;
            }
            return Ok(object);
        }
        if name == "createImageData" {
            if let Some(first) = args.first() {
                if let Some(index) = self.canvas_object_index(first) {
                    let dimensions = match &self.state.borrow().canvas_objects[index].1 {
                        CanvasObject::ImageData(w, h, _) => Some((*w, *h)), _ => None,
                    };
                    if let Some((w, h)) = dimensions {
                        let image = CanvasImageData::new(w, h).map_err(|_| self.canvas_exception("IndexSizeError"))?;
                        return self.canvas_image_data_object(image);
                    }
                }
            }
            let Some(v) = self.canvas_numbers(args, 2)? else { return Err(Throw::type_error("Non-finite ImageData dimensions")); };
            let (w, h) = (v[0].abs() as u32, v[1].abs() as u32);
            if w == 0 || h == 0 || w as u64 * h as u64 > 16_777_216 { return Err(self.canvas_exception("IndexSizeError")); }
            let image = CanvasImageData::new(w, h).map_err(|_| self.canvas_exception("IndexSizeError"))?;
            return self.canvas_image_data_object(image);
        }
        if name == "getImageData" {
            let Some(v) = self.canvas_numbers(args, 4)? else { return Err(Throw::type_error("Non-finite image data rectangle")); };
            if !self.state.borrow().canvas_slots.get(&node).is_some_and(|slot| slot.origin_clean) {
                return Err(self.canvas_exception("SecurityError"));
            }
            let image = {
                let state = self.state.borrow();
                state.canvas_slots.get(&node).unwrap().context.get_image_data_signed(v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32)
            }.map_err(|_| self.canvas_exception("IndexSizeError"))?;
            return self.canvas_image_data_object(image);
        }
        if name == "putImageData" {
            let image = self.canvas_read_image_data(args.first().ok_or_else(|| Throw::type_error("Missing ImageData"))?)?;
            let Some(v) = self.canvas_numbers(&args[1..], 2)? else { return Ok(JsValue::Undefined); };
            if args.len() >= 7 {
                let Some(dirty) = self.canvas_numbers(&args[3..], 4)? else { return Ok(JsValue::Undefined); };
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.put_image_data_dirty_signed(
                    &image, v[0] as i32, v[1] as i32,
                    dirty[0] as i32, dirty[1] as i32, dirty[2] as i32, dirty[3] as i32);
            } else {
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.put_image_data(&image, v[0] as i32, v[1] as i32);
            }
            self.state.borrow_mut().canvas_dirty.insert(node);
            return Ok(JsValue::Undefined);
        }
        if matches!(name, "createLinearGradient" | "createRadialGradient" | "createConicGradient") {
            let count = match name { "createLinearGradient" => 4, "createRadialGradient" => 6, _ => 3 };
            let Some(v) = self.canvas_numbers(args, count)? else { return Err(Throw::type_error("Non-finite gradient coordinate")); };
            let (gradient, kind) = match name {
                "createLinearGradient" => (canvas::Gradient::linear(v[0], v[1], v[2], v[3]), CanvasGradientKind::Linear),
                "createRadialGradient" => {
                    if v[2] < 0. || v[5] < 0. { return Err(self.canvas_exception("IndexSizeError")); }
                    (canvas::Gradient::radial(v[0], v[1], v[2], v[3], v[4], v[5]), CanvasGradientKind::Radial)
                }
                _ => (canvas::Gradient::conic(v[0], v[1], v[2]), CanvasGradientKind::Conic),
            };
            return self.canvas_new_object("CanvasGradient", CanvasObject::Gradient(gradient, kind));
        }
        if name == "createPattern" {
            let source = args.first().ok_or_else(|| Throw::type_error("Missing pattern source"))?;
            let source_node = self.canvas_element(source)?;
            let repetition = if args.get(1).is_none_or(|value| self.js.binding_is_nullish(value)) {
                String::new()
            } else { self.canvas_string(args.get(1))? };
            let repetition = match repetition.as_str() {
                "" | "repeat" => canvas::Repetition::Repeat,
                "repeat-x" => canvas::Repetition::RepeatX,
                "repeat-y" => canvas::Repetition::RepeatY,
                "no-repeat" => canvas::Repetition::NoRepeat,
                _ => return Err(self.canvas_exception("SyntaxError")),
            };
            let source = self.state.borrow().canvas_slots.get(&source_node).map(|slot|
                (slot.context.surface.width, slot.context.surface.height,
                    slot.context.surface.pixels.clone(), !slot.origin_clean));
            let source = source.or_else(|| {
                let (width, height) = (self.canvas_dimension(source_node, "width"), self.canvas_dimension(source_node, "height"));
                (width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216)
                    .then(|| (width, height, vec![0; width as usize * height as usize * 4], false))
            });
            let Some((width, height, pixels, tainted)) = source else { return Ok(JsValue::Null); };
            if width == 0 || height == 0 { return Ok(JsValue::Null); }
            let bitmap = canvas::Bitmap::new(width, height, pixels)
                .map_err(|_| self.canvas_exception("InvalidStateError"))?;
            return self.canvas_new_object("CanvasPattern", CanvasObject::Pattern(canvas::Pattern {
                bitmap, repetition, offset: canvas::Point::new(0., 0.),
            }, tainted));
        }
        if name == "drawImage" {
            return self.canvas_draw_image(node, args);
        }
        if matches!(name, "measureText" | "fillText" | "strokeText") {
            let text = self.canvas_string(args.first())?;
            let system = CanvasTextSystem(self.state.borrow().canvas_text_system.clone()
                .unwrap_or_else(|| Rc::new(amazing_text::FallbackTextSystem::new())));
            if name == "measureText" {
                let metrics_result = self.state.borrow().canvas_slots.get(&node).unwrap().context
                    .measure_text(&text, &system);
                let metrics = metrics_result.map_err(|_| self.canvas_exception("InvalidStateError"))?;
                return self.canvas_new_object("TextMetrics", CanvasObject::TextMetrics(metrics));
            }
            let Some(v) = self.canvas_numbers(&args[1..], 2)? else { return Ok(JsValue::Undefined); };
            self.canvas_refresh_styles(node);
            let max_width = if let Some(value) = args.get(3) { Some(self.js.binding_to_number(value)?) } else { None };
            let result = {
                let mut state = self.state.borrow_mut();
                let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
                if name == "fillText" { ctx.fill_text(&text, v[0], v[1], max_width, &system) }
                else { ctx.stroke_text(&text, v[0], v[1], max_width, &system) }
            };
            result.map_err(|_| self.canvas_exception("InvalidStateError"))?;
            self.state.borrow_mut().canvas_dirty.insert(node);
            return Ok(JsValue::Undefined);
        }
        if name == "fill" || name == "clip" || name == "stroke" {
            if name != "clip" { self.canvas_refresh_styles(node); }
            let path = args.first().and_then(|arg| self.canvas_object_index(arg)).and_then(|index|
                match &self.state.borrow().canvas_objects[index].1 { CanvasObject::Path(path) => Some(path.clone()), _ => None });
            let rule = if args.iter().any(|arg| self.js.binding_string_units(arg).is_some_and(|s| String::from_utf16_lossy(&s) == "evenodd")) {
                FillRule::EvenOdd
            } else { FillRule::NonZero };
            let mut state = self.state.borrow_mut();
            let context = &mut state.canvas_slots.get_mut(&node).unwrap().context;
            match (name, path) {
                ("fill", Some(path)) => context.fill_path(&path, rule),
                ("fill", None) => context.fill(rule),
                ("stroke", Some(path)) => context.stroke_path(&path),
                ("stroke", None) => context.stroke(),
                ("clip", Some(path)) => context.clip_path(&path, rule),
                ("clip", None) => context.clip(rule),
                _ => unreachable!(),
            }
            if name != "clip" { state.canvas_dirty.insert(node); }
            return Ok(JsValue::Undefined);
        }
        if name == "isPointInPath" || name == "isPointInStroke" {
            let (path, offset) = if let Some(first) = args.first() {
                if let Some(index) = self.canvas_object_index(first) {
                    if let CanvasObject::Path(path) = &self.state.borrow().canvas_objects[index].1 { (Some(path.clone()), 1) }
                    else { (None, 0) }
                } else { (None, 0) }
            } else { (None, 0) };
            let Some(v) = self.canvas_numbers(&args[offset..], 2)? else { return Ok(JsValue::Bool(false)); };
            let state = self.state.borrow();
            let context = &state.canvas_slots.get(&node).unwrap().context;
            let yes = match (name, path) {
                ("isPointInPath", Some(path)) => context.is_point_in_path2d(&path, v[0], v[1], FillRule::NonZero),
                ("isPointInPath", None) => context.is_point_in_path(v[0], v[1], FillRule::NonZero),
                (_, Some(path)) => context.is_point_in_stroke2d(&path, v[0], v[1]),
                (_, None) => context.is_point_in_stroke(v[0], v[1]),
            };
            return Ok(JsValue::Bool(yes));
        }
        if name == "getLineDash" {
            let dash = self.state.borrow().canvas_slots.get(&node).unwrap().context.state.line_dash.clone();
            return self.js.binding_create_array(dash.into_iter().map(JsValue::num).collect());
        }
        if name == "setLineDash" {
            let array = args.first().ok_or_else(|| Throw::type_error("Missing line dash"))?;
            let length_value = self.js.binding_get_property(array, "length")?;
            let length = self.js.binding_to_number(&length_value)? as usize;
            let mut dash = Vec::with_capacity(length.min(4096));
            for i in 0..length.min(4096) {
                let item = self.js.binding_get_property(array, &i.to_string())?;
                dash.push(self.js.binding_to_number(&item)?);
            }
            self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.set_line_dash(&dash);
            return Ok(JsValue::Undefined);
        }
        if name == "getTransform" {
            let matrix = self.state.borrow().canvas_slots.get(&node).unwrap().context.state.transform;
            let object = self.js.binding_create_object(Some(&self.js.binding_object_prototype()))?;
            for (field, value) in [("a", matrix.a), ("b", matrix.b), ("c", matrix.c),
                ("d", matrix.d), ("e", matrix.e), ("f", matrix.f)] {
                self.js.binding_define_data_property(&object, field, JsValue::num(value), BindingPropertyFlags::METHOD)?;
            }
            return Ok(object);
        }
        if name == "setTransform" && args.len() == 1 {
            let object = &args[0];
            let mut values = [1., 0., 0., 1., 0., 0.];
            for (index, field) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
                let value = self.js.binding_get_property(object, field)?;
                if !matches!(value, JsValue::Undefined) {
                    values[index] = self.js.binding_to_number(&value)?;
                }
            }
            let transform = Transform { a: values[0], b: values[1], c: values[2],
                d: values[3], e: values[4], f: values[5] };
            if transform.finite() {
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.set_transform(transform);
            }
            return Ok(JsValue::Undefined);
        }
        let count = match name {
            "scale" | "translate" | "moveTo" | "lineTo" => 2,
            "rotate" => 1,
            "transform" => 6,
            "setTransform" if args.is_empty() => 0,
            "setTransform" => 6,
            "clearRect" | "fillRect" | "strokeRect" | "rect" | "quadraticCurveTo" => 4,
            "bezierCurveTo" | "ellipse" => if name == "ellipse" { 7 } else { 6 },
            "arcTo" | "arc" => 5,
            "roundRect" => 4,
            _ => 0,
        };
        let Some(v) = self.canvas_numbers(args, count)? else { return Ok(JsValue::Undefined); };
        let radii = if name == "roundRect" { self.canvas_round_radii(args.get(4))? } else { [0.; 4] };
        if matches!(name, "fillRect" | "strokeRect") { self.canvas_refresh_styles(node); }
        if (name == "arc" && v[2] < 0.) || (name == "arcTo" && v[4] < 0.) ||
            (name == "ellipse" && (v[2] < 0. || v[3] < 0.)) {
            return Err(self.canvas_exception("IndexSizeError"));
        }
        let mut state = self.state.borrow_mut();
        let context = &mut state.canvas_slots.get_mut(&node).unwrap().context;
        match name {
            "save" => context.save(), "restore" => context.restore(),
            "reset" => { let (w, h) = (context.surface.width, context.surface.height); state.canvas_slots.get_mut(&node).unwrap().reset(w, h); state.canvas_style_values.retain(|(key, _), _| *key != node); },
            "scale" => context.scale(v[0], v[1]), "rotate" => context.rotate(v[0]),
            "translate" => context.translate(v[0], v[1]),
            "transform" => context.transform(Transform { a:v[0], b:v[1], c:v[2], d:v[3], e:v[4], f:v[5] }),
            "setTransform" => context.set_transform(if v.is_empty() { Transform::IDENTITY } else { Transform { a:v[0], b:v[1], c:v[2], d:v[3], e:v[4], f:v[5] } }),
            "resetTransform" => context.reset_transform(),
            "beginPath" => context.begin_path(), "closePath" => context.close_path(),
            "moveTo" => context.move_to(v[0], v[1]), "lineTo" => context.line_to(v[0], v[1]),
            "quadraticCurveTo" => context.quadratic_curve_to(v[0], v[1], v[2], v[3]),
            "bezierCurveTo" => context.bezier_curve_to(v[0], v[1], v[2], v[3], v[4], v[5]),
            "rect" => context.rect(v[0], v[1], v[2], v[3]),
            "arc" => context.arc(v[0], v[1], v[2], v[3], v[4], args.get(5).is_some_and(|v| self.js.binding_to_boolean(v))),
            "arcTo" => context.arc_to(v[0], v[1], v[2], v[3], v[4]),
            "ellipse" => context.ellipse(v[0], v[1], v[2], v[3], v[4], v[5], v[6], args.get(7).is_some_and(|v| self.js.binding_to_boolean(v))),
            "roundRect" => context.round_rect(v[0], v[1], v[2], v[3], radii),
            "clearRect" => context.clear_rect(v[0], v[1], v[2], v[3]),
            "fillRect" => context.fill_rect(v[0], v[1], v[2], v[3]),
            "strokeRect" => context.stroke_rect(v[0], v[1], v[2], v[3]),
            _ => return Err(Throw::type_error("Canvas operation is unsupported")),
        }
        if matches!(name, "clearRect" | "fillRect" | "strokeRect" | "reset") {
            state.canvas_dirty.insert(node);
        }
        Ok(JsValue::Undefined)
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_context_attribute(&mut self, node: NodeIdx, member: Member, args: &[JsValue]) -> Result<JsValue, Throw> {
        let name = member.name;
        if member.kind == CanvasMemberKind::Setter {
            let value = args.first().cloned().unwrap_or(JsValue::Undefined);
            if name == "fillStyle" || name == "strokeStyle" {
                let style = if let Some(index) = self.canvas_object_index(&value) {
                    match &self.state.borrow().canvas_objects[index].1 {
                        CanvasObject::Gradient(g, kind) => Some(match kind {
                            CanvasGradientKind::Linear => PaintStyle::LinearGradient(g.clone()),
                            CanvasGradientKind::Radial => PaintStyle::RadialGradient(g.clone()),
                            CanvasGradientKind::Conic => PaintStyle::ConicGradient(g.clone()),
                        }),
                        CanvasObject::Pattern(p, _) => Some(PaintStyle::Pattern(p.clone())),
                        _ => None,
                    }
                } else { PaintStyle::from_css_color(&self.canvas_string(Some(&value))?) };
                if let Some(style) = style {
                    let reflected = match &style {
                        PaintStyle::Color(color) => JsValue::str(canvas_color_text(*color)),
                        _ => value,
                    };
                    let mut state = self.state.borrow_mut();
                    let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
                    if name == "fillStyle" { ctx.state.fill_style = style; } else { ctx.state.stroke_style = style; }
                    state.canvas_style_values.insert((node, name), reflected);
                }
                return Ok(JsValue::Undefined);
            }
            if matches!(name, "lineCap" | "lineJoin" | "globalCompositeOperation" | "textAlign" | "textBaseline" | "filter" | "imageSmoothingQuality" | "direction" | "font" | "shadowColor" | "lang" | "letterSpacing" | "fontKerning" | "fontStretch" | "fontVariantCaps" | "textRendering" | "wordSpacing") {
                let value = self.canvas_string(Some(&value))?;
                let mut state = self.state.borrow_mut();
                let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
                let mut store = false;
                match name {
                    "lineCap" => if let Some(cap) = match value.as_str() { "butt" => Some(canvas::LineCap::Butt), "round" => Some(canvas::LineCap::Round), "square" => Some(canvas::LineCap::Square), _ => None } { ctx.state.line_cap = cap; },
                    "lineJoin" => if let Some(join) = match value.as_str() { "miter" => Some(canvas::LineJoin::Miter), "round" => Some(canvas::LineJoin::Round), "bevel" => Some(canvas::LineJoin::Bevel), _ => None } { ctx.state.line_join = join; },
                    "globalCompositeOperation" => if let Some(op) = canvas::CompositeOperation::parse(&value) { ctx.state.composite_operation = op; },
                    "textAlign" => if let Some(align) = match value.as_str() { "start" => Some(canvas::TextAlign::Start), "end" => Some(canvas::TextAlign::End), "left" => Some(canvas::TextAlign::Left), "right" => Some(canvas::TextAlign::Right), "center" => Some(canvas::TextAlign::Center), _ => None } { ctx.state.text_align = align; },
                    "textBaseline" => if let Some(base) = match value.as_str() { "top" => Some(canvas::TextBaseline::Top), "hanging" => Some(canvas::TextBaseline::Hanging), "middle" => Some(canvas::TextBaseline::Middle), "alphabetic" => Some(canvas::TextBaseline::Alphabetic), "ideographic" => Some(canvas::TextBaseline::Ideographic), "bottom" => Some(canvas::TextBaseline::Bottom), _ => None } { ctx.state.text_baseline = base; },
                    "filter" => { ctx.set_filter(&value); },
                    "shadowColor" => if let Some(PaintStyle::Color(color)) = PaintStyle::from_css_color(&value) {
                        ctx.state.shadow_color = color;
                        store = true;
                    },
                    "font" => {
                        if let Some(size) = value.split_whitespace().find_map(|part| part.strip_suffix("px").and_then(|s| s.parse::<f32>().ok())) {
                            if size.is_finite() && size > 0. {
                                ctx.state.text_style.size = size;
                                store = true;
                            }
                        }
                    },
                    "direction" => if matches!(value.as_str(), "ltr" | "rtl" | "inherit") {
                        ctx.state.text_style.direction = if value == "rtl" { amazing_text::Direction::RightToLeft }
                            else { amazing_text::Direction::LeftToRight };
                        store = true;
                    },
                    "imageSmoothingQuality" => store = matches!(value.as_str(), "low" | "medium" | "high"),
                    "fontKerning" => store = matches!(value.as_str(), "auto" | "normal" | "none"),
                    "fontStretch" => store = matches!(value.as_str(), "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed" | "normal" | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded"),
                    "fontVariantCaps" => store = matches!(value.as_str(), "normal" | "small-caps" | "all-small-caps" | "petite-caps" | "all-petite-caps" | "unicase" | "titling-caps"),
                    "textRendering" => store = matches!(value.as_str(), "auto" | "optimizeSpeed" | "optimizeLegibility" | "geometricPrecision"),
                    "lang" | "letterSpacing" | "wordSpacing" => store = true,
                    _ => unreachable!(),
                }
                if store { state.canvas_style_values.insert((node, name), JsValue::str(value)); }
                return Ok(JsValue::Undefined);
            }
            if name == "imageSmoothingEnabled" {
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.state.image_smoothing_enabled = self.js.binding_to_boolean(&value);
                return Ok(JsValue::Undefined);
            }
            let n = self.js.binding_to_number(&value)?;
            if !n.is_finite() { return Ok(JsValue::Undefined); }
            let mut state = self.state.borrow_mut();
            let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
            match name {
                "globalAlpha" => ctx.set_global_alpha(n),
                "lineWidth" => ctx.set_line_width(n),
                "miterLimit" if n > 0. => ctx.state.miter_limit = n,
                "lineDashOffset" => ctx.state.line_dash_offset = n,
                "shadowBlur" if n >= 0. => ctx.state.shadow_blur = n,
                "shadowOffsetX" => ctx.state.shadow_offset_x = n,
                "shadowOffsetY" => ctx.state.shadow_offset_y = n,
                _ => {},
            }
            return Ok(JsValue::Undefined);
        }
        let state = self.state.borrow();
        let ctx = &state.canvas_slots.get(&node).unwrap().context;
        let style = if name == "fillStyle" { Some(&ctx.state.fill_style) } else if name == "strokeStyle" { Some(&ctx.state.stroke_style) } else { None };
        if let Some(style) = style {
            if let Some(value) = state.canvas_style_values.get(&(node, name)) { return Ok(value.clone()); }
            return Ok(JsValue::str(match style { PaintStyle::Color(color) => canvas_color_text(*color), _ => String::new() }));
        }
        if let Some(value) = state.canvas_style_values.get(&(node, name)) {
            return Ok(value.clone());
        }
        Ok(match name {
            "globalAlpha" => JsValue::num(ctx.state.global_alpha),
            "globalCompositeOperation" => JsValue::str(canvas_composite_name(ctx.state.composite_operation)),
            "lineWidth" => JsValue::num(ctx.state.line_width),
            "lineCap" => JsValue::str(match ctx.state.line_cap { canvas::LineCap::Butt => "butt", canvas::LineCap::Round => "round", canvas::LineCap::Square => "square" }),
            "lineJoin" => JsValue::str(match ctx.state.line_join { canvas::LineJoin::Miter => "miter", canvas::LineJoin::Round => "round", canvas::LineJoin::Bevel => "bevel" }),
            "miterLimit" => JsValue::num(ctx.state.miter_limit),
            "lineDashOffset" => JsValue::num(ctx.state.line_dash_offset),
            "shadowBlur" => JsValue::num(ctx.state.shadow_blur),
            "shadowOffsetX" => JsValue::num(ctx.state.shadow_offset_x),
            "shadowOffsetY" => JsValue::num(ctx.state.shadow_offset_y),
            "imageSmoothingEnabled" => JsValue::Bool(ctx.state.image_smoothing_enabled),
            "imageSmoothingQuality" => JsValue::str("low"),
            "filter" => JsValue::str(ctx.state.filter.clone()),
            "font" => JsValue::str("10px sans-serif"),
            "lang" => JsValue::str("inherit"),
            "letterSpacing" | "wordSpacing" => JsValue::str("0px"),
            "fontKerning" | "textRendering" => JsValue::str("auto"),
            "fontStretch" | "fontVariantCaps" => JsValue::str("normal"),
            "direction" => JsValue::str("inherit"),
            "textAlign" => JsValue::str(match ctx.state.text_align { canvas::TextAlign::Start => "start", canvas::TextAlign::End => "end", canvas::TextAlign::Left => "left", canvas::TextAlign::Right => "right", canvas::TextAlign::Center => "center" }),
            "textBaseline" => JsValue::str(match ctx.state.text_baseline { canvas::TextBaseline::Top => "top", canvas::TextBaseline::Hanging => "hanging", canvas::TextBaseline::Middle => "middle", canvas::TextBaseline::Alphabetic => "alphabetic", canvas::TextBaseline::Ideographic => "ideographic", canvas::TextBaseline::Bottom => "bottom" }),
            "shadowColor" => JsValue::str("rgba(0, 0, 0, 0)"),
            _ => JsValue::Undefined,
        })
    }
}

fn canvas_composite_name(op: canvas::CompositeOperation) -> &'static str {
    use canvas::CompositeOperation as C;
    match op {
        C::SourceOver => "source-over", C::SourceIn => "source-in", C::SourceOut => "source-out", C::SourceAtop => "source-atop",
        C::DestinationOver => "destination-over", C::DestinationIn => "destination-in", C::DestinationOut => "destination-out", C::DestinationAtop => "destination-atop",
        C::Lighter => "lighter", C::Copy => "copy", C::Xor => "xor", C::Multiply => "multiply", C::Screen => "screen", C::Overlay => "overlay",
        C::Darken => "darken", C::Lighten => "lighten", C::ColorDodge => "color-dodge", C::ColorBurn => "color-burn", C::HardLight => "hard-light",
        C::SoftLight => "soft-light", C::Difference => "difference", C::Exclusion => "exclusion", C::Hue => "hue", C::Saturation => "saturation",
        C::Color => "color", C::Luminosity => "luminosity",
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_path_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected Path2D"))?;
        if !matches!(self.state.borrow().canvas_objects[index].1, CanvasObject::Path(_)) {
            return Err(Throw::type_error("Expected Path2D"));
        }
        if member.name == "addPath" {
            let source = args.first().and_then(|value| self.canvas_object_index(value))
                .and_then(|index| match &self.state.borrow().canvas_objects[index].1 { CanvasObject::Path(path) => Some(path.clone()), _ => None })
                .ok_or_else(|| Throw::type_error("Expected Path2D"))?;
            let mut matrix = Transform::IDENTITY;
            if let Some(object) = args.get(1) {
                for (name, slot) in [("a", &mut matrix.a), ("b", &mut matrix.b),
                    ("c", &mut matrix.c), ("d", &mut matrix.d),
                    ("e", &mut matrix.e), ("f", &mut matrix.f)] {
                    let value = self.js.binding_get_property(object, name)?;
                    if !matches!(value, JsValue::Undefined) { *slot = self.js.binding_to_number(&value)?; }
                }
            }
            if let CanvasObject::Path(path) = &mut self.state.borrow_mut().canvas_objects[index].1 {
                path.add_path(&source, matrix);
            }
            return Ok(JsValue::Undefined);
        }
        let count = match member.name {
            "closePath" => 0, "moveTo" | "lineTo" => 2,
            "quadraticCurveTo" | "rect" => 4, "bezierCurveTo" => 6,
            "arcTo" | "arc" => 5, "roundRect" => 4, "ellipse" => 7,
            _ => return Err(Throw::type_error("Unknown Path2D member")),
        };
        let Some(v) = self.canvas_numbers(args, count)? else { return Ok(JsValue::Undefined); };
        let radii = if member.name == "roundRect" { self.canvas_round_radii(args.get(4))? } else { [0.; 4] };
        if (member.name == "arc" && v[2] < 0.) || (member.name == "arcTo" && v[4] < 0.) ||
            (member.name == "ellipse" && (v[2] < 0. || v[3] < 0.)) {
            return Err(self.canvas_exception("IndexSizeError"));
        }
        let ccw = args.get(count).is_some_and(|value| self.js.binding_to_boolean(value));
        let mut state = self.state.borrow_mut();
        let CanvasObject::Path(path) = &mut state.canvas_objects[index].1 else { unreachable!() };
        match member.name {
            "closePath" => path.close_path(), "moveTo" => path.move_to(v[0], v[1]),
            "lineTo" => path.line_to(v[0], v[1]),
            "quadraticCurveTo" => path.quadratic_curve_to(v[0], v[1], v[2], v[3]),
            "bezierCurveTo" => path.bezier_curve_to(v[0], v[1], v[2], v[3], v[4], v[5]),
            "arcTo" => path.arc_to(v[0], v[1], v[2], v[3], v[4]),
            "rect" => path.rect(v[0], v[1], v[2], v[3]),
            "roundRect" => path.round_rect(v[0], v[1], v[2], v[3], radii),
            "arc" => path.arc(v[0], v[1], v[2], v[3], v[4], ccw),
            "ellipse" => path.ellipse(v[0], v[1], v[2], v[3], v[4], v[5], v[6], ccw),
            _ => unreachable!(),
        }
        Ok(JsValue::Undefined)
    }
    fn canvas_gradient_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected CanvasGradient"))?;
        if member.name != "addColorStop" { return Err(Throw::type_error("Unknown gradient member")); }
        let Some(v) = self.canvas_numbers(args, 1)? else { return Err(self.canvas_exception("IndexSizeError")); };
        let color = self.canvas_string(args.get(1))?;
        let result = {
            let mut state = self.state.borrow_mut();
            let CanvasObject::Gradient(gradient, _) = &mut state.canvas_objects[index].1 else { return Err(Throw::type_error("Expected CanvasGradient")); };
            gradient.add_color_stop(v[0], &color)
        };
        match result {
            Err(canvas::GradientError::InvalidOffset) => return Err(self.canvas_exception("IndexSizeError")),
            Err(canvas::GradientError::InvalidColor) => return Err(self.canvas_exception("SyntaxError")),
            Ok(()) => {},
        }
        Ok(JsValue::Undefined)
    }
    fn canvas_image_data_call(&mut self, member: Member, receiver: JsValue) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected ImageData"))?;
        match &self.state.borrow().canvas_objects[index].1 {
            CanvasObject::ImageData(w, h, data) => Ok(match member.name {
                "width" => JsValue::num(f64::from(*w)), "height" => JsValue::num(f64::from(*h)),
                "data" => data.clone(), "colorSpace" => JsValue::str("srgb"),
                _ => JsValue::Undefined,
            }),
            _ => Err(Throw::type_error("Expected ImageData")),
        }
    }
    fn canvas_metrics_call(&mut self, member: Member, receiver: JsValue) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected TextMetrics"))?;
        let state = self.state.borrow();
        let CanvasObject::TextMetrics(metrics) = &state.canvas_objects[index].1 else { return Err(Throw::type_error("Expected TextMetrics")); };
        let value = match member.name {
            "width" => metrics.width,
            "actualBoundingBoxLeft" => metrics.actual_bounding_box_left,
            "actualBoundingBoxRight" => metrics.actual_bounding_box_right,
            "actualBoundingBoxAscent" => metrics.actual_bounding_box_ascent,
            "actualBoundingBoxDescent" => metrics.actual_bounding_box_descent,
            "fontBoundingBoxAscent" => metrics.font_bounding_box_ascent,
            "fontBoundingBoxDescent" => metrics.font_bounding_box_descent,
            _ => 0.,
        };
        Ok(JsValue::num(value))
    }
    fn canvas_draw_image(&mut self, node: NodeIdx, args: &[JsValue]) -> Result<JsValue, Throw> {
        let source = args.first().ok_or_else(|| Throw::type_error("drawImage needs a source"))?;
        let source_node = self.node_brand(source).ok_or_else(|| Throw::type_error("Unsupported image source"))?;
        let source_name = {
            let host = self.state.borrow().host.clone();
            let name = host.borrow().dom.name(source_node).unwrap_or("").to_owned();
            name
        };
        let (bitmap, tainted) = if source_name == "canvas" {
            let source = self.state.borrow().canvas_slots.get(&source_node).map(|slot|
                (slot.context.surface.width, slot.context.surface.height,
                    slot.context.surface.pixels.clone(), !slot.origin_clean));
            let source = source.or_else(|| {
                let (width, height) = (self.canvas_dimension(source_node, "width"), self.canvas_dimension(source_node, "height"));
                (width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216)
                    .then(|| (width, height, vec![0; width as usize * height as usize * 4], false))
            });
            let (width, height, pixels, tainted) = source.ok_or_else(|| self.canvas_exception("InvalidStateError"))?;
            (canvas::Bitmap::new(width, height, pixels)
                .map_err(|_| self.canvas_exception("InvalidStateError"))?, tainted)
        } else if source_name == "img" {
            let source_url = {
                let host = self.state.borrow().host.clone();
                let host = host.borrow();
                host.dom.get_attribute_ns(source_node, None, "src").unwrap_or("").to_owned()
            };
            if source_url.is_empty() { return Err(self.canvas_exception("InvalidStateError")); }
            let page = self.state.borrow().document_url.clone().unwrap_or_default();
            let resolved = crate::cmd::js::resolve_script_url(&page, &source_url);
            let bytes = if resolved.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:")) {
                let url = amazing_net::ParsedUrl::parse(&resolved).map_err(|_| self.canvas_exception("InvalidStateError"))?;
                amazing_fetch::process_data_url(&url).ok_or_else(|| self.canvas_exception("InvalidStateError"))?.body
            } else {
                amazing_net::fetch(&resolved, &amazing_net::FetchOptions { max_response_bytes: 16 * 1024 * 1024, ..Default::default() })
                    .map_err(|_| self.canvas_exception("InvalidStateError"))?.body
            };
            let image = amazing_media::decode_image(&bytes).map_err(|_| self.canvas_exception("InvalidStateError"))?;
            let tainted = if resolved.starts_with("data:") { false } else {
                match (amazing_net::ParsedUrl::parse(&page), amazing_net::ParsedUrl::parse(&resolved)) {
                    (Ok(page), Ok(source)) => page.origin().serialize() != source.origin().serialize(),
                    _ => false,
                }
            };
            (canvas::Bitmap::new(image.width, image.height, image.pixels)
                .map_err(|_| self.canvas_exception("InvalidStateError"))?, tainted)
        } else { return Err(Throw::type_error("Unsupported image source")); };
        let count = match args.len() { 3..=4 => 2, 5..=8 => 4, 9.. => 8, _ => return Err(Throw::type_error("Invalid drawImage overload")) };
        let Some(v) = self.canvas_numbers(&args[1..], count)? else { return Ok(JsValue::Undefined); };
        let mut state = self.state.borrow_mut();
        if tainted {
            state.canvas_slots.get_mut(&node).unwrap().origin_clean = false;
        }
        let context = &mut state.canvas_slots.get_mut(&node).unwrap().context;
        match count {
            2 => context.draw_image(&bitmap, v[0], v[1]),
            4 => context.draw_image_scaled(&bitmap, v[0], v[1], v[2], v[3]),
            _ => context.draw_image_full(&bitmap, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]),
        }
        state.canvas_dirty.insert(node);
        Ok(JsValue::Undefined)
    }
}

/// Sized adapter for the driver-selected shaping/raster provider. Canvas2D's
/// text methods accept a concrete `impl TextSystem`.
struct CanvasTextSystem(Rc<dyn amazing_text::TextSystem>);
impl amazing_text::TextSystem for CanvasTextSystem {
    fn generation(&self) -> u64 { self.0.generation() }
    fn metrics(&self, style: &amazing_text::TextStyle) -> Result<amazing_text::FontMetrics, amazing_text::TextError> {
        self.0.metrics(style)
    }
    fn font_relative_metrics(&self, style: &amazing_text::TextStyle) -> amazing_text::FontRelativeMetrics {
        self.0.font_relative_metrics(style)
    }
    fn shape(&self, paragraph: amazing_text::Paragraph<'_>) -> Result<amazing_text::ShapedParagraph, amazing_text::TextError> {
        self.0.shape(paragraph)
    }
    fn shape_line(&self, request: amazing_text::LineRequest<'_>) -> Result<amazing_text::ShapedLine, amazing_text::TextError> {
        self.0.shape_line(request)
    }
    fn rasterize(&self, request: amazing_text::GlyphRasterRequest<'_>) -> Result<Option<amazing_text::GlyphImage>, amazing_text::TextError> {
        self.0.rasterize(request)
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_pattern_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if member.name != "setTransform" { return Err(Throw::type_error("Unknown CanvasPattern member")); }
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected CanvasPattern"))?;
        let mut offset = [0., 0.];
        if let Some(object) = args.first() {
            for (index, name) in ["e", "f"].iter().enumerate() {
                let value = self.js.binding_get_property(object, name)?;
                if !matches!(value, JsValue::Undefined) {
                    offset[index] = self.js.binding_to_number(&value)?;
                }
            }
        }
        if offset.iter().all(|n| n.is_finite()) {
            let mut state = self.state.borrow_mut();
            let CanvasObject::Pattern(pattern, _) = &mut state.canvas_objects[index].1 else {
                return Err(Throw::type_error("Expected CanvasPattern"));
            };
            pattern.offset = canvas::Point::new(offset[0], offset[1]);
        }
        Ok(JsValue::Undefined)
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_round_radii(&mut self, value: Option<&JsValue>) -> Result<[f64; 4], Throw> {
        let Some(value) = value else { return Ok([0.; 4]); };
        let values = if matches!(value, JsValue::Object(_)) {
            let length = self.js.binding_get_property(value, "length")?;
            if !matches!(length, JsValue::Undefined) {
                let length = self.js.binding_to_number(&length)? as usize;
                if !(1..=4).contains(&length) { return Err(Throw::range_error("roundRect radii length")); }
                let mut radii = Vec::with_capacity(length);
                for i in 0..length {
                    let item = self.js.binding_get_property(value, &i.to_string())?;
                    radii.push(self.js.binding_to_number(&item)?);
                }
                radii
            } else {
                let x = self.js.binding_get_property(value, "x")?;
                vec![if matches!(x, JsValue::Undefined) { 0. } else { self.js.binding_to_number(&x)? }]
            }
        } else { vec![self.js.binding_to_number(value)?] };
        if values.iter().any(|v| !v.is_finite() || *v < 0.) {
            return Err(Throw::range_error("Invalid roundRect radius"));
        }
        Ok(match values.as_slice() {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => unreachable!(),
        })
    }
}

fn canvas_color_text(color: [u8; 4]) -> String {
    if color[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
    } else {
        let alpha = (f64::from(color[3]) / 255.).to_string();
        format!("rgba({}, {}, {}, {})", color[0], color[1], color[2], alpha)
    }
}
