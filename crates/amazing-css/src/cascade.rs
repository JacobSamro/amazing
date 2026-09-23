//! Cascade, inheritance, and computed styles over the arena DOM.
//!
//! Implemented:
//! - rule compilation: selector strings from [`crate::parser`] are parsed
//!   once up front; per css-selectors a rule whose selector list contains
//!   ANY invalid selector is dropped whole (never partially applied)
//! - cascade per property with the full key: importance → origin
//!   (UA < presentational hints < author; reversed for `!important`) → inline style beats sheets →
//!   layer → specificity → scope proximity → source order
//! - per-declaration source order and the specificity of the most-specific
//!   selector in a list that actually matches
//! - a private specified-to-computed stage for property metadata,
//!   custom-property inheritance/cycles, `var()` substitution, CSS-wide
//!   keywords, `all`, and supported shorthand expansion
//! - a minimal built-in user-agent stylesheet (display defaults for HTML,
//!   heading sizes, link color) so Phase 3 layout starts from real values
//!
//! Deferred/deviations:
//! - `<style media>` conditions are ignored
//! - HTML hints needing browsing/document state are tracked in css_TODO.md
//! - unknown properties get no initial metadata; user-origin sheets, animations
//!   and transitions have no input seam yet

use crate::computed::{self, ComputedStyle, Rollback};
use crate::parser::{parse_declarations, parse_stylesheet, Declaration, Stylesheet};
use crate::selectors::{parse_selector, Specificity, Selector};
use amazing_dom::{Dom, NodeIdx};
use std::collections::{BTreeMap, HashMap};

/// Computed styles for a whole document, keyed by arena index.
#[derive(Debug, Default)]
pub struct StyledDom {
    styles: HashMap<u32, ComputedStyle>,
}

impl StyledDom {
    pub fn get(&self, idx: NodeIdx) -> Option<&ComputedStyle> {
        self.styles.get(&idx.0)
    }

    /// The properties of `idx` whose computed value differs from its parent
    /// element's — the readable "delta" view used by the CLI demo. Elements
    /// without a styled parent element report everything.
    pub fn delta_from_parent(&self, idx: NodeIdx, parent: Option<NodeIdx>) -> Vec<(String, String)> {
        let style = match self.get(idx) {
            Some(s) => s,
            None => return Vec::new(),
        };
        match parent.and_then(|p| self.get(p)) {
            None => style.iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
            Some(pstyle) => style
                .iter()
                .filter(|(k, v)| pstyle.get(k) != Some(*v))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }
}

/// Computes styles for every element of `dom`, combining the built-in UA
/// sheet with the given author sheets (in order).
pub fn compute(dom: &Dom, author_sheets: &[Stylesheet]) -> StyledDom {
    // Compile UA + author rules up front; invalid selectors drop their rule.
    let ua_rules = compile_sheets(&[parse_stylesheet(UA_SHEET)], true);
    let author_rules = compile_sheets(author_sheets, false);

    let body_link = hints::body_link(dom);
    let mut out = StyledDom::default();
    style_subtree(
        dom,
        NodeIdx(0),
        None,
        16.0,
        &ua_rules,
        &author_rules,
        body_link.as_deref(),
        &mut out,
    );
    out
}

fn style_subtree(
    dom: &Dom,
    node: NodeIdx,
    parent_style: Option<&ComputedStyle>,
    root_font_px: f32,
    ua_rules: &[CompiledRule],
    author_rules: &[CompiledRule],
    body_link: Option<&str>,
    out: &mut StyledDom,
) {
    if dom.name(node).is_some() {
        let style = compute_one(
            dom,
            node,
            parent_style,
            root_font_px,
            ua_rules,
            author_rules,
            body_link,
        );
        let descendant_root_font_px = if dom.name(node) == Some("html") {
            style
                .get("font-size")
                .and_then(|value| value.strip_suffix("px"))
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(root_font_px)
        } else {
            root_font_px
        };
        for child in dom.children(node) {
            style_subtree(
                dom,
                child,
                Some(&style),
                descendant_root_font_px,
                ua_rules,
                author_rules,
                body_link,
                out,
            );
        }
        out.styles.insert(node.0, style);
    } else {
        // Text/comment/document nodes carry no style; children of the
        // document node inherit from nothing.
        for child in dom.children(node) {
            style_subtree(
                dom,
                child,
                parent_style,
                root_font_px,
                ua_rules,
                author_rules,
                body_link,
                out,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Compilation + matching
// ---------------------------------------------------------------------------

#[path = "cascade_context.rs"]
mod context;
use context::{compile_sheets, CompiledRule};

#[path = "presentational_hints.rs"]
mod hints;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin { Ua, Hint, Author }

/// One declaration competing for a property during the cascade.
#[derive(Clone, Copy)]
struct Candidate<'a> {
    value: &'a str,
    important: bool,
    inline: bool,
    origin: Origin,
    author_declaration: bool,
    spec: Specificity,
    order: u64,
    layer: usize,
    proximity: usize,
}

/// Full cascade key (css-cascade-4 §6): importance first; origin next, which
/// flips for important declarations; then inline > sheet; layer (also reversed
/// for important), specificity, scope proximity, and finally source order.
fn beats(new: &Candidate<'_>, cur: &Candidate<'_>) -> bool {
    match (new.important, cur.important) {
        (true, false) => return true,
        (false, true) => return false,
        _ => {}
    }
    if new.origin != cur.origin {
        // Normal origin order: author beats UA. Important reverses it.
        return if new.important { new.origin < cur.origin } else { new.origin > cur.origin };
    }
    if new.inline != cur.inline {
        return new.inline;
    }
    if new.layer != cur.layer {
        return if new.important {
            new.layer < cur.layer
        } else {
            new.layer > cur.layer
        };
    }
    if new.spec != cur.spec {
        return new.spec > cur.spec;
    }
    if new.proximity != cur.proximity {
        return new.proximity < cur.proximity;
    }
    new.order > cur.order
}

/// Retain all candidates until rollback has excluded the whole origin/layer.
fn winners<'a>(
    candidates: &BTreeMap<&'a str, Vec<Candidate<'a>>>,
    context: Option<&ComputedStyle>,
) -> BTreeMap<&'a str, Candidate<'a>> {
    candidates
        .iter()
        .filter_map(|(property, values)| {
            let mut values = values.clone();
            loop {
                let winner = values
                    .iter()
                    .copied()
                    .reduce(|a, b| if beats(&b, &a) { b } else { a })?;
                let substituted = context.and_then(|style| {
                    computed::substituted_rollback(style, property, winner.value)
                });
                if winner.value.trim().eq_ignore_ascii_case("revert")
                    || substituted == Some(Rollback::Origin)
                {
                    // UA revert is unset; author revert removes both importance levels.
                    if winner.origin == Origin::Ua {
                        return None;
                    }
                    values.retain(|c| c.origin == Origin::Ua);
                } else if winner.value.trim().eq_ignore_ascii_case("revert-layer")
                    || substituted == Some(Rollback::Layer)
                {
                    values.retain(|c| {
                        if c.origin != winner.origin {
                            // The UA important-to-normal interval also spans
                            // every author declaration, not just UA layers.
                            return !(winner.origin == Origin::Ua && winner.important);
                        }
                        if winner.inline {
                            return !c.inline;
                        }
                        if c.inline {
                            return false;
                        }
                        // Important rollback excludes its layer and every
                        // weaker important layer, then crosses the normal
                        // interval down through the same layer. Otherwise
                        // a later important layer resurfaces after rollback.
                        if winner.important {
                            return c.layer < winner.layer;
                        }
                        c.layer != winner.layer
                    });
                } else {
                    return Some((*property, winner));
                }
            }
        })
        .collect()
}

fn compute_one(
    dom: &Dom,
    el: NodeIdx,
    parent_style: Option<&ComputedStyle>,
    root_font_px: f32,
    ua_rules: &[CompiledRule],
    author_rules: &[CompiledRule],
    body_link: Option<&str>,
) -> ComputedStyle {
    /// One declaration after shorthand expansion, flattened with the
    /// cascade metadata inherited from its source.
    struct FlatDecl {
        prop: String,
        value: String,
        important: bool,
        inline: bool,
        origin: Origin,
        author_declaration: bool,
        spec: Specificity,
        order: u64,
        layer: usize,
        proximity: usize,
    }

    // Collect declarations from every matching rule plus the inline
    // style="" attribute into one owned pool, expanding box/border
    // shorthands so each longhand cascades independently — an author
    // `border: 5px solid` must beat a UA `border-top-style: none`
    // even though the computed map is flat.
    let mut pool: Vec<FlatDecl> = Vec::new();
    let mut source_order = 0u64;
    for (prop, value) in hints::collect(dom, el, body_link) {
        source_order += 1;
        for (prop, value) in computed::expand_declaration(&prop, &value) {
            pool.push(FlatDecl {
                prop, value, important: false, inline: false,
                origin: Origin::Hint, author_declaration: false,
                spec: Specificity::default(), order: source_order,
                layer: usize::MAX, proximity: usize::MAX,
            });
        }
    }
    for rule in ua_rules.iter().chain(author_rules.iter()) {
        let Some((specificity, proximity)) = rule.matching(dom, el) else { continue; };
        for decl in &rule.decls {
            source_order += 1;
            for (property, value) in
                computed::expand_declaration(&decl.property, &decl.value)
            {
                pool.push(FlatDecl {
                    prop: property,
                    value,
                    important: decl.important,
                    inline: false,
                    origin: if rule.ua { Origin::Ua } else { Origin::Author },
                    author_declaration: !rule.ua,
                    spec: specificity,
                    order: source_order,
                    layer: rule.layer,
                    proximity,
                });
            }
        }
    }
    // Inline style="": author origin, beats every sheet rule.
    let inline_decls = attr_of(dom, el, "style")
        .map(parse_declarations)
        .unwrap_or_default();
    for decl in &inline_decls {
        source_order += 1;
        for (property, value) in computed::expand_declaration(&decl.property, &decl.value) {
            pool.push(FlatDecl {
                prop: property,
                value,
                important: decl.important,
                inline: true,
                origin: Origin::Author,
                author_declaration: true,
                spec: Specificity::default(),
                order: source_order,
                layer: usize::MAX,
                proximity: usize::MAX,
            });
        }
    }

    // Collect champions per property. Keys/values borrow from `pool`,
    // which is frozen from here on and outlives the loop below.
    let mut candidates: BTreeMap<&str, Vec<Candidate<'_>>> = BTreeMap::new();
    for d in &pool {
        let cand = Candidate {
            value: &d.value,
            important: d.important,
            inline: d.inline,
            origin: d.origin,
            author_declaration: d.author_declaration,
            spec: d.spec,
            order: d.order,
            layer: d.layer,
            proximity: d.proximity,
        };
        candidates.entry(&d.prop).or_default().push(cand);
    }

    let mut champs = winners(&candidates, None);

    // Logical properties map with the element's COMPUTED axes, not the raw
    // winning tokens: `direction: inherit` and `var()` must see the inherited
    // or substituted value before their physical side is chosen.
    let provisional = computed::resolve_with_root(
        parent_style,
        champs
            .iter()
            .map(|(property, candidate)| ((*property).to_owned(), candidate.value.to_owned()))
            .collect(),
        root_font_px,
    );
    champs = winners(&candidates, Some(&provisional));
    let provisional = computed::resolve_with_root(
        parent_style,
        champs.iter().map(|(p, c)| ((*p).to_owned(), c.value.to_owned())).collect(),
        root_font_px,
    );
    let rtl = provisional
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let writing_mode = provisional.get("writing-mode").unwrap_or("horizontal-tb");
    for logical in [
        "margin-inline-start", "margin-inline-end", "margin-block-start", "margin-block-end",
    ] {
        if let Some(values) = candidates.get(logical).cloned() {
            let physical = physical_margin_for(logical, rtl, writing_mode);
            candidates.entry(physical).or_default().extend(values);
        }
    }
    // Resolve logical border candidates against the same computed axes as margins.
    for (logical, margin, physicals) in [
        ("border-inline-start-width", "margin-inline-start", ["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"]),
        ("border-inline-start-style", "margin-inline-start", ["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"]),
        ("border-inline-start-color", "margin-inline-start", ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"]),
        ("border-inline-end-width", "margin-inline-end", ["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"]),
        ("border-inline-end-style", "margin-inline-end", ["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"]),
        ("border-inline-end-color", "margin-inline-end", ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"]),
        ("border-block-start-width", "margin-block-start", ["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"]),
        ("border-block-start-style", "margin-block-start", ["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"]),
        ("border-block-start-color", "margin-block-start", ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"]),
        ("border-block-end-width", "margin-block-end", ["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"]),
        ("border-block-end-style", "margin-block-end", ["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"]),
        ("border-block-end-color", "margin-block-end", ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"]),
    ] {
        if let Some(values) = candidates.get(logical).cloned() {
            let index = match physical_margin_for(margin, rtl, writing_mode) {
                "margin-top" => 0, "margin-right" => 1, "margin-bottom" => 2, _ => 3,
            };
            candidates.entry(physicals[index]).or_default().extend(values);
        }
    }
    champs = winners(&candidates, Some(&provisional));

    let author_properties: Vec<_> = champs.iter()
        .filter(|(_, candidate)| candidate.author_declaration)
        .map(|(property, _)| *property)
        .collect();
    let cascaded = champs
        .into_iter()
        .map(|(property, candidate)| (property.to_owned(), candidate.value.to_owned()))
        .collect();
    let mut style = computed::resolve_with_root(parent_style, cascaded, root_font_px);
    for property in author_properties {
        style.mark_author_specified(property);
    }
    style
}

/// Origin of the declaration that supplied a standalone paint property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementStyleSource {
    Initial,
    Inherited,
    PresentationHint,
    Author,
    Inline,
}

/// A computed value within the SVG paint subset. Unsupported values retain
/// their computed CSS text so consumers can decline rendering explicitly.
#[derive(Debug, Clone, PartialEq)]
pub enum ElementStyleValue<T> {
    Supported { value: T, source: ElementStyleSource },
    Unsupported { css: String, source: ElementStyleSource },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementFill {
    None,
    Color([u8; 4]),
}

/// Closed paint subset for an element parsed without a document DOM.
/// `computed` can be passed as the parent when computing a descendant.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementPaintStyle {
    pub computed: ComputedStyle,
    pub color: ElementStyleValue<[u8; 4]>,
    pub fill: ElementStyleValue<ElementFill>,
    pub fill_opacity: ElementStyleValue<f32>,
    pub opacity: ElementStyleValue<f32>,
}

/// Compute paint values for a standalone element using the document cascade's
/// hint, author and inline priority and the same variable/keyword resolver.
/// Hints are always normal declarations. Only the four typed paint properties
/// are exposed; custom properties remain internal substitution state.
pub fn compute_element_paint_style(
    parent: Option<&ComputedStyle>,
    presentation_hints: &[Declaration],
    author: &[Declaration],
    inline: &[Declaration],
) -> ElementPaintStyle {
    let mut pool = Vec::new();
    for (declarations, origin, is_inline) in [
        (presentation_hints, Origin::Hint, false),
        (author, Origin::Author, false),
        (inline, Origin::Author, true),
    ] {
        for declaration in declarations {
            for (property, value) in computed::expand_declaration(&declaration.property, &declaration.value) {
                pool.push((property, value, origin, is_inline, declaration.important && origin != Origin::Hint));
            }
        }
    }
    let mut candidates: BTreeMap<&str, Vec<Candidate<'_>>> = BTreeMap::new();
    for (order, (property, value, origin, is_inline, important)) in pool.iter().enumerate() {
        candidates.entry(property).or_default().push(Candidate {
            value,
            important: *important,
            inline: *is_inline,
            origin: *origin,
            author_declaration: *origin == Origin::Author,
            spec: Specificity::default(),
            order: order as u64 + 1,
            layer: usize::MAX,
            proximity: usize::MAX,
        });
    }
    let selected = winners(&candidates, None);
    let provisional = computed::resolve_with_root(parent, selected.iter().map(|(name, candidate)| ((*name).to_owned(), candidate.value.to_owned())).collect(), 16.0);
    let selected = winners(&candidates, Some(&provisional));
    let mut computed = computed::resolve_with_root(parent, selected.iter().map(|(name, candidate)| ((*name).to_owned(), candidate.value.to_owned())).collect(), 16.0);
    for (property, candidate) in &selected {
        if candidate.author_declaration { computed.mark_author_specified(property); }
    }
    let source = |property: &str, inherited: bool| match selected.get(property) {
        Some(candidate) if candidate.inline => ElementStyleSource::Inline,
        Some(candidate) if candidate.origin == Origin::Author => ElementStyleSource::Author,
        Some(_) => ElementStyleSource::PresentationHint,
        None if inherited && parent.is_some() => ElementStyleSource::Inherited,
        None => ElementStyleSource::Initial,
    };
    let color_css = computed.get("color").unwrap_or("black");
    let color = match crate::color::parse_css_color(color_css) {
        Some(value) => ElementStyleValue::Supported { value, source: source("color", true) },
        None => ElementStyleValue::Unsupported { css: color_css.to_owned(), source: source("color", true) },
    };
    let fill_css = computed.get("fill").unwrap_or("black");
    let fill_value = if fill_css.eq_ignore_ascii_case("none") {
        Some(ElementFill::None)
    } else if fill_css.eq_ignore_ascii_case("currentcolor") {
        crate::color::parse_css_color(color_css).map(ElementFill::Color)
    } else {
        crate::color::parse_css_color(fill_css).map(ElementFill::Color)
    };
    let fill = match fill_value {
        Some(value) => ElementStyleValue::Supported { value, source: source("fill", true) },
        None => ElementStyleValue::Unsupported { css: fill_css.to_owned(), source: source("fill", true) },
    };
    let alpha = |property: &str, inherited: bool| {
        let css = computed.get(property).unwrap_or("1");
        match css.parse::<f32>() {
            Ok(value) if value.is_finite() => ElementStyleValue::Supported { value, source: source(property, inherited) },
            _ => ElementStyleValue::Unsupported { css: css.to_owned(), source: source(property, inherited) },
        }
    };
    ElementPaintStyle { color, fill, fill_opacity: alpha("fill-opacity", true), opacity: alpha("opacity", false), computed }
}

fn physical_margin_for(logical: &str, rtl: bool, writing_mode: &str) -> &'static str {
    if writing_mode.eq_ignore_ascii_case("horizontal-tb") {
        match (logical, rtl) {
            ("margin-inline-start", false) | ("margin-inline-end", true) => "margin-left",
            ("margin-inline-start", true) | ("margin-inline-end", false) => "margin-right",
            ("margin-block-start", _) => "margin-top",
            _ => "margin-bottom",
        }
    } else {
        let rl = writing_mode.eq_ignore_ascii_case("vertical-rl")
            || writing_mode.eq_ignore_ascii_case("sideways-rl");
        match (logical, rtl, rl) {
            ("margin-inline-start", false, _) | ("margin-inline-end", true, _) => "margin-top",
            ("margin-inline-start", true, _) | ("margin-inline-end", false, _) => "margin-bottom",
            ("margin-block-start", _, true) | ("margin-block-end", _, false) => "margin-right",
            _ => "margin-left",
        }
    }
}

fn attr_of<'d>(dom: &'d Dom, el: NodeIdx, name: &str) -> Option<&'d str> {
    dom.attrs_of(el)?
        .iter()
        .find(|kv| kv.0 == name)
        .map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------------------
// Built-in user-agent stylesheet
// ---------------------------------------------------------------------------

/// Minimal standards-mode UA sheet (subset of the WHATWG rendering section).
/// Values approximate Chrome/Firefox defaults; logical properties are written
/// physically (`padding-left`) since RTL is unsupported.
const UA_SHEET: &str = r#"
html { display: block; color: black; background-color: white;
       font-family: serif; font-size: 16px; }
head, base, link, meta, script, style, template, title { display: none; }
[hidden] { display: none; }
body { display: block; margin: 8px; }

address, article, aside, blockquote, details, dialog, div, dl, fieldset,
figcaption, figure, footer, form, header, hgroup, main, menu, nav, ol, p,
pre, section, ul { display: block; }
blockquote, figure, menu, ol, p, pre, address, dl, ul { margin: 1em 0; }
dd { display: block; margin: 0 0 0 40px; }
dt { display: block; }

h1, h2, h3, h4, h5, h6 { display: block; font-weight: bold; }
h1 { font-size: 2em; margin: 0.67em 0; }
h2 { font-size: 1.5em; margin: 0.83em 0; }
h3 { font-size: 1.17em; margin: 1em 0; }
h4 { font-size: 1em; margin: 1.33em 0; }
h5 { font-size: 0.83em; margin: 1.67em 0; }
h6 { font-size: 0.67em; margin: 2.33em 0; }

b, strong { font-weight: bolder; }
i, em, cite, var, dfn { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
a:link { color: #0000ee; text-decoration: underline; }
a:visited { color: #551a8b; text-decoration: underline; }
code, kbd, samp, tt { font-family: monospace; }
small { font-size: 0.83em; }
big { font-size: 1.17em; }
center { text-align: center; }
abbr[title] { text-decoration: underline dotted; }

ul, dir { padding-left: 40px; list-style-type: disc; }
ol { padding-left: 40px; list-style-type: decimal; }
li { display: list-item; }

table { display: table; border-spacing: 2px; border-collapse: separate; }
caption { display: table-caption; }
colgroup { display: table-column-group; }
col { display: table-column; }
thead { display: table-header-group; }
tbody { display: table-row-group; }
tfoot { display: table-footer-group; }
tr { display: table-row; }
td, th { display: table-cell; padding: 1px; }
th { font-weight: bold; text-align: center; }

hr { display: block; margin: 0.5em auto;
     border-width: 1px; border-style: inset; overflow: hidden; }
button, input, select { display: inline-block; }
textarea { display: inline-block; font-family: monospace; }
ruby { display: ruby; }
rb { display: ruby-base; }
rt { display: ruby-text; font-size: 50%; }
rtc { display: ruby-text-container; font-size: 50%; }
rp { display: none; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_stylesheet;

    /// html > (head > title, body > h1.title, p#intro, a[href]).
    /// Returns the arena plus [html, head, title, body, h1, p, a].
    fn fixture() -> (Dom, Vec<NodeIdx>) {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let head = dom.create_element(html, "head");
        let title = dom.create_element(head, "title");
        dom.create_text(title, "t");
        let body = dom.create_element(html, "body");
        let h1 = dom.create_element(body, "h1");
        dom.set_attrs(h1, vec![("class".into(), "title".into())]);
        dom.create_text(h1, "Hello");
        let p = dom.create_element(body, "p");
        dom.set_attrs(p, vec![("id".into(), "intro".into())]);
        dom.create_text(p, "World");
        let a = dom.create_element(body, "a");
        dom.set_attrs(a, vec![("href".into(), "/next".into())]);
        (dom, vec![html, head, title, body, h1, p, a])
    }

    #[test]
    fn ua_important_reverses_origin_order_and_ua_revert_defaults() {
        let (dom, elements) = fixture();
        let author = compile_sheets(&[parse_stylesheet("p{color:red!important;display:flex}")], false);
        for (ua_css, expected) in [
            ("p{color:green!important}", "green"),
            ("p{color:revert!important}", "black"),
            ("p{color:revert-layer!important}", "black"),
            ("@layer a,b;@layer a{p{color:revert-layer!important}}@layer b{p{color:green!important}}", "green"),
        ] {
            let ua = compile_sheets(&[parse_stylesheet(ua_css)], true);
            let style = compute_one(&dom, elements[5], None, 16.0, &ua, &author, None);
            assert_eq!(style.get("color"), Some(expected));
        }
    }

    #[test]
    fn ua_defaults_apply_without_author_css() {
        let (dom, els) = fixture();
        let styled = compute(&dom, &[]);
        let body = styled.get(els[3]).unwrap();
        assert_eq!(body.get("display"), Some("block"));
        // The UA `margin: 8px` shorthand expands into per-side longhands.
        for side in ["top", "right", "bottom", "left"] {
            assert_eq!(body.get(&format!("margin-{side}")), Some("8px"));
        }
        // Inherited from html's UA rule.
        assert_eq!(body.get("font-family"), Some("serif"));
        assert_eq!(body.get("font-size"), Some("16px"));
        // Non-inherited default, not overridden by UA sheet on body.
        assert_eq!(body.get("background-color"), Some("transparent"));
        // Head is hidden; its children inherit visibility but stay unrendered
        // via display:none at layout time — here just check the rule hit.
        assert_eq!(styled.get(els[1]).unwrap().get("display"), Some("none"));
        // Heading picked up its UA font-size.
        assert_eq!(styled.get(els[4]).unwrap().get("font-size"), Some("32px"));
    }

    #[test]
    fn author_border_shorthand_beats_ua_longhands() {
        // Regression: the UA sheet declares border-*-style:none longhands;
        // without pre-cascade expansion they would outrank an author
        // `border:` shorthand in the flat computed map.
        let (mut dom, els) = fixture();
        dom.set_attrs(
            els[5],
            vec![("style".into(), "border: 5px solid black".into())],
        );
        let styled = compute(&dom, &[]);
        let p = styled.get(els[5]).unwrap();
        assert_eq!(p.get("border-top-width"), Some("5px"));
        assert_eq!(p.get("border-top-style"), Some("solid"));
        assert_eq!(p.get("border-left-color"), Some("black"));
    }

    #[test]
    fn author_beats_ua_specificity_and_order() {
        let (dom, els) = fixture();
        let sheet = parse_stylesheet("h1 { font-size: 32px } .title { font-size: 48px }");
        let styled = compute(&dom, &[sheet]);
        // Class (0,1,0) beats type (0,0,1).
        assert_eq!(styled.get(els[4]).unwrap().get("font-size"), Some("48px"));

        // Equal specificity: later rule wins.
        let sheet = parse_stylesheet(".title { color: red } .title { color: blue }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(els[4]).unwrap().get("color"), Some("blue"));

        // Lower specificity later still loses.
        let sheet = parse_stylesheet(".title { color: red } h1 { color: green }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(els[4]).unwrap().get("color"), Some("red"));
    }

    #[test]
    fn important_overrides_higher_specificity() {
        let (dom, els) = fixture();
        let sheet = parse_stylesheet("#intro { color: green !important } p { color: red }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
        // And an author !important beats a UA declaration for the same prop:
        let sheet = parse_stylesheet(
            "p { text-decoration: underline wavy !important }",
        );
        // UA gives links underlined text-decoration only for a:link; p has
        // initial none — author important wins over nothing to do here, so
        // just confirm it landed.
        let styled = compute(&dom, &[sheet]);
        assert_eq!(
            styled.get(els[5]).unwrap().get("text-decoration"),
            Some("underline wavy")
        );
    }

    #[test]
    fn inline_style_beats_sheet_unless_important() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let p = dom.create_element(body, "p");
        dom.set_attrs(p, vec![
            ("id".into(), "x".into()),
            ("style".into(), "color: purple".into()),
        ]);
        let sheet = parse_stylesheet("#x { color: gold } p { background-color: silver }");
        let styled = compute(&dom, &[sheet]);
        let st = styled.get(p).unwrap();
        assert_eq!(st.get("color"), Some("purple")); // inline beats #id
        assert_eq!(st.get("background-color"), Some("silver"));

        // !important in a sheet beats non-important inline…
        let sheet = parse_stylesheet("#x { color: olive !important }");
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(p).unwrap().get("color"), Some("olive"));
        // …but inline !important beats it back.
        dom.set_attrs(p, vec![
            ("id".into(), "x".into()),
            ("style".into(), "color: purple !important".into()),
        ]);
        let styled = compute(&dom, &[parse_stylesheet("#x { color: olive !important }")]);
        assert_eq!(styled.get(p).unwrap().get("color"), Some("purple"));
    }

    #[test]
    fn inheritance_flows_down_the_tree() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let div = dom.create_element(body, "div");
        dom.set_attrs(div, vec![("style".into(), "color: teal".into())]);
        let inner = dom.create_element(div, "span");
        let deep = dom.create_element(inner, "b");
        let styled = compute(&dom, &[]);
        assert_eq!(styled.get(div).unwrap().get("color"), Some("teal"));
        assert_eq!(styled.get(inner).unwrap().get("color"), Some("teal"));
        assert_eq!(styled.get(deep).unwrap().get("color"), Some("teal"));
        // Non-inherited props do NOT flow: span stays display:inline.
        assert_eq!(styled.get(inner).unwrap().get("display"), Some("inline"));
    }

    #[test]
    fn dir_hint_inherits_but_author_direction_overrides_it() {
        let mut dom = Dom::new();
        let div = dom.create_element(NodeIdx(0), "div");
        dom.set_attrs(div, vec![("dir".into(), "rtl".into())]);
        let span = dom.create_element(div, "span");
        dom.set_attrs(span, vec![("style".into(), "direction:ltr".into())]);
        dom.create_text(span, "x");
        let em = dom.create_element(div, "em");
        dom.create_text(em, "y");
        let styled = compute(&dom, &[]);

        assert_eq!(styled.get(div).unwrap().get("direction"), Some("rtl"));
        assert_eq!(styled.get(span).unwrap().get("direction"), Some("ltr"));
        assert_eq!(styled.get(em).unwrap().get("direction"), Some("rtl"));
    }

    #[test]
    fn vertical_text_policy_inherits_to_descendant_text_boxes() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        dom.set_attrs(
            root,
            vec![(
                "style".into(),
                "writing-mode:vertical-rl;text-orientation:upright;text-combine-upright:all"
                    .into(),
            )],
        );
        let child = dom.create_element(root, "span");

        let styled = compute(&dom, &[]);
        let child_style = styled.get(child).unwrap();

        assert_eq!(child_style.get("writing-mode"), Some("vertical-rl"));
        assert_eq!(child_style.get("text-orientation"), Some("upright"));
        assert_eq!(child_style.get("text-combine-upright"), Some("all"));
    }

    #[test]
    fn rem_uses_the_computed_root_font_size() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        dom.set_attrs(html, vec![("style".into(), "font-size:20px".into())]);
        let body = dom.create_element(html, "body");
        let paragraph = dom.create_element(body, "p");
        dom.set_attrs(paragraph, vec![("style".into(), "font-size:2rem".into())]);
        let styled = compute(&dom, &[]);

        assert_eq!(styled.get(paragraph).unwrap().get("font-size"), Some("40px"));
    }

    #[test]
    fn invalid_selector_drops_whole_rule() {
        let (dom, els) = fixture();
        // ::before unsupported → second selector invalidates the rule.
        let sheet = parse_stylesheet("h1 { font-weight: normal } h1::before { font-size: 9px }");
        let styled = compute(&dom, &[sheet]);
        // First rule still applies (it was a separate qualified rule)…
        assert_eq!(styled.get(els[2]).unwrap().get("font-weight"), Some("normal"));
        // …and there was never a 9px anywhere.
        assert_ne!(styled.get(els[2]).unwrap().get("font-size"), Some("9px"));

        // One rule, two selectors, one broken: entire rule gone.
        let sheet = parse_stylesheet("h1, :totally-bogus { font-size: 9px }");
        let styled = compute(&dom, &[sheet]);
        assert_ne!(styled.get(els[2]).unwrap().get("font-size"), Some("9px"));
    }

    #[test]
    fn pseudo_class_selectors_match_in_cascade() {
        let (dom, els) = fixture();
        let sheet = parse_stylesheet(
            "body > p:nth-child(2) { letter-spacing: 1px } \
             a:link { text-decoration: underline wavy }",
        );
        let styled = compute(&dom, &[sheet]);
        // p is body's second element child (the h1 precedes it).
        assert_eq!(
            styled.get(els[5]).unwrap().get("letter-spacing"),
            Some("1px")
        );
        // a:link matched.
        assert_eq!(
            styled.get(els[6]).unwrap().get("text-decoration"),
            Some("underline wavy")
        );
    }

    #[test]
    fn delta_view_reports_only_changes() {
        let (mut dom, els) = fixture(); // [html, head, title, body, h1, p, a]
        dom.set_attrs(
            els[5],
            vec![("id".into(), "intro".into()), ("style".into(), "color: red".into())],
        );
        let styled = compute(&dom, &[]);

        let html_delta = styled.delta_from_parent(els[0], None);
        assert!(!html_delta.is_empty()); // root reports everything

        let body_delta = styled.delta_from_parent(els[3], Some(els[0]));
        // body differs from html by display/margin; font props are equal.
        assert!(body_delta.iter().any(|(k, v)| k == "margin-top" && v == "8px"));
        assert!(body_delta.iter().all(|(k, _)| k != "font-family"));

        // p vs body: color override + UA margin difference. Both are
        // display:block per the UA sheet, so display is not a delta.
        let p_delta = styled.delta_from_parent(els[5], Some(els[3]));
        for (prop, want) in [("color", "red"), ("margin-top", "1em")] {
            assert!(
                p_delta.iter().any(|(k, v)| k == prop && v == want),
                "p delta missing {prop}: {want} — got {p_delta:?}"
            );
        }
        // Inherited-equal and display-equal props stay out of the delta.
        assert!(p_delta.iter().all(|(k, _)| k != "font-size"));
        assert!(p_delta.iter().all(|(k, _)| k != "display"));
    }

    #[test]
    fn ua_sheet_parses_with_supported_selectors_only() {
        // Guards against the built-in sheet drifting into syntax the
        // selector engine can't handle (which would silently drop rules).
        let sheet = parse_stylesheet(UA_SHEET);
        assert!(!sheet.rules.is_empty());
        for rule in &sheet.rules {
            for src in &rule.selector_srcs {
                assert!(
                    parse_selector(src).is_some(),
                    "UA sheet selector not supported: {src}"
                );
            }
        }
    }

    #[test]
    fn later_declarations_win_within_one_rule() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet("p { color:red; color:green }")],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
    }

    #[test]
    fn selector_list_uses_the_most_specific_selector_that_actually_matches() {
        let (mut dom, els) = fixture();
        dom.set_attrs(
            els[5],
            vec![("id".into(), "intro".into()), ("class".into(), "target".into())],
        );
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                "p, #does-not-match { color:red } .target { color:green }",
            )],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
    }

    #[test]
    fn custom_properties_preserve_case_inherit_and_substitute() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                ":root { --A:green; --a:crimson } p { color:var(--A); background-color:var(--a) }",
            )],
        );
        let paragraph = styled.get(els[5]).unwrap();
        assert_eq!(paragraph.get("color"), Some("green"));
        assert_eq!(paragraph.get("background-color"), Some("crimson"));
    }

    #[test]
    fn invalid_var_winner_defaults_instead_of_reviving_previous_declaration() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                "body { color:blue } p { color:green; color:var(--missing) }",
            )],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("blue"));
    }

    #[test]
    fn all_resets_supported_properties_but_not_direction_or_unicode_bidi() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                "p { color:red; background-color:blue; margin:10px; direction:rtl; \
                 unicode-bidi:bidi-override; all:initial; color:green }",
            )],
        );
        let paragraph = styled.get(els[5]).unwrap();
        assert_eq!(paragraph.get("color"), Some("green"));
        assert_eq!(paragraph.get("background-color"), Some("transparent"));
        assert_eq!(paragraph.get("margin-left"), Some("0"));
        assert_eq!(paragraph.get("direction"), Some("rtl"));
        assert_eq!(paragraph.get("unicode-bidi"), Some("bidi-override"));
    }

    #[test]
    fn all_does_not_reset_inherited_custom_properties() {
        let (dom, els) = fixture();
        let styled = compute(
            &dom,
            &[parse_stylesheet(
                ":root { --tone:green } p { all:initial; color:var(--tone) }",
            )],
        );
        assert_eq!(styled.get(els[5]).unwrap().get("color"), Some("green"));
    }
}
