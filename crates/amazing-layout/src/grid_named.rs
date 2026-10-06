// Portions ported from DioxusLabs/taffy src/compute/grid/explicit_grid.rs
// @ 2c81b8e09d172c2e4d7747be2b1e5a03e0f30bf0, MIT.
//! Named areas and named-line placement (CSS Grid 2 sections 7.3 and 8.3).
//! Canonical area rows are supplied by the CSS producer; names remain case sensitive.
use super::*;
use amazing_css::syntax::{tokenize, Token};

pub(super) fn add_template(spec: &mut GridSpec, value: &str, subgrid_cols: bool, subgrid_rows: bool) {
    if value.trim() == "none" { return; }
    let tokens = tokenize(value);
    let mut rows = Vec::new();
    for token in tokens {
        match token {
            Token::Str(row) => rows.push(row.split_ascii_whitespace().map(str::to_owned).collect::<Vec<_>>()),
            Token::Whitespace => {},
            _ => return,
        }
    }
    let Some(width) = rows.first().map(Vec::len).filter(|n| *n > 0) else { return; };
    if width > MAX_GRID_TRACKS || rows.len() > MAX_GRID_TRACKS || rows.iter().any(|r| r.len() != width) { return; }
    let mut areas = HashMap::<String, (usize, usize, usize, usize, usize)>::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, name) in row.iter().enumerate() {
            if name.bytes().all(|ch| ch == b'.') { continue; }
            // Mixed dot/name cells belong to CSS lexical normalization, never silently reinterpret them.
            if name.contains('.') { return; }
            let a = areas.entry(name.clone()).or_insert((r, r, c, c, 0));
            a.0 = a.0.min(r); a.1 = a.1.max(r); a.2 = a.2.min(c); a.3 = a.3.max(c); a.4 += 1;
        }
    }
    if areas.values().any(|a| (a.1-a.0+1)*(a.3-a.2+1) != a.4) { return; }
    if !subgrid_cols { spec.cols = track_vector(&spec.cols, &spec.auto_cols, spec.cols.len().max(width)); }
    if !subgrid_rows { spec.rows = track_vector(&spec.rows, &spec.auto_rows, spec.rows.len().max(rows.len())); }
    for (name, (r0, r1, c0, c1, _)) in areas {
        for (names, start, end) in [(&mut spec.col_line_names, c0, c1), (&mut spec.row_line_names, r0, r1)] {
            for (suffix, line) in [("start", start + 1), ("end", end + 2)] {
                let lines = names.entry(format!("{name}-{suffix}")).or_default();
                lines.push(line as i32); lines.sort_unstable(); lines.dedup();
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Slot { Auto, Line(i32), Span(u32, Option<String>) }

fn slot(value: &str, end: bool, count: usize, names: &HashMap<String, Vec<i32>>) -> Slot {
    let mut number = None;
    let mut name = None;
    let mut span = false;
    for token in tokenize(value) {
        match token {
            Token::Whitespace => {},
            Token::Ident(word) if word.eq_ignore_ascii_case("auto") => return Slot::Auto,
            Token::Ident(word) if word.eq_ignore_ascii_case("span") => {
                if span { return Slot::Auto; } span = true;
            },
            Token::Ident(word) => { if name.replace(word).is_some() { return Slot::Auto; } },
            Token::Number(n) if n.is_finite() && n.fract() == 0.0 && n != 0.0 => {
                if n < i32::MIN as f64 || n > i32::MAX as f64 { return Slot::Auto; }
                let n = n as i32;
                if number.replace(n).is_some() { return Slot::Auto; }
            },
            _ => return Slot::Auto,
        }
    }
    if span {
        if number.is_some_and(|n| n < 1) { return Slot::Auto; }
        return Slot::Span(number.unwrap_or(1).min(MAX_GRID_TRACKS as i32) as u32, name);
    }
    if let Some(name) = name {
        if number.is_none() {
            let edge = format!("{name}-{}", if end { "end" } else { "start" });
            if let Some(line) = names.get(&edge).and_then(|lines| lines.first()) { return Slot::Line(*line); }
        }
        let n = number.unwrap_or(1);
        let lines = names.get(&name).map(Vec::as_slice).unwrap_or(&[]);
        let line = if n > 0 {
            lines.get(n as usize - 1).map(|line| *line as i64)
                .unwrap_or(count as i64 + 1 + n as i64 - lines.len() as i64)
        } else {
            lines.len().checked_sub(n.unsigned_abs() as usize).and_then(|i| lines.get(i)).map(|line| *line as i64)
                .unwrap_or(1 + n as i64 + lines.len() as i64)
        };
        bounded_line(line)
    } else if let Some(n) = number {
        bounded_line(if n < 0 { count as i64 + 2 + n as i64 } else { n as i64 })
    } else { Slot::Auto }
}

// Reject impossible occurrences rather than mapping huge values onto a real
// boundary. This preserves auto placement and performs arithmetic before any allocation.
fn bounded_line(line: i64) -> Slot {
    if (-(MAX_GRID_TRACKS as i64)..=MAX_GRID_TRACKS as i64 + 1).contains(&line) {
        Slot::Line(line as i32)
    } else { Slot::Auto }
}

fn span_line(anchor: i32, amount: u32, name: &Option<String>, direction: i32, count: usize,
    names: &HashMap<String, Vec<i32>>) -> i32 {
    let Some(name) = name else { return anchor + direction * amount as i32; };
    let lines = names.get(name).map(Vec::as_slice).unwrap_or(&[]);
    let mut left = amount;
    let mut line = anchor;
    // The opposite implicit side does not acquire this name during a named span search.
    for _ in 0..MAX_GRID_TRACKS * 3 {
        line += direction;
        if lines.contains(&line) || (direction > 0 && line > count as i32 + 1)
            || (direction < 0 && line < 1) {
            left -= 1;
            if left == 0 { return line; }
        }
    }
    anchor + direction * MAX_GRID_TRACKS as i32
}

pub(super) fn axis(style: &ComputedStyle, property: &str, count: usize,
    names: &HashMap<String, Vec<i32>>) -> (Option<i32>, u32) {
    let start = slot(style.get(&format!("grid-{property}-start")).unwrap_or("auto"), false, count, names);
    let mut end = slot(style.get(&format!("grid-{property}-end")).unwrap_or("auto"), true, count, names);
    if matches!(start, Slot::Span(..)) && matches!(end, Slot::Span(..)) { end = Slot::Auto; }
    match (start, end) {
        (Slot::Line(a), Slot::Line(b)) => (Some(a.min(b)), a.abs_diff(b).max(1).min(MAX_GRID_TRACKS as u32)),
        (Slot::Line(a), Slot::Span(n, name)) => (Some(a), a.abs_diff(span_line(a,n,&name,1,count,names)).max(1)),
        (Slot::Span(n, name), Slot::Line(b)) => {
            let a = span_line(b,n,&name,-1,count,names); (Some(a), a.abs_diff(b).max(1))
        },
        (Slot::Line(a), Slot::Auto) => (Some(a), 1),
        (Slot::Auto, Slot::Line(b)) => (Some(b-1), 1),
        (Slot::Span(_, Some(_)), _) | (_, Slot::Span(_, Some(_))) => (None, 1),
        (Slot::Span(n, None), _) | (_, Slot::Span(n, None)) => (None, n),
        _ => (None, 1),
    }
}

/// Preserve signed explicit-grid coordinates until the whole batch's leading
/// implicit extent is known. Auto placement then consumes ordinary positive cells.
pub(super) fn prepare(spec: &mut GridSpec, items: &[LayoutBox], context: Option<&SubgridContext>) -> Vec<Placement> {
    let raw = items.iter().map(|item| (
        axis(&item.style,"column",spec.cols.len(),&spec.col_line_names),
        axis(&item.style,"row",spec.rows.len(),&spec.row_line_names))).collect::<Vec<_>>();
    let col_origin = if context.is_some_and(|c| c.cols.is_some()) { 0 } else {
        raw.iter().filter_map(|p| p.0.0).min().map_or(0, |n| (1-n).max(0) as usize).min(MAX_GRID_TRACKS)
    };
    let row_origin = if context.is_some_and(|c| c.rows.is_some()) { 0 } else {
        raw.iter().filter_map(|p| p.1.0).min().map_or(0, |n| (1-n).max(0) as usize).min(MAX_GRID_TRACKS)
    };
    spec.explicit_cols = spec.cols.len(); spec.explicit_rows = spec.rows.len();
    spec.col_origin = col_origin; spec.row_origin = row_origin;
    prepend(&mut spec.cols, &spec.auto_cols, &mut spec.col_auto_fit, &mut spec.col_line_names, col_origin);
    prepend(&mut spec.rows, &spec.auto_rows, &mut spec.row_auto_fit, &mut spec.row_line_names, row_origin);
    raw.into_iter().map(|(col,row)| Placement {
        col: col.0.map(|n| (n + col_origin as i32).max(1) as u32), col_span: col.1,
        row: row.0.map(|n| (n + row_origin as i32).max(1) as u32), row_span: row.1,
    }).collect()
}

// Portions ported from DioxusLabs/taffy src/compute/grid/explicit_grid.rs
// @ 2c81b8e09d172c2e4d7747be2b1e5a03e0f30bf0, MIT.
// The implicit sequence preceding the explicit grid ends at the pattern's last entry.
fn prepend(tracks: &mut Vec<TrackSize>, auto: &[TrackSize], fit: &mut Vec<bool>,
    names: &mut HashMap<String, Vec<i32>>, count: usize) {
    if count == 0 { return; }
    let mut leading = (0..count).map(|i| if auto.is_empty() { TrackSize::Auto }
        else { auto[(auto.len() - count % auto.len() + i) % auto.len()] }).collect::<Vec<_>>();
    leading.append(tracks); *tracks = leading;
    if !fit.is_empty() { let mut mask = vec![false; count]; mask.append(fit); *fit = mask; }
    for lines in names.values_mut() { for line in lines { *line += count as i32; } }
}
