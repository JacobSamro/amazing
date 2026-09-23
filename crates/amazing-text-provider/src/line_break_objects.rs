//! UAX #14 LB20 for unresolved U+FFFC contingent break opportunities.
//! ICU's line iterator omits some boundaries between adjacent inline objects.

use super::{BreakOpportunity, break_kind};
use amazing_text::TextSpan;
use swash::text::{Codepoint, LineBreak};

const OBJECT: char = '\u{fffc}';

fn adjacent_object(text: &str, offset: usize) -> bool {
    offset > 0
        && offset < text.len()
        && (text[..offset].ends_with(OBJECT) || text[offset..].starts_with(OBJECT))
}

pub(super) fn allows(text: &str, offset: usize) -> bool {
    if !adjacent_object(text, offset) {
        return true;
    }
    let left = text[..offset].chars().next_back().unwrap().line_break();
    let right = text[offset..].chars().next().unwrap().line_break();

    // LB6-8a, LB11-13, LB15d, and LB19 precede LB20. A contingent
    // object break must not defeat joining, glue, or closing punctuation.
    if matches!(
        left,
        LineBreak::WJ | LineBreak::GL | LineBreak::ZWJ | LineBreak::QU
    ) || matches!(
        right,
        LineBreak::WJ
            | LineBreak::GL
            | LineBreak::ZWJ
            | LineBreak::CM
            | LineBreak::QU
            | LineBreak::CL
            | LineBreak::CP
            | LineBreak::EX
            | LineBreak::IS
            | LineBreak::SY
            | LineBreak::SP
            | LineBreak::ZW
            | LineBreak::BK
            | LineBreak::CR
            | LineBreak::LF
            | LineBreak::NL
    ) {
        return false;
    }
    // LB14 applies across intervening spaces: OP SP* × CB.
    if text[offset..].starts_with(OBJECT)
        && text[..offset]
            .chars()
            .rev()
            .find(|ch| ch.line_break() != LineBreak::SP)
            .is_some_and(|ch| ch.line_break() == LineBreak::OP)
    {
        return false;
    }
    true
}

pub(super) fn opportunities(text: &str, spans: &[TextSpan]) -> Vec<BreakOpportunity> {
    text.char_indices()
        .skip(1)
        .filter_map(|(offset, _)| {
            if adjacent_object(text, offset) && allows(text, offset) {
                break_kind(text, spans, offset).map(|kind| BreakOpportunity { offset, kind })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_breaks;
    use amazing_text::{BreakKind, ShapingBoundary, TextStyle, WhiteSpace};

    fn offsets(text: &str, style: TextStyle) -> Vec<usize> {
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 0,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];
        line_breaks(text, &spans)
            .into_iter()
            .filter(|op| op.kind == BreakKind::Allowed)
            .map(|op| op.offset)
            .collect()
    }

    #[test]
    fn breaks_between_and_around_atomic_inlines() {
        assert_eq!(
            offsets("\u{fffc}\u{fffc}\u{fffc}", TextStyle::default()),
            vec![3, 6]
        );
        assert_eq!(offsets("a\u{fffc}b", TextStyle::default()), vec![1, 4]);
        assert!(
            offsets(
                "\u{fffc}\u{fffc}",
                TextStyle {
                    white_space: WhiteSpace::NoWrap,
                    ..TextStyle::default()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn higher_priority_neighbours_still_prohibit_breaks() {
        for prefix in ["「", "「 ", "\"", "\u{00a0}", "\u{2060}", "\u{200d}"] {
            let text = format!("{prefix}\u{fffc}");
            assert!(
                !offsets(&text, TextStyle::default()).contains(&prefix.len()),
                "{prefix:?}"
            );
        }
        for suffix in [
            "」", ")", "!", ",", "/", "\"", "\u{00a0}", "\u{2060}", "\u{200d}", " ", "\n",
            "\u{200b}",
        ] {
            let text = format!("\u{fffc}{suffix}");
            assert!(
                !offsets(&text, TextStyle::default()).contains(&OBJECT.len_utf8()),
                "{suffix:?}"
            );
        }
    }
}
