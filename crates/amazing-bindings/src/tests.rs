use super::*;
use crate::host::{BindingHost, CallbackId, HostCallPolicy, Interface, InterfaceBrand, InterfaceDefinition, IterableKind, LegacyPlatformHooks, Operation as O, PlatformKey};
use amazing_dom::NodeKind;

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
}
#[derive(Default)]
struct Host {
    dom: Dom,
    installed: Vec<Interface>,
    constants: Vec<(&'static str, u16)>,
    interface_constants: Vec<(Interface, &'static str, u32)>,
    conversions: Vec<&'static str>,
    arena_borrowed: bool,
    callbacks: Vec<Option<V>>,
    calls: Vec<u64>,
    reject_lone_surrogate: bool,
    legacy: Vec<(InterfaceBrand, LegacyPlatformHooks)>,
    iterables: Vec<(Interface, IterableKind)>,
}
impl BindingHost for Host {
    type Value = V;
    type Error = String;
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
    fn get_property(&mut self, object: &V, name: &str) -> Result<V, String> {
        assert!(!self.arena_borrowed);
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
    fn is_callable(&self, value: &V) -> bool { matches!(value, V::Callable(_)) }
    fn call(&mut self, function: &V, _this: &V, _arguments: &[V]) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        if let V::Callable(id) = function { self.calls.push(*id); }
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
    fn install_node_constant(&mut self, n: &'static str, v: u16) -> Result<(), String> {
        self.constants.push((n, v));
        Ok(())
    }
    fn install_interface_constant(&mut self, interface: Interface, name: &'static str, value: u32) -> Result<(), String> {
        self.interface_constants.push((interface, name, value)); Ok(())
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
        n == NodeIdx(0)
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
    let custom = node(call(&mut h, O::ImplementationCreateDocument, implementation, &[str_value("urn:test"), str_value("p:root"), V::Null]));
    let root = h.dom.children(custom).next().unwrap();
    assert_eq!(h.dom.qualified_name(root).as_deref(), Some("p:root"));
    assert_eq!(h.dom.namespace_uri(root), Some("urn:test"));
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
    assert_eq!(call(&mut h, O::CustomEventDetail, legacy, &[]), V::Null);
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
    assert_eq!(call(&mut h, O::ElementGetAttributeNS, element.clone(),
        &[str_value("urn:attr"), str_value("name")]), units);
    assert_eq!(call(&mut h, O::AttrOwnerElement, attr.clone(), &[]), element);
    assert_eq!(call(&mut h, O::NamedNodeMapRemoveNamedItemNS, map,
        &[str_value("urn:attr"), str_value("name")]), attr.clone());
    assert_eq!(call(&mut h, O::AttrOwnerElement, attr.clone(), &[]), V::Null);
    assert_eq!(call(&mut h, O::AttrValue, attr, &[]), V::String(vec![0xD800, 0x61]));
}

#[test]
fn processing_instruction_and_range_point_queries_preserve_units() {
    let mut h = Host::default();
    let xml = node(call(&mut h, O::DocumentConstructor, V::Undefined, &[]));
    let pi = call(&mut h, O::CreateProcessingInstruction, V::Node(xml),
        &[str_value("p"), V::String(vec![0xD800])]);
    assert_eq!(call(&mut h, O::NodeType, pi.clone(), &[]), V::Number(7.));
    assert_eq!(call(&mut h, O::Data, pi, &[]), V::String(vec![0xD800]));
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
    install(&mut h).unwrap();
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
fn class_list_is_live_canonical_and_validates_tokens() {
    let mut h = Host::default();
    let element = V::Node(element(&mut h));
    let list = call(&mut h, O::ElementClassList, element.clone(), &[]);
    assert_eq!(call(&mut h, O::ElementClassList, element.clone(), &[]), list);
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
fn html_collection_binding_and_legacy_hooks_are_live() {
    let mut h = Host::default();
    install(&mut h).unwrap();
    assert_eq!(h.legacy.len(), 4);
    assert_eq!(h.legacy[0].0, InterfaceBrand::COLLECTION);
    assert_eq!(h.legacy[0].1.named_getter, Some(O::HTMLCollectionNamedGetter));
    assert!(h.legacy[0].1.unenumerable_names);
    assert!(h.legacy.iter().all(|(_, hooks)| !hooks.override_builtins
        && hooks.indexed_setter.is_none() && hooks.named_setter.is_none()
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
