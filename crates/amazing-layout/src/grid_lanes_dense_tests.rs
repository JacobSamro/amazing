//! Dense packing controls use genuine solved areas and the document pipeline.
use super::*;
use crate::Flow;

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
}
fn items(css: &str, extra: &str) -> Vec<LayoutBox> {
    let root = crate::layout_document(
        "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div><div id=f></div></body>",
        &format!(
            "body{{display:grid-lanes;margin:0;width:150px;flow-tolerance:0;grid-auto-flow:dense;grid-template-columns:50px 50px 50px;{css}}}#a{{grid-column:1;height:20px}}#b{{grid-column:2;height:60px}}#c{{grid-column:3;height:60px}}#d{{grid-column:span 2;height:15px}}#e{{height:10px}}#f{{height:50px}}{extra}"
        ),
        150.0,
        &FontSource::fallback(),
    );
    let Flow::Grid(items) = root.flow else {
        panic!("real lanes missing")
    };
    items
}

#[test]
fn dense_column_backfill_preserves_normal_cursor_and_frontier() {
    let v = items("", "");
    close(v[3].border_box.y, 60.0);
    close(v[4].border_box.x, 0.0);
    close(v[4].border_box.y, 20.0);
    close(v[5].border_box.x, 100.0);
    close(v[5].border_box.y, 60.0);
}
#[test]
fn sparse_column_control_keeps_existing_forward_placement() {
    // With a 20px fifth item, C advances to80 while A/B remain75: the
    // normal sixth item chooses A. Retain the original four assertions.
    let v = items("grid-auto-flow:row;", "#e{height:20px}");
    close(v[4].border_box.x, 100.0);
    close(v[4].border_box.y, 60.0);
    close(v[5].border_box.x, 0.0);
    close(v[5].border_box.y, 75.0);
    // Preserve the original 10px fixture too: its real frontier is [75,75,70],
    // so the smallest running position remains C, regardless of the cursor.
    let original = items("grid-auto-flow:row;", "");
    close(original[4].border_box.x, 100.0);
    close(original[4].border_box.y, 60.0);
    close(original[5].border_box.x, 100.0);
    close(original[5].border_box.y, 70.0);
}
#[test]
fn dense_column_gutters_surround_each_backfilled_margin_box() {
    let v = items("row-gap:5px;", "#f{height:15px}");
    close(v[4].border_box.y, 25.0);
    close(v[5].border_box.y, 40.0);
}
#[test]
fn dense_signed_margins_use_outer_extent_not_visual_border_height() {
    for (margins, expected) in [
        ("margin-top:3px;margin-bottom:2px", 23.0),
        ("margin-top:-4px;margin-bottom:1px", 16.0),
    ] {
        let v = items("", &format!("#e{{{margins}}}"));
        close(v[4].border_box.y, expected);
    }
}
#[test]
fn dense_definite_grid_axis_cannot_escape_author_track() {
    for (track, y) in [(1, 20.0), (2, 75.0), (3, 60.0)] {
        let v = items("", &format!("#e{{grid-column:{track}}}"));
        close(v[4].border_box.x, (track - 1) as f32 * 50.0);
        close(v[4].border_box.y, y);
    }
}
#[test]
fn dense_requires_equal_actual_track_extent() {
    let v = items("grid-template-columns:40px 50px 60px;", "");
    close(v[4].border_box.x, 90.0);
    close(v[4].border_box.y, 60.0);
}
#[test]
fn dense_equal_nonadjacent_track_sizes_allow_reuse() {
    let v = items("grid-template-columns:50px 40px 50px;width:140px;", "");
    close(v[4].border_box.x, 0.0);
    close(v[4].border_box.y, 20.0);
}
#[test]
fn dense_rtl_tie_uses_logical_track_start() {
    let v = items("direction:rtl;", "");
    close(v[4].border_box.x, 100.0);
    close(v[4].border_box.y, 20.0);
}
#[test]
fn dense_fill_reverse_mirrors_real_final_margin_placement() {
    let ordinary = items("height:150px;", "");
    let reversed = items("height:150px;grid-lanes-direction:column fill-reverse;", "");
    for (a, b) in ordinary.iter().zip(&reversed) {
        close(b.border_box.x, a.border_box.x);
        close(b.border_box.y, 150.0 - a.border_box.y - a.border_box.h);
    }
}
#[test]
fn dense_relative_visual_shift_does_not_change_hole_accounting() {
    let v = items("", "#e{position:relative;top:100px}#f{height:10px}");
    close(v[4].border_box.y, 120.0);
    close(v[5].border_box.y, 30.0);
}
#[test]
fn dense_order_modified_items_create_holes_in_actual_layout_order() {
    let ordinary = items("", "");
    let reordered = items(
        "",
        "#a{order:1}#b{order:2}#c{order:3}#d{order:4}#e{order:5}#f{order:6}",
    );
    for (a, b) in ordinary.iter().zip(&reordered) {
        assert_eq!(a.border_box, b.border_box);
    }
}
#[test]
fn dense_row_backfill_preserves_frontier_and_axis_identity() {
    for gap in [0.0, 5.0] {
        let root = crate::layout_document(
            "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div><div id=f></div></body>",
            &format!(
                "body{{display:grid-lanes;margin:0;width:150px;flow-tolerance:0;grid-auto-flow:dense;grid-template-rows:50px 50px 50px;column-gap:{gap}px}}#a{{grid-row:1;width:20px}}#b{{grid-row:2;width:60px}}#c{{grid-row:3;width:60px}}#d{{grid-row:span 2;width:15px}}#e{{width:10px}}#f{{width:50px}}"
            ),
            150.0,
            &FontSource::fallback(),
        );
        let Flow::Grid(v) = root.flow else {
            panic!("row lanes missing")
        };
        close(v[4].border_box.x, 20.0 + gap);
        close(v[4].border_box.y, 0.0);
        close(v[5].border_box.x, 60.0 + gap);
        close(v[5].border_box.y, 100.0);
    }
}
#[test]
fn dense_subgrid_remaps_authentic_tracks_and_descendants() {
    let root = crate::layout_document(
        "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div><section><div></div></section></body>",
        "body{display:grid-lanes;margin:0;width:150px;flow-tolerance:0;grid-auto-flow:dense;grid-template-columns:[one]50px[two]50px[three]50px}#a{grid-column:1;height:20px}#b{grid-column:2;height:60px}#c{grid-column:3;height:60px}#d{grid-column:span 2;height:15px}section{display:grid;grid-template-columns:subgrid;height:10px}section>div{height:5px}",
        150.0,
        &FontSource::fallback(),
    );
    let Flow::Grid(v) = root.flow else {
        panic!("lanes missing")
    };
    close(v[4].border_box.x, 0.0);
    close(v[4].border_box.y, 20.0);
    let Flow::Grid(children) = &v[4].flow else {
        panic!("subgrid missing")
    };
    close(children[0].border_box.x, 0.0);
    close(children[0].border_box.y, 20.0);
    close(children[0].border_box.w, 50.0);
    assert!(
        v[4].grid_subgrid
            .as_ref()
            .unwrap()
            .cols
            .as_ref()
            .unwrap()
            .names
            .contains_key("one")
    );
}

#[test]
fn holes_require_common_free_space_across_entire_span() {
    let mut s = lanes_packing::Spaces::new(3);
    s.skipped(&[0.0, 20.0, 40.0], 0, 3, 80.0, 0.0).unwrap();
    assert_eq!(
        s.find(2, 30.0, 80.0, 0.0, None, |_| true),
        Some(Some((0, 20.0)))
    );
    s.occupy(0, 2, 20.0, 30.0, 0.0).unwrap();
    assert_eq!(
        s.find(3, 20.0, 80.0, 0.0, None, |_| true),
        Some(Some((0, 50.0)))
    );
}
#[test]
fn tolerance_ties_choose_startmost_not_cursor_or_exact_highest() {
    for tolerance in [0.0, 4.0, 5.0, f32::INFINITY] {
        let mut s = lanes_packing::Spaces::new(4);
        s.skipped(&[5.0, 5.0, 0.0, 0.0], 0, 4, 60.0, 0.0).unwrap();
        let expected = if tolerance < 5.0 { (2, 0.0) } else { (0, 5.0) };
        assert_eq!(
            s.find(2, 20.0, 60.0, tolerance, None, |_| true),
            Some(Some(expected))
        );
    }
}
#[test]
fn incompatible_or_locked_holes_do_not_become_guessed_area() {
    let mut s = lanes_packing::Spaces::new(3);
    s.skipped(&[0.0, 20.0, 40.0], 0, 3, 60.0, 0.0).unwrap();
    assert_eq!(s.find(1, 30.0, 60.0, 0.0, Some(2), |_| true), Some(None));
    assert_eq!(
        s.find(1, 30.0, 60.0, 0.0, None, |i| i == 1),
        Some(Some((1, 20.0)))
    );
}
#[test]
fn repeated_backfill_splits_real_holes_and_preserves_gap() {
    let mut s = lanes_packing::Spaces::new(1);
    s.skipped(&[10.0], 0, 1, 100.0, 5.0).unwrap();
    s.occupy(0, 1, 30.0, 20.0, 5.0).unwrap();
    assert_eq!(
        s.find(1, 15.0, 100.0, 0.0, None, |_| true),
        Some(Some((0, 10.0)))
    );
    assert_eq!(
        s.find(1, 20.0, 100.0, 0.0, None, |_| true),
        Some(Some((0, 55.0)))
    );
}
#[test]
fn candidate_measurement_uses_each_authentic_inherited_context() {
    let mut s = lanes_packing::Spaces::new(2);
    s.skipped(&[0.0, 10.0], 0, 2, 40.0, 0.0).unwrap();
    let mut seen = Vec::new();
    let got = s.find_measured(
        1,
        40.0,
        0.0,
        None,
        |_| true,
        |i| {
            seen.push(i);
            Some(if i == 0 { 50.0 } else { 20.0 })
        },
    );
    assert_eq!(seen, vec![0, 1]);
    assert_eq!(got, Some(Some((1, 10.0))));
}
#[test]
fn limit_or_invalid_measurement_is_explicit_not_sparse_success() {
    let mut s = lanes_packing::Spaces::new(1);
    assert_eq!(s.find(1, f32::NAN, 10.0, 0.0, None, |_| true), None);
    assert_eq!(
        s.find_measured(1, 10.0, 0.0, None, |_| true, |_| None),
        None
    );
    assert_eq!(s.find(0, 1.0, 10.0, 0.0, None, |_| true), None);
}
#[test]
fn area_extent_is_actual_span_and_rtl_safe() {
    assert_eq!(
        lanes_packing::extent(&[110.0, 50.0, 0.0], &[40.0, 50.0, 40.0], 0, 2),
        Some((50.0, 100.0))
    );
    assert_eq!(lanes_packing::extent(&[0.0], &[f32::INFINITY], 0, 1), None);
    assert_eq!(lanes_packing::extent(&[0.0], &[10.0], 0, 2), None);
}

#[test]
fn actual_dense_tolerance_tie_preserves_startmost_span() {
    for (tolerance, x, y) in [(0, 150.0, 0.0), (4, 150.0, 0.0), (5, 0.0, 5.0)] {
        let root = crate::layout_document(
            "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div></body>",
            &format!(
                "body{{display:grid-lanes;margin:0;width:300px;grid-template-columns:repeat(6,50px);grid-auto-flow:dense;flow-tolerance:{tolerance}px}}#a{{grid-column:1/span 2;height:5px}}#b{{grid-column:3;height:60px}}#c{{grid-column:6;height:60px}}#d{{grid-column:span 6;height:20px}}#e{{grid-column:span 2;height:20px}}"
            ),
            300.0,
            &FontSource::fallback(),
        );
        let Flow::Grid(v) = root.flow else {
            panic!("lanes missing")
        };
        close(v[4].border_box.x, x);
        close(v[4].border_box.y, y);
    }
}
#[test]
fn row_definite_area_and_fill_reverse_preserve_dense_identity() {
    for (track, x) in [(1, 20.0), (2, 75.0), (3, 60.0)] {
        let root = crate::layout_document(
            "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div></body>",
            &format!(
                "body{{display:grid-lanes;margin:0;width:150px;grid-template-rows:50px 50px 50px;grid-auto-flow:dense;flow-tolerance:0;grid-lanes-direction:row fill-reverse}}#a{{grid-row:1;width:20px}}#b{{grid-row:2;width:60px}}#c{{grid-row:3;width:60px}}#d{{grid-row:span 2;width:15px}}#e{{grid-row:{track};width:10px}}"
            ),
            150.0,
            &FontSource::fallback(),
        );
        let Flow::Grid(v) = root.flow else {
            panic!("row lanes missing")
        };
        close(v[4].border_box.x, 150.0 - x - 10.0);
        close(v[4].border_box.y, (track - 1) as f32 * 50.0);
    }
}
#[test]
fn equal_total_subgrid_extent_rebuilds_different_individual_tracks() {
    let root = crate::layout_document(
        "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div><div id=e></div><div id=f></div><section><div></div><div></div></section></body>",
        "body{display:grid-lanes;margin:0;width:200px;grid-template-columns:30px 70px 70px 30px;grid-auto-flow:dense;flow-tolerance:0}#a{grid-column:1;height:20px}#b{grid-column:2;height:20px}#c{grid-column:3;height:60px}#d{grid-column:4;height:60px}#e{grid-column:span 4;height:15px}#f{grid-column:2;height:100px}section{display:grid;grid-column:span 2;grid-template-columns:subgrid;height:10px}section>div{height:5px}",
        200.0,
        &FontSource::fallback(),
    );
    let Flow::Grid(v) = root.flow else {
        panic!("lanes missing")
    };
    close(v[6].border_box.x, 0.0);
    close(v[6].border_box.y, 20.0);
    let Flow::Grid(children) = &v[6].flow else {
        panic!("subgrid missing")
    };
    close(children[0].border_box.w, 30.0);
    close(children[1].border_box.w, 70.0);
    close(children[1].border_box.x, 30.0);
}
#[test]
fn dense_exact_hole_boundary_and_zero_outer_do_not_expand_frontier() {
    let mut s = lanes_packing::Spaces::new(1);
    s.skipped(&[10.0], 0, 1, 40.0, 5.0).unwrap();
    assert_eq!(
        s.find(1, 25.0, 40.0, 0.0, None, |_| true),
        Some(Some((0, 10.0)))
    );
    assert_eq!(s.find(1, 25.01, 40.0, 0.0, None, |_| true), Some(None));
    assert_eq!(
        s.find(1, -20.0, 40.0, 0.0, None, |_| true),
        Some(Some((0, 10.0)))
    );
}
#[test]
fn dense_disjoint_intersections_advance_to_next_genuine_hole() {
    let mut s = lanes_packing::Spaces::new(2);
    s.skipped(&[0.0, 10.0], 0, 2, 100.0, 0.0).unwrap();
    s.occupy(0, 1, 20.0, 40.0, 0.0).unwrap();
    s.occupy(1, 1, 10.0, 20.0, 0.0).unwrap();
    assert_eq!(
        s.find(2, 20.0, 100.0, 0.0, None, |_| true),
        Some(Some((0, 60.0)))
    );
}

#[test]
fn adjoining_line_blocks_preserve_actual_tracks_names_and_repetition() {
    let ctx = RepeatCtx { font_px:16.0,content_w:150.0,zoom:1.0,
        repeat_available:Some(150.0),gap:0.0 };
    let value = "[one]50px[two]50px[three]50px";
    assert_eq!(parse_track_list(value,&ctx,true),vec![TrackSize::Px(50.0);3]);
    let names = collect_line_names(value,&ctx,true);
    for (name,line) in [("one",1),("two",2),("three",3)] {
        assert_eq!(names[name],vec![line]);
    }
    let value = "repeat(2,[a]10px[b])";
    assert_eq!(parse_track_list(value,&ctx,true),vec![TrackSize::Px(10.0);2]);
    let names = collect_line_names(value,&ctx,true);
    assert_eq!(names["a"],vec![1,2]);
    assert_eq!(names["b"],vec![2,3]);
    assert_eq!(auto_fit_tracks("[start]repeat(auto-fit,[slot]50px)[end]",&ctx),vec![true;3]);
}
#[test]
fn adjoining_blocks_and_functions_keep_the_original_line_identity() {
    let ctx = RepeatCtx { font_px:16.0,content_w:150.0,zoom:1.0,
        repeat_available:Some(150.0),gap:0.0 };
    let names = collect_line_names("[a][b]10px[c][d]",&ctx,true);
    for name in ["a","b"] { assert_eq!(names[name],vec![1]); }
    for name in ["c","d"] { assert_eq!(names[name],vec![2]); }
    assert_eq!(top_tokens("[before]minmax(10px,20px)[after]repeat(2,[inside]5px)"),
        ["[before]","minmax(10px,20px)","[after]","repeat(2,[inside]5px)"].map(str::to_owned));
    assert!(contains_nested_repeat("repeat(2,[slot]repeat(3,1px))"));
    assert!(parse_track_list("repeat(2,[slot]repeat(3,1px))",&ctx,true).is_empty());
}
