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
//!   flattened into the parent's line boxes)
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
//!   text-align beyond left, no replaced elements (img) yet
//! - line boxes use the block's strut (font + line-height) for height and
//!   baseline; per-fragment line-height differences don't grow lines

pub mod block;
pub mod box_tree;
pub mod font;
pub mod inline;
pub mod values;

use amazing_css::ComputedStyle;
use amazing_dom::NodeIdx;

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
    let sheet = amazing_css::parse_stylesheet(author_css);
    let styled = amazing_css::compute(&dom, &[sheet]);
    let mut root = box_tree::build_layout_tree(&dom, &styled, fonts);
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
        out.push('\n');
    }
    match &b.flow {
        Flow::Blocks(children) => {
            for c in children {
                dump_rec(dom, c, depth + 1, out);
            }
        },
        Flow::Inlines(_) | Flow::Empty => {},
    }
}
