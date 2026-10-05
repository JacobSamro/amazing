//! Vetted text-provider adapter for untrusted web-font bytes.

#![forbid(unsafe_code)]

mod font_display;
mod font_matching;
mod font_relative_metrics;
mod font_variant_synthesis;
mod font_variations;
mod line_break_objects;
mod text_emphasis;
mod text_transform;
pub use font_display::FontDisplayPeriods;
pub use text_emphasis::text_emphasis_mark;
pub use text_transform::apply_text_transform;
mod emoji_presentation;
mod font_decode;
mod font_selection;
mod shaping;
#[cfg(test)]
mod tests;
pub use font_decode::decode_web_font_to_sfnt;

use amazing_text::{
    BreakKind, BreakOpportunity, Direction, FallbackTextSystem, FontAttributes, FontKey,
    FontSizeAdjustMetric, FontSlant, GlyphCacheKey, GlyphImage, GlyphRasterRequest,
    GlyphRasterScale, LineRequest, Paragraph, ShapedLine, ShapedParagraph, ShapedRun,
    ShapingBoundary, TextError, TextStyle, TextSystem,
};
use font_decode::{parse_font_format_hint, sniff_font_format, supported_font_technology};
use font_selection::font_covers;
use icu_segmenter::options::{LineBreakOptions, LineBreakStrictness, LineBreakWordOption};
use icu_segmenter::{GraphemeClusterSegmenter, LineSegmenter};
use shaping::{
    cluster_orientation, fallback_grapheme_run, font_metrics, hard_context_range,
    rasterize_font_glyph, runs_in_visual_cluster_order, shape_range,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};
// These four exist only so `tests.rs`'s `use super::*;` (and
// `web_font_decode_tests.rs`, spliced into it via `include!`) can still name
// them bare, the way they could before the TEXT-27 module split. A plain
// `cargo build` never activates `#[cfg(test)] mod tests;`, so it sees no use
// of them and warns; `cargo test`/`nextest` does use them, and the warning
// goes away for that build.
#[allow(unused_imports)]
use amazing_text::{
    FontMetrics, FontSynthesis, GlyphContent, GlyphFlags, GlyphId, GlyphOrientation, ShapedGlyph,
    TextOrientation, WritingMode,
};
#[allow(unused_imports)]
use emoji_presentation::{EmojiPresentation, emoji_presentation_intent, has_color_glyph_tables};
#[allow(unused_imports)]
use font_decode::sfnt_checksum;
#[allow(unused_imports)]
use font_selection::font_covers_space;
#[allow(unused_imports)]
use std::panic::{AssertUnwindSafe, catch_unwind};

const MAX_COMPRESSED_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SFNT_FONT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FONT_SET_BYTES: usize = 64 * 1024 * 1024;
const MAX_FONT_FACES: usize = 256;
const MAX_FONT_FACE_RULES: usize = 256;
const MAX_FONT_FACE_SOURCES: usize = 32;
const MAX_UNICODE_RANGES: usize = 1024;
const MAX_FONT_TABLES: usize = 256;
const MAX_FONT_GLYPHS: usize = 60_000;
const MAX_VARIATION_AXES: usize = 64;
/// TrueType Collection (.ttc) files bundle multiple fonts sharing table data
/// (for example large CJK families). We only ever register and rasterize the
/// collection's first font (index 0), but still bound how many entries the
/// header may claim before we trust it.
const MAX_TTC_FONTS: usize = 64;
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
    /// A TrueType/OpenType Collection (`ttcf` magic, `.ttc`). We register and
    /// rasterize only the collection's first font (index 0); `read-fonts`'
    /// `FontRef::from_index(_, 0)` and swash's `FontRef::from_index` both
    /// accept the raw collection bytes directly, so no bytes are copied out
    /// or repackaged.
    Collection,
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

/// Inclusive CSS descriptor range. Values are normalized by the CSS owner
/// before crossing the provider boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontDescriptorRange {
    pub min: f32,
    pub max: f32,
}

impl FontDescriptorRange {
    pub const fn single(value: f32) -> Self {
        Self {
            min: value,
            max: value,
        }
    }

    fn nearest(self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }

    fn valid(self) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min >= 1.0
            && self.min <= self.max
            && self.max <= 1000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontStyleDescriptor {
    Normal,
    Italic,
    Oblique { min_degrees: f32, max_degrees: f32 },
}

impl FontStyleDescriptor {
    fn valid(self) -> bool {
        match self {
            Self::Oblique {
                min_degrees,
                max_degrees,
            } => {
                min_degrees.is_finite()
                    && max_degrees.is_finite()
                    && (-90.0..=90.0).contains(&min_degrees)
                    && min_degrees <= max_degrees
                    && max_degrees <= 90.0
            }
            _ => true,
        }
    }

    fn slant(self) -> FontSlant {
        match self {
            Self::Normal => FontSlant::Normal,
            Self::Italic => FontSlant::Italic,
            Self::Oblique { .. } => FontSlant::Oblique,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnicodeRange {
    pub start: u32,
    pub end: u32,
}

impl UnicodeRange {
    fn valid(self) -> bool {
        self.start <= self.end && self.end <= 0x10FFFF
    }

    fn contains(self, codepoint: u32) -> bool {
        self.start <= codepoint && codepoint <= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontDisplay {
    #[default]
    Auto,
    Block,
    Swap,
    Fallback,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFaceSource {
    Local(String),
    Url {
        url: String,
        /// CSS `format()` names, in declaration order. Empty means sniff the
        /// bounded response bytes. Unsupported names make this source unusable.
        formats: Vec<String>,
        /// CSS `tech()` names. Unknown or unsupported technology is skipped.
        technologies: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceDescriptor {
    pub family: String,
    pub sources: Vec<FontFaceSource>,
    /// Empty means the full Unicode code space.
    pub unicode_ranges: Vec<UnicodeRange>,
    pub weight: FontDescriptorRange,
    pub stretch: FontDescriptorRange,
    pub style: FontStyleDescriptor,
    pub display: FontDisplay,
    pub size_adjust: f32,
    pub ascent_override: Option<f32>,
    pub descent_override: Option<f32>,
    pub line_gap_override: Option<f32>,
    pub features: Vec<amazing_text::FeatureSetting>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRegistration {
    pub family: String,
    pub display: FontDisplay,
    pub key: Option<FontKey>,
}

impl Default for FontFaceDescriptor {
    fn default() -> Self {
        Self {
            family: String::new(),
            sources: Vec::new(),
            unicode_ranges: Vec::new(),
            weight: FontDescriptorRange::single(400.0),
            stretch: FontDescriptorRange::single(100.0),
            style: FontStyleDescriptor::Normal,
            display: FontDisplay::Auto,
            size_adjust: 1.0,
            ascent_override: None,
            descent_override: None,
            line_gap_override: None,
            features: Vec::new(),
        }
    }
}

/// The network owner implements this boundary with a CORS-mode fetch. The
/// provider supplies a byte cap, never performs URL resolution or network I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFetchRequest<'a> {
    pub url: &'a str,
    pub cors_mode: bool,
    pub max_bytes: usize,
}

pub trait FontFaceLoader {
    fn load_local(&mut self, name: &str, max_bytes: usize) -> Result<Option<Vec<u8>>, TextError>;
    fn load_url(&mut self, request: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError>;
}

#[derive(Debug, Clone)]
struct FontFaceMetadata {
    weight: FontDescriptorRange,
    stretch: FontDescriptorRange,
    style: FontStyleDescriptor,
    unicode_ranges: Vec<UnicodeRange>,
    display: FontDisplay,
    size_adjust: f32,
    ascent_override: Option<f32>,
    descent_override: Option<f32>,
    line_gap_override: Option<f32>,
    features: Vec<amazing_text::FeatureSetting>,
}

#[derive(Debug)]
struct FontRecord {
    key: FontKey,
    family: String,
    weight: u16,
    stretch: u16,
    slant: FontSlant,
    sfnt: Vec<u8>,
    author_face: Option<FontFaceMetadata>,
}

impl FontRecord {
    fn attributes_for(&self, style: &TextStyle) -> FontAttributes {
        if let Some(face) = &self.author_face {
            FontAttributes {
                weight: face.weight.nearest(style.weight as f32).round() as u16,
                stretch: face
                    .stretch
                    .nearest(font_matching::requested_width(style))
                    .round() as u16,
                slant: face.style.slant(),
            }
        } else {
            FontAttributes {
                weight: self.weight,
                stretch: self.stretch,
                slant: self.slant,
            }
        }
    }
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
    author_families: BTreeSet<String>,
    face_rules: Vec<FontFaceRegistration>,
    font_set_bytes: usize,
    pending_font_loads: Vec<FontDisplay>,
    raster_cache: RefCell<HashMap<(GlyphCacheKey, GlyphRasterScale), Option<GlyphImage>>>,
    raster_cache_bytes: Cell<usize>,
    fallback: FallbackTextSystem,
}

impl Default for TextProvider {
    fn default() -> Self {
        Self {
            generation: 0,
            next_font_key: 1,
            fonts: Vec::new(),
            author_families: BTreeSet::new(),
            face_rules: Vec::new(),
            font_set_bytes: 0,
            pending_font_loads: Vec::new(),
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

    /// Number of asynchronous font loads still in progress. Font registration
    /// currently completes synchronously through a caller-owned loader.
    pub fn pending_font_loads(&self) -> usize {
        self.pending_font_loads.len()
    }

    pub fn pending_font_display_periods(&self) -> Vec<FontDisplayPeriods> {
        self.pending_font_loads
            .iter()
            .map(|display| display.periods())
            .collect()
    }

    pub fn begin_font_load(&mut self, display: FontDisplay) -> Result<(), TextError> {
        if self.pending_font_loads.len() >= MAX_FONT_FACE_RULES {
            return Err(TextError::ResourceLimit);
        }
        self.pending_font_loads.push(display);
        Ok(())
    }

    pub fn end_font_load(&mut self) {
        self.pending_font_loads.pop();
    }

    pub fn register_web_font(&mut self, font: WebFont) -> Result<FontKey, TextError> {
        self.register_font_bytes(font, None)
    }

    /// Registers one CSS `@font-face` rule through a caller-owned loader.
    /// Sources are attempted in declaration order. A missing or invalid source
    /// advances to the next source; a successfully loaded face retains the
    /// descriptor ranges and display policy for later matching and inspection.
    pub fn register_font_face<L: FontFaceLoader>(
        &mut self,
        descriptor: FontFaceDescriptor,
        loader: &mut L,
    ) -> Result<Option<FontKey>, TextError> {
        let family = descriptor.family.trim().to_ascii_lowercase();
        if family.is_empty()
            || family.len() > 256
            || descriptor.sources.is_empty()
            || descriptor.sources.len() > MAX_FONT_FACE_SOURCES
            || descriptor.unicode_ranges.len() > MAX_UNICODE_RANGES
            || !descriptor.unicode_ranges.iter().all(|range| range.valid())
            || !descriptor.weight.valid()
            || !descriptor.stretch.valid()
            || !descriptor.style.valid()
            || !descriptor.size_adjust.is_finite()
            || descriptor.size_adjust <= 0.0
            || descriptor.size_adjust > 10.0
            || [
                descriptor.ascent_override,
                descriptor.descent_override,
                descriptor.line_gap_override,
            ]
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || !(0.0..=10.0).contains(value))
        {
            return Err(TextError::InvalidStyle);
        }
        if self.fonts.len() >= MAX_FONT_FACES || self.face_rules.len() >= MAX_FONT_FACE_RULES {
            return Err(TextError::ResourceLimit);
        }
        let metadata = FontFaceMetadata {
            weight: descriptor.weight,
            stretch: descriptor.stretch,
            style: descriptor.style,
            unicode_ranges: descriptor.unicode_ranges,
            display: descriptor.display,
            size_adjust: descriptor.size_adjust,
            ascent_override: descriptor.ascent_override,
            descent_override: descriptor.descent_override,
            line_gap_override: descriptor.line_gap_override,
            features: descriptor.features,
        };
        // A declared author family shadows an installed family with the same
        // name even if none of its sources can be loaded.
        if self.author_families.insert(family.clone()) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or(TextError::ResourceLimit)?;
        }
        for source in descriptor.sources {
            let (bytes, allowed_formats) = match source {
                FontFaceSource::Local(name) => {
                    if name.trim().is_empty() {
                        continue;
                    }
                    (loader.load_local(&name, MAX_COMPRESSED_FONT_BYTES), None)
                }
                FontFaceSource::Url {
                    url,
                    formats,
                    technologies,
                } => {
                    if url.trim().is_empty()
                        || !technologies
                            .iter()
                            .all(|name| supported_font_technology(name))
                    {
                        continue;
                    }
                    let accepted = formats
                        .iter()
                        .filter_map(|name| parse_font_format_hint(name))
                        .collect::<Vec<_>>();
                    if !formats.is_empty() && accepted.is_empty() {
                        continue;
                    }
                    (
                        loader.load_url(FontFetchRequest {
                            url: &url,
                            cors_mode: true,
                            max_bytes: MAX_COMPRESSED_FONT_BYTES,
                        }),
                        Some(accepted),
                    )
                }
            };
            let Ok(Some(bytes)) = bytes else {
                continue;
            };
            if bytes.is_empty() || bytes.len() > MAX_COMPRESSED_FONT_BYTES {
                continue;
            }
            let Some(format) = sniff_font_format(&bytes) else {
                continue;
            };
            if allowed_formats
                .is_some_and(|formats| !formats.is_empty() && !formats.contains(&format))
            {
                continue;
            }
            let font = WebFont {
                family: family.clone(),
                weight: metadata.weight.min.round() as u16,
                stretch: metadata.stretch.min.round() as u16,
                slant: metadata.style.slant(),
                format,
                bytes,
            };
            match self.register_font_bytes(font, Some(metadata.clone())) {
                Ok(key) => {
                    self.face_rules.push(FontFaceRegistration {
                        family,
                        display: metadata.display,
                        key: Some(key),
                    });
                    return Ok(Some(key));
                }
                Err(TextError::MalformedFont | TextError::ResourceLimit) => continue,
                Err(error) => return Err(error),
            }
        }
        self.face_rules.push(FontFaceRegistration {
            family,
            display: metadata.display,
            key: None,
        });
        Ok(None)
    }

    /// Physical first-available-font aspect ratio for CSS font-size-adjust.
    /// This ignores paragraph orientation and returns None without an actual
    /// selected font. Missing metric/glyph fallbacks follow CSS Fonts.
    pub fn font_size_adjust_ratio(
        &self,
        style: &TextStyle,
        metric: FontSizeAdjustMetric,
    ) -> Option<f32> {
        let index = self.matching_font_with_space_policy(style, Some(" "), true)?;
        font_relative_metrics::aspect_ratio(&self.fonts[index], style, metric)
    }

    pub fn font_face_registrations(&self) -> &[FontFaceRegistration] {
        &self.face_rules
    }

    pub fn font_display(&self, key: FontKey) -> Option<FontDisplay> {
        self.fonts
            .iter()
            .find(|font| font.key == key)
            .and_then(|font| font.author_face.as_ref().map(|face| face.display))
    }

    fn register_font_bytes(
        &mut self,
        font: WebFont,
        author_face: Option<FontFaceMetadata>,
    ) -> Result<FontKey, TextError> {
        let family = font.family.trim();
        if family.is_empty()
            || !(1..=1000).contains(&font.weight)
            || font.stretch == 0
            || font.stretch > 1000
        {
            return Err(TextError::InvalidStyle);
        }
        let decoded = decode_web_font_to_sfnt(font.format, &font.bytes)?;
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
            author_face,
        });
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(TextError::ResourceLimit)?;
        self.raster_cache.get_mut().clear();
        self.raster_cache_bytes.set(0);
        Ok(key)
    }
}


impl TextSystem for TextProvider {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn metrics(&self, style: &TextStyle) -> Result<amazing_text::FontMetrics, TextError> {
        let Some(index) = self.matching_font_with_space_policy(style, Some(" "), true) else {
            return self.fallback.metrics(style);
        };
        font_metrics(&self.fonts[index], style.size)
    }

    fn font_relative_metrics(&self, style: &TextStyle) -> amazing_text::FontRelativeMetrics {
        let Some(index) = self.matching_font_with_space_policy(style, Some(" "), true) else {
            return self.fallback.font_relative_metrics(style);
        };
        font_relative_metrics::for_font(&self.fonts[index], style)
    }

    /// CSS Fonts 4 §2.6: resolve the requested `font-size-adjust` metric against
    /// the first available font, then scale the used size by the aspect value.
    fn adjusted_font_size(
        &self,
        style: &TextStyle,
        metric: FontSizeAdjustMetric,
        aspect: f32,
    ) -> f32 {
        if !style.size.is_finite() || style.size <= 0.0 || !aspect.is_finite() || aspect < 0.0 {
            return style.size;
        }
        if aspect == 0.0 {
            return 0.0;
        }
        let Some(ratio) = self.font_size_adjust_ratio(style, metric) else { return style.size; };
        if ratio.is_finite() && ratio > 0.0 {
            style.size * aspect / ratio
        } else {
            style.size
        }
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
        let scale = GlyphRasterScale::unscaled(request.key);
        self.rasterize_at_scale(request, scale)
    }

    fn rasterize_at_scale(
        &self,
        request: GlyphRasterRequest<'_>,
        scale: GlyphRasterScale,
    ) -> Result<Option<GlyphImage>, TextError> {
        let key = request.key;
        scale.device_size(key)?;
        if key.generation != self.generation {
            return Err(TextError::MissingFont);
        }
        let cache_key = (key, scale);
        if let Some(cached) = self.raster_cache.borrow().get(&cache_key) {
            return Ok(cached.clone());
        }
        let image = if key.font == FontKey(0) {
            self.fallback.rasterize_at_scale(
                GlyphRasterRequest {
                    key: GlyphCacheKey {
                        generation: 0,
                        ..key
                    },
                    variations: request.variations,
                    synthesis: request.synthesis,
                },
                scale,
            )?
        } else {
            let font = self
                .fonts
                .iter()
                .find(|font| font.key == key.font)
                .ok_or(TextError::MissingFont)?;
            rasterize_font_glyph(font, &request, scale)?
        };
        self.cache_device_raster(cache_key, image.clone());
        Ok(image)
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
            let orientation =
                cluster_orientation(&itemized[index].style, text, itemized[index].range.clone());
            while end < itemized.len()
                && selected[end] == Some(font_index)
                && itemized[end].boundary_before == ShapingBoundary::Continue
                && itemized[end].style == itemized[index].style
                && directions[end] == directions[index]
                && cluster_orientation(&itemized[end].style, text, itemized[end].range.clone())
                    == orientation
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

    #[cfg(test)]
    fn cache_raster(&self, key: GlyphCacheKey, image: Option<GlyphImage>) {
        self.cache_device_raster((key, GlyphRasterScale::unscaled(key)), image);
    }

    fn cache_device_raster(
        &self,
        key: (GlyphCacheKey, GlyphRasterScale),
        image: Option<GlyphImage>,
    ) {
        let entry_overhead =
            std::mem::size_of::<((GlyphCacheKey, GlyphRasterScale), Option<GlyphImage>)>();
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

/// Decodes and validates one untrusted web font using the same per-font limits
/// and sfnt checks as `TextProvider::register_web_font`.


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
            // `manual` behaves like `keep-all` everywhere ICU's own word
            // option applies (CJK ideograph adjacency); the two diverge for
            // complex-context/dictionary-segmented scripts (Thai, Lao,
            // Khmer, ...), where `KeepAll` does not suppress ICU's
            // dictionary-assisted word-internal breaks (verified: it still
            // produced them for Thai). `manual`'s extra reach for those
            // scripts is filtered separately below, since the ICU line
            // segmenter has no option to disable dictionary segmentation
            // itself.
            amazing_text::WordBreak::KeepAll | amazing_text::WordBreak::Manual => {
                LineBreakWordOption::KeepAll
            }
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
                .filter(|offset| {
                    // CSS Text 4 `word-break: manual`: "words are not broken
                    // by the UA for line-wrapping purposes" beyond breaks
                    // defined by an explicit separator (whitespace, soft
                    // hyphen -- handled by `break_kind` below) or a
                    // mandatory break. Drop every other opportunity ICU's
                    // dictionary segmentation contributed for complex-
                    // context scripts.
                    word_break != amazing_text::WordBreak::Manual
                        || *offset == text.len()
                        || mandatory_break_before(text, *offset)
                        || is_whitespace_adjacent(text, *offset)
                })
                .filter_map(|offset| {
                    break_kind(text, spans, offset).map(|kind| BreakOpportunity { offset, kind })
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
    // ICU does not expose every contingent break around U+FFFC. Apply LB20
    // after its higher-priority punctuation and glue rules at those seams.
    opportunities.retain(|opportunity| line_break_objects::allows(text, opportunity.offset));
    opportunities.extend(line_break_objects::opportunities(text, spans));
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

/// True when the character immediately before or after `offset` is
/// whitespace. Used by `word-break: manual` to keep only the break
/// opportunities an explicit separator justifies.
fn is_whitespace_adjacent(text: &str, offset: usize) -> bool {
    text[..offset]
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
        || text[offset..].chars().next().is_some_and(char::is_whitespace)
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
                span.style.writing_mode.uses_vertical_metrics()
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
    if spans.is_empty()
        || spans[0].range.start != 0
        || spans.last().unwrap().range.end != text.len()
    {
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

#[cfg(test)]
mod device_raster_tests;
