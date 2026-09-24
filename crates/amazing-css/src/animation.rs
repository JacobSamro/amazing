//! CSS Animations and Transitions parsing shared with the compositor.

use crate::parser::{Parser, Stylesheet};
use crate::syntax::{Token, serialize, tokenize};

#[path = "animation_handoff.rs"]
mod handoff;
pub use handoff::*;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub frames: Vec<Keyframe>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub offsets: Vec<f32>,
    pub timing_function: Option<String>,
    pub declarations: Vec<(String, String)>,
}

/// Extract the winning keyframes definitions in document order.
pub fn keyframes_rules(sheets: &[Stylesheet]) -> Vec<KeyframesRule> {
    let mut result: Vec<KeyframesRule> = Vec::new();
    for rule in sheets.iter().flat_map(|sheet| &sheet.keyframes) {
        result.retain(|prior| prior.name != rule.name);
        result.push(rule.clone());
    }
    result
}

pub(crate) fn parse_keyframes(prelude: &[Token], body: &[Token]) -> Option<KeyframesRule> {
    let name = match significant(prelude).as_slice() {
        [Token::Ident(name)]
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "none" | "initial" | "inherit" | "unset" | "revert" | "revert-layer"
            ) =>
        {
            name.clone()
        }
        [Token::Str(name)] if !name.is_empty() => name.clone(),
        _ => return None,
    };
    let mut frames = Vec::new();
    let mut p = Parser { toks: body, pos: 0 };
    while p.pos < body.len() {
        while matches!(body.get(p.pos), Some(Token::Whitespace | Token::Semicolon)) {
            p.pos += 1;
        }
        if p.pos >= body.len() {
            break;
        }
        if matches!(body.get(p.pos), Some(Token::AtKeyword(_))) {
            p.skip_at_rule();
            continue;
        }
        let start = p.pos;
        let mut depth = 0usize;
        while let Some(token) = body.get(p.pos) {
            match token {
                Token::OpenCurly if depth == 0 => break,
                Token::OpenParen | Token::OpenSquare => depth += 1,
                Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
                _ => {}
            }
            p.pos += 1;
        }
        if p.pos == body.len() {
            break;
        }
        let offsets = split_commas(&body[start..p.pos])
            .and_then(|parts| {
                parts
                    .into_iter()
                    .map(|part| match significant(part).as_slice() {
                        [Token::Ident(value)] if value.eq_ignore_ascii_case("from") => Some(0.0),
                        [Token::Ident(value)] if value.eq_ignore_ascii_case("to") => Some(1.0),
                        [Token::Percentage(value)]
                            if value.is_finite() && (0.0..=100.0).contains(value) =>
                        {
                            Some((*value / 100.0) as f32)
                        }
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .filter(|parts| !parts.is_empty());
        p.pos += 1;
        let declarations = p.block_declarations();
        let Some(offsets) = offsets else {
            continue;
        };
        let mut timing_function = None;
        let mut values = Vec::new();
        for declaration in declarations {
            if declaration.important {
                continue;
            }
            if declaration.property == "animation-timing-function" {
                timing_function =
                    canonical_longhand("animation-timing-function", &declaration.value);
                continue;
            }
            if matches!(declaration.property.as_str(), "animation" | "transition")
                || declaration.property.starts_with("animation-")
                || declaration.property.starts_with("transition-")
            {
                continue;
            }
            for (property, value) in
                crate::computed::expand_declaration(&declaration.property, &declaration.value)
            {
                let variable = tokenize(&value).iter().any(|token| matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("var")));
                if property.starts_with("--")
                    || (crate::computed::is_supported_property(&property)
                        && (variable || crate::values::parse(&property, &value).is_some()))
                {
                    values.push((property, value));
                }
            }
        }
        frames.push(Keyframe {
            offsets,
            timing_function,
            declarations: values,
        });
    }
    Some(KeyframesRule { name, frames })
}

fn significant(tokens: &[Token]) -> Vec<&Token> {
    tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace))
        .collect()
}

fn split_commas(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.checked_sub(1)?,
            Token::Comma if depth == 0 => {
                parts.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    parts.push(&tokens[start..]);
    Some(parts)
}

fn canonical_longhand(property: &str, value: &str) -> Option<String> {
    let tokens = tokenize(value);
    let items = split_commas(&tokens)?;
    let item_count = items.len();
    let mut out = Vec::new();
    for item in items {
        let s = serialize(item).trim().to_owned();
        if s.is_empty() {
            return None;
        }
        let lower = s.to_ascii_lowercase();
        let parsed = match property {
            "animation-name" => match significant(item).as_slice() {
                [Token::Ident(name)] if !css_wide_ident(name) => s,
                [Token::Str(_)] => s,
                _ => return None,
            },
            "transition-property" => match significant(item).as_slice() {
                [Token::Ident(name)]
                    if !css_wide_ident(name)
                        && !(name.eq_ignore_ascii_case("none") && item_count > 1) =>
                {
                    lower
                }
                _ => return None,
            },
            "animation-duration" if lower == "auto" => "0s".to_owned(),
            "animation-duration"
            | "transition-duration"
            | "animation-delay"
            | "transition-delay" => canonical_time(&lower, property.ends_with("duration"))?,
            "animation-timing-function" | "transition-timing-function" => canonical_easing(&lower)?,
            "animation-iteration-count" => {
                if lower == "infinite" {
                    lower
                } else {
                    let n: f64 = lower.parse().ok()?;
                    if !n.is_finite() || n < 0.0 {
                        return None;
                    }
                    n.to_string()
                }
            }
            "animation-direction" => keyword(
                &lower,
                &["normal", "reverse", "alternate", "alternate-reverse"],
            )?,
            "animation-fill-mode" => keyword(&lower, &["none", "forwards", "backwards", "both"])?,
            "animation-play-state" => keyword(&lower, &["running", "paused"])?,
            "animation-composition" => keyword(&lower, &["replace", "add", "accumulate"])?,
            "animation-timeline" => match significant(item).as_slice() {
                [Token::Ident(name)]
                    if matches!(name.as_str(), "auto" | "none") || name.starts_with("--") =>
                {
                    s
                }
                [
                    Token::Function(name),
                    Token::OpenParen,
                    ..,
                    Token::CloseParen,
                ] if matches!(name.as_str(), "scroll" | "view") => s,
                _ => return None,
            },
            "transition-behavior" => keyword(&lower, &["normal", "allow-discrete"])?,
            _ => return None,
        };
        out.push(parsed);
    }
    Some(out.join(", "))
}

fn keyword(s: &str, allowed: &[&str]) -> Option<String> {
    allowed.contains(&s).then(|| s.to_owned())
}
fn css_wide_ident(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    )
}
fn canonical_time(s: &str, nonnegative: bool) -> Option<String> {
    let tokens = tokenize(s);
    let [Token::Dimension(n, unit)] = significant(&tokens).as_slice() else {
        return None;
    };
    if !n.is_finite() || (nonnegative && *n < 0.0) {
        return None;
    }
    let seconds = match unit.as_str() {
        "s" => *n,
        "ms" => *n / 1000.0,
        _ => return None,
    };
    Some(format!("{}s", seconds))
}
fn canonical_easing(s: &str) -> Option<String> {
    if [
        "ease",
        "linear",
        "ease-in",
        "ease-out",
        "ease-in-out",
        "step-start",
        "step-end",
    ]
    .contains(&s)
    {
        return Some(s.to_owned());
    }
    let tokens = tokenize(s);
    match significant(&tokens).as_slice() {
        [
            Token::Function(name),
            Token::OpenParen,
            Token::Number(first),
            Token::Comma,
            Token::Number(second),
            Token::CloseParen,
        ] if name.eq_ignore_ascii_case("linear") && first.is_finite() && second.is_finite() => {
            Some(serialize(&tokens))
        }
        [
            Token::Function(name),
            Token::OpenParen,
            Token::Number(x1),
            Token::Comma,
            Token::Number(y1),
            Token::Comma,
            Token::Number(x2),
            Token::Comma,
            Token::Number(y2),
            Token::CloseParen,
        ] if name.eq_ignore_ascii_case("cubic-bezier")
            && [*x1, *y1, *x2, *y2].iter().all(|n| n.is_finite())
            && (0.0..=1.0).contains(x1)
            && (0.0..=1.0).contains(x2) =>
        {
            Some(serialize(&tokens))
        }
        [
            Token::Function(name),
            Token::OpenParen,
            Token::Number(count),
            Token::CloseParen,
        ] if name.eq_ignore_ascii_case("steps")
            && count.is_finite()
            && *count >= 1.0
            && count.fract() == 0.0 =>
        {
            Some(serialize(&tokens))
        }
        [
            Token::Function(name),
            Token::OpenParen,
            Token::Number(count),
            Token::Comma,
            Token::Ident(position),
            Token::CloseParen,
        ] if name.eq_ignore_ascii_case("steps")
            && count.is_finite()
            && *count >= 1.0
            && count.fract() == 0.0
            && [
                "jump-start",
                "jump-end",
                "jump-none",
                "jump-both",
                "start",
                "end",
            ]
            .contains(&position.as_str())
            && !(position == "jump-none" && *count == 1.0) =>
        {
            Some(serialize(&tokens))
        }
        _ => None,
    }
}

pub(crate) fn canonical(property: &str, value: &str) -> Option<String> {
    canonical_longhand(property, value)
}

pub(crate) fn expand_shorthand(property: &str, value: &str) -> Option<Vec<(String, String)>> {
    let names: &[&str] = if property == "animation" {
        &[
            "animation-name",
            "animation-duration",
            "animation-timing-function",
            "animation-delay",
            "animation-iteration-count",
            "animation-direction",
            "animation-fill-mode",
            "animation-play-state",
            "animation-composition",
            "animation-timeline",
        ]
    } else {
        &[
            "transition-property",
            "transition-duration",
            "transition-timing-function",
            "transition-delay",
            "transition-behavior",
        ]
    };
    let wide = ["initial", "inherit", "unset", "revert", "revert-layer"];
    if wide.contains(&value.trim().to_ascii_lowercase().as_str()) {
        return Some(
            names
                .iter()
                .map(|name| ((*name).to_owned(), value.trim().to_owned()))
                .collect(),
        );
    }
    let tokens = tokenize(value);
    let layers = split_commas(&tokens)?;
    let mut columns: Vec<Vec<String>> = names.iter().map(|_| Vec::new()).collect();
    for layer in layers {
        let components = split_components(layer)?;
        if components.is_empty() {
            return None;
        }
        let defaults = if property == "animation" {
            &[
                "none", "0s", "ease", "0s", "1", "normal", "none", "running", "replace", "auto",
            ][..]
        } else {
            &["all", "0s", "ease", "0s", "normal"][..]
        };
        let mut values = defaults
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>();
        let mut seen = vec![false; names.len()];
        let mut times = 0usize;
        for component in components {
            let raw = serialize(component).trim().to_owned();
            let lower = raw.to_ascii_lowercase();
            let is_time = if property == "animation" && lower == "auto" {
                Some("0s".to_owned())
            } else {
                canonical_time(&lower, false)
            };
            let slot = if let Some(time) = is_time {
                let slot = if times == 0 {
                    1
                } else if times == 1 {
                    3
                } else {
                    return None;
                };
                if slot == 1 && time.starts_with('-') {
                    return None;
                }
                times += 1;
                values[slot] = time;
                seen[slot] = true;
                continue;
            } else if canonical_easing(&lower).is_some() && !seen[2] {
                2
            } else if property == "transition" {
                if canonical_longhand("transition-behavior", &lower).is_some() && !seen[4] {
                    4
                } else if !seen[0] && canonical_longhand("transition-property", &raw).is_some() {
                    0
                } else {
                    return None;
                }
            } else if canonical_longhand("animation-iteration-count", &lower).is_some() && !seen[4]
            {
                4
            } else if canonical_longhand("animation-direction", &lower).is_some() && !seen[5] {
                5
            } else if canonical_longhand("animation-fill-mode", &lower).is_some() && !seen[6] {
                6
            } else if canonical_longhand("animation-play-state", &lower).is_some() && !seen[7] {
                7
            } else if canonical_longhand("animation-composition", &lower).is_some() && !seen[8] {
                8
            } else if !seen[0] && canonical_longhand("animation-name", &raw).is_some() {
                0
            } else {
                return None;
            };
            if seen[slot] {
                return None;
            }
            seen[slot] = true;
            values[slot] = canonical_longhand(names[slot], &raw)?;
        }
        for (column, value) in columns.iter_mut().zip(values) {
            column.push(value);
        }
    }
    Some(
        names
            .iter()
            .zip(columns)
            .map(|(name, column)| ((*name).to_owned(), column.join(", ")))
            .collect(),
    )
}

fn split_components(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut out = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.checked_sub(1)?,
            Token::Whitespace if depth == 0 => {
                if let Some(from) = start.take() {
                    out.push(&tokens[from..index]);
                }
                continue;
            }
            _ => {}
        }
        if start.is_none() {
            start = Some(index);
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(from) = start {
        out.push(&tokens[from..]);
    }
    Some(out)
}
