//! Cascade, inheritance, and computed styles over the arena DOM.
//!
//! Implemented:
//! - rule compilation: selector strings from [`crate::parser`] are parsed
//!   once up front; per css-selectors a rule whose selector list contains
//!   ANY invalid selector is dropped whole (never partially applied)
//! - cascade per property with the full key: importance → origin
//!   (UA < author; reversed for `!important`) → inline style beats sheets →
//!   specificity → source order
//! - per-declaration source order and the specificity of the most-specific
//!   selector in a list that actually matches
//! - a private specified-to-computed stage for property metadata,
//!   custom-property inheritance/cycles, `var()` substitution, CSS-wide
//!   keywords, `all`, and supported shorthand expansion
//! - a minimal built-in user-agent stylesheet (display defaults for HTML,
//!   heading sizes, link color) so Phase 3 layout starts from real values
//!
//! Deferred/deviations:
//! - presentational attributes (`bgcolor`, `width=…`) and `<style media>`
//!   conditions are ignored
//! - `revert`/`revert-layer` require retaining lower origin/layer candidates
//!   and remain unresolved; unknown properties get no initial metadata

use crate::computed::{self, ComputedStyle};
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
    let ua_rules = compile_sheet(&parse_stylesheet(UA_SHEET), true);
    let mut author_rules = Vec::new();
    for sheet in author_sheets {
        author_rules.extend(compile_sheet(sheet, false));
    }

    let mut out = StyledDom::default();
    style_subtree(
        dom,
        NodeIdx(0),
        None,
        16.0,
        &ua_rules,
        &author_rules,
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
                out,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Compilation + matching
// ---------------------------------------------------------------------------

struct CompiledRule {
    selectors: Vec<(Selector, Specificity)>,
    decls: Vec<Declaration>,
    ua: bool,
}

/// Parses every selector of a rule. Per spec, one bad selector invalidates
/// the entire rule, so any parse failure drops all of them.
fn compile_sheet(sheet: &Stylesheet, ua: bool) -> Vec<CompiledRule> {
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
        if valid && !rule.declarations.is_empty() {
            out.push(CompiledRule {
                selectors,
                decls: rule.declarations.clone(),
                ua,
            });
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
    order: u64,
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
    root_font_px: f32,
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
        order: u64,
    }

    // Collect declarations from every matching rule plus the inline
    // style="" attribute into one owned pool, expanding box/border
    // shorthands so each longhand cascades independently — an author
    // `border: 5px solid` must beat a UA `border-top-style: none`
    // even though the computed map is flat.
    let mut pool: Vec<FlatDecl> = Vec::new();
    let mut source_order = 0u64;
    if let Some(direction) = attr_of(dom, el, "dir")
        .filter(|value| value.eq_ignore_ascii_case("ltr") || value.eq_ignore_ascii_case("rtl"))
    {
        pool.push(FlatDecl {
            prop: "direction".into(),
            value: direction.to_ascii_lowercase(),
            important: false,
            inline: false,
            ua: false,
            spec: Specificity::default(),
            order: 0,
        });
    }
    for rule in ua_rules.iter().chain(author_rules.iter()) {
        let matched_specificity = rule
            .selectors
            .iter()
            .filter(|(selector, _)| crate::selectors::matches(dom, el, selector))
            .map(|(_, specificity)| *specificity)
            .max();
        let Some(specificity) = matched_specificity else {
            continue;
        };
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
                    ua: rule.ua,
                    spec: specificity,
                    order: source_order,
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
                ua: false,
                spec: Specificity::default(),
                order: source_order,
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

    let cascaded = champs
        .into_iter()
        .map(|(property, candidate)| (property.to_owned(), candidate.value.to_owned()))
        .collect();
    computed::resolve_with_root(parent_style, cascaded, root_font_px)
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
