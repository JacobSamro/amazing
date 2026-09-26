//! Resolved table decoration contract. Paint consumes these results verbatim;
//! it must not re-run border conflict resolution from computed styles.
use crate::{LayoutBox, Rect};
use amazing_dom::NodeIdx;

/// One winning collapsed border, split at grid intersections. Coordinates are
/// absolute CSS pixels, centered on the shared grid edge. `hidden` winners
/// are omitted, and consume zero geometry. Inset/outset are mapped to ridge/
/// groove as required for collapsed borders by CSS 2.1 §17.6.2.
#[derive(Debug, Clone, PartialEq)]
pub struct TableBorderSegment {
    pub rect: Rect,
    pub horizontal: bool,
    pub style: String,
    pub color: [u8; 4],
    pub source: Option<NodeIdx>,
}

/// Decoration decisions for this box only. Traverse normal and positioned
/// child collections as usual. The table owns all winning border segments;
/// internal boxes must skip their ordinary borders when requested. Background
/// holes are absolute rectangles to subtract from this box's background only
/// (not its descendants or text); this exposes the table layer through empty
/// cells even when a row/column has a background.
#[derive(Debug, Default)]
pub struct TableDecoration {
    pub suppress_border: bool,
    pub suppress_background: bool,
    pub background_holes: Vec<Rect>,
    pub borders: Vec<TableBorderSegment>,
}
impl LayoutBox {
    pub fn table_decoration(&self) -> &TableDecoration {
        &self.table_decoration
    }
}

#[derive(Debug, Clone)]
struct Winner {
    width: f32,
    style: String,
    color: [u8; 4],
    source: Option<NodeIdx>,
    origin: u8,
    row: usize,
    col: usize,
}
impl Winner {
    fn from_box(b: &LayoutBox, side: &str, origin: u8, row: usize, col: usize) -> Self {
        let style = b
            .style
            .get(&format!("border-{side}-style"))
            .unwrap_or("none")
            .to_ascii_lowercase();
        let widths = crate::values::resolve_borders(&b.style, b.font_size);
        let width = match side {
            "top" => widths.top,
            "bottom" => widths.bottom,
            "left" => widths.left,
            _ => widths.right,
        };
        let raw = b
            .style
            .get(&format!("border-{side}-color"))
            .unwrap_or("currentcolor");
        let raw = if raw.eq_ignore_ascii_case("currentcolor") {
            b.style.get("color").unwrap_or("black")
        } else {
            raw
        };
        let color = amazing_css::color::parse_css_color(raw).unwrap_or([0, 0, 0, 255]);
        Self {
            width,
            style,
            color,
            source: b.node,
            origin,
            row,
            col,
        }
    }
    fn rank(&self) -> u8 {
        match self.style.as_str() {
            "hidden" => 10,
            "double" => 9,
            "solid" => 8,
            "dashed" => 7,
            "dotted" => 6,
            "ridge" => 5,
            "outset" => 4,
            "groove" => 3,
            "inset" => 2,
            _ => 0,
        }
    }
    fn beats(&self, other: &Self) -> bool {
        // Grid column indexes are logical-start order even for RTL tables.
        if self.style == "hidden" || other.style == "hidden" {
            return self.style == "hidden";
        }
        (
            self.width,
            self.rank(),
            self.origin,
            std::cmp::Reverse(self.row),
            std::cmp::Reverse(self.col),
        ) > (
            other.width,
            other.rank(),
            other.origin,
            std::cmp::Reverse(other.row),
            std::cmp::Reverse(other.col),
        )
    }
}

// (horizontal, row-grid-line, column-grid-line). BTreeMap gives stable segment
// ordering without relying on randomized hash iteration.
pub(crate) struct Borders {
    edges: std::collections::BTreeMap<(bool, usize, usize), Winner>,
    rtl: bool,
}
impl Borders {
    pub(crate) fn new(b: &LayoutBox, g: &crate::table::Grid) -> Self {
        let mut out = Self {
            edges: Default::default(),
            rtl: b.style.get("direction") == Some("rtl"),
        };
        let rows = g.rows.len();
        let cols = g.count;
        if rows == 0 || cols == 0 {
            return out;
        }
        out.add(b, 0, 0, 0, rows, cols);
        for (i, path) in g.columns.iter().enumerate() {
            if i > 0 && g.columns[i - 1] == *path {
                continue;
            }
            let span = g.columns[i..].iter().take_while(|p| *p == path).count();
            out.add(
                crate::table::at(b, path),
                if path.len() == 1
                    && crate::table_tree::role(crate::table::at(b, path))
                        == crate::table_tree::Role::ColumnGroup
                {
                    1
                } else {
                    2
                },
                0,
                i,
                rows,
                span,
            );
        }
        for (i, group) in crate::table::children(b).iter().enumerate() {
            if crate::table_tree::role(group).group() {
                let members: Vec<_> = g
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.path.len() == 2 && r.path[0] == i)
                    .map(|(r, _)| r)
                    .collect();
                if let (Some(&first), Some(&last)) = (members.first(), members.last()) {
                    out.add(group, 3, first, 0, last - first + 1, cols);
                }
            } else if crate::table_tree::role(group) == crate::table_tree::Role::ColumnGroup {
                let members: Vec<_> = g
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p[0] == i)
                    .map(|(c, _)| c)
                    .collect();
                if let (Some(&first), Some(&last)) = (members.first(), members.last()) {
                    out.add(group, 1, 0, first, rows, last - first + 1);
                }
            }
        }
        for (r, row) in g.rows.iter().enumerate() {
            out.add(crate::table::at(b, &row.path), 4, r, 0, 1, cols);
        }
        for c in &g.cells {
            out.add(
                crate::table::at(b, &c.path),
                5,
                c.row,
                c.col,
                c.rows,
                c.cols,
            );
        }
        // An internal grid line cannot cut through a spanning cell.
        for c in &g.cells {
            for row in c.row + 1..c.row + c.rows {
                let keys: Vec<_> = out
                    .edges
                    .range((true, row, c.col)..(true, row, c.col + c.cols))
                    .map(|(k, _)| *k)
                    .collect();
                for key in keys {
                    out.edges.remove(&key);
                }
            }
            if c.cols > 1 {
                for row in c.row..c.row + c.rows {
                    let keys: Vec<_> = out
                        .edges
                        .range((false, row, c.col + 1)..(false, row, c.col + c.cols))
                        .map(|(k, _)| *k)
                        .collect();
                    for key in keys {
                        out.edges.remove(&key);
                    }
                }
            }
        }
        out
    }
    fn add(&mut self, b: &LayoutBox, origin: u8, r: usize, c: usize, rows: usize, cols: usize) {
        for (side, horizontal, line, start, count) in [
            ("top", true, r, c, cols),
            ("bottom", true, r + rows, c, cols),
            ("left", false, c, r, rows),
            ("right", false, c + cols, r, rows),
        ] {
            let side = if self.rtl {
                match side {
                    "left" => "right",
                    "right" => "left",
                    s => s,
                }
            } else {
                side
            };
            let winner = Winner::from_box(b, side, origin, r, c);
            for i in start..start + count {
                let key = if horizontal {
                    (true, line, i)
                } else {
                    (false, i, line)
                };
                let entry = self.edges.entry(key).or_insert_with(|| winner.clone());
                if winner.beats(entry) {
                    *entry = winner.clone();
                }
            }
        }
    }
    pub(crate) fn widths(
        &self,
        r: usize,
        c: usize,
        rows: usize,
        cols: usize,
    ) -> crate::values::Sides {
        let value = |key| {
            self.edges.get(&key).map_or(0.0, |w| {
                if w.style == "hidden" {
                    0.0
                } else {
                    w.width / 2.0
                }
            })
        };
        let logical = crate::values::Sides {
            top: (c..c + cols)
                .map(|i| value((true, r, i)))
                .fold(0.0, f32::max),
            bottom: (c..c + cols)
                .map(|i| value((true, r + rows, i)))
                .fold(0.0, f32::max),
            left: (r..r + rows)
                .map(|i| value((false, i, c)))
                .fold(0.0, f32::max),
            right: (r..r + rows)
                .map(|i| value((false, i, c + cols)))
                .fold(0.0, f32::max),
        };
        if self.rtl {
            crate::values::Sides {
                left: logical.right,
                right: logical.left,
                ..logical
            }
        } else {
            logical
        }
    }
    pub(crate) fn segments(&self, xs: &[f32], ys: &[f32]) -> Vec<TableBorderSegment> {
        // A vertical segment meets horizontal borders at both endpoints.
        // Their half-widths extend the painted join into an empty row, even
        // when that row has no cells and therefore no vertical edge of its own.
        let horizontal_join = |row: usize, column: usize| {
            [column.checked_sub(1), Some(column)]
                .into_iter()
                .flatten()
                .filter_map(|c| self.edges.get(&(true, row, c)))
                .filter(|winner| winner.style != "hidden" && winner.style != "none")
                .map(|winner| winner.width / 2.0)
                .fold(0.0, f32::max)
        };
        self.edges
            .iter()
            .filter_map(|(&(h, r, c), w)| {
                if w.width <= 0.0 || w.style == "hidden" || w.style == "none" {
                    return None;
                }
                let rect = if h {
                    Rect {
                        x: xs[c].min(xs[c + 1]),
                        y: ys[r] - w.width / 2.0,
                        w: (xs[c + 1] - xs[c]).abs(),
                        h: w.width,
                    }
                } else {
                    let start_join = horizontal_join(r, c);
                    let end_join = horizontal_join(r + 1, c);
                    Rect {
                        x: xs[c] - w.width / 2.0,
                        y: ys[r] - start_join,
                        w: w.width,
                        h: ys[r + 1] - ys[r] + start_join + end_join,
                    }
                };
                Some(TableBorderSegment {
                    rect,
                    horizontal: h,
                    style: match w.style.as_str() {
                        "inset" => "ridge",
                        "outset" => "groove",
                        s => s,
                    }
                    .into(),
                    color: w.color,
                    source: w.source,
                })
            })
            .collect()
    }
}
