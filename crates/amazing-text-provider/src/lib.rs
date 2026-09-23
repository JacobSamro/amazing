//! Vetted text-provider adapter for untrusted web-font bytes.

#![forbid(unsafe_code)]

use amazing_text::{
    BreakKind, BreakOpportunity, Direction, FallbackTextSystem, FontAttributes, FontKey,
    FontMetrics, FontSlant, GlyphCacheKey, GlyphContent, GlyphFlags, GlyphId, GlyphImage,
    GlyphOrientation, GlyphRasterRequest, LineRequest, Paragraph, ShapedGlyph, ShapedLine,
    ShapedParagraph, ShapedRun, ShapingBoundary, TextError, TextOrientation, TextStyle, TextSystem,
    WritingMode, font_match_rank, resolve_synthesis,
};
use harfrust::{
    BufferFlags, Feature, FontRef as HarfFontRef, ShapeOptions, ShaperData, ShaperInstance, Tag,
    UnicodeBuffer, Variation,
};
use icu_segmenter::options::{LineBreakOptions, LineBreakStrictness, LineBreakWordOption};
use icu_segmenter::{GraphemeClusterSegmenter, LineSegmenter};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::io::{Cursor, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::{FontRef as SwashFontRef, zeno};
use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};
use unicode_vo::Orientation as VerticalOrientation;

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
const MAX_TEXT_CLUSTERS: usize = 262_144;
const MAX_ITEMIZED_SPANS: usize = 524_288;
const MAX_BIDI_SCOPE_SEGMENTS: usize = 524_288;
const MAX_BIDI_SCOPE_BREAK_OPERATIONS: usize = 16 * 1024 * 1024;
const MAX_FONT_MATCH_OPERATIONS: usize = 16 * 1024 * 1024;

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
        let private_use = text.is_some_and(|value| value.chars().any(is_private_use));
        for family in &style.families {
            if private_use && is_generic_family(family) {
                continue;
            }
            let family = unquote_family(family);
            let mut best = None;
            for (index, font) in self.fonts.iter().enumerate() {
                if !font.family.eq_ignore_ascii_case(family) {
                    continue;
                }
                let rank = font_match_rank(
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
                if best.is_none_or(|(_, current)| rank < current) {
                    best = Some((index, rank));
                }
            }
            if let Some((index, _)) = best {
                if text.is_none_or(|value| font_covers(&self.fonts[index], value)) {
                    return Some(index);
                }
            }
        }
        if private_use {
            return None;
        }
        // The driver registers its platform faces under these generic aliases.
        // Once the author list is exhausted, use an installed face that covers
        // the cluster before the deterministic missing-glyph renderer.
        ["serif", "sans-serif", "system-ui", "monospace"]
            .iter()
            .find_map(|family| {
                self.fonts.iter().position(|font| {
                    font.family == *family
                        && text.is_none_or(|value| font_covers(font, value))
                })
            })
    }

    fn itemize_font_spans(
        &self,
        text: &str,
        spans: &[amazing_text::TextSpan],
    ) -> Result<FontItemization, TextError> {
        let segmenter = GraphemeClusterSegmenter::new();
        let mut itemized = Vec::new();
        let mut selected = Vec::new();
        let mut clusters = Vec::new();
        let mut previous_font = None;
        let boundaries = segmenter
            .segment_str(text)
            .take(MAX_TEXT_CLUSTERS + 2)
            .collect::<Vec<_>>();
        if boundaries.len() > MAX_TEXT_CLUSTERS + 1 {
            return Err(TextError::ResourceLimit);
        }
        let cluster_count = boundaries.len().saturating_sub(1);
        let max_families = spans
            .iter()
            .map(|span| span.style.families.len())
            .max()
            .unwrap_or(0)
            .max(1);
        let match_operations = cluster_count
            .checked_mul(self.fonts.len().max(1))
            .and_then(|count| count.checked_mul(max_families))
            .ok_or(TextError::ResourceLimit)?;
        if match_operations > MAX_FONT_MATCH_OPERATIONS {
            return Err(TextError::ResourceLimit);
        }
        let mut span_index = 0usize;
        for pair in boundaries.windows(2) {
            let cluster = pair[0]..pair[1];
            while span_index < spans.len() && spans[span_index].range.end <= cluster.start {
                span_index += 1;
            }
            let owner = spans.get(span_index).ok_or(TextError::InvalidParagraph)?;
            let font = self.matching_font(&owner.style, Some(&text[cluster.clone()]));
            let mut overlap_index = span_index;
            while let Some(span) = spans.get(overlap_index)
                && span.range.start < cluster.end
            {
                if itemized.len() >= MAX_ITEMIZED_SPANS {
                    return Err(TextError::ResourceLimit);
                }
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
                overlap_index += 1;
            }
        }
        Ok(FontItemization {
            spans: itemized,
            fonts: selected,
            clusters,
        })
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
        amazing_text::validate_paragraph(&Paragraph {
            text: paragraph.text,
            spans: paragraph.spans,
            sources: paragraph.sources,
            bidi_scopes: paragraph.bidi_scopes,
            base_direction: paragraph.base_direction,
        })?;
        if paragraph.text.is_empty() {
            return Ok(ShapedParagraph::default());
        }
        let runs = self.shape_runs(
                paragraph.text,
                paragraph.spans,
                0..paragraph.text.len(),
                paragraph.base_direction,
                paragraph.bidi_scopes,
            )?;
        let mut breaks = line_breaks(paragraph.text, paragraph.spans);
        breaks.sort_by_key(|opportunity| opportunity.offset);
        let unsafe_breaks = unsafe_break_offsets(&runs, &breaks);
        let result = ShapedParagraph {
            runs,
            breaks,
            unsafe_breaks,
            sources: paragraph.sources.to_vec(),
        };
        Ok(result)
    }

    fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError> {
        let text = request.paragraph.text;
        if request.range.start >= request.range.end
            || request.range.end > text.len()
            || !text.is_char_boundary(request.range.start)
            || !text.is_char_boundary(request.range.end)
            {
            return Err(TextError::InvalidParagraph);
            }
        amazing_text::validate_paragraph(&Paragraph {
            text,
            spans: request.paragraph.spans,
            sources: request.paragraph.sources,
            bidi_scopes: request.paragraph.bidi_scopes,
            base_direction: request.paragraph.base_direction,
        })?;
        let logical_runs = self.shape_runs(
            text,
            request.paragraph.spans,
            request.range.clone(),
            request.paragraph.base_direction,
            request.paragraph.bidi_scopes,
            )?;
        let visual_clusters = visual_clusters(
            text,
            request.paragraph.spans,
            request.range.clone(),
            request.paragraph.base_direction,
            request.paragraph.bidi_scopes,
        )?;
        let runs = runs_in_visual_cluster_order(&logical_runs, &visual_clusters);
        Ok(ShapedLine {
            text_range: request.range,
            runs,
            visual_clusters,
        })
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

fn runs_in_visual_cluster_order(
    runs: &[ShapedRun],
    visual_clusters: &[std::ops::Range<usize>],
) -> Vec<ShapedRun> {
    let mut by_cluster: HashMap<usize, Vec<ShapedRun>> = HashMap::new();
    for run in runs {
        let mut glyphs_by_cluster: HashMap<usize, Vec<ShapedGlyph>> = HashMap::new();
        for glyph in &run.glyphs {
            glyphs_by_cluster
                .entry(glyph.cluster.start)
                .or_default()
                .push(glyph.clone());
        }
        for (cluster_start, glyphs) in glyphs_by_cluster {
            let text_range = glyphs
                .iter()
                .map(|glyph| glyph.cluster.clone())
                .reduce(|left, right| left.start.min(right.start)..left.end.max(right.end))
                .unwrap_or(cluster_start..cluster_start);
            let visual_run = clone_run_with_glyphs(run, text_range, glyphs);
            by_cluster
                .entry(cluster_start)
                .or_default()
                .push(visual_run);
        }
    }
    let mut visual_runs = Vec::new();
    for cluster in visual_clusters {
        if let Some(mut runs) = by_cluster.remove(&cluster.start) {
            visual_runs.append(&mut runs);
        }
    }
    visual_runs
}

fn clone_run_with_glyphs(
    run: &ShapedRun,
    text_range: std::ops::Range<usize>,
    glyphs: Vec<ShapedGlyph>,
) -> ShapedRun {
    ShapedRun {
        text_range,
        source: run.source,
        font: run.font,
        font_size: run.font_size,
        direction: run.direction,
        writing_mode: run.writing_mode,
        metrics: run.metrics,
        variations: run.variations.clone(),
        synthesis: run.synthesis,
        glyphs,
    }
}

impl TextProvider {
    fn shape_runs(
        &self,
        text: &str,
        spans: &[amazing_text::TextSpan],
        selected_range: std::ops::Range<usize>,
        base_direction: Direction,
        bidi_scopes: &[amazing_text::BidiScope],
    ) -> Result<Vec<ShapedRun>, TextError> {
        let itemization = self.itemize_font_spans(text, spans)?;
        let itemized = itemization.spans;
        amazing_text::validate_paragraph(&Paragraph {
            text,
            spans: &itemized,
            sources: &[],
            bidi_scopes: &[],
            base_direction,
        })?;
        let selected = itemization.fonts;
        let clusters = itemization.clusters;
        let directions = bidi_directions(text, spans, &itemized, base_direction, bidi_scopes)?;
        let mut runs = Vec::new();
        let mut index = 0usize;
        while index < itemized.len() {
            let Some(font_index) = selected[index] else {
                let cluster = clusters[index].clone();
                let mut end = index + 1;
                while end < itemized.len() && selected[end].is_none() && clusters[end] == cluster {
                    end += 1;
                }
                if cluster.start < selected_range.end && selected_range.start < cluster.end {
                    let mut run =
                        fallback_grapheme_run(&self.fallback, text, cluster, &itemized[index])?;
                    run.direction = directions[index];
                    runs.push(run);
                }
                index = end;
                continue;
            };
            let mut end = index + 1;
            let orientation = cluster_orientation(
                &itemized[index].style,
                text,
                itemized[index].range.clone(),
            );
            while end < itemized.len()
                && selected[end] == Some(font_index)
                && itemized[end].boundary_before == ShapingBoundary::Continue
                && itemized[end].style == itemized[index].style
                && directions[end] == directions[index]
                && cluster_orientation(
                    &itemized[end].style,
                    text,
                    itemized[end].range.clone(),
                ) == orientation
            {
                end += 1;
            }
            let group_range = itemized[index].range.start..itemized[end - 1].range.end;
            let target = group_range.start.max(selected_range.start)
                ..group_range.end.min(selected_range.end);
            if target.start < target.end {
                let hard_context = hard_context_range(&itemized, index, end, text.len());
                runs.extend(shape_range(
                    &self.fonts[font_index],
                    text,
                    &itemized[index..end],
                    target,
                    hard_context,
                    directions[index],
                    orientation,
                )?);
            }
            index = end;
        }
        Ok(runs)
    }

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

fn is_generic_family(family: &str) -> bool {
    let family = family.trim();
    [
        "serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui",
        "ui-serif", "ui-sans-serif", "ui-monospace", "ui-rounded", "emoji",
        "math", "fangsong",
    ]
    .iter()
    .any(|generic| family.eq_ignore_ascii_case(generic))
}

fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
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
            vertical_ascent: metrics.vertical_ascent.max(0.0),
            vertical_descent: metrics.vertical_descent.abs(),
            vertical_leading: metrics.vertical_leading.max(0.0),
            has_canonical_vertical_metrics: metrics.has_vertical_metrics,
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
        sources: &[],
        bidi_scopes: &[],
        base_direction: span.style.direction,
    })?;
    let mut run = shaped.runs.pop().ok_or(TextError::InvalidParagraph)?;
    run.text_range = cluster.clone();
    for glyph in &mut run.glyphs {
        glyph.cluster = cluster.clone();
    }
    Ok(run)
}

fn line_breaks(text: &str, spans: &[amazing_text::TextSpan]) -> Vec<BreakOpportunity> {
    let mut policies = Vec::new();
    for span in spans {
        let policy = (span.style.line_break, span.style.word_break);
        if !policies.contains(&policy) {
            policies.push(policy);
        }
    }
    let mut opportunities = Vec::new();
    for (line_break, word_break) in policies {
        let mut options = LineBreakOptions::default();
        options.strictness = match line_break {
            amazing_text::LineBreak::Auto => None,
            amazing_text::LineBreak::Loose => Some(LineBreakStrictness::Loose),
            amazing_text::LineBreak::Normal => Some(LineBreakStrictness::Normal),
            amazing_text::LineBreak::Strict => Some(LineBreakStrictness::Strict),
            amazing_text::LineBreak::Anywhere => Some(LineBreakStrictness::Anywhere),
        };
        options.word_option = Some(match word_break {
            amazing_text::WordBreak::Normal => LineBreakWordOption::Normal,
            amazing_text::WordBreak::BreakAll => LineBreakWordOption::BreakAll,
            amazing_text::WordBreak::KeepAll => LineBreakWordOption::KeepAll,
        });
        opportunities.extend(
            LineSegmenter::new_auto(options)
                .segment_str(text)
                .filter(|offset| *offset != 0)
                .filter(|offset| {
                    *offset == text.len()
                        || mandatory_break_before(text, *offset)
                        || style_at_break(text, spans, *offset).is_some_and(|style| {
                            style.line_break == line_break && style.word_break == word_break
                        })
                })
                .filter_map(|offset| {
                    break_kind(text, spans, offset)
                        .map(|kind| BreakOpportunity { offset, kind })
                }),
        );
    }
    opportunities.extend(
        GraphemeClusterSegmenter::new()
            .segment_str(text)
            .filter(|offset| *offset != 0 && *offset != text.len())
            .filter(|offset| {
                style_at_break(text, spans, *offset).is_some_and(|style| {
                    style.overflow_wrap != amazing_text::OverflowWrap::Normal
                        && white_space_wraps(style.white_space)
                })
            })
            .map(|offset| BreakOpportunity {
                offset,
                kind: BreakKind::Emergency,
            }),
    );
    opportunities.sort_by_key(|opportunity| (opportunity.offset, break_priority(opportunity.kind)));
    opportunities.dedup_by_key(|opportunity| opportunity.offset);
    opportunities
}

fn style_at_break<'a>(
    text: &str,
    spans: &'a [amazing_text::TextSpan],
    offset: usize,
) -> Option<&'a TextStyle> {
    let character_start = text[..offset].char_indices().next_back()?.0;
    let index = spans.partition_point(|span| span.range.end <= character_start);
    spans
        .get(index)
        .filter(|span| span.range.contains(&character_start))
        .map(|span| &span.style)
}

fn break_priority(kind: BreakKind) -> u8 {
    match kind {
        BreakKind::Mandatory => 0,
        BreakKind::Hyphen => 1,
        BreakKind::Allowed => 2,
        BreakKind::Emergency => 3,
    }
}

fn unsafe_break_offsets(runs: &[ShapedRun], breaks: &[BreakOpportunity]) -> Vec<usize> {
    let offsets = breaks
        .iter()
        .map(|opportunity| opportunity.offset)
        .collect::<Vec<_>>();
    let mut unsafe_offsets = BTreeSet::new();
    for glyph in runs.iter().flat_map(|run| &run.glyphs) {
        let first_internal = offsets.partition_point(|offset| *offset <= glyph.cluster.start);
        let after_internal = offsets.partition_point(|offset| *offset < glyph.cluster.end);
        unsafe_offsets.extend(offsets[first_internal..after_internal].iter().copied());
        if (glyph.flags.unsafe_to_break || glyph.flags.unsafe_to_concat)
            && offsets.binary_search(&glyph.cluster.start).is_ok()
        {
            unsafe_offsets.insert(glyph.cluster.start);
        }
    }
    offsets
        .into_iter()
        .filter(|offset| unsafe_offsets.contains(offset))
        .collect()
}

fn break_kind(text: &str, spans: &[amazing_text::TextSpan], offset: usize) -> Option<BreakKind> {
    if offset == text.len() || mandatory_break_before(text, offset) {
        return Some(BreakKind::Mandatory);
    }
    let (_, previous) = text[..offset].char_indices().next_back()?;
    let style = style_at_break(text, spans, offset);
    if style.is_some_and(|style| !white_space_wraps(style.white_space)) {
        return None;
    }
    if previous != '\u{00ad}' {
        return Some(BreakKind::Allowed);
    }
    match style
        .map(|style| style.hyphens)
        .unwrap_or(amazing_text::Hyphens::Manual)
    {
        amazing_text::Hyphens::None => None,
        amazing_text::Hyphens::Manual | amazing_text::Hyphens::Auto => Some(BreakKind::Hyphen),
    }
}

fn white_space_wraps(white_space: amazing_text::WhiteSpace) -> bool {
    !matches!(
        white_space,
        amazing_text::WhiteSpace::NoWrap | amazing_text::WhiteSpace::Pre
    )
}

fn mandatory_break_before(text: &str, offset: usize) -> bool {
    text[..offset].chars().next_back().is_some_and(|ch| {
        matches!(
            ch,
            '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
        )
    })
}

fn visual_clusters(
    text: &str,
    spans: &[amazing_text::TextSpan],
    line: std::ops::Range<usize>,
    base_direction: Direction,
    bidi_scopes: &[amazing_text::BidiScope],
) -> Result<Vec<std::ops::Range<usize>>, TextError> {
    let default_level = Some(match base_direction {
        Direction::LeftToRight => Level::ltr(),
        Direction::RightToLeft => Level::rtl(),
    });
    let analysis = bidi_analysis(text, spans, bidi_scopes)?;
    let bidi = BidiInfo::new(&analysis.text, default_level);
    let analysis_line =
        analysis.original_to_analysis[line.start]..analysis.original_to_analysis[line.end];
    let paragraph = bidi
        .paragraphs
        .iter()
        .find(|paragraph| {
            paragraph.range.start <= analysis_line.start && analysis_line.end <= paragraph.range.end
        })
        .ok_or(TextError::InvalidParagraph)?;
    let (_, visual_runs) = bidi.visual_runs(paragraph, analysis_line);
    let boundaries = GraphemeClusterSegmenter::new()
        .segment_str(&text[line.clone()])
        .map(|offset| line.start + offset)
        .collect::<Vec<_>>();
    let logical_clusters = boundaries
        .windows(2)
        .map(|pair| pair[0]..pair[1])
        .collect::<Vec<_>>();
    let mut visual = Vec::with_capacity(logical_clusters.len());
    for run in visual_runs {
        let mut clusters = logical_clusters
            .iter()
            .filter(|cluster| run.contains(&analysis.original_to_analysis[cluster.start]))
            .cloned()
            .collect::<Vec<_>>();
        if bidi
            .levels
            .get(run.start)
            .is_some_and(|level| level.is_rtl())
        {
            clusters.reverse();
        }
        visual.extend(clusters);
    }
    if visual.len() != logical_clusters.len() {
        return Err(TextError::InvalidParagraph);
    }
    Ok(visual)
}

fn bidi_directions(
    text: &str,
    spans: &[amazing_text::TextSpan],
    itemized: &[amazing_text::TextSpan],
    base_direction: Direction,
    bidi_scopes: &[amazing_text::BidiScope],
) -> Result<Vec<Direction>, TextError> {
    let default_level = Some(match base_direction {
        Direction::LeftToRight => Level::ltr(),
        Direction::RightToLeft => Level::rtl(),
    });
    let analysis = bidi_analysis(text, spans, bidi_scopes)?;
    let bidi = BidiInfo::new(&analysis.text, default_level);
    itemized
        .iter()
        .map(|span| {
            bidi.levels
                .get(analysis.original_to_analysis[span.range.start])
                .map(|level| {
                    if level.is_rtl() {
                        Direction::RightToLeft
            } else {
                        Direction::LeftToRight
                    }
                })
                .ok_or(TextError::InvalidParagraph)
        })
        .collect()
}

struct BidiAnalysis {
    text: String,
    original_to_analysis: Vec<usize>,
}

fn bidi_analysis(
    text: &str,
    spans: &[amazing_text::TextSpan],
    bidi_scopes: &[amazing_text::BidiScope],
) -> Result<BidiAnalysis, TextError> {
    let mut synthesized = bidi_scopes.to_vec();
    if bidi_scopes.is_empty() {
        synthesized.extend(
            spans
            .iter()
            .filter(|span| span.style.unicode_bidi != amazing_text::UnicodeBidi::Normal)
            .map(|span| amazing_text::BidiScope {
                range: span.range.clone(),
                direction: span.style.direction,
                unicode_bidi: span.style.unicode_bidi,
            }),
        );
    }
    synthesized.extend(
        spans
            .iter()
            .filter(|span| {
                span.style.writing_mode.is_vertical_flow()
                    && span.style.text_orientation == amazing_text::TextOrientation::Upright
            })
            .map(|span| amazing_text::BidiScope {
                range: span.range.clone(),
                direction: Direction::LeftToRight,
                unicode_bidi: amazing_text::UnicodeBidi::BidiOverride,
            }),
    );
    let scopes = synthesized.as_slice();
    let mut ordered = scopes.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|scope| (scope.range.start, std::cmp::Reverse(scope.range.end)));
    let mut stack = Vec::new();
    for scope in &ordered {
        while stack.last().is_some_and(|end| scope.range.start >= *end) {
            stack.pop();
        }
        if stack.last().is_some_and(|end| scope.range.end > *end) {
            return Err(TextError::InvalidParagraph);
        }
        stack.push(scope.range.end);
    }
    let paragraph_breaks = paragraph_break_ranges(text);
    let mut scope_break_operations = 0usize;
    let mut scope_break_windows = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let first = paragraph_breaks.partition_point(|range| range.end <= scope.range.start);
        let end = paragraph_breaks.partition_point(|range| range.start < scope.range.end);
        scope_break_operations = scope_break_operations
            .checked_add(end.saturating_sub(first))
            .ok_or(TextError::ResourceLimit)?;
        if scope_break_operations > MAX_BIDI_SCOPE_BREAK_OPERATIONS {
            return Err(TextError::ResourceLimit);
        }
        scope_break_windows.push(first..end);
    }
    let mut split_scopes = Vec::new();
    for (scope, break_window) in scopes.iter().zip(scope_break_windows) {
        let mut segment_start = scope.range.start;
        for paragraph_break in &paragraph_breaks[break_window] {
            if segment_start < paragraph_break.start {
                if split_scopes.len() >= MAX_BIDI_SCOPE_SEGMENTS {
                    return Err(TextError::ResourceLimit);
                }
                split_scopes.push(amazing_text::BidiScope {
                    range: segment_start..paragraph_break.start,
                    direction: scope.direction,
                    unicode_bidi: scope.unicode_bidi,
                });
            }
            segment_start = paragraph_break.end.min(scope.range.end);
        }
        if segment_start < scope.range.end {
            if split_scopes.len() >= MAX_BIDI_SCOPE_SEGMENTS {
                return Err(TextError::ResourceLimit);
            }
            split_scopes.push(amazing_text::BidiScope {
                range: segment_start..scope.range.end,
                direction: scope.direction,
                unicode_bidi: scope.unicode_bidi,
            });
        }
    }
    let capacity = split_scopes
        .len()
        .checked_mul(12)
        .and_then(|extra| text.len().checked_add(extra))
        .ok_or(TextError::ResourceLimit)?;
    let mut openings: BTreeMap<usize, Vec<&amazing_text::BidiScope>> = BTreeMap::new();
    let mut closings: BTreeMap<usize, Vec<&amazing_text::BidiScope>> = BTreeMap::new();
    for scope in &split_scopes {
        openings.entry(scope.range.start).or_default().push(scope);
        closings.entry(scope.range.end).or_default().push(scope);
    }
    for values in openings.values_mut() {
        values.sort_by_key(|scope| std::cmp::Reverse(scope.range.end));
    }
    for values in closings.values_mut() {
        values.sort_by_key(|scope| std::cmp::Reverse(scope.range.start));
    }
    let mut analysis = String::with_capacity(capacity);
    let mut original_to_analysis = vec![0; text.len() + 1];
    for (original, ch) in text.char_indices() {
        if let Some(values) = closings.get(&original) {
            for scope in values {
                analysis.push_str(bidi_controls(scope.direction, scope.unicode_bidi).1);
            }
        }
        if let Some(values) = openings.get(&original) {
            for scope in values {
                analysis.push_str(bidi_controls(scope.direction, scope.unicode_bidi).0);
            }
        }
        let analysis_start = analysis.len();
        for byte in 0..ch.len_utf8() {
            original_to_analysis[original + byte] = analysis_start + byte;
        }
        analysis.push(ch);
        original_to_analysis[original + ch.len_utf8()] = analysis.len();
    }
    if let Some(values) = closings.get(&text.len()) {
        for scope in values {
            analysis.push_str(bidi_controls(scope.direction, scope.unicode_bidi).1);
        }
    }
    original_to_analysis[text.len()] = analysis.len();
    if spans.is_empty() || spans[0].range.start != 0 || spans.last().unwrap().range.end != text.len() {
        return Err(TextError::InvalidParagraph);
    }
    Ok(BidiAnalysis {
        text: analysis,
        original_to_analysis,
    })
}

fn bidi_controls(
    direction: Direction,
    unicode_bidi: amazing_text::UnicodeBidi,
) -> (&'static str, &'static str) {
    match (direction, unicode_bidi) {
        (_, amazing_text::UnicodeBidi::Normal) => ("", ""),
        (Direction::LeftToRight, amazing_text::UnicodeBidi::Embed) => ("\u{202a}", "\u{202c}"),
        (Direction::RightToLeft, amazing_text::UnicodeBidi::Embed) => ("\u{202b}", "\u{202c}"),
        (Direction::LeftToRight, amazing_text::UnicodeBidi::BidiOverride) => {
            ("\u{202d}", "\u{202c}")
        }
        (Direction::RightToLeft, amazing_text::UnicodeBidi::BidiOverride) => {
            ("\u{202e}", "\u{202c}")
        }
        (Direction::LeftToRight, amazing_text::UnicodeBidi::Isolate) => ("\u{2066}", "\u{2069}"),
        (Direction::RightToLeft, amazing_text::UnicodeBidi::Isolate) => ("\u{2067}", "\u{2069}"),
        (Direction::LeftToRight, amazing_text::UnicodeBidi::IsolateOverride) => {
            ("\u{2068}\u{202d}", "\u{202c}\u{2069}")
        }
        (Direction::RightToLeft, amazing_text::UnicodeBidi::IsolateOverride) => {
            ("\u{2068}\u{202e}", "\u{202c}\u{2069}")
        }
        (_, amazing_text::UnicodeBidi::Plaintext) => ("\u{2068}", "\u{2069}"),
    }
}

fn paragraph_break_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut breaks = Vec::new();
    let mut characters = text.char_indices().peekable();
    while let Some((start, ch)) = characters.next() {
        if ch == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n') {
            let (lf_start, lf) = characters.next().unwrap();
            breaks.push(start..lf_start + lf.len_utf8());
        } else if bidi_class(ch) == BidiClass::B {
            breaks.push(start..start + ch.len_utf8());
        }
    }
    breaks
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
    direction: Direction,
    orientation: GlyphOrientation,
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
        buffer.set_direction(if style.writing_mode.uses_vertical_metrics()
            && orientation == GlyphOrientation::Upright
        {
            harfrust::Direction::TopToBottom
        } else {
            match direction {
                Direction::LeftToRight => harfrust::Direction::LeftToRight,
                Direction::RightToLeft => harfrust::Direction::RightToLeft,
            }
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
                orientation,
                flags: GlyphFlags {
                    unsafe_to_break: info.unsafe_to_break(),
                    unsafe_to_concat: info.unsafe_to_concat(),
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
                direction,
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

fn cluster_orientation(
    style: &TextStyle,
    text: &str,
    cluster: std::ops::Range<usize>,
) -> GlyphOrientation {
    match style.writing_mode {
        WritingMode::HorizontalTb => GlyphOrientation::Upright,
        WritingMode::SidewaysRl => GlyphOrientation::SidewaysRight,
        WritingMode::SidewaysLr => GlyphOrientation::SidewaysLeft,
        WritingMode::VerticalRl | WritingMode::VerticalLr => match style.text_orientation {
            TextOrientation::Upright => GlyphOrientation::Upright,
            TextOrientation::Sideways => GlyphOrientation::SidewaysRight,
            TextOrientation::Mixed => {
                let character = text[cluster].chars().next().unwrap_or('\u{fffd}');
                match unicode_vo::char_orientation(character) {
                    VerticalOrientation::Upright | VerticalOrientation::TransformedOrUpright => {
                        GlyphOrientation::Upright
                    },
                    VerticalOrientation::Rotated | VerticalOrientation::TransformedOrRotated => {
                        GlyphOrientation::SidewaysRight
                    },
                }
            },
        },
    }
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
    use amazing_text::{
        Hyphens, LineBreak, LineRequest, OverflowWrap, ShapingBoundary, TextSpan, UnicodeBidi,
        WordBreak,
    };

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
                u32::from_be_bytes(too_many_glyphs[offset + 8..offset + 12].try_into().unwrap())
                    as usize
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
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
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
    fn synthetic_oblique_changes_the_registered_face_outline() {
        let mut provider = TextProvider::new();
        let font = provider.register_web_font(WebFont {
            family: "Regular only".into(), weight: 400, stretch: 100,
            slant: FontSlant::Normal, format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        }).unwrap();
        let key = GlyphCacheKey::new(provider.generation(), font, GlyphId(1), 30.0, 0, 0).unwrap();
        let normal = provider.rasterize(GlyphRasterRequest {
            key, variations: &[],
            synthesis: amazing_text::FontSynthesis { weight: false, style: false },
        }).unwrap().unwrap();
        let oblique = provider.rasterize(GlyphRasterRequest {
            key: key.with_style(&[], amazing_text::FontSynthesis { weight: false, style: true }),
            variations: &[],
            synthesis: amazing_text::FontSynthesis { weight: false, style: true },
        }).unwrap().unwrap();
        assert_ne!((normal.width, normal.bearing_x, normal.data),
                   (oblique.width, oblique.bearing_x, oblique.data));
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
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
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
    fn earlier_named_family_beats_later_face_regardless_of_attribute_distance() {
        let mut provider = TextProvider::new();
        let first = provider.register_web_font(WebFont {
            family: "CSSTest Multi Word".into(),
            weight: 1000,
            stretch: 1000,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        }).unwrap();
        provider.register_web_font(WebFont {
            family: "second".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        }).unwrap();
        let style = TextStyle {
            families: vec!["'csstest multi word'".into(), "second".into()],
            ..TextStyle::default()
        };
        assert_eq!(provider.fonts[provider.matching_font(&style, Some("A")).unwrap()].key, first);
    }

    #[test]
    fn missing_glyph_in_selected_face_advances_to_next_family() {
        let mut provider = TextProvider::new();
        for (family, slant, glyph) in [
            ("primary", FontSlant::Normal, 'A'),
            ("primary", FontSlant::Italic, 'B'),
            ("secondary", FontSlant::Normal, 'B'),
        ] {
            provider.register_web_font(WebFont {
                family: family.into(), weight: 400, stretch: 100, slant,
                format: FontFormat::OpenType, bytes: synthetic_font(glyph),
            }).unwrap();
        }
        let style = TextStyle {
            families: vec!["primary".into(), "secondary".into()],
            ..TextStyle::default()
        };
        let selected = provider.matching_font(&style, Some("B")).unwrap();
        assert_eq!(provider.fonts[selected].family, "secondary");
    }

    #[test]
    fn missing_author_family_uses_covering_installed_fallback() {
        let mut provider = TextProvider::new();
        let serif = provider.register_web_font(WebFont {
            family: "serif".into(), weight: 400, stretch: 100,
            slant: FontSlant::Normal, format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        }).unwrap();
        let style = TextStyle {
            families: vec!["'Uninstalled Family'".into()],
            ..TextStyle::default()
        };
        assert_eq!(provider.fonts[provider.matching_font(&style, Some("A")).unwrap()].key, serif);
        assert_eq!(provider.matching_font(&style, Some("B")), None);
    }

    #[test]
    fn private_use_ignores_generic_faces_but_uses_named_faces() {
        let mut provider = TextProvider::new();
        provider.register_web_font(WebFont {
            family: "serif".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('\u{E000}'),
        }).unwrap();
        let style = TextStyle { families: vec!["serif".into()], ..TextStyle::default() };
        assert_eq!(provider.matching_font(&style, Some("\u{E000}")), None);
        assert!(provider.matching_font(&style, Some("A")).is_none());
        let quoted = TextStyle { families: vec!["'serif'".into()], ..TextStyle::default() };
        assert!(provider.matching_font(&quoted, Some("\u{E000}")).is_some());
        provider.register_web_font(WebFont {
            family: "PUA fixture".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('\u{E000}'),
        }).unwrap();
        let style = TextStyle { families: vec!["serif".into(), "PUA fixture".into()], ..TextStyle::default() };
        assert!(provider.matching_font(&style, Some("\u{E000}")).is_some());
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
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
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
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();
        assert_eq!(shaped.runs.len(), 1);
        assert_eq!(shaped.runs[0].glyphs.len(), 1);
        assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..3);
        assert!(shaped.runs.iter().all(|run| run.font == FontKey(0)));
    }

    #[test]
    fn mixed_bidi_line_reorders_whole_grapheme_clusters() {
        let provider = TextProvider::new();
        let text = "abc אב\u{05b0}ג";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 30,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::Continue,
        }];

        let line = provider
            .shape_line(LineRequest {
                paragraph: Paragraph {
                    text,
                    spans: &spans,
                    sources: &[],
                bidi_scopes: &[],
                    base_direction: Direction::LeftToRight,
                },
                range: 0..text.len(),
            })
            .unwrap();

        assert_eq!(
            line.visual_clusters,
            vec![0..1, 1..2, 2..3, 3..4, 10..12, 6..10, 4..6]
        );
        let run_clusters = line
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.cluster.clone()))
            .collect::<Vec<_>>();
        assert_eq!(run_clusters, line.visual_clusters);
    }

    #[test]
    fn break_all_adds_only_grapheme_cluster_opportunities() {
        let provider = TextProvider::new();
        let text = "A\u{0301}B";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 31,
            style: TextStyle {
                word_break: WordBreak::BreakAll,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(
            shaped.breaks,
            vec![
                BreakOpportunity {
                    offset: 3,
                    kind: BreakKind::Allowed,
                },
                BreakOpportunity {
                    offset: 4,
                    kind: BreakKind::Mandatory,
                },
            ]
        );
    }

    #[test]
    fn bidi_override_applies_css_direction_to_every_cluster() {
        let provider = TextProvider::new();
        let text = "abאב";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 32,
            style: TextStyle {
                direction: Direction::RightToLeft,
                unicode_bidi: UnicodeBidi::BidiOverride,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];

        let line = provider
            .shape_line(LineRequest {
                paragraph: Paragraph {
                    text,
                    spans: &spans,
                    sources: &[],
                bidi_scopes: &[],
                    base_direction: Direction::LeftToRight,
                },
                range: 0..text.len(),
            })
            .unwrap();

        assert_eq!(line.visual_clusters, vec![4..6, 2..4, 1..2, 0..1]);
    }

    #[test]
    fn manual_soft_hyphen_exposes_a_discretionary_break() {
        let provider = TextProvider::new();
        let text = "ab\u{00ad}cd";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 33,
            style: TextStyle {
                hyphens: Hyphens::Manual,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert!(shaped.breaks.contains(&BreakOpportunity {
            offset: 4,
            kind: BreakKind::Hyphen,
        }));
    }

    #[test]
    fn line_break_anywhere_adds_typographic_character_boundaries() {
        let provider = TextProvider::new();
        let text = "A\u{0301}B";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 34,
            style: TextStyle {
                line_break: LineBreak::Anywhere,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(
            shaped.breaks,
            vec![
                BreakOpportunity {
                    offset: 3,
                    kind: BreakKind::Allowed,
                },
                BreakOpportunity {
                    offset: 4,
                    kind: BreakKind::Mandatory,
                },
            ]
        );
    }

    #[test]
    fn overflow_wrap_exposes_separate_emergency_cluster_breaks() {
        let provider = TextProvider::new();
        let text = "A\u{0301}B";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 35,
            style: TextStyle {
                overflow_wrap: OverflowWrap::BreakWord,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(
            shaped.breaks,
            vec![
                BreakOpportunity {
                    offset: 3,
                    kind: BreakKind::Emergency,
                },
                BreakOpportunity {
                    offset: 4,
                    kind: BreakKind::Mandatory,
                },
            ]
        );
    }

    #[test]
    fn keep_all_preserves_the_break_after_ideographic_space() {
        let provider = TextProvider::new();
        let text = "字字　字字";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 36,
            style: TextStyle {
                word_break: WordBreak::KeepAll,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];

        let shaped = provider
            .shape(Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(
            shaped.breaks,
            vec![
                BreakOpportunity {
                    offset: 9,
                    kind: BreakKind::Allowed,
                },
                BreakOpportunity {
                    offset: 15,
                    kind: BreakKind::Mandatory,
                },
            ]
        );
    }

    #[test]
    fn css_override_remains_resolved_when_a_line_ends_inside_the_span() {
        let provider = TextProvider::new();
        let text = "> abcd > אבגד >";
        let outer = TextStyle::default();
        let inner = TextStyle {
            direction: Direction::RightToLeft,
            unicode_bidi: UnicodeBidi::BidiOverride,
            ..TextStyle::default()
        };
        let spans = [
            TextSpan {
                range: 0..2,
                source: 40,
                style: outer.clone(),
                boundary_before: ShapingBoundary::Continue,
            },
            TextSpan {
                range: 2..17,
                source: 41,
                style: inner,
                boundary_before: ShapingBoundary::Continue,
            },
            TextSpan {
                range: 17..19,
                source: 42,
                style: outer,
                boundary_before: ShapingBoundary::Continue,
            },
        ];

        let line = provider
            .shape_line(LineRequest {
                paragraph: Paragraph {
                    text,
                    spans: &spans,
                    sources: &[],
                bidi_scopes: &[],
                    base_direction: Direction::LeftToRight,
                },
                range: 0..8,
            })
            .unwrap();

        assert_eq!(
            line.visual_clusters,
            vec![0..1, 1..2, 7..8, 6..7, 5..6, 4..5, 3..4, 2..3]
        );
    }

    #[test]
    fn ltr_paragraph_keeps_an_rtl_isolate_before_a_following_number() {
        let provider = TextProvider::new();
        let text = "א 3";
        let spans = [
            TextSpan {
                range: 0..2,
                source: 43,
                style: TextStyle {
                    direction: Direction::RightToLeft,
                    unicode_bidi: UnicodeBidi::Isolate,
                    ..TextStyle::default()
                },
                boundary_before: ShapingBoundary::Continue,
            },
            TextSpan {
                range: 2..4,
                source: 44,
                style: TextStyle::default(),
                boundary_before: ShapingBoundary::Continue,
            },
        ];

        let line = provider
            .shape_line(LineRequest {
                paragraph: Paragraph {
                    text,
                    spans: &spans,
                    sources: &[],
                bidi_scopes: &[],
                    base_direction: Direction::LeftToRight,
                },
                range: 0..text.len(),
            })
            .unwrap();

        assert_eq!(line.visual_clusters, vec![0..2, 2..3, 3..4]);
    }

    #[test]
    fn line_break_tailoring_follows_each_style_span() {
        let provider = TextProvider::new();
        let text = "AB CD";
        let spans = [
            TextSpan {
                range: 0..2,
                source: 45,
                style: TextStyle {
                    word_break: WordBreak::BreakAll,
                    ..TextStyle::default()
                },
                boundary_before: ShapingBoundary::Continue,
            },
            TextSpan {
                range: 2..5,
                source: 46,
                style: TextStyle::default(),
                boundary_before: ShapingBoundary::StyleChange,
            },
        ];
        let shaped = provider
            .shape(Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(
            shaped.breaks,
            vec![
                BreakOpportunity {
                    offset: 1,
                    kind: BreakKind::Allowed,
                },
                BreakOpportunity {
                    offset: 3,
                    kind: BreakKind::Allowed,
                },
                BreakOpportunity {
                    offset: 5,
                    kind: BreakKind::Mandatory,
                },
            ]
        );
    }

    #[test]
    fn nested_bidi_scope_is_stable_across_descendant_style_spans() {
        let provider = TextProvider::new();
        let text = "א 3";
        let style = TextStyle::default();
        let single = [TextSpan {
            range: 0..text.len(),
            source: 47,
            style: style.clone(),
            boundary_before: ShapingBoundary::Continue,
        }];
        let split = [
            TextSpan {
                range: 0..2,
                source: 47,
                style: style.clone(),
                boundary_before: ShapingBoundary::Continue,
            },
            TextSpan {
                range: 2..text.len(),
                source: 48,
                style,
                boundary_before: ShapingBoundary::StyleChange,
            },
        ];
        let scopes = [amazing_text::BidiScope {
            range: 0..text.len(),
            direction: Direction::RightToLeft,
            unicode_bidi: UnicodeBidi::Isolate,
        }];
        let shape = |spans: &[TextSpan]| {
            provider
                .shape_line(LineRequest {
                    paragraph: Paragraph {
                        text,
                        spans,
                        sources: &[],
                        bidi_scopes: &scopes,
                        base_direction: Direction::LeftToRight,
                    },
                    range: 0..text.len(),
                })
                .unwrap()
                .visual_clusters
        };

        assert_eq!(shape(&single), shape(&split));
    }

    #[test]
    fn bidi_scope_closes_before_and_reopens_after_paragraph_break() {
        for (text, expected) in [
            ("א\nא", "\u{2067}א\u{2069}\n\u{2067}א\u{2069}"),
            ("א\r\nא", "\u{2067}א\u{2069}\r\n\u{2067}א\u{2069}"),
            ("א\u{001c}א", "\u{2067}א\u{2069}\u{001c}\u{2067}א\u{2069}"),
        ] {
            let spans = [TextSpan {
                range: 0..text.len(),
                source: 47,
                style: TextStyle::default(),
                boundary_before: ShapingBoundary::Continue,
            }];
            let scopes = [amazing_text::BidiScope {
                range: 0..text.len(),
                direction: Direction::RightToLeft,
                unicode_bidi: UnicodeBidi::Isolate,
            }];

            let analysis = bidi_analysis(text, &spans, &scopes).unwrap();

            assert_eq!(analysis.text, expected);
        }
    }

    #[test]
    fn bidi_scope_does_not_reset_at_a_non_b_line_separator() {
        let text = "א\u{2028}א";
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 47,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::Continue,
        }];
        let scopes = [amazing_text::BidiScope {
            range: 0..text.len(),
            direction: Direction::RightToLeft,
            unicode_bidi: UnicodeBidi::Isolate,
        }];

        let analysis = bidi_analysis(text, &spans, &scopes).unwrap();

        assert_eq!(analysis.text, "\u{2067}א\u{2028}א\u{2069}");
    }

    #[test]
    fn isolate_override_uses_first_strong_isolate_around_override() {
        assert_eq!(
            bidi_controls(Direction::LeftToRight, UnicodeBidi::IsolateOverride),
            ("\u{2068}\u{202d}", "\u{202c}\u{2069}")
        );
        assert_eq!(
            bidi_controls(Direction::RightToLeft, UnicodeBidi::IsolateOverride),
            ("\u{2068}\u{202e}", "\u{202c}\u{2069}")
        );
    }

    #[test]
    fn bidi_scope_paragraph_break_cross_product_has_an_operation_cap() {
        let text = "\n".repeat(4097);
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 47,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::Continue,
        }];
        let scopes = (0..4097)
            .map(|_| amazing_text::BidiScope {
                range: 0..text.len(),
                direction: Direction::RightToLeft,
                unicode_bidi: UnicodeBidi::Isolate,
            })
            .collect::<Vec<_>>();

        assert!(matches!(
            bidi_analysis(&text, &spans, &scopes),
            Err(TextError::ResourceLimit)
        ));
    }

    #[test]
    fn unsafe_breaks_include_internal_and_flagged_harf_edges() {
        let run = ShapedRun {
            text_range: 0..3,
            source: 1,
            font: FontKey(0),
            font_size: 16.0,
            direction: Direction::LeftToRight,
            writing_mode: WritingMode::HorizontalTb,
            metrics: FontMetrics {
                ascent: 10.0,
                descent: 3.0,
                leading: 0.0,
                x_height: 5.0,
                cap_height: 7.0,
                vertical_ascent: 8.0,
                vertical_descent: 8.0,
                vertical_leading: 0.0,
                has_canonical_vertical_metrics: false,
            },
            variations: Vec::new(),
            synthesis: amazing_text::FontSynthesis::default(),
            glyphs: vec![
                ShapedGlyph {
                    id: GlyphId(1),
                    cluster: 0..2,
                    advance_x: 1.0,
                    advance_y: 0.0,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    orientation: amazing_text::GlyphOrientation::Upright,
                    flags: GlyphFlags::default(),
                },
                ShapedGlyph {
                    id: GlyphId(2),
                    cluster: 2..3,
                    advance_x: 1.0,
                    advance_y: 0.0,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    orientation: amazing_text::GlyphOrientation::Upright,
                    flags: GlyphFlags {
                        unsafe_to_break: true,
                        unsafe_to_concat: false,
                    },
                },
            ],
        };
        let breaks = [
            BreakOpportunity {
                offset: 1,
                kind: BreakKind::Allowed,
            },
            BreakOpportunity {
                offset: 2,
                kind: BreakKind::Allowed,
            },
        ];

        assert_eq!(unsafe_break_offsets(&[run], &breaks), vec![1, 2]);
    }

    #[test]
    fn paragraph_cluster_limit_is_checked_before_itemized_output_allocation() {
        let provider = TextProvider::new();
        let text = "a".repeat(MAX_TEXT_CLUSTERS + 1);
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 49,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::Continue,
        }];

        assert_eq!(
            provider.shape(Paragraph {
                text: &text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            }),
            Err(TextError::ResourceLimit)
        );
    }
}
