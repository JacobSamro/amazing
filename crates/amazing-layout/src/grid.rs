//! CSS Grid layout (css-grid-1 subset): track sizing, item placement and
//! alignment for `display: grid` containers.
//!
//! Implemented:
//! - `grid-template-columns` / `grid-template-rows` track lists: lengths,
//!   percentages (of the container's content width for BOTH axes — see
//!   deviations), `auto`, `min-content` / `max-content` (both approximated
//!   as max-content), `<number>fr`, `repeat(<integer> | auto-fill, …)` with
//!   nested track lists; `[line-names]` are consumed and dropped
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
//!   tracks sized from item measurements; `fr` tracks split the space left
//!   AFTER non-fr tracks (weighted equal-share); row heights measured by
//!   actually laying each item out at its final column width
//! - alignment: `justify-content` / `align-content`
//!   (start/end/center/stretch/space-between/around/evenly),
//!   `justify-items` / `align-items` + per-item `-self` longhands
//!   (stretch/start/end/center); stretch fills the area (items re-laid-out
//!   at their final width, stretched heights materialized and re-flowed)
//!
//! Deviations / deferrals (documented):
//! - named areas / `grid-area` / `grid-template-areas` are not implemented
//! - `span <line-name>` → `span 1`; a bare line-name position → `auto`
//! - percentages in `grid-template-rows` resolve against the content WIDTH
//!   (the height is indefinite at parse time in this engine)
//! - `auto-fill` only repeats beyond 1 when the repeated track size is
//!   definite (px); otherwise it behaves like `repeat(1, …)`
//! - items spanning several intrinsic tracks contribute an EQUAL share of
//!   their measurement to each spanned intrinsic track
//! - `fr` rows without a definite container height behave like `auto`;
//!   `fr` tracks get nothing when the leftover space is negative (no
//!   minimum floors, no fr shrink of non-fr tracks)
//! - auto margins on items count as zero (no free-space absorption);
//!   stretch is skipped when the relevant margins are auto
//! - dense placement rescans from the grid start but does not implement
//!   the spec's exact cursor-monotonicity bookkeeping

use amazing_css::ComputedStyle;
use std::collections::HashMap;

use crate::font::FontSource;
use crate::values::{self, MaybeLen};
use crate::LayoutBox;

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
}

impl TrackSize {
    fn is_intrinsic(self) -> bool {
        matches!(self, TrackSize::Auto | TrackSize::Content)
    }
    fn is_fr(self) -> bool {
        matches!(self, TrackSize::Fr(_))
    }
}

/// Parse one track-size component value.
fn parse_track_size(s: &str, font_px: f32, content_w: f32) -> TrackSize {
    let s = s.trim();
    // Lowercase first so `FR`/`Auto` spellings parse identically.
    let lower = s.to_ascii_lowercase();
    if let Some(stripped) = lower.strip_suffix("fr") {
        if let Ok(n) = stripped.trim().parse::<f32>() {
            return TrackSize::Fr(n.max(0.0));
        }
        return TrackSize::Auto;
    }
    match lower.as_str() {
        "auto" => return TrackSize::Auto,
        "min-content" | "max-content" => return TrackSize::Content,
        _ => {},
    }
    match MaybeLen::parse(s, font_px, content_w) {
        Some(MaybeLen::Px(v)) => TrackSize::Px(v.max(0.0)),
        _ => TrackSize::Auto, // unparseable → treated as auto
    }
}

/// Splits a declaration value into top-level whitespace-separated tokens,
/// keeping parenthesized groups (`repeat(2, 100px)`) as single tokens.
fn top_tokens(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in value.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            },
            ')' => {
                depth -= 1;
                cur.push(ch);
            },
            c if c.is_whitespace() && depth <= 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            },
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Context needed to expand `repeat(auto-fill, …)`.
struct RepeatCtx {
    font_px: f32,
    content_w: f32,
    gap: f32,
    /// Definite px already consumed by tracks emitted before this repeat.
    fixed_before: f32,
}

/// Parses a full track list: components, `[line names]` (dropped), and
/// `repeat()` groups. Total track count is capped defensively.
fn parse_track_list(value: &str, ctx: &RepeatCtx, allow_auto_fill: bool) -> Vec<TrackSize> {
    const MAX_TRACKS: usize = 1000;
    let mut out = Vec::new();
    for tok in top_tokens(value) {
        if out.len() >= MAX_TRACKS {
            break;
        }
        // Line names `[a b]` may split into several tokens; drop them all.
        if tok.contains('[') || tok.contains(']') {
            continue;
        }
        let lower = tok.to_ascii_lowercase();
        if let Some(inner) = lower.strip_prefix("repeat(").and_then(|s| s.strip_suffix(')')) {
            // repeat(<count>, <track-list>) — count is an integer or
            // auto-fill; the inner list re-enters this parser (nesting
            // comes free, inner auto-fill degrades to 1).
            let (count_part, list_part) = match inner.find(',') {
                Some(i) => (&inner[..i], &inner[i + 1..]),
                None => (inner, ""),
            };
            let n = match count_part.trim().parse::<usize>() {
                Ok(n) => n.min(MAX_TRACKS),
                Err(_) => {
                    // auto-fill: expandable ONLY when every repeated track
                    // is definite; otherwise one repetition (documented).
                    if !allow_auto_fill || !count_part.trim().eq_ignore_ascii_case("auto-fill") {
                        1
                    } else {
                        let sizes = parse_track_list(list_part, &RepeatCtx {
                            font_px: ctx.font_px,
                            content_w: ctx.content_w,
                            gap: ctx.gap,
                            fixed_before: ctx.fixed_before,
                        }, false);
                        let all_definite = sizes.iter().all(|s| matches!(*s, TrackSize::Px(_)));
                        let unit: f32 = sizes
                            .iter()
                            .map(|s| match *s {
                                TrackSize::Px(v) => v,
                                _ => 0.0,
                            })
                            .sum::<f32>()
                            + ctx.gap * sizes.len().saturating_sub(1) as f32;
                        let avail = (ctx.content_w - ctx.fixed_before).max(0.0);
                        if all_definite && unit > 0.0 {
                            (((avail + ctx.gap) / unit).floor() as usize).clamp(1, MAX_TRACKS)
                        } else {
                            1
                        }
                    }
                },
            };
            for _ in 0..n {
                if out.len() >= MAX_TRACKS {
                    break;
                }
                let parsed = parse_track_list(list_part, &RepeatCtx {
                    font_px: ctx.font_px,
                    content_w: ctx.content_w,
                    gap: ctx.gap,
                    fixed_before: ctx.fixed_before,
                }, false);
                out.extend(parsed);
            }
            continue;
        }
        out.push(parse_track_size(&tok, ctx.font_px, ctx.content_w));
    }
    out
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
    match v.map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("end") || v.eq_ignore_ascii_case("flex-end") => Pack::End,
        Some(v) if v.eq_ignore_ascii_case("center") => Pack::Center,
        Some(v) if v.eq_ignore_ascii_case("space-between") => Pack::SpaceBetween,
        Some(v) if v.eq_ignore_ascii_case("space-around") => Pack::SpaceAround,
        Some(v) if v.eq_ignore_ascii_case("space-evenly") => Pack::SpaceEvenly,
        Some(v) if v.eq_ignore_ascii_case("stretch") => Pack::Stretch,
        _ => Pack::Start,
    }
}

fn parse_item_align(v: Option<&str>) -> ItemAlign {
    match v.map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("start") || v.eq_ignore_ascii_case("flex-start") => ItemAlign::Start,
        Some(v) if v.eq_ignore_ascii_case("end") || v.eq_ignore_ascii_case("flex-end") => ItemAlign::End,
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
    let px = |s: &str| MaybeLen::parse(s, font_px, content_w).map_or(0.0, |m| m.unwrap_or(0.0));
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
    auto_cols: Vec<TrackSize>,
    auto_rows: Vec<TrackSize>,
    row_gap: f32,
    col_gap: f32,
    flow_column: bool,
    flow_dense: bool,
    justify_content: Pack,
    align_content: Pack,
    justify_items: ItemAlign,
    align_items: ItemAlign,
}

impl GridSpec {
    fn of(style: &ComputedStyle, font_px: f32, content_w: f32) -> Self {
        let (row_gap, col_gap) = parse_gaps(style, font_px, content_w);
        let ctx_cols = RepeatCtx { font_px, content_w, gap: col_gap, fixed_before: 0.0 };
        let ctx_rows = RepeatCtx { font_px, content_w, gap: row_gap, fixed_before: 0.0 };
        let empty_auto = vec![TrackSize::Auto];
        let flow = style.get("grid-auto-flow").unwrap_or("row").to_ascii_lowercase();
        let words: Vec<&str> = flow.split_ascii_whitespace().collect();
        GridSpec {
            cols: style
                .get("grid-template-columns")
                .map(|v| parse_track_list(v, &ctx_cols, true))
                .unwrap_or_default(),
            rows: style
                .get("grid-template-rows")
                .map(|v| parse_track_list(v, &ctx_rows, true))
                .unwrap_or_default(),
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
    const AUTO: LineSpec = LineSpec { line: None, span: 1 };
}

/// Parses one slot value: `auto`, `<integer>`, `span N`, or a bare /
/// spanned line NAME (names are unsupported → `auto` / `span 1`,
/// documented).
fn parse_line_slot(s: &str) -> LineSpec {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        return LineSpec::AUTO;
    }
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("span") {
        let rest = rest.trim();
        return match rest.parse::<u32>() {
            Ok(n) if n >= 1 => LineSpec { line: None, span: n.min(1000) },
            _ => LineSpec { line: None, span: 1 }, // `span <name>` → span 1
        };
    }
    match s.parse::<i32>() {
        Ok(0) | Err(_) => LineSpec::AUTO, // line 0 is invalid → auto
        Ok(n) => LineSpec { line: Some(n), span: 1 },
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
fn combine_slots(
    start: LineSpec,
    end: LineSpec,
    explicit_count: usize,
) -> (Option<u32>, u32) {
    let resolve = |l: i32| -> Option<u32> {
        if l < 0 {
            let from_end = explicit_count as i32 + 2 + l;
            if from_end < 1 { None } else { Some(from_end as u32) }
        } else if l == 0 {
            None
        } else {
            Some(l as u32)
        }
    };
    match (start.line.and_then(resolve), end.line.and_then(resolve)) {
        (Some(a), Some(b)) => {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            (Some(lo), (hi - lo).max(1))
        },
        (Some(a), None) => (Some(a), end.span.max(1)),
        (None, Some(b)) => {
            let span = start.span.max(1);
            (Some(if b > span { b - span } else { 1 }), span)
        },
        (None, None) => (None, start.span.max(end.span).max(1)),
    }
}

/// Reads an item's `grid-row` / `grid-column` (shorthand first, then the
/// four longhands override their slots).
fn parse_placement(style: &ComputedStyle, explicit_cols: usize, explicit_rows: usize) -> Placement {
    let side = |shorthand: &str, s_prop: &str, e_prop: &str, expl: usize| {
        let mut start = LineSpec::AUTO;
        let mut end = LineSpec::AUTO;
        if let Some(v) = style.get(shorthand) {
            let parts: Vec<&str> = v.split('/').collect();
            start = parse_line_slot(parts.first().copied().unwrap_or("auto"));
            end = match parts.get(1) {
                Some(e) => parse_line_slot(e),
                None => LineSpec::AUTO,
            };
        }
        if let Some(v) = style.get(s_prop) {
            start = parse_line_slot(v);
        }
        if let Some(v) = style.get(e_prop) {
            end = parse_line_slot(v);
        }
        combine_slots(start, end, expl)
    };
    let (col, col_span) = side("grid-column", "grid-column-start", "grid-column-end", explicit_cols);
    let (row, row_span) = side("grid-row", "grid-row-start", "grid-row-end", explicit_rows);
    Placement { row, row_span, col, col_span }
}

// ---------------------------------------------------------------------------
// Auto-placement (css-grid-1 §8.5, simplified sparse/dense model)
// ---------------------------------------------------------------------------

/// Occupied-cell set over 1-based (row, col) pairs; grows without bound.
#[derive(Default)]
struct Occupancy {
    cells: HashMap<(u32, u32), ()>,
}

impl Occupancy {
    fn fits(&self, r: u32, c: u32, rs: u32, cs: u32) -> bool {
        for i in 0..rs {
            for j in 0..cs {
                if self.cells.contains_key(&(r + i, c + j)) {
                    return false;
                }
            }
        }
        true
    }
    fn put(&mut self, r: u32, c: u32, rs: u32, cs: u32) {
        for i in 0..rs {
            for j in 0..cs {
                self.cells.insert((r + i, c + j), ());
            }
        }
    }
}

/// Row-flow placement in (major=row, minor=col) coordinates. Returns
/// definite `(row, row_span, col, col_span)` per item, document order.
///
/// Sparse packing keeps a forward-only cursor; dense packing rescans from
/// the grid start for every auto-placed item (backflow). Definite-major /
/// auto-minor items scan the minor axis unboundedly (creating implicit
/// tracks); both-auto items wrap at `expl_minor` (or the widest auto span).
fn place_row_flow(specs: &[Placement], dense: bool, expl_minor: usize) -> Vec<(u32, u32, u32, u32)> {
    // Wrap width for both-auto items: explicit minors, at least the widest
    // auto-placed span (so wide items always get columns to fit into).
    let mut limit = expl_minor.max(1);
    for p in specs {
        if p.row.is_none() && p.col.is_none() {
            limit = limit.max(p.col_span as usize);
        }
    }

    let mut occ = Occupancy::default();
    let mut out = Vec::with_capacity(specs.len());
    let (mut cur_r, mut cur_c) = (1u32, 1u32);

    for p in specs {
        let rs = p.row_span.max(1);
        let cs = p.col_span.max(1);
        match (p.row, p.col) {
            (Some(r), Some(c)) => {
                occ.put(r, c, rs, cs);
                out.push((r, rs, c, cs));
            },
            // Definite row, auto column: slide right within the locked row.
            (Some(r), None) => {
                let mut c = if dense {
                    1
                } else {
                    if 1 < cur_c {
                        cur_r += 1; // sparse monotonicity bookkeeping
                    }
                    1
                };
                while !occ.fits(r, c, rs, cs) {
                    c += 1;
                }
                occ.put(r, c, rs, cs);
                out.push((r, rs, c, cs));
                if !dense {
                    cur_c = c + cs;
                }
            },
            // Definite column, auto row: slide down within the locked col.
            (None, Some(c)) => {
                if !dense && c < cur_c {
                    cur_r += 1;
                }
                let mut r = if dense { 1 } else { cur_r };
                while !occ.fits(r, c, rs, cs) {
                    r += 1;
                }
                occ.put(r, c, rs, cs);
                out.push((r, rs, c, cs));
                if !dense {
                    cur_r = r; // cursor resumes at the item's row start
                    cur_c = c + cs;
                }
            },
            // Fully automatic.
            (None, None) => {
                let (r, c) = if dense {
                    // Reading-order rescan from the grid start; the wrap
                    // limit always accommodates the widest auto span, so a
                    // fitting cell exists — the cap is purely defensive.
                    let mut found = (1u32, 1u32);
                    'scan: for r in 1..=1_000_000u32 {
                        for c in 1..=limit as u32 {
                            if occ.fits(r, c, rs, cs) {
                                found = (r, c);
                                break 'scan;
                            }
                        }
                    }
                    found
                } else {
                    let (mut r, mut c) = (cur_r, cur_c);
                    loop {
                        if c > limit as u32 {
                            c = 1;
                            r += 1;
                            continue;
                        }
                        if occ.fits(r, c, rs, cs) {
                            break;
                        }
                        c += 1;
                    }
                    (r, c)
                };
                occ.put(r, c, rs, cs);
                out.push((r, rs, c, cs));
                if !dense {
                    cur_r = r;
                    cur_c = c + cs;
                }
            },
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Track sizing helpers
// ---------------------------------------------------------------------------

/// Builds the full track vector: explicit entries, then implicit ones
/// cycling through the `grid-auto-*` list.
fn track_vector(explicit: &[TrackSize], autos: &[TrackSize], total: usize) -> Vec<TrackSize> {
    (0..total)
        .map(|i| {
            explicit.get(i).copied().unwrap_or_else(|| {
                autos[(i - explicit.len().min(i)) % autos.len()]
            })
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
fn size_axis(kinds: &[TrackSize], gap: f32, available: Option<f32>, contribs: &[Contrib]) -> Vec<f32> {
    let n = kinds.len();
    // With an indefinite axis size, `fr` tracks have nothing to flex into
    // and behave like `auto` (documented) — they take contributions too.
    let fr_is_intrinsic = available.is_none();
    let mut base: Vec<f32> = kinds
        .iter()
        .map(|k| match *k {
            TrackSize::Px(v) => v.max(0.0),
            _ => 0.0,
        })
        .collect();

    // Intrinsic contributions.
    for c in contribs {
        let end = (c.start + c.span).min(n);
        let intr: Vec<usize> = (c.start..end)
            .filter(|&t| {
                kinds[t].is_intrinsic()
                    || (fr_is_intrinsic && matches!(kinds[t], TrackSize::Fr(_)))
            })
            .collect();
        match intr.len() {
            0 => {},
            1 => base[intr[0]] = base[intr[0]].max(c.outer),
            _ => {
                let share = c.outer / intr.len() as f32;
                for t in intr {
                    base[t] = base[t].max(share);
                }
            },
        }
    }

    // fr distribution over the space remaining after non-fr tracks.
    if let (Some(avail), true) = (available, kinds.iter().any(|k| k.is_fr())) {
        let gaps = gap * n.saturating_sub(1) as f32;
        let non_fr: f32 = base
            .iter()
            .zip(kinds)
            .filter(|(_, k)| !k.is_fr())
            .map(|(v, _)| *v)
            .sum();
        let leftover = avail - gaps - non_fr;
        if leftover > 0.0 {
            let sum_w: f32 = kinds.iter().map(|k| match *k {
                TrackSize::Fr(w) => w,
                _ => 0.0,
            }).sum();
            if sum_w > 0.0 {
                for (t, k) in kinds.iter().enumerate() {
                    if let TrackSize::Fr(w) = *k {
                        base[t] += leftover * w / sum_w;
                    }
                }
            }
        }
    }
    base
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
        },
    }
}

/// Outer (margin-box) height of a LAID OUT item.
fn outer_h(it: &LayoutBox) -> f32 {
    // border_box.h already includes padding/border — only margins add on.
    it.border_box.h + it.margin.top.unwrap_or(0.0) + it.margin.bottom.unwrap_or(0.0)
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
    let spec = GridSpec::of(container, font_px, content_w);

    // --- 1. placement ---------------------------------------------------
    let raw: Vec<Placement> = items
        .iter()
        .map(|it| parse_placement(&it.style, spec.cols.len(), spec.rows.len()))
        .collect();
    let placed: Vec<Placement> = if spec.flow_column {
        // Run the row algorithm on transposed specs, transpose back.
        let swapped: Vec<Placement> = raw
            .iter()
            .map(|p| Placement {
                row: p.col,
                row_span: p.col_span,
                col: p.row,
                col_span: p.row_span,
            })
            .collect();
        // Transposed: the minor axis of the swapped space is the ROW axis.
        place_row_flow(&swapped, spec.flow_dense, spec.rows.len())
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
        place_row_flow(&raw, spec.flow_dense, spec.cols.len())
            .into_iter()
            .map(|(maj, msp, min, xsp)| Placement {
                row: Some(maj),
                row_span: msp,
                col: Some(min),
                col_span: xsp,
            })
            .collect()
    };

    // --- 2. track extents (implicit tracks wherever items landed) -------
    let n_cols = placed
        .iter()
        .map(|p| (p.col.unwrap_or(1) + p.col_span - 1) as usize)
        .fold(spec.cols.len(), usize::max);
    let n_rows = placed
        .iter()
        .map(|p| (p.row.unwrap_or(1) + p.row_span - 1) as usize)
        .fold(spec.rows.len(), usize::max);
    if n_cols == 0 || n_rows == 0 {
        // No explicit tracks and nothing placed: degenerate grid.
        return 0.0;
    }
    let col_kinds = track_vector(&spec.cols, &spec.auto_cols, n_cols);
    let row_kinds = track_vector(&spec.rows, &spec.auto_rows, n_rows);

    // --- 3. column sizing -----------------------------------------------
    let col_contribs: Vec<Contrib> = items
        .iter()
        .zip(&placed)
        .map(|(it, p)| {
            let specified =
                values::resolve_extent(&it.style, "width", it.font_size, content_w);
            let inner = match specified {
                MaybeLen::Px(w) => w.max(0.0),
                MaybeLen::Auto => crate::flex::max_content_width(it, fonts),
            };
            let m = values::resolve_margins(&it.style, it.font_size, content_w);
            let pad = values::resolve_padding(&it.style, it.font_size, content_w);
            let bor = values::resolve_borders(&it.style, it.font_size);
            Contrib {
                start: (p.col.unwrap_or(1) - 1) as usize,
                span: p.col_span.max(1) as usize,
                outer: inner + m.left.unwrap_or(0.0) + m.right.unwrap_or(0.0)
                    + pad.left + pad.right + bor.left + bor.right,
            }
        })
        .collect();
    let col_sizes = size_axis(&col_kinds, spec.col_gap, Some(content_w), &col_contribs);

    // Column positions (with justify-content distribution).
    let cols_sum: f32 = col_sizes.iter().sum();
    let col_gaps_total = spec.col_gap * (n_cols.saturating_sub(1)) as f32;
    let (jc_lead, jc_between) =
        pack_pair(spec.justify_content, content_w - cols_sum - col_gaps_total, n_cols);
    let mut col_pos = Vec::with_capacity(n_cols);
    let mut cursor = content_x + jc_lead;
    for sz in &col_sizes {
        col_pos.push(cursor);
        cursor += sz + spec.col_gap + jc_between;
    }

    // Area width spanned by an item's columns (incl. internal gaps).
    let area_w_of = |p: &Placement| -> f32 {
        let s = (p.col.unwrap_or(1) - 1) as usize;
        let e = (s + p.col_span.max(1) as usize).min(n_cols);
        col_sizes[s..e].iter().sum::<f32>()
            + spec.col_gap * (e - s - 1) as f32
    };

    // --- 4. lay out every item at its final width, measure heights ------
    for (i, it) in items.iter_mut().enumerate() {
        let p = &placed[i];
        let area_w = area_w_of(p).max(0.0);
        let width_auto = matches!(
            values::resolve_extent(&it.style, "width", it.font_size, area_w),
            MaybeLen::Auto
        );
        it.margin = values::resolve_margins(&it.style, it.font_size, area_w);
        it.padding = values::resolve_padding(&it.style, it.font_size, area_w);
        it.border = values::resolve_borders(&it.style, it.font_size);
        let ml = it.margin.left.unwrap_or(0.0);
        let mr = it.margin.right.unwrap_or(0.0);
        let avail_content = (area_w - ml - mr - h_chrome(it)).max(0.0);
        let justify = container_self_align(it, "justify-self").unwrap_or(spec.justify_items);
        let stretchable = width_auto
            && !margin_is_auto(&it.style, "left")
            && !margin_is_auto(&it.style, "right");
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
        .map(|(it, p)| Contrib {
            start: (p.row.unwrap_or(1) - 1) as usize,
            span: p.row_span.max(1) as usize,
            outer: outer_h(it),
        })
        .collect();
    let mut row_sizes = size_axis(&row_kinds, spec.row_gap, definite_height, &row_contribs);

    // align-content: stretch grows AUTO rows into the spare definite height.
    if let Some(def_h) = definite_height {
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
        let ax = col_pos[ci];
        let ay = row_pos[ri];
        let aw = col_pos[ce - 1] + col_sizes[ce - 1] - ax;
        let ah = row_pos[re - 1] + row_sizes[re - 1] - ay;

        // Vertical stretch: grow the item to fill the area (materialize the
        // height and re-flow so children see the final block size).
        let align = container_self_align(it, "align-self").unwrap_or(spec.align_items);
        if align == ItemAlign::Stretch
            && height_is_auto(it)
            && !margin_is_auto(&it.style, "top")
            && !margin_is_auto(&it.style, "bottom")
        {
            let target_h = (ah - it.margin.top.unwrap_or(0.0)
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
        let justify = container_self_align(it, "justify-self").unwrap_or(spec.justify_items);
        let m = values::resolve_margins(&it.style, it.font_size, aw.max(0.0));
        let ml = m.left.unwrap_or(0.0);
        let mr = m.right.unwrap_or(0.0);
        let bw = it.border_box.w;
        // border_box already contains padding+border, so OUTER size here is
        // margins + border_box — no extra chrome term.
        let bx = ax + match justify {
            ItemAlign::End => (aw - mr - bw).max(0.0),
            ItemAlign::Center => (ml + (aw - ml - mr - bw) / 2.0).max(0.0),
            _ => ml, // Start; Stretch already fills via materialized width
        };
        let mt = m.top.unwrap_or(0.0);
        let mb = m.bottom.unwrap_or(0.0);
        let bh = it.border_box.h;
        let by = ay + match align {
            ItemAlign::End => (ah - mb - bh).max(0.0),
            ItemAlign::Center => (mt + (ah - mt - mb - bh) / 2.0).max(0.0),
            _ => mt,
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
        layout_grid(items, container, 16.0, 0.0, 0.0, content_w, definite_h, &FontSource::fallback())
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
        let h = lay(&mut items, &cs(&[
            ("grid-template-columns", "[start] 100px [mid] 200px [end]"),
            ("grid-template-rows", "50px 80px"),
            ("row-gap", "10px"),
            ("column-gap", "20px"),
        ]), 320.0, None);
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
        let mut items = grid_fixture(2, "grid-template-columns:100px 1fr; column-gap:50px", "height:20px");
        assert_eq!(items.len(), 2);
        lay(&mut items, &cs(&[
            ("grid-template-columns", "100px 1fr"),
            ("column-gap", "50px"),
        ]), 400.0, None);
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 150.0, 0.0, 250.0, 20.0));

        // Pure fr: weighted equal-share of the whole content width.
        let mut items = grid_fixture(2, "grid-template-columns:1fr 3fr", "height:20px");
        lay(&mut items, &cs(&[("grid-template-columns", "1fr 3fr")]), 400.0, None);
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 100.0, 0.0, 300.0, 20.0));
    }

    #[test]
    fn span_keyword_covers_two_columns_and_wraps_next_item() {
        let mut items = grid_fixture(2, "grid-template-columns:100px 200px", "height:20px");
        // First item spans both columns; second wraps to row 2.
        items[0].style.set("grid-column", "span 2");
        items[0].style.set("height", "30px");
        let h = lay(&mut items, &cs(&[("grid-template-columns", "100px 200px")]), 300.0, None);
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
        lay(&mut items, &cs(&[("grid-template-columns", "100px 200px")]), 300.0, None);
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
        lay(&mut items, &cs(&[
            ("grid-template-columns", "100px 200px"),
            ("grid-auto-columns", "30px 10px"),
        ]), 340.0, None);
        // Row 2 is implicit (auto): sized by the items' 40px heights.
        assert!(rect_eq(items[2].border_box, 0.0, 40.0, 100.0, 40.0));
        // Implicit columns 3 (30px) and 4 (10px) precede item d's cell.
        assert!(rect_eq(items[3].border_box, 330.0, 40.0, 10.0, 40.0));
    }

    #[test]
    fn dense_backflow_differs_from_sparse_cursor() {
        // b auto → (1,1); c locked to col 1 → (2,1);
        // d locked to col 2: sparse cursor keeps it at (2,2),
        // dense rescans and backfills (1,2).
        let mut sparse = grid_fixture(3, "grid-template-columns:50px 50px", "height:20px");
        sparse[1].style.set("grid-column", "1");
        sparse[2].style.set("grid-column", "2");
        lay(&mut sparse, &cs(&[("grid-template-columns", "50px 50px")]), 100.0, None);
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
        lay(&mut dense, &cs(&[
            ("grid-auto-flow", "dense"),
            ("grid-template-columns", "50px 50px"),
        ]), 100.0, None);
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
        let h = lay(&mut items, &cs(&[
            ("grid-template-columns", "200px"),
            ("grid-template-rows", "100px"),
            ("justify-items", "center"),
            ("align-items", "center"),
        ]), 200.0, None);
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
        lay(&mut items, &cs(&[
            ("grid-template-columns", "200px"),
            ("grid-template-rows", "100px"),
        ]), 200.0, None);
        assert!(rect_eq(items[0].border_box, 150.0, 0.0, 50.0, 20.0));
    }

    #[test]
    fn stretch_fills_area_through_padding() {
        let mut items = grid_fixture(
            1,
            "grid-template-columns:100px; grid-template-rows:60px",
            "padding:10px",
        );
        let h = lay(&mut items, &cs(&[
            ("grid-template-columns", "100px"),
            ("grid-template-rows", "60px"),
        ]), 100.0, None);
        assert!(close(h, 60.0));
        // Horizontal stretch laid out at 80px content; vertical stretch
        // re-flowed to 40px content inside the 60px row.
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 60.0));
        assert!(rect_eq(items[0].content_box, 10.0, 10.0, 80.0, 40.0));
    }

    #[test]
    fn percentage_tracks_resolve_against_content_width() {
        let mut items = grid_fixture(2, "grid-template-columns:50% 100px", "height:20px");
        lay(&mut items, &cs(&[("grid-template-columns", "50% 100px")]), 400.0, None);
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 200.0, 20.0));
        assert!(rect_eq(items[1].border_box, 200.0, 0.0, 100.0, 20.0));
    }

    #[test]
    fn repeat_fixed_count_and_auto_fill() {
        // repeat(<integer>, …) expands verbatim.
        let mut items = grid_fixture(2, "grid-template-columns:repeat(2, 100px)", "height:20px");
        lay(&mut items, &cs(&[("grid-template-columns", "repeat(2, 100px)")]), 400.0, None);
        assert!(rect_eq(items[0].border_box, 0.0, 0.0, 100.0, 20.0));
        assert!(rect_eq(items[1].border_box, 100.0, 0.0, 100.0, 20.0));

        // auto-fill: floor((400 + 0) / 100) = 4 definite tracks.
        let mut items = grid_fixture(4, "grid-template-columns:repeat(auto-fill, 100px)", "height:20px");
        assert_eq!(items.len(), 4);
        lay(&mut items, &cs(&[("grid-template-columns", "repeat(auto-fill, 100px)")]), 400.0, None);
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
        lay(&mut items, &cs(&[
            ("grid-template-columns", "repeat(2, 50px)"),
            ("justify-content", "space-between"),
        ]), 200.0, None);
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
        let h = lay(&mut items, &cs(&[
            ("grid-template-columns", "100px"),
            ("grid-template-rows", "20px"),
            ("align-content", "center"),
        ]), 100.0, Some(100.0));
        assert!(close(h, 20.0)); // natural track extent is returned
        // With center alignment the row shifts down by (100−20)/2.
        assert!(close(items[0].border_box.y, 40.0));
    }

    // --- parser units -----------------------------------------------------

    #[test]
    fn track_size_parsing_forms() {
        assert_eq!(parse_track_size("1fr", 16.0, 400.0), TrackSize::Fr(1.0));
        assert_eq!(parse_track_size("2.5FR", 16.0, 400.0), TrackSize::Fr(2.5));
        assert_eq!(parse_track_size("auto", 16.0, 400.0), TrackSize::Auto);
        assert_eq!(parse_track_size("min-content", 16.0, 400.0), TrackSize::Content);
        assert_eq!(parse_track_size("max-content", 16.0, 400.0), TrackSize::Content);
        assert_eq!(parse_track_size("25px", 16.0, 400.0), TrackSize::Px(25.0));
        assert_eq!(parse_track_size("50%", 16.0, 400.0), TrackSize::Px(200.0));
        assert_eq!(parse_track_size("nonsense", 16.0, 400.0), TrackSize::Auto);
    }

    #[test]
    fn top_level_tokens_keep_repeat_groups_together() {
        assert_eq!(
            top_tokens("repeat(2, 100px) 50px"),
            vec!["repeat(2, 100px)".to_string(), "50px".to_string()]
        );
        assert_eq!(top_tokens("[a b] 1fr"), vec!["[a".to_string(), "b]".to_string(), "1fr".to_string()]);
    }

    #[test]
    fn slot_combination_resolves_negatives_before_ordering() {
        let s = |l: Option<i32>, sp: u32| LineSpec { line: l, span: sp };
        // 1 / -1 over 2 explicit tracks: -1 → line 3, ordered pair (1,3).
        assert_eq!(combine_slots(s(Some(1), 1), s(Some(-1), 1), 2), (Some(1), 2));
        // -2 / -1 → lines 2..3 → starts at 2, span 1.
        assert_eq!(combine_slots(s(Some(-2), 1), s(Some(-1), 1), 2), (Some(2), 1));
        // Swapped ends order themselves after resolution.
        assert_eq!(combine_slots(s(Some(-1), 1), s(Some(1), 1), 2), (Some(1), 2));
        // Span-only both sides take the widest span.
        assert_eq!(combine_slots(s(None, 2), s(None, 3), 0), (None, 3));
        // Start auto + end line 3 with span 2 → start at line 1.
        assert_eq!(combine_slots(s(None, 2), s(Some(3), 1), 0), (Some(1), 2));
    }

    #[test]
    fn implicit_tracks_cycle_the_auto_list() {
        let kinds = track_vector(&[], &[TrackSize::Px(30.0), TrackSize::Px(10.0)], 3);
        assert_eq!(kinds, vec![TrackSize::Px(30.0), TrackSize::Px(10.0), TrackSize::Px(30.0)]);
        let explicit_only = track_vector(&[TrackSize::Auto], &[TrackSize::Px(5.0)], 1);
        assert_eq!(explicit_only, vec![TrackSize::Auto]);
    }
}
