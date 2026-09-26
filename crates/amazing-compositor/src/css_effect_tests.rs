use super::*;
use amazing_css::{effects as css, values::{LengthPercentage, RelativeUnit}};
fn changes(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}
fn filters(value: &str) -> Result<Vec<FilterOperation>, CssEffectError> {
    resolve_css_filters(&css::parse_filter_list(value).unwrap(), [10, 20, 30, 128], |_| None)
}

#[test]
fn css_filter_directory_numeric_values_preserve_units_and_order() {
    for amount in [0., 0.25, 0.5, 1., 2., 10.] {
        for (name, make) in [
            ("brightness", FilterOperation::Brightness as fn(f32) -> FilterOperation),
            ("contrast", FilterOperation::Contrast),
            ("saturate", FilterOperation::Saturate),
            ("grayscale", FilterOperation::Grayscale),
            ("invert", FilterOperation::Invert),
            ("opacity", FilterOperation::Opacity),
            ("sepia", FilterOperation::Sepia),
        ] {
            let bounded = matches!(name, "grayscale" | "invert" | "opacity" | "sepia");
            let expected = make(if bounded { f32::min(amount, 1.) } else { amount });
            for css in [format!("{name}({amount})"), format!("{name}({}%)", amount * 100.)] {
                assert_eq!(filters(&css).unwrap(), [expected], "{css}");
            }
        }
    }
    for (css, degrees) in [("hue-rotate(90deg)", 90.), ("hue-rotate(.25turn)", 90.),
        ("hue-rotate(100grad)", 90.), ("hue-rotate(3.141592653589793rad)", 180.)] {
        assert_eq!(filters(css).unwrap(), [FilterOperation::HueRotate(degrees)]);
    }
    assert_eq!(filters("blur(1pt) opacity(50%) blur(2px)").unwrap(),
        [FilterOperation::Blur(4./3.), FilterOperation::Opacity(0.5), FilterOperation::Blur(2.)]);
    assert!(filters("none").unwrap().is_empty());
}

#[test]
fn css_shadow_uses_resolved_lengths_current_color_and_sigma() {
    let parsed = css::parse_filter_list("drop-shadow(2em -1rem 3px currentColor)").unwrap();
    let result = resolve_css_filters(&parsed, [1, 2, 3, 99], |v| match v {
        LengthPercentage::Relative(v, RelativeUnit::Em) => Some(v as f32 * 12.),
        LengthPercentage::Relative(v, RelativeUnit::Rem) => Some(v as f32 * 16.),
        _ => None,
    }).unwrap();
    assert_eq!(result, [FilterOperation::DropShadow {
        offset: (24., -16.), sigma: 3., color: [1, 2, 3, 99],
    }]);
    for (css, color) in [("drop-shadow(1px 2px)", [10,20,30,128]),
        ("drop-shadow(1px 2px transparent)", [0;4]),
        ("drop-shadow(1px 2px red)", [255,0,0,255])] {
        assert_eq!(filters(css).unwrap(), [FilterOperation::DropShadow {
            offset: (1., 2.), sigma: 0., color,
        }]);
    }
    for css in ["blur(5%)", "blur(2em)", "blur(calc(1px + 2px))", "url(#filter)"] {
        assert_eq!(filters(css), Err(CssEffectError::UnresolvedValue), "{css}");
    }
    for css in ["blur(129px)", "drop-shadow(0 0 -1px)", "brightness(10001)"] {
        assert!(filters(css).is_err(), "{css}");
    }
}

#[test]
fn css_all_supported_blends_map_and_unknown_effects_fail_closed() {
    for (name, expected) in [
        ("normal", EffectBlend::Normal), ("multiply", EffectBlend::Multiply),
        ("screen", EffectBlend::Screen), ("overlay", EffectBlend::Overlay),
        ("darken", EffectBlend::Darken), ("lighten", EffectBlend::Lighten),
        ("color-dodge", EffectBlend::ColorDodge), ("color-burn", EffectBlend::ColorBurn),
        ("hard-light", EffectBlend::HardLight), ("soft-light", EffectBlend::SoftLight),
        ("difference", EffectBlend::Difference), ("exclusion", EffectBlend::Exclusion),
        ("hue", EffectBlend::Hue), ("saturation", EffectBlend::Saturation),
        ("color", EffectBlend::Color), ("luminosity", EffectBlend::Luminosity),
        ("plus-lighter", EffectBlend::PlusLighter),
    ] {
        let result = resolve_css_effect_delta(&EffectOperations::default(),
            &changes(&[("mix-blend-mode", name), ("isolation", "isolate")]), None, [0;4], |_| None).unwrap();
        assert_eq!(result.blend, expected);
        assert!(result.isolate);
    }
    let initial = EffectOperations { filters: vec![FilterOperation::Blur(2.)], ..Default::default() };
    for (key, value) in [("mix-blend-mode", "invalid"), ("filter", "url(#f)"),
        ("clip-path", "circle(20%)"), ("mask-image", "url(mask.png)"),
        ("mask-size", "cover"), ("opacity", ".4"), ("isolation", "invalid")] {
        assert!(resolve_css_effect_delta(&initial,
            &changes(&[("filter", "blur(3px)"), (key, value)]), None, [0;4], |_| None).is_err());
        assert_eq!(initial.filters, [FilterOperation::Blur(2.)]);
    }
}

#[test]
fn css_backdrop_resolution_requires_bounds_and_preserves_other_effects() {
    let initial = EffectOperations { clip_path: Some(EffectClip::Ellipse {
        center: (10., 20.), radii: (5., 6.),
    }), ..Default::default() };
    let delta = changes(&[("filter", "blur(2px)"), ("backdrop-filter", "invert(1)")]);
    assert_eq!(resolve_css_effect_delta(&initial, &delta, None, [0;4], |_| None),
        Err(CssEffectError::UnresolvedValue));
    let bounds = Rect::new(5., 6., 20., 30.);
    let result = resolve_css_effect_delta(&initial, &delta, Some(bounds), [0;4], |_| None).unwrap();
    assert_eq!(result.clip_path, initial.clip_path);
    assert_eq!(result.backdrop_bounds, Some(bounds));
    assert_eq!(result.backdrop_filters, [FilterOperation::Invert(1.)]);
    let cleared = resolve_css_effect_delta(&result,
        &changes(&[("filter", "none"), ("backdrop-filter", "none")]), None, [0;4], |_| None).unwrap();
    assert!(cleared.filters.is_empty() && cleared.backdrop_filters.is_empty());
    assert_eq!(cleared.backdrop_bounds, None);
    assert_eq!(cleared.clip_path, initial.clip_path);
}
