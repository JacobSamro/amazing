//! Scroll and view progress timelines sampled from retained scroll nodes.
use super::{
    AnimationFrameOutput, AnimationValue, CompositorBindings, ComputedValues, ElementId,
    KeyframeEffect, PropertyUpdate,
};
use crate::{Compositor, PropertyTrees, Rect, ScrollId, TreeError};

/// CSS block/inline axes are resolved to a physical axis by the style owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineAxis {
    X,
    Y,
}

/// `scroll()` source resolution (root, nearest, self) is done by the caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollProgressTimeline {
    pub source: ScrollId,
    pub axis: TimelineAxis,
}

/// Subject bounds are unscrolled paint/layout coordinates in the source's
/// content space. Geometry is refreshed after layout, not on each scroll tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewProgressTimeline {
    pub source: ScrollId,
    pub axis: TimelineAxis,
    pub subject: Rect,
    pub start_inset: f32,
    pub end_inset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDrivenTimeline {
    Scroll(ScrollProgressTimeline),
    View(ViewProgressTimeline),
}

impl ScrollDrivenTimeline {
    /// Returns `None` for a missing source or coincident timeline endpoints.
    /// Progress may be outside 0..1 for a view timeline so effect fill applies.
    pub fn progress(&self, trees: &PropertyTrees) -> Option<f64> {
        let (source, axis) = match self {
            Self::Scroll(t) => (t.source, t.axis),
            Self::View(t) => (t.source, t.axis),
        };
        let node = trees.scroll(source)?;
        let (offset, port_origin, port_size, contents_size) = match axis {
            TimelineAxis::X => (
                node.offset.0,
                node.container.x,
                node.container.w,
                node.contents.w,
            ),
            TimelineAxis::Y => (
                node.offset.1,
                node.container.y,
                node.container.h,
                node.contents.h,
            ),
        };
        if ![offset, port_origin, port_size, contents_size]
            .iter()
            .all(|v| v.is_finite())
        {
            return None;
        }
        match self {
            Self::Scroll(_) => {
                let range = contents_size - port_size;
                (range > 0.).then(|| (offset / range).clamp(0., 1.) as f64)
            }
            Self::View(t) => {
                if t.subject.empty() || !t.start_inset.is_finite() || !t.end_inset.is_finite() {
                    return None;
                }
                let (subject_start, subject_size) = match axis {
                    TimelineAxis::X => (t.subject.x, t.subject.w),
                    TimelineAxis::Y => (t.subject.y, t.subject.h),
                };
                let visible_size = port_size - t.start_inset - t.end_inset;
                if visible_size <= 0. {
                    return None;
                }
                let entry = subject_start - port_origin - port_size + t.end_inset;
                let range = subject_size + visible_size;
                let progress = (offset - entry) / range;
                progress.is_finite().then_some(progress as f64)
            }
        }
    }
}

/// A scroll-driven effect uses timeline percentage as its active interval.
/// Only transform and opacity reach property trees; other values are returned
/// for the style owner to apply at the next rendering update.
#[derive(Debug, Clone)]
pub struct ScrollDrivenAnimation {
    pub element: ElementId,
    pub timeline: ScrollDrivenTimeline,
    pub effect: KeyframeEffect,
    pub underlying: ComputedValues,
    pub bindings: CompositorBindings,
}

impl ScrollDrivenAnimation {
    pub fn sample(&self, trees: &PropertyTrees) -> Option<ComputedValues> {
        let progress = self.timeline.progress(trees)?;
        if !self.effect.valid() {
            return None;
        }
        let mut effect = self.effect.clone();
        // CSS animation-duration:auto maps the active interval to the full
        // timeline. A zero duration in this native seam denotes that default.
        // With infinite iterations, auto has zero active duration.
        if effect.timing.timing.duration_ms == 0. {
            if effect.timing.timing.iterations.is_infinite() {
                return None;
            }
            effect.timing.timing.duration_ms = 1.;
        }
        // Time-based animation-delay does not apply to a scroll-driven
        // timeline. Adding it here offsets EffectTiming's subtraction.
        let local_time = effect.timing.timing.delay_ms + progress * effect.timing.active_duration();
        effect.sample(local_time, &self.underlying)
    }

    pub fn tick(&self, compositor: &mut Compositor) -> Result<AnimationFrameOutput, TreeError> {
        // An inactive timeline or an unfilled phase removes the effect. The
        // caller supplies computed underlying values for restoration.
        let values = self
            .sample(compositor.trees())
            .unwrap_or_else(|| self.underlying.clone());
        let mut output = AnimationFrameOutput::default();
        for (property, value) in values {
            match (&*property, &value) {
                ("transform", AnimationValue::Transform(matrix))
                    if self.bindings.transform.is_some() =>
                {
                    let id = self.bindings.transform.unwrap();
                    if compositor
                        .trees()
                        .transform(id)
                        .is_some_and(|node| node.matrix != *matrix)
                    {
                        compositor.set_transform(id, *matrix)?;
                        output.compositor_updates += 1;
                    }
                }
                ("opacity", AnimationValue::Opacity(opacity))
                    if self.bindings.opacity.is_some() =>
                {
                    let id = self.bindings.opacity.unwrap();
                    if compositor
                        .trees()
                        .effect(id)
                        .is_some_and(|node| node.opacity != *opacity)
                    {
                        compositor.set_opacity(id, *opacity)?;
                        output.compositor_updates += 1;
                    }
                }
                _ => output.main_thread_updates.push(PropertyUpdate {
                    element: self.element,
                    property,
                    value,
                }),
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        CompositeMode, Direction, Easing, EffectTiming, Fill, IterationComposite, PropertyKeyframe,
        Timing,
    };
    use super::*;
    use crate::{BlendMode, EffectId, PropertyTrees};

    fn trees() -> (PropertyTrees, ScrollId, EffectId) {
        let mut trees = PropertyTrees::new();
        let scroll = trees
            .add_scroll(
                ScrollId(0),
                (0., 0.),
                Rect::new(0., 0., 100., 100.),
                Rect::new(0., 0., 100., 300.),
            )
            .unwrap();
        let effect = trees
            .add_effect(EffectId(0), 0., BlendMode::Normal, None)
            .unwrap();
        (trees, scroll, effect)
    }
    #[test]
    fn scroll_progress_uses_scrollable_extent_and_inactive_zero_range() {
        let (mut trees, scroll, _) = trees();
        let timeline = ScrollDrivenTimeline::Scroll(ScrollProgressTimeline {
            source: scroll,
            axis: TimelineAxis::Y,
        });
        assert_eq!(timeline.progress(&trees), Some(0.));
        trees.set_scroll(scroll, (0., 100.)).unwrap();
        assert_eq!(timeline.progress(&trees), Some(0.5));
        trees
            .set_scroll_contents(scroll, Rect::new(0., 0., 100., 100.))
            .unwrap();
        assert_eq!(timeline.progress(&trees), None);
    }
    #[test]
    fn view_cover_progress_and_insets() {
        let (mut trees, scroll, _) = trees();
        let timeline = ScrollDrivenTimeline::View(ViewProgressTimeline {
            source: scroll,
            axis: TimelineAxis::Y,
            subject: Rect::new(0., 150., 20., 50.),
            start_inset: 0.,
            end_inset: 0.,
        });
        assert!((timeline.progress(&trees).unwrap() + 1. / 3.).abs() < 1e-6);
        trees.set_scroll(scroll, (0., 50.)).unwrap();
        assert_eq!(timeline.progress(&trees), Some(0.));
        trees.set_scroll(scroll, (0., 125.)).unwrap();
        assert_eq!(timeline.progress(&trees), Some(0.5));
        let inset = ScrollDrivenTimeline::View(ViewProgressTimeline {
            start_inset: 10.,
            end_inset: 20.,
            ..match timeline {
                ScrollDrivenTimeline::View(v) => v,
                _ => unreachable!(),
            }
        });
        assert!((inset.progress(&trees).unwrap() - 55. / 120.).abs() < 1e-6);
    }
    #[test]
    fn scroll_tick_changes_opacity_without_raster_and_returns_layout_value() {
        let (trees, scroll, effect) = trees();
        let mut compositor = Compositor::new(trees, Vec::new(), 0).unwrap();
        let animation = ScrollDrivenAnimation {
            element: 7,
            timeline: ScrollDrivenTimeline::Scroll(ScrollProgressTimeline {
                source: scroll,
                axis: TimelineAxis::Y,
            }),
            effect: KeyframeEffect {
                timing: EffectTiming {
                    timing: Timing {
                        delay_ms: 0.,
                        duration_ms: 0.,
                        iterations: 1.,
                        direction: Direction::Normal,
                        fill: Fill::Both,
                        easing: Easing::Linear,
                    },
                    end_delay_ms: 0.,
                    iteration_start: 0.,
                },
                keyframes: vec![
                    PropertyKeyframe {
                        offset: 0.,
                        easing: Easing::Linear,
                        values: ComputedValues::from([
                            ("opacity".into(), AnimationValue::Opacity(0.)),
                            ("left".into(), AnimationValue::LengthPx(0.)),
                        ]),
                    },
                    PropertyKeyframe {
                        offset: 1.,
                        easing: Easing::Linear,
                        values: ComputedValues::from([
                            ("opacity".into(), AnimationValue::Opacity(1.)),
                            ("left".into(), AnimationValue::LengthPx(100.)),
                        ]),
                    },
                ],
                composite: CompositeMode::Replace,
                iteration_composite: IterationComposite::Replace,
            },
            underlying: ComputedValues::from([("opacity".into(), AnimationValue::Opacity(0.25))]),
            bindings: CompositorBindings {
                opacity: Some(effect),
                ..Default::default()
            },
        };
        compositor.set_scroll(scroll, (0., 100.)).unwrap();
        let output = animation.tick(&mut compositor).unwrap();
        assert_eq!(output.compositor_updates, 1);
        assert_eq!(compositor.trees().effect(effect).unwrap().opacity, 0.5);
        assert_eq!(
            output.main_thread_updates[0].value,
            AnimationValue::LengthPx(50.)
        );
        assert_eq!(compositor.stats().raster_calls, 0);
        compositor
            .set_scroll_contents(scroll, Rect::new(0., 0., 100., 100.))
            .unwrap();
        let inactive = animation.tick(&mut compositor).unwrap();
        assert_eq!(inactive.compositor_updates, 1);
        assert_eq!(compositor.trees().effect(effect).unwrap().opacity, 0.25);
    }
}
