//! Provider mutation, selected range, and admission behavior of frame pixels.
use crate::{
    controls::NoControls,
    display_list_view::{DisplayListView, PaintBounds},
    ImageProvider, RasterViewport, Surface,
};
use amazing_text::FallbackTextSystem;
use std::cell::Cell;
struct Images {
    node: amazing_dom::NodeIdx,
    second: Cell<bool>,
    red: amazing_media::Image,
    blue: amazing_media::Image,
}
impl ImageProvider for Images {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        if node != self.node {
            return None;
        }
        Some(if self.second.get() {
            &self.blue
        } else {
            &self.red
        })
    }
}
impl amazing_layout::ReplacedElementProvider for Images {
    fn intrinsic_size(&self, node: amazing_dom::NodeIdx) -> Option<amazing_layout::IntrinsicSize> {
        self.image(node).map(|image| amazing_layout::IntrinsicSize {
            width: image.width as f32,
            height: image.height as f32,
        })
    }
}
#[test]
fn prepared_pixels_own_one_resource_snapshot_and_next_preparation_observes_new_provider() {
    let mut dom = amazing_dom::Dom::new();
    let html = dom.create_element(amazing_dom::NodeIdx(0), "html");
    dom.create_element(html, "head");
    let body = dom.create_element(html, "body");
    let node = dom.create_element(body, "img");
    dom.set_attribute(node, "src", "x").unwrap();
    dom.set_attribute(node, "width", "20").unwrap();
    dom.set_attribute(node, "height", "20").unwrap();
    let text = FallbackTextSystem;
    let images = Images {
        node,
        second: Cell::new(false),
        red: amazing_media::Image {
            width: 1,
            height: 1,
            pixels: vec![240, 20, 30, 255],
        },
        blue: amazing_media::Image {
            width: 1,
            height: 1,
            pixels: vec![10, 40, 230, 255],
        },
    };
    let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        "body{margin:0}img{width:20px;height:20px;object-fit:fill}",
        40.,
        Some(30.),
        &text,
        &images,
    );
    for scale in [1., 1.5, 2.] {
        images.second.set(false);
        let metrics = RasterViewport::new((40., 30.), (40., 30.), scale).unwrap();
        let (w, h) = metrics.physical_size();
        let view =
            DisplayListView::with_dom_generations(&root, 40, 30, &text, &images, &NoControls, &dom)
                .with_raster_viewport(metrics)
                .unwrap();
        let first = view.prepare_baked_range(0..view.items().len()).unwrap();
        images.second.set(true);
        let next = view.prepare_baked_range(0..view.items().len()).unwrap();
        let clip = PaintBounds {
            x: 0.,
            y: 0.,
            width: w as f32,
            height: h as f32,
        };
        let mut a = Surface::new(w, h);
        a.clear([255; 4]);
        let mut b = a.clone();
        assert!(first.copy_to(&mut a, (0, 0), clip));
        assert!(next.copy_to(&mut b, (0, 0), clip));
        assert!(a
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [240, 20, 30, 255]));
        assert!(b
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [10, 40, 230, 255]));
        assert_ne!(a, b);
        let mut legacy = Surface::new(w, h);
        legacy.clear([255; 4]);
        assert!(view.raster_range(0..view.items().len(), &mut legacy, (0, 0), clip));
        assert_eq!(b, legacy);
    }
}
#[test]
fn prepared_range_preserves_existing_budget_and_empty_range_noop() {
    let root = amazing_layout::layout_document_with_text(
        "<body></body>",
        "body{margin:0}",
        32.,
        &FallbackTextSystem,
    );
    let view = DisplayListView::new(
        &root,
        32768,
        32768,
        &FallbackTextSystem,
        &crate::NoImages,
        &NoControls,
    );
    assert!(!view.items().is_empty());
    assert!(view.prepare_baked_range(0..view.items().len()).is_none());
    let empty = view.prepare_baked_range(0..0).unwrap();
    let mut target = Surface::new(3, 2);
    target.clear([10, 20, 30, 40]);
    let original = target.clone();
    assert!(empty.copy_to(
        &mut target,
        (i32::MIN, i32::MAX),
        PaintBounds {
            x: f32::NAN,
            y: 0.,
            width: 0.,
            height: 0.
        }
    ));
    assert_eq!(target, original);
}
