//! Box tree construction (CSS2.1 §9.2): classifies elements into block /
//! inline boxes, resolves the top-down font-size chain, groups mixed
//! block+inline content with anonymous blocks, and flattens inline
//! subtrees (spans) into their parent's inline flow. Absolute/fixed descendants
//! are retained separately so no formatting context treats them as in-flow
//! children or items.
//!
//! Percentages and `em` lengths are NOT resolved here — margins, padding,
//! borders and widths need the containing-block width, which exists only
//! during layout (`block.rs`). This pass resolves only what is known from
//! the parent chain: display values and font sizes / line heights.

use amazing_css::{ComputedStyle, StyledDom};
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_text::{
    Direction, FontSlant, Hyphens, LineBreak, OverflowWrap, ShapingBoundary, TextStyle,
    UnicodeBidi, WhiteSpace, WordBreak, WritingMode,
};

use crate::font::FontSource;
use crate::values;
use crate::{
    Display, Flow, InlineNode, LayoutBox, NoReplacedElements, ReplacedElementProvider,
    TextSourceMapping,
};

/// Builds the root box for a document: the `<body>` element (or `<html>`
/// when body is missing; an empty box otherwise).
pub fn build_layout_tree(dom: &Dom, styled: &StyledDom, fonts: &FontSource) -> LayoutBox {
    build_layout_tree_with_replaced(dom, styled, fonts, &NoReplacedElements)
}

/// Builds the root box while resolving replaced-element intrinsic sizes.
pub fn build_layout_tree_with_replaced(
    dom: &Dom,
    styled: &StyledDom,
    fonts: &FontSource,
    replaced: &dyn ReplacedElementProvider,
) -> LayoutBox {
    let root_style = ComputedStyle::default();
    let start = dom_html(dom)
        .and_then(|html| find_element(dom, html, "body"))
        .or_else(|| dom_html(dom));
    match start {
        Some(idx) => build_element(dom, styled, idx, values::ROOT_FONT_SIZE, fonts, replaced),
        None => LayoutBox::new(
            None,
            Display::Block,
            root_style,
            values::ROOT_FONT_SIZE,
            19.2,
        ),
    }
}

fn dom_html(dom: &Dom) -> Option<NodeIdx> {
    find_element(dom, NodeIdx(0), "html")
}

/// First element child of `parent` with the given tag name.
fn find_element(dom: &Dom, parent: NodeIdx, name: &str) -> Option<NodeIdx> {
    dom.children(parent).find(|&c| dom.name(c) == Some(name))
}

/// Builds one visible element's box, recursing into its children.
fn build_element(
    dom: &Dom,
    styled: &StyledDom,
    idx: NodeIdx,
    parent_font_px: f32,
    fonts: &FontSource,
    replaced: &dyn ReplacedElementProvider,
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
    box_.replaced_size = replaced.intrinsic_size(idx);
    let classified = classify_children(dom, styled, idx, font_size, fonts, replaced);
    box_.flow = classified.flow;
    box_.positioned = classified.positioned;
    box_
}

/// `display` computed value → [`Display`]. Unknown values fall back to
/// CSS's initial `inline`; `none` never reaches here (filtered upstream).
fn parse_display(style: &ComputedStyle) -> Display {
    match style.get("display") {
        Some(v)
            if v.eq_ignore_ascii_case("block")
            || v.eq_ignore_ascii_case("list-item")
            || v.eq_ignore_ascii_case("table")
            || v.eq_ignore_ascii_case("inline-block")
            || v.eq_ignore_ascii_case("inline-table")
            || v.eq_ignore_ascii_case("table-caption")
            || v.eq_ignore_ascii_case("table-row-group")
            || v.eq_ignore_ascii_case("table-header-group")
            || v.eq_ignore_ascii_case("table-footer-group")
            || v.eq_ignore_ascii_case("table-row")
            || v.eq_ignore_ascii_case("table-cell")
            || v.eq_ignore_ascii_case("table-column-group")
            || v.eq_ignore_ascii_case("table-column") =>
        {
            Display::Block
        }
        Some(v)
            if v.eq_ignore_ascii_case("flex")
            || v.eq_ignore_ascii_case("inline-flex")
            || v.eq_ignore_ascii_case("-webkit-flex") =>
        {
            Display::Flex
        }
        Some(v) if v.eq_ignore_ascii_case("grid") || v.eq_ignore_ascii_case("inline-grid") => {
            Display::Grid
        }
        _ => Display::Inline,
    }
}

/// True when the element should not generate any box at all.
fn is_display_none(style: &ComputedStyle) -> bool {
    style
        .get("display")
        .is_some_and(|v| v.eq_ignore_ascii_case("none"))
}

fn is_out_of_flow(style: &ComputedStyle) -> bool {
    style.get("position").is_some_and(|value| {
        value.eq_ignore_ascii_case("absolute") || value.eq_ignore_ascii_case("fixed")
    })
}

struct ClassifiedChildren {
    flow: Flow,
    positioned: Vec<LayoutBox>,
}

/// Collapses whitespace runs to single spaces per CSS text processing.
/// (Line-start/end trimming happens later, in the line breaker.)
fn collapse_whitespace(
    text: &str,
    preserve_segment_breaks: bool,
) -> (String, Vec<TextSourceMapping>) {
    let mut out = String::with_capacity(text.len());
    let mut mappings = Vec::new();
    let mut in_ws = false;
    for (source_start, c) in text.char_indices() {
        let source_end = source_start + c.len_utf8();
        if preserve_segment_breaks && c == '\n' {
            in_ws = false;
            let output_start = out.len();
            out.push(c);
            mappings.push(TextSourceMapping {
                text_range: output_start..out.len(),
                source_range: source_start..source_end,
            });
        } else if matches!(c, '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' ') {
            if !in_ws {
                let output_start = out.len();
                out.push(' ');
                mappings.push(TextSourceMapping {
                    text_range: output_start..out.len(),
                    source_range: source_start..source_end,
                });
            } else if let Some(mapping) = mappings.last_mut() {
                mapping.source_range.end = source_end;
            }
            in_ws = true;
        } else {
            in_ws = false;
            let output_start = out.len();
            out.push(c);
            mappings.push(TextSourceMapping {
                text_range: output_start..out.len(),
                source_range: source_start..source_end,
            });
        }
    }
    (out, mappings)
}

fn text_for_white_space(text: &str, style: &ComputedStyle) -> (String, Vec<TextSourceMapping>) {
    match style.get("white-space").unwrap_or("normal") {
        value
            if value.eq_ignore_ascii_case("pre")
                || value.eq_ignore_ascii_case("pre-wrap")
                || value.eq_ignore_ascii_case("break-spaces") =>
        {
            (
                text.to_owned(),
                (!text.is_empty())
                    .then_some(TextSourceMapping {
                        text_range: 0..text.len(),
                        source_range: 0..text.len(),
                    })
                    .into_iter()
                    .collect(),
            )
        }
        value if value.eq_ignore_ascii_case("pre-line") => collapse_whitespace(text, true),
        _ => collapse_whitespace(text, false),
    }
}

fn attribute<'a>(dom: &'a Dom, node: NodeIdx, name: &str) -> Option<&'a str> {
    dom.attrs_of(node)?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn inherited_attribute<'a>(dom: &'a Dom, mut node: NodeIdx, name: &str) -> Option<&'a str> {
    loop {
        if let Some(value) = attribute(dom, node, name) {
            return Some(value);
        }
        node = dom.get(node).parent?;
    }
}

fn split_families(value: &str) -> Vec<String> {
    let mut families = Vec::new();
    let mut start = 0usize;
    let mut quote = None;
    for (index, ch) in value.char_indices() {
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(expected), current) if expected == current => quote = None,
            (None, ',') => {
                let family = value[start..index].trim();
                if !family.is_empty() {
                    families.push(family.to_owned());
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let family = value[start..].trim();
    if !family.is_empty() {
        families.push(family.to_owned());
    }
    if families.is_empty() {
        families.push("serif".into());
    }
    families
}

fn text_style(
    dom: &Dom,
    owner: NodeIdx,
    style: &ComputedStyle,
    font_size: f32,
    inline_directionality: bool,
) -> TextStyle {
    let weight = match style.get("font-weight").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("bold") || value.eq_ignore_ascii_case("bolder") => 700,
        value if value.eq_ignore_ascii_case("lighter") => 300,
        value => value
            .parse::<u16>()
            .ok()
            .filter(|weight| (1..=1000).contains(weight))
            .unwrap_or(400),
    };
    let slant = match style.get("font-style").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("italic") => FontSlant::Italic,
        value if value.to_ascii_lowercase().starts_with("oblique") => FontSlant::Oblique,
        _ => FontSlant::Normal,
    };
    let direction_value = style.get("direction").unwrap_or("ltr");
    let direction = if direction_value.eq_ignore_ascii_case("rtl") {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    };
    let writing_mode = match style.get("writing-mode").unwrap_or("horizontal-tb") {
        value if value.eq_ignore_ascii_case("vertical-rl") => WritingMode::VerticalRl,
        value if value.eq_ignore_ascii_case("vertical-lr") => WritingMode::VerticalLr,
        _ => WritingMode::HorizontalTb,
    };
    let white_space = match style.get("white-space").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("nowrap") => WhiteSpace::NoWrap,
        value if value.eq_ignore_ascii_case("pre") => WhiteSpace::Pre,
        value if value.eq_ignore_ascii_case("pre-wrap") => WhiteSpace::PreWrap,
        value if value.eq_ignore_ascii_case("pre-line") => WhiteSpace::PreLine,
        value if value.eq_ignore_ascii_case("break-spaces") => WhiteSpace::BreakSpaces,
        _ => WhiteSpace::Normal,
    };
    let word_break = match style.get("word-break").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("break-all") => WordBreak::BreakAll,
        value if value.eq_ignore_ascii_case("keep-all") => WordBreak::KeepAll,
        _ => WordBreak::Normal,
    };
    let mut unicode_bidi = match style.get("unicode-bidi").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("embed") => UnicodeBidi::Embed,
        value if value.eq_ignore_ascii_case("isolate") => UnicodeBidi::Isolate,
        value if value.eq_ignore_ascii_case("bidi-override") => UnicodeBidi::BidiOverride,
        value if value.eq_ignore_ascii_case("isolate-override") => UnicodeBidi::IsolateOverride,
        value if value.eq_ignore_ascii_case("plaintext") => UnicodeBidi::Plaintext,
        _ => UnicodeBidi::Normal,
    };
    if inline_directionality && unicode_bidi == UnicodeBidi::Normal {
        unicode_bidi = match dom.name(owner) {
            Some("bdo") => UnicodeBidi::IsolateOverride,
            Some("bdi") => UnicodeBidi::Isolate,
            _ if attribute(dom, owner, "dir").is_some() => UnicodeBidi::Isolate,
            _ => UnicodeBidi::Normal,
        };
    }
    let hyphens = match style.get("hyphens").unwrap_or("manual") {
        value if value.eq_ignore_ascii_case("none") => Hyphens::None,
        value if value.eq_ignore_ascii_case("auto") => Hyphens::Auto,
        _ => Hyphens::Manual,
    };
    let line_break = match style.get("line-break").unwrap_or("auto") {
        value if value.eq_ignore_ascii_case("loose") => LineBreak::Loose,
        value if value.eq_ignore_ascii_case("normal") => LineBreak::Normal,
        value if value.eq_ignore_ascii_case("strict") => LineBreak::Strict,
        value if value.eq_ignore_ascii_case("anywhere") => LineBreak::Anywhere,
        _ => LineBreak::Auto,
    };
    let overflow_wrap = match style.get("overflow-wrap").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("break-word") => OverflowWrap::BreakWord,
        value if value.eq_ignore_ascii_case("anywhere") => OverflowWrap::Anywhere,
        _ => OverflowWrap::Normal,
    };
    let stretch = match style.get("font-stretch").unwrap_or("normal") {
        value if value.eq_ignore_ascii_case("ultra-condensed") => 50,
        value if value.eq_ignore_ascii_case("extra-condensed") => 62,
        value if value.eq_ignore_ascii_case("condensed") => 75,
        value if value.eq_ignore_ascii_case("semi-condensed") => 87,
        value if value.eq_ignore_ascii_case("semi-expanded") => 112,
        value if value.eq_ignore_ascii_case("expanded") => 125,
        value if value.eq_ignore_ascii_case("extra-expanded") => 150,
        value if value.eq_ignore_ascii_case("ultra-expanded") => 200,
        value => value
            .trim_end_matches('%')
            .parse::<u16>()
            .ok()
            .filter(|stretch| (1..=1000).contains(stretch))
            .unwrap_or(100),
    };
    TextStyle {
        families: split_families(style.get("font-family").unwrap_or("serif")),
        size: font_size,
        weight,
        stretch,
        slant,
        direction,
        writing_mode,
        white_space,
        word_break,
        unicode_bidi,
        hyphens,
        line_break,
        overflow_wrap,
        language: inherited_attribute(dom, owner, "lang").map(str::to_owned),
        features: Vec::new(),
        variations: Vec::new(),
        synthesis: amazing_text::FontSynthesis::default(),
    }
}

fn inline_text(
    dom: &Dom,
    owner: NodeIdx,
    node: NodeIdx,
    source_text: String,
    style: &ComputedStyle,
    font_size: f32,
    inline_box: bool,
) -> InlineNode {
    let (text, source_mappings) = text_for_white_space(&source_text, style);
    let padding = |property: &str| {
        style
            .get(property)
            .and_then(|value| values::parse_len(value, font_size, font_size))
            .unwrap_or(0.0)
            .max(0.0)
    };
    InlineNode::Text {
        node,
        text,
        source_mappings,
        font_size,
        style: Box::new(text_style(dom, owner, style, font_size, inline_box)),
        color: style.get("color").unwrap_or("black").to_owned(),
        background_color: if inline_box {
            style
                .get("background-color")
                .unwrap_or("transparent")
                .to_owned()
        } else {
            "transparent".into()
        },
        inline_padding: if inline_box {
            values::Sides {
                top: padding("padding-top"),
                right: padding("padding-right"),
                bottom: padding("padding-bottom"),
                left: padding("padding-left"),
            }
        } else {
            values::Sides::default()
        },
        boundary_before: ShapingBoundary::Continue,
        boundary_after: ShapingBoundary::Continue,
    }
}

fn nonzero_length(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value != "0" && value != "0px" && value != "0em" && value != "0rem" && value != "0%"
    })
}

fn hard_inline_edge(dom: &Dom, node: NodeIdx, style: &ComputedStyle) -> bool {
    if dom.name(node) == Some("bdi")
        || style
            .get("unicode-bidi")
            .is_some_and(|value| value.to_ascii_lowercase().contains("isolate"))
        || [
            "padding-left",
            "padding-right",
            "margin-left",
            "margin-right",
        ]
            .into_iter()
            .any(|property| nonzero_length(style.get(property)))
    {
        return true;
    }
    ["left", "right"].into_iter().any(|side| {
        let style_name = format!("border-{side}-style");
        let width_name = format!("border-{side}-width");
        style.get(&style_name).is_some_and(|value| {
            !value.eq_ignore_ascii_case("none") && !value.eq_ignore_ascii_case("hidden")
        }) && nonzero_length(style.get(&width_name))
    })
}

fn boundary_between(
    dom: &Dom,
    node: NodeIdx,
    parent_style: &ComputedStyle,
    parent_font_size: f32,
    child_style: &ComputedStyle,
    child_font_size: f32,
) -> ShapingBoundary {
    if hard_inline_edge(dom, node, child_style) {
        ShapingBoundary::Hard
    } else if text_style(dom, node, child_style, child_font_size, true)
        != text_style(
            dom,
            dom.get(node).parent.unwrap_or(node),
            parent_style,
            parent_font_size,
            parse_display(parent_style) == Display::Inline,
        )
    {
        ShapingBoundary::StyleChange
    } else {
        ShapingBoundary::Continue
    }
}

fn stronger_boundary(left: ShapingBoundary, right: ShapingBoundary) -> ShapingBoundary {
    match (left, right) {
        (ShapingBoundary::Hard, _) | (_, ShapingBoundary::Hard) => ShapingBoundary::Hard,
        (ShapingBoundary::StyleChange, _) | (_, ShapingBoundary::StyleChange) => {
            ShapingBoundary::StyleChange
        }
        _ => ShapingBoundary::Continue,
    }
}

fn mark_inline_edges(nodes: &mut [InlineNode], boundary: ShapingBoundary) {
    if boundary == ShapingBoundary::Continue {
        return;
    }
    if let Some(InlineNode::Text {
        boundary_before, ..
    }) = nodes
        .iter_mut()
        .find(|node| matches!(node, InlineNode::Text { .. }))
    {
        *boundary_before = stronger_boundary(*boundary_before, boundary);
    }
    if let Some(InlineNode::Text { boundary_after, .. }) = nodes
        .iter_mut()
        .rev()
        .find(|node| matches!(node, InlineNode::Text { .. }))
    {
        *boundary_after = stronger_boundary(*boundary_after, boundary);
    }
}


/// Classifies the children of a block container into its [`Flow`],
/// inserting anonymous blocks around runs of inline content (§9.2.1.3).
fn classify_children(
    dom: &Dom,
    styled: &StyledDom,
    idx: NodeIdx,
    font_size: f32,
    fonts: &FontSource,
    replaced: &dyn ReplacedElementProvider,
) -> ClassifiedChildren {
    // Flex/grid containers take every non-`none` element child as an ITEM
    // (css-flexbox-1 §3 / css-grid-1 §3 blockification); contiguous text
    // runs become one anonymous item each.
    let own_style = styled.get(idx).cloned().unwrap_or_default();
    let own_display = parse_display(&own_style);
    if matches!(own_display, Display::Flex | Display::Grid) {
        let mut items: Vec<LayoutBox> = Vec::new();
        let mut run: Vec<InlineNode> = Vec::new();
        let mut positioned = Vec::new();
        for child in dom.children(idx) {
            match &dom.get(child).kind {
                NodeKind::Text { data } => {
                    if !text_for_white_space(data, &own_style).0.is_empty() {
                        run.push(inline_text(
                            dom,
                            idx,
                            child,
                            data.clone(),
                            &own_style,
                            font_size,
                            false,
                        ));
                    }
                }
                NodeKind::Comment { .. }
                | NodeKind::ProcessingInstruction { .. }
                | NodeKind::Document => {}
                NodeKind::Element { .. } => {
                    let child_style = styled.get(child).cloned().unwrap_or_default();
                    if is_display_none(&child_style) {
                        continue;
                    }
                    if is_out_of_flow(&child_style) {
                        positioned.push(build_element(
                            dom, styled, child, font_size, fonts, replaced,
                        ));
                        continue;
                    }
                    flush_anonymous_into(&mut items, &mut run, font_size, fonts);
                    items.push(build_element(
                        dom, styled, child, font_size, fonts, replaced,
                    ));
                }
            }
        }
        flush_anonymous_into(&mut items, &mut run, font_size, fonts);
        let flow = if own_display == Display::Grid {
            Flow::Grid(items)
        } else {
            Flow::Flex(items)
        };
        return ClassifiedChildren { flow, positioned };
    }

    let mut blocks: Vec<LayoutBox> = Vec::new();
    let mut inlines: Vec<InlineNode> = Vec::new();
    let mut positioned = Vec::new();

    for child in dom.children(idx) {
        match &dom.get(child).kind {
            NodeKind::Text { data } => {
                // Whitespace-only runs between block-level children carry no
                // rendering; keep non-empty collapsed runs as separators.
                if !text_for_white_space(data, &own_style).0.is_empty() {
                    inlines.push(inline_text(
                        dom,
                        idx,
                        child,
                        data.clone(),
                        &own_style,
                        font_size,
                        false,
                    ));
                }
            }
            NodeKind::Comment { .. }
            | NodeKind::ProcessingInstruction { .. }
            | NodeKind::Document => {}
            NodeKind::Element { .. } => {
                let child_style = styled.get(child).cloned().unwrap_or_default();
                if is_display_none(&child_style) {
                    continue; // no box, no descendants
                }
                if is_out_of_flow(&child_style) {
                    positioned.push(build_element(
                        dom, styled, child, font_size, fonts, replaced,
                    ));
                    continue;
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
                        blocks.push(build_element(
                            dom, styled, child, font_size, fonts, replaced,
                        ));
                    }
                    Some("img") => {
                        if let Some(size) = replaced.intrinsic_size(child) {
                            inlines.push(InlineNode::Replaced { node: child, size });
                        }
                    }
                    _ => {
                        let child_font_size = values::resolve_font_size(
                            child_style.get("font-size").unwrap_or("medium"),
                            font_size,
                        );
                        let edge = boundary_between(
                            dom,
                            child,
                            &own_style,
                            font_size,
                            &child_style,
                            child_font_size,
                        );
                        flatten_inline(
                            dom,
                            styled,
                            child,
                            child_style,
                            font_size,
                            edge,
                            &mut inlines,
                            &mut blocks,
                            &mut positioned,
                            fonts,
                            replaced,
                        )
                }
                }
            }
        }
    }
    // Trailing inline content: when the container has no block children it
    // IS an inline formatting context — keep the items directly. Otherwise
    // the remainder becomes one last anonymous block.
    if !inlines.is_empty() && !blocks.is_empty() {
        flush_anonymous_into(&mut blocks, &mut inlines, font_size, fonts);
    }
    if blocks.is_empty() && !has_renderable_inline_content(&inlines) {
        inlines.clear();
    }
    let flow = match (blocks.is_empty(), inlines.is_empty()) {
        (true, true) => Flow::Empty,
        (true, false) => Flow::Inlines(inlines),
        (false, _) => Flow::Blocks(blocks),
    };
    ClassifiedChildren { flow, positioned }
}

/// Moves accumulated inline content into an anonymous block box whose strut
/// comes from the host font size.
fn flush_anonymous_into(
    blocks: &mut Vec<LayoutBox>,
    inlines: &mut Vec<InlineNode>,
    font_size: f32,
    fonts: &FontSource,
) {
    if inlines.is_empty() || !has_renderable_inline_content(inlines) {
        inlines.clear();
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

fn has_renderable_inline_content(inlines: &[InlineNode]) -> bool {
    inlines.iter().any(|item| match item {
        InlineNode::Text { text, style, .. } => {
            text.chars().any(|ch| {
                !matches!(ch, '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' ')
            }) || (style.white_space == amazing_text::WhiteSpace::PreLine && text.contains('\n'))
                || matches!(
                    style.white_space,
                    amazing_text::WhiteSpace::Pre
                        | amazing_text::WhiteSpace::PreWrap
                        | amazing_text::WhiteSpace::BreakSpaces
                )
        }
        InlineNode::Break | InlineNode::Replaced { .. } => true,
        InlineNode::BidiStart { .. } | InlineNode::BidiEnd => false,
    })
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
    edge_boundary: ShapingBoundary,
    inlines: &mut Vec<InlineNode>,
    blocks_out: &mut Vec<LayoutBox>,
    positioned_out: &mut Vec<LayoutBox>,
    fonts: &FontSource,
    replaced: &dyn ReplacedElementProvider,
) {
    let font_size =
        values::resolve_font_size(style.get("font-size").unwrap_or("medium"), parent_font_px);
    let bidi_style = text_style(dom, idx, &style, font_size, true);
    let inline_start = inlines.len();

    for child in dom.children(idx) {
        match &dom.get(child).kind {
            NodeKind::Text { data } => {
                if !text_for_white_space(data, &style).0.is_empty() {
                    inlines.push(inline_text(
                        dom,
                        idx,
                        child,
                        data.clone(),
                        &style,
                        font_size,
                        true,
                    ));
                }
            }
            NodeKind::Comment { .. }
            | NodeKind::ProcessingInstruction { .. }
            | NodeKind::Document => {}
            NodeKind::Element { .. } => {
                let cs = styled.get(child).cloned().unwrap_or_default();
                if is_display_none(&cs) {
                    continue;
                }
                if is_out_of_flow(&cs) {
                    positioned_out.push(build_element(
                        dom,
                        styled,
                        child,
                        parent_font_px,
                        fonts,
                        replaced,
                    ));
                    continue;
                }
                match dom.name(child) {
                    Some("br") => inlines.push(InlineNode::Break),
                    _ if matches!(
                        parse_display(&cs),
                        Display::Block | Display::Flex | Display::Grid
                    ) =>
                    {
                        // Hoist: end the current inline run here.
                        flush_anonymous_into(blocks_out, inlines, parent_font_px, fonts);
                        blocks_out.push(build_element(
                            dom,
                            styled,
                            child,
                            parent_font_px,
                            fonts,
                            replaced,
                        ));
                    }
                    Some("img") => {
                        if let Some(size) = replaced.intrinsic_size(child) {
                            inlines.push(InlineNode::Replaced { node: child, size });
                        }
                    }
                    _ => {
                        let child_font_size = values::resolve_font_size(
                            cs.get("font-size").unwrap_or("medium"),
                            font_size,
                        );
                        let edge =
                            boundary_between(dom, child, &style, font_size, &cs, child_font_size);
                        flatten_inline(
                            dom,
                            styled,
                            child,
                            cs,
                            font_size,
                            edge,
                            inlines,
                            blocks_out,
                            positioned_out,
                            fonts,
                            replaced,
                        )
                }
                }
            }
        }
    }
    if inline_start <= inlines.len() {
        mark_inline_edges(&mut inlines[inline_start..], edge_boundary);
        if bidi_style.unicode_bidi != UnicodeBidi::Normal {
            inlines.insert(
                inline_start,
                InlineNode::BidiStart {
                    direction: bidi_style.direction,
                    unicode_bidi: bidi_style.unicode_bidi,
                },
            );
            inlines.push(InlineNode::BidiEnd);
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
        b.node
            .and_then(|n| dom.name(n))
            .unwrap_or("#anon")
            .to_string()
    }

    #[test]
    fn body_is_root_with_block_children() {
        let (dom, root) = tree("<body><p>a</p><div>b</div></body>");
        assert_eq!(tag_of(&root, &dom), "body");
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
        assert_eq!(kids.len(), 2);
        assert_eq!(tag_of(&kids[0], &dom), "p");
        assert_eq!(tag_of(&kids[1], &dom), "div");
    }

    #[test]
    fn mixed_content_makes_anonymous_blocks() {
        let (dom, root) = tree("<body>text <b>bold</b><p>para</p>after</body>");
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
        // anon(text bold) | p | anon(after)
        assert_eq!(kids.len(), 3);
        assert_eq!(tag_of(&kids[0], &dom), "#anon");
        assert_eq!(tag_of(&kids[1], &dom), "p");
        assert_eq!(tag_of(&kids[2], &dom), "#anon");
        let Flow::Inlines(items) = &kids[0].flow else {
            panic!("anon wants inlines")
        };
        assert_eq!(items.len(), 2); // "text " + "bold"
    }

    #[test]
    fn inter_block_whitespace_vanishes() {
        let (_, root) = tree("<body>\n  <p>a</p>\n  <p>b</p>\n</body>");
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
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
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
        assert_eq!(kids.len(), 2);
        assert_eq!(tag_of(&kids[0], &dom), "p");
        assert_eq!(tag_of(&kids[1], &dom), "p");
    }

    #[test]
    fn nested_spans_flatten_to_words_at_own_font_size() {
        let (dom, root) = tree("<body><p>a <span style=\"font-size:32px\">big</span></p></body>");
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
        assert_eq!(tag_of(&kids[0], &dom), "p");
        let Flow::Inlines(items) = &kids[0].flow else {
            panic!("p wants inlines")
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text_text(), ("a ", 16.0));
        assert_eq!(items[1].text_text(), ("big", 32.0));
    }

    #[test]
    fn nested_descendants_keep_one_paired_inline_bidi_scope() {
        let (_, root) = tree("<body><p><span dir=rtl>A<em>B</em>C</span></p></body>");
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        assert_eq!(items.len(), 5);
        assert!(matches!(items.first(), Some(InlineNode::BidiStart { .. })));
        assert!(matches!(items.last(), Some(InlineNode::BidiEnd)));
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(item, InlineNode::Text { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn non_breaking_space_is_not_css_collapsible_whitespace() {
        let (_, root) = tree("<body><p>&nbsp;x</p></body>");
        let Flow::Blocks(children) = &root.flow else {
            panic!("want blocks")
        };
        let Flow::Inlines(items) = &children[0].flow else {
            panic!("want inlines")
        };

        assert_eq!(items[0].text_text().0, "\u{00a0}x");
    }

    impl InlineNode {
        fn text_text(&self) -> (&str, f32) {
            match self {
                InlineNode::Text {
                    text, font_size, ..
                } => (text.as_str(), *font_size),
                InlineNode::BidiStart { .. } => ("<bidi-start>", 0.0),
                InlineNode::BidiEnd => ("<bidi-end>", 0.0),
                InlineNode::Break => ("<br>", 0.0),
                InlineNode::Replaced { .. } => ("<replaced>", 0.0),
            }
        }
    }

    #[test]
    fn block_inside_inline_hoists_in_order() {
        let (dom, root) = tree("<body><span>one<div>two</div>three</span></body>");
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
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
        let Flow::Blocks(divs) = &root.flow else {
            panic!("want blocks")
        };
        assert!((divs[0].font_size - 32.0).abs() < 1e-4);
        let Flow::Blocks(ps) = &divs[0].flow else {
            panic!("div wants blocks")
        };
        assert!((ps[0].font_size - 16.0).abs() < 1e-4);
    }

    #[test]
    fn br_becomes_break_marker() {
        let (_, root) = tree("<body><p>a<br>b</p></body>");
        let Flow::Blocks(kids) = &root.flow else {
            panic!("want blocks")
        };
        let Flow::Inlines(items) = &kids[0].flow else {
            panic!("p wants inlines")
        };
        assert!(matches!(items[1], InlineNode::Break));
    }
}
