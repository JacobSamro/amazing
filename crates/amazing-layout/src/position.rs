//! Post-normal-flow positioning (CSS Position 3 / CSS2 §9.3, §10.3.7).
//!
//! Box construction removes absolute/fixed descendants from their parent's
//! formatting context. After normal layout establishes geometry, this module
//! applies relative offsets and lays out every out-of-flow subtree against the
//! nearest positioned padding box (or the viewport for fixed positioning).

use crate::block::{layout_block, layout_block_with_text};
use crate::font::FontSource;
use crate::position_constraints::Axis;
use crate::values;
use crate::resolved_geometry::{ResolvedPosition, InsetOverconstraint};
use crate::{Flow, LayoutBox, Rect};

#[derive(Clone, Copy)]
struct ContainingBlock {
    rect: Rect,
    rtl: bool,
}

fn is_rtl(box_: &LayoutBox) -> bool {
    box_.style
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"))
}

#[derive(Clone, Copy)]
struct StaticAnchor {
    x: f32,
    y: f32,
    right: bool,
}
use amazing_text::TextSystem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionKind {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

fn position_kind(box_: &LayoutBox) -> PositionKind {
    match box_
        .style
        .get("position")
        .unwrap_or("static")
        .to_ascii_lowercase()
        .as_str()
    {
        "relative" => PositionKind::Relative,
        "absolute" => PositionKind::Absolute,
        "fixed" => PositionKind::Fixed,
        "sticky" => PositionKind::Sticky,
        _ => PositionKind::Static,
    }
}

pub(crate) fn layout_positioned(
    root: &mut LayoutBox,
    viewport_width: f32,
    viewport_height: f32,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
) {
    clear_resolved_positions(root);
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_width.max(0.0),
        h: viewport_height.max(0.0),
    };
    let viewport = ContainingBlock {
        rect: viewport,
        rtl: root
            .root_font_metric_style
            .as_ref()
            .map(|style| style.direction == amazing_text::Direction::RightToLeft)
            .unwrap_or_else(|| is_rtl(root)),
    };
    position_subtree(root, viewport, viewport, true, viewport, viewport, fonts, text);
    crate::resolved_geometry::capture_geometry_snapshot(root);
}

fn clear_resolved_positions(box_: &mut LayoutBox) {
    box_.resolved_position = None;
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &mut box_.flow {
        for child in children { clear_resolved_positions(child); }
    }
    for child in &mut box_.positioned { clear_resolved_positions(child); }
}

fn position_subtree(
    box_: &mut LayoutBox,
    viewport: ContainingBlock,
    normal_containing_block: ContainingBlock,
    normal_cb_height_definite: bool,
    inherited_positioned_cb: ContainingBlock,
    inherited_fixed_cb: ContainingBlock,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
) {
    if box_.resolved_position.is_none() {
        box_.resolved_position = Some(ResolvedPosition::normal(normal_containing_block.rect));
    }
    if position_kind(box_) == PositionKind::Relative {
        let (dx, dy) = relative_offset(
            box_,
            normal_containing_block,
            normal_cb_height_definite,
            text,
        );
        shift_box(box_, dx, dy);
    }

    let own_padding_box = ContainingBlock {
        rect: padding_box(box_),
        rtl: is_rtl(box_),
    };
    let own_content_box = ContainingBlock {
        rect: box_.content_box,
        rtl: is_rtl(box_),
    };
    let own_height_definite = box_.style.get("height").is_some_and(|height| {
        !height.eq_ignore_ascii_case("auto") && (!height.contains('%') || normal_cb_height_definite)
    });
    let contained = crate::containment::independent_context(&box_.style);
    let descendant_fixed_cb = if contained { own_padding_box } else { inherited_fixed_cb };
    let descendant_positioned_cb = match position_kind(box_) {
        PositionKind::Relative
        | PositionKind::Absolute
        | PositionKind::Fixed
        | PositionKind::Sticky => own_padding_box,
        PositionKind::Static if contained => own_padding_box,
        PositionKind::Static => inherited_positioned_cb,
    };
    // Grid §10.1: when this grid container generates the abspos containing
    // block, each child's containing block is its grid area (padding box for
    // an auto-placed axis).
    let grid_generates_cb = matches!(box_.display, crate::Display::Grid)
        && matches!(position_kind(box_),
            PositionKind::Relative | PositionKind::Absolute | PositionKind::Fixed)
            || (matches!(box_.display, crate::Display::Grid) && contained);

    // Capture normal-flow static slots before relative offsets move siblings.
    let anchors: Vec<StaticAnchor> = box_
        .positioned
        .iter()
        .map(|child| static_anchor(box_, child))
        .collect();
    match &mut box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                position_subtree(
                    child,
                    viewport,
                    own_content_box,
                    own_height_definite,
                    descendant_positioned_cb,
                    descendant_fixed_cb,
                    fonts,
                    text,
                );
            }
        }
        Flow::Inlines(_) | Flow::Empty => {}
    }

    let mut positioned = std::mem::take(&mut box_.positioned);
    for (child, static_anchor) in positioned.iter_mut().zip(anchors) {
        if child.outside_marker {
            let width = match &child.flow {
                Flow::Inlines(items) => items
                    .iter()
                    .map(|item| match item {
                        crate::InlineNode::Text {
                            text, font_size, ..
                        } => fonts.measure(text, *font_size),
                        crate::InlineNode::Replaced { size, .. } => size.width,
                        _ => 0.0,
                    })
                    .sum::<f32>(),
                _ => 0.0,
            }
            .max(1.0);
            child.style.set("width", &format!("{width}px"));
            layout_child(child, 0.0, 0.0, width, fonts, text);
            let x = box_.content_box.x - child.border_box.w;
            let y = box_.content_box.y;
            shift_box(child, x - child.border_box.x, y - child.border_box.y);
            child.resolved_position = Some(ResolvedPosition::normal(box_.content_box));
            continue;
        }
        if child.inline_atomic {
            position_subtree(
                child,
                viewport,
                own_content_box,
                own_height_definite,
                descendant_positioned_cb,
                descendant_fixed_cb,
                fonts,
                text,
            );
            continue;
        }
        let containing_block = if position_kind(child) == PositionKind::Fixed {
            descendant_fixed_cb
        } else if grid_generates_cb {
            grid_area_containing_block(box_, child, descendant_positioned_cb)
        } else {
            descendant_positioned_cb
        };
        let units = font_units(child, text);
        let specified_margins =
            values::resolve_margins_with_units(&child.style, units, containing_block.rect.w);
        // Used sizes are temporary inputs to normal layout, not computed style.
        let computed_style = child.style.clone();
        prepare_sizes(child, containing_block.rect, static_anchor, fonts, text, grid_generates_cb);
        layout_child(
            child,
            containing_block.rect.x,
            containing_block.rect.y,
            containing_block.rect.w,
            fonts,
            text,
        );
        child.margin = specified_margins;
        let static_anchor = if matches!(box_.display, crate::Display::Grid) {
            let (x, y) =
                grid_child_static_anchor(box_, child, specified_margins, grid_generates_cb);
            StaticAnchor {
                x: x - specified_margins.left.unwrap_or(0.0),
                y: y - specified_margins.top.unwrap_or(0.0),
                right: false,
            }
        } else {
            static_anchor
        };
        let (x, y) = absolute_origin(
            child,
            containing_block,
            static_anchor,
            specified_margins,
            &computed_style,
            text,
        );
        shift_box(child, x - child.border_box.x, y - child.border_box.y);
        position_subtree(
            child,
            viewport,
            containing_block,
            true,
            ContainingBlock {
                rect: padding_box(child),
                rtl: is_rtl(child),
            },
            descendant_fixed_cb,
            fonts,
            text,
        );
        child.style = computed_style;
    }
    box_.positioned = positioned;
}

fn layout_child(
    child: &mut LayoutBox,
    x: f32,
    y: f32,
    width: f32,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
) {
    if let Some(text) = text {
        layout_block_with_text(child, x, y, width, fonts, text);
    } else {
        layout_block(child, x, y, width, fonts);
    }
}

fn font_units(box_: &LayoutBox, text: Option<&dyn TextSystem>) -> values::FontRelativeUnits {
    values::FontRelativeUnits::from_text_system(
        text,
        box_.font_metric_style.as_ref(),
        box_.root_font_metric_style.as_ref(),
        box_.font_size,
    ).with_line_heights(box_.line_height, box_.root_line_height)
}

fn padding_box(box_: &LayoutBox) -> Rect {
    Rect {
        x: box_.content_box.x - box_.padding.left,
        y: box_.content_box.y - box_.padding.top,
        w: box_.content_box.w + box_.padding.left + box_.padding.right,
        h: box_.content_box.h + box_.padding.top + box_.padding.bottom,
    }
}

fn resolve_inset(
    box_: &LayoutBox,
    property: &str,
    base: f32,
    text: Option<&dyn TextSystem>,
) -> Option<f32> {
    values::style_length_percentage_with_units(&box_.style, property, font_units(box_, text))?
        .resolve(Some(base))
}

fn relative_offset(
    box_: &mut LayoutBox,
    containing_block: ContainingBlock,
    cb_height_definite: bool,
    text: Option<&dyn TextSystem>,
) -> (f32, f32) {
    let left = resolve_inset(box_, "left", containing_block.rect.w, text);
    let right = resolve_inset(box_, "right", containing_block.rect.w, text);
    let vertical = |property| {
        let length = values::style_length_percentage_with_units(
            &box_.style,
            property,
            font_units(box_, text),
        )?;
        if length.has_percentage && !cb_height_definite {
            return None;
        }
        length.resolve(Some(containing_block.rect.h))
    };
    let top = vertical("top");
    let bottom = vertical("bottom");
    let rtl = containing_block.rtl;
    let dx = if rtl {
        right.map(|value| -value).or(left).unwrap_or(0.0)
    } else {
        left.or_else(|| right.map(|value| -value)).unwrap_or(0.0)
    };
    let dy = top.or_else(|| bottom.map(|value| -value)).unwrap_or(0.0);
    box_.resolved_position = Some(ResolvedPosition {
        snapshot: None,
        containing_block: containing_block.rect,
        insets: values::SidesM { left: Some(dx), right: Some(-dx), top: Some(dy), bottom: Some(-dy) },
        overconstrained: InsetOverconstraint {
            left: rtl && left.is_some() && right.is_some(),
            right: !rtl && left.is_some() && right.is_some(),
            top: false, bottom: top.is_some() && bottom.is_some(),
        },
    });
    (dx, dy)
}

fn prepare_sizes(
    box_: &mut LayoutBox,
    cb: Rect,
    anchor: StaticAnchor,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
    grid_area: bool,
) {
    let units = font_units(box_, text);
    let margins = values::resolve_margins_with_units(&box_.style, units, cb.w);
    let padding = values::resolve_padding_with_units(&box_.style, units, cb.w);
    let border = values::resolve_borders(&box_.style, box_.font_size);
    let horizontal = padding.left + padding.right + border.left + border.right;
    let vertical = padding.top + padding.bottom + border.top + border.bottom;
    let left = resolve_inset(box_, "left", cb.w, text);
    let right = resolve_inset(box_, "right", cb.w, text);
    let top = resolve_inset(box_, "top", cb.h, text);
    let bottom = resolve_inset(box_, "bottom", cb.h, text);
    box_.containing_block_height = Some(cb.h);
    box_.available_height = Some(cb.h);
    let replaced = box_.replaced_size.is_some() || box_.svg_sizing.is_some();
    let border_sizing = box_.style.get("box-sizing") == Some("border-box");
    if box_.style.get("width").is_none_or(|value| value == "auto")
        && (left.is_none() || right.is_none())
        && box_.positioned.iter().any(|child| child.inline_atomic)
    {
        crate::block::layout_atomic_inlines(
            box_,
            0.0,
            0.0,
            (cb.w - left.unwrap_or(0.0) - right.unwrap_or(0.0) - horizontal).max(0.0),
            fonts,
            text,
        );
    }

    // Normal block layout consumes content-box lengths. Resolve percentages
    // against the actual containing block before adapting border-box lengths.
    if border_sizing && !box_.table_wrapper {
        for (property, extent, chrome) in [
            ("width", cb.w, horizontal),
            ("min-width", cb.w, horizontal),
            ("max-width", cb.w, horizontal),
            ("height", cb.h, vertical),
            ("min-height", cb.h, vertical),
            ("max-height", cb.h, vertical),
        ] {
            if let Some(size) = resolve_inset(box_, property, extent, text) {
                set_used_length(box_, property, (size - chrome).max(0.0));
            }
        }
        box_.style.set("box-sizing", "content-box");
    }
    // Replaced auto width follows its natural ratio when height is specified;
    // opposing insets position it and never stretch its natural dimensions.
    if box_.style.get("width").is_none_or(|value| value == "auto")
        && let Some(natural) = box_.replaced_size
        && natural.height > 0.0
        && let Some(height) = resolve_inset(box_, "height", cb.h, text)
    {
        set_used_length(
            box_,
            "width",
            (height * natural.width / natural.height).max(0.0),
        );
    }
    for (property, extent, mut start, mut end, margin, chrome) in [
        (
            "width",
            cb.w,
            left,
            right,
            margins.left.unwrap_or(0.0) + margins.right.unwrap_or(0.0),
            horizontal,
        ),
        (
            "height",
            cb.h,
            top,
            bottom,
            margins.top.unwrap_or(0.0) + margins.bottom.unwrap_or(0.0),
            vertical,
        ),
    ] {
        let value = box_.style.get(property).unwrap_or("auto");
        let stretch = matches!(value, "stretch" | "-webkit-fill-available");
        let auto = value == "auto";
        if !stretch && (!auto || replaced) {
            continue;
        }
        // css-sizing-4 stretch-fit-sizing (stretch/positioned-non-replaced-1.html):
        // when both insets on an axis are auto, `absolute_origin` positions
        // the box at its static-position anchor (CSS2.1 §10.3.7 case 1 /
        // css-position-3 §3.5.1), unchanged by `stretch`. `stretch` does NOT
        // mean "fill the containing block regardless of where that anchor
        // falls" -- it fills from the anchor to the containing block's FAR
        // edge, i.e. exactly as far as a one-sided `left:<anchor>` (or
        // `top:<anchor>`) would let it grow. So the anchor is a real inset
        // for this size computation too, not position-only: treating it as
        // the known `start` (or `end`, right-anchored) below makes `available`
        // subtract it out, on both axes, whether the box is stretch-sized or
        // shrink-to-fit auto-sized (the latter already relied on this for
        // `width`; `height` never had an anchor fallback at all, since a
        // non-stretch auto `height` with both insets auto is content-sized
        // instead and never reaches `available` below).
        if start.is_none() && end.is_none() {
            match property {
                "width" if anchor.right => end = Some(cb.x + cb.w - anchor.x),
                "width" => start = Some(anchor.x - cb.x),
                "height" => start = Some(anchor.y - cb.y),
                _ => {}
            }
        }
        let available =
            (extent - start.unwrap_or(0.0) - end.unwrap_or(0.0) - margin - chrome).max(0.0);
        let size = if stretch || (start.is_some() && end.is_some()) {
            Some(available)
        } else if property == "width" {
            // CSS2.1 §10.3.7 cases 1/2/4 (css-position-3 §3.5.1): whenever
            // `width` is auto and `left`/`right` are NOT both definite (case
            // 1: neither set — pure static position; case 2/4: exactly one
            // set — still shrink-to-fit for the width, only the *position*
            // differs), the shrink-to-fit basis is the containing block
            // minus margin/border/padding only — never the single known
            // inset or the static-position anchor fallback above, which
            // determine position, not how much the box is allowed to
            // shrink into. Without this, an abspos child with only one
            // inset set was clamped too narrow by that inset (or, with no
            // insets at all, by the anchor fallback) instead of shrinking to
            // fit its content against the whole containing block.
            let shrink_to_fit_available = (extent - margin - chrome).max(0.0);
            // LAYOUT-31: when the containing block is a grid area, its
            // sibling *normal-flow* grid items are sized by grid.rs's own
            // fit-content formula (`crate::flex::max_content_width`), which
            // has no `text`/shaping parameter at all -- an abspos item using
            // the shaped-text measurement below instead diverges from that
            // sibling formula by the shaping-vs-advance-sum difference on
            // the same text run, breaking exact-pixel parity with a
            // reference that lays the same content out as a normal (not
            // abspos) grid item. Bisected (dev 512912d5, LFG-3 remerge) via
            // css/css-grid/abspos/{positioned,orthogonal-positioned}-grid-
            // items-{008,016}.html regressing from the shaped-text switch;
            // matching grid.rs's own unshaped formula here restores parity
            // without touching the (LAYOUT-24-owned) shaped path for every
            // other shrink-to-fit case, whose text-measurement improvement
            // this leaves untouched.
            let (minimum, preferred) =
                intrinsic_content_widths(box_, fonts, if grid_area { None } else { text });
            Some(minimum.max(shrink_to_fit_available).min(preferred))
        } else {
            None
        };
        if let Some(size) = size {
            set_used_length(box_, property, size.max(0.0));
        }
    }
}

// CSS lengths stored in ComputedStyle are unzoomed; geometry is already in
// used coordinates. Do not apply the effective zoom a second time on relayout.
fn set_used_length(box_: &mut LayoutBox, property: &str, value: f32) {
    let css_px = value / box_.style.effective_zoom() as f32;
    box_.style.set(property, &format!("{css_px}px"));
}

fn intrinsic_content_widths(
    box_: &LayoutBox,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
) -> (f32, f32) {
    if let (Flow::Inlines(items), Some(text)) = (&box_.flow, text)
        && let Some(pair) = crate::inline::intrinsic_widths_shaped(
            items,
            box_.font_metric_style
                .as_ref()
                .map(|style| style.direction)
                .unwrap_or_default(),
            text,
        )
    {
        return pair;
    }
    let padding = values::resolve_padding(&box_.style, box_.font_size, 0.0);
    let border = values::resolve_borders(&box_.style, box_.font_size);
    let chrome = padding.left + padding.right + border.left + border.right;
    (
        (shaped_intrinsic_width(box_, fonts, text, true) - chrome).max(0.0),
        (shaped_intrinsic_width(box_, fonts, text, false) - chrome).max(0.0),
    )
}

/// Like `crate::flex::min_content_width`/`max_content_width`, but keeps
/// descending through generic block-flow ancestors (not just when `box_`
/// itself is a table wrapper) so a `Role::Table` box anywhere in the subtree
/// is measured with real shaped text (LAYOUT-24). `flex::min/max_content_width`
/// always measures a table with `table::intrinsic`'s hardcoded `text: None`
/// (fallback glyph-width heuristics), which can diverge sharply from that
/// same table's own live `constraints()` call inside `table::layout` (which
/// always receives real `text`); a shrink-to-fit box (e.g. an abspos `<div>`
/// wrapping a `<table>`, directly or nested under other generic blocks) sized
/// from the fallback estimate alone can resolve narrower than the table
/// needs, squeezing its columns below their true max-content width once laid
/// out for real. Deliberately narrow: only the `Role::Table` leaf gets real
/// text here (an inline run's own top-level shaped check, in
/// `intrinsic_content_widths` above, is unaffected and unchanged) — an
/// earlier version of this fix also shaped nested inline runs and traded the
/// table regression for new off-by-a-few-px wrapping regressions elsewhere in
/// the corpus, so that part was dropped. Flex and grid containers are left to
/// the original, unchanged `flex`/`grid` functions (out of this fix's scope;
/// text-accuracy there is layout-flexgrid's concern, not amazing-layout's).
fn shaped_intrinsic_width(
    box_: &LayoutBox,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
    minimum: bool,
) -> f32 {
    if crate::table_tree::role(box_) == crate::table_tree::Role::Table {
        let (mn, mx) = crate::table::intrinsic_with_text(box_, fonts, text);
        return if minimum { mn } else { mx };
    }
    if let Flow::Blocks(children) = &box_.flow {
        return children
            .iter()
            .map(|child| {
                let margin = values::resolve_margins(&child.style, child.font_size, 0.0);
                shaped_intrinsic_width(child, fonts, text, minimum)
                    + margin.left.unwrap_or(0.0)
                    + margin.right.unwrap_or(0.0)
            })
            .fold(0.0f32, f32::max);
    }
    // A plain (non-table) inline-containing leaf found while descending: an
    // unstyled auto-sized box's own contribution is exactly its shaped-text
    // content width plus its chrome, same as `intrinsic_content_widths`'s
    // top-level `Flow::Inlines` special case already applies to `box_`
    // itself. Guarded to boxes with no width/min-width/max-width override so
    // this never has to re-implement `intrinsic_width_constraints`' clamping
    // -- those fall through unchanged to `flex::min/max_content_width`. A
    // broader version of this (shaping every inline run unconditionally) was
    // tried in LAYOUT-24 and reverted: it also changed measurement for table
    // *cells'* own short inline runs reached the same way, tipping some
    // previously-fitting lines into wrapping (17 corpus regressions). The
    // guard here keeps this leaf path from firing on anything with its own
    // sizing already pinned by CSS, which is the pattern those regressions
    // shared and this fix's target case (a bare `<div>` of plain text) does
    // not.
    let auto_sized = ["width", "min-width"]
        .into_iter()
        .all(|property| box_.style.get(property).is_none_or(|v| v.eq_ignore_ascii_case("auto")))
        && box_.style.get("max-width").is_none_or(|v| v.eq_ignore_ascii_case("none"));
    if let (Flow::Inlines(items), Some(text_system)) = (&box_.flow, text)
        && auto_sized
        && let Some((mn, mx)) = crate::inline::intrinsic_widths_shaped(
            items,
            box_.font_metric_style.as_ref().map(|style| style.direction).unwrap_or_default(),
            text_system,
        )
    {
        let padding = values::resolve_padding(&box_.style, box_.font_size, 0.0);
        let border = values::resolve_borders(&box_.style, box_.font_size);
        let chrome = padding.left + padding.right + border.left + border.right;
        return chrome + if minimum { mn } else { mx };
    }
    if minimum {
        crate::flex::min_content_width(box_, fonts)
    } else {
        crate::flex::max_content_width(box_, fonts)
    }
}

fn absolute_origin(
    box_: &mut LayoutBox,
    containing_block: ContainingBlock,
    anchor: StaticAnchor,
    margins: values::SidesM,
    original_style: &amazing_css::ComputedStyle,
    text: Option<&dyn TextSystem>,
) -> (f32, f32) {
    let cb = containing_block.rect;
    let x_axis = Axis {
        extent: cb.w,
        start: resolve_inset(box_, "left", cb.w, text),
        end: resolve_inset(box_, "right", cb.w, text),
        margin_start: margins.left,
        margin_end: margins.right,
        border_size: box_.border_box.w,
        static_edge: anchor.x - cb.x,
        static_end: anchor.right,
        end_wins: containing_block.rtl,
        split_negative: false,
    };
    let y_axis = Axis {
        extent: cb.h,
        start: resolve_inset(box_, "top", cb.h, text),
        end: resolve_inset(box_, "bottom", cb.h, text),
        margin_start: margins.top,
        margin_end: margins.bottom,
        border_size: box_.border_box.h,
        static_edge: anchor.y - cb.y,
        static_end: false,
        end_wins: false,
        split_negative: true,
    };
    let x = x_axis.solve();
    let y = y_axis.solve();
    // These are the actual constraint equation's margin-edge insets, including
    // auto/static solutions. Computed losers remain independently flagged.
    let left = x.border_start - x.margin_start;
    let right = cb.w - x.border_start - box_.border_box.w - x.margin_end;
    let top = y.border_start - y.margin_start;
    let bottom = cb.h - y.border_start - box_.border_box.h - y.margin_end;
    let over = |axis: Axis, property: &str, used_start: f32, used_end: f32| {
        let both = axis.start.is_some() && axis.end.is_some()
            && axis.margin_start.is_some() && axis.margin_end.is_some();
        let fixed_size = original_style.get(property).is_some_and(|value|
            !matches!(value, "auto" | "stretch" | "-webkit-fill-available"))
            || box_.replaced_size.is_some() || box_.svg_sizing.is_some();
        // Auto opposing-inset fill is not overconstrained unless min/max
        // sizing made the actual solved equation discard an author inset.
        let discarded = axis.start.is_some_and(|value| (value-used_start).abs() > 0.001)
            || axis.end.is_some_and(|value| (value-used_end).abs() > 0.001);
        both && (fixed_size || discarded)
    };
    let over_x = over(x_axis, "width", left, right);
    let over_y = over(y_axis, "height", top, bottom);
    box_.resolved_position = Some(ResolvedPosition {
        snapshot: None,
        containing_block: cb,
        insets: values::SidesM { left: Some(left), right: Some(right), top: Some(top), bottom: Some(bottom) },
        overconstrained: InsetOverconstraint { left: over_x && x_axis.end_wins,
            right: over_x && !x_axis.end_wins, top: false, bottom: over_y },
    });
    box_.margin = values::SidesM {
        left: Some(x.margin_start),
        right: Some(x.margin_end),
        top: Some(y.margin_start),
        bottom: Some(y.margin_end),
    };
    (cb.x + x.border_start, cb.y + y.border_start)
}

fn static_anchor(parent: &LayoutBox, positioned: &LayoutBox) -> StaticAnchor {
    let target = positioned.source_order.unwrap_or(u64::MAX);
    let preceding = match &parent.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children
            .iter()
            .filter(|child| {
                child
                    .source_end_order
                    .or(child.source_order)
                    .is_some_and(|order| order < target)
                    && crate::floats::side(&child.style).is_none()
            })
            .max_by_key(|child| child.source_order.unwrap_or(0)),
        Flow::Inlines(_) | Flow::Empty => None,
    };
    let y = preceding.map_or(parent.content_box.y, |sibling| {
        sibling.border_box.y + sibling.border_box.h + sibling.margin.bottom.unwrap_or(0.0)
    });
    let vertical_parent = parent
        .font_metric_style
        .as_ref()
        .is_some_and(|style| style.writing_mode.is_vertical_flow());
    // CSS2 §10.3.7: an inline-level abspos descendant's static position is
    // that of a hypothetical zero-width inline box at this point in the
    // flow -- narrowed by any floats already placed at this block position,
    // and shifted within that narrowed band by `text-align` the same way
    // real inline content is (matches `inline_alignment::apply`'s shift
    // formula for a zero-width run). A block-level descendant ignores both
    // (its static position is always the containing block's content edge);
    // physical mapping for vertical writing modes isn't handled by either
    // path yet (pre-existing limitation, unchanged here).
    if positioned.originally_inline_level && matches!(parent.flow, Flow::Blocks(_)) && !vertical_parent {
        let (left, right_edge) = float_narrowed_band(parent, y);
        let reversed = is_rtl(parent);
        let fraction = text_align_fraction(parent.style.get("text-align"), reversed);
        return StaticAnchor {
            x: left + fraction * (right_edge - left),
            y,
            right: reversed,
        };
    }
    // Block flow currently exposes horizontal physical block origins. Vertical
    // static rectangles follow that same origin until block-axis mapping exists.
    let right = is_rtl(parent) && !vertical_parent;
    StaticAnchor {
        x: parent.content_box.x + if right { parent.content_box.w } else { 0.0 },
        y,
        right,
    }
}

/// The available horizontal band at block position `y` inside `parent`'s
/// content box, narrowed by any of its floated `Flow::Blocks` children whose
/// border box spans `y` (CSS2 §9.5). Floats have no layout effect on an
/// actual flex/grid item (LAYOUT-30), so this is only ever consulted for a
/// `Flow::Blocks` parent -- the caller already gates on that.
fn float_narrowed_band(parent: &LayoutBox, y: f32) -> (f32, f32) {
    let mut left = parent.content_box.x;
    let mut right = parent.content_box.x + parent.content_box.w;
    if let Flow::Blocks(children) = &parent.flow {
        for child in children {
            let Some(side) = crate::floats::side(&child.style) else { continue };
            let r = child.border_box;
            if y < r.y || y >= r.y + r.h {
                continue;
            }
            match side {
                crate::floats::Side::Left => left = left.max(r.x + r.w),
                crate::floats::Side::Right => right = right.min(r.x),
            }
        }
    }
    (left, right.max(left))
}

/// CSS Text 3 alignment shift as a 0.0..=1.0 fraction of the available band,
/// matching `inline_alignment::apply`'s resolution for a zero-width run:
/// `left`/`right` are always physical; `start`/`end`/unset (and `justify`,
/// which has no free space to distribute for a zero-width box) resolve
/// against `reversed` (the containing block's direction).
fn text_align_fraction(value: Option<&str>, reversed: bool) -> f32 {
    let is = |s: &str| value.is_some_and(|v| v.eq_ignore_ascii_case(s));
    if is("left") {
        0.0
    } else if is("right") {
        1.0
    } else if is("center") {
        0.5
    } else if is("end") {
        if reversed { 0.0 } else { 1.0 }
    } else if reversed {
        1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy)]
enum StaticAlign {
    Start,
    Center,
    End,
}

fn grid_area_containing_block(
    parent: &LayoutBox,
    child: &LayoutBox,
    fallback: ContainingBlock,
) -> ContainingBlock {
    let Some(tracks) = parent.grid_tracks.as_ref() else {
        return fallback;
    };
    let (col, row) = tracks.area(&child.style);
    ContainingBlock {
        rect: Rect {
            x: col.map_or(fallback.rect.x, |(x, _)| x),
            w: col.map_or(fallback.rect.w, |(_, w)| w),
            y: row.map_or(fallback.rect.y, |(y, _)| y),
            h: row.map_or(fallback.rect.h, |(_, h)| h),
        },
        rtl: fallback.rtl,
    }
}

/// css-writing-modes-3: whether the block axis runs physically horizontal
/// (vertical-* / sideways-*) rather than vertical (horizontal-tb).
fn is_vertical_writing_mode(mode: &str) -> bool {
    !mode.eq_ignore_ascii_case("horizontal-tb")
}

/// Physical side the block axis starts from when it runs horizontally:
/// vertical-lr/sideways-lr progress left-to-right (block-start = physical
/// left, the minimum coordinate); vertical-rl/sideways-rl progress
/// right-to-left (block-start = physical right, the maximum coordinate).
fn block_start_is_min_x(mode: &str) -> bool {
    mode.eq_ignore_ascii_case("vertical-lr") || mode.eq_ignore_ascii_case("sideways-lr")
}

fn grid_child_static_anchor(
    parent: &LayoutBox,
    child: &LayoutBox,
    margins: values::SidesM,
    use_grid_area: bool,
) -> (f32, f32) {
    // The static position of an abspos child of a grid container is its grid
    // area when that axis has a definite `grid-placement` AND the grid
    // container is the one generating the child's abspos containing block
    // (i.e. it is itself positioned, like `grid_generates_cb`); the
    // container's content edges otherwise. A `position: static` grid
    // container does NOT establish a containing block per CSS2.1 (grid does
    // not get flexbox's unconditional special case), so its grid-placement
    // is irrelevant to an abspos descendant's static position/alignment
    // container in that case — confirmed by the pinned WPT
    // `grid-abspos-staticpos-justify-self-{002,rtl-003}` references, which
    // align entirely within the container's content box while grid-area
    // places the item away from it. (Both engines and the pinned WPT reflect
    // the grid area when the container IS the containing block; the
    // content-edge rule is the Grid 1 fallback otherwise/for unplaced
    // children.)
    let parent_rtl = parent.style.get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let child_rtl = child
        .style
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let choice = |self_property: &str, items_property: &str| {
        child
            .style
            .get(self_property)
            .filter(|value| !value.eq_ignore_ascii_case("auto"))
            .or_else(|| parent.style.get(items_property))
            .unwrap_or("normal")
    };
    // Box Alignment §5: `align-*` always targets the block axis and
    // `justify-*` the inline axis, regardless of writing mode. In a
    // horizontal-tb container the block axis is physical Y and the inline
    // axis physical X (the historical assumption below); in a vertical-* /
    // sideways-* container the axes swap physically (block = X, inline = Y).
    // `left`/`right` are only ever in `justify-*`'s grammar: they compute to
    // the true physical side when the inline axis is itself physical
    // horizontal (horizontal-tb), and to their start/end equivalent
    // otherwise (css-align-3 §8.3). They aren't in `align-*`'s grammar at
    // all, so both keywords there behave as if the whole declaration were
    // invalid — same as `start`/unset — never as distinct start vs. end.
    let parent_mode = parent.style.get("writing-mode").unwrap_or("horizontal-tb");
    let (horizontal, vertical) = if is_vertical_writing_mode(parent_mode) {
        let child_mode = child.style.get("writing-mode").unwrap_or(parent_mode);
        let x_min = block_start_is_min_x(parent_mode);
        let x_self_min = block_start_is_min_x(child_mode);
        let x = static_align(choice("align-self", "align-items"), x_min, x_self_min, LeftRight::Invalid);
        let y = static_align(choice("justify-self", "justify-items"), !parent_rtl, !child_rtl, LeftRight::Logical);
        (x, y)
    } else {
        let x = static_align(choice("justify-self", "justify-items"), !parent_rtl, !child_rtl, LeftRight::Physical);
        let y = static_align(choice("align-self", "align-items"), true, true, LeftRight::Invalid);
        (x, y)
    };
    let (col_area, row_area) = if use_grid_area {
        parent.grid_tracks.as_ref()
            .map(|tracks| tracks.area(&child.style))
            .unwrap_or((None, None))
    } else {
        (None, None)
    };
    let (cx, cw) = col_area.unwrap_or((parent.content_box.x, parent.content_box.w));
    let (cy, ch) = row_area.unwrap_or((parent.content_box.y, parent.content_box.h));
    let mut x = aligned_border_start(
        cx, cw, child.border_box.w,
        margins.left.unwrap_or(0.0), margins.right.unwrap_or(0.0), horizontal,
    );
    let mut y = aligned_border_start(
        cy, ch, child.border_box.h,
        margins.top.unwrap_or(0.0), margins.bottom.unwrap_or(0.0), vertical,
    );
    // Grid Lanes `fill-reverse`: neither axis has real tracks in the
    // waterfall direction (`area()` returns `None` there), so an unplaced
    // out-of-flow child's static position on that axis defaulted to the near
    // content edge above. Mirror it the same way real items are mirrored
    // after layout (css-grid-3 draft, open issue on the exact rule; this
    // matches the pinned WPT reference renders).
    if use_grid_area {
        if let Some(tracks) = parent.grid_tracks.as_ref() {
            if col_area.is_none() && tracks.reverse_x {
                x = cx + cw - (x - cx) - child.border_box.w;
            }
            if row_area.is_none() && tracks.reverse_y {
                y = cy + ch - (y - cy) - child.border_box.h;
            }
        }
    }
    (x, y)
}

/// How `left`/`right` resolve for the property being computed (see the
/// `grid_child_static_anchor` doc comment above): `Physical` for `justify-*`
/// on a physically-horizontal inline axis (true left/right, independent of
/// the `start`/`end` polarity), `Logical` for `justify-*` on a
/// physically-vertical inline axis (start/end equivalent), `Invalid` for
/// `align-*` (not in its grammar; both keywords fall back to `start`).
#[derive(Clone, Copy, PartialEq)]
enum LeftRight {
    Physical,
    Logical,
    Invalid,
}

/// Resolve a `justify-self`/`align-self`/`-items` keyword to a physical
/// `StaticAlign` for one axis. `physical_start_is_min`/`self_physical_start_is_min`
/// say whether that axis's logical `start` (resp. the item's own `self-start`)
/// lands on the smaller physical coordinate — already resolved by the caller
/// from the container's/item's direction and writing mode, so this function
/// only maps keywords, never re-derives polarity from `rtl`/writing mode.
fn static_align(
    value: &str,
    physical_start_is_min: bool,
    self_physical_start_is_min: bool,
    left_right: LeftRight,
) -> StaticAlign {
    let start = |min: bool| if min { StaticAlign::Start } else { StaticAlign::End };
    let end = |min: bool| if min { StaticAlign::End } else { StaticAlign::Start };
    // The safe fallback needs the abspos grid area as its alignment container.
    // Until that rectangle is carried here, use its start edge rather than
    // aligning beyond it when a safe position is requested.
    if value.split_ascii_whitespace().any(|part| part.eq_ignore_ascii_case("safe")) {
        return start(physical_start_is_min);
    }
    // <baseline-position> = [ first | last ]? baseline. Neither of these
    // fixed-size boxes (or the shrink-to-fit ones without a real line box)
    // has an actual baseline to align; a synthesized baseline falls back to
    // the box's own start edge for `baseline`/`first baseline`, and its end
    // edge for `last baseline` (css-align-3 §9, "Fallback Alignment").
    let mut words = value.split_ascii_whitespace();
    if let Some(last_word) = words.next_back() {
        if last_word.eq_ignore_ascii_case("baseline") {
            return if words.next_back().is_some_and(|w| w.eq_ignore_ascii_case("last")) {
                end(physical_start_is_min)
            } else {
                start(physical_start_is_min)
            };
        }
    }
    let keyword = value.split_ascii_whitespace().last().unwrap_or("normal").to_ascii_lowercase();
    match keyword.as_str() {
        "center" => StaticAlign::Center,
        "end" | "flex-end" => end(physical_start_is_min),
        "self-start" => start(self_physical_start_is_min),
        "self-end" => end(self_physical_start_is_min),
        "left" if left_right == LeftRight::Physical => StaticAlign::Start,
        "right" if left_right == LeftRight::Physical => StaticAlign::End,
        // Logical (justify-* on a non-horizontal inline axis): behaves like
        // the plain start/end keyword. Invalid (align-*, not in its
        // grammar): both keywords behave like the unset/`normal` default.
        "left" => start(physical_start_is_min),
        "right" if left_right == LeftRight::Logical => end(physical_start_is_min),
        _ => start(physical_start_is_min),
    }
}

fn aligned_border_start(
    container_start: f32,
    container_size: f32,
    border_size: f32,
    margin_start: f32,
    margin_end: f32,
    align: StaticAlign,
) -> f32 {
    let free = container_size - border_size - margin_start - margin_end;
    container_start
        + margin_start
        + match align {
            StaticAlign::Start => 0.0,
            StaticAlign::Center => free / 2.0,
            StaticAlign::End => free,
        }
}

fn shift_box(box_: &mut LayoutBox, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for segment in &mut box_.table_decoration.borders {
        segment.rect.x += dx;
        segment.rect.y += dy;
    }
    for hole in &mut box_.table_decoration.background_holes {
        hole.x += dx;
        hole.y += dy;
    }
    box_.border_box.x += dx;
    box_.border_box.y += dy;
    box_.content_box.x += dx;
    box_.content_box.y += dy;
    // Keep `grid_tracks` (absolute track positions used for an abspos
    // child's grid-area static position / containing block, see
    // `grid_area_containing_block`) in sync with a `position: relative`
    // offset applied to this grid container — see the matching fix in
    // `flex::translate` for the float-placement case.
    if let Some(tracks) = &mut box_.grid_tracks {
        for x in &mut tracks.col_positions { *x += dx; }
        for y in &mut tracks.row_positions { *y += dy; }
    }
    for line in &mut box_.lines {
        line.y_top += dy;
        line.baseline += dy;
        for word in &mut line.words {
            word.x += dx;
        }
        for glyph in &mut line.glyphs {
            glyph.x += dx;
        }
        for decoration in &mut line.decorations {
            decoration.x += dx;
            decoration.y += dy;
        }
        for replaced in &mut line.replaced {
            replaced.x += dx;
            replaced.y += dy;
        }
    }
    match &mut box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                shift_box(child, dx, dy);
            }
        }
        Flow::Inlines(_) | Flow::Empty => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSource;
    use amazing_dom::Dom;

    fn layout(html: &str, css: &str, width: f32, height: f32) -> (Dom, LayoutBox) {
        let dom = amazing_html::parse(html);
        let root = crate::layout_dom_document_in_viewport(
            &dom,
            css,
            width,
            Some(height),
            &FontSource::fallback(),
        );
        (dom, root)
    }

    fn find_id<'a>(box_: &'a LayoutBox, dom: &Dom, id: &str) -> Option<&'a LayoutBox> {
        if box_
            .node
            .and_then(|node| dom.attrs_of(node))
            .is_some_and(|attributes| {
                attributes
                    .iter()
                    .any(|(name, value)| name == "id" && value == id)
            })
        {
            return Some(box_);
        }
        let in_flow = match &box_.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
                children.iter().find_map(|child| find_id(child, dom, id))
            }
            Flow::Inlines(_) | Flow::Empty => None,
        };
        in_flow.or_else(|| {
            box_.positioned
                .iter()
                .find_map(|child| find_id(child, dom, id))
        })
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.01,
            "got {actual}, want {expected}"
        );
    }

    #[test]
    fn inline_static_slots_use_parent_direction_inside_float_bands() {
        for direction in ["ltr", "rtl"] {
            for (align, fraction) in [("left", 0.0), ("center", 0.5), ("right", 1.0)] {
                for side in ["left", "right"] {
                    for display in ["inline", "inline-table"] {
                        for position in ["absolute", "fixed"] {
                            let css = format!("body{{margin:0}}#parent{{position:relative;width:100px;direction:{direction};text-align:{align}}}#preceding{{height:50px}}#float{{float:{side};width:50px;height:50px}}#target{{position:{position};display:{display};width:50px;height:50px}}");
                            let (dom, root) = layout("<body><div id=parent><div id=preceding></div><div id=float></div><span id=target></span></div></body>", &css, 200.0, 200.0);
                            let target = find_id(&root, &dom, "target").unwrap();
                            let expected = if side == "left" { 50.0 } else { 0.0 }
                                + 50.0 * fraction - if direction == "rtl" { 50.0 } else { 0.0 };
                            assert!((target.border_box.x - expected).abs() < 0.01,
                                "{direction};{align};{side};{display};{position}: x={}, expected {expected}", target.border_box.x);
                            close(target.border_box.y, 50.0);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn blockified_inline_positioned_boxes_keep_centered_hypothetical_slots() {
        for position in ["absolute", "fixed"] {
            for display in ["inline", "inline-block", "inline-flex", "inline-grid", "inline-table", "ruby", "block"] {
                let css = format!("body{{margin:0}}#parent{{position:relative;width:200px;text-align:center}}#preceding{{height:20px}}#target{{position:{position};display:{display};width:10px;height:10px}}");
                let (dom, root) = layout("<body><div id=parent><div id=preceding></div><span id=target></span></div></body>", &css, 400.0, 300.0);
                let target = find_id(&root, &dom, "target").unwrap();
                let expected_x = if display == "block" { 0.0 } else { 100.0 };
                assert!((target.border_box.x - expected_x).abs() < 0.01,
                    "{display};{position}: x={}, expected {expected_x}", target.border_box.x);
                close(target.border_box.y, 20.0);
            }
        }
    }

    #[test]
    fn containment_padding_box_positions_absolute_and_fixed_descendants() {
        for contain in ["layout", "paint", "content", "strict", "size layout"] {
            for position in ["absolute", "fixed"] {
                let (dom, root) = layout(
                    "<body><div id=outer><div style='position:relative;margin:7px'><div id=inner><div id=target></div></div></div></div></body>",
                    &format!("body{{margin:0}}#outer{{contain:{contain};width:120px;height:140px;margin:30px;padding:10px;border:2px solid}}#inner{{contain:{contain};width:60px;height:70px;padding:5px;border:3px solid;margin:11px}}#target{{position:{position};left:10%;top:20%;width:10px;height:10px}}"),800.0,600.0);
                let inner = find_id(&root, &dom, "inner").unwrap();
                let target = find_id(&root, &dom, "target").unwrap();
                let cb = padding_box(inner);
                close(target.border_box.x, cb.x + cb.w * 0.1);
                close(target.border_box.y, cb.y + cb.h * 0.2);
            }
        }
    }

    #[test]
    fn relative_descendant_preserves_contained_fixed_block_but_updates_absolute_block() {
        for visibility in ["auto", "hidden"] {
            let (dom, root) = layout(
                "<body><div id=outer><div id=relative><div id=fixed></div><div id=absolute></div></div></div></body>",
                &format!("body{{margin:0}}#outer{{content-visibility:{visibility};margin:30px;width:120px;height:140px}}#relative{{position:relative;margin:15px;width:60px;height:70px}}#fixed{{position:fixed;left:0;top:0;width:10px;height:10px}}#absolute{{position:absolute;left:0;top:0;width:10px;height:10px}}"),800.0,600.0);
            let outer = find_id(&root, &dom, "outer").unwrap();
            let relative = find_id(&root, &dom, "relative").unwrap();
            let fixed = find_id(&root, &dom, "fixed").unwrap();
            let absolute = find_id(&root, &dom, "absolute").unwrap();
            close(fixed.border_box.x, outer.content_box.x);
            close(fixed.border_box.y, outer.content_box.y);
            close(absolute.border_box.x, relative.content_box.x);
            close(absolute.border_box.y, relative.content_box.y);
        }
    }

    #[test]
    fn absolute_child_does_not_size_block_parent() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=absolute></div><div id=flow></div></div></body>",
            "body{margin:0} #parent{position:relative} #absolute{position:absolute;width:50px;height:80px} #flow{height:20px}",
            800.0,
            600.0,
        );
        close(find_id(&root, &dom, "parent").unwrap().content_box.h, 20.0);
        close(find_id(&root, &dom, "flow").unwrap().border_box.y, 0.0);
    }

    #[test]
    fn relative_offset_keeps_following_sibling_in_original_slot() {
        let (dom, root) = layout(
            "<body><div id=relative></div><div id=sibling></div></body>",
            "body{margin:0} #relative{position:relative;left:25px;top:10px;height:20px} #sibling{height:20px}",
            800.0,
            600.0,
        );
        let relative = find_id(&root, &dom, "relative").unwrap();
        let sibling = find_id(&root, &dom, "sibling").unwrap();
        close(relative.border_box.x, 25.0);
        close(relative.border_box.y, 10.0);
        close(sibling.border_box.y, 20.0);
    }

    #[test]
    fn absolute_uses_nearest_positioned_padding_box_and_right_bottom() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=child></div></div></body>",
            "body{margin:0} #parent{position:relative;margin-left:100px;width:200px;height:150px;padding:10px} #child{position:absolute;right:15px;bottom:25px;width:40px;height:30px}",
            800.0,
            600.0,
        );
        let child = find_id(&root, &dom, "child").unwrap();
        close(child.border_box.x, 265.0);
        close(child.border_box.y, 115.0);
    }

    #[test]
    fn fixed_uses_viewport_and_negative_insets_are_preserved() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=fixed></div><div id=negative></div></div></body>",
            "body{margin:0} #parent{position:relative;margin:100px;width:200px;height:150px} #fixed{position:fixed;right:10px;top:20px;width:50px;height:40px} #negative{position:absolute;left:-99999999999px;width:10px;height:10px}",
            800.0,
            600.0,
        );
        let fixed = find_id(&root, &dom, "fixed").unwrap();
        let negative = find_id(&root, &dom, "negative").unwrap();
        close(fixed.border_box.x, 740.0);
        close(fixed.border_box.y, 20.0);
        assert!(negative.border_box.x < -1_000_000.0);
    }

    #[test]
    fn absolute_children_are_not_flex_or_grid_items() {
        for display in ["flex", "grid"] {
            let css = format!(
                "body{{margin:0}} #parent{{display:{display};width:200px}} #absolute{{position:absolute;width:50px;height:50px}} #flow{{width:100px;height:20px}}"
            );
            let (dom, root) = layout(
                "<body><div id=parent><div id=absolute></div><div id=flow></div></div></body>",
                &css,
                800.0,
                600.0,
            );
            let parent = find_id(&root, &dom, "parent").unwrap();
            let item_count = match &parent.flow {
                Flow::Flex(items) | Flow::Grid(items) => items.len(),
                _ => panic!("expected {display} flow"),
            };
            assert_eq!(item_count, 1);
            assert_eq!(parent.positioned.len(), 1);
        }
    }

    #[test]
    fn grid_abspos_static_position_uses_grid_area_only_when_positioned() {
        // A `position: static` grid container does not generate the abspos
        // containing block (CSS2.1 §10.1; grid does not get flexbox's
        // unconditional special case), so grid-placement is irrelevant to an
        // abspos descendant's static position/alignment container in that
        // case: it aligns within the full content box instead. Confirmed by
        // the pinned WPT `grid-abspos-staticpos-justify-self-002`/`-rtl-003`
        // references, which use plain content-box alignment for exactly this
        // static case. Only `position: relative` (etc.) uses the grid area.
        for position in ["static", "relative"] {
            let (dom, root) = layout(
                "<body><div id=grid><div id=auto></div><div id=start></div><div id=end></div><div id=self_start></div><div id=safe></div><div id=first></div><div id=unplaced></div></div></body>",
                &format!(
                    "body{{margin:0}} #grid{{display:grid;position:{position};width:100px;height:60px; \
                     grid-template-columns:3px 20px 3px;grid-template-rows:2px 30px 2px; \
                     justify-items:center;align-items:center}} \
                     #grid>div{{position:absolute;width:8px;height:6px;grid-area:2 / 2 / 3 / 3}} \
                     #grid>div#first{{grid-area:1 / 1 / 2 / 2;justify-self:start;align-self:start}} \
                     #grid>div#unplaced{{grid-area:auto;justify-self:center;align-self:center}} \
                     #start{{justify-self:normal;align-self:start}} #end{{justify-self:end;align-self:end}} \
                     #self_start{{direction:rtl;justify-self:self-start;align-self:start}} \
                     #safe{{justify-self:safe end;align-self:safe center}}"
                ),
                800.0,
                600.0,
            );
            let grid = find_id(&root, &dom, "grid").unwrap();
            let x = grid.content_box.x;
            let y = grid.content_box.y;
            if position == "relative" {
                // Grid area 2/2 is x[3,23) y[2,32): center, start, end, self-start.
                let auto = find_id(&root, &dom, "auto").unwrap();
                close(auto.border_box.x, x + 9.0);
                close(auto.border_box.y, y + 14.0);
                let start = find_id(&root, &dom, "start").unwrap();
                close(start.border_box.x, x + 3.0);
                close(start.border_box.y, y + 2.0);
                let end = find_id(&root, &dom, "end").unwrap();
                close(end.border_box.x, x + 15.0);
                close(end.border_box.y, y + 26.0);
                close(find_id(&root, &dom, "self_start").unwrap().border_box.x, x + 15.0);
                let safe = find_id(&root, &dom, "safe").unwrap();
                close(safe.border_box.x, x + 3.0);
                close(safe.border_box.y, y + 2.0);
                // Area 1/1 is x[0,3) y[0,2): start alignment clips to its origin.
                let first = find_id(&root, &dom, "first").unwrap();
                close(first.border_box.x, x);
                close(first.border_box.y, y);
            } else {
                // Full content box is 100x60: center, start, end, self-start
                // (RTL child -> end).
                let auto = find_id(&root, &dom, "auto").unwrap();
                close(auto.border_box.x, x + 46.0);
                close(auto.border_box.y, y + 27.0);
                let start = find_id(&root, &dom, "start").unwrap();
                close(start.border_box.x, x);
                close(start.border_box.y, y);
                let end = find_id(&root, &dom, "end").unwrap();
                close(end.border_box.x, x + 92.0);
                close(end.border_box.y, y + 54.0);
                close(find_id(&root, &dom, "self_start").unwrap().border_box.x, x + 92.0);
                let safe = find_id(&root, &dom, "safe").unwrap();
                close(safe.border_box.x, x);
                close(safe.border_box.y, y);
                let first = find_id(&root, &dom, "first").unwrap();
                close(first.border_box.x, x);
                close(first.border_box.y, y);
            }
            // An unplaced abspos child keeps the content-edge static position
            // regardless of the container's own position value.
            let unplaced = find_id(&root, &dom, "unplaced").unwrap();
            close(unplaced.border_box.x, x + (grid.content_box.w - 8.0) / 2.0);
            close(unplaced.border_box.y, y + (grid.content_box.h - 6.0) / 2.0);
        }
    }

    /// css/css-grid/grid-lanes/abspos/column-grid-lanes-oof-fill-reverse-001.html
    /// (pinned WPT `c2a2fce6`): three 40px items in a single `column
    /// fill-reverse` lane (natural height 140px) end up mirrored — item 1
    /// last (visually at the bottom), item 3 first (top). An out-of-flow
    /// descendant with `top`/`bottom` auto has no grid placement, so its
    /// static position falls back to the container's near content edge on
    /// the waterfall (row) axis; that fallback must be mirrored the same way
    /// real items are, landing flush with the far (bottom) edge, matching
    /// the WPT reference (`top: 120px` for a 20px-tall box in a 140px lane).
    #[test]
    fn grid_lanes_fill_reverse_mirrors_oof_static_position_on_waterfall_axis() {
        let (dom, root) = layout(
            "<body><div id=lanes><item>1</item><item>2</item><item>3</item><div id=oof></div></div></body>",
            "body{margin:0} #lanes{display:grid-lanes;grid-lanes-direction:column fill-reverse; \
             grid-template-columns:80px;gap:10px;width:200px;position:relative} \
             item{display:block;height:40px} \
             #oof{position:absolute;width:20px;height:20px;left:0px}",
            800.0,
            600.0,
        );
        let lanes = find_id(&root, &dom, "lanes").unwrap();
        let oof = find_id(&root, &dom, "oof").unwrap();
        close(lanes.content_box.h, 140.0);
        close(oof.border_box.y, lanes.content_box.y + 120.0);
    }

    #[test]
    fn absolute_descendant_of_inline_content_is_retained_out_of_flow() {
        let (dom, root) = layout(
            "<body><span>flow<b id=absolute></b></span></body>",
            "body{margin:0} #absolute{position:absolute;left:10px;top:15px;width:50px;height:50px}",
            800.0,
            600.0,
        );
        assert_eq!(root.positioned.len(), 1);
        let absolute = find_id(&root, &dom, "absolute").unwrap();
        close(absolute.border_box.x, 10.0);
        close(absolute.border_box.y, 15.0);
        assert!(root.content_box.h < 50.0);
    }

    #[test]
    fn static_absolute_child_uses_inline_block_content_origin() {
        let (dom, root) = layout(
            "<body><p>Test passes if there is a filled green square and <strong>no red</strong>.</p><div style='display:inline-block;width:100px;height:100px'></div> <div id=parent style='display:inline-block;width:100px;height:100px;background:red'><div id=absolute style='position:absolute;width:100px;height:100px;background:green'></div></div> <div style='display:inline-block;width:100px;height:100px'></div></body>",
            "",
            800.0,
            600.0,
        );
        let parent = find_id(&root, &dom, "parent").unwrap();
        let absolute = find_id(&root, &dom, "absolute").unwrap();
        assert!(
            (parent.border_box.x - parent.content_box.x).abs() < 0.001,
            "inline-block border x={}, content x={}",
            parent.border_box.x,
            parent.content_box.x
        );
        assert!(
            (absolute.border_box.x - parent.content_box.x).abs() < 0.001,
            "absolute x={}, inline-block content x={}",
            absolute.border_box.x,
            parent.content_box.x
        );
        let text = amazing_text::FallbackTextSystem::new();
        let shaped = crate::layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            "",
            800.0,
            Some(600.0),
            &text,
            &crate::NoReplacedElements,
        );
        let parent = find_id(&shaped, &dom, "parent").unwrap();
        let absolute = find_id(&shaped, &dom, "absolute").unwrap();
        assert!((absolute.border_box.x - parent.border_box.x).abs() < 0.001);
    }

    #[test]
    fn percentage_relative_and_absolute_insets_use_containing_block_axes() {
        let (dom, root) = layout(
            "<body><div id=relative><div id=absolute></div></div></body>",
            "body{margin:0;height:100px} #relative{position:relative;left:10%;top:25%;width:200px;height:100px} #absolute{position:absolute;left:50%;top:50%;width:20px;height:20px}",
            200.0,
            400.0,
        );
        let relative = find_id(&root, &dom, "relative").unwrap();
        let absolute = find_id(&root, &dom, "absolute").unwrap();
        close(relative.border_box.x, 20.0);
        close(relative.border_box.y, 25.0);
        close(absolute.border_box.x, 120.0);
        close(absolute.border_box.y, 75.0);
    }

    #[test]
    fn percentage_relative_top_with_auto_containing_height_is_auto() {
        let (dom, root) = layout(
            "<body><div id=fixed><div id=one></div></div><div id=auto><div id=two></div></div></body>",
            "body{margin:0} #fixed{height:100px} #one,#two{height:10px;position:relative;top:calc(25px + 50%)}",
            200.0,
            400.0,
        );
        let fixed = find_id(&root, &dom, "fixed").unwrap();
        let one = find_id(&root, &dom, "one").unwrap();
        let auto = find_id(&root, &dom, "auto").unwrap();
        let two = find_id(&root, &dom, "two").unwrap();
        close(one.border_box.y - fixed.content_box.y, 75.0);
        close(two.border_box.y - auto.content_box.y, 0.0);
    }

    #[test]
    fn dual_insets_stretch_auto_width_and_height() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=child></div></div></body>",
            "body{margin:0} #parent{position:relative;width:200px;height:100px} #child{position:absolute;left:10px;right:20px;top:5px;bottom:15px}",
            800.0,
            600.0,
        );
        let child = find_id(&root, &dom, "child").unwrap();
        close(child.border_box.x, 10.0);
        close(child.border_box.y, 5.0);
        close(child.border_box.w, 170.0);
        close(child.border_box.h, 80.0);
    }

    #[test]
    fn containing_block_direction_wins_over_child_direction() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=child></div></div></body>",
            "body{margin:0} #parent{position:relative;width:200px;height:100px} #child{direction:rtl;position:absolute;left:10px;right:20px;top:0;width:50px;height:10px}",
            800.0,
            600.0,
        );
        close(find_id(&root, &dom, "child").unwrap().border_box.x, 10.0);
        for (position, parent_direction, expected) in [
            ("absolute", "ltr", 10.0),
            ("absolute", "rtl", 30.0),
            ("fixed", "ltr", 130.0),
        ] {
            let (dom, root) = layout(
                "<html dir=rtl><body dir=ltr><div id=parent><div id=child></div></div></body></html>",
                &format!(
                    "body{{margin:0}} #parent{{position:relative;direction:{parent_direction};width:100px;height:100px}} #child{{position:{position};direction:ltr;left:10px;right:20px;width:50px;height:10px}}"
                ),
                200.0,
                600.0,
            );
            close(
                find_id(&root, &dom, "child").unwrap().border_box.x,
                expected,
            );
        }
    }

    #[test]
    fn auto_insets_use_static_slot_after_preceding_block() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=before></div><div id=absolute></div></div></body>",
            "body{margin:0} #parent{position:relative} #before{height:30px;margin-bottom:10px} #absolute{position:absolute;width:20px;height:20px}",
            800.0,
            600.0,
        );
        let absolute = find_id(&root, &dom, "absolute").unwrap();
        close(absolute.border_box.x, 0.0);
        close(absolute.border_box.y, 40.0);
    }

    #[test]
    fn contained_ink_and_local_scroll_overflow_do_not_extend_document_bounds() {
        for contain in ["layout", "paint", "content", "strict", "size layout"] {
            for overflow in ["visible", "clip", "hidden", "auto", "scroll"] {
                for position in ["static", "absolute", "fixed"] {
                    let (dom, root) = layout(
                        "<body><div id=container><div id=target></div></div></body>",
                        &format!("body{{margin:0}}#container{{contain:{contain};width:50px;height:50px;margin:10px;overflow:{overflow}}}#target{{position:{position};width:20px;height:300px;top:20px}}"),800.0,100.0);
                    let container = find_id(&root, &dom, "container").unwrap();
                    close(crate::layout_overflow_bottom(container), container.border_box.y + container.border_box.h + 10.0);
                    close(crate::layout_overflow_bottom(&root), 70.0);
                }
            }
        }
    }

    #[test]
    fn size_and_style_containment_leave_visible_document_overflow_intact() {
        for contain in ["none", "size", "inline-size", "style", "size style"] {
            let (dom, root) = layout(
                "<body><div id=container><div id=target></div></div></body>",
                &format!("body{{margin:0}}#container{{contain:{contain};height:50px;margin-top:10px}}#target{{height:300px}}"),800.0,100.0);
            let target = find_id(&root, &dom, "target").unwrap();
            close(crate::layout_overflow_bottom(&root), target.border_box.y + 300.0);
        }
    }

    #[test]
    fn scrollable_overflow_includes_absolute_but_excludes_fixed() {
        let (_dom, root) = layout(
            "<body><div id=absolute></div><div id=fixed></div></body>",
            "body{margin:0;height:20px} #absolute{position:absolute;top:200px;width:10px;height:50px} #fixed{position:fixed;top:500px;width:10px;height:50px}",
            800.0,
            600.0,
        );
        close(crate::layout_overflow_bottom(&root), 250.0);
    }
    #[test]
    fn absolute_auto_margins_resolve_both_axes_after_min_max() {
        for (extra, expected) in [
            ("width:80px;height:40px", (60.0, 30.0, 80.0, 40.0)),
            ("max-width:80px;max-height:40px", (60.0, 30.0, 80.0, 40.0)),
            (
                "width:10px;min-width:80px;height:10px;min-height:40px",
                (60.0, 30.0, 80.0, 40.0),
            ),
        ] {
            let (dom, root) = layout(
                "<div id=parent><div id=child></div></div>",
                &format!(
                    "body{{margin:0}} #parent{{position:relative;width:200px;height:100px}} #child{{position:absolute;left:10px;right:10px;top:10px;bottom:10px;margin:auto;{extra}}}"
                ),
                800.0,
                600.0,
            );
            let b = find_id(&root, &dom, "child").unwrap();
            for (actual, expected) in [
                (b.border_box.x, expected.0),
                (b.border_box.y, expected.1),
                (b.border_box.w, expected.2),
                (b.border_box.h, expected.3),
            ] {
                close(actual, expected);
            }
            assert_eq!(b.style.get("margin-left"), Some("auto"));
        }
    }

    #[test]
    fn absolute_negative_auto_margins_have_axis_specific_overflow() {
        for (direction, x) in [("ltr", 10.0), ("rtl", -50.0)] {
            let (dom, root) = layout(
                "<div id=parent><div id=child></div></div>",
                &format!(
                    "body{{margin:0}} #parent{{position:relative;direction:{direction};width:100px;height:100px}} #child{{position:absolute;left:10px;right:10px;top:10px;bottom:10px;margin:auto;width:140px;height:140px}}"
                ),
                800.0,
                600.0,
            );
            let b = find_id(&root, &dom, "child").unwrap();
            close(b.border_box.x, x);
            close(b.border_box.y, -20.0);
            close(b.margin.top.unwrap(), -30.0);
            close(b.margin.bottom.unwrap(), -30.0);
        }
    }

    #[test]
    fn absolute_single_auto_margin_absorbs_space_only_with_both_insets() {
        for (extra, x, margin) in [("right:20px", 130.0, 120.0), ("right:auto", 10.0, 0.0)] {
            let (dom, root) = layout(
                "<div id=parent><div id=child></div></div>",
                &format!(
                    "body{{margin:0}} #parent{{position:relative;width:200px}} #child{{position:absolute;left:10px;width:50px;height:10px;margin-left:auto;{extra}}}"
                ),
                800.0,
                600.0,
            );
            let b = find_id(&root, &dom, "child").unwrap();
            close(b.border_box.x, x);
            close(b.margin.left.unwrap(), margin);
        }
    }

    #[test]
    fn absolute_auto_width_shrinks_and_static_edge_uses_normal_parent() {
        for (parent_direction, child_direction, expected) in
            [("ltr", "rtl", 0.0), ("rtl", "ltr", 120.0)]
        {
            let (dom, root) = layout(
                "<div id=parent><div id=child><div style='width:80px;height:20px'></div></div></div>",
                &format!(
                    "body{{margin:0}} #parent{{position:relative;width:200px;direction:{parent_direction}}} #child{{position:absolute;direction:{child_direction}}}"
                ),
                800.0,
                600.0,
            );
            let b = find_id(&root, &dom, "child").unwrap();
            close(b.border_box.w, 80.0);
            close(b.border_box.x, expected);
            assert_eq!(b.style.get("width"), Some("auto"));
        }
    }

    #[test]
    fn absolute_empty_auto_width_keeps_padding_and_border() {
        let (dom, root) = layout(
            "<div id=parent><div id=child></div></div>",
            "body{margin:0} #parent{position:relative;width:200px} #child{position:absolute;right:20px;padding:10px;border:5px solid}",
            800.0,
            600.0,
        );
        let b = find_id(&root, &dom, "child").unwrap();
        close(b.border_box.w, 30.0);
        close(b.border_box.x, 150.0);
    }

    #[test]
    fn absolute_border_box_constraints_apply_before_margin_solution() {
        let (dom, root) = layout(
            "<div id=parent><div id=child><div id=percent></div></div></div>",
            "body{margin:0} #parent{position:relative;width:200px;height:200px} #child{position:absolute;box-sizing:border-box;left:0;right:0;top:0;bottom:0;width:100px;max-height:100px;margin:auto;padding:10px;border:5px solid} #percent{height:50%}",
            800.0,
            600.0,
        );
        let b = find_id(&root, &dom, "child").unwrap();
        close(b.border_box.x, 50.0);
        close(b.border_box.y, 50.0);
        close(b.border_box.w, 100.0);
        close(b.border_box.h, 100.0);
        close(find_id(&root, &dom, "percent").unwrap().content_box.h, 35.0);
        assert_eq!(b.style.get("box-sizing"), Some("border-box"));
    }

    #[test]
    fn relative_offsets_use_parent_direction_and_content_percentage_basis() {
        let (dom, root) = layout(
            "<div id=parent><div id=child></div></div>",
            "body{margin:0} #parent{direction:rtl;width:200px;height:100px;padding:20px} #child{direction:ltr;position:relative;left:50%;right:10%;top:50%;width:20px;height:10px}",
            800.0,
            600.0,
        );
        let p = find_id(&root, &dom, "parent").unwrap();
        let c = find_id(&root, &dom, "child").unwrap();
        close(c.border_box.x - p.content_box.x, 160.0);
        close(c.border_box.y - p.content_box.y, 50.0);
    }

    #[test]
    fn static_slot_ignores_preceding_relative_offset() {
        let (dom, root) = layout(
            "<div id=parent><div id=before></div><div id=child></div></div>",
            "body{margin:0} #parent{position:relative} #before{position:relative;top:90px;height:20px} #child{position:absolute;width:10px;height:10px}",
            800.0,
            600.0,
        );
        close(find_id(&root, &dom, "child").unwrap().border_box.y, 20.0);
    }

    #[test]
    fn sticky_ancestor_establishes_positioned_padding_box() {
        let (dom, root) = layout(
            "<div id=parent><div id=child></div></div>",
            "body{margin:0} #parent{position:sticky;width:200px;height:100px;padding:10px;border:5px solid} #child{position:absolute;right:0;bottom:0;width:20px;height:20px}",
            800.0,
            600.0,
        );
        let b = find_id(&root, &dom, "child").unwrap();
        close(b.border_box.x, 205.0);
        close(b.border_box.y, 105.0);
    }

    #[test]
    fn replaced_opposing_insets_keep_natural_size_and_ratio() {
        struct Natural;
        impl crate::ReplacedElementProvider for Natural {
            fn intrinsic_size(&self, _: amazing_dom::NodeIdx) -> Option<crate::IntrinsicSize> {
                Some(crate::IntrinsicSize {
                    width: 80.0,
                    height: 40.0,
                })
            }
        }
        for (extra, width, height) in [("", 80.0, 40.0), ("height:20px", 40.0, 20.0)] {
            let dom = amazing_html::parse("<div id=parent><img id=child></div>");
            // Supply dimensions only to the image, as a decoder would.
            struct ImageNatural<'a>(&'a Dom);
            impl crate::ReplacedElementProvider for ImageNatural<'_> {
                fn intrinsic_size(
                    &self,
                    node: amazing_dom::NodeIdx,
                ) -> Option<crate::IntrinsicSize> {
                    (self.0.name(node) == Some("img"))
                        .then(|| Natural.intrinsic_size(node).unwrap())
                }
            }
            let root = crate::layout_dom_document_with_replaced_in_viewport(
                &dom,
                &format!(
                    "body{{margin:0}} #parent{{position:relative;width:200px;height:100px}} img{{position:absolute;left:0;right:0;top:0;bottom:0;margin:auto;{extra}}}"
                ),
                800.0,
                Some(600.0),
                &FontSource::fallback(),
                &ImageNatural(&dom),
            );
            let b = find_id(&root, &dom, "child").unwrap();
            close(b.border_box.w, width);
            close(b.border_box.h, height);
            close(b.border_box.x, (200.0 - width) / 2.0);
            close(b.border_box.y, (100.0 - height) / 2.0);
        }
    }
    #[test]
    fn absolute_static_slot_stays_before_later_anonymous_inline_content() {
        let (dom, root) = layout(
            "<p style='height:20px;margin:0'>x</p>\n<div id=child></div>\n<span style='display:inline-table;width:100px;height:100px'></span>",
            "body{margin:0} #child{position:absolute;width:100px;height:100px}",
            800.0,
            600.0,
        );
        close(find_id(&root, &dom, "child").unwrap().border_box.y, 20.0);
        let (dom, root) = layout(
            "<div><span>text before <div id=child></div><div id=control></div></span></div>",
            "body{margin:0} #child,#control{width:100px;height:20px} #child{position:absolute}",
            800.0,
            600.0,
        );
        close(
            find_id(&root, &dom, "child").unwrap().border_box.y,
            find_id(&root, &dom, "control").unwrap().border_box.y,
        );
    }

    #[test]
    fn absolute_shrink_width_includes_atomic_table_contributions() {
        let dom = amazing_html::parse(
            "<div id=parent><div id=child><span style='display:inline-table;width:80px;height:20px'></span></div></div>",
        );
        let text = amazing_text::FallbackTextSystem::new();
        let root = crate::layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            "body{margin:0} #parent{position:relative;width:200px} #child{position:absolute;padding:2px}",
            800.0,
            Some(600.0),
            &text,
            &crate::NoReplacedElements,
        );
        close(find_id(&root, &dom, "child").unwrap().content_box.w, 80.0);
    }

    #[test]
    fn normal_rtl_and_absolute_static_boxes_share_inline_start() {
        let (dom, root) = layout(
            "<div id=parent><div id=abs></div><div id=normal></div></div>",
            "body{margin:0} #parent{direction:rtl;width:200px} #abs,#normal{direction:ltr;width:50px;height:20px} #abs{position:absolute}",
            800.0,
            600.0,
        );
        close(find_id(&root, &dom, "abs").unwrap().border_box.x, 150.0);
        close(find_id(&root, &dom, "normal").unwrap().border_box.x, 150.0);
    }
    #[test]
    fn positioned_used_lengths_apply_zoom_once() {
        for extra in ["", "width:80px", "max-width:80px"] {
            let (dom, root) = layout(
                "<div id=parent><div id=child></div></div>",
                &format!(
                    "body{{margin:0}} #parent{{position:relative;width:200px;height:100px}} #child{{position:absolute;zoom:2;left:10px;right:10px;top:0;bottom:0;height:20px;box-sizing:border-box;padding:2px;border:1px solid;margin:auto;{extra}}}"
                ),
                800.0,
                600.0,
            );
            let child = find_id(&root, &dom, "child").unwrap();
            close(child.border_box.w, 160.0);
            close(child.border_box.h, 40.0);
            close(child.border_box.x, 20.0);
            close(child.border_box.y, 30.0);
        }
    }
}
