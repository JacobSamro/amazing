//! UTR #51 / CSS Fonts' font-variant-emoji: a trailing U+FE0E/U+FE0F
//! variation selector forces a specific glyph presentation for the
//! preceding character regardless of the font's own default. Split out of
//! `lib.rs` (TEXT-27) alongside `font_selection`, which is the module that
//! actually applies these constraints during font matching.

use super::FontRecord;
use harfrust::FontRef as HarfFontRef;
use read_fonts::TableProvider;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Unicode emoji variation selectors (UTR #51 / CSS Fonts'
/// font-variant-emoji) force a specific presentation for the preceding
/// character regardless of the font's own default: U+FE0E requests the
/// plain "text" glyph, U+FE0F requests the colourful "emoji" glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EmojiPresentation {
    Text,
    Emoji,
}

pub(super) fn emoji_presentation_intent(text: &str) -> Option<EmojiPresentation> {
    match text.chars().next_back()? {
        '\u{FE0E}' => Some(EmojiPresentation::Text),
        '\u{FE0F}' => Some(EmojiPresentation::Emoji),
        _ => None,
    }
}

/// Whether `font` is an acceptable choice under a forced presentation: a
/// forced emoji presentation requires a colour-capable face, a forced text
/// presentation requires a monochrome-only one. No constraint when the text
/// carries no presentation selector.
pub(super) fn font_matches_presentation(
    font: &FontRecord,
    presentation: Option<EmojiPresentation>,
) -> bool {
    match presentation {
        None => true,
        Some(EmojiPresentation::Emoji) => has_color_glyph_tables(&font.sfnt),
        Some(EmojiPresentation::Text) => !has_color_glyph_tables(&font.sfnt),
    }
}

/// Whether this font provides colour glyphs via any of OpenType's colour
/// tables (COLR, CBDT, sbix, or SVG), rather than only monochrome outlines.
pub(super) fn has_color_glyph_tables(sfnt: &[u8]) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        HarfFontRef::from_index(sfnt, 0).is_ok_and(|face| {
            face.colr().is_ok() || face.cbdt().is_ok() || face.sbix().is_ok() || face.svg().is_ok()
        })
    }))
    .unwrap_or(false)
}
