//! CSS-29: the `box-shadow` grammar (CSS Backgrounds 3 SS7.1's `<shadow>`
//! list) lives in its own module -- it was the largest cohesive group added
//! while fixing the box-shadow-invalid.html strictness cluster, and kept
//! `computed.rs` under the 4,000-line policy. Only `valid_box_shadow` is
//! called from outside this module (from `values.rs`, as
//! `crate::computed::valid_box_shadow`, re-exported there); everything else
//! here is a private helper used only among these functions.

use super::{function_inner, top_level_components, trim_tokens};
use crate::syntax::{Token, serialize, tokenize};

pub(crate) fn valid_box_shadow(value: &str) -> bool {
    if value.trim().eq_ignore_ascii_case("none") {
        return true;
    }
    let tokens = tokenize(value);
    // CSS-29: `box-shadow` is a comma-separated LIST of `<shadow>`s, not a
    // single one -- the previous version rejected any top-level comma
    // outright, so every multi-shadow value (a common, spec-valid case) was
    // silently unsupported.
    let Some(groups) = top_level_comma_groups(&tokens) else {
        return false;
    };
    groups.iter().all(|group| {
        top_level_components(group).is_some_and(|components| {
            !components.is_empty() && valid_single_box_shadow(&components)
        })
    })
}

/// Splits on top-level (paren/bracket/brace-depth-zero) commas, same
/// depth-tracking shape as `top_level_components` but splitting on
/// `Token::Comma` instead of whitespace. Always returns at least one group
/// (an input with no top-level comma is one group spanning the whole slice).
fn top_level_comma_groups(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut groups = Vec::new();
    let mut stack = Vec::new();
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Comma if stack.is_empty() => {
                groups.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    groups.push(&tokens[start..]);
    Some(groups)
}

/// One `<shadow>` (CSS Backgrounds 3 SS7.1):
/// `<color>? && <length>{2,4} && inset?`.
///
/// CSS-29: the double-bar (`&&`) combinator lets the color, the length run,
/// and `inset` appear in any relative order, each at most once -- but the
/// length run itself is a SINGLE unit that must stay contiguous. `-4px red
/// 4px` is not "a shorter shadow plus a color"; a color or `inset`
/// interrupting the middle of the offsets is invalid, the same as having two
/// colors or two `inset`s. The previous version counted each kind
/// independently and never checked adjacency, so any reordering/splitting of
/// a well-formed shadow's tokens (or a color/`inset` alone, or too few/too
/// many lengths) parsed as valid; this is the box-shadow-invalid.html
/// cluster (40 failures).
fn valid_single_box_shadow(components: &[&[Token]]) -> bool {
    enum Part {
        Inset,
        Color,
        /// `Some(v)` for a literal length with a known sign; `None` for a
        /// percentage-free `calc()` length, whose sign is not evaluated at
        /// parse time (see `shadow_length`'s doc comment).
        Length(Option<f64>),
    }

    let mut parts = Vec::with_capacity(components.len());
    for component in components {
        match trim_tokens(component) {
            [Token::Ident(keyword)] if keyword.eq_ignore_ascii_case("inset") => {
                parts.push(Part::Inset);
            }
            [Token::Ident(keyword)] if valid_supported_color_name(keyword) => {
                parts.push(Part::Color);
            }
            [Token::Hash(hex)] if valid_hex_color(hex) => {
                parts.push(Part::Color);
            }
            component
                if function_inner(component, "rgb").is_some()
                    || function_inner(component, "rgba").is_some() =>
            {
                let inner =
                    function_inner(component, "rgb").or_else(|| function_inner(component, "rgba"));
                if !inner.is_some_and(valid_rgb_color_tokens) {
                    return false;
                }
                parts.push(Part::Color);
            }
            component => {
                let Some(length) = shadow_length(component) else {
                    return false;
                };
                parts.push(Part::Length(length));
            }
        }
    }

    if parts.iter().filter(|p| matches!(p, Part::Inset)).count() > 1 {
        return false;
    }
    if parts.iter().filter(|p| matches!(p, Part::Color)).count() > 1 {
        return false;
    }

    // The lengths must form exactly one contiguous run; a color or `inset`
    // splitting them into two runs (or there being no run, or more than
    // one) is invalid, regardless of the total length count.
    let mut runs: Vec<Vec<Option<f64>>> = Vec::new();
    let mut run_open = false;
    for part in &parts {
        match part {
            Part::Length(v) => {
                if !run_open {
                    runs.push(Vec::new());
                    run_open = true;
                }
                runs.last_mut().unwrap().push(*v);
            }
            _ => run_open = false,
        }
    }
    if runs.len() != 1 {
        return false;
    }
    let lengths = &runs[0];
    if !(2..=4).contains(&lengths.len()) {
        return false;
    }
    // Blur radius (the 3rd length) must be non-negative -- but only when
    // it's a literal token; a calc() result outside range is clamped at
    // computed-value time, not a parse error (see `shadow_length`).
    match lengths.get(2) {
        Some(Some(blur)) => *blur >= 0.0,
        _ => true,
    }
}

fn valid_supported_color_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "currentcolor"
            | "aliceblue"
            | "antiquewhite"
            | "aqua"
            | "aquamarine"
            | "azure"
            | "beige"
            | "black"
            | "blue"
            | "brown"
            | "chocolate"
            | "coral"
            | "crimson"
            | "cyan"
            | "darkgray"
            | "darkgrey"
            | "dimgray"
            | "dimgrey"
            | "fuchsia"
            | "gold"
            | "goldenrod"
            | "gray"
            | "green"
            | "greenyellow"
            | "grey"
            | "honeydew"
            | "hotpink"
            | "indigo"
            | "ivory"
            | "khaki"
            | "lavender"
            | "lime"
            | "magenta"
            | "maroon"
            | "navy"
            | "olive"
            | "orange"
            | "orchid"
            | "pink"
            | "plum"
            | "purple"
            | "rebeccapurple"
            | "red"
            | "salmon"
            | "seagreen"
            | "silver"
            | "skyblue"
            | "slategray"
            | "snow"
            | "steelblue"
            | "tan"
            | "teal"
            | "thistle"
            | "tomato"
            | "transparent"
            | "turquoise"
            | "violet"
            | "wheat"
            | "white"
            | "whitesmoke"
            | "yellow"
    )
}

fn valid_hex_color(hex: &str) -> bool {
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_rgb_color_tokens(tokens: &[Token]) -> bool {
    let value = serialize(tokens);
    if value.contains(',') {
        let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
        matches!(parts.len(), 3 | 4)
            && parts[..3].iter().all(|part| valid_rgb_channel(part))
            && parts.get(3).is_none_or(|alpha| valid_alpha_channel(alpha))
    } else {
        let normalized = value.replace('/', " / ");
        let parts = normalized.split_ascii_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            [red, green, blue] => [red, green, blue]
                .into_iter()
                .all(|part| valid_rgb_channel(part)),
            [red, green, blue, slash, alpha] if *slash == "/" => {
                [red, green, blue]
                    .into_iter()
                    .all(|part| valid_rgb_channel(part))
                    && valid_alpha_channel(alpha)
            }
            _ => false,
        }
    }
}

fn valid_rgb_channel(value: &str) -> bool {
    value
        .strip_suffix('%')
        .unwrap_or(value)
        .parse::<f32>()
        .is_ok_and(|value| value.is_finite())
}

fn valid_alpha_channel(value: &str) -> bool {
    valid_rgb_channel(value)
}

/// A `<shadow>`'s `<length>` component. `Some(Some(v))` for a literal length
/// with a known sign (any CSS length unit, not just `px`); `Some(None)` for
/// a percentage-free `calc()` length -- CSS Values 4: a `calc()` result
/// outside a property's allowed range is clamped at computed-value time, not
/// rejected as a parse error, unlike an out-of-range literal token, so its
/// sign is deliberately left unchecked here; `None` when this is not a
/// `<length>` at all (a bare non-zero number, a percentage -- box-shadow
/// offsets/blur/spread are plain `<length>`, never `<length-percentage>` --
/// or a percentage-containing `calc()`).
fn shadow_length(tokens: &[Token]) -> Option<Option<f64>> {
    let trimmed = trim_tokens(tokens);
    match trimmed {
        [Token::Number(value)] if *value == 0.0 => Some(Some(*value)),
        [Token::Dimension(value, unit)]
            if matches!(
                unit.to_ascii_lowercase().as_str(),
                "px" | "em" | "rem" | "ex" | "ch" | "vw" | "vh" | "vmin" | "vmax" | "cm" | "mm"
                    | "q" | "in" | "pc" | "pt"
            ) =>
        {
            Some(Some(*value))
        }
        _ if is_length_only_calc(trimmed) => Some(None),
        _ => None,
    }
}

/// A `calc(...)` expression containing no `Token::Percentage` anywhere in
/// its argument tokens. Not a full grammar check of `calc()`'s internal
/// arithmetic (this crate does not evaluate it), just enough to distinguish
/// `calc(1em - 2px)`/`calc(-1px)` (plain lengths, valid for `box-shadow`)
/// from `calc(2px + 2%)` (mixes in a percentage, invalid here).
fn is_length_only_calc(tokens: &[Token]) -> bool {
    match function_inner(tokens, "calc") {
        Some(inner) => !inner.iter().any(|token| matches!(token, Token::Percentage(_))),
        None => false,
    }
}
