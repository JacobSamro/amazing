//! Crate-level coverage for `markup_bindings.rs`'s top-level declarative-
//! shadow-root handling in `Element.setHTMLUnsafe`: the shape
//! `el.setHTMLUnsafe('<template shadowrootmode="open">...')`, where the
//! `<template>` is the fragment's very first/only top-level content and is
//! meant to attach to `el` itself, rather than to some real ancestor
//! already open within the fragment's own markup (that nested case is
//! handled entirely inside `amazing_html::parse_fragment_with_shadow_
//! roots` and is covered by `amazing-html`'s own tests). No test exercised
//! this plumbing at all before this ticket -- only the WPT corpus did, and
//! that corpus was itself the reason the top-level shape had to be routed
//! here instead of solved inside the parser (see `top_level_declarative_
//! shadow_template`'s doc comment in `markup_bindings.rs`). Uses `Host::
//! parse_fragment_with_shadow_roots`'s sentinel-markup override (see
//! `tests.rs`) rather than a real HTML parser.
use super::*;

#[test]
fn set_html_unsafe_attaches_a_top_level_declarative_shadow_root() {
    let mut h = Host::default();
    let host = h.dom.create_element(NodeIdx(0), "div");
    call(&mut h, O::ElementSetHTMLUnsafe, V::Node(host), &[str_value("<template-shadow-open>")]);
    let root = h.dom.shadow_root_of_host(host).expect("declarative shadow root attached");
    assert!(h.dom.children(host).next().is_none(),
        "the template must not survive as ordinary light-DOM content");
    let child = h.dom.children(root).next().expect("shadow root has the template's content");
    assert_eq!(h.dom.name(child), Some("b"));
    assert_eq!(call(&mut h, O::ElementInnerHTML, V::Node(root), &[]), str_value("<b>shadow-child</b>"));
}

#[test]
fn set_html_unsafe_top_level_shadow_shape_keeps_later_siblings_as_ordinary_content() {
    let mut h = Host::default();
    let host = h.dom.create_element(NodeIdx(0), "div");
    call(&mut h, O::ElementSetHTMLUnsafe, V::Node(host),
        &[str_value("<template-shadow-open-with-sibling>")]);
    assert!(h.dom.shadow_root_of_host(host).is_some());
    let sibling = h.dom.children(host).next().expect("sibling top-level content still imported");
    assert_eq!(h.dom.name(sibling), Some("i"));
}

/// Spec: "attach a shadow root" throwing (host already a shadow host) is
/// caught and the template is inserted as ordinary content instead -- the
/// pre-existing shadow root must be left untouched.
#[test]
fn set_html_unsafe_top_level_shadow_shape_falls_back_when_already_a_shadow_host() {
    let mut h = Host::default();
    let host = h.dom.create_element(NodeIdx(0), "div");
    let existing = call(&mut h, O::ElementAttachShadow, V::Node(host),
        &[V::Dictionary(vec![("mode", str_value("open"))])]);
    call(&mut h, O::ElementSetHTMLUnsafe, V::Node(host), &[str_value("<template-shadow-open>")]);
    assert_eq!(h.dom.shadow_root_of_host(host), Some(node(existing)));
    let template = h.dom.children(host).next().expect("template kept as ordinary content");
    assert_eq!(h.dom.name(template), Some("template"));
}
