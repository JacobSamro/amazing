//! CSS Text Module Level 3/4 `text-transform`.
//!
//! This is a pure string transform with no shaping or layout dependency, so
//! it is exposed as a free function rather than folded into `TextStyle`:
//! the caller (today, `amazing-layout`) resolves the CSS value itself from
//! its own computed style and calls `apply_text_transform` on a DOM text
//! run at the same point it already builds the paragraph's "transformed
//! text" (see `amazing_text::SourceMapping`'s doc comment, which already
//! distinguishes "the transformed paragraph text" from "the original DOM
//! text node") -- white-space collapsing happens at that same seam today.
//! Keeping this out of `TextStyle` avoids widening that struct's literal
//! constructors in a crate this lane does not own.

use amazing_text::{TextTransform, TextTransformCase};
use icu_segmenter::WordSegmenter;
use icu_segmenter::options::{WordBreakInvariantOptions, WordType};

/// Applies `transform` to `text`. Order matches major engines: the case
/// keyword first, then `full-width`, then `full-size-kana`, so e.g.
/// `uppercase full-width` uppercases ASCII letters and then maps the result
/// (including the uppercased letters) into the fullwidth block.
pub fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    if transform == TextTransform::default() {
        return text.to_string();
    }
    let mut out = match transform.case {
        TextTransformCase::None => text.to_string(),
        TextTransformCase::Uppercase => uppercase(text),
        TextTransformCase::Lowercase => lowercase(text),
        TextTransformCase::Capitalize => capitalize(text),
    };
    if transform.full_width {
        out = full_width(&out);
    }
    if transform.full_size_kana {
        out = full_size_kana(&out);
    }
    out
}

fn uppercase(text: &str) -> String {
    text.chars().flat_map(char::to_uppercase).collect()
}

/// CSS Text 3 section 2.4.1 calls out exactly one uppercase-to-lowercase
/// special case that must be honored regardless of `lang`: Greek capital
/// sigma (U+03A3) lowercases to final sigma (U+03C2, `ς`) when it is the
/// last cased letter of a word, and to regular sigma (U+03C3, `σ`)
/// otherwise. `char::to_lowercase` is context-free and always yields `σ`,
/// so sigma is special-cased per word using the same word segmentation
/// `capitalize` uses.
fn lowercase(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = 0usize;
    for (end, word_type) in word_boundaries(text) {
        let word = &text[start..end];
        if word_type == WordType::Letter {
            out.push_str(&lowercase_word(word));
        } else {
            out.extend(word.chars().flat_map(char::to_lowercase));
        }
        start = end;
    }
    out
}

fn lowercase_word(word: &str) -> String {
    let last_cased = word
        .char_indices()
        .rev()
        .find(|(_, ch)| is_cased(*ch))
        .map(|(i, _)| i);
    let mut out = String::with_capacity(word.len());
    for (i, ch) in word.char_indices() {
        if ch == '\u{03A3}' {
            // GREEK CAPITAL LETTER SIGMA
            out.push(if Some(i) == last_cased {
                '\u{03C2}' // GREEK SMALL LETTER FINAL SIGMA
            } else {
                '\u{03C3}' // GREEK SMALL LETTER SIGMA
            });
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// Approximates Unicode's "Cased" derived property (no such query exists in
/// `std`): alphabetic characters are cased for every script this needs to
/// disambiguate Greek sigma against (Latin/Greek/Cyrillic text mixed with
/// punctuation or digits).
fn is_cased(ch: char) -> bool {
    ch.is_alphabetic()
}

/// Puts the first typographic letter unit of each word in titlecase (here,
/// uppercase -- `char::to_uppercase` differs from true titlecase only for a
/// handful of digraphs this codebase does not otherwise special-case) and
/// leaves the rest of the word unchanged. Word boundaries come from
/// `icu_segmenter`'s default (language-invariant) word segmenter, which
/// already keeps internal apostrophes inside a word (`don't` -> `Don't`,
/// not `Don'T`).
fn capitalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = 0usize;
    for (end, word_type) in word_boundaries(text) {
        let word = &text[start..end];
        if word_type == WordType::Letter {
            out.push_str(&capitalize_word(word));
        } else {
            out.push_str(word);
        }
        start = end;
    }
    out
}

fn capitalize_word(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut done = false;
    for ch in word.chars() {
        if !done && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            done = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn word_boundaries(text: &str) -> impl Iterator<Item = (usize, WordType)> + '_ {
    WordSegmenter::new_auto(WordBreakInvariantOptions::default())
        .segment_str(text)
        .iter_with_word_type()
}

/// CSS Text 3's `full-width`: maps ASCII space and the ASCII printable range
/// U+0021-U+007E to their fullwidth forms (U+3000, U+FF01-U+FF5E). Other
/// characters, including anything already outside ASCII, pass through
/// unchanged, matching the property's own scope ("all Basic Latin and Latin
/// Supplement characters ... other than those already mapped").
fn full_width(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            ' ' => '\u{3000}',
            '\u{21}'..='\u{7e}' => {
                char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch)
            }
            _ => ch,
        })
        .collect()
}

fn full_size_kana(text: &str) -> String {
    amazing_text::map_text_transform(
        text,
        TextTransform { full_size_kana: true, ..TextTransform::default() },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_text::{TextTransform, TextTransformCase};

    fn case(case: TextTransformCase) -> TextTransform {
        TextTransform {
            case,
            full_width: false,
            full_size_kana: false,
        }
    }

    #[test]
    fn full_size_kana_maps_small_kana_only() {
        let transform = TextTransform { case: TextTransformCase::None, full_width: false, full_size_kana: true };
        assert_eq!(apply_text_transform("ぁっゕゖァッㇰｯあA", transform), "あつかけアツクﾂあA");
    }

    #[test]
    fn none_transform_returns_the_input_unchanged() {
        assert_eq!(apply_text_transform("Hello World", TextTransform::default()), "Hello World");
    }

    #[test]
    fn uppercase_maps_every_cased_letter() {
        assert_eq!(
            apply_text_transform("Straße hello", case(TextTransformCase::Uppercase)),
            "STRASSE HELLO"
        );
    }

    #[test]
    fn lowercase_maps_every_cased_letter() {
        assert_eq!(
            apply_text_transform("HELLO WORLD", case(TextTransformCase::Lowercase)),
            "hello world"
        );
    }

    #[test]
    fn lowercase_uses_final_sigma_only_at_the_end_of_a_word() {
        // "ΟΔΥΣΣΕΥΣ" (Odysseus): the two interior sigmas stay plain sigma,
        // the trailing one becomes final sigma. Built from char literals
        // (rather than a pasted literal) so the expected value cannot carry
        // a transcription mistake.
        let input: String = ['\u{39F}', '\u{394}', '\u{3A5}', '\u{3A3}', '\u{3A3}', '\u{395}', '\u{3A5}', '\u{3A3}']
            .into_iter()
            .collect();
        let expected: String = ['\u{3BF}', '\u{3B4}', '\u{3C5}', '\u{3C3}', '\u{3C3}', '\u{3B5}', '\u{3C5}', '\u{3C2}']
            .into_iter()
            .collect();
        assert_eq!(apply_text_transform(&input, case(TextTransformCase::Lowercase)), expected);
    }

    #[test]
    fn lowercase_final_sigma_direct() {
        assert_eq!(
            apply_text_transform("ΣΣ", case(TextTransformCase::Lowercase)),
            "σς"
        );
        assert_eq!(
            apply_text_transform("Σ", case(TextTransformCase::Lowercase)),
            "ς"
        );
    }

    #[test]
    fn capitalize_titlecases_the_first_letter_of_each_word() {
        assert_eq!(
            apply_text_transform("hello world", case(TextTransformCase::Capitalize)),
            "Hello World"
        );
    }

    #[test]
    fn capitalize_keeps_an_internal_apostrophe_inside_the_word() {
        assert_eq!(
            apply_text_transform("don't stop", case(TextTransformCase::Capitalize)),
            "Don't Stop"
        );
    }

    #[test]
    fn capitalize_skips_leading_punctuation_within_a_word_segment() {
        assert_eq!(
            apply_text_transform("\"hello\"", case(TextTransformCase::Capitalize)),
            "\"Hello\""
        );
    }

    #[test]
    fn full_width_maps_ascii_printable_and_space() {
        assert_eq!(
            apply_text_transform(
                "AZ az 09",
                TextTransform {
                    case: TextTransformCase::None,
                    full_width: true,
                    full_size_kana: false,
                }
            ),
            "\u{FF21}\u{FF3A}\u{3000}\u{FF41}\u{FF5A}\u{3000}\u{FF10}\u{FF19}"
        );
    }

    #[test]
    fn combined_case_and_full_width_apply_case_first() {
        assert_eq!(
            apply_text_transform(
                "ab",
                TextTransform {
                    case: TextTransformCase::Uppercase,
                    full_width: true,
                    full_size_kana: false,
                }
            ),
            "\u{FF21}\u{FF22}"
        );
    }
}
