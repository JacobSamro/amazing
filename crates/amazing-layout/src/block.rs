//! Block formatting context (CSS2.1 §9.4.1, §10.3.3, §10.6, §8.3):
//! recursive box placement with the box-model constraint equation,
//! auto-margin centering, auto widths/heights, and adjacent-sibling
//! margin collapsing.
//!
//! Signed sibling collapse and collapse-through empty blocks use one
//! adjoining-margin accumulator. Parent/child propagation is still
//! approximated (see lib.rs deviations): the
//! first child's margin-top and the last child's margin-bottom shrink
//! against the parent's own margins by `max(0, child − parent)` *inside*
//! the parent, instead of propagating the collapsed margin past the
//! parent's border edge. Separation by padding/border/BFC disables the adjustment,
//! matching §8.3.1.

use crate::floats::{self, FloatContext};
use crate::font::FontSource;
use crate::values::{self, MaybeLen};
use crate::{Flow, InlineNode, LayoutBox, Rect};
use amazing_css::ComputedStyle;

/// Lays out the root box (`<body>`) against the viewport. Returns the total
/// page height (bottom edge of the root's margin box).
pub fn layout_root(root: &mut LayoutBox, viewport_width: f32, fonts: &FontSource) -> f32 {
    layout_root_in_viewport(root, viewport_width, None, fonts)
}

pub fn layout_root_in_viewport(
    root: &mut LayoutBox,
    viewport_width: f32,
    viewport_height: Option<f32>,
    fonts: &FontSource,
) -> f32 {
    root.containing_block_height = viewport_height;
    // Margins are applied inside layout_block — no pre-offset here.
    layout_block(root, 0.0, 0.0, viewport_width, fonts);
    let page_height = root.border_box.y + root.border_box.h + root.margin.bottom.unwrap_or(0.0);
    crate::position::layout_positioned(
        root,
        viewport_width,
        viewport_height.unwrap_or(page_height),
        fonts,
    );
    page_height
}

/// Float-bearing documents shape lines during block placement so downstream
/// clearance and auto heights observe the actual provider metrics.
pub(crate) fn layout_root_with_text(
    root: &mut LayoutBox,
    width: f32,
    height: Option<f32>,
    text: &dyn amazing_text::TextSystem,
) {
    root.containing_block_height = height;
    let fonts = FontSource::fallback();
    layout_block_in_context(
        root,
        0.0,
        0.0,
        width,
        &fonts,
        &mut FloatContext::default(),
        Some(text),
        true,
    );
    crate::position::layout_positioned(
        root,
        width,
        height.unwrap_or(root.border_box.y + root.border_box.h),
        &fonts,
    );
    for child in &mut root.positioned {
        crate::apply_shaped_text(child, text);
    }
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

/// A set of adjoining vertical margins. CSS collapse keeps the largest
/// positive component and the most-negative component, then adds them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct CollapsedMargin {
    positive: f32,
    negative: f32,
}

impl CollapsedMargin {
    fn from_margin(value: f32) -> Self {
        let mut collapsed = Self::default();
        collapsed.add(value);
        collapsed
    }

    fn add(&mut self, value: f32) {
        if value >= 0.0 {
            self.positive = self.positive.max(value);
        } else {
            self.negative = self.negative.min(value);
        }
    }

    fn value(self) -> f32 {
        self.positive + self.negative
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct StackResult {
    content_end: f32,
    trailing: CollapsedMargin,
    trailing_clearance: bool,
}

/// Lays out one block-level box whose *margin* edge starts at (`x`, `y`)
/// inside a containing block `cbw` wide. Fills the model + geometry fields
/// (absolute px), lays out children, and returns the outer height
/// (border-box height + margin-top + margin-bottom).
pub fn layout_block(b: &mut LayoutBox, x: f32, y: f32, cbw: f32, fonts: &FontSource) -> f32 {
    layout_block_in_context(
        b,
        x,
        y,
        cbw,
        fonts,
        &mut FloatContext::default(),
        None,
        true,
    )
}

pub(crate) fn layout_table_content(b: &mut LayoutBox, x:f32, y:f32, width:f32,
    fonts:&FontSource, text:Option<&dyn amazing_text::TextSystem>) -> f32 {
    layout_block_in_context(b,x,y,width,fonts,&mut FloatContext::default(),text,true)
}

fn layout_block_in_context(
    b: &mut LayoutBox,
    x: f32,
    y: f32,
    cbw: f32,
    fonts: &FontSource,
    inherited: &mut FloatContext,
    text: Option<&dyn amazing_text::TextSystem>,
    root: bool,
) -> f32 {
    if b.table_wrapper { return crate::table::layout_wrapper(b,x,y,cbw,fonts,text); }
    if crate::table_tree::role(b) == crate::table_tree::Role::Table {
        return crate::table::layout(b, x, y, cbw, fonts, text, 0.0);
    }
    let font_px = b.font_size;
    b.margin = values::resolve_margins(&b.style, font_px, cbw);
    b.padding = values::resolve_padding(&b.style, font_px, cbw);
    b.border = values::resolve_borders(&b.style, font_px);

    // The collapse context describes THIS box as a parent; it can only be
    // built after the model above is resolved.
    let establishes_bfc = root || establishes_bfc(&b.style);
    let mut local_floats = FloatContext::default();
    let floats = if establishes_bfc {
        &mut local_floats
    } else {
        inherited
    };
    let floating = floats::side(&b.style).is_some();
    let ctx = CollapseCtx {
        parent_top: b.margin.top.unwrap_or(0.0),
        parent_bottom: b.margin.bottom.unwrap_or(0.0),
        top_separated: (!root && establishes_bfc) || b.padding.top > 0.0 || b.border.top > 0.0,
        bottom_separated: (!root && establishes_bfc)
            || b.padding.bottom > 0.0
            || b.border.bottom > 0.0,
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
            if floating || b.inline_atomic {
                b.margin.left = Some(b.margin.left.unwrap_or(0.0));
                b.margin.right = Some(b.margin.right.unwrap_or(0.0));
            } else {
                match (b.margin.left.is_none(), b.margin.right.is_none()) {
                    // Both auto → equal split centers the box (negatives allowed).
                    (true, true) => {
                        let each = (cbw - fixed) / 2.0;
                        b.margin.left = Some(each);
                        b.margin.right = Some(each);
                    }
                    // Only left auto → left absorbs the remainder.
                    (true, false) => {
                        let l = cbw - (b.margin.right.unwrap_or(0.0) + fixed);
                        b.margin.left = Some(l);
                    }
                    // Only right auto → right absorbs the remainder.
                    (false, true) => {
                        let r = cbw - (b.margin.left.unwrap_or(0.0) + fixed);
                        b.margin.right = Some(r);
                    }
                    // Over-constrained → ignore margin-right in LTR flow.
                    (false, false) => {
                        let r = cbw - (b.margin.left.unwrap_or(0.0) + fixed);
                        b.margin.right = Some(r);
                    }
                }
            }
            content_w = w;
        }
        MaybeLen::Auto => {
            if let Some(size) = b.replaced_size {
                content_w = size.width;
            } else if floating || (b.inline_atomic && is_inline_block(&b.style)) {
                let chrome = bl + pl + pr + br_;
                let available =
                    (cbw - b.margin.left.unwrap_or(0.0) - b.margin.right.unwrap_or(0.0)).max(0.0);
                content_w = (crate::flex::min_content_width(b, fonts)
                    .max(available)
                    .min(crate::flex::max_content_width(b, fonts))
                    - chrome)
                    .max(0.0);
            } else {
                // Width fills what remains after the side margins; auto side
                // margins count as zero here (they never absorb vertically).
                let used_ml = b.margin.left.unwrap_or(0.0);
                let used_mr = b.margin.right.unwrap_or(0.0);
                content_w = (cbw - used_ml - bl - pl - pr - br_ - used_mr).max(0.0);
            }
        }
    }

    // Margin edges: the box's border box starts after its own margins.
    let mt = b.margin.top.unwrap_or(0.0);
    let border_x = x + b.margin.left.unwrap_or(0.0);
    let border_y = y + mt;
    let content_x = border_x + bl + pl;
    let content_y = border_y + bt + pt(b);

    let def_h = specified_height(b, font_px);
    let inline_basis = def_h.map(|h| {
        constrain_height(
            h,
            size_constraint(b, "min-height", font_px).unwrap_or(0.0),
            size_constraint(b, "max-height", font_px),
        )
    });
    // Anonymous blocks inherit the host strut, including an explicit zero
    // line-height. They must not revert to normal after a float splits text.
    match &mut b.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for c in children.iter_mut() {
                c.containing_block_height = inline_basis;
                if c.node.is_none() {
                    c.line_height = b.line_height;
                    if let Some(value) = b.style.get("line-height") {
                        c.style.set("line-height", value);
                    }
                    if let Some(value) = b.style.get("direction") {
                        c.style.set("direction", value);
                    }
                }
            }
        }
        Flow::Inlines(_) | Flow::Empty => {}
    }

    // Atomic inline descendants have their own formatting context,
    // but their outer boxes take part in this box's line breaking.
    if matches!(b.flow, Flow::Inlines(_)) {
        layout_atomic_inlines(b, content_w, fonts, text);
    }

    // --- children ------------------------------------------------------
    let mut content = match &mut b.flow {
        Flow::Flex(children) => StackResult {
            content_end: crate::flex::layout_flex(
                children, &b.style, font_px, content_x, content_y, content_w, def_h, fonts,
            ),
            trailing: CollapsedMargin::default(),
            trailing_clearance: false,
        },
        Flow::Grid(children) => StackResult {
            content_end: crate::grid::layout_grid(
                children, &b.style, font_px, content_x, content_y, content_w, def_h, fonts,
            ),
            trailing: CollapsedMargin::default(),
            trailing_clearance: false,
        },
        Flow::Blocks(children) if !children.is_empty() => stack_blocks(
            children, content_x, content_y, content_w, ctx, fonts, floats, text,
        ),
        Flow::Inlines(items) => {
            crate::replaced::resolve(
                items,
                content_w,
                if b.node.is_none() {
                    b.containing_block_height
                } else {
                    inline_basis
                },
            );
            let lh = b.line_height;
            let strut = crate::inline::Strut::new(fonts, b.font_size, lh);
            let lines = crate::inline::layout_lines_with_floats(
                items, content_x, content_y, content_w, &strut, fonts, floats,
            );
            let height = lines
                .last()
                .map_or(0.0, |line| line.y_top + line.height - content_y);
            b.lines = lines;
            StackResult {
                content_end: height,
                trailing: CollapsedMargin::default(),
                trailing_clearance: false,
            }
        }
        _ => StackResult::default(),
    };

    b.float_context = floats.clone();
    if let Some(text) = text {
        b.content_box = Rect {
            x: content_x,
            y: content_y,
            w: content_w,
            h: def_h.unwrap_or(content.content_end),
        };
        if matches!(b.flow, Flow::Inlines(_) | Flow::Flex(_) | Flow::Grid(_)) {
            crate::apply_shaped_text(b, text);
            if matches!(b.flow, Flow::Inlines(_)) {
                content.content_end = b
                    .lines
                    .last()
                    .map_or(0.0, |line| line.y_top + line.height - content_y);
            }
        }
    }
    if matches!(b.flow, Flow::Inlines(_)) {
        place_atomic_inlines(b);
    }
    let float_extent = if establishes_bfc {
        (floats.bottom() - content_y).max(0.0)
    } else {
        0.0
    };

    // --- height (§10.6–10.7): specified size wins, then min/max constrain.
    // Percentage constraints stay unresolved while the containing block's
    // block size is indefinite. ---
    let replaced_height = b.replaced_size.map(|size| {
        if size.width > 0.0 {
            size.height * (content_w / size.width)
        } else {
            size.height
        }
    });
    let fixed_height = def_h.or(replaced_height);
    let min_height = size_constraint(b, "min-height", font_px).unwrap_or(0.0);
    let max_height = size_constraint(b, "max-height", font_px);
    let final_h = if let Some(height) = fixed_height {
        constrain_height(height, min_height, max_height)
    } else if ctx.bottom_separated || content.trailing_clearance {
        // A BFC or bottom border/padding traps the last child's margin inside
        // auto height. Negative trailing margins can therefore shrink it.
        constrain_height(
            (content.content_end + content.trailing.value())
                .max(float_extent)
                .max(0.0),
            min_height,
            max_height,
        )
    } else {
        let constrained = constrain_height(
            content.content_end.max(float_extent),
            min_height,
            max_height,
        );
        if (constrained - content.content_end).abs() <= 0.001 {
            // Auto height was not changed by min/max: the child's trailing
            // margin escapes and adjoins this box's own bottom margin.
            let mut escaped = content.trailing;
            escaped.add(ctx.parent_bottom);
            b.margin.bottom = Some(escaped.value());
        }
        // When a constraint changes the tentative height, the trailing margin
        // is trapped for collapse purposes but does not size the box from it.
        constrained
    };

    b.content_box = Rect {
        x: content_x,
        y: content_y,
        w: content_w,
        h: final_h,
    };
    b.border_box = Rect {
        x: border_x,
        y: border_y,
        w: bl + pl + content_w + pr + br_,
        h: bt + pt(b) + final_h + pb(b) + bb,
    };

    b.border_box.h + b.margin.top.unwrap_or(0.0) + b.margin.bottom.unwrap_or(0.0)
}

fn layout_atomic_inlines(parent: &mut LayoutBox, available: f32, fonts: &FontSource, text: Option<&dyn amazing_text::TextSystem>) {
    for child in parent.positioned.iter_mut().filter(|child| child.inline_atomic) {
        let inline_block = is_inline_block(&child.style);
        let outer_width = if inline_block {
            available
        } else if matches!(values::resolve_extent(&child.style, "width", child.font_size, available), MaybeLen::Auto) {
            let min = crate::flex::min_content_width(child, fonts);
            let max = crate::flex::max_content_width(child, fonts).max(min);
            min.max(available).min(max)
        } else {
            available
        };
        layout_block_in_context(child, 0.0, 0.0, outer_width, fonts, &mut FloatContext::default(), text, false);
    }
    refresh_atomic_inline_metrics(parent);
}

pub(crate) fn refresh_atomic_inline_metrics(parent: &mut LayoutBox) {
    let Flow::Inlines(items) = &mut parent.flow else { return };
    for child in parent.positioned.iter().filter(|child| child.inline_atomic) {
        let margin = child.margin;
        let width = child.border_box.w + margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0);
        let height = child.border_box.h + margin.top.unwrap_or(0.0) + margin.bottom.unwrap_or(0.0);
        if let Some(InlineNode::Atomic { size, baseline_offset, .. }) = items.iter_mut().find(|item| {
            matches!(item, InlineNode::Atomic { node, .. } if Some(*node) == child.node)
        }) {
            *size = crate::IntrinsicSize { width, height };
            *baseline_offset = if is_inline_block(&child.style) {
                inline_block_baseline(child) - child.border_box.y + margin.top.unwrap_or(0.0)
            } else {
                height
            };
        }
    }
}

pub(crate) fn place_atomic_inlines(parent: &mut LayoutBox) {
    for child in parent.positioned.iter_mut().filter(|child| child.inline_atomic) {
        let Some((line, fragment)) = parent.lines.iter().find_map(|line| {
            line.replaced.iter().find(|fragment| Some(fragment.node) == child.node).map(|fragment| (line, fragment))
        }) else { continue };
        let baseline = if is_inline_block(&child.style) {
            inline_block_baseline(child) - child.border_box.y
        } else {
            crate::flex::first_baseline(child) - child.border_box.y
        };
        let x = fragment.x + child.margin.left.unwrap_or(0.0);
        let y = if is_inline_block(&child.style) {
            fragment.y + child.margin.top.unwrap_or(0.0)
        } else {
            line.baseline - baseline
        };
        crate::flex::translate_after_positioning(child, x - child.border_box.x, y - child.border_box.y);
    }
}

fn is_inline_block(style: &ComputedStyle) -> bool {
    matches!(style.get("display"), Some("inline-block" | "inline flow-root"))
}

fn inline_block_baseline(box_: &LayoutBox) -> f32 {
    let overflow_visible = ["overflow-x", "overflow-y"].into_iter().all(|property| {
        box_.style.get(property).is_none_or(|value| value.eq_ignore_ascii_case("visible"))
    });
    if overflow_visible {
        if let Some(baseline) = last_in_flow_baseline(box_) {
            return baseline;
        }
    }
    box_.border_box.y + box_.border_box.h + box_.margin.bottom.unwrap_or(0.0)
}

fn last_in_flow_baseline(box_: &LayoutBox) -> Option<f32> {
    if let Some(line) = box_.lines.last() {
        return Some(line.baseline);
    }
    if let Flow::Blocks(children) = &box_.flow {
        return children.iter().rev()
            .filter(|child| floats::side(&child.style).is_none())
            .find_map(last_in_flow_baseline);
    }
    None
}

fn pt(b: &LayoutBox) -> f32 {
    b.padding.top
}
fn pb(b: &LayoutBox) -> f32 {
    b.padding.bottom
}

/// Specified `height` in px, or None (auto). Percentages need the
/// containing block's definite height; content-derived heights stay auto.
fn specified_height(b: &LayoutBox, font_px: f32) -> Option<f32> {
    let v = b.style.get("height")?;
    if v.trim_end().ends_with('%') && b.containing_block_height.is_none() {
        return None;
    }
    match MaybeLen::parse(v, font_px, b.containing_block_height.unwrap_or(0.0)) {
        Some(MaybeLen::Px(h)) => Some(h),
        _ => None,
    }
}

fn size_constraint(b: &LayoutBox, property: &str, font_px: f32) -> Option<f32> {
    let value = b.style.get(property)?.trim();
    if value.eq_ignore_ascii_case("none")
        || (value.ends_with('%') && b.containing_block_height.is_none())
    {
        return None;
    }
    match MaybeLen::parse(value, font_px, b.containing_block_height.unwrap_or(0.0)) {
        Some(MaybeLen::Px(size)) => Some(size.max(0.0)),
        _ => None,
    }
}

fn constrain_height(tentative: f32, min_height: f32, max_height: Option<f32>) -> f32 {
    max_height
        .map_or(tentative, |maximum| tentative.min(maximum))
        .max(min_height)
        .max(0.0)
}

/// Stacks block children vertically with one signed adjoining-margin group.
/// Empty zero-height blocks can collapse through without consuming the group;
/// the caller decides whether the final group escapes or contributes inside.
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
    floats: &mut FloatContext,
    text: Option<&dyn amazing_text::TextSystem>,
) -> StackResult {
    let mut cursor = content_y;
    let mut pending = CollapsedMargin::default();

    let mut trailing_clearance = false;
    let mut first_in_flow = true;
    for child in children.iter_mut() {
        // Child margins resolve against THIS containing block's width.
        let cm = values::resolve_margins(&child.style, child.font_size, content_w);
        let cmt = cm.top.unwrap_or(0.0);
        if let Some(side) = floats::side(&child.style) {
            let y = (cursor + pending.value())
                .max(content_y)
                .max(floats.clear(child.style.get("clear").unwrap_or("none")));
            layout_block_in_context(child, content_x, y, content_w, fonts, floats, text, false);
            let w = child.border_box.w + cm.left.unwrap_or(0.0) + cm.right.unwrap_or(0.0);
            let h = child.border_box.h + cmt + child.margin.bottom.unwrap_or(0.0);
            let placed = floats.place(side, content_x, content_w, y, w, h);
            crate::flex::translate(child, placed.x - content_x, placed.y - y);
            continue;
        }
        let top_component = if first_in_flow && !parent_ctx.top_separated {
            // Parent/first-child propagation remains represented as an
            // inside offset because the parent's border position was fixed by
            // its own caller. Unlike the old `max(0, child-parent)` shortcut,
            // this preserves mixed-sign collapse.
            let mut collapsed = CollapsedMargin::from_margin(parent_ctx.parent_top);
            collapsed.add(cmt);
            collapsed.value() - parent_ctx.parent_top
        } else {
            cmt
        };
        pending.add(top_component);
        let hypothetical_top = cursor + pending.value();
        let cleared = floats.clear(child.style.get("clear").unwrap_or("none"));
        let has_clearance = cleared > hypothetical_top;
        let border_top = if has_clearance {
            cleared.max(cursor + cmt)
        } else {
            hypothetical_top
        };
        first_in_flow = false;
        let collapses_through = can_collapse_through(child, content_w);

        // Horizontal margins are applied inside layout_block itself.
        if establishes_bfc(&child.style) {
            let needed =
                match values::resolve_extent(&child.style, "width", child.font_size, content_w) {
                    MaybeLen::Px(w) => {
                        let p = values::resolve_padding(&child.style, child.font_size, content_w);
                        let b = values::resolve_borders(&child.style, child.font_size);
                        w + p.left + p.right + b.left + b.right
                    }
                    MaybeLen::Auto => 0.0,
                };
            let (mut left, mut right, mut top) =
                floats.fit(content_x, content_w, border_top, 0.0, needed);
            loop {
                layout_block_in_context(
                    child,
                    left,
                    top - cmt,
                    right - left,
                    fonts,
                    floats,
                    text,
                    false,
                );
                let (next_left, next_right, next_top) =
                    floats.fit(content_x, content_w, top, child.border_box.h, needed);
                let (next_left, next_right) = if next_top == top {
                    (next_left.max(left), next_right.min(right))
                } else {
                    (next_left, next_right)
                };
                if (next_left, next_right, next_top) == (left, right, top) {
                    break;
                }
                left = next_left;
                right = next_right;
                top = next_top;
            }
        } else {
            layout_block_in_context(
                child,
                content_x,
                border_top - cmt,
                content_w,
                fonts,
                floats,
                text,
                false,
            );
        }
        if collapses_through {
            if has_clearance {
                // Clearance separates the previous margin group. The empty
                // box's own adjoining margins still collapse with following
                // siblings, but that group cannot escape the parent (§8.3.1).
                cursor = border_top - cmt;
                pending = CollapsedMargin::from_margin(cmt);
                trailing_clearance = true;
            }
            pending.add(child.margin.bottom.unwrap_or(0.0));
        } else {
            trailing_clearance = false;
            cursor = child.border_box.y + child.border_box.h;
            pending = CollapsedMargin::from_margin(child.margin.bottom.unwrap_or(0.0));
        }
    }

    StackResult {
        content_end: (cursor - content_y).max(0.0),
        trailing: pending,
        trailing_clearance,
    }
}

fn can_collapse_through(block: &LayoutBox, containing_width: f32) -> bool {
    if !matches!(&block.flow, Flow::Empty) {
        return false;
    }
    if establishes_bfc(&block.style) {
        return false;
    }
    let padding = values::resolve_padding(&block.style, block.font_size, containing_width);
    let border = values::resolve_borders(&block.style, block.font_size);
    if padding.top != 0.0 || padding.bottom != 0.0 || border.top != 0.0 || border.bottom != 0.0 {
        return false;
    }
    let min_height = size_constraint(block, "min-height", block.font_size).unwrap_or(0.0);
    let height = specified_height(block, block.font_size);
    min_height == 0.0 && height.is_none_or(|height| height == 0.0)
}

pub(crate) fn establishes_bfc(style: &ComputedStyle) -> bool {
    if floats::side(style).is_some()
        || matches!(
            style.get("display"),
            Some(
                "flow-root"
                    | "inline-block"
                    | "table-cell"
                    | "table-caption"
                    | "flex"
                    | "grid"
                    | "inline-flex"
                    | "inline-grid"
            )
        )
        || matches!(style.get("position"), Some("absolute" | "fixed"))
    {
        return true;
    }
    ["overflow-x", "overflow-y"].into_iter().any(|property| {
        style.get(property).is_some_and(|value| {
            !value.eq_ignore_ascii_case("visible") && !value.eq_ignore_ascii_case("clip")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSource;
    use amazing_dom::Dom;

    fn layout(html: &str, css: &str) -> (Dom, LayoutBox) {
        let dom = amazing_html::parse(html);
        let root = crate::layout_dom_document(&dom, css, 800.0, &FontSource::fallback());
        (dom, root)
    }

    fn find_id<'a>(layout: &'a LayoutBox, dom: &Dom, id: &str) -> Option<&'a LayoutBox> {
        let has_id = layout
            .node
            .and_then(|node| dom.attrs_of(node))
            .is_some_and(|attributes| {
                attributes
                    .iter()
                    .any(|(name, value)| name == "id" && value == id)
            });
        if has_id {
            return Some(layout);
        }
        match &layout.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
                children.iter().find_map(|child| find_id(child, dom, id))
            }
            Flow::Inlines(_) | Flow::Empty => None,
        }
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.01,
            "got {actual}, expected {expected}"
        );
    }

    #[test]
    fn mixed_sign_sibling_margins_collapse_by_positive_plus_negative_extrema() {
        let (dom, root) = layout(
            "<body><div id=a></div><div id=b></div></body>",
            "body{margin:0} #a{height:10px;margin-bottom:30px} #b{height:10px;margin-top:-12px}",
        );
        let a = find_id(&root, &dom, "a").unwrap();
        let b = find_id(&root, &dom, "b").unwrap();
        close(a.border_box.y, 0.0);
        close(b.border_box.y, 28.0);
    }

    #[test]
    fn negative_top_margin_shrinks_an_auto_height_parent() {
        let (dom, root) = layout(
            "<body><div id=outer><div id=first></div><div id=second></div></div><div id=after></div></body>",
            "body{margin:0} #first{height:200px} #second{height:20px;margin-top:-120px} #after{height:100px}",
        );
        let outer = find_id(&root, &dom, "outer").unwrap();
        let second = find_id(&root, &dom, "second").unwrap();
        let after = find_id(&root, &dom, "after").unwrap();
        close(second.border_box.y, 80.0);
        close(outer.content_box.h, 100.0);
        close(after.border_box.y, 100.0);
    }

    #[test]
    fn bfc_auto_height_includes_a_negative_trailing_margin() {
        let (dom, root) = layout(
            "<body><div id=outer><div id=first></div><div id=last></div></div></body>",
            "body{margin:0} #outer{overflow:hidden} #first{height:200px} #last{height:200px;margin-bottom:-200px}",
        );
        close(find_id(&root, &dom, "outer").unwrap().content_box.h, 200.0);
    }

    #[test]
    fn overflow_clip_does_not_establish_a_block_formatting_context() {
        let (dom, root) = layout(
            "<body><div id=outer><div id=first></div><div id=last></div></div></body>",
            "body{margin:0} #outer{overflow:clip} #first{height:200px} #last{height:200px;margin-bottom:-200px}",
        );
        close(find_id(&root, &dom, "outer").unwrap().content_box.h, 400.0);
    }

    #[test]
    fn empty_blocks_keep_one_adjoining_margin_group() {
        let (dom, root) = layout(
            "<body><div id=outer><div id=a></div><div id=b></div><div id=c></div></div></body>",
            "body{margin:0} #outer{overflow:hidden;height:400px} #a{margin-bottom:200px} #b{height:0} #c{height:200px;margin-top:100px}",
        );
        close(find_id(&root, &dom, "c").unwrap().border_box.y, 200.0);
    }

    #[test]
    fn min_height_that_raises_auto_height_traps_the_trailing_margin() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=child></div></div><div id=footer></div></body>",
            "body{margin:0} #parent{min-height:100px} #child{height:30px;margin-bottom:550px} #footer{height:50px}",
        );
        close(find_id(&root, &dom, "parent").unwrap().content_box.h, 100.0);
        close(find_id(&root, &dom, "footer").unwrap().border_box.y, 100.0);
    }

    #[test]
    fn min_height_below_content_allows_the_trailing_margin_to_escape() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=child></div></div><div id=footer></div></body>",
            "body{margin:0} #parent{min-height:5px} #child{height:30px;margin-bottom:50px} #footer{height:50px}",
        );
        close(find_id(&root, &dom, "parent").unwrap().content_box.h, 30.0);
        close(find_id(&root, &dom, "footer").unwrap().border_box.y, 80.0);
    }
}
