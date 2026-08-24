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
        Strut {
            font_size,
            line_height,
            ascent: m.ascent,
            descent: m.descent,
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
            InlineNode::BidiStart { .. } | InlineNode::BidiEnd => {},
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
            InlineNode::Replaced { node, size } => {
                tokens.push(Token::Replaced {
                    node: *node,
                    size: *size,
                    space_before: pending_space,
                });
                pending_space = false;
            }
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
        let image_height = replaced
            .iter()
            .map(|image| image.height)
            .fold(0.0, f32::max);
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
            Token::Word {
                text,
                node,
                font_size,
                space_before,
            } => {
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
                size,
                space_before,
            } => {
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
            }
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
    range: Range<usize>,
    context: Rc<ShapedContext>,
    preserve_space_before: bool,
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
struct InlinePaintInfo {
    background_color: String,
    padding: crate::values::Sides,
}

#[derive(Debug)]
struct ShapedContext {
    text: String,
    spans: Vec<TextSpan>,
    sources: Vec<SourceMapping>,
    bidi_scopes: Vec<amazing_text::BidiScope>,
    colors: HashMap<u32, String>,
    paint_info: HashMap<u32, InlinePaintInfo>,
    atomics: HashMap<usize, (NodeIdx, IntrinsicSize)>,
    base_direction: amazing_text::Direction,
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
        size: IntrinsicSize,
        range: Range<usize>,
        context: Rc<ShapedContext>,
    },
}

/// Shapes paragraph chunks before wrapping, then constructs word summaries,
/// positioned glyphs, and line boxes from the same advances.
pub(crate) fn layout_lines_shaped(
    items: &[InlineNode],
    content_x: f32,
    content_y: f32,
    content_width: f32,
    strut: &Strut,
    base_direction: amazing_text::Direction,
    text_system: &dyn TextSystem,
) -> Option<Vec<LineBox>> {
    let item_refs = items.iter().collect::<Vec<_>>();
    let units = shape_chunk(&item_refs, base_direction, text_system)?;

    let half_leading = (strut.line_height - (strut.ascent + strut.descent)) / 2.0;
    let right_edge = content_x + content_width;
    let mut lines = Vec::new();
    let mut words = Vec::new();
    let mut glyphs = Vec::new();
    let mut decorations = Vec::new();
    let mut replaced = Vec::new();
    let mut placed_text = Vec::new();
    let mut cursor = content_x;
    let mut may_break_after = true;
    let flush = |lines: &mut Vec<LineBox>,
                 words: &mut Vec<WordFrag>,
                 glyphs: &mut Vec<GlyphFrag>,
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
            ShapedUnit::Break => {
                flush(
                    &mut lines,
                    &mut words,
                    &mut glyphs,
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
                let mut spacing = if has_content || word.preserve_space_before {
                    word.space_before
                } else {
                    0.0
                };
                let break_width = if word.hyphen_after {
                    word.hyphen_advance
                } else {
                    0.0
                };
                if has_content
                    && may_break_after
                    && cursor + spacing + word.width + break_width > right_edge + f32::EPSILON
                {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut decorations,
                        &mut replaced,
                        &mut placed_text,
                        true,
                        false,
                        &mut cursor,
                    );
                    if !word.preserve_space_before {
                        spacing = 0.0;
                    }
                }
                let line_range_x = cursor;
                let x = cursor + spacing;
                placed_text.push(PlacedText {
                    context: Rc::clone(&word.context),
                    range: word.range.clone(),
                    x: if word.preserve_space_before {
                        line_range_x
                    } else {
                        x
                    },
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
                may_break_after = !word.hyphen_after
                    || cursor + word.hyphen_advance <= right_edge + f32::EPSILON;
            }
            ShapedUnit::Replaced {
                node,
                size,
                range,
                context,
            } => {
                if has_content
                    && may_break_after
                    && cursor + size.width > right_edge + f32::EPSILON
                {
                    flush(
                        &mut lines,
                        &mut words,
                        &mut glyphs,
                        &mut decorations,
                        &mut replaced,
                        &mut placed_text,
                        true,
                        false,
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
                placed_text.push(PlacedText {
                    context,
                    range,
                    x: cursor,
                    hyphen_after: false,
                    requires_reshape: false,
                });
                cursor += size.width;
                may_break_after = true;
            }
        }
    }
    flush(
        &mut lines,
        &mut words,
        &mut glyphs,
        &mut decorations,
        &mut replaced,
        &mut placed_text,
        false,
        false,
        &mut cursor,
    );
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
    let vertical = context.spans.first().is_some_and(|span| {
        span.style.writing_mode != amazing_text::WritingMode::HorizontalTb
    });
    let mut pen = if vertical { 0.0 } else { start_x };
    let mut active_source = None;
    let mut decoration_start = pen;
    let replaced_by_node = replaced
        .iter()
        .enumerate()
        .map(|(index, fragment)| (fragment.node.0, index))
        .collect::<HashMap<_, _>>();
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
                if let Some((node, size)) = context.atomics.get(&glyph.cluster.start)
                    && let Some(index) = replaced_by_node.get(&node.0)
                    && let Some(fragment) = replaced.get_mut(*index)
                {
                    fragment.x = if vertical { start_x } else { pen };
                    pen += if vertical { size.height } else { size.width };
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
                variations: run.variations.clone(),
                synthesis: run.synthesis,
                color: context
                    .colors
                    .get(&run.source)
                    .cloned()
                    .unwrap_or_else(|| "black".into()),
            });
            pen += if vertical {
                glyph.advance_y.abs()
            } else {
                glyph.advance_x.abs()
            };
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

fn shape_chunk(
    items: &[&InlineNode],
    base_direction: amazing_text::Direction,
    text_system: &dyn TextSystem,
) -> Option<Vec<ShapedUnit>> {
    let mut paragraph_text = String::new();
    let mut spans = Vec::new();
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
            InlineNode::Replaced { node, size } => {
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
                atomics.insert(start, (*node, *size));
                previous_after = ShapingBoundary::Hard;
                continue;
            }
            InlineNode::Text { .. } => {}
        }
        let InlineNode::Text {
            node,
            text,
            source_mappings,
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
                        source: node.0,
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
                    source: node.0,
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
            TextUnit::Atomic { offset, node, size } => {
                let end = offset + '\u{fffc}'.len_utf8();
                units.push(ShapedUnit::Replaced {
                    node,
                    size,
                    range: offset..end,
                    context: Rc::clone(&context),
                });
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
        let preserve_space_before = spans
            .iter()
            .find(|span| span.range.contains(&range.start))
            .is_some_and(|span| {
                matches!(
                    span.style.white_space,
                    amazing_text::WhiteSpace::Pre
                        | amazing_text::WhiteSpace::PreWrap
                        | amazing_text::WhiteSpace::BreakSpaces
                )
            });
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
                    amazing_text::WritingMode::VerticalRl
                    | amazing_text::WritingMode::VerticalLr => glyph.advance_y.abs(),
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
        let hyphen_after = break_kind == amazing_text::BreakKind::Hyphen;
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
            space_before: advance_in_range(&shaped, previous_end..range.start),
            glyphs,
            decorations,
            range: if preserve_space_before {
                previous_end..range.end
            } else {
                range.clone()
            },
            context: Rc::clone(&context),
            preserve_space_before,
            hyphen_after,
            hyphen_advance,
            requires_reshape: shaped.unsafe_breaks.binary_search(&range.end).is_ok(),
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

enum TextUnit {
    Word {
        range: Range<usize>,
        break_kind: amazing_text::BreakKind,
    },
    Atomic {
        offset: usize,
        node: NodeIdx,
        size: IntrinsicSize,
    },
    Break(usize),
}

fn text_units(
    text: &str,
    spans: &[TextSpan],
    breaks: &[amazing_text::BreakOpportunity],
    atomics: &HashMap<usize, (NodeIdx, IntrinsicSize)>,
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
                break_kind: opportunity.kind,
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
            break_kind: amazing_text::BreakKind::Mandatory,
        });
    }
    split_atomic_units(units, atomics)
}

fn split_atomic_units(
    units: Vec<TextUnit>,
    atomics: &HashMap<usize, (NodeIdx, IntrinsicSize)>,
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
                    break_kind: amazing_text::BreakKind::Allowed,
                });
            }
            let (node, size) = atomics[&offset];
            expanded.push(TextUnit::Atomic { offset, node, size });
            cursor = offset + '\u{fffc}'.len_utf8();
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
        .map(|(writing_mode, glyph)| match writing_mode {
            amazing_text::WritingMode::HorizontalTb => glyph.advance_x.abs(),
            amazing_text::WritingMode::VerticalRl | amazing_text::WritingMode::VerticalLr => {
                glyph.advance_y.abs()
            }
        })
        .sum()
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
    use crate::font::FontSource;

    fn text(t: &str) -> InlineNode {
        let len = t.len();
        InlineNode::Text {
            node: NodeIdx(99),
            text: t.into(),
            source_mappings: (!t.is_empty())
                .then_some(crate::TextSourceMapping {
                    text_range: 0..len,
                    source_range: 0..len,
                })
                .into_iter()
                .collect(),
            font_size: 16.0,
            style: Box::new(amazing_text::TextStyle::default()),
            color: "black".into(),
            background_color: "transparent".into(),
            inline_padding: crate::values::Sides::default(),
            boundary_before: amazing_text::ShapingBoundary::Continue,
            boundary_after: amazing_text::ShapingBoundary::Continue,
        }
    }

    fn strut() -> Strut {
        Strut {
            font_size: 16.0,
            line_height: 20.0,
            ascent: 14.0,
            descent: 4.0,
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
