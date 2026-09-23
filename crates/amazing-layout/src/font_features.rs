//! Converts computed CSS font feature controls to shaping features.

use amazing_css::ComputedStyle;
use amazing_text::FeatureSetting;

pub(crate) fn from_style(style: &ComputedStyle) -> Vec<FeatureSetting> {
    let mut features = Vec::new();
    if let Some(variant) = style.get("font-variant-ligatures") {
        let words = variant.split_ascii_whitespace().collect::<Vec<_>>();
        if words.iter().any(|word| word.eq_ignore_ascii_case("none")) {
            // Required ligatures (`rlig`) must remain enabled for complex scripts.
            for tag in [*b"liga", *b"clig", *b"dlig", *b"hlig", *b"calt"] {
                set(&mut features, tag, 0);
            }
        } else {
            for word in words {
                let group: &[([u8; 4], u32)] = match word.to_ascii_lowercase().as_str() {
                    "common-ligatures" => &[( *b"liga", 1), (*b"clig", 1)],
                    "no-common-ligatures" => &[( *b"liga", 0), (*b"clig", 0)],
                    "discretionary-ligatures" => &[(*b"dlig", 1)],
                    "no-discretionary-ligatures" => &[(*b"dlig", 0)],
                    "historical-ligatures" => &[(*b"hlig", 1)],
                    "no-historical-ligatures" => &[(*b"hlig", 0)],
                    "contextual" => &[(*b"calt", 1)],
                    "no-contextual" => &[(*b"calt", 0)],
                    _ => &[],
                };
                for &(tag, value) in group {
                    set(&mut features, tag, value);
                }
            }
        }
    }
    if let Some(settings) = style.get("font-feature-settings") {
        for entry in settings.split(',') {
            let entry = entry.trim();
            let Some(quote) = entry.as_bytes().first().copied() else {
                continue;
            };
            if quote != b'\'' && quote != b'"' {
                continue;
            }
            let Some(end) = entry[1..].find(quote as char).map(|offset| offset + 1) else {
                continue;
            };
            let tag = &entry.as_bytes()[1..end];
            if tag.len() != 4 || !tag.iter().all(u8::is_ascii_graphic) {
                continue;
            }
            let value = match entry[end + 1..].trim().to_ascii_lowercase().as_str() {
                "" | "on" => Some(1),
                "off" => Some(0),
                number => number.parse::<u32>().ok(),
            };
            if let Some(value) = value {
                set(&mut features, tag.try_into().expect("four-byte tag"), value);
            }
        }
    }
    features
}

fn set(features: &mut Vec<FeatureSetting>, tag: [u8; 4], value: u32) {
    if let Some(feature) = features.iter_mut().find(|feature| feature.tag == tag) {
        feature.value = value;
    } else {
        features.push(FeatureSetting { tag, value });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_none_preserves_required_ligatures_and_explicit_settings_win() {
        let mut style = ComputedStyle::default();
        style.set("font-variant-ligatures", "none");
        style.set("font-feature-settings", "'liga' on, \"rlig\" off");
        let features = from_style(&style);
        assert_eq!(features.iter().find(|f| &f.tag == b"liga").unwrap().value, 1);
        assert_eq!(features.iter().find(|f| &f.tag == b"rlig").unwrap().value, 0);
        assert_eq!(features.iter().find(|f| &f.tag == b"clig").unwrap().value, 0);
    }
}
