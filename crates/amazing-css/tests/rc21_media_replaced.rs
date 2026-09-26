use amazing_css::{compute, evaluate_media_query, parse_sizes_attribute, parse_stylesheet,
    ColorScheme, ContrastPreference, HoverCapability, MediaEnvironment, MediaQueryList, MediaType,
    PointerCapability};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn image_orientation_and_object_properties_compute_on_all_replaced_tags() {
    let tags = ["img", "video", "canvas", "iframe", "embed", "object", "svg"];
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let nodes: Vec<_> = tags.iter().map(|tag| dom.create_element(html, tag)).collect();
    let defaults = compute(&dom, &[]);
    for node in &nodes {
        let style = defaults.get(*node).unwrap();
        assert_eq!(style.get("image-orientation"), Some("from-image"));
        assert_eq!(style.get("object-fit"), Some("fill"));
        assert_eq!(style.get("object-position"), Some("50% 50%"));
    }
    let sheet = parse_stylesheet("img, video, canvas, iframe, embed, object, svg { image-orientation:none; object-fit:cover; object-position:right 10px bottom 20px }");
    let styles = compute(&dom, &[sheet]);
    for node in &nodes {
        let style = styles.get(*node).unwrap();
        assert_eq!(style.get("image-orientation"), Some("none"));
        assert_eq!(style.get("object-fit"), Some("cover"));
        assert_eq!(style.get("object-position"), Some("right 10px bottom 20px"));
    }
    let sheet = parse_stylesheet("img { image-orientation: 90deg flip; object-fit:invalid; object-position:garbage }");
    let style = compute(&dom, &[sheet]);
    let img = style.get(nodes[0]).unwrap();
    assert_eq!(img.get("image-orientation"), Some("90deg flip"));
    assert_eq!(img.get("object-fit"), Some("fill"));
    assert_eq!(img.get("object-position"), Some("50% 50%"));
    let sheet = parse_stylesheet("html {image-orientation:none} @supports (object-fit:contain) { img {object-fit:contain} } @supports (image-orientation:from-image) { img {object-position: left top} }");
    let style = compute(&dom, &[sheet]);
    let img = style.get(nodes[0]).unwrap();
    assert_eq!(img.get("image-orientation"), Some("none"));
    assert_eq!(img.get("object-fit"), Some("contain"));
    assert_eq!(img.get("object-position"), Some("left top"));
}

#[test]
fn media_queries_cover_types_ranges_preferences_and_boolean_forms() {
    let env = MediaEnvironment { viewport_width_px: 800.0, viewport_height_px: 600.0,
        device_pixel_ratio: 2.0, media_type: MediaType::Screen,
        prefers_color_scheme: ColorScheme::Dark, prefers_reduced_motion: true,
        prefers_contrast: ContrastPreference::More, ..Default::default() };
    for query in ["screen and (min-width: 50em)", "(width >= 800px)", "(800px <= width)",
        "(max-height: 600px) and (orientation: landscape)", "(resolution: 192dpi)",
        "(prefers-color-scheme: dark) and (prefers-reduced-motion: reduce)",
        "(prefers-contrast: more)", "(width)", "print, screen and (min-resolution: 2dppx)"] {
        assert!(evaluate_media_query(&MediaQueryList::parse(query), &env), "{query}");
    }
    for query in ["print", "(min-width: 801px)", "(width < 800px)",
        "(prefers-reduced-motion: no-preference)", "(unknown-feature: value)"] {
        assert!(!MediaQueryList::parse(query).evaluate(&env), "{query}");
    }
    assert!(MediaQueryList::parse("not print").evaluate(&env));
    assert!(MediaQueryList::parse("").evaluate(&env));
    assert!(!MediaQueryList::parse("not (unknown-feature)").evaluate(&env));
}

#[test]
fn hover_pointer_and_aspect_ratio_features_evaluate() {
    let mouse = MediaEnvironment { viewport_width_px: 1600.0, viewport_height_px: 900.0, ..Default::default() };
    for query in ["(hover: hover)", "(any-hover: hover)", "(pointer: fine)", "(any-pointer: fine)",
        "(aspect-ratio: 16/9)", "(min-aspect-ratio: 1/1)", "(max-aspect-ratio: 2/1)", "(aspect-ratio)"] {
        assert!(MediaQueryList::parse(query).evaluate(&mouse), "{query}");
    }
    for query in ["(hover: none)", "(pointer: coarse)", "(pointer: none)", "(aspect-ratio: 4/3)"] {
        assert!(!MediaQueryList::parse(query).evaluate(&mouse), "{query}");
    }
    let touch = MediaEnvironment {
        hover: HoverCapability::None, any_hover: HoverCapability::Hover,
        pointer: PointerCapability::Coarse, any_pointer: PointerCapability::Fine,
        ..Default::default()
    };
    assert!(MediaQueryList::parse("(hover: none) and (any-hover: hover)").evaluate(&touch));
    assert!(MediaQueryList::parse("(pointer: coarse) and (any-pointer: fine)").evaluate(&touch));
    assert!(!MediaQueryList::parse("(hover: hover)").evaluate(&touch));
}

#[test]
fn sizes_select_first_matching_entry_and_resolve_viewport_lengths() {
    let env = MediaEnvironment { viewport_width_px: 800.0, viewport_height_px: 600.0, ..Default::default() };
    assert_eq!(parse_sizes_attribute("(max-width: 600px) 100vw, (min-width: 700px) 50vw, 200px", &env, None), 400.0);
    assert_eq!(parse_sizes_attribute("(max-width: 600px) 100vw, 30em", &env, None), 480.0);
    assert_eq!(parse_sizes_attribute("auto, 50vw", &env, Some(320.0)), 320.0);
    assert_eq!(parse_sizes_attribute("auto, 50vw", &env, None), 400.0);
    assert_eq!(parse_sizes_attribute("(min-width: 700px) calc(50vw - 20px), 100vw", &env, None), 380.0);
    assert_eq!(parse_sizes_attribute("min(60vw, 300px)", &env, None), 300.0);
    assert_eq!(parse_sizes_attribute("garbage", &env, None), 800.0);
}
