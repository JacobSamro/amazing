//! Whole CSS Grid shorthand expansion and template-string computed values.
//! Uses our existing component/track validators and CSS Grid2 sections7.3/7.4/7.8.
//! No upstream implementation was copied; named-area layout is a separate seam.
use super::super::{
    DEFERRED_GRID, DEFERRED_GRID_AREAS, DEFERRED_GRID_COLUMNS, DEFERRED_GRID_ROWS,
    css_wide_keyword, deferred,
};
use super::{
    components::grid_components, is_line_names_component, top_level_solidus, trim_tokens,
    valid_grid_auto_flow_axis, valid_grid_template, valid_grid_track_list,
};
use crate::syntax::{Token, serialize, tokenize};
use std::collections::BTreeMap;

const TEMPLATE: [&str; 3] = [
    "grid-template-rows",
    "grid-template-columns",
    "grid-template-areas",
];
const GRID: [&str; 6] = [
    "grid-template-rows",
    "grid-template-columns",
    "grid-template-areas",
    "grid-auto-rows",
    "grid-auto-columns",
    "grid-auto-flow",
];

pub(crate) fn deferred_grid_template(value: &str) -> Vec<(String, String)> {
    TEMPLATE
        .into_iter()
        .zip([
            DEFERRED_GRID_ROWS,
            DEFERRED_GRID_COLUMNS,
            DEFERRED_GRID_AREAS,
        ])
        .map(|(name, marker)| (name.to_owned(), deferred(marker, value)))
        .collect()
}

pub(crate) fn deferred_grid(value: &str) -> Vec<(String, String)> {
    GRID.into_iter()
        .map(|name| (name.to_owned(), deferred(DEFERRED_GRID, value)))
        .collect()
}

/// Cell boundaries are periods, whitespace or a change to/from ident codepoints;
/// they are not merely whitespace-separated words. Digit-leading names are valid.
fn cells(row: &str) -> Option<Vec<Option<String>>> {
    fn ident(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-') || c >= '\u{80}'
    }
    let mut input = row.chars().peekable();
    let mut result = Vec::new();
    while let Some(c) = input.next() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
            continue;
        }
        if c == '.' {
            while input.peek() == Some(&'.') {
                input.next();
            }
            result.push(None);
        } else if ident(c) {
            let mut name = String::from(c);
            while input.peek().is_some_and(|&next| ident(next)) {
                name.push(input.next().unwrap());
            }
            result.push(Some(name));
        } else {
            return None;
        }
    }
    (!result.is_empty()).then_some(result)
}

pub(crate) fn canonical_areas(value: &str) -> Option<String> {
    if value.trim().eq_ignore_ascii_case("none") {
        return Some("none".into());
    }
    let tokens = tokenize(value);
    let mut rows = Vec::new();
    for token in &tokens {
        match token {
            Token::Whitespace => {}
            Token::Str(row) => rows.push(cells(row)?),
            _ => return None,
        }
    }
    let columns = rows.first()?.len();
    let mut bounds = BTreeMap::<String, (usize, usize, usize, usize, usize)>::new();
    for (r, row) in rows.iter().enumerate() {
        if row.len() != columns {
            return None;
        }
        for (c, cell) in row.iter().enumerate() {
            if let Some(name) = cell {
                let b = bounds.entry(name.clone()).or_insert((r, r, c, c, 0));
                b.0 = b.0.min(r);
                b.1 = b.1.max(r);
                b.2 = b.2.min(c);
                b.3 = b.3.max(c);
                b.4 += 1;
            }
        }
    }
    if bounds
        .values()
        .any(|&(r0, r1, c0, c1, count)| (r1 - r0 + 1) * (c1 - c0 + 1) != count)
    {
        return None;
    }
    Some(
        rows.into_iter()
            .map(|row| {
                let string = row
                    .into_iter()
                    .map(|cell| cell.unwrap_or_else(|| ".".into()))
                    .collect::<Vec<_>>()
                    .join(" ");
                serialize(&[Token::Str(string)])
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn append(out: &mut Vec<Token>, part: &[Token]) {
    // Adjacent trailing/leading line-name groups denote one boundary. Keep
    // their contiguous representation, which our existing name parser accepts.
    if !out.is_empty()
        && !(matches!(out.last(), Some(Token::CloseSquare))
            && matches!(part.first(), Some(Token::OpenSquare)))
    {
        out.push(Token::Whitespace);
    }
    out.extend_from_slice(part);
}

pub(crate) fn expand_grid_template(value: &str) -> Option<Vec<(String, String)>> {
    if css_wide_keyword(value).is_some() || value.trim().eq_ignore_ascii_case("none") {
        return Some(
            TEMPLATE
                .into_iter()
                .map(|name| (name.into(), value.trim().into()))
                .collect(),
        );
    }
    if !valid_grid_template(value) {
        return None;
    }
    let tokens = tokenize(value);
    let slash = top_level_solidus(&tokens);
    let before = trim_tokens(&tokens[..slash.unwrap_or(tokens.len())]);
    let columns = slash
        .map(|s| serialize(trim_tokens(&tokens[s + 1..])))
        .unwrap_or_else(|| "none".into());
    if !before.iter().any(|token| matches!(token, Token::Str(_))) {
        return Some(vec![
            (TEMPLATE[0].into(), serialize(before)),
            (TEMPLATE[1].into(), columns),
            (TEMPLATE[2].into(), "none".into()),
        ]);
    }
    let components = grid_components(before)?;
    let mut rows = Vec::new();
    let mut areas = Vec::new();
    let mut index = 0;
    while index < components.len() {
        if is_line_names_component(components[index]) {
            append(&mut rows, components[index]);
            index += 1;
        }
        let [Token::Str(area)] = components.get(index).copied()? else {
            return None;
        };
        areas.push(Token::Str(area.clone()));
        index += 1;
        if let Some(part) = components.get(index)
            && super::valid_track_size(part)
        {
            append(&mut rows, part);
            index += 1;
        } else {
            append(&mut rows, &[Token::Ident("auto".into())]);
        }
        if let Some(part) = components.get(index)
            && is_line_names_component(part)
        {
            append(&mut rows, part);
            index += 1;
        }
    }
    Some(vec![
        (TEMPLATE[0].into(), serialize(&rows)),
        (TEMPLATE[1].into(), columns),
        (TEMPLATE[2].into(), canonical_areas(&serialize(&areas))?),
    ])
}

fn flow_axis(parts: &[&[Token]]) -> Option<(String, bool)> {
    if !valid_grid_auto_flow_axis(parts) {
        return None;
    }
    let mut dense = false;
    let mut index = 0;
    while let Some(part) = parts.get(index) {
        match trim_tokens(part) {
            [Token::Ident(name)] if name.eq_ignore_ascii_case("auto-flow") => {}
            [Token::Ident(name)] if name.eq_ignore_ascii_case("dense") => dense = true,
            _ => break,
        }
        index += 1;
    }
    let tracks = if index == parts.len() {
        "auto".into()
    } else {
        parts[index..]
            .iter()
            .map(|part| serialize(part))
            .collect::<Vec<_>>()
            .join(" ")
    };
    Some((tracks, dense))
}

pub(crate) fn expand_grid(value: &str) -> Option<Vec<(String, String)>> {
    if css_wide_keyword(value).is_some() {
        return Some(
            GRID.into_iter()
                .map(|name| (name.into(), value.trim().into()))
                .collect(),
        );
    }
    if let Some(mut explicit) = expand_grid_template(value) {
        explicit.extend([
            ("grid-auto-rows".into(), "auto".into()),
            ("grid-auto-columns".into(), "auto".into()),
            ("grid-auto-flow".into(), "row".into()),
        ]);
        return Some(explicit);
    }
    let tokens = tokenize(value);
    let slash = top_level_solidus(&tokens)?;
    let rows = trim_tokens(&tokens[..slash]);
    let cols = trim_tokens(&tokens[slash + 1..]);
    let row_parts = grid_components(rows)?;
    let col_parts = grid_components(cols)?;
    let (explicit_rows, explicit_cols, auto_rows, auto_cols, axis, dense) =
        if let Some((tracks, dense)) = flow_axis(&col_parts)
            && valid_grid_track_list(&serialize(rows))
        {
            (
                serialize(rows),
                "none".into(),
                "auto".into(),
                tracks,
                "column",
                dense,
            )
        } else if let Some((tracks, dense)) = flow_axis(&row_parts)
            && valid_grid_track_list(&serialize(cols))
        {
            (
                "none".into(),
                serialize(cols),
                tracks,
                "auto".into(),
                "row",
                dense,
            )
        } else {
            return None;
        };
    Some(vec![
        ("grid-template-rows".into(), explicit_rows),
        ("grid-template-columns".into(), explicit_cols),
        ("grid-template-areas".into(), "none".into()),
        ("grid-auto-rows".into(), auto_rows),
        ("grid-auto-columns".into(), auto_cols),
        (
            "grid-auto-flow".into(),
            if dense {
                format!("{axis} dense")
            } else {
                axis.into()
            },
        ),
    ])
}
