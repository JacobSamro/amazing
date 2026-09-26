use super::trim_tokens;
use crate::syntax::{Token, tokenize};
use crate::values;

/// Substitute CSS Values 5 `attr()` after custom properties have been
/// resolved, using the originating element's attributes. A failed cast takes
/// the fallback; without one the declaration is invalid at computed time.
pub(super) fn substitute_attributes(
    tokens: &[Token],
    attrs: Option<&[(String, String)]>,
) -> Option<Vec<Token>> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if !matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("attr"))
        {
            result.push(tokens[index].clone());
            index += 1;
            continue;
        }
        if tokens.get(index + 1) != Some(&Token::OpenParen) {
            return None;
        }
        let mut depth = 0usize;
        let mut close = None;
        for at in index + 2..tokens.len() {
            match tokens[at] {
                Token::OpenParen => depth += 1,
                Token::CloseParen if depth == 0 => {
                    close = Some(at);
                    break;
                }
                Token::CloseParen => depth -= 1,
                _ => {}
            }
        }
        let close = close?;
        let inner = &tokens[index + 2..close];
        let mut depth = 0usize;
        let comma = inner.iter().position(|token| {
            match token {
                Token::OpenParen => depth += 1,
                Token::CloseParen => depth = depth.saturating_sub(1),
                Token::Comma if depth == 0 => return true,
                _ => {}
            }
            false
        });
        let head = trim_tokens(&inner[..comma.unwrap_or(inner.len())]);
        let fallback = comma.map(|at| &inner[at + 1..]);
        let (name, cast) = match head {
            [Token::Ident(name)] => (name.as_str(), None),
            [Token::Ident(name), Token::Whitespace, rest @ ..] => {
                (name.as_str(), Some(trim_tokens(rest)))
            }
            _ => return None,
        };
        let cast = cast.filter(|tokens| !tokens.is_empty());
        let raw = attrs.and_then(|attributes| {
            attributes
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        });
        let replacement = raw.and_then(|raw| cast_attribute(raw, cast));
        let replacement = match replacement {
            Some(tokens) => tokens,
            None if fallback.is_some() && cast.is_none() => match trim_tokens(fallback?) {
                [Token::Str(value)] => vec![Token::Str(value.clone())],
                _ => vec![Token::Str(String::new())],
            },
            None if fallback.is_some() => substitute_attributes(fallback?, attrs)?,
            None if cast.is_none() => vec![Token::Str(String::new())],
            None => return None,
        };
        result.extend(replacement);
        index = close + 1;
    }
    Some(result)
}

fn cast_attribute(raw: &str, cast: Option<&[Token]>) -> Option<Vec<Token>> {
    let Some(cast) = cast else {
        return Some(vec![Token::Str(raw.to_owned())]);
    };
    let kind = match cast {
        [
            Token::Function(function),
            Token::OpenParen,
            Token::Delim('<'),
            Token::Ident(kind),
            Token::Delim('>'),
            Token::CloseParen,
        ] if function.eq_ignore_ascii_case("type") => kind.to_ascii_lowercase(),
        [Token::Delim('%')] => {
            let value = raw
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())?;
            return Some(vec![Token::Percentage(value)]);
        }
        [Token::Ident(unit)] if matches!(unit.as_str(), "px" | "em" | "rem" | "vw" | "vh") => {
            let value = raw
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())?;
            return Some(vec![Token::Dimension(value, unit.clone())]);
        }
        _ => return None,
    };
    let parsed = tokenize(raw);
    let significant = trim_tokens(&parsed);
    if kind == "string" {
        return Some(vec![Token::Str(raw.to_owned())]);
    }
    let valid = match kind.as_str() {
        "length" => matches!(
            values::LengthPercentage::parse(raw),
            Some(values::LengthPercentage::Px(_) | values::LengthPercentage::Relative(_, _))
        ),
        "length-percentage" => values::LengthPercentage::parse(raw).is_some(),
        "color" => crate::color::parse_css_color(raw).is_some(),
        "number" => matches!(significant, [Token::Number(n)] if n.is_finite()),
        "integer" => matches!(significant, [Token::Number(n)] if n.is_finite() && n.fract() == 0.0),
        "percentage" => matches!(significant, [Token::Percentage(n)] if n.is_finite()),
        "angle" => values::Angle::parse(raw).is_some(),
        _ => false,
    };
    valid.then(|| significant.to_vec())
}
