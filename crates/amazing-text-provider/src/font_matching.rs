//! CSS Fonts 4 section 5.2 face ranking, within one already selected family.

use super::{FontDescriptorRange, FontRecord, FontStyleDescriptor};
use amazing_text::{FontSlant, TextStyle};

pub(super) fn requested_width(style: &TextStyle) -> f32 {
    if style.stretch_percent.is_finite() && style.stretch_percent > 0.0 {
        style.stretch_percent
    } else {
        style.stretch as f32
    }
}

pub(super) fn requested_oblique_angle(style: &TextStyle) -> f32 {
    style
        .oblique_angle_degrees
        .filter(|angle| angle.is_finite())
        .unwrap_or(14.0)
        .clamp(-90.0, 90.0)
}

// f32::to_bits preserves order for these nonnegative finite distances.
fn distance(value: f32) -> u32 {
    value.to_bits()
}

fn width_rank(request: f32, range: FontDescriptorRange) -> (u8, u32) {
    let nearest = range.nearest(request);
    let wrong_side = if request <= 100.0 {
        nearest > request
    } else {
        nearest < request
    };
    (u8::from(wrong_side), distance((nearest - request).abs()))
}

fn weight_rank(request: f32, range: FontDescriptorRange) -> (u8, u32) {
    let nearest = range.nearest(request);
    let stage = if (400.0..=500.0).contains(&request) {
        if nearest >= request && nearest <= 500.0 {
            0
        } else if nearest < request {
            1
        } else {
            2
        }
    } else if request < 400.0 {
        u8::from(nearest > request)
    } else {
        u8::from(nearest < request)
    };
    (stage, distance((nearest - request).abs()))
}

fn style_rank(style: &TextStyle, descriptor: FontStyleDescriptor) -> (u8, u32) {
    let angle = match descriptor {
        FontStyleDescriptor::Normal => 0.0,
        FontStyleDescriptor::Italic => {
            return match style.slant {
                FontSlant::Italic => (0, 0),
                FontSlant::Normal => (1, 0),
                FontSlant::Oblique => (2, 0),
            };
        }
        FontStyleDescriptor::Oblique {
            min_degrees,
            max_degrees,
        } => {
            let wanted = match style.slant {
                FontSlant::Oblique => requested_oblique_angle(style),
                FontSlant::Italic => 11.0,
                FontSlant::Normal => 0.0,
            };
            wanted.clamp(min_degrees, max_degrees)
        }
    };
    match style.slant {
        FontSlant::Normal => {
            if angle >= 0.0 {
                (0, distance(angle))
            } else {
                (2, distance(-angle))
            }
        }
        FontSlant::Italic => {
            if angle >= 11.0 {
                (1, distance(angle - 11.0))
            } else if angle > 0.0 {
                (2, distance(11.0 - angle))
            } else if angle == 0.0 {
                (3, 0)
            } else {
                (4, distance(-angle))
            }
        }
        FontSlant::Oblique => {
            let request = requested_oblique_angle(style);
            let sign = if request < 0.0 { -1.0 } else { 1.0 };
            let requested = request * sign;
            let candidate = angle * sign;
            if candidate >= 0.0 {
                if (0.0..11.0).contains(&requested) {
                    if candidate <= requested {
                        (0, distance(requested - candidate))
                    } else {
                        (1, distance(candidate - requested))
                    }
                } else if candidate == 0.0 {
                    (3, 0)
                } else if candidate >= requested {
                    (0, distance(candidate - requested))
                } else {
                    (1, distance(requested - candidate))
                }
            } else {
                (3, distance(-candidate))
            }
        }
    }
}

pub(super) fn rank(style: &TextStyle, font: &FontRecord) -> (u8, u32, u8, u32, u8, u32) {
    let (width, slant, weight) = if let Some(face) = &font.author_face {
        (face.stretch, face.style, face.weight)
    } else if let Some(face) = &font.installed_face {
        (face.stretch_range, face.style_descriptor(style), face.weight_range)
    } else {
        let slant = match font.slant {
            FontSlant::Normal => FontStyleDescriptor::Normal,
            FontSlant::Italic => FontStyleDescriptor::Italic,
            FontSlant::Oblique => FontStyleDescriptor::Oblique {
                min_degrees: 14.0,
                max_degrees: 14.0,
            },
        };
        (
            FontDescriptorRange::single(font.stretch as f32),
            slant,
            FontDescriptorRange::single(font.weight as f32),
        )
    };
    rank_descriptors(style,width,slant,weight)
}
pub(super) fn rank_descriptors(style:&TextStyle,width:FontDescriptorRange,slant:FontStyleDescriptor,weight:FontDescriptorRange)
    ->(u8,u32,u8,u32,u8,u32){
    let (width_side, width_distance) = width_rank(requested_width(style), width);
    let (style_stage, style_distance) = style_rank(style, slant);
    let (weight_stage, weight_distance) = weight_rank(style.weight as f32, weight);
    (
        width_side,
        width_distance,
        style_stage,
        style_distance,
        weight_stage,
        weight_distance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_width_and_direction_come_before_style_and_weight() {
        let style = TextStyle {
            stretch_percent: 62.5,
            weight: 900,
            ..TextStyle::default()
        };
        let exact = FontDescriptorRange {
            min: 62.5,
            max: 62.5,
        };
        let rounded = FontDescriptorRange {
            min: 63.0,
            max: 63.0,
        };
        assert!(
            width_rank(requested_width(&style), exact)
                < width_rank(requested_width(&style), rounded)
        );
        assert!(
            width_rank(75.0, FontDescriptorRange::single(70.0))
                < width_rank(75.0, FontDescriptorRange::single(76.0))
        );
        assert!(
            width_rank(125.0, FontDescriptorRange::single(130.0))
                < width_rank(125.0, FontDescriptorRange::single(120.0))
        );
    }

    #[test]
    fn exact_oblique_angle_and_style_order() {
        let style = TextStyle {
            slant: FontSlant::Oblique,
            oblique_angle_degrees: Some(-24.5),
            ..TextStyle::default()
        };
        let exact = FontStyleDescriptor::Oblique {
            min_degrees: -24.5,
            max_degrees: -24.5,
        };
        let near = FontStyleDescriptor::Oblique {
            min_degrees: -25.0,
            max_degrees: -25.0,
        };
        assert!(style_rank(&style, exact) < style_rank(&style, near));
        assert!(style_rank(&style, near) < style_rank(&style, FontStyleDescriptor::Italic));
        let italic = TextStyle {
            slant: FontSlant::Italic,
            ..TextStyle::default()
        };
        assert_eq!(
            style_rank(
                &italic,
                FontStyleDescriptor::Oblique {
                    min_degrees: 0.0,
                    max_degrees: 20.0
                }
            ),
            (1, 0)
        );
    }

    #[test]
    fn weight_four_hundred_to_five_hundred_order() {
        assert!(
            weight_rank(450.0, FontDescriptorRange::single(500.0))
                < weight_rank(450.0, FontDescriptorRange::single(400.0))
        );
        assert!(
            weight_rank(450.0, FontDescriptorRange::single(400.0))
                < weight_rank(450.0, FontDescriptorRange::single(501.0))
        );
    }
}
