use amazing_css::color::parse_css_color;
use amazing_css::{parse_stylesheet, compute};
use amazing_dom::{Dom, NodeIdx};

/// Two independently rounded color computations (e.g. a full `color-mix()`
/// vs. a directly parsed literal used as the expected reference) can differ
/// by ±1 in a channel from where the two paths round a `.5` boundary;
/// require exact alpha and near-exact channels instead of bit-for-bit
/// equality so the test tracks correctness, not a specific rounding tie-break.
fn assert_close(actual: Option<[u8; 4]>, expected: Option<[u8; 4]>, context: &str) {
    match (actual, expected) {
        (Some(a), Some(e)) => {
            assert_eq!(a[3], e[3], "{context}: alpha {a:?} vs {e:?}");
            for i in 0..3 {
                let diff = (a[i] as i16 - e[i] as i16).abs();
                assert!(diff <= 1, "{context}: channel {i} {a:?} vs {e:?}");
            }
        }
        (a, e) => assert_eq!(a, e, "{context}"),
    }
}

// CSS-24: `none`-channel support in the *absolute* color functions (CSS Color
// 4 SS4.4/SS13), color-mix() missing-component propagation and expanded space
// coverage with hue-interpolation methods (SS12.2, SS13), and calc-size()
// nesting (CSS Values 5 SS11) -- verified against the pinned reference this
// engine already trusts (`parse_css_color`, and equivalent explicit-value
// comparisons rather than raw byte math, so a rounding change in one call
// site does not desynchronize the test from the implementation).

#[test]
fn absolute_none_in_legacy_rgb_and_alpha() {
    assert_eq!(parse_css_color("rgb(none 128 0)"), Some([0, 128, 0, 255]));
    assert_eq!(parse_css_color("rgb(255 none 0)"), Some([255, 0, 0, 255]));
    assert_eq!(parse_css_color("rgb(255 0 0 / none)"), Some([255, 0, 0, 0]));
    assert_eq!(parse_css_color("rgba(none none none / none)"), Some([0, 0, 0, 0]));
}

#[test]
fn absolute_none_in_hsl_and_hwb() {
    assert_eq!(parse_css_color("hsl(none 50% 50%)"), parse_css_color("hsl(0deg 50% 50%)"));
    assert_eq!(parse_css_color("hsl(120deg none 50%)"), parse_css_color("hsl(120deg 0% 50%)"));
    assert_eq!(parse_css_color("hsl(120deg 50% none)"), parse_css_color("hsl(120deg 50% 0%)"));
    assert_eq!(parse_css_color("hwb(none 20% 30%)"), parse_css_color("hwb(0deg 20% 30%)"));
    assert_eq!(parse_css_color("hwb(90deg none 30%)"), parse_css_color("hwb(90deg 0% 30%)"));
    assert_eq!(parse_css_color("hwb(90deg 20% none)"), parse_css_color("hwb(90deg 20% 0%)"));
}

#[test]
fn absolute_none_in_lab_lch_oklab_oklch() {
    assert_eq!(parse_css_color("lab(50% none none)"), parse_css_color("lab(50% 0 0)"));
    assert_eq!(parse_css_color("lch(50% none none)"), parse_css_color("lch(50% 0 0deg)"));
    assert_eq!(parse_css_color("oklab(50% none none)"), parse_css_color("oklab(50% 0 0)"));
    assert_eq!(parse_css_color("oklch(50% none none)"), parse_css_color("oklch(50% 0 0deg)"));
    assert_eq!(parse_css_color("lab(none 20 -30)"), parse_css_color("lab(0% 20 -30)"));
    assert_eq!(parse_css_color("oklch(70% .1 none)"), parse_css_color("oklch(70% .1 0deg)"));
}

#[test]
fn absolute_none_in_predefined_color_function() {
    assert_eq!(
        parse_css_color("color(srgb none 0.5 0)"),
        parse_css_color("color(srgb 0 0.5 0)")
    );
    assert_eq!(
        parse_css_color("color(display-p3 0.1 none 0.1)"),
        parse_css_color("color(display-p3 0.1 0 0.1)")
    );
    assert_eq!(
        parse_css_color("color(xyz-d65 none none none)"),
        parse_css_color("color(xyz-d65 0 0 0)")
    );
}

#[test]
fn color_mix_propagates_missing_components_per_operand() {
    // CSS Color 4 SS13: missing in exactly one operand takes the other
    // operand's value directly, without applying that operand's own mix
    // weight; missing in both operands stays missing (rendered here as 0,
    // there being no further consumer in this direct-parse context).
    assert_eq!(
        parse_css_color("color-mix(in srgb, rgb(none 10 20) 50%, rgb(30 40 50) 50%)"),
        Some([30, 25, 35, 255])
    );
    assert_eq!(
        parse_css_color("color-mix(in srgb, rgb(none 10 20), rgb(none 40 50))"),
        Some([0, 25, 35, 255])
    );
    // Alpha follows the same substitution rule, but that only changes the
    // *alpha* channel's used value -- rgb still mixes by the normal
    // premultiplied weighted average, since neither operand's rgb channels
    // are themselves missing.
    assert_eq!(
        parse_css_color("color-mix(in srgb, rgb(10 20 30 / none), rgb(50 60 70 / 0.5))"),
        Some([30, 40, 50, 128])
    );
}

#[test]
fn color_mix_propagates_missing_hue_in_polar_spaces() {
    // A missing hue in one operand is not averaged against the other; the
    // other operand's hue passes through, and the non-hue channels still mix
    // normally (weighted average here, both operands' weights being equal).
    assert_close(
        parse_css_color("color-mix(in hsl, hsl(none 30% 40%), hsl(200deg 60% 60%))"),
        parse_css_color("hsl(200deg 45% 50%)"),
        "missing hue in one hsl operand",
    );
}

#[test]
fn color_mix_covers_the_expanded_rectangular_and_polar_spaces() {
    // Round-trip stability: mixing a color with itself 50/50 in any of the
    // newly supported spaces must reproduce that color exactly.
    for space in [
        "srgb", "srgb-linear", "lab", "oklab", "xyz", "xyz-d65", "xyz-d50",
        "hsl", "hwb", "lch", "oklch",
    ] {
        for color in ["red", "cornflowerblue", "#336699", "black", "white"] {
            let css = format!("color-mix(in {space}, {color} 50%, {color} 50%)");
            assert_eq!(parse_css_color(&css), parse_css_color(color), "{css}");
        }
    }
}

#[test]
fn color_mix_hsl_takes_the_circular_hue_path_not_a_muddy_linear_average() {
    // Mixing red (hue 0) and blue (hue 240) in hsl space by the shorter
    // (default) path goes through magenta (hue 300), not through the
    // desaturated middle a naive per-channel rgb average would give.
    assert_eq!(
        parse_css_color("color-mix(in hsl, red, blue)"),
        parse_css_color("hsl(300deg 100% 50%)")
    );
}

#[test]
fn color_mix_hue_interpolation_methods_take_opposite_arcs() {
    // hsl(60deg ..) mixed 50/50 with hsl(0deg ..): shorter/decreasing go
    // through 30deg, longer/increasing go the other way around through 210deg.
    for (method, expected_hue) in [
        ("shorter hue", 30),
        ("decreasing hue", 30),
        ("longer hue", 210),
        ("increasing hue", 210),
    ] {
        let css = format!("color-mix(in hsl {method}, hsl(60deg 50% 50%), hsl(0deg 50% 50%))");
        let expected = format!("hsl({expected_hue}deg 50% 50%)");
        assert_eq!(parse_css_color(&css), parse_css_color(&expected), "{css}");
    }
}

#[test]
fn color_mix_hwb_achromatic_side_is_white_plus_black_not_a_near_zero_channel() {
    // hwb(_, 0%, 0%) is fully chromatic (its "white" channel being 0 does not
    // make it achromatic); the achromatic test is white + black >= 100%.
    // Mixing a fully chromatic hwb hue against an achromatic (gray) one takes
    // the chromatic side's hue through unmixed.
    assert_close(
        parse_css_color("color-mix(in hwb, hwb(90deg 0% 0%), hwb(none 50% 50%))"),
        parse_css_color("hwb(90deg 25% 25%)"),
        "hwb achromatic side by white+black, not a near-zero channel",
    );
}

#[test]
fn calc_size_nests_inside_min_max_clamp_through_the_cascade() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "div { width: min(calc-size(auto, size + 20px), 300px) }",
        )],
    );
    // The declared value is retained verbatim except for the tokenizer's
    // canonical (no-space-after-comma) serialization at the top level.
    assert_eq!(
        styles.get(root).unwrap().get("width"),
        Some("min(calc-size(auto,size + 20px),300px)")
    );
}
