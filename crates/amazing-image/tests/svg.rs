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
fn byte_svg_strokes_rect_and_line_without_filling_the_line() {
    let rect = svg("", r#"<rect x="1" y="1" width="6" height="6" fill="none" stroke="red" stroke-width="2"/>"#);
    assert_eq!(pixel(&rect, 1, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&rect, 4, 4), [0; 4]);
    let line = svg("", r#"<line x1="0" y1="4" x2="8" y2="4" stroke="green" stroke-width="2"/>"#);
    assert_eq!(pixel(&line, 4, 4), [0, 128, 0, 255]);
    assert_eq!(pixel(&line, 4, 0), [0; 4]);
    let view_only = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 50 50"><line x1="0" y1="25" x2="50" y2="25" stroke-width="50" stroke="green" vector-effect="non-scaling-stroke"/></svg>"#;
    let decoded = decode(view_only, DecodeLimits::default()).unwrap();
    assert_eq!((decoded.width, decoded.height), (50, 50));
    assert!(decoded.pixels.chunks_exact(4).all(|pixel| pixel == [0, 128, 0, 255]));
}

#[test]
fn inherited_byte_polyline_stroke_survives_non_scaling_viewbox() {
    let source = br#"<svg xmlns='http://www.w3.org/2000/svg' stroke-width='1px' stroke='black' width='50' height='50' viewBox='0 0 50 50'><polyline vector-effect='non-scaling-stroke' points='50 0 0 50'/></svg>"#;
    let decoded = decode(source, DecodeLimits::default()).unwrap();
    assert_eq!((decoded.width, decoded.height), (50, 50));
    assert!(decoded.pixels.chunks_exact(4).any(|pixel| pixel == [0, 0, 0, 255]));
}
#[test]
fn unsupported_svg_fails_whole_image_without_partial_success() {
    for body in [
        r#"<rect width="8" height="8"/><script/>"#,
        r#"<path d="M0 0"/>"#,
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

#[test]
fn transform_lists_and_nested_groups_apply_in_svg_order() {
    let bytes = svg(
        "",
        r#"<g transform="translate(2, 1)"><rect width="2" height="2" fill="red" transform="scale(2)"/></g>"#,
    );
    assert_eq!(pixel(&bytes, 2, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&bytes, 5, 4), [255, 0, 0, 255]);
    assert_eq!(pixel(&bytes, 1, 1), [0; 4]);
    let equivalent = svg(
        "",
        r#"<rect width="2" height="2" fill="red" transform="translate(2 1) scale(2)"/>"#,
    );
    assert_eq!(
        decode(&bytes, Default::default()).unwrap().pixels,
        decode(&equivalent, Default::default()).unwrap().pixels
    );
}
#[test]
fn rotated_and_reflected_shapes_use_inverse_sampling() {
    let bytes = svg(
        "",
        r#"<rect x="1" y="1" width="2" height="3" fill="blue" transform="rotate(90 4 4)"/>"#,
    );
    assert_eq!(pixel(&bytes, 4, 1), [0, 0, 255, 255]);
    assert_eq!(pixel(&bytes, 6, 2), [0, 0, 255, 255]);
    assert_eq!(pixel(&bytes, 3, 1), [0; 4]);
    let reflected = svg(
        "",
        r#"<rect x="1" y="1" width="2" height="3" fill="red" transform="matrix(-1 0 0 1 8 0)"/>"#,
    );
    assert_eq!(pixel(&reflected, 5, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&reflected, 1, 1), [0; 4]);
}
#[test]
fn skew_and_nonuniform_scale_transform_curved_geometry() {
    let bytes = svg(
        "",
        r#"<circle cx="2" cy="2" r="1" fill="lime" transform="scale(2 1)"/>"#,
    );
    assert_eq!(pixel(&bytes, 2, 1), [0, 255, 0, 255]);
    assert_eq!(pixel(&bytes, 2, 2), [0, 255, 0, 255]);
    assert_eq!(pixel(&bytes, 1, 1), [0; 4]);
    let skew = svg(
        "",
        r#"<rect width="2" height="2" fill="red" transform="skewX(45)"/>"#,
    );
    assert_eq!(pixel(&skew, 2, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&skew, 0, 1), [0; 4]);
    let skew_y = svg(
        "",
        r#"<rect width="2" height="2" fill="red" transform="skewY(45)"/>"#,
    );
    assert_eq!(pixel(&skew_y, 1, 2), [255, 0, 0, 255]);
}
#[test]
fn singular_transform_hides_only_its_subtree() {
    let bytes = svg(
        "",
        r#"<g transform="scale(0)"><rect width="8" height="8" fill="red"/></g><rect width="1" height="1" fill="blue"/>"#,
    );
    assert_eq!(pixel(&bytes, 0, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&bytes, 4, 4), [0; 4]);
}
#[test]
fn transform_errors_and_limits_fail_the_entire_image() {
    for value in [
        "translate(1,)",
        "scale(NaN)",
        "matrix(1 2)",
        "translate(1px)",
        "rotate(2 3)",
        "scale(1e309)",
        "matrix(1e308 0 0 1e308 0 0)",
        "translate(1),",
        "scale(1,,2)",
    ] {
        let bytes = svg(
            "",
            &format!(r#"<rect width="8" height="8"/><rect transform="{value}"/>"#),
        );
        assert!(decode(&bytes, Default::default()).is_err(), "{value}");
    }
    let list = "scale(1) ".repeat(33);
    assert_eq!(
        decode(
            &svg("", &format!(r#"<g transform="{list}"/>"#)),
            Default::default()
        )
        .unwrap_err(),
        DecodeError::AllocationLimit
    );
    assert_eq!(
        decode(&svg(r#"transform="scale(2)""#, ""), Default::default()).unwrap_err(),
        DecodeError::UnsupportedSvg
    );
}

#[test]
fn inline_declarations_override_presentation_and_keep_inheritance() {
    let bytes = svg(
        "",
        r#"<g color="red"><rect width="2" height="2" fill="lime" style="fill: currentColor"/><rect x="2" width="2" height="2" fill="red" style="fill: blue; fill: lime !important; fill: red"/><rect x="4" width="2" height="2" style="color: blue; fill: currentColor"/><rect x="6" width="2" height="2" style="fill: none"/></g>"#,
    );
    assert_eq!(pixel(&bytes, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&bytes, 2, 0), [0, 255, 0, 255]);
    assert_eq!(pixel(&bytes, 4, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&bytes, 6, 0), [0; 4]);
}

#[test]
fn inline_fill_opacity_and_unsupported_css_fail_closed() {
    let bytes = svg("", r#"<rect width="8" height="8" style="fill: red; fill-opacity: .5"/>"#);
    assert_eq!(pixel(&bytes, 0, 0), [255, 0, 0, 128]);
    for body in [
        r#"<rect width="8" height="8" style="fill: url(#paint)"/>"#,
        r#"<rect width="8" height="8" style="fill: red; filter: blur(1px)"/>"#,
        r#"<g style="opacity: .5"><rect width="8" height="8"/></g>"#,
    ] {
        assert!(matches!(decode(&svg("", body), Default::default()), Err(DecodeError::UnsupportedSvg)), "{body}");
    }
    assert!(decode(&svg("", r#"<rect width="8" height="8" style="fill: lab(50% 0 0)"/>"#), Default::default()).is_ok());
    // A missing custom property invalidates the declaration; CSS then uses
    // fill's inherited/initial value instead of reporting unsupported paint.
    let missing = svg("", r#"<rect width="8" height="8" style="fill: var(--color)"/>"#);
    assert_eq!(pixel(&missing, 0, 0), [0, 0, 0, 255]);
}


#[test]
fn computed_svg_style_handles_wide_keywords_and_inherited_variables() {
    let inherited = svg("", r#"<g fill="red"><rect width="2" height="2" style="fill: inherit"/><rect x="2" width="2" height="2" style="fill: initial"/><rect x="4" width="2" height="2" style="fill: unset"/></g>"#);
    assert_eq!(pixel(&inherited, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&inherited, 2, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&inherited, 4, 0), [255, 0, 0, 255]);

    let variable = svg(r#"style="--ink: blue""#, r#"<g style="fill: var(--ink)"><rect width="4" height="4"/><rect x="4" width="4" height="4" fill="red" style="fill: var(--ink)"/></g>"#);
    assert_eq!(pixel(&variable, 0, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&variable, 4, 0), [0, 0, 255, 255]);
}

#[test]
fn computed_svg_style_resolves_alpha_and_rejects_unsupported_paint() {
    let bytes = svg(r#"style="--alpha: .5""#, r#"<rect width="8" height="8" fill="red" style="fill-opacity: var(--alpha); opacity: initial"/>"#);
    assert_eq!(pixel(&bytes, 0, 0), [255, 0, 0, 128]);
    for body in [
        r#"<rect width="8" height="8" style="fill: url(#paint)"/>"#,
        r#"<rect width="8" height="8" style="color: lab(50% 0 0); fill: currentColor"/>"#,
        r#"<g style="opacity: .5"><rect width="8" height="8"/></g>"#,
    ] {
        assert_eq!(decode(&svg("", body), Default::default()).unwrap_err(), DecodeError::UnsupportedSvg);
    }
}

#[test]
fn closed_paths_fill_relative_concave_and_transformed_outlines() {
    let absolute = svg("", r#"<path d="M1 1 L6 1 H6 V6 L1 6 Z" fill="red"/>"#);
    assert_eq!(pixel(&absolute, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&absolute, 0, 2), [0; 4]);
    let relative = svg("", r#"<path d="m1,1 5,0 v5 h-5 z" style="fill: blue"/>"#);
    assert_eq!(pixel(&relative, 2, 2), [0, 0, 255, 255]);
    let concave = svg("", r#"<path d="M1 1 H7 V3 H3 V7 H1 Z" fill="lime"/>"#);
    assert_eq!(pixel(&concave, 2, 5), [0, 255, 0, 255]);
    assert_eq!(pixel(&concave, 5, 5), [0; 4]);
    let transformed = svg("", r#"<g transform="translate(1 1)"><path d="M1 1 H4 V4 H1 Z" fill="red" fill-opacity=".5"/></g>"#);
    assert_eq!(pixel(&transformed, 2, 2), [255, 0, 0, 128]);
    assert_eq!(pixel(&transformed, 1, 1), [0; 4]);
}

#[test]
fn same_direction_double_wound_path_uses_default_nonzero_fill() {
    let bytes = svg(
        "",
        r#"<path d="M1 1 H6 V6 H1 V1 H6 V6 H1 Z" fill="red"/>"#,
    );
    assert_eq!(pixel(&bytes, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&bytes, 0, 2), [0; 4]);
}

#[test]
fn open_straight_line_paths_fill_with_implicit_closure() {
    let triangle = svg("", r#"<path d="M1 1 L6 1 L6 6" fill="blue"/>"#);
    assert_eq!(pixel(&triangle, 4, 2), [0, 0, 255, 255]);
    assert_eq!(pixel(&triangle, 2, 5), [0; 4]);

    let concave = svg("", r#"<path d="M1 1 H7 V3 H3 V7 H1" fill="lime"/>"#);
    assert_eq!(pixel(&concave, 2, 5), [0, 255, 0, 255]);
    assert_eq!(pixel(&concave, 5, 5), [0; 4]);

    let transformed = svg("", r#"<g transform="translate(1 1)"><path d="m1 1 3 0 v3 h-3" style="fill: blue; fill-opacity: .5"/></g>"#);
    assert_eq!(pixel(&transformed, 2, 2), [0, 0, 255, 128]);
    assert_eq!(pixel(&transformed, 1, 1), [0; 4]);
}

#[test]
fn open_and_explicitly_closed_paths_have_identical_fill() {
    let open = svg("", r#"<path d="M1 1 H6 V6 H1" fill="red"/>"#);
    let closed = svg("", r#"<path d="M1 1 H6 V6 H1 Z" fill="red"/>"#);
    assert_eq!(decode(&open, Default::default()).unwrap().pixels,
               decode(&closed, Default::default()).unwrap().pixels);
    assert_eq!(pixel(&open, 2, 2), [255, 0, 0, 255]);
}

#[test]
fn open_paths_reject_invalid_commands_and_unsupported_paint_whole_image() {
    for path in [
        r#"<path d="M1 1 L6 1"/>"#,
        r#"<path d="M1 1 L6 1 L6"/>"#,
        r#"<path d="M1 1 L6 1 M6 6"/>"#,
        r#"<path d="M1 1 L6 1 Q4 4 6 6"/>"#,
        r#"<path d="M1 1 L6 1 A2 2 0 0 0 6 6"/>"#,
        r#"<path d="M1 1 L6, L6 6"/>"#,
        r#"<path d="M1 1 L6 1 L6 6 Z L1 6"/>"#,
        r#"<path d="M1 1 L6 1 L6 6" fill-rule="nonzero"/>"#,
        r#"<path d="M1 1 L6 1 L6 6" href="other.svg"/>"#,
        r#"<path d="M1 1 L6 1 L6 6" fill="url(#paint)"/>"#,
    ] {
        let body = format!(r#"<rect width="8" height="8" fill="red"/>{path}"#);
        assert_eq!(decode(&svg("", &body), Default::default()).unwrap_err(),
                   DecodeError::UnsupportedSvg, "{path}");
    }
}

#[test]
fn open_paths_keep_attribute_and_raster_work_limits() {
    let at_limit = format!("M0 0{}", " 0 0".repeat(1023));
    assert!(decode(&svg("", &format!(r#"<path d="{at_limit}"/>"#)), Default::default()).is_ok());

    let over_attribute = format!("M0 0{}", " 0 0".repeat(1024));
    assert_eq!(decode(&svg("", &format!(r#"<path d="{over_attribute}"/>"#)), Default::default()).unwrap_err(),
               DecodeError::AllocationLimit);

    let costly = format!("M0 0 {}", "0 0 ".repeat(256));
    let large = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><path d="{costly}"/></svg>"#);
    assert_eq!(decode(large.as_bytes(), Default::default()).unwrap_err(), DecodeError::AllocationLimit);
}

#[test]
fn paths_reject_curved_malformed_and_unsupported_paint_whole_image() {
    for path in [
        r#"<path d="L1 1 L6 1 L6 6 Z"/>"#,
        r#"<path d="M1 1 L6 1 M2 2 L6 6 Z"/>"#,
        r#"<path d="M1 1 L6 1 Q4 4 6 6 Z"/>"#,
        r#"<path d="M1 1 L6 1 A2 2 0 0 0 6 6 Z"/>"#,
        r#"<path d="M1 1 L6, L6 6 Z"/>"#,
        r#"<path d="M1 1 L6 1 L6 6 Z,"/>"#,
        r#"<path d="M1 1 L6 1 L6 6 Z" fill-rule="evenodd"/>"#,
        r#"<path d="M1 1 L6 1 L6 6 Z" fill="url(#paint)"/>"#,
        r#"<path d="M1 1 L6 1 L6 6 Z" href="other.svg"/>"#,
    ] {
        let body = format!(r#"<rect width="8" height="8" fill="red"/>{path}"#);
        assert_eq!(decode(&svg("", &body), Default::default()).unwrap_err(), DecodeError::UnsupportedSvg, "{path}");
    }
    let oversized = format!("M0 0 {}Z", "1,1 ".repeat(1025));
    assert_eq!(
        decode(&svg("", &format!(r#"<path d="{oversized}"/>"#)), Default::default()).unwrap_err(),
        DecodeError::AllocationLimit
    );
}

#[test]
fn polygons_fill_simple_concave_transformed_and_double_wound_outlines() {
    let simple = svg("", r#"<polygon points="1,1 6,1 6,6 1,6" fill="red"/>"#);
    assert_eq!(pixel(&simple, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&simple, 0, 2), [0; 4]);
    let adjacent_negative = svg("", r#"<polygon points="1-1 6-1 6,6 1,6" fill="red"/>"#);
    assert_eq!(pixel(&adjacent_negative, 2, 2), [255, 0, 0, 255]);

    let concave = svg("", r#"<polygon points="1 1,7 1,7 3,3 3,3 7,1 7" fill="lime"/>"#);
    assert_eq!(pixel(&concave, 2, 5), [0, 255, 0, 255]);
    assert_eq!(pixel(&concave, 5, 5), [0; 4]);

    let transformed = svg("", r#"<g transform="translate(1 1)"><polygon points="1,1 4,1 4,4 1,4" style="fill: blue; fill-opacity: .5"/></g>"#);
    assert_eq!(pixel(&transformed, 2, 2), [0, 0, 255, 128]);
    assert_eq!(pixel(&transformed, 1, 1), [0; 4]);

    let double = svg("", r#"<polygon points="1,1 6,1 6,6 1,6 1,1 6,1 6,6 1,6" fill="red"/>"#);
    assert_eq!(pixel(&double, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&double, 0, 2), [0; 4]);
}

#[test]
fn polygons_reject_malformed_points_and_unsupported_features_whole_image() {
    for polygon in [
        r#"<polygon/>"#,
        r#"<polygon points=""/>"#,
        r#"<polygon points="0,0 1,0"/>"#,
        r#"<polygon points="0,0 1,0 1"/>"#,
        r#"<polygon points=",0 1,0 1,1"/>"#,
        r#"<polygon points="0,0 1,,0 1,1"/>"#,
        r#"<polygon points="0,0 1,0 1,1,"/>"#,
        r#"<polygon points="0,0 1,0 1px,1"/>"#,
        r#"<polygon points="0,0 1e309,0 1,1"/>"#,
        r#"<polygon points="0,0 1e,0 1,1"/>"#,
        r#"<polygon points="0,0-1,0 1,1"/>"#,
        r#"<polygon points="0,0 1,0 1,1" fill-rule="evenodd"/>"#,
        r#"<polygon points="0,0 1,0 1,1" fill-rule="nonzero"/>"#,
        r#"<polygon points="0,0 1,0 1,1" fill="url(#paint)"/>"#,
    ] {
        let body = format!(r#"<rect width="8" height="8" fill="red"/>{polygon}"#);
        assert_eq!(
            decode(&svg("", &body), Default::default()).unwrap_err(),
            DecodeError::UnsupportedSvg,
            "{polygon}"
        );
    }
    let oversized = format!("{}", "0 0 ".repeat(513));
    assert_eq!(
        decode(&svg("", &format!(r#"<polygon points="{oversized}"/>"#)), Default::default()).unwrap_err(),
        DecodeError::AllocationLimit
    );
    let costly = "0 0 ".repeat(257);
    let large = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><polygon points="{costly}"/></svg>"#);
    assert_eq!(decode(large.as_bytes(), Default::default()).unwrap_err(), DecodeError::AllocationLimit);
}

#[test]
fn polylines_fill_open_simple_concave_and_transformed_outlines() {
    let simple = svg("", r#"<polyline points="1,1 6,1 6,6 1,6" fill="red"/>"#);
    assert_eq!(pixel(&simple, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&simple, 0, 2), [0; 4]);

    let concave = svg("", r#"<polyline points="1 1,7 1,7 3,3 3,3 7,1 7" fill="lime"/>"#);
    assert_eq!(pixel(&concave, 2, 5), [0, 255, 0, 255]);
    assert_eq!(pixel(&concave, 5, 5), [0; 4]);

    let transformed = svg("", r#"<g transform="translate(1 1)"><polyline points="1,1 4,1 4,4 1,4" style="fill: blue; fill-opacity: .5"/></g>"#);
    assert_eq!(pixel(&transformed, 2, 2), [0, 0, 255, 128]);
    assert_eq!(pixel(&transformed, 1, 1), [0; 4]);
}

#[test]
fn polyline_fill_closes_endpoints_and_uses_nonzero_winding() {
    let triangle = svg("", r#"<polyline points="1,1 6,1 6,6" fill="blue"/>"#);
    assert_eq!(pixel(&triangle, 4, 2), [0, 0, 255, 255]);
    assert_eq!(pixel(&triangle, 2, 5), [0; 4]);

    let double = svg("", r#"<polyline points="1,1 6,1 6,6 1,6 1,1 6,1 6,6 1,6" fill="red"/>"#);
    assert_eq!(pixel(&double, 2, 2), [255, 0, 0, 255]);
}

#[test]
fn polylines_reject_malformed_points_and_unsupported_features_whole_image() {
    for polyline in [
        r#"<polyline/>"#,
        r#"<polyline points=""/>"#,
        r#"<polyline points="0,0 1,0 1"/>"#,
        r#"<polyline points=",0 1,0 1,1"/>"#,
        r#"<polyline points="0,0 1,,0 1,1"/>"#,
        r#"<polyline points="0,0 1,0 1,1,"/>"#,
        r#"<polyline points="0,0 1,0 1px,1"/>"#,
        r#"<polyline points="0,0 1e309,0 1,1"/>"#,
        r#"<polyline points="0,0 1e,0 1,1"/>"#,
        r#"<polyline points="0,0-1,0 1,1"/>"#,
        r#"<polyline points="0,0 1,0 1,1" fill-rule="evenodd"/>"#,
        r#"<polyline points="0,0 1,0 1,1" fill="url(#paint)"/>"#,
        r#"<polyline points="0,0 1,0 1,1" href="other.svg"/>"#,
    ] {
        let body = format!(r#"<rect width="8" height="8" fill="red"/>{polyline}"#);
        assert_eq!(
            decode(&svg("", &body), Default::default()).unwrap_err(),
            DecodeError::UnsupportedSvg,
            "{polyline}"
        );
    }
}

#[test]
fn polylines_obey_point_attribute_and_raster_work_limits() {
    let at_limit = "0 0 ".repeat(512);
    assert!(decode(&svg("", &format!(r#"<polyline points="{at_limit}"/>"#)), Default::default()).is_ok());

    let over_limit = "0 0 ".repeat(513);
    assert_eq!(
        decode(&svg("", &format!(r#"<polyline points="{over_limit}"/>"#)), Default::default()).unwrap_err(),
        DecodeError::AllocationLimit
    );
    let over_attribute = "0 0 ".repeat(1025);
    assert_eq!(
        decode(&svg("", &format!(r#"<polyline points="{over_attribute}"/>"#)), Default::default()).unwrap_err(),
        DecodeError::AllocationLimit
    );
    let costly = "0 0 ".repeat(257);
    let large = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"><polyline points="{costly}"/></svg>"#);
    assert_eq!(decode(large.as_bytes(), Default::default()).unwrap_err(), DecodeError::AllocationLimit);
}
