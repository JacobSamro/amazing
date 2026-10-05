//! Clip/scroll ownership remains separate from the transform of painted ink.
use amazing_compositor::*;
use amazing_paint::{
    NoImages, RasterViewport, controls::NoControls, display_list_view::DisplayListView,
    paint_document_at_viewport,
};
use amazing_text::FallbackTextSystem;

#[test]
fn owner_scrollers_and_clips_survive_descendant_updates_and_snapshot_validation() {
    let mut trees = PropertyTrees::new();
    let owner = trees
        .add_transform(
            TransformId(0),
            Matrix4::translation(7., 9., 0.),
            (0., 0., 0.),
            false,
        )
        .unwrap();
    let child = trees
        .add_transform(
            owner,
            Matrix4::translation(-13., 11., 0.),
            (0., 0., 0.),
            false,
        )
        .unwrap();
    let root = trees
        .add_scroll_with_owner(
            ScrollId(0),
            (0., 0.),
            Rect::new(0., 0., 100., 100.),
            Rect::new(0., 0., 200., 200.),
            Some(TransformId(0)),
        )
        .unwrap();
    let nested = trees
        .add_scroll_with_owner(
            root,
            (3., 5.),
            Rect::new(0., 0., 20., 15.),
            Rect::new(0., 0., 60., 50.),
            Some(owner),
        )
        .unwrap();
    let clip = trees
        .add_clip_with_scroll(
            ClipId(0),
            owner,
            root,
            ClipShape::Rect(Rect::new(0., 0., 20., 15.)),
        )
        .unwrap();
    let state = PropertyTreeState {
        transform: child,
        scroll: nested,
        clip,
        ..Default::default()
    };
    assert!(trees.visible(state, 8., 10.));
    assert!(!trees.visible(state, 6., 10.));
    assert!(!trees.visible(state, 28., 10.));
    trees
        .set_transform(child, Matrix4::translation(80., -30., 0.))
        .unwrap();
    assert!(trees.visible(state, 8., 10.));
    trees.set_scroll(root, (2., 4.)).unwrap();
    assert!(trees.visible(state, 6., 6.));
    assert!(!trees.visible(state, 4., 6.));
    let saved = trees.to_snapshot();
    let restored = PropertyTrees::from_snapshot(saved.clone()).unwrap();
    assert_eq!(restored.scroll(nested), trees.scroll(nested));
    assert_eq!(restored.clip(clip), trees.clip(clip));
    assert_eq!(
        restored.visible(state, 6., 6.),
        trees.visible(state, 6., 6.)
    );
    for version in [1, 2, u16::MAX] {
        let mut old = saved.clone();
        old.version = version;
        assert!(matches!(
            PropertyTrees::from_snapshot(old),
            Err(SnapshotError::UnsupportedVersion)
        ));
    }
    let mut bad = saved.clone();
    bad.scrolls[nested.0].container_transform = Some(TransformId(999));
    assert!(matches!(
        PropertyTrees::from_snapshot(bad),
        Err(SnapshotError::InvalidTree)
    ));
    let mut bad = saved;
    bad.clips[clip.0].scroll = ScrollId(999);
    assert!(matches!(
        PropertyTrees::from_snapshot(bad),
        Err(SnapshotError::InvalidTree)
    ));
}

#[test]
fn finite_no_ink_content_keeps_item_identity_without_invalid_scene_chunks() {
    let text = FallbackTextSystem;
    let dom = amazing_html::parse(
        "<body><div class=clip><div class=ink></div></div><div class=empty></div></body>",
    );
    let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        "body{margin:0}.clip{width:20px;height:15px;overflow:hidden}.ink{width:25px;height:12px;background:red;transform:translate(2px,3px)}.empty{width:8px;height:7px;overflow:hidden}",
        32.,
        Some(24.),
        &text,
        &amazing_layout::NoReplacedElements,
    );
    let viewport = RasterViewport::new((32., 24.), (32., 24.), 1.5).unwrap();
    let view =
        DisplayListView::with_dom_generations(&root, 32, 24, &text, &NoImages, &NoControls, &dom)
            .with_raster_viewport(viewport)
            .unwrap();
    let empty = view
        .items()
        .iter()
        .filter(|i| {
            i.kind == amazing_paint::display_list_view::PaintItemKind::Content
                && (i.local_bounds.width == 0. || i.local_bounds.height == 0.)
        })
        .count();
    assert!(empty > 0);
    let (_, metadata) = paint_view_metadata(&view).unwrap();
    assert_eq!(metadata.len(), view.items().len());
    for (index, chunk) in metadata.iter().enumerate() {
        assert_eq!(chunk.items, index..index + 1);
    }
    let source = PaintViewSource::new(&view);
    let (w, h) = viewport.physical_size();
    let mut scene = source.compositor(w, h).unwrap();
    for chunk in scene.chunks() {
        assert!(!chunk.bounds.empty());
        assert!(chunk.items.end <= view.items().len());
        for index in chunk.items.clone() {
            assert_eq!(view.items()[index].index, index);
        }
    }
    assert!(
        view.items()
            .iter()
            .filter(
                |i| i.kind == amazing_paint::display_list_view::PaintItemKind::Content
                    && (i.local_bounds.width == 0. || i.local_bounds.height == 0.)
            )
            .all(|i| i.key.is_some())
    );
    let frame = source.build_frame(&mut scene, w, h).unwrap();
    assert_eq!(
        ReferenceCpuPresenter::default().present(&frame).unwrap(),
        paint_document_at_viewport(&root, viewport, &text, &NoImages, &NoControls).unwrap()
    );
    // Generic scene admission is unchanged: zero/negative/nonfinite ink is invalid.
    for bounds in [
        Rect::new(0., 0., 0., 1.),
        Rect::new(0., 0., -1., 1.),
        Rect::new(f32::NAN, 0., 1., 1.),
    ] {
        let chunk = PaintChunk {
            items: 0..1,
            bounds,
            state: PropertyTreeState::default(),
            reasons: CompositingReasons::default(),
        };
        assert!(matches!(
            Compositor::new(PropertyTrees::new(), vec![chunk], 1),
            Err(SceneError::NonFiniteBounds)
        ));
    }
}

#[test]
fn local_effect_visibility_uses_world_centers_for_positive_negative_and_fractional_transforms() {
    let text = FallbackTextSystem;
    for scale in [1., 1.25, 1.5, 2., 3.] {
        for translation in ["2px,3px", "-2px,-3px", "2.5px,-1.25px"] {
            for overflow in ["visible", "hidden", "auto"] {
                let dom = amazing_html::parse(
                    "<body><div class=viewport><div class=ink></div></div></body>",
                );
                let css = format!(
                    "body{{margin:0;width:300px;height:260px;background:white}}.viewport{{width:280px;height:240px;overflow:{overflow}}}.ink{{width:270px;height:230px;background:rgba(200,30,90,.75);transform:translate({translation});filter:invert(1)}}"
                );
                let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
                    &dom,
                    &css,
                    300.,
                    Some(260.),
                    &text,
                    &amazing_layout::NoReplacedElements,
                );
                let viewport = RasterViewport::new((300., 260.), (300., 260.), scale).unwrap();
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
                let mut scene = source.compositor(w, h).unwrap();
                let frame = source.build_frame(&mut scene, w, h).unwrap();
                let expected =
                    paint_document_at_viewport(&root, viewport, &text, &NoImages, &NoControls)
                        .unwrap();
                let label = format!("{scale}/{translation}/{overflow}");
                let mut presenter = ReferenceCpuPresenter::default();
                assert_eq!(presenter.present(&frame).unwrap(), expected, "{label}");
                let cached = source.build_cached_frame(&mut scene, w, h).unwrap();
                assert!(cached.resources.uploads.is_empty());
                assert_eq!(
                    presenter.present(&cached).unwrap(),
                    expected,
                    "cached {label}"
                );
                let restored =
                    CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
                assert_eq!(
                    ReferenceCpuPresenter::default().present(&restored).unwrap(),
                    expected,
                    "snapshot {label}"
                );
            }
        }
    }
}
