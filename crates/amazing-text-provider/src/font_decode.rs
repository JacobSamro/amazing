//! Untrusted web-font byte decoding (WOFF/WOFF2 -> sfnt) and structural
//! validation: sniffing the container format, decompressing it, then
//! probing and bounds-checking the resulting sfnt's table directory before
//! any of it is trusted elsewhere in this crate. Split out of `lib.rs`
//! (TEXT-27).

use super::{
    FontFormat, MAX_COMPRESSED_FONT_BYTES, MAX_FONT_GLYPHS, MAX_FONT_TABLES, MAX_SFNT_FONT_BYTES,
    MAX_TTC_FONTS, MAX_VARIATION_AXES,
};
use amazing_text::TextError;
use harfrust::FontRef as HarfFontRef;
use swash::FontRef as SwashFontRef;
use std::error::Error;
use std::io::{Cursor, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Decodes and validates one untrusted web font using the same per-font limits
/// and sfnt checks as `TextProvider::register_web_font`.
pub fn decode_web_font_to_sfnt(format: FontFormat, bytes: &[u8]) -> Result<Vec<u8>, TextError> {
    if bytes.is_empty() || bytes.len() > MAX_COMPRESSED_FONT_BYTES {
        return Err(TextError::ResourceLimit);
    }
    let decoded = catch_unwind(AssertUnwindSafe(|| decode_font(format, bytes)))
        .map_err(|_| TextError::MalformedFont)??;
    if decoded.is_empty() || decoded.len() > MAX_SFNT_FONT_BYTES {
        return Err(TextError::ResourceLimit);
    }
    catch_unwind(AssertUnwindSafe(|| validate_font_structure(&decoded)))
        .map_err(|_| TextError::MalformedFont)??;
    Ok(decoded)
}

pub(super) fn decode_font(format: FontFormat, bytes: &[u8]) -> Result<Vec<u8>, TextError> {
    match format {
        FontFormat::OpenType | FontFormat::Collection => Ok(bytes.to_vec()),
        FontFormat::Woff => wuff::decompress_woff1_with_custom_z(bytes, &mut |compressed,
                                                                               expected_size|
         -> Result<
            Vec<u8>,
            Box<dyn Error>,
        > {
            if expected_size > MAX_SFNT_FONT_BYTES {
                return Err(Box::new(std::io::Error::other("font output exceeds limit")));
            }
            let mut output = vec![0; expected_size];
            let mut decompressor = flate2::Decompress::new(true);
            let status = decompressor.decompress(
                compressed,
                &mut output,
                flate2::FlushDecompress::Finish,
            )?;
            if status != flate2::Status::StreamEnd
                || decompressor.total_out() as usize != expected_size
                || decompressor.total_in() as usize != compressed.len()
            {
                return Err(Box::new(std::io::Error::other("invalid WOFF stream")));
            }
            Ok(output)
        })
        .map_err(|_| TextError::MalformedFont),
        FontFormat::Woff2 => wuff::decompress_woff2_with_custom_brotli(bytes, &mut |compressed,
                                                                                     expected_size|
         -> Result<
            Vec<u8>,
            Box<dyn Error>,
        > {
            if expected_size > MAX_SFNT_FONT_BYTES {
                return Err(Box::new(std::io::Error::other("font output exceeds limit")));
            }
            let mut output = BoundedWriter::new(expected_size);
            let mut input = Cursor::new(compressed);
            brotli_decompressor::BrotliDecompress(&mut input, &mut output)?;
            if output.bytes.len() != expected_size || input.position() as usize != compressed.len()
            {
                return Err(Box::new(std::io::Error::other("invalid WOFF2 stream")));
            }
            Ok(output.bytes)
        })
        .map_err(|_| TextError::MalformedFont),
    }
}

pub(super) fn sniff_font_format(bytes: &[u8]) -> Option<FontFormat> {
    match bytes.get(..4)? {
        b"wOFF" => Some(FontFormat::Woff),
        b"wOF2" => Some(FontFormat::Woff2),
        b"OTTO" | b"true" | [0, 1, 0, 0] => Some(FontFormat::OpenType),
        b"ttcf" => Some(FontFormat::Collection),
        _ => None,
    }
}

pub(super) fn parse_font_format_hint(hint: &str) -> Option<FontFormat> {
    match hint.trim().to_ascii_lowercase().as_str() {
        "woff" | "woff-variations" => Some(FontFormat::Woff),
        "woff2" | "woff2-variations" => Some(FontFormat::Woff2),
        "opentype" | "truetype" | "opentype-variations" | "truetype-variations" => {
            Some(FontFormat::OpenType)
        }
        _ => None,
    }
}

pub(super) fn supported_font_technology(hint: &str) -> bool {
    matches!(
        hint.trim().to_ascii_lowercase().as_str(),
        "features-opentype" | "variations"
    )
}

pub(super) fn validate_font_structure(bytes: &[u8]) -> Result<(), TextError> {
    let header_offset = match sniff_font_format(bytes) {
        Some(FontFormat::OpenType) => 0,
        // Validate only the collection's first font (index 0, the one we
        // register and rasterize): find its own sfnt header inside the file
        // via the TTC header's offset table, then run the same directory
        // checks against that offset. Table data can be (and for a well
        // formed collection, is) shared with later fonts in the file; we
        // never open those, so we never need their directories to validate.
        Some(FontFormat::Collection) => ttc_font_directory_offset(bytes, 0)?,
        _ => return Err(TextError::MalformedFont),
    };
    if bytes.len() < header_offset.checked_add(12).ok_or(TextError::MalformedFont)? {
        return Err(TextError::MalformedFont);
    }
    let table_count =
        u16::from_be_bytes([bytes[header_offset + 4], bytes[header_offset + 5]]) as usize;
    if table_count == 0 || table_count > MAX_FONT_TABLES {
        return Err(TextError::ResourceLimit);
    }
    let directory_start = header_offset + 12;
    let directory_end = directory_start
        .checked_add(
            table_count
                .checked_mul(16)
                .ok_or(TextError::ResourceLimit)?,
        )
        .ok_or(TextError::ResourceLimit)?;
    if directory_end > bytes.len() {
        return Err(TextError::MalformedFont);
    }
    let mut previous_tag = None;
    let mut ranges = Vec::with_capacity(table_count);
    let mut head = None;
    for index in 0..table_count {
        let record = directory_start + index * 16;
        let tag = &bytes[record..record + 4];
        if previous_tag.is_some_and(|previous: &[u8]| previous >= tag) {
            return Err(TextError::MalformedFont);
        }
        previous_tag = Some(tag);
        let expected_checksum =
            u32::from_be_bytes(bytes[record + 4..record + 8].try_into().unwrap());
        let offset =
            u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
        let length =
            u32::from_be_bytes(bytes[record + 12..record + 16].try_into().unwrap()) as usize;
        let end = offset.checked_add(length).ok_or(TextError::MalformedFont)?;
        let padded_end = end.checked_add(3).ok_or(TextError::MalformedFont)? & !3;
        if offset < directory_end || offset % 4 != 0 || padded_end > bytes.len() {
            return Err(TextError::MalformedFont);
        }
        let zero_adjustment = if tag == b"head" {
            if length < 54 || bytes.get(offset + 12..offset + 16) != Some(&b"\x5F\x0F\x3C\xF5"[..])
            {
                return Err(TextError::MalformedFont);
            }
            head = Some(offset);
            Some(8)
        } else {
            None
        };
        if sfnt_checksum(&bytes[offset..end], zero_adjustment) != expected_checksum {
            return Err(TextError::MalformedFont);
        }
        if bytes[end..padded_end].iter().any(|byte| *byte != 0) {
            return Err(TextError::MalformedFont);
        }
        ranges.push((offset, padded_end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) || head.is_none() {
        return Err(TextError::MalformedFont);
    }
    // Whole-file checksum adjustments are stale in many deployed fonts,
    // including the pinned CSSTest faces. Each table checksum still guards
    // its own contents, and directory bounds and overlaps are checked above.
    // `from_index(_, 0)` accepts both a lone sfnt and a collection's first
    // font, so this one call covers `FontFormat::OpenType` and `::Collection`
    // alike (`HarfFontRef::new` rejects `ttcf` outright).
    HarfFontRef::from_index(bytes, 0).map_err(|_| TextError::MalformedFont)?;
    let face = SwashFontRef::from_index(bytes, 0).ok_or(TextError::MalformedFont)?;
    if face.metrics(&[]).glyph_count as usize > MAX_FONT_GLYPHS
        || face.variations().count() > MAX_VARIATION_AXES
    {
        return Err(TextError::ResourceLimit);
    }
    Ok(())
}

/// Locates font `index`'s own sfnt header inside a `ttcf` buffer via the TTC
/// header's offset table. Bounds `numFonts` so a hostile header cannot force
/// an unbounded read.
pub(super) fn ttc_font_directory_offset(bytes: &[u8], index: u32) -> Result<usize, TextError> {
    if bytes.len() < 16 || &bytes[0..4] != b"ttcf" {
        return Err(TextError::MalformedFont);
    }
    let num_fonts = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if num_fonts == 0 || num_fonts > MAX_TTC_FONTS {
        return Err(TextError::ResourceLimit);
    }
    let offsets_start = 12usize;
    let offsets_end = offsets_start
        .checked_add(num_fonts.checked_mul(4).ok_or(TextError::ResourceLimit)?)
        .ok_or(TextError::ResourceLimit)?;
    if offsets_end > bytes.len() {
        return Err(TextError::MalformedFont);
    }
    let entry = offsets_start
        + (index as usize)
            .checked_mul(4)
            .ok_or(TextError::MalformedFont)?;
    if entry + 4 > offsets_end {
        return Err(TextError::MalformedFont);
    }
    let offset = u32::from_be_bytes(bytes[entry..entry + 4].try_into().unwrap()) as usize;
    if offset < offsets_end || offset % 4 != 0 {
        return Err(TextError::MalformedFont);
    }
    Ok(offset)
}

pub(super) fn sfnt_checksum(bytes: &[u8], zero_adjustment: Option<usize>) -> u32 {
    let mut sum = 0u32;
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        if zero_adjustment == Some(index * 4) {
            word = [0; 4];
        }
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

pub(super) struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedWriter {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit),
            limit,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("font output exceeds limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
