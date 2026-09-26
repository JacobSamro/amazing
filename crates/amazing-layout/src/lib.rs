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
//!   specified/auto/min/max heights; signed adjoining-margin collapse,
//!   collapse-through empty blocks, and BFC trailing-margin containment
//! - left/right floats, BFC exclusion bands, side-specific clearance and
//!   float containment in auto heights
//! - inline formatting context: greedy word wrapping, `<br>` breaks,
//!   half-leading line boxes, per-word effective font sizes (nested spans
//!   flattened into the parent's line boxes), and baseline-aligned replaced
//!   images whose intrinsic height can grow a line box
//! - anonymous block boxes for block/inline mixed content (§9.2.1), with
//!   block descendants hoisted out of inline wrappers in document order
//! - post-normal-flow positioning: relative subtree offsets; absolute/fixed
//!   descendants excluded from block/Flex/Grid sizing; nearest positioned
//!   padding-box and viewport containing blocks; fixed/percentage/negative
//!   insets, dual-inset stretching, and LTR/RTL over-constraint selection
//! - box model: margin/padding/border shorthand + longhand resolution,
//!   `border: w s c` shorthand, thin/medium/thick keywords, none/hidden
//!   zeroing; px/pt/pc/in/cm/mm/em/rem/%/unitless-0 lengths
//! - font sizing: absolute-size keyword ladder, em/%/px; line-height
//!   normal/number/length/%; provider-neutral paragraph shaping through
//!   `amazing-text`, with source clusters and positioned glyph ids retained for
//!   paint. [`font::FontSource`] remains a legacy compatibility path.
//!
//! Deferred/deviations (documented, revisit in later phases):
//! - floats interrupt inline runs at anonymous-block boundaries; resuming
//!   the same source line around a mid-inline float remains deferred. Float
//!   shrink-to-fit uses the existing legacy intrinsic-width approximation,
//!   and vertical writing-mode exclusion is not yet implemented.
//! - sticky constraints, z-index/stacking contexts, transforms as
//!   containing-block creators, and exact inline/Flex/Grid static-position
//!   rectangles are not implemented
//! - parent/child margin propagation is approximated: a first child's
//!   margin-top shrinks against the parent's margin-top by
//!   `max(0, child − parent)` inside the parent (the collapsed excess does
//!   not propagate above the parent's border edge); sibling signed collapse
//!   and direct empty-block collapse-through are modeled
//! - non-replaced inline-table outer flow remains blockified;
//!   table internals use CSS table formatting; `list-item` renders without markers
//! - no min/max-width clamping or intrinsic width keywords, no `box-sizing`
//!   (content-box always), no text-align beyond left
//! - text-only line boxes use the block's strut (font + line-height);
//!   replaced images grow the line while per-text-fragment line-height
//!   differences still do not
//!
//! Phase 3.5 additions — flexbox ([`flex`], css-flexbox-1 subset):
//! - `display:flex`/`inline-flex`, flex-direction (all four), flex-wrap,
//!   flex-grow/shrink/basis + `flex` shorthand, order
//! - justify-content (5 values), align-items/align-self, align-content,
//!   gap/row-gap/column-gap, auto-margin absorption on both axes
//!
//! Deviations: baseline alignment ≈ flex-start; column wrapping,
//! wrap-reverse cross-start, transferred aspect ratios, and inline outer
//! display remain deferred.
//!
//! Phase 3.5 additions — grid ([`grid`], css-grid-1 subset): track lists
//! (px/%/auto/min-max-content/minmax/fr/repeat incl. auto-fill), auto-flow row
//! | column | dense, numeric/named-line placement with implicit tracks, and
//! content/item alignment; see the module docs for remaining template-area,
//! auto-fit, intrinsic-spanning, subgrid, and Grid-Lanes deviations.

pub mod block;
pub mod box_tree;
mod table_tree;
mod table;
pub mod table_decoration;
pub mod flex;
mod flex_algorithm;
#[cfg(test)]
mod float_tests;
#[cfg(test)]
mod inline_replaced_tests;
#[cfg(test)]
mod svg_tests;

mod floats;
pub mod font;
mod font_features;
pub mod grid;
mod grid_algorithm;
pub mod inline;
pub mod position;
mod position_constraints;
pub mod replaced;
pub mod retained;
mod svg;
pub mod values;

use amazing_css::ComputedStyle;
use amazing_dom::NodeIdx;
use amazing_text::{FontKey, GlyphId, TextSystem};
use std::ops::Range;

/// Intrinsic dimensions supplied for replaced content such as `<img>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntrinsicSize {
    pub width: f32,
    pub height: f32,
}

/// Seam through which layout learns NATURAL content dimensions without
/// depending on fetchers, decoders, or platform code. Never return dimensions
/// adjusted by HTML width/height attributes, CSS sizes, padding or borders;
/// layout owns used-size resolution and needs the original image aspect ratio.
pub trait ReplacedElementProvider {
    fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize>;

    /// Natural dimensions of a CSS generated image, keyed by its resolved URL.
    fn intrinsic_size_by_url(&self, _url: &str) -> Option<IntrinsicSize> {
        None
    }
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
    /// Source-order key for synthetic boxes.
    pub source_order: Option<u64>,
    /// Last in-flow source slot, including anonymous inline runs.
    pub(crate) source_end_order: Option<u64>,
    pub(crate) generated_pseudo: Option<amazing_css::selectors::PseudoElement>,
    /// Computed ::first-line style; applied to the first formatted line.
    pub first_line_style: Option<ComputedStyle>,
    /// A list marker outside the principal list-item box.
    pub(crate) outside_marker: bool,
    pub display: Display,
    /// The element's computed declarations (raw strings; lengths resolve at
    /// layout time when the containing-block width is known).
    pub style: ComputedStyle,
    pub font_size: f32,
    pub line_height: f32,
    /// Computed face request used for font-relative CSS lengths on this box.
    pub(crate) font_metric_style: Option<amazing_text::TextStyle>,
    /// Root element's face request for rch/ric/rex/rcap.
    pub(crate) root_font_metric_style: Option<amazing_text::TextStyle>,
    pub flow: Flow,
    /// Absolute/fixed descendants removed from this box's formatting context.
    /// They are laid out by the post-normal-flow positioning pass.
    pub positioned: Vec<LayoutBox>,
    /// An in-flow atomic inline kept in `positioned` for tree traversal.
    pub(crate) inline_atomic: bool,
    /// Intrinsic dimensions when this box itself is replaced content.
    pub replaced_size: Option<IntrinsicSize>,
    /// Outer SVG root for a block/flex/grid item. Its resolved viewport is
    /// `content_box`; inline SVGs are exposed by `styled_replaced_fragments`.
    pub svg_root: Option<NodeIdx>,
    pub(crate) svg_sizing: Option<svg::SvgSizing>,
    pub(crate) table_decoration: table_decoration::TableDecoration,
    pub(crate) table_wrapper: bool,
    /// CSS2 HTML tables use the legacy border-edge width convention.
    pub(crate) html_table_width: bool,
    pub(crate) table_colspan: usize,
    pub(crate) table_rowspan: usize,
    /// Definite containing-block height, distinct from content-derived height.
    pub(crate) containing_block_height: Option<f32>,
    /// Direction of the normal containing block, for physical width constraints.
    pub(crate) containing_block_rtl: bool,
    /// Available physical height for orthogonal auto sizing; unlike a definite
    /// containing-block height, this does not resolve percentage heights.
    pub(crate) available_height: Option<f32>,
    /// Parent grid tracks inherited by a subgrid in either axis.
    pub(crate) grid_subgrid: Option<grid::SubgridContext>,
    /// Resolved track geometry of this box when it is a grid/grid-lanes
    /// container, used by absolutely-positioned descendants' static position.
    pub(crate) grid_tracks: Option<grid::GridTracks>,

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
    pub(crate) float_context: floats::FloatContext,
}

impl LayoutBox {
    pub fn new(
        node: Option<NodeIdx>,
        display: Display,
        style: ComputedStyle,
        font_size: f32,
        line_height: f32,
    ) -> Self {
        LayoutBox {
            node,
            source_order: None,
            source_end_order: None,
            generated_pseudo: None,
            first_line_style: None,
            outside_marker: false,
            display,
            style,
            font_size,
            line_height,
            font_metric_style: None,
            root_font_metric_style: None,
            flow: Flow::Empty,
            positioned: Vec::new(),
            inline_atomic: false,
            replaced_size: None,
            svg_root: None,
            svg_sizing: None,
            table_decoration: Default::default(),
            table_wrapper: false,
            html_table_width: false,
            table_colspan: 1,
            table_rowspan: 1,
            containing_block_height: None,
            containing_block_rtl: false,
            available_height: None,
            grid_subgrid: None,
            grid_tracks: None,
            margin: values::SidesM::default(),
            padding: values::Sides::default(),
            border: values::Sides::default(),
            border_box: Rect::default(),
            content_box: Rect::default(),
            lines: Vec::new(),
            float_context: floats::FloatContext::default(),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSourceMapping {
    /// Byte range in the transformed `InlineNode::Text::text` value.
    pub text_range: Range<usize>,
    /// Original DOM text node owning `source_range`.
    pub source: NodeIdx,
    /// Byte range in the original DOM text node.
    pub source_range: Range<usize>,
}

/// CSS tab stop spacing, resolved independently of the shaping provider.
#[derive(Debug, Clone, Copy)]
pub enum TabSize { Spaces(f32), Length(f32) }
impl Default for TabSize { fn default() -> Self { Self::Spaces(8.0) } }
impl TabSize {
    pub(crate) fn advance(self, position: f32, space: f32) -> f32 {
        let stop = match self { Self::Spaces(n) => n * space, Self::Length(px) => px };
        if stop <= 0.0 { return 0.0; }
        (stop - position.max(0.0).rem_euclid(stop)).max(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct RubyFormat {
    pub tab_size: TabSize,
    pub align: String,
    pub position: String,
    pub visibility: String,
    pub merge: bool,
    pub minimum_advance: f32,
}
impl Default for RubyFormat {
    fn default() -> Self {
        Self { tab_size: TabSize::default(), align: "space-around".into(), position: "alternate".into(),
            visibility: "visible".into(), merge: false, minimum_advance: 0.0 }
    }
}

#[derive(Debug, Clone)]
pub struct NestedRuby {
    pub range: Range<usize>,
    pub columns: Vec<RubyColumn>,
    pub overhang_spaces: bool,
}

#[derive(Debug, Clone)]
pub struct RubyAnnotation {
    pub text: RubyText,
    pub span: usize,
    pub level: usize,
    pub hidden: bool,
}

#[derive(Debug, Clone)]
pub struct RubyText {
    pub node: NodeIdx,
    /// Raw content is retained for auto-hiding before whitespace processing.
    pub raw_text: String,
    pub format: RubyFormat,
    pub nested: Vec<NestedRuby>,
    pub text: String,
    pub source_mappings: Vec<TextSourceMapping>,
    pub font_size: f32,
    pub style: Box<amazing_text::TextStyle>,
    pub color: String,
}

#[derive(Debug, Clone)]
pub struct RubyColumn {
    pub base: RubyText,
    pub annotation: Option<RubyText>,
    pub annotation_span: usize,
    pub annotation_hidden: bool,
    /// Higher annotation levels; the legacy fields above represent level zero.
    pub additional_annotations: Vec<RubyAnnotation>,
    pub intra_level_whitespace: bool,
}

#[derive(Debug, Clone)]
pub enum InlineNode {
    /// A whitespace-collapsed text run from one DOM text node, rendered at
    /// the effective font size of the inline chain it came from.
    Text {
        node: NodeIdx,
        source_order: Option<u64>,
        text: String,
        source_mappings: Vec<TextSourceMapping>,
        font_size: f32,
        style: Box<amazing_text::TextStyle>,
        /// Used CSS letter spacing in px; applied between visual character units.
        letter_spacing: f32,
        /// Used CSS word spacing; the percentage is relative to the current font size.
        word_spacing: values::LengthPercentage,
        tab_size: TabSize,
        color: String,
        background_color: String,
        inline_padding: values::Sides,
        boundary_before: amazing_text::ShapingBoundary,
        boundary_after: amazing_text::ShapingBoundary,
    },
    /// Paired CSS/HTML bidi scope markers retained through nested inline
    /// flattening, including around `<br>` and atomic content.
    BidiStart {
        direction: amazing_text::Direction,
        unicode_bidi: amazing_text::UnicodeBidi,
    },
    BidiEnd,
    /// Paired ruby bases and annotations retained as breakable column groups.
    Ruby {
        node: NodeIdx,
        columns: Vec<RubyColumn>,
        /// CSS Ruby 1: annotations may overlap adjacent preserved spaces.
        overhang_spaces: bool,
    },
    /// `<br>` — force a line break.
    Break,
    /// Atomic inline content. `size` is the resolved outer size for fitting.
    Replaced {
        node: NodeIdx,
        size: IntrinsicSize,
        replaced: Box<replaced::InlineReplaced>,
    },
    /// An inline formatting-context placeholder for a flex/grid subtree.
    Atomic { node: NodeIdx, size: IntrinsicSize, baseline_offset: f32, vertical_align: String },
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
    /// Positioned glyphs produced by the page's text adapter. Paint consumes
    /// these ids and advances directly; it never measures `words` again.
    pub glyphs: Vec<GlyphFrag>,
    pub decorations: Vec<InlineDecorationFrag>,
    pub replaced: Vec<ReplacedFrag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InlineDecorationFrag {
    pub node: NodeIdx,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphFrag {
    pub node: NodeIdx,
    pub font: FontKey,
    pub glyph: GlyphId,
    pub cluster: Range<usize>,
    /// Byte range in the original DOM text node represented by this cluster.
    pub source_range: Range<usize>,
    /// Every original DOM range participating in this cluster. A ligature or
    /// collapsed space can map to more than one text node.
    pub source_mappings: Vec<amazing_text::SourceMapping>,
    pub x: f32,
    /// Inline-axis offset from the line baseline for vertical runs.
    pub y: f32,
    pub advance_x: f32,
    pub advance_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub font_size: f32,
    pub writing_mode: amazing_text::WritingMode,
    pub orientation: amazing_text::GlyphOrientation,
    pub scale_x: f32,
    pub scale_y: f32,
    pub variations: Vec<amazing_text::VariationSetting>,
    pub synthesis: amazing_text::FontSynthesis,
    pub color: String,
}

/// Content rectangle of one positioned replaced inline. Decoration geometry
/// and computed style are available through `LayoutBox::styled_replaced_fragments`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplacedFrag {
    pub node: NodeIdx,
    /// Identity of a generated image; `None` for the element's own content.
    pub source_order: Option<u64>,
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
pub fn layout_document(
    html: &str,
    author_css: &str,
    viewport_width: f32,
    fonts: &font::FontSource,
) -> LayoutBox {
    let dom = amazing_html::parse(html);
    layout_dom_document(&dom, author_css, viewport_width, fonts)
}

/// Shaped-text entry point. This is the provider-neutral path used by the
/// browser driver; the legacy `FontSource` entry points remain compatibility
/// adapters during ticket #4a.
pub fn layout_document_with_text(
    html: &str,
    author_css: &str,
    viewport_width: f32,
    text: &dyn TextSystem,
) -> LayoutBox {
    let dom = amazing_html::parse(html);
    layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        author_css,
        viewport_width,
        None,
        text,
        &NoReplacedElements,
    )
}

pub fn layout_dom_document_with_text_and_replaced_in_viewport(
    dom: &amazing_dom::Dom,
    author_css: &str,
    viewport_width: f32,
    viewport_height: Option<f32>,
    text: &dyn TextSystem,
    replaced: &dyn ReplacedElementProvider,
) -> LayoutBox {
    let sheet = amazing_css::parse_stylesheet(author_css);
    let styled = amazing_css::compute(dom, &[sheet]);
    layout_styled_document(dom, &styled, text, replaced, viewport_width, viewport_height)
}

/// Lays out a document whose styles have already been computed by the caller.
/// This is the entry point for animation and incremental style updates.
pub fn layout_styled_document(
    dom: &amazing_dom::Dom,
    styled: &amazing_css::StyledDom,
    text: &dyn TextSystem,
    replaced: &dyn ReplacedElementProvider,
    viewport_width: f32,
    viewport_height: Option<f32>,
) -> LayoutBox {
    let fonts = font::FontSource::fallback();
    let mut root = box_tree::build_layout_tree_with_replaced(dom, styled, &fonts, replaced);
    block::layout_root_with_text(&mut root, viewport_width, viewport_height, text);
    root
}

/// Sheet-preserving counterpart of the string entry point. Callers retain
/// document order and stylesheet owner scope roots through style computation.
/// Keep the same fallback layout and shaped-text pass as the string path.
pub fn layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport(
    dom: &amazing_dom::Dom,
    author_sheets: &[amazing_css::Stylesheet],
    viewport_width: f32,
    viewport_height: Option<f32>,
    text: &dyn TextSystem,
    replaced: &dyn ReplacedElementProvider,
) -> LayoutBox {
    let styled = amazing_css::compute(dom, author_sheets);
    layout_styled_document(dom, &styled, text, replaced, viewport_width, viewport_height)
}

fn apply_shaped_text(layout: &mut LayoutBox, text: &dyn TextSystem) {
    for child in layout.positioned.iter_mut().filter(|child| child.inline_atomic) {
        apply_shaped_text(child, text);
    }
    block::refresh_atomic_inline_metrics(layout);
    if let Flow::Inlines(items) = &layout.flow {
        let first_style = layout.font_metric_style.as_ref().or_else(|| items.iter().find_map(|item| match item {
            InlineNode::Text { style, .. } => Some(style.as_ref()),
            InlineNode::Ruby { columns, .. } => columns.first().map(|column| column.base.style.as_ref()),
            InlineNode::BidiStart { .. }
            | InlineNode::BidiEnd
            | InlineNode::Break
            | InlineNode::Replaced { .. }
            | InlineNode::Atomic { .. } => None,
        }));
        if let Some(style) = first_style
            && let Ok(metrics) = text.metrics(style)
        {
            let line_height = if layout
                .style
                .get("line-height")
                .is_none_or(|value| value.eq_ignore_ascii_case("normal"))
            {
                metrics.line_height().max(style.size * 1.2)
            } else {
                layout.line_height
            };
            let strut = inline::Strut {
                font_size: style.size,
                line_height,
                ascent: metrics.ascent,
                descent: metrics.descent,
                vertical_ascent: metrics.vertical_ascent,
                vertical_descent: metrics.vertical_descent,
            };
            let base_direction = if style.writing_mode.is_vertical_flow()
                && style.text_orientation == amazing_text::TextOrientation::Upright
            {
                amazing_text::Direction::LeftToRight
            } else if layout
                .style
                .get("direction")
                .is_some_and(|value| value.eq_ignore_ascii_case("rtl"))
            {
                amazing_text::Direction::RightToLeft
            } else {
                amazing_text::Direction::LeftToRight
            };
            if let Some(lines) = inline::layout_lines_shaped(
                items,
                inline::InlineContainingBlock {
                    x: layout.content_box.x,
                    y: layout.content_box.y,
                    width: layout.content_box.w,
                    height: layout.content_box.h,
                },
                &strut,
                base_direction,
                style.writing_mode,
                text,
                &layout.float_context,
                !layout.style.get("white-space").is_some_and(|value| {
                    value.eq_ignore_ascii_case("nowrap") || value.eq_ignore_ascii_case("pre")
                }),
                layout.first_line_style.as_ref().and_then(|style| style.get("ruby-position")),
                layout.style.get("text-align").unwrap_or("start"),
                layout.style.get("text-align-last").unwrap_or("auto"),
                layout.style.get("unicode-bidi") == Some("plaintext"),
            ) {
                let old_height = if style.writing_mode.is_vertical_flow() {
                    layout.lines.iter().map(|line| line.height).fold(0.0, f32::max)
                } else { layout.lines.iter().map(|line| line.height).sum::<f32>() };
                let new_height = if style.writing_mode.is_vertical_flow() {
                    lines.iter().map(|line| line.height).fold(0.0, f32::max)
                } else {
                    lines.iter().map(|line| line.height).sum::<f32>()
                };
                if layout
                    .style
                    .get("height")
                    .is_none_or(|value| value.eq_ignore_ascii_case("auto"))
                {
                    let delta = new_height - old_height;
                    layout.content_box.h = (layout.content_box.h + delta).max(0.0);
                    layout.border_box.h = (layout.border_box.h + delta).max(0.0);
                }
                layout.line_height = line_height;
                layout.lines = lines;
                if let Some(color) = layout.first_line_style.as_ref().and_then(|style| style.get("color")) {
                    if let Some(first) = layout.lines.first_mut() {
                        for glyph in &mut first.glyphs { glyph.color = color.to_owned(); }
                    }
                }
            }
        }
    }
    match &mut layout.flow {
        Flow::Blocks(children) => {
            let mut shift = 0.0;
            for (index, child) in children.iter_mut().enumerate() {
                if index == 0 && child.first_line_style.is_none() {
                    child.first_line_style = layout.first_line_style.clone();
                }
                if shift != 0.0 {
                    flex::translate(child, 0.0, shift);
                }
                let old_height = child.border_box.h;
                apply_shaped_text(child, text);
                shift += child.border_box.h - old_height;
            }
            if shift != 0.0
                && layout
                    .style
                    .get("height")
                    .is_none_or(|value| value.eq_ignore_ascii_case("auto"))
            {
                layout.content_box.h = (layout.content_box.h + shift).max(0.0);
                layout.border_box.h = (layout.border_box.h + shift).max(0.0);
            }
        }
        Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                apply_shaped_text(child, text);
            }
        }
        Flow::Inlines(_) | Flow::Empty => {}
    }
    for child in &mut layout.positioned {
        if !child.inline_atomic {
            apply_shaped_text(child, text);
        }
    }
    block::place_atomic_inlines(layout);
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
    layout_dom_document_with_replaced(dom, author_css, viewport_width, fonts, &NoReplacedElements)
}

/// Lays out a live DOM with an optional definite viewport block size. Fixed
/// positioning uses this height; ordinary document layout may leave it
/// indefinite and fall back to the normal-flow page height.
pub fn layout_dom_document_in_viewport(
    dom: &amazing_dom::Dom,
    author_css: &str,
    viewport_width: f32,
    viewport_height: Option<f32>,
    fonts: &font::FontSource,
) -> LayoutBox {
    layout_dom_document_with_replaced_in_viewport(
        dom,
        author_css,
        viewport_width,
        viewport_height,
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
    layout_dom_document_with_replaced_in_viewport(
        dom,
        author_css,
        viewport_width,
        None,
        fonts,
        replaced,
    )
}

pub fn layout_dom_document_with_replaced_in_viewport(
    dom: &amazing_dom::Dom,
    author_css: &str,
    viewport_width: f32,
    viewport_height: Option<f32>,
    fonts: &font::FontSource,
    replaced: &dyn ReplacedElementProvider,
) -> LayoutBox {
    let sheet = amazing_css::parse_stylesheet(author_css);
    let styled = amazing_css::compute(&dom, &[sheet]);
    let mut root = box_tree::build_layout_tree_with_replaced(&dom, &styled, fonts, replaced);
    block::layout_root_in_viewport(&mut root, viewport_width, viewport_height, fonts);
    root
}

/// Bottom edge of scrollable layout overflow. Absolute descendants contribute;
/// fixed descendants stay viewport-attached and do not extend document height.
pub fn layout_overflow_bottom(layout: &LayoutBox) -> f32 {
    let mut bottom =
        layout.border_box.y + layout.border_box.h + layout.margin.bottom.unwrap_or(0.0);
    for line in &layout.lines {
        bottom = bottom.max(line.y_top + line.height);
    }
    match &layout.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                bottom = bottom.max(layout_overflow_bottom(child));
            }
        }
        Flow::Inlines(_) | Flow::Empty => {}
    }
    for child in &layout.positioned {
        let fixed = child
            .style
            .get("position")
            .is_some_and(|value| value.eq_ignore_ascii_case("fixed"));
        if !fixed {
            bottom = bottom.max(layout_overflow_bottom(child));
        }
    }
    bottom
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
        bb.x,
        bb.y,
        bb.w,
        bb.h,
        cb.x,
        cb.y,
        cb.w,
        cb.h,
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
            out.push_str(&format!(
                " | {:?}@x={:.1} {}px w={:.1}",
                w.text, w.x, w.font_size, w.width
            ));
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
        }
        Flow::Inlines(_) | Flow::Empty => {}
    }
    for child in &b.positioned {
        dump_rec(dom, child, depth + 1, out);
    }
}

#[cfg(test)]
mod replaced_tests {
    use super::*;

    struct OneImage(NodeIdx);

    impl ReplacedElementProvider for OneImage {
        fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize> {
            (node == self.0).then_some(IntrinsicSize {
                width: 40.0,
                height: 30.0,
            })
        }
    }

    fn find<'a>(layout: &'a LayoutBox, node: NodeIdx) -> Option<&'a LayoutBox> {
        if layout.node == Some(node) {
            return Some(layout);
        }
        match &layout.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
                children.iter().find_map(|child| find(child, node))
            }
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
        assert_eq!(
            (line.replaced[0].width, line.replaced[0].height),
            (40.0, 30.0)
        );
        assert!(line.words[0].x < line.replaced[0].x);
        assert!(line.replaced[0].x + 40.0 <= line.words[1].x);
        assert_eq!(line.height, 30.0);
    }

    #[test]
    fn block_replaced_content_uses_intrinsic_box_dimensions() {
        let dom = amazing_html::parse("<body><img src=x.ppm style=\"display:block\"></body>");
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

#[cfg(test)]
mod shaped_text_tests {
    use super::*;
    use amazing_text::{
        FallbackTextSystem, FontMetrics, GlyphImage, GlyphRasterRequest, LineRequest, Paragraph,
        ShapedLine, ShapedParagraph, TextError,
    };

    struct VisualLineFake(FallbackTextSystem);

    struct AtomicOrderFake(FallbackTextSystem);

    struct RelativeMetricsFake(FallbackTextSystem);

    impl TextSystem for RelativeMetricsFake {
        fn generation(&self) -> u64 { self.0.generation() }
        fn metrics(&self, style: &amazing_text::TextStyle) -> Result<FontMetrics, TextError> {
            self.0.metrics(style)
        }
        fn font_relative_metrics(&self, style: &amazing_text::TextStyle) -> amazing_text::FontRelativeMetrics {
            let mut metrics = self.0.font_relative_metrics(style);
            if style.families.first().is_some_and(|family| family.contains("Metric Face")) {
                metrics.ch_advance = 100.0;
                metrics.ic_advance = 80.0;
                metrics.x_height = 60.0;
                metrics.cap_height = 70.0;
            } else {
                metrics.ic_advance = 17.0;
            }
            metrics
        }
        fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
            self.0.shape(paragraph)
        }
        fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError> {
            self.0.shape_line(request)
        }
        fn rasterize(&self, request: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError> {
            self.0.rasterize(request)
        }
    }

    impl TextSystem for AtomicOrderFake {
        fn generation(&self) -> u64 { self.0.generation() }
        fn metrics(&self, style: &amazing_text::TextStyle) -> Result<FontMetrics, TextError> {
            self.0.metrics(style)
        }
        fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
            self.0.shape(paragraph)
        }
        fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError> {
            let mut line = self.0.shape_line(request)?;
            line.runs.reverse();
            line.visual_clusters.reverse();
            Ok(line)
        }
        fn rasterize(
            &self,
            request: GlyphRasterRequest<'_>,
        ) -> Result<Option<GlyphImage>, TextError> {
            self.0.rasterize(request)
        }
    }

    struct OneAtomic(NodeIdx);

    impl ReplacedElementProvider for OneAtomic {
        fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize> {
            (node == self.0).then_some(IntrinsicSize {
                width: 10.0,
                height: 8.0,
            })
        }
    }

    impl TextSystem for VisualLineFake {
        fn generation(&self) -> u64 {
            self.0.generation()
        }

        fn metrics(&self, style: &amazing_text::TextStyle) -> Result<FontMetrics, TextError> {
            self.0.metrics(style)
        }

        fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
            self.0.shape(paragraph)
        }

        fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError> {
            let mut line = self.0.shape_line(request)?;
            line.runs.swap(2, 3);
            line.visual_clusters.swap(2, 3);
            Ok(line)
        }

        fn rasterize(
            &self,
            request: GlyphRasterRequest<'_>,
        ) -> Result<Option<GlyphImage>, TextError> {
            self.0.rasterize(request)
        }
    }

    fn find<'a>(layout: &'a LayoutBox, node: NodeIdx) -> Option<&'a LayoutBox> {
        if layout.node == Some(node) {
            return Some(layout);
        }
        match &layout.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
                children.iter().find_map(|child| find(child, node))
            }
            Flow::Inlines(_) | Flow::Empty => None,
        }
    }

    #[test]
    fn shaped_entry_point_places_provider_glyphs_with_source_clusters() {
        let html = "<body><p>AB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];

        assert_eq!(line.glyphs.len(), 2);
        assert_eq!(line.glyphs[0].cluster, 0..1);
        assert_eq!(line.glyphs[1].cluster, 1..2);
        assert_eq!(line.glyphs[0].glyph, GlyphId('A' as u32));
        assert!(line.glyphs[1].x > line.glyphs[0].x);
    }

    #[test]
    fn precomputed_style_entry_point_matches_document_layout() {
        let dom = amazing_html::parse("<body><p>AB</p></body>");
        let sheet = amazing_css::parse_stylesheet("p{margin-left:7px;letter-spacing:2px}");
        let styled = amazing_css::compute(&dom, &[sheet]);
        let text = FallbackTextSystem::new();
        let precomputed = layout_styled_document(&dom, &styled, &text, &NoReplacedElements, 200.0, None);
        let direct = layout_dom_document_with_text_and_replaced_in_viewport(
            &dom, "p{margin-left:7px;letter-spacing:2px}", 200.0, None, &text, &NoReplacedElements,
        );
        let node = (0..dom.node_count()).map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p")).unwrap();
        let a = find(&precomputed, node).unwrap();
        let b = find(&direct, node).unwrap();
        assert_eq!(a.border_box, b.border_box);
        assert_eq!(a.lines[0].glyphs[1].x, b.lines[0].glyphs[1].x);
    }

    #[test]
    fn block_lengths_use_element_and_root_font_metrics() {
        let html = "<html><body><div id=target>\u{a0}</div></body></html>";
        let css = "body{margin:0} #target{font-family:'Metric Face';width:1ch;height:0.5ch;margin-top:0.5ch;padding-left:1ex}";
        let dom = amazing_html::parse(html);
        let target = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("div"))
            .unwrap();
        let layout = layout_document_with_text(html, css, 300.0, &RelativeMetricsFake(FallbackTextSystem::new()));
        let box_ = find(&layout, target).unwrap();
        assert_eq!(box_.content_box.w, 100.0);
        assert_eq!(box_.content_box.h, 50.0);
        assert_eq!(box_.margin.top, Some(50.0));
        assert_eq!(box_.padding.left, 60.0);
    }

    #[test]
    fn preserved_newline_wraps_without_losing_dom_source_offsets() {
        let html = "<body><p style=\"white-space:pre\">A  \n B</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].glyphs.last().unwrap().source_range, 5..6);
    }

    #[test]
    fn break_all_uses_provider_cluster_opportunities_for_wrapping() {
        let html = "<body><p style=\"word-break:break-all;width:9px\">AB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].glyphs[0].source_range, 0..1);
        assert_eq!(lines[1].glyphs[0].source_range, 1..2);
    }

    #[test]
    fn shaped_layout_consumes_visual_line_runs() {
        let html = "<body><p>abאב</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = VisualLineFake(FallbackTextSystem::new());
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];
        let glyphs = &line.glyphs;

        assert_eq!(
            glyphs
                .iter()
                .map(|glyph| glyph.cluster.clone())
                .collect::<Vec<_>>(),
            vec![0..1, 1..2, 4..6, 2..4]
        );
        assert!(glyphs.windows(2).all(|pair| pair[0].x < pair[1].x));
    }

    #[test]
    fn inline_background_geometry_follows_visual_bidi_order() {
        let html = "<body><p>A<span style=\"background:black;padding:1px\">B</span>C</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = AtomicOrderFake(FallbackTextSystem::new());
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];
        let a = line.glyphs.iter().find(|glyph| glyph.glyph == GlyphId('A' as u32)).unwrap();
        let c = line.glyphs.iter().find(|glyph| glyph.glyph == GlyphId('C' as u32)).unwrap();
        let background = &line.decorations[0];

        assert!(c.x < background.x && background.x < a.x);
        assert!(background.width > 0.55 * 16.0 + 1.9);
    }

    #[test]
    fn nowrap_suppresses_soft_provider_breaks() {
        let html = "<body><p style=\"white-space:nowrap;width:9px\">A B</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);

        assert_eq!(find(&layout, paragraph).unwrap().lines.len(), 1);
    }

    #[test]
    fn collapsed_whitespace_retains_original_dom_offsets() {
        let html = "<body><p>A \n\t B</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];
        let glyphs = &line.glyphs;

        assert_eq!(glyphs.last().unwrap().source_range, 5..6);
    }

    #[test]
    fn inherited_relative_font_size_is_not_resolved_twice() {
        let html = "<body><p style=\"font-size:150%\"><span>A</span></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyph = &find(&layout, paragraph).unwrap().lines[0].glyphs[0];

        assert_eq!(glyph.font_size, 24.0);
    }

    #[test]
    fn collapsible_space_runs_coalesce_across_dom_text_nodes() {
        let html = "<body><p>A <span> B</span></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;
        let a = glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('A' as u32))
            .unwrap();
        let b = glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('B' as u32))
            .unwrap();
        let space = glyphs
            .iter()
            .find(|glyph| glyph.cluster == (1..2))
            .unwrap();

        assert!((b.x - (a.x + a.advance_x.abs()) - 4.8).abs() < 0.01);
        assert_eq!(space.source_mappings.len(), 2);
    }

    #[test]
    fn standalone_whitespace_node_separates_adjacent_inline_boxes() {
        let html = "<body><p><span>A</span> <span>B</span></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;
        let a = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('A' as u32)).unwrap();
        let b = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('B' as u32)).unwrap();

        assert!((b.x - (a.x + a.advance_x.abs()) - 4.8).abs() < 0.01);
    }

    #[test]
    fn standalone_pre_line_newline_node_creates_a_line_break() {
        let html = "<body><p style=\"white-space:pre-line\"><span>A</span>\n<span>B</span></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);

        assert_eq!(find(&layout, paragraph).unwrap().lines.len(), 2);
    }

    #[test]
    fn selected_manual_hyphen_break_inserts_a_visible_hyphen() {
        let html = "<body><p style=\"width:20px;hyphens:manual\">A&shy;BB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].glyphs.last().unwrap().glyph, GlyphId('-' as u32));
    }

    #[test]
    fn unfittable_manual_hyphen_is_not_selected_as_a_wrap_point() {
        let html = "<body><p style=\"width:9px;hyphens:manual\">A&shy;BB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 1);
        assert!(lines[0]
            .glyphs
            .iter()
            .all(|glyph| glyph.glyph != GlyphId('-' as u32)));
    }

    #[test]
    fn rtl_manual_hyphen_is_fitted_and_painted_at_the_visual_start() {
        let html = "<body><p dir=ltr style=\"width:21px;hyphens:manual\"><span style=\"direction:rtl;unicode-bidi:embed;background:red\">א&shy;BB</span></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];
        let hyphen = line.glyphs.iter().find(|glyph| glyph.glyph == GlyphId('-' as u32)).unwrap();
        let letter = line.glyphs.iter().find(|glyph| glyph.glyph == GlyphId('א' as u32)).unwrap();
        let decoration = line.decorations.iter().find(|fragment| fragment.color == "red").unwrap();

        assert!(hyphen.x < letter.x);
        assert!(decoration.x <= letter.x);
        assert!(decoration.x + decoration.width >= letter.x + letter.advance_x.abs());
        assert!(line
            .glyphs
            .iter()
            .map(|glyph| glyph.x + glyph.advance_x.abs())
            .fold(0.0, f32::max)
            <= find(&layout, paragraph).unwrap().content_box.x + 21.0 + f32::EPSILON);
    }

    #[test]
    fn pre_line_preserves_segment_breaks_but_collapses_spaces() {
        let html = "<body><p style=\"white-space:pre-line\">A   \n   B</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].glyphs.len(), 1);
        assert_eq!(lines[1].glyphs.len(), 1);
        assert_eq!(lines[1].glyphs[0].source_range, 8..9);
    }

    #[test]
    fn pre_keeps_trailing_spaces_in_the_cluster_and_source_stream() {
        let html = "<body><p style=\"white-space:pre\">A  </p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;

        assert_eq!(glyphs.len(), 3);
        assert_eq!(glyphs[2].source_range, 2..3);
    }

    #[test]
    fn bidi_scope_survives_a_br_paragraph_boundary() {
        let html = "<body><p><bdi dir=rtl>A<br>B</bdi></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].glyphs.len(), 1);
        assert_eq!(lines[1].glyphs.len(), 1);
    }

    #[test]
    fn atomic_inline_participates_in_visual_bidi_order() {
        let html = "<body><p><span style=\"unicode-bidi:isolate\">A<img>B</span></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let image = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("img"))
            .unwrap();
        let text = AtomicOrderFake(FallbackTextSystem::new());
        let layout = layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            "img{width:20px;height:10px;padding:3px;border:2px solid red}",
            200.0,
            None,
            &text,
            &OneAtomic(image),
        );
        let line = &find(&layout, paragraph).unwrap().lines[0];
        let a = line.glyphs.iter().find(|glyph| glyph.glyph == GlyphId('A' as u32)).unwrap();
        let b = line.glyphs.iter().find(|glyph| glyph.glyph == GlyphId('B' as u32)).unwrap();
        let atomic = &line.replaced[0];

        assert!(
            b.x < atomic.x && atomic.x < a.x,
            "B={}, atomic={}, A={}",
            b.x,
            atomic.x,
            a.x
        );
        let fragment = find(&layout, paragraph).unwrap()
            .styled_replaced_fragments().next().unwrap();
        assert_eq!(fragment.content_box.w, 20.0);
        assert_eq!(fragment.border_box.w, 30.0);
        assert_eq!(fragment.content_box.x - fragment.border_box.x, 5.0);
        assert!(fragment.border_box.x + fragment.border_box.w <= a.x);
    }

    #[test]
    fn consecutive_preserved_breaks_create_an_empty_line_box() {
        let html = "<body><p style=\"white-space:pre\">A\n\nB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 3);
        assert!(lines[1].glyphs.is_empty());
    }

    #[test]
    fn shaped_inline_background_includes_horizontal_padding() {
        let html = "<body><p>A<span style=\"background:black;padding:2px\">B</span>C</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];

        assert_eq!(line.decorations.len(), 1);
        assert_eq!(line.decorations[0].color, "black");
        assert!(line.decorations[0].width > 0.55 * 16.0 + 3.9);
    }

    #[test]
    fn vertical_glyphs_keep_distinct_inline_axis_positions() {
        let html = "<body><p style=\"writing-mode:vertical-rl\">AB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;

        assert_eq!(glyphs.len(), 2);
        assert_eq!(glyphs[0].x, glyphs[1].x);
        assert!(glyphs[1].y > glyphs[0].y);
    }

    #[test]
    fn sideways_lr_uses_horizontal_shaping_on_a_vertical_inline_axis() {
        let html = "<body><p style=\"writing-mode:sideways-lr\">AB</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;

        assert_eq!(glyphs.len(), 2);
        assert_eq!(glyphs[0].writing_mode, amazing_text::WritingMode::SidewaysLr);
        assert_eq!(glyphs[0].orientation, amazing_text::GlyphOrientation::SidewaysLeft);
        assert!(glyphs[0].advance_x > 0.0);
        assert_eq!(glyphs[0].advance_y, 0.0);
        assert_eq!(glyphs[0].x, glyphs[1].x);
        assert!(glyphs[1].y > glyphs[0].y);
    }

    #[test]
    fn vertical_mixed_orientation_is_resolved_per_cluster() {
        let html = "<body><p style=\"writing-mode:vertical-rl;text-orientation:mixed\">A漢</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;

        assert_eq!(glyphs.len(), 2);
        assert_eq!(glyphs[0].orientation, amazing_text::GlyphOrientation::SidewaysRight);
        assert_eq!(glyphs[1].orientation, amazing_text::GlyphOrientation::Upright);
    }

    #[test]
    fn vertical_rl_wraps_against_height_and_adds_columns_to_the_left() {
        let html = "<body><p style=\"writing-mode:vertical-rl;width:100px;height:30px;font-size:20px;line-height:20px;word-break:break-all\">AAAA</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert!(lines[1].glyphs[0].x < lines[0].glyphs[0].x);
    }

    #[test]
    fn vertical_lr_preformatted_lines_advance_to_the_right() {
        let html = "<body><p style=\"writing-mode:vertical-lr;width:120px;height:120px;font-size:20px;line-height:20px;white-space:pre\">ab\ncd</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let layout = layout_document_with_text(html, "", 200.0, &FallbackTextSystem::new());
        let lines = &find(&layout, paragraph).unwrap().lines;
        assert_eq!(lines.len(), 2);
        assert!(lines[1].glyphs[0].x > lines[0].glyphs[0].x);
        assert_eq!(lines[0].glyphs[0].x, lines[0].glyphs[1].x);
        assert!(lines[0].glyphs[1].y > lines[0].glyphs[0].y);
    }

    #[test]
    fn horizontal_ruby_places_half_size_annotation_above_its_base() {
        let html = "<body><p><ruby><rb>漢</rb><rt>かん</rt></ruby></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let line = &find(&layout, paragraph).unwrap().lines[0];
        let glyphs = &line.glyphs;
        let base = glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('漢' as u32))
            .unwrap();
        let annotation = glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('か' as u32))
            .unwrap();

        assert_eq!(annotation.font_size, base.font_size * 0.5);
        assert!(annotation.y < base.y);
        assert!(line.height > base.font_size * 1.2);
        assert!(!base.source_mappings.is_empty());
        assert!(!annotation.source_mappings.is_empty());
        assert!(matches!(
            dom.get(NodeIdx(base.source_mappings[0].source)).kind,
            amazing_dom::NodeKind::Text { .. }
        ));
        assert!(matches!(
            dom.get(NodeIdx(annotation.source_mappings[0].source)).kind,
            amazing_dom::NodeKind::Text { .. }
        ));
    }

    #[test]
    fn one_ruby_pair_stays_together_between_surrounding_inline_text() {
        let html = "<body><p>A<ruby><rb>漢</rb><rt>かん</rt></ruby>B</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;
        let glyphs = &lines[0].glyphs;
        let a = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('A' as u32)).unwrap();
        let base = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('漢' as u32)).unwrap();
        let b = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('B' as u32)).unwrap();

        assert_eq!(lines.len(), 1);
        assert!(a.x < base.x);
        assert!(base.x < b.x);
    }

    #[test]
    fn ruby_does_not_wrap_at_an_illegal_latin_base_boundary() {
        let html = "<body><p style=\"width:8px;font-size:16px\"><ruby><rb>A</rb><rt>a</rt><rb>B</rb><rt>b</rt></ruby></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 1);
        assert!(lines[0].glyphs.iter().any(|glyph| glyph.glyph == GlyphId('A' as u32)));
        assert!(lines[0].glyphs.iter().any(|glyph| glyph.glyph == GlyphId('B' as u32)));
    }

    #[test]
    fn ruby_can_wrap_at_a_legal_han_base_boundary() {
        let html = "<body><p style=\"width:8px;font-size:16px\"><ruby><rb>漢</rb><rt>かん</rt><rb>字</rb><rt>じ</rt></ruby></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let lines = &find(&layout, paragraph).unwrap().lines;

        assert_eq!(lines.len(), 2);
        assert!(lines[0].glyphs.iter().any(|glyph| glyph.glyph == GlyphId('漢' as u32)));
        assert!(lines[1].glyphs.iter().any(|glyph| glyph.glyph == GlyphId('字' as u32)));
    }

    #[test]
    fn vertical_rl_ruby_places_annotation_on_the_line_over_side() {
        let html = "<body><p style=\"writing-mode:vertical-rl;width:80px;height:100px\"><ruby><rb>漢</rb><rt>かん</rt></ruby></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;
        let base = glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('漢' as u32))
            .unwrap();
        let annotation = glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('か' as u32))
            .unwrap();

        assert!(annotation.x > base.x);
        assert!(
            (annotation.y - base.y).abs() < base.font_size,
            "base={base:?} annotation={annotation:?}"
        );
    }

    #[test]
    fn text_combine_upright_forms_one_horizontal_atomic_cell() {
        let html = "<body><p style=\"writing-mode:vertical-rl;width:80px;height:100px;font-size:20px;line-height:20px\"><span style=\"text-combine-upright:all\">12</span>A</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;
        let one = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('1' as u32)).unwrap();
        let two = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('2' as u32)).unwrap();
        let following = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('A' as u32)).unwrap();

        assert!((one.y - two.y).abs() < 0.01);
        assert!(one.x < two.x);
        assert!(one.scale_x < 1.0);
        assert_eq!(one.scale_y, 1.0);
        assert!(following.y > one.y);
    }

    #[test]
    fn short_text_combine_is_centered_in_its_em_cell() {
        let html = "<body><p style=\"writing-mode:vertical-rl;width:80px;height:100px;font-size:20px;line-height:20px\"><span style=\"text-combine-upright:all;font-size:10px\">1</span>A</p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let paragraph_box = find(&layout, paragraph).unwrap();
        let glyph = paragraph_box.lines[0]
            .glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('1' as u32))
            .unwrap();
        let following = paragraph_box.lines[0]
            .glyphs
            .iter()
            .find(|glyph| glyph.glyph == GlyphId('A' as u32))
            .unwrap();
        // TCY is rebuilt through the visual U+FFFC pass. Its internal
        // horizontal composition is centered independently of the following
        // upright glyph's vertical origin.
        let column_center = following.x - following.offset_x;
        assert!(
            (glyph.x + glyph.advance_x * 0.5 - column_center).abs() < 0.01,
            "{glyph:?} center={column_center} box={:?}",
            paragraph_box.content_box
        );
    }

    #[test]
    fn spanning_ruby_annotation_centers_across_all_owned_bases() {
        let html = "<body><p><ruby><rb>A</rb><rb>B</rb><rtc>x</rtc></ruby></p></body>";
        let dom = amazing_html::parse(html);
        let paragraph = (0..dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("p"))
            .unwrap();
        let text = FallbackTextSystem::new();
        let layout = layout_document_with_text(html, "", 200.0, &text);
        let glyphs = &find(&layout, paragraph).unwrap().lines[0].glyphs;
        let a = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('A' as u32)).unwrap();
        let b = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('B' as u32)).unwrap();
        let annotation = glyphs.iter().find(|glyph| glyph.glyph == GlyphId('x' as u32)).unwrap();

        assert!(a.x < annotation.x);
        assert!(annotation.x < b.x);
    }
}

#[cfg(test)]
mod table_tests;

#[cfg(test)]
mod contents_tests;

#[cfg(test)]
mod ruby_tests;
