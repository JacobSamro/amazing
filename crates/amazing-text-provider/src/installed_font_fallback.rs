//! Genuine installed fallback and immutable registered-key introspection.
//! CSS Fonts4 allows content-language-aware composite generic families. Native
//! OS/2 range declarations influence preference, never actual glyph coverage.
use super::emoji_presentation::{EmojiPresentation, font_matches_presentation};
use super::font_selection::{font_covers, font_covers_space};
use super::{FontRecord, InstalledFontProfile, TextProvider, font_matching};
use amazing_text::{FontKey, TextStyle};

/// Borrowed identity from this exact provider, not a rematch or a global key map.
/// FontKey(0) is the deterministic missing-glyph renderer and has no native face.
#[derive(Clone, Copy, Debug)]
pub struct RegisteredFontFace<'a> {
    pub key: FontKey,
    pub family: &'a str,
    pub native: Option<&'a InstalledFontProfile>,
    pub author_descriptors: bool,
}
impl TextProvider {
    pub fn registered_font_face(&self, key: FontKey) -> Option<RegisteredFontFace<'_>> {
        let font = self.fonts.iter().find(|font| font.key == key)?;
        Some(RegisteredFontFace {
            key,
            family: &font.family,
            native: font.installed_face.as_deref(),
            author_descriptors: font.author_face.is_some(),
        })
    }
    pub(super) fn matching_native_fallback(
        &self,
        style: &TextStyle,
        text: Option<&str>,
        require_space: bool,
        presentation: Option<EmojiPresentation>,
        generic: Option<&str>,
        japanese_only: bool,
    ) -> Option<usize> {
        // CSS Text 3 white-space processing: control characters must stay visible.
        // Installed fonts often map them to blank glyphs, so only the deterministic
        // missing-glyph renderer may draw them.
        if text.is_some_and(|text| text.chars().any(|c| c.is_control() && !c.is_whitespace())) {
            return None;
        }
        self.fonts
            .iter()
            .enumerate()
            .filter(|(_, font)| {
                let Some(native) = &font.installed_face else {
                    return false;
                };
                font.author_face.is_none()
                    && self.font_display_available(font.key)
                    && (!japanese_only || native.declares_japanese_support())
                    && text.is_none_or(|text| font_covers(font, text))
                    && (!require_space || font_covers_space(font))
                    && font_matches_presentation(font, presentation)
            })
            .min_by_key(|(_, font)| {
                (
                    generic_penalty(font, generic),
                    font_matching::rank(style, font),
                )
            })
            .map(|(index, _)| index)
    }
}
pub(super) fn japanese_language(language: &str) -> bool {
    let mut parts = language.split('-').peekable();
    if !parts
        .next()
        .is_some_and(|primary| primary.eq_ignore_ascii_case("ja"))
    {
        return false;
    }
    while let Some(part) = parts.next() {
        if part.is_empty()
            || part.len() > 8
            || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            || (part.len() == 1 && parts.peek().is_none())
        {
            return false;
        }
    }
    true
}
impl InstalledFontProfile {
    /// Native OS/2 declarations only. Matching still requires actual cmap coverage.
    pub fn declares_japanese_support(&self) -> bool {
        self.code_page_ranges
            .is_some_and(|ranges| ranges[0] & (1 << 17) != 0)
            || self.unicode_ranges[1] & ((1 << 17) | (1 << 18)) == ((1 << 17) | (1 << 18))
    }
    pub fn declares_cjk_support(&self) -> bool {
        self.code_page_ranges
            .is_some_and(|ranges| ranges[0] & (0b11111 << 17) != 0)
            || self.unicode_ranges[1] & ((1 << 17) | (1 << 18) | (1 << 24) | (1 << 27)) != 0
    }
}
fn generic_penalty(font: &FontRecord, generic: Option<&str>) -> u8 {
    let Some(profile) = &font.installed_face else {
        return 2;
    };
    match generic {
        Some("monospace" | "ui-monospace") => u8::from(!profile.fixed_pitch),
        Some("serif" | "ui-serif") => match profile.serif {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        },
        Some("sans-serif" | "system-ui" | "ui-sans-serif" | "ui-rounded") => match profile.serif {
            Some(false) => 0,
            None => 1,
            Some(true) => 2,
        },
        _ => 0,
    }
}
