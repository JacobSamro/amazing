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
        for row in &mut rows {
            if row.end == 0 {
                row.end = row_count;
            }
        }
        let mut occupied_until = vec![0; MAX_COLUMNS];
        let mut cells = Vec::new();
        let mut count = columns.len();
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
        .and_then(|v| values::parse_len(v, b.font_size, 0.0))
        .unwrap_or(0.0)
        .max(0.0);
    let y = parts
        .next()
        .and_then(|v| values::parse_len(v, b.font_size, 0.0))
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
        let width = length(col, "width", basis)
            .or_else(|| {
                (path.len() == 2)
                    .then(|| length(at(b, &path[..1]), "width", basis))
                    .flatten()
            })
            .unwrap_or(0.0);
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
        let explicit = length(cell, "width", basis).unwrap_or(0.0) + edges;
        let (inner_min, inner_max) = content_widths(cell, fonts, text);
        let mn = (inner_min + edges).max(explicit);
        let mx = (inner_max + edges).max(mn);
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
    let g = Grid::new(b);
    let (min, max) = constraints(b, &g, fonts, 0.0, None);
    let edges = chrome(b, 0.0)
        + if g.count > 0 {
            spacing(b).0 * (g.count + 1) as f32
        } else {
            0.0
        };
    (
        min.iter().sum::<f32>() + edges,
        max.iter().sum::<f32>() + edges,
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
    let specified = length(b, "width", cbw);
    let fixed = specified.is_some()
        && b.style
            .get("table-layout")
            .is_some_and(|s| s.eq_ignore_ascii_case("fixed"));
    let available = (cbw - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0);
    let (min, max) = constraints(b, &g, fonts, specified.unwrap_or(0.0), text);
    let capmin = children(b)
        .iter()
        .filter(|c| role(c) == Role::Caption)
        .map(|c| crate::flex::min_content_width(c, fonts))
        .fold(0.0, f32::max);
    // CSS table width denotes the border box. Intrinsic cell constraints
    // include cell padding/borders, but exclude inter-cell border spacing.
    let mn = min.iter().sum::<f32>() + edges + gaps;
    let mx = max.iter().sum::<f32>() + edges + gaps;
    let mut width = if fixed {
        specified.unwrap()
    } else {
        specified.unwrap_or(available.min(mx)).max(mn)
    }
    .max(capmin)
    .max(caption_min);
    let mut widths = if fixed {
        vec![0.0; g.count]
    } else {
        min.clone()
    };
    if fixed {
        let mut known = vec![false; g.count];
        for (i, path) in g.columns.iter().enumerate() {
            if let Some(w) = length(at(b, path), "width", width) {
                widths[i] = w;
                known[i] = true;
            }
        }
        for c in g.cells.iter().filter(|c| c.row == 0) {
            let cell = at(b, &c.path);
            if let Some(w) = length(cell, "width", width) {
                let required = (w + chrome(cell, width) - sx * (c.cols - 1) as f32).max(0.0);
                let present: f32 = widths[c.col..c.col + c.cols].iter().sum();
                let missing = known[c.col..c.col + c.cols].iter().filter(|&&v| !v).count();
                if missing > 0 {
                    for i in c.col..c.col + c.cols {
                        if !known[i] {
                            widths[i] = (required - present).max(0.0) / missing as f32;
                            known[i] = true;
                        }
                    }
                }
            }
        }
        let remaining = (width - edges - gaps - widths.iter().sum::<f32>()).max(0.0);
        let missing = known.iter().filter(|&&v| !v).count();
        if missing > 0 {
            for i in 0..g.count {
                if !known[i] {
                    widths[i] = remaining / missing as f32;
                }
            }
        }
        width = width.max(widths.iter().sum::<f32>() + edges + gaps);
    }
    if g.count > 0 {
        let extra = (width - edges - gaps - widths.iter().sum::<f32>()).max(0.0);
        let flexibility: f32 = max.iter().zip(&min).map(|(a, b)| a - b).sum();
        let towards = if fixed { 0.0 } else { extra.min(flexibility) };
        for i in 0..g.count {
            widths[i] += if flexibility > 0.0 {
                towards * (max[i] - min[i]) / flexibility
            } else {
                0.0
            };
            widths[i] += (extra - towards) / g.count as f32;
        }
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
        for name in ["margin-top", "margin-right", "margin-bottom", "margin-left"] {
            cell.style.set(name, "0");
        }
        crate::block::layout_table_content(cell, cell_x, 0.0, w, fonts, text);
        cell.style = original;
        let h = cell.border_box.h;
        let baseline = first_baseline(cell).unwrap_or(cell.content_box.y + cell.content_box.h)
            - cell.border_box.y;
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
            (natural[i] + adjustment - sy * (c.rows - 1) as f32).max(0.0),
        );
    }
    let desired = length(b, "height", b.containing_block_height.unwrap_or(0.0)).unwrap_or(0.0);
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
        Flow::Inlines(items) => items
            .iter()
            .all(|i| matches!(i,crate::InlineNode::Text{text,..} if text.trim().is_empty())),
        Flow::Blocks(c) | Flow::Flex(c) | Flow::Grid(c) => c.iter().all(empty),
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
    let available = (cbw - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0);
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
    if let (Flow::Inlines(items), Some(text)) = (&b.flow, text) {
        let mut items = items.clone();
        crate::replaced::resolve(&mut items, 0.0, None);
        let direction = if b.style.get("direction") == Some("rtl") {
            amazing_text::Direction::RightToLeft
        } else {
            amazing_text::Direction::LeftToRight
        };
        if let Some(pair) = crate::inline::intrinsic_widths_shaped(&items, direction, text) {
            return pair;
        }
    }
    if let Flow::Blocks(c) = &b.flow {
        let mut mn = 0.0f32;
        let mut mx = 0.0f32;
        for child in c {
            let pair = if role(child) == Role::Table {
                intrinsic(child, fonts)
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
