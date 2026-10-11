use super::*;
use crate::{
    Direction, FallbackTextSystem, LineRequest, Paragraph, ShapingBoundary, TextOrientation,
    TextSpan, TextStyle, TextSystem, WritingMode,
};
fn line(text: &str, style: TextStyle, range: Range<usize>) -> ShapedLine {
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 7,
        style,
        boundary_before: ShapingBoundary::Hard,
    }];
    FallbackTextSystem::new()
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            },
            range,
        })
        .unwrap()
}
fn context(policy: TextAutospace) -> AutospaceContext {
    let system = FallbackTextSystem::new();
    let style = TextStyle::default();
    let metric = |text: &str| sum(&line(text, style.clone(), 0..text.len()));
    AutospaceContext {
        owner: 42,
        policy,
        ic_advance: system.font_relative_metrics(&style).ic_advance,
        nbsp_advance: metric("\u{a0}"),
        narrow_nbsp_advance: metric("\u{202f}"),
        french: false,
        adjoining: true,
    }
}
fn sum(line: &ShapedLine) -> f32 {
    line.runs
        .iter()
        .flat_map(|r| &r.glyphs)
        .map(|g| g.advance_x.abs() + g.advance_y.abs())
        .sum()
}
fn plan(text: &str, line: &ShapedLine, context: AutospaceContext) -> AutospacePlan {
    plan_autospace(text, &line.runs, &line.visual_clusters, |_, _| {
        Some(context)
    })
    .unwrap()
}
#[test]
fn unicode17_full_css_character_classes_and_supplementary_script_extensions() {
    for ch in [
        '日',
        'あ',
        'カ',
        'ー',
        '\u{31c0}',
        '\u{31ff}',
        '\u{20000}',
        '\u{30000}',
        '\u{323b0}',
        '\u{00b7}',
        '\u{3001}',
        '\u{30fb}',
    ] {
        assert_eq!(
            autospace_class(ch, false),
            AutospaceClass::Ideograph,
            "{ch:?}"
        );
    }
    for ch in ['A', 'é', 'א', 'م', '\u{0301}', '\u{10400}'] {
        assert_eq!(autospace_class(ch, false), AutospaceClass::Letter)
    }
    for ch in ['0', '٣', '५', '\u{1d7ce}'] {
        assert_eq!(autospace_class(ch, false), AutospaceClass::Numeral)
    }
    for ch in ['Ａ', '가', '!', '😀'] {
        assert_eq!(autospace_class(ch, false), AutospaceClass::Other)
    }
    assert_eq!(autospace_class('１', false), AutospaceClass::Other);
    for ch in ['A', '0', 'א'] {
        assert_eq!(autospace_class(ch, true), AutospaceClass::Other)
    }
    assert_eq!(autospace_class('日', true), AutospaceClass::Ideograph);
}
#[test]
fn all_flag_combinations_choose_alpha_numeric_independently_without_punctuation_guesses() {
    let text = "日A日1日!日";
    let shaped = line(text, TextStyle::default(), 0..text.len());
    for bits in 0..8 {
        let policy = TextAutospace {
            ideograph_alpha: bits & 1 != 0,
            ideograph_numeric: bits & 2 != 0,
            punctuation: bits & 4 != 0,
            mode: AutospaceMode::Insert,
        };
        assert_eq!(
            plan(text, &shaped, context(policy)).adjustments.len(),
            2 * usize::from(bits & 1 != 0) + 2 * usize::from(bits & 2 != 0)
        );
    }
    assert_eq!(
        plan(text, &shaped, context(TextAutospace::AUTO))
            .adjustments
            .len(),
        4
    );
    assert!(
        plan(text, &shaped, context(TextAutospace::NONE))
            .adjustments
            .is_empty()
    );
}
#[test]
fn actual_boundary_owner_metric_and_policy_override_both_leaf_fonts() {
    let text = "日A";
    let mut shaped = line(text, TextStyle::default(), 0..text.len());
    let original = shaped.clone();
    let mut ctx = context(TextAutospace::NORMAL);
    ctx.owner = 91;
    let parent = TextStyle {
        size: 32.0,
        ..TextStyle::default()
    };
    ctx.ic_advance = FallbackTextSystem::new()
        .font_relative_metrics(&parent)
        .ic_advance;
    let receipts = apply_autospace_plan(text, &mut shaped, plan(text, &original, ctx)).unwrap();
    assert_eq!(sum(&shaped) - sum(&original), 4.0);
    assert_eq!(receipts[0].owner, 91);
    assert_eq!(shaped.visual_clusters, original.visual_clusters);
    assert_eq!(shaped.runs[0].source, original.runs[0].source);
}
#[test]
fn combining_marks_and_invisible_controls_do_not_create_extra_units_or_lose_base_script() {
    for text in [
        "A\u{0301}日",
        "日\u{0301}A",
        "A\u{200b}日",
        "日\u{fe0f}A",
        "A\u{0301}\u{0302}日",
    ] {
        let shaped = line(text, TextStyle::default(), 0..text.len());
        assert_eq!(
            plan(text, &shaped, context(TextAutospace::NORMAL))
                .adjustments
                .len(),
            1,
            "{text:?}"
        );
    }
    let text = "\u{0301}日";
    let shaped = line(text, TextStyle::default(), 0..text.len());
    assert_eq!(
        plan(text, &shaped, context(TextAutospace::NORMAL))
            .adjustments
            .len(),
        1
    );
}
#[test]
fn insert_suppresses_every_unicode_separator_tabs_punctuation_and_atomic_characters() {
    for separator in [
        " ", "  ", "\u{a0}", "\u{202f}", "\u{3000}", "\t", "\n", "\u{fffc}", "\"", "(",
    ] {
        let text = format!("日{separator}A");
        let shaped = line(&text, TextStyle::default(), 0..text.len());
        assert!(
            plan(&text, &shaped, context(TextAutospace::NORMAL))
                .adjustments
                .is_empty(),
            "{separator:?}"
        );
    }
}
#[test]
fn replace_consumes_only_real_ascii_space_clusters_and_retains_their_source_identity() {
    let text = "日  A";
    let mut shaped = line(text, TextStyle::default(), 0..text.len());
    let old = shaped.clone();
    let mut ctx = context(TextAutospace::NORMAL);
    ctx.policy.mode = AutospaceMode::Replace;
    let native = plan(text, &shaped, ctx);
    assert_eq!(native.adjustments.len(), 1);
    assert_eq!(native.adjustments[0].replaced_spaces.len(), 2);
    let old_space: f32 = old
        .runs
        .iter()
        .flat_map(|r| &r.glyphs)
        .filter(|g| text[g.cluster.clone()].starts_with(' '))
        .map(|g| g.advance_x)
        .sum();
    apply_autospace_plan(text, &mut shaped, native).unwrap();
    assert!((sum(&shaped) - (sum(&old) - old_space + 2.0)).abs() < 0.001);
    assert_eq!(shaped.visual_clusters, old.visual_clusters);
    for separator in [" \u{a0}", "\u{202f}", "\u{3000}"] {
        let text = format!("日{separator}A");
        let shaped = line(&text, TextStyle::default(), 0..text.len());
        assert!(plan(&text, &shaped, ctx).adjustments.is_empty());
    }
}
#[test]
fn post_bidi_visual_adjacency_not_original_logical_adjacency_selects_spacing() {
    // The native fallback's actual RTL glyph/cluster order supplies the visual
    // sequence. A punctuation edge moved next to Han must suppress alpha there.
    let text = "日אב♥";
    let shaped = line(
        text,
        TextStyle {
            direction: Direction::RightToLeft,
            ..TextStyle::default()
        },
        0..text.len(),
    );
    let native = plan(text, &shaped, context(TextAutospace::NORMAL));
    assert_eq!(native.adjustments.len(), 1);
    assert_eq!(&text[native.adjustments[0].after_cluster.clone()], "א");
    assert_eq!(&text[native.adjustments[0].before_cluster.clone()], "日");
}
#[test]
fn line_edges_and_actual_ifc_box_edge_guards_never_create_external_spacing() {
    let text = "日A";
    let shaped = line(text, TextStyle::default(), 0..text.len());
    let mut ctx = context(TextAutospace::NORMAL);
    ctx.adjoining = false;
    assert!(plan(text, &shaped, ctx).adjustments.is_empty());
    assert!(
        plan_autospace(text, &shaped.runs, &shaped.visual_clusters, |_, _| None)
            .unwrap()
            .adjustments
            .is_empty()
    );
    for range in [0..3, 3..4] {
        let shaped = line(text, TextStyle::default(), range);
        assert!(
            plan(text, &shaped, context(TextAutospace::NORMAL))
                .adjustments
                .is_empty()
        );
    }
}
#[test]
fn real_upright_vertical_orientation_excludes_letters_and_numerals_but_sideways_includes() {
    let text = "日A1日";
    let upright = line(
        text,
        TextStyle {
            writing_mode: WritingMode::VerticalRl,
            text_orientation: TextOrientation::Upright,
            ..TextStyle::default()
        },
        0..text.len(),
    );
    assert!(
        plan(text, &upright, context(TextAutospace::NORMAL))
            .adjustments
            .is_empty()
    );
    let mut rotated = line(
        text,
        TextStyle {
            writing_mode: WritingMode::VerticalRl,
            text_orientation: TextOrientation::Sideways,
            ..TextStyle::default()
        },
        0..text.len(),
    );
    let old = sum(&rotated);
    let native = plan(text, &rotated, context(TextAutospace::NORMAL));
    assert_eq!(native.adjustments.len(), 2);
    apply_autospace_plan(text, &mut rotated, native).unwrap();
    assert!((sum(&rotated) - old - 4.0).abs() < 0.001);
}
#[test]
fn french_punctuation_uses_actual_nbsp_metrics_and_returns_nonbreaking_boundary_receipts() {
    let mut ctx = context(TextAutospace {
        punctuation: true,
        ..TextAutospace::NONE
    });
    ctx.french = true;
    for (text, narrow) in [
        ("a;", true),
        ("a!", true),
        ("a?", true),
        ("a:", false),
        ("«a", false),
        ("a»", false),
    ] {
        let shaped = line(text, TextStyle::default(), 0..text.len());
        let native = plan(text, &shaped, ctx);
        assert_eq!(native.adjustments.len(), 1, "{text}");
        assert!(native.adjustments[0].nonbreaking);
        assert_eq!(
            native.adjustments[0].advance,
            if narrow {
                ctx.narrow_nbsp_advance
            } else {
                ctx.nbsp_advance
            }
        );
    }
    let text = "a!?";
    let shaped = line(text, TextStyle::default(), 0..text.len());
    assert_eq!(plan(text, &shaped, ctx).adjustments.len(), 1);
    ctx.french = false;
    assert!(plan(text, &shaped, ctx).adjustments.is_empty());
}
#[test]
fn french_insert_and_replace_do_not_duplicate_existing_real_nbsp_nnbsp_or_ascii_spaces() {
    let mut ctx = context(TextAutospace {
        punctuation: true,
        ..TextAutospace::NONE
    });
    ctx.french = true;
    for text in ["a !", "a\u{a0}!", "a\u{202f}!", "« a"] {
        let shaped = line(text, TextStyle::default(), 0..text.len());
        assert!(plan(text, &shaped, ctx).adjustments.is_empty());
    }
    ctx.policy.mode = AutospaceMode::Replace;
    for text in ["a !", "« a"] {
        let shaped = line(text, TextStyle::default(), 0..text.len());
        assert_eq!(plan(text, &shaped, ctx).adjustments.len(), 1);
    }
    for text in ["a\u{a0}!", "a\u{202f}!"] {
        let shaped = line(text, TextStyle::default(), 0..text.len());
        assert!(plan(text, &shaped, ctx).adjustments.is_empty());
    }
}
#[test]
fn invalid_or_missing_cluster_and_bad_metrics_reject_before_any_line_mutation() {
    let text = "日A";
    let mut shaped = line(text, TextStyle::default(), 0..text.len());
    let old = shaped.clone();
    let mut native = plan(text, &shaped, context(TextAutospace::NORMAL));
    native.adjustments[0].before_cluster = 2..4;
    assert_eq!(
        apply_autospace_plan(text, &mut shaped, native),
        Err(AutospaceError::MissingCluster)
    );
    assert_eq!(shaped, old);
    assert_eq!(
        plan_autospace(text, &shaped.runs, &[0..2, 3..4], |_, _| Some(context(
            TextAutospace::NORMAL
        ))),
        Err(AutospaceError::InvalidCluster)
    );
    let mut ctx = context(TextAutospace::NORMAL);
    ctx.ic_advance = f32::NAN;
    assert_eq!(
        plan_autospace(text, &shaped.runs, &shaped.visual_clusters, |_, _| Some(
            ctx
        )),
        Err(AutospaceError::InvalidMetric)
    );
}
#[test]
fn candidate_shaping_boundaries_protect_spacing_from_optional_ligatures_without_breaking_marks() {
    let text = "A\u{0301}日1";
    assert_eq!(
        autospace_shaping_boundaries(text, |_| false, |_, _| Some(context(TextAutospace::NORMAL))),
        vec![3, 6]
    );
    assert!(
        autospace_shaping_boundaries(text, |_| false, |_, _| Some(context(TextAutospace::NONE)))
            .is_empty()
    );
    let text = "日A";
    assert!(
        autospace_shaping_boundaries(text, |_| true, |_, _| Some(context(TextAutospace::NORMAL)))
            .is_empty()
    );
}
#[test]
fn long_native_paragraph_and_boundary_owner_variation_keep_linear_sized_cluster_receipts() {
    let text = "日A".repeat(4096);
    let shaped = line(&text, TextStyle::default(), 0..text.len());
    let mut count = 0;
    let native = plan_autospace(&text, &shaped.runs, &shaped.visual_clusters, |_, _| {
        count += 1;
        Some(context(TextAutospace::NORMAL))
    })
    .unwrap();
    assert_eq!(count, 8191);
    assert_eq!(native.adjustments.len(), 8191);
}

#[test]
fn provider_cluster_contract_keeps_multigraph_ligatures_and_requires_real_reshaping_for_internal_gaps()
 {
    // Declarative provider-neutral cluster fixtures, not a claim that the fallback
    // font contains GSUB ligatures. Installed WPT exercises actual font shaping.
    let text = "AB日";
    let mut shaped = line(text, TextStyle::default(), 0..text.len());
    let mut glyph = shaped.runs[0].glyphs[0].clone();
    glyph.cluster = 0..2;
    glyph.advance_x += shaped.runs[1].glyphs[0].advance_x;
    shaped.runs[0].glyphs = vec![glyph];
    shaped.runs[0].text_range = 0..2;
    shaped.runs.remove(1);
    let old = sum(&shaped);
    let native = plan(text, &shaped, context(TextAutospace::NORMAL));
    assert_eq!(native.adjustments.len(), 1);
    assert_eq!(native.adjustments[0].after_cluster, 0..2);
    apply_autospace_plan(text, &mut shaped, native).unwrap();
    assert!((sum(&shaped) - old - 2.0).abs() < 0.001);
    let text = "日A";
    let mut shaped = line(text, TextStyle::default(), 0..text.len());
    shaped.runs[0].glyphs[0].cluster = 0..4;
    shaped.runs[0].text_range = 0..4;
    shaped.runs.remove(1);
    assert_eq!(
        plan_autospace(text, &shaped.runs, &shaped.visual_clusters, |_, _| Some(
            context(TextAutospace::NORMAL)
        )),
        Err(AutospaceError::NeedsReshape(3))
    );
    assert_eq!(
        autospace_shaping_boundaries(text, |_| false, |_, _| Some(context(TextAutospace::NORMAL))),
        vec![3]
    );
    let text = "A\u{200b}日";
    let mut shaped = line(text, TextStyle::default(), 0..text.len());
    shaped.runs.remove(1);
    assert_eq!(
        plan(text, &shaped, context(TextAutospace::NORMAL))
            .adjustments
            .len(),
        1
    );
    let mut ctx = context(TextAutospace::NORMAL);
    ctx.policy.mode = AutospaceMode::Replace;
    assert_eq!(
        autospace_shaping_boundaries("日 A", |_| false, |_, _| Some(ctx)),
        vec![3, 4]
    );
}

#[test]
fn policy_cache_identity_does_not_split_unchanged_glyph_shaping_but_real_font_features_do() {
    let original = TextStyle::default();
    let mut different = original.clone();
    different.autospace = TextAutospace::NONE;
    assert_ne!(original, different);
    assert!(original.shaping_equivalent(&different));
    different.weight = 700;
    assert!(!original.shaping_equivalent(&different));
    different = original.clone();
    different.direction = Direction::RightToLeft;
    assert!(!original.shaping_equivalent(&different));
    different = original.clone();
    different.features.push(crate::FeatureSetting {
        tag: *b"liga",
        value: 0,
    });
    assert!(!original.shaping_equivalent(&different));
}

#[test]
fn signed_native_glyph_advances_expand_in_the_real_inline_direction_instead_of_canceling() {
    // Native glyph-position fixtures cover the signed coordinates returned by
    // HarfRust (particularly negative vertical advance), not new font metrics.
    let text = "日A";
    for writing_mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        for negative in [false, true] {
            let mut shaped = line(
                text,
                TextStyle {
                    writing_mode,
                    ..TextStyle::default()
                },
                0..text.len(),
            );
            if negative {
                for glyph in shaped.runs.iter_mut().flat_map(|run| &mut run.glyphs) {
                    glyph.advance_x = -glyph.advance_x;
                    glyph.advance_y = -glyph.advance_y;
                }
            }
            let old = sum(&shaped);
            let native = plan(text, &shaped, context(TextAutospace::NORMAL));
            assert_eq!(native.adjustments.len(), 1);
            apply_autospace_plan(text, &mut shaped, native).unwrap();
            assert!((sum(&shaped) - old - 2.0).abs() < 0.001);
        }
    }
}
