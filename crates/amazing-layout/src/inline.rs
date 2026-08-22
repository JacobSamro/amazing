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
use crate::{InlineNode, LineBox, WordFrag};

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

/// One measured word queued for placement.
pub(crate) struct Token {
    pub text: String,
    pub node: NodeIdx,
    pub font_size: f32,
    /// A collapsible space precedes this word in the source flow.
    pub space_before: bool,
}

/// Splits flattened inline items into word tokens, tracking whether a
/// collapsible space sits between consecutive words across item boundaries.
pub(crate) fn tokenize(items: &[InlineNode]) -> (Vec<Token>, Vec<usize>) {
    let mut tokens = Vec::new();
    let mut breaks = Vec::new();
    let mut pending_space = false;
    for item in items {
        match item {
            InlineNode::Break => {
                breaks.push(tokens.len());
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
                    tokens.push(Token {
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
        }
    }
    (tokens, breaks)
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
    let (tokens, breaks) = tokenize(items);

    let half_leading = (strut.line_height - (strut.ascent + strut.descent)) / 2.0;
    let right_edge = content_x + content_width;

    let mut lines: Vec<LineBox> = Vec::new();
    let mut words: Vec<WordFrag> = Vec::new();
    let mut cx = content_x;

    let flush = |lines: &mut Vec<LineBox>, words: &mut Vec<WordFrag>, cx: &mut f32| {
        if words.is_empty() {
            return;
        }
        let index = lines.len() as u32;
        let y_top = content_y + index as f32 * strut.line_height;
        lines.push(LineBox {
            y_top,
            height: strut.line_height,
            baseline: y_top + half_leading + strut.ascent,
            words: std::mem::take(words),
        });
        *cx = content_x;
    };

    let mut break_set = breaks.as_slice();
    for (i, tok) in tokens.iter().enumerate() {
        // Any `<br>` positioned before this token forces the break first.
        while let Some((b, rest)) = break_set.split_first() {
            if *b <= i {
                flush(&mut lines, &mut words, &mut cx);
                break_set = rest;
            } else {
                break;
            }
        }

        let word_w = fonts.measure(&tok.text, tok.font_size);
        let space_w = if !words.is_empty() && tok.space_before {
            fonts.measure(" ", tok.font_size)
        } else {
            0.0
        };
        // Greedy fill: wrap when the word (plus its preceding space) no
        // longer fits beside existing content. A word alone on a line is
        // placed regardless of overflow.
        if !words.is_empty() && cx + space_w + word_w > right_edge + f32::EPSILON {
            flush(&mut lines, &mut words, &mut cx);
        }
        let wx = cx + space_w;
        words.push(WordFrag {
            node: tok.node,
            text: tok.text.clone(),
            x: wx,
            width: word_w,
            font_size: tok.font_size,
        });
        cx = wx + word_w;
    }
    // Trailing `<br>` (break index == token count).
    while let Some((b, rest)) = break_set.split_first() {
        if *b >= tokens.len() {
            flush(&mut lines, &mut words, &mut cx);
            break_set = rest;
        } else {
            break;
        }
    }
    flush(&mut lines, &mut words, &mut cx);

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
