//! Phase 3 layout: CSS2.1 §9 box tree + block and inline formatting
//! contexts, producing absolute-pixel geometry for a styled DOM.
//!
//! Pipeline: `amazing_html::parse` → `amazing_css::compute` →
//! [`box_tree::build_layout_tree`] (display classification, font-size
//! resolution, anonymous blocks) → [`block::layout_root`] (box model
//! arithmetic §10.3.3, margin collapsing, inline line layout).
//!
//! Phase 3 subset — implemented:
//! - block formatting context: vertical stacking, auto widths (fill the
//!   containing block), §10.3.3 constraint with `auto`-margin centering,
//!   specified/auto heights, adjacent-sibling margin collapsing (max rule)
//! - inline formatting context: greedy word wrapping, `<br>` breaks,
//!   half-leading line boxes, per-word effective font sizes (nested spans
//!   flattened into the parent's line boxes), and baseline-aligned replaced
//!   images whose intrinsic height can grow a line box
//! - anonymous block boxes for block/inline mixed content (§9.2.1), with
//!   block descendants hoisted out of inline wrappers in document order
//! - box model: margin/padding/border shorthand + longhand resolution,
//!   `border: w s c` shorthand, thin/medium/thick keywords, none/hidden
//!   zeroing; px/pt/pc/in/cm/mm/em/rem/%/unitless-0 lengths
//! - font sizing: absolute-size keyword ladder, em/%/px; line-height
//!   normal/number/length/%; text measurement via [`font::FontSource`]
//!   (real TTF metrics when a font is supplied, heuristic fallback otherwise)
//!
//! Deferred/deviations (documented, revisit in later phases):
//! - floats, absolute/fixed positioning, z-index: not implemented
//! - parent/child margin collapsing is approximated: a first child's
//!   margin-top shrinks against the parent's margin-top by
//!   `max(0, child − parent)` inside the parent (the collapsed excess does
//!   not propagate above the parent's border edge); sibling collapsing is
//!   exact
//! - inline-block/inline-table are flattened like inline elements;
//!   `list-item` renders as block without markers
//! - no min/max-width clamping, no `box-sizing` (content-box always), no
//!   text-align beyond left
//! - text-only line boxes use the block's strut (font + line-height);
//!   replaced images grow the line while per-text-fragment line-height
//!   differences still do not
//!
//! Phase 3.5 additions — flexbox ([`flex`], css-flexbox-1 subset):
//! - `display:flex`/`inline-flex`, flex-direction (all four), flex-wrap,
//!   flex-grow/shrink/basis + `flex` shorthand, order
//! - justify-content (5 values), align-items/align-self, align-content,
//!   gap/row-gap/column-gap, auto-margin absorption on both axes
//! Deviations: baseline alignment ≈ flex-start; min-content floors during
//! shrink are not applied; column-mode non-stretch items keep their
//! hypothetical-measure height; definite-height align-content only.
//!
//! Phase 3.5 additions — grid ([`grid`], css-grid-1 subset): track lists
//! (px/%/auto/min-max-content/fr/repeat incl. auto-fill), auto-flow row |
//! column | dense, line-number + span placement with implicit tracks,
//! content/item alignment; see the module docs for deviations (percent
//! rows resolve against width, no subgrids/named areas).

pub mod block;
pub mod box_tree;
pub mod flex;
pub mod font;
pub mod grid;
pub mod inline;
pub mod values;

use amazing_css::ComputedStyle;
use amazing_dom::NodeIdx;

/// Intrinsic dimensions supplied for replaced content such as `<img>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntrinsicSize {
    pub width: f32,
    pub height: f32,
}

/// Seam through which layout learns replaced-element dimensions without
/// depending on fetchers, decoders, or platform code.
pub trait ReplacedElementProvider {
    fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize>;
}

/// Adapter used by callers that have no decoded replaced content.
pub struct NoReplacedElements;

impl ReplacedElementProvider for NoReplacedElements {
    fn intrinsic_size(&self, _node: NodeIdx) -> Option<IntrinsicSize> {
        None
    }
}

/// Absolute-positioned rectangle in CSS px (border-box or content-box
/// depending on the field).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Layout-relevant display value (everything else was filtered at
/// classification time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    /// Flex container (css-flexbox-1); children become flex items.
    Flex,
    /// Grid container (css-grid-1); children become grid items.
    Grid,
}

/// One box of the layout tree. Spec fields are filled by
/// [`box_tree::build_layout_tree`]; geometry fields by [`block::layout_root`].
#[derive(Debug)]
pub struct LayoutBox {
    /// Origin DOM element; `None` for anonymous blocks (§9.2.1).
    pub node: Option<NodeIdx>,
    pub display: Display,
    /// The element's computed declarations (raw strings; lengths resolve at
    /// layout time when the containing-block width is known).
    pub style: ComputedStyle,
    pub font_size: f32,
    pub line_height: f32,
    pub flow: Flow,
    /// Intrinsic dimensions when this box itself is replaced content.
    pub replaced_size: Option<IntrinsicSize>,

    // --- box model, resolved at layout time ---
    pub margin: values::SidesM,
    pub padding: values::Sides,
    pub border: values::Sides,

    // --- geometry results (absolute px) ---
    /// Border-edge rectangle.
    pub border_box: Rect,
    /// Content-edge rectangle (inside border + padding).
    pub content_box: Rect,
    /// Line boxes for inline containers (empty for block containers).
    pub lines: Vec<LineBox>,
}

impl LayoutBox {
    pub fn new(node: Option<NodeIdx>, display: Display, style: ComputedStyle, font_size: f32, line_height: f32) -> Self {
        LayoutBox {
            node,
            display,
            style,
            font_size,
            line_height,
            flow: Flow::Empty,
            replaced_size: None,
            margin: values::SidesM::default(),
            padding: values::Sides::default(),
            border: values::Sides::default(),
            border_box: Rect::default(),
            content_box: Rect::default(),
            lines: Vec::new(),
        }
    }

    /// Human label for dumps: tag name, `#text`, or `#anon`.
    pub fn label<'d>(&self, dom: &'d amazing_dom::Dom) -> std::borrow::Cow<'d, str> {
        match self.node.and_then(|n| dom.name(n)) {
            Some(name) => std::borrow::Cow::Borrowed(name),
            None => std::borrow::Cow::Borrowed("#anon"),
        }
    }
}

/// How a box's children flow.
#[derive(Debug)]
pub enum Flow {
    Blocks(Vec<LayoutBox>),
    /// Flex items of a `display:flex` container (see [`flex`]).
    Flex(Vec<LayoutBox>),
    /// Grid items of a `display:grid` container (see [`grid`]).
    Grid(Vec<LayoutBox>),
    Inlines(Vec<InlineNode>),
    Empty,
}

/// Flattened inline-level content of an inline formatting context.
#[derive(Debug)]
pub enum InlineNode {
    /// A whitespace-collapsed text run from one DOM text node, rendered at
    /// the effective font size of the inline chain it came from.
    Text { node: NodeIdx, text: String, font_size: f32 },
    /// `<br>` — force a line break.
    Break,
    /// Replaced inline content with dimensions resolved before layout.
    Replaced { node: NodeIdx, size: IntrinsicSize },
}

/// One laid-out line of an inline formatting context.
#[derive(Debug)]
pub struct LineBox {
    /// Top edge of the line box (absolute px).
    pub y_top: f32,
    /// Line box height (the strut: block font + line-height).
    pub height: f32,
    /// Baseline y (absolute px).
    pub baseline: f32,
    pub words: Vec<WordFrag>,
    pub replaced: Vec<ReplacedFrag>,
}

/// One positioned replaced element in an inline formatting context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplacedFrag {
    pub node: NodeIdx,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A placed word (post-wrapping) with its absolute position.
#[derive(Debug)]
pub struct WordFrag {
    pub node: NodeIdx,
    pub text: String,
    pub x: f32,
    /// Width from the font source at this fragment's font size.
    pub width: f32,
    pub font_size: f32,
}

/// Convenience entry point: HTML + CSS in, laid-out root box out.
pub fn layout_document(html: &str, author_css: &str, viewport_width: f32, fonts: &font::FontSource) -> LayoutBox {
    let dom = amazing_html::parse(html);
    layout_dom_document(&dom, author_css, viewport_width, fonts)
}

/// Lays out an already-mutated DOM without reparsing its original source.
///
/// The GUI/DOM bridge uses this seam after inline JavaScript has changed text,
/// attributes, or inline style declarations. Keeping the operation here
/// avoids serializing a live arena back into HTML merely to repaint it.
pub fn layout_dom_document(
    dom: &amazing_dom::Dom,
    author_css: &str,
    viewport_width: f32,
    fonts: &font::FontSource,
) -> LayoutBox {
    layout_dom_document_with_replaced(
        dom,
        author_css,
        viewport_width,
        fonts,
        &NoReplacedElements,
    )
}

/// Lays out a live DOM while consulting an adapter for replaced content.
pub fn layout_dom_document_with_replaced(
    dom: &amazing_dom::Dom,
    author_css: &str,
    viewport_width: f32,
    fonts: &font::FontSource,
    replaced: &dyn ReplacedElementProvider,
) -> LayoutBox {
    let sheet = amazing_css::parse_stylesheet(author_css);
    let styled = amazing_css::compute(&dom, &[sheet]);
    let mut root = box_tree::build_layout_tree_with_replaced(&dom, &styled, fonts, replaced);
    block::layout_root(&mut root, viewport_width, fonts);
    root
}

/// Recursively prints the layout tree (demo output for `amazing layout`).
pub fn dump_layout(dom: &amazing_dom::Dom, box_: &LayoutBox) -> String {
    let mut out = String::new();
    dump_rec(dom, box_, 0, &mut out);
    out
}

fn dump_rec(dom: &amazing_dom::Dom, b: &LayoutBox, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    let bb = &b.border_box;
    let cb = &b.content_box;
    out.push_str(&format!(
        "{} border=({:.0},{:.0} {}x{}) content=({:.0},{:.0} {}x{})",
        b.label(dom),
        bb.x, bb.y, bb.w, bb.h,
        cb.x, cb.y, cb.w, cb.h,
    ));
    if b.padding.top != 0.0 || b.padding.left != 0.0 {
        out.push_str(&format!(
            " pad={},{},{},{}",
            b.padding.top, b.padding.right, b.padding.bottom, b.padding.left
        ));
    }
    if b.border.top != 0.0 || b.border.left != 0.0 {
        out.push_str(&format!(
            " border_w={},{},{},{}",
            b.border.top, b.border.right, b.border.bottom, b.border.left
        ));
    }
    if b.margin.top.is_some() || b.margin.left.is_some() {
        out.push_str(&format!(
            " margin={:?},{:?},{:?},{:?}",
            b.margin.top, b.margin.right, b.margin.bottom, b.margin.left
        ));
    }
    out.push('\n');
    for line in &b.lines {
        for _ in 0..=depth {
            out.push_str("  ");
        }
        out.push_str(&format!(
            "line y={:.1} h={:.1} baseline={:.1}",
            line.y_top, line.height, line.baseline
        ));
        for w in &line.words {
            out.push_str(&format!(" | {:?}@x={:.1} {}px w={:.1}", w.text, w.x, w.font_size, w.width));
        }
        for image in &line.replaced {
            out.push_str(&format!(
                " | <img node={}>@({:.1},{:.1}) {:.1}x{:.1}",
                image.node.0, image.x, image.y, image.width, image.height
            ));
        }
        out.push('\n');
    }
    match &b.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for c in children {
                dump_rec(dom, c, depth + 1, out);
            }
        },
        Flow::Inlines(_) | Flow::Empty => {},
    }
}

#[cfg(test)]
mod replaced_tests {
    use super::*;

    struct OneImage(NodeIdx);

    impl ReplacedElementProvider for OneImage {
        fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize> {
            (node == self.0).then_some(IntrinsicSize { width: 40.0, height: 30.0 })
        }
    }

    fn find<'a>(layout: &'a LayoutBox, node: NodeIdx) -> Option<&'a LayoutBox> {
        if layout.node == Some(node) {
            return Some(layout);
        }
        match &layout.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
                children.iter().find_map(|child| find(child, node))
            },
            Flow::Inlines(_) | Flow::Empty => None,
        }
    }

    #[test]
    fn inline_replaced_content_participates_in_line_geometry() {
        let dom = amazing_html::parse("<body><p>before <img src=x.ppm> after</p></body>");
        let image = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("img"))
            .expect("img node");
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .expect("paragraph node");
        let root = layout_dom_document_with_replaced(
            &dom,
            "",
            300.0,
            &font::FontSource::fallback(),
            &OneImage(image),
        );
        let line = &find(&root, paragraph).expect("paragraph box").lines[0];

        assert_eq!(line.replaced.len(), 1);
        assert_eq!((line.replaced[0].width, line.replaced[0].height), (40.0, 30.0));
        assert!(line.words[0].x < line.replaced[0].x);
        assert!(line.replaced[0].x + 40.0 <= line.words[1].x);
        assert_eq!(line.height, 30.0);
    }

    #[test]
    fn block_replaced_content_uses_intrinsic_box_dimensions() {
        let dom = amazing_html::parse(
            "<body><img src=x.ppm style=\"display:block\"></body>",
        );
        let image = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("img"))
            .expect("img node");
        let root = layout_dom_document_with_replaced(
            &dom,
            "",
            300.0,
            &font::FontSource::fallback(),
            &OneImage(image),
        );
        let image_box = find(&root, image).expect("image layout box");

        assert_eq!(image_box.content_box.w, 40.0);
        assert_eq!(image_box.content_box.h, 30.0);
    }
}
