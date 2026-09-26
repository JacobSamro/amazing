//! Flexbox layout (css-flexbox-1 subset): flex containers resolve their
//! items along a main axis with grow/shrink/basis distribution, wrap into
//! lines, and align on the cross axis.
//!
//! Implemented:
//! - `display: flex` / `inline-flex` (identical at layout time — inline
//!   level outer display is a later-phase concern)
//! - flex-direction: row / row-reverse / column / column-reverse
//! - flex-wrap: nowrap / wrap (row mode); align-content distribution and
//!   multi-line stretch against a definite container height
//! - flex-grow / flex-shrink / flex-basis + `flex` shorthand forms
//! - iterative §9.7 flexible-length resolution with min/max freezing,
//!   scaled shrink factors, automatic minimums, and intrinsic keywords
//! - order (stable sort), gap / row-gap / column-gap
//! - justify-content: flex-start/end, center, space-between/around/evenly
//! - align-items / align-self: stretch, flex-start/end, center, first baseline
//! - physical/logical margins and negative outer sizes; main-axis auto
//!   margins absorb free space before justify-content
//!
//! Deviations (documented):
//! - column mode is single-line unless the main size is definite
//! - cross-axis auto margins count as zero
//! - transferred aspect-ratio suggestions and inline outer display are
//!   deferred

use amazing_css::ComputedStyle;

use crate::font::FontSource;
use crate::flex_algorithm::{FlexConstraint, resolve_flexible_lengths};
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

fn physical_direction(style: &ComputedStyle, logical: Direction) -> Direction {
    let rtl = style
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let mode = style.get("writing-mode").unwrap_or("horizontal-tb");
    if mode.eq_ignore_ascii_case("horizontal-tb") {
        return match (logical, rtl) {
            (Direction::Row, true) => Direction::RowReverse,
            (Direction::RowReverse, true) => Direction::Row,
            _ => logical,
        };
    }
    let vertical_rl = mode.eq_ignore_ascii_case("vertical-rl")
        || mode.eq_ignore_ascii_case("sideways-rl");
    let sideways_lr = mode.eq_ignore_ascii_case("sideways-lr");
    match logical {
        Direction::Row if rtl ^ sideways_lr => Direction::ColumnReverse,
        Direction::Row => Direction::Column,
        Direction::RowReverse if rtl ^ sideways_lr => Direction::Column,
        Direction::RowReverse => Direction::ColumnReverse,
        Direction::Column if vertical_rl => Direction::RowReverse,
        Direction::Column => Direction::Row,
        Direction::ColumnReverse if vertical_rl => Direction::Row,
        Direction::ColumnReverse => Direction::RowReverse,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pack {
    Start,
    End,
    Center,
    Baseline,
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
#[derive(Clone, Copy)]
struct ContainerSpec {
    direction: Direction,
    wrap: bool,
    wrap_reverse: bool,
    justify: Pack,
    align_items: Pack,
    align_content: Pack,
    row_gap: f32,
    col_gap: f32,
    definite_height: Option<f32>,
}

impl ContainerSpec {
    fn of(style: &ComputedStyle, font_px: f32, content_w: f32, definite_height: Option<f32>) -> Self {
        let logical_direction = match style.get("flex-direction").unwrap_or("row") {
            v if v.eq_ignore_ascii_case("row-reverse") => Direction::RowReverse,
            v if v.eq_ignore_ascii_case("column") => Direction::Column,
            v if v.eq_ignore_ascii_case("column-reverse") => Direction::ColumnReverse,
            _ => Direction::Row,
        };
        let direction = physical_direction(style, logical_direction);
        let wrap = style
            .get("flex-wrap")
            .map(|v| v.eq_ignore_ascii_case("wrap") || v.eq_ignore_ascii_case("wrap-reverse"))
            .unwrap_or(false);
        // The solvers place cross lines left-to-right/top-to-bottom. Map
        // logical cross-start first, then apply wrap-reverse (Flexbox §5.2).
        let mode = style.get("writing-mode").unwrap_or("horizontal-tb");
        let cross_reversed = match logical_direction {
            Direction::Row | Direction::RowReverse =>
                mode.eq_ignore_ascii_case("vertical-rl") || mode.eq_ignore_ascii_case("sideways-rl"),
            Direction::Column | Direction::ColumnReverse =>
                style.get("direction").is_some_and(|value| value.eq_ignore_ascii_case("rtl"))
                    ^ mode.eq_ignore_ascii_case("sideways-lr"),
        };
        let wrap_reverse = cross_reversed ^ style.get("flex-wrap")
            .is_some_and(|value| value.eq_ignore_ascii_case("wrap-reverse"));
        let justify = parse_pack(style.get("justify-content"), Pack::Start);
        let align_items = parse_align(style.get("align-items"), Pack::Stretch);
        let align_content = if style
            .get("align-content")
            .map(|v| {
                v.eq_ignore_ascii_case("normal") || v.eq_ignore_ascii_case("stretch")
            })
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
            wrap_reverse,
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
    let v = v.map(strip_overflow_position);
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

fn parse_align(v: Option<&str>, default: Pack) -> Pack {
    let v = v.map(strip_overflow_position);
    match v {
        Some(v) if v.eq_ignore_ascii_case("baseline") || v.eq_ignore_ascii_case("first baseline") => Pack::Baseline,
        Some(v) if v.eq_ignore_ascii_case("flex-start") || v.eq_ignore_ascii_case("start") => {
            Pack::Start
        },
        Some(v) if v.eq_ignore_ascii_case("flex-end") || v.eq_ignore_ascii_case("end") => Pack::End,
        Some(v) if v.eq_ignore_ascii_case("center") => Pack::Center,
        Some(v) if v.eq_ignore_ascii_case("stretch") => Pack::Stretch,
        _ => default,
    }
}

pub(crate) fn strip_overflow_position(value: &str) -> &str {
    let value = value.trim();
    if let Some((keyword, position)) = value.split_once(' ')
        && (keyword.eq_ignore_ascii_case("safe") || keyword.eq_ignore_ascii_case("unsafe"))
    {
        position.trim()
    } else {
        value
    }
}

/// `gap: r c` / `row-gap` / `column-gap`. Percentages resolve against the
/// inline size for both axes (row-gap % against height unsupported → same
/// base, documented).
fn parse_gaps(style: &ComputedStyle, font_px: f32, content_w: f32) -> (f32, f32) {
    let px = |s: &str| values::parse_used_maybe_len(style, s, font_px, content_w)
        .map_or(0.0, |m| m.unwrap_or(0.0));
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
                if let Some(n) = parse_flex_number(parts[0]) {
                    // `flex: <number>` → grow n, basis 0% (§7.1.1).
                    grow = n;
                    basis = Some(MaybeLen::Px(0.0));
                } else {
                    // One-value length form: `flex: 10em` → 1 1 10em.
                    grow = 1.0;
                    shrink = 1.0;
                    basis = values::parse_used_maybe_len(style, parts[0], font_px, main_extent);
                }
            } else {
                let nums: Vec<f32> = parts
                    .iter()
                    .filter_map(|part| parse_flex_number(part))
                    .collect();
                grow = nums.first().copied().unwrap_or(0.0);
                shrink = nums.get(1).copied().unwrap_or(1.0);
                // First non-number token is the basis (`auto` parses to Auto).
                basis = parts
                    .iter()
                    .find(|part| parse_flex_number(part).is_none())
                    .and_then(|p| values::parse_used_maybe_len(style, p, font_px, main_extent))
                    .or_else(|| nums.get(2).map(|n| MaybeLen::Px(*n)));
            }
        }
        if let Some(v) = style.get("flex-grow") {
            grow = parse_flex_number(v).unwrap_or(grow);
        }
        if let Some(v) = style.get("flex-shrink") {
            shrink = parse_flex_number(v).unwrap_or(shrink);
        }
        if let Some(v) = style.get("flex-basis") {
            basis = values::style_length_percentage(style, "flex-basis", font_px, font_px * 0.6)
                .and_then(|value| value.resolve(Some(main_extent)))
                .map(MaybeLen::Px)
                .or_else(|| values::parse_used_maybe_len(style, v, font_px, main_extent))
                .or(basis);
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

fn parse_flex_number(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("calc(infinity)") {
        Some(f32::INFINITY)
    } else if value.eq_ignore_ascii_case("calc(-infinity)") {
        Some(f32::NEG_INFINITY)
    } else if value.eq_ignore_ascii_case("calc(nan)") {
        Some(f32::NAN)
    } else {
        value.parse().ok()
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
    let spec = ContainerSpec::of(container, font_px, content_w, definite_height);
    let main_extent = match spec.direction {
        Direction::Row | Direction::RowReverse => content_w,
        Direction::Column | Direction::ColumnReverse => definite_height.unwrap_or(0.0),
    };
    let props: Vec<ItemProps> = items
        .iter()
        .map(|it| ItemProps::of(&it.style, it.font_size, main_extent))
        .collect();
    // Stable sort by `order` (css-flexbox-1 §4.4).
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| props[i].order);

    let content_height = match spec.direction {
        Direction::Row | Direction::RowReverse => {
            layout_row(items, &props, &order, &spec, content_x, content_y, content_w, fonts)
        },
        Direction::Column | Direction::ColumnReverse if spec.wrap && definite_height.is_some() => {
            layout_wrapped_column(items, &props, &order, &spec, content_x, content_y, content_w, fonts)
        },
        Direction::Column | Direction::ColumnReverse => {
            layout_column(items, &props, &order, &spec, content_x, content_y, content_w, fonts)
        },
    };
    if spec.wrap_reverse {
        match spec.direction {
            Direction::Row | Direction::RowReverse => {
                let cross_size = definite_height.unwrap_or(content_height);
                for item in items {
                    let new_y = content_y + cross_size - (item.border_box.y - content_y) - item.border_box.h;
                    translate(item, 0.0, new_y - item.border_box.y);
                }
            },
            Direction::Column | Direction::ColumnReverse => {
                for item in items {
                    let new_x = content_x + content_w - (item.border_box.x - content_x) - item.border_box.w;
                    translate(item, new_x - item.border_box.x, 0.0);
                }
            },
        }
    }
    content_height
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
/// Outer cross size of a laid-out item. The border box already contains
/// padding and border, so only margins are added here.
fn outer_cross(items: &[LayoutBox], i: usize) -> f32 {
    items[i].border_box.h
        + items[i].margin.top.unwrap_or(0.0)
        + items[i].margin.bottom.unwrap_or(0.0)
}

/// The first in-flow line supplies the alignment baseline. A box without a
/// usable line synthesizes one at its border-box block end.
pub(crate) fn first_baseline(item: &LayoutBox) -> f32 {
    if let Some(line) = item.lines.first() {
        return line.baseline;
    }
    let children = match &item.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children,
        _ => return item.border_box.y + item.border_box.h,
    };
    children
        .first()
        .map(first_baseline)
        .unwrap_or(item.border_box.y + item.border_box.h)
}

fn resolve_model(it: &mut LayoutBox, cbw: f32) {
    it.margin = values::resolve_margins(&it.style, it.font_size, cbw);
    it.padding = values::resolve_padding(&it.style, it.font_size, cbw);
    it.border = values::resolve_borders(&it.style, it.font_size);
}

fn main_limits(it: &LayoutBox, containing: f32, fonts: &FontSource) -> (f32, f32) {
    let min_content = min_content_inner_width(it, fonts);
    let max_content = max_content_inner_width(it, fonts).max(min_content);
    let calc_size_limit = |source: &str| {
        let basis = match values::calc_size_basis(source)? {
            "auto" | "min-content" => min_content,
            "max-content" => max_content,
            _ => return None,
        };
        values::used_calc_size_with_basis(
            &it.style, source,
            values::FontRelativeUnits::legacy(it.font_size, it.font_size * 0.6),
            basis, Some(containing),
        )
    };
    let maximum = match it.style.get("max-width") {
        Some(value) if value.eq_ignore_ascii_case("min-content") => min_content,
        Some(value) if value.eq_ignore_ascii_case("max-content") => max_content,
        Some(source) => calc_size_limit(source).or_else(|| values::style_length_percentage(&it.style, "max-width", it.font_size, it.font_size * 0.6)
            .and_then(|value| value.resolve(Some(containing)))
        ).unwrap_or(f32::INFINITY),
        None => f32::INFINITY,
    }
    .max(0.0);
    let minimum = match it.style.get("min-width") {
        Some(value) if value.eq_ignore_ascii_case("min-content") => min_content,
        Some(value) if value.eq_ignore_ascii_case("max-content") => max_content,
        Some(value) if !value.eq_ignore_ascii_case("auto") => {
            calc_size_limit(value).or_else(|| values::style_length_percentage(&it.style, "min-width", it.font_size, it.font_size * 0.6)
                .and_then(|value| value.resolve(Some(containing))))
                .unwrap_or(0.0)
        },
        _ if automatic_minimum_applies(&it.style) => {
            match values::resolve_extent(&it.style, "width", it.font_size, containing) {
                MaybeLen::Px(specified) => min_content.min(specified),
                MaybeLen::Auto => min_content,
            }
        },
        _ => 0.0,
    }
    .max(0.0)
    .min(maximum);
    (minimum, maximum)
}

fn automatic_minimum_applies(style: &ComputedStyle) -> bool {
    ["overflow", "overflow-x", "overflow-y"]
        .into_iter()
        .filter_map(|property| style.get(property))
        .all(|overflow| {
            overflow.eq_ignore_ascii_case("visible") || overflow.eq_ignore_ascii_case("clip")
        })
}

fn column_limits(it: &LayoutBox, measured_content: f32, containing: f32) -> (f32, f32) {
    let calc_size_limit = |source: &str| {
        match values::calc_size_basis(source)? {
            "auto" | "min-content" | "max-content" | "fit-content" =>
                values::used_calc_size_with_basis(
                    &it.style, source,
                    values::FontRelativeUnits::legacy(it.font_size, it.font_size * 0.6),
                    measured_content, Some(containing),
                ),
            _ => None,
        }
    };
    let maximum = match it.style.get("max-height") {
        Some(value)
            if value.eq_ignore_ascii_case("min-content")
                || value.eq_ignore_ascii_case("max-content") =>
        {
            measured_content
        },
        Some(source) => calc_size_limit(source).or_else(|| values::style_length_percentage(&it.style, "max-height", it.font_size, it.font_size * 0.6)
            .and_then(|value| value.resolve(Some(containing))))
            .unwrap_or(f32::INFINITY),
        None => f32::INFINITY,
    }
    .max(0.0);
    let minimum = match it.style.get("min-height") {
        Some(value)
            if value.eq_ignore_ascii_case("min-content")
                || value.eq_ignore_ascii_case("max-content") =>
        {
            measured_content
        },
        Some(value) if !value.eq_ignore_ascii_case("auto") => {
            let automatic = values::resolve_extent(&it.style, "height", it.font_size, containing);
            let basis = match automatic {
                MaybeLen::Px(specified) => measured_content.min(specified),
                MaybeLen::Auto => measured_content,
            };
            if values::calc_size_basis(value).is_some() {
                values::used_calc_size_with_basis(
                    &it.style, value,
                    values::FontRelativeUnits::legacy(it.font_size, it.font_size * 0.6),
                    basis, Some(containing),
                ).unwrap_or(0.0)
            } else {
                values::style_length_percentage(&it.style, "min-height", it.font_size, it.font_size * 0.6)
                    .and_then(|value| value.resolve(Some(containing))).unwrap_or(0.0)
            }
        },
        _ if automatic_minimum_applies(&it.style) => {
            match values::resolve_extent(&it.style, "height", it.font_size, containing) {
                MaybeLen::Px(specified) => measured_content.min(specified),
                MaybeLen::Auto => measured_content,
            }
        },
        _ => 0.0,
    }
    .max(0.0)
    .min(maximum);
    (minimum, maximum)
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
                    MaybeLen::Auto => {
                        (max_content_width(it, fonts) - h_chrome(it)).max(0.0).min(content_w)
                    },
                },
            }
        })
        .collect();
    let limits = items
        .iter()
        .map(|item| main_limits(item, content_w, fonts))
        .collect::<Vec<_>>();
    let hypothetical = base
        .iter()
        .enumerate()
        .map(|(index, size)| size.clamp(limits[index].0, limits[index].1))
        .collect::<Vec<_>>();

    // Wrap into lines (greedy by outer base + column gaps).
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut used = 0.0f32;
    for &i in order {
        let gap = if cur.is_empty() { 0.0 } else { spec.col_gap };
        if spec.wrap
            && !cur.is_empty()
            && used + gap + outer_main(items, i, hypothetical[i]) > content_w
        {
            lines.push(std::mem::take(&mut cur));
            used = 0.0;
        }
        let gap = if cur.is_empty() { 0.0 } else { spec.col_gap };
        used += gap + outer_main(items, i, hypothetical[i]);
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
        let constraints = line
            .iter()
            .map(|&i| FlexConstraint {
                base: base[i],
                hypothetical: hypothetical[i],
                min: limits[i].0,
                max: limits[i].1,
                grow: props[i].grow,
                shrink: props[i].shrink,
                scaled_shrink: props[i].shrink * base[i],
                outer: outer_main(items, i, 0.0),
            })
            .collect::<Vec<_>>();
        let resolved = resolve_flexible_lengths(content_w - gaps_total, &constraints);
        for (&i, size) in line.iter().zip(resolved) {
            base[i] = size;
        }
        let used = line.iter().map(|&i| outer_main(items, i, base[i])).sum::<f32>()
            + gaps_total;
        let free = content_w - used;

        // Materialize every flexed content-box width. `layout_block` accepts
        // a containing width, not a used item width; leaving an auto width in
        // place would subtract the item's own border/padding a second time.
        for &i in line {
            items[i]
                .style
                .set("width", &format!("{}px", base[i].max(0.0)));
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
        // A baseline-sharing group contributes its largest ascent plus its
        // largest descent to the line cross size. Those extrema can belong to
        // different items, so the tallest single item is not sufficient.
        let baseline_metrics = line
            .iter()
            .filter(|&&i| props[i].align_self.unwrap_or(spec.align_items) == Pack::Baseline)
            .filter(|&&i| {
                !margin_is_auto(&items[i].style, "top")
                    && !margin_is_auto(&items[i].style, "bottom")
            })
            .map(|&i| {
                let baseline = first_baseline(&items[i]) - items[i].border_box.y;
                let ascent = items[i].margin.top.unwrap_or(0.0) + baseline;
                (ascent, outer_cross(items, i) - ascent)
            })
            .fold(None, |acc: Option<(f32, f32)>, (ascent, descent)| {
                Some(match acc {
                    Some((a, d)) => (a.max(ascent), d.max(descent)),
                    None => (ascent, descent),
                })
            });
        if let Some((ascent, descent)) = baseline_metrics {
            line_cross = line_cross.max(ascent + descent);
        }
        // A single-line flex container's line fills its definite cross size
        // regardless of align-content. That property distributes *lines* in
        // multi-line containers; align-items positions items within this line.
        if lines.len() == 1 {
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
                let target = line_cross
                    - items[i].margin.top.unwrap_or(0.0)
                    - items[i].margin.bottom.unwrap_or(0.0);
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
        // Margins are resolved fresh from the style after auto-margin
        // absorption so the cursor advances by the margins items actually
        // carry; the solver's constraints above intentionally still saw auto
        // margins as zero, and `layout_block` re-solves the model's margins,
        // so the struct field is not authoritative here.
        let finals: Vec<f32> = line
            .iter()
            .map(|&i| {
                let margin =
                    values::resolve_margins(&items[i].style, items[i].font_size, content_w);
                base[i]
                    + h_chrome(&items[i])
                    + margin.left.unwrap_or(0.0)
                    + margin.right.unwrap_or(0.0)
            })
            .collect();
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
            // Block layout solves CSS2 horizontal constraints and may fill
            // an auto opposite margin. Flex placement uses the item's
            // authored/resolved margins instead.
            let margin = values::resolve_margins(&it.style, it.font_size, content_w);
            let ml = margin.left.unwrap_or(0.0);
            let mr = margin.right.unwrap_or(0.0);
            let bx = if reverse {
                cursor - mr - h_chrome(it) - base[i]
            } else {
                cursor + ml
            };
            let align = props[i].align_self.unwrap_or(spec.align_items);
            let bh = it.border_box.h;
            let vc = v_chrome(it);
            let by = if align == Pack::Baseline {
                if let Some((ascent, _)) = baseline_metrics {
                    line_top + ascent - (first_baseline(it) - it.border_box.y)
                } else {
                    line_top + it.margin.top.unwrap_or(0.0)
                }
            } else {
                line_top
                    + it.margin.top.unwrap_or(0.0)
                    + match align {
                        Pack::End => line_cross - bh - vc,
                        Pack::Center => (line_cross - bh - vc) / 2.0,
                        _ => 0.0,
                    }
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
            if spec.align_content == Pack::Stretch {
                let extra = spare / lines.len() as f32;
                let mut shift = 0.0;
                for (line_index, line) in lines.iter().enumerate() {
                    for &item_index in line {
                        if shift > 0.0 {
                            translate(&mut items[item_index], 0.0, shift);
                        }
                        let align = props[item_index].align_self.unwrap_or(spec.align_items);
                        let height_auto = matches!(
                            values::resolve_extent(
                                &items[item_index].style,
                                "height",
                                items[item_index].font_size,
                                0.0,
                            ),
                            MaybeLen::Auto
                        );
                        if align == Pack::Stretch
                            && height_auto
                            && !margin_is_auto(&items[item_index].style, "top")
                            && !margin_is_auto(&items[item_index].style, "bottom")
                        {
                            let target = line_crosses[line_index]
                                + extra
                                - items[item_index].margin.top.unwrap_or(0.0)
                                - items[item_index].margin.bottom.unwrap_or(0.0);
                            stretch_height(&mut items[item_index], target.max(0.0));
                        }
                    }
                    line_crosses[line_index] += extra;
                    shift += extra;
                }
                content_height = def_h;
            } else {
                let (lead, between) = justify_pair(spec.align_content, spare, lines.len());
                for (line_index, line) in lines.iter().enumerate() {
                    let shift = lead + between * line_index as f32;
                    for &i in line {
                        if shift > 0.0 {
                            translate(&mut items[i], 0.0, shift);
                        }
                    }
                }
            }
        }
    }

    content_height
}

// ---------------------------------------------------------------------------
// Column direction
// ---------------------------------------------------------------------------

/// A definite column main size permits flex line collection. Each collected
/// line uses the ordinary column solver; the cross sizes and column gaps place
/// those lines from cross-start. In vertical writing modes a logical row is a
/// physical column, so this is also the wrapping path for `row` there.
fn layout_wrapped_column(
    items: &mut Vec<LayoutBox>, props: &[ItemProps], order: &[usize], spec: &ContainerSpec,
    content_x: f32, content_y: f32, content_w: f32, fonts: &FontSource,
) -> f32 {
    let height = spec.definite_height.unwrap();
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for &index in order {
        let item = &mut items[index];
        resolve_model(item, content_w);
        let margin = values::resolve_margins(&item.style, item.font_size, content_w);
        let width = match values::resolve_extent(&item.style, "width", item.font_size, content_w) {
            MaybeLen::Px(value) => value.max(0.0),
            MaybeLen::Auto => max_content_width(item, fonts).min(content_w),
        };
        crate::block::layout_block(item, 0.0, 0.0, width, fonts);
        let main = props[index].basis_px.or_else(|| {
            match values::resolve_extent(&item.style, "height", item.font_size, height) {
                MaybeLen::Px(value) => Some(value),
                MaybeLen::Auto => None,
            }
        }).unwrap_or((item.border_box.h - v_chrome(item)).max(0.0))
            + v_chrome(item) + margin.top.unwrap_or(0.0) + margin.bottom.unwrap_or(0.0);
        let next = if lines.last().is_some_and(|line| !line.is_empty()) {
            used + spec.row_gap + main
        } else {
            main
        };
        if next > height && lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(Vec::new());
            used = main;
        } else {
            if lines.is_empty() { lines.push(Vec::new()); }
            used = next;
        }
        lines.last_mut().unwrap().push(index);
    }
    let mut widths = lines.iter().map(|line| {
        line.iter().map(|&index| {
            let item = &items[index];
            let margin = values::resolve_margins(&item.style, item.font_size, content_w);
            item.border_box.w + margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0)
        }).fold(0.0f32, f32::max)
    }).collect::<Vec<_>>();
    let total = widths.iter().sum::<f32>() + spec.col_gap * widths.len().saturating_sub(1) as f32;
    if spec.align_content == Pack::Stretch && total < content_w && !widths.is_empty() {
        let extra = (content_w - total) / widths.len() as f32;
        for width in &mut widths { *width += extra; }
    }
    let mut owned = std::mem::take(items).into_iter().map(Some).collect::<Vec<_>>();
    let mut x = content_x;
    for (line, width) in lines.iter().zip(widths) {
        let mut group = line.iter().map(|&index| owned[index].take().unwrap()).collect::<Vec<_>>();
        let group_props = line.iter().map(|&index| props[index]).collect::<Vec<_>>();
        let group_order = (0..line.len()).collect::<Vec<_>>();
        let line_spec = ContainerSpec { wrap: false, ..*spec };
        layout_column(&mut group, &group_props, &group_order, &line_spec, x, content_y, width, fonts);
        for (&index, item) in line.iter().zip(group) { owned[index] = Some(item); }
        x += width + spec.col_gap;
    }
    *items = owned.into_iter().map(Option::unwrap).collect();
    height
}

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
    let mut layout_widths = vec![0.0; items.len()];
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
        layout_widths[i] = w;
        crate::block::layout_block(&mut items[i], 0.0, 0.0, w, fonts);
    }

    // Main (vertical) sizes use the same §9.7 solver as rows. Measurement
    // supplies content suggestions; explicit flex-basis/height supplies the
    // base when definite.
    let n = order.len();
    let gaps_total = spec.row_gap * n.saturating_sub(1) as f32;
    let measured = order
        .iter()
        .map(|&i| (items[i].border_box.h - v_chrome(&items[i])).max(0.0))
        .collect::<Vec<_>>();
    let bases = order
        .iter()
        .enumerate()
        .map(|(position, &i)| {
            props[i].basis_px.unwrap_or_else(|| {
                match values::resolve_extent(
                    &items[i].style,
                    "height",
                    items[i].font_size,
                    spec.definite_height.unwrap_or(0.0),
                ) {
                    MaybeLen::Px(height) => height.max(0.0),
                    MaybeLen::Auto => measured[position],
                }
            })
        })
        .collect::<Vec<_>>();
    let limits = order
        .iter()
        .enumerate()
        .map(|(position, &i)| {
            column_limits(
                &items[i],
                measured[position],
                spec.definite_height.unwrap_or(0.0),
            )
        })
        .collect::<Vec<_>>();
    let constraints = order
        .iter()
        .enumerate()
        .map(|(position, &i)| {
            let margin = values::resolve_margins(&items[i].style, items[i].font_size, content_w);
            FlexConstraint {
                base: bases[position],
                hypothetical: bases[position].clamp(limits[position].0, limits[position].1),
                min: limits[position].0,
                max: limits[position].1,
                grow: props[i].grow,
                shrink: props[i].shrink,
                scaled_shrink: props[i].shrink * bases[position],
                outer: v_chrome(&items[i])
                    + margin.top.unwrap_or(0.0)
                    + margin.bottom.unwrap_or(0.0),
            }
        })
        .collect::<Vec<_>>();
    let base = spec.definite_height.map_or_else(
        || constraints.iter().map(|item| item.hypothetical).collect(),
        |height| resolve_flexible_lengths(height - gaps_total, &constraints),
    );
    for (position, &i) in order.iter().enumerate() {
        items[i]
            .style
            .set("height", &format!("{}px", base[position].max(0.0)));
        crate::block::layout_block(&mut items[i], 0.0, 0.0, layout_widths[i], fonts);
    }

    // Main-axis placement (justify-content along the vertical axis).
    let used: f32 = base
        .iter()
        .enumerate()
        .map(|(position, size)| size + constraints[position].outer)
        .sum();
    let mut justify_free = if n > 0 {
        let avail = spec.definite_height.unwrap_or(used + gaps_total);
        (avail - used - gaps_total).max(0.0)
    } else {
        0.0
    };
    // Auto main margins absorb remaining free space before justify-content
    // (§8.1), mirroring the row path. Only a definite container height
    // leaves free space to absorb.
    if justify_free > 0.0 {
        let mut auto_sides = 0usize;
        for &i in order {
            if margin_is_auto(&items[i].style, "top") {
                auto_sides += 1;
            }
            if margin_is_auto(&items[i].style, "bottom") {
                auto_sides += 1;
            }
        }
        if auto_sides > 0 {
            let share = justify_free / auto_sides as f32;
            for &i in order {
                for side in ["top", "bottom"] {
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
    }
    // Placement steps by post-absorption outer sizes, resolved fresh from
    // the style for the same reason as the row path.
    let finals: Vec<f32> = order
        .iter()
        .enumerate()
        .map(|(position, &i)| {
            let margin = values::resolve_margins(&items[i].style, items[i].font_size, content_w);
            base[position]
                + v_chrome(&items[i])
                + margin.top.unwrap_or(0.0)
                + margin.bottom.unwrap_or(0.0)
        })
        .collect();
    let (lead, between) = justify_pair(spec.justify, justify_free, n);

    let reverse = spec.direction == Direction::ColumnReverse;
    let available = spec
        .definite_height
        .unwrap_or_else(|| finals.iter().sum::<f32>() + gaps_total);
    let mut cursor = if reverse {
        content_y + available - lead
    } else {
        content_y + lead
    };
    for (pos, &i) in order.iter().enumerate() {
        if pos > 0 {
            if reverse {
                cursor -= spec.row_gap + between;
            } else {
                cursor += spec.row_gap + between;
            }
        }
        let it = &mut items[i];
        let margin = values::resolve_margins(&it.style, it.font_size, content_w);
        let mt = margin.top.unwrap_or(0.0);
        let outer = finals[pos];
        let by = if reverse {
            cursor -= outer;
            cursor + mt
        } else {
            cursor + mt
        };
        let align = props[i].align_self.unwrap_or(spec.align_items);
        let bw = it.border_box.w;
        let ml = margin.left.unwrap_or(0.0);
        let mr = margin.right.unwrap_or(0.0);
        let bx = content_x
            + ml
            + match align {
                Pack::End => content_w - bw - ml - mr,
                Pack::Center => (content_w - bw - ml - mr) / 2.0,
                _ => 0.0,
            };
        translate(it, bx - it.border_box.x, by - it.border_box.y);
        if !reverse {
            cursor += outer;
        }
    }

    finals.iter().sum::<f32>() + gaps_total
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
    for segment in &mut b.table_decoration.borders {segment.rect=shift(segment.rect,dx,dy);}
    for hole in &mut b.table_decoration.background_holes {*hole=shift(*hole,dx,dy);}
    b.content_box = shift(b.content_box, dx, dy);
    for line in &mut b.lines {
        line.y_top += dy;
        line.baseline += dy;
        for w in &mut line.words {
            w.x += dx;
        }
        for glyph in &mut line.glyphs {
            glyph.x += dx;
        }
        for decoration in &mut line.decorations {
            decoration.x += dx;
            decoration.y += dy;
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
    for child in &mut b.positioned {
        if child.inline_atomic {
            translate(child, dx, dy);
        }
    }
}

/// Move an atomic inline after shaping has changed its inline position.
/// Absolute descendants were already placed against the pre-shaping position;
/// move them with the atomic subtree. Fixed descendants remain viewport-based.
pub(crate) fn translate_after_positioning(b: &mut LayoutBox, dx: f32, dy: f32) {
    translate(b, dx, dy);
    translate_absolute_descendants(b, dx, dy);
}

fn translate_absolute_descendants(b: &mut LayoutBox, dx: f32, dy: f32) {
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &mut b.flow {
        for child in children {
            translate_absolute_descendants(child, dx, dy);
        }
    }
    for child in &mut b.positioned {
        if child.inline_atomic {
            translate_absolute_descendants(child, dx, dy);
        } else if !child.style.get("position").is_some_and(|value| value.eq_ignore_ascii_case("fixed")) {
            translate(child, dx, dy);
            translate_absolute_descendants(child, dx, dy);
        }
    }
}

fn shift(r: Rect, dx: f32, dy: f32) -> Rect {
    Rect { x: r.x + dx, y: r.y + dy, w: r.w, h: r.h }
}

/// Max-content width: the unwrapped run width for inline content, the
/// widest child (plus horizontal margins) for block/flex children, plus
/// this box's own horizontal padding and border.
pub(crate) fn max_content_width(b: &LayoutBox, fonts: &FontSource) -> f32 {
    if b.table_wrapper {return max_content_inner_width(b,fonts);}
    if crate::table_tree::role(b) == crate::table_tree::Role::Table { return crate::table::intrinsic(b, fonts).1; }
    let em = b.font_size;
    let pad = values::resolve_padding(&b.style, em, 0.0);
    let bor = values::resolve_borders(&b.style, em);
    let chrome = pad.left + pad.right + bor.left + bor.right;
    let inner = match intrinsic_preferred_width(b) {
        Some(width) => width,
        None => max_content_inner_width(b, fonts),
    };
    let inner = intrinsic_calc_size(b, "width", inner, fonts).unwrap_or(inner);
    chrome + intrinsic_width_constraints(b, inner, fonts)
}

/// Approximate min-content width: the widest unbreakable inline token or
/// descendant min-content contribution, plus this box's horizontal chrome.
pub(crate) fn min_content_width(b: &LayoutBox, fonts: &FontSource) -> f32 {
    if b.table_wrapper {return min_content_inner_width(b,fonts);}
    if crate::table_tree::role(b) == crate::table_tree::Role::Table { return crate::table::intrinsic(b, fonts).0; }
    let em = b.font_size;
    let pad = values::resolve_padding(&b.style, em, 0.0);
    let bor = values::resolve_borders(&b.style, em);
    let chrome = pad.left + pad.right + bor.left + bor.right;
    let inner = match intrinsic_preferred_width(b) {
        Some(width) => width,
        None => min_content_inner_width(b, fonts),
    };
    let inner = intrinsic_calc_size(b, "width", inner, fonts).unwrap_or(inner);
    chrome + intrinsic_width_constraints(b, inner, fonts)
}

/// Cyclic preferred percentages behave as auto for intrinsic contributions
/// (CSS Sizing 3 §5.2.1); zero is not a definite percentage basis here.
fn intrinsic_preferred_width(b: &LayoutBox) -> Option<f32> {
    if b.style.get("width").is_some_and(|value| value.contains('%')) {
        return None;
    }
    let MaybeLen::Px(width) = values::resolve_extent(&b.style, "width", b.font_size, 0.0) else {
        return None;
    };
    Some(width.max(0.0))
}

fn intrinsic_calc_size(b: &LayoutBox, property: &str, automatic: f32, fonts: &FontSource) -> Option<f32> {
    let source = b.style.get(property)?;
    let basis = match values::calc_size_basis(source)? {
        "auto" | "fit-content" => automatic,
        "min-content" => min_content_inner_width(b, fonts),
        "max-content" => max_content_inner_width(b, fonts),
        _ => return None,
    };
    values::used_calc_size_with_basis(
        &b.style, source,
        values::FontRelativeUnits::legacy(b.font_size, b.font_size * 0.6),
        basis, Some(0.0),
    ).map(|value| value.max(0.0))
}

fn intrinsic_width_constraints(b: &LayoutBox, inner: f32, fonts: &FontSource) -> f32 {
    // CSS Sizing 3 §5.2.1: a cyclic percentage in a min size contributes
    // zero during intrinsic sizing. The definite length term still counts.
    let min = values::style_length_percentage(&b.style, "min-width", b.font_size, b.font_size * 0.6)
        .and_then(|value| value.resolve(Some(0.0)))
        .or_else(|| intrinsic_calc_size(b, "min-width", inner, fonts))
        .unwrap_or(0.0).max(0.0);
    // Cyclic percentages in a maximum size behave as the initial `none`.
    let max = values::style_length_percentage(&b.style, "max-width", b.font_size, b.font_size * 0.6)
        .filter(|value| !value.has_percentage)
        .and_then(|value| value.resolve(None))
        .or_else(|| intrinsic_calc_size(b, "max-width", inner, fonts));
    max.map_or(inner, |maximum| inner.min(maximum.max(0.0))).max(min)
}

pub(crate) fn max_content_inner_width(b: &LayoutBox, fonts: &FontSource) -> f32 {
    match &b.flow {
        Flow::Inlines(items) => inline_run_width(items, fonts),
        Flow::Flex(children) if matches!(
            ContainerSpec::of(&b.style, b.font_size, 0.0, None).direction,
            Direction::Row | Direction::RowReverse
        ) => {
            let gap = parse_gaps(&b.style, b.font_size, 0.0).1;
            let items = children.iter().map(|child| {
                let margin = values::resolve_margins(&child.style, child.font_size, 0.0);
                max_content_width(child, fonts)
                    + margin.left.unwrap_or(0.0)
                    + margin.right.unwrap_or(0.0)
            }).sum::<f32>();
            items + gap * children.len().saturating_sub(1) as f32
        }
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children
            .iter()
            .map(|child| {
                let margin = values::resolve_margins(&child.style, child.font_size, 0.0);
                max_content_width(child, fonts)
                    + margin.left.unwrap_or(0.0)
                    + margin.right.unwrap_or(0.0)
            })
            .fold(0.0f32, f32::max),
        Flow::Empty => 0.0,
    }
}

pub(crate) fn min_content_inner_width(b: &LayoutBox, fonts: &FontSource) -> f32 {
    match &b.flow {
        Flow::Inlines(items) => inline_min_content_width(items, fonts),
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children
            .iter()
            .map(|child| {
                let margin = values::resolve_margins(&child.style, child.font_size, 0.0);
                min_content_width(child, fonts)
                    + margin.left.unwrap_or(0.0)
                    + margin.right.unwrap_or(0.0)
            })
            .fold(0.0f32, f32::max),
        Flow::Empty => 0.0,
    }
}

fn inline_min_content_width(items: &[crate::InlineNode], fonts: &FontSource) -> f32 {
    inline::tokenize(items)
        .into_iter()
        .map(|token| match token {
            inline::Token::Word { text, font_size, .. } => fonts.measure(&text, font_size),
            inline::Token::Replaced { size, .. } => size.width,
            inline::Token::Break => 0.0,
        })
        .fold(0.0f32, f32::max)
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

    #[test]
    fn cyclic_percent_min_width_contributes_only_its_length_to_intrinsic_size() {
        let fonts = FontSource::fallback();
        for (value, expected) in [
            ("calc(50% - 3px)", 1.0),
            ("calc(5em - 3px)", 47.0),
            ("calc(5em - 0%)", 50.0),
            ("calc(50%)", 1.0),
            ("calc(50px)", 50.0),
            ("calc(25% + 25%)", 1.0),
        ] {
            let style: amazing_css::ComputedStyle = [
                ("width".to_owned(), "1px".to_owned()),
                ("min-width".to_owned(), value.to_owned()),
            ].into_iter().collect();
            let box_ = LayoutBox::new(None, crate::Display::Block, style, 10.0, 12.0);
            assert_eq!(max_content_width(&box_, &fonts), expected, "{value}");
            assert_eq!(min_content_width(&box_, &fonts), expected, "{value}");
        }
    }

    #[test]
    fn calc_size_minimum_transforms_intrinsic_basis() {
        let style: amazing_css::ComputedStyle = [
            ("width".to_owned(), "50px".to_owned()),
            ("min-width".to_owned(), "calc-size(auto, size + 20px)".to_owned()),
        ].into_iter().collect();
        let box_ = LayoutBox::new(None, crate::Display::Block, style, 16.0, 19.0);
        let fonts = FontSource::fallback();
        assert_eq!(min_content_width(&box_, &fonts), 70.0);
        assert_eq!(max_content_width(&box_, &fonts), 70.0);
    }

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
    fn sideways_lr_row_follows_the_logical_inline_direction() {
        let ltr = lay(
            "<body><div style=\"display:flex;writing-mode:sideways-lr;width:20px;height:60px\"><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div></div></body>",
            "",
            100.0,
        );
        let rtl = lay(
            "<body><div style=\"display:flex;writing-mode:sideways-lr;direction:rtl;width:20px;height:60px\"><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div></div></body>",
            "",
            100.0,
        );
        let column_reverse = lay(
            "<body><div style=\"display:flex;writing-mode:sideways-lr;flex-direction:column-reverse;width:80px\"><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div></div></body>",
            "",
            100.0,
        );
        let ltr = kids(&ltr);
        let rtl = kids(&rtl);
        let column_reverse = kids(&column_reverse);

        assert!(ltr[0].border_box.y > ltr[2].border_box.y);
        assert!(rtl[0].border_box.y < rtl[2].border_box.y);
        assert!(column_reverse[0].border_box.x > column_reverse[2].border_box.x);
        assert!(column_reverse[0].border_box.x < 80.0);
    }

    #[test]
    fn sideways_lr_auto_width_uses_intrinsic_block_extent_for_column_reverse() {
        let root = lay(
            "<body><div style='display:flex;writing-mode:sideways-lr;flex-direction:column-reverse'><div style='width:20px;height:20px'></div><div style='width:20px;height:20px'></div><div style='width:20px;height:20px'></div></div></body>",
            "body{margin:0}", 800.0,
        );
        let Flow::Blocks(containers) = &root.flow else { panic!("want container") };
        assert!(close(containers[0].content_box.w, 60.0));
        let children = kids(&root);
        assert!(close(children[0].border_box.x, 40.0));
        assert!(close(children[2].border_box.x, 0.0));
    }

    #[test]
    fn vertical_rl_column_reverse_runs_left_to_right() {
        let root = lay(
            "<body><div style=\"display:flex;writing-mode:vertical-rl;flex-direction:column-reverse;width:100px;height:30px\"><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div></div></body>",
            "",
            120.0,
        );
        let items = kids(&root);

        assert!(items[0].border_box.x < items[1].border_box.x);
    }

    #[test]
    fn sideways_lr_row_reverse_uses_the_bottom_as_flex_start() {
        let root = lay(
            "<body><div style=\"display:flex;writing-mode:sideways-lr;width:20px;height:100px\"><div style=\"width:20px;height:20px\"></div><div style=\"width:20px;height:20px\"></div></div></body>",
            "",
            120.0,
        );
        let items = kids(&root);

        assert!(items[0].border_box.y > items[1].border_box.y);
        assert!(items[0].border_box.y > 60.0);
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
    fn wrap_reverse_places_first_row_at_cross_end() {
        let root = lay(
            "<body><div style='display:flex;flex-wrap:wrap-reverse;align-items:flex-start;width:40px;height:60px'><div style='width:30px;height:10px'></div><div style='width:30px;height:10px'></div></div></body>",
            "",
            100.0,
        );
        let items = kids(&root);
        assert!(items[0].border_box.y > items[1].border_box.y);
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
    fn safe_alignment_uses_position_when_space_is_available() {
        assert_eq!(parse_pack(Some("safe center"), Pack::Start), Pack::Center);
        assert_eq!(parse_align(Some("safe flex-end"), Pack::Start), Pack::End);
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
    fn single_flex_line_fills_cross_size_even_with_align_content_center() {
        let root = lay(
            "<body><div style='display:flex;height:100px;align-content:center;align-items:start'><div style='width:50px;height:20px'></div></div></body>",
            "",
            800.0,
        );
        let item = &kids(&root)[0];
        assert!(close(item.border_box.y, 8.0), "y={}", item.border_box.y);
    }

    #[test]
    fn baseline_aligned_flex_items_share_their_text_baseline() {
        let root = lay(
            "<body><div style=\"display:flex;align-items:baseline\"><div style=\"font-size:12px\">small</div><div style=\"font-size:30px\">large</div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert_eq!(k.len(), 2);
        assert!(close(first_baseline(&k[0]), first_baseline(&k[1])));
        assert!(k[0].border_box.y > k[1].border_box.y);
    }

    #[test]
    fn align_self_baseline_uses_the_same_group_as_align_items() {
        let root = lay(
            "<body><div style=\"display:flex;align-items:baseline\"><div style=\"font-size:12px\">small</div><div style=\"font-size:30px;align-self:first baseline\">large</div></div></body>",
            "",
            800.0,
        );
        let k = kids(&root);
        assert!(close(first_baseline(&k[0]), first_baseline(&k[1])));
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
