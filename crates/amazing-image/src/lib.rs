//! Bounded, first-frame web-image decoding. Codec types never cross this adapter.
//! Animation, ICC color management and EXIF orientation are not implemented.
#![forbid(unsafe_code)]

pub mod svg;

use image::ImageDecoder;
use std::io::Cursor;

/// Independent input, geometry, output and decoder working-allocation budgets.
#[derive(Clone, Copy, Debug)]
pub struct DecodeLimits {
    pub max_input_bytes: usize,
    pub max_dimension: u32,
    pub max_rgba_bytes: u64,
    pub max_decoder_bytes: u64,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_dimension: 8192,
            max_rgba_bytes: 64 * 1024 * 1024,
            max_decoder_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InputLimit,
    GeometryLimit,
    AllocationLimit,
    InvalidImage,
    UnsupportedSvg,
    DecoderPanic,
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DecodeError {}

/// Decode a single still image or first animation frame into owned straight RGBA8.
/// Limits are per image; callers must additionally bound retained page resources.
/// Decoder allocation limits are best-effort inside codecs; input, dimensions,
/// and RGBA output are independently checked. Panics become closed errors.
pub fn decode(bytes: &[u8], limits: DecodeLimits) -> Result<amazing_media::Image, DecodeError> {
    if bytes.len() > limits.max_input_bytes {
        return Err(DecodeError::InputLimit);
    }
    std::panic::catch_unwind(|| decode_inner(bytes, limits))
        .unwrap_or(Err(DecodeError::DecoderPanic))
}
fn decode_inner(bytes: &[u8], limits: DecodeLimits) -> Result<amazing_media::Image, DecodeError> {
    if std::str::from_utf8(bytes).is_ok_and(|s| s.trim_start_matches('\u{feff}').trim_start().starts_with('<')) {
        return svg::decode(bytes, limits);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| DecodeError::InvalidImage)?;
    // Feature unification with GPUI can enable extra image codecs. Keep the
    // web adapter's accepted formats independent of the enclosing graph.
    use image::ImageFormat::{Bmp, Gif, Ico, Jpeg, Png, Pnm, WebP};
    if !matches!(reader.format(), Some(Bmp | Gif | Ico | Jpeg | Png | Pnm | WebP)) {
        return Err(DecodeError::InvalidImage);
    }
    let mut codec_limits = image::Limits::default();
    codec_limits.max_image_width = Some(limits.max_dimension);
    codec_limits.max_image_height = Some(limits.max_dimension);
    codec_limits.max_alloc = Some(limits.max_decoder_bytes);
    reader.limits(codec_limits);
    let decoder = reader
        .into_decoder()
        .map_err(|_| DecodeError::InvalidImage)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || width > limits.max_dimension || height > limits.max_dimension {
        return Err(DecodeError::GeometryLimit);
    }
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4))
        .ok_or(DecodeError::AllocationLimit)?;
    if rgba_bytes > limits.max_rgba_bytes
        || rgba_bytes > usize::MAX as u64
        || decoder
            .total_bytes()
            .checked_add(rgba_bytes)
            .is_none_or(|n| n > limits.max_decoder_bytes)
    {
        return Err(DecodeError::AllocationLimit);
    }
    let rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| DecodeError::InvalidImage)?
        .into_rgba8();
    if rgba.dimensions() != (width, height) || rgba.as_raw().len() as u64 != rgba_bytes {
        return Err(DecodeError::InvalidImage);
    }
    Ok(amazing_media::Image {
        width,
        height,
        pixels: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            3,
            2,
            image::Rgb([20, 80, 140]),
        ));
        let image = if format == image::ImageFormat::Ico {
            image::DynamicImage::ImageRgba8(image.into_rgba8())
        } else {
            image
        };
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }
    #[test]
    fn web_formats_return_owned_rgba_with_intrinsic_dimensions() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
            image::ImageFormat::Ico,
            image::ImageFormat::Bmp,
            image::ImageFormat::Pnm,
        ] {
            let decoded = decode(&encoded(format), DecodeLimits::default())
                .unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!(
                (decoded.width, decoded.height, decoded.pixels.len()),
                (3, 2, 24),
                "{format:?}"
            );
            assert!(decoded.pixels.chunks_exact(4).all(|p| p[3] == 255));
            for (got, expected) in decoded.pixels[..3].iter().zip([20u8, 80, 140]) {
                assert!(
                    got.abs_diff(expected) <= 3,
                    "{format:?}: {got} vs {expected}"
                );
            }
        }
    }
    #[test]
    fn transparent_png_and_first_gif_frame_preserve_alpha_and_frame_policy() {
        let mut png = Cursor::new(Vec::new());
        let pixels = image::RgbaImage::from_pixel(2, 1, image::Rgba([10, 30, 50, 64]));
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            decode(png.get_ref(), Default::default()).unwrap().pixels,
            vec![10, 30, 50, 64, 10, 30, 50, 64]
        );
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        2,
                        1,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        assert_eq!(
            decode(&gif, Default::default()).unwrap().pixels,
            vec![255, 0, 0, 255, 255, 0, 0, 255]
        );
    }
    #[test]
    fn budgets_reject_before_returning_pixels() {
        let bytes = encoded(image::ImageFormat::Png);
        assert_eq!(
            decode(
                &bytes,
                DecodeLimits {
                    max_input_bytes: 1,
                    ..Default::default()
                }
            ),
            Err(DecodeError::InputLimit)
        );
        assert!(
            decode(
                &bytes,
                DecodeLimits {
                    max_dimension: 2,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            decode(
                &bytes,
                DecodeLimits {
                    max_rgba_bytes: 23,
                    ..Default::default()
                }
            ),
            Err(DecodeError::AllocationLimit)
        );
        assert!(
            decode(
                &bytes,
                DecodeLimits {
                    max_decoder_bytes: 1,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn malformed_and_truncated_inputs_fail_closed() {
        assert!(decode(b"not an image", DecodeLimits::default()).is_err());
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
            image::ImageFormat::Ico,
        ] {
            let bytes = encoded(format);
            for length in [0, 1, 4, 8, bytes.len() / 2] {
                assert!(
                    decode(&bytes[..length], DecodeLimits::default()).is_err(),
                    "{format:?} {length}"
                );
            }
        }
    }
}
