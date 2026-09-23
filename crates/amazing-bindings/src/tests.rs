use super::*;
use crate::host::{BindingHost, HostCallPolicy, Interface, InterfaceDefinition, Operation as O};
use amazing_dom::NodeKind;

#[derive(Clone, Debug, PartialEq)]
enum V {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Vec<u16>),
    Node(NodeIdx),
    List(NodeIdx),
    Coerce(&'static str),
    Throw,
}
#[derive(Default)]
struct Host {
    dom: Dom,
    installed: Vec<Interface>,
    constants: Vec<(&'static str, u16)>,
    conversions: Vec<&'static str>,
    arena_borrowed: bool,
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
    fn get_property(&mut self, _object: &V, _name: &str) -> Result<V, String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn set_property(&mut self, _object: &V, _name: &str, _value: V) -> Result<(), String> {
        Err("installation capability unused by this invocation mock".into())
    }
    fn install_interface(&mut self, d: &'static InterfaceDefinition) -> Result<(), String> {
        self.installed.push(d.interface);
        Ok(())
    }
    fn install_node_constant(&mut self, n: &'static str, v: u16) -> Result<(), String> {
        self.constants.push((n, v));
        Ok(())
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
    fn node_brand(&self, value: &V) -> Option<NodeIdx> {
        if let V::Node(n) = value {
            Some(*n)
        } else {
            None
        }
    }
    fn wrap_node(&mut self, n: NodeIdx) -> Result<V, String> {
        assert!(!self.arena_borrowed);
        Ok(V::Node(n))
    }
    fn child_nodes(&mut self, n: NodeIdx) -> Result<V, String> {
        Ok(V::List(n))
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
    fn string(&self, v: &[u16]) -> V {
        V::String(v.to_vec())
    }
    fn is_undefined(&self, v: &V) -> bool {
        *v == V::Undefined
    }
    fn is_nullish(&self, v: &V) -> bool {
        matches!(v, V::Undefined | V::Null)
    }
    fn to_boolean(&self, v: &V) -> bool {
        !matches!(v, V::Undefined | V::Null | V::Bool(false))
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
