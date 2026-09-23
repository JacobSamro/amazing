//! UAX #14 LB20 for unresolved U+FFFC contingent break opportunities.
//! ICU's line iterator omits some boundaries between adjacent inline objects.

use super::{BreakOpportunity, break_kind};
use amazing_text::{ObjectBreakClass, TextSpan, object_break_allowed};
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
    object_break_allowed(text, offset, object_break_class)
}

fn object_break_class(ch: char) -> ObjectBreakClass {
    match ch.line_break() {
        LineBreak::OP => ObjectBreakClass::Opening,
        LineBreak::SP => ObjectBreakClass::Space,
        LineBreak::WJ | LineBreak::GL | LineBreak::ZWJ | LineBreak::QU => {
            ObjectBreakClass::BothProhibited
        }
        LineBreak::CM
        | LineBreak::CL
        | LineBreak::CP
        | LineBreak::EX
        | LineBreak::IS
        | LineBreak::SY
        | LineBreak::ZW
        | LineBreak::BK
        | LineBreak::CR
        | LineBreak::LF
        | LineBreak::NL => ObjectBreakClass::RightProhibited,
        _ => ObjectBreakClass::Other,
    }
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
