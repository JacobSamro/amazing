//! CSS 2.1 table grid sizing and placement. Tree repair is in table_tree;
//! this module consumes only computed styles and normalized layout boxes.
use crate::font::FontSource;
use crate::table_tree::{Role, role};
use crate::values::{self, MaybeLen, Sides};
use crate::{Flow, LayoutBox, Rect};

const MAX_COLUMNS: usize = 4096;

pub(crate) fn children(b: &LayoutBox) -> &[LayoutBox] {
    match &b.flow {
        Flow::Blocks(c) => c,
        _ => &[],
    }
}
pub(crate) fn at<'a>(mut b: &'a LayoutBox, path: &[usize]) -> &'a LayoutBox {
    for &i in path {
        b = &children(b)[i];
    }
    b
}
fn at_mut<'a>(mut b: &'a mut LayoutBox, path: &[usize]) -> &'a mut LayoutBox {
    for &i in path {
        let Flow::Blocks(c) = &mut b.flow else {
            unreachable!()
        };
        b = &mut c[i];
    }
    b
}
#[derive(Debug)]
pub(crate) struct Row {
    pub(crate) path: Vec<usize>,
    end: usize,
}
#[derive(Debug)]
pub(crate) struct Cell {
    pub(crate) path: Vec<usize>,
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
}
pub(crate) struct Grid {
    pub(crate) rows: Vec<Row>,
    pub(crate) cells: Vec<Cell>,
    pub(crate) columns: Vec<Vec<usize>>,
    pub(crate) count: usize,
}

impl Grid {
    fn new(b: &LayoutBox) -> Self {
        let mut rows = Vec::new();
        let mut columns = Vec::new();
        let mut order: Vec<usize> = (0..children(b).len()).collect();
        let header = order
            .iter()
            .copied()
            .find(|&i| role(&children(b)[i]) == Role::Header);
        let footer = order
            .iter()
            .copied()
            .find(|&i| role(&children(b)[i]) == Role::Footer);
        order.sort_by_key(|i| {
            if Some(*i) == header {
                0
            } else if Some(*i) == footer {
                2
            } else {
                1
            }
        });
        for i in order {
            let child = &children(b)[i];
            if role(child).group() {
                let start = rows.len();
                for j in 0..children(child).len() {
                    if role(&children(child)[j]) == Role::Row {
                        rows.push(Row {
                            path: vec![i, j],
                            end: 0,
                        });
                    }
                }
                let end = rows.len();
                for row in &mut rows[start..] {
                    row.end = end;
                }
            } else if role(child) == Role::Row {
                rows.push(Row {
                    path: vec![i],
                    end: 0,
                });
            } else if role(child) == Role::Column {
                for _ in 0..child.table_colspan.min(MAX_COLUMNS - columns.len()) {
                    columns.push(vec![i]);
                }
            } else if role(child) == Role::ColumnGroup {
                if children(child).is_empty() {
                    for _ in 0..child.table_colspan.min(MAX_COLUMNS - columns.len()) {
                        columns.push(vec![i]);
                    }
                } else {
                    for (j, col) in children(child).iter().enumerate() {
                        for _ in 0..col.table_colspan.min(MAX_COLUMNS - columns.len()) {
                            columns.push(vec![i, j]);
                        }
                    }
                }
            }
        }
        let row_count = rows.len();
        for i in 0..row_count {
            if rows[i].end == 0 {
                // Direct rows form an implicit group; their spans cannot enter
                // the next explicit row group (including a reordered footer).
                rows[i].end = (i + 1..row_count)
                    .find(|&j| rows[j].path.len() == 2)
                    .unwrap_or(row_count);
            }
        }
        let mut occupied_until = vec![0; MAX_COLUMNS];
        let mut cells = Vec::new();
        let mut count = 0;
        for (r, row) in rows.iter().enumerate() {
            let mut col = 0;
            for (i, cell) in children(at(b, &row.path)).iter().enumerate() {
                while col < MAX_COLUMNS && occupied_until[col] > r {
                    col += 1;
                }
                if col == MAX_COLUMNS {
                    break;
                }
                let cols = cell.table_colspan.min(MAX_COLUMNS - col).max(1);
                let span = if cell.table_rowspan == 0 {
                    row.end - r
                } else {
                    cell.table_rowspan
                };
                let span = span.min(row.end - r).max(1);
                // §17.5 permits overlap when a colspan crosses a prior rowspan.
                // Keep the HTML source-order placement, never move an earlier cell.
                for slot in &mut occupied_until[col..col + cols] {
                    *slot = (*slot).max(r + span);
                }
                let mut path = row.path.clone();
                path.push(i);
                cells.push(Cell {
                    path,
                    row: r,
                    col,
                    rows: span,
                    cols,
                });
                col += cols;
                count = count.max(col);
            }
        }
        // css-tables-3 #missing-cells-fixup: a `<colgroup>`/`<col>` beyond
        // the last column any real cell occupies only forces a "missing
        // cell" placeholder column to exist when that column itself has a
        // definite (absolute-length) `width` or `min-width` -- a `%`/
        // `calc()`-with-percentage size, or no size at all, does not, and
        // that trailing column is dropped as if the colgroup had never
        // declared it (`col-definite-min-size-001.html`: tables with
        // `min-width: 50%`/`calc(100px + 1%)`/unset collapse to only the
        // columns real cells use, while `min-width: 100px` keeps all four).
        // Scanned from the end so a definite column past a non-definite one
        // still extends the count, matching ordinary trailing-trim.
        count = count.max(
            columns
                .iter()
                .enumerate()
                .skip(count)
                .filter(|(_, path)| column_has_definite_size(at(b, path)))
                .map(|(i, _)| i + 1)
                .max()
                .unwrap_or(count),
        );
        // Every other reader of `g.columns` (constraints(), decoration,
        // colgroup border painting) indexes parallel per-column vecs of
        // length `g.count` by the same `i` from `g.columns.iter().enumerate()`
        // -- `columns.len() == count` is a load-bearing invariant, not just
        // the common case, so a dropped trailing non-definite column must be
        // removed from `columns` itself here, not just excluded from `count`.
        columns.truncate(count);
        Self {
            rows,
            cells,
            columns,
            count,
        }
    }
}
fn length(b: &LayoutBox, name: &str, basis: f32) -> Option<f32> {
    match values::resolve_extent(&b.style, name, b.font_size, basis) {
        MaybeLen::Px(v) => Some(v.max(0.0)),
        MaybeLen::Auto => None,
    }
}
/// True when a `<col>`/`<colgroup>` has a definite (absolute-length)
/// `width` or `min-width` -- a bare number/px, not a percentage or a
/// `calc()` that contains one. Resolving against basis `0.0` and checking
/// for `%` in the raw source (rather than trusting `MaybeLen::Px` alone,
/// which a percentage still produces once resolved against *some* basis)
/// is what distinguishes "genuinely definite" from "resolves to a number
/// only because there was a basis to resolve against" for this
/// structural, pre-layout column-count decision.
fn column_has_definite_size(col: &LayoutBox) -> bool {
    ["width", "min-width"].into_iter().any(|prop| {
        col.style
            .get(prop)
            .is_some_and(|raw| !raw.contains('%') && length(col, prop, 0.0).is_some())
    })
}
fn spacing(b: &LayoutBox) -> (f32, f32) {
    if collapsed(b) {
        return (0.0, 0.0);
    }
    let mut parts = b
        .style
        .get("border-spacing")
        .unwrap_or("0")
        .split_whitespace();
    let x = parts
        .next()
        .and_then(|v| values::used_length_percentage(&b.style, v, b.font_size, b.font_size * 0.6))
        .and_then(|length| length.resolve(Some(0.0)))
        .unwrap_or(0.0)
        .max(0.0);
    let y = parts
        .next()
        .and_then(|v| values::used_length_percentage(&b.style, v, b.font_size, b.font_size * 0.6))
        .and_then(|length| length.resolve(Some(0.0)))
        .unwrap_or(x)
        .max(0.0);
    (x, y)
}
fn collapsed(b: &LayoutBox) -> bool {
    b.style
        .get("border-collapse")
        .is_some_and(|s| s.eq_ignore_ascii_case("collapse"))
}
fn chrome(b: &LayoutBox, basis: f32) -> f32 {
    let p = values::resolve_padding(&b.style, b.font_size, basis);
    let e = values::resolve_borders(&b.style, b.font_size);
    p.left + p.right + e.left + e.right
}
/// Convert a cell's specified size to its outer size. Track sizes already
/// denote track breadths, so this adjustment applies only to cells.
fn outer_cell_size(cell: &LayoutBox, size: f32, edges: f32) -> f32 {
    if cell.style.get("box-sizing") == Some("border-box") {
        size.max(edges)
    } else {
        size + edges
    }
}

// CSS2 §17.6.1 distinguishes HTML table widths from CSS table boxes on
// ordinary elements. Only the latter add content-box padding and borders.
fn outer_table_width(b: &LayoutBox, size: f32, edges: f32) -> f32 {
    if b.html_table_width || b.style.get("box-sizing") == Some("border-box") {
        size
    } else {
        size + edges
    }
}

fn column_width<'a>(b: &'a LayoutBox, path: &[usize]) -> &'a LayoutBox {
    let col = at(b, path);
    if path.len() == 2 && col.style.get("width").is_none_or(|w| w == "auto") {
        at(b, &path[..1])
    } else {
        col
    }
}

/// CSS Tables 3 §3.9.3 fixed mode: preserve absolute contributions, reduce
/// percentages when they overcommit the assignable space, then give remaining
/// space to auto columns (or proportionally to specified columns).
fn fixed_widths(
    b: &LayoutBox,
    g: &Grid,
    assignable: f32,
    sx: f32,
    borders: Option<&crate::table_decoration::Borders>,
) -> Vec<f32> {
    let mut absolute = vec![0.0f32; g.count];
    let mut percent = absolute.clone();
    let mut known = vec![false; g.count];
    for (i, path) in g.columns.iter().enumerate() {
        let col = column_width(b, path);
        if let Some(w) =
            values::style_length_percentage(&col.style, "width", col.font_size, col.font_size * 0.6)
        {
            absolute[i] = w.px;
            percent[i] = w.percentage / 100.0;
            known[i] = true;
        }
    }
    for c in g.cells.iter().filter(|c| c.row == 0) {
        let cell = at(b, &c.path);
        let Some(w) = values::style_length_percentage(
            &cell.style,
            "width",
            cell.font_size,
            cell.font_size * 0.6,
        ) else {
            continue;
        };
        let missing = known[c.col..c.col + c.cols].iter().filter(|&&v| !v).count();
        if missing == 0 {
            continue;
        }
        let edges = if let Some(borders) = borders {
            let e = borders.widths(c.row, c.col, c.rows, c.cols);
            let p = values::resolve_padding(&cell.style, cell.font_size, assignable);
            e.left + e.right + p.left + p.right
        } else {
            chrome(cell, assignable)
        };
        // Keep offsets in the absolute contribution even for percentages:
        // CSS2 fixed-table-layout-026 requires a 50% content width plus its
        // borders to fill the table. Mixed calc terms remain separate so an
        // overcommitted percentage cannot shrink those offsets away.
        let required = if w.has_percentage {
            w.px + if cell.style.get("box-sizing") == Some("border-box") {
                0.0
            } else {
                edges
            }
        } else {
            outer_cell_size(cell, w.px.max(0.0), edges)
        };
        let present: f32 = absolute[c.col..c.col + c.cols].iter().sum();
        let present_percent: f32 = percent[c.col..c.col + c.cols].iter().sum();
        for i in c.col..c.col + c.cols {
            if !known[i] {
                absolute[i] = (required - sx * (c.cols - 1) as f32 - present) / missing as f32;
                percent[i] = (w.percentage / 100.0 - present_percent) / missing as f32;
                known[i] = true;
            }
        }
    }
    let px: f32 = absolute.iter().sum();
    let pct: f32 = percent.iter().sum();
    let pct_basis = if pct > 0.0 {
        assignable.min((assignable - px).max(0.0) / pct)
    } else {
        assignable
    };
    let mut widths: Vec<f32> = absolute
        .iter()
        .zip(&percent)
        .map(|(a, p)| (a + p * pct_basis).max(0.0))
        .collect();
    let extra = (assignable - widths.iter().sum::<f32>()).max(0.0);
    let missing = known.iter().filter(|&&v| !v).count();
    let positive_px: f32 = absolute.iter().map(|v| v.max(0.0)).sum();
    let positive_pct: f32 = percent.iter().map(|v| v.max(0.0)).sum();
    for i in 0..g.count {
        widths[i] += if missing > 0 {
            if known[i] {
                0.0
            } else {
                extra / missing as f32
            }
        } else if positive_px > 0.0 {
            extra * absolute[i].max(0.0) / positive_px
        } else if positive_pct > 0.0 {
            extra * percent[i].max(0.0) / positive_pct
        } else {
            extra / g.count as f32
        };
    }
    widths
}

fn percentage_contribution(b: &LayoutBox) -> f32 {
    let percentage = |name| {
        values::style_length_percentage(&b.style, name, b.font_size, b.font_size * 0.6)
            .filter(|v| v.has_percentage && v.px == 0.0)
            .map(|v| v.percentage.max(0.0) / 100.0)
    };
    percentage("width")
        .unwrap_or(0.0)
        .min(percentage("max-width").unwrap_or(f32::INFINITY))
}

fn intrinsic_limit(b: &LayoutBox, name: &str) -> Option<f32> {
    values::style_length_percentage(&b.style, name, b.font_size, b.font_size * 0.6)
        .filter(|v| !v.has_percentage)
        .map(|v| v.px.max(0.0))
}

/// Keep column kinds separate from intrinsic widths: percentages are targets,
/// not pixel minima, and unconstrained columns receive excess space first.
fn auto_widths(b: &LayoutBox, g: &Grid, min: &[f32], max: &[f32], available: f32) -> Vec<f32> {
    let mut percent = vec![0.0f32; g.count];
    let mut constrained = vec![false; g.count];
    let mut originating = vec![false; g.count];
    let mut record = |i: usize, cell: &LayoutBox, span: usize| {
        if let Some(w) = values::style_length_percentage(
            &cell.style,
            "width",
            cell.font_size,
            cell.font_size * 0.6,
        ) {
            percent[i] = percent[i].max(percentage_contribution(cell) / span as f32);
            constrained[i] |= !w.has_percentage;
        }
    };
    for (i, path) in g.columns.iter().enumerate() {
        record(i, at(b, path), 1);
        if path.len() == 2 {
            record(i, at(b, &path[..1]), 1);
        }
    }
    for c in &g.cells {
        originating[c.col] = true;
        if c.cols == 1 {
            record(c.col, at(b, &c.path), 1);
        }
    }
    // Process one colspan size at a time using the previous span's values.
    // Existing percentage columns keep their share; only zero-percentage
    // columns accept a spanning cell's remaining contribution.
    let mut spans: Vec<_> = g.cells.iter().filter(|c| c.cols > 1).collect();
    spans.sort_by_key(|c| c.cols);
    let mut start = 0;
    while start < spans.len() {
        let count = spans[start].cols;
        let end = start
            + spans[start..]
                .iter()
                .take_while(|c| c.cols == count)
                .count();
        let previous = percent.clone();
        for c in &spans[start..end] {
            let cell = at(b, &c.path);
            if values::style_length_percentage(
                &cell.style,
                "width",
                cell.font_size,
                cell.font_size * 0.6,
            )
            .is_some()
            {
                let present: f32 = previous[c.col..c.col + c.cols].iter().sum();
                let extra = (percentage_contribution(cell) - present).max(0.0);
                let eligible: Vec<_> = (c.col..c.col + c.cols)
                    .filter(|&i| previous[i] == 0.0)
                    .collect();
                let weight: f32 = eligible.iter().map(|&i| max[i]).sum();
                for &i in &eligible {
                    let ratio = if weight > 0.0 {
                        max[i] / weight
                    } else {
                        1.0 / eligible.len() as f32
                    };
                    percent[i] = percent[i].max(extra * ratio);
                }
            }
        }
        start = end;
    }
    let mut remaining = 1.0f32;
    for p in &mut percent {
        *p = p.min(remaining);
        remaining -= *p;
    }
    let mut widths = min.to_vec();
    // Interpolate consecutive min-content, percentage, specified, and
    // max-content guesses. This preserves the priority of percentage columns.
    for stage in 0..3 {
        let next: Vec<f32> = (0..g.count)
            .map(|i| {
                if percent[i] > 0.0 {
                    min[i].max(percent[i] * available)
                } else if stage == 2 || (stage == 1 && constrained[i]) {
                    max[i]
                } else {
                    min[i]
                }
            })
            .collect();
        let current: f32 = widths.iter().sum();
        let target: f32 = next.iter().sum();
        if target > available && target > current {
            let fraction = ((available - current) / (target - current)).clamp(0.0, 1.0);
            for i in 0..g.count {
                widths[i] += (next[i] - widths[i]) * fraction;
            }
            return widths;
        }
        widths = next;
    }
    let extra = (available - widths.iter().sum::<f32>()).max(0.0);
    let mut weights = vec![0.0f32; g.count];
    for rule in 0..6 {
        for i in 0..g.count {
            weights[i] = match rule {
                0 if !constrained[i] && percent[i] == 0.0 && originating[i] => max[i],
                1 if !constrained[i] && percent[i] == 0.0 && originating[i] => 1.0,
                2 if constrained[i] && percent[i] == 0.0 => max[i],
                3 => percent[i],
                4 if originating[i] => 1.0,
                5 => 1.0,
                _ => 0.0,
            };
        }
        let total: f32 = weights.iter().sum();
        if total > 0.0 {
            for i in 0..g.count {
                widths[i] += extra * weights[i] / total;
            }
            break;
        }
    }
    widths
}

fn distribute_min(widths: &mut [f32], start: usize, count: usize, required: f32) {
    let current: f32 = widths[start..start + count].iter().sum();
    let extra = (required - current).max(0.0) / count as f32;
    for v in &mut widths[start..start + count] {
        *v += extra;
    }
}
fn constraints(
    b: &LayoutBox,
    g: &Grid,
    fonts: &FontSource,
    basis: f32,
    text: Option<&dyn amazing_text::TextSystem>,
) -> (Vec<f32>, Vec<f32>) {
    let mut min = vec![0.0f32; g.count];
    let mut max = min.clone();
    let borders = collapsed(b).then(|| crate::table_decoration::Borders::new(b, g));
    let sx = spacing(b).0;
    for (i, path) in g.columns.iter().enumerate() {
        let col = at(b, path);
        // css-tables-3 #computing-column-measures: a `<col>`/`<colgroup>`'s
        // own used width is `width` clamped by its own `min-width`/
        // `max-width` -- checked on both the `<col>` and its owning
        // `<colgroup>` for each property independently (a `<col>` can
        // declare `width` while its `<colgroup>` declares `min-width`, and
        // vice versa), matching how `width` alone already falls back to the
        // colgroup. Only a definite (non-percentage) `min-width`/`max-width`
        // participates (col-definite-min-size-001.html /
        // col-definite-max-size-001.html use `%` and `calc()`-with-`%`
        // variants specifically to probe this); a percentage `width` still
        // resolves through `length()`'s own basis as before. Applying
        // `max-width` first and `min-width` second means `min-width` wins
        // when the two conflict (col-definite-max-size-001.html's
        // `max-width:0; min-width:100px` case), same precedence CSS box
        // sizing uses everywhere else.
        let col_or_group_limit = |prop| {
            intrinsic_limit(col, prop).or_else(|| {
                (path.len() == 2)
                    .then(|| intrinsic_limit(at(b, &path[..1]), prop))
                    .flatten()
            })
        };
        let mut width = length(col, "width", basis)
            .or_else(|| {
                (path.len() == 2)
                    .then(|| length(at(b, &path[..1]), "width", basis))
                    .flatten()
            })
            .unwrap_or(0.0);
        if let Some(max_width) = col_or_group_limit("max-width") {
            width = width.min(max_width);
        }
        if let Some(min_width) = col_or_group_limit("min-width") {
            width = width.max(min_width);
        }
        min[i] = width;
        max[i] = width;
    }
    let mut cells: Vec<_> = g.cells.iter().collect();
    cells.sort_by_key(|c| c.cols);
    for c in cells {
        let cell = at(b, &c.path);
        let edges = if let Some(borders) = &borders {
            let p = values::resolve_padding(&cell.style, cell.font_size, basis);
            let e = borders.widths(c.row, c.col, c.rows, c.cols);
            p.left + p.right + e.left + e.right
        } else {
            chrome(cell, basis)
        };
        let explicit = length(cell, "width", basis)
            .map(|w| outer_cell_size(cell, w, edges))
            .unwrap_or(edges);
        let (inner_min, inner_max) = content_widths(cell, fonts, text);
        let minimum = intrinsic_limit(cell, "min-width")
            .map(|w| outer_cell_size(cell, w, edges))
            .unwrap_or(edges);
        let maximum = intrinsic_limit(cell, "max-width")
            .map(|w| outer_cell_size(cell, w, edges))
            .unwrap_or(f32::INFINITY);
        // A scroll container can accommodate content wider than its specified
        // maximum without enlarging the column. Preserve that maximum in the
        // intrinsic pass, before materializing the assigned content width.
        let scrolls = matches!(
            cell.style
                .get("overflow-x")
                .or_else(|| cell.style.get("overflow")),
            Some("auto" | "scroll" | "hidden")
        );
        let content_min = if scrolls {
            (inner_min + edges).min(maximum)
        } else {
            inner_min + edges
        };
        let mn = content_min.max(explicit).max(minimum);
        let mx = (inner_max + edges).min(maximum).max(mn);
        distribute_min(
            &mut min,
            c.col,
            c.cols,
            (mn - sx * (c.cols - 1) as f32).max(0.0),
        );
        distribute_min(
            &mut max,
            c.col,
            c.cols,
            (mx - sx * (c.cols - 1) as f32).max(0.0),
        );
    }
    for i in 0..g.count {
        max[i] = max[i].max(min[i]);
    }
    (min, max)
}

pub(crate) fn intrinsic(b: &LayoutBox, fonts: &FontSource) -> (f32, f32) {
    intrinsic_with_text(b, fonts, None)
}

/// Like [`intrinsic`], but threads real shaped text through when available.
/// `intrinsic`'s hardcoded `text: None` measures every cell with fallback
/// glyph-width heuristics; that estimate can diverge sharply from the same
/// table's own live [`constraints`] call inside [`layout`] (which always
/// receives real `text`), so a shrink-to-fit box sized from `intrinsic`
/// alone can resolve narrower than the table actually needs once real
/// shaping runs, squeezing columns below their true max-content width
/// (LAYOUT-24). Callers that already have `text` in scope — e.g. an abspos
/// table wrapper's shrink-to-fit computation — should call this instead.
pub(crate) fn intrinsic_with_text(
    b: &LayoutBox,
    fonts: &FontSource,
    text: Option<&dyn amazing_text::TextSystem>,
) -> (f32, f32) {
    let g = Grid::new(b);
    let (min, max) = constraints(b, &g, fonts, 0.0, text);
    let model_edges = if collapsed(b) {
        let edges =
            crate::table_decoration::Borders::new(b, &g).widths(0, 0, g.rows.len(), g.count);
        edges.left + edges.right
    } else {
        chrome(b, 0.0)
    };
    let edges = model_edges
        + if g.count > 0 {
            spacing(b).0 * (g.count + 1) as f32
        } else {
            0.0
        };
    let specified = length(b, "width", 0.0)
        .map(|w| outer_table_width(b, w, model_edges))
        .unwrap_or(0.0);
    (
        (min.iter().sum::<f32>() + edges).max(specified),
        (max.iter().sum::<f32>() + edges).max(specified),
    )
}

pub(crate) fn layout(
    b: &mut LayoutBox,
    x: f32,
    y: f32,
    cbw: f32,
    fonts: &FontSource,
    text: Option<&dyn amazing_text::TextSystem>,
    caption_min: f32,
) -> f32 {
    let g = Grid::new(b);
    let (sx, sy) = spacing(b);
    b.margin = values::resolve_margins(&b.style, b.font_size, cbw);
    b.padding = if collapsed(b) {
        Sides::default()
    } else {
        values::resolve_padding(&b.style, b.font_size, cbw)
    };
    let borders = collapsed(b).then(|| crate::table_decoration::Borders::new(b, &g));
    b.border = borders.as_ref().map_or_else(
        || values::resolve_borders(&b.style, b.font_size),
        |v| v.widths(0, 0, g.rows.len(), g.count),
    );
    b.table_decoration = Default::default();
    b.table_decoration.suppress_border = borders.is_some();
    let edges = b.padding.left + b.padding.right + b.border.left + b.border.right;
    let gaps = if g.count > 0 {
        sx * (g.count + 1) as f32
    } else {
        0.0
    };
    let specified = length(b, "width", cbw).map(|w| outer_table_width(b, w, edges));
    let fixed = specified.is_some()
        && b.style
            .get("table-layout")
            .is_some_and(|s| s.eq_ignore_ascii_case("fixed"));
    let available = (cbw - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0);
    let (min, max) = if fixed {
        (vec![0.0; g.count], vec![0.0; g.count])
    } else {
        constraints(b, &g, fonts, 0.0, text)
    };
    let capmin = children(b)
        .iter()
        .filter(|c| role(c) == Role::Caption)
        .map(|c| crate::flex::min_content_width(c, fonts))
        .fold(0.0, f32::max);
    // Intrinsic cell constraints include cell padding/borders, but exclude
    // table padding, borders and inter-cell border spacing.
    let mn = min.iter().sum::<f32>() + edges + gaps;
    let mx = max.iter().sum::<f32>() + edges + gaps;
    let mut width = if fixed {
        specified.unwrap()
    } else {
        specified.unwrap_or(available.min(mx)).max(mn)
    }
    .max(capmin)
    .max(caption_min)
    .max(
        length(b, "min-width", cbw)
            .map(|w| outer_table_width(b, w, edges))
            .unwrap_or(0.0),
    );
    let widths = if fixed {
        fixed_widths(b, &g, (width - edges - gaps).max(0.0), sx, borders.as_ref())
    } else {
        auto_widths(b, &g, &min, &max, (width - edges - gaps).max(0.0))
    };
    if fixed {
        width = width.max(widths.iter().sum::<f32>() + edges + gaps);
    }
    let free =
        (cbw - width - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0);
    match (b.margin.left.is_none(), b.margin.right.is_none()) {
        (true, true) => {
            b.margin.left = Some(free / 2.0);
            b.margin.right = Some(free / 2.0);
        }
        (true, false) => b.margin.left = Some(free),
        (false, true) => b.margin.right = Some(free),
        _ => {}
    }
    let bx = x + b.margin.left.unwrap_or(0.0);
    let by = y + b.margin.top.unwrap_or(0.0);
    let cx = bx + b.border.left + b.padding.left;
    let mut cy = by + b.border.top + b.padding.top;
    // Captions are laid out in document order on their selected side.
    let captions: Vec<usize> = children(b)
        .iter()
        .enumerate()
        .filter(|(_, c)| role(c) == Role::Caption)
        .map(|(i, _)| i)
        .collect();
    for &i in &captions {
        if !at(b, &[i])
            .style
            .get("caption-side")
            .is_some_and(|s| s.eq_ignore_ascii_case("bottom"))
        {
            cy += crate::block::layout_table_content(at_mut(b, &[i]), bx, cy, width, fonts, text);
        }
    }
    let grid_y = cy;
    let mut heights = vec![0.0f32; g.rows.len()];
    let mut baselines = heights.clone();
    let mut natural = Vec::new();
    let mut minimum_heights = Vec::new();
    let mut cell_baselines = Vec::new();
    for (r, row) in g.rows.iter().enumerate() {
        heights[r] = length(at(b, &row.path), "height", 0.0).unwrap_or(0.0);
    }
    let rtl = b
        .style
        .get("direction")
        .is_some_and(|s| s.eq_ignore_ascii_case("rtl"));
    for c in &g.cells {
        let w = widths[c.col..c.col + c.cols].iter().sum::<f32>() + sx * (c.cols - 1) as f32;
        let offset = sx + widths[..c.col].iter().sum::<f32>() + sx * c.col as f32;
        let cell_x = if rtl {
            cx + width - edges - offset - w
        } else {
            cx + offset
        };
        let cell = at_mut(b, &c.path);
        let minimum_height = length(cell, "height", 0.0).unwrap_or(0.0);
        let original = cell.style.clone();
        cell.style.set("height", "auto");
        cell.table_decoration = Default::default();
        if let Some(borders) = &borders {
            let e = borders.widths(c.row, c.col, c.rows, c.cols);
            for (side, width) in [
                ("top", e.top),
                ("right", e.right),
                ("bottom", e.bottom),
                ("left", e.left),
            ] {
                cell.style
                    .set(&format!("border-{side}-width"), &format!("{width}px"));
                cell.style.set(&format!("border-{side}-style"), "solid");
            }
            cell.table_decoration.suppress_border = true;
        }
        cell.style.set(
            "width",
            &format!("{}px", (w - chrome(cell, width)).max(0.0)),
        );
        // The column algorithm has already applied outer min/max constraints.
        // Hand block layout a content size, independent of the author's box
        // sizing, so a second constraint pass cannot widen the assigned track.
        cell.style.set("box-sizing", "content-box");
        cell.style.set("min-width", "0");
        cell.style.set("max-width", "none");
        for name in ["margin-top", "margin-right", "margin-bottom", "margin-left"] {
            cell.style.set(name, "0");
        }
        crate::block::layout_table_content(cell, cell_x, 0.0, w, fonts, text);
        cell.style = original;
        // Fixed tracks can be narrower than cell padding/borders. Their
        // decoration box still occupies the assigned track; block layout's
        // nonnegative content width must not expand it into the next column.
        cell.border_box.w = w;
        let h = cell.border_box.h;
        let baseline = cell_baseline(cell) - cell.border_box.y;
        let align = cell.style.get("vertical-align").unwrap_or("baseline");
        if !matches!(align, "top" | "middle" | "bottom") {
            baselines[c.row] = baselines[c.row].max(baseline);
        }
        if c.rows == 1 {
            heights[c.row] = heights[c.row].max(h).max(
                minimum_height
                    + cell.padding.top
                    + cell.padding.bottom
                    + cell.border.top
                    + cell.border.bottom,
            );
        }
        minimum_heights.push(
            minimum_height
                + cell.padding.top
                + cell.padding.bottom
                + cell.border.top
                + cell.border.bottom,
        );
        natural.push(h);
        cell_baselines.push(baseline);
    }
    for (i, c) in g.cells.iter().enumerate() {
        let align = at(b, &c.path)
            .style
            .get("vertical-align")
            .unwrap_or("baseline");
        let adjustment = if !matches!(align, "top" | "middle" | "bottom") {
            (baselines[c.row] - cell_baselines[i]).max(0.0)
        } else {
            0.0
        };
        distribute_min(
            &mut heights,
            c.row,
            c.rows,
            ((natural[i] + adjustment).max(minimum_heights[i]) - sy * (c.rows - 1) as f32).max(0.0),
        );
    }
    // A table's `min-height`/`max-height` apply to its WRAPPER box (the
    // padding/border edge), not the grid box being sized here, so both
    // convert into a grid-box target by subtracting the table's own
    // vertical padding+border (css-tables/#computing-the-table-height).
    // `min-height` can grow the grid past its intrinsic row heights (the
    // `desired`/`extra` distribution below already does that for an
    // explicit `height`; min-height is the same mechanism). `max-height`
    // is only applied here as a lower cap on this target, never by
    // shrinking already-natural-sized rows -- matching every other
    // min/max clamp in this function, min always wins when it exceeds
    // max (`min-height-table-2.html`'s "min trumps max, like everywhere
    // else").
    let cbh = b.containing_block_height.unwrap_or(0.0);
    let vertical_edges = b.padding.top + b.padding.bottom + b.border.top + b.border.bottom;
    let mut desired = length(b, "height", cbh).unwrap_or(0.0);
    if let Some(max_h) = length(b, "max-height", cbh) {
        desired = desired.min((max_h - vertical_edges).max(0.0));
    }
    if let Some(min_h) = length(b, "min-height", cbh) {
        desired = desired.max((min_h - vertical_edges).max(0.0));
    }
    let grid_height = heights.iter().sum::<f32>()
        + if heights.is_empty() {
            0.0
        } else {
            sy * (heights.len() + 1) as f32
        };
    if !heights.is_empty() {
        let extra = (desired - grid_height).max(0.0) / heights.len() as f32;
        for h in &mut heights {
            *h += extra;
        }
    }
    let mut ys = vec![grid_y + sy];
    for &h in &heights {
        ys.push(ys.last().unwrap() + h + sy);
    }
    for (r, row) in g.rows.iter().enumerate() {
        let rect = Rect {
            x: cx + sx,
            y: ys[r],
            w: (width - edges - 2.0 * sx).max(0.0),
            h: heights[r],
        };
        let row = at_mut(b, &row.path);
        row.border_box = rect;
        row.content_box = rect;
        row.margin = Default::default();
        row.border = Default::default();
        row.padding = Default::default();
    }
    for (i, c) in g.cells.iter().enumerate() {
        let cell = at_mut(b, &c.path);
        crate::flex::translate(cell, 0.0, ys[c.row] - cell.border_box.y);
        let h = heights[c.row..c.row + c.rows].iter().sum::<f32>() + sy * (c.rows - 1) as f32;
        let extra = (h - natural[i]).max(0.0);
        let dy = match cell.style.get("vertical-align").unwrap_or("baseline") {
            "top" => 0.0,
            "middle" => extra / 2.0,
            "bottom" => extra,
            _ => (baselines[c.row] - cell_baselines[i]).max(0.0),
        };
        // Extra space is padding, not a larger content area (percentages in
        // descendants must not gain a definite containing block).
        crate::flex::translate(cell, 0.0, dy);
        cell.border_box.y -= dy;
        cell.border_box.h = h;
        cell.padding.top += dy;
        cell.padding.bottom += (extra - dy).max(0.0);
    }
    let table_height = (heights.iter().sum::<f32>()
        + if heights.is_empty() {
            0.0
        } else {
            sy * (heights.len() + 1) as f32
        })
    .max(desired);
    // Row groups and columns retain their real boxes for paint/hit traversal.
    for i in 0..children(b).len() {
        if role(at(b, &[i])).group() {
            let members: Vec<_> = g
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.path.len() == 2 && r.path[0] == i)
                .map(|(r, _)| r)
                .collect();
            if let (Some(&start), Some(&end)) = (members.first(), members.last()) {
                let rect = Rect {
                    x: cx + sx,
                    y: ys[start],
                    w: (width - edges - 2.0 * sx).max(0.0),
                    h: ys[end] + heights[end] - ys[start],
                };
                let group = at_mut(b, &[i]);
                group.border_box = rect;
                group.content_box = rect;
            }
        }
    }
    for (i, path) in g.columns.iter().enumerate() {
        if i > 0 && g.columns[i - 1] == *path {
            continue;
        }
        let span = g.columns[i..].iter().take_while(|p| *p == path).count();
        let w = widths[i..i + span].iter().sum::<f32>() + sx * (span - 1) as f32;
        let offset = sx + widths[..i].iter().sum::<f32>() + sx * i as f32;
        let rect = Rect {
            x: if rtl {
                cx + width - edges - offset - w
            } else {
                cx + offset
            },
            y: grid_y + sy,
            w,
            h: (table_height - 2.0 * sy).max(0.0),
        };
        let col = at_mut(b, path);
        col.border_box = rect;
        col.content_box = rect;
    }
    for i in 0..children(b).len() {
        if role(at(b, &[i])) == Role::ColumnGroup && !children(at(b, &[i])).is_empty() {
            let rects: Vec<_> = children(at(b, &[i])).iter().map(|c| c.border_box).collect();
            let left = rects.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);
            let right = rects
                .iter()
                .map(|r| r.x + r.w)
                .fold(f32::NEG_INFINITY, f32::max);
            let rect = Rect {
                x: left,
                y: grid_y + sy,
                w: (right - left).max(0.0),
                h: (table_height - 2.0 * sy).max(0.0),
            };
            let group = at_mut(b, &[i]);
            group.border_box = rect;
            group.content_box = rect;
        }
    }
    if let Some(borders) = &borders {
        let mut xs = vec![if rtl { cx + width - edges } else { cx }];
        for &w in &widths {
            xs.push(xs.last().unwrap() + if rtl { -w } else { w });
        }
        b.table_decoration.borders = borders.segments(&xs, &ys);
    }
    let mut holes = Vec::new();
    if borders.is_none() {
        for c in &g.cells {
            let cell = at_mut(b, &c.path);
            if cell
                .style
                .get("empty-cells")
                .is_some_and(|s| s.eq_ignore_ascii_case("hide"))
                && empty(cell)
            {
                cell.table_decoration.suppress_border = true;
                cell.table_decoration.suppress_background = true;
                holes.push(cell.border_box);
            }
        }
    }
    for child in match &mut b.flow {
        Flow::Blocks(c) => c.as_mut_slice(),
        _ => &mut [],
    } {
        if matches!(
            role(child),
            Role::Column
                | Role::ColumnGroup
                | Role::Row
                | Role::RowGroup
                | Role::Header
                | Role::Footer
        ) {
            suppress_internal(child, &holes);
        }
    }
    cy = grid_y + table_height;
    for &i in &captions {
        if at(b, &[i])
            .style
            .get("caption-side")
            .is_some_and(|s| s.eq_ignore_ascii_case("bottom"))
        {
            cy += crate::block::layout_table_content(at_mut(b, &[i]), bx, cy, width, fonts, text);
        }
    }
    b.content_box = Rect {
        x: cx,
        y: by + b.border.top + b.padding.top,
        w: (width - edges).max(0.0),
        h: cy - (by + b.border.top + b.padding.top),
    };
    b.border_box = Rect {
        x: bx,
        y: by,
        w: width,
        h: cy - by + b.padding.bottom + b.border.bottom,
    };
    b.lines.clear();
    b.border_box.h + b.margin.top.unwrap_or(0.0) + b.margin.bottom.unwrap_or(0.0)
}
fn cell_baseline(cell: &LayoutBox) -> f32 {
    first_baseline(cell).unwrap_or_else(|| {
        // Without a line box, the cell synthesizes its baseline at the bottom
        // content edge. Its specified minimum height still participates even
        // though row-height excess is represented separately as padding.
        cell.content_box.y
            + cell
                .content_box
                .h
                .max(length(cell, "height", 0.0).unwrap_or(0.0))
    })
}

fn first_baseline(b: &LayoutBox) -> Option<f32> {
    b.lines
        .first()
        .map(|l| l.baseline)
        .or_else(|| children(b).iter().find_map(first_baseline))
}

fn empty(b: &LayoutBox) -> bool {
    if b.replaced_size.is_some() {
        return false;
    }
    match &b.flow {
        Flow::Empty => true,
        Flow::Inlines(items) => items.iter().all(|i| match i {
            crate::InlineNode::Text { text, style, .. } => {
                if matches!(
                    style.white_space,
                    amazing_text::WhiteSpace::Pre
                        | amazing_text::WhiteSpace::PreWrap
                        | amazing_text::WhiteSpace::BreakSpaces
                ) {
                    text.is_empty()
                } else {
                    text.trim().is_empty()
                }
            }
            _ => false,
        }),
        Flow::Blocks(c) | Flow::Flex(c) | Flow::Grid(c) => c.is_empty(),
    }
}
fn suppress_internal(b: &mut LayoutBox, holes: &[Rect]) {
    b.table_decoration.suppress_border = true;
    b.table_decoration.background_holes = holes
        .iter()
        .copied()
        .filter(|r| {
            r.x < b.border_box.x + b.border_box.w
                && r.x + r.w > b.border_box.x
                && r.y < b.border_box.y + b.border_box.h
                && r.y + r.h > b.border_box.y
        })
        .collect();
    if let Flow::Blocks(c) = &mut b.flow {
        for child in c {
            if role(child) != Role::Cell {
                suppress_internal(child, holes);
            }
        }
    }
}

pub(crate) fn layout_wrapper(
    b: &mut LayoutBox,
    x: f32,
    y: f32,
    cbw: f32,
    fonts: &FontSource,
    text: Option<&dyn amazing_text::TextSystem>,
) -> f32 {
    b.margin = values::resolve_margins(&b.style, b.font_size, cbw);
    // CSS2.1 10.3.7's "both insets known" case: an absolutely positioned
    // wrapper with definite `left` AND `right` has a definite used width,
    // `cb.w - left - right - margins`, computed directly from those insets
    // (LAYOUT-26: using `cbw - margins` unconditionally ignored insets
    // entirely, so such a wrapper rendered at the full containing-block
    // width instead). Checking `left`/`right` themselves rather than reading
    // back the wrapper's own `width` style is deliberate: `prepare_sizes`
    // also writes a resolved `width` for the *shrink-to-fit* case (at most
    // one inset known), computed from this box's own `intrinsic_content_widths`
    // estimate of the table's min/max -- a good estimate, but not always
    // bit-identical to `table::layout`'s own authoritative per-column
    // min/max below (`mn`/`mx`, from this same `constraints()` call with
    // real text). Overriding on `width` alone regressed
    // `position/CSS2__positioning__abspos-008.xht` (only one inset set) for
    // exactly that reason; only the both-known case is a closed-form exact
    // value with no such estimate involved, so only it is safe to prefer
    // here. The shrink-to-fit case is left to fall through unchanged, exactly
    // as before this ticket, so `table::layout`'s own
    // `specified.unwrap_or(available.min(mx)).max(mn)` clamp remains the
    // sole source of truth for it.
    let available = if length(b, "left", cbw).is_some() && length(b, "right", cbw).is_some() {
        // css-tables-3 #abspos: "the given available-size can never exceed
        // the available-size of the containing block" -- unlike an ordinary
        // CSS2.1 abspos box, a table's resolved insets are additionally
        // clamped to `cbw` (a negative `left` paired with `right: 0`, for
        // instance, must not stretch the table wider than its containing
        // block; `absolute-tables-009.tentative.html`).
        (cbw - length(b, "left", cbw).unwrap() - length(b, "right", cbw).unwrap()
            - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0).min(cbw)
    } else {
        (cbw - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0)
    };
    let capmin = children(b)[1..]
        .iter()
        .map(|c| crate::flex::min_content_width(c, fonts))
        .fold(0.0, f32::max);
    at_mut(b, &[0]).containing_block_height = b.containing_block_height;
    let basis = if length(at(b, &[0]), "width", cbw).is_some() {
        cbw
    } else {
        available
    };
    layout(at_mut(b, &[0]), 0.0, 0.0, basis, fonts, text, capmin);
    let width = at(b, &[0]).border_box.w;
    let free = (available - width).max(0.0);
    match (b.margin.left.is_none(), b.margin.right.is_none()) {
        (true, true) => {
            b.margin.left = Some(free / 2.0);
            b.margin.right = Some(free / 2.0);
        }
        (true, false) => b.margin.left = Some(free),
        (false, true) => b.margin.right = Some(free),
        _ => {}
    }
    let bx = x + b.margin.left.unwrap_or(0.0);
    let by = y + b.margin.top.unwrap_or(0.0);
    let mut cy = by;
    for i in 1..children(b).len() {
        if !at(b, &[i])
            .style
            .get("caption-side")
            .is_some_and(|s| s.eq_ignore_ascii_case("bottom"))
        {
            cy += crate::block::layout_table_content(at_mut(b, &[i]), bx, cy, width, fonts, text);
        }
    }
    crate::flex::translate(at_mut(b, &[0]), bx, cy);
    cy += at(b, &[0]).border_box.h;
    for i in 1..children(b).len() {
        if at(b, &[i])
            .style
            .get("caption-side")
            .is_some_and(|s| s.eq_ignore_ascii_case("bottom"))
        {
            cy += crate::block::layout_table_content(at_mut(b, &[i]), bx, cy, width, fonts, text);
        }
    }
    b.border_box = Rect {
        x: bx,
        y: by,
        w: width,
        h: cy - by,
    };
    b.content_box = b.border_box;
    b.padding = Default::default();
    b.border = Default::default();
    b.lines.clear();
    b.border_box.h + b.margin.top.unwrap_or(0.0) + b.margin.bottom.unwrap_or(0.0)
}

fn content_widths(
    b: &LayoutBox,
    fonts: &FontSource,
    text: Option<&dyn amazing_text::TextSystem>,
) -> (f32, f32) {
    if let Flow::Inlines(items) = &b.flow {
        let mut minimum_items = items.clone();
        let mut maximum_items = items.clone();
        for (minimum, maximum) in minimum_items.iter_mut().zip(&mut maximum_items) {
            if let (
                crate::InlineNode::Atomic {
                    node,
                    size: min_size,
                    ..
                },
                crate::InlineNode::Atomic { size: max_size, .. },
            ) = (minimum, maximum)
                && let Some(atom) = b
                    .positioned
                    .iter()
                    .find(|c| c.inline_atomic && c.node == Some(*node))
            {
                let (mn, mx) = if atom.table_wrapper {
                    content_widths(atom, fonts, text)
                } else {
                    (
                        crate::flex::min_content_width(atom, fonts),
                        crate::flex::max_content_width(atom, fonts),
                    )
                };
                min_size.width = mn;
                max_size.width = mx;
            }
        }
        crate::replaced::resolve(&mut minimum_items, 0.0, None);
        crate::replaced::resolve(&mut maximum_items, 0.0, None);
        if let Some(text) = text {
            let direction = if b.style.get("direction") == Some("rtl") {
                amazing_text::Direction::RightToLeft
            } else {
                amazing_text::Direction::LeftToRight
            };
            if let Some(mn) =
                crate::inline::intrinsic_widths_shaped(&minimum_items, direction, text)
            {
                if !b.positioned.iter().any(|child| child.inline_atomic) {
                    return mn;
                }
                if let Some(mx) =
                    crate::inline::intrinsic_widths_shaped(&maximum_items, direction, text)
                {
                    return (mn.0, mx.1);
                }
            }
        }
        let mut scratch = LayoutBox::new(
            None,
            crate::Display::Block,
            b.style.clone(),
            b.font_size,
            b.line_height,
        );
        scratch.flow = Flow::Inlines(maximum_items);
        let mx = crate::flex::max_content_inner_width(&scratch, fonts);
        scratch.flow = Flow::Inlines(minimum_items);
        let mn = if matches!(b.style.get("white-space"), Some("nowrap" | "pre")) {
            mx
        } else {
            crate::flex::min_content_inner_width(&scratch, fonts)
        };
        return (mn, mx);
    }
    if let Flow::Blocks(c) = &b.flow {
        let mut mn = 0.0f32;
        let mut mx = 0.0f32;
        for child in c {
            let pair = if role(child) == Role::Table {
                intrinsic_with_text(child, fonts, text)
            } else {
                content_widths(child, fonts, text)
            };
            let edges = chrome(child, 0.0);
            let explicit = length(child, "width", 0.0).unwrap_or(0.0);
            mn = mn.max(pair.0.max(explicit) + edges);
            mx = mx.max(pair.1.max(explicit) + edges);
        }
        return (mn, mx);
    }
    let max = crate::flex::max_content_inner_width(b, fonts);
    let min = if matches!(b.style.get("white-space"), Some("nowrap" | "pre")) {
        max
    } else {
        crate::flex::min_content_inner_width(b, fonts)
    };
    (min, max)
}

/// The inline-table baseline belongs to its first row, not the caption or the
/// last line of its last cell. Empty tables synthesize the bottom grid edge.
pub(crate) fn inline_baseline(wrapper: &LayoutBox) -> f32 {
    let table = at(wrapper, &[0]);
    let grid = Grid::new(table);
    let cells = || {
        grid.cells
            .iter()
            .filter(|c| c.row == 0)
            .map(|c| at(table, &c.path))
    };
    cells()
        .filter(|cell| {
            !matches!(
                cell.style.get("vertical-align"),
                Some("top" | "middle" | "bottom")
            )
        })
        .map(cell_baseline)
        .reduce(f32::max)
        .or_else(|| {
            cells()
                .map(|cell| cell.content_box.y + cell.content_box.h)
                .reduce(f32::max)
        })
        .unwrap_or(table.border_box.y + table.border_box.h)
}
