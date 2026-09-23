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

#[test]
fn content_bounds_scroll_extent_and_sticky_constraints_are_local() {
    let root = amazing_layout::layout_document_with_text(
        "<body><div class='scroller'><div class='sticky'>S</div><div class='overflow'></div></div></body>",
        "body{margin:0}.scroller{width:40px;height:30px;overflow:scroll}.sticky{position:sticky;top:5px;width:20px;height:10px;font-size:4px}.overflow{width:90px;height:70px;background:blue}",
        120.0,
        &FallbackTextSystem::new(),
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(&root, 120, 120, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    let scroll = view.scrolls().iter().skip(1).next().expect("scroller");
    assert_eq!(scroll.initial_offset, (0.0, 0.0));
    assert!(scroll.contents.width > scroll.bounds.width);
    assert!(scroll.contents.height > scroll.bounds.height);
    let sticky = view.items().iter().find_map(|item| item.sticky).expect("sticky metadata");
    assert_eq!(sticky.top, Some(5.0));
    assert!(sticky.scroll > 0 && (sticky.scroll as usize) < view.scrolls().len());
    assert!(sticky.containing_block.width >= sticky.normal.width);
    let content = view.items().iter().find(|item| item.kind == PaintItemKind::Content && item.local_bounds.width > 0.0).unwrap();
    assert!(content.local_bounds.width < 120.0);

    let offsets = DisplayListView::with_scroll_offsets(
        &root, 120, 120, &text, &crate::paint::NoImages, &crate::controls::NoControls,
        &|_| false, &|_| Some((3.0, 7.0)),
    );
    assert!(offsets.scrolls().iter().skip(1).all(|node| node.initial_offset == (3.0, 7.0)));
}
#[test]
fn raw_tile_raster_skips_group_effects_and_uses_tile_origin() {
    let root = amazing_layout::layout_document_with_text(
        "<body><div class='box'></div></body>",
        "body{margin:0}.box{width:20px;height:20px;background:red;transform:translate(5px,0);opacity:.5}",
        48.0,
        &FallbackTextSystem::new(),
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(&root, 48, 48, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    let item = view.items().iter().find(|item| item.kind == PaintItemKind::Decorations && item.properties.transform != 0).unwrap();
    let mut tile = Surface::new(8, 8);
    view.raster_raw_local(item.index..item.index + 1, &mut tile, (10, 0)).unwrap();
    assert_eq!(tile.get_pixel(2, 2), Some([255, 0, 0, 255]));
    let mut baked = Surface::new(48, 48);
    assert!(view.raster_range(item.index..item.index + 1, &mut baked, (0, 0), PaintBounds::viewport(48, 48)));
    assert_ne!(baked.get_pixel(2, 2), Some([255, 0, 0, 255]));
}

#[test]
fn raw_tile_rejects_unseparable_control_without_mutating_target() {
    let root = amazing_layout::layout_document_with_text(
        "<body><input type='checkbox'></body>", "body{margin:0}", 48.0, &FallbackTextSystem::new(),
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(&root, 48, 48, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    let item = view.items().iter().find(|item| item.kind == PaintItemKind::Control).unwrap();
    let mut tile = Surface::new(8, 8);
    tile.clear([7, 11, 13, 17]);
    let before = tile.clone();
    assert_eq!(view.raster_raw_local(item.index..item.index + 1, &mut tile, (0, 0)), Err(RawRasterError::UnseparableItem(item.index)));
    assert_eq!(tile, before);
}

#[test]
fn raw_tile_rasterizes_shaped_text_at_tile_origin() {
    let root = amazing_layout::layout_document_with_text(
        "<body><p>TEXT</p></body>", "body{margin:0}p{margin:0;color:black}", 48.0, &FallbackTextSystem::new(),
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(&root, 48, 48, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    let item = view.items().iter().find(|item| item.kind == PaintItemKind::Content && matches!(&view.commands[view.item_commands[item.index]], PaintCommand::PaintContent(box_) if box_.lines.iter().any(|line| !line.glyphs.is_empty()))).unwrap();
    let mut raw = Surface::new(48, 48);
    view.raster_raw_local(item.index..item.index + 1, &mut raw, (0, 0)).unwrap();
    let mut baked = Surface::new(48, 48);
    assert!(view.raster_range(item.index..item.index + 1, &mut baked, (0, 0), PaintBounds::viewport(48, 48)));
    assert!(raw.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
    assert_eq!(raw.pixels, baked.pixels);
}

#[test]
fn raw_tile_leaves_ancestor_clip_to_compositor() {
    let root = amazing_layout::layout_document_with_text(
        "<body><div class='outer'><div class='inner'></div></div></body>",
        "body{margin:0}.outer{width:10px;height:10px;overflow:hidden}.inner{width:20px;height:10px;background:blue}",
        32.0,
        &FallbackTextSystem::new(),
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(&root, 32, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    let item = view.items().iter().find(|item| item.kind == PaintItemKind::Decorations && item.properties.clip != 0 && matches!(&view.commands[view.item_commands[item.index]], PaintCommand::PaintDecorations(box_, _) if box_.style.get("background-color") == Some("blue"))).unwrap();
    let mut raw = Surface::new(20, 10);
    view.raster_raw_local(item.index..item.index + 1, &mut raw, (0, 0)).unwrap();
    assert_eq!(raw.get_pixel(15, 2), Some([0, 0, 255, 255]));
    let mut baked = Surface::new(20, 10);
    assert!(view.raster_range(item.index..item.index + 1, &mut baked, (0, 0), PaintBounds::viewport(20, 10)));
    assert_eq!(baked.get_pixel(15, 2), Some([0, 0, 0, 0]));
}

#[test]
fn generated_source_order_reorders_boxes_without_changing_dom_fallback() {
    use amazing_layout::Flow;
    fn set_order(box_: &mut LayoutBox) {
        match box_.style.get("background-color") {
            Some("red") => box_.source_order = Some(100),
            Some("blue") => box_.source_order = Some(1),
            _ => {}
        }
        if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &mut box_.flow {
            for child in children { set_order(child); }
        }
        for child in &mut box_.positioned { set_order(child); }
    }
    fn colors(view: &DisplayListView<'_>) -> Vec<String> {
        view.items().iter().filter_map(|item| match &view.commands[view.item_commands[item.index]] {
            PaintCommand::PaintDecorations(box_, _) => box_.style.get("background-color")
                .filter(|color| matches!(*color, "red" | "blue")).map(str::to_owned),
            _ => None,
        }).collect()
    }
    let mut root = fixture();
    let text = FallbackTextSystem::new();
    let normal = DisplayListView::new(&root, 48, 48, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    assert_eq!(colors(&normal), ["red", "blue"]);
    drop(normal);
    set_order(&mut root);
    let generated = DisplayListView::new(&root, 48, 48, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    assert_eq!(colors(&generated), ["blue", "red"]);
}

#[test]
fn display_list_follows_insert_before_instead_of_arena_id() {
    use amazing_dom::{Dom, NodeIdx};

    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let old = dom.create_element(body, "div");
    dom.set_attrs(old, vec![("class".into(), "red".into())]);
    let inserted = dom.create_element(body, "div");
    dom.set_attrs(inserted, vec![("class".into(), "blue".into())]);
    dom.detach(inserted);
    dom.attach_before(body, Some(old), inserted);
    assert!(inserted.0 > old.0);

    let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        "body{margin:0}.red,.blue{width:20px;height:20px}.red{background:red}.blue{background:blue}",
        48.0,
        Some(48.0),
        &FallbackTextSystem::new(),
        &amazing_layout::NoReplacedElements,
    );
    let text = FallbackTextSystem::new();
    let view = DisplayListView::new(&root, 48, 48, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    let painted = view.items().iter().filter_map(|item| match &view.commands[view.item_commands[item.index]] {
        PaintCommand::PaintDecorations(box_, _) => box_.style.get("background-color")
            .filter(|color| matches!(*color, "red" | "blue"))
            .map(str::to_owned),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(painted, ["blue", "red"]);
}
