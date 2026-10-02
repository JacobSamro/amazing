//! CSS Fonts 4 font-variant shorthand: seven independently cascading axes.
use crate::syntax::{Token, serialize as serialize_tokens, tokenize};
use std::collections::BTreeMap;
pub(crate) const AXES: [&str; 7] = [
    "font-variant-ligatures",
    "font-variant-caps",
    "font-variant-alternates",
    "font-variant-numeric",
    "font-variant-east-asian",
    "font-variant-position",
    "font-variant-emoji",
];
const NO_LIGATURES: &str =
    "no-common-ligatures no-discretionary-ligatures no-historical-ligatures no-contextual";

pub(crate) fn expand(value: &str) -> Option<Vec<(String, String)>> {
    let tokens = tokenize(value);
    let significant: Vec<_> = tokens
        .into_iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    if let [Token::Ident(keyword)] = significant.as_slice() {
        let keyword = keyword.to_ascii_lowercase();
        if matches!(
            keyword.as_str(),
            "normal" | "none" | "initial" | "inherit" | "unset" | "revert" | "revert-layer"
        ) {
            return Some(
                AXES.iter()
                    .enumerate()
                    .map(|(index, axis)| {
                        let value = match keyword.as_str() {
                            "none" if index == 0 => "none",
                            "none" => "normal",
                            _ => &keyword,
                        };
                        ((*axis).into(), value.into())
                    })
                    .collect(),
            );
        }
    }
    if significant.is_empty() {
        return None;
    }
    let mut selected: [Vec<Token>; 7] = std::array::from_fn(|_| Vec::new());
    let mut position = 0;
    while position < significant.len() {
        let start = position;
        let axis = match &significant[position] {
            Token::Ident(word) => match word.to_ascii_lowercase().as_str() {
                "common-ligatures"
                | "no-common-ligatures"
                | "discretionary-ligatures"
                | "no-discretionary-ligatures"
                | "historical-ligatures"
                | "no-historical-ligatures"
                | "contextual"
                | "no-contextual" => 0,
                "small-caps" | "all-small-caps" | "petite-caps" | "all-petite-caps" | "unicase"
                | "titling-caps" => 1,
                "historical-forms" => 2,
                "lining-nums" | "oldstyle-nums" | "proportional-nums" | "tabular-nums"
                | "diagonal-fractions" | "stacked-fractions" | "ordinal" | "slashed-zero" => 3,
                "jis78" | "jis83" | "jis90" | "jis04" | "simplified" | "traditional"
                | "full-width" | "proportional-width" | "ruby" => 4,
                "sub" | "super" => 5,
                "text" | "emoji" | "unicode" => 6,
                _ => return None,
            },
            Token::Function(_) => {
                position += 1;
                if !matches!(significant.get(position), Some(Token::OpenParen)) {
                    return None;
                }
                position += 1;
                while position < significant.len()
                    && !matches!(significant[position], Token::CloseParen)
                {
                    if !matches!(significant[position], Token::Ident(_) | Token::Comma) {
                        return None;
                    }
                    position += 1;
                }
                if position == significant.len() {
                    return None;
                }
                2
            }
            _ => return None,
        };
        position += 1;
        if !selected[axis].is_empty() {
            selected[axis].push(Token::Whitespace);
        }
        selected[axis].extend_from_slice(&significant[start..position]);
    }
    AXES.iter()
        .enumerate()
        .map(|(index, axis)| {
            let css = if selected[index].is_empty() {
                "normal".into()
            } else {
                serialize_tokens(&selected[index])
            };
            let canonical = if index == 0 {
                crate::computed::canonical_font_feature_value(axis, &css)
            } else {
                crate::font_properties::canonical(axis, &css)
            }?;
            Some(((*axis).into(), canonical))
        })
        .collect()
}

pub(crate) fn shorthand(properties: &BTreeMap<String, String>) -> String {
    let values: Vec<_> = AXES
        .iter()
        .map(|axis| {
            properties
                .get(*axis)
                .map(String::as_str)
                .unwrap_or("normal")
        })
        .collect();
    if values[0] == "none" && values[1..].iter().all(|v| *v == "normal") {
        return "none".into();
    }
    let parts: Vec<_> = values
        .into_iter()
        .filter(|value| *value != "normal")
        .map(|value| if value == "none" { NO_LIGATURES } else { value })
        .collect();
    if parts.is_empty() {
        "normal".into()
    } else {
        parts.join(" ")
    }
}
pub(crate) fn serialize(properties: &mut BTreeMap<String, String>) {
    properties.insert("font-variant".into(), shorthand(properties));
}
