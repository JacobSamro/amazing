use amazing_canvas::*;
use amazing_text::FallbackTextSystem;

#[test]
fn paths_fill_rules_and_antialiasing_have_golden_pixels() {
    let mut canvas = Canvas2D::new(16, 16);
    canvas.set_fill_color("#f00");
    canvas.begin_path();
    canvas.rect(0., 0., 16., 16.);
    canvas.rect(4., 4., 8., 8.);
    canvas.fill(FillRule::EvenOdd);
    assert_eq!(canvas.surface.get_pixel(2, 2), Some([255, 0, 0, 255]));
    assert_eq!(canvas.surface.get_pixel(8, 8), Some([0, 0, 0, 0]));
    canvas.begin_path();
    canvas.move_to(0., 0.);
    canvas.line_to(16., 0.);
    canvas.line_to(0., 16.);
    canvas.close_path();
    canvas.set_fill_color("#0f0");
    canvas.fill(FillRule::NonZero);
    assert_eq!(canvas.surface.get_pixel(1, 1), Some([0, 255, 0, 255]));
    let edge = canvas.surface.get_pixel(7, 8).unwrap();
    assert!(edge[3] > 0 && edge[3] < 255, "edge {edge:?}");
}

#[test]
fn gradients_composite_and_image_data_pixels() {
    let mut canvas = Canvas2D::new(4, 2);
    let mut gradient = Gradient::linear(0., 0., 4., 0.);
    gradient.add_color_stop(0., "#000").unwrap();
    gradient.add_color_stop(1., "#fff").unwrap();
    canvas.state.fill_style = PaintStyle::LinearGradient(gradient);
    canvas.fill_rect(0., 0., 4., 2.);
    let left = canvas.surface.get_pixel(0, 0).unwrap();
    let right = canvas.surface.get_pixel(3, 0).unwrap();
    assert!(left[0] < right[0]);
    canvas.state.composite_operation = CompositeOperation::Copy;
    canvas.set_fill_color("rgba(255, 0, 0, 0.5)");
    canvas.fill_rect(0., 0., 1., 1.);
    assert_eq!(canvas.surface.get_pixel(0, 0), Some([255, 0, 0, 128]));
    let data = canvas.get_image_data(0, 0, 1, 1).unwrap();
    assert_eq!(data.data, [255, 0, 0, 128]);
    canvas.put_image_data(
        &ImageData::from_rgba(1, 1, vec![0, 30, 255, 64]).unwrap(),
        1,
        0,
    );
    assert_eq!(canvas.surface.get_pixel(1, 0), Some([0, 30, 255, 64]));
}

#[test]
fn bitmap_smoothing_and_text_system_are_used() {
    let image = Bitmap::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut canvas = Canvas2D::new(8, 16);
    canvas.state.image_smoothing_enabled = false;
    canvas.draw_image_scaled(&image, 0., 0., 4., 1.);
    assert_eq!(canvas.surface.get_pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(canvas.surface.get_pixel(3, 0), Some([0, 0, 255, 255]));
    let text = FallbackTextSystem::new();
    let metrics = canvas.measure_text("A", &text).unwrap();
    assert!(metrics.width > 0.);
    canvas.fill_text("A", 0., 14., None, &text).unwrap();
    assert!(canvas.surface.pixels.chunks_exact(4).any(|p| p[3] > 0));
}

#[test]
fn svg_paths_stroke_caps_and_composite_modes() {
    let path = Path2D::from_svg("M 1 1 L 9 1 L 9 9 Z").unwrap();
    let mut canvas = Canvas2D::new(12, 12);
    canvas.set_fill_color("#f00");
    canvas.fill_path(&path, FillRule::NonZero);
    assert_eq!(canvas.surface.get_pixel(8, 2), Some([255, 0, 0, 255]));
    assert_eq!(canvas.surface.get_pixel(2, 8), Some([0, 0, 0, 0]));
    let arc = Path2D::from_svg("M 2 6 A 4 4 0 0 1 10 6").unwrap();
    assert!(arc.current_point().is_some());
    canvas.begin_path();
    canvas.move_to(4., 4.);
    canvas.line_to(8., 4.);
    canvas.state.line_width = 2.;
    canvas.state.line_cap = LineCap::Square;
    canvas.set_stroke_color("#0f0");
    canvas.stroke();
    assert_eq!(canvas.surface.get_pixel(3, 4), Some([0, 255, 0, 255]));
    for name in [
        "source-over",
        "source-in",
        "source-out",
        "source-atop",
        "destination-over",
        "destination-in",
        "destination-out",
        "destination-atop",
        "lighter",
        "copy",
        "xor",
        "multiply",
        "screen",
        "overlay",
        "darken",
        "lighten",
        "color-dodge",
        "color-burn",
        "hard-light",
        "soft-light",
        "difference",
        "exclusion",
        "hue",
        "saturation",
        "color",
        "luminosity",
    ] {
        assert!(CompositeOperation::parse(name).is_some(), "{name}");
    }
    let mut copy = Canvas2D::new(3, 1);
    copy.set_fill_color("#f00");
    copy.fill_rect(0., 0., 3., 1.);
    copy.state.composite_operation = CompositeOperation::Copy;
    copy.set_fill_color("#00f");
    copy.fill_rect(0., 0., 1., 1.);
    assert_eq!(copy.surface.get_pixel(0, 0), Some([0, 0, 255, 255]));
    assert_eq!(copy.surface.get_pixel(2, 0), Some([0, 0, 0, 0]));
}

#[test]
fn gradients_patterns_shadows_state_and_pixel_bypass() {
    let mut radial = Gradient::radial(4., 4., 0., 4., 4., 4.);
    radial.add_color_stop(0., "#000").unwrap();
    radial.add_color_stop(1., "#fff").unwrap();
    let center = PaintStyle::RadialGradient(radial.clone()).sample(Point::new(4., 4.));
    let edge = PaintStyle::RadialGradient(radial).sample(Point::new(8., 4.));
    assert!(center[0] < edge[0]);
    let mut conic = Gradient::conic(0., 4., 4.);
    conic.add_color_stop(0., "#f00").unwrap();
    conic.add_color_stop(1., "#00f").unwrap();
    assert_ne!(
        PaintStyle::ConicGradient(conic.clone()).sample(Point::new(8., 4.)),
        PaintStyle::ConicGradient(conic).sample(Point::new(4., 8.))
    );
    let bitmap = Bitmap::new(1, 1, vec![20, 40, 60, 255]).unwrap();
    let pattern = Pattern {
        bitmap,
        repetition: Repetition::Repeat,
        offset: Point::new(0., 0.),
    };
    assert_eq!(pattern.sample(Point::new(10., 20.)), [20, 40, 60, 255]);

    let mut canvas = Canvas2D::new(8, 8);
    canvas.state.shadow_color = [0, 0, 255, 255];
    canvas.state.shadow_offset_x = 2.;
    canvas.set_fill_color("#f00");
    canvas.fill_rect(1., 1., 2., 2.);
    assert_eq!(canvas.surface.get_pixel(1, 1), Some([255, 0, 0, 255]));
    assert_eq!(canvas.surface.get_pixel(4, 1), Some([0, 0, 255, 255]));
    canvas.save();
    canvas.set_global_alpha(0.25);
    canvas.set_fill_color("#0f0");
    canvas.restore();
    assert_eq!(canvas.state.global_alpha, 1.);
    assert_eq!(canvas.state.fill_style, PaintStyle::Color([255, 0, 0, 255]));
    assert_eq!(canvas.set_filter("blur(2px)"), FilterStatus::Unsupported);
    canvas.translate(5., 5.);
    canvas.set_global_alpha(0.1);
    let data = ImageData::from_rgba(1, 1, vec![10, 20, 30, 100]).unwrap();
    canvas.put_image_data(&data, 0, 0);
    assert_eq!(canvas.surface.get_pixel(0, 0), Some([10, 20, 30, 100]));
}

#[test]
fn porter_duff_opaque_goldens_and_negative_image_dimensions() {
    let expected = [
        (CompositeOperation::SourceOver, [0, 0, 255, 255]),
        (CompositeOperation::SourceIn, [0, 0, 255, 255]),
        (CompositeOperation::SourceOut, [0, 0, 0, 0]),
        (CompositeOperation::SourceAtop, [0, 0, 255, 255]),
        (CompositeOperation::DestinationOver, [255, 0, 0, 255]),
        (CompositeOperation::DestinationIn, [255, 0, 0, 255]),
        (CompositeOperation::DestinationOut, [0, 0, 0, 0]),
        (CompositeOperation::DestinationAtop, [255, 0, 0, 255]),
        (CompositeOperation::Lighter, [255, 0, 255, 255]),
        (CompositeOperation::Copy, [0, 0, 255, 255]),
        (CompositeOperation::Xor, [0, 0, 0, 0]),
        (CompositeOperation::Multiply, [0, 0, 0, 255]),
        (CompositeOperation::Screen, [255, 0, 255, 255]),
    ];
    for (operation, pixel) in expected {
        let mut canvas = Canvas2D::new(1, 1);
        canvas.set_fill_color("#f00");
        canvas.fill_rect(0., 0., 1., 1.);
        canvas.state.composite_operation = operation;
        canvas.set_fill_color("#00f");
        canvas.fill_rect(0., 0., 1., 1.);
        assert_eq!(canvas.surface.get_pixel(0, 0), Some(pixel), "{operation:?}");
    }
    let image = Bitmap::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut canvas = Canvas2D::new(2, 1);
    canvas.state.image_smoothing_enabled = false;
    canvas.draw_image_scaled(&image, 2., 0., -2., 1.);
    assert_eq!(canvas.surface.get_pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(canvas.surface.get_pixel(1, 0), Some([0, 0, 255, 255]));
}

#[test]
fn text_alignment_stroke_and_signed_image_data() {
    let system = FallbackTextSystem::new();
    let mut canvas = Canvas2D::new(64, 32);
    canvas.state.text_align = TextAlign::Center;
    canvas.state.text_baseline = TextBaseline::Top;
    let width = canvas.measure_text("A", &system).unwrap().width;
    assert!(width > 0.);
    canvas.set_fill_color("#f00");
    canvas.fill_text("A", 32., 1., None, &system).unwrap();
    assert!(canvas.surface.get_pixel(32, 3).unwrap()[3] > 0);
    canvas.set_stroke_color("#00f");
    canvas.state.line_width = 2.;
    canvas.stroke_text("A", 48., 1., None, &system).unwrap();
    assert!(canvas.surface.pixels.chunks_exact(4).any(|p| p[2] > 0));

    let opaque = ImageData::from_rgba(2, 1, vec![7, 8, 9, 0, 10, 20, 30, 255]).unwrap();
    canvas.put_image_data_dirty_signed(&opaque, 0, 0, 2, 0, -2, 1);
    assert_eq!(canvas.surface.get_pixel(0, 0), Some([0, 0, 0, 0]));
    assert_eq!(canvas.surface.get_pixel(1, 0), Some([10, 20, 30, 255]));
    assert_eq!(
        canvas.get_image_data_signed(2, 1, -2, -1).unwrap().data[4..8],
        [10, 20, 30, 255]
    );
}

#[test]
fn path_curves_arcs_round_rect_dashes_and_hit_testing() {
    let mut canvas = Canvas2D::new(32, 32);
    canvas.begin_path();
    canvas.arc(8., 8., 6., 0., std::f64::consts::TAU, false);
    assert!(canvas.is_point_in_path(8., 8., FillRule::NonZero));
    assert!(!canvas.is_point_in_path(0., 0., FillRule::NonZero));
    canvas.set_fill_color("#f00");
    canvas.fill(FillRule::NonZero);
    assert_eq!(canvas.surface.get_pixel(8, 8), Some([255, 0, 0, 255]));

    let mut rounded = Path2D::new();
    rounded.round_rect(18., 2., 12., 12., [4.; 4]);
    assert!(canvas.is_point_in_path2d(&rounded, 24., 8., FillRule::NonZero));
    assert!(!canvas.is_point_in_path2d(&rounded, 18., 2., FillRule::NonZero));
    canvas.set_fill_color("#0f0");
    canvas.fill_path(&rounded, FillRule::NonZero);
    assert_eq!(canvas.surface.get_pixel(24, 8), Some([0, 255, 0, 255]));

    canvas.begin_path();
    canvas.move_to(1., 20.);
    canvas.quadratic_curve_to(8., 10., 15., 20.);
    canvas.bezier_curve_to(20., 30., 25., 10., 30., 20.);
    canvas.ellipse(24., 20., 3., 2., 0., 0., std::f64::consts::PI, false);
    assert!(canvas.current_path().current_point().is_some());
    canvas.begin_path();
    canvas.move_to(1., 26.);
    canvas.arc_to(8., 26., 8., 31., 3.);
    assert!(canvas.current_path().current_point().is_some());

    canvas.begin_path();
    canvas.move_to(1., 30.);
    canvas.line_to(17., 30.);
    canvas.state.line_width = 2.;
    canvas.set_line_dash(&[3., 3.]);
    assert!(canvas.is_point_in_stroke(2., 30.));
    assert!(!canvas.is_point_in_stroke(5., 30.));
    let dashed = canvas.current_path().clone();
    assert!(canvas.is_point_in_stroke2d(&dashed, 2., 30.));
}
