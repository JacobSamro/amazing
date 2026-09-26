//! Shared integer source-over compositing for paint and retained tiles.

/// Quantize a CSS/group opacity to RGBA8 precision. NaN maps to zero;
/// finite values and infinities clamp to `[0, 1]`, then round to nearest.
pub fn quantize_opacity(opacity: f32) -> u8 {
    if opacity.is_nan() {
        return 0;
    }
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Blend straight-alpha RGBA8 `src` over `dst` with an 8-bit group opacity.
///
/// All products use integer premultiplied numerators. The effective source
/// alpha is rounded to nearest after opacity multiplication; output alpha and
/// each unpremultiplied color channel are then rounded to nearest with ties
/// upward. A completely transparent result is canonical transparent black.
pub fn blend_source_over(src: [u8; 4], dst: [u8; 4], opacity: u8) -> [u8; 4] {
    let source_alpha = div_255(u32::from(src[3]) * u32::from(opacity));
    let destination_alpha = u32::from(dst[3]);
    let inverse = 255 - source_alpha;
    let alpha_numerator = source_alpha * 255 + destination_alpha * inverse;
    if alpha_numerator == 0 {
        return [0; 4];
    }
    let mut result = [0; 4];
    for channel in 0..3 {
        let numerator = u32::from(src[channel]) * source_alpha * 255
            + u32::from(dst[channel]) * destination_alpha * inverse;
        result[channel] = ((numerator + alpha_numerator / 2) / alpha_numerator) as u8;
    }
    result[3] = div_255(alpha_numerator) as u8;
    result
}

fn div_255(value: u32) -> u32 {
    (value + 127) / 255
}

/// Composite an effect-group layer `src` onto `dst` with a CSS blend mode and
/// 8-bit opacity, per CSS Compositing and Blending: blend each color channel
/// against the backdrop, then source-over the blended result. Integer
/// arithmetic (the non-separable modes round once at the end) keeps the
/// reference deterministic across platforms and targets.
pub fn blend_group(
    mode: amazing_css::effects::BlendMode,
    src: [u8; 4],
    dst: [u8; 4],
    opacity: u8,
) -> [u8; 4] {
    use amazing_css::effects::BlendMode;
    if mode == BlendMode::Normal {
        return blend_source_over(src, dst, opacity);
    }
    let source_alpha = div_255(u32::from(src[3]) * u32::from(opacity));
    if source_alpha == 0 {
        return dst;
    }
    let destination_alpha = u32::from(dst[3]);
    let inverse = 255 - source_alpha;
    let alpha_numerator = source_alpha * 255 + destination_alpha * inverse;
    if alpha_numerator == 0 {
        return [0; 4];
    }
    let mut result = [0; 4];
    for channel in 0..3 {
        let cb = u32::from(dst[channel]);
        let cs = u32::from(src[channel]);
        let blended = u32::from(blend_channel(mode, dst, src, channel));
        // C' = (1 - ab) * Cs + ab * B(Cb, Cs), in 8-bit units.
        let premixed = div_255((255 - destination_alpha) * cs + destination_alpha * blended);
        let numerator = premixed * source_alpha * 255 + cb * destination_alpha * inverse;
        result[channel] = ((numerator + alpha_numerator / 2) / alpha_numerator) as u8;
    }
    result[3] = div_255(alpha_numerator) as u8;
    result
}

fn blend_channel(
    mode: amazing_css::effects::BlendMode,
    dst: [u8; 4],
    src: [u8; 4],
    channel: usize,
) -> u8 {
    use amazing_css::effects::BlendMode;
    let cb = u32::from(dst[channel]);
    let cs = u32::from(src[channel]);
    match mode {
        BlendMode::Normal => cs as u8,
        BlendMode::Multiply => mul_255(cb, cs),
        BlendMode::Screen => screen_255(cb, cs),
        BlendMode::Overlay => hard_light(cs, cb),
        BlendMode::Darken => cb.min(cs) as u8,
        BlendMode::Lighten => cb.max(cs) as u8,
        BlendMode::ColorDodge => {
            if cb == 0 {
                0
            } else if cs >= 255 {
                255
            } else {
                ((cb * 255) / (255 - cs)).min(255) as u8
            }
        }
        BlendMode::ColorBurn => {
            if cb >= 255 {
                255
            } else if cs == 0 {
                0
            } else {
                (255 - ((255 - cb) * 255 / cs).min(255)) as u8
            }
        }
        BlendMode::HardLight => hard_light(cb, cs),
        BlendMode::SoftLight => soft_light(cb, cs),
        BlendMode::Difference => cb.abs_diff(cs) as u8,
        BlendMode::Exclusion => (cb + cs - div_255(2 * cb * cs)) as u8,
        BlendMode::PlusLighter => (cb + cs).min(255) as u8,
        BlendMode::Hue
        | BlendMode::Saturation
        | BlendMode::Color
        | BlendMode::Luminosity => non_separable(mode, dst, src, channel),
    }
}

fn mul_255(a: u32, b: u32) -> u8 {
    div_255(a * b) as u8
}
fn screen_255(a: u32, b: u32) -> u8 {
    (a + b - div_255(a * b)) as u8
}
fn hard_light(cb: u32, cs: u32) -> u8 {
    if cs <= 127 {
        mul_255(cb, 2 * cs)
    } else {
        screen_255(cb, 2 * cs - 255)
    }
}
fn soft_light(cb: u32, cs: u32) -> u8 {
    let back = cb as f32 / 255.0;
    let source = cs as f32 / 255.0;
    let result = if source <= 0.5 {
        back - (1.0 - 2.0 * source) * back * (1.0 - back)
    } else {
        let d = if back <= 0.25 {
            ((16.0 * back - 12.0) * back + 4.0) * back
        } else {
            back.sqrt()
        };
        back + (2.0 * source - 1.0) * (d - back)
    };
    (result * 255.0).round().clamp(0.0, 255.0) as u8
}

fn non_separable(
    mode: amazing_css::effects::BlendMode,
    dst: [u8; 4],
    src: [u8; 4],
    channel: usize,
) -> u8 {
    use amazing_css::effects::BlendMode;
    let back = [
        dst[0] as f32 / 255.0,
        dst[1] as f32 / 255.0,
        dst[2] as f32 / 255.0,
    ];
    let source = [
        src[0] as f32 / 255.0,
        src[1] as f32 / 255.0,
        src[2] as f32 / 255.0,
    ];
    let blended = match mode {
        BlendMode::Hue => set_lum(set_sat(source, sat(back)), lum(back)),
        BlendMode::Saturation => set_lum(set_sat(back, sat(source)), lum(back)),
        BlendMode::Color => set_lum(source, lum(back)),
        BlendMode::Luminosity => set_lum(back, lum(source)),
        _ => back,
    };
    (blended[channel] * 255.0).round().clamp(0.0, 255.0) as u8
}

fn lum(color: [f32; 3]) -> f32 {
    0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2]
}
fn sat(color: [f32; 3]) -> f32 {
    color[0].max(color[1]).max(color[2]) - color[0].min(color[1]).min(color[2])
}
fn clip_color(mut color: [f32; 3]) -> [f32; 3] {
    let l = lum(color);
    let min = color[0].min(color[1]).min(color[2]);
    let max = color[0].max(color[1]).max(color[2]);
    if min < 0.0 && (l - min) > 0.0 {
        for value in &mut color {
            *value = l + (*value - l) * l / (l - min);
        }
    }
    if max > 1.0 && (max - l) > 0.0 {
        for value in &mut color {
            *value = l + (*value - l) * (1.0 - l) / (max - l);
        }
    }
    color
}
fn set_lum(mut color: [f32; 3], l: f32) -> [f32; 3] {
    let delta = l - lum(color);
    for value in &mut color {
        *value += delta;
    }
    clip_color(color)
}
fn set_sat(mut color: [f32; 3], s: f32) -> [f32; 3] {
    let min = color[0].min(color[1]).min(color[2]);
    let max = color[0].max(color[1]).max(color[2]);
    if max > min {
        let factor = s / (max - min);
        for value in &mut color {
            *value = (*value - min) * factor;
        }
    } else {
        color = [0.0; 3];
    }
    color
}

#[cfg(test)]
mod tests {
    use super::{blend_group, blend_source_over, quantize_opacity};

    #[test]
    fn opacity_quantization_handles_edges_and_nan() {
        assert_eq!(quantize_opacity(f32::NAN), 0);
        assert_eq!(quantize_opacity(f32::NEG_INFINITY), 0);
        assert_eq!(quantize_opacity(f32::INFINITY), 255);
        assert_eq!(quantize_opacity(0.5), 128);
    }

    #[test]
    fn source_over_uses_one_integer_rounding_rule() {
        assert_eq!(blend_source_over([0, 0, 0, 128], [255; 4], 255), [127, 127, 127, 255]);
        assert_eq!(blend_source_over([255, 0, 0, 128], [0, 0, 255, 128], 255), [170, 0, 85, 192]);
        assert_eq!(blend_source_over([95, 30, 10, 255], [0, 0, 0, 0], 128), [95, 30, 10, 128]);
        assert_eq!(blend_source_over([20, 30, 40, 0], [9, 8, 7, 255], 255), [9, 8, 7, 255]);
        assert_eq!(blend_source_over([255, 0, 0, 255], [0, 0, 255, 255], 0), [0, 0, 255, 255]);
        assert_eq!(blend_source_over([10, 20, 30, 0], [30, 20, 10, 0], 255), [0; 4]);
    }

    #[test]
    fn separable_blend_modes_follow_the_compositing_spec() {
        use amazing_css::effects::BlendMode::*;
        let white = [255, 255, 255, 255];
        let black = [0, 0, 0, 255];
        let gray = [128, 128, 128, 255];
        assert_eq!(blend_group(Multiply, gray, white, 255), gray);
        assert_eq!(blend_group(Screen, gray, black, 255), gray);
        assert_eq!(blend_group(Darken, gray, white, 255), gray);
        assert_eq!(blend_group(Lighten, gray, black, 255), gray);
        assert_eq!(blend_group(Difference, white, black, 255), white);
        assert_eq!(blend_group(Exclusion, white, black, 255), white);
        assert_eq!(blend_group(PlusLighter, gray, gray, 255), white);
    }

    #[test]
    fn blend_group_composites_with_alpha_and_opacity() {
        use amazing_css::effects::BlendMode::*;
        // A transparent source leaves the backdrop untouched.
        assert_eq!(blend_group(Multiply, [0, 0, 0, 0], [10, 20, 30, 255], 255), [10, 20, 30, 255]);
        // Half-opacity multiply of gray over opaque white: 0.5*128 + 0.5*255.
        let pixel = blend_group(Multiply, gray(), [255, 255, 255, 255], 128);
        assert!(pixel[0].abs_diff(191) <= 1, "{pixel:?}");
    }

    #[test]
    fn non_separable_color_keeps_backdrop_luminance() {
        use amazing_css::effects::BlendMode::*;
        let pixel = blend_group(Color, [255, 0, 0, 255], [128, 128, 128, 255], 255);
        assert!(pixel[0] > 200, "hue comes from the source: {pixel:?}");
        assert!(pixel[1].abs_diff(pixel[2]) <= 2, "green/blue stay equal: {pixel:?}");
        assert!(pixel[1] < 120, "luminance comes from the backdrop: {pixel:?}");
    }

    fn gray() -> [u8; 4] {
        [128, 128, 128, 255]
    }
}
