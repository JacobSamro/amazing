//! Post-normal-flow positioning (CSS Position 3 / CSS2 §9.3, §10.3.7).
//!
//! Box construction removes absolute/fixed descendants from their parent's
//! formatting context. After normal layout establishes geometry, this module
//! applies relative offsets and lays out every out-of-flow subtree against the
//! nearest positioned padding box (or the viewport for fixed positioning).

use crate::block::layout_block;
use crate::font::FontSource;
use crate::values::{self, MaybeLen};
use crate::{Flow, LayoutBox, Rect};

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
) {
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_width.max(0.0),
        h: viewport_height.max(0.0),
    };
    position_subtree(root, viewport, viewport, viewport, fonts);
}

fn position_subtree(
    box_: &mut LayoutBox,
    viewport: Rect,
    normal_containing_block: Rect,
    inherited_positioned_cb: Rect,
    fonts: &FontSource,
) {
    if position_kind(box_) == PositionKind::Relative {
        let (dx, dy) = relative_offset(box_, normal_containing_block);
        shift_box(box_, dx, dy);
    }

    let own_padding_box = padding_box(box_);
    let descendant_positioned_cb = match position_kind(box_) {
        PositionKind::Relative | PositionKind::Absolute | PositionKind::Fixed => own_padding_box,
        PositionKind::Static | PositionKind::Sticky => inherited_positioned_cb,
    };

    match &mut box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                position_subtree(
                    child,
                    viewport,
                    own_padding_box,
                    descendant_positioned_cb,
                    fonts,
                );
            }
        },
        Flow::Inlines(_) | Flow::Empty => {},
    }

    let anchors: Vec<(f32, f32)> = box_
        .positioned
        .iter()
        .map(|child| static_anchor(box_, child))
        .collect();
    let mut positioned = std::mem::take(&mut box_.positioned);
    for (child, static_anchor) in positioned.iter_mut().zip(anchors) {
        let containing_block = if position_kind(child) == PositionKind::Fixed {
            viewport
        } else {
            descendant_positioned_cb
        };
        let specified_margins =
            values::resolve_margins(&child.style, child.font_size, containing_block.w);
        prepare_stretch(child, containing_block);
        layout_block(
            child,
            containing_block.x,
            containing_block.y,
            containing_block.w,
            fonts,
        );
        child.margin = specified_margins;
        let (x, y) = absolute_origin(
            child,
            containing_block,
            static_anchor,
            specified_margins,
        );
        shift_box(child, x - child.border_box.x, y - child.border_box.y);
        position_subtree(
            child,
            viewport,
            containing_block,
            padding_box(child),
            fonts,
        );
    }
    box_.positioned = positioned;
}

fn padding_box(box_: &LayoutBox) -> Rect {
    Rect {
        x: box_.content_box.x - box_.padding.left,
        y: box_.content_box.y - box_.padding.top,
        w: box_.content_box.w + box_.padding.left + box_.padding.right,
        h: box_.content_box.h + box_.padding.top + box_.padding.bottom,
    }
}

fn resolve_inset(box_: &LayoutBox, property: &str, base: f32) -> Option<f32> {
    match MaybeLen::parse(box_.style.get(property)?, box_.font_size, base)? {
        MaybeLen::Px(value) => Some(value),
        MaybeLen::Auto => None,
    }
}

fn relative_offset(box_: &LayoutBox, containing_block: Rect) -> (f32, f32) {
    let left = resolve_inset(box_, "left", containing_block.w);
    let right = resolve_inset(box_, "right", containing_block.w);
    let top = resolve_inset(box_, "top", containing_block.h);
    let bottom = resolve_inset(box_, "bottom", containing_block.h);
    let rtl = box_
        .style
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let dx = if rtl {
        right.map(|value| -value).or(left).unwrap_or(0.0)
    } else {
        left.or_else(|| right.map(|value| -value)).unwrap_or(0.0)
    };
    let dy = top.or_else(|| bottom.map(|value| -value)).unwrap_or(0.0);
    (dx, dy)
}

fn prepare_stretch(box_: &mut LayoutBox, containing_block: Rect) {
    let left = resolve_inset(box_, "left", containing_block.w);
    let right = resolve_inset(box_, "right", containing_block.w);
    let top = resolve_inset(box_, "top", containing_block.h);
    let bottom = resolve_inset(box_, "bottom", containing_block.h);
    let margins = values::resolve_margins(&box_.style, box_.font_size, containing_block.w);
    let padding = values::resolve_padding(&box_.style, box_.font_size, containing_block.w);
    let border = values::resolve_borders(&box_.style, box_.font_size);

    let width_auto = box_
        .style
        .get("width")
        .is_none_or(|value| value.eq_ignore_ascii_case("auto"));
    if width_auto
        && let (Some(left), Some(right)) = (left, right)
    {
        let border_width = containing_block.w
            - left
            - right
            - margins.left.unwrap_or(0.0)
            - margins.right.unwrap_or(0.0);
        let content_width = border_width
            - padding.left
            - padding.right
            - border.left
            - border.right;
        box_.style
            .set("width", &format!("{}px", content_width.max(0.0)));
    }

    let height_auto = box_
        .style
        .get("height")
        .is_none_or(|value| value.eq_ignore_ascii_case("auto"));
    if height_auto
        && let (Some(top), Some(bottom)) = (top, bottom)
    {
        let border_height = containing_block.h
            - top
            - bottom
            - margins.top.unwrap_or(0.0)
            - margins.bottom.unwrap_or(0.0);
        let content_height = border_height
            - padding.top
            - padding.bottom
            - border.top
            - border.bottom;
        box_.style
            .set("height", &format!("{}px", content_height.max(0.0)));
    }
}

fn absolute_origin(
    box_: &LayoutBox,
    containing_block: Rect,
    static_anchor: (f32, f32),
    margins: values::SidesM,
) -> (f32, f32) {
    let left = resolve_inset(box_, "left", containing_block.w);
    let right = resolve_inset(box_, "right", containing_block.w);
    let top = resolve_inset(box_, "top", containing_block.h);
    let bottom = resolve_inset(box_, "bottom", containing_block.h);
    let rtl = box_
        .style
        .get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let x = if rtl {
        right
            .map(|right| {
                containing_block.x + containing_block.w
                    - right
                    - box_.border_box.w
                    - margins.right.unwrap_or(0.0)
            })
            .or_else(|| {
                left.map(|left| containing_block.x + left + margins.left.unwrap_or(0.0))
            })
            .unwrap_or(static_anchor.0)
    } else {
        left.map(|left| containing_block.x + left + margins.left.unwrap_or(0.0))
            .or_else(|| {
                right.map(|right| {
                    containing_block.x + containing_block.w
                        - right
                        - box_.border_box.w
                        - margins.right.unwrap_or(0.0)
                })
            })
            .unwrap_or(static_anchor.0)
    };
    let y = top
        .map(|top| containing_block.y + top + margins.top.unwrap_or(0.0))
        .or_else(|| {
            bottom.map(|bottom| {
                containing_block.y + containing_block.h
                    - bottom
                    - box_.border_box.h
                    - margins.bottom.unwrap_or(0.0)
            })
        })
        .unwrap_or(static_anchor.1);
    (x, y)
}

fn static_anchor(parent: &LayoutBox, positioned: &LayoutBox) -> (f32, f32) {
    let target = positioned.node.map(|node| node.0).unwrap_or(u32::MAX);
    let preceding = match &parent.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children
            .iter()
            .filter(|child| child.node.is_some_and(|node| node.0 < target))
            .max_by_key(|child| child.node.map(|node| node.0).unwrap_or(0)),
        Flow::Inlines(_) | Flow::Empty => None,
    };
    preceding.map_or((parent.content_box.x, parent.content_box.y), |sibling| {
        (
            parent.content_box.x,
            sibling.border_box.y
                + sibling.border_box.h
                + sibling.margin.bottom.unwrap_or(0.0),
        )
    })
}

fn shift_box(box_: &mut LayoutBox, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
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
        },
        Flow::Inlines(_) | Flow::Empty => {},
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
            },
            Flow::Inlines(_) | Flow::Empty => None,
        };
        in_flow.or_else(|| {
            box_.positioned
                .iter()
                .find_map(|child| find_id(child, dom, id))
        })
    }

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.01, "got {actual}, want {expected}");
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
    fn percentage_relative_and_absolute_insets_use_containing_block_axes() {
        let (dom, root) = layout(
            "<body><div id=relative><div id=absolute></div></div></body>",
            "body{margin:0} #relative{position:relative;left:10%;top:25%;width:200px;height:100px} #absolute{position:absolute;left:50%;top:50%;width:20px;height:20px}",
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
    fn rtl_overconstraint_uses_right_inset() {
        let (dom, root) = layout(
            "<body><div id=parent><div id=child></div></div></body>",
            "body{margin:0} #parent{position:relative;width:200px;height:100px} #child{direction:rtl;position:absolute;left:10px;right:20px;top:0;width:50px;height:10px}",
            800.0,
            600.0,
        );
        close(find_id(&root, &dom, "child").unwrap().border_box.x, 130.0);
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
}
