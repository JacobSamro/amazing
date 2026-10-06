//! CSS Grid 3 §4.2: the complete flow-tolerance computed-value producer.
//! Grid-axis percentage resolution and normal/infinite placement behavior
//! remain with layout, which consumes this property through ComputedStyle.
use super::*;

pub(crate) fn parse(css: &str) -> Option<ComputedValue> {
    if css_wide_keyword(css).is_some() {
        return Some(ComputedValue::Unparsed);
    }
    let tokens = tokenize(css);
    let tokens = trim_tokens(&tokens);
    if matches!(tokens, [Token::Ident(word)] if word.eq_ignore_ascii_case("normal") || word.eq_ignore_ascii_case("infinite") || css_wide_keyword(word).is_some())
    {
        return Some(ComputedValue::Unparsed);
    }
    if let Some(length) = values::LengthPercentage::parse(css) {
        return length
            .is_nonnegative()
            .then_some(ComputedValue::LengthPercentage(length));
    }
    crate::math::length_percentage(css, true)
        .then(|| ComputedValue::LengthPercentageExpression(serialize_computed(tokens)))
}

pub(super) fn resolve(properties: &mut BTreeMap<String, String>, root: f64) {
    let Some(css) = properties.get("flow-tolerance").cloned() else {
        return;
    };
    let font = properties
        .get("font-size")
        .and_then(|s| s.strip_suffix("px"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(16.0);
    let computed = match parse(&css) {
        Some(ComputedValue::Unparsed) => serialize_computed(&tokenize(&css)).to_ascii_lowercase(),
        Some(ComputedValue::LengthPercentage(length)) => {
            let mut value = ComputedValue::LengthPercentage(length);
            values::resolve_font_units(&mut value, font, root, false);
            match value {
                ComputedValue::LengthPercentage(values::LengthPercentage::Px(px)) => {
                    format!("{}px", px.max(0.0))
                }
                _ => serialize_computed(&tokenize(&css)),
            }
        }
        Some(ComputedValue::LengthPercentageExpression(canonical)) => {
            if crate::math::percentage(&css) {
                crate::math::font_size_px(&css, 100.0, f64::NAN)
                    .map(|percentage| format!("{}%", percentage.max(0.0)))
                    .unwrap_or(canonical)
            } else if tokenize(&css)
                .iter()
                .any(|token| matches!(token, Token::Percentage(_)))
            {
                // The grid-axis box is unavailable at computed-value time.
                canonical
            } else {
                crate::math::font_size_px(&css, font, root)
                    .map(|px| format!("{}px", px.max(0.0)))
                    .unwrap_or(canonical)
            }
        }
        _ => "normal".to_owned(),
    };
    properties.insert("flow-tolerance".to_owned(), computed);
}
