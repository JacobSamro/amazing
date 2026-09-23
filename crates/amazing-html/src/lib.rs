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
//! - Doctype identifiers are preserved; document quirks-mode handling remains
//!   approximate.
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
pub mod tokenizer;
pub mod tree;
pub mod xml;
pub use tree::IncrementalParser;
pub use tree::{ParserConstruction, ParserCustomElementHost};
pub use byte_stream::IncrementalByteParser;
pub use xml::{parse_xml, XmlError, XmlErrorKind};

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
mod template_tests;
#[cfg(test)]
mod custom_element_tests;
#[cfg(test)]
mod declarative_shadow_tests;
#[cfg(test)]
mod frameset_tests;

/// Decodes and parses a complete HTML byte stream. `transport_label` is the
/// optional charset from out-of-band transport metadata.
pub fn parse_bytes(bytes: &[u8], transport_label: Option<&str>) -> Dom {
    parse(&encoding::decode(bytes, transport_label).text)
}

/// Parses an HTML document into an arena-backed DOM.
///
/// The returned `Dom` always has the document node at index 0 (`NodeIdx(0)`);
/// use [`Dom::dump`] on it for the inspectable terminal view.
pub fn parse(src: &str) -> Dom {
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
    tree::build(src)
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

/// Parses an HTML fragment in a supported HTML context and
/// returns the fragment children under a standalone document node.
///
/// Other contexts are rejected until their tokenizer
/// initialization rules are implemented; callers never receive a plausible
/// but context-insensitive tree.
/// `noscript` uses this parser's fixed scripting-enabled RAWTEXT behavior;
/// there is no scripting-disabled fragment mode.
pub fn parse_fragment(context: &str, src: &str) -> Result<Dom, String> {
    let context = context.to_ascii_lowercase();
    let supported = [
        "div", "table", "tbody", "thead", "tfoot", "tr", "td", "th", "caption", "colgroup",
        "textarea", "title", "style", "script", "noscript", "plaintext", "select", "option", "template",
    ];
    if !supported.contains(&context.as_str()) {
        return Err("unsupported HTML fragment context".into());
    }
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        src
    };
    let source = tree::build_fragment(src, &context);

    fn clone_subtree(source: &Dom, source_node: NodeIdx, target: &mut Dom, parent: NodeIdx) {
        let created = target.append_child(parent, source.get(source_node).kind.clone());
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
    let virtual_context = matches!(context.as_str(), "textarea" | "title" | "style" | "script" | "noscript" | "plaintext" | "select" | "option" | "template");
    let mut fragment = Dom::new();
    if virtual_context {
        for child in source.children(root) {
            clone_subtree(&source, child, &mut fragment, NodeIdx(0));
        }
        return Ok(fragment);
    }
    let context_node = source
        .children(root)
        .find(|node| {
            source.namespace(*node) == Some(Namespace::Html)
                && source.name(*node) == Some(context.as_str())
        })
        .ok_or_else(|| "fragment context was not constructed".to_owned())?;
    for child in source.children(root) {
        if child == context_node {
            for context_child in source.children(context_node) {
                clone_subtree(&source, context_child, &mut fragment, NodeIdx(0));
            }
        } else {
            clone_subtree(&source, child, &mut fragment, NodeIdx(0));
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
