//! Driver bridge from computed CSS to the retained CSS animation clock.
use std::collections::{BTreeMap, BTreeSet};

use amazing_compositor::{
    AnimationColor, AnimationFrameOutput, AnimationValue, ColorSpace, Compositor,
    ComputedValues, CssAnimationEngine, CssAnimationLonghands, CssKeyframeInput,
    CssTransitionLonghands, PropertyTrees, PropertyUpdate, resolve_css_keyframes,
};
use amazing_css::{AnimatedValues, StyledDom, Stylesheet};
use amazing_dom::{Dom, NodeIdx, NodeKind};

pub(crate) struct AnimationScene {
    engine: CssAnimationEngine,
    // The CSS animation engine needs a compositor even when all values still
    // take the main-thread path. Property bindings are installed by the live
    // retained presenter when it owns the matching scene.
    compositor: Compositor,
    previous: BTreeMap<u64, ComputedValues>,
    animated: AnimatedValues,
    active_transitions: BTreeSet<(u64, String)>,
}

impl AnimationScene {
    pub(crate) fn new() -> Self {
        Self {
            engine: CssAnimationEngine::default(),
            compositor: Compositor::new(PropertyTrees::default(), Vec::new(), 0)
                .expect("empty animation compositor"),
            previous: BTreeMap::new(),
            animated: AnimatedValues::default(),
            active_transitions: BTreeSet::new(),
        }
    }

    /// Refresh base styles after the script task, then sample one document
    /// timeline checkpoint. The caller applies sampled values before paint and
    /// dispatches events after releasing its DOM borrow.
    pub(crate) fn frame(
        &mut self,
        dom: &Dom,
        sheets: &[Stylesheet],
        now_ms: f64,
    ) -> Result<AnimationFrameOutput, String> {
        let styled = amazing_css::compute(dom, sheets);
        self.frame_from_styles(dom, &styled, sheets, now_ms)
    }

    pub(crate) fn animated(&self) -> &AnimatedValues { &self.animated }

    fn frame_from_styles(
        &mut self,
        dom: &Dom,
        styled: &StyledDom,
        sheets: &[Stylesheet],
        now_ms: f64,
    ) -> Result<AnimationFrameOutput, String> {
        let keyframes = amazing_css::keyframes_rules(sheets);
        let mut next = BTreeMap::new();
        for index in 0..dom.node_count() {
            let node = NodeIdx(index as u32);
            if !matches!(dom.get(node).kind, NodeKind::Element { .. }) {
                continue;
            }
            let Some(style) = styled.get(node) else { continue };
            let element = index as u64;
            let underlying = computed_values(style);
            let animation_style = CssAnimationLonghands::from_computed_get(|name| style.get(name));
            let transition_style = CssTransitionLonghands::from_computed_get(|name| style.get(name));
            let rules = keyframes.iter().map(|rule| {
                let frames = rule.frames.iter().map(|frame| CssKeyframeInput {
                    offsets: frame.offsets.clone(),
                    timing_function: frame.timing_function.clone(),
                    declarations: frame.declarations.clone(),
                }).collect::<Vec<_>>();
                resolve_css_keyframes(&rule.name, &frames, &underlying, parse_value)
            }).collect::<Vec<_>>();
            self.engine.update_animations(element, &animation_style, &rules, now_ms, &underlying);
            if let Some(before) = self.previous.get(&element) {
                self.engine.update_transitions(element, before, &underlying, &transition_style, now_ms);
            }
            next.insert(element, underlying);
        }
        for element in self.previous.keys().filter(|element| !next.contains_key(element)) {
            self.engine.cancel_element(*element, now_ms);
        }
        self.previous = next;
        let output = self.engine.tick(now_ms, &mut self.compositor)
            .map_err(|error| format!("animation property update failed: {error:?}"))?;
        self.animated = sampled_values(&output, &mut self.active_transitions);
        Ok(output)
    }
}

fn sampled_values(
    output: &AnimationFrameOutput,
    active_transitions: &mut BTreeSet<(u64, String)>,
) -> AnimatedValues {
    use amazing_compositor::CssAnimationEventKind as Kind;
    let mut ended = BTreeSet::new();
    for event in &output.events {
        let key = (event.element, event.name.clone());
        match event.kind {
            Kind::TransitionRun | Kind::TransitionStart => { active_transitions.insert(key); }
            Kind::TransitionEnd | Kind::TransitionCancel => { ended.insert(key); }
            _ => {}
        }
    }
    // The compositor emits animation samples before transition samples. Keep
    // both cascade origins while a transition is active; CSS resolves their
    // precedence. PropertyUpdate does not yet carry its origin, so the last
    // sample for an active transition is its transition sample.
    let mut groups: BTreeMap<(u64, String), Vec<&PropertyUpdate>> = BTreeMap::new();
    for update in &output.main_thread_updates {
        groups.entry((update.element, update.property.clone())).or_default().push(update);
    }
    let mut animated = AnimatedValues::default();
    for (key, updates) in groups {
        let node = NodeIdx(key.0 as u32);
        let animation = if active_transitions.contains(&key) && updates.len() > 1 {
            updates.get(updates.len() - 2)
        } else if !active_transitions.contains(&key) {
            updates.last()
        } else { None };
        if let Some(update) = animation
            && let Some(css) = css_value(&update.value) {
                animated.insert(node, &key.1, &css);
        }
        if active_transitions.contains(&key) && !ended.contains(&key)
            && let Some(update) = updates.last()
            && let Some(css) = css_value(&update.value) {
                animated.insert_transition(node, &key.1, &css);
        }
    }
    for key in ended { active_transitions.remove(&key); }
    animated
}

fn computed_values(style: &amazing_css::ComputedStyle) -> ComputedValues {
    style.iter().filter_map(|(name, value)| {
        parse_value(name, value).map(|value| (name.to_owned(), value))
    }).collect()
}

fn parse_value(property: &str, css: &str) -> Option<AnimationValue> {
    let css = css.trim();
    if property == "opacity" {
        return css.parse::<f32>().ok().filter(|v| v.is_finite())
            .map(|v| AnimationValue::Opacity(v.clamp(0., 1.)));
    }
    if property.ends_with("color") || property == "color" {
        let [r, g, b, a] = amazing_css::color::parse_css_color(css)?;
        return Some(AnimationValue::Color(AnimationColor::rgba8(r, g, b, a, ColorSpace::Srgb)));
    }
    if let Some(px) = css.strip_suffix("px") {
        return px.trim().parse::<f32>().ok().filter(|v| v.is_finite())
            .map(AnimationValue::LengthPx);
    }
    if let Some(percent) = css.strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().filter(|v| v.is_finite())
            .map(AnimationValue::Percentage);
    }
    if let Ok(value) = css.parse::<f32>() {
        if value.is_finite() {
            return Some(AnimationValue::Number(value));
        }
    }
    None
}

fn css_value(value: &AnimationValue) -> Option<String> {
    match value {
        AnimationValue::LengthPx(value) => Some(format!("{value}px")),
        AnimationValue::Percentage(value) => Some(format!("{value}%")),
        AnimationValue::Number(value) | AnimationValue::Opacity(value) => Some(value.to_string()),
        AnimationValue::Color(color) if color.space == ColorSpace::Srgb => {
            let rgba = color.components.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
            Some(format!("rgba({}, {}, {}, {})", rgba[0], rgba[1], rgba[2], rgba[3] as f32 / 255.))
        }
        AnimationValue::Discrete(value) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css14_styles_drive_engine_samples_and_transition_lifecycle() {
        let dom = amazing_html::parse("<html><body><div id='box'></div></body></html>");
        let sheet = amazing_css::parse_stylesheet("@keyframes fade { from { opacity: 0 } to { opacity: 1 } } #box { opacity: 1; animation: fade 100ms linear both; transition: width 100ms linear }");
        let box_node = (0..dom.node_count()).map(|n| NodeIdx(n as u32))
            .find(|node| dom.get_attribute_ns(*node, None, "id") == Some("box"))
            .unwrap();
        let mut scene = AnimationScene::new();
        let start = scene.frame(&dom, &[sheet.clone()], 0.).unwrap();
        assert!(start.events.iter().any(|event| event.element == box_node.0 as u64
            && event.kind == amazing_compositor::CssAnimationEventKind::AnimationStart));
        let middle = scene.frame(&dom, &[sheet.clone()], 50.).unwrap();
        assert!(middle.main_thread_updates.iter().any(|update| update.element == box_node.0 as u64
            && update.property == "opacity" && matches!(update.value, AnimationValue::Opacity(v) if (v - 0.5).abs() < 0.01)));
        let sampled = amazing_css::compute_with_animations(&dom, &[sheet.clone()], scene.animated());
        assert_eq!(sampled.source(box_node, "opacity"), Some(amazing_css::ElementStyleSource::Animation));
        let end = scene.frame(&dom, &[sheet], 100.).unwrap();
        assert!(end.events.iter().any(|event| event.kind == amazing_compositor::CssAnimationEventKind::AnimationEnd));
    }

    #[test]
    fn computed_style_change_starts_transition_and_samples_intermediate_width() {
        let mut dom = amazing_html::parse("<html><body><div id='box'></div></body></html>");
        let box_node = (0..dom.node_count()).map(|n| NodeIdx(n as u32))
            .find(|node| dom.get_attribute_ns(*node, None, "id") == Some("box"))
            .unwrap();
        let sheet = amazing_css::parse_stylesheet("#box { width: 8px; transition: width 100ms linear } #box.wide { width: 32px }");
        let mut scene = AnimationScene::new();
        scene.frame(&dom, &[sheet.clone()], 0.).unwrap();
        dom.set_attribute(box_node, "class", "wide").unwrap();
        let started = scene.frame(&dom, &[sheet.clone()], 10.).unwrap();
        assert!(started.events.iter().any(|event| event.element == box_node.0 as u64
            && event.kind == amazing_compositor::CssAnimationEventKind::TransitionRun
            && event.name == "width"));
        let middle = scene.frame(&dom, &[sheet.clone()], 60.).unwrap();
        assert!(middle.main_thread_updates.iter().any(|update| update.element == box_node.0 as u64
            && update.property == "width" && matches!(update.value, AnimationValue::LengthPx(v) if (v - 20.).abs() < 0.1)));
        let sampled = amazing_css::compute_with_animations(&dom, &[sheet.clone()], scene.animated());
        assert_eq!(sampled.source(box_node, "width"), Some(amazing_css::ElementStyleSource::Transition));
        let ended = scene.frame(&dom, &[sheet], 110.).unwrap();
        assert!(ended.events.iter().any(|event| event.kind == amazing_compositor::CssAnimationEventKind::TransitionEnd));
        assert!(scene.animated().get(box_node, "width").is_none());
    }

    #[test]
    fn transition_sample_preserves_animation_origin_under_it() {
        use amazing_compositor::{CssAnimationEvent, CssAnimationEventKind};
        let mut active = BTreeSet::new();
        let output = AnimationFrameOutput {
            events: vec![CssAnimationEvent { element: 7, kind: CssAnimationEventKind::TransitionRun,
                name: "opacity".into(), elapsed_time_ms: 0. }],
            main_thread_updates: vec![
                PropertyUpdate { element: 7, property: "opacity".into(), value: AnimationValue::Opacity(0.25) },
                PropertyUpdate { element: 7, property: "opacity".into(), value: AnimationValue::Opacity(0.75) },
            ],
            ..Default::default()
        };
        let values = sampled_values(&output, &mut active);
        assert_eq!(values.get(NodeIdx(7), "opacity").unwrap().css, "0.75");
        let dom = amazing_html::parse("<body><div></div></body>");
        let node = (0..dom.node_count()).map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("div")).unwrap();
        let mut values = AnimatedValues::default();
        values.insert(node, "opacity", "0.25");
        values.insert_transition(node, "opacity", "0.75");
        let styled = amazing_css::compute_with_animations(&dom, &[], &values);
        assert_eq!(styled.get(node).unwrap().get("opacity"), Some("0.75"));
        values.remove(node, "opacity", amazing_css::AnimatedOrigin::Transition);
        let styled = amazing_css::compute_with_animations(&dom, &[], &values);
        assert_eq!(styled.get(node).unwrap().get("opacity"), Some("0.25"));
    }
}
