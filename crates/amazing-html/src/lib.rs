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
//! Crate-level deviations from full spec (deliberate):
//! - Encoding is assumed lossy UTF-8; byte-level encoding detection is not
//!   performed (caller decodes bytes before handing them to [`parse`]).
//! - CDATA sections (`<![CDATA[…]]>`) tokenize as bogus comments.
//! - Doctype PUBLIC/SYSTEM identifiers are consumed but not captured; only
//!   their presence feeds quirks-mode approximation.
//! - MathML/SVG namespaces and integration points are implemented; some
//!   foreign attribute-name and namespace adjustments remain incomplete.
//!
//! # Example
//!
//! ```
//! let dom = amazing_html::parse("<p>Hello <b>world</b></p>");
//! assert!(dom.dump(amazing_dom::NodeIdx(0)).contains("<b>"));
//! ```

pub mod entities;
pub mod tokenizer;
pub mod tree;

use amazing_dom::{Dom, Namespace, NodeIdx};

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

/// Parses an HTML fragment in the supported ordinary HTML `div` context and
/// returns the fragment children under a standalone document node.
///
/// Other contexts are rejected until their tokenizer and table/template
/// initialization rules are implemented; callers never receive a plausible
/// but context-insensitive tree.
pub fn parse_fragment(context: &str, src: &str) -> Result<Dom, String> {
    if !context.eq_ignore_ascii_case("div") {
        return Err("only the HTML div fragment context is supported".into());
    }
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        src
    };
    let source = tree::build_div_fragment(src);

    fn clone_subtree(
        source: &Dom,
        source_node: NodeIdx,
        target: &mut Dom,
        parent: NodeIdx,
    ) {
        let created = target.append_child(parent, source.get(source_node).kind.clone());
        for child in source.children(source_node) {
            clone_subtree(source, child, target, created);
        }
    }

    let root = source
        .children(NodeIdx(0))
        .find(|node| {
            source.namespace(*node) == Some(Namespace::Html)
                && source.name(*node) == Some("html")
        })
        .ok_or_else(|| "fragment root was not constructed".to_owned())?;
    let mut fragment = Dom::new();
    for child in source.children(root) {
        clone_subtree(&source, child, &mut fragment, NodeIdx(0));
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
        let dom = parse_fragment(
            "div",
            "<b><em><foo><foob><fooc><aside></b></em>",
        )
        .expect("fragment");
        assert_eq!(
            dom.dump(NodeIdx(0)),
            "#document\n  <b>\n    <em>\n      <foo>\n        <foob>\n          <fooc>\n  <aside>\n    <b>\n"
        );
        let recovery = parse_fragment("div", "</div><span>x")
            .expect("stray context end tag recovery");
        assert_eq!(
            recovery.dump(NodeIdx(0)),
            "#document\n  <span>\n    \"x\"\n"
        );
        assert!(parse_fragment("table", "<td>x").is_err());
        let normalized = parse_fragment("div", "<span>a\r\nb\rc")
            .expect("fragment newline normalization");
        assert_eq!(
            normalized.dump(NodeIdx(0)),
            "#document\n  <span>\n    \"a\nb\nc\"\n"
        );
    }
}
