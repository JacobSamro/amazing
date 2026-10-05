use super::*;
use amazing_layout::{values::Sides, Display};
fn box_(pairs: &[(&str, &str)]) -> LayoutBox {
    let mut b = LayoutBox::new(
        None,
        Display::Block,
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        16.0,
        19.2,
    );
    b.border_box = Rect {
        x: 10.0,
        y: 10.0,
        w: 30.0,
        h: 30.0,
    };
    b.content_box = Rect {
        x: 15.0,
        y: 15.0,
        w: 20.0,
        h: 20.0,
    };
    b.border = Sides {
        top: 5.0,
        right: 5.0,
        bottom: 5.0,
        left: 5.0,
    };
    b
}
struct Pixels(amazing_media::Image);
impl ImageProvider for Pixels {
    fn image(&self, _: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        None
    }
    fn image_by_url(&self, _: &str) -> Option<&amazing_media::Image> {
        Some(&self.0)
    }
}
fn pixels() -> Pixels {
    let mut p = Vec::new();
    for y in 0..6 {
        for x in 0..6 {
            p.extend([x * 40, y * 40, 100, 255]);
        }
    }
    Pixels(amazing_media::Image {
        width: 6,
        height: 6,
        pixels: p,
    })
}
fn render(b: &LayoutBox, provider: &dyn ImageProvider) -> (Surface, Outcome) {
    let mut s = Surface::new(60, 60);
    s.clear([255; 4]);
    let result = paint(&mut s, b, provider, [0, 0, 60, 60], &mut 100000);
    (s, result)
}
#[test]
fn decoded_corners_center_fill_and_radius_independence() {
    let p = pixels();
    for fill in ["2", "2 fill"] {
        let b = box_(&[
            ("border-image-source", "url(x)"),
            ("border-image-slice", fill),
            ("border-radius", "100px"),
        ]);
        let (s, r) = render(&b, &p);
        assert_eq!(r, Outcome::Painted);
        assert_eq!(s.get_pixel(10, 10), Some([0, 0, 100, 255]));
        assert_eq!(s.get_pixel(39, 39), Some([200, 200, 100, 255]));
        assert_eq!(s.get_pixel(25, 25) == Some([255; 4]), fill == "2");
    }
}
#[test]
fn asymmetric_cuts_overlap_reduction_auto_and_outsets() {
    let b = box_(&[
        ("border-image-source", "url(x)"),
        ("border-image-slice", "10% 2 3 1"),
        ("border-image-width", "20px 30px 40px 50px"),
        ("border-image-outset", "1 2 3 4"),
    ]);
    let p = BorderImagePlan::from_style(&b.style).unwrap();
    let g = p.geometry(&b, (6, 6)).unwrap();
    assert_eq!(
        (g.area.x, g.area.y, g.area.w, g.area.h),
        (-10.0, 5.0, 60.0, 50.0)
    );
    assert!((g.w[0] - 15.0).abs() < 0.0001);
    assert_eq!(g.sx, [0.0, 1.0, 4.0, 6.0]);
    let b = box_(&[
        ("border-image-source", "url(x)"),
        ("border-image-slice", "2"),
        ("border-image-width", "auto"),
    ]);
    let g = BorderImagePlan::from_style(&b.style)
        .unwrap()
        .geometry(&b, (6, 6))
        .unwrap();
    assert_eq!(g.w, [2.0; 4]);
}
#[test]
fn repeats_round_space_and_centered_repeat_have_real_gaps() {
    let stretch = Axis::new(20.0, 6.0, Repeat::Stretch);
    assert_eq!(stretch.sample(10.0), Some(0.5));
    let round = Axis::new(20.0, 6.0, Repeat::Round);
    assert!((round.tile - 20.0 / 3.0).abs() < 0.00001);
    let repeat = Axis::new(20.0, 6.0, Repeat::Repeat);
    assert_eq!(repeat.sample(10.0), Some(0.5));
    let space = Axis::new(20.0, 6.0, Repeat::Space);
    assert_eq!(space.sample(0.0), None);
    assert_eq!(space.sample(0.5), Some(0.0));
    assert_eq!(Axis::new(3.0, 6.0, Repeat::Space).sample(1.0), None);
    for mode in [
        "stretch",
        "repeat",
        "round",
        "space",
        "stretch round",
        "space repeat",
    ] {
        let b = box_(&[
            ("border-image-source", "url(x)"),
            ("border-image-slice", "2 fill"),
            ("border-image-repeat", mode),
        ]);
        assert_eq!(render(&b, &pixels()).1, Outcome::Painted);
    }
}
#[test]
fn clipping_budget_missing_and_unresolved_sources_fall_back() {
    let mut b = box_(&[
        ("border-image-source", "url(x)"),
        ("border-image-slice", "2"),
        ("border-image-outset", "10px"),
    ]);
    let mut s = Surface::new(60, 60);
    s.clear([255; 4]);
    assert_eq!(
        paint(&mut s, &b, &pixels(), [10, 10, 40, 40], &mut 10000),
        Outcome::Painted
    );
    assert_eq!(s.get_pixel(0, 0), Some([255; 4]));
    assert_eq!(
        paint(&mut s, &b, &pixels(), [0, 0, 60, 60], &mut 1),
        Outcome::BudgetExceeded
    );
    assert_eq!(render(&b, &crate::paint::NoImages).1, Outcome::Unavailable);
    let malformed = Pixels(amazing_media::Image {
        width: 2,
        height: 2,
        pixels: vec![0; 3],
    });
    assert_eq!(render(&b, &malformed).1, Outcome::Unavailable);
    let mut huge = box_(&[
        ("border-image-source", "linear-gradient(red,blue)"),
        ("border-image-slice", "2"),
    ]);
    huge.border_box.w = 5000.0;
    huge.border_box.h = 5000.0;
    assert_eq!(
        paint(
            &mut s,
            &huge,
            &crate::paint::NoImages,
            [0, 0, 60, 60],
            &mut (16 * 1024 * 1024)
        ),
        Outcome::BudgetExceeded
    );
    b.style = vec![
        ("border-image-source".into(), "url(x)".into()),
        ("border-image-outset".into(), "1cqw".into()),
    ]
    .into_iter()
    .collect();
    assert_eq!(render(&b, &pixels()).1, Outcome::Unavailable);
}
#[test]
fn retained_outset_damage_matches_actual_full_raster() {
    use crate::display_list_view::{
        DisplayListView, PaintDamage, PaintItemKind, RetainedPaintView,
    };
    let mut old = box_(&[
        ("border-image-source", "url(x)"),
        ("border-image-slice", "2"),
        ("border-image-outset", "2px"),
    ]);
    old.node = Some(amazing_dom::NodeIdx(1));
    let mut next = box_(&[]);
    next.node = old.node;
    next.style = vec![
        ("border-image-source".into(), "url(x)".into()),
        ("border-image-slice".into(), "2".into()),
        ("border-image-outset".into(), "8px".into()),
    ]
    .into_iter()
    .collect();
    let text = amazing_text::FallbackTextSystem::new();
    let provider = pixels();
    let view = |b| {
        DisplayListView::with_node_generations(
            b,
            60,
            60,
            &text,
            &provider,
            &crate::controls::NoControls,
            &|_| Some(1),
        )
    };
    let mut old_root = box_(&[]);
    old_root.node = Some(amazing_dom::NodeIdx(0));
    old_root.border_box = Rect {
        x: 0.0,
        y: 0.0,
        w: 60.0,
        h: 60.0,
    };
    old_root.content_box = old_root.border_box;
    old_root.border = Sides::default();
    old_root.flow = amazing_layout::Flow::Blocks(vec![old]);
    let mut next_root = box_(&[]);
    next_root.node = Some(amazing_dom::NodeIdx(0));
    next_root.border_box = old_root.border_box;
    next_root.content_box = old_root.content_box;
    next_root.border = Sides::default();
    next_root.flow = amazing_layout::Flow::Blocks(vec![next]);
    let before = view(&old_root);
    let after = view(&next_root);
    let amazing_layout::Flow::Blocks(children) = &next_root.flow else {
        unreachable!()
    };
    let next = &children[0];
    let item = *after
        .items()
        .iter()
        .find(|i| {
            i.kind == PaintItemKind::Decorations
                && i.key
                    .is_some_and(|k| k.node.node == amazing_dom::NodeIdx(1))
        })
        .unwrap();
    assert_eq!(
        (
            item.local_bounds.x,
            item.local_bounds.y,
            item.local_bounds.width,
            item.local_bounds.height
        ),
        (2.0, 2.0, 46.0, 46.0)
    );
    let mut retained = RetainedPaintView::from_view(&before).unwrap();
    let delta = retained
        .apply_damage(
            &[PaintDamage::Upsert { item, box_: &next }],
            &text,
            &provider,
        )
        .unwrap();
    assert_eq!(delta.changes.len(), 1);
    let raster = retained.raster(item.key.unwrap()).unwrap();
    let mut oracle = Surface::new(raster.surface.width, raster.surface.height);
    after
        .raster_raw_local(item.index..item.index + 1, &mut oracle, raster.origin)
        .unwrap();
    assert_eq!(*raster.surface, oracle);
    assert!(oracle.pixels.chunks_exact(4).any(|p| p[3] != 0));
}
#[test]
fn invalid_or_missing_border_image_keeps_ordinary_border_pixels() {
    for source in ["none", "url(missing)", "paint(unsupported)"] {
        let b = box_(&[
            ("border-image-source", source),
            ("border-top-style", "solid"),
            ("border-right-style", "solid"),
            ("border-bottom-style", "solid"),
            ("border-left-style", "solid"),
            ("border-top-color", "green"),
            ("border-right-color", "green"),
            ("border-bottom-color", "green"),
            ("border-left-color", "green"),
        ]);
        let surface =
            crate::paint_document(&b, 60, 60, &amazing_layout::font::FontSource::fallback());
        assert_eq!(surface.get_pixel(12, 12), Some([0, 128, 0, 255]));
    }
}
#[test]
fn decoded_and_gradient_axes_scale_absolute_lengths_once() {
    let mut b = box_(&[
        ("border-image-source", "url(x)"),
        ("border-image-slice", "2"),
        ("border-image-width", "25% 3px"),
        ("border-image-outset", "2px"),
    ]);
    b.style.set("zoom", "2");
    b.border_box.w = 60.0;
    b.border_box.h = 60.0;
    let plan = BorderImagePlan::from_style(&b.style).unwrap();
    let g = plan.geometry(&b, (6, 6)).unwrap();
    assert_eq!(g.area.w, 68.0);
    assert_eq!(g.w, [17.0, 6.0, 17.0, 6.0]);
    assert_eq!(g.sx, [0.0, 2.0, 4.0, 6.0]);
    b.style
        .set("border-image-source", "linear-gradient(red,blue)");
    b.style.set("border-image-width", "auto");
    let plan = BorderImagePlan::from_style(&b.style).unwrap();
    let g = plan.geometry(&b, (68, 68)).unwrap();
    assert_eq!(g.sx, [0.0, 4.0, 64.0, 68.0]);
    assert_eq!(g.w, [5.0; 4]);
    let (s, outcome) = render(&b, &crate::paint::NoImages);
    assert_eq!(outcome, Outcome::Painted);
    assert_ne!(s.get_pixel(7, 7), Some([255; 4]));
}
#[test]
fn changed_urls_or_stylesheet_bases_retire_the_actual_resource_identity() {
    use crate::display_list_view::{
        DisplayListView, PaintDamage, PaintItemKind, PaintResourceKey, PaintViewUpdate,
        RetainedPaintView,
    };
    fn style(url: &str, base: &str) -> ComputedStyle {
        let dom = amazing_dom::Dom::default();
        // Style provenance comes from a real stylesheet computation, not an injected base.
        let mut sheet = amazing_css::parse_stylesheet(&format!(
            "*{{border-image-source:url({url});border-image-slice:2}}"
        ));
        sheet.base_url = Some(base.into());
        let mut dom = dom;
        let node = dom.create_element(amazing_dom::NodeIdx(0), "div");
        amazing_css::compute(&dom, &[sheet])
            .get(node)
            .unwrap()
            .clone()
    }
    fn root(style: ComputedStyle) -> LayoutBox {
        let mut child = box_(&[]);
        child.node = Some(amazing_dom::NodeIdx(1));
        child.style = style;
        let mut r = box_(&[]);
        r.node = Some(amazing_dom::NodeIdx(0));
        r.border = Sides::default();
        r.flow = amazing_layout::Flow::Blocks(vec![child]);
        r
    }
    let text = amazing_text::FallbackTextSystem::new();
    let p = pixels();
    for (url, base) in [
        ("y", "https://example.test/a.css"),
        ("x", "https://example.test/b.css"),
    ] {
        let old = root(style("x", "https://example.test/a.css"));
        let next = root(style(url, base));
        let make = |r| {
            DisplayListView::with_node_generations(
                r,
                60,
                60,
                &text,
                &p,
                &crate::controls::NoControls,
                &|_| Some(1),
            )
        };
        let before = make(&old);
        let after = make(&next);
        let expected = PaintResourceKey::BorderImageUrl {
            url: "x".into(),
            source_base: Some("https://example.test/a.css".into()),
        };
        let retired = match before.update(&after) {
            PaintViewUpdate::Delta(d) => d.retired_resources,
            PaintViewUpdate::RequiresFullRebuild {
                retired_resources, ..
            } => retired_resources,
        };
        assert_eq!(retired, [expected.clone()]);
        let mut retained = RetainedPaintView::from_view(&before).unwrap();
        let item = *after
            .items()
            .iter()
            .find(|i| {
                i.kind == PaintItemKind::Decorations
                    && i.key
                        .is_some_and(|k| k.node.node == amazing_dom::NodeIdx(1))
            })
            .unwrap();
        let amazing_layout::Flow::Blocks(children) = &next.flow else {
            unreachable!()
        };
        let delta = retained
            .apply_damage(
                &[PaintDamage::Upsert {
                    item,
                    box_: &children[0],
                }],
                &text,
                &p,
            )
            .unwrap();
        assert_eq!(delta.retired_resources, [expected]);
        assert!(!delta.changes.is_empty());
    }
}

#[test]
fn device_generated_border_failures_are_explicit_and_leave_target_unwritten() {
    for (css, extent) in [("linear-gradient(red, blue)", 1500.0),
                          ("linear-gradient(unsupported-color, blue)", 30.0)] {
        let mut b = box_(&[("border-image-source", css)]);
        b.border_box.w = extent;
        b.border_box.h = extent;
        let mut target = Surface::new(60, 60);
        target.clear([20, 30, 40, 255]);
        let original = target.pixels.clone();
        let mut budget = 16 * 1024 * 1024;
        let result = paint_at_lattice(&mut target, &b, &crate::paint::NoImages,
            [0, 0, 60, 60], &mut budget,
            crate::raster_viewport::RasterLattice { scale: 2.0, offset: (0.0, 0.0) });
        assert_eq!(result, Outcome::RasterFailure);
        assert_eq!(target.pixels, original);
    }
    let b = box_(&[("border-image-source", "linear-gradient(unsupported-color, blue)")]);
    assert_eq!(render(&b, &crate::paint::NoImages).1, Outcome::Unavailable);
    let b = box_(&[("border-image-source", "linear-gradient(red, blue)")]);
    let mut target = Surface::new(120, 120);
    assert_eq!(paint_at_lattice(&mut target, &b, &crate::paint::NoImages,
        [0, 0, 120, 120], &mut 100000,
        crate::raster_viewport::RasterLattice { scale: 2.0, offset: (0.0, 0.0) }), Outcome::Painted);
}
