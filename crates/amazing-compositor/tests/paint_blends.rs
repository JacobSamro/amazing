#[path = "support/paint_blends.rs"]
mod corpus;
#[test]
fn painter_blend_isolation_directory_and_cached_frames() {
    let (_, _, unexpected) = corpus::run(false);
    assert_eq!(unexpected, 0, "unclassified painter/compositor differences");
}
