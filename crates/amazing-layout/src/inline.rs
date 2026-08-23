//! Inline formatting context (CSS2.1 §9.4.2 + §10.8 line box math):
//! flattens [`InlineNode`] runs into words, wraps them greedily, and emits
//! absolutely-positioned [`LineBox`]es with baseline positions.
//!
//! Simplifications (documented in lib.rs): one strut per block (the block's
//! font-size/line-height decide every line's height and baseline);
//! text-align left only; no soft-hyphen/shaping — measurement is per-word
//! advance sums from [`FontSource`].

use amazing_dom::NodeIdx;

use crate::font::FontSource;
use crate::{InlineNode, IntrinsicSize, LineBox, ReplacedFrag, WordFrag};

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
            InlineNode::Text { node, text, font_size } => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSource;

    fn text(t: &str) -> InlineNode {
        InlineNode::Text { node: NodeIdx(99), text: t.into(), font_size: 16.0 }
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
