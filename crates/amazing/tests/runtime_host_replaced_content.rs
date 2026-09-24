#[path = "../src/runtime_host_replaced_content.rs"]
mod runtime_host_replaced_content;

use amazing_dom::{Dom, NodeIdx};
use amazing_layout::{Display, Flow, LayoutBox, Rect};
use amazing_paint::display_list_view::{DisplayListView, PaintFallbackReason, PaintItemKind};
use amazing_text::FallbackTextSystem;
use runtime_host_replaced_content::{
    apply_layout_paint_damage, seed_retained_paint, LayoutPaintDamage,
};

#[test]
fn picture_source_mutation_invalidates_only_following_images() {
    let mut dom = Dom::new();
    let picture = dom.create_element(NodeIdx(0), "picture");
    let before = dom.create_element(picture, "img");
    let source = dom.create_element(picture, "source");
    let after = dom.create_element(picture, "img");
    dom.set_attribute(source, "srcset", "new.webp").unwrap();
    let mutations = dom.take_image_relevant_mutations();
    let affected = runtime_host_replaced_content::affected_images(&dom, &mutations);
    assert!(!affected.contains(&before));
    assert!(affected.contains(&after));
}

#[test]
fn picture_selects_only_preceding_compatible_source() {
    let dom = amazing_html::parse(
        "<picture><source srcset='wide.webp 1x, wide2.webp 2x' media='(min-width: 600px)' type='image/webp'><img src='fallback.png' srcset='small.png 1x'><source srcset='late.webp' type='image/webp'></picture>",
    );
    fn find_img(dom: &Dom, node: NodeIdx) -> Option<NodeIdx> {
        if dom.name(node) == Some("img") { return Some(node); }
        dom.children(node).find_map(|child| find_img(dom, child))
    }
    let img = find_img(&dom, NodeIdx(0)).unwrap();
    let env = amazing_media::responsive::SourceEnvironment {
        viewport_width: 800.0, device_pixel_ratio: 2.0,
        slot_width: None, allow_auto_sizes: false,
    };
    let selected = runtime_host_replaced_content::select_image_source(
        &dom, img, env,
        |query| runtime_host_replaced_content::matches_image_media(query, 800.0),
        |mime| runtime_host_replaced_content::supports_image_type(mime, true),
    ).unwrap();
    assert_eq!(selected.url, "wide2.webp");
    assert!(selected.from_picture);
    assert!(!runtime_host_replaced_content::supports_image_type("image/avif", true));
}

#[test]
fn lazy_request_starts_after_viewport_intersection_and_rejects_stale_decode() {
    use amazing_media::loading::{ImageRequest, Loading, ViewportRect};
    use runtime_host_replaced_content::{ImageDecodeDecision, ImageFetchDecision, decode_image_decision, schedule_image_fetch};
    let mut request = ImageRequest::default();
    let viewport = ViewportRect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
    let below = ViewportRect { x: 0.0, y: 900.0, width: 20.0, height: 20.0 };
    assert_eq!(schedule_image_fetch(&mut request, Some("a.png"), Loading::Lazy, Some(below), viewport, 0.0), ImageFetchDecision::Deferred);
    assert!(!request.fetch_started);
    assert_eq!(schedule_image_fetch(&mut request, Some("a.png"), Loading::Lazy, Some(below), viewport, 300.0), ImageFetchDecision::Start);
    assert_eq!(decode_image_decision(&request), ImageDecodeDecision::Wait(request.generation));
    let old = request.generation;
    request.select(Some("b.png"));
    assert!(!request.finish(old, true));
}

fn box_with(node: NodeIdx, x: f32, color: &str) -> LayoutBox {
    let mut style = amazing_css::ComputedStyle::default();
    style.set("background-color", color);
    let mut box_ = LayoutBox::new(Some(node), Display::Block, style, 16.0, 19.2);
    box_.content_box = Rect {
        x,
        y: 0.0,
        w: 12.0,
        h: 12.0,
    };
    box_.border_box = box_.content_box;
    box_
}

fn root(root_node: NodeIdx, children: Vec<LayoutBox>) -> LayoutBox {
    let mut root = box_with(root_node, 0.0, "transparent");
    root.content_box = Rect {
        x: 0.0,
        y: 0.0,
        w: 64.0,
        h: 32.0,
    };
    root.border_box = root.content_box;
    root.flow = Flow::Blocks(children);
    root
}

#[test]
fn synthetic_layout_damage_uses_dom_generation_and_only_changes_one_raster() {
    let mut dom = Dom::new();
    let root_node = dom.create_element(NodeIdx(0), "body");
    let red_node = dom.create_element(root_node, "div");
    let blue_node = dom.create_element(root_node, "div");
    let old_root = root(
        root_node,
        vec![
            box_with(red_node, 15.0, "red"),
            box_with(blue_node, 40.0, "blue"),
        ],
    );
    let new_root = root(
        root_node,
        vec![
            box_with(red_node, 17.0, "green"),
            box_with(blue_node, 40.0, "blue"),
        ],
    );
    let text = FallbackTextSystem::new();
    let mut retained = seed_retained_paint(
        &old_root,
        64,
        32,
        &text,
        &amazing_paint::NoImages,
        &amazing_paint::controls::NoControls,
        &dom,
    )
    .unwrap();
    let next = DisplayListView::with_dom_generations(
        &new_root,
        64,
        32,
        &text,
        &amazing_paint::NoImages,
        &amazing_paint::controls::NoControls,
        &dom,
    );
    let mut changed = *next
        .items()
        .iter()
        .find(|item| {
            item.key.is_some_and(|key| key.node.node == red_node)
                && item.kind == PaintItemKind::Decorations
        })
        .unwrap();
    let old_key = changed.key.unwrap();
    changed.key = None; // layout passes geometry; the adapter resolves identity.
    let Flow::Blocks(children) = &new_root.flow else {
        unreachable!()
    };
    let delta = apply_layout_paint_damage(
        &mut retained,
        &[LayoutPaintDamage::Upsert {
            box_: &children[0],
            item: changed,
            ordinal: old_key.ordinal,
        }],
        &dom,
        &text,
        &amazing_paint::NoImages,
    )
    .unwrap();
    assert_eq!(delta.changes.len(), 1);
    assert_eq!(
        delta.changes[0].key.node.generation,
        dom.node_generation(red_node).unwrap()
    );
    let actual = retained.raster(old_key).unwrap();
    let mut oracle = amazing_paint::Surface::new(actual.surface.width, actual.surface.height);
    next.raster_raw_local(changed.index..changed.index + 1, &mut oracle, actual.origin)
        .unwrap();
    assert_eq!(*actual.surface, oracle);

    // Retired or unknown nodes must not mutate the retained scene.
    let unknown = box_with(NodeIdx(u32::MAX), 0.0, "red");
    let before = actual.surface.pixels.clone();
    assert!(matches!(
        apply_layout_paint_damage(
            &mut retained,
            &[LayoutPaintDamage::Upsert {
                box_: &unknown,
                item: changed,
                ordinal: 0
            }],
            &dom,
            &text,
            &amazing_paint::NoImages,
        ),
        Err(PaintFallbackReason::AmbiguousGeneration)
    ));
    assert_eq!(retained.raster(old_key).unwrap().surface.pixels, before);
}
