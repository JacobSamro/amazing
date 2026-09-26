//! Traverse every COMP-15 fixture's inline effect declarations through the CSS
//! parser and retained-value adapter. This measures adoption, NOT pixel parity.
use amazing_compositor::{resolve_css_effect_delta, CssEffectError, EffectOperations, Rect};
use amazing_dom::NodeIdx;
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(||
        "crates/amazing-compositor/fixtures/comp15".into());
    let mut paths: Vec<_> = std::fs::read_dir(root).unwrap().map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "html")).collect();
    paths.sort();
    assert!(!paths.is_empty(), "empty corpus");
    println!("fixture,resolved,fallback,failures");
    for path in paths {
        let dom = amazing_html::parse(&std::fs::read_to_string(&path).unwrap());
        let mut queue = vec![NodeIdx(0)];
        let (mut resolved, mut fallback, mut failures) = (0, 0, 0);
        while let Some(node) = queue.pop() {
            queue.extend(dom.children(node));
            let Some(style) = dom.attrs_of(node).and_then(|attrs|
                attrs.iter().find(|(key, _)| key == "style").map(|(_, v)| v)) else { continue; };
            for declaration in amazing_css::parser::parse_declarations(style) {
                let name = declaration.property.as_str();
                if !matches!(name, "filter" | "backdrop-filter" | "mix-blend-mode" |
                    "isolation" | "clip-path") && !name.starts_with("mask-") { continue; }
                let supported = matches!(name, "filter" | "backdrop-filter" | "mix-blend-mode" | "isolation");
                let result = resolve_css_effect_delta(&EffectOperations::default(),
                    &[(declaration.property.clone(), declaration.value.clone())],
                    Some(Rect::new(0., 0., 64., 64.)), [0, 0, 0, 255], |_| None);
                match result {
                    Ok(_) if supported => resolved += 1,
                    Err(CssEffectError::UnsupportedProperty) if !supported => fallback += 1,
                    other => { failures += 1; eprintln!("{}: {}={}: {other:?}",
                        path.display(), declaration.property, declaration.value); }
                }
            }
        }
        println!("{},{resolved},{fallback},{failures}", path.file_name().unwrap().to_string_lossy());
        assert!(resolved + fallback > 0, "fixture has no effect cases");
        assert_eq!(failures, 0, "{}", path.display());
    }
}
