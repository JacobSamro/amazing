use amazing_compositor::*;
use amazing_paint::{controls::NoControls, display_list_view::DisplayListView, NoImages};
use amazing_text::FallbackTextSystem;

fn input(effect: EffectId) -> PaintFilterInput {
    PaintFilterInput {
        effect, space: PaintFilterSpace::Raster, filters: vec![],
        backdrop_filters: vec![], backdrop_bounds: None, clip_path: None,
    }
}

#[test]
fn filter_import_directory_preserves_values_and_cached_pixels() {
    use FilterOperation as F;
    for (filter, expected) in [
        (F::Brightness(2.), [255, 200, 100, 255]),
        (F::Contrast(0.), [128, 128, 128, 255]),
        (F::Invert(1.), [55, 155, 205, 255]),
        (F::Opacity(0.5), [110, 70, 55, 255]),
        (F::Grayscale(0.), [200, 100, 50, 255]),
        (F::HueRotate(0.), [200, 100, 50, 255]),
        (F::Saturate(1.), [200, 100, 50, 255]),
        (F::Sepia(0.), [200, 100, 50, 255]),
        (F::Blur(0.), [200, 100, 50, 255]),
        (F::DropShadow { offset: (0., 0.), sigma: 0., color: [0; 4] },
            [200, 100, 50, 255]),
    ] {
        let text = FallbackTextSystem::new();
        let root = amazing_layout::layout_document_with_text(
            "<body><div></div></body>",
            "body{margin:0;background:rgb(20,40,60)}div{width:32px;height:32px;isolation:isolate;background:rgb(200,100,50)}",
            64., &text);
        let view = DisplayListView::new(&root,64,64,&text,&NoImages,&NoControls);
        let id = EffectId(view.effects().len()-1);
        let mut item = input(id);
        item.filters = vec![filter];
        let mut provider = vec![item];
        let source = PaintViewSource::with_filters(&view, &provider).unwrap();
        provider[0].filters.clear(); // input is snapshotted at construction
        let mut compositor = source.compositor(64,64).unwrap();
        let ops = compositor.trees().effect_operations(id).unwrap();
        assert_eq!(ops.filters, [filter]);
        assert!(ops.isolate);
        let frame = source.build_frame(&mut compositor,64,64).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        let actual = presenter.present(&frame).unwrap();
        assert_eq!(actual.get_pixel(8,8), Some(expected), "{filter:?}");
        let cached = source.build_cached_frame(&mut compositor,64,64).unwrap();
        assert_eq!(presenter.present(&cached).unwrap(), actual);
    }
}

#[test]
fn backdrop_import_clips_to_resolved_border_box_and_reuses_tiles() {
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text(
        "<body><div></div></body>",
        "body{margin:0;background:rgb(20,40,60)}div{width:32px;height:32px;isolation:isolate}",
        64., &text);
    let view = DisplayListView::new(&root,64,64,&text,&NoImages,&NoControls);
    let id = EffectId(view.effects().len()-1);
    let mut item = input(id);
    item.backdrop_filters = vec![FilterOperation::Invert(1.)];
    item.backdrop_bounds = Some(Rect::new(4.,4.,16.,16.));
    let source = PaintViewSource::with_filters(&view,&vec![item]).unwrap();
    let mut compositor = source.compositor(64,64).unwrap();
    let frame = source.build_frame(&mut compositor,64,64).unwrap();
    let mut presenter = ReferenceCpuPresenter::default();
    let actual = presenter.present(&frame).unwrap();
    assert_eq!(actual.get_pixel(8,8), Some([235,215,195,255]));
    assert_eq!(actual.get_pixel(2,2), Some([20,40,60,255]));
    let cached = source.build_cached_frame(&mut compositor,64,64).unwrap();
    assert_eq!(presenter.present(&cached).unwrap(), actual);
}

#[test]
fn manual_clip_input_imports_and_renders_the_shape() {
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text(
        "<body><div></div></body>",
        "body{margin:0;background:rgb(20,40,60)}div{width:32px;height:32px;isolation:isolate;background:rgb(200,100,50)}",
        64., &text);
    let view = DisplayListView::new(&root,64,64,&text,&NoImages,&NoControls);
    let id = EffectId(view.effects().len()-1);
    let mut item = input(id);
    item.clip_path = Some(EffectClip::Inset { rect: Rect::new(0.,0.,16.,32.), radii: [0.;4] });
    let source = PaintViewSource::with_filters(&view,&vec![item]).unwrap();
    let mut compositor = source.compositor(64,64).unwrap();
    let ops = compositor.trees().effect_operations(id).unwrap();
    assert!(ops.clip_path.is_some());
    let frame = source.build_frame(&mut compositor,64,64).unwrap();
    let actual = ReferenceCpuPresenter::default().present(&frame).unwrap();
    assert_eq!(actual.get_pixel(8,8), Some([200,100,50,255]), "inside the clipped half");
    assert_eq!(actual.get_pixel(24,8), Some([20,40,60,255]), "outside the clipped half");
}

#[test]
fn invalid_filter_batches_never_escape_and_plain_import_is_unchanged() {
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text("<body><div></div></body>",
        "div{width:32px;height:32px;opacity:.5;mix-blend-mode:screen;isolation:isolate}",64.,&text);
    let view = DisplayListView::new(&root,64,64,&text,&NoImages,&NoControls);
    let id = EffectId(view.effects().len()-1);
    let valid = input(id);
    let mut variants = vec![input(EffectId(0)), input(EffectId(usize::MAX))];
    let mut local = valid.clone(); local.space=PaintFilterSpace::Local; variants.push(local);
    let mut missing = valid.clone(); missing.backdrop_filters=vec![FilterOperation::Blur(1.)]; variants.push(missing);
    for bound in [Rect::new(f32::NAN,0.,1.,1.),Rect::new(0.,0.,f32::INFINITY,1.),Rect::new(0.,0.,0.,1.)] {
        let mut v=valid.clone();v.backdrop_bounds=Some(bound);variants.push(v);
    }
    let mut negative=valid.clone();negative.filters=vec![FilterOperation::Blur(-1.)];variants.push(negative);
    let mut long=valid.clone();long.filters=vec![FilterOperation::Invert(1.);33];variants.push(long);
    for bad in variants {
        assert!(PaintViewSource::with_filters(&view,&vec![bad]).is_err());
    }
    assert!(PaintViewSource::with_filters(&view,&vec![valid.clone(),valid]).is_err());
    let (plain,_) = paint_view_metadata(&view).unwrap();
    let (empty,_) = paint_view_metadata_with_filters(&view,&[]).unwrap();
    assert_eq!(plain.effect_operations(id), empty.effect_operations(id));
    assert_eq!(empty.effect_operations(id).unwrap().blend, EffectBlend::Screen);
    assert!(empty.effect_operations(id).unwrap().isolate);
}
