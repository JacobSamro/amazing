//! Post-normal-flow positioning (CSS Position 3 / CSS2 §9.3, §10.3.7).
//!
//! Box construction removes absolute/fixed descendants from their parent's
//! formatting context. After normal layout establishes geometry, this module
//! applies relative offsets and lays out every out-of-flow subtree against the
//! nearest positioned padding box (or the viewport for fixed positioning).

use crate::block::{layout_block, layout_block_with_text};
use crate::font::FontSource;
use crate::values;
use crate::{Flow, LayoutBox, Rect};
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
    position_subtree(root, viewport, viewport, true, viewport, fonts, text);
}

fn position_subtree(
    box_: &mut LayoutBox,
    viewport: Rect,
    normal_containing_block: Rect,
    normal_cb_height_definite: bool,
    inherited_positioned_cb: Rect,
    fonts: &FontSource,
    text: Option<&dyn TextSystem>,
) {
    if position_kind(box_) == PositionKind::Relative {
        let (dx, dy) = relative_offset(box_, normal_containing_block, normal_cb_height_definite, text);
        shift_box(box_, dx, dy);
    }

    let own_padding_box = padding_box(box_);
    let own_height_definite = box_.style.get("height").is_some_and(|height| {
        !height.eq_ignore_ascii_case("auto")
            && (!height.contains('%') || normal_cb_height_definite)
    });
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
                    own_height_definite,
                    descendant_positioned_cb,
                    fonts,
                    text,
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
            position_subtree(child, viewport, own_padding_box, own_height_definite, descendant_positioned_cb, fonts, text);
            continue;
        }
        let containing_block = if position_kind(child) == PositionKind::Fixed {
            viewport
        } else {
            descendant_positioned_cb
        };
        let units = font_units(child, text);
        let specified_margins =
            values::resolve_margins_with_units(&child.style, units, containing_block.w);
        prepare_stretch(child, containing_block, text);
        layout_child(
            child,
            containing_block.x,
            containing_block.y,
            containing_block.w,
            fonts,
            text,
        );
        child.margin = specified_margins;
        let static_anchor = if matches!(box_.display, crate::Display::Grid) {
            grid_child_static_anchor(box_, child, specified_margins)
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
            padding_box(child),
            fonts,
            text,
        );
    }
    box_.positioned = positioned;
}

fn layout_child(child: &mut LayoutBox, x: f32, y: f32, width: f32,
    fonts: &FontSource, text: Option<&dyn TextSystem>) {
    if let Some(text) = text {
        layout_block_with_text(child, x, y, width, fonts, text);
    } else {
        layout_block(child, x, y, width, fonts);
    }
}

fn font_units(box_: &LayoutBox, text: Option<&dyn TextSystem>) -> values::FontRelativeUnits {
    values::FontRelativeUnits::from_text_system(
        text, box_.font_metric_style.as_ref(), box_.root_font_metric_style.as_ref(), box_.font_size,
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

fn resolve_inset(box_: &LayoutBox, property: &str, base: f32,
    text: Option<&dyn TextSystem>) -> Option<f32> {
    values::style_length_percentage_with_units(&box_.style, property, font_units(box_, text))?
        .resolve(Some(base))
}

fn relative_offset(box_: &LayoutBox, containing_block: Rect, cb_height_definite: bool, text: Option<&dyn TextSystem>) -> (f32, f32) {
    let left = resolve_inset(box_, "left", containing_block.w, text);
    let right = resolve_inset(box_, "right", containing_block.w, text);
    let vertical = |property| {
        let length = values::style_length_percentage_with_units(&box_.style, property, font_units(box_, text))?;
        if length.has_percentage && !cb_height_definite { return None; }
        length.resolve(Some(containing_block.h))
    };
    let top = vertical("top");
    let bottom = vertical("bottom");
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

fn prepare_stretch(box_: &mut LayoutBox, containing_block: Rect, text: Option<&dyn TextSystem>) {
    let left = resolve_inset(box_, "left", containing_block.w, text);
    let right = resolve_inset(box_, "right", containing_block.w, text);
    let top = resolve_inset(box_, "top", containing_block.h, text);
    let bottom = resolve_inset(box_, "bottom", containing_block.h, text);
    let units = font_units(box_, text);
    let margins = values::resolve_margins_with_units(&box_.style, units, containing_block.w);
    let padding = values::resolve_padding_with_units(&box_.style, units, containing_block.w);
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
    text: Option<&dyn TextSystem>,
) -> (f32, f32) {
    let left = resolve_inset(box_, "left", containing_block.w, text);
    let right = resolve_inset(box_, "right", containing_block.w, text);
    let top = resolve_inset(box_, "top", containing_block.h, text);
    let bottom = resolve_inset(box_, "bottom", containing_block.h, text);
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
            .unwrap_or(static_anchor.0 + margins.left.unwrap_or(0.0))
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
            .unwrap_or(static_anchor.0 + margins.left.unwrap_or(0.0))
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
        .unwrap_or(static_anchor.1 + margins.top.unwrap_or(0.0));
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
    let parent_rtl = parent.style.get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let child_rtl = child.style.get("direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("rtl"));
    let choice = |self_property: &str, items_property: &str| {
        child.style.get(self_property)
            .filter(|value| !value.eq_ignore_ascii_case("auto"))
            .or_else(|| parent.style.get(items_property))
            .unwrap_or("normal")
    };
    let horizontal = static_align(choice("justify-self", "justify-items"), parent_rtl, child_rtl, true);
    let vertical = static_align(choice("align-self", "align-items"), parent_rtl, child_rtl, false);
    let x = aligned_border_start(
        parent.content_box.x, parent.content_box.w, child.border_box.w,
        margins.left.unwrap_or(0.0), margins.right.unwrap_or(0.0), horizontal,
    );
    let y = aligned_border_start(
        parent.content_box.y, parent.content_box.h, child.border_box.h,
        margins.top.unwrap_or(0.0), margins.bottom.unwrap_or(0.0), vertical,
    );
    (x, y)
}

fn static_align(value: &str, parent_rtl: bool, child_rtl: bool, horizontal: bool) -> StaticAlign {
    // The safe fallback needs the abspos grid area as its alignment container.
    // Until that rectangle is carried here, use its start edge rather than
    // aligning beyond it when a safe position is requested.
    if value.split_ascii_whitespace().any(|part| part.eq_ignore_ascii_case("safe")) {
        return if horizontal && parent_rtl { StaticAlign::End } else { StaticAlign::Start };
    }
    let keyword = value.split_ascii_whitespace().last().unwrap_or("normal").to_ascii_lowercase();
    let start = |rtl: bool| if horizontal && rtl { StaticAlign::End } else { StaticAlign::Start };
    let end = |rtl: bool| if horizontal && rtl { StaticAlign::Start } else { StaticAlign::End };
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
    container_start + margin_start + match align {
        StaticAlign::Start => 0.0,
        StaticAlign::Center => free / 2.0,
        StaticAlign::End => free,
    }
}

fn shift_box(box_: &mut LayoutBox, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for segment in &mut box_.table_decoration.borders {segment.rect.x+=dx;segment.rect.y+=dy;}
    for hole in &mut box_.table_decoration.background_holes {hole.x+=dx;hole.y+=dy;}
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
            close(find_id(&root, &dom, "self_start").unwrap().border_box.x, x + 80.0);
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
        assert!((parent.border_box.x - parent.content_box.x).abs() < 0.001,
            "inline-block border x={}, content x={}", parent.border_box.x, parent.content_box.x);
        assert!((absolute.border_box.x - parent.content_box.x).abs() < 0.001,
            "absolute x={}, inline-block content x={}", absolute.border_box.x, parent.content_box.x);
        let text = amazing_text::FallbackTextSystem::new();
        let shaped = crate::layout_dom_document_with_text_and_replaced_in_viewport(
            &dom, "", 800.0, Some(600.0), &text, &crate::NoReplacedElements,
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
