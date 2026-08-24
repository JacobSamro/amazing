//! Vetted text-provider adapter for untrusted web-font bytes.

#![forbid(unsafe_code)]

use amazing_text::{
    font_match_score, resolve_synthesis, BreakKind, BreakOpportunity, Direction,
    FallbackTextSystem, FontAttributes, FontKey, FontMetrics, FontSlant, GlyphCacheKey,
    GlyphContent, GlyphFlags, GlyphId, GlyphImage, GlyphRasterRequest, Paragraph,
    ShapedGlyph, ShapedParagraph, ShapedRun, ShapingBoundary, TextError, TextStyle, TextSystem,
    WritingMode,
};
use harfrust::{
    BufferFlags, Feature, FontRef as HarfFontRef, ShapeOptions, ShaperData, ShaperInstance, Tag,
    UnicodeBuffer, Variation,
};
use icu_segmenter::{GraphemeClusterSegmenter, LineSegmenter};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::error::Error;
use std::io::{Cursor, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::{zeno, FontRef as SwashFontRef};

const MAX_COMPRESSED_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SFNT_FONT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FONT_SET_BYTES: usize = 64 * 1024 * 1024;
const MAX_FONT_FACES: usize = 256;
const MAX_FONT_TABLES: usize = 256;
const MAX_FONT_GLYPHS: usize = 60_000;
const MAX_VARIATION_AXES: usize = 64;
const MAX_GLYPH_DIMENSION: u32 = 4096;
const MAX_GLYPH_BYTES: usize = 64 * 1024 * 1024;
const MAX_RASTER_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_RASTER_CACHE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFormat {
    OpenType,
    Woff,
    Woff2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebFont {
    pub family: String,
    pub weight: u16,
    pub stretch: u16,
    pub slant: FontSlant,
    pub format: FontFormat,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
struct FontRecord {
    key: FontKey,
    family: String,
    weight: u16,
    stretch: u16,
    slant: FontSlant,
    sfnt: Vec<u8>,
}

struct FontItemization {
    spans: Vec<amazing_text::TextSpan>,
    fonts: Vec<Option<usize>>,
    clusters: Vec<std::ops::Range<usize>>,
}

#[derive(Debug)]
pub struct TextProvider {
    generation: u64,
    next_font_key: u64,
    fonts: Vec<FontRecord>,
    font_set_bytes: usize,
    raster_cache: RefCell<HashMap<GlyphCacheKey, Option<GlyphImage>>>,
    raster_cache_bytes: Cell<usize>,
    fallback: FallbackTextSystem,
}

impl Default for TextProvider {
    fn default() -> Self {
        Self {
            generation: 0,
            next_font_key: 1,
            fonts: Vec::new(),
            font_set_bytes: 0,
            raster_cache: RefCell::new(HashMap::new()),
            raster_cache_bytes: Cell::new(0),
            fallback: FallbackTextSystem::new(),
        }
    }
}

impl TextProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_web_font(&mut self, font: WebFont) -> Result<FontKey, TextError> {
        let family = font.family.trim();
        if family.is_empty()
            || !(1..=1000).contains(&font.weight)
            || font.stretch == 0
            || font.stretch > 1000
        {
            return Err(TextError::InvalidStyle);
        }
        if font.bytes.is_empty() || font.bytes.len() > MAX_COMPRESSED_FONT_BYTES {
            return Err(TextError::ResourceLimit);
        }
        let decoded = catch_unwind(AssertUnwindSafe(|| decode_font(font.format, font.bytes)))
            .map_err(|_| TextError::MalformedFont)??;
        if decoded.is_empty() || decoded.len() > MAX_SFNT_FONT_BYTES {
            return Err(TextError::ResourceLimit);
        }
        catch_unwind(AssertUnwindSafe(|| validate_font_structure(&decoded)))
            .map_err(|_| TextError::MalformedFont)??;
        if self.fonts.len() >= MAX_FONT_FACES
            || self.font_set_bytes.saturating_add(decoded.len()) > MAX_FONT_SET_BYTES
        {
            return Err(TextError::ResourceLimit);
        }

        let key = FontKey(self.next_font_key);
        self.next_font_key = self
            .next_font_key
            .checked_add(1)
            .ok_or(TextError::ResourceLimit)?;
        self.font_set_bytes += decoded.len();
        self.fonts.push(FontRecord {
            key,
            family: family.to_ascii_lowercase(),
            weight: font.weight,
            stretch: font.stretch,
            slant: font.slant,
            sfnt: decoded,
        });
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(TextError::ResourceLimit)?;
        self.raster_cache.get_mut().clear();
        self.raster_cache_bytes.set(0);
        Ok(key)
    }

    fn matching_font(&self, style: &TextStyle, text: Option<&str>) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (family_order, family) in style.families.iter().enumerate() {
            let family = unquote_family(family);
            for (index, font) in self.fonts.iter().enumerate() {
                if !font.family.eq_ignore_ascii_case(family) {
                    continue;
                }
                if text.is_some_and(|value| !font_covers(font, value)) {
                    continue;
                }
                let score = family_order as u32 * 1_000_000
                    + font_match_score(
                        FontAttributes {
                            weight: style.weight,
                            stretch: style.stretch,
                            slant: style.slant,
                        },
                        FontAttributes {
                            weight: font.weight,
                            stretch: font.stretch,
                            slant: font.slant,
                        },
                    );
                if best.is_none_or(|(_, current)| score < current) {
                    best = Some((index, score));
                }
            }
            if best.is_some_and(|(_, score)| score / 1_000_000 == family_order as u32) {
                break;
            }
        }
        best.map(|(index, _)| index)
    }

    fn itemize_font_spans(&self, text: &str, spans: &[amazing_text::TextSpan]) -> FontItemization {
        let segmenter = GraphemeClusterSegmenter::new();
        let mut itemized = Vec::new();
        let mut selected = Vec::new();
        let mut clusters = Vec::new();
        let mut previous_font = None;
        let boundaries = segmenter.segment_str(text).collect::<Vec<_>>();
        for pair in boundaries.windows(2) {
            let cluster = pair[0]..pair[1];
            let owner = spans
                .iter()
                .find(|span| span.range.contains(&cluster.start))
                .unwrap_or(&spans[0]);
            let font = self.matching_font(&owner.style, Some(&text[cluster.clone()]));
            for span in spans
                .iter()
                .filter(|span| span.range.start < cluster.end && span.range.end > cluster.start)
            {
                let range = span.range.start.max(cluster.start)..span.range.end.min(cluster.end);
                let mut boundary = if range.start == span.range.start {
                    span.boundary_before
                } else {
                    ShapingBoundary::Continue
                };
                if !itemized.is_empty()
                    && boundary != ShapingBoundary::Hard
                    && previous_font != font
                {
                    boundary = ShapingBoundary::StyleChange;
                }
                itemized.push(amazing_text::TextSpan {
                    range,
                    source: span.source,
                    style: span.style.clone(),
                    boundary_before: boundary,
                });
                selected.push(font);
                clusters.push(cluster.clone());
                previous_font = font;
            }
        }
        FontItemization {
            spans: itemized,
            fonts: selected,
            clusters,
        }
    }
}

impl TextSystem for TextProvider {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn metrics(&self, style: &TextStyle) -> Result<amazing_text::FontMetrics, TextError> {
        let Some(index) = self.matching_font(style, None) else {
            return self.fallback.metrics(style);
        };
        font_metrics(&self.fonts[index], style.size)
    }

    fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
        let validated = self.fallback.shape(Paragraph {
            text: paragraph.text,
            spans: paragraph.spans,
        })?;
        if paragraph.text.is_empty() {
            return Ok(validated);
        }

        let itemization = self.itemize_font_spans(paragraph.text, paragraph.spans);
        let itemized = itemization.spans;
        self.fallback.shape(Paragraph {
            text: paragraph.text,
            spans: &itemized,
        })?;
        let selected = itemization.fonts;
        let clusters = itemization.clusters;
        let mut result = ShapedParagraph {
            runs: Vec::new(),
            breaks: line_breaks(paragraph.text),
        };
        let mut index = 0usize;
        while index < itemized.len() {
            let Some(font_index) = selected[index] else {
                let cluster = clusters[index].clone();
                let mut end = index + 1;
                while end < itemized.len()
                    && selected[end].is_none()
                    && clusters[end] == cluster
                {
                    end += 1;
                }
                result.runs.push(fallback_grapheme_run(
                    &self.fallback,
                    paragraph.text,
                    cluster,
                    &itemized[index],
                )?);
                index = end;
                continue;
            };
            let mut end = index + 1;
            while end < itemized.len()
                && selected[end] == Some(font_index)
                && itemized[end].boundary_before == ShapingBoundary::Continue
                && itemized[end].style == itemized[index].style
            {
                end += 1;
            }
            let target = itemized[index].range.start..itemized[end - 1].range.end;
            let hard_context = hard_context_range(&itemized, index, end, paragraph.text.len());
            let shaped = shape_range(
                &self.fonts[font_index],
                paragraph.text,
                &itemized[index..end],
                target,
                hard_context,
            )?;
            result.runs.extend(shaped);
            index = end;
        }
        Ok(result)
    }

    fn rasterize(&self, request: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError> {
        let key = request.key;
        if key.generation != self.generation {
            return Err(TextError::MissingFont);
        }
        if let Some(cached) = self.raster_cache.borrow().get(&key) {
            return Ok(cached.clone());
        }
        let image = if key.font == FontKey(0) {
            let fallback_key = GlyphCacheKey {
                generation: 0,
                ..key
            };
            self.fallback.rasterize(GlyphRasterRequest {
                key: fallback_key,
                variations: request.variations,
                synthesis: request.synthesis,
            })?
        } else {
            let font = self
                .fonts
                .iter()
                .find(|font| font.key == key.font)
                .ok_or(TextError::MissingFont)?;
            rasterize_font_glyph(font, &request)?
        };
        self.cache_raster(key, image.clone());
        Ok(image)
    }
}

impl TextProvider {
    fn cache_raster(&self, key: GlyphCacheKey, image: Option<GlyphImage>) {
        let entry_overhead = std::mem::size_of::<(GlyphCacheKey, Option<GlyphImage>)>();
        let charge =
            entry_overhead.saturating_add(image.as_ref().map_or(0, |image| image.data.capacity()));
        let cache = self.raster_cache.borrow();
        if cache.len() >= MAX_RASTER_CACHE_ENTRIES
            || charge > MAX_RASTER_CACHE_BYTES
            || self.raster_cache_bytes.get().saturating_add(charge) > MAX_RASTER_CACHE_BYTES
        {
            return;
        }
        drop(cache);
        self.raster_cache_bytes
            .set(self.raster_cache_bytes.get() + charge);
        self.raster_cache.borrow_mut().insert(key, image);
    }
}

fn decode_font(format: FontFormat, bytes: Vec<u8>) -> Result<Vec<u8>, TextError> {
    match format {
        FontFormat::OpenType => Ok(bytes),
        FontFormat::Woff => wuff::decompress_woff1_with_custom_z(&bytes, &mut |compressed,
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
            {
                return Err(Box::new(std::io::Error::other("invalid WOFF stream")));
            }
            Ok(output)
        })
        .map_err(|_| TextError::MalformedFont),
        FontFormat::Woff2 => wuff::decompress_woff2_with_custom_brotli(&bytes, &mut |compressed,
                                                                                     expected_size|
         -> Result<
            Vec<u8>,
            Box<dyn Error>,
        > {
            if expected_size > MAX_SFNT_FONT_BYTES {
                return Err(Box::new(std::io::Error::other("font output exceeds limit")));
            }
            let mut output = BoundedWriter::new(expected_size);
            brotli_decompressor::BrotliDecompress(&mut Cursor::new(compressed), &mut output)?;
            if output.bytes.len() != expected_size {
                return Err(Box::new(std::io::Error::other("invalid WOFF2 stream")));
            }
            Ok(output.bytes)
        })
        .map_err(|_| TextError::MalformedFont),
    }
}

fn validate_font_structure(bytes: &[u8]) -> Result<(), TextError> {
    if bytes.len() < 12 || &bytes[..4] == b"ttcf" {
        return Err(TextError::MalformedFont);
    }
    let table_count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    if table_count == 0 || table_count > MAX_FONT_TABLES {
        return Err(TextError::ResourceLimit);
    }
    HarfFontRef::new(bytes).map_err(|_| TextError::MalformedFont)?;
    let face = SwashFontRef::from_index(bytes, 0).ok_or(TextError::MalformedFont)?;
    if face.metrics(&[]).glyph_count as usize > MAX_FONT_GLYPHS
        || face.variations().count() > MAX_VARIATION_AXES
    {
        return Err(TextError::ResourceLimit);
    }
    Ok(())
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
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

fn unquote_family(family: &str) -> &str {
    let trimmed = family.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn font_covers(font: &FontRecord, text: &str) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        SwashFontRef::from_index(&font.sfnt, 0).is_some_and(|face| {
            let charmap = face.charmap();
            text.chars()
                .all(|ch| ch.is_whitespace() || is_default_ignorable(ch) || charmap.map(ch) != 0)
        })
    }))
    .unwrap_or(false)
}

fn is_default_ignorable(ch: char) -> bool {
    matches!(
        ch as u32,
        0x00AD
            | 0x034F
            | 0x061C
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

fn font_metrics(font: &FontRecord, size: f32) -> Result<FontMetrics, TextError> {
    if !size.is_finite() || size <= 0.0 || size > 4096.0 {
        return Err(TextError::InvalidStyle);
    }
    catch_unwind(AssertUnwindSafe(|| {
        let face = SwashFontRef::from_index(&font.sfnt, 0).ok_or(TextError::MalformedFont)?;
        let metrics = face.metrics(&[]).scale(size);
        Ok(FontMetrics {
            ascent: metrics.ascent.max(0.0),
            descent: metrics.descent.abs(),
            leading: metrics.leading.max(0.0),
            x_height: metrics.x_height.max(0.0),
            cap_height: metrics.cap_height.max(0.0),
        })
    }))
    .map_err(|_| TextError::MalformedFont)?
}

fn fallback_grapheme_run(
    fallback: &FallbackTextSystem,
    text: &str,
    cluster: std::ops::Range<usize>,
    span: &amazing_text::TextSpan,
) -> Result<ShapedRun, TextError> {
    let first = text[cluster.clone()]
        .chars()
        .next()
        .ok_or(TextError::InvalidParagraph)?;
    let first = first.to_string();
    let local_spans = [amazing_text::TextSpan {
        range: 0..first.len(),
        source: span.source,
        style: span.style.clone(),
        boundary_before: span.boundary_before,
    }];
    let mut shaped = fallback.shape(Paragraph {
        text: &first,
        spans: &local_spans,
    })?;
    let mut run = shaped.runs.pop().ok_or(TextError::InvalidParagraph)?;
    run.text_range = cluster.clone();
    for glyph in &mut run.glyphs {
        glyph.cluster = cluster.clone();
    }
    Ok(run)
}

fn line_breaks(text: &str) -> Vec<BreakOpportunity> {
    LineSegmenter::new_auto(Default::default())
        .segment_str(text)
        .filter(|offset| *offset != 0)
        .map(|offset| BreakOpportunity {
            offset,
            kind: if offset == text.len() {
                BreakKind::Mandatory
            } else {
                BreakKind::Allowed
            },
        })
        .collect()
}

fn hard_context_range(
    spans: &[amazing_text::TextSpan],
    start: usize,
    end: usize,
    text_len: usize,
) -> std::ops::Range<usize> {
    let mut context_start = 0;
    for span in spans[..=start].iter().rev() {
        if span.boundary_before == ShapingBoundary::Hard {
            context_start = span.range.start;
            break;
        }
    }
    let mut context_end = text_len;
    for span in &spans[end..] {
        if span.boundary_before == ShapingBoundary::Hard {
            context_end = span.range.start;
            break;
        }
    }
    context_start..context_end
}

fn shape_range(
    font: &FontRecord,
    text: &str,
    spans: &[amazing_text::TextSpan],
    target: std::ops::Range<usize>,
    context: std::ops::Range<usize>,
) -> Result<Vec<ShapedRun>, TextError> {
    let style = &spans[0].style;
    let metrics = font_metrics(font, style.size)?;
    catch_unwind(AssertUnwindSafe(|| {
        let face = HarfFontRef::new(&font.sfnt).map_err(|_| TextError::MalformedFont)?;
        let shaper_data = ShaperData::new(&face);
        let variations = style
            .variations
            .iter()
            .map(|variation| Variation {
                tag: Tag::new(&variation.tag),
                value: variation.value,
            })
            .collect::<Vec<_>>();
        let instance = ShaperInstance::from_variations(&face, &variations);
        let shaper = shaper_data.shaper(&face).instance(Some(&instance)).build();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&text[target.clone()]);
        if target.start > context.start {
            buffer.set_pre_context(&text[context.start..target.start]);
        }
        if target.end < context.end {
            buffer.set_post_context(&text[target.end..context.end]);
        }
        buffer.set_direction(match (style.writing_mode, style.direction) {
            (WritingMode::HorizontalTb, Direction::LeftToRight) => harfrust::Direction::LeftToRight,
            (WritingMode::HorizontalTb, Direction::RightToLeft) => harfrust::Direction::RightToLeft,
            (_, _) => harfrust::Direction::TopToBottom,
        });
        if let Some(language) = style
            .language
            .as_deref()
            .and_then(|value| value.parse().ok())
        {
            buffer.set_language(language);
        }
        buffer.guess_segment_properties();
        let mut flags = BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
        if target.start == context.start {
            flags |= BufferFlags::BEGINNING_OF_TEXT;
        }
        if target.end == context.end {
            flags |= BufferFlags::END_OF_TEXT;
        }
        buffer.set_flags(flags);
        let scale = (style.size * 64.0).round().clamp(1.0, i32::MAX as f32) as i32;
        let features = style
            .features
            .iter()
            .map(|feature| Feature::new(Tag::new(&feature.tag), feature.value, ..))
            .collect::<Vec<_>>();
        let output = shaper.shape(
            buffer,
            ShapeOptions::new().scale(Some(scale)).features(&features),
        );
        let cluster_ends = cluster_end_map(output.glyph_infos(), target.clone());
        let mut runs: Vec<ShapedRun> = Vec::new();
        for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
            let cluster_start = target.start + info.cluster as usize;
            let cluster_end = cluster_ends
                .iter()
                .find_map(|(start, end)| (*start == cluster_start).then_some(*end))
                .unwrap_or(target.end);
            let source_span = spans
                .iter()
                .find(|span| span.range.contains(&cluster_start))
                .unwrap_or(&spans[0]);
            let glyph = ShapedGlyph {
                id: GlyphId(info.glyph_id),
                cluster: cluster_start..cluster_end,
                advance_x: position.x_advance as f32 / 64.0,
                advance_y: position.y_advance as f32 / 64.0,
                offset_x: position.x_offset as f32 / 64.0,
                offset_y: position.y_offset as f32 / 64.0,
                flags: GlyphFlags {
                    unsafe_to_break: info.unsafe_to_break(),
                },
            };
            if let Some(run) = runs.last_mut()
                && run.source == source_span.source
            {
                run.glyphs.push(glyph);
                run.text_range.start = run.text_range.start.min(cluster_start);
                run.text_range.end = run.text_range.end.max(cluster_end);
                continue;
            }
            runs.push(ShapedRun {
                text_range: cluster_start..cluster_end,
                source: source_span.source,
                font: font.key,
                font_size: style.size,
                direction: style.direction,
                writing_mode: style.writing_mode,
                metrics,
                variations: style.variations.clone(),
                synthesis: resolve_synthesis(
                    FontAttributes {
                        weight: style.weight,
                        stretch: style.stretch,
                        slant: style.slant,
                    },
                    FontAttributes {
                        weight: font.weight,
                        stretch: font.stretch,
                        slant: font.slant,
                    },
                    style.synthesis,
                ),
                glyphs: vec![glyph],
            });
        }
        Ok(runs)
    }))
    .map_err(|_| TextError::MalformedFont)?
}

fn cluster_end_map(
    infos: &[harfrust::GlyphInfo],
    target: std::ops::Range<usize>,
) -> Vec<(usize, usize)> {
    let mut starts = infos
        .iter()
        .map(|info| target.start + info.cluster as usize)
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| (*start, starts.get(index + 1).copied().unwrap_or(target.end)))
        .collect()
}

fn rasterize_font_glyph(
    font: &FontRecord,
    request: &GlyphRasterRequest<'_>,
) -> Result<Option<GlyphImage>, TextError> {
    let key = request.key;
    let glyph_id = u16::try_from(key.glyph.0).map_err(|_| TextError::MalformedFont)?;
    catch_unwind(AssertUnwindSafe(|| {
        let face = SwashFontRef::from_index(&font.sfnt, 0).ok_or(TextError::MalformedFont)?;
        let mut context = ScaleContext::new();
        let variation_tags = request
            .variations
            .iter()
            .map(|variation| {
                (
                    String::from_utf8_lossy(&variation.tag).into_owned(),
                    variation.value,
                )
            })
            .collect::<Vec<_>>();
        let mut scaler = context
            .builder(face)
            .size(key.size())
            .hint(true)
            .variations(
                variation_tags
                    .iter()
                    .map(|(tag, value)| (tag.as_str(), *value)),
            )
            .build();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ]);
        render.format(zeno::Format::Alpha).offset(zeno::Vector::new(
            key.subpixel_x_64 as f32 / 64.0,
            key.subpixel_y_64 as f32 / 64.0,
        ));
        if request.synthesis.weight {
            render.embolden(key.size() / 32.0);
        }
        if request.synthesis.style {
            render.transform(Some(zeno::Transform::skew(
                zeno::Angle::from_degrees(-12.0),
                zeno::Angle::ZERO,
            )));
        }
        let image = render.render(&mut scaler, glyph_id);
        let Some(image) = image else {
            return Ok(None);
        };
        if image.placement.width > MAX_GLYPH_DIMENSION
            || image.placement.height > MAX_GLYPH_DIMENSION
            || image.data.len() > MAX_GLYPH_BYTES
        {
            return Err(TextError::ResourceLimit);
        }
        let (content, data) = match image.content {
            Content::Mask => (GlyphContent::Mask, image.data),
            Content::Color => (GlyphContent::Color, image.data),
            Content::SubpixelMask => {
                let data = image
                    .data
                    .chunks_exact(3)
                    .map(|channels| channels[0].max(channels[1]).max(channels[2]))
                    .collect();
                (GlyphContent::Mask, data)
            }
        };
        let channels = if content == GlyphContent::Color { 4 } else { 1 };
        let expected = image
            .placement
            .width
            .checked_mul(image.placement.height)
            .and_then(|pixels| pixels.checked_mul(channels))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(TextError::ResourceLimit)?;
        if data.len() != expected {
            return Err(TextError::MalformedFont);
        }
        Ok(Some(GlyphImage {
            content,
            data,
            width: image.placement.width,
            height: image.placement.height,
            bearing_x: image.placement.left,
            bearing_y: image.placement.top,
        }))
    }))
    .map_err(|_| TextError::MalformedFont)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_text::{ShapingBoundary, TextSpan};

    fn be16(value: u16) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }

    fn be32(value: u32) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }

    fn assemble_sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut tables = tables.to_vec();
        tables.sort_by_key(|(tag, _)| **tag);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&be32(0x0001_0000));
        bytes.extend_from_slice(&be16(tables.len() as u16));
        let entry_selector = (usize::BITS - tables.len().leading_zeros() - 1) as u16;
        let search_range = (1u16 << entry_selector) * 16;
        bytes.extend_from_slice(&be16(search_range));
        bytes.extend_from_slice(&be16(entry_selector));
        bytes.extend_from_slice(&be16(tables.len() as u16 * 16 - search_range));
        let mut offset = (12 + tables.len() * 16) as u32;
        for (tag, table) in &tables {
            bytes.extend_from_slice(*tag);
            bytes.extend_from_slice(&be32(0));
            bytes.extend_from_slice(&be32(offset));
            bytes.extend_from_slice(&be32(table.len() as u32));
            offset += (table.len() as u32 + 3) & !3;
        }
        for (_, table) in &tables {
            bytes.extend_from_slice(table);
            while bytes.len() % 4 != 0 {
                bytes.push(0);
            }
        }
        bytes
    }

    fn synthetic_font(mapped: char) -> Vec<u8> {
        let mut glyf = vec![0u8; 10];
        glyf[0..2].copy_from_slice(&be16(1));
        glyf[6..8].copy_from_slice(&be16(500));
        glyf[8..10].copy_from_slice(&be16(700));
        glyf.extend_from_slice(&be16(3));
        glyf.extend_from_slice(&be16(0));
        glyf.extend_from_slice(&[0x31, 0x21, 0x11, 0x21]);
        glyf.extend_from_slice(&be16(500));
        glyf.extend_from_slice(&be16(0xFE0C));
        glyf.extend_from_slice(&be16(700));

        let mut cmap_format12 = Vec::new();
        cmap_format12.extend_from_slice(&be16(12));
        cmap_format12.extend_from_slice(&be16(0));
        cmap_format12.extend_from_slice(&be32(28));
        cmap_format12.extend_from_slice(&be32(0));
        cmap_format12.extend_from_slice(&be32(1));
        cmap_format12.extend_from_slice(&be32(mapped as u32));
        cmap_format12.extend_from_slice(&be32(mapped as u32));
        cmap_format12.extend_from_slice(&be32(1));
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&be16(0));
        cmap.extend_from_slice(&be16(1));
        cmap.extend_from_slice(&be16(3));
        cmap.extend_from_slice(&be16(10));
        cmap.extend_from_slice(&be32(12));
        cmap.extend_from_slice(&cmap_format12);

        let mut head = vec![0u8; 54];
        head[12..16].copy_from_slice(&be32(0x5F0F_3CF5));
        head[18..20].copy_from_slice(&be16(1000));
        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&be16(800));
        hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
        hhea[34..36].copy_from_slice(&be16(2));
        let mut maxp = vec![0u8; 32];
        maxp[0..4].copy_from_slice(&be32(0x0001_0000));
        maxp[4..6].copy_from_slice(&be16(2));
        let loca = [be16(0), be16(0), be16(glyf.len() as u16 / 2)].concat();
        let hmtx = [be16(600), be16(0), be16(500), be16(0)].concat();
        assemble_sfnt(&[
            (b"head", head),
            (b"hhea", hhea),
            (b"maxp", maxp),
            (b"hmtx", hmtx),
            (b"cmap", cmap),
            (b"loca", loca),
            (b"glyf", glyf),
        ])
    }

    #[test]
    fn malformed_woff2_fails_closed_without_changing_generation() {
        let mut provider = TextProvider::new();
        let result = provider.register_web_font(WebFont {
            family: "Broken".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::Woff2,
            bytes: vec![0; 64],
        });

        assert_eq!(result, Err(TextError::MalformedFont));
        assert_eq!(provider.generation(), 0);
    }

    #[test]
    fn oversized_web_font_is_rejected_before_decode() {
        let mut provider = TextProvider::new();
        let result = provider.register_web_font(WebFont {
            family: "TooLarge".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::Woff2,
            bytes: vec![0; MAX_COMPRESSED_FONT_BYTES + 1],
        });

        assert_eq!(result, Err(TextError::ResourceLimit));
        assert_eq!(provider.generation(), 0);
    }

    #[test]
    fn aggregate_font_set_bytes_are_bounded() {
        let bytes = synthetic_font('A');
        let mut provider = TextProvider::new();
        provider.font_set_bytes = MAX_FONT_SET_BYTES - bytes.len() + 1;

        assert_eq!(
            provider.register_web_font(WebFont {
                family: "AggregateLimit".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes,
            }),
            Err(TextError::ResourceLimit)
        );
    }

    #[test]
    fn font_table_and_glyph_counts_have_explicit_limits() {
        let mut too_many_tables = synthetic_font('A');
        too_many_tables[4..6].copy_from_slice(&((MAX_FONT_TABLES + 1) as u16).to_be_bytes());
        assert_eq!(
            TextProvider::new().register_web_font(WebFont {
                family: "Tables".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: too_many_tables,
            }),
            Err(TextError::ResourceLimit)
        );

        let mut too_many_glyphs = synthetic_font('A');
        let table_count = u16::from_be_bytes([too_many_glyphs[4], too_many_glyphs[5]]) as usize;
        let maxp_offset = (0..table_count)
            .map(|index| 12 + index * 16)
            .find(|offset| &too_many_glyphs[*offset..*offset + 4] == b"maxp")
            .map(|offset| {
                u32::from_be_bytes(
                    too_many_glyphs[offset + 8..offset + 12]
                        .try_into()
                        .unwrap(),
                ) as usize
            })
            .unwrap();
        too_many_glyphs[maxp_offset + 4..maxp_offset + 6]
            .copy_from_slice(&((MAX_FONT_GLYPHS + 1) as u16).to_be_bytes());
        assert_eq!(
            TextProvider::new().register_web_font(WebFont {
                family: "Glyphs".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: too_many_glyphs,
            }),
            Err(TextError::ResourceLimit)
        );
    }

    #[test]
    fn blank_raster_cache_entries_have_a_hard_count_limit() {
        let provider = TextProvider::new();
        for glyph in 0..(MAX_RASTER_CACHE_ENTRIES as u32 + 100) {
            provider.cache_raster(
                GlyphCacheKey {
                    generation: 0,
                    font: FontKey(0),
                    glyph: GlyphId(glyph),
                    size_64: 16 * 64,
                    subpixel_x_64: 0,
                    subpixel_y_64: 0,
                    variation_hash: 0,
                    synthesis: 0,
                },
                None,
            );
        }

        assert_eq!(
            provider.raster_cache.borrow().len(),
            MAX_RASTER_CACHE_ENTRIES
        );
        assert!(provider.raster_cache_bytes.get() > 0);
    }

    #[test]
    fn registered_face_shapes_clusters_and_rasterizes_the_same_glyph_id() {
        let mut provider = TextProvider::new();
        let key = provider
            .register_web_font(WebFont {
                family: "Synthetic".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font('A'),
            })
            .unwrap();
        let style = TextStyle {
            families: vec!["Synthetic".into()],
            size: 10.0,
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..1,
            source: 7,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text: "A",
                spans: &spans,
            })
            .unwrap();
        assert_eq!(shaped.runs.len(), 1);
        assert_eq!(shaped.runs[0].font, key);
        assert_eq!(shaped.runs[0].glyphs[0].id, amazing_text::GlyphId(1));
        assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..1);
        assert!((shaped.runs[0].glyphs[0].advance_x - 5.0).abs() < 0.02);

        let raster_key = GlyphCacheKey::new(
            provider.generation(),
            key,
            shaped.runs[0].glyphs[0].id,
            10.0,
            0,
            0,
        )
        .unwrap();
        let image = provider
            .rasterize(GlyphRasterRequest {
                key: raster_key,
                variations: &[],
                synthesis: shaped.runs[0].synthesis,
            })
            .unwrap()
            .unwrap();
        assert_eq!((image.width, image.height), (5, 7));
        assert_eq!(image.content, amazing_text::GlyphContent::Mask);
    }

    #[test]
    fn fallback_selects_a_covering_face_per_grapheme_cluster() {
        let mut provider = TextProvider::new();
        let primary = provider
            .register_web_font(WebFont {
                family: "Primary".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font('A'),
            })
            .unwrap();
        let secondary = provider
            .register_web_font(WebFont {
                family: "Secondary".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font('B'),
            })
            .unwrap();
        let style = TextStyle {
            families: vec!["Primary".into(), "Secondary".into()],
            size: 10.0,
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..2,
            source: 9,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text: "AB",
                spans: &spans,
            })
            .unwrap();
        assert_eq!(shaped.runs.len(), 2);
        assert_eq!(shaped.runs[0].font, primary);
        assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..1);
        assert_eq!(shaped.runs[1].font, secondary);
        assert_eq!(shaped.runs[1].glyphs[0].cluster, 1..2);
        assert!(shaped.runs.iter().all(|run| run.glyphs[0].id == GlyphId(1)));
    }

    #[test]
    fn default_ignorable_controls_stay_with_the_surrounding_face() {
        let mut provider = TextProvider::new();
        let primary = provider
            .register_web_font(WebFont {
                family: "Primary".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font('A'),
            })
            .unwrap();
        let style = TextStyle {
            families: vec!["Primary".into()],
            size: 10.0,
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..4,
            source: 11,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text: "A\u{200d}",
                spans: &spans,
            })
            .unwrap();
        assert!(!shaped.runs.is_empty());
        assert!(shaped.runs.iter().all(|run| run.font == primary));
    }

    #[test]
    fn fallback_grapheme_is_not_split_at_a_dom_span_boundary() {
        let mut provider = TextProvider::new();
        provider
            .register_web_font(WebFont {
                family: "Primary".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font('A'),
            })
            .unwrap();
        provider
            .register_web_font(WebFont {
                family: "Secondary".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font('\u{0301}'),
            })
            .unwrap();
        let style = TextStyle {
            families: vec!["Primary".into(), "Secondary".into()],
            size: 10.0,
            ..TextStyle::default()
        };
        let spans = [
            TextSpan {
                range: 0..1,
                source: 20,
                style: style.clone(),
                boundary_before: ShapingBoundary::Continue,
            },
            TextSpan {
                range: 1..3,
                source: 21,
                style,
                boundary_before: ShapingBoundary::Continue,
            },
        ];

        let shaped = provider
            .shape(Paragraph {
                text: "A\u{0301}",
                spans: &spans,
            })
            .unwrap();
        assert_eq!(shaped.runs.len(), 1);
        assert_eq!(shaped.runs[0].glyphs.len(), 1);
        assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..3);
        assert!(shaped.runs.iter().all(|run| run.font == FontKey(0)));
    }
}
