//! Inline formatting context (CSS2.1 §9.4.2 + §10.8 line box math):
//! flattens [`InlineNode`] runs into words, wraps them greedily, and emits
//! absolutely-positioned [`LineBox`]es with baseline positions.
//!
//! Simplifications (documented in lib.rs): one strut per block (the block's
//! font-size/line-height decide every line's height and baseline);
//! text-align left only; no soft-hyphen/shaping — measurement is per-word
//! advance sums from [`FontSource`].

use amazing_dom::NodeIdx;
use amazing_text::{Paragraph, ShapingBoundary, TextSpan, TextSystem};
use std::collections::HashMap;
use std::ops::Range;

use crate::font::FontSource;
use crate::{GlyphFrag, InlineNode, IntrinsicSize, LineBox, ReplacedFrag, WordFrag};

/// The block's typographic strut (§10.8): every line box gets this height
/// and this baseline placement.
pub struct Strut {
    pub font_size: f32,
    pub line_height: f32,
    /// Ascent/descent of the strut font at `font_size` (positive numbers).
    pub ascent: f32,
    pub descent: f32,
}

impl Strut {
    pub fn new(fonts: &FontSource, font_size: f32, line_height: f32) -> Strut {
        let m = fonts.scaled_metrics(font_size);
        Strut { font_size, line_height, ascent: m.ascent, descent: m.descent }
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
            InlineNode::Break => {
                tokens.push(Token::Break);
                // Whitespace following a <br> starts the new line stripped,
                // so the pending flag survives the break.
            },
            InlineNode::Text { node, text, font_size, .. } => {
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
            },
            InlineNode::Replaced { node, size } => {
                tokens.push(Token::Replaced {
                    node: *node,
                    size: *size,
                    space_before: pending_space,
                });
                pending_space = false;
            },
        }
    }
    tokens
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
    let tokens = tokenize(items);

    let half_leading = (strut.line_height - (strut.ascent + strut.descent)) / 2.0;
    let right_edge = content_x + content_width;

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
        let image_height = replaced.iter().map(|image| image.height).fold(0.0, f32::max);
        let height = strut.line_height.max(image_height);
        let y_top = content_y + lines.iter().map(|line| line.height).sum::<f32>();
        let text_baseline = y_top + half_leading + strut.ascent;
        let baseline = text_baseline.max(y_top + image_height);
        for image in replaced.iter_mut() {
            image.y = baseline - image.height;
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
        *cx = content_x;
    };

    for token in tokens {
        let has_content = !words.is_empty() || !replaced.is_empty();
        match token {
            Token::Break => flush(&mut lines, &mut words, &mut replaced, &mut cx),
            Token::Word { text, node, font_size, space_before } => {
                let word_w = fonts.measure(&text, font_size);
                let space_w = if has_content && space_before {
                    fonts.measure(" ", font_size)
                } else {
                    0.0
                };
                if has_content && cx + space_w + word_w > right_edge + f32::EPSILON {
                    flush(&mut lines, &mut words, &mut replaced, &mut cx);
                }
                let wx = cx + if cx > content_x { space_w } else { 0.0 };
                words.push(WordFrag { node, text, x: wx, width: word_w, font_size });
                cx = wx + word_w;
            },
            Token::Replaced { node, size, space_before } => {
                let space_w = if has_content && space_before {
                    fonts.measure(" ", strut.font_size)
                } else {
                    0.0
                };
                if has_content && cx + space_w + size.width > right_edge + f32::EPSILON {
                    flush(&mut lines, &mut words, &mut replaced, &mut cx);
                }
                let x = cx + if cx > content_x { space_w } else { 0.0 };
                replaced.push(ReplacedFrag {
                    node,
                    x,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                });
                cx = x + size.width;
            },
        }
    }
    flush(&mut lines, &mut words, &mut replaced, &mut cx);

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

#[derive(Clone)]
struct InlinePaintInfo {
    background_color: String,
    padding: crate::values::Sides,
}

enum ShapedUnit {
    Word(ShapedWord),
    Break,
    Replaced { node: NodeIdx, size: IntrinsicSize },
}

/// Shapes paragraph chunks before wrapping, then constructs word summaries,
/// positioned glyphs, and line boxes from the same advances.
pub(crate) fn layout_lines_shaped(
    items: &[InlineNode],
    content_x: f32,
    content_y: f32,
    content_width: f32,
    strut: &Strut,
    text_system: &dyn TextSystem,
) -> Option<Vec<LineBox>> {
    let mut units = Vec::new();
    let mut text_chunk = Vec::new();
    for item in items {
        match item {
            InlineNode::Text { .. } => text_chunk.push(item),
            InlineNode::Break => {
                units.extend(shape_chunk(&text_chunk, text_system)?);
                text_chunk.clear();
                units.push(ShapedUnit::Break);
            },
            InlineNode::Replaced { node, size } => {
                units.extend(shape_chunk(&text_chunk, text_system)?);
                text_chunk.clear();
                units.push(ShapedUnit::Replaced { node: *node, size: *size });
            },
        }
    }
    units.extend(shape_chunk(&text_chunk, text_system)?);

    let half_leading = (strut.line_height - (strut.ascent + strut.descent)) / 2.0;
    let right_edge = content_x + content_width;
    let mut lines = Vec::new();
    let mut words = Vec::new();
    let mut glyphs = Vec::new();
    let mut decorations = Vec::new();
    let mut replaced = Vec::new();
    let mut cursor = content_x;
    let flush = |lines: &mut Vec<LineBox>,
                 words: &mut Vec<WordFrag>,
                 glyphs: &mut Vec<GlyphFrag>,
                 decorations: &mut Vec<crate::InlineDecorationFrag>,
                 replaced: &mut Vec<ReplacedFrag>,
                 cursor: &mut f32| {
        if words.is_empty() && replaced.is_empty() {
            return;
        }
        let image_height = replaced.iter().map(|image| image.height).fold(0.0, f32::max);
        let height = strut.line_height.max(image_height);
        let y_top = content_y + lines.iter().map(|line| line.height).sum::<f32>();
        let text_baseline = y_top + half_leading + strut.ascent;
        let baseline = text_baseline.max(y_top + image_height);
        for image in replaced.iter_mut() {
            image.y = baseline - image.height;
        }
        for decoration in decorations.iter_mut() {
            decoration.y += baseline;
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
        *cursor = content_x;
    };

    for unit in units {
        let has_content = !words.is_empty() || !replaced.is_empty();
        match unit {
            ShapedUnit::Break => flush(
                &mut lines,
                &mut words,
                &mut glyphs,
                &mut decorations,
                &mut replaced,
                &mut cursor,
            ),
            ShapedUnit::Word(mut word) => {
                let spacing = if has_content { word.space_before } else { 0.0 };
                if has_content && cursor + spacing + word.width > right_edge + f32::EPSILON {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut decorations,
                        &mut replaced,
                        &mut cursor,
                    );
                }
                let x = cursor + if cursor > content_x { word.space_before } else { 0.0 };
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
            },
            ShapedUnit::Replaced { node, size } => {
                if has_content && cursor + size.width > right_edge + f32::EPSILON {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut decorations,
                        &mut replaced,
                        &mut cursor,
                    );
                }
                replaced.push(ReplacedFrag {
                    node,
                    x: cursor,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                });
                cursor += size.width;
            },
        }
    }
    flush(
        &mut lines,
        &mut words,
        &mut glyphs,
        &mut decorations,
        &mut replaced,
        &mut cursor,
    );
    Some(lines)
}

fn shape_chunk(items: &[&InlineNode], text_system: &dyn TextSystem) -> Option<Vec<ShapedUnit>> {
    let mut paragraph_text = String::new();
    let mut spans = Vec::new();
    let mut colors = HashMap::new();
    let mut paint_info = HashMap::new();
    let mut previous_after = ShapingBoundary::Continue;
    for item in items {
        let InlineNode::Text {
            node,
            text,
            style,
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
        if text.is_empty() {
            continue;
        }
        let start = paragraph_text.len();
        paragraph_text.push_str(text);
        let end = paragraph_text.len();
        spans.push(TextSpan {
            range: start..end,
            source: node.0,
            style: style.clone(),
            boundary_before: stronger_boundary(previous_after, *boundary_before),
        });
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
    let shaped = text_system.shape(Paragraph {
        text: &paragraph_text,
        spans: &spans,
    }).ok()?;
    let word_ranges = word_ranges(&paragraph_text);
    let mut units = Vec::new();
    let mut previous_end = 0usize;
    for range in word_ranges {
        let mut glyphs = Vec::new();
        let mut decorations = Vec::new();
        let mut pen = 0.0;
        let mut node = spans[0].source;
        let mut font_size = spans[0].style.size;
        let mut active_source = None;
        let mut decoration_start = 0.0;
        for run in &shaped.runs {
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
                glyphs.push(GlyphFrag {
                        node: NodeIdx(run.source),
                        font: run.font,
                        glyph: glyph.id,
                        cluster: glyph.cluster.clone(),
                    x: if run.writing_mode == amazing_text::WritingMode::HorizontalTb {
                        pen + glyph.offset_x
                    } else {
                        glyph.offset_x
                    },
                    y: if run.writing_mode == amazing_text::WritingMode::HorizontalTb {
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
                        variations: run.variations.clone(),
                        synthesis: run.synthesis,
                        color: colors
                            .get(&run.source)
                            .cloned()
                            .unwrap_or_else(|| "black".into()),
                });
                pen += match run.writing_mode {
                    amazing_text::WritingMode::HorizontalTb => glyph.advance_x.abs(),
                    amazing_text::WritingMode::VerticalRl | amazing_text::WritingMode::VerticalLr => {
                        glyph.advance_y.abs()
                    }
                };
            }
        }
        finish_source_decoration(
            active_source,
            decoration_start,
            &mut pen,
            &paint_info,
            &mut decorations,
        );
        units.push(ShapedUnit::Word(ShapedWord {
            node: NodeIdx(node),
            text: paragraph_text[range.clone()].to_owned(),
            width: pen,
            font_size,
            space_before: advance_in_range(&shaped, previous_end..range.start),
            glyphs,
            decorations,
        }));
        previous_end = range.end;
    }
    Some(units)
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

fn word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (offset, ch) in text.char_indices() {
        if ch == ' ' {
            if let Some(word_start) = start.take() {
                ranges.push(word_start..offset);
            }
        } else if start.is_none() {
            start = Some(offset);
        }
    }
    if let Some(word_start) = start {
        ranges.push(word_start..text.len());
    }
    ranges
}

fn advance_in_range(shaped: &amazing_text::ShapedParagraph, range: Range<usize>) -> f32 {
    shaped
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(move |glyph| (run.writing_mode, glyph)))
        .filter(|(_, glyph)| range.contains(&glyph.cluster.start))
        .map(|(writing_mode, glyph)| match writing_mode {
            amazing_text::WritingMode::HorizontalTb => glyph.advance_x.abs(),
            amazing_text::WritingMode::VerticalRl | amazing_text::WritingMode::VerticalLr => {
                glyph.advance_y.abs()
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSource;

    fn text(t: &str) -> InlineNode {
        InlineNode::Text {
            node: NodeIdx(99),
            text: t.into(),
            font_size: 16.0,
            style: amazing_text::TextStyle::default(),
            color: "black".into(),
            background_color: "transparent".into(),
            inline_padding: crate::values::Sides::default(),
            boundary_before: amazing_text::ShapingBoundary::Continue,
            boundary_after: amazing_text::ShapingBoundary::Continue,
        }
    }

    fn strut() -> Strut {
        Strut { font_size: 16.0, line_height: 20.0, ascent: 14.0, descent: 4.0 }
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
        let lines = layout_lines(&[text("a "), InlineNode::Break, text(" b")], 0.0, 0.0, 500.0, &strut(), &fonts);
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
        let lines = layout_lines(&[text("one "), text("two")], 0.0, 0.0, 500.0, &strut(), &fonts);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 2);
        assert!(lines[0].words[1].x > lines[0].words[0].x + lines[0].words[0].width);
    }

    #[test]
    fn oversized_word_overflows_but_places() {
        let fonts = FontSource::fallback();
        let lines = layout_lines(&[text("aaaaaaaaaaaaaaaaaaaa")], 0.0, 0.0, 30.0, &strut(), &fonts);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].words[0].width > 30.0);
    }
}
