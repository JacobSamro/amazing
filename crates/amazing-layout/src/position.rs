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
    position_subtree(root, viewport, viewport, true, viewport, fonts, text);
}

fn position_subtree(
    box_: &mut LayoutBox,
    viewport: ContainingBlock,
    normal_containing_block: ContainingBlock,
    normal_cb_height_definite: bool,
    inherited_positioned_cb: ContainingBlock,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
) {
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
    let descendant_positioned_cb = match position_kind(box_) {
        PositionKind::Relative
        | PositionKind::Absolute
        | PositionKind::Fixed
        | PositionKind::Sticky => own_padding_box,
        PositionKind::Static => inherited_positioned_cb,
    };

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
            continue;
        }
        if child.inline_atomic {
            position_subtree(
                child,
                viewport,
                own_content_box,
                own_height_definite,
                descendant_positioned_cb,
                fonts,
                text,
            );
            continue;
        }
        let containing_block = if position_kind(child) == PositionKind::Fixed {
            viewport
        } else {
            descendant_positioned_cb
        };
        let units = font_units(child, text);
        let specified_margins =
            values::resolve_margins_with_units(&child.style, units, containing_block.rect.w);
        // Used sizes are temporary inputs to normal layout, not computed style.
        let computed_style = child.style.clone();
        prepare_sizes(child, containing_block.rect, static_anchor, fonts, text);
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
            let (x, y) = grid_child_static_anchor(box_, child, specified_margins);
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
    )
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
    box_: &LayoutBox,
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
    (dx, dy)
}

fn prepare_sizes(
    box_: &mut LayoutBox,
    cb: Rect,
    anchor: StaticAnchor,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
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
        if property == "width" && start.is_none() && end.is_none() {
            if anchor.right {
                end = Some(cb.x + cb.w - anchor.x);
            } else {
                start = Some(anchor.x - cb.x);
            }
        }
        let available =
            (extent - start.unwrap_or(0.0) - end.unwrap_or(0.0) - margin - chrome).max(0.0);
        let size = if stretch || (start.is_some() && end.is_some()) {
            Some(available)
        } else if property == "width" {
            let (minimum, preferred) = intrinsic_content_widths(box_, fonts, text);
            Some(minimum.max(available).min(preferred))
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
        (crate::flex::min_content_width(box_, fonts) - chrome).max(0.0),
        (crate::flex::max_content_width(box_, fonts) - chrome).max(0.0),
    )
}

fn absolute_origin(
    box_: &mut LayoutBox,
    containing_block: ContainingBlock,
    anchor: StaticAnchor,
    margins: values::SidesM,
    text: Option<&dyn TextSystem>,
) -> (f32, f32) {
    let cb = containing_block.rect;
    let x = Axis {
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
    }
    .solve();
    let y = Axis {
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
    }
    .solve();
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
    // Block flow currently exposes horizontal physical block origins. Vertical
    // static rectangles follow that same origin until block-axis mapping exists.
    let right = is_rtl(parent)
        && !parent
            .font_metric_style
            .as_ref()
            .is_some_and(|style| style.writing_mode.is_vertical_flow());
    StaticAnchor {
        x: parent.content_box.x + if right { parent.content_box.w } else { 0.0 },
        y: preceding.map_or(parent.content_box.y, |sibling| {
            sibling.border_box.y + sibling.border_box.h + sibling.margin.bottom.unwrap_or(0.0)
        }),
        right,
    }
}

#[derive(Clone, Copy)]
enum StaticAlign {
    Start,
    Center,
    End,
}

fn grid_child_static_anchor(
    parent: &LayoutBox,
    child: &LayoutBox,
    margins: values::SidesM,
) -> (f32, f32) {
    // Grid 1 §9.2: static position uses the grid container's content edges,
    // independent of grid-placement properties and the abspos containing block.
    let parent_rtl = parent
        .style
        .get("direction")
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
    let horizontal = static_align(
        choice("justify-self", "justify-items"),
        parent_rtl,
        child_rtl,
        true,
    );
    let vertical = static_align(
        choice("align-self", "align-items"),
        parent_rtl,
        child_rtl,
        false,
    );
    let x = aligned_border_start(
        parent.content_box.x,
        parent.content_box.w,
        child.border_box.w,
        margins.left.unwrap_or(0.0),
        margins.right.unwrap_or(0.0),
        horizontal,
    );
    let y = aligned_border_start(
        parent.content_box.y,
        parent.content_box.h,
        child.border_box.h,
        margins.top.unwrap_or(0.0),
        margins.bottom.unwrap_or(0.0),
        vertical,
    );
    (x, y)
}

fn static_align(value: &str, parent_rtl: bool, child_rtl: bool, horizontal: bool) -> StaticAlign {
    // The safe fallback needs the abspos grid area as its alignment container.
    // Until that rectangle is carried here, use its start edge rather than
    // aligning beyond it when a safe position is requested.
    if value
        .split_ascii_whitespace()
        .any(|part| part.eq_ignore_ascii_case("safe"))
    {
        return if horizontal && parent_rtl {
            StaticAlign::End
        } else {
            StaticAlign::Start
        };
    }
    let keyword = value
        .split_ascii_whitespace()
        .last()
        .unwrap_or("normal")
        .to_ascii_lowercase();
    let start = |rtl: bool| {
        if horizontal && rtl {
            StaticAlign::End
        } else {
            StaticAlign::Start
        }
    };
    let end = |rtl: bool| {
        if horizontal && rtl {
            StaticAlign::Start
        } else {
            StaticAlign::End
        }
    };
    match keyword.as_str() {
        "center" => StaticAlign::Center,
        "end" | "flex-end" => end(parent_rtl),
        "self-start" => start(child_rtl),
        "self-end" => end(child_rtl),
        "left" => StaticAlign::Start,
        "right" => StaticAlign::End,
        _ => start(parent_rtl),
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
    fn grid_child_static_position_uses_content_box_self_alignment() {
        for position in ["static", "relative"] {
            let (dom, root) = layout(
                "<body><div id=grid><div id=auto></div><div id=start></div><div id=end></div><div id=self_start></div><div id=safe></div></div></body>",
                &format!(
                    "body{{margin:0}} #grid{{display:grid;position:{position};width:100px;height:60px;padding:3px 5px;justify-items:center;align-items:center}} \
                     #grid>div{{position:absolute;width:20px;height:10px;grid-area:2 / 2 / 3 / 3}} \
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
            let auto = find_id(&root, &dom, "auto").unwrap();
            close(auto.border_box.x, x + 40.0);
            close(auto.border_box.y, y + 25.0);
            let start = find_id(&root, &dom, "start").unwrap();
            close(start.border_box.x, x);
            close(start.border_box.y, y);
            let end = find_id(&root, &dom, "end").unwrap();
            close(end.border_box.x, x + 80.0);
            close(end.border_box.y, y + 50.0);
            close(
                find_id(&root, &dom, "self_start").unwrap().border_box.x,
                x + 80.0,
            );
            let safe = find_id(&root, &dom, "safe").unwrap();
            close(safe.border_box.x, x);
            close(safe.border_box.y, y);
        }
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
