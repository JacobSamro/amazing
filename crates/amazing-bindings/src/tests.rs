use super::*;
use crate::host::{AbortHost, BindingHost, CallbackId, DocumentField, DocumentHost, FragmentContext, FragmentParser, HostCallPolicy, Interface, InterfaceBrand, InterfaceDefinition, IterableKind, LegacyPlatformHooks, Operation as O, OptionalHookInstall, PlatformKey, SelectorEngine, UrlReflection};
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
    Promise(u64),
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
    arena_borrowed: bool,
    callbacks: Vec<Option<V>>,
    calls: Vec<u64>,
    reject_lone_surrogate: bool,
    legacy: Vec<(InterfaceBrand, LegacyPlatformHooks)>,
    iterables: Vec<(Interface, IterableKind)>,
    static_methods: Vec<(Interface, &'static str, O, u8)>,
    abort_timers: Vec<(amazing_dom::abort::AbortSignalId, u64)>,
    abort_algorithms: Vec<u64>,
    abort_trace: Vec<u64>,
    active: Option<NodeIdx>,
    focused: bool,
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
        if let V::Callable(id) = function { self.calls.push(*id); self.abort_trace.push(*id); }
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
    fn is_constructor(&mut self, value: &V) -> Result<bool, String> {
        Ok(matches!(value, V::Dictionary(entries) if entries.iter().any(|(name, value)|
            *name == "__constructor" && *value == V::Bool(true))))
    }
    fn require_object(&mut self, value: &V) -> Result<(), String> {
        if matches!(value, V::Dictionary(_) | V::Array(_)) { Ok(()) }
        else { Err("TypeError: Expected an object".into()) }
    }
    fn sequence_dom_strings(&mut self, value: &V) -> Result<Vec<Vec<u16>>, String> {
        let V::Array(values) = value else { return Err("TypeError: Expected a sequence".into()) };
        values.iter().map(|value| self.to_dom_string(value)).collect()
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
    fn report_custom_element_exception(&mut self, error: String) -> Result<(), String> {
        self.custom_element_errors.push(error);
        Ok(())
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
    fn document_field(&mut self, _document: NodeIdx, field: DocumentField) -> Result<Vec<u16>, String> {
        assert!(!self.arena_borrowed);
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
    assert_eq!(report.not_installed.len(), INTERFACES.len() + 2);
    assert_eq!(report.not_installed[0], crate::OptionalCapability::InterfaceClassString(INTERFACES[0].interface));
    assert_eq!(report.not_installed[INTERFACES.len()], crate::OptionalCapability::DomExceptionErrorPrototype);
    assert_eq!(report.not_installed.last(), Some(&crate::OptionalCapability::CustomElementsGlobal));
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
    assert_eq!(h.legacy.len(), 5);
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
