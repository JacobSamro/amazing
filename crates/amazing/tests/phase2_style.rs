//! Phase 2 end-to-end gate: HTML (amazing-html) + CSS (amazing-css) →
//! computed styles over the shared arena DOM. Lives in the driver crate
//! because the component graph forbids amazing-css from depending on
//! amazing-html.

use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_css::{compute, parse_stylesheet};

/// Concatenates the text content of every `<style>` element in document
/// order (mirrors what the `amazing style` demo subcommand does).
fn collect_style_text(dom: &Dom, node: NodeIdx, out: &mut String) {
    if dom.name(node) == Some("style") {
        for child in dom.children(node) {
            if let NodeKind::Text { data } = &dom.get(child).kind {
                out.push_str(data);
            }
        }
        return;
    }
    for child in dom.children(node) {
        collect_style_text(dom, child, out);
    }
}

#[test]
fn html_plus_css_yields_computed_styles() {
    let dom = amazing_html::parse(
        "<!DOCTYPE html><title>t</title>\
         <style>\
           p { color: navy }\
           .warn { color: darkred !important }\
           #special { background-color: yellow }\
           div > span:first-child { font-weight: bold }\
         </style>\
         <body>\
           <p style=\"letter-spacing:1px\">hi</p>\
           <p class=\"warn\">careful</p>\
           <div><span id=\"special\">x</span></div>",
    );
    let mut css = String::new();
    collect_style_text(&dom, NodeIdx(0), &mut css);
    assert!(css.contains("navy"));

    let styled = compute(&dom, &[parse_stylesheet(&css)]);

    // Find elements by walking.
    let find = |name: &str, attrs: &[(&str, &str)]| -> Option<NodeIdx> {
        fn walk(
            dom: &Dom,
            n: NodeIdx,
            name: &str,
            attrs: &[(&str, &str)],
        ) -> Option<NodeIdx> {
            let attr_match = |haystack: &[(String, String)]| {
                attrs.iter().all(|(k, v)| {
                    haystack
                        .iter()
                        .any(|(ak, av)| ak.as_str() == *k && av.as_str() == *v)
                })
            };
            if dom.name(n) == Some(name)
                && dom.attrs_of(n).map(attr_match).unwrap_or(!attrs.is_empty())
            {
                return Some(n);
            }
            for c in dom.children(n) {
                if let Some(f) = walk(dom, c, name, attrs) {
                    return Some(f);
                }
            }
            None
        }
        walk(&dom, NodeIdx(0), name, attrs)
    };

    let p_plain = find("p", &[("style", "letter-spacing:1px")]).expect("plain p");
    let p_warn = find("p", &[("class", "warn")]).expect("warn p");
    let special = find("span", &[("id", "special")]).expect("span#special");

    let s0 = styled.get(p_plain).unwrap();
    assert_eq!(s0.get("color"), Some("navy")); // type selector
    assert_eq!(s0.get("letter-spacing"), Some("1px")); // inline style attr
    assert_eq!(s0.get("display"), Some("block")); // UA sheet
    assert_eq!(s0.get("font-family"), Some("serif")); // UA html, inherited

    let s1 = styled.get(p_warn).unwrap();
    assert_eq!(s1.get("color"), Some("darkred")); // !important beats type rule

    let s2 = styled.get(special).unwrap();
    assert_eq!(s2.get("background-color"), Some("yellow")); // id selector
    assert_eq!(s2.get("font-weight"), Some("700")); // selector winner bold computes to700
    // …and the UA bold for span is NOT set (spans stay normal weight).
    assert_eq!(s2.get("font-style"), Some("normal"));
}
