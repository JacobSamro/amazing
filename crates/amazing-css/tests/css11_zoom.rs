use amazing_css::{compute, parse_stylesheet, values::ComputedValue, ElementStyleSource as Source};
use amazing_dom::{Dom, NodeIdx};

fn tree() -> (Dom, NodeIdx, NodeIdx, NodeIdx) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let parent = dom.create_element(html, "div");
    let child = dom.create_element(parent, "span");
    (dom, html, parent, child)
}

#[test]
fn zoom_computes_number_and_percentage_and_multiplies_through_ancestors() {
    let (dom, html, parent, child) = tree();
    let sheet = parse_stylesheet("html{zoom:2} div{zoom:150%;width:10px} span{width:25%;font-size:12px}");
    let styles = compute(&dom, &[sheet]);
    let h = styles.get(html).unwrap();
    let p = styles.get(parent).unwrap();
    let c = styles.get(child).unwrap();
    assert_eq!(h.get("zoom"), Some("2"));
    assert_eq!(p.get("zoom"), Some("1.5"));
    assert_eq!(p.typed("zoom"), Some(&ComputedValue::Number(1.5)));
    assert_eq!(c.get("zoom"), Some("1"));
    assert_eq!(h.effective_zoom(), 2.0);
    assert_eq!(p.effective_zoom(), 3.0);
    assert_eq!(c.effective_zoom(), 3.0);
    assert_eq!(p.get("width"), Some("10px"));
    assert_eq!(p.scale_used_length(10.0), 30.0);
    assert_eq!(p.used_length_px("width"), Some(30.0));
    assert_eq!(c.get("width"), Some("25%"));
    assert_eq!(c.used_length_px("width"), None);
    assert_eq!(c.get("font-size"), Some("12px"));
    assert_eq!(c.scale_used_length(12.0), 36.0);
    assert_eq!(c.used_length_px("font-size"), Some(36.0));
    assert_eq!(styles.source(child, "zoom"), Some(Source::Initial));
}

#[test]
fn zoom_zero_is_computed_zero_but_effective_one_and_invalid_values_drop() {
    let (dom, _, parent, child) = tree();
    for input in ["0", "0%"] {
        let styles = compute(&dom, &[parse_stylesheet(&format!("div{{zoom:{input}}} span{{zoom:2}}"))]);
        assert_eq!(styles.get(parent).unwrap().get("zoom"), Some("0"));
        assert_eq!(styles.get(parent).unwrap().effective_zoom(), 1.0);
        assert_eq!(styles.get(child).unwrap().effective_zoom(), 2.0);
    }
    for input in ["-1", "-5%", "NaN", "normal", "1px"] {
        let styles = compute(&dom, &[parse_stylesheet(&format!("div{{zoom:2;zoom:{input}}}"))]);
        assert_eq!(styles.get(parent).unwrap().get("zoom"), Some("2"), "{input}");
    }
}

#[test]
fn zoom_is_not_implicitly_inherited_but_explicit_inherit_and_vars_work() {
    let (dom, html, parent, child) = tree();
    let styles = compute(&dom, &[parse_stylesheet("html{zoom:2}div{zoom:inherit}span{--factor:50%;zoom:var(--factor)}")]);
    assert_eq!(styles.get(parent).unwrap().get("zoom"), Some("2"));
    assert_eq!(styles.get(parent).unwrap().effective_zoom(), 4.0);
    assert_eq!(styles.get(child).unwrap().get("zoom"), Some("0.5"));
    assert_eq!(styles.get(child).unwrap().effective_zoom(), 2.0);
    assert_eq!(styles.source(html, "zoom"), Some(Source::Author));
    assert_eq!(styles.source(parent, "zoom"), Some(Source::Author));
    assert_eq!(styles.source(child, "zoom"), Some(Source::Author));
}
