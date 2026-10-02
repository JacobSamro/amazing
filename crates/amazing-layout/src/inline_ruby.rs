//! Ruby measurement, justification and annotation placement.
use super::*;

// Nested ruby can be measured again when an outer annotation widens it. Charge
// all of those passes to one budget, preventing exponential reshaping of a
// deliberately deep chain while preserving the existing input bounds.
struct WorkBudget(std::cell::Cell<usize>);
impl WorkBudget {
    fn new() -> Self {
        Self(std::cell::Cell::new(MAX_RUBY_TEXT_BYTES * 4))
    }
    fn spend(&self, amount: usize) -> Option<()> {
        self.0.set(self.0.get().checked_sub(amount.max(1))?);
        Some(())
    }
}

fn shape_ruby_leaf(
    text: &crate::RubyText,
    block_offset: f32,
    text_system: &dyn TextSystem,
) -> Option<(Vec<GlyphFrag>, f32, amazing_text::FontMetrics)> {
    let adjusted = crate::font_policy::adjusted_style(&text.style, text.font_size_adjust, text_system);
    let style = adjusted.as_ref().unwrap_or(text.style.as_ref());
    let metrics = text_system.metrics(style).ok()?;
    if text.text.is_empty() {
        return Some((Vec::new(), 0.0, metrics));
    }
    let spans = [TextSpan {
        range: 0..text.text.len(),
        source: text.node.0,
        style: style.clone(),
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
            let character = text.text[glyph.cluster.clone()].chars().next();
            if character == Some('\t') {
                pen += text.format.tab_size.advance(
                    pen,
                    inline_advance(run.writing_mode, glyph.advance_x, glyph.advance_y)
                        .max(run.font_size * 0.3),
                );
                continue;
            }
            if character.is_some_and(is_invisible_text_control) {
                continue;
            }
            fragments.push(GlyphFrag {
                node: text.node,
                font: run.font,
                glyph: glyph.id,
                cluster: glyph.cluster.clone(),
                source_range: source_range_for_cluster(&sources, run.source, glyph.cluster.clone()),
                source_mappings: source_mappings_for_cluster(&sources, glyph.cluster.clone()),
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

struct RubyRun {
    glyphs: Vec<GlyphFrag>,
    width: f32,
    metrics: amazing_text::FontMetrics,
    over: f32,
    under: f32,
}

pub(super) fn shape_ruby_text(
    text: &crate::RubyText,
    block: f32,
    system: &dyn TextSystem,
) -> Option<(Vec<GlyphFrag>, f32, amazing_text::FontMetrics)> {
    let mut run = shape_text(text, system, 0, &WorkBudget::new())?;
    shift(
        &mut run.glyphs,
        text.style.writing_mode.is_vertical_flow(),
        0.0,
        block,
    );
    Some((run.glyphs, run.width, run.metrics))
}

fn shift(glyphs: &mut [GlyphFrag], vertical: bool, inline: f32, block: f32) {
    for glyph in glyphs {
        glyph.x += if vertical { block } else { inline };
        glyph.y += if vertical { inline } else { block };
    }
}

fn shape_text(
    text: &crate::RubyText,
    system: &dyn TextSystem,
    depth: usize,
    budget: &WorkBudget,
) -> Option<RubyRun> {
    budget.spend(text.text.len())?;
    if depth >= 64 {
        return None;
    }
    let (mut glyphs, mut width, metrics) = shape_ruby_leaf(text, 0.0, system)?;
    let vertical = text.style.writing_mode.is_vertical_flow();
    let mut over = 0.0f32;
    let mut under = 0.0f32;
    let mut delta = 0.0;
    for nested in &text.nested {
        let shaped = shape_ruby(
            text.node,
            &nested.columns,
            nested.overhang_spaces,
            system,
            depth + 1,
            0.0,
            budget,
        )?;
        let placeholder = glyphs
            .iter()
            .find(|g| g.cluster.start == nested.range.start)?;
        let origin = if vertical {
            placeholder.y
        } else {
            placeholder.x
        } + delta;
        let old_width = glyphs
            .iter()
            .filter(|g| g.cluster.start >= nested.range.start && g.cluster.start < nested.range.end)
            .map(|g| inline_advance(g.writing_mode, g.advance_x, g.advance_y))
            .sum::<f32>();
        glyphs.retain(|g| {
            g.cluster.start < nested.range.start || g.cluster.start >= nested.range.end
        });
        let difference = shaped.width - old_width;
        for glyph in &mut glyphs {
            if glyph.cluster.start >= nested.range.end {
                shift(std::slice::from_mut(glyph), vertical, difference, 0.0);
            }
        }
        let mut nested_glyphs = shaped.glyphs;
        shift(&mut nested_glyphs, vertical, origin, 0.0);
        // The outer paragraph treats a nested ruby as one indivisible cluster.
        for glyph in &mut nested_glyphs {
            glyph.cluster = nested.range.clone();
        }
        glyphs.extend(nested_glyphs);
        width += difference;
        // Following placeholders have already moved with the glyphs above.
        delta = 0.0;
        over = over.max(shaped.line_over);
        under = under.max(shaped.line_under);
    }
    if text.format.visibility != "visible" {
        glyphs.clear();
    }
    Some(RubyRun {
        glyphs,
        width: width.max(text.format.minimum_advance),
        metrics,
        over,
        under,
    })
}

fn annotations(
    column: &crate::RubyColumn,
) -> impl Iterator<Item = (&crate::RubyText, usize, usize, bool)> {
    column
        .annotation
        .iter()
        .map(|text| (text, column.annotation_span, 0, column.annotation_hidden))
        .chain(
            column
                .additional_annotations
                .iter()
                .map(|a| (&a.text, a.span, a.level, a.hidden)),
        )
}

pub(super) fn shape_ruby_units(
    node: NodeIdx,
    columns: &[crate::RubyColumn],
    spaces: bool,
    system: &dyn TextSystem,
) -> Option<Vec<ShapedRuby>> {
    let budget = WorkBudget::new();
    let mut units = Vec::new();
    let mut start = 0;
    while start < columns.len() {
        let mut end = start + 1;
        let mut cursor = start;
        // All overlapping annotations travel with their bases across a line break.
        while cursor < end {
            for (_, span, _, _) in annotations(&columns[cursor]) {
                end = end.max(cursor.saturating_add(span.max(1)).min(columns.len()));
            }
            cursor += 1;
        }
        let mut shaped = shape_ruby(node, &columns[start..end], spaces, system, 0, 0.0, &budget)?;
        if end < columns.len() {
            shaped.break_after =
                ruby_base_break_allowed(&columns[end - 1].base, &columns[end].base, system);
        }
        units.push(shaped);
        start = end;
    }
    Some(units)
}

fn ruby_base_break_allowed(
    left: &crate::RubyText,
    right: &crate::RubyText,
    system: &dyn TextSystem,
) -> bool {
    if left.text.ends_with(' ') || right.text.starts_with(' ') {
        return true;
    }
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
            style: crate::font_policy::adjusted_style(&left.style, left.font_size_adjust, system)
                .unwrap_or_else(|| left.style.as_ref().clone()),
            boundary_before: ShapingBoundary::Hard,
        },
        TextSpan {
            range: boundary..text.len(),
            source: right.node.0,
            style: crate::font_policy::adjusted_style(&right.style, right.font_size_adjust, system)
                .unwrap_or_else(|| right.style.as_ref().clone()),
            boundary_before: ShapingBoundary::Continue,
        },
    ];
    system
        .shape(Paragraph {
            text: &text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: left.style.direction,
        })
        .ok()
        .is_some_and(|p| {
            p.breaks
                .iter()
                .any(|b| b.offset == boundary && b.kind != amazing_text::BreakKind::Mandatory)
        })
        || (left.style.word_break != amazing_text::WordBreak::KeepAll
            && right.style.word_break != amazing_text::WordBreak::KeepAll
            && left.text.chars().next_back().is_some_and(is_ideographic)
            && right.text.chars().next().is_some_and(is_ideographic))
}

fn is_ideographic(c: char) -> bool {
    matches!(c as u32, 0x2e80..=0x2fff | 0x3040..=0x30ff | 0x31f0..=0x31ff
        | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x3ffff)
}

// Justification uses typographic boundaries, never glyph indices (ligatures and
// combining marks may share a cluster). Latin letters do not stretch apart.
fn align(
    run: &mut RubyRun,
    text: &crate::RubyText,
    available: f32,
    origin: f32,
    system: &dyn TextSystem,
    depth: usize,
    budget: &WorkBudget,
) {
    if text.text == "\u{fffc}" && text.nested.len() == 1 && available > run.width {
        let nested = &text.nested[0];
        if let Some(shaped) = shape_ruby(
            text.node,
            &nested.columns,
            nested.overhang_spaces,
            system,
            depth + 1,
            available,
            budget,
        ) {
            run.glyphs = shaped.glyphs;
            run.width = shaped.width;
        }
    }
    let extra = (available - run.width).max(0.0);
    let vertical = text.style.writing_mode.is_vertical_flow();
    let mut opportunities = Vec::new();
    let mut previous = None;
    for (offset, c) in text.text.char_indices() {
        if previous.is_some_and(|p: char| {
            (p.is_whitespace() && !c.is_whitespace()) || (is_ideographic(p) && is_ideographic(c))
        }) {
            opportunities.push(offset);
        }
        previous = Some(c);
    }
    let distribute = matches!(text.format.align.as_str(), "space-around" | "space-between")
        && !opportunities.is_empty();
    let around = text.format.align == "space-around";
    let step = if distribute {
        extra / (opportunities.len() + usize::from(around)) as f32
    } else {
        0.0
    };
    let edge = if distribute {
        if around { step * 0.5 } else { 0.0 }
    } else if text.format.align == "start" {
        if text.style.direction == amazing_text::Direction::RightToLeft {
            extra
        } else {
            0.0
        }
    } else {
        extra * 0.5
    };
    for glyph in &mut run.glyphs {
        let count = opportunities.partition_point(|&offset| offset <= glyph.cluster.start);
        let advance = origin + edge + step * count as f32;
        shift(std::slice::from_mut(glyph), vertical, advance, 0.0);
    }
}

struct PositionedAnnotation<'a> {
    text: &'a crate::RubyText,
    start: usize,
    end: usize,
    level: usize,
    run: RubyRun,
}

fn shape_ruby(
    node: NodeIdx,
    columns: &[crate::RubyColumn],
    spaces: bool,
    system: &dyn TextSystem,
    depth: usize,
    minimum_width: f32,
    budget: &WorkBudget,
) -> Option<ShapedRuby> {
    budget.spend(columns.len())?;
    if columns.is_empty() || columns.len() > MAX_RUBY_COLUMNS || depth >= 64 {
        return None;
    }
    let bytes = columns.iter().try_fold(0usize, |sum, c| {
        annotations(c).try_fold(sum.checked_add(c.base.text.len())?, |sum, (t, _, _, _)| {
            sum.checked_add(t.text.len())
        })
    })?;
    if bytes > MAX_RUBY_TEXT_BYTES {
        return None;
    }
    let font_size = columns[0].base.font_size;
    let mode = columns[0].base.style.writing_mode;
    let vertical = mode.is_vertical_flow();
    let mut bases = columns
        .iter()
        .map(|c| shape_text(&c.base, system, depth, budget))
        .collect::<Option<Vec<_>>>()?;
    let base_width = bases.iter().map(|b| b.width).sum();
    let mut widths: Vec<_> = bases.iter().map(|b| b.width).collect();
    let mut notes = Vec::new();
    for (start, column) in columns.iter().enumerate() {
        for (text, span, level, hidden) in annotations(column) {
            if hidden {
                continue;
            }
            let end = start.saturating_add(span.max(1)).min(columns.len());
            notes.push(PositionedAnnotation {
                text,
                start,
                end,
                level,
                run: shape_text(text, system, depth, budget)?,
            });
        }
    }
    // Constraints with the smallest span are satisfied before spanning levels.
    notes.sort_by_key(|n| (n.end - n.start, n.level));
    for note in &notes {
        let width: f32 = widths[note.start..note.end].iter().sum();
        let extra = (note.run.width - width).max(0.0) / (note.end - note.start) as f32;
        for width in &mut widths[note.start..note.end] {
            *width += extra;
        }
    }
    let extra = (minimum_width - widths.iter().sum::<f32>()).max(0.0) / widths.len() as f32;
    for width in &mut widths {
        *width += extra;
    }
    let mut positions = vec![0.0];
    for width in &widths {
        positions.push(positions.last()? + width);
    }
    let mut over = bases.iter().map(|b| b.over).fold(0.0, f32::max);
    let mut under = bases.iter().map(|b| b.under).fold(0.0, f32::max);
    let base_over = bases
        .iter()
        .map(|b| {
            if vertical {
                b.metrics.vertical_ascent
            } else {
                b.metrics.ascent
            }
        })
        .fold(0.0, f32::max);
    let base_under = bases
        .iter()
        .map(|b| {
            if vertical {
                b.metrics.vertical_descent
            } else {
                b.metrics.descent
            }
        })
        .fold(0.0, f32::max);
    notes.sort_by_key(|n| n.level);
    let mut first = 0;
    while first < notes.len() {
        let mut end = first + 1;
        while end < notes.len() && notes[end].level == notes[first].level {
            end += 1;
        }
        let mut level_over = over;
        let mut level_under = under;
        for note in &mut notes[first..end] {
            let position = note.text.format.position.as_str();
            let below = position.contains("under")
                ^ (position.contains("alternate") && note.level % 2 == 1);
            let ascent = if vertical {
                note.run.metrics.vertical_ascent
            } else {
                note.run.metrics.ascent
            };
            let descent = if vertical {
                note.run.metrics.vertical_descent
            } else {
                note.run.metrics.descent
            };
            let height = ascent + descent + note.run.over + note.run.under;
            let distance = if below {
                base_under + under + ascent + note.run.over
            } else {
                base_over + over + descent + note.run.under
            };
            let block = if vertical {
                if below { -distance } else { distance }
            } else {
                if below { distance } else { -distance }
            };
            align(
                &mut note.run,
                note.text,
                positions[note.end] - positions[note.start],
                positions[note.start],
                system,
                depth,
                budget,
            );
            shift(&mut note.run.glyphs, vertical, 0.0, block);
            if below {
                level_under = level_under.max(under + height);
            } else {
                level_over = level_over.max(over + height);
            }
        }
        over = level_over;
        under = level_under;
        first = end;
    }
    let mut glyphs = Vec::new();
    let mut text = String::new();
    for (index, (column, base)) in columns.iter().zip(&mut bases).enumerate() {
        align(
            base,
            &column.base,
            widths[index],
            positions[index],
            system,
            depth,
            budget,
        );
        glyphs.append(&mut base.glyphs);
        text.push_str(&column.base.raw_text);
    }
    for note in notes {
        glyphs.extend(note.run.glyphs);
    }
    Some(ShapedRuby {
        paragraph_offset: 0,
        node,
        text,
        width: *positions.last()?,
        base_width,
        overhang_spaces: spaces,
        font_size,
        glyphs,
        line_over: over,
        line_under: under,
        break_after: true,
        space_before: 0.0,
        preserved_space_before: 0.0,
    })
}
