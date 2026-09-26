//! Resolve plaintext paragraph alignment through the shaping provider's bidi
//! result. A leading neutral inside the paragraph's FSI resolves to its base
//! level, even if its first visible character is a number or an isolate.
use super::ShapedContext;
use amazing_text::{
    BidiScope, Direction, Paragraph, ShapingBoundary, TextSpan, TextSystem, UnicodeBidi,
};
use std::ops::Range;

pub(super) fn plaintext_directions(
    context: &ShapedContext,
    text_system: &dyn TextSystem,
) -> Vec<(Range<usize>, Direction)> {
    let mut paragraphs = Vec::new();
    let mut start = 0;
    for (offset, ch) in context.text.char_indices() {
        if matches!(
            ch,
            '\n' | '\r' | '\u{001c}' | '\u{001d}' | '\u{001e}' | '\u{0085}' | '\u{2029}'
        ) {
            if start < offset {
                paragraphs.push(start..offset);
            }
            start = offset + ch.len_utf8();
        }
    }
    if start < context.text.len() {
        paragraphs.push(start..context.text.len());
    }
    // Bound repeated scope traversal for deeply nested scopes crossing many
    // paragraphs. Provider failure and budget exhaustion use the specified
    // direction, as the existing shaping fallback does.
    let mut work = context.text.len().saturating_mul(4).saturating_add(262_144);
    paragraphs
        .into_iter()
        .map(|range| {
            let cost = range.len().saturating_add(context.bidi_scopes.len());
            let direction = if cost <= work {
                work -= cost;
                probe(context, range.clone(), text_system).unwrap_or(context.base_direction)
            } else {
                context.base_direction
            };
            (range, direction)
        })
        .collect()
}

fn probe(
    context: &ShapedContext,
    range: Range<usize>,
    text_system: &dyn TextSystem,
) -> Option<Direction> {
    let mut text = String::with_capacity(range.len() + 1);
    text.push(' ');
    text.push_str(&context.text[range.clone()]);
    let first = context
        .spans
        .partition_point(|span| span.range.end <= range.start);
    let style = context.spans.get(first)?.style.clone();
    let mut spans = vec![TextSpan {
        range: 0..1,
        source: 0,
        style,
        boundary_before: ShapingBoundary::Hard,
    }];
    for span in &context.spans[first..] {
        if span.range.start >= range.end {
            break;
        }
        let mut span = span.clone();
        span.range = span.range.start.max(range.start) - range.start + 1
            ..span.range.end.min(range.end) - range.start + 1;
        spans.push(span);
    }
    let mut scopes = vec![BidiScope {
        range: 0..text.len(),
        direction: Direction::LeftToRight,
        unicode_bidi: UnicodeBidi::Plaintext,
    }];
    for scope in &context.bidi_scopes {
        let start = scope.range.start.max(range.start);
        let end = scope.range.end.min(range.end);
        if start < end {
            scopes.push(BidiScope {
                range: start - range.start + 1..end - range.start + 1,
                ..scope.clone()
            });
        }
    }
    text_system
        .shape(Paragraph {
            text: &text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &scopes,
            base_direction: context.base_direction,
        })
        .ok()?
        .runs
        .iter()
        .find(|run| run.glyphs.iter().any(|glyph| glyph.cluster.start == 0))
        .map(|run| run.direction)
}
