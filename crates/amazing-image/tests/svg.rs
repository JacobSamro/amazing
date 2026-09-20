use amazing_image::{DecodeError, DecodeLimits, decode};
fn svg(attrs: &str, body: &str) -> Vec<u8> {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" {attrs}>{body}</svg>"#)
        .into_bytes()
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    let image = decode(bytes, DecodeLimits::default()).unwrap();
    image.pixels[(y * 8 + x) * 4..][..4].try_into().unwrap()
}
#[test]
fn viewport_alignment_meet_slice_and_none() {
    let body = r#"<rect width="8" height="4" fill="red"/>"#;
    let meet = svg(r#"viewBox="0 0 8 4""#, body);
    assert_eq!(pixel(&meet, 4, 0), [0; 4]);
    assert_eq!(pixel(&meet, 4, 2), [255, 0, 0, 255]);
    for mode in ["none", "xMidYMid slice"] {
        let bytes = svg(
            &format!(r#"viewBox="0 0 8 4" preserveAspectRatio="{mode}""#),
            body,
        );
        assert_eq!(pixel(&bytes, 4, 0), [255, 0, 0, 255]);
    }
    let offset = svg(
        r#"viewBox="4 2 8 8""#,
        r#"<rect x="4" y="2" width="1" height="1" fill="blue"/>"#,
    );
    assert_eq!(pixel(&offset, 0, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&offset, 1, 0), [0; 4]);
}
#[test]
fn shapes_inherit_fill_and_resolve_currentcolor_after_child_color() {
    let bytes = svg(
        "",
        r#"<g fill="currentColor" color="red"><rect color="blue" width="2" height="2"/><circle cx="4" cy="4" r="2"/><ellipse cx="6" cy="1" rx="2" ry="1" fill="lime"/></g>"#,
    );
    assert_eq!(pixel(&bytes, 0, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&bytes, 4, 4), [255, 0, 0, 255]);
    assert_eq!(pixel(&bytes, 6, 0), [0, 255, 0, 255]);
    assert_eq!(pixel(&bytes, 7, 7), [0; 4]);
}
#[test]
fn shape_alpha_composites_in_document_order() {
    let bytes = svg(
        "",
        r#"<rect width="8" height="8" fill="red"/><rect width="4" height="4" fill="blue" opacity="0.5"/>"#,
    );
    assert_eq!(pixel(&bytes, 0, 0), [127, 0, 128, 255]);
    assert_eq!(pixel(&bytes, 5, 5), [255, 0, 0, 255]);
}
#[test]
fn unsupported_svg_fails_whole_image_without_partial_success() {
    for body in [
        r#"<rect width="8" height="8"/><script/>"#,
        r#"<path d="M0 0"/>"#,
        r#"<rect width="8" height="8" stroke="red"/>"#,
        r#"<g opacity="0.5"><rect width="1" height="1"/></g>"#,
        r#"<image href="https://example.com/image.png"/>"#,
    ] {
        assert_eq!(
            decode(&svg("", body), Default::default()).unwrap_err(),
            DecodeError::UnsupportedSvg
        );
    }
    assert!(decode(&svg("", r#"<rect><rect/></rect>"#), Default::default()).is_err());
    assert!(
        decode(
            &svg("", r#"<rect width="1" width="2"/>"#),
            Default::default()
        )
        .is_err()
    );
    assert!(decode(b"<svg>", Default::default()).is_err());
}
#[test]
fn svg_obeys_input_geometry_output_and_work_budgets() {
    let bytes = svg("", r#"<rect width="8" height="8"/>"#);
    for limits in [
        DecodeLimits {
            max_input_bytes: 1,
            ..Default::default()
        },
        DecodeLimits {
            max_dimension: 7,
            ..Default::default()
        },
        DecodeLimits {
            max_rgba_bytes: 255,
            ..Default::default()
        },
        DecodeLimits {
            max_decoder_bytes: 255,
            ..Default::default()
        },
    ] {
        assert!(decode(&bytes, limits).is_err());
    }
    let many = svg("", &"<rect/>".repeat(1025));
    assert_eq!(
        decode(&many, Default::default()).unwrap_err(),
        DecodeError::AllocationLimit
    );
}
