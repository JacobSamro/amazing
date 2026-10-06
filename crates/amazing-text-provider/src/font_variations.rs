//! Resolve the selected CSS face's registered axes before author feature
//! variations. Shaping and rasterization must use identical coordinates.

use super::{FontRecord, FontStyleDescriptor};
use amazing_text::{FontSlant, TextStyle, VariationSetting};

pub(super) fn for_face(font: &FontRecord, style: &TextStyle) -> Vec<VariationSetting> {
    let mut axes = Vec::with_capacity(style.variations.len() + 5);
    // CSS Fonts 4 section 7.2: `font-optical-sizing: auto` (the initial and
    // default value) sets the `opsz` axis to the used font size unless an
    // explicit `font-variation-settings` value overrides it below. A font
    // without a registered `opsz` axis silently ignores this coordinate.
    axes.push(VariationSetting {
        tag: *b"opsz",
        value: style.size,
    });
    if let Some(face) = &font.author_face {
        axes.push(VariationSetting {
            tag: *b"wght",
            value: face.weight.nearest(style.weight as f32),
        });
        axes.push(VariationSetting {
            tag: *b"wdth",
            value: face
                .stretch
                .nearest(super::font_matching::requested_width(style)),
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
                // CSS Fonts 4 section 6.3 (and 7.2's font-variation-settings
                // note): the OpenType `slnt` axis is positive
                // counterclockwise, the opposite of CSS's positive-clockwise
                // `font-style: oblique <angle>`. The face-selection clamp
                // above stays in CSS-degree space (matching the descriptor's
                // author-written range and the matching algorithm's own
                // distance math); only the coordinate actually fed to the
                // variable font is negated.
                value: -super::font_matching::requested_oblique_angle(style)
                    .clamp(min_degrees, max_degrees),
            }),
            _ => {}
        }
    }
    if let Some(face) = &font.installed_face {
        if face.registered_axes.contains(b"wght") { axes.push(VariationSetting { tag: *b"wght", value: face.weight_range.nearest(style.weight as f32) }); }
        if face.registered_axes.contains(b"wdth") { axes.push(VariationSetting { tag: *b"wdth", value: face.stretch_range.nearest(super::font_matching::requested_width(style)) }); }
        if let Some(range) = face.italic_range {
            axes.push(VariationSetting { tag: *b"ital", value: range.nearest(if style.slant == FontSlant::Italic { 1.0 } else { 0.0 }) });
        }
        if let Some(range) = face.oblique_range
            && !(style.slant == FontSlant::Italic && face.italic_range.is_some_and(|range| range.min <= 1.0 && range.max >= 1.0))
        {
            let requested = match style.slant { FontSlant::Normal => 0.0, FontSlant::Italic => 11.0, FontSlant::Oblique => super::font_matching::requested_oblique_angle(style) };
            axes.push(VariationSetting { tag: *b"slnt", value: -range.nearest(requested) });
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

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_text::FontKey;

    fn plain_record() -> FontRecord {
        FontRecord {
            installed_aliases: std::collections::BTreeSet::new(),
            installed_face: None,
            key: FontKey(1),
            family: "test".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            sfnt: Vec::new(),
            author_face: None,
        }
    }

    fn axis(axes: &[VariationSetting], tag: [u8; 4]) -> Option<f32> {
        axes.iter().find(|axis| axis.tag == tag).map(|axis| axis.value)
    }

    #[test]
    fn opsz_auto_tracks_used_font_size() {
        let mut style = TextStyle::default();
        style.size = 32.0;
        let axes = for_face(&plain_record(), &style);
        assert_eq!(axis(&axes, *b"opsz"), Some(32.0));
    }

    #[test]
    fn opsz_follows_size_changes_without_author_face() {
        let mut style = TextStyle::default();
        style.size = 9.0;
        let small = for_face(&plain_record(), &style);
        style.size = 96.0;
        let large = for_face(&plain_record(), &style);
        assert_eq!(axis(&small, *b"opsz"), Some(9.0));
        assert_eq!(axis(&large, *b"opsz"), Some(96.0));
    }

    #[test]
    fn explicit_variation_settings_override_automatic_opsz() {
        let mut style = TextStyle::default();
        style.size = 20.0;
        style.variations.push(VariationSetting {
            tag: *b"opsz",
            value: 144.0,
        });
        let axes = for_face(&plain_record(), &style);
        // Only one `opsz` entry survives, and it is the explicit value, not
        // the automatic used-size value.
        let matches: Vec<f32> = axes
            .iter()
            .filter(|axis| axis.tag == *b"opsz")
            .map(|axis| axis.value)
            .collect();
        assert_eq!(matches, vec![144.0]);
    }

    #[test]
    fn opsz_coexists_with_author_face_weight_axis() {
        use super::super::{FontDescriptorRange, FontFaceMetadata, FontDisplay};

        let mut style = TextStyle::default();
        style.size = 48.0;
        style.weight = 700;
        let record = FontRecord {
            installed_aliases: std::collections::BTreeSet::new(),
            installed_face: None,
            key: FontKey(2),
            family: "variable-test".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            sfnt: Vec::new(),
            author_face: Some(FontFaceMetadata {
                declaration_order:0,
                weight: FontDescriptorRange {
                    min: 100.0,
                    max: 900.0,
                },
                stretch: FontDescriptorRange::single(100.0),
                style: FontStyleDescriptor::Normal,
                unicode_ranges: Vec::new(),
                display: FontDisplay::Auto,
                size_adjust: 1.0,
                ascent_override: None,
                descent_override: None,
                line_gap_override: None,
                features: Vec::new(),
            }),
        };
        let axes = for_face(&record, &style);
        assert_eq!(axis(&axes, *b"opsz"), Some(48.0));
        assert_eq!(axis(&axes, *b"wght"), Some(700.0));
    }
}
