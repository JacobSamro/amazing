use amazing_css::{ComputedStyle, keyframes_rules, parse_stylesheet};

#[test]
fn keyframes_extract_last_named_rule_and_valid_frames() {
    let first = parse_stylesheet("@keyframes pulse { from { opacity: 0 } }");
    let second = parse_stylesheet(
        "@layer motion { @keyframes pulse { from, 50%, to { opacity: .5; width: 10px !important; nonsense: 3; animation-timing-function: steps(2, end) } banana { opacity: 1 } 100% { opacity: 1 } } }",
    );
    let rules = keyframes_rules(&[first, second]);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name, "pulse");
    assert_eq!(rules[0].frames.len(), 2);
    assert_eq!(rules[0].frames[0].offsets, vec![0.0, 0.5, 1.0]);
    assert_eq!(
        rules[0].frames[0].declarations,
        vec![("opacity".into(), ".5".into())]
    );
    assert!(
        rules[0].frames[0]
            .timing_function
            .as_deref()
            .unwrap()
            .starts_with("steps(")
    );
    assert_eq!(rules[0].frames[1].offsets, vec![1.0]);
}

#[test]
fn animation_and_transition_shorthand_expand_to_computed_lists() {
    let mut style = ComputedStyle::default();
    style.set(
        "animation",
        "slide 2s ease-in 150ms 3 alternate both paused, fade 1s linear",
    );
    assert_eq!(style.get("animation-name"), Some("slide, fade"));
    assert_eq!(style.get("animation-duration"), Some("2s, 1s"));
    assert_eq!(style.get("animation-delay"), Some("0.15s, 0s"));
    assert_eq!(style.get("animation-iteration-count"), Some("3, 1"));
    assert_eq!(style.get("animation-direction"), Some("alternate, normal"));
    assert_eq!(style.get("animation-fill-mode"), Some("both, none"));
    assert_eq!(style.get("animation-play-state"), Some("paused, running"));
    style.set(
        "transition",
        "opacity .2s ease-out .1s, transform 1s linear allow-discrete",
    );
    assert_eq!(style.get("transition-property"), Some("opacity, transform"));
    assert_eq!(style.get("transition-duration"), Some("0.2s, 1s"));
    assert_eq!(style.get("transition-delay"), Some("0.1s, 0s"));
    assert_eq!(
        style.get("transition-behavior"),
        Some("normal, allow-discrete")
    );
}

#[test]
fn invalid_shorthand_does_not_replace_existing_longhands() {
    let mut style = ComputedStyle::default();
    style.set("animation-duration", "2s");
    style.set("animation", "broken -2s");
    assert_eq!(style.get("animation-duration"), Some("2s"));
    style.set("transition-duration", "3s");
    style.set("transition", "opacity -1s");
    assert_eq!(style.get("transition-duration"), Some("3s"));
}

#[test]
fn cascade_exposes_defaults_shorthand_reset_and_longhand_canonicalization() {
    use amazing_css::compute;
    use amazing_dom::{Dom, NodeIdx};
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(root, "span");
    let css = "div { animation-duration: 200ms, 1s; transition-delay: -100ms } \
               span { animation: fade 2s linear; animation-delay: 100ms, -50ms; \
                      transition: opacity 300ms ease-in 50ms; }";
    let styles = compute(&dom, &[parse_stylesheet(css)]);
    let parent = styles.get(root).unwrap();
    assert_eq!(parent.get("animation-duration"), Some("0.2s, 1s"));
    assert_eq!(parent.get("transition-delay"), Some("-0.1s"));
    let child = styles.get(child).unwrap();
    assert_eq!(child.get("animation-name"), Some("fade"));
    assert_eq!(child.get("animation-duration"), Some("2s"));
    assert_eq!(child.get("animation-delay"), Some("0.1s, -0.05s"));
    assert_eq!(child.get("transition-duration"), Some("0.3s"));
    assert_eq!(child.get("transition-timing-function"), Some("ease-in"));
}

#[test]
fn shorthand_variables_are_expanded_after_substitution() {
    use amazing_css::compute;
    use amazing_dom::{Dom, NodeIdx};
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    let css = "div { --motion: glow 250ms linear 50ms; animation: var(--motion); \
               --change: opacity 100ms ease; transition: var(--change) }";
    let styles = compute(&dom, &[parse_stylesheet(css)]);
    let style = styles.get(node).unwrap();
    assert_eq!(style.get("animation-name"), Some("glow"));
    assert_eq!(style.get("animation-duration"), Some("0.25s"));
    assert_eq!(style.get("transition-property"), Some("opacity"));
    assert_eq!(style.get("transition-duration"), Some("0.1s"));
}

#[test]
fn pinned_wpt_invalid_shorthand_examples_are_atomic() {
    // css-animations/parsing/animation-invalid.html and
    // css-transitions/parsing/transition-invalid.html at c2a2fce.
    use amazing_css::compute;
    use amazing_dom::{Dom, NodeIdx};
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    for value in ["1s 2s 3s", "-1s -2s", "steps(1) steps(2)"] {
        let css = format!(
            "div {{ animation-duration: 4s; animation: {value}; transition-duration: 4s; transition: {value} }}"
        );
        let styles = compute(&dom, &[parse_stylesheet(&css)]);
        let style = styles.get(node).unwrap();
        assert_eq!(style.get("animation-duration"), Some("4s"), "{value}");
        assert_eq!(style.get("transition-duration"), Some("4s"), "{value}");
    }
    for value in ["none top", "initial 1s"] {
        let css = format!("div {{ transition-duration: 4s; transition: {value} }}");
        let styles = compute(&dom, &[parse_stylesheet(&css)]);
        assert_eq!(
            styles.get(node).unwrap().get("transition-duration"),
            Some("4s"),
            "{value}"
        );
    }
}

#[test]
fn keyframe_custom_properties_and_deferred_values_survive_parsing() {
    let sheet = parse_stylesheet(
        "@keyframes fill { 25% { --accent: red; color: var(--accent); width: bogus; opacity: .5 !important } }",
    );
    let frame = &keyframes_rules(&[sheet])[0].frames[0];
    assert_eq!(frame.offsets, vec![0.25]);
    assert_eq!(
        frame.declarations,
        vec![
            ("--accent".into(), "red".into()),
            ("color".into(), "var(--accent)".into())
        ]
    );
}
