//! CSS Grid Level 1/2 value grammar: the `grid-template-columns`/`-rows`
//! track-list grammar (`<line-names>?`/`repeat()`/`fit-content()`/
//! `minmax()`/`subgrid`), `<grid-line>` (`grid-row-start`/`-end`,
//! `grid-column-start`/`-end`) and its `grid-row`/`grid-column`/`grid-area`
//! shorthand pairs/quads, `grid-auto-columns`/`-rows`/`-flow`,
//! `grid-template-areas`, and the `grid-template`/`grid` shorthands.
//!
//! Split out of `computed.rs` in CSS-29 (was already over this repo's
//! 4,000-line policy before CSS-28 added most of this module; CSS-28 pushed
//! it further). No behavior change from the split itself -- every function
//! here kept its exact body; only visibility (`fn` -> `pub(crate) fn` where
//! a function was previously only called from within `computed.rs`) and the
//! module boundary changed. `computed.rs` re-exports every name below via
//! `pub(crate) use grid_values::{...}` so existing call sites --
//! `crate::computed::valid_grid_line` etc. from `values.rs`, and bare calls
//! from `computed.rs`'s own `expand_declaration`/`resolve_deferred_shorthand`
//! -- keep working unchanged.

use crate::syntax::{Token, serialize, tokenize};
use crate::values;
use super::{
    contains_var_function, css_wide_keyword,
    function_inner, skip_css_whitespace, trim_tokens,
};

#[path = "grid_components.rs"]
mod components;
use components::grid_components;

#[path = "grid_shorthand.rs"]
mod shorthands;
pub(crate) use shorthands::{canonical_areas, deferred_grid, deferred_grid_template, expand_grid, expand_grid_template};

pub(crate) fn split_top_level_comma(tokens: &[Token]) -> Option<(&[Token], &[Token])> {
    let mut stack = Vec::new();
    let mut comma = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Comma if stack.is_empty() && comma.replace(index).is_some() => {
                return None;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let comma = comma?;
    Some((
        trim_tokens(&tokens[..comma]),
        trim_tokens(&tokens[comma + 1..]),
    ))
}

pub(crate) fn valid_grid_track_list(value: &str) -> bool {
    if contains_var_function(value) {
        return true;
    }
    let tokens = tokenize(value);
    let tokens = trim_tokens(&tokens);
    if matches!(tokens, [Token::Ident(value)] if value.eq_ignore_ascii_case("none")) {
        return true;
    }
    if matches!(tokens.first(), Some(Token::Ident(name)) if name.eq_ignore_ascii_case("subgrid")) {
        return parse_subgrid(value).is_some();
    }
    valid_grid_track_tokens(tokens, true)
}

/// Grid Level 2 §7.2: `subgrid <line-name-list>?`. Track sizes cannot
/// participate in this branch of the grid-template grammar.
pub(crate) fn parse_subgrid(value: &str) -> Option<values::Subgrid> {
    use values::{Subgrid, SubgridLineNames, SubgridRepeatCount};
    let tokens = tokenize(value);
    let mut position = 0;
    skip_css_whitespace(&tokens, &mut position);
    match tokens.get(position)? {
        Token::Ident(name) if name.eq_ignore_ascii_case("subgrid") => position += 1,
        _ => return None,
    }
    let mut line_names = Vec::new();
    let mut auto_fill_seen = false;
    loop {
        skip_css_whitespace(&tokens, &mut position);
        let Some(token) = tokens.get(position) else { break };
        if matches!(token, Token::OpenSquare) {
            let (names, end) = parse_subgrid_names(&tokens, position)?;
            line_names.push(SubgridLineNames::Names(names));
            position = end;
            continue;
        }
        if matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("repeat")) {
            let start = position;
            let mut depth = 0usize;
            loop {
                match tokens.get(position)? {
                    Token::OpenParen => depth += 1,
                    Token::CloseParen => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 { position += 1; break; }
                    }
                    _ => {}
                }
                position += 1;
            }
            let inner = function_inner(&tokens[start..position], "repeat")?;
            let (count, names) = split_top_level_comma(inner)?;
            let count = match trim_tokens(count) {
                [Token::Number(n)] if n.is_finite() && *n >= 1.0 && n.fract() == 0.0 && *n <= u32::MAX as f64 => SubgridRepeatCount::Integer(*n as u32),
                [Token::Ident(name)] if name.eq_ignore_ascii_case("auto-fill") && !auto_fill_seen => {
                    auto_fill_seen = true;
                    SubgridRepeatCount::AutoFill
                }
                _ => return None,
            };
            let mut names_position = 0;
            let mut groups = Vec::new();
            while names_position < names.len() {
                skip_css_whitespace(names, &mut names_position);
                if names_position == names.len() { break; }
                let (group, end) = parse_subgrid_names(names, names_position)?;
                groups.push(group);
                names_position = end;
            }
            if groups.is_empty() { return None; }
            line_names.push(SubgridLineNames::Repeat { count, names: groups });
            continue;
        }
        return None;
    }
    Some(Subgrid { line_names })
}

pub(crate) fn parse_subgrid_names(tokens: &[Token], start: usize) -> Option<(Vec<String>, usize)> {
    if !matches!(tokens.get(start), Some(Token::OpenSquare)) { return None; }
    let end = tokens[start + 1..].iter().position(|token| matches!(token, Token::CloseSquare))? + start + 1;
    let mut names = Vec::new();
    for token in &tokens[start + 1..end] {
        match token {
            Token::Whitespace => {}
            Token::Ident(name) if valid_grid_line_name(name) => names.push(name.clone()),
            _ => return None,
        }
    }
    Some((names, end + 1))
}

/// Grid Level 1 §7.2 explicit track lists let an auto-repeat's siblings and
/// its own repeated tracks use only `<fixed-size>` sizing, because auto-fill
/// / auto-fit need a definite total size to compute the repetition count.
/// This walks the top-level components in strict `<line-names>? [
/// <track-size> | <track-repeat> ] <line-names>?` sequencing (at most one
/// `<line-names>` block per slot) and, when an auto-repeat is present
/// anywhere in the list, requires every other component to be fixed-size too.
pub(crate) fn valid_grid_track_tokens(tokens: &[Token], allow_repeat: bool) -> bool {
    let Some(components) = grid_components(tokens) else {
        return false;
    };
    if components.is_empty() {
        return false;
    }
    let has_auto_repeat = allow_repeat
        && components.iter().any(|component| matches!(valid_repeat(component), Some((true, _))));
    let mut auto_repeat_seen = false;
    let mut saw_track = false;
    let mut index = 0;
    while index < components.len() {
        if is_line_names_component(components[index]) {
            if !valid_line_names_block(components[index]) {
                return false;
            }
            index += 1;
            if index == components.len() {
                break;
            }
            if is_line_names_component(components[index]) {
                return false;
            }
        }
        let component = components[index];
        if function_inner(component, "repeat").is_some() {
            if !allow_repeat {
                return false;
            }
            let Some((auto_repeat, tracks)) = valid_repeat(component) else {
                return false;
            };
            if auto_repeat {
                if auto_repeat_seen || !valid_fixed_track_tokens(tracks) {
                    return false;
                }
                auto_repeat_seen = true;
            } else if has_auto_repeat {
                if !valid_fixed_track_tokens(tracks) {
                    return false;
                }
            } else if !valid_grid_track_tokens(tracks, false) {
                return false;
            }
        } else if has_auto_repeat {
            if !valid_fixed_size(component) {
                return false;
            }
        } else if !valid_grid_track_component(component) {
            return false;
        }
        saw_track = true;
        index += 1;
    }
    saw_track
}

pub(crate) fn is_line_names_component(tokens: &[Token]) -> bool {
    matches!(
        (tokens.first(), tokens.last()),
        (Some(Token::OpenSquare), Some(Token::CloseSquare))
    )
}

/// One or more adjacent `[name]` blocks (`[a][b]` is valid without
/// intervening whitespace, and each block -- including an empty `[]` -- is
/// itself a complete `<line-names>`); each block validates independently.
pub(crate) fn valid_line_names_block(tokens: &[Token]) -> bool {
    let mut rest: &[Token] = tokens;
    loop {
        let Some((first, tail)) = rest.split_first() else {
            return false;
        };
        if !matches!(first, Token::OpenSquare) {
            return false;
        }
        let mut depth = 1usize;
        let mut close = None;
        for (index, token) in tail.iter().enumerate() {
            match token {
                Token::OpenSquare => depth += 1,
                Token::CloseSquare => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            return false;
        };
        let inner = trim_tokens(&tail[..close]);
        if !inner.iter().all(|token| match token {
            Token::Ident(name) => valid_grid_line_name(name),
            Token::Whitespace => true,
            _ => false,
        }) {
            return false;
        }
        rest = trim_tokens(&tail[close + 1..]);
        if rest.is_empty() {
            return true;
        }
    }
}

/// A single `<track-size>`: `<track-breadth> | minmax(...) | fit-content(...)`.
/// Unlike [`valid_grid_track_component`] this never accepts a `<line-names>`
/// block, matching properties (`grid-auto-columns`/`-rows`, a
/// `grid-template` string row's inline size) whose grammar is `<track-size>`
/// without line names.
pub(crate) fn valid_track_size(tokens: &[Token]) -> bool {
    let tokens = trim_tokens(tokens);
    if valid_grid_breadth(tokens, true) {
        return true;
    }
    if let Some(inner) = function_inner(tokens, "fit-content") {
        return valid_fixed_breadth(inner);
    }
    if let Some(inner) = function_inner(tokens, "minmax") {
        return split_top_level_comma(inner).is_some_and(|(minimum, maximum)| {
            valid_grid_breadth(minimum, false) && valid_grid_breadth(maximum, true)
        });
    }
    false
}

pub(crate) fn valid_grid_track_component(tokens: &[Token]) -> bool {
    let tokens = trim_tokens(tokens);
    if valid_track_size(tokens) {
        return true;
    }
    if is_line_names_component(tokens) {
        return valid_line_names_block(tokens);
    }
    false
}

pub(crate) fn valid_grid_line_name(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "auto" | "span" | "default" | "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    )
}

pub(crate) fn grid_line_custom_ident(value: &str) -> bool {
    matches!(trim_tokens(&tokenize(value)), [Token::Ident(name)] if valid_grid_line_name(name))
}

/// Grid Level 1 §8.3: `<grid-line> = auto | [ <custom-ident> | [ <integer>
/// && <custom-ident>? ] | [ span && [ <integer> || <custom-ident> ] ] ]`.
/// The `&&`/`||` combinators are formally order-free, but this crate matches
/// the order real UAs implement: `span` may lead or trail an integer+name
/// pair, but not sit between them (`span foo 2` and `foo span` are valid;
/// `foo span 2` is not). Words are matched textually (not just by numeric
/// value) so a token like `1.0` -- a `<number>`, not a lexical `<integer>` --
/// is correctly rejected rather than accepted via `fract() == 0.0`.
/// Splits on whitespace like `str::split_whitespace`, except (a) a CSS
/// escape's own mandatory single trailing whitespace (`\31 st` decodes to
/// the one ident `1st`; the space after `\31` is the escape's terminator,
/// not a separator) does not start a new word -- a naive `split_whitespace`
/// would cut `\31 st` into `\31` and `st`, each of which then fails to
/// tokenize as the single ident/number a real UA sees -- and (b) whitespace
/// inside a parenthesized function call (`calc(sibling-index() - 2)`,
/// `min(-1, 6)`, standing in for `<integer>` per CSS Values 4) stays part of
/// that one word rather than being split at every internal space.
pub(crate) fn split_grid_line_words(value: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut word_start: Option<usize> = None;
    let mut depth: i32 = 0;
    let mut chars = value.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '(' {
            if word_start.is_none() {
                word_start = Some(i);
            }
            depth += 1;
            continue;
        }
        if ch == ')' {
            if word_start.is_none() {
                word_start = Some(i);
            }
            depth = (depth - 1).max(0);
            continue;
        }
        if depth > 0 {
            if word_start.is_none() {
                word_start = Some(i);
            }
            continue;
        }
        if ch == '\\' {
            if word_start.is_none() {
                word_start = Some(i);
            }
            let mut hex_digits = 0;
            while hex_digits < 6 {
                match chars.peek() {
                    Some((_, c)) if c.is_ascii_hexdigit() => {
                        hex_digits += 1;
                        chars.next();
                    }
                    _ => break,
                }
            }
            if hex_digits > 0 {
                if let Some((_, c)) = chars.peek() {
                    if matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c') {
                        chars.next();
                    }
                }
            } else {
                // `\` + any other single (non-hex, non-newline) char.
                chars.next();
            }
            continue;
        }
        if ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                words.push(&value[start..i]);
            }
            continue;
        }
        if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        words.push(&value[start..]);
    }
    words
}

pub(crate) fn valid_grid_line(value: &str) -> bool {
    // A variable can supply the complete grid-line grammar after substitution.
    if contains_var_function(value) {
        return true;
    }
    let words: Vec<&str> = split_grid_line_words(value);
    if words.is_empty() {
        return false;
    }
    if words.len() == 1 && words[0].eq_ignore_ascii_case("auto") {
        return true;
    }

    enum Kind {
        Span,
        Integer(Option<f64>),
        Name,
    }

    // CSS Values 4 lets a math function stand in for `<integer>` (e.g.
    // `span calc(sibling-index() - 2)`, `span min(-1, 6)`). Its numeric
    // value may depend on layout (container queries, `sibling-index()`)
    // this crate cannot statically evaluate, so it is accepted for
    // grammar/validity purposes with an unknown value, exempt from the
    // sign checks below (those only reject a *known* non-positive count).
    fn integer_calc_function(tokens: &[Token]) -> bool {
        ["calc", "min", "max", "clamp"]
            .iter()
            .any(|name| function_inner(tokens, name).is_some())
    }

    let mut span_pos = None;
    let mut integer: Option<(usize, Option<f64>)> = None;
    let mut name_pos = None;
    for (position, word) in words.iter().enumerate() {
        let tokens = tokenize(word);
        let trimmed = trim_tokens(&tokens);
        let kind = match trimmed {
            [Token::Ident(name)] if name.eq_ignore_ascii_case("span") => Kind::Span,
            [Token::Number(n)] if n.is_finite() && n.fract() == 0.0 && !word.contains(['.', 'e', 'E']) => {
                Kind::Integer(Some(*n))
            }
            _ if integer_calc_function(trimmed) => Kind::Integer(None),
            [Token::Ident(name)] if valid_grid_line_name(name) => Kind::Name,
            _ => return false,
        };
        match kind {
            Kind::Span if span_pos.is_none() => span_pos = Some(position),
            Kind::Integer(n) if integer.is_none() => integer = Some((position, n)),
            Kind::Name if name_pos.is_none() => name_pos = Some(position),
            _ => return false, // a second `span`, integer, or name.
        }
    }

    if let (Some(span_pos), Some((int_pos, _)), Some(name_pos)) = (span_pos, integer, name_pos) {
        let (min, max) = (int_pos.min(name_pos), int_pos.max(name_pos));
        if span_pos > min && span_pos < max {
            return false; // `span` sandwiched between the integer and the name.
        }
    }

    if span_pos.is_some() {
        (integer.is_some_and(|(_, n)| n.is_none_or(|n| n > 0.0)) || name_pos.is_some())
            && integer.is_none_or(|(_, n)| n.is_none_or(|n| n > 0.0))
    } else {
        (integer.is_some() || name_pos.is_some())
            && integer.is_none_or(|(_, n)| n.is_none_or(|n| n != 0.0))
    }
}

/// `grid-row`/`grid-column`: `<grid-line> [ / <grid-line> ]?`.
pub(crate) fn valid_grid_line_pair(value: &str) -> bool {
    if css_wide_keyword(value).is_some() || contains_var_function(value) {
        return true;
    }
    match value.split_once('/') {
        Some((start, end)) if !end.contains('/') => valid_grid_line(start.trim()) && valid_grid_line(end.trim()),
        Some(_) => false,
        None => valid_grid_line(value.trim()),
    }
}

/// `grid-area`: `<grid-line> [ / <grid-line> ]{0,3}`.
pub(crate) fn valid_grid_area(value: &str) -> bool {
    if css_wide_keyword(value).is_some() || contains_var_function(value) {
        return true;
    }
    let segments: Vec<&str> = value.split('/').map(str::trim).collect();
    segments.len() <= 4 && segments.iter().all(|segment| valid_grid_line(segment))
}

/// `grid-auto-columns`/`grid-auto-rows`: `<track-size>+`. Unlike the
/// `grid-template-rows`/`-columns` longhands this never accepts line names
/// or `repeat()`.
pub(crate) fn valid_grid_auto_track_list(value: &str) -> bool {
    if css_wide_keyword(value).is_some() || contains_var_function(value) {
        return true;
    }
    let tokens = tokenize(value);
    let Some(components) = grid_components(&tokens) else {
        return false;
    };
    !components.is_empty() && components.iter().all(|component| valid_track_size(component))
}

/// `grid-auto-flow`: `[ row | column ] || dense`, order-free per its `||`
/// combinator (unlike `<grid-line>`'s `span`).
pub(crate) fn valid_grid_auto_flow(value: &str) -> bool {
    if css_wide_keyword(value).is_some() || contains_var_function(value) {
        return true;
    }
    let words: Vec<&str> = value.split_whitespace().collect();
    if words.is_empty() || words.len() > 2 {
        return false;
    }
    let mut row_or_column = false;
    let mut dense = false;
    for word in &words {
        if word.eq_ignore_ascii_case("dense") && !dense {
            dense = true;
        } else if (word.eq_ignore_ascii_case("row") || word.eq_ignore_ascii_case("column")) && !row_or_column {
            row_or_column = true;
        } else {
            return false;
        }
    }
    true
}

/// `grid-template-areas`: `none | <string>+`, where every string is a
/// whitespace-separated row of cell tokens and every row has the same
/// column count (an empty or all-whitespace row is never valid).
pub(crate) fn valid_grid_template_areas(value: &str) -> bool {
    css_wide_keyword(value).is_some() || contains_var_function(value) || canonical_areas(value).is_some()
}

/// `[ auto-flow && dense? ] <track-size>*` -- one axis of the `grid`
/// shorthand's auto-flow alternative. `components` are the tokens on one
/// side of the shorthand's single top-level `/`.
pub(crate) fn valid_grid_auto_flow_axis(components: &[&[Token]]) -> bool {
    if components.is_empty() {
        return false;
    }
    let mut index = 0;
    let mut saw_auto_flow = false;
    let mut saw_dense = false;
    while index < components.len() {
        let is_auto_flow = matches!(trim_tokens(components[index]), [Token::Ident(name)] if name.eq_ignore_ascii_case("auto-flow"));
        let is_dense = matches!(trim_tokens(components[index]), [Token::Ident(name)] if name.eq_ignore_ascii_case("dense"));
        if is_auto_flow && !saw_auto_flow {
            saw_auto_flow = true;
        } else if is_dense && !saw_dense {
            saw_dense = true;
        } else {
            break;
        }
        index += 1;
    }
    saw_auto_flow && components[index..].iter().all(|component| valid_track_size(component))
}

/// The `grid` shorthand's two `auto-flow`-bearing alternatives:
/// `<'grid-template-rows'> / [ auto-flow && dense? ] <'grid-auto-columns'>?`
/// and its row/column mirror. Requires exactly one top-level `/`.
pub(crate) fn valid_grid_auto_flow_form(value: &str) -> bool {
    let tokens = tokenize(value);
    let Some(slash) = top_level_solidus(&tokens) else {
        return false;
    };
    let rows = trim_tokens(&tokens[..slash]);
    let columns = trim_tokens(&tokens[slash + 1..]);
    let Some(row_components) = grid_components(rows) else { return false };
    let Some(column_components) = grid_components(columns) else { return false };
    (valid_grid_track_list(&serialize(rows)) && valid_grid_auto_flow_axis(&column_components))
        || (valid_grid_auto_flow_axis(&row_components) && valid_grid_track_list(&serialize(columns)))
}

/// Finds the single top-level (not nested in `()`/`[]`/`{}`) `/` in `tokens`.
/// Returns `None` if there is none, or more than one.
pub(crate) fn top_level_solidus(tokens: &[Token]) -> Option<usize> {
    let mut stack = Vec::new();
    let mut slash = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Delim('/') if stack.is_empty() && slash.replace(index).is_some() => return None,
            _ => {}
        }
    }
    stack.is_empty().then(|| slash).flatten()
}

/// `grid-template`: `none | [ <'grid-template-rows'> / <'grid-template-columns'>
/// ] | [ <line-names>? <string> <track-size>? <line-names>? ]+ [ /
/// <explicit-track-list> ]?`. The two forms are disambiguated by whether the
/// part before any top-level `/` contains a `<string>`.
pub(crate) fn valid_grid_template(value: &str) -> bool {
    if css_wide_keyword(value).is_some() {
        return true;
    }
    if value.trim().eq_ignore_ascii_case("none") {
        return true;
    }

    let tokens = tokenize(value);
    let mut stack = Vec::new();
    let mut slash = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return false,
            Token::CloseSquare if stack.pop() != Some(']') => return false,
            Token::CloseCurly if stack.pop() != Some('}') => return false,
            Token::Delim('/') if stack.is_empty() && slash.replace(index).is_some() => return false,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return false;
    }

    let main = match slash {
        Some(index) => trim_tokens(&tokens[..index]),
        None => trim_tokens(&tokens),
    };
    let has_string = main.iter().any(|token| matches!(token, Token::Str(_)));

    if !has_string {
        let Some(slash) = slash else {
            return false;
        };
        let rows = serialize(trim_tokens(&tokens[..slash]));
        let columns = serialize(trim_tokens(&tokens[slash + 1..]));
        return valid_grid_track_list(&rows) && valid_grid_track_list(&columns);
    }

    if let Some(slash) = slash {
        let columns = trim_tokens(&tokens[slash + 1..]);
        if !valid_grid_track_tokens(columns, false) {
            return false;
        }
    }

    let Some(components) = grid_components(main) else {
        return false;
    };
    if components.is_empty() {
        return false;
    }

    let mut area_strings = Vec::new();
    let mut index = 0;
    while index < components.len() {
        if is_line_names_component(components[index]) {
            if !valid_line_names_block(components[index]) {
                return false;
            }
            index += 1;
        }
        let Some(row) = components.get(index).and_then(|component| match trim_tokens(component) {
            [Token::Str(row)] => Some(row),
            _ => None,
        }) else {
            return false;
        };
        index += 1;
        area_strings.push(Token::Str(row.clone()));
        if let Some(component) = components.get(index) {
            if !is_line_names_component(component) && valid_track_size(component) {
                index += 1;
            }
        }
        if let Some(component) = components.get(index) {
            if is_line_names_component(component) {
                if !valid_line_names_block(component) {
                    return false;
                }
                index += 1;
            }
        }
    }
    canonical_areas(&serialize(&area_strings)).is_some()
}

/// `grid`: `<'grid-template'> | <'grid-template-rows'> / [ auto-flow &&
/// dense? ] <'grid-auto-columns'>? | [ auto-flow && dense? ]
/// <'grid-auto-rows'>? / <'grid-template-columns'>`.
pub(crate) fn valid_grid_shorthand(value: &str) -> bool {
    valid_grid_template(value) || valid_grid_auto_flow_form(value)
}

/// Returns whether this is an auto-repeat plus its track-list body.
pub(crate) fn valid_repeat(tokens: &[Token]) -> Option<(bool, &[Token])> {
    let inner = function_inner(tokens, "repeat")?;
    let (count, tracks) = split_top_level_comma(inner)?;
    let auto_repeat = match count {
        [Token::Number(value)] if value.is_finite() && *value >= 1.0 && value.fract() == 0.0 => {
            false
        }
        [Token::Ident(value)]
            if value.eq_ignore_ascii_case("auto-fill")
                || value.eq_ignore_ascii_case("auto-fit") =>
        {
            true
        }
        _ => return None,
    };
    Some((auto_repeat, tracks))
}

pub(crate) fn valid_fixed_track_tokens(tokens: &[Token]) -> bool {
    let Some(components) = grid_components(tokens) else {
        return false;
    };
    let mut saw_track = false;
    for component in components {
        let component = trim_tokens(component);
        if matches!(
            (component.first(), component.last()),
            (Some(Token::OpenSquare), Some(Token::CloseSquare))
        ) {
            if !valid_grid_track_component(component) {
                return false;
            }
            continue;
        }
        saw_track = true;
        if !valid_fixed_size(component) {
            return false;
        }
    }
    saw_track
}

pub(crate) fn valid_fixed_size(tokens: &[Token]) -> bool {
    if valid_fixed_breadth(tokens) {
        return true;
    }
    function_inner(tokens, "minmax")
        .and_then(split_top_level_comma)
        .is_some_and(|(minimum, maximum)| {
            (valid_fixed_breadth(minimum) && valid_grid_breadth(maximum, true))
                || (valid_grid_breadth(minimum, false) && valid_fixed_breadth(maximum))
        })
}

pub(crate) fn valid_fixed_breadth(tokens: &[Token]) -> bool {
    let tokens = trim_tokens(tokens);
    match tokens {
        [Token::Number(value)] => value.is_finite() && *value == 0.0,
        [Token::Percentage(value)] => value.is_finite() && *value >= 0.0,
        [Token::Dimension(value, unit)] => {
            value.is_finite()
                && *value >= 0.0
                && matches!(
                    unit.to_ascii_lowercase().as_str(),
                    "px" | "em"
                        | "rem"
                        | "ex"
                        | "ch"
                        | "vw"
                        | "vh"
                        | "vmin"
                        | "vmax"
                        | "cm"
                        | "mm"
                        | "q"
                        | "in"
                        | "pc"
                        | "pt"
                )
        }
        _ => valid_calc_length_percentage(tokens, true),
    }
}

/// `calc()`/`min()`/`max()`/`clamp()` resolving to a length or
/// length-percentage. Parse-time validity does not evaluate the
/// expression's sign (`calc(-0.5em + 10px)` is valid CSS even though it may
/// resolve negative for some `em`), matching WPT's grid track-size tests.
pub(crate) fn valid_calc_length_percentage(tokens: &[Token], allow_percentage: bool) -> bool {
    matches!(tokens.first(), Some(Token::Function(name))
        if matches!(name.to_ascii_lowercase().as_str(), "calc" | "min" | "max" | "clamp"))
        && crate::math::length_percentage(&serialize(tokens), allow_percentage)
}

pub(crate) fn valid_grid_breadth(tokens: &[Token], allow_fr: bool) -> bool {
    let tokens = trim_tokens(tokens);
    match tokens {
        [Token::Ident(value)] => matches!(
            value.to_ascii_lowercase().as_str(),
            "auto" | "min-content" | "max-content"
        ),
        [Token::Number(value)] => value.is_finite() && *value == 0.0,
        [Token::Percentage(value)] => value.is_finite() && *value >= 0.0,
        [Token::Dimension(value, unit)] => {
            value.is_finite()
                && *value >= 0.0
                && match unit.to_ascii_lowercase().as_str() {
                    "fr" => allow_fr,
                    "px" | "em" | "rem" | "ex" | "ch" | "vw" | "vh" | "vmin" | "vmax" | "cm"
                    | "mm" | "q" | "in" | "pc" | "pt" => true,
                    _ => false,
                }
        }
        _ => valid_calc_length_percentage(tokens, true),
    }
}
