//! Amazing HTML parser — WHATWG HTML Standard tokenizer + tree construction.
//!
//! Tokenizer states are named exactly as in the spec
//! (<https://html.spec.whatwg.org/multipage/parsing.html#tokenization>)
//! because the spec's error-recovery behavior *is* the interop contract;
//! see [`tokenizer`] for its module-level deviation list.
//!
//! Tree construction follows §13.2.6, including the "in body" insertion-mode
//! rules that produce browser-compatible handling of mismatched tags,
//! unclosed `<li>`/`<p>`, implicit `<tbody>`, foster parenting, and the
//! adoption agency algorithm. See [`tree`] for its deviation list.
//!
//! Output is a `amazing_dom::Dom` tree built through arena indices —
//! never `Rc<RefCell<Node>>`.
//!
//! XML/XHTML uses [`parse_xml`], a separate fatal-error parser; see [`xml`]
//! for supported syntax, namespace handling, and explicit DTD/encoding limits.
//!
//! HTML deviations from full spec (deliberate):
//! - [`parse`] accepts caller-decoded text. [`parse_bytes`] uses the pinned
//!   WHATWG Encoding codec; see [`encoding`] for sniffing precedence.
//! - [`IncrementalParser`] accepts already-decoded chunks and finishes only
//!   when the caller explicitly ends the document.
//! - Doctype identifiers and all three document quirks modes are preserved;
//!   srcdoc is explicitly locked to no quirks.
//! - MathML/SVG namespaces and integration points are implemented; some
//!   foreign attribute-name and namespace adjustments remain incomplete.
//!
//! # Example
//!
//! ```
//! let dom = amazing_html::parse("<p>Hello <b>world</b></p>");
//! assert!(dom.dump(amazing_dom::NodeIdx(0)).contains("<b>"));
//! ```

pub mod encoding;
mod byte_stream;
pub mod entities;
pub mod resumable;
pub mod tokenizer;
pub mod tree;
pub mod xml;
pub use tree::IncrementalParser;
pub use tree::{is_javascript_mime_type, strip_ascii_whitespace, ParserConstruction, ParserCustomElementHost, ScriptMode};
pub use resumable::{ParseSignal, ResumableParser};
pub use byte_stream::IncrementalByteParser;
pub use xml::{parse_xml, parse_xml_fragment, XmlFragmentContext, XmlError, XmlErrorKind};

/// Whether scripting affects HTML tree construction. Enabled parsing prepares
/// script events; the embedding is responsible for actually executing them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptingMode {
    #[default]
    Enabled,
    Disabled,
}

use amazing_dom::{Dom, Namespace, NodeIdx};

#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod byte_stream_tests;
#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod fragment_tests;
#[cfg(test)]
mod actual_fragment_tests;
#[cfg(test)]
mod template_tests;
#[cfg(test)]
mod custom_element_tests;
#[cfg(test)]
mod declarative_shadow_tests;
#[cfg(test)]
mod frameset_tests;
#[cfg(test)]
mod tail_mode_tests;
#[cfg(test)]
mod scripting_tests;
#[cfg(test)]
mod document_mode_tests;

/// Decodes and parses a complete HTML byte stream. `transport_label` is the
/// optional charset from out-of-band transport metadata.
pub fn parse_bytes(bytes: &[u8], transport_label: Option<&str>) -> Dom {
    parse(&encoding::decode(bytes, transport_label).text)
}

/// Decodes a complete byte stream and parses with its document scripting mode.
pub fn parse_bytes_with_scripting_mode(
    bytes: &[u8], transport_label: Option<&str>, scripting_mode: ScriptingMode,
) -> Dom {
    parse_with_scripting_mode(&encoding::decode(bytes, transport_label).text, scripting_mode)
}

/// Parses an HTML document into an arena-backed DOM.
///
/// The returned `Dom` always has the document node at index 0 (`NodeIdx(0)`);
/// use [`Dom::dump`] on it for the inspectable terminal view.
pub fn parse(src: &str) -> Dom {
    parse_with_scripting_mode(src, ScriptingMode::Enabled)
}

/// Parses decoded HTML with its document scripting mode.
pub fn parse_with_scripting_mode(src: &str, scripting_mode: ScriptingMode) -> Dom {
    // Newline normalization (part of the spec's encoding step): CRLF and
    // lone CR become LF before tokenization, mirroring what browsers do
    // while decoding the byte stream.
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        src
    };
    tree::build_with_scripting_mode(src, scripting_mode)
}

/// Parses into a DOM with custom-element definitions installed before the
/// first token. A matching definition invokes `host` synchronously; the
/// returned DOM holds its reactions for the embedding to deliver afterward.
pub fn parse_with_custom_element_host(
    src: &str,
    dom: Dom,
    host: Box<dyn ParserCustomElementHost>,
) -> Dom {
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        src
    };
    tree::build_with_custom_element_host(src, dom, host)
}

/// Expanded attribute snapshot of the actual fragment context element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlFragmentAttribute {
    pub namespace_uri: Option<String>,
    pub prefix: Option<String>,
    pub local_name: String,
    pub value: Vec<u16>,
}

/// Actual context identity and owner-document state; names are case-sensitive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlFragmentContext {
    pub local_name: String,
    pub namespace_uri: Option<String>,
    pub attributes: Vec<HtmlFragmentAttribute>,
    pub has_form_ancestor: bool,
    pub scripting_enabled: bool,
    pub quirks_mode: amazing_dom::QuirksMode,
}

/// Parse using an actual DOM context snapshot. Script execution/marking belongs
/// to the caller (innerHTML and Range have different script preparation rules).
pub fn parse_fragment_with_context(
    context: &HtmlFragmentContext, src: &str, allow_shadow_roots: bool,
) -> Result<Dom, String> {
    if context.local_name.is_empty() || context.local_name.chars().any(|c|
        c.is_ascii_whitespace() || matches!(c, '<' | '>' | '\0')) {
        return Err("invalid HTML fragment context name".into());
    }
    let normalized = src.replace("\r\n", "\n").replace('\r', "\n");
    let source = tree::build_fragment_with_context(&normalized, context, allow_shadow_roots)?;
    export_fragment(source, &context.local_name,
        context.namespace_uri.as_deref() == Some(xml::HTML_NAMESPACE))
}

/// Parses an HTML fragment in any HTML element context and
/// returns the fragment children under a standalone document node.
///
/// Scripting is enabled by default. [`parse_fragment_with_scripting_mode`]
/// accepts the context document's explicit scripting mode.
pub fn parse_fragment(context: &str, src: &str) -> Result<Dom, String> {
    parse_fragment_with_namespace(context, Namespace::Html, src)
}

/// `parse_fragment`, but for a context element in a foreign namespace (e.g.
/// `Element.innerHTML` set on a MathML/SVG element) -- HTML-40 (2): needed
/// so the tokenizer's CDATA-allowed determination sees the context
/// element's real namespace instead of always the HTML fragment root's
/// (see `tree::build_fragment`'s own doc comment).
pub fn parse_fragment_with_namespace(context: &str, namespace: Namespace, src: &str) -> Result<Dom, String> {
    parse_fragment_inner(context, namespace, src, false, ScriptingMode::Enabled)
}

/// Parses a fragment with its context namespace and document scripting mode.
pub fn parse_fragment_with_scripting_mode(
    context: &str, namespace: Namespace, src: &str, scripting_mode: ScriptingMode,
) -> Result<Dom, String> {
    parse_fragment_inner(context, namespace, src, false, scripting_mode)
}

/// `parse_fragment`, but with declarative shadow roots enabled (HTML
/// namespace context only, matching `parse_fragment`'s own scope) -- the
/// fragment-parsing entry point `Element.setHTMLUnsafe`/`Document.
/// parseHTMLUnsafe` need per spec, as opposed to ordinary `innerHTML`/
/// `insertAdjacentHTML`, which must keep this off (`parse_fragment`/
/// `parse_fragment_with_namespace` are unchanged and still always parse
/// with it off, so every existing caller keeps its current behavior).
///
/// A `<template shadowrootmode>` whose host is a real, already-open
/// ancestor element *within this same fragment's own markup* (e.g. `<div>
/// <template shadowrootmode=open>...`) attaches to that ancestor exactly as
/// it would in a full document parse, and the attached shadow root's
/// content survives into the returned `Dom` (see `clone_subtree` below).
///
/// A *top-level* `<template shadowrootmode>` -- with no such ancestor
/// element present in the fragment at all, which is `Element.
/// setHTMLUnsafe`'s single most common shape (`el.setHTMLUnsafe('<template
/// shadowrootmode="open">...')`, meant to attach to `el` itself) -- still
/// comes back as an ordinary leftover `<template>` child instead. The
/// fragment parser's internal synthetic root standing in for the context
/// element is never a valid shadow host (`Dom::attach_shadow` rejects the
/// element name "html"), and even if it were, this function has no real
/// `NodeIdx` for the caller's actual host element to attach to -- only the
/// caller does. The caller detects this shape itself (the fragment's first
/// top-level child is a `<template>` with a `shadowrootmode` attribute
/// whose value is `"open"` or `"closed"`) and attaches it to its own real
/// host via `Dom::attach_declarative_shadow`, using that template's content
/// as the shadow root's children and dropping the template from the
/// fragment before inserting the rest.
pub fn parse_fragment_with_shadow_roots(context: &str, src: &str) -> Result<Dom, String> {
    parse_fragment_with_shadow_roots_and_scripting_mode(context, src, ScriptingMode::Enabled)
}

/// Parses an HTML fragment allowing declarative shadow roots, using the
/// context document's explicit scripting mode.
pub fn parse_fragment_with_shadow_roots_and_scripting_mode(
    context: &str, src: &str, scripting_mode: ScriptingMode,
) -> Result<Dom, String> {
    parse_fragment_inner(context, Namespace::Html, src, true, scripting_mode)
}

fn parse_fragment_inner(
    context: &str,
    namespace: Namespace,
    src: &str,
    allow_declarative_shadow_roots: bool,
    scripting_mode: ScriptingMode,
) -> Result<Dom, String> {
    let context = context.to_ascii_lowercase();
    if context.is_empty() || context.chars().any(|c| c.is_ascii_whitespace() || matches!(c, '<' | '>' | '\0')) {
        return Err("invalid HTML fragment context name".into());
    }
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        src
    };
    let source = tree::build_fragment_with_scripting_mode(
        src, &context, namespace, allow_declarative_shadow_roots, scripting_mode,
    );

    export_fragment(source, &context, namespace == Namespace::Html)
}

fn export_fragment(source: Dom, context: &str, html_context: bool) -> Result<Dom, String> {
    fn clone_subtree(source: &Dom, source_node: NodeIdx, target: &mut Dom, parent: NodeIdx) {
        // Namespace identifiers belong to an arena; intern URI identities anew.
        let created = if matches!(source.get(source_node).kind, amazing_dom::NodeKind::Element { .. }) {
            let document = target.owner_document(parent).expect("fragment owner");
            let node = target.create_element_expanded_in(document, source.namespace_uri(source_node),
                source.prefix(source_node), source.local_name(source_node).expect("element name"))
                .expect("parser element identity");
            target.attach_last(parent, node);
            for attribute in source.attributes_expanded(source_node).unwrap_or_default() {
                target.set_attribute_expanded(node, attribute.namespace_uri, attribute.prefix,
                    attribute.local_name, attribute.value).expect("parser attribute identity");
                if let Some(units) = source.attribute_ns_utf16(source_node, attribute.namespace_uri, attribute.local_name) {
                    if !units.iter().copied().eq(attribute.value.encode_utf16()) {
                        let attr = target.get_attribute_node(node, attribute.namespace_uri, attribute.local_name)
                            .expect("cloned attribute");
                        target.set_attr_value(attr, units).expect("cloned attribute units");
                    }
                }
            }
            node
        } else {
            target.append_child(parent, source.get(source_node).kind.clone())
        };
        // A declarative shadow root's content lives under a separate
        // `ShadowRootData`-tracked node (`shadow_root_of_host`), never as an
        // ordinary DOM child of its host -- the template that requested it
        // is never attached to the tree at all (`start_template`'s shadow
        // branch pushes it onto the open-elements stack without ever
        // calling `attach_before`/`append_child`), so without this, cloning
        // silently dropped every declarative shadow tree a fragment parse
        // produced.
        if let Some(shadow_root) = source.shadow_root_of_host(source_node) {
            if let Some(data) = source.shadow_data(shadow_root) {
                let options = data.options;
                let keep_registry_null = data.keep_custom_element_registry_null;
                if let Ok(target_root) = target.attach_declarative_shadow(created, options, keep_registry_null) {
                    for child in source.children(shadow_root) {
                        clone_subtree(source, child, target, target_root);
                    }
                }
            }
        }
        if let Some(contents) = source.template_contents(source_node) {
            let target_contents = target.ensure_template_contents(created).expect("cloned template");
            for child in source.children(contents) {
                clone_subtree(source, child, target, target_contents);
            }
        }
        for child in source.children(source_node) {
            clone_subtree(source, child, target, created);
        }
    }

    let root = source
        .children(NodeIdx(0))
        .find(|node| {
            source.namespace(*node) == Some(Namespace::Html) && source.name(*node) == Some("html")
        })
        .ok_or_else(|| "fragment root was not constructed".to_owned())?;
    let mut fragment = Dom::new();
    fragment.set_document_scripting_enabled(NodeIdx(0), source.document_scripting_enabled(NodeIdx(0)).expect("parser document"))
        .expect("fragment root is a document");
    fragment.set_document_quirks_mode(NodeIdx(0), source.document_quirks_mode(NodeIdx(0)).expect("parser document"))
        .expect("fragment root is a document");
    for child in source.children(root) {
        clone_subtree(&source, child, &mut fragment, NodeIdx(0));
    }
    // After an explicit </html> in the html fragment context, comments are
    // document siblings of the synthetic root but still belong in the result.
    if html_context && context == "html" {
        for child in source.children(NodeIdx(0)) {
            if child != root {
                clone_subtree(&source, child, &mut fragment, NodeIdx(0));
            }
        }
    }
    Ok(fragment)
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_fragment};
    use amazing_dom::NodeIdx;

    /// Integration gate: tokenizer + tree construction wired together.
    #[test]
    fn parses_minimal_document() {
        let dom = parse("<!DOCTYPE html><title>t</title><p>x");
        let dump = dom.dump(NodeIdx(0));
        assert!(dump.contains("<html>"), "implied html missing:\n{dump}");
        assert!(dump.contains("<body>"), "implied body missing:\n{dump}");
        assert_eq!(dom.doctype.as_deref(), Some("html"));
    }

    /// Regression: CRLF pages must not have their `\r` treated as content
    /// (which used to imply head/body prematurely and corrupt the tree).
    #[test]
    fn crlf_is_normalized_before_parsing() {
        let dom = parse("<!DOCTYPE html>\r\n<html>\r\n<body>\r\n<p>hi</p>\r\n</body>\r\n");
        let dump = dom.dump(NodeIdx(0));
        assert!(dump.contains("<body>"), "explicit body missing:\n{dump}");
        assert!(
            dump.contains("  <html>\n    <head>\n    <body>"),
            "whitespace leaked around implied head:\n{dump}"
        );
    }

    #[test]
    fn parses_div_context_fragment_without_document_wrappers() {
        let dom =
            parse_fragment("div", "<b><em><foo><foob><fooc><aside></b></em>").expect("fragment");
        assert_eq!(
            dom.dump(NodeIdx(0)),
            "#document\n  <b>\n    <em>\n      <foo>\n        <foob>\n          <fooc>\n  <aside>\n    <b>\n"
        );
        let recovery =
            parse_fragment("div", "</div><span>x").expect("stray context end tag recovery");
        assert_eq!(
            recovery.dump(NodeIdx(0)),
            "#document\n  <span>\n    \"x\"\n"
        );
        assert!(parse_fragment("template", "x").is_ok());
        let table = parse_fragment("table", "<td>x").expect("table fragment context");
        assert_eq!(
            table.dump(NodeIdx(0)),
            "#document\n  <tbody>\n    <tr>\n      <td>\n        \"x\"\n"
        );
        let body = parse_fragment("tbody", "<tr><td>x").expect("tbody fragment context");
        assert_eq!(
            body.dump(NodeIdx(0)),
            "#document\n  <tr>\n    <td>\n      \"x\"\n"
        );
        let row = parse_fragment("tr", "<td>x").expect("tr fragment context");
        assert_eq!(row.dump(NodeIdx(0)), "#document\n  <td>\n    \"x\"\n");
        let normalized =
            parse_fragment("div", "<span>a\r\nb\rc").expect("fragment newline normalization");
        assert_eq!(
            normalized.dump(NodeIdx(0)),
            "#document\n  <span>\n    \"a\nb\nc\"\n"
        );
    }
}
