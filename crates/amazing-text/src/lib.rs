//! Provider-neutral text interface shared by layout and paint.
//!
//! The interface accepts paragraph text with shaping-affecting style spans and
//! returns positioned glyph clusters. Layout uses those advances; paint asks
//! the same adapter to rasterize the returned font/glyph ids. Provider types
//! never cross this seam.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::ops::Range;

const MAX_PARAGRAPH_BYTES: usize = 8 * 1024 * 1024;
const MAX_PARAGRAPH_SPANS: usize = 65_536;
const MAX_SOURCE_MAPPINGS: usize = 262_144;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontKey(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    #[default]
    LeftToRight,
    RightToLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WritingMode {
    #[default]
    HorizontalTb,
    VerticalRl,
    VerticalLr,
    SidewaysRl,
    SidewaysLr,
}

impl WritingMode {
    pub const fn is_vertical_flow(self) -> bool {
        !matches!(self, Self::HorizontalTb)
    }

    pub const fn uses_vertical_metrics(self) -> bool {
        matches!(self, Self::VerticalRl | Self::VerticalLr)
    }

    pub const fn default_glyph_orientation(self) -> GlyphOrientation {
        match self {
            Self::SidewaysRl => GlyphOrientation::SidewaysRight,
            Self::SidewaysLr => GlyphOrientation::SidewaysLeft,
            _ => GlyphOrientation::Upright,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GlyphOrientation {
    #[default]
    Upright,
    SidewaysRight,
    SidewaysLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextOrientation {
    #[default]
    Mixed,
    Upright,
    Sideways,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextCombine {
    #[default]
    None,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
    BreakSpaces,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WordBreak {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Hyphens {
    None,
    #[default]
    Manual,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineBreak {
    #[default]
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontSlant {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontAttributes {
    pub weight: u16,
    pub stretch: u16,
    pub slant: FontSlant,
}

/// Provider-neutral distance used after CSS has supplied one family at a
/// time in ordered preference. The adapter enumerates available faces; the
/// engine contract owns attribute preference and synthesis policy.
pub fn font_match_score(request: FontAttributes, candidate: FontAttributes) -> u32 {
    let slant = u32::from(request.slant != candidate.slant) * 100_000;
    let stretch = request.stretch.abs_diff(candidate.stretch) as u32 * 100;
    let weight = request.weight.abs_diff(candidate.weight) as u32;
    slant + stretch + weight
}

/// CSS Fonts 4 matches width, then style, then weight within a single family.
/// The tuple is ordered so callers can select the least matching face without
/// allowing any attribute difference to reorder the requested family list.
pub fn font_match_rank(
    request: FontAttributes,
    candidate: FontAttributes,
) -> (u8, u16, u8, u8, u16) {
    let width_side = if request.stretch <= 100 {
        u8::from(candidate.stretch > request.stretch)
    } else {
        u8::from(candidate.stretch < request.stretch)
    };
    let style = match (request.slant, candidate.slant) {
        (FontSlant::Normal, FontSlant::Normal)
        | (FontSlant::Italic, FontSlant::Italic)
        | (FontSlant::Oblique, FontSlant::Oblique) => 0,
        (FontSlant::Normal, FontSlant::Oblique)
        | (FontSlant::Italic, FontSlant::Oblique)
        | (FontSlant::Oblique, FontSlant::Italic) => 1,
        _ => 2,
    };
    let weight_side = if (400..=500).contains(&request.weight) {
        if candidate.weight >= request.weight && candidate.weight <= 500 {
            0
        } else if candidate.weight < request.weight {
            1
        } else {
            2
        }
    } else if request.weight < 400 {
        u8::from(candidate.weight > request.weight)
    } else {
        u8::from(candidate.weight < request.weight)
    };
    (
        width_side,
        request.stretch.abs_diff(candidate.stretch),
        style,
        weight_side,
        request.weight.abs_diff(candidate.weight),
    )
}

pub fn resolve_synthesis(
    request: FontAttributes,
    candidate: FontAttributes,
    policy: FontSynthesis,
) -> FontSynthesis {
    FontSynthesis {
        weight: policy.weight && request.weight > candidate.weight,
        style: policy.style
            && request.slant != FontSlant::Normal
            && candidate.slant == FontSlant::Normal,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShapingBoundary {
    /// Paint-only changes do not divide shaping context.
    #[default]
    Continue,
    /// Shaping style changes, while surrounding text remains context.
    StyleChange,
    /// Padding, border, isolation, forced break, or atomic inline content.
    Hard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureSetting {
    pub tag: [u8; 4],
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariationSetting {
    pub tag: [u8; 4],
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontSynthesis {
    pub weight: bool,
    pub style: bool,
}

impl Default for FontSynthesis {
    fn default() -> Self {
        Self {
            weight: true,
            style: true,
        }
    }
}

impl FontSynthesis {
    pub fn bits(self) -> u8 {
        u8::from(self.weight) | (u8::from(self.style) << 1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub families: Vec<String>,
    pub size: f32,
    pub weight: u16,
    pub stretch: u16,
    /// Exact computed CSS font width percentage. Zero means the legacy
    /// integer `stretch` is used until the layout caller supplies this value.
    pub stretch_percent: f32,
    pub slant: FontSlant,
    /// Exact computed CSS oblique angle in degrees; `None` uses the CSS
    /// default 14 degrees when `slant` is oblique.
    pub oblique_angle_degrees: Option<f32>,
    pub direction: Direction,
    pub writing_mode: WritingMode,
    pub text_orientation: TextOrientation,
    pub text_combine: TextCombine,
    pub white_space: WhiteSpace,
    pub word_break: WordBreak,
    pub unicode_bidi: UnicodeBidi,
    pub hyphens: Hyphens,
    pub line_break: LineBreak,
    pub overflow_wrap: OverflowWrap,
    pub language: Option<String>,
    pub features: Vec<FeatureSetting>,
    pub variations: Vec<VariationSetting>,
    pub synthesis: FontSynthesis,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            families: vec!["serif".into()],
            size: 16.0,
            weight: 400,
            stretch: 100,
            stretch_percent: 0.0,
            slant: FontSlant::Normal,
            oblique_angle_degrees: None,
            direction: Direction::LeftToRight,
            writing_mode: WritingMode::HorizontalTb,
            text_orientation: TextOrientation::Mixed,
            text_combine: TextCombine::None,
            white_space: WhiteSpace::Normal,
            word_break: WordBreak::Normal,
            unicode_bidi: UnicodeBidi::Normal,
            hyphens: Hyphens::Manual,
            line_break: LineBreak::Auto,
            overflow_wrap: OverflowWrap::Normal,
            language: None,
            features: Vec::new(),
            variations: Vec::new(),
            synthesis: FontSynthesis::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub range: Range<usize>,
    pub source: u32,
    pub style: TextStyle,
    pub boundary_before: ShapingBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMapping {
    /// Byte range in the transformed paragraph text.
    pub text_range: Range<usize>,
    pub source: u32,
    /// Byte range in the original DOM text node.
    pub source_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiScope {
    pub range: Range<usize>,
    pub direction: Direction,
    pub unicode_bidi: UnicodeBidi,
}

pub struct Paragraph<'a> {
    pub text: &'a str,
    pub spans: &'a [TextSpan],
    pub sources: &'a [SourceMapping],
    pub bidi_scopes: &'a [BidiScope],
    pub base_direction: Direction,
}

pub struct LineRequest<'a> {
    pub paragraph: Paragraph<'a>,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub x_height: f32,
    pub cap_height: f32,
    pub vertical_ascent: f32,
    pub vertical_descent: f32,
    pub vertical_leading: f32,
    pub has_canonical_vertical_metrics: bool,
}

impl FontMetrics {
    pub fn line_height(self) -> f32 {
        self.ascent + self.descent + self.leading
    }

    pub fn vertical_line_width(self) -> f32 {
        self.vertical_ascent + self.vertical_descent + self.vertical_leading
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlyphFlags {
    pub unsafe_to_break: bool,
    pub unsafe_to_concat: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    pub id: GlyphId,
    pub cluster: Range<usize>,
    pub advance_x: f32,
    pub advance_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub orientation: GlyphOrientation,
    pub flags: GlyphFlags,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    pub text_range: Range<usize>,
    pub source: u32,
    pub font: FontKey,
    pub font_size: f32,
    pub direction: Direction,
    pub writing_mode: WritingMode,
    pub metrics: FontMetrics,
    pub variations: Vec<VariationSetting>,
    pub synthesis: FontSynthesis,
    pub glyphs: Vec<ShapedGlyph>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    Allowed,
    Hyphen,
    Emergency,
    Mandatory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakOpportunity {
    pub offset: usize,
    pub kind: BreakKind,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShapedParagraph {
    pub runs: Vec<ShapedRun>,
    pub breaks: Vec<BreakOpportunity>,
    /// Break offsets whose adjacent Harf cluster requires line-edge reshaping.
    pub unsafe_breaks: Vec<usize>,
    pub sources: Vec<SourceMapping>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShapedLine {
    pub text_range: Range<usize>,
    pub runs: Vec<ShapedRun>,
    /// Original logical cluster ranges in display order. Callers use this for
    /// placement, hit testing, and caret mapping without reversing scalars.
    pub visual_clusters: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphCacheKey {
    pub generation: u64,
    pub font: FontKey,
    pub glyph: GlyphId,
    /// CSS pixel size in 1/64-pixel units.
    pub size_64: u32,
    pub subpixel_x_64: u8,
    pub subpixel_y_64: u8,
    pub variation_hash: u64,
    pub synthesis: u8,
}

impl GlyphCacheKey {
    pub fn new(
        generation: u64,
        font: FontKey,
        glyph: GlyphId,
        size: f32,
        subpixel_x_64: u8,
        subpixel_y_64: u8,
    ) -> Option<Self> {
        if !size.is_finite() || size <= 0.0 || size > 4096.0 {
            return None;
        }
        Some(Self {
            generation,
            font,
            glyph,
            size_64: (size * 64.0).round() as u32,
            subpixel_x_64: subpixel_x_64.min(63),
            subpixel_y_64: subpixel_y_64.min(63),
            variation_hash: 0,
            synthesis: 0,
        })
    }

    pub fn with_style(mut self, variations: &[VariationSetting], synthesis: FontSynthesis) -> Self {
        self.variation_hash = variation_hash(variations);
        self.synthesis = synthesis.bits();
        self
    }

    pub fn size(self) -> f32 {
        self.size_64 as f32 / 64.0
    }
}

fn variation_hash(variations: &[VariationSetting]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for variation in variations {
        for byte in variation
            .tag
            .into_iter()
            .chain(variation.value.to_bits().to_le_bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

pub struct GlyphRasterRequest<'a> {
    pub key: GlyphCacheKey,
    pub variations: &'a [VariationSetting],
    pub synthesis: FontSynthesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphContent {
    Mask,
    Color,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphImage {
    pub content: GlyphContent,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    InvalidParagraph,
    InvalidStyle,
    MissingFont,
    MalformedFont,
    ResourceLimit,
}

pub trait TextSystem {
    fn generation(&self) -> u64;
    fn metrics(&self, style: &TextStyle) -> Result<FontMetrics, TextError>;
    fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError>;
    fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError>;
    fn rasterize(&self, request: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError>;
}

#[derive(Debug, Default)]
pub struct FallbackTextSystem;

impl FallbackTextSystem {
    pub fn new() -> Self {
        Self
    }
}

impl TextSystem for FallbackTextSystem {
    fn generation(&self) -> u64 {
        0
    }

    fn metrics(&self, style: &TextStyle) -> Result<FontMetrics, TextError> {
        validate_style(style)?;
        Ok(FontMetrics {
            ascent: style.size * 0.8,
            descent: style.size * 0.2,
            leading: style.size * 0.2,
            x_height: style.size * 0.5,
            cap_height: style.size * 0.7,
            vertical_ascent: style.size * 0.5,
            vertical_descent: style.size * 0.5,
            vertical_leading: 0.0,
            has_canonical_vertical_metrics: false,
        })
    }

    fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
        validate_paragraph(&paragraph)?;
        let mut shaped = ShapedParagraph {
            sources: paragraph.sources.to_vec(),
            ..ShapedParagraph::default()
        };
        for span in paragraph.spans {
            let metrics = self.metrics(&span.style)?;
            let mut glyphs = paragraph.text[span.range.clone()]
                .char_indices()
                .map(|(relative_start, ch)| {
                    let cluster_start = span.range.start + relative_start;
                    let cluster_end = cluster_start + ch.len_utf8();
                    let advance = fallback_advance(ch, span.style.size);
                    let orientation = fallback_glyph_orientation(
                        span.style.writing_mode,
                        span.style.text_orientation,
                        ch,
                    );
                    let vertical_metrics = span.style.writing_mode.uses_vertical_metrics()
                        && orientation == GlyphOrientation::Upright;
                    let (advance_x, advance_y) = if vertical_metrics {
                        (0.0, advance)
                    } else {
                        (advance, 0.0)
                    };
                    ShapedGlyph {
                        id: GlyphId(ch as u32),
                        cluster: cluster_start..cluster_end,
                        advance_x,
                        advance_y,
                        offset_x: if vertical_metrics {
                            -advance * 0.5
                        } else {
                            0.0
                        },
                        offset_y: if vertical_metrics {
                            metrics.ascent
                        } else {
                            0.0
                        },
                        orientation,
                        flags: GlyphFlags::default(),
                    }
                })
                .collect::<Vec<_>>();
            if span.style.direction == Direction::RightToLeft {
                glyphs.reverse();
            }
            shaped.runs.push(ShapedRun {
                text_range: span.range.clone(),
                source: span.source,
                font: FontKey(0),
                font_size: span.style.size,
                direction: span.style.direction,
                writing_mode: span.style.writing_mode,
                metrics,
                variations: span.style.variations.clone(),
                synthesis: span.style.synthesis,
                glyphs,
            });
        }
        for (start, ch) in paragraph.text.char_indices() {
            let offset = start + ch.len_utf8();
            let style = paragraph
                .spans
                .iter()
                .find(|span| span.range.contains(&start))
                .map(|span| &span.style);
            if ch.is_whitespace() {
                shaped.breaks.push(BreakOpportunity {
                    offset,
                    kind: if is_mandatory_break(ch) {
                        BreakKind::Mandatory
                    } else {
                        BreakKind::Allowed
                    },
                });
            } else if ch == '\u{00ad}'
                && style.is_some_and(|style| style.hyphens == Hyphens::Manual)
            {
                shaped.breaks.push(BreakOpportunity {
                    offset,
                    kind: BreakKind::Hyphen,
                });
            } else if style.is_some_and(|style| style.word_break == WordBreak::BreakAll) {
                shaped.breaks.push(BreakOpportunity {
                    offset,
                    kind: BreakKind::Allowed,
                });
            }
        }
        if !paragraph.text.is_empty() {
            shaped.breaks.push(BreakOpportunity {
                offset: paragraph.text.len(),
                kind: BreakKind::Mandatory,
            });
        }
        shaped.breaks.retain(|opportunity| {
            opportunity.kind == BreakKind::Mandatory
                || style_at_break(paragraph.text, paragraph.spans, opportunity.offset)
                    .is_none_or(|style| white_space_wraps(style.white_space))
        });
        shaped.breaks.sort_by_key(|opportunity| opportunity.offset);
        shaped.breaks.dedup_by(|right, left| {
            if left.offset != right.offset {
                return false;
            }
            if right.kind == BreakKind::Mandatory {
                left.kind = BreakKind::Mandatory;
            }
            true
        });
        Ok(shaped)
    }

    fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError> {
        validate_line_request(&request)?;
        let shaped = self.shape(Paragraph {
            text: request.paragraph.text,
            spans: request.paragraph.spans,
            sources: request.paragraph.sources,
            bidi_scopes: request.paragraph.bidi_scopes,
            base_direction: request.paragraph.base_direction,
        })?;
        let mut visual_clusters = shaped
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.cluster.clone()))
            .filter(|cluster| request.range.contains(&cluster.start))
            .collect::<Vec<_>>();
        visual_clusters.sort_by_key(|cluster| cluster.start);
        visual_clusters.dedup();
        if request
            .paragraph
            .spans
            .iter()
            .find(|span| span.range.contains(&request.range.start))
            .is_some_and(|span| span.style.direction == Direction::RightToLeft)
        {
            visual_clusters.reverse();
        }
        let runs = runs_in_visual_cluster_order(&shaped.runs, &visual_clusters);
        Ok(ShapedLine {
            text_range: request.range,
            runs,
            visual_clusters,
        })
    }

    fn rasterize(&self, request: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError> {
        let key = request.key;
        if key.generation != self.generation() || key.font != FontKey(0) {
            return Err(TextError::MissingFont);
        }
        let Some(ch) = char::from_u32(key.glyph.0) else {
            return Err(TextError::InvalidParagraph);
        };
        if ch.is_whitespace() || is_default_ignorable(ch) {
            return Ok(None);
        }
        let size = key.size();
        let width = fallback_advance(ch, size).round().max(1.0) as u32;
        let height = (size * 0.7).round().max(1.0) as u32;
        if width > 4096 || height > 4096 {
            return Err(TextError::ResourceLimit);
        }
        let len = width
            .checked_mul(height)
            .and_then(|pixels| usize::try_from(pixels).ok())
            .ok_or(TextError::ResourceLimit)?;
        Ok(Some(GlyphImage {
            content: GlyphContent::Mask,
            data: vec![255; len],
            width,
            height,
            bearing_x: 0,
            bearing_y: height as i32,
        }))
    }
}

fn runs_in_visual_cluster_order(
    runs: &[ShapedRun],
    visual_clusters: &[Range<usize>],
) -> Vec<ShapedRun> {
    let mut by_cluster: BTreeMap<usize, Vec<ShapedRun>> = BTreeMap::new();
    for run in runs {
        let mut glyphs_by_cluster: BTreeMap<usize, Vec<ShapedGlyph>> = BTreeMap::new();
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
    text_range: Range<usize>,
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

fn validate_style(style: &TextStyle) -> Result<(), TextError> {
    if !style.size.is_finite()
        || style.size <= 0.0
        || style.size > 4096.0
        || !(1..=1000).contains(&style.weight)
        || style.stretch == 0
        || style.stretch > 1000
        || style.families.len() > 64
        || style.features.len() > 256
        || style.variations.len() > 64
        || style
            .variations
            .iter()
            .any(|variation| !variation.value.is_finite())
    {
        return Err(TextError::InvalidStyle);
    }
    Ok(())
}

/// Validates bounded paragraph/style/source structure without shaping or
/// allocating provider output.
pub fn validate_paragraph(paragraph: &Paragraph<'_>) -> Result<(), TextError> {
    if paragraph.text.len() > MAX_PARAGRAPH_BYTES
        || paragraph.spans.len() > MAX_PARAGRAPH_SPANS
        || paragraph.sources.len() > MAX_SOURCE_MAPPINGS
        || paragraph.bidi_scopes.len() > MAX_PARAGRAPH_SPANS
    {
        return Err(TextError::ResourceLimit);
    }
    if paragraph.text.is_empty() {
        return if paragraph.spans.is_empty()
            && paragraph.sources.is_empty()
            && paragraph.bidi_scopes.is_empty()
        {
            Ok(())
        } else {
            Err(TextError::InvalidParagraph)
        };
    }
    let mut cursor = 0usize;
    for span in paragraph.spans {
        if span.range.start != cursor
            || span.range.end <= span.range.start
            || span.range.end > paragraph.text.len()
            || !paragraph.text.is_char_boundary(span.range.start)
            || !paragraph.text.is_char_boundary(span.range.end)
        {
            return Err(TextError::InvalidParagraph);
        }
        validate_style(&span.style)?;
        cursor = span.range.end;
    }
    if cursor != paragraph.text.len() {
        return Err(TextError::InvalidParagraph);
    }
    if paragraph.sources.iter().any(|mapping| {
        mapping.text_range.start >= mapping.text_range.end
            || mapping.text_range.end > paragraph.text.len()
            || !paragraph.text.is_char_boundary(mapping.text_range.start)
            || !paragraph.text.is_char_boundary(mapping.text_range.end)
            || mapping.source_range.start > mapping.source_range.end
    }) {
        return Err(TextError::InvalidParagraph);
    }
    if paragraph.bidi_scopes.iter().any(|scope| {
        scope.range.start >= scope.range.end
            || scope.range.end > paragraph.text.len()
            || !paragraph.text.is_char_boundary(scope.range.start)
            || !paragraph.text.is_char_boundary(scope.range.end)
            || scope.unicode_bidi == UnicodeBidi::Normal
    }) {
        return Err(TextError::InvalidParagraph);
    }
    Ok(())
}

fn validate_line_request(request: &LineRequest<'_>) -> Result<(), TextError> {
    validate_paragraph(&request.paragraph)?;
    if request.range.start >= request.range.end
        || request.range.end > request.paragraph.text.len()
        || !request.paragraph.text.is_char_boundary(request.range.start)
        || !request.paragraph.text.is_char_boundary(request.range.end)
    {
        return Err(TextError::InvalidParagraph);
    }
    Ok(())
}

fn fallback_advance(ch: char, size: f32) -> f32 {
    let em = if is_default_ignorable(ch) {
        0.0
    } else if ch.is_ascii_alphanumeric() {
        0.55
    } else if ch.is_ascii_punctuation() || ch.is_whitespace() {
        0.30
    } else {
        1.0
    };
    em * size
}

fn fallback_glyph_orientation(
    writing_mode: WritingMode,
    text_orientation: TextOrientation,
    ch: char,
) -> GlyphOrientation {
    match writing_mode {
        WritingMode::HorizontalTb => GlyphOrientation::Upright,
        WritingMode::SidewaysRl => GlyphOrientation::SidewaysRight,
        WritingMode::SidewaysLr => GlyphOrientation::SidewaysLeft,
        WritingMode::VerticalRl | WritingMode::VerticalLr => match text_orientation {
            TextOrientation::Upright => GlyphOrientation::Upright,
            TextOrientation::Sideways => GlyphOrientation::SidewaysRight,
            TextOrientation::Mixed if fallback_mixed_upright(ch) => GlyphOrientation::Upright,
            TextOrientation::Mixed => GlyphOrientation::SidewaysRight,
        },
    }
}

fn fallback_mixed_upright(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x11ff
            | 0x2e80..=0x9fff
            | 0xac00..=0xd7ff
            | 0xf900..=0xfaff
            | 0x20000..=0x3ffff
    )
}

fn is_mandatory_break(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

fn is_default_ignorable(ch: char) -> bool {
    matches!(
        ch as u32,
        0x00AD | 0x061C | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF
    )
}

fn style_at_break<'a>(text: &str, spans: &'a [TextSpan], offset: usize) -> Option<&'a TextStyle> {
    let character_start = text[..offset].char_indices().next_back()?.0;
    spans
        .iter()
        .find(|span| span.range.contains(&character_start))
        .map(|span| &span.style)
}

fn white_space_wraps(white_space: WhiteSpace) -> bool {
    !matches!(white_space, WhiteSpace::NoWrap | WhiteSpace::Pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_shapes_utf8_clusters_and_rasterizes_the_returned_glyph() {
        let style = TextStyle::default();
        let spans = [TextSpan {
            range: 0..3,
            source: 42,
            style: style.clone(),
            boundary_before: ShapingBoundary::Continue,
        }];
        let text = FallbackTextSystem::new();
        let shaped = text
            .shape(Paragraph {
                text: "Aب",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(shaped.runs.len(), 1);
        assert_eq!(shaped.runs[0].source, 42);
        assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..1);
        assert_eq!(shaped.runs[0].glyphs[1].cluster, 1..3);

        let glyph = &shaped.runs[0].glyphs[0];
        let key = GlyphCacheKey::new(
            text.generation(),
            shaped.runs[0].font,
            glyph.id,
            shaped.runs[0].font_size,
            0,
            0,
        )
        .unwrap();
        let image = text
            .rasterize(GlyphRasterRequest {
                key,
                variations: &[],
                synthesis: FontSynthesis::default(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(image.content, GlyphContent::Mask);
        assert_eq!(image.data.len(), (image.width * image.height) as usize);
    }

    #[test]
    fn vertical_fallback_preserves_the_inline_axis_advance() {
        let style = TextStyle {
            writing_mode: WritingMode::VerticalRl,
            text_orientation: TextOrientation::Upright,
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..1,
            source: 1,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];
        let shaped = FallbackTextSystem::new()
            .shape(Paragraph {
                text: "A",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap();

        assert_eq!(shaped.runs[0].glyphs[0].advance_x, 0.0);
        assert!(shaped.runs[0].glyphs[0].advance_y > 0.0);
    }

    #[test]
    fn fallback_soft_hyphen_has_no_advance_until_selected() {
        assert_eq!(fallback_advance('\u{00ad}', 16.0), 0.0);
    }

    #[test]
    fn fallback_exposes_a_centered_synthesized_vertical_baseline() {
        let metrics = FallbackTextSystem::new()
            .metrics(&TextStyle {
                size: 16.0,
                writing_mode: WritingMode::VerticalRl,
                ..TextStyle::default()
            })
            .unwrap();

        assert_eq!(metrics.vertical_ascent, 8.0);
        assert_eq!(metrics.vertical_descent, 8.0);
        assert_eq!(metrics.vertical_leading, 0.0);
        assert!(!metrics.has_canonical_vertical_metrics);
    }

    #[test]
    fn font_matching_policy_prefers_attribute_distance_inside_one_family() {
        let request = FontAttributes {
            weight: 700,
            stretch: 100,
            slant: FontSlant::Italic,
        };
        assert!(
            font_match_score(
                request,
                FontAttributes {
                    weight: 700,
                    stretch: 100,
                    slant: FontSlant::Italic,
                }
            ) < font_match_score(
                request,
                FontAttributes {
                    weight: 400,
                    stretch: 75,
                    slant: FontSlant::Normal,
                }
            )
        );
    }

    #[test]
    fn font_matching_uses_width_style_then_css_weight_direction() {
        let request = FontAttributes {
            weight: 400,
            stretch: 100,
            slant: FontSlant::Italic,
        };
        let face = |weight, stretch, slant| FontAttributes {
            weight,
            stretch,
            slant,
        };
        assert!(
            font_match_rank(request, face(900, 100, FontSlant::Italic))
                < font_match_rank(request, face(400, 90, FontSlant::Italic))
        );
        assert!(
            font_match_rank(request, face(900, 100, FontSlant::Italic))
                < font_match_rank(request, face(400, 100, FontSlant::Normal))
        );
        assert!(
            font_match_rank(request, face(500, 100, FontSlant::Italic))
                < font_match_rank(request, face(300, 100, FontSlant::Italic))
        );
        assert!(
            font_match_rank(request, face(300, 100, FontSlant::Italic))
                < font_match_rank(request, face(600, 100, FontSlant::Italic))
        );
    }

    #[test]
    fn synthesis_only_adds_missing_weight_and_slant() {
        let face = |weight, slant| FontAttributes {
            weight,
            stretch: 100,
            slant,
        };
        let auto = FontSynthesis {
            weight: true,
            style: true,
        };
        assert_eq!(
            resolve_synthesis(
                face(700, FontSlant::Italic),
                face(400, FontSlant::Normal),
                auto
            ),
            auto
        );
        assert_eq!(
            resolve_synthesis(
                face(700, FontSlant::Italic),
                face(700, FontSlant::Oblique),
                auto
            ),
            FontSynthesis {
                weight: false,
                style: false
            }
        );
        assert_eq!(
            resolve_synthesis(
                face(400, FontSlant::Normal),
                face(400, FontSlant::Italic),
                auto
            ),
            FontSynthesis {
                weight: false,
                style: false
            }
        );
        assert_eq!(
            resolve_synthesis(
                face(700, FontSlant::Italic),
                face(400, FontSlant::Normal),
                FontSynthesis {
                    weight: false,
                    style: false
                }
            ),
            FontSynthesis {
                weight: false,
                style: false
            }
        );
    }

    #[test]
    fn variation_and_synthesis_state_participate_in_the_cache_key() {
        let base = GlyphCacheKey::new(1, FontKey(2), GlyphId(3), 16.0, 0, 0).unwrap();
        let varied = base.with_style(
            &[VariationSetting {
                tag: *b"wght",
                value: 700.0,
            }],
            FontSynthesis {
                weight: true,
                style: false,
            },
        );

        assert_ne!(base, varied);
    }

    #[test]
    fn non_finite_variation_values_are_rejected_at_the_interface() {
        let style = TextStyle {
            variations: vec![VariationSetting {
                tag: *b"wght",
                value: f32::NAN,
            }],
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..1,
            source: 1,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];

        assert_eq!(
            FallbackTextSystem::new().shape(Paragraph {
                text: "A",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            }),
            Err(TextError::InvalidStyle)
        );
    }
}
