//! Layout geometry collection: a DOM node → bounding rectangle map built by
//! walking one laid-out [`LayoutBox`] tree.
//!
//! Block/flex/grid boxes carry their own `border_box`. Inline content is
//! flattened by the layout engine, so text geometry comes from each line's
//! placed words and replaced fragments; an inline element with no box of its
//! own falls back to the union of its descendant text rectangles so the
//! accessibility tree still reports a sensible bound.

use amazing_dom::{Dom, NodeIdx};
use amazing_layout::{Flow, InlineNode, LayoutBox, Rect};
use std::collections::HashMap;

/// Bounding rectangle in CSS pixels, relative to the document origin.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct AxRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl AxRect {
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

impl From<Rect> for AxRect {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
        }
    }
}

/// Per-DOM-node bounding boxes for one layout pass.
#[derive(Clone, Debug, Default)]
pub struct BoundsMap {
    rects: HashMap<NodeIdx, AxRect>,
}

impl BoundsMap {
    pub fn get(&self, node: NodeIdx) -> Option<AxRect> {
        self.rects.get(&node).copied()
    }

    pub fn len(&self) -> usize {
        self.rects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    fn union(&mut self, node: NodeIdx, rect: AxRect) {
        if rect.is_empty() && !self.rects.contains_key(&node) {
            self.rects.insert(node, rect);
            return;
        }
        match self.rects.get_mut(&node) {
            Some(existing) => *existing = union(*existing, rect),
            None => {
                self.rects.insert(node, rect);
            }
        }
    }
}

fn union(a: AxRect, b: AxRect) -> AxRect {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    AxRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

/// Collects geometry for every node reachable from `layout`.
pub fn collect_bounds(dom: &Dom, layout: &LayoutBox) -> BoundsMap {
    let mut map = BoundsMap::default();
    visit(dom, layout, &mut map);

    // Elements without their own box (inline elements flattened by layout, and
    // html/head above the layout root) inherit the union of their descendants.
    for node in dom.tree_order(dom_root(dom)) {
        if map.get(node).is_some() {
            continue;
        }
        let mut rect: Option<AxRect> = None;
        for descendant in dom.tree_order(node) {
            if descendant == node {
                continue;
            }
            if let Some(child) = map.get(descendant) {
                rect = Some(match rect {
                    Some(current) => union(current, child),
                    None => child,
                });
            }
        }
        if let Some(rect) = rect {
            map.rects.insert(node, rect);
        }
    }
    map
}

fn dom_root(_dom: &Dom) -> NodeIdx {
    NodeIdx(0)
}

fn visit(_dom: &Dom, layout: &LayoutBox, map: &mut BoundsMap) {
    if let Some(node) = layout.node {
        if !layout.border_box.w.is_nan() {
            map.union(node, layout.border_box.into());
        }
    }
    for line in &layout.lines {
        for word in &line.words {
            // A padding edge carries logical spacing only. It is neither a
            // word nor a physical accessible rectangle (especially vertically).
            if word.text.is_empty() && word.font_size == 0.0 {
                continue;
            }
            map.union(
                word.node,
                AxRect {
                    x: word.x,
                    y: line.y_top,
                    width: word.width,
                    height: line.height,
                },
            );
        }
        for replaced in &line.replaced {
            map.union(
                replaced.node,
                AxRect {
                    x: replaced.x,
                    y: replaced.y,
                    width: replaced.width,
                    height: replaced.height,
                },
            );
        }
        for glyph in &line.glyphs {
            // Glyph `y` is an offset from the baseline; use the line box for
            // the vertical extent so vertical writing modes still report a box.
            map.union(
                glyph.node,
                AxRect {
                    x: glyph.x,
                    y: line.y_top,
                    width: glyph.advance_x.abs().max(glyph.advance_y.abs()),
                    height: line.height,
                },
            );
        }
    }
    match &layout.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                visit(_dom, child, map);
            }
        }
        Flow::Inlines(items) => {
            for item in items {
                match item {
                    InlineNode::Text { node, .. } => {
                        // Exact geometry comes from the placed words above; if
                        // no word was emitted the node keeps its zero box.
                        if !map.rects.contains_key(node) {
                            map.rects.entry(*node).or_default();
                        }
                    }
                    InlineNode::Replaced { node, .. } | InlineNode::Atomic { node, .. } => {
                        if !map.rects.contains_key(node) {
                            map.rects.entry(*node).or_default();
                        }
                    }
                    InlineNode::Ruby { node, columns, .. } => {
                        if !map.rects.contains_key(node) {
                            map.rects.entry(*node).or_default();
                        }
                        for column in columns {
                            if !map.rects.contains_key(&column.base.node) {
                                map.rects.entry(column.base.node).or_default();
                            }
                        }
                    }
                    InlineNode::PaddingEdge { .. }
                    | InlineNode::BidiStart { .. }
                    | InlineNode::BidiEnd
                    | InlineNode::Break => {}
                }
            }
        }
        Flow::Empty => {}
    }
    for child in &layout.positioned {
        visit(_dom, child, map);
    }
}

#[cfg(test)]
mod padding_tests {
    use super::*;

    #[test]
    fn vertical_padding_metadata_does_not_invent_accessible_word_bounds() {
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
            let dom = amazing_html::parse("<body><p><span id=scope>A</span><i id=empty></i></p>");
            let find = |id: &str| {
                (0..dom.node_count())
                    .map(|index| NodeIdx(index as u32))
                    .find(|node| {
                        dom.attrs_of(*node).is_some_and(|attrs| {
                            attrs
                                .iter()
                                .any(|(name, value)| name == "id" && value == id)
                        })
                    })
                    .unwrap()
            };
            let scope = find("scope");
            let empty = find("empty");
            let text = dom.children(scope).next().unwrap();
            let css=format!("body{{margin:0;font:20px/20px serif}}p{{margin:0;writing-mode:{mode};width:100px;height:200px}}span,i{{padding-inline:20%;padding-block:10%}}");
            let layout = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
                &dom,
                &css,
                500.0,
                Some(500.0),
                &amazing_text::FallbackTextSystem::new(),
                &amazing_layout::NoReplacedElements,
            );
            let bounds = collect_bounds(&dom, &layout);
            let descendant = bounds.get(text).unwrap();
            assert!(descendant.width > 0.0 && descendant.height > 0.0, "{mode}");
            assert_eq!(
                bounds.get(scope),
                Some(descendant),
                "{mode}: inline accessible bound must come from actual text"
            );
            assert!(
                bounds.get(empty).is_none(),
                "{mode}: empty padding must not invent word identity/rectangle"
            );
        }
    }
}
