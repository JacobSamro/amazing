//! Flexbox layout (css-flexbox-1 subset): flex containers resolve their
//! items along a main axis with grow/shrink/basis distribution, wrap into
//! lines, and align on the cross axis.
//!
//! Implemented:
//! - `display: flex` / `inline-flex` (identical at layout time — inline
//!   level outer display is a later-phase concern)
//! - flex-direction: row / row-reverse / column / column-reverse
//! - flex-wrap: nowrap / wrap (row mode); align-content against a definite
//!   container height
//! - flex-grow / flex-shrink / flex-basis + `flex` shorthand forms
//! - order (stable sort), gap / row-gap / column-gap
//! - justify-content: flex-start/end, center, space-between/around/evenly
//! - align-items / align-self: stretch, flex-start/end, center
//! - auto margins absorb main-axis free space before justify-content
//!   (materialized into the style so the box-model resolver agrees)
//!
//! Deviations (documented):
//! - baseline alignment behaves like flex-start
//! - shrink has no min-content floor
//! - column mode is single-line; column-reverse degrades to normal
//!   stacking order (top-down)
//! - cross-axis auto margins count as zero
//! - a flexed item with a specified width has that width materialized to
//!   the flexed size (the used size wins, per spec)

use amazing_css::ComputedStyle;

use crate::font::FontSource;
use crate::inline;
use crate::values::{self, MaybeLen};
use crate::{Flow, LayoutBox, Rect};

// ---------------------------------------------------------------------------
// Property parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Direction {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pack {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

/// Leading space before the first item and EXTRA space between items for
/// a given pack mode, free space, and item count.
fn justify_pair(mode: Pack, free: f32, n: usize) -> (f32, f32) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let nf = n as f32;
    match mode {
        Pack::End => (free, 0.0),
        Pack::Center => (free / 2.0, 0.0),
        Pack::SpaceBetween => (0.0, free / (nf - 1.0)),
        Pack::SpaceAround => (free / nf / 2.0, free / nf),
        Pack::SpaceEvenly => (free / (nf + 1.0), free / (nf + 1.0)),
        _ => (0.0, 0.0),
    }
}

/// Parsed container properties.
struct ContainerSpec {
    direction: Direction,
    wrap: bool,
    justify: Pack,
    align_items: Pack,
    align_content: Pack,
    row_gap: f32,
    col_gap: f32,
    definite_height: Option<f32>,
}

impl ContainerSpec {
    fn of(style: &ComputedStyle, font_px: f32, content_w: f32, definite_height: Option<f32>) -> Self {
        let direction = match style.get("flex-direction").unwrap_or("row") {
            v if v.eq_ignore_ascii_case("row-reverse") => Direction::RowReverse,
            v if v.eq_ignore_ascii_case("column") => Direction::Column,
            v if v.eq_ignore_ascii_case("column-reverse") => Direction::ColumnReverse,
            _ => Direction::Row,
        };
        let wrap = style
            .get("flex-wrap")
            .map(|v| v.eq_ignore_ascii_case("wrap") || v.eq_ignore_ascii_case("wrap-reverse"))
            .unwrap_or(false);
        let justify = parse_pack(style.get("justify-content"), Pack::Start);
        let align_items = parse_align(style.get("align-items"), Pack::Stretch);
        let align_content = if style
            .get("align-content")
            .map(|v| v.eq_ignore_ascii_case("stretch"))
            .unwrap_or(true)
        {
            Pack::Stretch
        } else {
            parse_pack(style.get("align-content"), Pack::Start)
        };
        let (row_gap, col_gap) = parse_gaps(style, font_px, content_w);
        ContainerSpec {
            direction,
            wrap,
            justify,
            align_items,
            align_content,
            row_gap,
            col_gap,
            definite_height,
        }
    }
}

fn parse_pack(v: Option<&str>, default: Pack) -> Pack {
    match v {
        Some(v) if v.eq_ignore_ascii_case("flex-end") || v.eq_ignore_ascii_case("end") => Pack::End,
        Some(v) if v.eq_ignore_ascii_case("center") => Pack::Center,
        Some(v) if v.eq_ignore_ascii_case("space-between") => Pack::SpaceBetween,
        Some(v) if v.eq_ignore_ascii_case("space-around") => Pack::SpaceAround,
        Some(v) if v.eq_ignore_ascii_case("space-evenly") => Pack::SpaceEvenly,
        Some(v) if v.eq_ignore_ascii_case("stretch") => Pack::Stretch,
        _ => default,
    }
}

/// Alignment keywords; baseline ≈ flex-start (documented).
fn parse_align(v: Option<&str>, default: Pack) -> Pack {
    match v {
        Some(v) if v.eq_ignore_ascii_case("flex-start") || v.eq_ignore_ascii_case("start") => {
            Pack::Start
        },
        Some(v) if v.eq_ignore_ascii_case("flex-end") || v.eq_ignore_ascii_case("end") => Pack::End,
        Some(v) if v.eq_ignore_ascii_case("center") => Pack::Center,
        Some(v) if v.eq_ignore_ascii_case("stretch") => Pack::Stretch,
        _ => default,
    }
}

/// `gap: r c` / `row-gap` / `column-gap`. Percentages resolve against the
/// inline size for both axes (row-gap % against height unsupported → same
/// base, documented).
fn parse_gaps(style: &ComputedStyle, font_px: f32, content_w: f32) -> (f32, f32) {
    let px =
        |s: &str| MaybeLen::parse(s, font_px, content_w).map_or(0.0, |m| m.unwrap_or(0.0));
    let mut row_gap = 0.0f32;
    let mut col_gap = 0.0f32;
    if let Some(v) = style.get("gap") {
        let parts: Vec<&str> = v.split_ascii_whitespace().collect();
        let r = parts.first().copied().unwrap_or("0");
        let c = parts.get(1).copied().unwrap_or(r);
        row_gap = px(r);
        col_gap = px(c);
    }
    if let Some(v) = style.get("row-gap") {
        row_gap = px(v);
    }
    if let Some(v) = style.get("column-gap") {
        col_gap = px(v);
    }
    (row_gap.max(0.0), col_gap.max(0.0))
}

/// Resolved flex-item properties.
#[derive(Debug, Clone, Copy)]
struct ItemProps {
    order: i32,
    grow: f32,
    shrink: f32,
    /// Resolved flex-basis in px along the main axis, or None for `auto`.
    basis_px: Option<f32>,
    align_self: Option<Pack>,
}

impl ItemProps {
    fn of(style: &ComputedStyle, font_px: f32, main_extent: f32) -> Self {
        // `flex` shorthand slots first; longhands override individually
        // (this shorthand is not expanded by the cascade).
        let mut grow = 0.0f32;
        let mut shrink = 1.0f32;
        let mut basis: Option<MaybeLen> = None; // auto
        if let Some(v) = style.get("flex") {
            let parts: Vec<&str> = v.split_ascii_whitespace().collect();
            if parts.len() == 1 && parts[0].eq_ignore_ascii_case("none") {
                grow = 0.0;
                shrink = 0.0;
            } else if parts.len() == 1 && parts[0].eq_ignore_ascii_case("auto") {
                grow = 1.0;
                shrink = 1.0;
            } else if parts.len() == 1 {
                if let Ok(n) = parts[0].parse::<f32>() {
                    // `flex: <number>` → grow n, basis 0% (§7.1.1).
                    grow = n;
                    basis = Some(MaybeLen::Px(0.0));
                } else {
                    // One-value length form: `flex: 10em` → 1 1 10em.
                    grow = 1.0;
                    shrink = 1.0;
                    basis = MaybeLen::parse(parts[0], font_px, main_extent);
                }
            } else {
                let nums: Vec<f32> = parts.iter().filter_map(|p| p.parse().ok()).collect();
                grow = nums.first().copied().unwrap_or(0.0);
                shrink = nums.get(1).copied().unwrap_or(1.0);
                // First non-number token is the basis (`auto` parses to Auto).
                basis = parts
                    .iter()
                    .find(|p| p.parse::<f32>().is_err())
                    .and_then(|p| MaybeLen::parse(p, font_px, main_extent))
                    .or_else(|| nums.get(2).map(|n| MaybeLen::Px(*n)));
            }
        }
        if let Some(v) = style.get("flex-grow") {
            grow = v.trim().parse().unwrap_or(grow);
        }
        if let Some(v) = style.get("flex-shrink") {
            shrink = v.trim().parse().unwrap_or(shrink);
        }
        if let Some(v) = style.get("flex-basis") {
            basis = MaybeLen::parse(v, font_px, main_extent).or(basis);
        }
        ItemProps {
            order: style
                .get("order")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            grow: grow.max(0.0),
            shrink: shrink.max(0.0),
            basis_px: match basis {
                Some(MaybeLen::Px(v)) => Some(v),
                _ => None,
            },
            align_self: style.get("align-self").and_then(|v| {
                if v.eq_ignore_ascii_case("auto") {
                    None
                } else {
                    Some(parse_align(Some(v), Pack::Stretch))
                }
            }),
        }
    }
}

/// True when the STYLE explicitly says `auto` for that margin side. The
/// computed map stores raw declarations; ABSENT means the initial 0, not
/// auto — conflating the two would make every unstyled item absorb space.
fn margin_is_auto(style: &ComputedStyle, side: &str) -> bool {
    let longhand = format!("margin-{side}");
    if let Some(v) = style.get(&longhand) {
        return v.trim().eq_ignore_ascii_case("auto");
    }
    if let Some(v) = style.get("margin") {
        // Replicate the 1–4 shorthand onto the requested side.
        let parts: Vec<&str> = v.split_ascii_whitespace().collect();
        let i = match side {
            "top" => 0,
            "right" => 1,
            "bottom" => 2,
            _ => 3,
        };
        let idx = match parts.len() {
            1 => 0,
            2 => i % 2,
            3 => match i {
                0 => 0,
                1 | 3 => 1,
                _ => 2,
            },
            _ => i.min(3),
        };
        if let Some(p) = parts.get(idx) {
            return p.eq_ignore_ascii_case("auto");
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Lays out `items` (already blockified flex items) inside the container's
/// content box at (`content_x`, `content_y`) with width `content_w`.
/// Fills each item's model + geometry and returns the content height.
pub fn layout_flex(
    items: &mut Vec<LayoutBox>,
    container: &ComputedStyle,
    font_px: f32,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    definite_height: Option<f32>,
    fonts: &FontSource,
) -> f32 {
    let props: Vec<ItemProps> = items
        .iter()
        .map(|it| ItemProps::of(&it.style, it.font_size, content_w))
        .collect();
    // Stable sort by `order` (css-flexbox-1 §4.4).
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| props[i].order);

    let spec = ContainerSpec::of(container, font_px, content_w, definite_height);
    match spec.direction {
        Direction::Row | Direction::RowReverse => {
            layout_row(items, &props, &order, &spec, content_x, content_y, content_w, fonts)
        },
        Direction::Column | Direction::ColumnReverse => {
            layout_column(items, &props, &order, &spec, content_x, content_y, content_w, fonts)
        },
    }
}

/// Horizontal padding + border chrome of an item.
fn h_chrome(it: &LayoutBox) -> f32 {
    it.padding.left + it.padding.right + it.border.left + it.border.right
}
/// Vertical padding + border chrome.
fn v_chrome(it: &LayoutBox) -> f32 {
    it.padding.top + it.padding.bottom + it.border.top + it.border.bottom
}

/// Outer main size (content + horizontal chrome + side margins).
fn outer_main(items: &[LayoutBox], i: usize, base: f32) -> f32 {
    base + h_chrome(&items[i])
        + items[i].margin.left.unwrap_or(0.0)
        + items[i].margin.right.unwrap_or(0.0)
}
/// Outer cross size of a LAID OUT item (border box + vertical chrome +
/// top/bottom margins).
fn outer_cross(items: &[LayoutBox], i: usize) -> f32 {
    items[i].border_box.h + v_chrome(&items[i])
        + items[i].margin.top.unwrap_or(0.0)
        + items[i].margin.bottom.unwrap_or(0.0)
}

/// Column-mode outer main (vertical) size for an adjusted base height.
fn item_outer_h(base_h: f32, it: &LayoutBox) -> f32 {
    base_h + v_chrome(it)
        + it.margin.top.unwrap_or(0.0)
        + it.margin.bottom.unwrap_or(0.0)
}

fn resolve_model(it: &mut LayoutBox, cbw: f32) {
    it.margin = values::resolve_margins(&it.style, it.font_size, cbw);
    it.padding = values::resolve_padding(&it.style, it.font_size, cbw);
    it.border = values::resolve_borders(&it.style, it.font_size);
}

// ---------------------------------------------------------------------------
// Row direction
// ---------------------------------------------------------------------------

fn layout_row(
    items: &mut [LayoutBox],
    props: &[ItemProps],
    order: &[usize],
    spec: &ContainerSpec,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    fonts: &FontSource,
) -> f32 {
    for i in 0..items.len() {
        resolve_model(&mut items[i], content_w);
    }

    // Hypothetical main (content) sizes: basis > width > max-content.
    let mut base: Vec<f32> = (0..items.len())
        .map(|i| {
            let it = &items[i];
            match props[i].basis_px {
                Some(b) => b.max(0.0),
                None => match values::resolve_extent(&it.style, "width", it.font_size, content_w) {
                    MaybeLen::Px(w) => w.max(0.0),
                    MaybeLen::Auto => max_content_width(it, fonts).min(content_w),
                },
            }
        })
        .collect();

    // Wrap into lines (greedy by outer base + column gaps).
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut used = 0.0f32;
    for &i in order {
        let gap = if cur.is_empty() { 0.0 } else { spec.col_gap };
        if spec.wrap && !cur.is_empty() && used + gap + outer_main(items, i, base[i]) > content_w {
            lines.push(std::mem::take(&mut cur));
            used = 0.0;
        }
        let gap = if cur.is_empty() { 0.0 } else { spec.col_gap };
        used += gap + outer_main(items, i, base[i]);
        cur.push(i);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }

    let reverse = spec.direction == Direction::RowReverse;
    let mut line_top = content_y;
    let mut content_height = 0.0f32;
    let mut line_crosses: Vec<f32> = Vec::with_capacity(lines.len());

    for line in &lines {
        let n = line.len();
        let gaps_total = spec.col_gap * (n.saturating_sub(1)) as f32;
        let used: f32 =
            line.iter().map(|&i| outer_main(items, i, base[i])).sum::<f32>() + gaps_total;
        let mut free = content_w - used;

        // Grow / shrink adjust content sizes (the used size wins over a
        // specified width — materialized below).
        if free > 0.0 {
            let sum_grow: f32 = line.iter().map(|&i| props[i].grow).sum();
            if sum_grow > 0.0 {
                for &i in line {
                    base[i] += free * props[i].grow / sum_grow;
                }
                free = 0.0;
            }
        } else if free < 0.0 {
            let sum_w: f32 = line
                .iter()
                .map(|&i| props[i].shrink * outer_main(items, i, base[i]))
                .sum();
            if sum_w > 0.0 {
                let over = -free;
                for &i in line {
                    let take = over * (props[i].shrink * outer_main(items, i, base[i])) / sum_w;
                    base[i] = (base[i] - take).max(0.0);
                }
                free = 0.0;
            }
        }

        // Materialize flexed widths for items with a specified width so
        // the box-model resolver lays out at the used size.
        for &i in line {
            if matches!(
                values::resolve_extent(&items[i].style, "width", items[i].font_size, content_w),
                MaybeLen::Px(_)
            ) {
                items[i]
                    .style
                    .set("width", &format!("{}px", base[i].max(0.0)));
            }
        }

        // Auto main margins absorb remaining free space before
        // justify-content (§8.1), leaving justify nothing.
        let mut auto_sides = 0usize;
        for &i in line {
            if margin_is_auto(&items[i].style, "left") {
                auto_sides += 1;
            }
            if margin_is_auto(&items[i].style, "right") {
                auto_sides += 1;
            }
        }
        let mut justify_free = free.max(0.0);
        if auto_sides > 0 && free > 0.0 {
            let share = free / auto_sides as f32;
            for &i in line {
                for side in ["left", "right"] {
                    if margin_is_auto(&items[i].style, side) {
                        items[i]
                            .style
                            .set(&format!("margin-{side}"), &format!("{share}px"));
                    }
                }
                resolve_model(&mut items[i], content_w);
            }
            justify_free = 0.0;
        }

        // Lay out at final content widths (origin 0,0; translated below).
        for &i in line {
            crate::block::layout_block(&mut items[i], 0.0, 0.0, base[i].max(0.0), fonts);
        }

        // Line cross size, then stretch items whose cross size is auto.
        let mut line_cross =
            line.iter().map(|&i| outer_cross(items, i)).fold(0.0f32, f32::max);
        // A single line under default `align-content: stretch` first
        // grows to the container's definite cross size; items then
        // stretch into the grown line below.
        if lines.len() == 1 && spec.align_content == Pack::Stretch {
            if let Some(def_h) = spec.definite_height {
                line_cross = line_cross.max(def_h);
            }
        }
        for &i in line {
            let align = props[i].align_self.unwrap_or(spec.align_items);
            let height_auto = matches!(
                values::resolve_extent(&items[i].style, "height", items[i].font_size, 0.0),
                MaybeLen::Auto
            );
            if align == Pack::Stretch
                && height_auto
                && !margin_is_auto(&items[i].style, "top")
                && !margin_is_auto(&items[i].style, "bottom")
            {
                let target = line_cross - v_chrome(&items[i]);
                if target > items[i].border_box.h {
                    stretch_height(&mut items[i], target);
                }
            }
        }
        line_cross = line
            .iter()
            .map(|&i| outer_cross(items, i))
            .fold(line_cross, f32::max);

        // Main-axis placement: leading offset + extra between-item space.
        let finals: Vec<f32> = line.iter().map(|&i| outer_main(items, i, base[i])).collect();
        let (lead, between) = justify_pair(spec.justify, justify_free, n);
        let mut cursor = if reverse {
            content_x + content_w - lead
        } else {
            content_x + lead
        };
        for (pos, &i) in line.iter().enumerate() {
            if pos > 0 {
                let step = spec.col_gap + between;
                if reverse {
                    cursor -= step;
                } else {
                    cursor += step;
                }
            }
            let it = &mut items[i];
            let ml = it.margin.left.unwrap_or(0.0);
            let bx = if reverse {
                cursor - ml - h_chrome(it) - base[i]
            } else {
                cursor + ml
            };
            let align = props[i].align_self.unwrap_or(spec.align_items);
            let bh = it.border_box.h;
            let vc = v_chrome(it);
            let by = line_top
                + it.margin.top.unwrap_or(0.0)
                + match align {
                    Pack::End => line_cross - bh - vc,
                    Pack::Center => (line_cross - bh - vc) / 2.0,
                    _ => 0.0,
                };
            translate(it, bx - it.border_box.x, by - it.border_box.y);
            if reverse {
                cursor -= finals[pos];
            } else {
                cursor += finals[pos];
            }
        }

        content_height += line_cross;
        line_top += line_cross + spec.row_gap;
        line_crosses.push(line_cross);
    }

    // align-content: distribute spare cross space against a definite
    // container height by shifting whole lines down.
    if let Some(def_h) = spec.definite_height {
        let lines_h: f32 = line_crosses.iter().sum::<f32>()
            + spec.row_gap * (lines.len().saturating_sub(1)) as f32;
        let spare = def_h - lines_h;
        if spare > 0.0 && !line_crosses.is_empty() {
            // Default `stretch` behaves like start here (per-line stretch
            // into spare space is deferred).
            let ac = if spec.align_content == Pack::Stretch {
                Pack::Start
            } else {
                spec.align_content
            };
            let (mut shift, between) = justify_pair(ac, spare, lines.len());
            for (li, line) in lines.iter().enumerate() {
                if shift > 0.0 {
                    for &i in line {
                        translate(&mut items[i], 0.0, shift);
                    }
                }
                shift += line_crosses[li] + spec.row_gap + between;
            }
        }
    }

    content_height
}

// ---------------------------------------------------------------------------
// Column direction
// ---------------------------------------------------------------------------

fn layout_column(
    items: &mut [LayoutBox],
    props: &[ItemProps],
    order: &[usize],
    spec: &ContainerSpec,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    fonts: &FontSource,
) -> f32 {
    for i in 0..items.len() {
        resolve_model(&mut items[i], content_w);
    }

    // Cross (horizontal) sizes: stretch fills the line; otherwise
    // specified width or max-content, clamped to the container.
    for &i in order {
        let align = props[i].align_self.unwrap_or(spec.align_items);
        let width_auto = matches!(
            values::resolve_extent(&items[i].style, "width", items[i].font_size, content_w),
            MaybeLen::Auto
        );
        let w = {
            let it = &mut items[i];
            let ml = it.margin.left.unwrap_or(0.0);
            let mr = it.margin.right.unwrap_or(0.0);
            let avail = (content_w - ml - mr).max(0.0);
            match align {
                Pack::Stretch
                    if width_auto
                        && !margin_is_auto(&it.style, "left")
                        && !margin_is_auto(&it.style, "right") =>
                {
                    avail
                },
                _ => match values::resolve_extent(&it.style, "width", it.font_size, content_w) {
                    MaybeLen::Px(v) => v.min(avail),
                    MaybeLen::Auto => max_content_width(it, fonts).min(avail),
                },
            }
        };
        crate::block::layout_block(&mut items[i], 0.0, 0.0, w, fonts);
    }

    // Main (vertical) sizes: measured outer heights. With a definite
    // container height, grow/shrink engage; otherwise natural stacking.
    let n = order.len();
    let mut base: Vec<f32> = order.iter().map(|&i| items[i].border_box.h).collect();
    let gaps_total = spec.row_gap * n.saturating_sub(1) as f32;
    let used: f32 = order
        .iter()
        .enumerate()
        .map(|(p, &i)| item_outer_h(base[p], &items[i]))
        .sum::<f32>()
        + gaps_total;

    if let Some(def_h) = spec.definite_height {
        let free = def_h - used;
        if free > 0.0 {
            let sum_grow: f32 = order.iter().map(|&i| props[i].grow).sum();
            if sum_grow > 0.0 {
                for (p, &i) in order.iter().enumerate() {
                    base[p] += free * props[i].grow / sum_grow;
                }
            }
        } else if free < 0.0 {
            let sum_w: f32 = order
                .iter()
                .enumerate()
                .map(|(p, &i)| props[i].shrink * item_outer_h(base[p], &items[i]))
                .sum();
            if sum_w > 0.0 {
                let over = -free;
                for (p, &i) in order.iter().enumerate() {
                    let take = over * props[i].shrink * item_outer_h(base[p], &items[i]) / sum_w;
                    base[p] = (base[p] - take).max(0.0);
                }
            }
        }
        // Apply adjusted heights (top-anchored content, documented).
        for (p, &i) in order.iter().enumerate() {
            let target = base[p] + v_chrome(&items[i]);
            stretch_height(&mut items[i], target);
        }
    }

    // Main-axis placement (justify-content along the vertical axis).
    let finals: Vec<f32> = order
        .iter()
        .enumerate()
        .map(|(p, &i)| item_outer_h(base[p], &items[i]))
        .collect();
    let justify_free = if n > 0 {
        let avail = spec.definite_height.unwrap_or_else(|| {
            finals.iter().sum::<f32>()
        });
        (avail - finals.iter().sum::<f32>()).max(0.0)
    } else {
        0.0
    };
    let (lead, between) = justify_pair(spec.justify, justify_free, n);

    let mut cursor = content_y + lead;
    for (pos, &i) in order.iter().enumerate() {
        if pos > 0 {
            cursor += spec.row_gap + between;
        }
        let it = &mut items[i];
        let mt = it.margin.top.unwrap_or(0.0);
        let by = cursor + mt;
        let align = props[i].align_self.unwrap_or(spec.align_items);
        let bw = it.border_box.w;
        let hc = h_chrome(it);
        let ml = it.margin.left.unwrap_or(0.0);
        let mr = it.margin.right.unwrap_or(0.0);
        let bx = content_x
            + ml
            + match align {
                Pack::End => content_w - bw - hc - ml - mr,
                Pack::Center => (content_w - bw - hc - ml - mr) / 2.0,
                _ => 0.0,
            };
        translate(it, bx - it.border_box.x, by - it.border_box.y);
        cursor = by + base[pos] + v_chrome(it) + it.margin.bottom.unwrap_or(0.0);
    }

    cursor - lead - content_y
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extends a box's height without reflowing children (top-anchored
/// content) — cross-axis stretch and column flexing.
fn stretch_height(it: &mut LayoutBox, target_border_h: f32) {
    let delta = target_border_h - it.border_box.h;
    if delta <= 0.0 {
        return;
    }
    it.border_box.h += delta;
    it.content_box.h += delta;
}

/// Shifts a laid-out subtree by (`dx`, `dy`) — border/content boxes, line
/// boxes, word fragments, recursively.
pub(crate) fn translate(b: &mut LayoutBox, dx: f32, dy: f32) {
    b.border_box = shift(b.border_box, dx, dy);
    b.content_box = shift(b.content_box, dx, dy);
    for line in &mut b.lines {
        line.y_top += dy;
        line.baseline += dy;
        for w in &mut line.words {
            w.x += dx;
        }
        for image in &mut line.replaced {
            image.x += dx;
            image.y += dy;
        }
    }
    match &mut b.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for c in children {
                translate(c, dx, dy);
            }
        },
        Flow::Inlines(_) | Flow::Empty => {},
    }
}

fn shift(r: Rect, dx: f32, dy: f32) -> Rect {
    Rect { x: r.x + dx, y: r.y + dy, w: r.w, h: r.h }
}

/// Max-content width: the unwrapped run width for inline content, the
/// widest child (plus horizontal margins) for block/flex children, plus
/// this box's own horizontal padding and border.
pub(crate) fn max_content_width(b: &LayoutBox, fonts: &FontSource) -> f32 {
    let em = b.font_size;
    let pad = values::resolve_padding(&b.style, em, 0.0);
    let bor = values::resolve_borders(&b.style, em);
    let chrome = pad.left + pad.right + bor.left + bor.right;
    let inner = match &b.flow {
        Flow::Inlines(items) => inline_run_width(items, fonts),
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children
            .iter()
            .map(|c| {
                let m = values::resolve_margins(&c.style, c.font_size, 0.0);
                max_content_width(c, fonts) + m.left.unwrap_or(0.0) + m.right.unwrap_or(0.0)
            })
            .fold(0.0f32, f32::max),
        Flow::Empty => 0.0,
    };
    chrome + inner
}

/// Unwrapped width of an inline run: every word plus the collapsible
/// spaces between them (reuses the line breaker's tokenizer).
fn inline_run_width(items: &[crate::InlineNode], fonts: &FontSource) -> f32 {
    let tokens = inline::tokenize(items);
    let mut line_width = 0.0f32;
    let mut max_width = 0.0f32;
    for token in tokens {
        match token {
            inline::Token::Break => {
                max_width = max_width.max(line_width);
                line_width = 0.0;
            },
            inline::Token::Word { text, font_size, space_before, .. } => {
                if space_before && line_width > 0.0 {
                    line_width += fonts.measure(" ", font_size);
                }
                line_width += fonts.measure(&text, font_size);
            },
            inline::Token::Replaced { size, space_before, .. } => {
                if space_before && line_width > 0.0 {
                    line_width += fonts.measure(" ", 16.0);
                }
                line_width += size.width;
            },
        }
    }
    max_width.max(line_width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    /// Full pipeline: html+css → laid-out root at `width` px.
    fn lay(html: &str, css: &str, width: f32) -> LayoutBox {
        let dom = amazing_html::parse(html);
        let sheet = amazing_css::parse_stylesheet(css);
        let styled = amazing_css::compute(&dom, &[sheet]);
        let fonts = FontSource::fallback();
        let mut root = crate::box_tree::build_layout_tree(&dom, &styled, &fonts);
        block::layout_root(&mut root, width, &fonts);
        root
    }

    /// The flex ITEMS of the first element child of the root (the
    /// `display:flex` container div).
    fn kids<'a>(root: &'a LayoutBox) -> &'a Vec<LayoutBox> {
        let container = match &root.flow {
            Flow::Blocks(k) | Flow::Flex(k) => &k[0],
            _ => panic!("root has element children"),
        };
        match &container.flow {
            Flow::Flex(items) => items,
            other => panic!("first child is not a flex container: {other:?}"),
        }
    }

    #[test]
    fn equal_grow_splits_free_space_evenly() {
        let root = lay(
            "<body><div style=\"display:flex\"><div style=\"flex:1;height:10px\"></div><div style=\"flex:1;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        // Both items grow from basis 0 → equal widths filling the
        // container, adjacent to each other.
        assert!(close(k[0].border_box.w, k[1].border_box.w), "w0={} w1={}", k[0].border_box.w, k[1].border_box.w);
        assert!(close(k[1].border_box.x - (k[0].border_box.x + k[0].border_box.w), 0.0));
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn proportional_grow_is_two_to_one() {
        let root = lay(
            "<body><div style=\"display:flex\"><div style=\"flex:2;width:20px\"></div><div style=\"flex:1;width:40px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        let w0 = k[0].border_box.w;
        let w1 = k[1].border_box.w;
        assert!(close(w0, 2.0 * w1), "w0={w0} w1={w1}");
    }

    #[test]
    fn oversized_items_shrink_to_fit() {
        let root = lay(
            "<body><div style=\"display:flex\"><div style=\"width:600px;height:10px\"></div><div style=\"width:600px;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        let total = k[0].border_box.w + k[1].border_box.w;
        assert!(close(total, 784.0), "total={total} want 784");
        assert!(close(k[1].border_box.x, k[0].border_box.x + k[0].border_box.w));
    }

    #[test]
    fn justify_center_centers_the_row() {
        let root = lay(
            "<body><div style=\"display:flex;justify-content:center\"><div style=\"width:100px;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        // Container content x = body border x + 0 padding; body content
        // starts at 8. Item should start at 8 + (784-100)/2.
        assert!(close(k[0].border_box.x - 8.0, (784.0 - 100.0) / 2.0), "x={}", k[0].border_box.x);
    }

    #[test]
    fn justify_space_between_pins_to_edges() {
        let root = lay(
            "<body><div style=\"display:flex;justify-content:space-between\"><div style=\"width:50px;height:10px\"></div><div style=\"width:50px;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(close(k[0].border_box.x, 8.0));
        assert!(close(k[1].border_box.x + 50.0, 792.0), "right={}", k[1].border_box.x + 50.0);
    }

    #[test]
    fn align_center_centers_on_cross_axis() {
        let root = lay(
            "<body><div style=\"display:flex;align-items:center;height:100px\"><div style=\"width:50px;height:40px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(close(k[0].border_box.y - 8.0, 30.0), "y={}", k[0].border_box.y);
    }

    #[test]
    fn stretch_is_default_and_fills_height() {
        let root = lay(
            "<body><div style=\"display:flex;height:80px\"><div style=\"width:50px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(close(k[0].border_box.h, 80.0), "h={}", k[0].border_box.h);
    }

    #[test]
    fn wrap_makes_two_lines() {
        let root = lay(
            "<body><div style=\"display:flex;flex-wrap:wrap;width:300px\"><div style=\"width:200px;height:10px\"></div><div style=\"width:200px;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(k[1].border_box.y > k[0].border_box.y, "second item must be on line 2");
        assert!(close(k[0].border_box.y, k[1].border_box.y - 10.0));
    }

    #[test]
    fn column_direction_stacks_and_stretches_width() {
        let root = lay(
            "<body><div style=\"display:flex;flex-direction:column\"><div style=\"height:20px\"></div><div style=\"height:30px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(k[1].border_box.y == k[0].border_box.y + 20.0);
        assert!(close(k[0].border_box.w, 784.0), "stretch fills width: {}", k[0].border_box.w);
        assert!(close(k[1].border_box.w, 784.0));
    }

    #[test]
    fn auto_left_margin_pushes_item_right() {
        let root = lay(
            "<body><div style=\"display:flex\"><div style=\"width:100px;height:10px;margin-left:auto\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(close(k[0].border_box.x + 100.0, 792.0), "right edge={}", k[0].border_box.x + 100.0);
    }

    #[test]
    fn order_property_reorders_visual_placement() {
        let root = lay(
            "<body><div style=\"display:flex\"><div id=\"a\" style=\"order:2;width:50px;height:10px\"></div><div id=\"b\" style=\"order:1;width:50px;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        // Document order is a then b; visual order must swap.
        assert!(k[1].border_box.x < k[0].border_box.x, "b should come first visually");
    }

    #[test]
    fn column_gap_separates_items() {
        let root = lay(
            "<body><div style=\"display:flex;column-gap:20px\"><div style=\"flex:1;height:10px\"></div><div style=\"flex:1;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        let gap_x = k[1].border_box.x - (k[0].border_box.x + k[0].border_box.w);
        assert!(close(gap_x, 20.0), "gap={gap_x}");
    }

    #[test]
    fn anonymous_text_becomes_an_item() {
        let root = lay(
            "<body><div style=\"display:flex\">hi<div style=\"width:50px;height:10px\"></div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert_eq!(k.len(), 2, "text run + div = 2 items");
        assert!(k[0].node.is_none(), "first item is anonymous");
        // The text item sits left of the div (max-content fits).
        assert!(k[0].border_box.x < k[1].border_box.x);
    }
}
