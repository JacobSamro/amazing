//! Box tree construction (CSS2.1 §9.2): classifies elements into block /
//! inline boxes, resolves the top-down font-size chain, groups mixed
//! block+inline content with anonymous blocks, and flattens inline
//! subtrees (spans) into their parent's inline flow.
//!
//! Percentages and `em` lengths are NOT resolved here — margins, padding,
//! borders and widths need the containing-block width, which exists only
//! during layout (`block.rs`). This pass resolves only what is known from
//! the parent chain: display values and font sizes / line heights.

use amazing_css::{ComputedStyle, StyledDom};
use amazing_dom::{Dom, NodeIdx, NodeKind};

use crate::font::FontSource;
use crate::values;
use crate::{Display, Flow, InlineNode, LayoutBox};

/// Builds the root box for a document: the `<body>` element (or `<html>`
/// when body is missing; an empty box otherwise).
pub fn build_layout_tree(dom: &Dom, styled: &StyledDom, fonts: &FontSource) -> LayoutBox {
    let root_style = ComputedStyle::default();
    let start = dom_html(dom)
        .and_then(|html| find_element(dom, html, "body"))
        .or_else(|| dom_html(dom));
    match start {
        Some(idx) => build_element(dom, styled, idx, values::ROOT_FONT_SIZE, fonts),
        None => LayoutBox::new(None, Display::Block, root_style, values::ROOT_FONT_SIZE, 19.2),
    }
}

fn dom_html(dom: &Dom) -> Option<NodeIdx> {
    find_element(dom, NodeIdx(0), "html")
}

/// First element child of `parent` with the given tag name.
fn find_element(dom: &Dom, parent: NodeIdx, name: &str) -> Option<NodeIdx> {
    dom.children(parent)
        .find(|&c| dom.name(c) == Some(name))
}

/// Builds one visible element's box, recursing into its children.
fn build_element(
    dom: &Dom,
    styled: &StyledDom,
    idx: NodeIdx,
    parent_font_px: f32,
    fonts: &FontSource,
) -> LayoutBox {
    let style = styled.get(idx).cloned().unwrap_or_default();
    let font_size =
        values::resolve_font_size(style.get("font-size").unwrap_or("medium"), parent_font_px);
    let normal_lh = fonts.scaled_metrics(font_size).line_height;
    let line_height = values::resolve_line_height(
        style.get("line-height").unwrap_or("normal"),
        font_size,
        normal_lh,
    );
    let display = parse_display(&style);
    let mut box_ = LayoutBox::new(Some(idx), display, style, font_size, line_height);
    box_.flow = classify_children(dom, styled, idx, font_size, fonts);
    box_
}

/// `display` computed value → [`Display`]. Unknown values fall back to
/// CSS's initial `inline`; `none` never reaches here (filtered upstream).
fn parse_display(style: &ComputedStyle) -> Display {
    match style.get("display") {
        Some(v) if v.eq_ignore_ascii_case("block")
            || v.eq_ignore_ascii_case("list-item")
            || v.eq_ignore_ascii_case("table") =>
        {
            Display::Block
        },
        Some(v) if v.eq_ignore_ascii_case("flex")
            || v.eq_ignore_ascii_case("inline-flex")
            || v.eq_ignore_ascii_case("-webkit-flex") =>
        {
            Display::Flex
        },
        Some(v) if v.eq_ignore_ascii_case("grid") || v.eq_ignore_ascii_case("inline-grid") => {
            Display::Grid
        },
        _ => Display::Inline,
    }
}

/// True when the element should not generate any box at all.
fn is_display_none(style: &ComputedStyle) -> bool {
    style.get("display").is_some_and(|v| v.eq_ignore_ascii_case("none"))
}

/// Collapses whitespace runs to single spaces per CSS text processing.
/// (Line-start/end trimming happens later, in the line breaker.)
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for c in text.chars() {
        if c.is_whitespace() {
            in_ws = true;
        } else {
            if in_ws && !out.is_empty() {
                out.push(' ');
            }
            in_ws = false;
            out.push(c);
        }
    }
    out
}

/// Classifies the children of a block container into its [`Flow`],
/// inserting anonymous blocks around runs of inline content (§9.2.1.3).
fn classify_children(
    dom: &Dom,
    styled: &StyledDom,
    idx: NodeIdx,
    font_size: f32,
    fonts: &FontSource,
) -> Flow {
    // Flex/grid containers take every non-`none` element child as an ITEM
    // (css-flexbox-1 §3 / css-grid-1 §3 blockification); contiguous text
    // runs become one anonymous item each.
    let own_display =
        parse_display(&styled.get(idx).cloned().unwrap_or_default());
    if matches!(own_display, Display::Flex | Display::Grid) {
        let mut items: Vec<LayoutBox> = Vec::new();
        let mut run: Vec<InlineNode> = Vec::new();
        for child in dom.children(idx) {
            match &dom.get(child).kind {
                NodeKind::Text { data } => {
                    let collapsed = collapse_whitespace(data);
                    if !collapsed.is_empty() {
                        run.push(InlineNode::Text { node: child, text: collapsed, font_size });
                    }
                },
                NodeKind::Comment { .. } | NodeKind::Document => {},
                NodeKind::Element { .. } => {
                    let child_style = styled.get(child).cloned().unwrap_or_default();
                    if is_display_none(&child_style) {
                        continue;
                    }
                    flush_anonymous_into(&mut items, &mut run, font_size, fonts);
                    items.push(build_element(dom, styled, child, font_size, fonts));
                },
            }
        }
        flush_anonymous_into(&mut items, &mut run, font_size, fonts);
        return if own_display == Display::Grid {
            Flow::Grid(items)
        } else {
            Flow::Flex(items)
        };
    }

    let mut blocks: Vec<LayoutBox> = Vec::new();
    let mut inlines: Vec<InlineNode> = Vec::new();

    for child in dom.children(idx) {
        match &dom.get(child).kind {
            NodeKind::Text { data } => {
                // Whitespace-only runs between block-level children carry no
                // rendering; keep non-empty collapsed runs as separators.
                let collapsed = collapse_whitespace(data);
                if !collapsed.is_empty() {
                    inlines.push(InlineNode::Text { node: child, text: collapsed, font_size });
                }
            },
            NodeKind::Comment { .. } | NodeKind::Document => {},
            NodeKind::Element { .. } => {
                let child_style = styled.get(child).cloned().unwrap_or_default();
                if is_display_none(&child_style) {
                    continue; // no box, no descendants
                }
                match dom.name(child) {
                    // Void line-break element: emits a marker itself (it has
                    // no children, so flatten_inline would see nothing).
                    Some("br") => inlines.push(InlineNode::Break),
                    _ if matches!(
                        parse_display(&child_style),
                        Display::Block | Display::Flex | Display::Grid
                    ) =>
                    {
                        // Flush pending inline content into an anonymous block.
                        flush_anonymous_into(&mut blocks, &mut inlines, font_size, fonts);
                        blocks.push(build_element(dom, styled, child, font_size, fonts));
                    },
                    _ => flatten_inline(
                        dom, styled, child, child_style, font_size,
                        &mut inlines, &mut blocks, fonts,
                    ),
                }
            },
        }
    }
    // Trailing inline content: when the container has no block children it
    // IS an inline formatting context — keep the items directly. Otherwise
    // the remainder becomes one last anonymous block.
    if !inlines.is_empty() && !blocks.is_empty() {
        flush_anonymous_into(&mut blocks, &mut inlines, font_size, fonts);
    }
    match (blocks.is_empty(), inlines.is_empty()) {
        (true, true) => Flow::Empty,
        (true, false) => Flow::Inlines(inlines),
        (false, _) => Flow::Blocks(blocks),
    }
}

/// Moves accumulated inline content into an anonymous block box whose strut
/// comes from the host font size.
fn flush_anonymous_into(
    blocks: &mut Vec<LayoutBox>,
    inlines: &mut Vec<InlineNode>,
    font_size: f32,
    fonts: &FontSource,
) {
    if inlines.is_empty() {
        return;
    }
    let taken = std::mem::take(inlines);
    let lh = fonts.scaled_metrics(font_size).line_height;
    let mut anon = LayoutBox::new(
        None,
        Display::Block,
        ComputedStyle::default(),
        font_size,
        lh,
    );
    anon.flow = Flow::Inlines(taken);
    blocks.push(anon);
}

/// Flattens an inline element subtree into `inlines`. A block descendant
/// stops the flattening: pending inline content is flushed by the caller
/// into an anonymous box and the block joins the sibling list in document
/// order (a simplification of §9.2.1.1 that preserves ordering but not the
/// interleaved splitting of the wrapping inline).
#[allow(clippy::too_many_arguments)]
fn flatten_inline(
    dom: &Dom,
    styled: &StyledDom,
    idx: NodeIdx,
    style: ComputedStyle,
    parent_font_px: f32,
    inlines: &mut Vec<InlineNode>,
    blocks_out: &mut Vec<LayoutBox>,
    fonts: &FontSource,
) {
    let font_size =
        values::resolve_font_size(style.get("font-size").unwrap_or("medium"), parent_font_px);

    for child in dom.children(idx) {
        match &dom.get(child).kind {
            NodeKind::Text { data } => {
                let collapsed = collapse_whitespace(data);
                if !collapsed.is_empty() {
                    inlines.push(InlineNode::Text { node: child, text: collapsed, font_size });
                }
            },
            NodeKind::Comment { .. } | NodeKind::Document => {},
            NodeKind::Element { .. } => {
                let cs = styled.get(child).cloned().unwrap_or_default();
                if is_display_none(&cs) {
                    continue;
                }
                match dom.name(child) {
                    Some("br") => inlines.push(InlineNode::Break),
                    _ if matches!(
                        parse_display(&cs),
                        Display::Block | Display::Flex | Display::Grid
                    ) => {
                        // Hoist: end the current inline run here.
                        flush_anonymous_into(blocks_out, inlines, parent_font_px, fonts);
                        blocks_out.push(build_element(dom, styled, child, parent_font_px, fonts));
                    },
                    _ => flatten_inline(
                        dom, styled, child, cs, font_size,
                        inlines, blocks_out, fonts,
                    ),
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSource;

    /// Builds a tree through the real pipeline for a tiny page.
    fn tree(html: &str) -> (amazing_dom::Dom, LayoutBox) {
        let dom = amazing_html::parse(html);
        let styled = amazing_css::compute(&dom, &[]);
        let t = build_layout_tree(&dom, &styled, &FontSource::fallback());
        (dom, t)
    }

    fn tag_of(b: &LayoutBox, dom: &Dom) -> String {
        b.node.and_then(|n| dom.name(n)).unwrap_or("#anon").to_string()
    }

    #[test]
    fn body_is_root_with_block_children() {
        let (dom, root) = tree("<body><p>a</p><div>b</div></body>");
        assert_eq!(tag_of(&root, &dom), "body");
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        assert_eq!(kids.len(), 2);
        assert_eq!(tag_of(&kids[0], &dom), "p");
        assert_eq!(tag_of(&kids[1], &dom), "div");
    }

    #[test]
    fn mixed_content_makes_anonymous_blocks() {
        let (dom, root) = tree("<body>text <b>bold</b><p>para</p>after</body>");
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        // anon(text bold) | p | anon(after)
        assert_eq!(kids.len(), 3);
        assert_eq!(tag_of(&kids[0], &dom), "#anon");
        assert_eq!(tag_of(&kids[1], &dom), "p");
        assert_eq!(tag_of(&kids[2], &dom), "#anon");
        let Flow::Inlines(items) = &kids[0].flow else { panic!("anon wants inlines") };
        assert_eq!(items.len(), 2); // "text " + "bold"
    }

    #[test]
    fn inter_block_whitespace_vanishes() {
        let (_, root) = tree("<body>\n  <p>a</p>\n  <p>b</p>\n</body>");
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        assert_eq!(kids.len(), 2); // no anonymous boxes from indentation
    }

    #[test]
    fn display_none_subtree_skipped() {
        // A whole display:none subtree generates no box. (`<div>` inside
        // `<p>` can't be used here: the tree builder auto-closes the `<p>`
        // before a block start tag, making the div a SIBLING per spec.)
        let (dom, root) = tree(
            "<body><p>a</p><div style=\"display:none\"><p>x</p><span>y</span></div><p>c</p></body>",
        );
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        assert_eq!(kids.len(), 2);
        assert_eq!(tag_of(&kids[0], &dom), "p");
        assert_eq!(tag_of(&kids[1], &dom), "p");
    }

    #[test]
    fn nested_spans_flatten_to_words_at_own_font_size() {
        let (dom, root) = tree("<body><p>a <span style=\"font-size:32px\">big</span></p></body>");
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        assert_eq!(tag_of(&kids[0], &dom), "p");
        let Flow::Inlines(items) = &kids[0].flow else { panic!("p wants inlines") };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text_text(), ("a", 16.0));
        assert_eq!(items[1].text_text(), ("big", 32.0));
    }

    impl InlineNode {
        fn text_text(&self) -> (&str, f32) {
            match self {
                InlineNode::Text { text, font_size, .. } => (text.as_str(), *font_size),
                InlineNode::Break => ("<br>", 0.0),
            }
        }
    }

    #[test]
    fn block_inside_inline_hoists_in_order() {
        let (dom, root) = tree("<body><span>one<div>two</div>three</span></body>");
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        assert_eq!(kids.len(), 3);
        assert_eq!(tag_of(&kids[0], &dom), "#anon"); // "one"
        assert_eq!(tag_of(&kids[1], &dom), "div");
        assert_eq!(tag_of(&kids[2], &dom), "#anon"); // "three"
    }

    #[test]
    fn font_size_em_chain_compounds() {
        // 16px body → div 200% = 32px → p inside inherits 32, em = 0.5 → 16
        let (_, root) = tree(
            "<body><div style=\"font-size:200%\"><p style=\"font-size:.5em\">x</p></div></body>",
        );
        let Flow::Blocks(divs) = &root.flow else { panic!("want blocks") };
        assert!((divs[0].font_size - 32.0).abs() < 1e-4);
        let Flow::Blocks(ps) = &divs[0].flow else { panic!("div wants blocks") };
        assert!((ps[0].font_size - 16.0).abs() < 1e-4);
    }

    #[test]
    fn br_becomes_break_marker() {
        let (_, root) = tree("<body><p>a<br>b</p></body>");
        let Flow::Blocks(kids) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &kids[0].flow else { panic!("p wants inlines") };
        assert!(matches!(items[1], InlineNode::Break));
    }
}
