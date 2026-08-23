//! Block formatting context (CSS2.1 §9.4.1, §10.3.3, §10.6, §8.3):
//! recursive box placement with the box-model constraint equation,
//! auto-margin centering, auto widths/heights, and adjacent-sibling
//! margin collapsing.
//!
//! Parent/child collapsing is approximated (see lib.rs deviations): the
//! first child's margin-top and the last child's margin-bottom shrink
//! against the parent's own margins by `max(0, child − parent)` *inside*
//! the parent, instead of propagating the collapsed margin past the
//! parent's border edge. Sibling-to-sibling collapsing follows the max
//! rule exactly. Separation by padding/border disables the adjustment,
//! matching §8.3.1.

use crate::font::FontSource;
use crate::values::{self, MaybeLen};
use crate::{Flow, LayoutBox, Rect};

/// Lays out the root box (`<body>`) against the viewport. Returns the total
/// page height (bottom edge of the root's margin box).
pub fn layout_root(root: &mut LayoutBox, viewport_width: f32, fonts: &FontSource) -> f32 {
    // Margins are applied inside layout_block — no pre-offset here.
    layout_block(root, 0.0, 0.0, viewport_width, fonts);
    root.border_box.y + root.border_box.h + root.margin.bottom.unwrap_or(0.0)
}

/// Parent-side context for the first/last-child margin approximation.
#[derive(Debug, Clone, Copy)]
struct CollapseCtx {
    /// This box's own top/bottom margins (auto counts as 0).
    parent_top: f32,
    parent_bottom: f32,
    /// Padding/border on that side separates this box from its children
    /// and disables the collapse adjustment (§8.3.1).
    top_separated: bool,
    bottom_separated: bool,
}

/// Lays out one block-level box whose *margin* edge starts at (`x`, `y`)
/// inside a containing block `cbw` wide. Fills the model + geometry fields
/// (absolute px), lays out children, and returns the outer height
/// (border-box height + margin-top + margin-bottom).
pub fn layout_block(b: &mut LayoutBox, x: f32, y: f32, cbw: f32, fonts: &FontSource) -> f32 {
    let font_px = b.font_size;
    b.margin = values::resolve_margins(&b.style, font_px, cbw);
    b.padding = values::resolve_padding(&b.style, font_px, cbw);
    b.border = values::resolve_borders(&b.style, font_px);

    // The collapse context describes THIS box as a parent; it can only be
    // built after the model above is resolved.
    let ctx = CollapseCtx {
        parent_top: b.margin.top.unwrap_or(0.0),
        parent_bottom: b.margin.bottom.unwrap_or(0.0),
        top_separated: b.padding.top > 0.0 || b.border.top > 0.0,
        bottom_separated: b.padding.bottom > 0.0 || b.border.bottom > 0.0,
    };

    let bl = b.border.left;
    let br_ = b.border.right;
    let bt = b.border.top;
    let bb = b.border.bottom;
    let pl = b.padding.left;
    let pr = b.padding.right;

    // --- §10.3.3 horizontal constraint ---------------------------------
    let content_w;
    match values::resolve_extent(&b.style, "width", font_px, cbw) {
        MaybeLen::Px(w) => {
            let fixed = bl + pl + w + pr + br_;
            match (b.margin.left.is_none(), b.margin.right.is_none()) {
                // Both auto → equal split centers the box (negatives allowed).
                (true, true) => {
                    let each = (cbw - fixed) / 2.0;
                    b.margin.left = Some(each);
                    b.margin.right = Some(each);
                },
                // Only left auto → left absorbs the remainder.
                (true, false) => {
                    let l = cbw - (b.margin.right.unwrap_or(0.0) + fixed);
                    b.margin.left = Some(l);
                },
                // Only right auto → right absorbs the remainder.
                (false, true) => {
                    let r = cbw - (b.margin.left.unwrap_or(0.0) + fixed);
                    b.margin.right = Some(r);
                },
                // Over-constrained → ignore margin-right in LTR flow.
                (false, false) => {
                    let r = cbw - (b.margin.left.unwrap_or(0.0) + fixed);
                    b.margin.right = Some(r);
                },
            }
            content_w = w;
        },
        MaybeLen::Auto => {
            if let Some(size) = b.replaced_size {
                content_w = size.width;
            } else {
                // Width fills what remains after the side margins; auto side
                // margins count as zero here (they never absorb vertically).
                let used_ml = b.margin.left.unwrap_or(0.0);
                let used_mr = b.margin.right.unwrap_or(0.0);
                content_w = (cbw - used_ml - bl - pl - pr - br_ - used_mr).max(0.0);
            }
        },
    }

    // Margin edges: the box's border box starts after its own margins.
    let mt = b.margin.top.unwrap_or(0.0);
    let border_x = x + b.margin.left.unwrap_or(0.0);
    let border_y = y + mt;
    let content_x = border_x + bl + pl;
    let content_y = border_y + bt + pt(b);

    // Anonymous blocks were built with an unresolved strut (line_height 0)
    // whenever their host's metrics weren't in scope during tree building;
    // they inherit the host's line-height now.
    match &mut b.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for c in children.iter_mut() {
                if c.node.is_none() && c.line_height == 0.0 {
                    c.line_height = b.line_height;
                }
            }
        },
        Flow::Inlines(_) | Flow::Empty => {},
    }

    // --- children ------------------------------------------------------
    let def_h = specified_height(b, font_px);
    let content_h = match &mut b.flow {
        Flow::Flex(children) => crate::flex::layout_flex(
            children,
            &b.style,
            font_px,
            content_x,
            content_y,
            content_w,
            def_h,
            fonts,
        ),
        Flow::Grid(children) => crate::grid::layout_grid(
            children,
            &b.style,
            font_px,
            content_x,
            content_y,
            content_w,
            def_h,
            fonts,
        ),
        Flow::Blocks(children) if !children.is_empty() => {
            stack_blocks(children, content_x, content_y, content_w, ctx, fonts)
        },
        Flow::Inlines(items) => {
            let lh = if b.line_height > 0.0 {
                b.line_height
            } else {
                fonts.scaled_metrics(b.font_size).line_height
            };
            let strut = crate::inline::Strut::new(fonts, b.font_size, lh);
            let lines =
                crate::inline::layout_lines(items, content_x, content_y, content_w, &strut, fonts);
            let height = lines.iter().map(|line| line.height).sum();
            b.lines = lines;
            height
        },
        _ => 0.0,
    };

    // --- height (§10.6): specified wins; % heights stay auto (the CB's
    // height is not available in this single pass) ---
    let replaced_height = b.replaced_size.map(|size| {
        if size.width > 0.0 {
            size.height * (content_w / size.width)
        } else {
            size.height
        }
    });
    let final_h = specified_height(b, font_px).or(replaced_height).unwrap_or(content_h);

    b.content_box = Rect { x: content_x, y: content_y, w: content_w, h: final_h };
    b.border_box = Rect {
        x: border_x,
        y: border_y,
        w: bl + pl + content_w + pr + br_,
        h: bt + pt(b) + final_h + pb(b) + bb,
    };

    b.border_box.h + b.margin.top.unwrap_or(0.0) + b.margin.bottom.unwrap_or(0.0)
}

fn pt(b: &LayoutBox) -> f32 {
    b.padding.top
}
fn pb(b: &LayoutBox) -> f32 {
    b.padding.bottom
}

/// Specified `height` in px, or None (auto). Percentages need the
/// containing block's height — unavailable in this pass — so they stay auto.
fn specified_height(b: &LayoutBox, font_px: f32) -> Option<f32> {
    let v = b.style.get("height")?;
    if v.trim_end().ends_with('%') {
        return None;
    }
    match MaybeLen::parse(v, font_px, 0.0) {
        Some(MaybeLen::Px(h)) => Some(h),
        _ => None,
    }
}

/// Stacks block children vertically: sibling margins collapse per the max
/// rule; the first/last children additionally shrink against the parent's
/// own margins (documented approximation). Returns the content height.
///
/// Children are positioned by their intended BORDER-BOX top edge; since
/// [`layout_block`] treats its `y` argument as the margin-box top edge
/// (adding the child's own resolved margin-top), the caller subtracts
/// `cmt` back out so margins enter exactly once.
fn stack_blocks(
    children: &mut [LayoutBox],
    content_x: f32,
    content_y: f32,
    content_w: f32,
    parent_ctx: CollapseCtx,
    fonts: &FontSource,
) -> f32 {
    let mut prev_border_bottom = content_y;
    let mut prev_mb: Option<f32> = None;

    for (i, child) in children.iter_mut().enumerate() {
        // Child margins resolve against THIS containing block's width.
        let cm = values::resolve_margins(&child.style, child.font_size, content_w);
        let cmt = cm.top.unwrap_or(0.0);

        let border_top = if i == 0 {
            // First-child collapse approximation: the child's top margin
            // only pushes down by what exceeds the parent's own margin.
            let eff = if parent_ctx.top_separated {
                cmt
            } else {
                (cmt - parent_ctx.parent_top).max(0.0)
            };
            content_y + eff
        } else {
            // §8.3.1 sibling max rule, measured between border edges.
            prev_border_bottom + prev_mb.map_or(cmt, |p| p.max(cmt))
        };

        // Horizontal margins are applied inside layout_block itself.
        layout_block(child, content_x, border_top - cmt, content_w, fonts);

        prev_border_bottom = child.border_box.y + child.border_box.h;
        prev_mb = Some(child.margin.bottom.unwrap_or(0.0));
    }

    // Auto height (§10.6.3): last child's border-box bottom minus the part
    // of its margin-bottom that collapses against ours.
    let last_child = children.last().expect("non-empty checked by caller");
    let mut end = last_child.border_box.y + last_child.border_box.h;
    if !parent_ctx.bottom_separated {
        let cmb = last_child.margin.bottom.unwrap_or(0.0);
        end -= cmb.min(parent_ctx.parent_bottom).max(0.0);
    }
    end - content_y
}
