//! CSS Grid layout (css-grid-1 subset): track sizing, item placement and
//! alignment for `display: grid` containers.
//!
//! Implemented:
//! - `grid-template-columns` / `grid-template-rows` track lists: lengths,
//!   percentages, `auto`, `min-content` / `max-content`, two-sided
//!   `minmax()`, `<number>fr`, and `repeat(<integer> | auto-fill, …)`;
//!   named-line occurrences are retained through repeated fragments
//! - `grid-auto-columns` / `grid-auto-rows` (a list is cycled through the
//!   implicit tracks), `gap` / `row-gap` / `column-gap`
//! - `grid-auto-flow`: `row` (default) | `column` | `dense` (dense
//!   backflow implemented via occupancy rescans)
//! - item placement: `grid-row` / `grid-column` (+ the four longhands)
//!   with line numbers incl. negatives-from-the-explicit-grid-end,
//!   `span N`; `span <line-name>` degrades to `span 1` (named lines are
//!   dropped by the parser); sparse auto-placement cursor honoring dense
//!   mode; implicit tracks created wherever items land outside the
//!   explicit grid
//! - track sizing: definite px/% bases; intrinsic (`auto`/`min/max-content`)
//!   tracks sized from item measurements; `fr` tracks use a restart loop when
//!   a base exceeds its provisional share; row heights are measured by laying
//!   each item out at its final column width
//! - alignment: `justify-content` / `align-content`
//!   (start/end/center/stretch/space-between/around/evenly),
//!   `justify-items` / `align-items` + per-item `-self` longhands
//!   (stretch/start/end/center); stretch fills the area (items re-laid-out
//!   at their final width, stretched heights materialized and re-flowed)
//!
//! Deviations / deferrals (documented):
//! - named areas / `grid-area` / `grid-template-areas` are not implemented
//! - `span <line-name>` → `span 1`; implicit tracks before the explicit grid
//!   do not yet participate in out-of-range negative named occurrences
//! - percentages in `grid-template-rows` resolve against the content WIDTH
//!   (the height is indefinite at parse time in this engine)
//! - `auto-fill` only repeats beyond 1 when the repeated track size has a
//!   definite repeat contribution; auto-fit empty-track collapse is deferred
//! - items spanning several intrinsic tracks contribute an EQUAL share of
//!   their measurement to each spanned intrinsic track
//! - `fr` rows without a definite container height behave like `auto`
//! - stretch is skipped when the relevant margins are auto; Grid item auto
//!   margins otherwise absorb area free space after track sizing
//! - dense placement rescans from the grid start but does not implement
//!   the spec's exact cursor-monotonicity bookkeeping

use amazing_css::ComputedStyle;
use amazing_css::values::{ComputedValue, Display as CssDisplay, SubgridLineNames, SubgridRepeatCount};
use std::collections::HashMap;

use crate::LayoutBox;
use crate::font::FontSource;
use crate::grid_algorithm::{maximize_tracks, resolve_flexible_tracks};
use crate::values::{self, MaybeLen};

const MAX_GRID_TRACKS: usize = 1000;
const MAX_OCCUPIED_CELLS: usize = MAX_GRID_TRACKS * MAX_GRID_TRACKS;
const MAX_GRID_PLACEMENT_WORK: usize = MAX_OCCUPIED_CELLS * 4;

/// A subgrid's view of the parent tracks covered by its grid area. Line
/// numbers are already remapped to the child's 1-based local grid.
#[derive(Debug, Clone)]
pub(crate) struct SubgridAxis {
    sizes: Vec<f32>,
    gap: f32,
    names: HashMap<String, Vec<i32>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SubgridContext {
    cols: Option<SubgridAxis>,
    rows: Option<SubgridAxis>,
}

fn typed_subgrid(style: &ComputedStyle, property: &str) -> bool {
    matches!(style.typed(property), Some(ComputedValue::Subgrid(_)))
}

fn local_subgrid_names(
    style: &ComputedStyle,
    property: &str,
    inherited: &HashMap<String, Vec<i32>>,
    tracks: usize,
) -> HashMap<String, Vec<i32>> {
    let mut names = inherited.clone();
    let Some(ComputedValue::Subgrid(subgrid)) = style.typed(property) else {
        return names;
    };
    let mut line = 1usize;
    for entry in &subgrid.line_names {
        match entry {
            SubgridLineNames::Names(group) => {
                for name in group {
                    if line <= tracks + 1 {
                        names.entry(name.clone()).or_default().push(line as i32);
                    }
                }
                line += 1;
            }
            SubgridLineNames::Repeat { count, names: groups } => {
                let times = match count {
                    SubgridRepeatCount::Integer(n) => *n as usize,
                    SubgridRepeatCount::AutoFill => tracks.saturating_add(1).saturating_sub(line),
                }.min(tracks + 1);
                for _ in 0..times {
                    for group in groups {
                        for name in group {
                            if line <= tracks + 1 {
                                names.entry(name.clone()).or_default().push(line as i32);
                            }
                        }
                        line += 1;
                    }
                }
            }
        }
    }
    for occurrences in names.values_mut() {
        occurrences.sort_unstable();
        occurrences.dedup();
    }
    names
}

// ---------------------------------------------------------------------------
// Track model
// ---------------------------------------------------------------------------

/// One track sizing function. `Auto` and [`TrackSize::Content`] stay
/// distinct because `align-content: stretch` only grows `auto` tracks.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TrackSize {
    /// Definite length (lengths AND percentages resolved against
    /// `content_w` at parse time).
    Px(f32),
    /// The `auto` keyword.
    Auto,
    /// `min-content` / `max-content` (conflated, documented).
    Content,
    /// `<number>fr` flexible track (weight).
    Fr(f32),
    /// Independent minimum and maximum track sizing functions.
    MinMax {
        min: TrackBreadth,
        max: TrackBreadth,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TrackBreadth {
    Px(f32),
    Auto,
    Content,
    Fr(f32),
}

impl TrackSize {
    fn is_intrinsic(self) -> bool {
        matches!(self, TrackSize::Auto | TrackSize::Content)
            || matches!(
                self,
                TrackSize::MinMax {
                    min: TrackBreadth::Auto | TrackBreadth::Content,
                    ..
                } | TrackSize::MinMax {
                    max: TrackBreadth::Auto | TrackBreadth::Content,
                    ..
                }
            )
    }
    fn is_fr(self) -> bool {
        self.fr_factor() > 0.0
    }
    fn fr_factor(self) -> f32 {
        match self {
            TrackSize::Fr(factor)
            | TrackSize::MinMax {
                max: TrackBreadth::Fr(factor),
                ..
            } => factor,
            _ => 0.0,
        }
    }
    fn base_floor(self) -> f32 {
        match self {
            TrackSize::Px(value)
            | TrackSize::MinMax {
                min: TrackBreadth::Px(value),
                ..
            } => value,
            _ => 0.0,
        }
    }
    fn growth_limit(self) -> Option<f32> {
        match self {
            TrackSize::Px(value)
            | TrackSize::MinMax {
                max: TrackBreadth::Px(value),
                ..
            } => Some(value),
            _ => None,
        }
    }
    fn repeat_size(self) -> Option<f32> {
        match self {
            TrackSize::Px(value) => Some(value),
            TrackSize::MinMax {
                min: TrackBreadth::Px(min),
                max: TrackBreadth::Px(max),
            } => Some(max.max(min)),
            TrackSize::MinMax {
                min: TrackBreadth::Px(min),
                ..
            } => Some(min),
            _ => None,
        }
    }
}

/// Parse one track-size component value.
fn parse_track_size(s: &str, font_px: f32, content_w: f32, zoom: f32) -> TrackSize {
    let s = s.trim();
    // Lowercase first so `FR`/`Auto` spellings parse identically.
    let lower = s.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("minmax(")
        .and_then(|value| value.strip_suffix(')'))
        && let Some((minimum, maximum)) = inner.split_once(',')
    {
        let min = parse_track_breadth(minimum, font_px, content_w, zoom);
        let mut max = parse_track_breadth(maximum, font_px, content_w, zoom);
        if matches!(min, TrackBreadth::Fr(_)) {
            return TrackSize::Auto;
        }
        if let (TrackBreadth::Px(minimum), TrackBreadth::Px(maximum)) = (min, max)
            && maximum < minimum
        {
            max = TrackBreadth::Px(minimum);
        }
        return TrackSize::MinMax { min, max };
    }
    if let Some(stripped) = lower.strip_suffix("fr") {
        if let Ok(n) = stripped.trim().parse::<f32>() {
            return TrackSize::Fr(n.max(0.0));
        }
        return TrackSize::Auto;
    }
    match lower.as_str() {
        "auto" => return TrackSize::Auto,
        "min-content" | "max-content" => return TrackSize::Content,
        _ => {}
    }
    zoomed_track_length(s, font_px, content_w, zoom)
        .map_or(TrackSize::Auto, |value| TrackSize::Px(value.max(0.0)))
}

fn parse_track_breadth(s: &str, font_px: f32, content_w: f32, zoom: f32) -> TrackBreadth {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(factor) = lower.strip_suffix("fr")
        && let Ok(factor) = factor.trim().parse::<f32>()
    {
        return TrackBreadth::Fr(factor.max(0.0));
    }
    match lower.as_str() {
        "auto" => TrackBreadth::Auto,
        "min-content" | "max-content" => TrackBreadth::Content,
        _ => zoomed_track_length(s, font_px, content_w, zoom)
            .map_or(TrackBreadth::Auto, |value| TrackBreadth::Px(value.max(0.0))),
    }
}

fn zoomed_track_length(s: &str, font_px: f32, basis: f32, zoom: f32) -> Option<f32> {
    let mut length = values::parse_length_percentage(s, font_px / zoom, font_px * 0.6 / zoom)?;
    length.px *= zoom;
    length.resolve(Some(basis))
}

/// Splits a declaration value into top-level whitespace-separated tokens,
/// keeping parenthesized groups (`repeat(2, 100px)`) as single tokens.
fn top_tokens(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    for ch in value.chars() {
        match ch {
            '(' => {
                paren_depth += 1;
                cur.push(ch);
            }
            ')' => {
                paren_depth -= 1;
                cur.push(ch);
            }
            '[' => {
                bracket_depth += 1;
                cur.push(ch);
            },
            ']' => {
                bracket_depth -= 1;
                cur.push(ch);
            },
            c if c.is_whitespace() && paren_depth <= 0 && bracket_depth <= 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn repeat_parts(token: &str) -> Option<(&str, &str)> {
    let open = token.find('(')?;
    if !token[..open].eq_ignore_ascii_case("repeat") || !token.ends_with(')') {
        return None;
    }
    let inner = &token[open + 1..token.len() - 1];
    let comma = inner.find(',')?;
    Some((&inner[..comma], &inner[comma + 1..]))
}

/// Nested repeat() is invalid Grid grammar. Reject it before either track or
/// named-line expansion recurses, so hostile nesting cannot consume the Rust
/// call stack even when a ComputedStyle is synthesized outside the CSS parser.
fn contains_nested_repeat(value: &str) -> bool {
    top_tokens(value).iter().any(|token| {
        repeat_parts(token).is_some_and(|(_, list)| {
            top_tokens(list)
                .iter()
                .any(|inner| repeat_parts(inner).is_some())
        })
    })
}

/// Context needed to expand `repeat(auto-fill, …)`.
struct RepeatCtx {
    font_px: f32,
    content_w: f32,
    zoom: f32,
    repeat_available: Option<f32>,
    gap: f32,
    /// Definite px already consumed by tracks emitted before this repeat.
    fixed_before: f32,
}

/// Parses a full track list: components, `[line names]` (dropped), and
/// `repeat()` groups. Total track count is capped defensively.
fn parse_track_list(value: &str, ctx: &RepeatCtx, allow_auto_fill: bool) -> Vec<TrackSize> {
    if value.trim().eq_ignore_ascii_case("none") || contains_nested_repeat(value) {
        return Vec::new();
    }
    let tokens = top_tokens(value);
    let mut outside_fixed_sum = 0.0;
    let mut outside_fixed_count = 0usize;
    for token in &tokens {
        let lower = token.to_ascii_lowercase();
        if (token.starts_with('[') && token.ends_with(']')) || lower.starts_with("repeat(") {
            continue;
        }
        let fixed = [parse_track_size(token, ctx.font_px, ctx.content_w, ctx.zoom)];
        if let Some(sizes) = fixed
            .iter()
            .map(|size| size.repeat_size())
            .collect::<Option<Vec<_>>>()
        {
            outside_fixed_sum += sizes.iter().sum::<f32>();
            outside_fixed_count += sizes.len();
        }
    }
    let mut out = Vec::new();
    for tok in tokens {
        if out.len() >= MAX_GRID_TRACKS {
            break;
        }
        // Line names `[a b]` may split into several tokens; drop them all.
        if tok.starts_with('[') && tok.ends_with(']') {
            continue;
        }
        if let Some((count_part, list_part)) = repeat_parts(&tok) {
            // repeat(<count>, <track-list>) — count is an integer or
            // auto-fill. Nested repeat() was rejected before this loop.
            let n = match count_part.trim().parse::<usize>() {
                Ok(n) => n.min(MAX_GRID_TRACKS),
                Err(_) => {
                    // auto-fill: expandable ONLY when every repeated track
                    // is definite; otherwise one repetition (documented).
                    if !allow_auto_fill || !count_part.trim().eq_ignore_ascii_case("auto-fill") {
                        1
                    } else {
                        let sizes = parse_track_list(
                            list_part,
                            &RepeatCtx {
                                font_px: ctx.font_px,
                                content_w: ctx.content_w,
                                zoom: ctx.zoom,
                                repeat_available: ctx.repeat_available,
                                gap: ctx.gap,
                                fixed_before: ctx.fixed_before,
                            },
                            false,
                        );
                        let repeat_sizes = sizes
                            .iter()
                            .map(|size| size.repeat_size())
                            .collect::<Option<Vec<_>>>();
                        let unit: f32 = repeat_sizes
                            .as_ref()
                            .map(|sizes| sizes.iter().sum())
                            .unwrap_or(0.0)
                            // Include the gap after this repetition. The
                            // numerator adds one trailing gap back, yielding
                            // `tracks + (track_count - 1) gaps` overall.
                            + ctx.gap * sizes.len() as f32;
                        ctx.repeat_available.map_or(1, |available| {
                            let avail = (available + ctx.gap
                                - ctx.fixed_before
                                - outside_fixed_sum
                                - ctx.gap * outside_fixed_count as f32)
                                .max(0.0);
                            if repeat_sizes.is_some() && unit > 0.0 {
                                (((avail + ctx.gap) / unit).floor() as usize)
                                    .clamp(1, MAX_GRID_TRACKS)
                            } else {
                                1
                            }
                        })
                    }
                }
            };
            for _ in 0..n {
                if out.len() >= MAX_GRID_TRACKS {
                    break;
                }
                let parsed = parse_track_list(
                    list_part,
                    &RepeatCtx {
                        font_px: ctx.font_px,
                        content_w: ctx.content_w,
                        zoom: ctx.zoom,
                        repeat_available: ctx.repeat_available,
                        gap: ctx.gap,
                        fixed_before: ctx.fixed_before,
                    },
                    false,
                );
                out.extend(parsed.into_iter().take(MAX_GRID_TRACKS - out.len()));
            }
            continue;
        }
        out.push(parse_track_size(&tok, ctx.font_px, ctx.content_w, ctx.zoom));
    }
    out
}

fn collect_line_names(
    value: &str,
    ctx: &RepeatCtx,
    allow_auto_fill: bool,
) -> HashMap<String, Vec<i32>> {
    if contains_nested_repeat(value) {
        return HashMap::new();
    }
    let tokens = top_tokens(value);
    let total_tracks = parse_track_list(value, ctx, allow_auto_fill).len();
    let simple_tracks = tokens
        .iter()
        .filter(|token| !token.contains('[') && !token.to_ascii_lowercase().starts_with("repeat("))
        .count();
    let mut names = HashMap::<String, Vec<i32>>::new();
    let mut line = 1i32;
    for token in tokens {
        if token.starts_with('[') && token.ends_with(']') {
            // Adjacent blocks (`[a][b]`) arrive as one token; `][` separates
            // their name lists just like whitespace.
            for name in token[1..token.len() - 1]
                .split("][")
                .flat_map(|part| part.split_ascii_whitespace())
            {
                names.entry(name.to_owned()).or_default().push(line);
            }
            continue;
        }
        if let Some((_, list)) = repeat_parts(&token) {
            let inner_tracks = parse_track_list(list, ctx, false);
            if inner_tracks.is_empty() {
                continue;
            }
            let repetitions = total_tracks
                .saturating_sub(simple_tracks)
                .checked_div(inner_tracks.len())
                .unwrap_or(0)
                .max(1);
            let inner_names = collect_line_names(list, ctx, false);
            for repetition in 0..repetitions {
                let offset = line - 1 + (repetition * inner_tracks.len()) as i32;
                for (name, occurrences) in &inner_names {
                    for occurrence in occurrences {
                        names
                            .entry(name.clone())
                            .or_default()
                            .push(offset + occurrence);
                    }
                }
            }
            line += (repetitions * inner_tracks.len()) as i32;
            continue;
        }
        line += 1;
    }
    names
}

// ---------------------------------------------------------------------------
// Container spec
// ---------------------------------------------------------------------------

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

/// Per-item alignment keywords (justify-items / align-items).
#[derive(Debug, Clone, Copy, PartialEq)]
enum ItemAlign {
    Stretch,
    Start,
    End,
    Center,
}

fn parse_pack(v: Option<&str>) -> Pack {
    match v.map(crate::flex::strip_overflow_position) {
        Some(v) if v.eq_ignore_ascii_case("end") || v.eq_ignore_ascii_case("flex-end") => Pack::End,
        Some(v) if v.eq_ignore_ascii_case("center") => Pack::Center,
        Some(v) if v.eq_ignore_ascii_case("space-between") => Pack::SpaceBetween,
        Some(v) if v.eq_ignore_ascii_case("space-around") => Pack::SpaceAround,
        Some(v) if v.eq_ignore_ascii_case("space-evenly") => Pack::SpaceEvenly,
        Some(v) if v.eq_ignore_ascii_case("normal") || v.eq_ignore_ascii_case("stretch") => {
            Pack::Stretch
        },
        _ => Pack::Start,
    }
}

fn parse_item_align(v: Option<&str>) -> ItemAlign {
    match v.map(crate::flex::strip_overflow_position) {
        Some(v) if v.eq_ignore_ascii_case("start") || v.eq_ignore_ascii_case("flex-start") => {
            ItemAlign::Start
        }
        Some(v) if v.eq_ignore_ascii_case("end") || v.eq_ignore_ascii_case("flex-end") => {
            ItemAlign::End
        }
        Some(v) if v.eq_ignore_ascii_case("center") => ItemAlign::Center,
        _ => ItemAlign::Stretch,
    }
}

/// Leading space before the first track and EXTRA space between tracks for
/// a content-distribution mode over `free` space and `n` tracks.
fn pack_pair(mode: Pack, free: f32, n: usize) -> (f32, f32) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let nf = n as f32;
    let free = free.max(0.0);
    match mode {
        Pack::End => (free, 0.0),
        Pack::Center => (free / 2.0, 0.0),
        Pack::SpaceBetween if nf > 1.0 => (0.0, free / (nf - 1.0)),
        Pack::SpaceAround => (free / nf / 2.0, free / nf),
        Pack::SpaceEvenly => (free / (nf + 1.0), free / (nf + 1.0)),
        _ => (0.0, 0.0), // Start, Stretch (stretch handled during sizing)
    }
}

/// `gap: r c` / `row-gap` / `column-gap`. Percentages resolve against the
/// content width for both axes (flex.rs precedent, documented).
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

/// Everything layout_grid needs from the CONTAINER's computed style.
struct GridSpec {
    cols: Vec<TrackSize>,
    rows: Vec<TrackSize>,
    col_line_names: HashMap<String, Vec<i32>>,
    row_line_names: HashMap<String, Vec<i32>>,
    auto_cols: Vec<TrackSize>,
    auto_rows: Vec<TrackSize>,
    row_gap: f32,
    col_gap: f32,
    flow_column: bool,
    flow_dense: bool,
    rtl: bool,
    justify_content: Pack,
    align_content: Pack,
    justify_items: ItemAlign,
    align_items: ItemAlign,
}

impl GridSpec {
    fn of(
        style: &ComputedStyle,
        context: Option<&SubgridContext>,
        font_px: f32,
        content_w: f32,
        definite_height: Option<f32>,
    ) -> Self {
        let (mut row_gap, mut col_gap) = parse_gaps(style, font_px, content_w);
        let normal_gap = |longhand: &str| {
            style.get(longhand).is_none_or(|v| v.eq_ignore_ascii_case("normal"))
                && style.get("gap").is_none_or(|v| v.eq_ignore_ascii_case("normal"))
        };
        if let Some(axis) = context.and_then(|c| c.cols.as_ref())
            && normal_gap("column-gap") {
            col_gap = axis.gap;
        }
        if let Some(axis) = context.and_then(|c| c.rows.as_ref())
            && normal_gap("row-gap") {
            row_gap = axis.gap;
        }
        let ctx_cols = RepeatCtx {
            font_px,
            content_w,
            zoom: style.effective_zoom() as f32,
            repeat_available: Some(content_w),
            gap: col_gap,
            fixed_before: 0.0,
        };
        let ctx_rows = RepeatCtx {
            font_px,
            content_w: definite_height.unwrap_or(content_w),
            zoom: style.effective_zoom() as f32,
            repeat_available: definite_height,
            gap: row_gap,
            fixed_before: 0.0,
        };
        let empty_auto = vec![TrackSize::Auto];
        let flow = style
            .get("grid-auto-flow")
            .unwrap_or("row")
            .to_ascii_lowercase();
        let words: Vec<&str> = flow.split_ascii_whitespace().collect();
        let col_value = style.get("grid-template-columns").unwrap_or("none");
        let row_value = style.get("grid-template-rows").unwrap_or("none");
        GridSpec {
            cols: context.and_then(|c| c.cols.as_ref()).map_or_else(
                || parse_track_list(col_value, &ctx_cols, true),
                |axis| axis.sizes.iter().copied().map(TrackSize::Px).collect(),
            ),
            rows: context.and_then(|c| c.rows.as_ref()).map_or_else(
                || parse_track_list(row_value, &ctx_rows, true),
                |axis| axis.sizes.iter().copied().map(TrackSize::Px).collect(),
            ),
            col_line_names: context.and_then(|c| c.cols.as_ref()).map_or_else(
                || collect_line_names(col_value, &ctx_cols, true),
                |axis| local_subgrid_names(style, "grid-template-columns", &axis.names, axis.sizes.len()),
            ),
            row_line_names: context.and_then(|c| c.rows.as_ref()).map_or_else(
                || collect_line_names(row_value, &ctx_rows, true),
                |axis| local_subgrid_names(style, "grid-template-rows", &axis.names, axis.sizes.len()),
            ),
            auto_cols: style
                .get("grid-auto-columns")
                .map(|v| parse_track_list(v, &ctx_cols, false))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| empty_auto.clone()),
            auto_rows: style
                .get("grid-auto-rows")
                .map(|v| parse_track_list(v, &ctx_rows, false))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| empty_auto.clone()),
            row_gap,
            col_gap,
            flow_column: words.iter().any(|w| *w == "column"),
            flow_dense: words.iter().any(|w| *w == "dense"),
            rtl: style
                .get("direction")
                .is_some_and(|value| value.eq_ignore_ascii_case("rtl")),
            justify_content: parse_pack(style.get("justify-content")),
            align_content: parse_pack(style.get("align-content")),
            justify_items: parse_item_align(style.get("justify-items")),
            align_items: parse_item_align(style.get("align-items")),
        }
    }
}

// ---------------------------------------------------------------------------
// Item placement specs
// ---------------------------------------------------------------------------

/// One side of a placement (`grid-row-start` etc.) after parsing:
/// an optional line number plus a span (≥ 1).
#[derive(Debug, Clone, Copy)]
struct LineSpec {
    line: Option<i32>,
    span: u32,
}

impl LineSpec {
    const AUTO: LineSpec = LineSpec {
        line: None,
        span: 1,
    };
}

/// Parses one slot value: `auto`, `<integer>`, `span N`, or a named-line
/// occurrence such as `middle`, `middle 2`, or `-1 middle`.
fn parse_line_slot(
    s: &str,
    names: &HashMap<String, Vec<i32>>,
    explicit_count: usize,
) -> LineSpec {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        return LineSpec::AUTO;
    }
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("span") {
        let rest = rest.trim();
        return match rest.parse::<u32>() {
            Ok(n) if n >= 1 => LineSpec {
                line: None,
                span: n.min(1000),
            },
            _ => LineSpec {
                line: None,
                span: 1,
            }, // `span <name>` → span 1
        };
    }
    match s.parse::<i32>() {
        Ok(0) => LineSpec::AUTO,
        Ok(n) => LineSpec {
            line: Some(n),
            span: 1,
        },
        Err(_) => resolve_named_line(s, names, explicit_count).map_or(LineSpec::AUTO, |line| {
            LineSpec {
                line: Some(line),
                span: 1,
            }
        }),
    }
}

fn resolve_named_line(
    value: &str,
    names: &HashMap<String, Vec<i32>>,
    explicit_count: usize,
) -> Option<i32> {
    let parts = value.split_ascii_whitespace().collect::<Vec<_>>();
    let (name, occurrence) = match parts.as_slice() {
        [name] => (*name, 1),
        [first, second] => {
            if let Ok(occurrence) = first.parse::<i32>() {
                (*second, occurrence)
            } else if let Ok(occurrence) = second.parse::<i32>() {
                (*first, occurrence)
            } else {
                return None;
            }
        },
        _ => return None,
    };
    if occurrence == 0 {
        return None;
    }
    let occurrences = names.get(name).map(Vec::as_slice).unwrap_or(&[]);
    if occurrence > 0 {
        let index = usize::try_from(occurrence).ok()?.checked_sub(1)?;
        occurrences.get(index).copied().or_else(|| {
            i32::try_from(explicit_count)
                .ok()?
                .checked_add(1)?
                .checked_add(occurrence)?
                .checked_sub(i32::try_from(occurrences.len()).ok()?)
                .filter(|line| *line <= MAX_GRID_TRACKS as i32 + 1)
        })
    } else {
        let from_end = occurrence.unsigned_abs() as usize;
        occurrences
            .len()
            .checked_sub(from_end)
            .and_then(|index| occurrences.get(index).copied())
    }
}

/// Resolved placement of one item: 1-based start lines (None = needs auto
/// placement) and spans ≥ 1.
#[derive(Debug, Clone, Copy, Default)]
struct Placement {
    row: Option<u32>,
    row_span: u32,
    col: Option<u32>,
    col_span: u32,
}

/// Combines a start/end slot pair into a definite start + span. Negative
/// line numbers resolve FIRST (`-k` counts back from the END of the
/// explicit grid: line `tracks + 2 - k`, so `-1` is the last line), then
/// out-of-order ends swap per §8.3.2.1.
fn combine_slots(start: LineSpec, end: LineSpec, explicit_count: usize) -> (Option<u32>, u32) {
    let resolve = |l: i32| -> Option<u32> {
        if l < 0 {
            let from_end = explicit_count as i32 + 2 + l;
            if from_end < 1 {
                None
            } else {
                Some(from_end as u32)
            }
        } else if l == 0 {
            None
        } else {
            u32::try_from(l)
                .ok()
                .filter(|line| *line <= MAX_GRID_TRACKS as u32 + 1)
        }
    };
    match (start.line.and_then(resolve), end.line.and_then(resolve)) {
        (Some(a), Some(b)) => {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            (Some(lo), (hi - lo).max(1))
        }
        (Some(a), None) => (Some(a), end.span.max(1)),
        (None, Some(b)) => {
            let span = start.span.max(1);
            (Some(if b > span { b - span } else { 1 }), span)
        }
        (None, None) => (None, start.span.max(end.span).max(1)),
    }
}

/// Reads an item's `grid-row` / `grid-column` (shorthand first, then the
/// four longhands override their slots).
fn parse_placement(
    style: &ComputedStyle,
    explicit_cols: usize,
    explicit_rows: usize,
    col_names: &HashMap<String, Vec<i32>>,
    row_names: &HashMap<String, Vec<i32>>,
) -> Placement {
    let side = |shorthand: &str,
                s_prop: &str,
                e_prop: &str,
                expl: usize,
                names: &HashMap<String, Vec<i32>>| {
        let mut start = LineSpec::AUTO;
        let mut end = LineSpec::AUTO;
        if let Some(v) = style.get(shorthand) {
            let parts: Vec<&str> = v.split('/').collect();
            start = parse_line_slot(parts.first().copied().unwrap_or("auto"), names, expl);
            end = match parts.get(1) {
                Some(e) => parse_line_slot(e, names, expl),
                None => LineSpec::AUTO,
            };
        }
        if let Some(v) = style.get(s_prop) {
            start = parse_line_slot(v, names, expl);
        }
        if let Some(v) = style.get(e_prop) {
            end = parse_line_slot(v, names, expl);
        }
        combine_slots(start, end, expl)
    };
    let (col, col_span) = side(
        "grid-column",
        "grid-column-start",
        "grid-column-end",
        explicit_cols,
        col_names,
    );
    let (row, row_span) = side(
        "grid-row",
        "grid-row-start",
        "grid-row-end",
        explicit_rows,
        row_names,
    );
    Placement {
        row,
        row_span,
        col,
        col_span,
    }
}

// ---------------------------------------------------------------------------
// Auto-placement (css-grid-1 §8.5, simplified sparse/dense model)
// ---------------------------------------------------------------------------

/// Occupied-cell set over bounded 1-based (row, col) pairs.
#[derive(Default)]
struct Occupancy {
    cells: HashMap<(u32, u32), ()>,
}

impl Occupancy {
    fn region_is_bounded(r: u32, c: u32, rs: u32, cs: u32) -> bool {
        r >= 1
            && c >= 1
            && rs >= 1
            && cs >= 1
            && r.checked_add(rs - 1)
                .is_some_and(|end| end <= MAX_GRID_TRACKS as u32)
            && c.checked_add(cs - 1)
                .is_some_and(|end| end <= MAX_GRID_TRACKS as u32)
    }

    fn charge(work: &mut usize, rs: u32, cs: u32) -> bool {
        let Some(area) = (rs as usize).checked_mul(cs as usize) else {
            return false;
        };
        let Some(next) = work.checked_add(area) else {
            return false;
        };
        if next > MAX_GRID_PLACEMENT_WORK {
            return false;
        }
        *work = next;
        true
    }

    fn fits(&self, r: u32, c: u32, rs: u32, cs: u32, work: &mut usize) -> Option<bool> {
        if r < 1
            || c < 1
            || rs < 1
            || cs < 1
            || r > MAX_GRID_TRACKS as u32
            || c > MAX_GRID_TRACKS as u32
            || rs > MAX_GRID_TRACKS as u32
            || cs > MAX_GRID_TRACKS as u32
        {
            return None;
        }
        if !Self::region_is_bounded(r, c, rs, cs) {
            return Some(false);
        }
        if !Self::charge(work, rs, cs) {
            return None;
        }
        for i in 0..rs {
            for j in 0..cs {
                if self.cells.contains_key(&(r + i, c + j)) {
                    return Some(false);
                }
            }
        }
        Some(true)
    }

    fn put(&mut self, r: u32, c: u32, rs: u32, cs: u32, work: &mut usize) -> bool {
        let Some(area) = (rs as usize).checked_mul(cs as usize) else {
            return false;
        };
        if !Self::region_is_bounded(r, c, rs, cs)
            || self.cells.len().saturating_add(area) > MAX_OCCUPIED_CELLS
            || !Self::charge(work, rs, cs)
        {
            return false;
        }
        for i in 0..rs {
            for j in 0..cs {
                self.cells.insert((r + i, c + j), ());
            }
        }
        true
    }
}

/// Row-flow placement in (major=row, minor=col) coordinates. Returns
/// definite `(row, row_span, col, col_span)` per item, document order.
///
/// Sparse packing keeps a forward-only cursor; dense packing rescans from
/// the grid start for every auto-placed item (backflow). Definite-major /
/// auto-minor items scan the minor axis unboundedly (creating implicit
/// tracks); both-auto items wrap at `expl_minor` (or the widest auto span).
fn place_row_flow(
    specs: &[Placement],
    dense: bool,
    expl_minor: usize,
) -> Option<Vec<(u32, u32, u32, u32)>> {
    if specs.len() > MAX_OCCUPIED_CELLS || expl_minor > MAX_GRID_TRACKS {
        return None;
    }
    // Wrap width for both-auto items: explicit minors, at least the widest
    // auto-placed span (so wide items always get columns to fit into).
    let mut limit = expl_minor.max(1);
    for p in specs {
        if let Some(col) = p.col {
            limit = limit.max(col.saturating_add(p.col_span).saturating_sub(1) as usize);
        } else if p.row.is_none() {
            limit = limit.max(p.col_span as usize);
        }
    }
    if limit > MAX_GRID_TRACKS {
        return None;
    }

    let mut occ = Occupancy::default();
    let mut out = vec![None; specs.len()];
    let (mut cur_r, mut cur_c) = (1u32, 1u32);
    let mut work = 0usize;

    // Grid 1 §8.5: definite placements occupy cells before auto placement,
    // regardless of their order among the items. Overlapping definite areas
    // are valid; Occupancy records their union.
    for (index, p) in specs.iter().enumerate() {
        if let (Some(r), Some(c)) = (p.row, p.col) {
            let rs = p.row_span.max(1);
            let cs = p.col_span.max(1);
            if !occ.put(r, c, rs, cs, &mut work) {
                return None;
            }
            out[index] = Some((r, rs, c, cs));
        }
    }

    // Items locked to a row precede the fully automatic pass. Sparse packing
    // advances only within the same locked row; it does not move the later
    // auto-placement cursor.
    let mut row_ends = HashMap::<u32, u32>::new();
    for (index, p) in specs.iter().enumerate() {
        let (Some(r), None) = (p.row, p.col) else { continue };
        let rs = p.row_span.max(1);
        let cs = p.col_span.max(1);
        let mut c = if dense { 1 } else { row_ends.get(&r).copied().unwrap_or(1) };
        loop {
            match occ.fits(r, c, rs, cs, &mut work)? {
                true => break,
                false => c = c.checked_add(1)?,
            }
        }
        if !occ.put(r, c, rs, cs, &mut work) {
            return None;
        }
        out[index] = Some((r, rs, c, cs));
        if !dense {
            row_ends.insert(r, c.checked_add(cs)?);
        }
        limit = limit.max(c.saturating_add(cs).saturating_sub(1) as usize);
    }

    // The final pass follows order-modified document order. Definite-column
    // items move the cursor in sparse mode; fully auto items search from it.
    for (index, p) in specs.iter().enumerate() {
        if out[index].is_some() {
            continue;
        }
        let rs = p.row_span.max(1);
        let cs = p.col_span.max(1);
        match (p.row, p.col) {
            (Some(_), _) => unreachable!(),
            // Definite column, auto row: slide down within the locked col.
            (None, Some(c)) => {
                if !dense && c < cur_c {
                    cur_r = cur_r.checked_add(1)?;
                }
                let mut r = if dense { 1 } else { cur_r };
                loop {
                    match occ.fits(r, c, rs, cs, &mut work)? {
                        true => break,
                        false => r = r.checked_add(1)?,
                    }
                }
                if !occ.put(r, c, rs, cs, &mut work) {
                    return None;
                }
                out[index] = Some((r, rs, c, cs));
                if !dense {
                    cur_r = r; // cursor resumes at the item's row start
                    cur_c = c.checked_add(cs)?;
                }
            }
            // Fully automatic.
            (None, None) => {
                let (r, c) = if dense {
                    // Reading-order rescan from the grid start; the wrap
                    // limit always accommodates the widest auto span, so a
                    // fitting cell exists — the cap is purely defensive.
                    let mut found = None;
                    'scan: for r in 1..=MAX_GRID_TRACKS as u32 {
                        for c in 1..=limit as u32 {
                            if occ.fits(r, c, rs, cs, &mut work)? {
                                found = Some((r, c));
                                break 'scan;
                            }
                        }
                    }
                    found?
                } else {
                    let (mut r, mut c) = (cur_r, cur_c);
                    loop {
                        if c > limit as u32 {
                            c = 1;
                            r = r.checked_add(1)?;
                            continue;
                        }
                        if occ.fits(r, c, rs, cs, &mut work)? {
                            break;
                        }
                        c = c.checked_add(1)?;
                    }
                    (r, c)
                };
                if !occ.put(r, c, rs, cs, &mut work) {
                    return None;
                }
                out[index] = Some((r, rs, c, cs));
                if !dense {
                    cur_r = r;
                    cur_c = c.checked_add(cs)?;
                }
            }
        }
    }
    out.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Track sizing helpers
// ---------------------------------------------------------------------------

/// Builds the full track vector: explicit entries, then implicit ones
/// cycling through the `grid-auto-*` list.
fn track_vector(explicit: &[TrackSize], autos: &[TrackSize], total: usize) -> Vec<TrackSize> {
    (0..total)
        .map(|i| {
            explicit
                .get(i)
                .copied()
                .unwrap_or_else(|| autos[(i - explicit.len().min(i)) % autos.len()])
        })
        .collect()
}

/// One item's measurement contribution along one axis.
struct Contrib {
    start: usize,
    span: usize,
    outer: f32,
}

/// Sizes one axis: definite bases first, intrinsic contributions
/// distributed equally among spanned intrinsic tracks (documented
/// simplification), then `fr` tracks split what is left of `available`.
/// `available` of `None` means indefinite (fr acts like auto, which then
/// relies on the contributions themselves).
fn size_axis(
    kinds: &[TrackSize],
    gap: f32,
    available: Option<f32>,
    contribs: &[Contrib],
) -> Vec<f32> {
    let n = kinds.len();
    // With an indefinite axis size, `fr` tracks have nothing to flex into
    // and behave like `auto` (documented) — they take contributions too.
    let fr_is_intrinsic = available.is_none();
    let mut base: Vec<f32> = kinds
        .iter()
        .map(|kind| kind.base_floor().max(0.0))
        .collect();

    // Intrinsic contributions.
    for c in contribs {
        let end = (c.start + c.span).min(n);
        let intr: Vec<usize> = (c.start..end)
            .filter(|&t| {
                kinds[t].is_intrinsic() || (fr_is_intrinsic && kinds[t].is_fr())
            })
            .collect();
        if !intr.is_empty() {
            let fixed = (c.start..end).filter(|track| intr.binary_search(track).is_err())
                .map(|track| base[track]).sum::<f32>();
            let internal_gaps = gap * (end - c.start).saturating_sub(1) as f32;
            let share = (c.outer - fixed - internal_gaps).max(0.0) / intr.len() as f32;
            for track in intr {
                base[track] = base[track].max(share)
                    .min(kinds[track].growth_limit().unwrap_or(f32::INFINITY));
            }
        }
    }

    if let Some(available) = available {
        let limits = kinds
            .iter()
            .map(|kind| kind.growth_limit())
            .collect::<Vec<_>>();
        let gaps = gap * n.saturating_sub(1) as f32;
        base = maximize_tracks(available, gaps, &base, &limits);
    }

    // Flexible tracks use the spec restart loop: a track whose base exceeds
    // its provisional flex share becomes inflexible before recomputation.
    if let (Some(avail), true) = (available, kinds.iter().any(|k| k.is_fr())) {
        let gaps = gap * n.saturating_sub(1) as f32;
        let factors = kinds
            .iter()
            .map(|kind| kind.fr_factor())
            .collect::<Vec<_>>();
        base = resolve_flexible_tracks(avail, gaps, &base, &factors);
    }
    base
}

fn stretch_auto_tracks(sizes: &mut [f32], kinds: &[TrackSize], available: f32, gap: f32) {
    let autos = kinds.iter().enumerate().filter_map(|(index, kind)|
        (*kind == TrackSize::Auto).then_some(index)).collect::<Vec<_>>();
    if autos.is_empty() { return; }
    let spare = available - sizes.iter().sum::<f32>()
        - gap * sizes.len().saturating_sub(1) as f32;
    if spare > 0.0 {
        let each = spare / autos.len() as f32;
        for index in autos { sizes[index] += each; }
    }
}

// ---------------------------------------------------------------------------
// Item helpers (box-model glue shared conceptually with flex.rs)
// ---------------------------------------------------------------------------

/// True when the STYLE explicitly says `auto` for that margin side
/// (absent means initial 0 — see flex.rs).
fn margin_is_auto(style: &ComputedStyle, side: &str) -> bool {
    let longhand = format!("margin-{side}");
    if let Some(v) = style.get(&longhand) {
        return v.trim().eq_ignore_ascii_case("auto");
    }
    if let Some(v) = style.get("margin") {
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

/// Horizontal padding + border chrome of an item.
fn h_chrome(it: &LayoutBox) -> f32 {
    it.padding.left + it.padding.right + it.border.left + it.border.right
}
/// Vertical padding + border chrome.
fn v_chrome(it: &LayoutBox) -> f32 {
    it.padding.top + it.padding.bottom + it.border.top + it.border.bottom
}

/// `height` is auto when absent, `auto`, or a percentage (percent heights
/// stay auto in this engine — block.rs precedent).
fn height_is_auto(it: &LayoutBox) -> bool {
    match it.style.get("height") {
        None => true,
        Some(v) => {
            let v = v.trim();
            v.eq_ignore_ascii_case("auto") || v.ends_with('%')
        }
    }
}

/// Outer (margin-box) height of a LAID OUT item.
fn outer_h(it: &LayoutBox) -> f32 {
    // border_box.h already includes padding/border — only margins add on.
    it.border_box.h + it.margin.top.unwrap_or(0.0) + it.margin.bottom.unwrap_or(0.0)
}

fn inherited_names(
    parent: &HashMap<String, Vec<i32>>,
    start: usize,
    tracks: usize,
) -> HashMap<String, Vec<i32>> {
    let first = start as i32;
    parent.iter().filter_map(|(name, lines)| {
        let local = lines.iter().filter_map(|line| {
            (*line >= first && *line <= first + tracks as i32)
                .then_some(*line - first + 1)
        }).collect::<Vec<_>>();
        (!local.is_empty()).then_some((name.clone(), local))
    }).collect()
}

fn subgrid_axis_for_item(
    item: &LayoutBox,
    property: &str,
    parent_sizes: &[f32],
    parent_gap: f32,
    parent_names: &HashMap<String, Vec<i32>>,
    start_line: usize,
    span: usize,
) -> Option<SubgridAxis> {
    if !typed_subgrid(&item.style, property) {
        return None;
    }
    let start = start_line.saturating_sub(1);
    let end = (start + span).min(parent_sizes.len());
    if start >= end {
        return None;
    }
    let basis = parent_sizes[start..end].iter().sum::<f32>()
        + parent_gap * (end - start).saturating_sub(1) as f32;
    let (row_gap, col_gap) = parse_gaps(&item.style, item.font_size, basis);
    let (longhand, own_gap) = if property == "grid-template-columns" {
        ("column-gap", col_gap)
    } else {
        ("row-gap", row_gap)
    };
    let normal_gap = item.style.get(longhand).is_none_or(|v| v.eq_ignore_ascii_case("normal"))
        && item.style.get("gap").is_none_or(|v| v.eq_ignore_ascii_case("normal"));
    let gap = if normal_gap { parent_gap } else { own_gap };
    let margin = values::resolve_margins(&item.style, item.font_size, basis);
    let padding = values::resolve_padding(&item.style, item.font_size, basis);
    let border = values::resolve_borders(&item.style, item.font_size);
    let (start_chrome, end_chrome) = if property == "grid-template-columns" {
        (margin.left.unwrap_or(0.0) + padding.left + border.left,
            margin.right.unwrap_or(0.0) + padding.right + border.right)
    } else {
        (margin.top.unwrap_or(0.0) + padding.top + border.top,
            margin.bottom.unwrap_or(0.0) + padding.bottom + border.bottom)
    };
    let mut sizes = parent_sizes[start..end].to_vec();
    if let Some(first) = sizes.first_mut() { *first -= start_chrome; }
    if let Some(last) = sizes.last_mut() { *last -= end_chrome; }
    let half_difference = (parent_gap - gap) * 0.5;
    for index in 0..sizes.len().saturating_sub(1) {
        sizes[index] += half_difference;
        sizes[index + 1] += half_difference;
    }
    for size in &mut sizes { *size = size.max(0.0); }
    Some(SubgridAxis {
        sizes,
        gap,
        names: inherited_names(parent_names, start_line, end - start),
    })
}

/// A subgrid is empty for sizing its parent in this axis; its children
/// contribute in the parent's tracks, with the subgrid edge chrome added to
/// their outer contributions (Grid 2 §9). The child placement remains local.
fn subgrid_column_contribs(
    item: &LayoutBox,
    placement: &Placement,
    parent: &GridSpec,
    content_w: f32,
    fonts: &FontSource,
) -> Option<Vec<Contrib>> {
    if !typed_subgrid(&item.style, "grid-template-columns") {
        return None;
    }
    let crate::Flow::Grid(children) = &item.flow else { return Some(Vec::new()); };
    let span = placement.col_span.max(1) as usize;
    let parent_start = placement.col.unwrap_or(1) as usize;
    let inherited = inherited_names(&parent.col_line_names, parent_start, span);
    let names = local_subgrid_names(&item.style, "grid-template-columns", &inherited, span);
    let own_margin = values::resolve_margins(&item.style, item.font_size, content_w);
    let own_pad = values::resolve_padding(&item.style, item.font_size, content_w);
    let own_border = values::resolve_borders(&item.style, item.font_size);
    let start_chrome = own_margin.left.unwrap_or(0.0) + own_pad.left + own_border.left;
    let end_chrome = own_margin.right.unwrap_or(0.0) + own_pad.right + own_border.right;
    let raw = children.iter().map(|child| {
        parse_placement(&child.style, span, 1, &names, &HashMap::new())
    }).collect::<Vec<_>>();
    let Some(placed) = place_row_flow(&raw, false, span) else { return Some(Vec::new()); };
    let mut contributions = Vec::new();
    for (child, (_, _, col, col_span)) in children.iter().zip(placed) {
        let col_start = col.clamp(1, span as u32) as usize;
        let col_span = (col_span as usize).min(span + 1 - col_start);
        let specified = values::resolve_extent(&child.style, "width", child.font_size, content_w);
        let inner = match specified {
            MaybeLen::Px(width) => width.max(0.0),
            MaybeLen::Auto => crate::flex::max_content_width(child, fonts),
        };
        let margin = values::resolve_margins(&child.style, child.font_size, content_w);
        let padding = values::resolve_padding(&child.style, child.font_size, content_w);
        let border = values::resolve_borders(&child.style, child.font_size);
        let mut outer = inner + margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0)
            + padding.left + padding.right + border.left + border.right;
        if col_start == 1 { outer += start_chrome; }
        if col_start + col_span - 1 == span { outer += end_chrome; }
        contributions.push(Contrib {
            start: parent_start - 1 + col_start - 1,
            span: col_span,
            outer,
        });
    }
    if children.is_empty() {
        contributions.push(Contrib { start: parent_start - 1, span, outer: start_chrome + end_chrome });
    }
    Some(contributions)
}

fn subgrid_row_contribs(
    item: &LayoutBox,
    placement: &Placement,
    parent: &GridSpec,
    content_w: f32,
) -> Option<Vec<Contrib>> {
    if !typed_subgrid(&item.style, "grid-template-rows") {
        return None;
    }
    let crate::Flow::Grid(children) = &item.flow else { return Some(Vec::new()); };
    let span = placement.row_span.max(1) as usize;
    let parent_start = placement.row.unwrap_or(1) as usize;
    let columns = if typed_subgrid(&item.style, "grid-template-columns") {
        placement.col_span.max(1) as usize
    } else {
        GridSpec::of(&item.style, None, item.font_size, content_w, None).cols.len().max(1)
    };
    let inherited = inherited_names(&parent.row_line_names, parent_start, span);
    let names = local_subgrid_names(&item.style, "grid-template-rows", &inherited, span);
    let own_margin = values::resolve_margins(&item.style, item.font_size, content_w);
    let own_pad = values::resolve_padding(&item.style, item.font_size, content_w);
    let own_border = values::resolve_borders(&item.style, item.font_size);
    let start_chrome = own_margin.top.unwrap_or(0.0) + own_pad.top + own_border.top;
    let end_chrome = own_margin.bottom.unwrap_or(0.0) + own_pad.bottom + own_border.bottom;
    let raw = children.iter().map(|child| {
        parse_placement(&child.style, columns, span, &HashMap::new(), &names)
    }).collect::<Vec<_>>();
    let Some(placed) = place_row_flow(&raw, false, columns) else { return Some(Vec::new()); };
    let mut contributions = Vec::new();
    for (child, (row, row_span, _, _)) in children.iter().zip(placed) {
        let row_start = row.clamp(1, span as u32) as usize;
        let row_span = (row_span as usize).min(span + 1 - row_start);
        let mut outer = outer_h(child);
        if row_start == 1 { outer += start_chrome; }
        if row_start + row_span - 1 == span { outer += end_chrome; }
        contributions.push(Contrib {
            start: parent_start - 1 + row_start - 1,
            span: row_span,
            outer,
        });
    }
    if children.is_empty() {
        contributions.push(Contrib { start: parent_start - 1, span, outer: start_chrome + end_chrome });
    }
    Some(contributions)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Lays out `items` as the children of a `display: grid` container inside
/// the container's content box at (`content_x`, `content_y`) with content
/// width `content_w`. Fills every item's box-model + absolute geometry and
/// returns the grid's CONTENT HEIGHT (total track extent + gaps; equals
/// `definite_height` when stretch distributed the spare space into rows).
pub fn layout_grid(
    items: &mut Vec<LayoutBox>,
    container: &ComputedStyle,
    font_px: f32,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    definite_height: Option<f32>,
    fonts: &FontSource,
) -> f32 {
    layout_grid_with_context(items, container, None, font_px, content_x, content_y,
        content_w, definite_height, fonts)
}

pub(crate) fn layout_grid_with_context(
    items: &mut Vec<LayoutBox>,
    container: &ComputedStyle,
    context: Option<&SubgridContext>,
    font_px: f32,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    definite_height: Option<f32>,
    fonts: &FontSource,
) -> f32 {
    let spec = GridSpec::of(container, context, font_px, content_w, definite_height);

    // --- 1. placement ---------------------------------------------------
    let raw: Vec<Placement> = items
        .iter()
        .map(|it| {
            parse_placement(
                &it.style,
                spec.cols.len(),
                spec.rows.len(),
                &spec.col_line_names,
                &spec.row_line_names,
            )
        })
        .collect();
    let mut order = (0..items.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| items[index].style.get("order")
        .and_then(|value| value.parse::<i32>().ok()).unwrap_or(0));
    let ordered_raw = order.iter().map(|&index| raw[index]).collect::<Vec<_>>();
    let placed_in_order: Vec<Placement> = if spec.flow_column {
        // Run the row algorithm on transposed specs, transpose back.
        let swapped: Vec<Placement> = ordered_raw
            .iter()
            .map(|p| Placement {
                row: p.col,
                row_span: p.col_span,
                col: p.row,
                col_span: p.row_span,
            })
            .collect();
        // Transposed: the minor axis of the swapped space is the ROW axis.
        let Some(transposed) = place_row_flow(&swapped, spec.flow_dense, spec.rows.len()) else {
            return 0.0;
        };
        transposed
            .into_iter()
            .map(|(maj, msp, min, xsp)| Placement {
                row: Some(min),
                row_span: xsp,
                col: Some(maj),
                col_span: msp,
            })
            .collect()
    } else {
        // Row flow wraps auto items across the COLUMN axis.
        let Some(row_flow) = place_row_flow(&ordered_raw, spec.flow_dense, spec.cols.len()) else {
            return 0.0;
        };
        row_flow
            .into_iter()
            .map(|(maj, msp, min, xsp)| Placement {
                row: Some(maj),
                row_span: msp,
                col: Some(min),
                col_span: xsp,
            })
            .collect()
    };
    let mut placed = vec![Placement::default(); items.len()];
    for (index, placement) in order.into_iter().zip(placed_in_order) {
        placed[index] = placement;
    }

    // A subgridded axis has exactly the parent's covered tracks. Placement
    // lines outside that local explicit grid are clamped, never extended by
    // implicit tracks (Grid 2 §9).
    for placement in &mut placed {
        if let Some(axis) = context.and_then(|context| context.cols.as_ref()) {
            let count = axis.sizes.len() as u32;
            placement.col = Some(placement.col.unwrap_or(1).clamp(1, count));
            placement.col_span = placement.col_span.min(count + 1 - placement.col.unwrap());
        }
        if let Some(axis) = context.and_then(|context| context.rows.as_ref()) {
            let count = axis.sizes.len() as u32;
            placement.row = Some(placement.row.unwrap_or(1).clamp(1, count));
            placement.row_span = placement.row_span.min(count + 1 - placement.row.unwrap());
        }
    }

    // --- 2. track extents (implicit tracks wherever items landed) -------
    let n_cols = placed
        .iter()
        .map(|p| (p.col.unwrap_or(1) + p.col_span - 1) as usize)
        .fold(spec.cols.len(), usize::max);
    let n_rows = placed
        .iter()
        .map(|p| (p.row.unwrap_or(1) + p.row_span - 1) as usize)
        .fold(spec.rows.len(), usize::max);
    if n_cols == 0
        || n_rows == 0
        || n_cols > MAX_GRID_TRACKS
        || n_rows > MAX_GRID_TRACKS
        || n_cols.checked_mul(n_rows).is_none_or(|cells| cells > MAX_OCCUPIED_CELLS)
    {
        // No explicit tracks and nothing placed: degenerate grid.
        return 0.0;
    }
    let col_kinds = track_vector(&spec.cols, &spec.auto_cols, n_cols);
    let row_kinds = track_vector(&spec.rows, &spec.auto_rows, n_rows);

    // --- 3. column sizing -----------------------------------------------
    let col_contribs: Vec<Contrib> = items
        .iter()
        .zip(&placed)
        .flat_map(|(it, p)| {
            if let Some(children) = subgrid_column_contribs(it, p, &spec, content_w, fonts) {
                return children;
            }
            let specified = values::resolve_extent(&it.style, "width", it.font_size, content_w);
            let inner = match specified {
                MaybeLen::Px(w) => w.max(0.0),
                MaybeLen::Auto => crate::flex::max_content_width(it, fonts),
            };
            let m = values::resolve_margins(&it.style, it.font_size, content_w);
            let pad = values::resolve_padding(&it.style, it.font_size, content_w);
            let bor = values::resolve_borders(&it.style, it.font_size);
            vec![Contrib {
                start: (p.col.unwrap_or(1) - 1) as usize,
                span: p.col_span.max(1) as usize,
                outer: inner
                    + m.left.unwrap_or(0.0)
                    + m.right.unwrap_or(0.0)
                    + pad.left
                    + pad.right
                    + bor.left
                    + bor.right,
            }]
        })
        .collect();
    let mut col_sizes = context.and_then(|context| context.cols.as_ref()).map_or_else(
        || size_axis(&col_kinds, spec.col_gap, Some(content_w), &col_contribs),
        |axis| axis.sizes.clone(),
    );
    if spec.justify_content == Pack::Stretch
        && context.and_then(|context| context.cols.as_ref()).is_none()
    {
        stretch_auto_tracks(&mut col_sizes, &col_kinds, content_w, spec.col_gap);
    }

    // Column positions (with justify-content distribution).
    let cols_sum: f32 = col_sizes.iter().sum();
    let col_gaps_total = spec.col_gap * (n_cols.saturating_sub(1)) as f32;
    let (jc_lead, jc_between) = pack_pair(
        spec.justify_content,
        content_w - cols_sum - col_gaps_total,
        n_cols,
    );
    let mut col_pos = Vec::with_capacity(n_cols);
    if spec.rtl {
        let mut cursor = content_x + content_w - jc_lead;
        for size in &col_sizes {
            cursor -= size;
            col_pos.push(cursor);
            cursor -= spec.col_gap + jc_between;
        }
    } else {
        let mut cursor = content_x + jc_lead;
        for size in &col_sizes {
            col_pos.push(cursor);
            cursor += size + spec.col_gap + jc_between;
        }
    }

    // Area width spanned by an item's columns (incl. internal gaps).
    let area_w_of = |p: &Placement| -> f32 {
        let s = (p.col.unwrap_or(1) - 1) as usize;
        let e = (s + p.col_span.max(1) as usize).min(n_cols);
        col_sizes[s..e].iter().sum::<f32>() + spec.col_gap * (e - s - 1) as f32
    };

    // --- 4. lay out every item at its final width, measure heights ------
    let mut width_auto_flags = Vec::with_capacity(items.len());
    for (i, it) in items.iter_mut().enumerate() {
        let p = &placed[i];
        let area_w = area_w_of(p).max(0.0);
        let provisional_rows = row_kinds.iter().map(|kind| kind.base_floor()).collect::<Vec<_>>();
        let subgrid_context = SubgridContext {
            cols: subgrid_axis_for_item(it, "grid-template-columns", &col_sizes,
                spec.col_gap, &spec.col_line_names, p.col.unwrap_or(1) as usize,
                p.col_span.max(1) as usize),
            rows: subgrid_axis_for_item(it, "grid-template-rows", &provisional_rows,
                spec.row_gap, &spec.row_line_names, p.row.unwrap_or(1) as usize,
                p.row_span.max(1) as usize),
        };
        it.grid_subgrid = (subgrid_context.cols.is_some() || subgrid_context.rows.is_some())
            .then_some(subgrid_context);
        let width_auto = matches!(
            values::resolve_extent(&it.style, "width", it.font_size, area_w),
            MaybeLen::Auto
        );
        width_auto_flags.push(width_auto);
        it.margin = values::resolve_margins(&it.style, it.font_size, area_w);
        it.padding = values::resolve_padding(&it.style, it.font_size, area_w);
        it.border = values::resolve_borders(&it.style, it.font_size);
        let ml = it.margin.left.unwrap_or(0.0);
        let mr = it.margin.right.unwrap_or(0.0);
        let avail_content = (area_w - ml - mr - h_chrome(it)).max(0.0);
        let justify = container_self_align(it, "justify-self").unwrap_or(spec.justify_items);
        let stretchable =
            width_auto && !margin_is_auto(&it.style, "left") && !margin_is_auto(&it.style, "right");
        let target = if stretchable && justify == ItemAlign::Stretch {
            avail_content
        } else {
            match values::resolve_extent(&it.style, "width", it.font_size, area_w) {
                MaybeLen::Px(w) => w.max(0.0),
                MaybeLen::Auto => crate::flex::max_content_width(it, fonts).min(avail_content),
            }
        };
        // Materialize so the §10.3.3 resolver lays out at exactly this size.
        it.style.set("width", &format!("{}px", target));
        crate::block::layout_block(it, 0.0, 0.0, area_w, fonts);
    }

    // --- 5. row sizing ----------------------------------------------------
    let row_contribs: Vec<Contrib> = items
        .iter()
        .zip(&placed)
        .flat_map(|(it, p)| {
            subgrid_row_contribs(it, p, &spec, content_w).unwrap_or_else(|| vec![Contrib {
                start: (p.row.unwrap_or(1) - 1) as usize,
                span: p.row_span.max(1) as usize,
                outer: outer_h(it),
            }])
        })
        .collect();
    let mut row_sizes = context.and_then(|context| context.rows.as_ref()).map_or_else(
        || size_axis(&row_kinds, spec.row_gap, definite_height, &row_contribs),
        |axis| axis.sizes.clone(),
    );

    // align-content: stretch grows AUTO rows into the spare definite height.
    if let Some(def_h) = definite_height.filter(|_| context.and_then(|c| c.rows.as_ref()).is_none()) {
        if spec.align_content == Pack::Stretch {
            let gaps = spec.row_gap * (n_rows.saturating_sub(1)) as f32;
            let spare = def_h - gaps - row_sizes.iter().sum::<f32>();
            let autos: Vec<usize> = (0..n_rows)
                .filter(|&t| row_kinds[t] == TrackSize::Auto)
                .collect();
            if spare > 0.0 && !autos.is_empty() {
                let each = spare / autos.len() as f32;
                for t in autos {
                    row_sizes[t] += each;
                }
            }
        }
    }

    // Row positions (with align-content distribution).
    let rows_sum: f32 = row_sizes.iter().sum();
    let row_gaps_total = spec.row_gap * (n_rows.saturating_sub(1)) as f32;
    let ac_free = definite_height.map_or(0.0, |d| d - rows_sum - row_gaps_total);
    let (ac_lead, ac_between) = pack_pair(spec.align_content, ac_free, n_rows);
    let mut row_pos = Vec::with_capacity(n_rows);
    let mut cursor = content_y + ac_lead;
    for sz in &row_sizes {
        row_pos.push(cursor);
        cursor += sz + spec.row_gap + ac_between;
    }

    let content_height = rows_sum + row_gaps_total;

    // --- 6. finalize item geometry ---------------------------------------
    for (i, it) in items.iter_mut().enumerate() {
        let p = &placed[i];
        let cs = p.col_span.max(1) as usize;
        let rs = p.row_span.max(1) as usize;
        let ci = (p.col.unwrap_or(1) - 1) as usize;
        let ri = (p.row.unwrap_or(1) - 1) as usize;
        let ce = (ci + cs).min(n_cols);
        let re = (ri + rs).min(n_rows);
        let ax = col_pos[ci..ce]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let ay = row_pos[ri];
        let right = (ci..ce)
            .map(|column| col_pos[column] + col_sizes[column])
            .fold(f32::NEG_INFINITY, f32::max);
        let aw = right - ax;
        let ah = row_pos[re - 1] + row_sizes[re - 1] - ay;

        if it.grid_subgrid.as_ref().is_some_and(|context| context.rows.is_some()) {
            let axis = subgrid_axis_for_item(it, "grid-template-rows", &row_sizes,
                spec.row_gap, &spec.row_line_names, p.row.unwrap_or(1) as usize,
                p.row_span.max(1) as usize);
            if let Some(context) = it.grid_subgrid.as_mut() {
                context.rows = axis;
            }
            crate::block::layout_block(it, 0.0, 0.0, aw.max(0.0), fonts);
        }

        // Vertical stretch: grow the item to fill the area (materialize the
        // height and re-flow so children see the final block size).
        let align = container_self_align(it, "align-self").unwrap_or(spec.align_items);
        if align == ItemAlign::Stretch
            && height_is_auto(it)
            && !margin_is_auto(&it.style, "top")
            && !margin_is_auto(&it.style, "bottom")
        {
            let target_h = (ah
                - it.margin.top.unwrap_or(0.0)
                - it.margin.bottom.unwrap_or(0.0)
                - v_chrome(it))
            .max(0.0);
            if target_h > it.border_box.h {
                it.style.set("height", &format!("{}px", target_h));
                crate::block::layout_block(it, 0.0, 0.0, aw.max(0.0), fonts);
            }
        }

        // Position within the area using OUTER sizes. Margins are
        // re-resolved from the style because layout_block's §10.3.3
        // pass may have overwritten left/right with constraint fillers.
        let mut justify = container_self_align(it, "justify-self").unwrap_or(spec.justify_items);
        if justify == ItemAlign::Stretch && !width_auto_flags[i] {
            // A non-auto inline size cannot stretch; Box Alignment falls
            // back to the logical start edge of the grid area.
            justify = ItemAlign::Start;
        }
        if spec.rtl {
            justify = match justify {
                ItemAlign::Start => ItemAlign::End,
                ItemAlign::End => ItemAlign::Start,
                other => other,
            };
        }
        let m = values::resolve_margins(&it.style, it.font_size, aw.max(0.0));
        let bw = it.border_box.w;
        let auto_left = margin_is_auto(&it.style, "left");
        let auto_right = margin_is_auto(&it.style, "right");
        let mut ml = m.left.unwrap_or(0.0);
        let mr = m.right.unwrap_or(0.0);
        let horizontal_free = (aw - bw - ml - mr).max(0.0);
        match (auto_left, auto_right) {
            (true, true) => {
                ml = horizontal_free * 0.5;
            },
            (true, false) => ml = horizontal_free,
            (false, true) => {},
            (false, false) => {},
        }
        // border_box already contains padding+border, so OUTER size here is
        // margins + border_box — no extra chrome term.
        let bx = if auto_left || auto_right {
            ax + ml
        } else {
            ax
            + match justify {
                ItemAlign::End => (aw - mr - bw).max(0.0),
                ItemAlign::Center => (ml + (aw - ml - mr - bw) / 2.0).max(0.0),
                _ => ml, // Start; Stretch already fills via materialized width
            }
        };
        let auto_top = margin_is_auto(&it.style, "top");
        let auto_bottom = margin_is_auto(&it.style, "bottom");
        let mut mt = m.top.unwrap_or(0.0);
        let mb = m.bottom.unwrap_or(0.0);
        let bh = it.border_box.h;
        let vertical_free = (ah - bh - mt - mb).max(0.0);
        match (auto_top, auto_bottom) {
            (true, true) => {
                mt = vertical_free * 0.5;
            },
            (true, false) => mt = vertical_free,
            (false, true) => {},
            (false, false) => {},
        }
        let by = if auto_top || auto_bottom {
            ay + mt
        } else {
            ay
            + match align {
                ItemAlign::End => (ah - mb - bh).max(0.0),
                ItemAlign::Center => (mt + (ah - mt - mb - bh) / 2.0).max(0.0),
                _ => mt,
            }
        };
        crate::flex::translate(it, bx - it.border_box.x, by - it.border_box.y);
    }

    content_height
}

/// Per-item `-self` override (`justify-self` / `align-self`); None when
/// unset or `auto`.
fn container_self_align(it: &LayoutBox, prop: &str) -> Option<ItemAlign> {
    match it.style.get(prop) {
        Some(v) if !v.trim().eq_ignore_ascii_case("auto") => Some(parse_item_align(Some(v))),
        _ => None,
    }
}

pub(crate) fn is_grid_lanes(style: &ComputedStyle) -> bool {
    matches!(style.typed("display"),
        Some(ComputedValue::Display(CssDisplay::GridLanes | CssDisplay::InlineGridLanes)))
}

/// Intrinsic outer width for an atomic inline Grid Lanes container. Column
/// lanes combine their track contributions across the grid axis rather than
/// taking the widest child, which is the block-flow intrinsic-width fallback.
pub(crate) fn intrinsic_lanes_width(box_: &LayoutBox, fonts: &FontSource, minimum: bool) -> f32 {
    let padding = values::resolve_padding(&box_.style, box_.font_size, 0.0);
    let border = values::resolve_borders(&box_.style, box_.font_size);
    let chrome = padding.left + padding.right + border.left + border.right;
    let crate::Flow::Grid(items) = &box_.flow else { return chrome; };
    let spec = GridSpec::of(&box_.style, None, box_.font_size, 0.0, None);
    let measure = |item: &LayoutBox| {
        let width = if minimum {
            crate::flex::min_content_width(item, fonts)
        } else {
            crate::flex::max_content_width(item, fonts)
        };
        let margin = values::resolve_margins(&item.style, item.font_size, 0.0);
        width + margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0)
    };
    if spec.cols.is_empty() && !spec.rows.is_empty() {
        return chrome + items.iter().map(measure).sum::<f32>()
            + spec.col_gap * items.len().saturating_sub(1) as f32;
    }
    let raw = items.iter().map(|item| parse_placement(&item.style, spec.cols.len(),
        spec.rows.len(), &spec.col_line_names, &spec.row_line_names)).collect::<Vec<_>>();
    let tracks = raw.iter().map(|p| p.col.map_or(p.col_span as usize,
        |start| (start + p.col_span - 1) as usize))
        .fold(spec.cols.len().max(1), usize::max);
    if tracks > MAX_GRID_TRACKS { return chrome; }
    let kinds = track_vector(&spec.cols, &spec.auto_cols, tracks);
    let contributions = items.iter().zip(&raw).enumerate().map(|(index, (item, p))| Contrib {
        start: p.col.map_or(index % tracks, |line| line.saturating_sub(1) as usize),
        span: p.col_span as usize,
        outer: measure(item),
    }).collect::<Vec<_>>();
    chrome + size_axis(&kinds, spec.col_gap, None, &contributions).iter().sum::<f32>()
        + spec.col_gap * tracks.saturating_sub(1) as f32
}

/// Grid 3 §4.4: the best position is the shortest span. Positions within
/// `tolerance` of it tie; the auto-placement cursor chooses the first tied
/// start at or after itself, wrapping to the first when needed.
fn choose_lane(running: &[f32], span: usize, tolerance: f32, cursor: usize) -> (usize, f32) {
    let span = span.clamp(1, running.len());
    let candidates = (0..=running.len() - span)
        .map(|start| (start, running[start..start + span].iter().copied().fold(0.0, f32::max)))
        .collect::<Vec<_>>();
    let shortest = candidates.iter().map(|(_, pos)| *pos).fold(f32::INFINITY, f32::min);
    let tied = candidates.into_iter().filter(|(_, pos)| *pos <= shortest + tolerance)
        .collect::<Vec<_>>();
    tied.iter().copied().find(|(start, _)| *start >= cursor).unwrap_or(tied[0])
}

fn flow_tolerance(style: &ComputedStyle, font_px: f32, basis: f32) -> f32 {
    match style.get("flow-tolerance") {
        Some(value) if value.eq_ignore_ascii_case("infinite") => f32::INFINITY,
        Some(value) if !value.eq_ignore_ascii_case("normal") =>
            values::parse_used_maybe_len(style, value, font_px, basis).and_then(|v| match v {
                MaybeLen::Px(px) => Some(px.max(0.0)),
                MaybeLen::Auto => None,
            }).unwrap_or(font_px),
        _ => font_px,
    }
}

/// Grid Lanes (Grid 3, 21 January 2026 WD). The grid axis uses the normal
/// track model; the stacking axis maintains one running position per track.
pub(crate) fn layout_grid_lanes(
    items: &mut Vec<LayoutBox>,
    container: &ComputedStyle,
    font_px: f32,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    definite_height: Option<f32>,
    fonts: &FontSource,
) -> f32 {
    let spec = GridSpec::of(container, None, font_px, content_w, definite_height);
    let fill_reverse = container.get("grid-lanes-direction")
        .is_some_and(|value| value.split_whitespace().any(|token| token.eq_ignore_ascii_case("fill-reverse")));
    let row_lanes = spec.cols.is_empty() && !spec.rows.is_empty();
    if row_lanes {
        let height = layout_row_lanes(items, &spec, content_x, content_y, content_w,
            definite_height, font_px, container, fonts);
        if fill_reverse {
            for item in items.iter_mut() {
                let mirrored = content_x + content_w - (item.border_box.x - content_x) - item.border_box.w;
                crate::flex::translate(item, mirrored - item.border_box.x, 0.0);
            }
        }
        return height;
    }
    let raw = items.iter().map(|it| parse_placement(&it.style, spec.cols.len(),
        spec.rows.len(), &spec.col_line_names, &spec.row_line_names)).collect::<Vec<_>>();
    let tracks = raw.iter().map(|p| p.col.map_or(p.col_span as usize,
        |start| (start + p.col_span - 1) as usize))
        .fold(spec.cols.len().max(1), usize::max);
    if tracks > MAX_GRID_TRACKS { return 0.0; }
    let kinds = track_vector(&spec.cols, &spec.auto_cols, tracks);
    let mut contributions = Vec::new();
    for (it, p) in items.iter().zip(&raw) {
        if let Some(children) = subgrid_column_contribs(it, p, &spec, content_w, fonts) {
            contributions.extend(children);
            continue;
        }
        let width = match values::resolve_extent(&it.style, "width", it.font_size, content_w) {
            MaybeLen::Px(width) => width.max(0.0),
            MaybeLen::Auto => crate::flex::max_content_width(it, fonts),
        };
        let margin = values::resolve_margins(&it.style, it.font_size, content_w);
        let padding = values::resolve_padding(&it.style, it.font_size, content_w);
        let border = values::resolve_borders(&it.style, it.font_size);
        contributions.push(Contrib {
            start: p.col.unwrap_or(1).saturating_sub(1) as usize,
            span: p.col_span as usize,
            outer: width + margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0)
                + padding.left + padding.right + border.left + border.right,
        });
    }
    let mut sizes = size_axis(&kinds, spec.col_gap, Some(content_w), &contributions);
    if spec.justify_content == Pack::Stretch {
        stretch_auto_tracks(&mut sizes, &kinds, content_w, spec.col_gap);
    }
    let free = content_w - sizes.iter().sum::<f32>()
        - spec.col_gap * tracks.saturating_sub(1) as f32;
    let (lead, between) = pack_pair(spec.justify_content, free, tracks);
    let mut positions = Vec::with_capacity(tracks);
    if spec.rtl {
        let mut x = content_x + content_w - lead;
        for size in &sizes { x -= size; positions.push(x); x -= spec.col_gap + between; }
    } else {
        let mut x = content_x + lead;
        for size in &sizes { positions.push(x); x += size + spec.col_gap + between; }
    }
    let mut running = vec![0.0f32; tracks];
    let tolerance = flow_tolerance(container, font_px, content_w);
    let mut cursor = 0usize;
    let mut order = (0..items.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| items[*index].style.get("order")
        .and_then(|s| s.parse::<i32>().ok()).unwrap_or(0));
    for index in order {
        let p = raw[index];
        let span = (p.col_span as usize).clamp(1, tracks);
        let (start, stack) = if let Some(line) = p.col {
            let start = (line.saturating_sub(1) as usize).min(tracks - span);
            (start, running[start..start + span].iter().copied().fold(0.0, f32::max))
        } else {
            choose_lane(&running, span, tolerance, cursor)
        };
        cursor = start + span;
        let x = positions[start..start + span].iter().copied().fold(f32::INFINITY, f32::min);
        let right = (start..start + span).map(|track| positions[track] + sizes[track])
            .fold(f32::NEG_INFINITY, f32::max);
        let area_w = (right - x).max(0.0);
        let it = &mut items[index];
        let cols = subgrid_axis_for_item(it, "grid-template-columns", &sizes,
            spec.col_gap, &spec.col_line_names, start + 1, span);
        it.grid_subgrid = cols.map(|cols| SubgridContext { cols: Some(cols), rows: None });
        it.margin = values::resolve_margins(&it.style, it.font_size, area_w);
        it.padding = values::resolve_padding(&it.style, it.font_size, area_w);
        it.border = values::resolve_borders(&it.style, it.font_size);
        let available = (area_w - it.margin.left.unwrap_or(0.0)
            - it.margin.right.unwrap_or(0.0) - h_chrome(it)).max(0.0);
        let width = match values::resolve_extent(&it.style, "width", it.font_size, area_w) {
            MaybeLen::Px(width) => width.max(0.0),
            MaybeLen::Auto => available,
        };
        it.style.set("width", &format!("{}px", width));
        crate::block::layout_block(it, 0.0, 0.0, area_w, fonts);
        let y = content_y + stack + it.margin.top.unwrap_or(0.0);
        crate::flex::translate(it, x + it.margin.left.unwrap_or(0.0) - it.border_box.x,
            y - it.border_box.y);
        let advance = outer_h(it).max(0.0) + spec.row_gap;
        for position in &mut running[start..start + span] {
            *position = stack + advance;
        }
    }
    let natural_height = (running.into_iter().fold(0.0, f32::max) - spec.row_gap).max(0.0);
    if let Some(height) = definite_height {
        // Grid Lanes 3 §6.3 has one stacking-range alignment subject. Align
        // the entire waterfall, never distribute free space between its items.
        let (lead, _) = pack_pair(spec.align_content, height - natural_height, 1);
        if lead != 0.0 {
            for item in items.iter_mut() {
                crate::flex::translate(item, 0.0, lead);
            }
        }
    }
    if fill_reverse {
        let extent = definite_height.unwrap_or(natural_height);
        for item in items.iter_mut() {
            let mirrored = content_y + extent - (item.border_box.y - content_y) - item.border_box.h;
            crate::flex::translate(item, 0.0, mirrored - item.border_box.y);
        }
    }
    natural_height
}

fn layout_row_lanes(
    items: &mut Vec<LayoutBox>,
    spec: &GridSpec,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    definite_height: Option<f32>,
    font_px: f32,
    container: &ComputedStyle,
    fonts: &FontSource,
) -> f32 {
    let raw = items.iter().map(|it| parse_placement(&it.style, spec.cols.len(),
        spec.rows.len(), &spec.col_line_names, &spec.row_line_names)).collect::<Vec<_>>();
    let tracks = raw.iter().map(|p| p.row.map_or(p.row_span as usize,
        |start| (start + p.row_span - 1) as usize))
        .fold(spec.rows.len().max(1), usize::max);
    if tracks > MAX_GRID_TRACKS { return 0.0; }
    let kinds = track_vector(&spec.rows, &spec.auto_rows, tracks);
    let mut contributions = Vec::new();
    for (it, p) in items.iter_mut().zip(&raw) {
        crate::block::layout_block(it, 0.0, 0.0, content_w, fonts);
        contributions.push(Contrib {
            start: p.row.unwrap_or(1).saturating_sub(1) as usize,
            span: p.row_span as usize,
            outer: outer_h(it),
        });
    }
    let sizes = size_axis(&kinds, spec.row_gap, definite_height, &contributions);
    let natural_height = sizes.iter().sum::<f32>()
        + spec.row_gap * tracks.saturating_sub(1) as f32;
    let (lead, between) = pack_pair(spec.align_content,
        definite_height.unwrap_or(natural_height) - natural_height, tracks);
    let mut positions = Vec::with_capacity(tracks);
    let mut y = content_y + lead;
    for size in &sizes { positions.push(y); y += size + spec.row_gap + between; }
    let mut running = vec![0.0f32; tracks];
    let tolerance = flow_tolerance(container, font_px, definite_height.unwrap_or(content_w));
    let mut cursor = 0usize;
    let mut order = (0..items.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| items[*index].style.get("order")
        .and_then(|s| s.parse::<i32>().ok()).unwrap_or(0));
    for index in order {
        let p = raw[index];
        let span = (p.row_span as usize).clamp(1, tracks);
        let (start, stack) = if let Some(line) = p.row {
            let start = (line.saturating_sub(1) as usize).min(tracks - span);
            (start, running[start..start + span].iter().copied().fold(0.0, f32::max))
        } else {
            choose_lane(&running, span, tolerance, cursor)
        };
        cursor = start + span;
        let area_y = positions[start];
        let end = start + span - 1;
        let area_h = positions[end] + sizes[end] - area_y;
        let it = &mut items[index];
        let rows = subgrid_axis_for_item(it, "grid-template-rows", &sizes,
            spec.row_gap, &spec.row_line_names, start + 1, span);
        it.grid_subgrid = rows.map(|rows| SubgridContext { cols: None, rows: Some(rows) });
        let width = match values::resolve_extent(&it.style, "width", it.font_size, content_w) {
            MaybeLen::Px(width) => width.max(0.0),
            MaybeLen::Auto => crate::flex::max_content_width(it, fonts).min(content_w),
        };
        it.style.set("width", &format!("{}px", width));
        if height_is_auto(it) {
            let margin = values::resolve_margins(&it.style, it.font_size, content_w);
            let padding = values::resolve_padding(&it.style, it.font_size, content_w);
            let border = values::resolve_borders(&it.style, it.font_size);
            let chrome = margin.top.unwrap_or(0.0) + margin.bottom.unwrap_or(0.0)
                + padding.top + padding.bottom + border.top + border.bottom;
            it.style.set("height", &format!("{}px", (area_h - chrome).max(0.0)));
        }
        crate::block::layout_block(it, 0.0, 0.0, content_w, fonts);
        let margin = values::resolve_margins(&it.style, it.font_size, content_w);
        let x = content_x + stack + margin.left.unwrap_or(0.0);
        let y = area_y + it.margin.top.unwrap_or(0.0);
        crate::flex::translate(it, x - it.border_box.x, y - it.border_box.y);
        let outer_width = (it.border_box.w + margin.left.unwrap_or(0.0)
            + margin.right.unwrap_or(0.0)).max(0.0);
        for position in &mut running[start..start + span] {
            *position = stack + outer_width + spec.col_gap;
        }
    }
    if !spec.rtl {
        let natural_width = (running.into_iter().fold(0.0, f32::max) - spec.col_gap).max(0.0);
        let (lead, _) = pack_pair(spec.justify_content, content_w - natural_width, 1);
        if lead != 0.0 {
            for item in items {
                crate::flex::translate(item, lead, 0.0);
            }
        }
    }
    natural_height
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Flow, Rect};

    /// Builds grid ITEMS through the real pipeline: the test container —
    /// `<body>` itself — carries `display:grid`, so box_tree classifies the
    /// children into real [`Flow::Grid`] item boxes, which layout_grid
    /// consumes directly.
    ///
    /// Yields `item_count` empty `<div id="i0">…<div id="i{n-1}">` items;
    /// `container_css` lands on the container, `item_decls` on every item.
    fn grid_fixture(item_count: usize, container_css: &str, item_decls: &str) -> Vec<LayoutBox> {
        let mut inner = String::new();
        for i in 0..item_count {
            inner.push_str(&format!("<div id=\"i{}\"></div>", i));
        }
        let selectors: Vec<String> = (0..item_count).map(|i| format!("#i{}", i)).collect();
        let sheet = amazing_css::parse_stylesheet(&format!(
            "body {{ display: grid; {} }}\n{} {{ {} }}",
            container_css,
            selectors.join(", "),
            item_decls
        ));
        let dom = amazing_html::parse(&format!("<body>{}</body>", inner));
        let styled = amazing_css::compute(&dom, &[sheet]);
        let root = crate::box_tree::build_layout_tree(&dom, &styled, &FontSource::fallback());
        match root.flow {
            Flow::Grid(items) => items,
            other => panic!("expected container items, got {other:?}"),
        }
    }

    fn cs(pairs: &[(&str, &str)]) -> ComputedStyle {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn lay(
        items: &mut Vec<LayoutBox>,
        container: &ComputedStyle,
        content_w: f32,
        definite_h: Option<f32>,
    ) -> f32 {
        layout_grid(
            items,
            container,
            16.0,
            0.0,
            0.0,
            content_w,
            definite_h,
            &FontSource::fallback(),
        )
    }

    /// Exact-ish f32 compare (all fixtures compute to round pixels).
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn rect_eq(r: Rect, x: f32, y: f32, w: f32, h: f32) -> bool {
        let ok = close(r.x, x) && close(r.y, y) && close(r.w, w) && close(r.h, h);
        if !ok {
            eprintln!(
                "rect mismatch: got ({},{},{},{}) want ({},{},{},{})",
                r.x, r.y, r.w, r.h, x, y, w, h
            );
        }
        ok
    }

    #[test]
    fn two_by_two_px_tracks_gaps_and_stretch() {
        // Line names in the template must be consumed and dropped.
        let mut items = grid_fixture(
            4,
            "grid-template-columns:[start] 100px [mid] 200px [end]; grid-template-rows:50px 80px; row-gap:10px; column-gap:20px",
            "margin:0",
        );
        assert_eq!(items.len(), 4);
        let h = lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "[start] 100px [mid] 200px [end]"),
                ("grid-template-rows", "50px 80px"),
                ("row-gap", "10px"),
                ("column-gap", "20px"),
            ]),
            320.0,
            None,
        );
        assert!(close(h, 140.0)); // 50 + 10 + 80
        // Row-major auto placement; items stretch to fill their areas.
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 50.0));
        assert!(rect_eq(items[1].border_box, 120.0, 0.0, 200.0, 50.0));
        assert!(rect_eq(items[2].border_box, 0.0, 60.0, 100.0, 80.0));
        assert!(rect_eq(items[3].border_box, 120.0, 60.0, 200.0, 80.0));
        // Stretched border == area (no chrome), content box matches too.
        assert!(close(items[0].content_box.w, 100.0));
        assert!(close(items[3].content_box.h, 80.0));
    }

    #[test]
    fn fr_tracks_split_leftover_after_fixed_tracks() {
        // 100px fixed + 50px gap leaves 250px for the single fr track.
        let mut items = grid_fixture(
            2,
            "grid-template-columns:100px 1fr; column-gap:50px",
            "height:20px",
        );
        assert_eq!(items.len(), 2);
        lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "100px 1fr"),
                ("column-gap", "50px"),
            ]),
            400.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 150.0, 0.0, 250.0, 20.0));

        // Pure fr: weighted equal-share of the whole content width.
        let mut items = grid_fixture(2, "grid-template-columns:1fr 3fr", "height:20px");
        lay(
            &mut items,
            &cs(&[("grid-template-columns", "1fr 3fr")]),
            400.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 100.0, 0.0, 300.0, 20.0));
    }

    #[test]
    fn span_keyword_covers_two_columns_and_wraps_next_item() {
        let mut items = grid_fixture(2, "grid-template-columns:100px 200px", "height:20px");
        // First item spans both columns; second wraps to row 2.
        items[0].style.set("grid-column", "span 2");
        items[0].style.set("height", "30px");
        let h = lay(
            &mut items,
            &cs(&[("grid-template-columns", "100px 200px")]),
            300.0,
            None,
        );
        // The span is on COLUMNS, so item a occupies row 1 alone and its
        // whole 30px sizes that auto row; the wrapped item sizes row 2.
        assert!(close(h, 50.0), "grid height {} want 50", h);
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 300.0, 30.0));
        assert!(rect_eq(items[1].border_box, 0.0, 30.0, 100.0, 20.0));
    }

    #[test]
    fn negative_line_numbers_anchor_to_explicit_grid_end() {
        let mut items = grid_fixture(2, "grid-template-columns:100px 200px", "height:20px");
        // -1 is the LAST line of the 2-track explicit grid → line 3.
        items[0].style.set("grid-column", "1 / -1");
        // -2 / -1 covers exactly column 2.
        items[1].style.set("grid-column", "-2 / -1");
        lay(
            &mut items,
            &cs(&[("grid-template-columns", "100px 200px")]),
            300.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 300.0, 20.0));
        // Item b's column-start (2) precedes the sparse cursor (3 after the
        // spanning item) AND cell (1,2) is taken — §8.5 wraps it to row 2.
        assert!(rect_eq(items[1].border_box, 100.0, 20.0, 200.0, 20.0));
    }

    #[test]
    fn implicit_rows_and_auto_columns_created_on_demand() {
        let mut items = grid_fixture(
            4,
            "grid-template-columns:100px 200px; grid-auto-columns:30px 10px",
            "height:40px",
        );
        assert_eq!(items.len(), 4);
        items[3].style.set("grid-column", "4");
        lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "100px 200px"),
                ("grid-auto-columns", "30px 10px"),
            ]),
            340.0,
            None,
        );
        // A definite position at column 4 grows the implicit grid before
        // auto-placement, so the third auto item can use implicit column 3.
        assert!(rect_eq(items[2].border_box, 300.0, 0.0, 30.0, 40.0));
        assert!(rect_eq(items[3].border_box, 330.0, 0.0, 10.0, 40.0));
    }

    #[test]
    fn definite_items_and_locked_rows_precede_auto_placement() {
        let specs = [
            Placement { row: None, row_span: 1, col: None, col_span: 1 },
            Placement { row: Some(1), row_span: 1, col: None, col_span: 1 },
            Placement { row: Some(1), row_span: 1, col: Some(1), col_span: 1 },
        ];
        assert_eq!(
            place_row_flow(&specs, false, 3),
            Some(vec![(1, 1, 3, 1), (1, 1, 2, 1), (1, 1, 1, 1)])
        );
    }

    #[test]
    fn grid_order_changes_auto_placement_without_changing_item_storage() {
        let mut items = grid_fixture(3, "grid-template-columns:20px 20px 20px", "height:10px");
        items[0].style.set("order", "2");
        items[1].style.set("order", "-1");
        lay(&mut items, &cs(&[("grid-template-columns", "20px 20px 20px")]), 60.0, None);
        assert!(rect_eq(items[1].border_box, 0.0, 0.0, 20.0, 10.0));
        assert!(rect_eq(items[2].border_box, 20.0, 0.0, 20.0, 10.0));
        assert!(rect_eq(items[0].border_box, 40.0, 0.0, 20.0, 10.0));
    }

    #[test]
    fn grid_safe_alignment_keeps_its_requested_position() {
        assert_eq!(parse_pack(Some("safe end")), Pack::End);
        assert_eq!(parse_item_align(Some("safe center")), ItemAlign::Center);
    }

    #[test]
    fn dense_backflow_differs_from_sparse_cursor() {
        // b auto → (1,1); c locked to col 1 → (2,1);
        // d locked to col 2: sparse cursor keeps it at (2,2),
        // dense rescans and backfills (1,2).
        let mut sparse = grid_fixture(3, "grid-template-columns:50px 50px", "height:20px");
        sparse[1].style.set("grid-column", "1");
        sparse[2].style.set("grid-column", "2");
        lay(
            &mut sparse,
            &cs(&[("grid-template-columns", "50px 50px")]),
            100.0,
            None,
        );
        assert!(rect_eq(sparse[0].border_box, 0.0, 0.0, 50.0, 20.0));
        assert!(rect_eq(sparse[1].border_box, 0.0, 20.0, 50.0, 20.0));
        assert!(rect_eq(sparse[2].border_box, 50.0, 20.0, 50.0, 20.0));

        let mut dense = grid_fixture(
            3,
            "grid-auto-flow:dense; grid-template-columns:50px 50px",
            "height:20px",
        );
        dense[1].style.set("grid-column", "1");
        dense[2].style.set("grid-column", "2");
        lay(
            &mut dense,
            &cs(&[
                ("grid-auto-flow", "dense"),
                ("grid-template-columns", "50px 50px"),
            ]),
            100.0,
            None,
        );
        assert!(rect_eq(dense[0].border_box, 0.0, 0.0, 50.0, 20.0));
        assert!(rect_eq(dense[1].border_box, 0.0, 20.0, 50.0, 20.0));
        assert!(rect_eq(dense[2].border_box, 50.0, 0.0, 50.0, 20.0)); // backfilled
    }

    #[test]
    fn center_alignment_offsets_within_the_area() {
        let mut items = grid_fixture(
            1,
            "grid-template-columns:200px; grid-template-rows:100px; justify-items:center; align-items:center",
            "width:50px; height:20px",
        );
        assert_eq!(items.len(), 1);
        let h = lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "200px"),
                ("grid-template-rows", "100px"),
                ("justify-items", "center"),
                ("align-items", "center"),
            ]),
            200.0,
            None,
        );
        assert!(close(h, 100.0));
        assert!(rect_eq(items[0].border_box, 75.0, 40.0, 50.0, 20.0));

        // start/end keywords on the same fixture via -self overrides.
        let mut items = grid_fixture(
            1,
            "grid-template-columns:200px; grid-template-rows:100px",
            "width:50px; height:20px",
        );
        items[0].style.set("justify-self", "end");
        items[0].style.set("align-self", "start");
        lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "200px"),
                ("grid-template-rows", "100px"),
            ]),
            200.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 150.0, 0.0, 50.0, 20.0));
    }

    #[test]
    fn stretch_fills_area_through_padding() {
        let mut items = grid_fixture(
            1,
            "grid-template-columns:100px; grid-template-rows:60px",
            "padding:10px",
        );
        let h = lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "100px"),
                ("grid-template-rows", "60px"),
            ]),
            100.0,
            None,
        );
        assert!(close(h, 60.0));
        // Horizontal stretch laid out at 80px content; vertical stretch
        // re-flowed to 40px content inside the 60px row.
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 60.0));
        assert!(rect_eq(items[0].content_box, 10.0, 10.0, 80.0, 40.0));
    }

    #[test]
    fn percentage_tracks_resolve_against_content_width() {
        let mut items = grid_fixture(2, "grid-template-columns:50% 100px", "height:20px");
        lay(
            &mut items,
            &cs(&[("grid-template-columns", "50% 100px")]),
            400.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 200.0, 20.0));
        assert!(rect_eq(items[1].border_box, 200.0, 0.0, 100.0, 20.0));
    }

    #[test]
    fn repeat_fixed_count_and_auto_fill() {
        // repeat(<integer>, …) expands verbatim.
        let mut items = grid_fixture(2, "grid-template-columns:repeat(2, 100px)", "height:20px");
        lay(
            &mut items,
            &cs(&[("grid-template-columns", "repeat(2, 100px)")]),
            400.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 100.0, 0.0, 100.0, 20.0));

        // auto-fill: floor((400 + 0) / 100) = 4 definite tracks.
        let mut items = grid_fixture(
            4,
            "grid-template-columns:repeat(auto-fill, 100px)",
            "height:20px",
        );
        assert_eq!(items.len(), 4);
        lay(
            &mut items,
            &cs(&[("grid-template-columns", "repeat(auto-fill, 100px)")]),
            400.0,
            None,
        );
        assert!(rect_eq(items[3].border_box, 300.0, 0.0, 100.0, 20.0));
    }

    #[test]
    fn justify_content_distributes_free_columns_space() {
        // Two 50px tracks in 200px with space-between → positions 0 and 150.
        let mut items = grid_fixture(
            2,
            "grid-template-columns:repeat(2, 50px); justify-content:space-between",
            "height:20px",
        );
        lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "repeat(2, 50px)"),
                ("justify-content", "space-between"),
            ]),
            200.0,
            None,
        );
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 50.0, 20.0));
        assert!(rect_eq(items[1].border_box, 150.0, 0.0, 50.0, 20.0));
    }

    #[test]
    fn align_content_centers_rows_in_definite_height() {
        let mut items = grid_fixture(
            1,
            "grid-template-columns:100px; grid-template-rows:20px; align-content:center",
            "height:20px",
        );
        let h = lay(
            &mut items,
            &cs(&[
                ("grid-template-columns", "100px"),
                ("grid-template-rows", "20px"),
                ("align-content", "center"),
            ]),
            100.0,
            Some(100.0),
        );
        assert!(close(h, 20.0)); // natural track extent is returned
        // With center alignment the row shifts down by (100−20)/2.
        assert!(close(items[0].border_box.y, 40.0));
    }

    // --- parser units -----------------------------------------------------

    #[test]
    fn track_size_parsing_forms() {
        assert_eq!(parse_track_size("1fr", 16.0, 400.0, 1.0), TrackSize::Fr(1.0));
        assert_eq!(parse_track_size("2.5FR", 16.0, 400.0, 1.0), TrackSize::Fr(2.5));
        assert_eq!(parse_track_size("auto", 16.0, 400.0, 1.0), TrackSize::Auto);
        assert_eq!(
            parse_track_size("min-content", 16.0, 400.0, 1.0),
            TrackSize::Content
        );
        assert_eq!(
            parse_track_size("max-content", 16.0, 400.0, 1.0),
            TrackSize::Content
        );
        assert_eq!(parse_track_size("25px", 16.0, 400.0, 1.0), TrackSize::Px(25.0));
        assert_eq!(parse_track_size("50%", 16.0, 400.0, 1.0), TrackSize::Px(200.0));
        assert_eq!(parse_track_size("nonsense", 16.0, 400.0, 1.0), TrackSize::Auto);
        assert_eq!(parse_track_size("25px", 32.0, 400.0, 2.0), TrackSize::Px(50.0));
        assert_eq!(parse_track_size("50%", 32.0, 400.0, 2.0), TrackSize::Px(200.0));
    }

    #[test]
    fn nested_repeat_is_rejected_before_recursive_track_expansion() {
        let ctx = RepeatCtx {
            font_px: 16.0,
            content_w: 400.0,
            zoom: 1.0,
            repeat_available: Some(400.0),
            gap: 0.0,
            fixed_before: 0.0,
        };
        assert!(parse_track_list("repeat(2,repeat(2,10px))", &ctx, true).is_empty());
        assert!(collect_line_names("repeat(2,repeat(2,[x] 10px))", &ctx, true).is_empty());
    }

    #[test]
    fn top_level_tokens_keep_repeat_groups_together() {
        assert_eq!(
            top_tokens("repeat(2, 100px) 50px"),
            vec!["repeat(2, 100px)".to_string(), "50px".to_string()]
        );
        assert_eq!(
            top_tokens("[a b] 1fr"),
            vec!["[a b]".to_string(), "1fr".to_string()]
        );
    }

    #[test]
    fn slot_combination_resolves_negatives_before_ordering() {
        let s = |l: Option<i32>, sp: u32| LineSpec { line: l, span: sp };
        // 1 / -1 over 2 explicit tracks: -1 → line 3, ordered pair (1,3).
        assert_eq!(
            combine_slots(s(Some(1), 1), s(Some(-1), 1), 2),
            (Some(1), 2)
        );
        // -2 / -1 → lines 2..3 → starts at 2, span 1.
        assert_eq!(
            combine_slots(s(Some(-2), 1), s(Some(-1), 1), 2),
            (Some(2), 1)
        );
        // Swapped ends order themselves after resolution.
        assert_eq!(
            combine_slots(s(Some(-1), 1), s(Some(1), 1), 2),
            (Some(1), 2)
        );
        // Span-only both sides take the widest span.
        assert_eq!(combine_slots(s(None, 2), s(None, 3), 0), (None, 3));
        // Start auto + end line 3 with span 2 → start at line 1.
        assert_eq!(combine_slots(s(None, 2), s(Some(3), 1), 0), (Some(1), 2));
    }

    #[test]
    fn implicit_tracks_cycle_the_auto_list() {
        let kinds = track_vector(&[], &[TrackSize::Px(30.0), TrackSize::Px(10.0)], 3);
        assert_eq!(
            kinds,
            vec![
                TrackSize::Px(30.0),
                TrackSize::Px(10.0),
                TrackSize::Px(30.0)
            ]
        );
        let explicit_only = track_vector(&[TrackSize::Auto], &[TrackSize::Px(5.0)], 1);
        assert_eq!(explicit_only, vec![TrackSize::Auto]);
    }

    #[test]
    fn spanning_intrinsic_contribution_excludes_internal_gap_and_fixed_tracks() {
        let kinds = [TrackSize::Auto, TrackSize::Auto];
        let sizes = size_axis(&kinds, 10.0, None,
            &[Contrib { start: 0, span: 2, outer: 210.0 }]);
        assert!(close(sizes[0], 100.0) && close(sizes[1], 100.0));
        let mixed = [TrackSize::Px(100.0), TrackSize::Auto];
        let sizes = size_axis(&mixed, 10.0, None,
            &[Contrib { start: 0, span: 2, outer: 170.0 }]);
        assert!(close(sizes[0], 100.0) && close(sizes[1], 60.0));
    }

    #[test]
    fn column_subgrid_inherits_tracks_gap_and_parent_line_names() {
        let root = crate::layout_document(
            "<body><section><div id=a></div><div id=b></div></section></body>",
            "body{display:grid;margin:0;grid-template-columns:[first] 50px [middle] 80px [last];column-gap:10px}section{display:grid;grid-column:1/span 2;grid-template-columns:subgrid [local-start] [local-mid] [local-end]}#a{grid-column:middle;height:5px}#b{grid-column:local-start;height:5px}",
            300.0, &FontSource::fallback(),
        );
        let Flow::Grid(parent_items) = &root.flow else { panic!("parent grid missing") };
        let Flow::Grid(children) = &parent_items[0].flow else { panic!("subgrid missing") };
        assert!(close(children[0].border_box.x, 60.0));
        assert!(close(children[0].border_box.w, 80.0));
        assert!(close(children[1].border_box.x, 0.0));
        assert!(close(children[1].border_box.w, 50.0));
    }

    #[test]
    fn subgrid_child_sizing_contributes_to_parent_auto_track() {
        let root = crate::layout_document(
            "<body><section><div></div></section></body>",
            "body{display:grid;margin:0;grid-template-columns:auto 50px;justify-content:start}section{display:grid;grid-column:1/span 2;grid-template-columns:subgrid}section>div{width:70px;height:10px}",
            200.0, &FontSource::fallback(),
        );
        let Flow::Grid(parent_items) = &root.flow else { panic!("parent grid missing") };
        let Flow::Grid(children) = &parent_items[0].flow else { panic!("subgrid missing") };
        assert!(close(children[0].border_box.w, 70.0));
        assert!(close(parent_items[0].border_box.w, 120.0));
    }

    #[test]
    fn subgrid_padding_consumes_parent_edge_track_space() {
        let root = crate::layout_document(
            "<body><section><div id=a></div><div id=b></div></section></body>",
            "body{display:grid;margin:0;grid-template-columns:50px 80px;column-gap:10px}section{display:grid;grid-column:1/span 2;grid-template-columns:subgrid;padding-left:5px}#a,#b{height:5px}",
            200.0, &FontSource::fallback(),
        );
        let Flow::Grid(parent_items) = &root.flow else { panic!("parent grid missing") };
        let Flow::Grid(children) = &parent_items[0].flow else { panic!("subgrid missing") };
        assert!(rect_eq(children[0].border_box, 5.0, 0.0, 45.0, 5.0));
        assert!(rect_eq(children[1].border_box, 60.0, 0.0, 80.0, 5.0));
    }

    #[test]
    fn subgrid_explicit_gap_adjusts_child_tracks_and_positions() {
        let root = crate::layout_document(
            "<body><section><div></div><div></div></section></body>",
            "body{display:grid;margin:0;grid-template-columns:50px 80px;column-gap:10px}section{display:grid;grid-column:1/span 2;grid-template-columns:subgrid;column-gap:20px}section>div{height:5px}",
            140.0, &FontSource::fallback(),
        );
        let Flow::Grid(parent_items) = &root.flow else { panic!("parent grid missing") };
        let Flow::Grid(children) = &parent_items[0].flow else { panic!("subgrid missing") };
        assert!(rect_eq(children[0].border_box, 0.0, 0.0, 45.0, 5.0));
        assert!(rect_eq(children[1].border_box, 65.0, 0.0, 75.0, 5.0));
    }

    #[test]
    fn grid_lanes_chooses_shortest_with_cursor_tolerance_and_span() {
        assert_eq!(choose_lane(&[40.0, 20.0, 30.0], 1, 0.0, 0), (1, 20.0));
        assert_eq!(choose_lane(&[40.0, 20.0, 30.0], 1, 16.0, 2), (2, 30.0));
        assert_eq!(choose_lane(&[40.0, 20.0, 30.0], 2, 0.0, 0), (1, 30.0));
    }

    #[test]
    fn grid_lanes_places_items_in_shortest_columns() {
        let root = crate::layout_document(
            "<body><div id=a></div><div id=b></div><div id=c></div></body>",
            "body{display:grid-lanes;margin:0;grid-template-columns:50px 50px;column-gap:10px;row-gap:5px}#a{height:30px}#b{height:10px}#c{height:10px}",
            110.0, &FontSource::fallback(),
        );
        let Flow::Grid(items) = &root.flow else { panic!("grid lanes items missing") };
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 50.0, 30.0));
        assert!(rect_eq(items[1].border_box, 60.0, 0.0, 50.0, 10.0));
        assert!(rect_eq(items[2].border_box, 60.0, 15.0, 50.0, 10.0));
    }

    #[test]
    fn grid_lanes_align_content_moves_the_whole_stacking_range() {
        let root = crate::layout_document(
            "<body><div></div><div></div></body>",
            "body{display:grid-lanes;margin:0;height:100px;grid-template-columns:50px 50px;align-content:end}body>div{height:10px}",
            100.0, &FontSource::fallback(),
        );
        let Flow::Grid(items) = &root.flow else { panic!("lanes missing") };
        assert!(rect_eq(items[0].border_box, 0.0, 90.0, 50.0, 10.0));
        assert!(rect_eq(items[1].border_box, 50.0, 90.0, 50.0, 10.0));
    }

    #[test]
    fn grid_lanes_accepts_column_subgrid_child() {
        let root = crate::layout_document(
            "<body><section><div id=a></div><div id=b></div></section></body>",
            "body{display:grid-lanes;margin:0;grid-template-columns:50px 80px;column-gap:10px}section{display:grid;grid-column:1/span 2;grid-template-columns:subgrid}#a,#b{height:5px}",
            140.0, &FontSource::fallback(),
        );
        let Flow::Grid(parent_items) = &root.flow else { panic!("lanes missing") };
        let Flow::Grid(children) = &parent_items[0].flow else { panic!("subgrid missing") };
        assert!(rect_eq(children[0].border_box, 0.0, 0.0, 50.0, 5.0));
        assert!(rect_eq(children[1].border_box, 60.0, 0.0, 80.0, 5.0));
    }

    #[test]
    fn grid_lanes_row_orientation_stacks_into_shortest_row() {
        let root = crate::layout_document(
            "<body><div id=a></div><div id=b></div><div id=c></div></body>",
            "body{display:grid-lanes;margin:0;grid-template-rows:20px 20px;column-gap:5px}#a{width:30px}#b,#c{width:10px}",
            100.0, &FontSource::fallback(),
        );
        let Flow::Grid(items) = &root.flow else { panic!("lanes missing") };
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 30.0, 20.0));
        assert!(rect_eq(items[1].border_box, 0.0, 20.0, 10.0, 20.0));
        assert!(rect_eq(items[2].border_box, 15.0, 20.0, 10.0, 20.0));
    }

    #[test]
    fn grid_lanes_fill_reverse_mirrors_the_stacking_axis() {
        let root = crate::layout_document(
            "<body><div></div><div></div></body>",
            "body{display:grid-lanes;margin:0;grid-lanes-direction:row fill-reverse;grid-template-rows:20px 20px;width:100px}body>div{width:20px}",
            100.0, &FontSource::fallback(),
        );
        let Flow::Grid(items) = &root.flow else { panic!("lanes missing") };
        assert!(items.iter().all(|item| item.border_box.x >= 80.0));
    }

    #[test]
    fn normal_grid_empty_items_stretch_to_definite_row_tracks() {
        let root = crate::layout_document(
            "<body><div></div><div></div><div></div></body>",
            "body{display:grid;margin:0;width:100px;height:100px;grid-template-rows:20px 1fr 30%}body>div{background:orange}",
            200.0, &FontSource::fallback(),
        );
        let Flow::Grid(items) = &root.flow else { panic!("grid missing") };
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 0.0, 20.0, 100.0, 50.0));
        assert!(rect_eq(items[2].border_box, 0.0, 70.0, 100.0, 30.0));
    }
}
