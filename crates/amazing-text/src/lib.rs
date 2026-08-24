//! Provider-neutral text interface shared by layout and paint.
//!
//! The interface accepts paragraph text with shaping-affecting style spans and
//! returns positioned glyph clusters. Layout uses those advances; paint asks
//! the same adapter to rasterize the returned font/glyph ids. Provider types
//! never cross this seam.

#![forbid(unsafe_code)]

use std::ops::Range;

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

pub fn resolve_synthesis(
    request: FontAttributes,
    candidate: FontAttributes,
    policy: FontSynthesis,
) -> FontSynthesis {
    FontSynthesis {
        weight: policy.weight && request.weight > candidate.weight,
        style: policy.style && request.slant != candidate.slant,
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
    pub slant: FontSlant,
    pub direction: Direction,
    pub writing_mode: WritingMode,
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
            slant: FontSlant::Normal,
            direction: Direction::LeftToRight,
            writing_mode: WritingMode::HorizontalTb,
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

pub struct Paragraph<'a> {
    pub text: &'a str,
    pub spans: &'a [TextSpan],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub x_height: f32,
    pub cap_height: f32,
}

impl FontMetrics {
    pub fn line_height(self) -> f32 {
        self.ascent + self.descent + self.leading
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlyphFlags {
    pub unsafe_to_break: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    pub id: GlyphId,
    pub cluster: Range<usize>,
    pub advance_x: f32,
    pub advance_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
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
        })
    }

    fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
        validate_paragraph(&paragraph)?;
        let mut shaped = ShapedParagraph::default();
        for span in paragraph.spans {
            let metrics = self.metrics(&span.style)?;
            let mut glyphs = paragraph.text[span.range.clone()]
                .char_indices()
                .map(|(relative_start, ch)| {
                    let cluster_start = span.range.start + relative_start;
                    let cluster_end = cluster_start + ch.len_utf8();
                    let advance = fallback_advance(ch, span.style.size);
                    let (advance_x, advance_y) = match span.style.writing_mode {
                        WritingMode::HorizontalTb => (advance, 0.0),
                        WritingMode::VerticalRl | WritingMode::VerticalLr => (0.0, advance),
                    };
                    ShapedGlyph {
                        id: GlyphId(ch as u32),
                        cluster: cluster_start..cluster_end,
                        advance_x,
                        advance_y,
                        offset_x: 0.0,
                        offset_y: 0.0,
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
            if ch.is_whitespace() {
                shaped.breaks.push(BreakOpportunity {
                    offset: start + ch.len_utf8(),
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
        Ok(shaped)
    }

    fn rasterize(&self, request: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError> {
        let key = request.key;
        if key.generation != self.generation() || key.font != FontKey(0) {
            return Err(TextError::MissingFont);
        }
        let Some(ch) = char::from_u32(key.glyph.0) else {
            return Err(TextError::InvalidParagraph);
        };
        if ch.is_whitespace() {
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

fn validate_paragraph(paragraph: &Paragraph<'_>) -> Result<(), TextError> {
    if paragraph.text.is_empty() {
        return if paragraph.spans.is_empty() {
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
    Ok(())
}

fn fallback_advance(ch: char, size: f32) -> f32 {
    let em = if ch.is_ascii_alphanumeric() {
        0.55
    } else if ch.is_ascii_punctuation() || ch.is_whitespace() {
        0.30
    } else {
        1.0
    };
    em * size
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
            })
            .unwrap();

        assert_eq!(shaped.runs[0].glyphs[0].advance_x, 0.0);
        assert!(shaped.runs[0].glyphs[0].advance_y > 0.0);
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
            }),
            Err(TextError::InvalidStyle)
        );
    }
}
