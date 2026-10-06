//! Flex factor/basis grammar and computed shorthand over winning longhands.
//! Existing number/length math owns dimensions; used flex layout stays in Layout.
use std::collections::BTreeMap;
use crate::{syntax::{Token, tokenize, serialize}, values::{self, ComputedValue, LengthPercentage}};

pub(crate) const LONGHANDS: [&str; 3] = ["flex-grow", "flex-shrink", "flex-basis"];

fn components(css: &str) -> Option<Vec<String>> {
    let tokens = tokenize(css);
    let mut result = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], Token::Whitespace) { i += 1; continue; }
        let start = i;
        match &tokens[i] {
            Token::Function(_) => {
                i += 1;
                if !matches!(tokens.get(i), Some(Token::OpenParen)) { return None; }
                let mut depth = 0usize;
                while let Some(token) = tokens.get(i) {
                    match token {
                        Token::OpenParen => depth += 1,
                        Token::CloseParen => depth = depth.checked_sub(1)?,
                        _ => {},
                    }
                    i += 1;
                    if depth == 0 { break; }
                }
                if depth != 0 { return None; }
            }
            Token::Ident(_) | Token::Number(_) | Token::Percentage(_) | Token::Dimension(_, _) => i += 1,
            _ => return None,
        }
        let text = match &tokens[start..i] {
            [Token::Ident(name)] => name.to_ascii_lowercase(),
            other => serialize(other),
        };
        result.push(text);
        if result.len() > 3 { return None; }
    }
    Some(result)
}

/// Preserve the existing raw/Unparsed nonfinite policy for Layout's consumer.
pub(crate) fn constant(css: &str) -> Option<&'static str> {
    let tokens: Vec<_> = tokenize(css).into_iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    match tokens.as_slice() {
        [Token::Function(function), Token::OpenParen, Token::Ident(name), Token::CloseParen]
            if function.eq_ignore_ascii_case("calc") => match name.to_ascii_lowercase().as_str() {
                "infinity" => Some("calc(infinity)"),
                "-infinity" => Some("calc(-infinity)"),
                "nan" => Some("calc(nan)"),
                _ => None,
            },
        _ => None,
    }
}

pub(crate) fn factor(css: &str) -> bool {
    let tokens: Vec<_> = tokenize(css).into_iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    match tokens.as_slice() {
        [Token::Number(n)] => n.is_finite() && *n >= 0.0,
        _ => matches!(tokens.first(), Some(Token::Function(_)))
            && (constant(css).is_some() || crate::math::number_percentage(css, false)),
    }
}
fn basis(css: &str) -> bool { values::parse("flex-basis", css).is_some() }

pub(crate) fn parse(css: &str) -> Option<[String; 3]> {
    let parts = components(css)?;
    let text: Vec<_> = parts.iter().map(String::as_str).collect();
    let result: [&str; 3] = match text.as_slice() {
        ["none"] => ["0", "0", "auto"],
        ["auto"] => ["1", "1", "auto"],
        [grow] if factor(grow) => [*grow, "1", "0%"],
        [base] if basis(base) => ["1", "1", *base],
        [grow, shrink] if factor(grow) && factor(shrink) => [*grow, *shrink, "0%"],
        [grow, base] if factor(grow) && basis(base) => [*grow, "1", *base],
        [base, grow] if basis(base) && factor(grow) => [*grow, "1", *base],
        [grow, shrink, base] if factor(grow) && factor(shrink) && basis(base) => [*grow, *shrink, *base],
        [base, grow, shrink] if basis(base) && factor(grow) && factor(shrink) => [*grow, *shrink, *base],
        _ => return None,
    };
    Some(result.map(str::to_owned))
}

pub(crate) fn expand(css: &str) -> Option<Vec<(String, String)>> {
    let values = parse(css)?;
    Some(LONGHANDS.into_iter().zip(values).map(|(name, value)| (name.into(), value)).collect())
}
fn number(n: f64) -> String { if n == 0.0 { "0".into() } else { n.to_string() } }

/// Font-dependent bases compute from the installed style context. Percentages
/// and box-dependent expressions remain symbolic for their actual consumer.
pub(crate) fn resolve(props: &mut BTreeMap<String, String>, font: f64, root: f64) {
    for name in &LONGHANDS[..2] {
        if let Some(css) = props.get(*name).cloned() {
            let value = if let Some(constant) = constant(&css) { constant.to_owned() }
                else { crate::math::computed_number(&css).filter(|n| n.is_finite())
                    .map(|n| number(n.max(0.0))).unwrap_or(css) };
            props.insert((*name).into(), value);
        }
    }
    if let Some(css) = props.get("flex-basis").cloned() {
        let mut typed = values::parse("flex-basis", &css);
        if let Some(value) = &mut typed { values::resolve_font_units(value, font, root, false); }
        let computed = match typed {
            Some(ComputedValue::LengthPercentage(LengthPercentage::Px(n))) => Some(format!("{}px", number(n.max(0.0)))),
            Some(ComputedValue::LengthPercentage(LengthPercentage::Percentage(n))) => Some(format!("{}%", number(n.max(0.0)))),
            Some(ComputedValue::LengthPercentageExpression(_)) => {
                // font-size's percentage basis is its parent font. Flex basis
                // belongs to the real container; preserve its percentage type,
                // including 0%, while computing only CSS-owned font dimensions.
                let resolved = crate::computed::individual_transforms::length(&css, font, root);
                Some(match LengthPercentage::parse(&resolved) {
                    Some(LengthPercentage::Px(n)) => format!("{}px", number(n.max(0.0))),
                    Some(LengthPercentage::Percentage(n)) => format!("{}%", number(n.max(0.0))),
                    _ => resolved,
                })
            },
            _ => None,
        };
        if let Some(value) = computed { props.insert("flex-basis".into(), value); }
    }
    match LONGHANDS.map(|name| props.get(name).cloned()) {
        [Some(grow), Some(shrink), Some(basis)] => { props.insert("flex".into(), format!("{grow} {shrink} {basis}")); },
        _ => { props.remove("flex"); },
    }
}
