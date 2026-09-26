//! Driver-owned WebIDL installation over the same arena used for page parsing.
//! DOMString values cross the runtime boundary as exact UTF-16 code units.
use std::{cell::RefCell, collections::{HashMap, HashSet}, rc::Rc};
use std::collections::{BTreeMap, VecDeque};

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
use amazing_bindings::host::FormNavigationRequest;
use amazing_bindings::storage_interfaces::{StorageInterface as SI, MemberKind as SM};
use amazing_net::url::{UrlSearchParams, UrlWithSearchParams};
use amazing_js::value::Exotic;
use amazing_js::{
    DomHost,
    env::{EnvHandle, EnvKind},
    interp::{BindingPropertyFlags, HostNativeDispatch, Interpreter, LegacyPlatformHooks as RuntimeLegacyHooks},
    value::{
        FunctionBody, HostCallPolicy as JsCallPolicy, HostFunctionId, HostHandle, JsString, JsValue, Throw,
    },
};

#[path = "runtime_host_storage.rs"]
mod runtime_host_storage;
use runtime_host_storage::StorageRuntime;
#[path = "runtime_host_net.rs"]
mod runtime_host_net;
use runtime_host_net::{NetNativeEntry, NetRuntime};

#[path = "runtime_host_messaging.rs"]
pub(crate) mod runtime_host_messaging;
use runtime_host_messaging::MessagingOp;

use crate::dom_host::BrowserDomHost;
use amazing_bindings::canvas_interfaces::{CanvasSlot, Member};
use self::compositor::{CanvasObject, CanvasTarget};
#[path = "runtime_host_compositor.rs"] pub(crate) mod compositor;
#[path = "runtime_host_html.rs"] pub(crate) mod html;

#[path = "runtime_host_dom_bindings.rs"]
mod dom_bindings;

#[derive(Clone, Copy)]
enum NativeEntry {
    Canvas(CanvasTarget, Member),
    Operation(Operation),
    Net(NetNativeEntry),
    IllegalConstructor,
    CapturePromiseResolvers,
    Storage(SI, &'static str, SM),
    StorageTick,
    IFrameContentDocument,
    IFrameSrcGet,
    IFrameSrcSet,
    OptionConstructor,
    Messaging(MessagingOp),
    StorageLockSettled(u64, bool),
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
    constructed_documents: HashSet<NodeIdx>,
    abort_timeouts: Vec<(AbortSignalId, u64)>,
    document_url: Option<String>,
    document_character_set: Option<String>,
    document_content_type: Option<String>,
    document_referrer: Option<String>,
    web_objects: Vec<WebObject>,
    web_object_handles: HashMap<usize, usize>,
    font_sets: HashMap<NodeIdx, u64>,
    window_event_target_slot: u64,
    storage: Option<StorageRuntime>,
    storage_classes: HashMap<SI, (JsValue, JsValue)>,
    storage_inbox: VecDeque<runtime_host_storage::StorageEventDelivery>,
    frames: html::FrameRuntime,
    pending_form_selection_events: Vec<NodeIdx>,
    messaging: Option<runtime_host_messaging::MessagingRealm>,
    net: NetRuntime,
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
    pub(crate) fn take_form_selection_events(&self) -> Vec<NodeIdx> {
        std::mem::take(&mut self.state.borrow_mut().pending_form_selection_events)
    }
    pub(crate) fn create_inert_document(&self, js: &mut Interpreter, bytes: &[u8],
        content_type: &str, url: &str, charset: Option<&str>) -> Result<NodeIdx, DomError> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        adapter.create_inert_document(bytes, content_type, url, charset)
    }

    pub(crate) fn set_document_url(&self, url: &str) {
        self.state.borrow_mut().document_url = Some(url.to_owned());
        self.navigate_iframes();
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
        amazing_bindings::dispatch_user_agent_event(&mut adapter,
            amazing_dom::events::EventTarget::Node(NodeIdx(node)), name,
            amazing_dom::events::EventInit { bubbles: true, cancelable: false, composed: false })
    }

    pub(crate) fn dispatch_window_event(&self, js: &mut Interpreter, name: &str,
        init: amazing_dom::events::EventInit) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        let slot = adapter.with_dom(|dom| dom.window_event_target().ok_or(DomError::InvalidStateError))?;
        amazing_bindings::dispatch_user_agent_event(&mut adapter,
            amazing_dom::events::EventTarget::External(slot), name, init)
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
        if brand == InterfaceBrand::EVENT {
            return self.dom_event_interface(key);
        }
        if brand == InterfaceBrand::NODE {
            let node = self
                .node_from_platform_key(key)
                .ok_or_else(|| Throw::type_error("Invalid node wrapper"))?;
            let constructed = self.state.borrow().constructed_documents.contains(&node);
            let host = self.state.borrow().host.clone();
            let host = host.borrow();
            return Ok(match &host.dom.get(node).kind {
                NodeKind::Document if !constructed && host.dom.document_metadata(node).ok().flatten()
                    .is_some_and(|metadata| metadata.content_type != "text/html") => Interface::XMLDocument,
                NodeKind::Document => Interface::Document,
                NodeKind::DocumentFragment if host.dom.is_shadow_root(node) => Interface::ShadowRoot,
                NodeKind::DocumentFragment => Interface::DocumentFragment,
                NodeKind::DocumentType { .. } => Interface::DocumentType,
                NodeKind::Element { .. } => {
                    if host.dom.namespace(node) == Some(Namespace::Html) {
                        html_element_interface(host.dom.name(node).unwrap_or(""))
                    } else if host.dom.namespace(node) == Some(Namespace::Svg) {
                        Interface::SVGElement
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
            InterfaceBrand::SELECTION => Interface::Selection,
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
            InterfaceBrand::SELECTED_OPTIONS => Interface::HTMLCollection,
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
    let window_event_target_slot = {
        let mut host = host.borrow_mut();
        let slot = host.dom.create_event_target();
        host.dom.set_window_event_target(slot).expect("new Window event target slot");
        slot
    };
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
        constructed_documents: HashSet::new(),
        abort_timeouts: Vec::new(),
        document_url: None,
        document_character_set: None,
        document_content_type: None,
        document_referrer: None,
        web_objects: Vec::new(),
        web_object_handles: HashMap::new(),
        font_sets: HashMap::new(),
        window_event_target_slot,
        storage: None,
        storage_classes: HashMap::new(),
        storage_inbox: VecDeque::new(),
        frames: html::FrameRuntime::default(),
        pending_form_selection_events: Vec::new(),
        messaging: None,
        net: NetRuntime::new(),
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
            NativeEntry::Operation(operation) => adapter.invoke_operation_with_html_hooks(operation, this, &args),
            NativeEntry::Net(entry) => adapter.invoke_net(entry, this, &args),
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
            NativeEntry::IFrameContentDocument => adapter.iframe_content_document(this),
            NativeEntry::IFrameSrcGet => adapter.iframe_src_get(this),
            NativeEntry::IFrameSrcSet => adapter.iframe_src_set(this, &args),
            NativeEntry::OptionConstructor => adapter.option_constructor(&args),
            NativeEntry::Messaging(op) => adapter.messaging_invoke(op, this, &args),
            NativeEntry::StorageLockSettled(id,success) => adapter.storage_lock_settled(id,success,&args),
        }
    });
    js.set_host_native_dispatcher(dispatch);
    let mut adapter = RuntimeBindingHost {
        js,
        state: state.clone(),
    };
    install_interfaces(&mut adapter)?;
    adapter.install_dom_unscopables()?;
    adapter.install_window_event_target()?;
    adapter.install_selection_global()?;
    adapter.install_storage_interfaces()?;
    adapter.install_messaging()?;
    adapter.install_canvas()?;
    adapter.install_iframe_members()?;
    adapter.install_html_form_globals()?;
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
    let (local_storage,session_storage)=adapter.storage_lexical_areas()?;
    fresh.declare_lexical("localStorage",local_storage,false)?;
    fresh.declare_lexical("sessionStorage",session_storage,false)?;
    adapter.js.global_env = fresh;
    adapter.install_net()?;
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
        let value = self.wrap_platform_object(PlatformKey { arena, slot, generation: 1 }, brand)?;
        if let JsValue::Object(object) = &value {
            self.state.borrow_mut().web_object_handles.insert(Rc::as_ptr(&object.0) as usize, slot as usize);
        }
        Ok(value)
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
                Operation::SelectedOptionsSupportedIndexCount => dom.selected_options(NodeIdx(handle.slot as u32)).map(|items| items.len()),
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
    fn window_event_target_slot(&self, value: &JsValue) -> Option<u64> {
        self.dom_window_event_target_slot(value)
    }
    fn window_event_target_value(&self, slot: u64) -> Option<JsValue> {
        self.dom_window_event_target_value(slot)
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
    fn form_selection_changed(&mut self, node: NodeIdx) -> Result<(), Throw> {
        let mut state = self.state.borrow_mut();
        if !state.pending_form_selection_events.contains(&node) {
            state.pending_form_selection_events.push(node);
        }
        Ok(())
    }
    fn form_submit_event(&mut self, form: NodeIdx, submitter: Option<NodeIdx>) -> Result<bool, Throw> {
        let prototype = self.js.binding_object_prototype();
        let options = self.create_object(Some(&prototype))?;
        self.js.binding_define_data_property(&options, "bubbles", JsValue::Bool(true),
            BindingPropertyFlags::METHOD)?;
        self.js.binding_define_data_property(&options, "cancelable", JsValue::Bool(true),
            BindingPropertyFlags::METHOD)?;
        let event = invoke(self, Operation::EventConstructor, JsValue::Undefined,
            &[JsValue::str("submit"), options])?;
        let submitter = match submitter {
            Some(node) => self.wrap_node(node)?,
            None => JsValue::Null,
        };
        self.js.binding_define_data_property(&event, "submitter", submitter,
            BindingPropertyFlags::METHOD)?;
        let target = self.wrap_node(form)?;
        let allowed = invoke(self, Operation::DispatchEvent, target, &[event])?;
        Ok(matches!(allowed, JsValue::Bool(true)))
    }
    fn form_data_event(&mut self, form: NodeIdx, data: FormData) -> Result<FormData, Throw> {
        let form_data = self.form_data_new(data)?;
        let prototype = self.js.binding_object_prototype();
        let options = self.create_object(Some(&prototype))?;
        self.js.binding_define_data_property(&options, "bubbles", JsValue::Bool(true),
            BindingPropertyFlags::METHOD)?;
        let event = invoke(self, Operation::EventConstructor, JsValue::Undefined,
            &[JsValue::str("formdata"), options])?;
        self.js.binding_define_data_property(&event, "formData", form_data.clone(),
            BindingPropertyFlags::METHOD)?;
        let target = self.wrap_node(form)?;
        invoke(self, Operation::DispatchEvent, target, &[event])?;
        self.form_data_record(&form_data)
    }
    fn form_navigate(&mut self, request: FormNavigationRequest) -> Result<(), Throw> {
        self.navigate_form(request)
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
    fn mark_constructed_document(&mut self, document: NodeIdx) {
        self.state.borrow_mut().constructed_documents.insert(document);
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
            DocumentField::CompatMode => Some("CSS1Compat".into()),
        };
        let Some(value) = value else {
            return Err(throw_dom_error(self, DomError::NotSupportedError));
        };
        Ok(value.encode_utf16().collect())
    }
    fn active_element(&mut self, document: NodeIdx) -> Result<Option<NodeIdx>, Throw> {
        self.with_dom(|dom| Ok(dom.active_element(document)))
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
