use crate::{Bitmap, Point};
use amazing_css::color::parse_css_color;
use std::f64::consts::TAU;

#[derive(Debug, Clone, PartialEq)]
pub enum PaintStyle {
    Color([u8; 4]),
    LinearGradient(Gradient),
    RadialGradient(Gradient),
    ConicGradient(Gradient),
    Pattern(Pattern),
}

impl Default for PaintStyle {
    fn default() -> Self {
        Self::Color([0, 0, 0, 255])
    }
}

impl PaintStyle {
    pub fn from_css_color(value: &str) -> Option<Self> {
        parse_css_color(value).map(Self::Color)
    }
    pub fn sample(&self, p: Point) -> [u8; 4] {
        match self {
            Self::Color(c) => *c,
            Self::LinearGradient(g) => g.color(g.linear_t(p)),
            Self::RadialGradient(g) => g.color(g.radial_t(p)),
            Self::ConicGradient(g) => g.color(g.conic_t(p)),
            Self::Pattern(pattern) => pattern.sample(p),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    pub offset: f64,
    pub color: [u8; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub start: Point,
    pub end: Point,
    pub start_radius: f64,
    pub end_radius: f64,
    pub angle: f64,
    pub stops: Vec<ColorStop>,
}

impl Gradient {
    pub fn linear(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            start: Point::new(x0, y0),
            end: Point::new(x1, y1),
            start_radius: 0.,
            end_radius: 0.,
            angle: 0.,
            stops: Vec::new(),
        }
    }
    pub fn radial(x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64) -> Self {
        Self {
            start: Point::new(x0, y0),
            end: Point::new(x1, y1),
            start_radius: r0,
            end_radius: r1,
            angle: 0.,
            stops: Vec::new(),
        }
    }
    pub fn conic(angle: f64, x: f64, y: f64) -> Self {
        Self {
            start: Point::new(x, y),
            end: Point::new(x, y),
            start_radius: 0.,
            end_radius: 0.,
            angle,
            stops: Vec::new(),
        }
    }
    pub fn add_color_stop(&mut self, offset: f64, color: &str) -> Result<(), GradientError> {
        if !offset.is_finite() || !(0. ..=1.).contains(&offset) {
            return Err(GradientError::InvalidOffset);
        }
        let color = parse_css_color(color).ok_or(GradientError::InvalidColor)?;
        let at = self.stops.partition_point(|stop| stop.offset <= offset);
        self.stops.insert(at, ColorStop { offset, color });
        Ok(())
    }
    fn color(&self, t: f64) -> [u8; 4] {
        let Some(first) = self.stops.first() else {
            return [0; 4];
        };
        if t <= first.offset {
            return first.color;
        }
        for pair in self.stops.windows(2) {
            if t <= pair[1].offset {
                let q = if pair[0].offset == pair[1].offset {
                    1.
                } else {
                    (t - pair[0].offset) / (pair[1].offset - pair[0].offset)
                };
                return interpolate_color(pair[0].color, pair[1].color, q);
            }
        }
        self.stops.last().unwrap().color
    }
    fn linear_t(&self, p: Point) -> f64 {
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;
        let d = dx * dx + dy * dy;
        if d <= 1e-12 {
            return 1.;
        }
        ((p.x - self.start.x) * dx + (p.y - self.start.y) * dy) / d
    }
    fn radial_t(&self, p: Point) -> f64 {
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;
        let dr = self.end_radius - self.start_radius;
        let px = p.x - self.start.x;
        let py = p.y - self.start.y;
        let a = dx * dx + dy * dy - dr * dr;
        let b = -2. * (px * dx + py * dy + self.start_radius * dr);
        let c = px * px + py * py - self.start_radius * self.start_radius;
        if a.abs() < 1e-12 {
            return if b.abs() < 1e-12 { 0. } else { -c / b };
        }
        let disc = b * b - 4. * a * c;
        if disc < 0. {
            return f64::NAN;
        }
        let u = (-b - disc.sqrt()) / (2. * a);
        let v = (-b + disc.sqrt()) / (2. * a);
        if v >= 0. {
            v
        } else {
            u
        }
    }
    fn conic_t(&self, p: Point) -> f64 {
        ((p.y - self.start.y).atan2(p.x - self.start.x) - self.angle).rem_euclid(TAU) / TAU
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientError {
    InvalidOffset,
    InvalidColor,
}

fn interpolate_color(a: [u8; 4], b: [u8; 4], t: f64) -> [u8; 4] {
    let t = t.clamp(0., 1.);
    let aa = a[3] as f64 / 255.;
    let ba = b[3] as f64 / 255.;
    let alpha = aa + (ba - aa) * t;
    if alpha <= 0. {
        return [0; 4];
    }
    let mut out = [0; 4];
    for i in 0..3 {
        out[i] = ((a[i] as f64 * aa * (1. - t) + b[i] as f64 * ba * t) / alpha)
            .round()
            .clamp(0., 255.) as u8;
    }
    out[3] = (alpha * 255.).round().clamp(0., 255.) as u8;
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repetition {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub bitmap: Bitmap,
    pub repetition: Repetition,
    pub offset: Point,
}

impl Pattern {
    pub fn sample(&self, p: Point) -> [u8; 4] {
        let w = self.bitmap.width as i64;
        let h = self.bitmap.height as i64;
        if w == 0 || h == 0 {
            return [0; 4];
        }
        let mut x = (p.x - self.offset.x).floor() as i64;
        let mut y = (p.y - self.offset.y).floor() as i64;
        if matches!(self.repetition, Repetition::Repeat | Repetition::RepeatX) {
            x = x.rem_euclid(w);
        }
        if matches!(self.repetition, Repetition::Repeat | Repetition::RepeatY) {
            y = y.rem_euclid(h);
        }
        if x < 0 || y < 0 || x >= w || y >= h {
            return [0; 4];
        }
        self.bitmap.pixel(x as u32, y as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositeOperation {
    #[default]
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Lighter,
    Copy,
    Xor,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl CompositeOperation {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "source-over" => Self::SourceOver,
            "source-in" => Self::SourceIn,
            "source-out" => Self::SourceOut,
            "source-atop" => Self::SourceAtop,
            "destination-over" => Self::DestinationOver,
            "destination-in" => Self::DestinationIn,
            "destination-out" => Self::DestinationOut,
            "destination-atop" => Self::DestinationAtop,
            "lighter" => Self::Lighter,
            "copy" => Self::Copy,
            "xor" => Self::Xor,
            "multiply" => Self::Multiply,
            "screen" => Self::Screen,
            "overlay" => Self::Overlay,
            "darken" => Self::Darken,
            "lighten" => Self::Lighten,
            "color-dodge" => Self::ColorDodge,
            "color-burn" => Self::ColorBurn,
            "hard-light" => Self::HardLight,
            "soft-light" => Self::SoftLight,
            "difference" => Self::Difference,
            "exclusion" => Self::Exclusion,
            "hue" => Self::Hue,
            "saturation" => Self::Saturation,
            "color" => Self::Color,
            "luminosity" => Self::Luminosity,
            _ => return None,
        })
    }
}

pub(crate) fn composite(dst: [u8; 4], src: [u8; 4], mode: CompositeOperation) -> [u8; 4] {
    let sa = src[3] as f64 / 255.;
    let da = dst[3] as f64 / 255.;
    let s = [
        src[0] as f64 / 255.,
        src[1] as f64 / 255.,
        src[2] as f64 / 255.,
    ];
    let d = [
        dst[0] as f64 / 255.,
        dst[1] as f64 / 255.,
        dst[2] as f64 / 255.,
    ];
    let (sf, df) = match mode {
        CompositeOperation::SourceOver => (1., 1. - sa),
        CompositeOperation::SourceIn => (da, 0.),
        CompositeOperation::SourceOut => (1. - da, 0.),
        CompositeOperation::SourceAtop => (da, 1. - sa),
        CompositeOperation::DestinationOver => (1. - da, 1.),
        CompositeOperation::DestinationIn => (0., sa),
        CompositeOperation::DestinationOut => (0., 1. - sa),
        CompositeOperation::DestinationAtop => (1. - da, sa),
        CompositeOperation::Lighter => (1., 1.),
        CompositeOperation::Copy => (1., 0.),
        CompositeOperation::Xor => (1. - da, 1. - sa),
        _ => (1., 1. - sa),
    };
    let blend = matches!(
        mode,
        CompositeOperation::Multiply
            | CompositeOperation::Screen
            | CompositeOperation::Overlay
            | CompositeOperation::Darken
            | CompositeOperation::Lighten
            | CompositeOperation::ColorDodge
            | CompositeOperation::ColorBurn
            | CompositeOperation::HardLight
            | CompositeOperation::SoftLight
            | CompositeOperation::Difference
            | CompositeOperation::Exclusion
            | CompositeOperation::Hue
            | CompositeOperation::Saturation
            | CompositeOperation::Color
            | CompositeOperation::Luminosity
    );
    let alpha = if blend {
        sa + da * (1. - sa)
    } else {
        (sa * sf + da * df).min(1.)
    };
    if alpha <= 0. {
        return [0; 4];
    }
    let mixed = if blend {
        blend_rgb(s, d, mode)
    } else {
        [0.; 3]
    };
    let mut result = [0; 4];
    for k in 0..3 {
        let value = if blend {
            sa * (1. - da) * s[k] + da * (1. - sa) * d[k] + sa * da * mixed[k]
        } else {
            s[k] * sa * sf + d[k] * da * df
        };
        result[k] = (value / alpha * 255.).round().clamp(0., 255.) as u8;
    }
    result[3] = (alpha * 255.).round().clamp(0., 255.) as u8;
    result
}

fn blend_rgb(s: [f64; 3], d: [f64; 3], mode: CompositeOperation) -> [f64; 3] {
    use CompositeOperation::*;
    match mode {
        Hue => set_lum(set_sat(s, saturation(d)), lum(d)),
        Saturation => set_lum(set_sat(d, saturation(s)), lum(d)),
        Color => set_lum(s, lum(d)),
        Luminosity => set_lum(d, lum(s)),
        _ => std::array::from_fn(|i| {
            let a = s[i];
            let b = d[i];
            match mode {
                Multiply => a * b,
                Screen => a + b - a * b,
                Overlay => {
                    if b <= 0.5 {
                        2. * a * b
                    } else {
                        1. - 2. * (1. - a) * (1. - b)
                    }
                }
                Darken => a.min(b),
                Lighten => a.max(b),
                ColorDodge => {
                    if a >= 1. {
                        1.
                    } else {
                        (b / (1. - a)).min(1.)
                    }
                }
                ColorBurn => {
                    if a <= 0. {
                        0.
                    } else {
                        1. - ((1. - b) / a).min(1.)
                    }
                }
                HardLight => {
                    if a <= 0.5 {
                        2. * a * b
                    } else {
                        1. - 2. * (1. - a) * (1. - b)
                    }
                }
                SoftLight => {
                    if a <= 0.5 {
                        b - (1. - 2. * a) * b * (1. - b)
                    } else {
                        b + (2. * a - 1.)
                            * (if b <= 0.25 {
                                ((16. * b - 12.) * b + 4.) * b
                            } else {
                                b.sqrt()
                            } - b)
                    }
                }
                Difference => (b - a).abs(),
                Exclusion => a + b - 2. * a * b,
                _ => a,
            }
        }),
    }
}

fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn saturation(c: [f64; 3]) -> f64 {
    c.iter().copied().fold(0., f64::max) - c.iter().copied().fold(1., f64::min)
}
fn clip_color(mut c: [f64; 3]) -> [f64; 3] {
    let l = lum(c);
    let min = c.iter().copied().fold(f64::INFINITY, f64::min);
    let max = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if min < 0. {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - min);
        }
    }
    if max > 1. {
        for v in &mut c {
            *v = l + (*v - l) * (1. - l) / (max - l);
        }
    }
    c.map(|v| v.clamp(0., 1.))
}
fn set_lum(mut c: [f64; 3], l: f64) -> [f64; 3] {
    let d = l - lum(c);
    for v in &mut c {
        *v += d;
    }
    clip_color(c)
}
fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let mut order = [0usize, 1, 2];
    order.sort_by(|a, b| c[*a].total_cmp(&c[*b]));
    let mut out = [0.; 3];
    let (lo, mid, hi) = (order[0], order[1], order[2]);
    if c[hi] > c[lo] {
        out[mid] = (c[mid] - c[lo]) * s / (c[hi] - c[lo]);
        out[hi] = s;
    }
    out
}
