//! The "in foreign content" insertion mode (§13.2.6.5) and the predicates
//! that decide whether a token is processed there at all (MathML text
//! integration points, HTML integration points, the breakout start-tag
//! list). Pure move out of `tree.rs` (no behavior change) to keep that file
//! under the workspace's 4,000-line cap; see `tree.rs`'s own module doc
//! comment and `docs/lanes/html_TODO.md` for the split.
//!
//! `insert_foreign_element` and the SVG/MathML tag/attribute-name adjustment
//! tables stay in `tree.rs` itself: `in_body_start_tag` (for a bare
//! `<svg>`/`<math>` start tag encountered in body) calls them too, so they
//! are not exclusive to this mode.

use super::*;

/// HTML start tags that break out of foreign content (§13.2.6.5).
const FOREIGN_BREAKOUT_START_TAGS: &[&str] = &[
    "b", "big", "blockquote", "body", "br", "center", "code", "dd",
    "div", "dl", "dt", "em", "embed", "h1", "h2", "h3", "h4", "h5",
    "h6", "head", "hr", "i", "img", "li", "listing", "menu", "meta",
    "nobr", "ol", "p", "pre", "ruby", "s", "small", "span", "strong",
    "strike", "sub", "sup", "table", "tt", "u", "ul", "var",
];

impl TreeBuilder {
    fn is_mathml_text_integration_point(&self, idx: NodeIdx) -> bool {
        self.dom.namespace(idx) == Some(Namespace::MathMl)
            && matches!(self.dom.name(idx), Some("mi" | "mo" | "mn" | "ms" | "mtext"))
    }

    fn is_html_integration_point(&self, idx: NodeIdx) -> bool {
        match (self.dom.namespace(idx), self.dom.name(idx)) {
            (Some(Namespace::Svg), Some("foreignObject" | "desc" | "title")) => true,
            (Some(Namespace::MathMl), Some("annotation-xml")) => self
                .dom
                .attrs_of(idx)
                .is_some_and(|attrs| {
                    attrs.iter().any(|(name, value)| {
                        name == "encoding"
                            && (value.eq_ignore_ascii_case("text/html")
                                || value.eq_ignore_ascii_case("application/xhtml+xml"))
                    })
                }),
            _ => false,
        }
    }

    pub(super) fn should_process_in_foreign_content(&self, tok: &Token) -> bool {
        let Some(current) = self.current() else { return false };
        if self.dom.namespace(current) == Some(Namespace::Html) {
            return false;
        }
        match tok {
            Token::Eof => false,
            Token::Characters(_) if self.is_mathml_text_integration_point(current) => false,
            Token::StartTag { name, .. }
                if self.is_mathml_text_integration_point(current)
                    && !matches!(name.as_str(), "mglyph" | "malignmark") =>
            {
                false
            },
            Token::StartTag { name, .. }
                if self.dom.namespace(current) == Some(Namespace::MathMl)
                    && self.dom.name(current) == Some("annotation-xml")
                    && name == "svg" =>
            {
                false
            },
            Token::Characters(_) | Token::StartTag { .. }
                if self.is_html_integration_point(current) =>
            {
                false
            },
            _ => true,
        }
    }

    pub(super) fn in_foreign_content(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                if data.chars().any(|c| !is_ws_char(c)) {
                    self.frameset_ok = false;
                }
                self.insert_text(&data.replace('\0', "\u{FFFD}"));
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing } => {
                let font_breakout = name == "font"
                    && attrs.iter().any(|(attr, _)| {
                        matches!(attr.as_str(), "color" | "face" | "size")
                    });
                if FOREIGN_BREAKOUT_START_TAGS.contains(&name.as_str()) || font_breakout {
                    while let Some(current) = self.current() {
                        if self.dom.namespace(current) == Some(Namespace::Html)
                            || self.is_mathml_text_integration_point(current)
                            || self.is_html_integration_point(current)
                        {
                            break;
                        }
                        self.open.pop();
                    }
                    self.process_token_in_html(Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    });
                    return;
                }
                let namespace = self
                    .current()
                    .and_then(|idx| self.dom.namespace(idx))
                    .unwrap_or(Namespace::Html);
                let node = self.insert_foreign_element(&name, attrs, namespace);
                if self_closing {
                    // HTML-32: a self-closing SVG `<script/>` (necessarily
                    // `src`-only -- self-closing leaves no room for inline
                    // text content) used to be popped here with no classify/
                    // pause check at all, so it was silently never collected
                    // -- the same bug HTML-31 fixed for its non-self-closing
                    // `<script>...</script>` sibling, just reached through
                    // the self-closing branch instead of the `EndTag` match
                    // arm below. Same guards: not a fragment parse, not
                    // inside `<template>` content, SVG namespace only
                    // (MathML has no script element).
                    if !self.fragment
                        && self.template_modes.is_empty()
                        && name.eq_ignore_ascii_case("script")
                        && namespace == Namespace::Svg
                    {
                        let attrs = self.dom.attrs_of(node).map(<[_]>::to_vec).unwrap_or_default();
                        match classify_script(&attrs) {
                            Some(ScriptMode::Blocking) => self.pending_script = Some(node),
                            Some(mode) => self.script_events.push((node, mode)),
                            None => {},
                        }
                    }
                    self.open.pop();
                }
            },
            Token::EndTag { name } => {
                for position in (0..self.open.len()).rev() {
                    let node = self.open[position];
                    if self
                        .dom
                        .name(node)
                        .is_some_and(|local| local.eq_ignore_ascii_case(&name))
                    {
                        // HTML-31: an SVG `<script>` (MathML has no script
                        // element) is never tokenized as rawtext/script-data
                        // -- per spec, foreign content's "in foreign
                        // content" insertion mode has no special case for
                        // `<script>` at all, so it is tokenized as ordinary
                        // markup and closes here, through this generic
                        // stack-popping match, never through `text_mode`'s
                        // script handling above. Run the same classify/pause
                        // decision here so an SVG script actually executes
                        // (WPT's execution-timing/131.html: "scheduler:
                        // inline svg script").
                        if !self.fragment
                            && self.template_modes.is_empty()
                            && name.eq_ignore_ascii_case("script")
                            && self.dom.namespace(node) == Some(Namespace::Svg)
                        {
                            let attrs = self.dom.attrs_of(node).map(<[_]>::to_vec).unwrap_or_default();
                            match classify_script(&attrs) {
                                Some(ScriptMode::Blocking) => self.pending_script = Some(node),
                                Some(mode) => self.script_events.push((node, mode)),
                                None => {},
                            }
                        }
                        self.open.truncate(position);
                        return;
                    }
                    if self.dom.namespace(node) == Some(Namespace::Html) {
                        // The in-body rules synthesize an element for a stray
                        // </p> or </br>. Those recovery elements belong after
                        // the foreign subtree. Other unmatched end tags must
                        // leave that subtree open.
                        if matches!(name.as_str(), "p" | "br") {
                            self.open.truncate(position + 1);
                        }
                        self.process_token_in_html(Token::EndTag { name });
                        return;
                    }
                }
            },
            Token::Eof => self.process_token_in_html(Token::Eof),
            Token::ProcessingInstruction { .. } => unreachable!("handled by dispatcher"),
        }
    }
}
