//! Sampling is a source contract, independent of DPR and tile caching.
use amazing_compositor::{
    PaintViewRasterMode, PaintViewSource, Presenter, RasterSampling, ReferenceCpuPresenter,
};
use amazing_paint::{
    NoImages, RasterViewport, controls::NoControls, display_list_view::DisplayListView,
    paint_document_at_viewport,
};
use amazing_text::FallbackTextSystem;

#[test]
fn paint_transform_sampling_preserves_full_direct_retained_cached_and_snapshot_pixels() {
    let text = FallbackTextSystem;
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        for translation in ["2px,3px", "-1.25px,2.5px", "3.5px,-.75px"] {
            for (alpha, clip) in [("1", "visible"), (".5", "hidden")] {
                let dom = amazing_html::parse(
                    "<body><div class=clip><div class=paint></div></div></body>",
                );
                let css = format!(
                    "body{{margin:0;width:32px;height:24px;background:white}}.clip{{width:22px;height:16px;overflow:{clip}}}.paint{{width:20px;height:12px;background:rgba(200,30,90,{alpha});transform:translate({translation})}}"
                );
                let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
                    &dom,
                    &css,
                    32.0,
                    Some(24.0),
                    &text,
                    &amazing_layout::NoReplacedElements,
                );
                let viewport = RasterViewport::new((32.0, 24.0), (32.0, 24.0), scale).unwrap();
                let view = DisplayListView::with_dom_generations(
                    &root,
                    32,
                    24,
                    &text,
                    &NoImages,
                    &NoControls,
                    &dom,
                )
                .with_raster_viewport(viewport)
                .unwrap();
                let source = PaintViewSource::new(&view);
                assert_eq!(source.mode(), PaintViewRasterMode::RawLocal);
                let (w, h) = viewport.physical_size();
                let expected =
                    paint_document_at_viewport(&root, viewport, &text, &NoImages, &NoControls)
                        .unwrap();
                let mut scene = source.compositor(w, h).unwrap();
                assert_eq!(scene.raster_sampling(), RasterSampling::Nearest);
                let frame = source.build_frame(&mut scene, w, h).unwrap();
                let label = format!("{scale}/{translation}/{alpha}/{clip}");
                let mut presenter = ReferenceCpuPresenter::default();
                assert_eq!(presenter.present(&frame).unwrap(), expected, "{label}");
                let restored = amazing_compositor::CompositorFrame::from_effects_snapshot(
                    frame.to_effects_snapshot(),
                )
                .unwrap();
                assert_eq!(restored.raster_sampling, RasterSampling::Nearest);
                assert_eq!(
                    ReferenceCpuPresenter::default().present(&restored).unwrap(),
                    expected,
                    "snapshot {label}"
                );
                let cached = source.build_cached_frame(&mut scene, w, h).unwrap();
                assert_eq!(
                    presenter.present(&cached).unwrap(),
                    expected,
                    "cached {label}"
                );
                let mut direct = source.compositor(w, h).unwrap();
                assert_eq!(
                    direct.render(&source, w, h).unwrap(),
                    expected,
                    "direct {label}"
                );
            }
        }
    }
}

#[test]
fn generic_sources_preserve_bilinear_sampling_and_snapshot_policy() {
    use amazing_compositor::*;
    let mut pixels = amazing_paint::Surface::new(2, 2);
    pixels.clear([255, 0, 0, 255]);
    let mut trees = PropertyTrees::new();
    let transform = trees
        .add_transform(
            TransformId(0),
            Matrix4::translation(0.25, 0.25, 0.),
            (0., 0., 0.),
            true,
        )
        .unwrap();
    let mut scene = Compositor::new(
        trees,
        vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState {
                transform,
                ..Default::default()
            },
            bounds: Rect::new(0., 0., 2., 2.),
            reasons: CompositingReasons::WILL_CHANGE,
        }],
        1,
    )
    .unwrap();
    assert_eq!(scene.raster_sampling(), RasterSampling::Bilinear);
    let frame = scene.build_frame(&pixels, 4, 4).unwrap();
    let restored = CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
    assert_eq!(restored.raster_sampling, RasterSampling::Bilinear);
    let output = ReferenceCpuPresenter::default().present(&restored).unwrap();
    let edge = output.get_pixel(0, 0).unwrap();
    assert!(edge[3] > 0 && edge[3] < 255, "bilinear edge {edge:?}");
    assert_eq!(
        output,
        ReferenceCpuPresenter::default().present(&frame).unwrap()
    );
}

#[test]
fn nearest_paint_sampling_survives_tile_edges_and_real_effect_passes() {
    let text = FallbackTextSystem;
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        for effect in ["", "opacity:.5", "filter:invert(1)", "clip-path:inset(2px)"] {
            let dom = amazing_html::parse("<body><div></div></body>");
            let css = format!(
                "body{{margin:0;width:300px;height:260px;background:white}}div{{width:270px;height:230px;background:rgba(200,30,90,.75);transform:translate(2px,3px);{effect}}}"
            );
            let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
                &dom,
                &css,
                300.0,
                Some(260.0),
                &text,
                &amazing_layout::NoReplacedElements,
            );
            let viewport = RasterViewport::new((300.0, 260.0), (300.0, 260.0), scale).unwrap();
            let view = DisplayListView::with_dom_generations(
                &root,
                300,
                260,
                &text,
                &NoImages,
                &NoControls,
                &dom,
            )
            .with_raster_viewport(viewport)
            .unwrap();
            let source = PaintViewSource::new(&view);
            let (w, h) = viewport.physical_size();
            let expected =
                paint_document_at_viewport(&root, viewport, &text, &NoImages, &NoControls).unwrap();
            let mut scene = source.compositor(w, h).unwrap();
            let frame = source.build_frame(&mut scene, w, h).unwrap();
            let mut presenter = ReferenceCpuPresenter::default();
            assert_eq!(
                presenter.present(&frame).unwrap(),
                expected,
                "{scale}/{effect}"
            );
            let cached = source.build_cached_frame(&mut scene, w, h).unwrap();
            assert_eq!(
                presenter.present(&cached).unwrap(),
                expected,
                "cached {scale}/{effect}"
            );
            let restored = amazing_compositor::CompositorFrame::from_effects_snapshot(
                frame.to_effects_snapshot(),
            )
            .unwrap();
            assert_eq!(
                ReferenceCpuPresenter::default().present(&restored).unwrap(),
                expected,
                "effect snapshot {scale}/{effect}"
            );
        }
    }
}
