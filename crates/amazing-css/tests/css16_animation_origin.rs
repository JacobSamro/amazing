use amazing_css::{
    compute, compute_with_animations, parse_stylesheet, AnimatedOrigin, AnimatedValues, ElementStyleSource,
};
use amazing_dom::{Dom, NodeIdx};

fn fixture(css: &str) -> (Dom, amazing_css::parser::Stylesheet, NodeIdx, NodeIdx) {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "span");
    (dom, parse_stylesheet(css), parent, child)
}

#[test]
fn animation_origin_beats_author_normal_layers_and_inline() {
    let (mut dom, sheet, parent, child) =
        fixture("@layer a,b; @layer a { div { color: red } } @layer b { div { color: green } }");
    dom.set_attribute(parent, "style", "color:blue").unwrap();
    let mut animated = AnimatedValues::default();
    assert!(animated.insert(parent, "color", "rgb(255, 165, 0)"));
    let styles = compute_with_animations(&dom, &[sheet.clone()], &animated);
    assert_eq!(
        styles.get(parent).unwrap().get("color"),
        Some("rgb(255, 165, 0)")
    );
    assert_eq!(
        styles.get(child).unwrap().get("color"),
        Some("rgb(255, 165, 0)")
    );
    assert_eq!(
        styles.source(parent, "color"),
        Some(ElementStyleSource::Animation)
    );
    assert!(!styles.get(parent).unwrap().is_author_specified("color"));
}

#[test]
fn important_author_beats_animation_but_transition_beats_important() {
    let (dom, sheet, parent, _) = fixture("div { opacity:.8!important }");
    let mut animated = AnimatedValues::default();
    assert!(animated.insert(parent, "opacity", "0.25"));
    let styles = compute_with_animations(&dom, &[sheet.clone()], &animated);
    assert_eq!(styles.get(parent).unwrap().get("opacity"), Some("0.8"));
    assert_eq!(
        styles.source(parent, "opacity"),
        Some(ElementStyleSource::Author)
    );
    assert!(animated.insert_transition(parent, "opacity", "0.5"));
    let styles = compute_with_animations(&dom, &[sheet.clone()], &animated);
    assert_eq!(styles.get(parent).unwrap().get("opacity"), Some("0.5"));
    assert_eq!(
        styles.source(parent, "opacity"),
        Some(ElementStyleSource::Transition)
    );
    animated.remove(parent, "opacity", AnimatedOrigin::Transition);
    let styles = compute_with_animations(&dom, &[sheet], &animated);
    assert_eq!(styles.get(parent).unwrap().get("opacity"), Some("0.8"));
}

#[test]
fn author_revert_exposes_animation_origin() {
    let (dom, sheet, parent, _) = fixture("div { color:red!important; color:revert!important }");
    let mut animated = AnimatedValues::default();
    assert!(animated.insert(parent, "color", "blue"));
    let styles = compute_with_animations(&dom, &[sheet], &animated);
    assert_eq!(styles.get(parent).unwrap().get("color"), Some("blue"));
    assert_eq!(
        styles.source(parent, "color"),
        Some(ElementStyleSource::Animation)
    );
}

#[test]
fn typed_samples_are_validated_and_plain_compute_remains_stable() {
    let (dom, sheet, parent, _) = fixture("div { width:10px; opacity:.8 }");
    let mut animated = AnimatedValues::default();
    assert!(!animated.insert(parent, "width", "bad"));
    assert!(!animated.insert(parent, "unknown-property", "3px"));
    assert!(!animated.insert(parent, "width", "var(--x)"));
    assert!(!animated.insert(parent, "color", "inherit"));
    assert!(animated.insert(parent, "width", "12.5px"));
    assert!(animated.get(parent, "width").is_some());
    let with_animation = compute_with_animations(&dom, &[sheet.clone()], &animated);
    assert_eq!(
        with_animation.get(parent).unwrap().get("width"),
        Some("12.5px")
    );
    assert_eq!(
        with_animation.get(parent).unwrap().used_length_px("width"),
        Some(12.5)
    );
    let normal = compute(&dom, &[sheet]);
    assert_eq!(normal.get(parent).unwrap().get("width"), Some("10px"));
}
