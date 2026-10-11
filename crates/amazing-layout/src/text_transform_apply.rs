//! Applies the computed `text-transform` to prepared (white-space processed) text.
//! Only rendered text changes: mappings keep pointing at the original DOM characters.
use crate::TextSourceMapping;
use amazing_css::ComputedStyle;
use amazing_text::{TextTransform, TextTransformCase};
use amazing_text::map_text_transform as apply_text_transform;

/// Parses the `text-transform` keyword set (`none | [capitalize | uppercase | lowercase]
/// || full-width || full-size-kana`).
pub(crate) fn parse_text_transform(value: &str) -> TextTransform {
    let mut transform = TextTransform::default();
    for word in value.split_ascii_whitespace() {
        match word.to_ascii_lowercase().as_str() {
            "capitalize" => transform.case = TextTransformCase::Capitalize,
            "uppercase" => transform.case = TextTransformCase::Uppercase,
            "lowercase" => transform.case = TextTransformCase::Lowercase,
            "full-width" => transform.full_width = true,
            "full-size-kana" => transform.full_size_kana = true,
            _ => {}
        }
    }
    transform
}

pub(crate) fn transform_prepared_text(
    text: String,
    mappings: Vec<TextSourceMapping>,
    style: &ComputedStyle,
) -> (String, Vec<TextSourceMapping>) {
    let transform = parse_text_transform(style.get("text-transform").unwrap_or("none"));
    if transform == TextTransform::default() || text.is_empty() {
        return (text, mappings);
    }
    let whole = apply_text_transform(&text, transform);
    // Fast path: one output scalar per input scalar keeps every mapping 1:1.
    let aligned = whole.chars().count() == text.chars().count();
    let mut out = String::with_capacity(whole.len());
    let mut remapped = Vec::with_capacity(mappings.len());
    if aligned {
        // Old byte offset -> new byte offset for every scalar boundary.
        let mut offsets = std::collections::HashMap::with_capacity(text.len() + 1);
        let mut old_offset = 0usize;
        for (new, old) in whole.chars().zip(text.chars()) {
            offsets.insert(old_offset, out.len());
            out.push(new);
            old_offset += old.len_utf8();
        }
        offsets.insert(old_offset, out.len());
        for mut mapping in mappings {
            let (Some(&start), Some(&end)) =
                (offsets.get(&mapping.text_range.start), offsets.get(&mapping.text_range.end))
            else {
                continue;
            };
            mapping.text_range = start..end;
            remapped.push(mapping);
        }
        return (out, remapped);
    }
    // Length-changing mappings (e.g. sharp s): transform per mapping.
    for mapping in mappings {
        let piece = &text[mapping.text_range.clone()];
        let start = out.len();
        out.push_str(&apply_text_transform(piece, transform));
        remapped.push(TextSourceMapping { text_range: start..out.len(), ..mapping });
    }
    (out, remapped)
}
