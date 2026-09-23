mod dom_model_tests {
    use amazing_dom::NodeIdx;
    use amazing_image::{svg::{decode, from_dom, render_svg}, DecodeLimits};

    fn render_inline_svg(html: &str, width: u32, height: u32, css: &str) -> amazing_media::Image {
        let dom = amazing_html::parse(html);
        let root = (0..dom.node_count() as u32).map(NodeIdx)
            .find(|&node| dom.name(node) == Some("svg")).unwrap();
        let sheets = if css.is_empty() { Vec::new() } else { vec![amazing_css::parse_stylesheet(css)] };
        let styles = amazing_css::compute(&dom, &sheets);
        let model = from_dom(&dom, &styles, root, width, height, DecodeLimits::default()).unwrap();
        render_svg(&model, width, height, 1.0, DecodeLimits::default()).unwrap()
    }

    #[test]
    fn byte_and_dom_builders_share_rect_renderer() {
        let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8"><rect x="1" y="2" width="4" height="3" fill="#f00"/></svg>"##;
        let expected = decode(bytes, DecodeLimits::default()).unwrap();
        let dom = amazing_html::parse(
            "<svg width='8' height='8' viewBox='0 0 8 8'><rect x='1' y='2' width='4' height='3' fill='#f00'></rect></svg>",
        );
        let root = (0..dom.node_count() as u32)
            .map(NodeIdx)
            .find(|&n| dom.name(n) == Some("svg"))
            .unwrap();
        let styled = amazing_css::compute(&dom, &[]);
        let model = from_dom(&dom, &styled, root, 8, 8, DecodeLimits::default()).unwrap();
        let actual = render_svg(&model, 8, 8, 1.0, DecodeLimits::default()).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
        assert!(
            actual
                .pixels
                .chunks_exact(4)
                .any(|px| px == [255, 0, 0, 255])
        );
    }

    #[test]
    fn document_selector_overrides_svg_presentation_hint() {
        let dom = amazing_html::parse(
            "<style>rect { fill: blue }</style><svg width='8' height='8'><rect width='8' height='8' fill='red'></rect></svg>",
        );
        let root = (0..dom.node_count() as u32)
            .map(NodeIdx)
            .find(|&n| dom.name(n) == Some("svg"))
            .unwrap();
        let sheet = amazing_css::parse_stylesheet("rect { fill: blue }");
        let styled = amazing_css::compute(&dom, &[sheet]);
        let model = from_dom(&dom, &styled, root, 8, 8, DecodeLimits::default()).unwrap();
        let image = render_svg(&model, 8, 8, 1.0, DecodeLimits::default()).unwrap();
        assert_eq!(&image.pixels[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn byte_and_dom_builders_share_group_transform_and_inherited_fill() {
        let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><g transform="translate(1 0)" fill="#00f"><circle cx="2" cy="3" r="2"/></g></svg>"##;
        let expected = decode(bytes, DecodeLimits::default()).unwrap();
        let dom = amazing_html::parse("<svg width='8' height='8'><g transform='translate(1 0)' fill='#00f'><circle cx='2' cy='3' r='2'></circle></g></svg>");
        let root = (0..dom.node_count() as u32).map(NodeIdx).find(|&n| dom.name(n) == Some("svg")).unwrap();
        let styled = amazing_css::compute(&dom, &[]);
        let model = from_dom(&dom, &styled, root, 8, 8, DecodeLimits::default()).unwrap();
        let actual = render_svg(&model, 8, 8, 1.0, DecodeLimits::default()).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
        assert!(actual.pixels.chunks_exact(4).any(|px| px == [0, 0, 255, 255]));
    }

    #[test]
    fn nested_viewbox_renders_and_clips() {
        let dom = amazing_html::parse("<svg width='8' height='8' viewBox='0 0 4 4'><svg x='1' y='1' width='2' height='2' viewBox='0 0 256 256'><rect width='256' height='256' fill='green'></rect></svg></svg>");
        let root = (0..dom.node_count() as u32).map(NodeIdx).find(|&n| dom.name(n) == Some("svg")).unwrap();
        let styled = amazing_css::compute(&dom, &[]);
        let model = from_dom(&dom, &styled, root, 8, 8, DecodeLimits::default()).unwrap();
        let image = render_svg(&model, 8, 8, 1.0, DecodeLimits::default()).unwrap();
        let pixel = |x: usize, y: usize| &image.pixels[((y * 8 + x) * 4)..][..4];
        assert_eq!(pixel(0, 0), &[0, 0, 0, 0]);
        assert_eq!(pixel(3, 3), &[0, 128, 0, 255]);
        assert_eq!(pixel(7, 7), &[0, 0, 0, 0]);
    }

    #[test]
    fn dom_clip_path_resolves_auto_ellipse_radius() {
        let dom = amazing_html::parse("<svg width='8' height='8'><defs><clipPath id='cut'><ellipse cx='4' cy='4' rx='auto' ry='2'></ellipse></clipPath></defs><rect width='8' height='8' fill='green' clip-path='url(#cut)'></rect></svg>");
        let root = (0..dom.node_count() as u32).map(NodeIdx).find(|&n| dom.name(n) == Some("svg")).unwrap();
        let styled = amazing_css::compute(&dom, &[]);
        let model = from_dom(&dom, &styled, root, 8, 8, DecodeLimits::default()).unwrap();
        let image = render_svg(&model, 8, 8, 1.0, DecodeLimits::default()).unwrap();
        let pixel = |x: usize, y: usize| &image.pixels[((y * 8 + x) * 4)..][..4];
        assert_eq!(pixel(0, 0), &[0, 0, 0, 0]);
        assert_eq!(pixel(4, 4), &[0, 128, 0, 255]);
        assert_eq!(pixel(7, 7), &[0, 0, 0, 0]);
    }

    #[test]
    fn foreign_object_background_respects_missing_and_rect_masks() {
        let dom = amazing_html::parse("<svg width='8' height='8'><defs><mask id='cut'><rect x='2' y='2' width='2' height='2'></rect></mask></defs><foreignObject width='8' height='8' style=\"mask: url('#missing')\"><div style='background: red'></div></foreignObject><foreignObject width='8' height='8' style=\"mask: url('#cut')\"><div style='background: green'></div></foreignObject></svg>");
        let root = (0..dom.node_count() as u32).map(NodeIdx).find(|&n| dom.name(n) == Some("svg")).unwrap();
        let styled = amazing_css::compute(&dom, &[]);
        let model = from_dom(&dom, &styled, root, 8, 8, DecodeLimits::default()).unwrap();
        let image = render_svg(&model, 8, 8, 1.0, DecodeLimits::default()).unwrap();
        let pixel = |x: usize, y: usize| &image.pixels[((y * 8 + x) * 4)..][..4];
        assert_eq!(pixel(0, 0), &[0, 0, 0, 0]);
        assert_eq!(pixel(2, 2), &[0, 128, 0, 255]);
        assert_eq!(pixel(4, 4), &[0, 0, 0, 0]);
    }

    #[test]
    fn ahem_text_path_resolves_auto_ellipse_radius_and_paints_glyphs() {
        let css = "text { font-family: Ahem; font-size: 20px; fill: black }";
        let actual = render_inline_svg("<svg width='320' height='200'><defs><ellipse id='path' cx='80' cy='100' rx='auto' ry='60'></ellipse></defs><text><textPath href='#path'>XXXXXXXX</textPath></text></svg>", 320, 200, css);
        let reference = render_inline_svg("<svg width='320' height='200'><defs><ellipse id='path' cx='80' cy='100' rx='60' ry='60'></ellipse></defs><text><textPath href='#path'>XXXXXXXX</textPath></text></svg>", 320, 200, css);
        assert_eq!(actual.pixels, reference.pixels);
        assert!(actual.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }

    #[test]
    fn nested_non_scaling_path_stroke_matches_device_space_reference() {
        let actual = render_inline_svg("<svg width='100' height='100'><svg width='100%' height='100%' viewBox='0 0 1 1' preserveAspectRatio='none'><path d='M 0 0 L 0 50 L 50 0 z' fill='none' stroke='currentColor' stroke-width='20' vector-effect='non-scaling-stroke'></path></svg></svg>", 100, 100, "");
        let reference = render_inline_svg("<svg width='100' height='100'><path d='M 0 0 L 0 300 L 300 0 z' fill='none' stroke='currentColor' stroke-width='20'></path></svg>", 100, 100, "");
        let differences = actual.pixels.chunks_exact(4).zip(reference.pixels.chunks_exact(4))
            .enumerate().filter_map(|(index, (actual, reference))|
                (actual != reference).then_some((index % 100, index / 100, actual.to_vec(), reference.to_vec())))
            .collect::<Vec<_>>();
        assert!(differences.is_empty(), "{} differing pixels; first: {:?}", differences.len(), differences.first());
        assert!(actual.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}
