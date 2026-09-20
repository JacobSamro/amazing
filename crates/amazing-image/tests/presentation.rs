use amazing_dom::{Dom, NodeIdx};
use amazing_layout::{IntrinsicSize, ReplacedElementProvider};
use amazing_paint::ImageProvider;
struct Resource {
    node: NodeIdx,
    image: amazing_media::Image,
    size: IntrinsicSize,
}
impl ReplacedElementProvider for Resource {
    fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize> {
        (node == self.node).then_some(self.size)
    }
}
impl ImageProvider for Resource {
    fn image(&self, node: NodeIdx) -> Option<&amazing_media::Image> {
        (node == self.node).then_some(&self.image)
    }
}
fn image_node(dom: &Dom, node: NodeIdx) -> Option<NodeIdx> {
    if dom.name(node) == Some("img") {
        return Some(node);
    }
    dom.children(node).find_map(|child| image_node(dom, child))
}
fn render(css: &str) -> amazing_paint::Surface {
    let dom = amazing_html::parse("<body><img width=8 height=8></body>");
    let node = image_node(&dom, NodeIdx(0)).unwrap();
    let size = amazing_media::ImageSize::resolve(2, 1, Some(8.0), Some(8.0)).unwrap();
    let resources = Resource {
        node,
        size: IntrinsicSize {
            width: size.width,
            height: size.height,
        },
        image: amazing_media::Image {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        },
    };
    let css = format!("body{{margin:0}} img{{display:block}} {css}");
    let fonts = amazing_layout::font::FontSource::fallback();
    let layout = amazing_layout::layout_dom_document_with_replaced_in_viewport(
        &dom,
        &css,
        20.0,
        Some(20.0),
        &fonts,
        &resources,
    );
    let styles = amazing_css::compute(&dom, &[amazing_css::parse_stylesheet(&css)]);
    let images = amazing_paint::image::StyledImages {
        images: &resources,
        styles: &styles,
    };
    amazing_paint::paint_document_with_resources(
        &layout,
        20,
        20,
        &fonts,
        &amazing_paint::RectInkProvider,
        &images,
    )
}
#[test]
fn html_cascade_layout_and_paint_keep_content_size_separate_from_fit() {
    let contain = render("img{object-fit:contain}");
    assert_eq!(contain.get_pixel(0, 0), Some([255; 4]));
    assert_eq!(contain.get_pixel(0, 2), Some([255, 0, 0, 255]));
    assert_eq!(contain.get_pixel(7, 5), Some([0, 0, 255, 255]));
    assert_eq!(contain.get_pixel(0, 6), Some([255; 4]));
    let none = render("img{object-fit:none;object-position:right bottom}");
    assert_eq!(none.get_pixel(6, 7), Some([255, 0, 0, 255]));
    assert_eq!(none.get_pixel(7, 7), Some([0, 0, 255, 255]));
    assert_eq!(none.get_pixel(5, 7), Some([255; 4]));
}
#[test]
fn images_participate_in_opacity_and_overflow_groups() {
    let s = render("body{height:4px;overflow:hidden} img{object-fit:contain;opacity:.5}");
    assert_eq!(s.get_pixel(0, 2), Some([255, 128, 128, 255]));
    assert_eq!(s.get_pixel(0, 4), Some([255; 4]));
}

#[test]
fn inline_fragment_uses_its_own_computed_fit_through_provider() {
    let fill = render("img{display:inline;object-fit:fill}");
    let contain = render("img{display:inline;object-fit:contain}");
    let colored = |surface: &amazing_paint::Surface| {
        (0..surface.height).flat_map(|y| (0..surface.width).map(move |x| (x, y)))
            .filter(|&(x, y)| surface.get_pixel(x, y) != Some([255; 4])).count()
    };
    assert_eq!(colored(&fill), 64);
    assert_eq!(colored(&contain), 32);
}
