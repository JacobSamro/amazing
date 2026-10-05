//! Resolved compositor animations and one deterministic sampler for both threads.
use amazing_dom::NodeIdx;
use crate::values::{ComputedValue, LengthPercentage, Transform};
use super::KeyframesRule;

pub type LocalMatrix4 = [[f32; 4]; 4];
const IDENTITY: LocalMatrix4 = [
    [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnimationProperty { Opacity, Transform }
impl AnimationProperty {
    pub fn as_str(self) -> &'static str {
        match self { Self::Opacity => "opacity", Self::Transform => "transform" }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind { Animation, Transition }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDirection { Normal, Reverse, Alternate, AlternateReverse }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFill { None, Forwards, Backwards, Both }
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationEasing {
    Linear,
    CubicBezier(f64, f64, f64, f64),
    Steps(u32, bool), // true: jump at the start
}
impl AnimationEasing {
    pub fn valid(self) -> bool {
        match self {
            Self::Linear => true,
            Self::Steps(count, _) => count > 0,
            Self::CubicBezier(x1, y1, x2, y2) =>
                [x1, y1, x2, y2].iter().all(|value| value.is_finite())
                    && (0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2),
        }
    }

    pub fn parse(css: &str) -> Option<Self> {
        let lower = css.trim().to_ascii_lowercase();
        match lower.as_str() {
            "linear" => return Some(Self::Linear),
            "ease" => return Some(Self::CubicBezier(0.25, 0.1, 0.25, 1.0)),
            "ease-in" => return Some(Self::CubicBezier(0.42, 0.0, 1.0, 1.0)),
            "ease-out" => return Some(Self::CubicBezier(0.0, 0.0, 0.58, 1.0)),
            "ease-in-out" => return Some(Self::CubicBezier(0.42, 0.0, 0.58, 1.0)),
            "step-start" => return Some(Self::Steps(1, true)),
            "step-end" => return Some(Self::Steps(1, false)),
            _ => {}
        }
        if let Some(inner) = lower.strip_prefix("cubic-bezier(").and_then(|s| s.strip_suffix(')')) {
            let values = inner.split(',').map(str::trim).map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>().ok()?;
            if values.len() == 4 && values.iter().all(|v| v.is_finite())
                && (0.0..=1.0).contains(&values[0]) && (0.0..=1.0).contains(&values[2]) {
                return Some(Self::CubicBezier(values[0], values[1], values[2], values[3]));
            }
        }
        if let Some(inner) = lower.strip_prefix("steps(").and_then(|s| s.strip_suffix(')')) {
            let mut parts = inner.split(',').map(str::trim);
            let count = parts.next()?.parse::<u32>().ok()?;
            if count == 0 { return None; }
            let start = match parts.next().unwrap_or("end") {
                "start" | "jump-start" => true,
                "end" | "jump-end" => false,
                _ => return None,
            };
            return parts.next().is_none().then_some(Self::Steps(count, start));
        }
        None
    }

    fn apply(self, progress: f64) -> f64 {
        let x = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => x,
            Self::Steps(count, jump_start) if count > 0 => {
                let scaled = x * f64::from(count);
                let step = if jump_start { scaled.floor() + 1.0 } else { scaled.floor() };
                (step / f64::from(count)).clamp(0.0, 1.0)
            }
            Self::Steps(_, _) => x,
            Self::CubicBezier(x1, y1, x2, y2) => {
                let bezier = |t: f64, a: f64, b: f64| {
                    3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t.powi(3)
                };
                let mut lo = 0.0;
                let mut hi = 1.0;
                for _ in 0..32 {
                    let mid = (lo + hi) * 0.5;
                    if bezier(mid, x1, x2) < x { lo = mid; } else { hi = mid; }
                }
                bezier((lo + hi) * 0.5, y1, y2)
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationTiming {
    pub duration_ms: f64,
    pub delay_ms: f64,
    pub iterations: f64,
    pub direction: AnimationDirection,
    pub fill: AnimationFill,
    pub easing: AnimationEasing,
}
impl AnimationTiming {
    /// Resolve one animation/transition list item from computed CSS longhands.
    pub fn from_computed_style(style: &crate::ComputedStyle, index: usize, kind: AnimationKind) -> Option<Self> {
        fn item(style: &crate::ComputedStyle, property: &str, index: usize) -> Option<String> {
            let tokens = crate::syntax::tokenize(style.get(property)?);
            let parts = super::split_commas(&tokens)?;
            let selected = parts.get(index % parts.len())?;
            Some(crate::syntax::serialize(selected).trim().to_ascii_lowercase())
        }
        fn time(css: String) -> Option<f64> {
            let seconds: f64 = css.strip_suffix('s')?.parse().ok()?;
            seconds.is_finite().then_some(seconds * 1000.0)
        }
        let (prefix, iterations, direction, fill) = match kind {
            AnimationKind::Animation => {
                let iterations = match item(style, "animation-iteration-count", index)?.as_str() {
                    "infinite" => f64::INFINITY,
                    number => number.parse().ok()?,
                };
                let direction = match item(style, "animation-direction", index)?.as_str() {
                    "normal" => AnimationDirection::Normal,
                    "reverse" => AnimationDirection::Reverse,
                    "alternate" => AnimationDirection::Alternate,
                    "alternate-reverse" => AnimationDirection::AlternateReverse,
                    _ => return None,
                };
                let fill = match item(style, "animation-fill-mode", index)?.as_str() {
                    "none" => AnimationFill::None,
                    "forwards" => AnimationFill::Forwards,
                    "backwards" => AnimationFill::Backwards,
                    "both" => AnimationFill::Both,
                    _ => return None,
                };
                ("animation", iterations, direction, fill)
            }
            AnimationKind::Transition => ("transition", 1.0, AnimationDirection::Normal, AnimationFill::Forwards),
        };
        let timing = Self {
            duration_ms: time(item(style, &format!("{prefix}-duration"), index)?)?,
            delay_ms: time(item(style, &format!("{prefix}-delay"), index)?)?,
            iterations, direction, fill,
            easing: AnimationEasing::parse(&item(style, &format!("{prefix}-timing-function"), index)?)?,
        };
        timing.valid().then_some(timing)
    }

    pub fn valid(self) -> bool {
        self.duration_ms.is_finite() && self.duration_ms >= 0.0
            && self.delay_ms.is_finite()
            && (self.iterations.is_finite() || self.iterations == f64::INFINITY)
            && self.iterations >= 0.0
            && !(self.duration_ms == 0.0 && self.iterations == f64::INFINITY)
            && self.easing.valid()
    }
    pub fn end_time(self, start_time_ms: f64) -> f64 {
        start_time_ms + self.delay_ms + self.duration_ms * self.iterations
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedTransformFunction {
    Translate(f32, f32), Rotate(f32), Scale(f32, f32), SkewX(f32), SkewY(f32),
}
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTransform {
    pub matrix: LocalMatrix4,
    pub functions: Vec<ResolvedTransformFunction>,
    pub origin: [f32; 2],
}
impl ResolvedTransform {
    pub fn from_css(css: &str, box_size: [f32; 2]) -> Option<Self> {
        if !box_size.iter().all(|v| v.is_finite() && *v >= 0.0) { return None; }
        let ComputedValue::Transform(ops) = crate::values::parse("transform", css)? else { return None; };
        if ops.is_empty() {
            return Some(Self { matrix: IDENTITY, functions: Vec::new(),
                origin: [box_size[0] / 2.0, box_size[1] / 2.0] });
        }
        let mut functions = Vec::new();
        let length = |value: LengthPercentage, basis: f32| match value {
            LengthPercentage::Px(value) => Some(value as f32),
            LengthPercentage::Percentage(value) => Some((value * f64::from(basis) / 100.0) as f32),
            LengthPercentage::Relative(_, _) => None,
        };
        for op in ops {
            functions.push(match op {
                Transform::Translate(x, y) => ResolvedTransformFunction::Translate(
                    length(x, box_size[0])?, length(y, box_size[1])?),
                Transform::Rotate(angle) => ResolvedTransformFunction::Rotate(angle.radians as f32),
                Transform::Scale(x, y) => ResolvedTransformFunction::Scale(x as f32, y as f32),
                Transform::SkewX(angle) => ResolvedTransformFunction::SkewX(angle.radians as f32),
                Transform::SkewY(angle) => ResolvedTransformFunction::SkewY(angle.radians as f32),
                Transform::Matrix(_) | Transform::Matrix3D(_) | Transform::Translate3D(..) | Transform::TranslateExpression(_) | Transform::Scale3D(_) | Transform::Rotate3D(..) | Transform::Skew(..) | Transform::Perspective(_) => return None, // matrix decomposition remains on the main thread
            });
        }
        let origin = [box_size[0] / 2.0, box_size[1] / 2.0];
        let matrix = compose(&functions, origin)?;
        Some(Self { matrix, functions, origin })
    }
    pub fn to_css(&self) -> String {
        if self.functions.is_empty() { return "none".into(); }
        self.functions.iter().map(|function| match function {
            ResolvedTransformFunction::Translate(x, y) => format!("translate({x}px,{y}px)"),
            ResolvedTransformFunction::Rotate(v) => format!("rotate({v}rad)"),
            ResolvedTransformFunction::Scale(x, y) => format!("scale({x},{y})"),
            ResolvedTransformFunction::SkewX(v) => format!("skewX({v}rad)"),
            ResolvedTransformFunction::SkewY(v) => format!("skewY({v}rad)"),
        }).collect::<Vec<_>>().join(" ")
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationFrameValue { Opacity(f32), Transform(ResolvedTransform) }
impl AnimationFrameValue {
    pub fn finite(&self) -> bool {
        match self {
            Self::Opacity(value) => value.is_finite(),
            Self::Transform(value) => value.matrix.iter().flatten().all(|v| v.is_finite())
                && value.origin.iter().all(|v| v.is_finite()),
        }
    }
    pub fn property(&self) -> AnimationProperty {
        match self { Self::Opacity(_) => AnimationProperty::Opacity, Self::Transform(_) => AnimationProperty::Transform }
    }
    pub fn to_css(&self) -> String {
        match self { Self::Opacity(value) => value.to_string(), Self::Transform(value) => value.to_css() }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedKeyframe {
    pub offset: f32,
    pub value: AnimationFrameValue,
    pub easing: Option<AnimationEasing>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CompositorAnimation {
    pub id: u64,
    pub node: NodeIdx,
    pub generation: u64,
    pub property: AnimationProperty,
    pub kind: AnimationKind,
    pub name: String,
    pub keyframes: Vec<ResolvedKeyframe>,
    pub timing: AnimationTiming,
    pub start_time_ms: f64,
}
impl CompositorAnimation {
    pub fn for_transition(
        id: u64, node: NodeIdx, generation: u64, property: AnimationProperty,
        old_css: &str, new_css: &str, timing: AnimationTiming,
        start_time_ms: f64, box_size: Option<[f32; 2]>,
    ) -> Option<Self> {
        let animation = Self {
            id, node, generation, property, kind: AnimationKind::Transition,
            name: property.as_str().into(),
            keyframes: vec![
                ResolvedKeyframe { offset: 0.0, value: resolved_value(property, old_css, box_size)?, easing: None },
                ResolvedKeyframe { offset: 1.0, value: resolved_value(property, new_css, box_size)?, easing: None },
            ],
            timing, start_time_ms,
        };
        animation.valid().then_some(animation)
    }

    pub fn valid(&self) -> bool {
        self.start_time_ms.is_finite() && self.timing.valid()
            && self.keyframes.len() >= 2
            && self.keyframes.first().is_some_and(|frame| frame.offset == 0.0)
            && self.keyframes.last().is_some_and(|frame| frame.offset == 1.0)
            && self.keyframes.windows(2).all(|frames| frames[0].offset < frames[1].offset)
            && self.keyframes.iter().all(|frame| frame.offset.is_finite()
                && frame.value.property() == self.property && frame.value.finite()
                && frame.easing.is_none_or(AnimationEasing::valid))
            && self.keyframes.windows(2).all(|frames| compatible(&frames[0].value, &frames[1].value))
    }
    /// Resolve a CSS `@keyframes` property. Mixed main-thread properties or
    /// unresolved geometry reject the whole handoff.
    pub fn from_keyframes(
        id: u64, node: NodeIdx, generation: u64, rule: &KeyframesRule,
        property: AnimationProperty, timing: AnimationTiming, start_time_ms: f64,
        box_size: Option<[f32; 2]>,
    ) -> Option<Self> {
        if rule.frames.iter().any(|frame| frame.declarations.iter().any(|(name, _)|
            name != "opacity" && name != "transform")) { return None; }
        let mut keyframes = Vec::new();
        for frame in &rule.frames {
            if let Some((_, css)) = frame.declarations.iter().rev().find(|(name, _)| name == property.as_str()) {
                let value = resolved_value(property, css, box_size)?;
                for &offset in &frame.offsets {
                    let easing = match frame.timing_function.as_deref() {
                        Some(css) => Some(AnimationEasing::parse(css)?),
                        None => None,
                    };
                    keyframes.push(ResolvedKeyframe { offset, value: value.clone(), easing });
                }
            }
        }
        keyframes.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        keyframes.dedup_by(|a, b| {
            if a.offset == b.offset { *a = b.clone(); true } else { false }
        });
        let animation = Self { id, node, generation, property, kind: AnimationKind::Animation,
            name: rule.name.clone(), keyframes, timing, start_time_ms };
        animation.valid().then_some(animation)
    }
}

fn resolved_value(property: AnimationProperty, css: &str, box_size: Option<[f32; 2]>)
    -> Option<AnimationFrameValue> {
    match property {
        AnimationProperty::Opacity => {
            let ComputedValue::Number(value) = crate::values::parse("opacity", css)? else { return None; };
            Some(AnimationFrameValue::Opacity(value as f32))
        }
        AnimationProperty::Transform => Some(AnimationFrameValue::Transform(
            ResolvedTransform::from_css(css, box_size?)?)),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPhase { Before, Active, After }
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationSample {
    pub phase: AnimationPhase,
    pub value: Option<AnimationFrameValue>,
    pub finished: bool,
}

/// Completion/cancellation message sent by the compositor to the CSS owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationPropertyUpdate {
    pub id: u64,
    pub node: NodeIdx,
    pub generation: u64,
    pub property: AnimationProperty,
    pub time_ms: f64,
    pub cancelled: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationMainThreadEventKind { AnimationEnd, AnimationCancel, TransitionEnd, TransitionCancel }
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationMainThreadEvent {
    pub node: NodeIdx,
    pub generation: u64,
    pub name: String,
    pub kind: AnimationMainThreadEventKind,
    pub elapsed_time_ms: f64,
}
/// Pure sampler shared by the compositor frame and main-thread style reads.
/// `t_ms` is the document timeline time in milliseconds.
pub fn sample(animation: &CompositorAnimation, t_ms: f64) -> Option<AnimationSample> {
    if !animation.valid() || !t_ms.is_finite() { return None; }
    let timing = animation.timing;
    let elapsed = t_ms - animation.start_time_ms - timing.delay_ms;
    let active_duration = timing.duration_ms * timing.iterations;
    let phase = if elapsed < 0.0 { AnimationPhase::Before }
        else if elapsed >= active_duration { AnimationPhase::After }
        else { AnimationPhase::Active };
    let progress = match phase {
        AnimationPhase::Before if matches!(timing.fill, AnimationFill::Backwards | AnimationFill::Both) => {
            if matches!(timing.direction, AnimationDirection::Reverse | AnimationDirection::AlternateReverse) { 1.0 } else { 0.0 }
        },
        AnimationPhase::After if matches!(timing.fill, AnimationFill::Forwards | AnimationFill::Both) => {
            if timing.iterations > 0.0 && timing.iterations.fract() != 0.0 {
                timing.iterations.fract()
            } else { 1.0 }
        },
        AnimationPhase::Before | AnimationPhase::After => return Some(AnimationSample {
            phase, value: None, finished: phase == AnimationPhase::After,
        }),
        AnimationPhase::Active => {
            let exact = if timing.duration_ms == 0.0 { 1.0 } else { elapsed / timing.duration_ms };
            let iteration = exact.floor();
            let mut fraction = exact.fract();
            let reverse = match timing.direction {
                AnimationDirection::Normal => false,
                AnimationDirection::Reverse => true,
                AnimationDirection::Alternate => iteration as u64 % 2 == 1,
                AnimationDirection::AlternateReverse => iteration as u64 % 2 == 0,
            };
            if reverse { fraction = 1.0 - fraction; }
            fraction
        }
    };
    let progress = if phase == AnimationPhase::After {
        let final_iteration = timing.iterations.ceil().max(1.0) as u64 - 1;
        let reverse = match timing.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate => final_iteration % 2 == 1,
            AnimationDirection::AlternateReverse => final_iteration % 2 == 0,
        };
        if reverse { 0.0 } else { progress }
    } else { progress };
    let progress = progress as f32;
    let value = if progress <= animation.keyframes[0].offset {
        animation.keyframes[0].value.clone()
    } else if progress >= animation.keyframes.last()?.offset {
        animation.keyframes.last()?.value.clone()
    } else {
        let pair = animation.keyframes.windows(2).find(|pair| progress <= pair[1].offset)?;
        let fraction = (progress - pair[0].offset) / (pair[1].offset - pair[0].offset);
        let fraction = pair[0].easing.unwrap_or(timing.easing).apply(f64::from(fraction)) as f32;
        interpolate(&pair[0].value, &pair[1].value, fraction)?
    };
    Some(AnimationSample { phase, value: Some(value), finished: phase == AnimationPhase::After })
}
fn compatible(a: &AnimationFrameValue, b: &AnimationFrameValue) -> bool {
    match (a, b) {
        (AnimationFrameValue::Opacity(_), AnimationFrameValue::Opacity(_)) => true,
        (AnimationFrameValue::Transform(a), AnimationFrameValue::Transform(b)) =>
            a.origin == b.origin && (a.functions.is_empty() || b.functions.is_empty()
                || (a.functions.len() == b.functions.len()
                    && a.functions.iter().zip(&b.functions).all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b)))),
        _ => false,
    }
}
fn interpolate(a: &AnimationFrameValue, b: &AnimationFrameValue, t: f32) -> Option<AnimationFrameValue> {
    if !compatible(a, b) { return None; }
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Some(match (a, b) {
        (AnimationFrameValue::Opacity(a), AnimationFrameValue::Opacity(b)) =>
            AnimationFrameValue::Opacity(lerp(*a, *b)),
        (AnimationFrameValue::Transform(a), AnimationFrameValue::Transform(b)) => {
            let neutral = |function: &ResolvedTransformFunction| match function {
                ResolvedTransformFunction::Translate(_, _) => ResolvedTransformFunction::Translate(0.0, 0.0),
                ResolvedTransformFunction::Rotate(_) => ResolvedTransformFunction::Rotate(0.0),
                ResolvedTransformFunction::Scale(_, _) => ResolvedTransformFunction::Scale(1.0, 1.0),
                ResolvedTransformFunction::SkewX(_) => ResolvedTransformFunction::SkewX(0.0),
                ResolvedTransformFunction::SkewY(_) => ResolvedTransformFunction::SkewY(0.0),
            };
            let from: Vec<_> = if a.functions.is_empty() { b.functions.iter().map(neutral).collect() }
                else { a.functions.clone() };
            let to: Vec<_> = if b.functions.is_empty() { a.functions.iter().map(neutral).collect() }
                else { b.functions.clone() };
            let functions = from.iter().zip(&to).map(|(a, b)| match (a, b) {
                (ResolvedTransformFunction::Translate(ax, ay), ResolvedTransformFunction::Translate(bx, by)) =>
                    Some(ResolvedTransformFunction::Translate(lerp(*ax, *bx), lerp(*ay, *by))),
                (ResolvedTransformFunction::Rotate(a), ResolvedTransformFunction::Rotate(b)) =>
                    Some(ResolvedTransformFunction::Rotate(lerp(*a, *b))),
                (ResolvedTransformFunction::Scale(ax, ay), ResolvedTransformFunction::Scale(bx, by)) =>
                    Some(ResolvedTransformFunction::Scale(lerp(*ax, *bx), lerp(*ay, *by))),
                (ResolvedTransformFunction::SkewX(a), ResolvedTransformFunction::SkewX(b)) =>
                    Some(ResolvedTransformFunction::SkewX(lerp(*a, *b))),
                (ResolvedTransformFunction::SkewY(a), ResolvedTransformFunction::SkewY(b)) =>
                    Some(ResolvedTransformFunction::SkewY(lerp(*a, *b))),
                _ => None,
            }).collect::<Option<Vec<_>>>()?;
            AnimationFrameValue::Transform(ResolvedTransform {
                matrix: compose(&functions, a.origin)?, functions, origin: a.origin,
            })
        }
        _ => return None,
    })
}
fn mul(a: LocalMatrix4, b: LocalMatrix4) -> LocalMatrix4 {
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 { for j in 0..4 { out[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum(); } }
    out
}
fn translation(x: f32, y: f32) -> LocalMatrix4 {
    let mut matrix = IDENTITY;
    matrix[0][3] = x; matrix[1][3] = y;
    matrix
}
fn compose(functions: &[ResolvedTransformFunction], origin: [f32; 2]) -> Option<LocalMatrix4> {
    let mut matrix = IDENTITY;
    for function in functions {
        let next = match function {
            ResolvedTransformFunction::Translate(x, y) => translation(*x, *y),
            ResolvedTransformFunction::Rotate(angle) => {
                let (sin, cos) = angle.sin_cos();
                [[cos, -sin, 0.0, 0.0], [sin, cos, 0.0, 0.0],
                 [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
            }
            ResolvedTransformFunction::Scale(x, y) => {
                [[*x, 0.0, 0.0, 0.0], [0.0, *y, 0.0, 0.0],
                 [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
            }
            ResolvedTransformFunction::SkewX(angle) => {
                let mut next = IDENTITY; next[0][1] = angle.tan(); next
            }
            ResolvedTransformFunction::SkewY(angle) => {
                let mut next = IDENTITY; next[1][0] = angle.tan(); next
            }
        };
        matrix = mul(matrix, next);
    }
    let matrix = mul(mul(translation(origin[0], origin[1]), matrix),
        translation(-origin[0], -origin[1]));
    matrix.iter().flatten().all(|value| value.is_finite()).then_some(matrix)
}
