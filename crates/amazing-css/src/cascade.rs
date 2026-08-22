//! Cascade, inheritance, and computed styles over the arena DOM.
//!
//! Phase 2 subset — implemented:
//! - rule compilation: selector strings from [`crate::parser`] are parsed
//!   once up front; per css-selectors a rule whose selector list contains
//!   ANY invalid selector is dropped whole (never partially applied)
//! - cascade per property with the full key: importance → origin
//!   (UA < author; reversed for `!important`) → inline style beats sheets →
//!   specificity → source order
//! - inheritance of the standard inheritable-property set down the tree,
//!   initial values for everything else we track
//! - a minimal built-in user-agent stylesheet (display defaults for HTML,
//!   heading sizes, link color) so Phase 3 layout starts from real values
//!
//! Deferred/deviations:
//! - shorthand↔longhand interaction is NOT modeled: `margin` and
//!   `margin-top` are independent cascade keys until layout needs expansion
//! - presentational attributes (`bgcolor`, `width=…`) and `<style media>`
//!   conditions are ignored
//! - unknown/unsupported properties still cascade and appear in computed
//!   styles but get no inheritance or initial value

use crate::parser::{parse_declarations, parse_stylesheet, Declaration, Stylesheet};
use crate::selectors::{parse_selector, Specificity, Selector};
use amazing_dom::{Dom, NodeIdx};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Computed style output type
// ---------------------------------------------------------------------------

/// Final computed declarations for one element. Keys are lowercased property
/// names in sorted order (deterministic printing).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputedStyle {
    props: BTreeMap<String, String>,
}

impl ComputedStyle {
    pub fn get(&self, prop: &str) -> Option<&str> {
        self.props.get(prop).map(String::as_str)
    }

    /// Sorted (property, value) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn insert(&mut self, prop: String, value: String) {
        self.props.insert(prop, value);
    }

    /// Public mutation for layout-time synthesis: flexbox materializes
    /// absorbed auto margins and flexed widths back into the style so the
    /// box-model resolver sees concrete values.
    pub fn set(&mut self, prop: &str, value: &str) {
        self.props
            .insert(prop.to_ascii_lowercase(), value.to_owned());
    }
}

/// Collecting (property, value) pairs into a style — used by callers that
/// synthesize styles (tests, demos) rather than cascading them.
impl FromIterator<(String, String)> for ComputedStyle {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        let mut s = ComputedStyle::default();
        for (prop, value) in iter {
            s.insert(prop.to_ascii_lowercase(), value);
        }
        s
    }
}

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
    let mut order = 0u32;
    let ua_rules = compile_sheet(&parse_stylesheet(UA_SHEET), true, &mut order);
    let mut author_rules = Vec::new();
    for sheet in author_sheets {
        author_rules.extend(compile_sheet(sheet, false, &mut order));
    }

    let mut out = StyledDom::default();
    style_subtree(dom, NodeIdx(0), None, &ua_rules, &author_rules, &mut out);
    out
}

fn style_subtree(
    dom: &Dom,
    node: NodeIdx,
    parent_style: Option<&ComputedStyle>,
    ua_rules: &[CompiledRule],
    author_rules: &[CompiledRule],
    out: &mut StyledDom,
) {
    if dom.name(node).is_some() {
        let style =
            compute_one(dom, node, parent_style, ua_rules, author_rules);
        for child in dom.children(node) {
            style_subtree(dom, child, Some(&style), ua_rules, author_rules, out);
        }
        out.styles.insert(node.0, style);
    } else {
        // Text/comment/document nodes carry no style; children of the
        // document node inherit from nothing.
        for child in dom.children(node) {
            style_subtree(dom, child, parent_style, ua_rules, author_rules, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Compilation + matching
// ---------------------------------------------------------------------------

struct CompiledRule {
    selectors: Vec<(Selector, Specificity)>,
    decls: Vec<Declaration>,
    order: u32,
    ua: bool,
}

/// Parses every selector of a rule. Per spec, one bad selector invalidates
/// the entire rule, so any parse failure drops all of them.
fn compile_sheet(sheet: &Stylesheet, ua: bool, order: &mut u32) -> Vec<CompiledRule> {
    let mut out = Vec::new();
    for rule in &sheet.rules {
        let mut selectors = Vec::with_capacity(rule.selector_srcs.len());
        let mut valid = true;
        for src in &rule.selector_srcs {
            match parse_selector(src) {
                Some(sel) => {
                    let spec = sel.specificity();
                    selectors.push((sel, spec));
                },
                None => {
                    valid = false;
                    break;
                },
            }
        }
        *order += 1;
        if valid && !rule.declarations.is_empty() {
            out.push(CompiledRule { selectors, decls: rule.declarations.clone(), order: *order, ua });
        }
    }
    out
}

/// One declaration competing for a property during the cascade.
struct Candidate<'a> {
    value: &'a str,
    important: bool,
    inline: bool,
    ua: bool,
    spec: Specificity,
    order: u32,
}

/// Full cascade key (css-cascade-4 §6): importance first; origin next, which
/// flips for important declarations; then inline > sheet; then specificity;
/// finally later source order.
fn beats(new: &Candidate<'_>, cur: &Candidate<'_>) -> bool {
    match (new.important, cur.important) {
        (true, false) => return true,
        (false, true) => return false,
        _ => {},
    }
    if new.ua != cur.ua {
        // Normal origin order: author beats UA. Important reverses it.
        return if new.important { new.ua } else { !new.ua };
    }
    if new.inline != cur.inline {
        return new.inline;
    }
    if new.spec != cur.spec {
        return new.spec > cur.spec;
    }
    new.order > cur.order
}

/// Installs `cand` for `prop` unless the current champion already outranks it.
/// One named lifetime ties the map's key/value borrows to the arguments.
fn consider<'d>(
    champs: &mut BTreeMap<&'d str, Candidate<'d>>,
    prop: &'d str,
    cand: Candidate<'d>,
) {
    match champs.get(prop) {
        Some(cur) if !beats(&cand, cur) => {},
        _ => {
            champs.insert(prop, cand);
        },
    }
}

fn compute_one(
    dom: &Dom,
    el: NodeIdx,
    parent_style: Option<&ComputedStyle>,
    ua_rules: &[CompiledRule],
    author_rules: &[CompiledRule],
) -> ComputedStyle {
    /// One declaration after shorthand expansion, flattened with the
    /// cascade metadata inherited from its source.
    struct FlatDecl {
        prop: String,
        value: String,
        important: bool,
        inline: bool,
        ua: bool,
        spec: Specificity,
        order: u32,
    }

    // Collect declarations from every matching rule plus the inline
    // style="" attribute into one owned pool, expanding box/border
    // shorthands so each longhand cascades independently — an author
    // `border: 5px solid` must beat a UA `border-top-style: none`
    // even though the computed map is flat.
    let mut pool: Vec<FlatDecl> = Vec::new();
    for rule in ua_rules.iter().chain(author_rules.iter()) {
        let matched = rule
            .selectors
            .iter()
            .any(|(sel, _)| crate::selectors::matches(dom, el, sel));
        if !matched {
            continue;
        }
        for decl in &rule.decls {
            for (p, v) in expand_shorthand(&decl.property, &decl.value) {
                pool.push(FlatDecl {
                    prop: p,
                    value: v,
                    important: decl.important,
                    inline: false,
                    ua: rule.ua,
                    spec: rule.selectors.iter().map(|(_, s)| *s).max().unwrap_or_default(),
                    order: rule.order,
                });
            }
        }
    }
    // Inline style="": author origin, beats every sheet rule.
    let inline_decls = attr_of(dom, el, "style")
        .map(parse_declarations)
        .unwrap_or_default();
    for decl in &inline_decls {
        for (p, v) in expand_shorthand(&decl.property, &decl.value) {
            pool.push(FlatDecl {
                prop: p,
                value: v,
                important: decl.important,
                inline: true,
                ua: false,
                spec: Specificity::default(),
                order: 0,
            });
        }
    }

    // Collect champions per property. Keys/values borrow from `pool`,
    // which is frozen from here on and outlives the loop below.
    let mut champs: BTreeMap<&str, Candidate<'_>> = BTreeMap::new();
    for d in &pool {
        let cand = Candidate {
            value: &d.value,
            important: d.important,
            inline: d.inline,
            ua: d.ua,
            spec: d.spec,
            order: d.order,
        };
        consider(&mut champs, &d.prop, cand);
    }

    // Merge: cascaded winner > inherited (inheritable props only) > initial.
    let mut result = ComputedStyle::default();
    for &(prop, init) in DEFAULTS {
        if let Some(c) = champs.remove(prop) {
            result.insert(prop.to_owned(), c.value.to_owned());
        } else if INHERITED.contains(&prop) {
            // Inheritable: take the parent's computed value, else initial.
            let inherited =
                parent_style.and_then(|p| p.get(prop)).unwrap_or(init);
            result.insert(prop.to_owned(), inherited.to_owned());
        } else {
            result.insert(prop.to_owned(), init.to_owned());
        }
    }
    for prop in INHERITED.iter().copied() {
        if DEFAULTS.iter().any(|(p, _)| *p == prop) {
            continue; // already handled above
        }
        if let Some(c) = champs.remove(prop) {
            result.insert(prop.to_owned(), c.value.to_owned());
        } else if let Some(v) = parent_style.and_then(|p| p.get(prop)) {
            result.insert(prop.to_owned(), v.to_owned());
        }
    }
    // Properties outside our tracked set still surface when declared.
    for (prop, c) in champs {
        result.insert(prop.to_owned(), c.value.to_owned());
    }
    result
}

fn attr_of<'d>(dom: &'d Dom, el: NodeIdx, name: &str) -> Option<&'d str> {
    dom.attrs_of(el)?
        .iter()
        .find(|kv| kv.0 == name)
        .map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------------------
// Shorthand expansion
// ---------------------------------------------------------------------------

/// Expands the box/border shorthands this engine tracks into their
/// longhand (property, value) pairs; every other property passes through
/// unchanged. Expansion happens BEFORE cascading so each longhand competes
/// with its source's origin and specificity (CSS2.1 §8.1–§8.5 shorthand
/// semantics), which the flat computed-value map could not express.
fn expand_shorthand(prop: &str, value: &str) -> Vec<(String, String)> {
    const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
    let parts: Vec<&str> = value.split_ascii_whitespace().collect();
    match prop {
        // 1–4 value replication (CSS2.1 §8.3): t | t r | t r b | t r b l.
        "margin" | "padding" | "border-width" | "border-style" | "border-color" => {
            if parts.is_empty() {
                return vec![(prop.to_owned(), value.to_owned())];
            }
            SIDES
                .iter()
                .enumerate()
                .map(|(i, side)| {
                    let v = parts[replicate_index(parts.len(), i)];
                    (longhand_name(prop, side), v.to_owned())
                })
                .collect()
        },
        // `border: <line-width> || <line-style> || <color>` — components in
        // any order; omitted ones reset to their initial values on all sides.
        "border" if !parts.is_empty() => {
            let mut w = "medium";
            let mut st = "none";
            let mut co = "currentcolor";
            for p in &parts {
                if is_border_style(p) {
                    st = p;
                } else if is_border_width(p) {
                    w = p;
                } else {
                    co = p;
                }
            }
            let mut out = Vec::with_capacity(12);
            for side in SIDES {
                out.push((format!("border-{side}-width"), w.to_owned()));
                out.push((format!("border-{side}-style"), st.to_owned()));
                out.push((format!("border-{side}-color"), co.to_owned()));
            }
            out
        },
        _ => vec![(prop.to_owned(), value.to_owned())],
    }
}

/// Replication of a 1–4 component list onto side `i` (t r b l).
fn replicate_index(n: usize, i: usize) -> usize {
    match n {
        0 => 0,
        1 => 0,
        2 => i % 2,
        3 if i == 3 => 1,
        3 => i.min(2),
        _ => i.min(3),
    }
}

/// `margin` → `margin-top`; `border-width` → `border-top-width`.
fn longhand_name(shorthand: &str, side: &str) -> String {
    match shorthand.rsplit_once('-') {
        Some((head, tail)) => format!("{head}-{side}-{tail}"),
        None => format!("{shorthand}-{side}"),
    }
}

fn is_border_style(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "none" | "hidden" | "dotted" | "dashed" | "solid"
            | "double" | "groove" | "ridge" | "inset" | "outset"
    )
}

/// Width keywords or anything that starts like a length/percentage
/// (no color keyword begins with a digit or sign).
fn is_border_width(s: &str) -> bool {
    if matches!(
        s.to_ascii_lowercase().as_str(),
        "thin" | "medium" | "thick"
    ) {
        return true;
    }
    matches!(
        s.chars().next(),
        Some('0'..='9') | Some('.') | Some('-') | Some('+')
    )
}

// ---------------------------------------------------------------------------
// Initial values + inheritance sets
// ---------------------------------------------------------------------------

/// Initial (CSS-defined default) values for the properties this engine
/// tracks. Shorthands intentionally absent — see module deviations.
const DEFAULTS: &[(&str, &str)] = &[
    ("background-color", "transparent"),
    ("border-bottom-color", "currentcolor"),
    ("border-bottom-style", "none"),
    ("border-bottom-width", "medium"),
    ("border-left-color", "currentcolor"),
    ("border-left-style", "none"),
    ("border-left-width", "medium"),
    ("border-right-color", "currentcolor"),
    ("border-right-style", "none"),
    ("border-right-width", "medium"),
    ("border-top-color", "currentcolor"),
    ("border-top-style", "none"),
    ("border-top-width", "medium"),
    ("clear", "none"),
    ("color", "black"),
    ("direction", "ltr"),
    ("display", "inline"),
    ("float", "none"),
    ("font-size", "medium"),
    ("font-style", "normal"),
    ("font-variant", "normal"),
    ("font-weight", "normal"),
    ("height", "auto"),
    ("letter-spacing", "normal"),
    ("line-height", "normal"),
    ("list-style-image", "none"),
    ("list-style-position", "outside"),
    ("list-style-type", "disc"),
    ("opacity", "1"),
    ("overflow", "visible"),
    ("position", "static"),
    ("text-align", "start"),
    ("text-decoration", "none"),
    ("text-indent", "0"),
    ("text-transform", "none"),
    ("vertical-align", "baseline"),
    ("visibility", "visible"),
    ("white-space", "normal"),
    ("width", "auto"),
    ("word-spacing", "normal"),
    ("z-index", "auto"),
];

/// Standard inheritable properties (CSS2.1 §6.4.1 + common CSS3 additions
/// relevant here).
const INHERITED: &[&str] = &[
    "border-collapse",
    "caption-side",
    "color",
    "cursor",
    "direction",
    "empty-cells",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "letter-spacing",
    "line-height",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "orphans",
    "quotes",
    "text-align",
    "text-indent",
    "text-transform",
    "visibility",
    "white-space",
    "widows",
    "word-spacing",
];

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
        assert_eq!(styled.get(els[4]).unwrap().get("font-size"), Some("2em"));
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
}
