//! Font matching and fallback: given a style's declared `font-family` list
//! and the text a cluster needs to cover, decide which registered face (if
//! any) to shape it with. Covers per-family/per-face ranking (delegated to
//! `font_matching`), coverage checks against a font's charmap/unicode-range,
//! the driver's generic-family fallback chain, presentation-selector-aware
//! selection (`emoji_presentation`), and the per-cluster itemization that
//! walks a paragraph and assigns each grapheme cluster its font. Split out
//! of `lib.rs` (TEXT-27).

use super::emoji_presentation::{EmojiPresentation, emoji_presentation_intent, font_matches_presentation};
use super::{
    FontItemization, FontRecord, MAX_FONT_MATCH_OPERATIONS, MAX_ITEMIZED_SPANS, MAX_TEXT_CLUSTERS,
    TextProvider, font_matching,
};
use amazing_text::{ShapingBoundary, TextError, TextSpan, TextStyle};
use icu_segmenter::GraphemeClusterSegmenter;
use swash::FontRef as SwashFontRef;
use std::panic::{AssertUnwindSafe, catch_unwind};

impl TextProvider {
    pub(super) fn matching_font(&self, style: &TextStyle, text: Option<&str>) -> Option<usize> {
        self.matching_font_with_space_policy(style, text, false)
    }

    /// A trailing U+FE0E/U+FE0F variation selector forces a specific
    /// presentation regardless of the declared font's own default coverage
    /// (real browsers still switch to a colour-capable face even when the
    /// declared family technically maps the base code point), so this tries
    /// a presentation-constrained search first and only relaxes it -- same
    /// result as before this existed -- when no installed face satisfies the
    /// constraint at all, rather than ever rendering nothing.
    pub(super) fn matching_font_with_space_policy(
        &self,
        style: &TextStyle,
        text: Option<&str>,
        require_space: bool,
    ) -> Option<usize> {
        let presentation = text.and_then(emoji_presentation_intent);
        if presentation.is_some()
            && let Some(index) =
                self.matching_font_impl(style, text, require_space, presentation)
        {
            return Some(index);
        }
        self.matching_font_impl(style, text, require_space, None)
    }

    fn matching_font_impl(
        &self,
        style: &TextStyle,
        text: Option<&str>,
        require_space: bool,
        presentation: Option<EmojiPresentation>,
    ) -> Option<usize> {
        let private_use = text.is_some_and(|value| value.chars().any(is_private_use));
        for family in &style.families {
            if private_use && is_generic_family(family) {
                continue;
            }
            let family = unquote_family(family);
            let family = family.to_ascii_lowercase();
            let author_only = self.author_families.contains(&family);
            let mut best_rank = None;
            let mut candidates = Vec::new();
            for (index, font) in self.fonts.iter().enumerate() {
                if !self.font_display_available(font.key)
                    || !font.matches_family(&family)
                    || (author_only && font.author_face.is_none())
                {
                    continue;
                }
                let rank = font_matching::rank(style, font);
                if best_rank.is_none_or(|current| rank < current) {
                    best_rank = Some(rank);
                    candidates.clear();
                    candidates.push(index);
                } else if best_rank == Some(rank) {
                    candidates.push(index);
                }
            }
            if author_only {
                // Equal-descriptor faces form a composite face. Later rules
                // win for overlapping unicode ranges.
                candidates.sort_by_key(|index|self.fonts[*index].author_face.as_ref().map_or(0,|face|face.declaration_order));
                candidates.reverse();
            }
            for index in candidates {
                if text.is_none_or(|value| font_covers(&self.fonts[index], value))
                    && (!require_space || font_covers_space(&self.fonts[index]))
                    && font_matches_presentation(&self.fonts[index], presentation)
                {
                    return Some(index);
                }
            }
        }
        if private_use {
            return None;
        }
        // The driver registers its platform faces under these generic
        // aliases. Once the author list is exhausted, use an installed face
        // that covers the cluster before the deterministic missing-glyph
        // renderer. A forced emoji presentation (U+FE0F) searches the
        // dedicated "emoji" alias first, ahead of the plain-text generics, so
        // it prefers a colour-capable face over a monochrome one when both
        // are installed.
        let mut generics: Vec<&str> = Vec::new();
        if presentation == Some(EmojiPresentation::Emoji) {
            generics.push("emoji");
        }
        generics.extend(["serif", "sans-serif", "system-ui", "monospace"]);
        generics.iter().find_map(|family| {
            self.fonts
                .iter()
                .enumerate()
                .filter(|(_, font)| {
                    font.matches_family(family)
                        && font.author_face.is_none()
                        && text.is_none_or(|value| font_covers(font, value))
                        && (!require_space || font_covers_space(font))
                        && font_matches_presentation(font, presentation)
                })
                .min_by_key(|(_, font)| font_matching::rank(style, font))
                .map(|(index, _)| index)
        })
    }

    pub(super) fn itemize_font_spans(
        &self,
        text: &str,
        spans: &[TextSpan],
    ) -> Result<FontItemization, TextError> {
        let segmenter = GraphemeClusterSegmenter::new();
        let mut itemized = Vec::new();
        let mut selected = Vec::new();
        let mut visibility=Vec::new();
        let mut clusters = Vec::new();
        let mut previous_font = None;
        let mut previous_visibility=amazing_text::GlyphVisibility::Visible;
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
            let visible=self.font_display_visibility(&owner.style,&text[cluster.clone()],font)?;
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
                    && (previous_font != font || previous_visibility!=visible)
                {
                    boundary = ShapingBoundary::StyleChange;
                }
                itemized.push(TextSpan {
                    range,
                    source: span.source,
                    style: span.style.clone(),
                    boundary_before: boundary,
                });
                selected.push(font);
                visibility.push(visible);
                clusters.push(cluster.clone());
                previous_font = font;
                previous_visibility=visible;
                overlap_index += 1;
            }
        }
        Ok(FontItemization {
            spans: itemized,
            fonts: selected,
            clusters,
            visibility,
        })
    }
}

pub(super) fn unquote_family(family: &str) -> &str {
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

pub(super) fn is_generic_family(family: &str) -> bool {
    let family = family.trim();
    [
        "serif",
        "sans-serif",
        "monospace",
        "cursive",
        "fantasy",
        "system-ui",
        "ui-serif",
        "ui-sans-serif",
        "ui-monospace",
        "ui-rounded",
        "emoji",
        "math",
        "fangsong",
    ]
    .iter()
    .any(|generic| family.eq_ignore_ascii_case(generic))
}

pub(super) fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

pub(super) fn font_covers(font: &FontRecord, text: &str) -> bool {
    if let Some(face) = &font.author_face {
        if !face.unicode_ranges.is_empty()
            && !text.chars().all(|ch| {
                face.unicode_ranges
                    .iter()
                    .any(|range| range.contains(ch as u32))
            })
        {
            return false;
        }
    }
    catch_unwind(AssertUnwindSafe(|| {
        SwashFontRef::from_index(&font.sfnt, 0).is_some_and(|face| {
            let charmap = face.charmap();
            text.chars()
                .all(|ch| ch.is_whitespace() || is_default_ignorable(ch) || charmap.map(ch) != 0)
        })
    }))
    .unwrap_or(false)
}

/// CSS's first available font for metric units and the line strut must
/// actually contain U+0020. Ordinary shaping may keep an uncoded space with
/// its surrounding run, so it uses the separate `font_covers` policy above.
pub(super) fn font_covers_space(font: &FontRecord) -> bool {
    if let Some(face) = &font.author_face
        && !face.unicode_ranges.is_empty()
        && !face.unicode_ranges.iter().any(|range| range.contains(0x20))
    {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        SwashFontRef::from_index(&font.sfnt, 0).is_some_and(|face| face.charmap().map(' ') != 0)
    }))
    .unwrap_or(false)
}

pub(super) fn is_default_ignorable(ch: char) -> bool {
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
