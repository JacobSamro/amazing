//! Driver-owned WebIDL installation over the same arena used for page parsing.
//! The JS runtime currently stores strings as Unicode scalars. Until its
//! UTF-16 migration, a lone surrogate is an explicit TypeError at this seam.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use amazing_bindings::{
    deliver_mutation_checkpoint,
    dom_exception::throw_dom_error,
    host::{
        BindingHost, CallbackId, HostCallPolicy, Interface, InterfaceBrand, InterfaceDefinition,
        MemberKind, Operation, PlatformKey, RootId,
    },
    install as install_interfaces, invoke,
};
use amazing_dom::{Dom, DomError, NodeIdx, NodeKind};
use amazing_js::{
    DomHost,
    env::{EnvHandle, EnvKind},
    interp::{BindingPropertyFlags, HostNativeDispatch, Interpreter},
    value::{
        HostCallPolicy as JsCallPolicy, HostFunctionId, HostHandle, JsValue, Throw,
        to_display_string,
    },
};

use crate::dom_host::BrowserDomHost;

#[derive(Clone, Copy)]
enum NativeEntry {
    Operation(Operation),
    IllegalConstructor,
    LegacyDocument(LegacyDocument),
    ListIndex(u32),
}

#[derive(Clone, Copy)]
enum LegacyDocument {
    Body,
    DocumentElement,
    TitleGet,
    TitleSet,
    QuerySelector,
    QuerySelectorAll,
    GetElementById,
    GetElementsByTagName,
    GetElementsByClassName,
}

struct BindingState {
    host: Rc<RefCell<BrowserDomHost>>,
    arena: u64,
    functions: Vec<NativeEntry>,
    interfaces: Vec<(Interface, JsValue, JsValue)>,
    roots: Vec<Option<JsValue>>,
    lists: Vec<(PlatformKey, JsValue, usize)>,
    legacy_document: JsValue,
    legacy_elements: HashMap<u32, JsValue>,
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
            extensions.iter().filter_map(|name| old.own_prop(name).cloned().map(|prop| (*name, prop)))
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
                NodeKind::DocumentFragment => Interface::DocumentFragment,
                NodeKind::DocumentType { .. } => Interface::DocumentType,
                NodeKind::Element { .. } => Interface::Element,
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
            InterfaceBrand::COLLECTION => Interface::NodeList,
            InterfaceBrand::NODE_LIST => Interface::NodeList,
            InterfaceBrand::STATIC_NODE_LIST => Interface::NodeList,
            InterfaceBrand::NAMED_NODE_MAP => Interface::NamedNodeMap,
            InterfaceBrand::DOM_TOKEN_LIST => Interface::DOMTokenList,
            InterfaceBrand::DOM_EXCEPTION => Interface::DOMException,
            InterfaceBrand::DOM_IMPLEMENTATION => Interface::DOMImplementation,
            _ => return Err(Throw::type_error("Unknown platform brand")),
        })
    }

    fn legacy_document(
        &mut self,
        operation: LegacyDocument,
        args: &[JsValue],
    ) -> Result<JsValue, Throw> {
        let text = if let Some(value) = args.first() {
            Some(
                String::from_utf16(&self.to_dom_string(value)?)
                    .map_err(|_| Throw::type_error("lone surrogate not yet representable"))?,
            )
        } else {
            None
        };
        let host = self.state.borrow().host.clone();
        match operation {
            LegacyDocument::Body | LegacyDocument::DocumentElement => {
                let selector = if matches!(operation, LegacyDocument::Body) {
                    "body"
                } else {
                    "html"
                };
                let found = host.borrow_mut().query_selector(selector);
                found
                    .map(|node| self.wrap_node(NodeIdx(node)))
                    .transpose()
                    .map(|value| value.unwrap_or(JsValue::Null))
            }
            LegacyDocument::TitleGet => Ok(JsValue::str(host.borrow().get_document_title())),
            LegacyDocument::TitleSet => {
                host.borrow_mut()
                    .set_document_title(text.as_deref().unwrap_or("undefined"))
                    .map_err(Throw::type_error)?;
                Ok(JsValue::Undefined)
            }
            LegacyDocument::QuerySelector | LegacyDocument::GetElementById => {
                let selector = if matches!(operation, LegacyDocument::GetElementById) {
                    format!("#{}", text.as_deref().unwrap_or("undefined"))
                } else {
                    text.unwrap_or_else(|| "undefined".into())
                };
                let found = host.borrow_mut().query_selector(&selector);
                found
                    .map(|node| self.wrap_node(NodeIdx(node)))
                    .transpose()
                    .map(|value| value.unwrap_or(JsValue::Null))
            }
            LegacyDocument::QuerySelectorAll
            | LegacyDocument::GetElementsByTagName
            | LegacyDocument::GetElementsByClassName => {
                let selector = match operation {
                    LegacyDocument::GetElementsByClassName => {
                        format!(".{}", text.as_deref().unwrap_or("undefined"))
                    }
                    _ => text.unwrap_or_else(|| "undefined".into()),
                };
                let nodes = host.borrow_mut().query_selector_all(&selector);
                let values = nodes
                    .into_iter()
                    .map(|node| self.wrap_node(NodeIdx(node)))
                    .collect::<Result<Vec<_>, _>>()?;
                self.create_array(values)
            }
        }
    }

    fn install_legacy_document(&mut self, document: &JsValue) -> Result<(), Throw> {
        for (name, operation, length) in [
            ("querySelector", LegacyDocument::QuerySelector, 1),
            ("querySelectorAll", LegacyDocument::QuerySelectorAll, 1),
            ("getElementById", LegacyDocument::GetElementById, 1),
            (
                "getElementsByTagName",
                LegacyDocument::GetElementsByTagName,
                1,
            ),
            (
                "getElementsByClassName",
                LegacyDocument::GetElementsByClassName,
                1,
            ),
        ] {
            let function = self.native(
                name,
                length,
                NativeEntry::LegacyDocument(operation),
                HostCallPolicy::CallOnly,
            );
            self.define_data_property(document, name, function, true, false, true)?;
        }
        for (name, getter, setter) in [
            ("body", LegacyDocument::Body, None),
            ("documentElement", LegacyDocument::DocumentElement, None),
            (
                "title",
                LegacyDocument::TitleGet,
                Some(LegacyDocument::TitleSet),
            ),
        ] {
            let getter = self.native(
                name,
                0,
                NativeEntry::LegacyDocument(getter),
                HostCallPolicy::CallOnly,
            );
            let setter = setter.map(|operation| {
                self.native(
                    name,
                    1,
                    NativeEntry::LegacyDocument(operation),
                    HostCallPolicy::CallOnly,
                )
            });
            self.define_accessor_property(document, name, Some(getter), setter, true, true)?;
        }
        Ok(())
    }

    fn refresh_lists(&mut self) -> Result<(), Throw> {
        let lists = self.state.borrow().lists.clone();
        for (key, list, installed) in lists {
            let Some(parent) = self.node_from_platform_key(key) else {
                continue;
            };
            let host = self.state.borrow().host.clone();
            let count = host.borrow().dom.children(parent).count();
            for index in installed..count {
                let getter = self.native(
                    "get index",
                    0,
                    NativeEntry::ListIndex(index as u32),
                    HostCallPolicy::CallOnly,
                );
                self.js.binding_define_accessor_property(
                    &list,
                    &index.to_string(),
                    Some(&getter),
                    None,
                    true,
                    true,
                )?;
            }
            let mut state = self.state.borrow_mut();
            if let Some(entry) = state.lists.iter_mut().find(|entry| entry.0 == key) {
                entry.2 = count.max(installed);
            }
        }
        Ok(())
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
        lists: Vec::new(),
        legacy_document: js.binding_get_property(&js.binding_global_object(), "document")?,
        legacy_elements: HashMap::new(),
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
            NativeEntry::LegacyDocument(operation) => adapter.legacy_document(operation, &args),
            NativeEntry::ListIndex(index) => {
                let result = invoke(
                    &mut adapter,
                    Operation::NodeListItem,
                    this,
                    &[JsValue::num(index as f64)],
                )?;
                Ok(if result.is_null() {
                    JsValue::Undefined
                } else {
                    result
                })
            }
        }
    });
    js.set_host_native_dispatcher(dispatch);
    let mut adapter = RuntimeBindingHost {
        js,
        state: state.clone(),
    };
    install_interfaces(&mut adapter)?;
    let document = adapter.wrap_node(NodeIdx(0))?;
    adapter.install_legacy_document(&document)?;
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
        if brand == InterfaceBrand::NODE_LIST {
            let mut state = self.state.borrow_mut();
            if !state.lists.iter().any(|entry| entry.0 == key) {
                state.lists.push((key, value.clone(), 0));
            }
            drop(state);
            self.refresh_lists()?;
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
        self.refresh_lists()?;
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
        String::from_utf16(value)
            .map(JsValue::str)
            .map_err(|_| Throw::type_error("lone surrogate not yet representable"))
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
    fn to_dom_string(&mut self, value: &JsValue) -> Result<Vec<u16>, Throw> {
        let primitive = if value.is_object() {
            let mut primitive = None;
            for name in ["toString", "valueOf"] {
                let method = self.js.binding_get_property(value, name)?;
                if !self.js.binding_is_callable(&method) {
                    continue;
                }
                let result = binding_call(self.js, &method, value, &[])?;
                if !result.is_object() {
                    primitive = Some(result);
                    break;
                }
            }
            primitive.ok_or_else(|| Throw::type_error("cannot convert object to primitive"))?
        } else {
            value.clone()
        };
        Ok(to_display_string(&primitive).encode_utf16().collect())
    }
    fn type_error(&mut self, message: &'static str) -> Throw {
        Throw::type_error(message)
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
