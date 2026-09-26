use super::*;
use crate::background::{BackgroundPlan, BoxArea, Repeat};
use amazing_dom::NodeIdx;
use amazing_layout::{Display, Flow, Rect};
struct Images {
    red: amazing_media::Image,
    blue: amazing_media::Image,
    stripes: amazing_media::Image,
}
impl Images {
    fn new() -> Self {
        Self {
            red: amazing_media::Image {
                width: 2,
                height: 1,
                pixels: vec![255, 0, 0, 255, 255, 0, 0, 255],
            },
            blue: amazing_media::Image {
                width: 1,
                height: 1,
                pixels: vec![0, 0, 255, 255],
            },
            stripes: amazing_media::Image {
                width: 2,
                height: 1,
                pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
            },
        }
    }
}
impl ImageProvider for Images {
    fn image(&self, _node: NodeIdx) -> Option<&amazing_media::Image> {
        None
    }
    fn background_image(&self, _node: NodeIdx, url: &str) -> Option<&amazing_media::Image> {
        match url {
            "red" => Some(&self.red),
            "blue" => Some(&self.blue),
            "stripes" => Some(&self.stripes),
            _ => None,
        }
    }
}
fn block(pairs: &[(&str, &str)], w: f32, h: f32) -> LayoutBox {
    let style = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut b = LayoutBox::new(Some(NodeIdx(1)), Display::Block, style, 16.0, 19.2);
    b.content_box = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    b.border_box = b.content_box;
    b
}
fn draw(b: &LayoutBox) -> Surface {
    let fonts = FontSource::fallback();
    paint_document_with_resources(b, 16, 16, &fonts, &RectInkProvider, &Images::new())
}
#[test]
fn defaults_repeat_and_negative_positions_are_phase_correct() {
    let b = block(
        &[
            ("background-image", "url(stripes)"),
            ("background-position", "-1px 0"),
        ],
        6.0,
        4.0,
    );
    let s = draw(&b);
    assert_eq!(s.get_pixel(0, 0), Some([0, 0, 255, 255]));
    assert_eq!(s.get_pixel(1, 3), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(6, 0), Some([255; 4]));
}
#[test]
fn repeat_x_y_and_no_repeat_do_not_fill_other_axes() {
    for (mode, point, expected) in [
        ("repeat-x", (4, 0), [255, 0, 0, 255]),
        ("repeat-x", (0, 1), [255; 4]),
        ("repeat-y", (2, 1), [255; 4]),
        ("repeat-y", (0, 4), [255, 0, 0, 255]),
        ("no-repeat", (2, 0), [255; 4]),
    ] {
        let s = draw(&block(
            &[
                ("background-image", "url(red)"),
                ("background-repeat", mode),
            ],
            8.0,
            8.0,
        ));
        assert_eq!(
            s.get_pixel(point.0, point.1),
            Some(expected),
            "{mode} {point:?}"
        );
    }
}
#[test]
fn size_keywords_auto_percent_and_position_use_positioning_area() {
    let common = [
        ("background-image", "url(red)"),
        ("background-repeat", "no-repeat"),
        ("background-position", "center"),
    ];
    for (size, top, bottom) in [
        ("contain", [255; 4], [255; 4]),
        ("cover", [255, 0, 0, 255], [255, 0, 0, 255]),
        ("50% auto", [255; 4], [255; 4]),
    ] {
        let mut style = common.to_vec();
        style.push(("background-size", size));
        let s = draw(&block(&style, 8.0, 8.0));
        assert_eq!(s.get_pixel(3, 0), Some(top));
        assert_eq!(s.get_pixel(3, 7), Some(bottom));
        assert_eq!(s.get_pixel(3, 4), Some([255, 0, 0, 255]));
    }
}
#[test]
fn position_percentages_use_leftover_space_not_box_width() {
    let s = draw(&block(
        &[
            ("background-image", "url(red)"),
            ("background-repeat", "no-repeat"),
            ("background-position", "100% 100%"),
        ],
        8.0,
        8.0,
    ));
    assert_eq!(s.get_pixel(6, 7), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(5, 7), Some([255; 4]));
}
#[test]
fn origin_and_clip_are_independent_and_color_uses_bottom_layer_clip() {
    let mut b = block(
        &[
            ("background-image", "url(blue), none"),
            ("background-color", "red"),
            ("background-repeat", "no-repeat"),
            ("background-size", "100% 100%"),
            ("background-origin", "padding-box"),
            ("background-clip", "content-box,padding-box"),
        ],
        4.0,
        4.0,
    );
    b.content_box = Rect {
        x: 2.0,
        y: 2.0,
        w: 4.0,
        h: 4.0,
    };
    b.border_box = Rect {
        x: 0.0,
        y: 0.0,
        w: 8.0,
        h: 8.0,
    };
    b.padding = amazing_layout::values::Sides {
        top: 1.0,
        right: 1.0,
        bottom: 1.0,
        left: 1.0,
    };
    b.border = b.padding;
    for side in ["top", "right", "bottom", "left"] {
        b.style.set(&format!("border-{side}-color"), "transparent");
    }
    let s = draw(&b);
    assert_eq!(s.get_pixel(0, 0), Some([255; 4]));
    assert_eq!(s.get_pixel(1, 1), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(2, 2), Some([0, 0, 255, 255]));
    assert_eq!(s.get_pixel(6, 6), Some([255, 0, 0, 255]));
}
#[test]
fn topmost_layer_is_first_and_none_keeps_longhand_list_slot() {
    let s = draw(&block(
        &[
            ("background-image", "url(red), none, url(blue)"),
            ("background-repeat", "no-repeat,repeat"),
            ("background-position", "0 0,center,0 0"),
        ],
        8.0,
        8.0,
    ));
    assert_eq!(s.get_pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(4, 4), Some([255; 4]));
    let s = draw(&block(
        &[
            ("background-image", "url(red), url(blue)"),
            ("background-repeat", "no-repeat,repeat"),
        ],
        8.0,
        8.0,
    ));
    assert_eq!(s.get_pixel(4, 4), Some([0, 0, 255, 255]));
}
#[test]
fn rounded_backgrounds_opacity_and_children_keep_display_list_order() {
    let mut b = block(
        &[
            ("background-image", "url(red)"),
            ("border-radius", "4px"),
            ("opacity", ".5"),
        ],
        8.0,
        8.0,
    );
    let child = block(&[("background-image", "url(blue)")], 2.0, 2.0);
    b.flow = Flow::Blocks(vec![child]);
    let s = draw(&b);
    assert_eq!(s.get_pixel(0, 0), Some([127, 127, 255, 255]));
    assert_eq!(s.get_pixel(4, 4), Some([255, 127, 127, 255]));
    assert_eq!(s.get_pixel(7, 0), Some([255; 4]));
}
#[test]
fn space_and_round_repeats_are_bounded_and_have_expected_gaps() {
    let s = draw(&block(
        &[
            ("background-image", "url(red)"),
            ("background-repeat", "space no-repeat"),
            ("background-size", "3px 1px"),
        ],
        8.0,
        2.0,
    ));
    assert_eq!(s.get_pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(3, 0), Some([255; 4]));
    assert_eq!(s.get_pixel(7, 0), Some([255, 0, 0, 255]));
    let s = draw(&block(
        &[
            ("background-image", "url(stripes)"),
            ("background-repeat", "round no-repeat"),
            ("background-size", "3px 1px"),
        ],
        8.0,
        2.0,
    ));
    assert!((0..8).all(|x| s.get_pixel(x, 0) != Some([255; 4])));
}
#[test]
fn url_commas_css_escapes_and_list_limits_are_not_naive_split() {
    let b = block(
        &[
            ("background-image", r#"url("a,b"), none, URL('r\65 d')"#),
            ("background-origin", "content-box,padding-box"),
            ("background-repeat", "repeat-x"),
        ],
        1.0,
        1.0,
    );
    let plan = BackgroundPlan::from_style(&b.style);
    assert_eq!(plan.layers.len(), 3);
    assert_eq!(plan.layers[0].url.as_deref(), Some("a,b"));
    assert_eq!(plan.layers[2].url.as_deref(), Some("red"));
    assert_eq!(plan.layers[2].origin, BoxArea::Content);
    assert_eq!(plan.layers[2].repeat_y, Repeat::NoRepeat);
    assert!(crate::background::split_layers(&vec!["none"; 33].join(",")).is_empty());
}
#[test]
fn sampling_budget_fails_closed_instead_of_spinning_on_tiny_tiles() {
    let b = block(
        &[
            ("background-image", "url(red)"),
            ("background-size", "0.000001px 0.000001px"),
        ],
        8.0,
        8.0,
    );
    let plan = BackgroundPlan::from_style(&b.style);
    let mut s = Surface::new(8, 8);
    let mut budget = 63;
    let clip = ClipRect::viewport(&s);
    assert!(!background_raster::paint(
        &mut s,
        &b,
        &plan,
        &Images::new(),
        clip,
        &mut budget
    ));
}

#[test]
fn center_before_horizontal_edge_keeps_vertical_axis() {
    let b = block(
        &[
            ("background-image", "url(red)"),
            ("background-repeat", "no-repeat"),
            ("background-position", "center right 1px"),
        ],
        8.0,
        9.0,
    );
    let s = draw(&b);
    assert_eq!(s.get_pixel(5, 4), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(5, 0), Some([255; 4]));
}

#[test]
fn fractional_object_edges_and_backgrounds_share_pixel_center_coverage() {
    let images = Images::new();
    let mut image_surface = Surface::new(8, 9);
    image_surface.clear([255; 4]);
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 8.0,
        h: 9.0,
    };
    let presentation = ImagePresentation {
        fit: crate::image::ObjectFit::Contain,
        x: Default::default(),
        y: crate::image::ObjectPosition {
            fraction: 0.5,
            offset: 0.0,
        },
    };
    let clip = ClipRect::viewport(&image_surface);
    paint_image(&mut image_surface, &rect, &images.red, presentation, clip);
    let background = draw(&block(
        &[
            ("background-image", "url(red)"),
            ("background-size", "contain"),
            ("background-position", "center"),
            ("background-repeat", "no-repeat"),
        ],
        8.0,
        9.0,
    ));
    for y in 0..9 {
        for x in 0..8 {
            assert_eq!(
                image_surface.get_pixel(x, y),
                background.get_pixel(x, y),
                "{x},{y}"
            );
        }
    }
    assert_eq!(image_surface.get_pixel(0, 2), Some([255, 0, 0, 255]));
    assert_eq!(image_surface.get_pixel(0, 6), Some([255; 4]));
}

#[test]
fn linear_gradient_background_fills_the_positioning_area() {
    let s = draw(&block(
        &[("background-image", "linear-gradient(to right, red, blue)")],
        8.0,
        8.0,
    ));
    let left = s.get_pixel(1, 4).unwrap();
    let right = s.get_pixel(6, 4).unwrap();
    assert_eq!(left[3], 255, "gradient is opaque: {left:?}");
    assert!(left[0] > left[2], "left edge is red-dominant: {left:?}");
    assert!(right[2] > right[0], "right edge is blue-dominant: {right:?}");
}

#[test]
fn repeating_gradient_tiles_through_background_size() {
    let s = draw(&block(
        &[
            (
                "background-image",
                "repeating-linear-gradient(to right, red 0 2px, blue 2px 4px)",
            ),
            ("background-size", "4px 4px"),
        ],
        8.0,
        4.0,
    ));
    assert_eq!(s.get_pixel(0, 1), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(2, 1), Some([0, 0, 255, 255]));
    assert_eq!(s.get_pixel(4, 1), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(6, 1), Some([0, 0, 255, 255]));
}

#[test]
fn first_listed_gradient_layer_paints_above_later_layers() {
    let s = draw(&block(
        &[
            (
                "background-image",
                "linear-gradient(to right, red, red), linear-gradient(to right, blue, blue)",
            ),
            ("background-size", "4px 8px, 8px 8px"),
            ("background-position", "left top, left top"),
            ("background-repeat", "no-repeat"),
        ],
        8.0,
        8.0,
    ));
    assert_eq!(s.get_pixel(1, 4), Some([255, 0, 0, 255]));
    assert_eq!(s.get_pixel(6, 4), Some([0, 0, 255, 255]));
}

#[test]
fn unsupported_gradient_syntax_paints_no_layer() {
    for image in [
        "linear-gradient(to right, red, blue, not-a-color)",
        "linear-gradient(to right, red)",
        "radial-gradient(to right, red, blue)",
    ] {
        let s = draw(&block(&[("background-image", image)], 8.0, 8.0));
        assert_eq!(s.get_pixel(4, 4), Some([255; 4]), "{image}");
    }
}
