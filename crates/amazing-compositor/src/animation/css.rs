//! CSS animation and transition lifecycle from typed computed-style input.
use super::{
    AnimationValue, CompositeMode, Direction, Easing, EffectPhase, EffectTiming, Fill,
    IterationComposite, KeyframeEffect, PlayState, PromiseSignal, PropertyKeyframe, Timing,
    WebAnimation,
};
use crate::{Compositor, EffectId, TransformId, TreeError};
use std::collections::BTreeMap;

pub type ElementId = u64;
pub type ComputedValues = BTreeMap<String, AnimationValue>;

#[derive(Debug, Clone, PartialEq)]
pub struct CssAnimationLonghands {
    pub names: Vec<String>,
    pub durations_ms: Vec<f64>,
    pub delays_ms: Vec<f64>,
    pub iteration_counts: Vec<f64>,
    pub directions: Vec<Direction>,
    pub fills: Vec<Fill>,
    pub easings: Vec<Easing>,
    pub paused: Vec<bool>,
}
impl Default for CssAnimationLonghands {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            durations_ms: vec![0.],
            delays_ms: vec![0.],
            iteration_counts: vec![1.],
            directions: vec![Direction::Normal],
            fills: vec![Fill::None],
            easings: vec![Easing::Linear],
            paused: vec![false],
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct CssTransitionLonghands {
    pub properties: Vec<String>,
    pub durations_ms: Vec<f64>,
    pub delays_ms: Vec<f64>,
    pub easings: Vec<Easing>,
    pub allow_discrete: bool,
}
impl Default for CssTransitionLonghands {
    fn default() -> Self {
        Self {
            properties: Vec::new(),
            durations_ms: vec![0.],
            delays_ms: vec![0.],
            easings: vec![Easing::Linear],
            allow_discrete: false,
        }
    }
}
fn comma_list(input: &str) -> Vec<String> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut values = Vec::new();
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                values.push(input[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    values.push(input[start..].trim().to_string());
    values
}
fn times(input: Option<&str>, default: f64) -> Vec<f64> {
    input
        .map(comma_list)
        .unwrap_or_default()
        .iter()
        .map(|value| {
            let v = value.trim().to_ascii_lowercase();
            if let Some(seconds) = v.strip_suffix("ms") {
                seconds.trim().parse::<f64>().unwrap_or(default)
            } else if let Some(seconds) = v.strip_suffix('s') {
                seconds
                    .trim()
                    .parse::<f64>()
                    .map(|n| n * 1000.)
                    .unwrap_or(default)
            } else {
                default
            }
        })
        .collect()
}
fn easings(input: Option<&str>) -> Vec<Easing> {
    input
        .map(comma_list)
        .unwrap_or_default()
        .iter()
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            if value == "linear" {
                Easing::Linear
            } else if value == "ease" {
                Easing::CubicBezier(0.25, 0.1, 0.25, 1.)
            } else if value == "ease-in" {
                Easing::CubicBezier(0.42, 0., 1., 1.)
            } else if value == "ease-out" {
                Easing::CubicBezier(0., 0., 0.58, 1.)
            } else if value == "ease-in-out" {
                Easing::CubicBezier(0.42, 0., 0.58, 1.)
            } else if let Some(body) = value
                .strip_prefix("cubic-bezier(")
                .and_then(|s| s.strip_suffix(')'))
            {
                let numbers: Vec<_> = body
                    .split(',')
                    .filter_map(|s| s.trim().parse::<f64>().ok())
                    .collect();
                if numbers.len() == 4 {
                    Easing::CubicBezier(numbers[0], numbers[1], numbers[2], numbers[3])
                } else {
                    Easing::Linear
                }
            } else if let Some(body) = value
                .strip_prefix("steps(")
                .and_then(|s| s.strip_suffix(')'))
            {
                let mut parts = body.split(',');
                let count = parts
                    .next()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .unwrap_or(1);
                let position = match parts.next().map(str::trim) {
                    Some("start" | "jump-start") => super::StepPosition::JumpStart,
                    Some("jump-none") => super::StepPosition::JumpNone,
                    Some("jump-both") => super::StepPosition::JumpBoth,
                    _ => super::StepPosition::JumpEnd,
                };
                Easing::Steps(count, position)
            } else {
                Easing::Linear
            }
        })
        .collect()
}
impl CssAnimationLonghands {
    /// Accepts `ComputedStyle::get` as a callback until CSS-14 lands in dev.
    pub fn from_computed_get<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Self {
        let names = get("animation-name").map(comma_list).unwrap_or_default();
        let durations_ms = times(get("animation-duration"), 0.);
        let delays_ms = times(get("animation-delay"), 0.);
        let iteration_counts = get("animation-iteration-count")
            .map(comma_list)
            .unwrap_or_default()
            .iter()
            .map(|s| {
                if s == "infinite" {
                    f64::INFINITY
                } else {
                    s.parse().unwrap_or(1.)
                }
            })
            .collect();
        let directions = get("animation-direction")
            .map(comma_list)
            .unwrap_or_default()
            .iter()
            .map(|s| match s.as_str() {
                "reverse" => Direction::Reverse,
                "alternate" => Direction::Alternate,
                "alternate-reverse" => Direction::AlternateReverse,
                _ => Direction::Normal,
            })
            .collect();
        let fills = get("animation-fill-mode")
            .map(comma_list)
            .unwrap_or_default()
            .iter()
            .map(|s| match s.as_str() {
                "forwards" => Fill::Forwards,
                "backwards" => Fill::Backwards,
                "both" => Fill::Both,
                _ => Fill::None,
            })
            .collect();
        let paused = get("animation-play-state")
            .map(comma_list)
            .unwrap_or_default()
            .iter()
            .map(|s| s == "paused")
            .collect();
        Self {
            names,
            durations_ms,
            delays_ms,
            iteration_counts,
            directions,
            fills,
            easings: easings(get("animation-timing-function")),
            paused,
        }
    }
}
impl CssTransitionLonghands {
    pub fn from_computed_get<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Self {
        Self {
            properties: get("transition-property")
                .map(comma_list)
                .unwrap_or_default(),
            durations_ms: times(get("transition-duration"), 0.),
            delays_ms: times(get("transition-delay"), 0.),
            easings: easings(get("transition-timing-function")),
            allow_discrete: get("transition-behavior") == Some("allow-discrete"),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedKeyframes {
    pub name: String,
    pub frames: Vec<PropertyKeyframe>,
}

/// The public CSS-14 keyframe fields, kept independent of `amazing-css` so the
/// driver can adapt its parsed rule without adding a compositor dependency.
#[derive(Debug, Clone, PartialEq)]
pub struct CssKeyframeInput {
    pub offsets: Vec<f32>,
    pub timing_function: Option<String>,
    pub declarations: Vec<(String, String)>,
}

/// Converts parsed CSS declarations with the caller's computed-value resolver.
/// Unsupported declarations are omitted, leaving their underlying value in place.
pub fn resolve_css_keyframes(
    name: impl Into<String>,
    frames: &[CssKeyframeInput],
    underlying: &ComputedValues,
    mut resolve: impl FnMut(&str, &str) -> Option<AnimationValue>,
) -> ResolvedKeyframes {
    let mut by_offset: BTreeMap<u32, PropertyKeyframe> = BTreeMap::new();
    for frame in frames {
        let easing = frame
            .timing_function
            .as_deref()
            .and_then(|value| easings(Some(value)).into_iter().next())
            .unwrap_or(Easing::Linear);
        for &offset in &frame.offsets {
            if !offset.is_finite() || !(0. ..=1.).contains(&offset) {
                continue;
            }
            let entry = by_offset
                .entry(offset.to_bits())
                .or_insert_with(|| PropertyKeyframe {
                    offset: offset as f64,
                    easing: easing.clone(),
                    values: BTreeMap::new(),
                });
            entry.easing = easing.clone();
            for (property, css_value) in &frame.declarations {
                if let Some(value) = resolve(property, css_value) {
                    entry.values.insert(property.clone(), value);
                }
            }
        }
    }
    let mut frames: Vec<_> = by_offset.into_values().collect();
    frames.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let properties: Vec<_> = frames
        .iter()
        .flat_map(|frame| frame.values.keys().cloned())
        .collect();
    for offset in [0., 1.] {
        if !frames.iter().any(|frame| frame.offset == offset) {
            frames.push(PropertyKeyframe {
                offset,
                easing: Easing::Linear,
                values: BTreeMap::new(),
            });
        }
    }
    frames.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    for property in properties {
        if let Some(base) = underlying.get(&property) {
            for frame in frames
                .iter_mut()
                .filter(|frame| frame.offset == 0. || frame.offset == 1.)
            {
                frame
                    .values
                    .entry(property.clone())
                    .or_insert_with(|| base.clone());
            }
        }
    }
    ResolvedKeyframes {
        name: name.into(),
        frames,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssAnimationEventKind {
    AnimationStart,
    AnimationIteration,
    AnimationEnd,
    AnimationCancel,
    TransitionRun,
    TransitionStart,
    TransitionEnd,
    TransitionCancel,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CssAnimationEvent {
    pub element: ElementId,
    pub kind: CssAnimationEventKind,
    pub name: String,
    pub elapsed_time_ms: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyUpdate {
    pub element: ElementId,
    pub property: String,
    pub value: AnimationValue,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationFrameOutput {
    pub compositor_updates: usize,
    pub main_thread_updates: Vec<PropertyUpdate>,
    pub events: Vec<CssAnimationEvent>,
    pub promises: Vec<(u64, PromiseSignal)>,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct CompositorBindings {
    pub transform: Option<TransformId>,
    pub opacity: Option<EffectId>,
}

#[derive(Debug, Clone)]
struct Active {
    animation: WebAnimation,
    element: ElementId,
    name: String,
    is_transition: bool,
    last_phase: Option<EffectPhase>,
    last_iteration: u64,
    from: Option<AnimationValue>,
    underlying: ComputedValues,
}

#[derive(Debug, Default)]
pub struct CssAnimationEngine {
    next_id: u64,
    animations: BTreeMap<(ElementId, usize), Active>,
    transitions: BTreeMap<(ElementId, String), Active>,
    bindings: BTreeMap<ElementId, CompositorBindings>,
    queued_events: Vec<CssAnimationEvent>,
    queued_promises: Vec<(u64, PromiseSignal)>,
}

fn drain_promises(active: &mut Active, queue: &mut Vec<(u64, PromiseSignal)>) {
    struct Collector<'a>(&'a mut Vec<(u64, PromiseSignal)>);
    impl super::AnimationHostCallbacks for Collector<'_> {
        fn animation_promise(&mut self, id: u64, signal: PromiseSignal) {
            self.0.push((id, signal));
        }
    }
    active.animation.flush_callbacks(&mut Collector(queue));
}

fn cancel_active(
    active: &mut Active,
    now_ms: f64,
    events: &mut Vec<CssAnimationEvent>,
    promises: &mut Vec<(u64, PromiseSignal)>,
) {
    let current = active.animation.current_time(Some(now_ms)).unwrap_or(0.);
    let completed = active.animation.state == PlayState::Finished
        || active.animation.effect.timing.phase(current) == Some(EffectPhase::After);
    active.animation.cancel();
    if !completed {
        events.push(event(
            active,
            if active.is_transition {
                CssAnimationEventKind::TransitionCancel
            } else {
                CssAnimationEventKind::AnimationCancel
            },
            (current - active.animation.effect.timing.timing.delay_ms)
                .max(0.)
                .min(active.animation.effect.timing.active_duration()),
        ));
    }
    drain_promises(active, promises);
}

fn list<T: Clone>(values: &[T], index: usize, default: T) -> T {
    if values.is_empty() {
        default
    } else {
        values[index % values.len()].clone()
    }
}
fn transition_index(style: &CssTransitionLonghands, property: &str) -> Option<usize> {
    // The last matching transition-property item controls this property.
    style
        .properties
        .iter()
        .rposition(|name| name == property || name == "all")
}
fn event(active: &Active, kind: CssAnimationEventKind, elapsed_time_ms: f64) -> CssAnimationEvent {
    CssAnimationEvent {
        element: active.element,
        kind,
        name: active.name.clone(),
        elapsed_time_ms,
    }
}
impl CssAnimationEngine {
    pub fn bind(&mut self, element: ElementId, binding: CompositorBindings) {
        self.bindings.insert(element, binding);
    }
    pub fn update_animations(
        &mut self,
        element: ElementId,
        style: &CssAnimationLonghands,
        rules: &[ResolvedKeyframes],
        now_ms: f64,
        underlying: &ComputedValues,
    ) {
        let old_keys: Vec<_> = self
            .animations
            .keys()
            .copied()
            .filter(|(id, _)| *id == element)
            .collect();
        for key in old_keys {
            let keep = style.names.get(key.1).is_some_and(|name| {
                self.animations
                    .get(&key)
                    .is_some_and(|active| active.name == *name)
            });
            if !keep {
                if let Some(mut active) = self.animations.remove(&key) {
                    cancel_active(
                        &mut active,
                        now_ms,
                        &mut self.queued_events,
                        &mut self.queued_promises,
                    );
                }
            }
        }
        for (index, name) in style.names.iter().enumerate() {
            if name == "none" {
                continue;
            }
            if let Some(active) = self.animations.get_mut(&(element, index)) {
                active.animation.effect.timing.timing = Timing {
                    delay_ms: list(&style.delays_ms, index, 0.),
                    duration_ms: list(&style.durations_ms, index, 0.),
                    iterations: list(&style.iteration_counts, index, 1.),
                    direction: list(&style.directions, index, Direction::Normal),
                    fill: list(&style.fills, index, Fill::None),
                    easing: list(&style.easings, index, Easing::Linear),
                };
                if let Some(rule) = rules.iter().rev().find(|rule| rule.name == *name) {
                    active.animation.effect.keyframes = rule.frames.clone();
                }
                let should_pause = list(&style.paused, index, false);
                if should_pause {
                    active.animation.pause(Some(now_ms))
                } else if active.animation.state == PlayState::Paused {
                    active.animation.play(Some(now_ms))
                }
                continue;
            }
            let Some(rule) = rules.iter().rev().find(|rule| rule.name == *name) else {
                continue;
            };
            let timing = Timing {
                delay_ms: list(&style.delays_ms, index, 0.),
                duration_ms: list(&style.durations_ms, index, 0.),
                iterations: list(&style.iteration_counts, index, 1.),
                direction: list(&style.directions, index, Direction::Normal),
                fill: list(&style.fills, index, Fill::None),
                easing: list(&style.easings, index, Easing::Linear),
            };
            let effect = KeyframeEffect {
                timing: EffectTiming {
                    timing,
                    end_delay_ms: 0.,
                    iteration_start: 0.,
                },
                keyframes: rule.frames.clone(),
                composite: CompositeMode::Replace,
                iteration_composite: IterationComposite::Replace,
            };
            let Some(mut animation) = WebAnimation::new(self.next_id, effect) else {
                continue;
            };
            self.next_id += 1;
            animation.play(Some(now_ms));
            if list(&style.paused, index, false) {
                animation.pause(Some(now_ms))
            }
            self.animations.insert(
                (element, index),
                Active {
                    animation,
                    element,
                    name: name.clone(),
                    is_transition: false,
                    last_phase: None,
                    last_iteration: 0,
                    from: None,
                    underlying: underlying.clone(),
                },
            );
        }
    }
    pub fn update_transitions(
        &mut self,
        element: ElementId,
        before: &ComputedValues,
        after: &ComputedValues,
        style: &CssTransitionLonghands,
        now_ms: f64,
    ) {
        let obsolete: Vec<_> = self
            .transitions
            .keys()
            .filter(|(id, property)| {
                *id == element
                    && transition_index(style, property)
                        .is_none_or(|index| list(&style.durations_ms, index, 0.) <= 0.)
            })
            .cloned()
            .collect();
        for key in obsolete {
            if let Some(mut active) = self.transitions.remove(&key) {
                cancel_active(
                    &mut active,
                    now_ms,
                    &mut self.queued_events,
                    &mut self.queued_promises,
                );
            }
        }
        for (property, new_value) in after {
            let Some(old_value) = before.get(property) else {
                continue;
            };
            if old_value == new_value {
                continue;
            }
            let Some(index) = transition_index(style, property) else {
                continue;
            };
            let duration = list(&style.durations_ms, index, 0.);
            let delay = list(&style.delays_ms, index, 0.);
            if !duration.is_finite() || duration <= 0. || !delay.is_finite() {
                continue;
            }
            let discrete = matches!(old_value, AnimationValue::Discrete(_))
                || matches!(new_value, AnimationValue::Discrete(_));
            if discrete && !style.allow_discrete {
                continue;
            }
            let key = (element, property.clone());
            let mut start_value = old_value.clone();
            let mut adjusted_duration = duration;
            if let Some(mut previous) = self.transitions.remove(&key) {
                if let Some(sample) = previous
                    .animation
                    .sample(Some(now_ms), &previous.underlying)
                {
                    if let Some(value) = sample.get(property) {
                        start_value = value.clone()
                    }
                }
                if previous.from.as_ref() == Some(new_value) {
                    let progress = previous
                        .animation
                        .effect
                        .timing
                        .sample(previous.animation.current_time(Some(now_ms)).unwrap_or(0.))
                        .map_or(0., |s| s.progress);
                    adjusted_duration *= progress.clamp(0.1, 1.);
                }
                cancel_active(
                    &mut previous,
                    now_ms,
                    &mut self.queued_events,
                    &mut self.queued_promises,
                );
            }
            let mut from = ComputedValues::new();
            from.insert(property.clone(), start_value.clone());
            let mut to = ComputedValues::new();
            to.insert(property.clone(), new_value.clone());
            let effect = KeyframeEffect {
                timing: EffectTiming {
                    timing: Timing {
                        delay_ms: delay,
                        duration_ms: adjusted_duration,
                        iterations: 1.,
                        direction: Direction::Normal,
                        fill: Fill::Both,
                        easing: list(&style.easings, index, Easing::Linear),
                    },
                    end_delay_ms: 0.,
                    iteration_start: 0.,
                },
                keyframes: vec![
                    PropertyKeyframe {
                        offset: 0.,
                        easing: Easing::Linear,
                        values: from,
                    },
                    PropertyKeyframe {
                        offset: 1.,
                        easing: Easing::Linear,
                        values: to,
                    },
                ],
                composite: CompositeMode::Replace,
                iteration_composite: IterationComposite::Replace,
            };
            if let Some(mut animation) = WebAnimation::new(self.next_id, effect) {
                self.next_id += 1;
                animation.play(Some(now_ms));
                let active = Active {
                    animation,
                    element,
                    name: property.clone(),
                    is_transition: true,
                    last_phase: None,
                    last_iteration: 0,
                    from: Some(start_value),
                    underlying: before.clone(),
                };
                self.queued_events
                    .push(event(&active, CssAnimationEventKind::TransitionRun, 0.));
                self.transitions.insert(key, active);
            }
        }
    }
    pub fn cancel_element(&mut self, element: ElementId, now_ms: f64) {
        let keys: Vec<_> = self
            .animations
            .keys()
            .copied()
            .filter(|(id, _)| *id == element)
            .collect();
        for key in keys {
            if let Some(mut active) = self.animations.remove(&key) {
                cancel_active(
                    &mut active,
                    now_ms,
                    &mut self.queued_events,
                    &mut self.queued_promises,
                );
            }
        }
        let keys: Vec<_> = self
            .transitions
            .keys()
            .filter(|(id, _)| *id == element)
            .cloned()
            .collect();
        for key in keys {
            if let Some(mut active) = self.transitions.remove(&key) {
                cancel_active(
                    &mut active,
                    now_ms,
                    &mut self.queued_events,
                    &mut self.queued_promises,
                );
            }
        }
    }
    pub fn tick(
        &mut self,
        now_ms: f64,
        compositor: &mut Compositor,
    ) -> Result<AnimationFrameOutput, TreeError> {
        let mut out = AnimationFrameOutput {
            events: std::mem::take(&mut self.queued_events),
            promises: std::mem::take(&mut self.queued_promises),
            ..Default::default()
        };
        for active in self
            .animations
            .values_mut()
            .chain(self.transitions.values_mut())
        {
            let current = active.animation.current_time(Some(now_ms));
            let phase = current.and_then(|time| active.animation.effect.timing.phase(time));
            if let Some(phase) = phase {
                if active.last_phase != Some(EffectPhase::Active)
                    && active.last_phase != Some(EffectPhase::After)
                    && phase != EffectPhase::Before
                {
                    out.events.push(event(
                        active,
                        if active.is_transition {
                            CssAnimationEventKind::TransitionStart
                        } else {
                            CssAnimationEventKind::AnimationStart
                        },
                        if active.is_transition {
                            0.
                        } else {
                            (-active.animation.effect.timing.timing.delay_ms).max(0.)
                        },
                    ));
                }
                if !active.is_transition && phase != EffectPhase::Before {
                    let duration = active.animation.effect.timing.timing.duration_ms;
                    let iterations = active.animation.effect.timing.timing.iterations;
                    if duration > 0. && duration.is_finite() && iterations.is_finite() {
                        let elapsed = (current.unwrap_or(0.)
                            - active.animation.effect.timing.timing.delay_ms)
                            .max(0.)
                            / duration;
                        let crossed = (elapsed.floor() as u64)
                            .min((iterations.ceil() as u64).saturating_sub(1));
                        let first = if active.last_phase.is_none() {
                            crossed.saturating_add(1)
                        } else {
                            active.last_iteration.saturating_add(1)
                        };
                        for iteration in first..=crossed {
                            out.events.push(event(
                                active,
                                CssAnimationEventKind::AnimationIteration,
                                iteration as f64 * duration,
                            ));
                        }
                        active.last_iteration = crossed;
                    }
                }
                if active.last_phase != Some(EffectPhase::After) && phase == EffectPhase::After {
                    out.events.push(event(
                        active,
                        if active.is_transition {
                            CssAnimationEventKind::TransitionEnd
                        } else {
                            CssAnimationEventKind::AnimationEnd
                        },
                        active.animation.effect.timing.active_duration(),
                    ));
                }
                active.last_phase = Some(phase);
            }
            if let Some(values) = active.animation.sample(Some(now_ms), &active.underlying) {
                let binding = self
                    .bindings
                    .get(&active.element)
                    .copied()
                    .unwrap_or_default();
                for (property, value) in values {
                    match (&*property, &value) {
                        ("transform", AnimationValue::Transform(matrix))
                            if binding.transform.is_some() =>
                        {
                            let id = binding.transform.unwrap();
                            if compositor
                                .trees()
                                .transform(id)
                                .is_some_and(|node| node.matrix != *matrix)
                            {
                                compositor.set_transform(id, *matrix)?;
                                out.compositor_updates += 1
                            }
                        }
                        ("opacity", AnimationValue::Opacity(opacity))
                            if binding.opacity.is_some() =>
                        {
                            let id = binding.opacity.unwrap();
                            if compositor
                                .trees()
                                .effect(id)
                                .is_some_and(|node| node.opacity != *opacity)
                            {
                                compositor.set_opacity(id, *opacity)?;
                                out.compositor_updates += 1
                            }
                        }
                        _ => out.main_thread_updates.push(PropertyUpdate {
                            element: active.element,
                            property,
                            value,
                        }),
                    }
                }
            }
            drain_promises(active, &mut out.promises);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlendMode, PropertyTrees};
    fn pair(property: &str, from: AnimationValue, to: AnimationValue) -> Vec<PropertyKeyframe> {
        vec![
            PropertyKeyframe {
                offset: 0.,
                easing: Easing::Linear,
                values: BTreeMap::from([(property.into(), from)]),
            },
            PropertyKeyframe {
                offset: 1.,
                easing: Easing::Linear,
                values: BTreeMap::from([(property.into(), to)]),
            },
        ]
    }
    #[test]
    fn computed_get_uses_css14_canonical_lists() {
        let properties = BTreeMap::from([
            ("animation-name", "fade, move"),
            ("animation-duration", "100ms, 2s"),
            ("animation-delay", "-50ms"),
            ("animation-iteration-count", "2, infinite"),
            (
                "animation-timing-function",
                "cubic-bezier(0.1, 0.2, 0.3, 0.4), steps(4, jump-end)",
            ),
        ]);
        let style = CssAnimationLonghands::from_computed_get(|name| properties.get(name).copied());
        assert_eq!(style.names, ["fade", "move"]);
        assert_eq!(style.durations_ms, [100., 2000.]);
        assert_eq!(style.delays_ms, [-50.]);
        assert_eq!(style.iteration_counts, [2., f64::INFINITY]);
        assert!(matches!(style.easings[0], Easing::CubicBezier(..)));
        assert_eq!(
            style.easings[1],
            Easing::Steps(4, super::super::StepPosition::JumpEnd)
        );
    }
    #[test]
    fn css14_keyframes_expand_selectors_and_keep_last_declaration() {
        let frames = [
            CssKeyframeInput {
                offsets: vec![1., 0.],
                timing_function: Some("steps(2, end)".into()),
                declarations: vec![("opacity".into(), "0.2".into())],
            },
            CssKeyframeInput {
                offsets: vec![1.],
                timing_function: None,
                declarations: vec![("opacity".into(), "1".into())],
            },
        ];
        let rule = resolve_css_keyframes(
            "fade",
            &frames,
            &ComputedValues::new(),
            |property, value| {
                (property == "opacity").then(|| AnimationValue::Opacity(value.parse().unwrap()))
            },
        );
        assert_eq!(
            rule.frames
                .iter()
                .map(|frame| frame.offset)
                .collect::<Vec<_>>(),
            [0., 1.]
        );
        assert_eq!(
            rule.frames[1].values["opacity"],
            AnimationValue::Opacity(1.)
        );
        assert_eq!(
            rule.frames[0].easing,
            Easing::Steps(2, super::super::StepPosition::JumpEnd)
        );
        let middle = [CssKeyframeInput {
            offsets: vec![0.5],
            timing_function: None,
            declarations: vec![("opacity".into(), "0.2".into())],
        }];
        let base = BTreeMap::from([("opacity".into(), AnimationValue::Opacity(1.))]);
        let rule = resolve_css_keyframes("pulse", &middle, &base, |_, value| {
            Some(AnimationValue::Opacity(value.parse().unwrap()))
        });
        assert_eq!(rule.frames.len(), 3);
        assert_eq!(
            rule.frames[0].values["opacity"],
            AnimationValue::Opacity(1.)
        );
        assert_eq!(
            rule.frames[2].values["opacity"],
            AnimationValue::Opacity(1.)
        );
    }
    #[test]
    fn skipped_iterations_and_cancel_promises_are_ordered() {
        let mut compositor = Compositor::new(PropertyTrees::new(), Vec::new(), 0).unwrap();
        let mut engine = CssAnimationEngine::default();
        let style = CssAnimationLonghands {
            names: vec!["first".into(), "second".into()],
            durations_ms: vec![100.],
            iteration_counts: vec![3.],
            ..Default::default()
        };
        let rules: Vec<_> = style
            .names
            .iter()
            .map(|name| ResolvedKeyframes {
                name: name.clone(),
                frames: pair(
                    "opacity",
                    AnimationValue::Opacity(0.),
                    AnimationValue::Opacity(1.),
                ),
            })
            .collect();
        engine.update_animations(2, &style, &rules, 0., &ComputedValues::new());
        engine.tick(0., &mut compositor).unwrap();
        let output = engine.tick(300., &mut compositor).unwrap();
        let kinds: Vec<_> = output
            .events
            .iter()
            .map(|event| (event.name.as_str(), event.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("first", CssAnimationEventKind::AnimationIteration),
                ("first", CssAnimationEventKind::AnimationIteration),
                ("first", CssAnimationEventKind::AnimationEnd),
                ("second", CssAnimationEventKind::AnimationIteration),
                ("second", CssAnimationEventKind::AnimationIteration),
                ("second", CssAnimationEventKind::AnimationEnd),
            ]
        );
        engine.cancel_element(2, 300.);
        let canceled = engine.tick(300., &mut compositor).unwrap();
        assert!(canceled.events.is_empty());

        engine.update_animations(2, &style, &rules, 400., &ComputedValues::new());
        engine.cancel_element(2, 450.);
        let canceled = engine.tick(450., &mut compositor).unwrap();
        assert_eq!(
            canceled
                .events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [
                CssAnimationEventKind::AnimationCancel,
                CssAnimationEventKind::AnimationCancel
            ]
        );
        assert_eq!(
            canceled
                .promises
                .iter()
                .filter(|(_, signal)| *signal == PromiseSignal::FinishedRejected)
                .count(),
            2
        );
    }
    #[test]
    fn pinned_wpt_negative_delay_iteration_events() {
        // WPT c2a2fce: css/css-animations/animation-iteration-event-001.html.
        // A 1s, -4s, seven-iteration animation emits only the 5s and 6s
        // iteration events after it starts, followed by animationend.
        let mut compositor = Compositor::new(PropertyTrees::new(), Vec::new(), 0).unwrap();
        let mut engine = CssAnimationEngine::default();
        let style = CssAnimationLonghands {
            names: vec!["anim".into()],
            durations_ms: vec![1000.],
            delays_ms: vec![-4000.],
            iteration_counts: vec![7.],
            ..Default::default()
        };
        let rule = ResolvedKeyframes {
            name: "anim".into(),
            frames: pair(
                "opacity",
                AnimationValue::Opacity(0.),
                AnimationValue::Opacity(1.),
            ),
        };
        engine.update_animations(1, &style, &[rule], 0., &ComputedValues::new());
        let start = engine.tick(0., &mut compositor).unwrap();
        assert_eq!(
            start
                .events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [CssAnimationEventKind::AnimationStart]
        );
        assert_eq!(start.events[0].elapsed_time_ms, 4000.);
        let fifth = engine.tick(1000., &mut compositor).unwrap();
        let sixth = engine.tick(2000., &mut compositor).unwrap();
        assert_eq!(fifth.events[0].elapsed_time_ms, 5000.);
        assert_eq!(sixth.events[0].elapsed_time_ms, 6000.);
        let end = engine.tick(3000., &mut compositor).unwrap();
        assert_eq!(
            end.events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [CssAnimationEventKind::AnimationEnd]
        );
    }
    #[test]
    fn css_animation_events_and_compositor_opacity_tick() {
        let mut trees = PropertyTrees::new();
        let effect = trees
            .add_effect(EffectId(0), 0., BlendMode::Normal, None)
            .unwrap();
        let mut compositor = Compositor::new(trees, Vec::new(), 0).unwrap();
        let mut engine = CssAnimationEngine::default();
        engine.bind(
            9,
            CompositorBindings {
                opacity: Some(effect),
                ..Default::default()
            },
        );
        let style = CssAnimationLonghands {
            names: vec!["fade".into()],
            durations_ms: vec![100.],
            delays_ms: vec![10.],
            fills: vec![Fill::Both],
            ..Default::default()
        };
        let rule = ResolvedKeyframes {
            name: "fade".into(),
            frames: pair(
                "opacity",
                AnimationValue::Opacity(0.),
                AnimationValue::Opacity(1.),
            ),
        };
        engine.update_animations(9, &style, &[rule], 0., &ComputedValues::new());
        let before = engine.tick(0., &mut compositor).unwrap();
        assert!(before.events.is_empty());
        assert_eq!(before.promises.len(), 1);
        let start = engine.tick(10., &mut compositor).unwrap();
        assert_eq!(start.events[0].kind, CssAnimationEventKind::AnimationStart);
        let middle = engine.tick(60., &mut compositor).unwrap();
        assert!(middle.main_thread_updates.is_empty());
        assert_eq!(compositor.trees().effect(effect).unwrap().opacity, 0.5);
        assert_eq!(middle.compositor_updates, 1);
        let end = engine.tick(110., &mut compositor).unwrap();
        assert_eq!(end.events[0].kind, CssAnimationEventKind::AnimationEnd);
        assert!(end
            .promises
            .iter()
            .any(|(_, signal)| *signal == PromiseSignal::FinishedResolved));
    }
    #[test]
    fn pinned_wpt_transition_run_start_end_and_main_thread_value() {
        // WPT c2a2fce: css/css-transitions/event-dispatch.tentative.html,
        // Idle -> Before -> Active -> After event ordering.
        let mut compositor = Compositor::new(PropertyTrees::new(), Vec::new(), 0).unwrap();
        let mut engine = CssAnimationEngine::default();
        let before = BTreeMap::from([("margin-left".into(), AnimationValue::LengthPx(0.))]);
        let after = BTreeMap::from([("margin-left".into(), AnimationValue::LengthPx(100.))]);
        let style = CssTransitionLonghands {
            properties: vec!["margin-left".into()],
            durations_ms: vec![100.],
            delays_ms: vec![50.],
            easings: vec![Easing::Linear],
            allow_discrete: false,
        };
        engine.update_transitions(4, &before, &after, &style, 0.);
        let run = engine.tick(0., &mut compositor).unwrap();
        assert_eq!(
            run.events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [CssAnimationEventKind::TransitionRun]
        );
        let start = engine.tick(50., &mut compositor).unwrap();
        assert_eq!(start.events[0].kind, CssAnimationEventKind::TransitionStart);
        let middle = engine.tick(100., &mut compositor).unwrap();
        assert_eq!(
            middle.main_thread_updates[0].value,
            AnimationValue::LengthPx(50.)
        );
        let end = engine.tick(150., &mut compositor).unwrap();
        assert_eq!(end.events[0].kind, CssAnimationEventKind::TransitionEnd);
        assert_eq!(end.events[0].elapsed_time_ms, 100.);
    }
    #[test]
    fn transition_last_matching_item_and_removed_property_cancel() {
        let mut compositor = Compositor::new(PropertyTrees::new(), Vec::new(), 0).unwrap();
        let mut engine = CssAnimationEngine::default();
        let before = BTreeMap::from([("left".into(), AnimationValue::LengthPx(0.))]);
        let after = BTreeMap::from([("left".into(), AnimationValue::LengthPx(100.))]);
        let style = CssTransitionLonghands {
            properties: vec!["all".into(), "left".into()],
            durations_ms: vec![10., 100.],
            ..Default::default()
        };
        engine.update_transitions(1, &before, &after, &style, 0.);
        engine.tick(0., &mut compositor).unwrap();
        let middle = engine.tick(50., &mut compositor).unwrap();
        assert_eq!(
            middle.main_thread_updates[0].value,
            AnimationValue::LengthPx(50.)
        );
        engine.update_transitions(
            1,
            &after,
            &after,
            &CssTransitionLonghands {
                properties: vec!["none".into()],
                ..Default::default()
            },
            50.,
        );
        let canceled = engine.tick(50., &mut compositor).unwrap();
        assert_eq!(
            canceled.events[0].kind,
            CssAnimationEventKind::TransitionCancel
        );
        assert!(canceled
            .promises
            .iter()
            .any(|(_, signal)| *signal == PromiseSignal::FinishedRejected));
    }
}
