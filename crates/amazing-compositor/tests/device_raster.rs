use amazing_compositor::{PaintViewRasterMode, PaintViewSource, Presenter, ReferenceCpuPresenter};
use amazing_paint::controls::NoControls;
use amazing_paint::display_list_view::{DisplayListView, RetainedPaintView};
use amazing_paint::{NoImages, RasterViewport, paint_document_at_viewport};
use amazing_text::FallbackTextSystem;
#[test]
fn physical_tiles_full_retained_and_cached_frames_share_one_device_lattice() {
    let text = FallbackTextSystem;
    let dom = amazing_html::parse("<body><div></div></body>");
    let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        "body{margin:0}div{width:20px;height:12px;background:red;transform:translate(2px,3px)}",
        32.0,
        Some(24.0),
        &text,
        &amazing_layout::NoReplacedElements,
    );
    for scale in [1.25, 1.5, 2.0, 3.0] {
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
        let mut compositor = source.compositor(w, h).unwrap();
        let frame = source.build_frame(&mut compositor, w, h).unwrap();
        let expected =
            paint_document_at_viewport(&root, viewport, &text, &NoImages, &NoControls).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        let actual = presenter.present(&frame).unwrap();
        assert_eq!(actual, expected, "density {scale}");
        let cached = source.build_cached_frame(&mut compositor, w, h).unwrap();
        assert!(cached.resources.uploads.is_empty());
        assert!(matches!(
            ReferenceCpuPresenter::default().present(&cached),
            Err(amazing_compositor::PresenterError::MissingResource(_))
        ));
        assert_eq!(presenter.present(&cached).unwrap(), expected);
        let retained = RetainedPaintView::from_view(&view).unwrap();
        for item in retained.items() {
            let raster = retained.raster(item.key.unwrap()).unwrap();
            let mut oracle =
                amazing_paint::Surface::new(raster.surface.width, raster.surface.height);
            view.raster_raw_local(item.index..item.index + 1, &mut oracle, raster.origin)
                .unwrap();
            assert_eq!(&oracle, raster.surface);
        }
        assert!(source.compositor(w + 1, h).is_err());
    }
}
#[test]
fn generated_background_uses_physical_baked_raster_without_css_bitmap_upscale() {
    let text = FallbackTextSystem;
    let dom = amazing_html::parse("<body><div></div></body>");
    let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        "body{margin:0}div{width:20px;height:12px;background-image:linear-gradient(to right,red 0px,blue 20px)}",
        32.0,
        Some(24.0),
        &text,
        &amazing_layout::NoReplacedElements,
    );
    let v = RasterViewport::new((32.0, 24.0), (32.0, 24.0), 2.0).unwrap();
    let view =
        DisplayListView::with_dom_generations(&root, 32, 24, &text, &NoImages, &NoControls, &dom)
            .with_raster_viewport(v)
            .unwrap();
    let source = PaintViewSource::new(&view);
    assert_eq!(source.mode(), PaintViewRasterMode::BakedFallback);
    let (w, h) = v.physical_size();
    let mut compositor = source.compositor(w, h).unwrap();
    let frame = source.build_frame(&mut compositor, w, h).unwrap();
    assert_eq!(
        ReferenceCpuPresenter::default().present(&frame).unwrap(),
        paint_document_at_viewport(&root, v, &text, &NoImages, &NoControls).unwrap()
    );
}
