//! Automatic-repeat counting and its shared expansion consumers.
use super::*;

fn ctx(space: Option<f32>, gap: f32) -> RepeatCtx {
    RepeatCtx { font_px: 16.0, content_w: space.unwrap_or(0.0), zoom: 1.0,
        repeat_available: space, gap }
}
fn counts(value: &str, space: f32, gap: f32) -> Vec<usize> {
    repeat_counts(&top_tokens(value), &ctx(Some(space), gap), true)
}

#[test]
fn fixed_zero_tracks_consume_counting_floor_without_changing_sizes() {
    let context = ctx(Some(3.0), 0.0);
    let tracks = parse_track_list("0px repeat(auto-fill, 1px)", &context, true);
    assert_eq!(tracks.len(), 3);
    assert!(matches!(tracks[0], TrackSize::Px(0.0)));
}
#[test]
fn all_fixed_repetitions_participate_in_the_floor() {
    assert_eq!(counts("repeat(3, 0px) repeat(auto-fill, 1px)", 8.0, 0.0), vec![3,5]);
    assert_eq!(counts("repeat(3, 0px 0px) repeat(auto-fit, 1px)", 8.0, 0.0), vec![3,2]);
}
#[test]
fn unresolved_fixed_siblings_require_one_repetition() {
    for fixed in ["auto", "min-content", "max-content", "1fr", "minmax(auto, 1fr)"] {
        assert_eq!(counts(&format!("{fixed} repeat(auto-fill, 10px)"), 100.0, 0.0), vec![1,1]);
        assert_eq!(counts(&format!("repeat(2, {fixed}) repeat(auto-fit, 10px)"), 100.0, 0.0), vec![2,1]);
    }
}
#[test]
fn definite_minmax_uses_maximum_floored_by_minimum() {
    for (track, expected) in [("minmax(30px,10px)",3), ("minmax(auto,25px)",4),
        ("minmax(20px,1fr)",5), ("minmax(0px,auto)",100)] {
        assert_eq!(counts(&format!("repeat(auto-fill, {track})"), 100.0, 0.0), vec![expected]);
    }
}
#[test]
fn multi_track_fragment_accounts_for_internal_and_external_gaps() {
    assert_eq!(counts("10px repeat(auto-fill, 20px 30px) 10px", 170.0, 5.0), vec![1,2,1]);
    assert_eq!(counts("10px repeat(auto-fill, 20px 30px) 10px", 204.0, 5.0), vec![1,2,1]);
    assert_eq!(counts("10px repeat(auto-fill, 20px 30px) 10px", 205.0, 5.0), vec![1,3,1]);
}
#[test]
fn underflow_still_has_one_fragment() {
    for width in [0.0, 1.0, 10.0, 19.0] {
        assert_eq!(counts("100px repeat(auto-fill, 20px 30px)", width, 10.0), vec![1,1]);
    }
}
#[test]
fn indefinite_axis_falls_back_to_one() {
    let context = ctx(None, 0.0);
    assert_eq!(repeat_counts(&top_tokens("repeat(auto-fit, 10px)"), &context, true), vec![1]);
}
#[test]
fn fractional_track_floor_and_gap_boundaries_are_stable() {
    for (space, expected) in [(2.9,1), (3.0,2), (4.9,3), (5.0,4)] {
        assert_eq!(counts("0px repeat(auto-fill, 0.2px)", space, 0.0), vec![1,expected]);
    }
}
#[test]
fn line_names_share_the_counting_floor() {
    let context = ctx(Some(5.0), 0.0);
    let names = collect_line_names("[begin] 0px repeat(auto-fill, [slot] 1px [tail]) [end]", &context, true);
    assert_eq!(names["slot"], vec![2,3,4,5]);
    assert_eq!(names["tail"], vec![3,4,5,6]);
    assert_eq!(names["end"], vec![6]);
}
#[test]
fn auto_fit_mask_covers_only_repeated_tracks() {
    let context = ctx(Some(5.0), 0.0);
    assert_eq!(auto_fit_tracks("0px repeat(auto-fit, 1px)", &context), vec![false,true,true,true,true]);
    assert_eq!(auto_fit_tracks("0px repeat(auto-fill, 1px)", &context), vec![false;5]);
}
#[test]
fn collapsed_holes_keep_explicit_lines_and_zero_sizes() {
    let mask = auto_fit_tracks("0px repeat(auto-fit, 1px)", &ctx(Some(5.0),0.0));
    let axis = ActiveAxis::new(5, &mask, [(2,1)].into_iter());
    assert_eq!(axis.prefix, vec![0,1,1,2,2,2]);
    assert_eq!(axis.size(&[TrackSize::Px(0.0),TrackSize::Px(1.0),TrackSize::Px(1.0),
        TrackSize::Px(1.0),TrackSize::Px(1.0)], 0.0, Some(5.0), &[]), vec![0.0,0.0,1.0,0.0,0.0]);
}
#[test]
fn cap_is_shared_by_masks_names_and_tracks() {
    let context = ctx(Some(1e9),0.0);
    let value = "repeat(auto-fit, [slot] 0px 0px)";
    assert_eq!(parse_track_list(value, &context,true).len(),MAX_GRID_TRACKS);
    assert_eq!(auto_fit_tracks(value,&context).len(),MAX_GRID_TRACKS);
    assert_eq!(collect_line_names(value,&context,true)["slot"].len(),MAX_GRID_TRACKS / 2);
    let names = collect_line_names("repeat(1000, 1px) [edge] 1px [omitted]", &context,true);
    assert_eq!(names["edge"],vec![1001]);
    assert!(!names.contains_key("omitted"));
}
#[test]
fn nonfinite_counting_unit_does_not_generate_a_large_grid() {
    let context = ctx(Some(100.0), f32::INFINITY);
    assert_eq!(repeat_counts(&top_tokens("repeat(auto-fill, 1px)"), &context,true),vec![1]);
}

#[test]
fn counting_floor_scales_with_zoom_without_changing_zero_tracks() {
    for zoom in [0.25, 0.5, 1.0, 2.0, 4.0] {
        let mut context = ctx(Some(8.0 * zoom), 0.0);
        context.zoom = zoom;
        context.content_w = 8.0 * zoom;
        let value = "repeat(3, 0px) repeat(auto-fit, 1px)";
        assert_eq!(repeat_counts(&top_tokens(value), &context,true),vec![3,5]);
        let tracks = parse_track_list(value,&context,true);
        assert_eq!(tracks.len(),8);
        assert!(tracks[..3].iter().all(|t|matches!(t,TrackSize::Px(0.0))));
        assert_eq!(auto_fit_tracks(value,&context),vec![false,false,false,true,true,true,true,true]);
    }
}
#[test]
fn zoomed_fractional_fragments_and_gaps_use_one_coordinate_system() {
    for zoom in [0.5, 1.0, 2.0, 4.0] {
        let mut context = ctx(Some(9.0 * zoom), zoom);
        context.zoom = zoom;
        context.content_w = 9.0 * zoom;
        assert_eq!(repeat_counts(&top_tokens("0px repeat(auto-fill, 0.1px 0.2px)"), &context,true),vec![1,2]);
    }
}
