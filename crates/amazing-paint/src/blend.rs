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

#[cfg(test)]
mod tests {
    use super::{blend_source_over, quantize_opacity};

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
}
