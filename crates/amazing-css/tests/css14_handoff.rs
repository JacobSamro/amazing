use amazing_css::animation::{
    sample, AnimationDirection, AnimationEasing, AnimationFill, AnimationFrameValue,
    AnimationKind, AnimationMainThreadEventKind, AnimationProperty, AnimationPropertyUpdate,
    AnimationTiming, CompositorAnimation,
};
use amazing_css::{compute, parse_stylesheet, AnimatedValues};
use amazing_dom::{Dom, NodeIdx};

fn timing() -> AnimationTiming {
    AnimationTiming {
        duration_ms: 1000.0, delay_ms: 100.0, iterations: 1.0,
        direction: AnimationDirection::Normal, fill: AnimationFill::Both,
        easing: AnimationEasing::Linear,
    }
}
fn animation(css: &str, property: AnimationProperty, box_size: Option<[f32; 2]>) -> CompositorAnimation {
    let sheet = parse_stylesheet(css);
    CompositorAnimation::from_keyframes(
        7, NodeIdx(2), 3, &sheet.keyframes[0], property, timing(), 1000.0, box_size,
    ).unwrap_or_else(|| panic!("unresolved handoff: {:?}", sheet.keyframes))
}

#[test]
fn shared_opacity_sampler_ownership_style_read_and_finish_event() {
    let animation = animation("@keyframes fade { from { opacity:.2 } to { opacity:.8 } }",
        AnimationProperty::Opacity, None);
    assert_eq!(sample(&animation, 1099.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.2)));
    let middle = sample(&animation, 1600.0).unwrap();
    assert_eq!(middle.value, Some(AnimationFrameValue::Opacity(0.5)));
    assert_eq!(sample(&animation, 1600.0), Some(middle.clone()));
    assert!(sample(&animation, 2100.0).unwrap().finished);

    let mut values = AnimatedValues::default();
    assert!(values.handoff(animation));
    assert!(values.compositor_owned(NodeIdx(2), "opacity"));
    assert!(!values.insert(NodeIdx(2), "opacity", "0.4"));
    assert!(values.get(NodeIdx(2), "opacity").is_none());
    assert_eq!(values.read_owned_style(NodeIdx(2), "opacity", 1600.0).unwrap().css, "0.5");
    let mut events = Vec::new();
    let update = AnimationPropertyUpdate {
        id: 7, node: NodeIdx(2), generation: 3, property: AnimationProperty::Opacity,
        time_ms: 2100.0, cancelled: false,
    };
    assert!(!values.commit_property_update(AnimationPropertyUpdate { generation: 4, ..update }, |event| events.push(event)));
    assert!(!values.commit_property_update(AnimationPropertyUpdate { time_ms: 1500.0, ..update }, |event| events.push(event)));
    assert!(values.commit_property_update(update, |event| events.push(event)));
    assert_eq!(values.get(NodeIdx(2), "opacity").unwrap().css, "0.8");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, AnimationMainThreadEventKind::AnimationEnd);
}

#[test]
fn transform_sampler_composes_origin_and_rejects_mixed_or_unresolved_handoffs() {
    let css = "@keyframes move { from { transform:translate(0%,0px) rotate(0deg) scale(1) } to { transform:translate(20%,0px) rotate(90deg) scale(2) } }";
    let animation = animation(css, AnimationProperty::Transform, Some([100.0, 40.0]));
    let sampled = sample(&animation, 1600.0).unwrap().value.unwrap();
    let AnimationFrameValue::Transform(value) = sampled else { panic!("transform sample") };
    let matrix = value.matrix;
    let x = matrix[0][0] * 50.0 + matrix[0][1] * 20.0 + matrix[0][3];
    let y = matrix[1][0] * 50.0 + matrix[1][1] * 20.0 + matrix[1][3];
    assert!((x - 60.0).abs() < 0.001);
    assert!((y - 20.0).abs() < 0.001);
    assert!(value.to_css().contains("rotate("));

    let sheet = parse_stylesheet(css);
    assert!(CompositorAnimation::from_keyframes(7, NodeIdx(2), 3, &sheet.keyframes[0],
        AnimationProperty::Transform, timing(), 1000.0, None).is_none());
    let mixed = parse_stylesheet("@keyframes mixed { from { opacity:0; color:red } to { opacity:1; color:blue } }");
    assert!(CompositorAnimation::from_keyframes(7, NodeIdx(2), 3, &mixed.keyframes[0],
        AnimationProperty::Opacity, timing(), 1000.0, None).is_none());
}

#[test]
fn transition_completion_and_mutation_cancellation_are_main_thread_events() {
    let mut transition = animation("@keyframes fade { from { opacity:.2 } to { opacity:.8 } }",
        AnimationProperty::Opacity, None);
    transition.kind = AnimationKind::Transition;
    transition.name = "opacity".into();
    let mut values = AnimatedValues::default();
    assert!(values.handoff(transition.clone()));
    let mut events = Vec::new();
    let resumed = values.cancel_for_mutation(NodeIdx(2), 3, 1500.0, |event| events.push(event));
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].id, 7);
    assert_eq!(events[0].kind, AnimationMainThreadEventKind::TransitionCancel);
    assert!(!values.compositor_owned(NodeIdx(2), "opacity"));
    assert!(values.get(NodeIdx(2), "opacity").is_some());
    assert!(values.handoff(transition));
    assert!(values.commit_property_update(AnimationPropertyUpdate {
        id: 7, node: NodeIdx(2), generation: 3, property: AnimationProperty::Opacity,
        time_ms: 2100.0, cancelled: false,
    }, |event| events.push(event)));
    assert_eq!(events[1].kind, AnimationMainThreadEventKind::TransitionEnd);
    assert_eq!(values.get(NodeIdx(2), "opacity").unwrap().css, "0.8");
}

#[test]
fn direction_fill_and_easing_are_sampled_by_one_function() {
    let mut animation = animation("@keyframes fade { from { opacity:0 } to { opacity:1 } }",
        AnimationProperty::Opacity, None);
    animation.timing.direction = AnimationDirection::Alternate;
    animation.timing.iterations = 2.0;
    animation.timing.fill = AnimationFill::Forwards;
    assert_eq!(sample(&animation, 1100.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.0)));
    assert_eq!(sample(&animation, 2100.0).unwrap().value, Some(AnimationFrameValue::Opacity(1.0)));
    assert_eq!(sample(&animation, 2600.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.5)));
    assert_eq!(sample(&animation, 3100.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.0)));
    animation.timing.easing = AnimationEasing::Steps(2, false);
    assert_eq!(sample(&animation, 1350.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.0)));
    assert_eq!(sample(&animation, 1600.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.5)));
}

#[test]
fn multi_property_animation_emits_one_end_event_after_both_commits() {
    let css = "@keyframes both { from { opacity:.2; transform:translate(0px,0px) } to { opacity:.8; transform:translate(20px,0px) } }";
    let mut values = AnimatedValues::default();
    assert!(values.handoff(animation(css, AnimationProperty::Opacity, None)));
    assert!(values.handoff(animation(css, AnimationProperty::Transform, Some([100.0, 40.0]))));
    let mut events = Vec::new();
    for property in [AnimationProperty::Opacity, AnimationProperty::Transform] {
        assert!(values.commit_property_update(AnimationPropertyUpdate {
            id: 7, node: NodeIdx(2), generation: 3, property,
            time_ms: 2100.0, cancelled: false,
        }, |event| events.push(event)));
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, AnimationMainThreadEventKind::AnimationEnd);
    assert!(values.get(NodeIdx(2), "transform").is_some());
}

#[test]
fn computed_timing_and_transition_builder_use_css_longhands() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let node = dom.create_element(html, "div");
    let sheet = parse_stylesheet("div { transition-duration: 2s; transition-delay: 100ms; transition-timing-function: linear; animation-duration: 1s; animation-delay: -100ms; animation-iteration-count: 2; animation-direction: alternate; animation-fill-mode: both; animation-timing-function: ease-in }");
    let styled = compute(&dom, &[sheet]);
    let style = styled.get(node).unwrap();
    let transition_timing = AnimationTiming::from_computed_style(style, 0, AnimationKind::Transition).unwrap();
    assert_eq!(transition_timing.duration_ms, 2000.0);
    assert_eq!(transition_timing.delay_ms, 100.0);
    let transition = CompositorAnimation::for_transition(9, node, 4,
        AnimationProperty::Opacity, "0.2", "0.8", transition_timing, 1000.0, None).unwrap();
    assert_eq!(sample(&transition, 2100.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.5)));
    let animation_timing = AnimationTiming::from_computed_style(style, 0, AnimationKind::Animation).unwrap();
    assert_eq!(animation_timing.delay_ms, -100.0);
    assert_eq!(animation_timing.iterations, 2.0);
    assert_eq!(animation_timing.direction, AnimationDirection::Alternate);
    assert_eq!(animation_timing.fill, AnimationFill::Both);
}

#[test]
fn keyframe_easing_applies_within_its_segment() {
    let css = "@keyframes steps { 0% { opacity:0; animation-timing-function:steps(2,end) } 50% { opacity:.5 } 100% { opacity:1 } }";
    let mut animation = animation(css, AnimationProperty::Opacity, None);
    animation.timing.easing = AnimationEasing::CubicBezier(0.42, 0.0, 1.0, 1.0);
    assert_eq!(sample(&animation, 1350.0).unwrap().value, Some(AnimationFrameValue::Opacity(0.25)));
}

#[test]
fn transition_from_none_uses_identity_functions() {
    let transition = CompositorAnimation::for_transition(10, NodeIdx(2), 3,
        AnimationProperty::Transform, "none", "translate(20px,0px)", timing(),
        1000.0, Some([100.0, 40.0])).unwrap();
    let value = sample(&transition, 1600.0).unwrap().value.unwrap();
    let AnimationFrameValue::Transform(value) = value else { panic!("transform") };
    assert!((value.matrix[0][3] - 10.0).abs() < 0.001);
}
