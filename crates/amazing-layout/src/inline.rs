//! Inline formatting context (CSS2.1 §9.4.2 + §10.8 line box math):
//! flattens [`InlineNode`] runs into words, wraps them greedily, and emits
//! absolutely-positioned [`LineBox`]es with baseline positions.
//!
//! Simplifications (documented in lib.rs): one strut per block (the block's
//! font-size/line-height decide every line's height and baseline);
//! text-align left only; no soft-hyphen/shaping — measurement is per-word
//! advance sums from [`FontSource`].

use amazing_dom::NodeIdx;
use amazing_text::{LineRequest, Paragraph, ShapingBoundary, SourceMapping, TextSpan, TextSystem};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use crate::font::FontSource;
use crate::{GlyphFrag, InlineNode, IntrinsicSize, LineBox, ReplacedFrag, WordFrag};

const FIT_TOLERANCE_PX: f32 = 0.01;
const MAX_RUBY_COLUMNS: usize = 65_536;
const MAX_RUBY_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// The block's typographic strut (§10.8): every line box gets this height
/// and this baseline placement.
pub struct Strut {
    pub font_size: f32,
    pub line_height: f32,
    /// Ascent/descent of the strut font at `font_size` (positive numbers).
    pub ascent: f32,
    pub descent: f32,
    pub vertical_ascent: f32,
    pub vertical_descent: f32,
}

impl Strut {
    pub fn new(fonts: &FontSource, font_size: f32, line_height: f32) -> Strut {
        let m = fonts.scaled_metrics(font_size);
        Strut {
            font_size,
            line_height,
            ascent: m.ascent,
            descent: m.descent,
            vertical_ascent: font_size * 0.5,
            vertical_descent: font_size * 0.5,
        }
    }
}

/// One ordered inline item queued for placement.
pub(crate) enum Token {
    Word {
        text: String,
        node: NodeIdx,
        font_size: f32,
        space_before: bool,
    },
    Break,
    Replaced {
        node: NodeIdx,
        source_order: Option<u64>,
        size: IntrinsicSize,
        space_before: bool,
    },
}

/// Splits flattened inline items into word tokens, tracking whether a
/// collapsible space sits between consecutive words across item boundaries.
pub(crate) fn tokenize(items: &[InlineNode]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut pending_space = false;
    for item in items {
        match item {
            InlineNode::BidiStart { .. } | InlineNode::BidiEnd | InlineNode::Ruby { .. } => {},
            InlineNode::Break => {
                tokens.push(Token::Break);
                // Whitespace following a <br> starts the new line stripped,
                // so the pending flag survives the break.
            }
            InlineNode::Text {
                node,
                text,
                font_size,
                ..
            } => {
                // Collapsed text contains at most single-space runs.
                let mut start = 0usize;
                if let Some(rest) = text.strip_prefix(' ') {
                    pending_space = true;
                    start = text.len() - rest.len();
                }
                for word in text[start..].split(' ') {
                    if word.is_empty() {
                        continue; // the run separator itself
                    }
                    tokens.push(Token::Word {
                        text: word.to_owned(),
                        node: *node,
                        font_size: *font_size,
                        space_before: pending_space,
                    });
                    pending_space = true;
                }
                if !text.ends_with(' ') && !text.is_empty() {
                    pending_space = false;
                }
            }
            InlineNode::Replaced { node, size, .. } | InlineNode::Atomic { node, size, .. } => {
                let source_order = match item {
                    InlineNode::Replaced { replaced, .. } => replaced.source_order,
                    _ => None,
                };
                tokens.push(Token::Replaced {
                    node: *node,
                    source_order,
                    size: *size,
                    space_before: pending_space,
                });
                pending_space = false;
            }
        }
    }
    tokens
}

struct AtomicLineStyle {
    baseline_offset: f32,
    vertical_align: String,
}

fn atomic_baselines(items: &[InlineNode]) -> HashMap<u32, AtomicLineStyle> {
    items.iter().filter_map(|item| match item {
        InlineNode::Atomic { node, baseline_offset, vertical_align, .. } => Some((node.0, AtomicLineStyle {
            baseline_offset: *baseline_offset,
            vertical_align: vertical_align.clone(),
        })),
        _ => None,
    }).collect()
}

fn atomic_baseline_offset(style: &AtomicLineStyle, height: f32, strut: &Strut) -> Option<f32> {
    let half_leading = (strut.line_height - (strut.ascent + strut.descent)) / 2.0;
    let text_ascent = half_leading + strut.ascent;
    let text_descent = strut.line_height - text_ascent;
    match style.vertical_align.as_str() {
        "top" | "bottom" => None,
        "middle" => Some(height * 0.5 + strut.font_size * 0.25),
        "text-top" => Some(text_ascent),
        "text-bottom" => Some(height - text_descent),
        "sub" => Some(style.baseline_offset - strut.font_size * 0.2),
        "super" => Some(style.baseline_offset + strut.font_size * 0.2),
        _ => Some(style.baseline_offset),
    }
}

fn line_metrics(
    fragments: &[ReplacedFrag],
    atomic_baselines: &HashMap<u32, AtomicLineStyle>,
    strut: &Strut,
    ruby_over: f32,
) -> (f32, f32) {
    let half_leading = (strut.line_height - (strut.ascent + strut.descent)) / 2.0;
    let text_ascent = half_leading + strut.ascent + ruby_over;
    let tallest = fragments.iter().map(|fragment| fragment.height).fold(0.0, f32::max);
    if !fragments.iter().any(|fragment| {
        atomic_baselines.get(&fragment.node.0).is_some_and(|style| {
            style.vertical_align != "baseline" || style.baseline_offset < fragment.height - 0.001
        })
    }) {
        return ((strut.line_height + ruby_over).max(tallest), text_ascent.max(tallest));
    }
    let ascent = fragments.iter().fold(text_ascent, |ascent, fragment| {
        let offset = atomic_baselines.get(&fragment.node.0)
            .and_then(|style| atomic_baseline_offset(style, fragment.height, strut));
        ascent.max(offset.unwrap_or(0.0))
    });
    let descent = fragments.iter().fold(strut.line_height - half_leading - strut.ascent, |descent, fragment| {
        let offset = atomic_baselines.get(&fragment.node.0)
            .and_then(|style| atomic_baseline_offset(style, fragment.height, strut));
        descent.max(offset.map_or(0.0, |offset| fragment.height - offset))
    });
    (fragments.iter().filter(|fragment| {
        atomic_baselines.get(&fragment.node.0).is_some_and(|style| {
            matches!(style.vertical_align.as_str(), "top" | "bottom")
        })
    }).fold(ascent + descent, |height, fragment| height.max(fragment.height)), ascent)
}

fn atomic_fragment_y(
    fragment: &ReplacedFrag,
    styles: &HashMap<u32, AtomicLineStyle>,
    strut: &Strut,
    y_top: f32,
    height: f32,
    baseline: f32,
) -> f32 {
    match styles.get(&fragment.node.0) {
        Some(style) if style.vertical_align == "top" => y_top,
        Some(style) if style.vertical_align == "bottom" => y_top + height - fragment.height,
        Some(style) => baseline - atomic_baseline_offset(style, fragment.height, strut)
            .unwrap_or(fragment.height),
        None => baseline - fragment.height,
    }
}

/// Lays out inline content inside a content box starting at
/// (`content_x`, `content_y`). Returns the produced line boxes with
/// absolute coordinates.
pub fn layout_lines(
    items: &[InlineNode],
    content_x: f32,
    content_y: f32,
    content_width: f32,
    strut: &Strut,
    fonts: &FontSource,
) -> Vec<LineBox> {
    layout_lines_with_floats(
        items, content_x, content_y, content_width, strut, fonts,
        &crate::floats::FloatContext::default(),
        true,
    )
}

pub(crate) fn layout_lines_with_floats(
    items: &[InlineNode],
    content_x: f32,
    content_y: f32,
    content_width: f32,
    strut: &Strut,
    fonts: &FontSource,
    floats: &crate::floats::FloatContext,
    soft_wrap: bool,
) -> Vec<LineBox> {
    let tokens = tokenize(items);
    let atomic_baselines = atomic_baselines(items);

    let line_y = std::cell::Cell::new(content_y);
    let line_left = std::cell::Cell::new(content_x);
    let right_edge = std::cell::Cell::new(content_x + content_width);

    let mut lines: Vec<LineBox> = Vec::new();
    let mut words: Vec<WordFrag> = Vec::new();
    let mut replaced: Vec<ReplacedFrag> = Vec::new();
    let mut cx = content_x;

    let flush = |lines: &mut Vec<LineBox>,
                 words: &mut Vec<WordFrag>,
                 replaced: &mut Vec<ReplacedFrag>,
                 cx: &mut f32| {
        if words.is_empty() && replaced.is_empty() {
            return;
        }
        let (height, baseline_offset) = line_metrics(replaced, &atomic_baselines, strut, 0.0);
        let y_top = line_y.get();
        let baseline = y_top + baseline_offset;
        for image in replaced.iter_mut() {
            image.y = atomic_fragment_y(image, &atomic_baselines, strut, y_top, height, baseline);
        }
        lines.push(LineBox {
            y_top,
            height,
            baseline,
            words: std::mem::take(words),
            glyphs: Vec::new(),
            decorations: Vec::new(),
            replaced: std::mem::take(replaced),
        });
        line_y.set(y_top + height);
        let (left, right) = floats.band(content_x, content_width, line_y.get(), strut.line_height);
        line_left.set(left);
        right_edge.set(right);
        *cx = left;
    };

    let prepare = |cx: &mut f32, needed: f32, height: f32| {
        let (left, right, y) = floats.fit(content_x, content_width, line_y.get(), height, needed);
        line_y.set(y);
        line_left.set(left);
        right_edge.set(right);
        *cx = left;
    };
    for token in tokens {
        let has_content = !words.is_empty() || !replaced.is_empty();
        match token {
            Token::Break => flush(&mut lines, &mut words, &mut replaced, &mut cx),
            Token::Word {
                text,
                node,
                font_size,
                space_before,
            } => {
                let word_w = fonts.measure(&text, font_size);
                if !has_content {
                    prepare(&mut cx, word_w, strut.line_height);
                }
                let space_w = if has_content && space_before {
                    fonts.measure(" ", font_size)
                } else {
                    0.0
                };
                if soft_wrap && has_content && cx + space_w + word_w > right_edge.get() + FIT_TOLERANCE_PX {
                    flush(&mut lines, &mut words, &mut replaced, &mut cx);
                    prepare(&mut cx, word_w, strut.line_height);
                }
                let wx = cx + if cx > line_left.get() { space_w } else { 0.0 };
                words.push(WordFrag {
                    node,
                    text,
                    x: wx,
                    width: word_w,
                    font_size,
                });
                cx = wx + word_w;
            }
            Token::Replaced {
                node,
                source_order,
                size,
                space_before,
            } => {
                if !has_content {
                    prepare(&mut cx, size.width, strut.line_height.max(size.height));
                }
                let space_w = if has_content && space_before {
                    fonts.measure(" ", strut.font_size)
                } else {
                    0.0
                };
                if soft_wrap && has_content && cx + space_w + size.width > right_edge.get() + FIT_TOLERANCE_PX {
                    flush(&mut lines, &mut words, &mut replaced, &mut cx);
                    prepare(&mut cx, size.width, strut.line_height.max(size.height));
                }
                let x = cx + if cx > line_left.get() { space_w } else { 0.0 };
                replaced.push(ReplacedFrag {
                    node,
                    source_order,
                    x,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                });
                cx = x + size.width;
            }
        }
    }
    flush(&mut lines, &mut words, &mut replaced, &mut cx);

    crate::replaced::finish_lines(&mut lines, items);
    lines
}

#[derive(Debug)]
struct ShapedWord {
    node: NodeIdx,
    text: String,
    width: f32,
    font_size: f32,
    space_before: f32,
    glyphs: Vec<GlyphFrag>,
    decorations: Vec<ShapedDecoration>,
    range: Range<usize>,
    context: Rc<ShapedContext>,
    preserved_space_before: f32,
    break_after: bool,
    hyphen_after: bool,
    hyphen_advance: f32,
    requires_reshape: bool,
}

#[derive(Debug)]
struct ShapedDecoration {
    node: NodeIdx,
    x: f32,
    width: f32,
    color: String,
    padding_top: f32,
    padding_bottom: f32,
}

#[derive(Debug, Clone)]
struct ShapedRuby {
    node: NodeIdx,
    text: String,
    width: f32,
    base_width: f32,
    overhang_spaces: bool,
    font_size: f32,
    glyphs: Vec<GlyphFrag>,
    line_over: f32,
    break_after: bool,
    space_before: f32,
    preserved_space_before: f32,
}

#[derive(Debug, Clone)]
enum AtomicInline {
    Replaced(NodeIdx, IntrinsicSize, Option<u64>),
    Combined(ShapedRuby),
    Ruby(ShapedRuby),
}

#[derive(Debug, Clone)]
struct InlinePaintInfo {
    background_color: String,
    padding: crate::values::Sides,
}

#[derive(Debug)]
struct ShapedContext {
    text: String,
    spans: Vec<TextSpan>,
    spacing: Vec<SpacingSpan>,
    sources: Vec<SourceMapping>,
    bidi_scopes: Vec<amazing_text::BidiScope>,
    colors: HashMap<u32, String>,
    paint_info: HashMap<u32, InlinePaintInfo>,
    atomics: HashMap<usize, AtomicInline>,
    base_direction: amazing_text::Direction,
}

#[derive(Debug, Clone, Copy)]
struct SpacingSpan {
    start: usize,
    end: usize,
    letter: f32,
    word: crate::values::LengthPercentage,
    font_size: f32,
}

fn spacing_at(spans: &[SpacingSpan], offset: usize) -> (f32, crate::values::LengthPercentage, f32) {
    spans.iter().find(|span| span.start <= offset && offset < span.end)
        .map_or((0.0, crate::values::LengthPercentage::default(), 0.0),
            |span| (span.letter, span.word, span.font_size))
}

fn separator_spacing(text: &str, range: Range<usize>, spans: &[SpacingSpan]) -> f32 {
    text[range.clone()].char_indices().filter(|(_, ch)| *ch == ' ' || *ch == '\u{00a0}')
        .map(|(offset, _)| {
            let (letter, word, font_size) = spacing_at(spans, range.start + offset);
            // One gap on either side of each surviving word separator.
            2.0 * letter + word.resolve(Some(font_size)).unwrap_or(0.0)
        }).sum()
}

#[derive(Clone)]
struct PlacedText {
    context: Rc<ShapedContext>,
    range: Range<usize>,
    x: f32,
    hyphen_after: bool,
    requires_reshape: bool,
}

enum ShapedUnit {
    Word(ShapedWord),
    Break,
    Replaced {
        node: NodeIdx,
        source_order: Option<u64>,
        break_after: bool,
        size: IntrinsicSize,
        space_before: f32,
        preserved_space_before: f32,
        range: Range<usize>,
        context: Rc<ShapedContext>,
    },
    Ruby(ShapedRuby),
    Combined {
        shaped: ShapedRuby,
        break_after: bool,
        range: Range<usize>,
        context: Rc<ShapedContext>,
    },
}

/// A line break can only precede a run whose internal provider boundaries are
/// unbreakable. Look ahead before placing its first unit, so an opening
/// bracket and following atomic text move to the next line together.
fn unbreakable_run_widths(units: &[ShapedUnit]) -> Vec<f32> {
    let mut needed = vec![0.0; units.len()];
    for index in (0..units.len()).rev() {
        let (width, break_after) = match &units[index] {
            ShapedUnit::Word(word) => (word.width, word.break_after),
            ShapedUnit::Replaced { size, break_after, .. } => (size.width, *break_after),
            ShapedUnit::Ruby(ruby) => (ruby.width, ruby.break_after),
            ShapedUnit::Combined { shaped, break_after, .. } => (shaped.width, *break_after),
            ShapedUnit::Break => continue,
        };
        let continuation = if break_after { 0.0 } else {
            match units.get(index + 1) {
                Some(ShapedUnit::Word(next)) => next.space_before + needed[index + 1],
                Some(ShapedUnit::Ruby(next)) => next.space_before + needed[index + 1],
                Some(ShapedUnit::Replaced { space_before, .. }) => space_before + needed[index + 1],
                Some(ShapedUnit::Break) | None => 0.0,
                Some(_) => needed[index + 1],
            }
        };
        needed[index] = width + continuation;
    }
    needed
}

/// Shapes paragraph chunks before wrapping, then constructs word summaries,
/// positioned glyphs, and line boxes from the same advances.
pub(crate) struct InlineContainingBlock {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub(crate) fn layout_lines_shaped(
    items: &[InlineNode],
    containing: InlineContainingBlock,
    strut: &Strut,
    base_direction: amazing_text::Direction,
    container_writing_mode: amazing_text::WritingMode,
    text_system: &dyn TextSystem,
    floats: &crate::floats::FloatContext,
    soft_wrap: bool,
) -> Option<Vec<LineBox>> {
    let InlineContainingBlock {
        x: content_x,
        y: content_y,
        width: content_width,
        height: content_height,
    } = containing;
    let item_refs = items.iter().collect::<Vec<_>>();
    let units = shape_inline_items(&item_refs, base_direction, text_system)?;
    let writing_mode = items
        .iter()
        .find_map(|item| match item {
            InlineNode::Text { style, .. } => Some(style.writing_mode),
            InlineNode::Ruby { columns, .. } => {
                columns.first().map(|column| column.base.style.writing_mode)
            },
            _ => None,
        })
        .unwrap_or(container_writing_mode);
    let vertical_flow = writing_mode.is_vertical_flow();

    let atomic_baselines = atomic_baselines(items);
    let right_edge = content_x + content_width;
    let inline_start = if vertical_flow { content_y } else { content_x };
    let inline_size = if vertical_flow && content_height > 0.0 {
        content_height
    } else {
        content_width
    };
    let inline_end = std::cell::Cell::new(inline_start + inline_size);
    let line_y = std::cell::Cell::new(content_y);
    let mut lines = Vec::new();
    let mut words = Vec::new();
    let mut glyphs = Vec::new();
    let mut ruby_glyphs = Vec::new();
    let mut ruby_line_over = 0.0f32;
    let mut decorations = Vec::new();
    let mut replaced = Vec::new();
    let mut placed_text = Vec::new();
    let mut cursor = inline_start;
    let mut may_break_after = true;
    let flush = |lines: &mut Vec<LineBox>,
                 words: &mut Vec<WordFrag>,
                 glyphs: &mut Vec<GlyphFrag>,
                 ruby_glyphs: &mut Vec<GlyphFrag>,
                 ruby_line_over: &mut f32,
                 decorations: &mut Vec<crate::InlineDecorationFrag>,
                 replaced: &mut Vec<ReplacedFrag>,
                 placed_text: &mut Vec<PlacedText>,
                 select_discretionary: bool,
                 force_empty: bool,
                 cursor: &mut f32| {
        if words.is_empty() && replaced.is_empty() && !force_empty {
            return;
        }
        let image_height = replaced
            .iter()
            .map(|image| image.height)
            .fold(0.0, f32::max);
        let (atomic_height, baseline_offset) = line_metrics(replaced, &atomic_baselines, strut, *ruby_line_over);
        let height = if vertical_flow {
            (*cursor - inline_start).max(strut.line_height).max(image_height)
        } else {
            atomic_height
        };
        let y_top = if vertical_flow {
            content_y
        } else {
            line_y.get()
        };
        let baseline = y_top + baseline_offset;
        for image in replaced.iter_mut() {
            image.y = atomic_fragment_y(image, &atomic_baselines, strut, y_top, height, baseline);
        }
        for decoration in decorations.iter_mut() {
            decoration.y += baseline;
        }
        reshape_visual_line(
            placed_text,
            glyphs,
            decorations,
            replaced,
            baseline,
            strut,
            select_discretionary,
            text_system,
        );
        glyphs.append(ruby_glyphs);
        if vertical_flow {
            glyphs.sort_by(|left, right| left.y.total_cmp(&right.y));
        } else {
            glyphs.sort_by(|left, right| left.x.total_cmp(&right.x));
        }
        if vertical_flow {
            let column_offset = lines.len() as f32 * strut.line_height;
            let desired_baseline = match writing_mode {
                amazing_text::WritingMode::VerticalRl
                | amazing_text::WritingMode::SidewaysRl => {
                    right_edge - strut.vertical_ascent - column_offset
                },
                amazing_text::WritingMode::VerticalLr
                | amazing_text::WritingMode::SidewaysLr => {
                    // The provider's upright glyph origin sits a horizontal
                    // descent to the left of the line's content edge.
                    content_x + strut.descent + column_offset
                },
                amazing_text::WritingMode::HorizontalTb => content_x,
            };
            let shift = desired_baseline - inline_start;
            for glyph in glyphs.iter_mut() {
                glyph.x += shift;
            }
            for word in words.iter_mut() {
                word.x += shift;
            }
            for decoration in decorations.iter_mut() {
                decoration.x += shift;
            }
            for image in replaced.iter_mut() {
                image.x += shift;
            }
        }
        lines.push(LineBox {
            y_top,
            height,
            baseline,
            words: std::mem::take(words),
            glyphs: std::mem::take(glyphs),
            decorations: std::mem::take(decorations),
            replaced: std::mem::take(replaced),
        });
        *ruby_line_over = 0.0;
        line_y.set(y_top + height);
        *cursor = if vertical_flow { inline_start } else {
            let (left, right) = floats.band(content_x, content_width, line_y.get(), strut.line_height);
            inline_end.set(right);
            left
        };
    };

    let prepare = |cursor: &mut f32, needed: f32, height: f32| {
        if !vertical_flow {
            let (left, right, y) = floats.fit(content_x, content_width, line_y.get(), height, needed);
            *cursor = left;
            inline_end.set(right);
            line_y.set(y);
        }
    };
    let needed = unbreakable_run_widths(&units);
    for (unit_index, unit) in units.into_iter().enumerate() {
        let has_content = !words.is_empty() || !replaced.is_empty();
        match unit {
            ShapedUnit::Break => {
                flush(
                    &mut lines,
                    &mut words,
                    &mut glyphs,
                    &mut ruby_glyphs,
                    &mut ruby_line_over,
                    &mut decorations,
                    &mut replaced,
                    &mut placed_text,
                    true,
                    true,
                    &mut cursor,
                );
                may_break_after = true;
            }
            ShapedUnit::Word(mut word) => {
                if !has_content {
                    prepare(&mut cursor, word.width, strut.line_height);
                }
                let mut spacing = if has_content {
                    word.space_before
                } else {
                    word.preserved_space_before
                };
                let break_width = if word.hyphen_after {
                    word.hyphen_advance
                } else {
                    0.0
                };
                if has_content
                    && soft_wrap
                    && may_break_after
                    && cursor + spacing + needed[unit_index].max(word.width + break_width)
                        > inline_end.get() + FIT_TOLERANCE_PX
                {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut ruby_glyphs,
                        &mut ruby_line_over,
                        &mut decorations,
                        &mut replaced,
                        &mut placed_text,
                        true,
                        false,
                        &mut cursor,
                    );
                    prepare(&mut cursor, word.width, strut.line_height);
                    spacing = word.preserved_space_before;
                }
                let x = cursor + spacing;
                placed_text.push(PlacedText {
                    context: Rc::clone(&word.context),
                    range: word.range.clone(),
                    x: x - word.preserved_space_before,
                    hyphen_after: word.hyphen_after,
                    requires_reshape: word.requires_reshape,
                });
                for glyph in &mut word.glyphs {
                    glyph.x += x;
                }
                glyphs.extend(word.glyphs);
                for decoration in word.decorations {
                    decorations.push(crate::InlineDecorationFrag {
                        node: decoration.node,
                        x: x + decoration.x,
                        y: -(strut.ascent + decoration.padding_top),
                        width: decoration.width,
                        height: strut.ascent
                            + strut.descent
                            + decoration.padding_top
                            + decoration.padding_bottom,
                        color: decoration.color,
                    });
                }
                words.push(WordFrag {
                    node: word.node,
                    text: word.text,
                    x,
                    width: word.width,
                    font_size: word.font_size,
                });
                cursor = x + word.width;
                may_break_after = word.break_after && (!word.hyphen_after
                    || cursor + word.hyphen_advance <= inline_end.get() + FIT_TOLERANCE_PX);
            }
            ShapedUnit::Replaced {
                node,
                source_order,
                break_after,
                size,
                space_before,
                preserved_space_before,
                range,
                context,
            } => {
                if !has_content {
                    prepare(&mut cursor, size.width, strut.line_height.max(size.height));
                }
                let mut spacing = if has_content { space_before } else { preserved_space_before };
                if has_content
                    && soft_wrap
                    && may_break_after
                    && cursor + spacing + needed[unit_index] > inline_end.get() + FIT_TOLERANCE_PX
                {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut ruby_glyphs,
                        &mut ruby_line_over,
                        &mut decorations,
                        &mut replaced,
                        &mut placed_text,
                        true,
                        false,
                        &mut cursor,
                    );
                    prepare(&mut cursor, size.width, strut.line_height.max(size.height));
                    spacing = preserved_space_before;
                }
                cursor += spacing;
                replaced.push(ReplacedFrag {
                    node,
                    source_order,
                    x: cursor,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                });
                placed_text.push(PlacedText {
                    context,
                    range,
                    x: cursor,
                    hyphen_after: false,
                    requires_reshape: false,
                });
                cursor += size.width;
                may_break_after = break_after;
            }
            ShapedUnit::Ruby(mut ruby) => {
                if !has_content {
                    prepare(&mut cursor, ruby.width, strut.line_height + ruby.line_over);
                }
                let mut spacing = if has_content { ruby.space_before } else { ruby.preserved_space_before };
                if has_content
                    && soft_wrap
                    && may_break_after
                    && cursor + spacing + needed[unit_index] > inline_end.get() + FIT_TOLERANCE_PX
                {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut ruby_glyphs,
                        &mut ruby_line_over,
                        &mut decorations,
                        &mut replaced,
                        &mut placed_text,
                        true,
                        false,
                        &mut cursor,
                    );
                    prepare(&mut cursor, ruby.width, strut.line_height + ruby.line_over);
                    spacing = ruby.preserved_space_before;
                }
                cursor += spacing;
                for glyph in &mut ruby.glyphs {
                    if glyph.writing_mode.is_vertical_flow() {
                        glyph.y += cursor;
                    } else {
                        glyph.x += cursor;
                    }
                }
                ruby_glyphs.extend(ruby.glyphs);
                ruby_line_over = ruby_line_over.max(ruby.line_over);
                words.push(WordFrag {
                    node: ruby.node,
                    text: ruby.text,
                    x: cursor,
                    width: ruby.width,
                    font_size: ruby.font_size,
                });
                cursor += ruby.width;
                may_break_after = ruby.break_after;
            }
            ShapedUnit::Combined {
                shaped: combined,
                break_after,
                range,
                context,
            } => {
                if !has_content {
                    prepare(&mut cursor, combined.width, strut.line_height);
                }
                if has_content
                    && soft_wrap
                    && may_break_after
                    && cursor + needed[unit_index] > inline_end.get() + FIT_TOLERANCE_PX
                {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut ruby_glyphs,
                        &mut ruby_line_over,
                        &mut decorations,
                        &mut replaced,
                        &mut placed_text,
                        true,
                        false,
                        &mut cursor,
                    );
                    prepare(&mut cursor, combined.width, strut.line_height);
                }
                placed_text.push(PlacedText {
                    context,
                    range,
                    x: cursor,
                    hyphen_after: false,
                    requires_reshape: false,
                });
                words.push(WordFrag {
                    node: combined.node,
                    text: combined.text,
                    x: cursor,
                    width: combined.width,
                    font_size: combined.font_size,
                });
                cursor += combined.width;
                may_break_after = break_after;
            }
        }
    }
    flush(
        &mut lines,
        &mut words,
        &mut glyphs,
        &mut ruby_glyphs,
        &mut ruby_line_over,
        &mut decorations,
        &mut replaced,
        &mut placed_text,
        false,
        false,
        &mut cursor,
    );
    crate::replaced::finish_lines(&mut lines, items);
    Some(lines)
}

fn reshape_visual_line(
    placed: &mut Vec<PlacedText>,
    glyphs: &mut Vec<GlyphFrag>,
    decorations: &mut Vec<crate::InlineDecorationFrag>,
    replaced: &mut [ReplacedFrag],
    baseline: f32,
    strut: &Strut,
    select_discretionary: bool,
    text_system: &dyn TextSystem,
) {
    let Some(first) = placed.first() else {
        return;
    };
    let one_context = placed
        .iter()
        .all(|item| Rc::ptr_eq(&item.context, &first.context));
    if !one_context {
        let mut rebuilt = Vec::new();
        let mut rebuilt_decorations = Vec::new();
        let mut start = 0usize;
        while start < placed.len() {
            let mut end = start + 1;
            while end < placed.len()
                && Rc::ptr_eq(&placed[start].context, &placed[end].context)
            {
                end += 1;
            }
            let mut group = placed[start..end].to_vec();
            let mut group_glyphs = Vec::new();
            let mut group_decorations = Vec::new();
            reshape_visual_line(
                &mut group,
                &mut group_glyphs,
                &mut group_decorations,
                &mut [],
                baseline,
                strut,
                false,
                text_system,
            );
            if group_glyphs.is_empty()
                && placed[start..end].iter().any(|item| item.requires_reshape)
            {
                glyphs.clear();
                placed.clear();
                return;
            }
            rebuilt.extend(group_glyphs);
            rebuilt_decorations.extend(group_decorations);
            start = end;
        }
        if select_discretionary && placed.last().is_some_and(|item| item.hyphen_after) {
            let pen = rebuilt
                .last()
                .map_or(first.x, |glyph| glyph.x + glyph.advance_x.abs());
            let last = placed.last().unwrap();
            let rtl = resolved_direction_at_break(
                &last.context,
                last.range.clone(),
                last.range.end,
                text_system,
            ) == amazing_text::Direction::RightToLeft;
            append_discretionary_hyphen(
                &last.context,
                &mut rebuilt,
                &mut rebuilt_decorations,
                replaced,
                DiscretionaryPlacement {
                    line_start: first.x,
                    pen,
                    break_offset: last.range.end,
                    rtl,
                },
                text_system,
            );
        }
        *glyphs = rebuilt;
        *decorations = rebuilt_decorations;
        placed.clear();
        return;
    }
    let requires_reshape = placed.iter().any(|item| item.requires_reshape);
    let range = first.range.start..placed.last().unwrap().range.end;
    let start_x = first.x;
    let context = Rc::clone(&first.context);
    let line_result = text_system.shape_line(LineRequest {
        paragraph: Paragraph {
            text: &context.text,
            spans: &context.spans,
            sources: &context.sources,
            bidi_scopes: &context.bidi_scopes,
            base_direction: context.base_direction,
        },
        range: range.clone(),
    });
    let Ok(line) = line_result else {
        if requires_reshape {
            glyphs.clear();
        }
        placed.clear();
        return;
    };
    let discretionary_rtl = placed.last().filter(|item| select_discretionary && item.hyphen_after)
        .map(|item| {
            resolved_direction_from_runs(&line.runs, item.range.end, context.base_direction)
                == amazing_text::Direction::RightToLeft
        });
    let space_advance = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(move |glyph| (run, glyph)))
        .find(|(_, glyph)| context.text[glyph.cluster.clone()].starts_with(' '))
        .map(|(_, glyph)| glyph.advance_x.abs())
        .filter(|advance| *advance > 0.0);
    glyphs.clear();
    let mut rebuilt_decorations = Vec::new();
    let vertical = context
        .spans
        .first()
        .is_some_and(|span| span.style.writing_mode.is_vertical_flow());
    let mut pen = start_x;
    let mut previous_letter: Option<f32> = None;
    let mut active_source = None;
    let mut decoration_start = pen;
    let mut replaced_by_node = HashMap::<(u32, Option<u64>), usize>::new();
    for (index, fragment) in replaced.iter().enumerate() {
        replaced_by_node.insert((fragment.node.0, fragment.source_order), index);
    }
    for run in line.runs {
        if active_source != Some(run.source) {
            finish_visual_decoration(
                active_source,
                decoration_start,
                &mut pen,
                &context.paint_info,
                baseline,
                strut,
                &mut rebuilt_decorations,
            );
            active_source = Some(run.source);
            decoration_start = pen;
            if let Some(info) = context.paint_info.get(&run.source) {
                pen += info.padding.left;
            }
        }
        for glyph in run.glyphs {
            let character = context.text[glyph.cluster.clone()].chars().next();
            if character == Some('\u{fffc}') {
                match context.atomics.get(&glyph.cluster.start) {
                    Some(AtomicInline::Replaced(node, size, source_order)) => {
                        if let Some(index) = replaced_by_node.get(&(node.0, *source_order))
                            && let Some(fragment) = replaced.get_mut(*index)
                        {
                            fragment.x = if vertical { start_x } else { pen };
                            pen += if vertical { size.height } else { size.width };
                        }
                    },
                    Some(AtomicInline::Combined(combined)) => {
                        for atomic_glyph in &combined.glyphs {
                            let mut atomic_glyph = atomic_glyph.clone();
                            if vertical {
                                atomic_glyph.x += start_x;
                                atomic_glyph.y += pen;
                            } else {
                                atomic_glyph.x += pen;
                            }
                            glyphs.push(atomic_glyph);
                        }
                        pen += combined.width;
                    },
                    Some(AtomicInline::Ruby(ruby)) => {
                        // Ruby glyphs are retained in the separate ruby layer;
                        // reshaping the surrounding text only advances past it.
                        pen += ruby.width;
                    },
                    None => {},
                }
                continue;
            }
            if character == Some('\t') {
                let unit = space_advance.unwrap_or(run.font_size * 0.6).max(0.01);
                let stop = unit * 8.0;
                let relative = (pen - start_x).max(0.0);
                pen = start_x + (relative / stop).floor().mul_add(stop, stop);
                continue;
            }
            if character.is_some_and(is_invisible_text_control) {
                continue;
            }
            let (letter, word_spacing, font_size) = spacing_at(&context.spacing, glyph.cluster.start);
            if let Some(previous) = previous_letter {
                pen += (previous + letter) * 0.5;
            }
            glyphs.push(GlyphFrag {
                node: NodeIdx(run.source),
                font: run.font,
                glyph: glyph.id,
                cluster: glyph.cluster.clone(),
                source_range: source_range_for_cluster(
                    &context.sources,
                    run.source,
                    glyph.cluster.clone(),
                ),
                source_mappings: source_mappings_for_cluster(
                    &context.sources,
                    glyph.cluster.clone(),
                ),
                x: if vertical {
                    start_x + glyph.offset_x
                } else {
                    pen + glyph.offset_x
                },
                y: if vertical { pen + glyph.offset_y } else { 0.0 },
                advance_x: glyph.advance_x,
                advance_y: glyph.advance_y,
                offset_x: glyph.offset_x,
                offset_y: glyph.offset_y,
                font_size: run.font_size,
                writing_mode: run.writing_mode,
                orientation: glyph.orientation,
                scale_x: 1.0,
                scale_y: 1.0,
                variations: run.variations.clone(),
                synthesis: run.synthesis,
                color: context
                    .colors
                    .get(&run.source)
                    .cloned()
                    .unwrap_or_else(|| "black".into()),
            });
            let advance = inline_advance(run.writing_mode, glyph.advance_x, glyph.advance_y);
            pen += advance;
            if matches!(character, Some(' ' | '\u{00a0}')) {
                pen += word_spacing.resolve(Some(font_size)).unwrap_or(0.0);
            }
            previous_letter = Some(letter);
        }
    }
    finish_visual_decoration(
        active_source,
        decoration_start,
        &mut pen,
        &context.paint_info,
        baseline,
        strut,
        &mut rebuilt_decorations,
    );
    *decorations = rebuilt_decorations;
    if let Some(rtl) = discretionary_rtl {
        append_discretionary_hyphen(
            &context,
            glyphs,
            decorations,
            replaced,
            DiscretionaryPlacement {
                line_start: start_x,
                pen,
                break_offset: placed.last().unwrap().range.end,
                rtl,
            },
            text_system,
        );
    }
    placed.clear();
}

fn resolved_direction_at_break(
    context: &ShapedContext,
    range: Range<usize>,
    break_offset: usize,
    text_system: &dyn TextSystem,
) -> amazing_text::Direction {
    text_system
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text: &context.text,
                spans: &context.spans,
                sources: &context.sources,
                bidi_scopes: &context.bidi_scopes,
                base_direction: context.base_direction,
            },
            range,
        })
        .ok()
        .map(|line| {
            resolved_direction_from_runs(&line.runs, break_offset, context.base_direction)
        })
        .unwrap_or(context.base_direction)
}

fn resolved_direction_from_runs(
    runs: &[amazing_text::ShapedRun],
    break_offset: usize,
    fallback: amazing_text::Direction,
) -> amazing_text::Direction {
    runs.iter()
        .find(|run| run.text_range.start < break_offset && break_offset <= run.text_range.end)
        .or_else(|| {
            runs.iter()
                .filter(|run| run.text_range.end <= break_offset)
                .max_by_key(|run| run.text_range.end)
        })
        .map_or(fallback, |run| run.direction)
}

fn finish_visual_decoration(
    source: Option<u32>,
    start: f32,
    pen: &mut f32,
    paint_info: &HashMap<u32, InlinePaintInfo>,
    baseline: f32,
    strut: &Strut,
    decorations: &mut Vec<crate::InlineDecorationFrag>,
) {
    let Some(source) = source else { return };
    let Some(info) = paint_info.get(&source) else { return };
    *pen += info.padding.right;
    if !info.background_color.eq_ignore_ascii_case("transparent") {
        decorations.push(crate::InlineDecorationFrag {
            node: NodeIdx(source),
            x: start,
            y: baseline - (strut.ascent + info.padding.top),
            width: *pen - start,
            height: strut.ascent + strut.descent + info.padding.top + info.padding.bottom,
            color: info.background_color.clone(),
        });
    }
}

struct DiscretionaryPlacement {
    line_start: f32,
    pen: f32,
    break_offset: usize,
    rtl: bool,
}

fn append_discretionary_hyphen(
    context: &ShapedContext,
    glyphs: &mut Vec<GlyphFrag>,
    decorations: &mut [crate::InlineDecorationFrag],
    replaced: &mut [ReplacedFrag],
    placement: DiscretionaryPlacement,
    text_system: &dyn TextSystem,
) {
    let Some((source, cluster, source_range, line)) =
        shape_discretionary_hyphen(context, placement.break_offset, text_system)
    else {
        return;
    };
    let width = line
        .runs
        .iter()
        .flat_map(|run| &run.glyphs)
        .map(|glyph| glyph.advance_x.abs())
        .sum::<f32>();
    if placement.rtl {
        for glyph in glyphs.iter_mut() {
            glyph.x += width;
        }
        for fragment in replaced {
            fragment.x += width;
        }
        for decoration in decorations {
            decoration.x += width;
        }
    }
    let mut inserted = Vec::new();
    let mut hyphen_pen = if placement.rtl {
        placement.line_start
    } else {
        placement.pen
    };
    for run in line.runs {
        for glyph in run.glyphs {
            inserted.push(GlyphFrag {
                node: NodeIdx(source),
                font: run.font,
                glyph: glyph.id,
                cluster: cluster.clone(),
                source_range: source_range.clone(),
                source_mappings: source_mappings_for_cluster(&context.sources, cluster.clone()),
                x: hyphen_pen + glyph.offset_x,
                y: 0.0,
                advance_x: glyph.advance_x,
                advance_y: glyph.advance_y,
                offset_x: glyph.offset_x,
                offset_y: glyph.offset_y,
                font_size: run.font_size,
                writing_mode: run.writing_mode,
                orientation: glyph.orientation,
                scale_x: 1.0,
                scale_y: 1.0,
                variations: run.variations.clone(),
                synthesis: run.synthesis,
                color: context
                    .colors
                    .get(&source)
                    .cloned()
                    .unwrap_or_else(|| "black".into()),
            });
            hyphen_pen += glyph.advance_x.abs();
        }
    }
    if placement.rtl {
        inserted.append(glyphs);
        *glyphs = inserted;
    } else {
        glyphs.extend(inserted);
    }
}

fn shape_discretionary_hyphen(
    context: &ShapedContext,
    break_offset: usize,
    text_system: &dyn TextSystem,
) -> Option<(u32, Range<usize>, Range<usize>, amazing_text::ShapedLine)> {
    let (character_start, _) = context.text[..break_offset].char_indices().next_back()?;
    let span = context
        .spans
        .iter()
        .find(|span| span.range.contains(&character_start))?;
    let cluster = character_start..break_offset;
    let source_range = source_range_for_cluster(&context.sources, span.source, cluster.clone());
    let spans = [TextSpan {
        range: 0..1,
        source: span.source,
        style: span.style.clone(),
        boundary_before: ShapingBoundary::Hard,
    }];
    let line = text_system
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text: "-",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: context.base_direction,
            },
            range: 0..1,
        })
        .ok()?;
    Some((span.source, cluster, source_range, line))
}

fn discretionary_hyphen_advance(
    context: &ShapedContext,
    break_offset: usize,
    text_system: &dyn TextSystem,
) -> f32 {
    shape_discretionary_hyphen(context, break_offset, text_system)
        .map(|(_, _, _, line)| {
            line.runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.advance_x.abs())
                .sum()
        })
        .unwrap_or(0.0)
}

fn shape_inline_items(
    items: &[&InlineNode],
    base_direction: amazing_text::Direction,
    text_system: &dyn TextSystem,
) -> Option<Vec<ShapedUnit>> {
    shape_chunk(items, base_direction, text_system)
}

fn shape_combined_text(
    node: NodeIdx,
    text: &str,
    source_mappings: &[crate::TextSourceMapping],
    font_size: f32,
    style: &amazing_text::TextStyle,
    color: &str,
    text_system: &dyn TextSystem,
) -> Option<ShapedRuby> {
    let outer_writing_mode = style.writing_mode;
    let mut inner_style = style.clone();
    inner_style.writing_mode = amazing_text::WritingMode::HorizontalTb;
    inner_style.text_combine = amazing_text::TextCombine::None;
    let combined = crate::RubyText {
        node,
        text: text.to_owned(),
        source_mappings: source_mappings.to_vec(),
        font_size,
        style: Box::new(inner_style),
        color: color.to_owned(),
    };
    let (mut glyphs, natural_width, metrics) = shape_ruby_text(&combined, 0.0, text_system)?;
    let scale = if natural_width > 0.0 {
        (font_size / natural_width).min(1.0)
    } else {
        1.0
    };
    let fitted_width = natural_width * scale;
    let baseline = (font_size - metrics.line_height()) * 0.5 + metrics.ascent;
    for glyph in &mut glyphs {
        glyph.x = -fitted_width * 0.5 + glyph.x * scale;
        glyph.y = baseline;
        glyph.writing_mode = outer_writing_mode;
        glyph.orientation = amazing_text::GlyphOrientation::Upright;
        glyph.scale_x = scale;
    }
    Some(ShapedRuby {
        node,
        text: text.to_owned(),
        width: font_size,
        base_width: font_size,
        overhang_spaces: false,
        font_size,
        glyphs,
        line_over: 0.0,
        break_after: true,
        space_before: 0.0,
        preserved_space_before: 0.0,
    })
}

fn shape_ruby_units(
    node: NodeIdx,
    columns: &[crate::RubyColumn],
    overhang_spaces: bool,
    text_system: &dyn TextSystem,
) -> Option<Vec<ShapedRuby>> {
    let mut units = Vec::new();
    let mut starts = Vec::new();
    let mut start = 0usize;
    while start < columns.len() {
        starts.push(start);
        let span = columns[start].annotation_span.max(1);
        let end = start.checked_add(span)?.min(columns.len());
        units.push(shape_ruby(node, &columns[start..end], overhang_spaces, text_system)?);
        start = end;
    }
    for index in 0..units.len().saturating_sub(1) {
        let left_group_end = starts[index + 1];
        let left = &columns[left_group_end - 1].base;
        let right = &columns[left_group_end].base;
        units[index].break_after = ruby_base_break_allowed(left, right, text_system);
    }
    Some(units)
}

fn ruby_base_break_allowed(
    left: &crate::RubyText,
    right: &crate::RubyText,
    text_system: &dyn TextSystem,
) -> bool {
    let mut text = left.text.clone();
    let boundary = text.len();
    text.push_str(&right.text);
    if boundary == 0 || boundary == text.len() {
        return false;
    }
    let spans = [
        TextSpan {
            range: 0..boundary,
            source: left.node.0,
            style: left.style.as_ref().clone(),
            boundary_before: ShapingBoundary::Hard,
        },
        TextSpan {
            range: boundary..text.len(),
            source: right.node.0,
            style: right.style.as_ref().clone(),
            boundary_before: ShapingBoundary::Continue,
        },
    ];
    let provider_break = text_system
        .shape(Paragraph {
            text: &text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: left.style.direction,
        })
        .ok()
        .is_some_and(|paragraph| {
            paragraph.breaks.iter().any(|opportunity| {
                opportunity.offset == boundary
                    && opportunity.kind != amazing_text::BreakKind::Mandatory
            })
        });
    provider_break
        || (left.style.word_break != amazing_text::WordBreak::KeepAll
            && right.style.word_break != amazing_text::WordBreak::KeepAll
            && left.text.chars().next_back().is_some_and(is_ideographic)
            && right.text.chars().next().is_some_and(is_ideographic))
}

fn is_ideographic(character: char) -> bool {
    matches!(
        character as u32,
        0x2e80..=0x2fff
            | 0x3040..=0x30ff
            | 0x31f0..=0x31ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xf900..=0xfaff
            | 0x20000..=0x3ffff
    )
}

fn shape_ruby(
    node: NodeIdx,
    columns: &[crate::RubyColumn],
    overhang_spaces: bool,
    text_system: &dyn TextSystem,
) -> Option<ShapedRuby> {
    if columns.len() > MAX_RUBY_COLUMNS {
        return None;
    }
    let text_bytes = columns.iter().try_fold(0usize, |total, column| {
        let total = total.checked_add(column.base.text.len())?;
        total.checked_add(
            column
                .annotation
                .as_ref()
                .map_or(0, |annotation| annotation.text.len()),
        )
    })?;
    if text_bytes > MAX_RUBY_TEXT_BYTES {
        return None;
    }
    let font_size = columns.first()?.base.font_size;
    let vertical_flow = columns[0].base.style.writing_mode.is_vertical_flow();
    let mut bases = columns
        .iter()
        .map(|column| shape_ruby_text(&column.base, 0.0, text_system))
        .collect::<Option<Vec<_>>>()?;
    let base_width = bases.iter().map(|(_, width, _)| *width).sum();
    let mut widths = bases.iter().map(|(_, width, _)| *width).collect::<Vec<_>>();
    let mut annotations = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        let shaped = if column.annotation_hidden {
            None
        } else if let Some(annotation) = &column.annotation {
            let metrics = text_system.metrics(&annotation.style).ok()?;
            let base_metrics = bases[index].2;
            let block_offset = if vertical_flow {
                match column.base.style.writing_mode {
                    amazing_text::WritingMode::VerticalRl
                    | amazing_text::WritingMode::SidewaysRl => {
                        base_metrics.vertical_ascent + metrics.vertical_descent
                    },
                    amazing_text::WritingMode::VerticalLr
                    | amazing_text::WritingMode::SidewaysLr => {
                        -(base_metrics.vertical_descent + metrics.vertical_ascent)
                    },
                    amazing_text::WritingMode::HorizontalTb => 0.0,
                }
            } else {
                -(base_metrics.ascent + metrics.descent)
            };
            Some(shape_ruby_text(annotation, block_offset, text_system)?)
        } else {
            None
        };
        if let Some((_, annotation_width, _)) = &shaped {
            let end = (index + column.annotation_span).min(columns.len());
            let spanned_width = widths[index..end].iter().sum::<f32>();
            if *annotation_width > spanned_width {
                let extra = (*annotation_width - spanned_width) / (end - index) as f32;
                for width in &mut widths[index..end] {
                    *width += extra;
                }
            }
        }
        annotations.push(shaped);
    }
    let mut positions = Vec::with_capacity(widths.len() + 1);
    positions.push(0.0);
    for width in &widths {
        positions.push(positions.last().copied().unwrap_or(0.0) + width);
    }
    let line_over = if vertical_flow {
        0.0
    } else {
        annotations
            .iter()
            .filter_map(|annotation| annotation.as_ref().map(|(_, _, metrics)| metrics.line_height()))
            .fold(0.0, f32::max)
    };
    let mut glyphs = Vec::new();
    let mut text = String::new();
    for (index, column) in columns.iter().enumerate() {
        let (base_glyphs, base_width, _) = &mut bases[index];
        let base_offset = positions[index] + (widths[index] - *base_width) * 0.5;
        for glyph in base_glyphs.iter_mut() {
            if vertical_flow {
                glyph.y += base_offset;
            } else {
                glyph.x += base_offset;
            }
        }
        text.push_str(&column.base.text);
        glyphs.append(base_glyphs);
        if let Some((mut annotation_glyphs, annotation_width, _)) = annotations[index].take() {
            let end = (index + column.annotation_span).min(columns.len());
            let span_width = positions[end] - positions[index];
            let annotation_offset =
                positions[index] + (span_width - annotation_width) * 0.5;
            for glyph in &mut annotation_glyphs {
                if vertical_flow {
                    glyph.y += annotation_offset;
                } else {
                    glyph.x += annotation_offset;
                }
            }
            glyphs.extend(annotation_glyphs);
        }
    }
    Some(ShapedRuby {
        node,
        text,
        width: positions.last().copied().unwrap_or(0.0),
        base_width,
        overhang_spaces,
        font_size,
        glyphs,
        line_over,
        break_after: true,
        space_before: 0.0,
        preserved_space_before: 0.0,
    })
}

fn shape_ruby_text(
    text: &crate::RubyText,
    block_offset: f32,
    text_system: &dyn TextSystem,
) -> Option<(Vec<GlyphFrag>, f32, amazing_text::FontMetrics)> {
    let metrics = text_system.metrics(&text.style).ok()?;
    if text.text.is_empty() {
        return Some((Vec::new(), 0.0, metrics));
    }
    let spans = [TextSpan {
        range: 0..text.text.len(),
        source: text.node.0,
        style: text.style.as_ref().clone(),
        boundary_before: ShapingBoundary::Hard,
    }];
    let sources = text
        .source_mappings
        .iter()
        .map(|mapping| SourceMapping {
            text_range: mapping.text_range.clone(),
            source: mapping.source.0,
            source_range: mapping.source_range.clone(),
        })
        .collect::<Vec<_>>();
    let line = text_system
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text: &text.text,
                spans: &spans,
                sources: &sources,
                bidi_scopes: &[],
                base_direction: text.style.direction,
            },
            range: 0..text.text.len(),
        })
        .ok()?;
    let mut fragments = Vec::new();
    let mut pen = 0.0;
    for run in line.runs {
        for glyph in run.glyphs {
            fragments.push(GlyphFrag {
                node: text.node,
                font: run.font,
                glyph: glyph.id,
                cluster: glyph.cluster.clone(),
                source_range: source_range_for_cluster(
                    &sources,
                    run.source,
                    glyph.cluster.clone(),
                ),
                source_mappings: source_mappings_for_cluster(
                    &sources,
                    glyph.cluster.clone(),
                ),
                x: if run.writing_mode.is_vertical_flow() {
                    block_offset + glyph.offset_x
                } else {
                    pen + glyph.offset_x
                },
                y: if run.writing_mode.is_vertical_flow() {
                    pen + glyph.offset_y
                } else {
                    block_offset + glyph.offset_y
                },
                advance_x: glyph.advance_x,
                advance_y: glyph.advance_y,
                offset_x: glyph.offset_x,
                offset_y: glyph.offset_y,
                font_size: run.font_size,
                writing_mode: run.writing_mode,
                orientation: glyph.orientation,
                scale_x: 1.0,
                scale_y: 1.0,
                variations: run.variations.clone(),
                synthesis: run.synthesis,
                color: text.color.clone(),
            });
            pen += inline_advance(run.writing_mode, glyph.advance_x, glyph.advance_y);
        }
    }
    Some((fragments, pen, metrics))
}

fn shape_chunk(
    items: &[&InlineNode],
    base_direction: amazing_text::Direction,
    text_system: &dyn TextSystem,
) -> Option<Vec<ShapedUnit>> {
    let mut paragraph_text = String::new();
    let mut spans = Vec::new();
    let mut spacing = Vec::new();
    let mut sources = Vec::new();
    let mut bidi_scopes = Vec::new();
    let mut bidi_scope_stack = Vec::new();
    let mut atomics = HashMap::new();
    let mut colors = HashMap::new();
    let mut paint_info = HashMap::new();
    let mut previous_after = ShapingBoundary::Continue;
    let mut following_styles = vec![None; items.len()];
    let mut following_style = None;
    for (index, item) in items.iter().enumerate().rev() {
        following_styles[index] = following_style.clone();
        if let InlineNode::Text { node, style, .. } = item {
            following_style = Some((node.0, style.as_ref().clone()));
        }
    }
    let default_style = amazing_text::TextStyle {
        direction: base_direction,
        ..amazing_text::TextStyle::default()
    };
    let mut previous_style = None;
    for (item_index, item) in items.iter().enumerate() {
        match item {
            InlineNode::BidiStart {
                direction,
                unicode_bidi,
            } => {
                bidi_scope_stack.push((paragraph_text.len(), *direction, *unicode_bidi));
                continue;
            }
            InlineNode::BidiEnd => {
                let (scope_start, direction, unicode_bidi) = bidi_scope_stack.pop()?;
                if scope_start < paragraph_text.len() {
                    bidi_scopes.push(amazing_text::BidiScope {
                        range: scope_start..paragraph_text.len(),
                        direction,
                        unicode_bidi,
                    });
                }
                continue;
            }
            InlineNode::Break => {
                let (source, style) = previous_style
                    .clone()
                    .or_else(|| following_styles[item_index].clone())
                    .unwrap_or_else(|| (0, default_style.clone()));
                let start = paragraph_text.len();
                paragraph_text.push('\n');
                spans.push(TextSpan {
                    range: start..paragraph_text.len(),
                    source,
                    style,
                    boundary_before: ShapingBoundary::Hard,
                });
                previous_after = ShapingBoundary::Hard;
                continue;
            }
            InlineNode::Replaced { node, size, .. } | InlineNode::Atomic { node, size, .. } => {
                let (_, style) = previous_style
                    .clone()
                    .or_else(|| following_styles[item_index].clone())
                    .unwrap_or_else(|| (0, default_style.clone()));
                let start = paragraph_text.len();
                paragraph_text.push('\u{fffc}');
                let end = paragraph_text.len();
                spans.push(TextSpan {
                    range: start..end,
                    source: node.0,
                    style,
                    boundary_before: ShapingBoundary::Hard,
                });
                sources.push(SourceMapping {
                    text_range: start..end,
                    source: node.0,
                    source_range: 0..0,
                });
                let source_order = match item {
                    InlineNode::Replaced { replaced, .. } => replaced.source_order,
                    _ => None,
                };
                atomics.insert(start, AtomicInline::Replaced(*node, *size, source_order));
                previous_after = ShapingBoundary::Hard;
                continue;
            }
            InlineNode::Text {
                node,
                text,
                source_mappings,
                font_size,
                style,
                color,
                ..
            } if style.text_combine == amazing_text::TextCombine::All
                && style.writing_mode.is_vertical_flow() =>
            {
                let collapse = matches!(style.white_space,
                    amazing_text::WhiteSpace::Normal
                        | amazing_text::WhiteSpace::NoWrap
                        | amazing_text::WhiteSpace::PreLine);
                let (combined_text, combined_mappings) =
                    trim_combined_edge_spaces(text, source_mappings, collapse);
                let start = paragraph_text.len();
                paragraph_text.push('\u{fffc}');
                let end = paragraph_text.len();
                spans.push(TextSpan {
                    range: start..end,
                    source: node.0,
                    style: style.as_ref().clone(),
                    boundary_before: ShapingBoundary::Hard,
                });
                sources.extend(combined_mappings.iter().map(|mapping| SourceMapping {
                    text_range: start..end,
                    source: node.0,
                    source_range: mapping.source_range.clone(),
                }));
                atomics.insert(
                    start,
                    AtomicInline::Combined(shape_combined_text(
                        *node,
                        combined_text,
                        &combined_mappings,
                        *font_size,
                        style,
                        color,
                        text_system,
                    )?),
                );
                previous_style = Some((node.0, style.as_ref().clone()));
                previous_after = ShapingBoundary::Hard;
                continue;
            }
            InlineNode::Text { .. } => {}
            InlineNode::Ruby { node, columns, overhang_spaces } => {
                let units = shape_ruby_units(*node, columns, *overhang_spaces, text_system)?;
                if units.is_empty() {
                    continue;
                }
                let style = columns.first()?.base.style.as_ref().clone();
                for ruby in units {
                    let start = paragraph_text.len();
                    paragraph_text.push('\u{fffc}');
                    let end = paragraph_text.len();
                    spans.push(TextSpan {
                        range: start..end,
                        source: node.0,
                        style: style.clone(),
                        boundary_before: ShapingBoundary::Hard,
                    });
                    atomics.insert(start, AtomicInline::Ruby(ruby));
                }
                previous_style = Some((node.0, style));
                previous_after = ShapingBoundary::Hard;
                continue;
            }
        }
        let InlineNode::Text {
            node,
            text,
            source_mappings,
            font_size,
            style,
            letter_spacing,
            word_spacing,
            color,
            background_color,
            inline_padding,
            boundary_before,
            boundary_after,
            ..
        } = item
        else {
            previous_after = ShapingBoundary::Hard;
            continue;
        };
        previous_style = Some((node.0, style.as_ref().clone()));
        if text.is_empty() {
            continue;
        }
        let collapses_spaces = matches!(
            style.white_space,
            amazing_text::WhiteSpace::Normal
                | amazing_text::WhiteSpace::NoWrap
                | amazing_text::WhiteSpace::PreLine
        );
        let skip =
            usize::from(collapses_spaces && paragraph_text.ends_with(' ') && text.starts_with(' '));
        let start = paragraph_text.len();
        if skip != 0 {
            sources.extend(
                source_mappings
                    .iter()
                    .filter(|mapping| mapping.text_range.start < skip)
                    .map(|mapping| SourceMapping {
                        text_range: start - 1..start,
                        source: mapping.source.0,
                        source_range: mapping.source_range.clone(),
                    }),
            );
        }
        paragraph_text.push_str(&text[skip..]);
        let end = paragraph_text.len();
        sources.extend(
            source_mappings
                .iter()
                .filter(|mapping| mapping.text_range.end > skip)
                .map(|mapping| SourceMapping {
                    text_range: start + mapping.text_range.start.saturating_sub(skip)
                        ..start + mapping.text_range.end - skip,
                    source: mapping.source.0,
                    source_range: mapping.source_range.clone(),
                }),
        );
        if start == end {
            previous_after = stronger_boundary(previous_after, *boundary_after);
            continue;
        }
        spans.push(TextSpan {
            range: start..end,
            source: node.0,
            style: style.as_ref().clone(),
            boundary_before: stronger_boundary(previous_after, *boundary_before),
        });
        spacing.push(SpacingSpan { start, end, letter: *letter_spacing,
            word: *word_spacing, font_size: *font_size });
        colors.insert(node.0, color.clone());
        paint_info.insert(
            node.0,
            InlinePaintInfo {
                background_color: background_color.clone(),
                padding: *inline_padding,
            },
        );
        previous_after = *boundary_after;
    }
    if paragraph_text.is_empty() || spans.is_empty() {
        return Some(Vec::new());
    }
    if !bidi_scope_stack.is_empty() {
        return None;
    }
    let shaped = text_system
        .shape(Paragraph {
        text: &paragraph_text,
        spans: &spans,
        sources: &sources,
        bidi_scopes: &bidi_scopes,
        base_direction,
        })
        .ok()?;
    let visual_line = text_system
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text: &paragraph_text,
                spans: &spans,
                sources: &sources,
                bidi_scopes: &bidi_scopes,
                base_direction,
            },
            range: 0..paragraph_text.len(),
        })
        .ok();
    let context = Rc::new(ShapedContext {
        text: paragraph_text.clone(),
        spans: spans.clone(),
        spacing: spacing.clone(),
        sources: sources.clone(),
        bidi_scopes: bidi_scopes.clone(),
        colors: colors.clone(),
        paint_info: paint_info.clone(),
        atomics: atomics.clone(),
        base_direction,
    });
    let mut units = Vec::new();
    let mut previous_end = 0usize;
    for text_unit in text_units(&paragraph_text, &spans, &shaped.breaks, &atomics) {
        let (range, break_kind) = match text_unit {
            TextUnit::Word { range, break_kind } => (range, break_kind),
            TextUnit::Atomic { offset, atomic, break_after } => {
                let end = offset + '\u{fffc}'.len_utf8();
                let space_before = advance_in_range(&shaped, previous_end..offset)
                    + separator_spacing(&paragraph_text, previous_end..offset, &spacing);
                let preserved_start = spans.iter().find(|span| {
                    span.range.end > previous_end && span.range.start < offset
                        && matches!(span.style.white_space,
                            amazing_text::WhiteSpace::Pre | amazing_text::WhiteSpace::PreWrap
                                | amazing_text::WhiteSpace::BreakSpaces)
                }).map(|span| span.range.start.max(previous_end)).unwrap_or(offset);
                let preserved_space_before = advance_in_range(&shaped, preserved_start..offset);
                match atomic {
                    AtomicInline::Replaced(node, size, source_order) => units.push(ShapedUnit::Replaced {
                        node,
                        size,
                        source_order,
                        break_after,
                        space_before,
                        preserved_space_before,
                        range: offset..end,
                        context: Rc::clone(&context),
                    }),
                    AtomicInline::Combined(combined) => units.push(ShapedUnit::Combined {
                        shaped: combined,
                        break_after,
                        range: offset..end,
                        context: Rc::clone(&context),
                    }),
                    AtomicInline::Ruby(mut ruby) => {
                        ruby.space_before = advance_in_range(&shaped, previous_end..offset)
                            + separator_spacing(&paragraph_text, previous_end..offset, &spacing);
                        ruby.preserved_space_before = 0.0;
                        // Adjacent ruby columns use the base-text break rule.
                        // A paragraph shaper may omit a break between two
                        // U+FFFC placeholders even when the bases are Han.
                        // At the ruby boundary, use the paragraph break.
                        ruby.break_after = if matches!(atomics.get(&end), Some(AtomicInline::Ruby(next)) if next.node == ruby.node) {
                            ruby.break_after
                        } else {
                            break_after
                        };
                        units.push(ShapedUnit::Ruby(ruby));
                    },
                }
                previous_end = end;
                continue;
            }
            TextUnit::Break(offset) => {
                units.push(ShapedUnit::Break);
                previous_end = offset;
                continue;
            }
        };
        let mut glyphs = Vec::new();
        let mut decorations = Vec::new();
        let mut pen = 0.0;
        let mut node = spans[0].source;
        let mut font_size = spans[0].style.size;
        let mut active_source = None;
        let mut decoration_start = 0.0;
        let mut previous_letter = None;
        // Whitespace belongs to the span that supplied it, not the next word.
        // Drop a collapsible prefix at line start, retaining preformatted gaps.
        let preserved_start = spans
            .iter()
            .find(|span| {
                span.range.end > previous_end
                    && span.range.start < range.start
                    && matches!(
                        span.style.white_space,
                        amazing_text::WhiteSpace::Pre
                            | amazing_text::WhiteSpace::PreWrap
                            | amazing_text::WhiteSpace::BreakSpaces
                    )
            })
            .map(|span| span.range.start.max(previous_end))
            .unwrap_or(range.start);
        let preserved_space_before = advance_in_range(&shaped, preserved_start..range.start);
        let local_line;
        let visual_runs = if let Some(visual_line) = &visual_line {
            &visual_line.runs
        } else {
            local_line = text_system
                .shape_line(LineRequest {
                    paragraph: Paragraph {
                        text: &paragraph_text,
                        spans: &spans,
                        sources: &sources,
                        bidi_scopes: &bidi_scopes,
                        base_direction,
                    },
                    range: range.clone(),
                })
                .ok()?;
            &local_line.runs
        };
        for run in visual_runs {
            for glyph in &run.glyphs {
                if !range.contains(&glyph.cluster.start) {
                    continue;
                }
                if glyphs.is_empty() {
                    node = run.source;
                    font_size = run.font_size;
                }
                if active_source != Some(run.source) {
                    finish_source_decoration(
                        active_source,
                        decoration_start,
                        &mut pen,
                        &paint_info,
                        &mut decorations,
                    );
                    active_source = Some(run.source);
                    decoration_start = pen;
                    if let Some(info) = paint_info.get(&run.source) {
                        pen += info.padding.left;
                    }
                }
                let (letter, word_spacing, font_size) = spacing_at(&spacing, glyph.cluster.start);
                if let Some(previous) = previous_letter {
                    pen += (previous + letter) * 0.5;
                }
                glyphs.push(GlyphFrag {
                        node: NodeIdx(run.source),
                        font: run.font,
                        glyph: glyph.id,
                        cluster: glyph.cluster.clone(),
                    source_range: source_range_for_cluster(
                        &shaped.sources,
                        run.source,
                        glyph.cluster.clone(),
                    ),
                    source_mappings: source_mappings_for_cluster(
                        &shaped.sources,
                        glyph.cluster.clone(),
                    ),
                    x: if !run.writing_mode.is_vertical_flow() {
                        pen + glyph.offset_x
                    } else {
                        glyph.offset_x
                    },
                    y: if !run.writing_mode.is_vertical_flow() {
                        0.0
                    } else {
                        pen
                    },
                        advance_x: glyph.advance_x,
                        advance_y: glyph.advance_y,
                        offset_x: glyph.offset_x,
                        offset_y: glyph.offset_y,
                        font_size: run.font_size,
                        writing_mode: run.writing_mode,
                        orientation: glyph.orientation,
                        scale_x: 1.0,
                        scale_y: 1.0,
                        variations: run.variations.clone(),
                        synthesis: run.synthesis,
                        color: colors
                            .get(&run.source)
                            .cloned()
                            .unwrap_or_else(|| "black".into()),
                });
                let advance = inline_advance(run.writing_mode, glyph.advance_x, glyph.advance_y);
                pen += advance;
                if matches!(paragraph_text[glyph.cluster.clone()].chars().next(), Some(' ' | '\u{00a0}')) {
                    pen += word_spacing.resolve(Some(font_size)).unwrap_or(0.0);
                }
                previous_letter = Some(letter);
            }
        }
        finish_source_decoration(
            active_source,
            decoration_start,
            &mut pen,
            &paint_info,
            &mut decorations,
        );
        let hyphen_after = break_kind == Some(amazing_text::BreakKind::Hyphen);
        let hyphen_advance = if hyphen_after {
            discretionary_hyphen_advance(&context, range.end, text_system)
        } else {
            0.0
        };
        units.push(ShapedUnit::Word(ShapedWord {
            node: NodeIdx(node),
            text: paragraph_text[range.clone()].to_owned(),
            width: pen,
            font_size,
            space_before: advance_in_range(&shaped, previous_end..range.start)
                + separator_spacing(&paragraph_text, previous_end..range.start, &spacing),
            glyphs,
            decorations,
            range: preserved_start..range.end,
            context: Rc::clone(&context),
            preserved_space_before,
            break_after: break_kind.is_some(),
            hyphen_after,
            hyphen_advance,
            requires_reshape: shaped.unsafe_breaks.binary_search(&range.end).is_ok(),
        }));
        previous_end = range.end;
    }
    apply_ruby_overhang(&mut units);
    Some(units)
}

fn apply_ruby_overhang(units: &mut [ShapedUnit]) {
    // The annotation can consume the blank advances bordering its container.
    // Base glyphs keep their original position within the ruby columns.
    for index in 0..units.len() {
        let right_space = match units.get(index + 1) {
            Some(ShapedUnit::Word(word)) => word.space_before,
            Some(ShapedUnit::Ruby(ruby)) => ruby.space_before,
            _ => 0.0,
        };
        let ShapedUnit::Ruby(ruby) = &mut units[index] else { continue };
        if !ruby.overhang_spaces || ruby.width <= ruby.base_width {
            continue;
        }
        let half_excess = (ruby.width - ruby.base_width) * 0.5;
        let left = half_excess.min(ruby.space_before).max(0.0);
        let right = half_excess.min(right_space).max(0.0);
        ruby.width -= left + right;
        for glyph in &mut ruby.glyphs {
            if glyph.writing_mode.is_vertical_flow() {
                glyph.y -= left;
            } else {
                glyph.x -= left;
            }
        }
    }
}

fn trim_combined_edge_spaces<'a>(text: &'a str,
    mappings: &[crate::TextSourceMapping], collapse: bool,
) -> (&'a str, Vec<crate::TextSourceMapping>) {
    if !collapse { return (text, mappings.to_vec()); }
    let start = text.len() - text.trim_start_matches(' ').len();
    let end = text.trim_end_matches(' ').len().max(start);
    let trimmed = &text[start..end];
    let mappings = mappings.iter().filter_map(|mapping| {
        let begin = mapping.text_range.start.max(start);
        let finish = mapping.text_range.end.min(end);
        (begin < finish).then(|| crate::TextSourceMapping {
            text_range: begin - start..finish - start,
            source: mapping.source,
            source_range: mapping.source_range.clone(),
        })
    }).collect();
    (trimmed, mappings)
}

fn finish_source_decoration(
    source: Option<u32>,
    start: f32,
    pen: &mut f32,
    paint_info: &HashMap<u32, InlinePaintInfo>,
    decorations: &mut Vec<ShapedDecoration>,
) {
    let Some(source) = source else {
        return;
    };
    let Some(info) = paint_info.get(&source) else {
        return;
    };
    *pen += info.padding.right;
    if !info.background_color.eq_ignore_ascii_case("transparent") {
        decorations.push(ShapedDecoration {
            node: NodeIdx(source),
            x: start,
            width: *pen - start,
            color: info.background_color.clone(),
            padding_top: info.padding.top,
            padding_bottom: info.padding.bottom,
        });
    }
}

fn stronger_boundary(left: ShapingBoundary, right: ShapingBoundary) -> ShapingBoundary {
    match (left, right) {
        (ShapingBoundary::Hard, _) | (_, ShapingBoundary::Hard) => ShapingBoundary::Hard,
        (ShapingBoundary::StyleChange, _) | (_, ShapingBoundary::StyleChange) => {
            ShapingBoundary::StyleChange
        },
        _ => ShapingBoundary::Continue,
    }
}

enum TextUnit {
    Word {
        range: Range<usize>,
        break_kind: Option<amazing_text::BreakKind>,
    },
    Atomic {
        offset: usize,
        atomic: AtomicInline,
        break_after: bool,
    },
    Break(usize),
}

fn text_units(
    text: &str,
    spans: &[TextSpan],
    breaks: &[amazing_text::BreakOpportunity],
    atomics: &HashMap<usize, AtomicInline>,
) -> Vec<TextUnit> {
    let mut units = Vec::new();
    let mut segment_start = 0usize;
    for opportunity in breaks {
        if opportunity.offset <= segment_start || opportunity.offset > text.len() {
            continue;
        }
        let segment = segment_start..opportunity.offset;
        if let Some(range) = visible_segment(text, segment.clone(), spans) {
            units.push(TextUnit::Word {
                range,
                break_kind: Some(opportunity.kind),
            });
        }
        let forced = opportunity.kind == amazing_text::BreakKind::Mandatory
            && mandatory_break_before(text, opportunity.offset);
        if forced {
            units.push(TextUnit::Break(opportunity.offset));
        }
        segment_start = opportunity.offset;
    }
    let segment = segment_start..text.len();
    if let Some(range) = visible_segment(text, segment.clone(), spans) {
        units.push(TextUnit::Word {
            range,
            break_kind: Some(amazing_text::BreakKind::Mandatory),
        });
    }
    split_atomic_units(units, atomics)
}

fn split_atomic_units(
    units: Vec<TextUnit>,
    atomics: &HashMap<usize, AtomicInline>,
) -> Vec<TextUnit> {
    let mut offsets = atomics.keys().copied().collect::<Vec<_>>();
    offsets.sort_unstable();
    let mut expanded = Vec::new();
    let mut offset_index = 0usize;
    for unit in units {
        let TextUnit::Word { range, break_kind } = unit else {
            expanded.push(unit);
            continue;
        };
        let mut cursor = range.start;
        while offsets
            .get(offset_index)
            .is_some_and(|offset| *offset < range.start)
        {
            offset_index += 1;
        }
        while let Some(offset) = offsets
            .get(offset_index)
            .copied()
            .filter(|offset| *offset < range.end)
        {
            if cursor < offset {
                expanded.push(TextUnit::Word {
                    range: cursor..offset,
                    break_kind: None,
                });
            }
            let end = offset + '\u{fffc}'.len_utf8();
            expanded.push(TextUnit::Atomic {
                offset,
                atomic: atomics[&offset].clone(),
                break_after: end == range.end,
            });
            cursor = end;
            offset_index += 1;
        }
        if cursor < range.end {
            expanded.push(TextUnit::Word {
                range: cursor..range.end,
                break_kind,
            });
        }
    }
    expanded
}

fn visible_segment(text: &str, range: Range<usize>, spans: &[TextSpan]) -> Option<Range<usize>> {
    let segment = &text[range.clone()];
    let start = segment
        .char_indices()
        .find(|(offset, ch)| {
            !is_css_segment_whitespace(*ch)
                || preserves_space_at(spans, range.start + *offset)
        })
        .map(|(offset, _)| range.start + offset)?;
    let end = segment
        .char_indices()
        .rev()
        .find(|(offset, ch)| {
            !is_mandatory_break_character(*ch)
                && (!is_css_segment_whitespace(*ch)
                    || preserves_space_at(spans, range.start + *offset))
        })
        .map(|(offset, ch)| range.start + offset + ch.len_utf8())?;
    Some(start..end)
}

fn preserves_space_at(spans: &[TextSpan], offset: usize) -> bool {
    let index = spans.partition_point(|span| span.range.end <= offset);
    spans.get(index).is_some_and(|span| {
        span.range.contains(&offset)
            && matches!(
                span.style.white_space,
                amazing_text::WhiteSpace::Pre
                    | amazing_text::WhiteSpace::PreWrap
                    | amazing_text::WhiteSpace::BreakSpaces
            )
    })
}

fn is_mandatory_break_character(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

fn is_css_segment_whitespace(ch: char) -> bool {
    matches!(ch, '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | ' ')
}

fn mandatory_break_before(text: &str, offset: usize) -> bool {
    text[..offset].chars().next_back().is_some_and(|ch| {
        matches!(
            ch,
            '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
        )
    })
}

fn is_invisible_text_control(ch: char) -> bool {
    matches!(
        ch as u32,
        0x00AD
            | 0x000A..=0x000D
            | 0x061C
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0xFEFF
    )
}

fn advance_in_range(shaped: &amazing_text::ShapedParagraph, range: Range<usize>) -> f32 {
    shaped
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(move |glyph| (run.writing_mode, glyph)))
        .filter(|(_, glyph)| range.contains(&glyph.cluster.start))
        .map(|(writing_mode, glyph)| {
            inline_advance(writing_mode, glyph.advance_x, glyph.advance_y)
        })
        .sum()
}

fn inline_advance(
    writing_mode: amazing_text::WritingMode,
    advance_x: f32,
    advance_y: f32,
) -> f32 {
    if writing_mode.is_vertical_flow() && advance_y != 0.0 {
        advance_y.abs()
    } else {
        advance_x.abs()
    }
}

fn source_range_for_cluster(
    mappings: &[SourceMapping],
    source: u32,
    cluster: Range<usize>,
) -> Range<usize> {
    let Some(mapping) = mappings.iter().find(|mapping| {
        mapping.source == source
            && mapping.text_range.start < cluster.end
            && cluster.start < mapping.text_range.end
    }) else {
        return cluster;
    };
    if mapping.text_range.len() == mapping.source_range.len() {
        let start =
            mapping.source_range.start + cluster.start.saturating_sub(mapping.text_range.start);
        let end = mapping.source_range.start
            + cluster
                .end
                .min(mapping.text_range.end)
                .saturating_sub(mapping.text_range.start);
        start..end
    } else {
        mapping.source_range.clone()
    }
}

fn source_mappings_for_cluster(
    mappings: &[SourceMapping],
    cluster: Range<usize>,
) -> Vec<SourceMapping> {
    mappings
        .iter()
        .filter(|mapping| {
            mapping.text_range.start < cluster.end && cluster.start < mapping.text_range.end
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_an_atomic_preserves_only_provider_breaks() {
        let atomic = AtomicInline::Replaced(
            NodeIdx(1),
            IntrinsicSize { width: 8.0, height: 8.0 },
            None,
        );
        // The provider supplies only the final opportunity for 「 U+FFFC.
        // Splitting its word must not add a break between the opening bracket
        // and the atomic inline.
        let units = split_atomic_units(
            vec![TextUnit::Word {
                range: 0..6,
                break_kind: Some(amazing_text::BreakKind::Mandatory),
            }],
            &HashMap::from([(3, atomic.clone())]),
        );
        assert!(matches!(&units[0], TextUnit::Word { range, break_kind: None } if *range == (0..3)));
        assert!(matches!(&units[1], TextUnit::Atomic { break_after: true, .. }));

        let units = split_atomic_units(
            vec![TextUnit::Word {
                range: 0..5,
                break_kind: Some(amazing_text::BreakKind::Mandatory),
            }],
            &HashMap::from([(1, atomic)]),
        );
        assert!(matches!(&units[0], TextUnit::Word { break_kind: None, .. }));
        assert!(matches!(&units[1], TextUnit::Atomic { break_after: false, .. }));
        assert!(matches!(&units[2], TextUnit::Word { break_kind: Some(amazing_text::BreakKind::Mandatory), .. }));

        let units = split_atomic_units(
            vec![
                TextUnit::Word {
                    range: 0..1,
                    break_kind: Some(amazing_text::BreakKind::Allowed),
                },
                TextUnit::Word {
                    range: 1..4,
                    break_kind: Some(amazing_text::BreakKind::Mandatory),
                },
            ],
            &HashMap::from([(
                1,
                AtomicInline::Replaced(
                    NodeIdx(1),
                    IntrinsicSize { width: 8.0, height: 8.0 },
                    None,
                ),
            )]),
        );
        assert!(matches!(&units[0], TextUnit::Word { break_kind: Some(amazing_text::BreakKind::Allowed), .. }));
        assert!(matches!(&units[1], TextUnit::Atomic { break_after: true, .. }));
    }
    use crate::font::FontSource;

    fn text(t: &str) -> InlineNode {
        let len = t.len();
        InlineNode::Text {
            node: NodeIdx(99),
            source_order: None,
            text: t.into(),
            source_mappings: (!t.is_empty())
                .then_some(crate::TextSourceMapping {
                    text_range: 0..len,
                    source: NodeIdx(99),
                    source_range: 0..len,
                })
                .into_iter()
                .collect(),
            font_size: 16.0,
            style: Box::new(amazing_text::TextStyle::default()),
            letter_spacing: 0.0,
            word_spacing: crate::values::LengthPercentage::default(),
            color: "black".into(),
            background_color: "transparent".into(),
            inline_padding: crate::values::Sides::default(),
            boundary_before: amazing_text::ShapingBoundary::Continue,
            boundary_after: amazing_text::ShapingBoundary::Continue,
        }
    }

    #[test]
    fn shaped_letter_spacing_has_no_trailing_gap() {
        let mut item = text("abc");
        let InlineNode::Text { letter_spacing, .. } = &mut item else { unreachable!() };
        *letter_spacing = 3.0;
        let provider = amazing_text::FallbackTextSystem::new();
        let base = shape_chunk(&[&text("abc")], amazing_text::Direction::LeftToRight, &provider).unwrap();
        let spaced = shape_chunk(&[&item], amazing_text::Direction::LeftToRight, &provider).unwrap();
        let (ShapedUnit::Word(base), ShapedUnit::Word(spaced)) = (&base[0], &spaced[0]) else { panic!("want words") };
        assert!((spaced.width - base.width - 6.0).abs() < 0.01);
        assert!((spaced.glyphs[1].x - base.glyphs[1].x - 3.0).abs() < 0.01);
    }

    #[test]
    fn shaped_word_spacing_adds_to_the_surviving_separator() {
        let mut item = text("a b");
        let InlineNode::Text { word_spacing, .. } = &mut item else { unreachable!() };
        word_spacing.px = 5.0;
        let provider = amazing_text::FallbackTextSystem::new();
        let base = shape_chunk(&[&text("a b")], amazing_text::Direction::LeftToRight, &provider).unwrap();
        let spaced = shape_chunk(&[&item], amazing_text::Direction::LeftToRight, &provider).unwrap();
        let (ShapedUnit::Word(base), ShapedUnit::Word(spaced)) = (&base[1], &spaced[1]) else { panic!("want words") };
        assert!((spaced.space_before - base.space_before - 5.0).abs() < 0.01);
        let InlineNode::Text { word_spacing, .. } = &mut item else { unreachable!() };
        word_spacing.px = 0.0;
        word_spacing.percentage = 50.0;
        word_spacing.has_percentage = true;
        let percent = shape_chunk(&[&item], amazing_text::Direction::LeftToRight, &provider).unwrap();
        let ShapedUnit::Word(percent) = &percent[1] else { panic!("want word") };
        assert!((percent.space_before - base.space_before - 8.0).abs() < 0.01);
    }

    #[test]
    fn unbreakable_opening_bracket_and_atomic_wrap_together() {
        let mut before = text("A 「");
        let mut combined = text("12");
        for item in [&mut before, &mut combined] {
            let InlineNode::Text { style, .. } = item else { unreachable!() };
            style.writing_mode = amazing_text::WritingMode::VerticalRl;
        }
        let InlineNode::Text { style, .. } = &mut combined else { unreachable!() };
        style.text_combine = amazing_text::TextCombine::All;
        let provider = amazing_text::FallbackTextSystem::new();
        let lines = layout_lines_shaped(
            &[before, combined],
            InlineContainingBlock { x: 0.0, y: 0.0, width: 100.0, height: 32.0 },
            &strut(), amazing_text::Direction::LeftToRight,
            amazing_text::WritingMode::VerticalRl, &provider,
            &crate::floats::FloatContext::default(), true,
        ).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].words.iter().all(|word| !word.text.contains('「')));
        assert!(lines[1].words.iter().any(|word| word.text.contains('「')));
        assert!(lines[1].words.iter().any(|word| word.text == "12"));
    }

    #[test]
    fn combined_text_trims_collapsible_edges_but_keeps_middle_space() {
        let mut item = text("  5 6  ");
        let InlineNode::Text { style, .. } = &mut item else { unreachable!() };
        style.writing_mode = amazing_text::WritingMode::VerticalRl;
        style.text_combine = amazing_text::TextCombine::All;
        let provider = amazing_text::FallbackTextSystem::new();
        let units = shape_chunk(&[&item], amazing_text::Direction::LeftToRight, &provider).unwrap();
        let [ShapedUnit::Combined { shaped, .. }] = units.as_slice() else { panic!("want combined") };
        assert_eq!(shaped.text, "5 6");
    }

    #[test]
    fn atomic_only_vertical_line_uses_containing_block_writing_mode() {
        let item = InlineNode::Atomic {
            node: NodeIdx(1),
            size: IntrinsicSize { width: 20.0, height: 20.0 },
            baseline_offset: 20.0,
            vertical_align: "baseline".to_owned(),
        };
        let lines = layout_lines_shaped(
            &[item],
            InlineContainingBlock { x: 10.0, y: 0.0, width: 100.0, height: 100.0 },
            &strut(), amazing_text::Direction::LeftToRight,
            amazing_text::WritingMode::VerticalRl,
            &amazing_text::FallbackTextSystem::new(),
            &crate::floats::FloatContext::default(), true,
        ).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].replaced[0].x > 80.0);
    }

    fn strut() -> Strut {
        Strut {
            font_size: 16.0,
            line_height: 20.0,
            ascent: 14.0,
            descent: 4.0,
            vertical_ascent: 8.0,
            vertical_descent: 8.0,
        }
    }

    #[test]
    fn short_text_is_one_line_with_baseline() {
        let fonts = FontSource::fallback();
        let lines = layout_lines(&[text("hello world")], 10.0, 100.0, 500.0, &strut(), &fonts);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].baseline - (100.0 + 1.0 + 14.0)).abs() < 1e-4); // half-leading 1
        assert_eq!(lines[0].y_top, 100.0);
    }

    #[test]
    fn words_wrap_greedily_at_the_edge() {
        let fonts = FontSource::fallback();
        // Fallback measures ~0.55em/char → "aaaa" ≈ 35px wide at 16px.
        let lines = layout_lines(
            &[text("aaaa bbbb cccc dddd")],
            0.0,
            0.0,
            80.0,
            &strut(),
            &fonts,
        );
        assert_eq!(lines.len(), 2, "expected a wrap: {lines:#?}");
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[1].words.len(), 2);
        assert!((lines[1].y_top - 20.0).abs() < 1e-4);
    }

    #[test]
    fn br_forces_break_and_strips_leading_space() {
        let fonts = FontSource::fallback();
        let lines = layout_lines(
            &[text("a "), InlineNode::Break, text(" b")],
            0.0,
            0.0,
            500.0,
            &strut(),
            &fonts,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.len(), 1);
        assert_eq!(lines[1].words.len(), 1);
        // Second line's word starts flush at the content edge (no space).
        assert!((lines[1].words[0].x - 0.0).abs() < 1e-4);
    }

    #[test]
    fn whitespace_only_content_makes_no_lines() {
        let fonts = FontSource::fallback();
        let lines = layout_lines(&[text("   ")], 0.0, 0.0, 100.0, &strut(), &fonts);
        assert!(lines.is_empty());
    }

    #[test]
    fn space_between_items_survives_item_boundary() {
        let fonts = FontSource::fallback();
        // "one" ends without space, second item begins without space: the
        // tokenizer must still join them with the space that lived in the
        // FIRST item's tail ("one " + "two").
        let lines = layout_lines(
            &[text("one "), text("two")],
            0.0,
            0.0,
            500.0,
            &strut(),
            &fonts,
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 2);
        assert!(lines[0].words[1].x > lines[0].words[0].x + lines[0].words[0].width);
    }

    #[test]
    fn oversized_word_overflows_but_places() {
        let fonts = FontSource::fallback();
        let lines = layout_lines(
            &[text("aaaaaaaaaaaaaaaaaaaa")],
            0.0,
            0.0,
            30.0,
            &strut(),
            &fonts,
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].words[0].width > 30.0);
    }
}

/// Intrinsic contributions from the same shaped units used for line fitting.
/// This keeps table sizing on provider advances, including atomic content and
/// legal no-wrap runs, instead of estimating text using fallback glyph widths.
pub(crate) fn intrinsic_widths_shaped(
    items: &[InlineNode],
    direction: amazing_text::Direction,
    text: &dyn TextSystem,
) -> Option<(f32, f32)> {
    let refs: Vec<_> = items.iter().collect();
    let units = shape_inline_items(&refs, direction, text)?;
    let mut minimum = 0.0f32;
    let mut maximum = 0.0f32;
    let mut line = 0.0f32;
    let mut ruby_run = 0.0f32;
    for unit in units {
        match unit {
            ShapedUnit::Break => { maximum = maximum.max(line); line = 0.0; ruby_run = 0.0; }
            ShapedUnit::Word(word) => {
                minimum = minimum.max(word.width + word.hyphen_advance);
                line += word.width + if line > 0.0 {word.space_before} else {word.preserved_space_before};
                ruby_run = 0.0;
            }
            ShapedUnit::Replaced { size, space_before, preserved_space_before, .. } => {
                minimum = minimum.max(size.width);
                line += size.width + if line > 0.0 { space_before } else { preserved_space_before };
                ruby_run = 0.0;
            }
            ShapedUnit::Combined { shaped, .. } => {minimum = minimum.max(shaped.width); line += shaped.width; ruby_run = 0.0;}
            ShapedUnit::Ruby(ruby) => {
                ruby_run += ruby.width; minimum = minimum.max(ruby_run); line += ruby.width;
                if ruby.break_after {ruby_run = 0.0;}
            }
        }
    }
    Some((minimum, maximum.max(line)))
}
