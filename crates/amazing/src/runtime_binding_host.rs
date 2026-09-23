//! Driver-owned WebIDL installation over the same arena used for page parsing.
//! DOMString values cross the runtime boundary as exact UTF-16 code units.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use amazing_bindings::{
    deliver_mutation_checkpoint, fire_abort_timeout, html_element_interface,
    dom_exception::throw_dom_error,
    host::{
        BindingHost, CallbackId, HostCallPolicy, Interface, InterfaceBrand, InterfaceDefinition,
        MemberKind, Operation, OptionalHookInstall, PlatformKey, RootId, LegacyPlatformHooks, IterableKind,
        FragmentParser, FragmentContext, SelectorEngine, AbortHost, DocumentHost, DocumentField, UrlReflection,
    },
    install as install_interfaces, invoke,
};
use amazing_dom::{Dom, DomError, NodeIdx, NodeKind, Namespace, abort::AbortSignalId};
use amazing_js::{
    DomHost,
    env::{EnvHandle, EnvKind},
    interp::{BindingPropertyFlags, HostNativeDispatch, Interpreter, LegacyPlatformHooks as RuntimeLegacyHooks},
    value::{
        HostCallPolicy as JsCallPolicy, HostFunctionId, HostHandle, JsValue, Throw,
    },
};

use crate::dom_host::BrowserDomHost;

#[derive(Clone, Copy)]
enum NativeEntry {
    Operation(Operation),
    IllegalConstructor,
}

struct BindingState {
    host: Rc<RefCell<BrowserDomHost>>,
    arena: u64,
    functions: Vec<NativeEntry>,
    interfaces: Vec<(Interface, JsValue, JsValue)>,
    roots: Vec<Option<JsValue>>,
    legacy_document: JsValue,
    legacy_elements: HashMap<u32, JsValue>,
    abort_timeouts: Vec<(AbortSignalId, u64)>,
    document_url: Option<String>,
    document_character_set: Option<String>,
    document_content_type: Option<String>,
    document_referrer: Option<String>,
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

    pub(crate) fn deliver_mutations(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost {
            js,
            state: self.state.clone(),
        };
        deliver_mutation_checkpoint(&mut adapter)
    }

    #[cfg(test)]
    pub(crate) fn seed_test_root(&self, id: RootId, value: JsValue) {
        let mut state = self.state.borrow_mut();
        state.roots.resize(id.0 as usize + 1, None);
        state.roots[id.0 as usize] = Some(value);
    }
}

impl RuntimeBindingHost<'_> {
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
        if !host.borrow().dom.is_connected(node) { return Ok(()); }
        let old = self.state.borrow().legacy_elements.get(&node.0).cloned();
        let old = if let Some(old) = old { old } else {
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
            InterfaceBrand::NODE_LIST => Interface::NodeList,
            InterfaceBrand::STATIC_NODE_LIST => Interface::NodeList,
            InterfaceBrand::FILTERED_NODE_LIST => Interface::NodeList,
            InterfaceBrand::NAMED_NODE_MAP => Interface::NamedNodeMap,
            InterfaceBrand::DOM_TOKEN_LIST => Interface::DOMTokenList,
            InterfaceBrand::DOM_EXCEPTION => Interface::DOMException,
            InterfaceBrand::DOM_IMPLEMENTATION => Interface::DOMImplementation,
            InterfaceBrand::ABORT_CONTROLLER => Interface::AbortController,
            InterfaceBrand::ABORT_SIGNAL => Interface::AbortSignal,
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
        host,
        arena,
        functions: Vec::new(),
        interfaces: Vec::new(),
        roots: Vec::new(),
        legacy_document: js.binding_get_property(&js.binding_global_object(), "document")?,
        legacy_elements: HashMap::new(),
        abort_timeouts: Vec::new(),
        document_url: None,
        document_character_set: None,
        document_content_type: None,
        document_referrer: None,
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
            NativeEntry::Operation(operation) => invoke(&mut adapter, operation, this, &args),
            NativeEntry::IllegalConstructor => Err(Throw::type_error("Illegal constructor")),
        }
    });
    js.set_host_native_dispatcher(dispatch);
    let mut adapter = RuntimeBindingHost {
        js,
        state: state.clone(),
    };
    install_interfaces(&mut adapter)?;
    let document = adapter.wrap_node(NodeIdx(0))?;
    // set_dom_host installs an immutable lexical legacy document. Shadow it in
    // a new global environment so every new page script sees the canonical
    // private wrapper while older browser globals remain available upstream.
    let fresh = EnvHandle::new(EnvKind::Global, Some(adapter.js.global_env.clone()));
    fresh.declare_lexical("document", document.clone(), false)?;
    adapter.js.global_env = fresh;
    adapter
        .js
        .global_obj
        .borrow_mut()
        .set_own("document", document);
    Ok(RuntimeBindings { state })
}

impl BindingHost for RuntimeBindingHost<'_> {
    type Value = JsValue;
    type Error = Throw;

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
                _ => Err(DomError::NotSupportedError),
            }.map_err(|error| Throw::type_error(error.to_string()))?;
            u32::try_from(length).map_err(|_| Throw::range_error("collection length exceeds u32"))
        });
        let names = hooks.supported_names.map(|_| {
            let state = self.state.clone();
            Rc::new(move |handle: HostHandle| {
                let borrowed = state.borrow();
                if handle.arena != borrowed.arena || handle.generation != 1 || handle.brand != brand.0 {
                    return Err(Throw::type_error("Invalid legacy platform object"));
                }
                borrowed.host.borrow_mut().dom.collection_names(handle.slot as usize)
                    .map_err(|error| Throw::type_error(error.to_string()))
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
        self.js.binding_define_legacy_platform_object(brand.0, RuntimeLegacyHooks {
            supported_index_count: count, supported_names: names, indexed_getter, named_getter,
            indexed_setter: None, named_setter: None, named_deleter: None,
            unenumerable_names: hooks.unenumerable_names, override_builtins: hooks.override_builtins,
        });
        Ok(())
    }
    fn define_iterable(&mut self, interface: Interface, _kind: IterableKind) -> Result<(), Throw> {
        let brand = match interface {
            Interface::NodeList => InterfaceBrand::NODE_LIST,
            Interface::DOMTokenList => InterfaceBrand::DOM_TOKEN_LIST,
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
        self.materialize_accessors(&value);
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
        if document != NodeIdx(0) { return Err(throw_dom_error(self, DomError::NotSupportedError)); }
        let value = match field {
            DocumentField::URL | DocumentField::DocumentURI => self.state.borrow().document_url.clone(),
            DocumentField::CharacterSet => self.state.borrow().document_character_set.clone(),
            DocumentField::ContentType => self.state.borrow().document_content_type.clone(),
            DocumentField::Referrer => self.state.borrow().document_referrer.clone(),
            DocumentField::CompatMode => None,
        }.ok_or_else(|| throw_dom_error(self, DomError::NotSupportedError))?;
        Ok(value.encode_utf16().collect())
    }
    fn active_element(&mut self, _document: NodeIdx) -> Result<Option<NodeIdx>, Throw> {
        Err(throw_dom_error(self, DomError::NotSupportedError))
    }
    fn has_focus(&mut self, _document: NodeIdx) -> Result<bool, Throw> {
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
