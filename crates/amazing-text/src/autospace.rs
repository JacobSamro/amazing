//! CSS Text 4 §8.4: spacing of authentic visual typographic units.
//! This module never inserts characters or changes source ranges. The caller
//! supplies the real boundary owner's policy and used font metrics after bidi.
use crate::{GlyphOrientation, ShapedLine, ShapedRun};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
#[path = "autospace_unicode.rs"]
mod unicode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AutospaceMode {
    #[default]
    Insert,
    Replace,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextAutospace {
    pub ideograph_alpha: bool,
    pub ideograph_numeric: bool,
    pub punctuation: bool,
    pub mode: AutospaceMode,
}
impl TextAutospace {
    pub const NORMAL: Self = Self {
        ideograph_alpha: true,
        ideograph_numeric: true,
        punctuation: false,
        mode: AutospaceMode::Insert,
    };
    pub const NONE: Self = Self {
        ideograph_alpha: false,
        ideograph_numeric: false,
        punctuation: false,
        mode: AutospaceMode::Insert,
    };
    /// The UA's explicit `auto` choice uses conservative normal inter-script spacing.
    pub const AUTO: Self = Self::NORMAL;
}
impl Default for TextAutospace {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// A real inline boundary owner's resolved facts, not inferred from text style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutospaceContext {
    pub owner: u32,
    pub policy: TextAutospace,
    /// Actual CJK advance measure of the boundary owner's used font.
    pub ic_advance: f32,
    /// Actual shaped U+00A0 and U+202F advances, for French punctuation.
    pub nbsp_advance: f32,
    pub narrow_nbsp_advance: f32,
    pub french: bool,
    /// False for actual intervening margin, border, padding, atomic or IFC edge.
    pub adjoining: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutospaceClass {
    Ideograph,
    Letter,
    Numeral,
    Space,
    Other,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AutospaceAdjustment {
    /// The authentic shaped cluster after which the visual gap is placed.
    pub after_cluster: Range<usize>,
    pub before_cluster: Range<usize>,
    pub owner: u32,
    pub advance: f32,
    pub replaced_spaces: Vec<Range<usize>>,
    pub nonbreaking: bool,
}
#[derive(Debug, PartialEq, Default)]
pub struct AutospacePlan {
    pub adjustments: Vec<AutospaceAdjustment>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutospaceError {
    InvalidCluster,
    MissingCluster,
    InvalidMetric,
    /// A real glyph spans the required spacing/removal boundary.
    NeedsReshape(usize),
}

fn properties(ch: char) -> u16 {
    let cp = ch as u32;
    let i = unicode::PROPERTIES.partition_point(|range| range.1 < cp);
    unicode::PROPERTIES
        .get(i)
        .filter(|range| range.0 <= cp)
        .map_or(0, |range| range.2)
}
/// Unicode 17 CSS classes; upright is the actual shaped orientation, including
/// text-combine-upright, rather than a guess from the Unicode script.
pub fn autospace_class(ch: char, upright_vertical: bool) -> AutospaceClass {
    let p = properties(ch);
    let cp = ch as u32;
    // The insert/replace separator rule takes precedence over any shared Han script extension.
    if p & 16 != 0 {
        return AutospaceClass::Space;
    }
    let ideograph = p & 64 != 0
        || ((0x3041..=0x30ff).contains(&cp) && p & 8 == 0)
        || (0x31c0..=0x31ff).contains(&cp);
    if ideograph {
        AutospaceClass::Ideograph
    } else if p & 3 != 0 && p & (128 | 256) == 0 && !upright_vertical {
        AutospaceClass::Letter
    } else if p & 4 != 0 && p & 256 == 0 && !upright_vertical {
        AutospaceClass::Numeral
    } else if p & 16 != 0 {
        AutospaceClass::Space
    } else {
        AutospaceClass::Other
    }
}
/// Conservative, policy-independent test: could any pair inside `text` ever
/// receive autospace under some policy? Requires an ideograph beside a
/// letter or numeral, or a French punctuation candidate. Callers use it to
/// avoid reshaping candidate lines that cannot change.
pub fn autospace_possible(text: &str) -> bool {
    let (mut ideograph, mut other_script) = (false, false);
    for ch in text.chars() {
        if matches!(ch, ';' | '!' | '?' | ':' | '\u{ab}' | '\u{bb}') { return true; }
        match autospace_class(ch, false) {
            AutospaceClass::Ideograph => ideograph = true,
            AutospaceClass::Letter | AutospaceClass::Numeral => other_script = true,
            _ => {}
        }
        if ideograph && other_script { return true; }
    }
    false
}
#[derive(Debug)]
struct Unit {
    base: char,
    class: AutospaceClass,
    clusters: Vec<Range<usize>>,
    glyph_clusters: Vec<Range<usize>>,
    first: usize,
}
fn base_position(
    text: &str,
    range: &Range<usize>,
    bases: &[(usize, char)],
) -> Option<(usize, char)> {
    let next = bases.partition_point(|(offset, _)| *offset < range.start);
    if let Some(&(offset, ch)) = bases.get(next).filter(|(offset, _)| *offset < range.end) {
        return Some((offset, ch));
    }
    if next > 0 {
        Some(bases[next - 1])
    } else {
        text[range.clone()]
            .chars()
            .next()
            .map(|ch| (range.start, ch))
    }
}
fn units(
    text: &str,
    runs: &[ShapedRun],
    visual: &[Range<usize>],
) -> Result<Vec<Unit>, AutospaceError> {
    let mut by_base: BTreeMap<usize, Unit> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let minimum = visual.iter().map(|r| r.start).min().unwrap_or(0);
    let maximum = visual.iter().map(|r| r.end).max().unwrap_or(0);
    if maximum > text.len() || !text.is_char_boundary(minimum) || !text.is_char_boundary(maximum) {
        return Err(AutospaceError::InvalidCluster);
    }
    let bases: Vec<_> = text[minimum..maximum]
        .char_indices()
        .filter(|(_, ch)| properties(*ch) & (2 | 32) == 0)
        .map(|(i, ch)| (minimum + i, ch))
        .collect();
    let mut glyphs = BTreeMap::new();
    for run in runs {
        for glyph in &run.glyphs {
            glyphs.entry(glyph.cluster.start).or_insert((run, glyph));
        }
    }
    for (index, range) in visual.iter().enumerate() {
        if range.start >= range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
            || !seen.insert((range.start, range.end))
        {
            return Err(AutospaceError::InvalidCluster);
        }
        let Some((base, character)) = base_position(text, range, &bases) else {
            continue;
        };
        let actual = glyphs
            .range(..=range.start)
            .next_back()
            .map(|(_, pair)| pair)
            .filter(|(_, g)| g.cluster.start <= range.start && g.cluster.end >= range.end);
        let Some(actual) = actual else {
            if text[range.clone()]
                .chars()
                .all(|ch| properties(ch) & 32 != 0)
            {
                continue;
            }
            return Err(AutospaceError::MissingCluster);
        };
        let base_glyph = glyphs
            .range(..=base)
            .next_back()
            .map(|(_, pair)| *pair)
            .filter(|(_, glyph)| glyph.cluster.contains(&base))
            .unwrap_or(*actual);
        let upright = base_glyph.0.writing_mode.uses_vertical_metrics()
            && base_glyph.1.orientation == GlyphOrientation::Upright;
        let unit = by_base.entry(base).or_insert_with(|| Unit {
            base: character,
            class: autospace_class(character, upright),
            clusters: Vec::new(),
            glyph_clusters: Vec::new(),
            first: index,
        });
        unit.clusters.push(range.clone());
        if unit.glyph_clusters.last() != Some(&actual.1.cluster) {
            unit.glyph_clusters.push(actual.1.cluster.clone());
        }
        unit.first = unit.first.min(index);
    }
    let mut units: Vec<_> = by_base.into_values().collect();
    units.sort_by_key(|unit| unit.first);
    Ok(units)
}
fn interscript(left: AutospaceClass, right: AutospaceClass, policy: TextAutospace) -> bool {
    use AutospaceClass::*;
    match (left, right) {
        (Ideograph, Letter) | (Letter, Ideograph) => policy.ideograph_alpha,
        (Ideograph, Numeral) | (Numeral, Ideograph) => policy.ideograph_numeric,
        _ => false,
    }
}
/// Policy-independent test: can this pair ever receive spacing under any
/// policy? Lets the planner skip the (expensive) owner/metric resolution for
/// ordinary pairs such as kana-kana or Latin-Latin.
fn may_space(left: char, right: char, lclass: AutospaceClass, rclass: AutospaceClass) -> bool {
    use AutospaceClass::*;
    matches!((lclass, rclass), (Ideograph, Letter | Numeral) | (Letter | Numeral, Ideograph))
        || matches!(right, ';' | '!' | '?' | ':' | '\u{bb}')
        || left == '\u{ab}'
}
fn french_gap(left: char, right: char, context: AutospaceContext) -> Option<f32> {
    if !context.french || !context.policy.punctuation {
        return None;
    }
    if matches!(left, ';' | '!' | '?' | ':') && matches!(right, ';' | '!' | '?' | ':') {
        return None;
    }
    if matches!(right, ';' | '!' | '?') {
        Some(context.narrow_nbsp_advance)
    } else if right == ':' || right == '»' || left == '«' {
        Some(context.nbsp_advance)
    } else {
        None
    }
}
/// Plan a single actual line in display order. `resolve` must return the
/// innermost common boundary owner's policy/metrics and real box adjacency.
/// It receives genuine unit ranges, including ranges in distinct text nodes.
/// Existing spaces are kept unless a qualifying `replace` boundary consumes
/// exclusively U+0020 clusters; every other Unicode separator suppresses it.
pub fn plan_autospace(
    text: &str,
    runs: &[ShapedRun],
    visual_clusters: &[Range<usize>],
    mut resolve: impl FnMut(&Range<usize>, &Range<usize>) -> Option<AutospaceContext>,
) -> Result<AutospacePlan, AutospaceError> {
    let units = units(text, runs, visual_clusters)?;
    let mut result = AutospacePlan::default();
    let mut glyph_by_start = BTreeMap::new();
    for run in runs {
        for glyph in &run.glyphs {
            glyph_by_start.entry(glyph.cluster.start).or_insert(glyph);
        }
    }
    for (left_index, left) in units.iter().enumerate() {
        if left.class == AutospaceClass::Space {
            continue;
        }
        let mut right_index = left_index + 1;
        let mut spaces = Vec::new();
        while let Some(unit) = units.get(right_index) {
            if unit.class != AutospaceClass::Space {
                break;
            }
            if unit.base != ' '
                || unit
                    .clusters
                    .iter()
                    .any(|r| !text[r.clone()].chars().all(|ch| ch == ' '))
            {
                spaces.clear();
                right_index = units.len();
                break;
            }
            spaces.extend(unit.clusters.iter().cloned());
            right_index += 1;
        }
        let Some(right) = units.get(right_index) else {
            continue;
        };
        let right_unit = &right.clusters[0];
        let left_unit = left.clusters.last().unwrap();
        let before = &right.glyph_clusters[0];
        let after = left.glyph_clusters.last().unwrap();
        if !may_space(left.base, right.base, left.class, right.class) {
            continue;
        }
        let Some(context) = resolve(left_unit, right_unit) else {
            continue;
        };
        if !context.adjoining
            || (!spaces.is_empty() && context.policy.mode != AutospaceMode::Replace)
        {
            continue;
        }
        let french = french_gap(left.base, right.base, context);
        let advance = if let Some(advance) = french {
            advance
        } else if interscript(left.class, right.class, context.policy) {
            context.ic_advance * 0.125
        } else {
            continue;
        };
        if !advance.is_finite() || advance < 0.0 {
            return Err(AutospaceError::InvalidMetric);
        }
        if after.start < before.end && before.start < after.end {
            return Err(AutospaceError::NeedsReshape(
                if left_unit.start < right_unit.start {
                    right_unit.start
                } else {
                    left_unit.start
                },
            ));
        }
        let mut actual_spaces = BTreeSet::new();
        for space in &spaces {
            let glyph = glyph_by_start
                .range(..=space.start)
                .next_back()
                .map(|(_, glyph)| *glyph)
                .filter(|glyph| {
                    glyph.cluster.start <= space.start && glyph.cluster.end >= space.end
                })
                .ok_or(AutospaceError::MissingCluster)?;
            if !text[glyph.cluster.clone()].chars().all(|ch| ch == ' ') {
                return Err(AutospaceError::NeedsReshape(space.start));
            }
            actual_spaces.insert((glyph.cluster.start, glyph.cluster.end));
        }
        result.adjustments.push(AutospaceAdjustment {
            after_cluster: after.clone(),
            before_cluster: before.clone(),
            owner: context.owner,
            advance,
            replaced_spaces: actual_spaces
                .into_iter()
                .map(|(start, end)| start..end)
                .collect(),
            nonbreaking: french.is_some(),
        });
    }
    Ok(result)
}
/// Apply a native plan to freshly shaped runs exactly once. Glyphs, clusters,
/// source mappings and text stay intact; only inline advances are changed.
/// Layout retains the plan for boundary-owner paint and nonbreaking decisions.
pub fn apply_autospace_plan(
    text: &str,
    line: &mut ShapedLine,
    plan: AutospacePlan,
) -> Result<Vec<AutospaceAdjustment>, AutospaceError> {
    let mut glyph_indices: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for (r, run) in line.runs.iter().enumerate() {
        for (g, glyph) in run.glyphs.iter().enumerate() {
            glyph_indices
                .entry((glyph.cluster.start, glyph.cluster.end))
                .or_default()
                .push((r, g));
        }
    }
    // Validate every operand before mutation, so an invalid/stale plan cannot
    // leave a partially changed line or accept fabricated cluster locations.
    for change in &plan.adjustments {
        if !change.advance.is_finite() || change.advance < 0.0 {
            return Err(AutospaceError::InvalidMetric);
        }
        for range in std::iter::once(&change.after_cluster)
            .chain(std::iter::once(&change.before_cluster))
            .chain(change.replaced_spaces.iter())
        {
            if range.start < line.text_range.start
                || range.end > line.text_range.end
                || !glyph_indices.contains_key(&(range.start, range.end))
            {
                return Err(AutospaceError::MissingCluster);
            }
        }
        for space in &change.replaced_spaces {
            if !text
                .get(space.clone())
                .is_some_and(|s| s.chars().all(|ch| ch == ' '))
            {
                return Err(AutospaceError::InvalidCluster);
            }
        }
    }
    for change in &plan.adjustments {
        for space in &change.replaced_spaces {
            for &(r, g) in &glyph_indices[&(space.start, space.end)] {
                line.runs[r].glyphs[g].advance_x = 0.0;
                line.runs[r].glyphs[g].advance_y = 0.0;
            }
        }
        let &(run_index, glyph_index) = glyph_indices
            [&(change.after_cluster.start, change.after_cluster.end)]
            .last()
            .unwrap();
        let run = &mut line.runs[run_index];
        let glyph = &mut run.glyphs[glyph_index];
        if run.writing_mode.uses_vertical_metrics()
            && glyph.orientation == GlyphOrientation::Upright
        {
            glyph.advance_y += if glyph.advance_y.is_sign_negative() {
                -change.advance
            } else {
                change.advance
            }
        } else {
            glyph.advance_x += if glyph.advance_x.is_sign_negative() {
                -change.advance
            } else {
                change.advance
            }
        }
    }
    Ok(plan.adjustments)
}

/// Candidate shaping boundaries before optional ligatures are selected. The
/// caller supplies genuine resolved upright orientation (including combine),
/// boundary policy and box adjacency. This does not add breaks to the line-break
/// graph: it prevents a font ligature from swallowing a spacing location.
pub fn autospace_shaping_boundaries(
    text: &str,
    mut upright: impl FnMut(&Range<usize>) -> bool,
    mut resolve: impl FnMut(&Range<usize>, &Range<usize>) -> Option<AutospaceContext>,
) -> Vec<usize> {
    let mut logical: Vec<(Range<usize>, char, AutospaceClass)> = Vec::new();
    for (start, ch) in text.char_indices() {
        let range = start..start + ch.len_utf8();
        if properties(ch) & (2 | 32) != 0 && !logical.is_empty() {
            logical.last_mut().unwrap().0.end = range.end;
            continue;
        }
        let class = autospace_class(ch, upright(&range));
        logical.push((range, ch, class));
    }
    let mut boundaries = BTreeSet::new();
    for (index, (left, lc, lclass)) in logical.iter().enumerate() {
        if *lclass == AutospaceClass::Space {
            continue;
        }
        let mut right_index = index + 1;
        let mut spaces = Vec::new();
        while let Some((range, ch, class)) = logical.get(right_index) {
            if *class != AutospaceClass::Space {
                break;
            }
            if *ch != ' ' {
                right_index = logical.len();
                break;
            }
            spaces.push(range.clone());
            right_index += 1;
        }
        let Some((right, rc, rclass)) = logical.get(right_index) else {
            continue;
        };
        if !may_space(*lc, *rc, *lclass, *rclass) {
            continue;
        }
        let Some(context) = resolve(left, right) else {
            continue;
        };
        if !context.adjoining
            || (!spaces.is_empty() && context.policy.mode != AutospaceMode::Replace)
        {
            continue;
        }
        if interscript(*lclass, *rclass, context.policy) || french_gap(*lc, *rc, context).is_some()
        {
            boundaries.insert(right.start);
            for space in spaces {
                boundaries.insert(space.start);
                boundaries.insert(space.end);
            }
        }
    }
    boundaries.into_iter().collect()
}
#[cfg(test)]
#[path = "autospace_tests.rs"]
mod tests;
