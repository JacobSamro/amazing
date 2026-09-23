use amazing_compositor::{
    resolve_css_keyframes, AnimationValue, CompositeMode, ComputedValues, CssAnimationLonghands,
    CssKeyframeInput, CssTransitionLonghands,
};
use amazing_css::{keyframes_rules, parse_stylesheet, ComputedStyle};

#[test]
fn css14_computed_lists_and_keyframe_records_feed_compositor() {
    let mut style = ComputedStyle::default();
    style.set("animation", "pulse 2s linear 100ms 3 alternate both paused");
    style.set("animation-composition", "add");
    style.set("animation-timeline", "scroll(y)");
    style.set(
        "transition",
        "opacity .2s ease-out .1s, display 1s linear allow-discrete",
    );
    let animations = CssAnimationLonghands::from_computed_get(|name| style.get(name));
    let transitions = CssTransitionLonghands::from_computed_get(|name| style.get(name));
    assert_eq!(animations.names, ["pulse"]);
    assert_eq!(animations.durations_ms, [2000.]);
    assert_eq!(animations.delays_ms, [100.]);
    assert_eq!(animations.compositions, [CompositeMode::Add]);
    assert_eq!(animations.timelines, ["scroll(y)"]);
    assert_eq!(transitions.properties, ["opacity", "display"]);
    assert_eq!(transitions.allow_discrete, [false, true]);

    let sheet =
        parse_stylesheet("@keyframes pulse { from, 50% { opacity: .2 } to { opacity: 1 } }");
    let rules = keyframes_rules(&[sheet]);
    let input: Vec<_> = rules[0]
        .frames
        .iter()
        .map(|frame| CssKeyframeInput {
            offsets: frame.offsets.clone(),
            timing_function: frame.timing_function.clone(),
            declarations: frame.declarations.clone(),
        })
        .collect();
    let underlying = ComputedValues::from([("opacity".into(), AnimationValue::Opacity(0.))]);
    let resolved = resolve_css_keyframes(&rules[0].name, &input, &underlying, |property, value| {
        (property == "opacity").then(|| AnimationValue::Opacity(value.parse().unwrap()))
    });
    assert_eq!(resolved.frames.len(), 3);
    assert_eq!(resolved.frames[1].offset, 0.5);
    assert_eq!(
        resolved.frames[2].values["opacity"],
        AnimationValue::Opacity(1.)
    );
}
