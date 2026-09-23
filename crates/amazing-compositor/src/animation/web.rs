//! Document-timeline animation state and keyframe effects.
use super::{AnimationValue, CompositeMode, Direction, Easing, Fill, IterationComposite, Timing};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocumentTimeline {
    pub origin_ms: f64,
    pub current_ms: Option<f64>,
}
impl DocumentTimeline {
    pub fn new(origin_ms: f64) -> Self {
        Self {
            origin_ms,
            current_ms: None,
        }
    }
    pub fn advance(&mut self, monotonic_ms: f64) {
        if monotonic_ms.is_finite() {
            let time = (monotonic_ms - self.origin_ms).max(0.);
            self.current_ms = Some(self.current_ms.map_or(time, |old| old.max(time)));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPhase {
    Before,
    Active,
    After,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSample {
    pub progress: f64,
    pub iteration: u64,
    pub phase: EffectPhase,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectTiming {
    pub timing: Timing,
    pub end_delay_ms: f64,
    pub iteration_start: f64,
}
impl EffectTiming {
    pub fn active_duration(&self) -> f64 {
        if self.timing.duration_ms == 0. || self.timing.iterations == 0. {
            0.
        } else {
            self.timing.duration_ms * self.timing.iterations
        }
    }
    pub fn end_time(&self) -> f64 {
        (self.timing.delay_ms + self.active_duration() + self.end_delay_ms).max(0.)
    }
    pub fn valid(&self) -> bool {
        self.timing.valid()
            && self.end_delay_ms.is_finite()
            && self.iteration_start.is_finite()
            && self.iteration_start >= 0.
    }
    pub fn phase(&self, local_time_ms: f64) -> Option<EffectPhase> {
        if !self.valid() || !local_time_ms.is_finite() {
            return None;
        }
        let local = local_time_ms - self.timing.delay_ms;
        Some(if local < 0. {
            EffectPhase::Before
        } else if local >= self.active_duration() {
            EffectPhase::After
        } else {
            EffectPhase::Active
        })
    }
    pub fn sample(&self, local_time_ms: f64) -> Option<EffectSample> {
        if !self.valid() || !local_time_ms.is_finite() {
            return None;
        }
        let active = self.active_duration();
        let local = local_time_ms - self.timing.delay_ms;
        let phase = self.phase(local_time_ms)?;
        let time = if phase == EffectPhase::Before {
            if !matches!(self.timing.fill, Fill::Backwards | Fill::Both) {
                return None;
            }
            0.
        } else if phase == EffectPhase::After {
            if !matches!(self.timing.fill, Fill::Forwards | Fill::Both) {
                return None;
            }
            active
        } else {
            local
        };
        let at_end = phase == EffectPhase::After;
        let exact = self.iteration_start
            + if self.timing.duration_ms == 0. {
                if at_end {
                    self.timing.iterations
                } else {
                    0.
                }
            } else {
                time / self.timing.duration_ms
            };
        let (iteration, simple) =
            if at_end && self.timing.iterations > 0. && exact > 0. && exact.fract() == 0. {
                ((exact as u64).saturating_sub(1), 1.)
            } else {
                (exact.floor() as u64, exact.fract())
            };
        let reverse = match self.timing.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => iteration % 2 == 1,
            Direction::AlternateReverse => iteration % 2 == 0,
        };
        let before = (phase == EffectPhase::Before && !reverse) || (at_end && reverse);
        let progress = self
            .timing
            .easing
            .apply_with_before(if reverse { 1. - simple } else { simple }, before);
        Some(EffectSample {
            progress,
            iteration,
            phase,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyKeyframe {
    pub offset: f64,
    pub easing: Easing,
    pub values: BTreeMap<String, AnimationValue>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeEffect {
    pub timing: EffectTiming,
    pub keyframes: Vec<PropertyKeyframe>,
    pub composite: CompositeMode,
    pub iteration_composite: IterationComposite,
}
impl KeyframeEffect {
    pub fn valid(&self) -> bool {
        self.timing.valid()
            && self.keyframes.len() >= 2
            && self
                .keyframes
                .first()
                .is_some_and(|frame| frame.offset == 0.)
            && self
                .keyframes
                .last()
                .is_some_and(|frame| frame.offset == 1.)
            && self
                .keyframes
                .windows(2)
                .all(|pair| pair[0].offset <= pair[1].offset)
            && self.keyframes.iter().all(|frame| {
                frame.offset.is_finite()
                    && frame.easing.valid()
                    && frame.values.values().all(AnimationValue::finite)
            })
    }
    pub fn sample(
        &self,
        local_time_ms: f64,
        underlying: &BTreeMap<String, AnimationValue>,
    ) -> Option<BTreeMap<String, AnimationValue>> {
        let sample = self.timing.sample(local_time_ms)?;
        let mut result = BTreeMap::new();
        let properties: std::collections::BTreeSet<_> = self
            .keyframes
            .iter()
            .flat_map(|frame| frame.values.keys().cloned())
            .collect();
        for property in properties {
            let keyframes: Vec<_> = self
                .keyframes
                .iter()
                .filter_map(|frame| {
                    frame
                        .values
                        .get(&property)
                        .map(|value| (frame.offset, &frame.easing, value))
                })
                .collect();
            let Some(first) = keyframes.first() else {
                continue;
            };
            let value = if sample.progress <= first.0 {
                (*first.2).clone()
            } else {
                let mut value = (*keyframes.last().unwrap().2).clone();
                for pair in keyframes.windows(2) {
                    if sample.progress <= pair[1].0 {
                        let portion = if pair[1].0 == pair[0].0 {
                            1.
                        } else {
                            (sample.progress - pair[0].0) / (pair[1].0 - pair[0].0)
                        };
                        value = pair[0]
                            .2
                            .interpolate(pair[1].2, pair[0].1.apply(portion) as f32);
                        break;
                    }
                }
                value
            };
            let value = if self.iteration_composite == IterationComposite::Accumulate
                && sample.iteration > 0
            {
                let start = keyframes.first().unwrap().2;
                let end = keyframes.last().unwrap().2;
                start
                    .delta(end)
                    .and_then(|delta| delta.scaled(sample.iteration))
                    .and_then(|delta| value.add(&delta))
                    .unwrap_or(value)
            } else {
                value
            };
            let value = match self.composite {
                CompositeMode::Replace => value,
                CompositeMode::Add | CompositeMode::Accumulate => underlying
                    .get(&property)
                    .and_then(|base| base.add(&value))
                    .unwrap_or(value),
            };
            result.insert(property, value);
        }
        Some(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Idle,
    Running,
    Paused,
    Finished,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseSignal {
    ReadyResolved,
    FinishedResolved,
    FinishedRejected,
}
pub trait AnimationHostCallbacks {
    fn animation_promise(&mut self, id: u64, signal: PromiseSignal);
}

#[derive(Debug, Clone)]
pub struct WebAnimation {
    pub id: u64,
    pub effect: KeyframeEffect,
    pub playback_rate: f64,
    pub start_time_ms: Option<f64>,
    pub hold_time_ms: Option<f64>,
    pub state: PlayState,
    pending: Vec<PromiseSignal>,
}
impl WebAnimation {
    pub fn new(id: u64, effect: KeyframeEffect) -> Option<Self> {
        effect.valid().then_some(Self {
            id,
            effect,
            playback_rate: 1.,
            start_time_ms: None,
            hold_time_ms: None,
            state: PlayState::Idle,
            pending: Vec::new(),
        })
    }
    pub fn current_time(&self, timeline_ms: Option<f64>) -> Option<f64> {
        self.hold_time_ms.or_else(|| {
            self.start_time_ms
                .zip(timeline_ms)
                .map(|(start, now)| (now - start) * self.playback_rate)
        })
    }
    pub fn play(&mut self, timeline_ms: Option<f64>) {
        let now = timeline_ms.unwrap_or(0.);
        let held = self.current_time(timeline_ms).unwrap_or_else(|| {
            if self.playback_rate < 0. {
                self.effect.timing.end_time()
            } else {
                0.
            }
        });
        if self.playback_rate != 0. && timeline_ms.is_some() {
            self.start_time_ms = Some(now - held / self.playback_rate);
            self.hold_time_ms = None;
        } else {
            self.hold_time_ms = Some(held);
            self.start_time_ms = None
        }
        self.state = PlayState::Running;
        self.pending.push(PromiseSignal::ReadyResolved);
    }
    pub fn pause(&mut self, timeline_ms: Option<f64>) {
        if self.state == PlayState::Running {
            self.hold_time_ms = self.current_time(timeline_ms);
            self.start_time_ms = None;
            self.state = PlayState::Paused;
            self.pending.push(PromiseSignal::ReadyResolved);
        }
    }
    pub fn reverse(&mut self, timeline_ms: Option<f64>) {
        self.set_playback_rate(
            if self.playback_rate == 0. {
                -1.
            } else {
                -self.playback_rate
            },
            timeline_ms,
        );
        self.play(timeline_ms);
    }
    pub fn set_playback_rate(&mut self, rate: f64, timeline_ms: Option<f64>) {
        if !rate.is_finite() {
            return;
        }
        let current = self.current_time(timeline_ms);
        self.playback_rate = rate;
        if self.state == PlayState::Running
            && rate != 0.
            && timeline_ms.is_some()
            && current.is_some()
        {
            self.start_time_ms = Some(timeline_ms.unwrap() - current.unwrap() / rate);
            self.hold_time_ms = None;
        } else {
            self.hold_time_ms = current;
            self.start_time_ms = None;
        }
    }
    pub fn seek(&mut self, current_ms: f64, timeline_ms: Option<f64>) {
        if !current_ms.is_finite() {
            return;
        }
        self.hold_time_ms = Some(current_ms);
        self.start_time_ms = None;
        if self.state == PlayState::Running && self.playback_rate != 0. {
            self.play(timeline_ms)
        }
    }
    pub fn finish(&mut self) {
        let end = self.effect.timing.end_time();
        if !end.is_finite() {
            return;
        }
        self.hold_time_ms = Some(if self.playback_rate < 0. { 0. } else { end });
        self.start_time_ms = None;
        self.state = PlayState::Finished;
        self.pending.push(PromiseSignal::FinishedResolved);
    }
    pub fn cancel(&mut self) {
        if self.state != PlayState::Idle {
            self.state = PlayState::Idle;
            self.start_time_ms = None;
            self.hold_time_ms = None;
            self.pending.push(PromiseSignal::FinishedRejected);
        }
    }
    pub fn sample(
        &mut self,
        timeline_ms: Option<f64>,
        underlying: &BTreeMap<String, AnimationValue>,
    ) -> Option<BTreeMap<String, AnimationValue>> {
        let current = self.current_time(timeline_ms)?;
        if self.state == PlayState::Running
            && ((self.playback_rate >= 0. && current >= self.effect.timing.end_time())
                || (self.playback_rate < 0. && current <= 0.))
        {
            self.finish();
        }
        self.effect
            .sample(self.current_time(timeline_ms)?, underlying)
    }
    pub fn flush_callbacks(&mut self, host: &mut impl AnimationHostCallbacks) {
        for signal in self.pending.drain(..) {
            host.animation_promise(self.id, signal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn timing(duration: f64, iterations: f64, iteration_start: f64) -> EffectTiming {
        EffectTiming {
            timing: Timing {
                delay_ms: 1.,
                duration_ms: duration,
                iterations,
                direction: Direction::Normal,
                fill: Fill::Both,
                easing: Easing::Linear,
            },
            end_delay_ms: 0.,
            iteration_start,
        }
    }
    #[test]
    fn pinned_wpt_simple_iteration_progress_and_current_iteration() {
        // WPT c2a2fce: web-animations/timing-model/animation-effects/
        // simple-iteration-progress.html and current-iteration.html.
        let cases = [
            (0., 0., 0., 0., 0, 0., 0),
            (100., 0., 2.5, 0.5, 2, 0.5, 2),
            (0., 3., 0., 0., 0, 1., 2),
            (100., 3., 2.5, 0.5, 2, 0.5, 5),
            (100., 3.5, 0., 0., 0, 0.5, 3),
            (0., 3.5, 2.5, 0.5, 2, 1., 5),
        ];
        for (
            duration,
            iterations,
            start,
            before_progress,
            before_iteration,
            after_progress,
            after_iteration,
        ) in cases
        {
            let effect = timing(duration, iterations, start);
            let before = effect.sample(0.).unwrap();
            let after = effect.sample(1000.).unwrap();
            assert_eq!(
                (before.progress, before.iteration),
                (before_progress, before_iteration)
            );
            assert_eq!(
                (after.progress, after.iteration),
                (after_progress, after_iteration)
            );
        }
        let spec = EffectTiming {
            timing: Timing {
                delay_ms: 3000.,
                duration_ms: 2000.,
                iterations: 2.,
                direction: Direction::Normal,
                fill: Fill::Both,
                easing: Easing::Linear,
            },
            end_delay_ms: 0.,
            iteration_start: 0.,
        };
        assert_eq!(spec.sample(6000.).unwrap().progress, 0.5);
        assert_eq!(spec.sample(6000.).unwrap().iteration, 1);
    }
    #[test]
    fn play_pause_rate_reverse_finish_cancel_signal_host() {
        let mut from = BTreeMap::new();
        from.insert("opacity".into(), AnimationValue::Opacity(0.));
        let mut to = BTreeMap::new();
        to.insert("opacity".into(), AnimationValue::Opacity(1.));
        let effect = KeyframeEffect {
            timing: timing(100., 1., 0.),
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
        let mut animation = WebAnimation::new(7, effect).unwrap();
        let mut timeline = DocumentTimeline::new(1000.);
        timeline.advance(1010.);
        animation.play(timeline.current_ms);
        assert_eq!(animation.current_time(Some(60.)), Some(50.));
        animation.pause(Some(60.));
        assert_eq!(animation.current_time(Some(90.)), Some(50.));
        animation.play(Some(90.));
        animation.set_playback_rate(2., Some(100.));
        assert_eq!(animation.current_time(Some(110.)), Some(80.));
        animation.reverse(Some(110.));
        assert_eq!(animation.current_time(Some(120.)), Some(60.));
        animation.finish();
        assert_eq!(animation.state, PlayState::Finished);
        animation.cancel();
        assert_eq!(animation.state, PlayState::Idle);
        struct Host(Vec<(u64, PromiseSignal)>);
        impl AnimationHostCallbacks for Host {
            fn animation_promise(&mut self, id: u64, signal: PromiseSignal) {
                self.0.push((id, signal))
            }
        }
        let mut host = Host(Vec::new());
        animation.flush_callbacks(&mut host);
        assert!(host.0.contains(&(7, PromiseSignal::ReadyResolved)));
        assert!(host.0.contains(&(7, PromiseSignal::FinishedResolved)));
        assert!(host.0.contains(&(7, PromiseSignal::FinishedRejected)));
    }
    #[test]
    fn keyframes_add_and_iteration_accumulate() {
        let mut from = BTreeMap::new();
        from.insert("left".into(), AnimationValue::LengthPx(10.));
        let mut to = BTreeMap::new();
        to.insert("left".into(), AnimationValue::LengthPx(20.));
        let effect = KeyframeEffect {
            timing: timing(100., 3., 0.),
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
            composite: CompositeMode::Add,
            iteration_composite: IterationComposite::Accumulate,
        };
        let mut underlying = BTreeMap::new();
        underlying.insert("left".into(), AnimationValue::LengthPx(5.));
        assert_eq!(
            effect.sample(151., &underlying).unwrap().get("left"),
            Some(&AnimationValue::LengthPx(30.))
        );
    }
}
