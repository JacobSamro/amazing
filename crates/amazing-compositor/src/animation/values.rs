//! Typed animation values supplied after CSS computed-value resolution.
use super::interpolate_matrix;
use crate::Matrix4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    LinearSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationColor {
    pub components: [f32; 4],
    pub space: ColorSpace,
}

impl AnimationColor {
    pub fn rgba8(red: u8, green: u8, blue: u8, alpha: u8, space: ColorSpace) -> Self {
        Self {
            components: [
                red as f32 / 255.,
                green as f32 / 255.,
                blue as f32 / 255.,
                alpha as f32 / 255.,
            ],
            space,
        }
    }
    fn in_space(self, space: ColorSpace) -> Self {
        if self.space == space {
            return self;
        }
        let mut out = self;
        for channel in &mut out.components[..3] {
            *channel = match space {
                ColorSpace::LinearSrgb => {
                    if *channel <= 0.04045 {
                        *channel / 12.92
                    } else {
                        ((*channel + 0.055) / 1.055).powf(2.4)
                    }
                }
                ColorSpace::Srgb => {
                    if *channel <= 0.0031308 {
                        *channel * 12.92
                    } else {
                        1.055 * channel.powf(1. / 2.4) - 0.055
                    }
                }
            };
        }
        out.space = space;
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnimationValue {
    LengthPx(f32),
    Percentage(f32),
    Number(f32),
    Opacity(f32),
    Color(AnimationColor),
    Transform(Matrix4),
    Discrete(String),
}

impl AnimationValue {
    pub fn interpolate(&self, other: &Self, fraction: f32) -> Self {
        let t = fraction.clamp(0., 1.);
        match (self, other) {
            (Self::LengthPx(a), Self::LengthPx(b)) => Self::LengthPx(a + (b - a) * t),
            (Self::Percentage(a), Self::Percentage(b)) => Self::Percentage(a + (b - a) * t),
            (Self::Number(a), Self::Number(b)) => Self::Number(a + (b - a) * t),
            (Self::Opacity(a), Self::Opacity(b)) => Self::Opacity((a + (b - a) * t).clamp(0., 1.)),
            (Self::Transform(a), Self::Transform(b)) => {
                Self::Transform(interpolate_matrix(*a, *b, t))
            }
            (Self::Color(a), Self::Color(b)) => {
                let b = b.in_space(a.space);
                let mut out = *a;
                for i in 0..4 {
                    out.components[i] = a.components[i] + (b.components[i] - a.components[i]) * t
                }
                Self::Color(out)
            }
            _ if t < 0.5 => self.clone(),
            _ => other.clone(),
        }
    }
    pub fn add(&self, other: &Self) -> Option<Self> {
        Some(match (self, other) {
            (Self::LengthPx(a), Self::LengthPx(b)) => Self::LengthPx(a + b),
            (Self::Percentage(a), Self::Percentage(b)) => Self::Percentage(a + b),
            (Self::Number(a), Self::Number(b)) => Self::Number(a + b),
            (Self::Opacity(a), Self::Opacity(b)) => Self::Opacity((a + b).clamp(0., 1.)),
            (Self::Transform(a), Self::Transform(b)) => Self::Transform(a.mul(*b)),
            (Self::Color(a), Self::Color(b)) => {
                let b = b.in_space(a.space);
                let mut out = *a;
                for i in 0..4 {
                    out.components[i] = (a.components[i] + b.components[i]).clamp(0., 1.)
                }
                Self::Color(out)
            }
            _ => return None,
        })
    }
    pub fn delta(&self, other: &Self) -> Option<Self> {
        Some(match (self, other) {
            (Self::LengthPx(a), Self::LengthPx(b)) => Self::LengthPx(b - a),
            (Self::Percentage(a), Self::Percentage(b)) => Self::Percentage(b - a),
            (Self::Number(a), Self::Number(b)) => Self::Number(b - a),
            (Self::Opacity(a), Self::Opacity(b)) => Self::Opacity(b - a),
            _ => return None,
        })
    }
    pub fn scaled(&self, count: u64) -> Option<Self> {
        let n = count as f32;
        Some(match self {
            Self::LengthPx(v) => Self::LengthPx(v * n),
            Self::Percentage(v) => Self::Percentage(v * n),
            Self::Number(v) => Self::Number(v * n),
            Self::Opacity(v) => Self::Opacity(v * n),
            _ => return None,
        })
    }
    pub fn finite(&self) -> bool {
        match self {
            Self::LengthPx(v) | Self::Percentage(v) | Self::Number(v) | Self::Opacity(v) => {
                v.is_finite()
            }
            Self::Color(c) => c.components.iter().all(|v| v.is_finite()),
            Self::Transform(m) => m.0.iter().flatten().all(|v| v.is_finite()),
            Self::Discrete(_) => true,
        }
    }
    pub fn compositor_eligible(property: &str, value: &Self) -> bool {
        matches!(
            (property, value),
            ("transform", Self::Transform(_)) | ("opacity", Self::Opacity(_))
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositeMode {
    Replace,
    Add,
    Accumulate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterationComposite {
    Replace,
    Accumulate,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_interpolation_and_discrete_boundary() {
        assert_eq!(
            AnimationValue::LengthPx(10.).interpolate(&AnimationValue::LengthPx(30.), 0.25),
            AnimationValue::LengthPx(15.)
        );
        assert_eq!(
            AnimationValue::Percentage(0.).interpolate(&AnimationValue::Percentage(100.), 0.5),
            AnimationValue::Percentage(50.)
        );
        assert_eq!(
            AnimationValue::Number(2.).interpolate(&AnimationValue::Number(4.), 0.5),
            AnimationValue::Number(3.)
        );
        let a = AnimationValue::Discrete("none".into());
        let b = AnimationValue::Discrete("block".into());
        assert_eq!(a.interpolate(&b, 0.49), a);
        assert_eq!(a.interpolate(&b, 0.5), b);
    }
    #[test]
    fn color_interpolates_in_requested_space() {
        let black = AnimationValue::Color(AnimationColor::rgba8(0, 0, 0, 255, ColorSpace::Srgb));
        let white =
            AnimationValue::Color(AnimationColor::rgba8(255, 255, 255, 255, ColorSpace::Srgb));
        let AnimationValue::Color(mid) = black.interpolate(&white, 0.5) else {
            panic!("color")
        };
        assert_eq!(mid.space, ColorSpace::Srgb);
        assert!((mid.components[0] - 0.5).abs() < 1e-6);
        let linear_white = AnimationValue::Color(AnimationColor::rgba8(
            255,
            255,
            255,
            255,
            ColorSpace::LinearSrgb,
        ));
        let AnimationValue::Color(converted) = black.interpolate(&linear_white, 0.5) else {
            panic!("color")
        };
        assert!((converted.components[0] - 0.5).abs() < 1e-6);
    }
}
