//! CSS Fonts 4 section 6 synthesis fallback for `font-variant-caps`'
//! `small-caps`/`all-small-caps` and `font-variant-position`'s `super`/`sub`,
//! for a selected face whose GSUB table has no `smcp`/`c2sc`/`sups`/`subs`
//! feature. Sized like the existing synthetic bold/oblique path: shaping
//! decides which glyphs need synthesis (and, for small-caps on an originally
//! lowercase letter, which glyph id to substitute); rasterization applies a
//! uniform outline scale. The exact scale and baseline-shift constants below
//! are approximations, the same way the existing bold stroke width and -14
//! degree oblique skew are; CSS does not mandate exact synthesis metrics.

use amazing_text::GlyphId;
use harfrust::{Feature, FontRef as HarfFontRef, Tag};
use read_fonts::TableProvider;

/// Outline scale for a synthesized small-caps glyph (either a substituted
/// uppercase letter for `smcp`, or an already-uppercase letter for `c2sc`).
pub(super) const SMALL_CAPS_SCALE: f32 = 0.72;
/// Outline scale for a synthesized superscript/subscript glyph.
pub(super) const POSITION_SCALE: f32 = 0.66;
/// Baseline raise for synthesized superscript, in units of the used font
/// size (applied as a shape-time `offset_y`, so raster only needs the scale).
const SUPERSCRIPT_RAISE_EM: f32 = 0.33;
/// Baseline lowering for synthesized subscript, in the same units.
const SUBSCRIPT_LOWER_EM: f32 = 0.14;

/// Which of the four synthesizable features a face's GSUB table actually
/// registers. A tag present anywhere in the table counts as supported; this
/// does not narrow to the run's specific script, the same coarse-grained
/// level the rest of this adapter's feature handling already uses.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FaceFeatureSupport {
    pub smcp: bool,
    pub c2sc: bool,
    pub sups: bool,
    pub subs: bool,
}

impl FaceFeatureSupport {
    pub(super) fn probe(face: &HarfFontRef<'_>) -> Self {
        let mut support = Self::default();
        if let Ok(gsub) = face.gsub()
            && let Ok(list) = gsub.feature_list()
        {
            let smcp = Tag::new(b"smcp");
            let c2sc = Tag::new(b"c2sc");
            let sups = Tag::new(b"sups");
            let subs = Tag::new(b"subs");
            for record in list.feature_records() {
                let tag = record.feature_tag();
                support.smcp |= tag == smcp;
                support.c2sc |= tag == c2sc;
                support.sups |= tag == sups;
                support.subs |= tag == subs;
            }
        }
        support
    }
}

/// Whether `tag` is present in the merged feature list with a nonzero value,
/// using the last matching entry the way HarfBuzz treats overlapping
/// full-buffer feature ranges (author descriptor features are pushed first,
/// explicit `TextStyle.features` after, so the explicit one wins).
pub(super) fn feature_on(features: &[Feature], tag: [u8; 4]) -> bool {
    let tag = Tag::new(&tag);
    features
        .iter()
        .filter(|feature| feature.tag == tag)
        .last()
        .is_some_and(|feature| feature.value != 0)
}

/// Per-cluster synthesis decided while shaping.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct ClusterSynthesis {
    /// Substitute glyph id for a synthesized `smcp` lowercase letter (its
    /// face-mapped uppercase counterpart). `None` keeps the shaped id.
    pub glyph_id: Option<GlyphId>,
    pub small_caps: bool,
    pub position: bool,
    /// Baseline shift in units of the used font size; zero when `position`
    /// is false.
    pub offset_em: f32,
}

/// Decides small-caps/position synthesis for one cluster. `needs_*` are
/// already the full gate (policy allows it, the feature was requested, and
/// the face does not support it) computed once per shaping call, not once
/// per cluster. `lookup_uppercase` maps a character to the selected face's
/// glyph id for that character's uppercase form, or `None` if the face has
/// no such glyph (in which case `smcp` synthesis is skipped, leaving the
/// original lowercase glyph rather than guessing).
pub(super) fn for_cluster(
    needs_smcp: bool,
    needs_c2sc: bool,
    needs_sups: bool,
    needs_subs: bool,
    lookup_uppercase: &dyn Fn(char) -> Option<GlyphId>,
    representative: char,
) -> ClusterSynthesis {
    let mut plan = ClusterSynthesis::default();
    if needs_smcp && representative.is_lowercase() {
        let mut upper = representative.to_uppercase();
        let first = upper.next();
        if let Some(mapped) = first
            && upper.next().is_none()
            && let Some(glyph) = lookup_uppercase(mapped)
        {
            plan.glyph_id = Some(glyph);
            plan.small_caps = true;
        }
    } else if needs_c2sc && representative.is_uppercase() {
        plan.small_caps = true;
    }
    if needs_sups {
        plan.position = true;
        plan.offset_em = SUPERSCRIPT_RAISE_EM;
    } else if needs_subs {
        plan.position = true;
        plan.offset_em = -SUBSCRIPT_LOWER_EM;
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_uppercase(_: char) -> Option<GlyphId> {
        None
    }

    #[test]
    fn lowercase_letter_substitutes_the_mapped_uppercase_glyph() {
        let lookup = |c: char| (c == 'A').then_some(GlyphId(42));
        let plan = for_cluster(true, false, false, false, &lookup, 'a');
        assert_eq!(
            plan,
            ClusterSynthesis {
                glyph_id: Some(GlyphId(42)),
                small_caps: true,
                position: false,
                offset_em: 0.0,
            }
        );
    }

    #[test]
    fn lowercase_letter_with_no_uppercase_glyph_keeps_the_original_id() {
        let plan = for_cluster(true, false, false, false, &no_uppercase, 'a');
        assert_eq!(plan, ClusterSynthesis::default());
    }

    #[test]
    fn smcp_does_not_apply_to_an_already_uppercase_letter() {
        let lookup = |c: char| (c == 'A').then_some(GlyphId(42));
        let plan = for_cluster(true, false, false, false, &lookup, 'A');
        assert_eq!(plan, ClusterSynthesis::default());
    }

    #[test]
    fn c2sc_scales_an_uppercase_letter_without_substitution() {
        let plan = for_cluster(false, true, false, false, &no_uppercase, 'A');
        assert_eq!(
            plan,
            ClusterSynthesis {
                glyph_id: None,
                small_caps: true,
                position: false,
                offset_em: 0.0,
            }
        );
    }

    #[test]
    fn c2sc_does_not_apply_to_a_lowercase_letter() {
        let plan = for_cluster(false, true, false, false, &no_uppercase, 'a');
        assert_eq!(plan, ClusterSynthesis::default());
    }

    #[test]
    fn digits_and_punctuation_are_untouched_by_caps_synthesis() {
        for representative in ['3', '.', ' ', '\u{4e2d}'] {
            let lookup = |c: char| (c == 'A').then_some(GlyphId(1));
            assert_eq!(
                for_cluster(true, true, false, false, &lookup, representative),
                ClusterSynthesis::default(),
                "unexpected synthesis for {representative:?}"
            );
        }
    }

    #[test]
    fn superscript_raises_the_baseline_and_flags_position_scale() {
        let plan = for_cluster(false, false, true, false, &no_uppercase, 'x');
        assert_eq!(
            plan,
            ClusterSynthesis {
                glyph_id: None,
                small_caps: false,
                position: true,
                offset_em: SUPERSCRIPT_RAISE_EM,
            }
        );
        assert!(plan.offset_em > 0.0);
    }

    #[test]
    fn subscript_lowers_the_baseline_and_flags_position_scale() {
        let plan = for_cluster(false, false, false, true, &no_uppercase, 'x');
        assert_eq!(
            plan,
            ClusterSynthesis {
                glyph_id: None,
                small_caps: false,
                position: true,
                offset_em: -SUBSCRIPT_LOWER_EM,
            }
        );
        assert!(plan.offset_em < 0.0);
    }

    #[test]
    fn superscript_takes_precedence_when_both_are_somehow_requested() {
        // font-variant-position is single-valued in CSS (super and sub are
        // mutually exclusive keywords), so both being "needed" at once should
        // not happen; this pins a defined, non-panicking choice if it does.
        let plan = for_cluster(false, false, true, true, &no_uppercase, 'x');
        assert_eq!(plan.offset_em, SUPERSCRIPT_RAISE_EM);
    }

    #[test]
    fn small_caps_and_position_can_combine_on_the_same_cluster() {
        // e.g. small-caps numerals inside a superscript context.
        let plan = for_cluster(false, true, true, false, &no_uppercase, 'A');
        assert!(plan.small_caps);
        assert!(plan.position);
        assert_eq!(plan.offset_em, SUPERSCRIPT_RAISE_EM);
    }

    #[test]
    fn nothing_needed_produces_the_default_no_op_plan() {
        assert_eq!(
            for_cluster(false, false, false, false, &no_uppercase, 'a'),
            ClusterSynthesis::default()
        );
    }
}
