use super::*;
use amazing_text::FallbackTextSystem;

fn fixture() -> LayoutBox {
    amazing_layout::layout_document_with_text(
        "<body><div class='red'></div><div class='blue'></div></body>",
        "body{margin:0;background:#fff}.red{width:20px;height:20px;background:red}.blue{width:20px;height:20px;background:blue}",
        48.0,
        &FallbackTextSystem::new(),
    )
}

#[test]
fn ordered_items_raster_to_the_same_document_pixels() {
    let root = fixture();
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(
        &root,
        48,
        48,
        &text,
        &crate::paint::NoImages,
        &crate::controls::NoControls,
    );
    assert!(!view.items().is_empty());
    assert!(view
        .items()
        .iter()
        .enumerate()
        .all(|(i, item)| item.index == i));
    let mut target = Surface::new(48, 48);
    target.clear([255; 4]);
    assert!(view.raster_range(
        0..view.items().len(),
        &mut target,
        (0, 0),
        PaintBounds::viewport(48, 48),
    ));
    let full = crate::paint::paint_document_with_text_resources_and_controls(
        &root,
        48,
        48,
        &text,
        &crate::paint::NoImages,
        &crate::controls::NoControls,
    );
    assert_eq!(target.pixels, full.pixels);
}

#[test]
fn range_translation_and_target_clip_select_only_requested_ink() {
    let root = fixture();
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(
        &root,
        48,
        48,
        &text,
        &crate::paint::NoImages,
        &crate::controls::NoControls,
    );
    let mut target = Surface::new(10, 10);
    let clip = PaintBounds {
        x: 0.0,
        y: 0.0,
        width: 5.0,
        height: 10.0,
    };
    assert!(view.raster_range(0..view.items().len(), &mut target, (-10, 0), clip));
    assert_eq!(target.get_pixel(2, 2), Some([255, 0, 0, 255]));
    assert_eq!(target.get_pixel(7, 2), Some([0, 0, 0, 0]));
    let before = target.pixels.clone();
    assert!(view.raster_range(0..0, &mut target, (0, 0), clip));
    assert_eq!(target.pixels, before);
    assert!(!view.raster_range(
        view.items().len()..view.items().len() + 1,
        &mut target,
        (0, 0),
        clip
    ));

    let red_only = view.items().iter().find_map(|item| {
        let mut selected = Surface::new(48, 48);
        view.raster_range(
            item.index..item.index + 1,
            &mut selected,
            (0, 0),
            PaintBounds::viewport(48, 48),
        )
        .then_some(selected)
        .filter(|selected| selected.get_pixel(2, 2) == Some([255, 0, 0, 255]))
    });
    let selected = red_only.expect("one paint item provides the red box");
    assert_eq!(selected.get_pixel(2, 25), Some([0, 0, 0, 0]));
}

#[test]
fn property_ids_and_direct_hints_follow_nested_state() {
    let root = amazing_layout::layout_document_with_text(
        "<body><div class='outer'><div class='inner'></div></div></body>",
        "body{margin:0}.outer{width:30px;height:30px;overflow:scroll;transform:translate(2px,3px);opacity:.5;will-change:transform}.inner{width:10px;height:10px;background:blue;position:sticky}",
        48.0,
        &FallbackTextSystem::new(),
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::with_video_hint(
        &root,
        48,
        48,
        &text,
        &crate::paint::NoImages,
        &crate::controls::NoControls,
        &|_| true,
    );
    assert!(view.items().iter().any(|item| item.hints.will_change));
    assert!(view.items().iter().any(|item| item.hints.scroller));
    assert!(view.items().iter().any(|item| item.hints.sticky));
    assert!(view.items().iter().any(|item| item.hints.video));
    assert!(view
        .items()
        .iter()
        .any(|item| item.properties.transform > 0));
    assert!(view.items().iter().any(|item| item.properties.clip > 0));
    assert!(view.items().iter().any(|item| item.properties.effect > 0));
    assert!(view.items().iter().any(|item| item.properties.scroll > 0));
    for item in view.items() {
        assert!((item.properties.transform as usize) < view.transforms().len());
        assert!((item.properties.clip as usize) < view.clips().len());
        assert!((item.properties.effect as usize) < view.effects().len());
        assert!((item.properties.scroll as usize) < view.scrolls().len());
    }
}
