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
    Direction, FontSlant, Hyphens, LineBreak, OverflowWrap, ShapingBoundary, TextCombine,
    TextOrientation, TextStyle, UnicodeBidi, WhiteSpace, WordBreak, WritingMode,
};

use crate::font::FontSource;
use crate::values;
use crate::{
    Display, Flow, InlineNode, LayoutBox, NoReplacedElements, ReplacedElementProvider,
    RubyColumn, RubyText, TextSourceMapping,
};

const MAX_RUBY_COLUMNS: usize = 65_536;
const MAX_RUBY_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RUBY_SOURCE_MAPPINGS: usize = 262_144;

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
        Some(idx) => {
            let mut root = build_element(dom, styled, idx, values::ROOT_FONT_SIZE, fonts, replaced);
            if is_contents(&root.style) {
                let original = std::mem::take(&mut root.style);
                root.node = None;
                root.display = Display::Block;
                // The engine starts at body. Keep an anonymous viewport host,
                // but no body box, decoration, positioning or sizing.
                for property in [
                    "direction", "text-align", "writing-mode", "white-space", "line-height",
                ] {
                    if let Some(value) = original.get(property) {
                        root.style.set(property, value);
                    }
                }
                for property in ["margin-top", "margin-right", "margin-bottom", "margin-left"] {
                    root.style.set(property, "0");
                }
            }
            crate::table_tree::normalize(&mut root);
            root
        },
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
    let mut style = styled.get(idx).cloned().unwrap_or_default();
    let table_role = crate::table_tree::Role::of(&style);
    if !matches!(table_role, crate::table_tree::Role::Other | crate::table_tree::Role::Table)
        && (crate::floats::side(&style).is_some() || is_out_of_flow(&style)) {
        style.set("display", "block");
    }
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
    if matches!(dom.name(idx), Some("td" | "th")) {
        box_.table_colspan = table_span(attribute(dom, idx, "colspan"), 1000, false);
        box_.table_rowspan = table_span(attribute(dom, idx, "rowspan"), 65534, true);
    } else if matches!(dom.name(idx), Some("col" | "colgroup")) {
        box_.table_colspan = table_span(attribute(dom, idx, "span"), 1000, false);
    }
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
            || v.eq_ignore_ascii_case("flow-root")
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
        _ if crate::floats::side(style).is_some() => Display::Block,
        _ => Display::Inline,
    }
}

/// True when the element should not generate any box at all.
fn is_display_none(style: &ComputedStyle) -> bool {
    style
        .get("display")
        .is_some_and(|v| v.eq_ignore_ascii_case("none"))
}

/// Box-tree children after removing `display: contents` ancestors. The DOM
/// remains intact: text style and relative font metrics still follow its
/// original parent chain, even when formatting parents differ.
struct FormattingChild {
    node: NodeIdx,
    owner: NodeIdx,
    parent_font: f32,
}

fn is_contents(style: &ComputedStyle) -> bool {
    style
        .get("display")
        .is_some_and(|value| value.eq_ignore_ascii_case("contents"))
}

fn contents_suppresses_subtree(dom: &Dom, node: NodeIdx) -> bool {
    matches!(
        dom.name(node),
        Some(
            "br" | "wbr"
                | "meter"
                | "progress"
                | "canvas"
                | "embed"
                | "object"
                | "audio"
                | "iframe"
                | "img"
                | "video"
                | "frame"
                | "frameset"
                | "input"
                | "textarea"
                | "select"
        )
    )
}

fn formatting_children(
    dom: &Dom,
    styled: &StyledDom,
    owner: NodeIdx,
    font_size: f32,
) -> Vec<FormattingChild> {
    let mut pending: Vec<_> = dom
        .children(owner)
        .map(|node| FormattingChild {
            node,
            owner,
            parent_font: font_size,
        })
        .collect();
    pending.reverse();
    let mut children = Vec::new();
    while let Some(child) = pending.pop() {
        if matches!(dom.get(child.node).kind, NodeKind::Element { .. }) {
            let style = styled.get(child.node).cloned().unwrap_or_default();
            if is_display_none(&style) || crate::replaced::hidden(dom, child.node) {
                continue;
            }
            if is_contents(&style) {
                // Do this before positioning, float blockification, controls,
                // or flex/grid item construction: there is no principal box.
                if contents_suppresses_subtree(dom, child.node) {
                    continue;
                }
                let font_size = values::resolve_font_size(
                    style.get("font-size").unwrap_or("medium"),
                    child.parent_font,
                );
                let start = pending.len();
                pending.extend(dom.children(child.node).map(|node| FormattingChild {
                    node,
                    owner: child.node,
                    parent_font: font_size,
                }));
                pending[start..].reverse();
                continue;
            }
        }
        children.push(child);
    }
    children
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
    source: NodeIdx,
    max_mappings: usize,
) -> (String, Vec<TextSourceMapping>) {
    let mut out = String::with_capacity(text.len());
    let mut mappings = Vec::new();
    let mut in_ws = false;
    for (source_start, c) in text.char_indices() {
        let source_end = source_start + c.len_utf8();
        let whitespace = matches!(c, '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' ');
        let needs_mapping = (preserve_segment_breaks && c == '\n') || !whitespace || !in_ws;
        if needs_mapping && mappings.len() >= max_mappings {
            break;
        }
        if preserve_segment_breaks && c == '\n' {
            in_ws = false;
            let output_start = out.len();
            out.push(c);
            mappings.push(TextSourceMapping {
                text_range: output_start..out.len(),
                source,
                source_range: source_start..source_end,
            });
        } else if whitespace {
            if !in_ws {
                let output_start = out.len();
                out.push(' ');
                mappings.push(TextSourceMapping {
                    text_range: output_start..out.len(),
                    source,
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
                source,
                source_range: source_start..source_end,
            });
        }
    }
    (out, mappings)
}

fn text_for_white_space(
    text: &str,
    style: &ComputedStyle,
    source: NodeIdx,
    max_mappings: usize,
) -> (String, Vec<TextSourceMapping>) {
    match style.get("white-space").unwrap_or("normal") {
        value
            if value.eq_ignore_ascii_case("pre")
                || value.eq_ignore_ascii_case("pre-wrap")
                || value.eq_ignore_ascii_case("break-spaces") =>
        {
            if max_mappings == 0 {
                return (String::new(), Vec::new());
            }
            (
                text.to_owned(),
                (!text.is_empty())
                    .then_some(TextSourceMapping {
                        text_range: 0..text.len(),
                        source,
                        source_range: 0..text.len(),
                    })
                    .into_iter()
                    .collect(),
            )
        }
        value if value.eq_ignore_ascii_case("pre-line") => {
            collapse_whitespace(text, true, source, max_mappings)
        },
        _ => collapse_whitespace(text, false, source, max_mappings),
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
    let writing_mode = match style.get("writing-mode").unwrap_or("horizontal-tb") {
        value if value.eq_ignore_ascii_case("vertical-rl") => WritingMode::VerticalRl,
        value if value.eq_ignore_ascii_case("vertical-lr") => WritingMode::VerticalLr,
        value if value.eq_ignore_ascii_case("sideways-rl") => WritingMode::SidewaysRl,
        value if value.eq_ignore_ascii_case("sideways-lr") => WritingMode::SidewaysLr,
        _ => WritingMode::HorizontalTb,
    };
    let text_orientation = match style.get("text-orientation").unwrap_or("mixed") {
        value if value.eq_ignore_ascii_case("upright") => TextOrientation::Upright,
        value if value.eq_ignore_ascii_case("sideways") => TextOrientation::Sideways,
        _ => TextOrientation::Mixed,
    };
    let direction_value = style.get("direction").unwrap_or("ltr");
    let direction = if writing_mode.is_vertical_flow()
        && text_orientation == TextOrientation::Upright
    {
        // Writing Modes §5.1: upright's used direction is ltr and every
        // character participates in bidi as strong LTR.
        Direction::LeftToRight
    } else if direction_value.eq_ignore_ascii_case("rtl") {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    };
    let text_combine = match style.get("text-combine-upright").unwrap_or("none") {
        value if value.eq_ignore_ascii_case("all") => TextCombine::All,
        _ => TextCombine::None,
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
        text_orientation,
        text_combine,
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
    let (text, source_mappings) = text_for_white_space(&source_text, style, node, usize::MAX);
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

fn collect_ruby_text(
    dom: &Dom,
    styled: &StyledDom,
    node: NodeIdx,
    output: &mut Vec<(NodeIdx, String)>,
    budget: &mut usize,
    segment_budget: &mut usize,
) {
    if *budget == 0 || *segment_budget == 0 {
        return;
    }
    if styled.get(node).is_some_and(is_display_none) {
        return;
    }
    match &dom.get(node).kind {
        NodeKind::Text { data } => {
            *segment_budget -= 1;
            let mut end = data.len().min(*budget);
            while !data.is_char_boundary(end) {
                end -= 1;
            }
            output.push((node, data[..end].to_owned()));
            *budget -= end;
        },
        NodeKind::Element { .. } => {
            for child in dom.children(node) {
                collect_ruby_text(dom, styled, child, output, budget, segment_budget);
                if *budget == 0 || *segment_budget == 0 {
                    break;
                }
            }
        },
        NodeKind::DocumentFragment | NodeKind::DocumentType { .. } => {},
        NodeKind::Comment { .. }
        | NodeKind::ProcessingInstruction { .. }
        | NodeKind::Document => {},
    }
}

fn ruby_text(
    dom: &Dom,
    styled: &StyledDom,
    owner: NodeIdx,
    source_node: NodeIdx,
    parent_font_size: f32,
    text_budget: &mut usize,
    mapping_budget: &mut usize,
    segment_budget: &mut usize,
) -> RubyText {
    let style = styled.get(owner).cloned().unwrap_or_default();
    let font_size = values::resolve_font_size(
        style.get("font-size").unwrap_or("medium"),
        parent_font_size,
    );
    let mut segments = Vec::new();
    collect_ruby_text(
        dom,
        styled,
        source_node,
        &mut segments,
        text_budget,
        segment_budget,
    );
    let collapses_spaces = !matches!(
        style.get("white-space").unwrap_or("normal"),
        value if value.eq_ignore_ascii_case("pre")
            || value.eq_ignore_ascii_case("pre-wrap")
            || value.eq_ignore_ascii_case("break-spaces")
    );
    let mut text = String::new();
    let mut source_mappings = Vec::new();
    for (source_node, source) in segments {
        let (part, mappings) = text_for_white_space(
            &source,
            &style,
            source_node,
            mapping_budget.saturating_sub(source_mappings.len()),
        );
        let skip = usize::from(collapses_spaces && text.ends_with(' ') && part.starts_with(' '));
        let start = text.len();
        if skip != 0 {
            source_mappings.extend(
                mappings
                    .iter()
                    .filter(|mapping| mapping.text_range.start < skip)
                    .map(|mapping| TextSourceMapping {
                        text_range: start - 1..start,
                        source: mapping.source,
                        source_range: mapping.source_range.clone(),
                    }),
            );
        }
        text.push_str(&part[skip..]);
        source_mappings.extend(
            mappings
                .into_iter()
                .filter(|mapping| mapping.text_range.end > skip)
                .map(|mapping| TextSourceMapping {
                    text_range: start + mapping.text_range.start.saturating_sub(skip)
                        ..start + mapping.text_range.end - skip,
                    source: mapping.source,
                    source_range: mapping.source_range,
                }),
        );
        if source_mappings.len() >= *mapping_budget {
            break;
        }
    }
    *mapping_budget = mapping_budget.saturating_sub(source_mappings.len());
    RubyText {
        node: owner,
        text,
        source_mappings,
        font_size,
        style: Box::new(text_style(dom, owner, &style, font_size, true)),
        color: style.get("color").unwrap_or("black").to_owned(),
    }
}

fn empty_ruby_text(
    dom: &Dom,
    styled: &StyledDom,
    owner: NodeIdx,
    parent_font_size: f32,
) -> RubyText {
    let style = styled.get(owner).cloned().unwrap_or_default();
    let font_size = values::resolve_font_size(
        style.get("font-size").unwrap_or("medium"),
        parent_font_size,
    );
    RubyText {
        node: owner,
        text: String::new(),
        source_mappings: Vec::new(),
        font_size,
        style: Box::new(text_style(dom, owner, &style, font_size, true)),
        color: style.get("color").unwrap_or("black").to_owned(),
    }
}

fn ruby_text_sequence(
    dom: &Dom,
    styled: &StyledDom,
    owner: NodeIdx,
    nodes: &[NodeIdx],
    parent_font_size: f32,
    text_budget: &mut usize,
    mapping_budget: &mut usize,
    segment_budget: &mut usize,
) -> RubyText {
    let mut combined = empty_ruby_text(dom, styled, owner, parent_font_size);
    for &source_node in nodes {
        let part = ruby_text(
            dom,
            styled,
            owner,
            source_node,
            parent_font_size,
            text_budget,
            mapping_budget,
            segment_budget,
        );
        let skip = usize::from(combined.text.ends_with(' ') && part.text.starts_with(' '));
        let start = combined.text.len();
        combined.text.push_str(&part.text[skip..]);
        combined.source_mappings.extend(
            part.source_mappings
                .into_iter()
                .map(|mapping| TextSourceMapping {
                    text_range: if skip != 0 && mapping.text_range.start < skip {
                        start - 1..start + mapping.text_range.end.saturating_sub(skip)
                    } else {
                        start + mapping.text_range.start.saturating_sub(skip)
                            ..start + mapping.text_range.end.saturating_sub(skip)
                    },
                    source: mapping.source,
                    source_range: mapping.source_range,
                }),
        );
    }
    combined
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RubyRole {
    Base,
    Annotation,
    BaseContainer,
    AnnotationContainer,
}

fn ruby_role(_dom: &Dom, styled: &StyledDom, node: NodeIdx) -> Option<RubyRole> {
    match styled.get(node).and_then(|style| style.get("display")) {
        Some(value) if value.eq_ignore_ascii_case("ruby-base") => Some(RubyRole::Base),
        Some(value) if value.eq_ignore_ascii_case("ruby-text") => Some(RubyRole::Annotation),
        Some(value) if value.eq_ignore_ascii_case("ruby-base-container") => {
            Some(RubyRole::BaseContainer)
        },
        Some(value) if value.eq_ignore_ascii_case("ruby-text-container") => {
            Some(RubyRole::AnnotationContainer)
        },
        _ => None,
    }
}

fn is_ruby_container(_dom: &Dom, styled: &StyledDom, node: NodeIdx) -> bool {
    styled
        .get(node)
        .and_then(|style| style.get("display"))
        .is_some_and(|value| value.eq_ignore_ascii_case("ruby"))
}

fn ruby_inline(
    dom: &Dom,
    styled: &StyledDom,
    node: NodeIdx,
    parent_font_size: f32,
) -> InlineNode {
    let mut text_budget = MAX_RUBY_TEXT_BYTES;
    let mut mapping_budget = MAX_RUBY_SOURCE_MAPPINGS;
    let mut segment_budget = MAX_RUBY_SOURCE_MAPPINGS;
    let mut bases = Vec::new();
    let mut annotations = Vec::new();
    enum Entry {
        Role(RubyRole, NodeIdx),
        Anonymous(Vec<NodeIdx>),
    }
    let mut entries = Vec::new();
    for child in dom.children(node).take(MAX_RUBY_COLUMNS) {
        if styled.get(child).is_some_and(is_display_none) {
            continue;
        }
        if let Some(role) = ruby_role(dom, styled, child) {
            entries.push(Entry::Role(role, child));
        } else if let Some(Entry::Anonymous(nodes)) = entries.last_mut() {
            nodes.push(child);
        } else {
            entries.push(Entry::Anonymous(vec![child]));
        }
    }
    for entry in entries {
        let (role, child) = match entry {
            Entry::Role(role, child) => (role, child),
            Entry::Anonymous(nodes) => {
                let candidate = ruby_text_sequence(
                    dom,
                    styled,
                    node,
                    &nodes,
                    parent_font_size,
                    &mut text_budget,
                    &mut mapping_budget,
                    &mut segment_budget,
                );
                if bases.len() < MAX_RUBY_COLUMNS && !candidate.text.trim().is_empty() {
                    bases.push(candidate);
                }
                continue;
            },
        };
        match role {
            RubyRole::Annotation if annotations.len() < MAX_RUBY_COLUMNS => {
                annotations.push((
                    ruby_text(dom, styled, child, child, parent_font_size, &mut text_budget, &mut mapping_budget, &mut segment_budget),
                    1,
                ));
            },
            RubyRole::Annotation => {},
            RubyRole::AnnotationContainer => {
                let mut found = false;
                for annotation in dom
                    .children(child)
                    .take(MAX_RUBY_COLUMNS.saturating_sub(annotations.len()))
                {
                    if ruby_role(dom, styled, annotation) == Some(RubyRole::Annotation) {
                        found = true;
                        annotations.push((
                            ruby_text(
                                dom,
                                styled,
                                annotation,
                                annotation,
                                parent_font_size,
                                &mut text_budget,
                                &mut mapping_budget,
                                &mut segment_budget,
                            ),
                            1,
                        ));
                    }
                }
                if !found {
                    if annotations.len() < MAX_RUBY_COLUMNS {
                        annotations.push((
                            ruby_text(dom, styled, child, child, parent_font_size, &mut text_budget, &mut mapping_budget, &mut segment_budget),
                            usize::MAX,
                        ));
                    }
                }
            },
            RubyRole::BaseContainer => {
                let mut found = false;
                for base in dom
                    .children(child)
                    .take(MAX_RUBY_COLUMNS.saturating_sub(bases.len()))
                {
                    if ruby_role(dom, styled, base) == Some(RubyRole::Base) {
                        found = true;
                        bases.push(ruby_text(
                            dom,
                            styled,
                            base,
                            base,
                            parent_font_size,
                            &mut text_budget,
                            &mut mapping_budget,
                            &mut segment_budget,
                        ));
                    }
                }
                if !found {
                    if bases.len() < MAX_RUBY_COLUMNS {
                        bases.push(ruby_text(
                            dom,
                            styled,
                            child,
                            child,
                            parent_font_size,
                            &mut text_budget,
                            &mut mapping_budget,
                            &mut segment_budget,
                        ));
                    }
                }
            },
            RubyRole::Base if bases.len() < MAX_RUBY_COLUMNS => {
                bases.push(ruby_text(
                    dom,
                    styled,
                    child,
                    child,
                    parent_font_size,
                    &mut text_budget,
                    &mut mapping_budget,
                    &mut segment_budget,
                ));
            },
            RubyRole::Base => {},
        }
    }
    // Build the fallback only once. Re-traversing the full ruby subtree for
    // every missing base makes adversarial annotation-only markup quadratic.
    let empty_base_template = empty_ruby_text(dom, styled, node, parent_font_size);
    let column_count = bases.len().max(annotations.len()).min(MAX_RUBY_COLUMNS);
    let mut columns = Vec::with_capacity(column_count);
    for index in 0..column_count {
        let mut base = bases
            .get(index)
            .cloned()
            .unwrap_or_else(|| empty_base_template.clone());
        if index >= bases.len() {
            base.text.clear();
            base.source_mappings.clear();
        }
        columns.push(RubyColumn {
            base,
            annotation: None,
            annotation_span: 1,
            annotation_hidden: false,
        });
    }
    let mut column = 0usize;
    for (annotation, requested_span) in annotations {
        if column >= columns.len() {
            if columns.len() >= MAX_RUBY_COLUMNS {
                break;
            }
            let base = empty_base_template.clone();
            columns.push(RubyColumn {
                base,
                annotation: None,
                annotation_span: 1,
                annotation_hidden: false,
            });
        }
        let remaining = columns.len() - column;
        let span = if requested_span == usize::MAX {
            remaining
        } else {
            requested_span.min(remaining)
        }
        .max(1);
        let hidden = annotation.text
            == columns[column..column + span]
                .iter()
                .map(|column| column.base.text.as_str())
                .collect::<String>();
        columns[column].annotation = Some(annotation);
        columns[column].annotation_span = span;
        columns[column].annotation_hidden = hidden;
        column += span;
    }
    InlineNode::Ruby { node, columns }
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
        for entry in formatting_children(dom, styled, idx, font_size) {
            let child = entry.node;
            let text_parent = styled.get(entry.owner).cloned().unwrap_or_default();
            match &dom.get(child).kind {
                NodeKind::Text { data } => {
                    if !text_for_white_space(data, &text_parent, child, usize::MAX).0.is_empty() {
                        run.push(inline_text(
                            dom,
                            entry.owner,
                            child,
                            data.clone(),
                            &text_parent,
                            entry.parent_font,
                            false,
                        ));
                    }
                }
                NodeKind::DocumentFragment | NodeKind::DocumentType { .. } => {},
                NodeKind::Comment { .. }
                | NodeKind::ProcessingInstruction { .. }
                | NodeKind::Document => {}
                NodeKind::Element { .. } => {
                    let child_style = styled.get(child).cloned().unwrap_or_default();
                    if is_display_none(&child_style) || crate::replaced::hidden(dom, child) {
                        continue;
                    }
                    if is_out_of_flow(&child_style) {
                        positioned.push(build_element(
                            dom, styled, child, entry.parent_font, fonts, replaced,
                        ));
                        continue;
                    }
                    flush_anonymous_into(&mut items, &mut run, font_size, fonts);
                    items.push(build_element(
                        dom, styled, child, entry.parent_font, fonts, replaced,
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

    for entry in formatting_children(dom, styled, idx, font_size) {
        let child = entry.node;
        let text_parent = styled.get(entry.owner).cloned().unwrap_or_default();
        match &dom.get(child).kind {
            NodeKind::Text { data } => {
                // Whitespace-only runs between block-level children carry no
                // rendering; keep non-empty collapsed runs as separators.
                if !text_for_white_space(data, &text_parent, child, usize::MAX).0.is_empty() {
                    inlines.push(inline_text(
                        dom,
                        entry.owner,
                        child,
                        data.clone(),
                        &text_parent,
                        entry.parent_font,
                        false,
                    ));
                }
            }
            NodeKind::DocumentFragment | NodeKind::DocumentType { .. } => {},
            NodeKind::Comment { .. }
            | NodeKind::ProcessingInstruction { .. }
            | NodeKind::Document => {}
            NodeKind::Element { .. } => {
                let child_style = styled.get(child).cloned().unwrap_or_default();
                if is_display_none(&child_style) || crate::replaced::hidden(dom, child) {
                    continue; // no box, no descendants
                }
                if is_out_of_flow(&child_style) {
                    positioned.push(build_element(
                        dom, styled, child, entry.parent_font, fonts, replaced,
                    ));
                    continue;
                }
                match dom.name(child) {
                    // Void line-break element: emits a marker itself (it has
                    // no children, so flatten_inline would see nothing).
                    Some("br") => inlines.push(InlineNode::Break),
                    _ if is_ruby_container(dom, styled, child) => {
                        inlines.push(ruby_inline(dom, styled, child, entry.parent_font));
                    },
                    _ if crate::replaced::inline_atomic(dom, child, &child_style) => {
                        if let Some(item) =
                            crate::replaced::construct(dom, child, &child_style, entry.parent_font, replaced)
                        {
                            inlines.push(item);
                        }
                    }
                    _ if matches!(
                        parse_display(&child_style),
                        Display::Block | Display::Flex | Display::Grid
                    ) =>
                    {
                        // Flush pending inline content into an anonymous block.
                        flush_anonymous_into(&mut blocks, &mut inlines, font_size, fonts);
                        blocks.push(build_element(
                            dom, styled, child, entry.parent_font, fonts, replaced,
                        ));
                    }
                    _ => {
                        let child_font_size = values::resolve_font_size(
                            child_style.get("font-size").unwrap_or("medium"),
                            entry.parent_font,
                        );
                        let edge = boundary_between(
                            dom,
                            child,
                            &text_parent,
                            entry.parent_font,
                            &child_style,
                            child_font_size,
                        );
                        flatten_inline(
                            dom,
                            styled,
                            child,
                            child_style,
                            entry.parent_font,
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
        InlineNode::Break | InlineNode::Replaced { .. } | InlineNode::Ruby { .. } => true,
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

    for entry in formatting_children(dom, styled, idx, font_size) {
        let child = entry.node;
        let text_parent = styled.get(entry.owner).cloned().unwrap_or_default();
        match &dom.get(child).kind {
            NodeKind::Text { data } => {
                if !text_for_white_space(data, &text_parent, child, usize::MAX).0.is_empty() {
                    inlines.push(inline_text(
                        dom,
                        entry.owner,
                        child,
                        data.clone(),
                        &text_parent,
                        entry.parent_font,
                        entry.owner == idx,
                    ));
                }
            }
            NodeKind::DocumentFragment | NodeKind::DocumentType { .. } => {},
            NodeKind::Comment { .. }
            | NodeKind::ProcessingInstruction { .. }
            | NodeKind::Document => {}
            NodeKind::Element { .. } => {
                let cs = styled.get(child).cloned().unwrap_or_default();
                if is_display_none(&cs) || crate::replaced::hidden(dom, child) {
                    continue;
                }
                if is_out_of_flow(&cs) {
                    positioned_out.push(build_element(
                        dom,
                        styled,
                        child,
                        entry.parent_font,
                        fonts,
                        replaced,
                    ));
                    continue;
                }
                match dom.name(child) {
                    Some("br") => inlines.push(InlineNode::Break),
                    _ if is_ruby_container(dom, styled, child) => {
                        inlines.push(ruby_inline(dom, styled, child, entry.parent_font));
                    },
                    _ if crate::replaced::inline_atomic(dom, child, &cs) => {
                        if let Some(item) =
                            crate::replaced::construct(dom, child, &cs, entry.parent_font, replaced)
                        {
                            inlines.push(item);
                        }
                    }
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
                            entry.parent_font,
                            fonts,
                            replaced,
                        ));
                    }
                    _ => {
                        let child_font_size = values::resolve_font_size(
                            cs.get("font-size").unwrap_or("medium"),
                            entry.parent_font,
                        );
                        let edge =
                            boundary_between(dom, child, &text_parent, entry.parent_font, &cs, child_font_size);
                        flatten_inline(
                            dom,
                            styled,
                            child,
                            cs,
                            entry.parent_font,
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
                InlineNode::Ruby { .. } => ("<ruby>", 0.0),
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

    #[test]
    fn ruby_keeps_paired_base_and_annotation_structure() {
        let (_, root) = tree("<body><p><ruby><rb>漢</rb><rt>かん</rt></ruby></p></body>");
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want one ruby inline: {items:#?}")
        };

        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].base.text, "漢");
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "かん");
    }

    #[test]
    fn computed_ruby_text_display_creates_annotations() {
        let (_, root) = tree(
            "<body><p><ruby><rb>base</rb><span style=\"display:ruby-text\">note</span></ruby></p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want one ruby inline: {items:#?}")
        };

        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].base.text, "base");
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "note");
    }

    #[test]
    fn computed_ruby_container_display_uses_ruby_fixup() {
        let (_, root) = tree(
            "<body><p><span style=\"display:ruby\"><b style=\"display:ruby-base\">base</b><i style=\"display:ruby-text\">note</i></span></p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want computed ruby inline: {items:#?}")
        };

        assert_eq!(columns[0].base.text, "base");
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "note");
    }

    #[test]
    fn computed_ruby_role_overrides_html_tag_default() {
        let (_, root) = tree(
            "<body><p><ruby><rb style=\"display:ruby-text\">note</rb></ruby></p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want ruby inline: {items:#?}")
        };

        assert!(columns[0].base.text.is_empty());
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "note");
    }

    #[test]
    fn ruby_fixup_groups_consecutive_anonymous_inline_content() {
        let (_, root) = tree(
            "<body><p><ruby>A<span>B</span><rt>x</rt></ruby></p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want ruby inline: {items:#?}")
        };

        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].base.text, "AB");
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "x");
    }

    #[test]
    fn ruby_parentheses_are_suppressed_by_the_ua_display_rule() {
        let (_, root) = tree(
            "<body><p><ruby>base<rp>(</rp><rt>note</rt><rp>)</rp></ruby></p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want ruby inline: {items:#?}")
        };

        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].base.text, "base");
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "note");
    }

    #[test]
    fn upright_vertical_text_forces_used_ltr_direction() {
        let (_, root) = tree(
            "<body><p style=\"writing-mode:vertical-rl;text-orientation:upright;direction:rtl\">אב</p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let InlineNode::Text { style, .. } = &items[0] else { panic!("want text") };

        assert_eq!(style.direction, Direction::LeftToRight);
    }

    #[test]
    fn ruby_annotation_container_spans_all_base_columns() {
        let (_, root) = tree(
            "<body><p><ruby><rb>A</rb><rb>B</rb><rtc>shared</rtc></ruby></p></body>",
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("want blocks") };
        let Flow::Inlines(items) = &blocks[0].flow else { panic!("want inlines") };
        let [InlineNode::Ruby { columns, .. }] = items.as_slice() else {
            panic!("want one ruby inline: {items:#?}")
        };

        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].annotation_span, 2);
        assert_eq!(columns[0].annotation.as_ref().unwrap().text, "shared");
        assert!(columns[1].annotation.is_none());
    }
}


fn table_span(value: Option<&str>, limit: usize, zero_allowed: bool) -> usize {
    let Some(value) = value else { return 1; };
    let value = value.trim_start_matches([' ', '\t', '\n', '\r', '\u{000c}']);
    let value = value.strip_prefix('+').unwrap_or(value);
    let digits: Vec<_> = value.bytes().take_while(u8::is_ascii_digit).collect();
    if digits.is_empty() { return 1; }
    let value = digits.into_iter().fold(0usize, |n,d| n.saturating_mul(10).saturating_add((d-b'0') as usize).min(limit));
    if zero_allowed {value} else {value.max(1)}
}
