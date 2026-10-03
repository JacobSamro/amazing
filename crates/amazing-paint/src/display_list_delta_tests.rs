use super::*;
use amazing_dom::NodeIdx;
use amazing_layout::{Display, Flow, Rect};
use amazing_text::FallbackTextSystem;

fn box_with(node: u32, x: f32, color: &str) -> LayoutBox {
    let mut style = amazing_css::ComputedStyle::default();
    style.set("background-color", color);
    let mut box_ = LayoutBox::new(Some(NodeIdx(node)), Display::Block, style, 16.0, 19.2);
    box_.content_box = Rect { x, y: 0.0, w: 12.0, h: 12.0 };
    box_.border_box = box_.content_box;
    box_
}

fn root(mut children: Vec<LayoutBox>) -> LayoutBox {
    let mut root = box_with(1, 0.0, "transparent");
    root.content_box = Rect { x: 0.0, y: 0.0, w: 64.0, h: 32.0 };
    root.border_box = root.content_box;
    for (ordinal, child) in children.iter_mut().enumerate() {
        child.source_order = Some(ordinal as u64);
    }
    root.flow = Flow::Blocks(children);
    root
}

fn view<'a>(root: &'a LayoutBox, text: &'a FallbackTextSystem) -> DisplayListView<'a> {
    DisplayListView::with_node_generations(
        root, 64, 32, text, &crate::paint::NoImages, &crate::controls::NoControls,
        &|_| Some(1),
    )
}

fn full_pixels(view: &DisplayListView<'_>) -> Vec<u8> {
    let mut surface = Surface::new(64, 32);
    surface.clear([255; 4]);
    assert!(view.raster_range(0..view.items().len(), &mut surface, (0, 0), PaintBounds::viewport(64, 32)));
    surface.pixels
}

#[test]
fn stable_keys_and_tile_edge_damage_match_fresh_full_view() {
    let old_root = root(vec![box_with(2, 15.0, "red"), box_with(3, 40.0, "blue")]);
    let new_root = root(vec![box_with(2, 17.0, "green"), box_with(3, 40.0, "blue")]);
    let text = FallbackTextSystem::new();
    let old = view(&old_root, &text);
    let new = view(&new_root, &text);
    let old_keys: Vec<_> = old.items().iter().map(|item| item.key).collect();
    let new_keys: Vec<_> = new.items().iter().map(|item| item.key).collect();
    assert_eq!(old_keys, new_keys);
    let PaintViewUpdate::Delta(delta) = old.update(&new) else { panic!("safe item change must return a delta") };
    assert_eq!(delta.changes.len(), 1);
    assert_eq!(delta.changed_ranges, [2..3]);
    assert!(delta.changes[0].old_bounds.x < 16.0);
    assert!(delta.changes[0].new_bounds.x + delta.changes[0].new_bounds.width > 16.0);
    assert_ne!(full_pixels(&old), full_pixels(&new));
    let oracle = crate::paint::paint_document_with_text_resources_and_controls(
        &new_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
    );
    assert_eq!(full_pixels(&new), oracle.pixels);
}

#[test]
fn removed_reused_and_reordered_nodes_require_fresh_view() {
    let old_root = root(vec![box_with(2, 0.0, "red"), box_with(3, 20.0, "blue")]);
    let removed = root(vec![box_with(3, 20.0, "blue")]);
    let reordered = root(vec![box_with(3, 20.0, "blue"), box_with(2, 0.0, "red")]);
    let text = FallbackTextSystem::new();
    let old = view(&old_root, &text);
    for new_root in [&removed, &reordered] {
        let new = view(new_root, &text);
        assert!(matches!(old.update(&new), PaintViewUpdate::RequiresFullRebuild { reason: PaintFallbackReason::OrderChanged, .. }));
        let oracle = crate::paint::paint_document_with_text_resources_and_controls(
            new_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
        );
        assert_eq!(full_pixels(&new), oracle.pixels);
    }
    let reused = DisplayListView::with_node_generations(
        &old_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
        &|node| Some(if node == NodeIdx(2) { 2 } else { 1 }),
    );
    assert!(matches!(old.update(&reused), PaintViewUpdate::RequiresFullRebuild { reason: PaintFallbackReason::OrderChanged, .. }));
}

#[test]
fn missing_generation_and_property_changes_fall_back() {
    let old_root = root(vec![box_with(2, 0.0, "red")]);
    let mut moved = root(vec![box_with(2, 0.0, "red")]);
    if let Flow::Blocks(children) = &mut moved.flow {
        children[0].style.set("transform", "translate(2px, 0px)");
    }
    let text = FallbackTextSystem::new();
    let old = view(&old_root, &text);
    let no_generation = DisplayListView::new(&old_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls);
    assert!(matches!(old.update(&no_generation), PaintViewUpdate::RequiresFullRebuild { reason: PaintFallbackReason::AmbiguousGeneration, .. }));
    let new = view(&moved, &text);
    assert!(matches!(old.update(&new), PaintViewUpdate::RequiresFullRebuild { reason: PaintFallbackReason::PropertyStateChanged, .. }));
    let oracle = crate::paint::paint_document_with_text_resources_and_controls(
        &moved, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
    );
    assert_eq!(full_pixels(&new), oracle.pixels);
}

#[test]
fn retired_background_resource_is_reported() {
    let mut old_box = box_with(2, 0.0, "transparent");
    old_box.style.set("background-image", "url(old)");
    let mut new_box = box_with(2, 0.0, "transparent");
    new_box.style.set("background-image", "url(new)");
    let old_root = root(vec![old_box]);
    let new_root = root(vec![new_box]);
    let text = FallbackTextSystem::new();
    let old = view(&old_root, &text);
    let new = view(&new_root, &text);
    let PaintViewUpdate::Delta(delta) = old.update(&new) else { panic!("resource change remains item-local") };
    assert_eq!(delta.retired_resources, [PaintResourceKey::BackgroundUrl("old".into())]);
}

#[test]
fn identical_relative_backgrounds_retain_distinct_stylesheet_resource_lifetimes() {
    let sourced=|base:&str| {
        let mut dom=amazing_dom::Dom::new();let node=dom.create_element(NodeIdx(0),"div");
        let mut sheet=amazing_css::parse_stylesheet("div{background-image:url(pixel.png)}");
        sheet.base_url=Some(base.into());
        let styles=amazing_css::compute(&dom,&[sheet]);
        let mut box_=box_with(2,0.0,"transparent");box_.style=styles.get(node).unwrap().clone();box_
    };
    let old_root=root(vec![sourced("https://one.test/sheet.css")]);
    let new_root=root(vec![sourced("https://two.test/sheet.css")]);
    let text=FallbackTextSystem::new();let old=view(&old_root,&text);let new=view(&new_root,&text);
    let old_resource=PaintResourceKey::BackgroundUrlWithBase {url:"pixel.png".into(),source_base:"https://one.test/sheet.css".into()};
    let PaintViewUpdate::Delta(delta)=old.update(&new) else {panic!("base change preserves item identity")};
    assert_eq!(delta.retired_resources,[old_resource.clone()]);
    let mut retained=crate::retained_paint::RetainedPaintView::from_view(&old).unwrap();
    let changed=*new.items().iter().find(|item|item.key.is_some_and(|key|key.node.node==NodeIdx(2))
        &&item.kind==crate::display_list_view::PaintItemKind::Decorations).unwrap();
    let Flow::Blocks(children)=&new_root.flow else {unreachable!()};
    let delta=retained.apply_damage(&[crate::retained_paint::PaintDamage::Upsert {item:changed,box_:&children[0]}],
        &text,&crate::paint::NoImages).unwrap();
    assert_eq!(delta.retired_resources,[old_resource]);
}

#[test]
fn text_mutation_is_checked_against_full_oracle() {
    let text = FallbackTextSystem::new();
    let old_root = amazing_layout::layout_document_with_text(
        "<body><p>Hello</p></body>", "body{margin:0}p{margin:0}", 64.0, &text,
    );
    let new_root = amazing_layout::layout_document_with_text(
        "<body><p>Goodbye world</p></body>", "body{margin:0}p{margin:0}", 64.0, &text,
    );
    let old = view(&old_root, &text);
    let new = view(&new_root, &text);
    assert!(matches!(old.update(&new), PaintViewUpdate::RequiresFullRebuild {
        reason: PaintFallbackReason::PropertyStateChanged, ..
    }));
    let oracle = crate::paint::paint_document_with_text_resources_and_controls(
        &new_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
    );
    assert_eq!(full_pixels(&new), oracle.pixels);
}

#[test]
fn scroll_offset_and_stacking_order_use_full_fallback() {
    let mut low = box_with(2, 0.0, "red");
    low.style.set("position", "absolute");
    low.style.set("z-index", "1");
    let mut high = box_with(3, 0.0, "blue");
    high.style.set("position", "absolute");
    high.style.set("z-index", "2");
    let old_root = root(vec![low, high]);
    let mut new_low = box_with(2, 0.0, "red");
    new_low.style.set("position", "absolute");
    new_low.style.set("z-index", "3");
    let mut new_high = box_with(3, 0.0, "blue");
    new_high.style.set("position", "absolute");
    new_high.style.set("z-index", "2");
    let new_root = root(vec![new_low, new_high]);
    let text = FallbackTextSystem::new();
    let old = view(&old_root, &text);
    let new = view(&new_root, &text);
    assert!(matches!(old.update(&new), PaintViewUpdate::RequiresFullRebuild { .. }));
    let oracle = crate::paint::paint_document_with_text_resources_and_controls(
        &new_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
    );
    assert_eq!(full_pixels(&new), oracle.pixels);

    let old_scroll = DisplayListView::with_node_generations_and_scroll_offsets(
        &old_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
        &|_| false, &|_| Some((0.0, 0.0)), &|_| Some(1),
    );
    let new_scroll = DisplayListView::with_node_generations_and_scroll_offsets(
        &old_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
        &|_| false, &|_| Some((0.0, 4.0)), &|_| Some(1),
    );
    assert!(matches!(old_scroll.update(&new_scroll), PaintViewUpdate::RequiresFullRebuild {
        reason: PaintFallbackReason::PropertyStateChanged, ..
    }));
}

#[test]
fn changed_replaced_pixels_keep_key_and_damage_content_item() {
    struct NodeImage(amazing_media::Image);
    impl crate::paint::ImageProvider for NodeImage {
        fn image(&self, _node: NodeIdx) -> Option<&amazing_media::Image> { Some(&self.0) }
    }
    let mut child = box_with(2, 0.0, "transparent");
    child.replaced_size = Some(amazing_layout::IntrinsicSize { width: 1.0, height: 1.0 });
    let root = root(vec![child]);
    let text = FallbackTextSystem::new();
    let red = NodeImage(amazing_media::Image { width: 1, height: 1, pixels: vec![255, 0, 0, 255] });
    let blue = NodeImage(amazing_media::Image { width: 1, height: 1, pixels: vec![0, 0, 255, 255] });
    let old = DisplayListView::with_node_generations(&root, 64, 32, &text, &red, &crate::controls::NoControls, &|_| Some(1));
    let new = DisplayListView::with_node_generations(&root, 64, 32, &text, &blue, &crate::controls::NoControls, &|_| Some(1));
    let PaintViewUpdate::Delta(delta) = old.update(&new) else { panic!("resource pixels should damage the item") };
    assert_eq!(delta.changes.len(), 1);
    assert_ne!(full_pixels(&old), full_pixels(&new));
}

#[test]
fn dom_generation_resolver_is_used_for_item_keys() {
    let mut dom = amazing_dom::Dom::new();
    let root_node = dom.create_element(NodeIdx(0), "div");
    let child_node = dom.create_element(root_node, "div");
    assert_eq!((root_node, child_node), (NodeIdx(1), NodeIdx(2)));
    let root = root(vec![box_with(child_node.0, 0.0, "red")]);
    let text = FallbackTextSystem::new();
    let view = DisplayListView::with_dom_generations(
        &root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls, &dom,
    );
    assert!(view.items().iter().all(|item| item.key.is_some_and(|key|
        key.node.generation == dom.node_generation(key.node.node).unwrap())));
}

#[test]
fn changed_opacity_group_and_invalid_ink_bounds_fall_back() {
    let mut old_child = box_with(2, 0.0, "red");
    old_child.style.set("opacity", "0.5");
    let mut new_child = box_with(2, 0.0, "blue");
    new_child.style.set("opacity", "0.5");
    let old_root = root(vec![old_child]);
    let new_root = root(vec![new_child]);
    let text = FallbackTextSystem::new();
    let old = view(&old_root, &text);
    let new = view(&new_root, &text);
    assert!(matches!(old.update(&new), PaintViewUpdate::RequiresFullRebuild {
        reason: PaintFallbackReason::UnseparableGroup, ..
    }));
    let oracle = crate::paint::paint_document_with_text_resources_and_controls(
        &new_root, 64, 32, &text, &crate::paint::NoImages, &crate::controls::NoControls,
    );
    assert_eq!(full_pixels(&new), oracle.pixels);

    let mut invalid = root(vec![box_with(2, f32::NAN, "blue")]);
    if let Flow::Blocks(children) = &mut invalid.flow {
        children[0].border_box.x = f32::NAN;
    }
    let valid_root = root(vec![box_with(2, 0.0, "red")]);
    let valid = view(&valid_root, &text);
    let bad = view(&invalid, &text);
    assert!(matches!(valid.update(&bad), PaintViewUpdate::RequiresFullRebuild {
        reason: PaintFallbackReason::UnsafeBounds, ..
    }));
}
