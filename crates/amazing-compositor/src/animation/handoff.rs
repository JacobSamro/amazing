//! Compositor clock for CSS-owned opacity and transform animations.
//! CSS supplies a pure sampler for its resolved animation model. The clock
//! never computes style or asks a paint source for pixels.

use crate::{AnimationValue, BoundProperty, BoundPropertyChange, BoundPropertyValue, Compositor,
    NodePropertyKey, PropertyBindingError, PropertyUpdate};
use amazing_css::animation::{
    self as css_animation, AnimationFrameValue, AnimationProperty, AnimationPropertyUpdate,
    CompositorAnimation as CssCompositorAnimation,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandoffSample {
    Pending,
    Active(BoundPropertyValue),
    Finished(BoundPropertyValue),
    Canceled(BoundPropertyValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffEnd {
    Finished,
    Canceled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HandoffCompletion {
    pub key: NodePropertyKey,
    pub update: PropertyUpdate,
    pub end: HandoffEnd,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HandoffTickOutput {
    pub compositor_updates: usize,
    /// Sent once to the driver for CSS cascade commit and end/cancel events.
    pub completions: Vec<HandoffCompletion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    InvalidTime,
    DuplicateProperty,
    UnsupportedProperty,
    UnknownAnimation,
    Binding(PropertyBindingError),
}

struct Active<A> {
    key: NodePropertyKey,
    animation: A,
    last_value: Option<BoundPropertyValue>,
}

/// The sampler remains generic for deterministic clock tests; production uses
/// the specialized CSS-14 adapter below.
pub struct CompositorAnimationTimeline<A> {
    active: Vec<Active<A>>,
}

fn css_value(value: AnimationFrameValue) -> BoundPropertyValue {
    match value {
        AnimationFrameValue::Opacity(value) => BoundPropertyValue::Opacity(value),
        AnimationFrameValue::Transform(value) => BoundPropertyValue::Transform(crate::Matrix4(value.matrix)),
    }
}

fn css_sample(animation: &CssCompositorAnimation, now_ms: f64) -> HandoffSample {
    let Some(sample) = css_animation::sample(animation, now_ms) else {
        return HandoffSample::Pending;
    };
    match (sample.finished, sample.value) {
        (false, Some(value)) => HandoffSample::Active(css_value(value)),
        (true, Some(value)) => HandoffSample::Finished(css_value(value)),
        // CSS will clear an unfilled value at completion. Keep the last
        // resolved keyframe until the driver consumes the completion.
        (true, None) => HandoffSample::Finished(css_value(
            animation.keyframes.last().expect("validated animation").value.clone())),
        (false, None) => HandoffSample::Pending,
    }
}

/// Production timeline: CSS owns timing/keyframes and both threads call its
/// pure sampler. Completion messages carry CSS's identity and timeline time.
pub type CssCompositorTimeline = CompositorAnimationTimeline<CssCompositorAnimation>;
impl CompositorAnimationTimeline<CssCompositorAnimation> {
    pub fn install_css(&mut self, animation: CssCompositorAnimation) -> Result<(), HandoffError> {
        if !animation.valid() { return Err(HandoffError::UnsupportedProperty); }
        let property = match animation.property {
            AnimationProperty::Opacity => BoundProperty::Opacity,
            AnimationProperty::Transform => BoundProperty::Transform,
        };
        let key = NodePropertyKey {
            node: animation.node.0, generation: animation.generation, property,
        };
        self.install(key, animation)
    }

    pub fn tick_css(
        &mut self, now_ms: f64, compositor: &mut Compositor,
    ) -> Result<(HandoffTickOutput, Vec<AnimationPropertyUpdate>), HandoffError> {
        let pending = self.active.iter().map(|active| (active.key, active.animation.clone()))
            .collect::<Vec<_>>();
        let output = self.tick(now_ms, compositor, css_sample)?;
        let updates = output.completions.iter().filter_map(|completion| {
            let animation = &pending.iter().find(|(key, _)| *key == completion.key)?.1;
            Some(AnimationPropertyUpdate {
                id: animation.id, node: animation.node,
                generation: completion.key.generation, property: animation.property,
                time_ms: now_ms, cancelled: completion.end == HandoffEnd::Canceled,
            })
        }).collect();
        Ok((output, updates))
    }

    pub fn cancel_css(
        &mut self, key: NodePropertyKey, now_ms: f64, compositor: &mut Compositor,
    ) -> Result<(HandoffCompletion, AnimationPropertyUpdate), HandoffError> {
        if !now_ms.is_finite() { return Err(HandoffError::InvalidTime); }
        let animation = self.active.iter().find(|active| active.key == key)
            .ok_or(HandoffError::UnknownAnimation)?.animation.clone();
        let value = css_animation::sample(&animation, now_ms)
            .and_then(|sample| sample.value)
            .unwrap_or_else(|| animation.keyframes[0].value.clone());
        let completion = self.cancel(key, css_value(value), compositor)?;
        let update = AnimationPropertyUpdate {
            id: animation.id, node: animation.node, generation: animation.generation,
            property: animation.property, time_ms: now_ms, cancelled: true,
        };
        Ok((completion, update))
    }
}

impl<A> Default for CompositorAnimationTimeline<A> {
    fn default() -> Self { Self { active: Vec::new() } }
}

fn property_update(key: NodePropertyKey, value: BoundPropertyValue) -> Result<PropertyUpdate, HandoffError> {
    let (property, value) = match (key.property, value) {
        (BoundProperty::Opacity, BoundPropertyValue::Opacity(value)) =>
            ("opacity", AnimationValue::Opacity(value)),
        (BoundProperty::Transform, BoundPropertyValue::Transform(value)) =>
            ("transform", AnimationValue::Transform(value)),
        _ => return Err(HandoffError::UnsupportedProperty),
    };
    Ok(PropertyUpdate { element: u64::from(key.node), property: property.into(), value })
}

impl<A> CompositorAnimationTimeline<A> {
    pub fn active_count(&self) -> usize { self.active.len() }
    pub fn active_keys(&self) -> impl Iterator<Item = NodePropertyKey> + '_ {
        self.active.iter().map(|active| active.key)
    }

    pub fn install(&mut self, key: NodePropertyKey, animation: A) -> Result<(), HandoffError> {
        if !matches!(key.property, BoundProperty::Opacity | BoundProperty::Transform) {
            return Err(HandoffError::UnsupportedProperty);
        }
        if self.active.iter().any(|active| active.key == key) {
            return Err(HandoffError::DuplicateProperty);
        }
        self.active.push(Active { key, animation, last_value: None });
        Ok(())
    }

    /// One vsync sample. A running animation returns no main-thread update;
    /// finish/cancel returns exactly one final value for CSS to commit.
    pub fn tick(
        &mut self,
        now_ms: f64,
        compositor: &mut Compositor,
        sample: impl Fn(&A, f64) -> HandoffSample,
    ) -> Result<HandoffTickOutput, HandoffError> {
        if !now_ms.is_finite() { return Err(HandoffError::InvalidTime); }
        let samples = self.active.iter().map(|active| sample(&active.animation, now_ms))
            .collect::<Vec<_>>();
        let mut changes = Vec::new();
        let mut completions = Vec::new();
        for (active, result) in self.active.iter().zip(&samples) {
            let (value, end) = match *result {
                HandoffSample::Pending => continue,
                HandoffSample::Active(value) => (value, None),
                HandoffSample::Finished(value) => (value, Some(HandoffEnd::Finished)),
                HandoffSample::Canceled(value) => (value, Some(HandoffEnd::Canceled)),
            };
            if active.last_value != Some(value) {
                changes.push(BoundPropertyChange { key: active.key, value });
            }
            if let Some(end) = end {
                completions.push(HandoffCompletion {
                    key: active.key, update: property_update(active.key, value)?, end,
                });
            }
        }
        compositor.apply_bound_property_changes(&changes).map_err(HandoffError::Binding)?;
        for (active, result) in self.active.iter_mut().zip(&samples) {
            match *result {
                HandoffSample::Pending => {}
                HandoffSample::Active(value) | HandoffSample::Finished(value)
                | HandoffSample::Canceled(value) => active.last_value = Some(value),
            }
        }
        self.active.retain(|active| !completions.iter().any(|done| done.key == active.key));
        Ok(HandoffTickOutput { compositor_updates: changes.len(), completions })
    }

    /// A DOM or style mutation can revoke the handoff before its next sample.
    /// The driver resumes the normal main-thread animation path afterward.
    pub fn cancel(
        &mut self,
        key: NodePropertyKey,
        value: BoundPropertyValue,
        compositor: &mut Compositor,
    ) -> Result<HandoffCompletion, HandoffError> {
        let index = self.active.iter().position(|active| active.key == key)
            .ok_or(HandoffError::UnknownAnimation)?;
        let update = property_update(key, value)?;
        compositor.apply_bound_property_changes(&[BoundPropertyChange { key, value }])
            .map_err(HandoffError::Binding)?;
        self.active.remove(index);
        Ok(HandoffCompletion { key, update, end: HandoffEnd::Canceled })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlendMode, BoundPropertyNode, CompositingReasons, EffectId, Matrix4,
        PaintChunk, PaintSource, Presenter, PropertyTreeState, PropertyTrees, Rect,
        ReferenceCpuPresenter, TransformId};
    use amazing_paint::Surface;
    use amazing_css::animation::{
        AnimationDirection, AnimationEasing, AnimationFill, AnimationFrameValue,
        AnimationProperty, AnimationTiming, CompositorAnimation,
    };
    use amazing_dom::NodeIdx;
    use std::{cell::Cell, ops::Range};

    fn timing() -> AnimationTiming {
        AnimationTiming {
            duration_ms: 100., delay_ms: 0., iterations: 1.,
            direction: AnimationDirection::Normal, fill: AnimationFill::Forwards,
            easing: AnimationEasing::Linear,
        }
    }

    struct Source<'a>(&'a Cell<usize>);
    impl PaintSource for Source<'_> {
        fn item_count(&self) -> usize { 1 }
        fn rasterize(&self, _: Range<usize>, _: Rect, output: &mut Surface) {
            self.0.set(self.0.get() + 1);
            output.clear([30, 90, 150, 255]);
        }
    }

    #[test]
    fn opacity_transition_ticks_at_vsync_without_paint_and_hands_back_final_value() {
        let mut trees = PropertyTrees::new();
        let effect = trees.add_effect(EffectId(0), 1., BlendMode::Normal, None).unwrap();
        let chunk = PaintChunk { items: 0..1,
            state: PropertyTreeState { effect, ..Default::default() },
            bounds: Rect::new(0., 0., 64., 64.), reasons: CompositingReasons::WILL_CHANGE };
        let mut compositor = crate::Compositor::new(trees, vec![chunk.clone()], 1).unwrap();
        let key = NodePropertyKey { node: 7, generation: 3, property: BoundProperty::Opacity };
        compositor.bind_property_nodes(&[(key, BoundPropertyNode::Opacity(effect))]).unwrap();
        let calls = Cell::new(0);
        let first = compositor.build_frame(&Source(&calls), 64, 64).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        presenter.present(&first).unwrap();
        let mut timeline = CompositorAnimationTimeline::default();
        timeline.install_css(CompositorAnimation::for_transition(1, NodeIdx(7), 3,
            AnimationProperty::Opacity, "1", "0", timing(), 0., None).unwrap()).unwrap();
        for vsync in [0., 16., 32., 50.] {
            let (output, updates) = timeline.tick_css(vsync, &mut compositor).unwrap();
            assert!(output.completions.is_empty(), "no main-thread work at {vsync}ms");
            assert!(updates.is_empty());
            let frame = compositor.build_cached_frame(64, 64).unwrap();
            assert!(frame.resources.uploads.is_empty());
            assert_eq!(calls.get(), 1);
            if vsync == 50. {
                let actual = presenter.present(&frame).unwrap();
                let mut full_trees = PropertyTrees::new();
                let id = full_trees.add_effect(EffectId(0), 0.5, BlendMode::Normal, None).unwrap();
                let mut full = crate::Compositor::new(full_trees, vec![PaintChunk {
                    state: PropertyTreeState { effect: id, ..Default::default() }, ..chunk.clone()
                }], 1).unwrap();
                let expected_frame = full.build_frame(&Source(&calls), 64, 64).unwrap();
                let expected = ReferenceCpuPresenter::default().present(&expected_frame).unwrap();
                assert_eq!(actual.pixels, expected.pixels);
            }
        }
        let (output, updates) = timeline.tick_css(100., &mut compositor).unwrap();
        assert_eq!(output.completions.len(), 1);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].id, 1);
        assert_eq!(output.completions[0].end, HandoffEnd::Finished);
        assert_eq!(output.completions[0].update.property, "opacity");
        assert_eq!(output.completions[0].update.value, AnimationValue::Opacity(0.));
        assert_eq!(timeline.active_count(), 0);
        assert!(timeline.tick_css(116., &mut compositor).unwrap().0.completions.is_empty());
    }

    #[test]
    fn transform_animation_uses_same_pure_sample_and_cancel_returns_value_once() {
        let mut trees = PropertyTrees::new();
        let transform = trees.add_transform(TransformId(0), Matrix4::IDENTITY,
            (0., 0., 0.), false).unwrap();
        let mut compositor = crate::Compositor::new(trees, Vec::new(), 0).unwrap();
        let key = NodePropertyKey { node: 9, generation: 4, property: BoundProperty::Transform };
        compositor.bind_property_nodes(&[(key, BoundPropertyNode::Transform(transform))]).unwrap();
        let mut timeline = CompositorAnimationTimeline::default();
        let animation = CompositorAnimation::for_transition(2, NodeIdx(9), 4,
            AnimationProperty::Transform, "none", "translate(20px,0px)",
            timing(), 0., Some([20., 20.])).unwrap();
        let expected = css_animation::sample(&animation, 50.).unwrap().value.unwrap();
        timeline.install_css(animation).unwrap();
        assert!(timeline.tick_css(50., &mut compositor).unwrap().0.completions.is_empty());
        let AnimationFrameValue::Transform(transform_value) = expected else { panic!() };
        let matrix = Matrix4(transform_value.matrix);
        assert_eq!(compositor.trees().transform(transform).unwrap().matrix, matrix);
        let (canceled, css_update) = timeline.cancel_css(key, 50., &mut compositor).unwrap();
        assert_eq!(canceled.end, HandoffEnd::Canceled);
        assert_eq!(canceled.update.value, AnimationValue::Transform(matrix));
        assert!(css_update.cancelled);
        assert_eq!(timeline.active_count(), 0);
        assert_eq!(timeline.cancel_css(key, 50., &mut compositor),
            Err(HandoffError::UnknownAnimation));
        let scroll = NodePropertyKey { property: BoundProperty::Scroll, ..key };
        assert_eq!(timeline.install(scroll, CompositorAnimation::for_transition(
            3, NodeIdx(9), 4, AnimationProperty::Opacity, "0", "1",
            timing(), 0., None).unwrap()),
            Err(HandoffError::UnsupportedProperty));
    }
}
