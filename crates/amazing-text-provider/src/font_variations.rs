//! Resolve the selected CSS face's registered axes before author feature
//! variations. Shaping and rasterization must use identical coordinates.

use super::{FontRecord, FontStyleDescriptor};
use amazing_text::{FontSlant, TextStyle, VariationSetting};

pub(super) fn for_face(font: &FontRecord, style: &TextStyle) -> Vec<VariationSetting> {
    let mut axes = Vec::with_capacity(style.variations.len() + 4);
    if let Some(face) = &font.author_face {
        axes.push(VariationSetting {
            tag: *b"wght",
            value: face.weight.nearest(style.weight) as f32,
        });
        axes.push(VariationSetting {
            tag: *b"wdth",
            value: face.stretch.nearest(style.stretch) as f32,
        });
        match (style.slant, face.style) {
            (FontSlant::Italic, FontStyleDescriptor::Italic) => axes.push(VariationSetting {
                tag: *b"ital",
                value: 1.0,
            }),
            (
                FontSlant::Oblique,
                FontStyleDescriptor::Oblique {
                    min_degrees,
                    max_degrees,
                },
            ) => axes.push(VariationSetting {
                tag: *b"slnt",
                value: 14.0f32.clamp(min_degrees, max_degrees),
            }),
            _ => {}
        }
    }
    for requested in &style.variations {
        if let Some(axis) = axes.iter_mut().find(|axis| axis.tag == requested.tag) {
            *axis = requested.clone();
        } else {
            axes.push(requested.clone());
        }
    }
    axes
}
