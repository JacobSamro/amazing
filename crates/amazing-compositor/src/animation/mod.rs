//! Stateless Web Animations timing and compositor-owned property animations.
mod css;
mod scroll;
mod values;
mod web;
use crate::{Compositor, EffectId, Matrix4, TransformId, TreeError};
pub use css::*;
pub use scroll::*;
pub use values::*;
pub use web::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    None,
    Backwards,
    Forwards,
    Both,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepPosition {
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Easing {
    Linear,
    CubicBezier(f64, f64, f64, f64),
    Steps(u32, StepPosition),
    LinearStops(Vec<(f64, f64)>),
}
impl Easing {
    pub fn apply(&self, p: f64) -> f64 {
        self.apply_with_before(p, false)
    }
    fn apply_with_before(&self, p: f64, before: bool) -> f64 {
        let p = p.clamp(0., 1.);
        match self {
            Self::Linear => p,
            Self::CubicBezier(x1, y1, x2, y2) => {
                if p == 0. || p == 1. {
                    return p;
                }
                let cubic = |t: f64, a: f64, b: f64| {
                    let u = 1. - t;
                    3. * u * u * t * a + 3. * u * t * t * b + t * t * t
                };
                let mut lo = 0.;
                let mut hi = 1.;
                for _ in 0..40 {
                    let mid = (lo + hi) * 0.5;
                    if cubic(mid, *x1, *x2) < p {
                        lo = mid
                    } else {
                        hi = mid
                    }
                }
                cubic((lo + hi) * 0.5, *y1, *y2)
            }
            Self::Steps(n, pos) => {
                if before {
                    return 0.;
                }
                if p == 1. {
                    return 1.;
                }
                let n = *n as f64;
                if n < 1. {
                    return p;
                }
                match pos {
                    StepPosition::JumpStart => ((p * n).floor() + 1.).min(n) / n,
                    StepPosition::JumpEnd => (p * n).floor() / n,
                    StepPosition::JumpNone => {
                        if n < 2. {
                            p
                        } else {
                            ((p * n).floor() / (n - 1.)).min(1.)
                        }
                    }
                    StepPosition::JumpBoth => ((p * n).floor() + 1.) / (n + 1.),
                }
            }
            Self::LinearStops(stops) => {
                if stops.is_empty() {
                    return p;
                }
                if p <= stops[0].0 {
                    return stops[0].1;
                }
                for pair in stops.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if p <= b.0 {
                        if b.0 == a.0 {
                            return b.1;
                        }
                        return a.1 + (b.1 - a.1) * (p - a.0) / (b.0 - a.0);
                    }
                }
                stops.last().unwrap().1
            }
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Linear => true,
            Self::CubicBezier(x1, y1, x2, y2) => {
                [x1, y1, x2, y2].iter().all(|v| v.is_finite())
                    && (0. ..=1.).contains(x1)
                    && (0. ..=1.).contains(x2)
            }
            Self::Steps(n, pos) => *n > 0 && (*pos != StepPosition::JumpNone || *n > 1),
            Self::LinearStops(stops) => {
                stops.len() >= 2
                    && stops.iter().all(|(x, y)| x.is_finite() && y.is_finite())
                    && stops.windows(2).all(|w| w[0].0 <= w[1].0)
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    pub delay_ms: f64,
    pub duration_ms: f64,
    pub iterations: f64,
    pub direction: Direction,
    pub fill: Fill,
    pub easing: Easing,
}
impl Timing {
    pub fn valid(&self) -> bool {
        self.delay_ms.is_finite()
            && !self.duration_ms.is_nan()
            && self.duration_ms >= 0.
            && !self.iterations.is_nan()
            && self.iterations >= 0.
            && self.easing.valid()
    }
    pub fn progress(&self, elapsed_ms: f64) -> Option<f64> {
        EffectTiming {
            timing: self.clone(),
            end_delay_ms: 0.,
            iteration_start: 0.,
        }
        .sample(elapsed_ms)
        .map(|sample| sample.progress)
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimatedValue {
    Opacity(f32),
    Transform(Matrix4),
}
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub offset: f64,
    pub value: AnimatedValue,
    pub easing: Easing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationTarget {
    Transform(TransformId),
    Opacity(EffectId),
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationError {
    InvalidTiming,
    InvalidKeyframes,
    InvalidTarget,
}
impl std::fmt::Display for AnimationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AnimationError {}
#[derive(Debug, Clone)]
pub struct CompositorAnimation {
    pub start_ms: f64,
    pub timing: Timing,
    pub target: AnimationTarget,
    pub keyframes: Vec<Keyframe>,
    base: AnimatedValue,
}
impl CompositorAnimation {
    pub fn sample(&self, now_ms: f64) -> Option<AnimatedValue> {
        let p = self.timing.progress(now_ms - self.start_ms)?;
        let first = self.keyframes.first()?;
        if p <= first.offset {
            return Some(first.value);
        }
        for pair in self.keyframes.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if p <= b.offset {
                let local = if b.offset == a.offset {
                    1.
                } else {
                    (p - a.offset) / (b.offset - a.offset)
                };
                return Some(interpolate(a.value, b.value, a.easing.apply(local) as f32));
            }
        }
        self.keyframes.last().map(|k| k.value)
    }
}
fn interpolate(a: AnimatedValue, b: AnimatedValue, t: f32) -> AnimatedValue {
    match (a, b) {
        (AnimatedValue::Opacity(x), AnimatedValue::Opacity(y)) => {
            AnimatedValue::Opacity((x + (y - x) * t).clamp(0., 1.))
        }
        (AnimatedValue::Transform(x), AnimatedValue::Transform(y)) => {
            AnimatedValue::Transform(interpolate_matrix(x, y, t))
        }
        _ => a,
    }
}
#[derive(Debug, Clone, Copy)]
struct Decomposed {
    translation: [f32; 3],
    scale: [f32; 3],
    skew: [f32; 3],
    rotation: [f32; 4],
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f32; 3], b: [f32; 3], scale: f32) -> [f32; 3] {
    [
        a[0] - b[0] * scale,
        a[1] - b[1] * scale,
        a[2] - b[2] * scale,
    ]
}
fn norm(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn quaternion_from_columns(c: [[f32; 3]; 3]) -> [f32; 4] {
    let m = [
        [c[0][0], c[1][0], c[2][0]],
        [c[0][1], c[1][1], c[2][1]],
        [c[0][2], c[1][2], c[2][2]],
    ];
    let trace = m[0][0] + m[1][1] + m[2][2];
    let mut q = [0.; 4];
    if trace > 0. {
        let s = (trace + 1.).sqrt() * 2.;
        q = [
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            s * 0.25,
        ]
    } else {
        let i = if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            0
        } else if m[1][1] > m[2][2] {
            1
        } else {
            2
        };
        let j = (i + 1) % 3;
        let k = (i + 2) % 3;
        let s = (1. + m[i][i] - m[j][j] - m[k][k]).sqrt() * 2.;
        q[i] = s * 0.25;
        q[j] = (m[j][i] + m[i][j]) / s;
        q[k] = (m[k][i] + m[i][k]) / s;
        q[3] = (m[k][j] - m[j][k]) / s
    }
    q
}
fn decompose(m: Matrix4) -> Option<Decomposed> {
    let a = m.0;
    if a[3] != [0., 0., 0., 1.] || !a.iter().flatten().all(|v| v.is_finite()) {
        return None;
    }
    let mut c = [
        [a[0][0], a[1][0], a[2][0]],
        [a[0][1], a[1][1], a[2][1]],
        [a[0][2], a[1][2], a[2][2]],
    ];
    let mut scale = [0.; 3];
    let mut skew = [0.; 3];
    scale[0] = norm(c[0]);
    if scale[0] < 1e-8 {
        return None;
    }
    for v in &mut c[0] {
        *v /= scale[0]
    }
    skew[0] = dot(c[0], c[1]);
    c[1] = sub(c[1], c[0], skew[0]);
    scale[1] = norm(c[1]);
    if scale[1] < 1e-8 {
        return None;
    }
    for v in &mut c[1] {
        *v /= scale[1]
    }
    skew[0] /= scale[1];
    skew[1] = dot(c[0], c[2]);
    c[2] = sub(c[2], c[0], skew[1]);
    skew[2] = dot(c[1], c[2]);
    c[2] = sub(c[2], c[1], skew[2]);
    scale[2] = norm(c[2]);
    if scale[2] < 1e-8 {
        return None;
    }
    for v in &mut c[2] {
        *v /= scale[2]
    }
    skew[1] /= scale[2];
    skew[2] /= scale[2];
    if dot(c[0], cross(c[1], c[2])) < 0. {
        for v in &mut scale {
            *v = -*v
        }
        for col in &mut c {
            for v in col {
                *v = -*v
            }
        }
    }
    Some(Decomposed {
        translation: [a[0][3], a[1][3], a[2][3]],
        scale,
        skew,
        rotation: quaternion_from_columns(c),
    })
}
fn slerp(mut a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut d = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    if d < 0. {
        for v in &mut b {
            *v = -*v
        }
        d = -d
    }
    let (wa, wb) = if d > 0.9995 {
        (1. - t, t)
    } else {
        let angle = d.clamp(-1., 1.).acos();
        (
            ((1. - t) * angle).sin() / angle.sin(),
            (t * angle).sin() / angle.sin(),
        )
    };
    for i in 0..4 {
        a[i] = a[i] * wa + b[i] * wb
    }
    let n = a.iter().map(|v| v * v).sum::<f32>().sqrt();
    for v in &mut a {
        *v /= n
    }
    a
}
fn recompose(v: Decomposed) -> Matrix4 {
    let [x, y, z, w] = v.rotation;
    let r = Matrix4([
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
            0.,
        ],
        [
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
            0.,
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
            0.,
        ],
        [0., 0., 0., 1.],
    ]);
    let k = Matrix4([
        [1., v.skew[0], v.skew[1], 0.],
        [0., 1., v.skew[2], 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]);
    Matrix4::translation(v.translation[0], v.translation[1], v.translation[2])
        .mul(r)
        .mul(k)
        .mul(Matrix4::scale(v.scale[0], v.scale[1], v.scale[2]))
}
/// CSS Transforms 2 affine matrix decomposition: translation, scale, skew and
/// quaternion rotation are interpolated separately. Singular/projective input
/// falls back to element interpolation because it has no affine decomposition.
pub fn interpolate_matrix(a: Matrix4, b: Matrix4, t: f32) -> Matrix4 {
    if let (Some(x), Some(y)) = (decompose(a), decompose(b)) {
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let mut z = x;
        for i in 0..3 {
            z.translation[i] = lerp(x.translation[i], y.translation[i]);
            z.scale[i] = lerp(x.scale[i], y.scale[i]);
            z.skew[i] = lerp(x.skew[i], y.skew[i])
        }
        z.rotation = slerp(x.rotation, y.rotation, t);
        recompose(z)
    } else {
        let mut z = a;
        for i in 0..4 {
            for j in 0..4 {
                z.0[i][j] = a.0[i][j] + (b.0[i][j] - a.0[i][j]) * t
            }
        }
        z
    }
}
impl Compositor {
    pub fn add_animation(
        &mut self,
        start_ms: f64,
        timing: Timing,
        target: AnimationTarget,
        keyframes: Vec<Keyframe>,
    ) -> Result<usize, AnimationError> {
        if !start_ms.is_finite() || !timing.valid() {
            return Err(AnimationError::InvalidTiming);
        }
        if keyframes.len() < 2
            || keyframes.first().unwrap().offset != 0.
            || keyframes.last().unwrap().offset != 1.
            || !keyframes.windows(2).all(|w| w[0].offset <= w[1].offset)
        {
            return Err(AnimationError::InvalidKeyframes);
        }
        if !keyframes.iter().all(|k| {
            k.offset.is_finite()
                && k.easing.valid()
                && match (target, k.value) {
                    (AnimationTarget::Opacity(_), AnimatedValue::Opacity(v)) => v.is_finite(),
                    (AnimationTarget::Transform(_), AnimatedValue::Transform(m)) => {
                        m.0.iter().flatten().all(|v| v.is_finite())
                    }
                    _ => false,
                }
        }) {
            return Err(AnimationError::InvalidKeyframes);
        }
        let base = match target {
            AnimationTarget::Transform(id) => self
                .trees
                .transform(id)
                .filter(|_| id.0 != 0)
                .map(|n| AnimatedValue::Transform(n.matrix)),
            AnimationTarget::Opacity(id) => self
                .trees
                .effect(id)
                .filter(|_| id.0 != 0)
                .map(|n| AnimatedValue::Opacity(n.opacity)),
        }
        .ok_or(AnimationError::InvalidTarget)?;
        let id = self.animations.len();
        self.animations.push(CompositorAnimation {
            start_ms,
            timing,
            target,
            keyframes,
            base,
        });
        Ok(id)
    }
    /// Samples the monotonic timeline and mutates only compositor property trees.
    pub fn tick(&mut self, now_ms: f64) -> Result<usize, TreeError> {
        let mut changed = 0;
        for animation in &self.animations {
            let value = animation.sample(now_ms).unwrap_or(animation.base);
            match (animation.target, value) {
                (AnimationTarget::Opacity(id), AnimatedValue::Opacity(v)) => {
                    if self.trees.effect(id).is_some_and(|n| n.opacity != v) {
                        self.trees.set_opacity(id, v)?;
                        changed += 1
                    }
                }
                (AnimationTarget::Transform(id), AnimatedValue::Transform(m)) => {
                    if self.trees.transform(id).is_some_and(|n| n.matrix != m) {
                        self.trees.set_transform(id, m)?;
                        changed += 1
                    }
                }
                _ => {}
            }
        }
        Ok(changed)
    }
    pub fn animations(&self) -> &[CompositorAnimation] {
        &self.animations
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spec_timing_delay_iterations_and_direction() {
        let t = Timing {
            delay_ms: 3000.,
            duration_ms: 2000.,
            iterations: 2.,
            direction: Direction::Normal,
            fill: Fill::Both,
            easing: Easing::Linear,
        };
        assert_eq!(t.progress(6000.), Some(0.5));
        assert_eq!(t.progress(3000.), Some(0.));
        assert_eq!(t.progress(5000.), Some(0.));
        assert_eq!(t.progress(7000.), Some(1.));
        let alt = Timing {
            direction: Direction::Alternate,
            ..t
        };
        assert_eq!(alt.progress(6000.), Some(0.5));
        assert_eq!(alt.progress(5000.), Some(1.));
        assert_eq!(alt.progress(7000.), Some(0.));
    }
    #[test]
    fn spec_easing_examples() {
        assert!((Easing::CubicBezier(0., 0., 1., 1.).apply(0.25) - 0.25).abs() < 1e-8);
        assert_eq!(Easing::Steps(4, StepPosition::JumpEnd).apply(0.375), 0.25);
        assert_eq!(Easing::Steps(4, StepPosition::JumpStart).apply(0.), 0.25);
        assert_eq!(
            Easing::LinearStops(vec![(0., 0.), (0.75, 0.25), (1., 1.)]).apply(0.75),
            0.25
        );
        let steps = Timing {
            delay_ms: 100.,
            duration_ms: 1000.,
            iterations: 1.,
            direction: Direction::Normal,
            fill: Fill::Both,
            easing: Easing::Steps(4, StepPosition::JumpStart),
        };
        assert_eq!(steps.progress(50.), Some(0.));
        assert_eq!(steps.progress(100.), Some(0.25));
        assert_eq!(steps.progress(1100.), Some(1.));
    }
    #[test]
    fn decomposed_rotation_follows_short_arc() {
        let a = Matrix4::rotation_z(170f32.to_radians());
        let b = Matrix4::rotation_z((-170f32).to_radians());
        let mid = interpolate_matrix(a, b, 0.5);
        let p = mid.map(1., 0.).unwrap();
        assert!((p.0 + 1.).abs() < 1e-4);
        assert!(p.1.abs() < 1e-4);
        let scale = interpolate_matrix(Matrix4::IDENTITY, Matrix4::scale(3., 5., 7.), 0.5);
        assert_eq!(scale.map(1., 1.), Some((2., 3.)));
        assert!((scale.0[2][2] - 4.).abs() < 1e-5);
    }
}
