//! Used padding for a non-atomic inline scope. The computed values stay
//! untouched until the actual containing block's logical width is available.
use crate::values::{self, FontRelativeUnits, Sides};
use amazing_css::ComputedStyle;
use amazing_text::{TextStyle, TextSystem};

pub(crate) struct Context<'a> {
    /// None is the cyclic-percentage intrinsic contribution, not a font basis.
    pub inline_size: Option<f32>,
    pub root_style: Option<&'a TextStyle>,
    pub root_line_height: f32,
}

pub(crate) fn resolve(
    style: &ComputedStyle,
    font_style: &TextStyle,
    text: &dyn TextSystem,
    context: &Context<'_>,
 ) -> Sides {
    resolve_kind(style, font_style, text, context, crate::InlineEdgeKind::Padding)
}

pub(crate) fn resolve_kind(
    style: &ComputedStyle, font_style: &TextStyle, text: &dyn TextSystem,
    context: &Context<'_>, kind: crate::InlineEdgeKind,
) -> Sides {
    let normal = text
        .metrics(font_style)
        .ok()
        .map(|metrics| metrics.line_height().max(font_style.size * 1.2))
        .unwrap_or(font_style.size * 1.2);
    let line_height = values::resolve_used_line_height(style, font_style.size, normal);
    let units = FontRelativeUnits::from_text_system(
        Some(text),
        Some(font_style),
        context.root_style,
        font_style.size,
    )
    .with_line_heights(line_height, context.root_line_height);
    let side = |property: &str| {
        let Some(source) = style.get(property) else {
            return 0.0;
        };
        let basis = context.inline_size.unwrap_or(0.0);
        // Piecewise math must choose its branch with the actual basis. The
        // lower CSS evaluator handles computed px/percentage expressions.
        let zoom = style.effective_zoom();
        let value = amazing_css::used_position_axis(source, basis as f64 / zoom)
            .map(|value| (value * zoom) as f32)
            .or_else(|| {
                values::used_length_percentage_with_units(style, source, units)
                    .and_then(|length| length.resolve(Some(basis)))
            })
            .filter(|value| value.is_finite())
            .unwrap_or(0.0);
        if kind == crate::InlineEdgeKind::Padding { value.max(0.0) } else { value }
    };
    let prefix = if kind == crate::InlineEdgeKind::Padding { "padding" } else { "margin" };
    Sides {
        top: side(&format!("{prefix}-top")),
        right: side(&format!("{prefix}-right")),
        bottom: side(&format!("{prefix}-bottom")),
        left: side(&format!("{prefix}-left")),
    }
}

/// Physical sides participate along the containing block's inline axis. The
/// scope direction chooses its logical start/end; the percentage basis does
/// not change to the child's writing mode or to a float-reduced line width.
pub(crate) fn edge(
    sides: Sides,
    start: bool,
    mode: amazing_text::WritingMode,
    direction: amazing_text::Direction,
) -> f32 {
    let first = start == (direction == amazing_text::Direction::LeftToRight);
    if mode.is_vertical_flow() {
        if first {
            sides.top
        } else {
            sides.bottom
        }
    } else if first {
        sides.left
    } else {
        sides.right
    }
}

/// Preserve one decoration for adjacent pieces of the same actual inline
/// box. Geometry is measured by visual layout; unrelated boxes never merge.
pub(crate) fn coalesce(decorations: &mut Vec<crate::InlineDecorationFrag>) {
    let mut merged: Vec<crate::InlineDecorationFrag> = Vec::with_capacity(decorations.len());
    for fragment in decorations.drain(..) {
        if let Some(previous) = merged.last_mut() {
            if previous.node == fragment.node
                && previous.color == fragment.color
                && (previous.y - fragment.y).abs() < 0.001
                && (previous.height - fragment.height).abs() < 0.001
                && (previous.x + previous.width - fragment.x).abs() < 0.001
            {
                previous.width += fragment.width;
                continue;
            }
        }
        merged.push(fragment);
    }
    *decorations = merged;
}

/// Ask the text provider for lexical breaks without padding placeholders,
/// then lift those offsets back into the actual shaping paragraph. Padding
/// is spacing, not an object replacement or an invented word boundary.
pub(crate) fn lexical_breaks(
    paragraph: amazing_text::Paragraph<'_>,
    mut edges: Vec<(usize, bool)>,
    text: &dyn TextSystem,
) -> Option<Vec<amazing_text::BreakOpportunity>> {
    use amazing_text::{BreakKind, BreakOpportunity, Paragraph};
    edges.sort_by_key(|edge| edge.0);
    let marker_len = '\u{fffc}'.len_utf8();
    let mut lexical = String::with_capacity(paragraph.text.len());
    let mut removed = Vec::with_capacity(edges.len());
    let mut cursor = 0;
    for (offset, start) in edges {
        lexical.push_str(&paragraph.text[cursor..offset]);
        removed.push((offset, lexical.len(), start));
        cursor = offset + marker_len;
    }
    lexical.push_str(&paragraph.text[cursor..]);
    if lexical.is_empty() {
        return Some(vec![BreakOpportunity {
            offset: paragraph.text.len(),
            kind: BreakKind::Mandatory,
        }]);
    }
    let compress = |offset: usize| {
        offset - marker_len * removed.partition_point(|(original, _, _)| *original < offset)
    };
    let range =
        |original: &std::ops::Range<usize>| compress(original.start)..compress(original.end);
    let spans = paragraph
        .spans
        .iter()
        .filter_map(|span| {
            let mut span = span.clone();
            span.range = range(&span.range);
            (!span.range.is_empty()).then_some(span)
        })
        .collect::<Vec<_>>();
    let sources = paragraph
        .sources
        .iter()
        .filter_map(|source| {
            let mut source = source.clone();
            source.text_range = range(&source.text_range);
            (!source.text_range.is_empty()).then_some(source)
        })
        .collect::<Vec<_>>();
    let bidi_scopes = paragraph
        .bidi_scopes
        .iter()
        .filter_map(|scope| {
            let mut scope = scope.clone();
            scope.range = range(&scope.range);
            (!scope.range.is_empty()).then_some(scope)
        })
        .collect::<Vec<_>>();
    let shaped = text
        .shape(Paragraph {
            text: &lexical,
            spans: &spans,
            sources: &sources,
            bidi_scopes: &bidi_scopes,
            base_direction: paragraph.base_direction,
        })
        .ok()?;
    Some(
        shaped
            .breaks
            .into_iter()
            .map(|mut opportunity| {
                let forced = opportunity.kind == BreakKind::Mandatory
                    && lexical[..opportunity.offset]
                        .chars()
                        .next_back()
                        .is_some_and(|ch| {
                            matches!(
                                ch,
                                '\n' | '\r'
                                    | '\u{000b}'
                                    | '\u{000c}'
                                    | '\u{0085}'
                                    | '\u{2028}'
                                    | '\u{2029}'
                            )
                        });
                if forced {
                    let before = removed
                        .partition_point(|(_, compressed, _)| *compressed < opportunity.offset);
                    opportunity.offset += before * marker_len;
                } else if opportunity.offset == lexical.len() {
                    opportunity.offset = paragraph.text.len();
                } else {
                    let index = removed
                        .partition_point(|(_, compressed, _)| *compressed < opportunity.offset);
                    let mut original = opportunity.offset + index * marker_len;
                    // A break keeps closing edges with the preceding content and
                    // opening edges with following content, including empty scopes.
                    for &(offset, compressed, start) in &removed[index..] {
                        if compressed != opportunity.offset || start {
                            break;
                        }
                        original = offset + marker_len;
                    }
                    opportunity.offset = original;
                }
                opportunity
            })
            .collect(),
    )
}
