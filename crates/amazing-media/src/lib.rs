//! Phase 9 media decoding behind one format-independent interface.
//!
//! The first slice accepts uncompressed BMP and binary PPM images and
//! normalizes them into top-left-origin RGBA8 pixels. Container parsing,
//! row orientation, padding, and channel order stay private to this module.

#![forbid(unsafe_code)]

use std::fmt;

/// A decoded, tightly packed RGBA8 image with a top-left origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Failures observable at the media-decoding seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    UnsupportedFormat,
    InvalidData(&'static str),
    DimensionOverflow,
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::UnsupportedFormat => f.write_str("unsupported image format"),
            MediaError::InvalidData(reason) => write!(f, "invalid image data: {reason}"),
            MediaError::DimensionOverflow => f.write_str("image dimensions exceed addressable memory"),
        }
    }
}

impl std::error::Error for MediaError {}

/// Detects and decodes one supported image into RGBA8 pixels.
pub fn decode_image(bytes: &[u8]) -> Result<Image, MediaError> {
    if bytes.starts_with(b"P6") {
        return decode_ppm(bytes);
    }
    if bytes.starts_with(b"BM") {
        return decode_bmp(bytes);
    }
    Err(MediaError::UnsupportedFormat)
}

fn decode_bmp(bytes: &[u8]) -> Result<Image, MediaError> {
    if bytes.len() < 54 {
        return Err(MediaError::InvalidData("truncated BMP header"));
    }
    let pixel_offset = read_u32(bytes, 10)? as usize;
    let dib_size = read_u32(bytes, 14)? as usize;
    if dib_size < 40 || 14usize.checked_add(dib_size).is_none_or(|end| end > bytes.len()) {
        return Err(MediaError::InvalidData("unsupported or truncated BMP DIB header"));
    }
    let signed_width = read_i32(bytes, 18)?;
    let signed_height = read_i32(bytes, 22)?;
    if signed_width <= 0 || signed_height == 0 || signed_height == i32::MIN {
        return Err(MediaError::InvalidData("BMP dimensions must be representable and non-zero"));
    }
    if read_u16(bytes, 26)? != 1 {
        return Err(MediaError::InvalidData("BMP must contain one color plane"));
    }
    if read_u16(bytes, 28)? != 24 {
        return Err(MediaError::InvalidData("only 24-bit BMP pixels are supported"));
    }
    if read_u32(bytes, 30)? != 0 {
        return Err(MediaError::InvalidData("compressed BMP pixels are unsupported"));
    }

    let width = signed_width as u32;
    let height = signed_height.unsigned_abs();
    let row_data = (width as usize)
        .checked_mul(3)
        .ok_or(MediaError::DimensionOverflow)?;
    let row_stride = row_data
        .checked_add(3)
        .ok_or(MediaError::DimensionOverflow)?
        / 4
        * 4;
    let raster_len = row_stride
        .checked_mul(height as usize)
        .ok_or(MediaError::DimensionOverflow)?;
    let raster_end = pixel_offset
        .checked_add(raster_len)
        .ok_or(MediaError::DimensionOverflow)?;
    if pixel_offset < 14 + dib_size || raster_end > bytes.len() {
        return Err(MediaError::InvalidData("truncated BMP raster"));
    }
    let rgba_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(MediaError::DimensionOverflow)?;
    let mut pixels = Vec::with_capacity(rgba_len);
    let top_down = signed_height < 0;
    for y in 0..height as usize {
        let source_y = if top_down { y } else { height as usize - 1 - y };
        let row_start = pixel_offset + source_y * row_stride;
        for bgr in bytes[row_start..row_start + row_data].chunks_exact(3) {
            pixels.extend_from_slice(&[bgr[2], bgr[1], bgr[0], 255]);
        }
    }
    Ok(Image { width, height, pixels })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, MediaError> {
    let raw: [u8; 2] = bytes
        .get(offset..offset + 2)
        .ok_or(MediaError::InvalidData("truncated BMP field"))?
        .try_into()
        .expect("slice length checked");
    Ok(u16::from_le_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, MediaError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(MediaError::InvalidData("truncated BMP field"))?
        .try_into()
        .expect("slice length checked");
    Ok(u32::from_le_bytes(raw))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, MediaError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(MediaError::InvalidData("truncated BMP field"))?
        .try_into()
        .expect("slice length checked");
    Ok(i32::from_le_bytes(raw))
}

fn decode_ppm(bytes: &[u8]) -> Result<Image, MediaError> {
    let mut cursor = 2;
    let width = parse_u32(next_ppm_token(bytes, &mut cursor)?, "PPM width")?;
    let height = parse_u32(next_ppm_token(bytes, &mut cursor)?, "PPM height")?;
    let max_value = parse_u32(next_ppm_token(bytes, &mut cursor)?, "PPM max value")?;
    if width == 0 || height == 0 {
        return Err(MediaError::InvalidData("PPM dimensions must be positive"));
    }
    if max_value == 0 || max_value > 255 {
        return Err(MediaError::InvalidData("PPM max value must be between 1 and 255"));
    }

    match bytes.get(cursor) {
        Some(b'\r') if bytes.get(cursor + 1) == Some(&b'\n') => cursor += 2,
        Some(byte) if byte.is_ascii_whitespace() => cursor += 1,
        _ => return Err(MediaError::InvalidData("PPM header lacks a raster separator")),
    }

    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(MediaError::DimensionOverflow)?;
    let rgb_len = pixel_count
        .checked_mul(3)
        .ok_or(MediaError::DimensionOverflow)?;
    let rgba_len = pixel_count
        .checked_mul(4)
        .ok_or(MediaError::DimensionOverflow)?;
    let raster = bytes
        .get(cursor..cursor.checked_add(rgb_len).ok_or(MediaError::DimensionOverflow)?)
        .ok_or(MediaError::InvalidData("truncated PPM raster"))?;

    let mut pixels = Vec::with_capacity(rgba_len);
    for rgb in raster.chunks_exact(3) {
        if rgb.iter().any(|sample| *sample as u32 > max_value) {
            return Err(MediaError::InvalidData("PPM sample exceeds declared max value"));
        }
        let normalize = |sample: u8| {
            ((sample as u32 * 255 + max_value / 2) / max_value) as u8
        };
        pixels.extend_from_slice(&[
            normalize(rgb[0]),
            normalize(rgb[1]),
            normalize(rgb[2]),
            255,
        ]);
    }
    Ok(Image { width, height, pixels })
}

fn next_ppm_token<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], MediaError> {
    loop {
        while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
        if bytes.get(*cursor) != Some(&b'#') {
            break;
        }
        while bytes.get(*cursor).is_some_and(|byte| *byte != b'\n' && *byte != b'\r') {
            *cursor += 1;
        }
    }
    let start = *cursor;
    while bytes
        .get(*cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'#')
    {
        *cursor += 1;
    }
    if *cursor == start {
        return Err(MediaError::InvalidData("missing PPM header token"));
    }
    Ok(&bytes[start..*cursor])
}

fn parse_u32(token: &[u8], field: &'static str) -> Result<u32, MediaError> {
    let text = std::str::from_utf8(token).map_err(|_| MediaError::InvalidData(field))?;
    text.parse().map_err(|_| MediaError::InvalidData(field))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp_24(width: u32, height: u32, rows_top_down: &[u8]) -> Vec<u8> {
        let row_bytes = (width as usize * 3).div_ceil(4) * 4;
        let image_size = row_bytes * height as usize;
        let mut out = Vec::with_capacity(54 + image_size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((54 + image_size) as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(image_size as u32).to_le_bytes());
        out.extend_from_slice(&[0; 16]);
        for row in (0..height as usize).rev() {
            for rgb in rows_top_down[row * width as usize * 3..][..width as usize * 3]
                .chunks_exact(3)
            {
                out.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
            }
            out.resize(out.len() + row_bytes - width as usize * 3, 0);
        }
        out
    }

    #[test]
    fn decodes_binary_ppm_through_the_public_interface() {
        let bytes = b"P6\n2 1\n255\n\xff\x00\x00\x00\x80\xff";
        let image = decode_image(bytes).expect("valid P6 image");

        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(
            image.pixels,
            vec![255, 0, 0, 255, 0, 128, 255, 255]
        );
    }

    #[test]
    fn normalizes_eight_bit_ppm_samples_from_declared_max_value() {
        let image = decode_image(b"P6\n1 1\n15\n\x0f\x08\x00")
            .expect("valid low-range P6 image");

        assert_eq!(image.pixels, vec![255, 136, 0, 255]);
    }

    #[test]
    fn decodes_bottom_up_bmp_and_removes_row_padding() {
        let bytes = bmp_24(
            2,
            2,
            &[
                255, 0, 0, 0, 255, 0,
                0, 0, 255, 255, 255, 255,
            ],
        );
        let image = decode_image(&bytes).expect("valid 24-bit BMP");

        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(
            image.pixels,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255,
                0, 0, 255, 255, 255, 255, 255, 255,
            ]
        );
    }
}
