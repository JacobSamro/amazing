use super::*;
use crate::image::{ImagePresentation, ObjectFit, ObjectPosition};
use amazing_layout::Rect;
fn sample() -> amazing_media::Image {
    amazing_media::Image {
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
    }
}
#[test]
fn contain_leaves_letterbox_and_cover_clips_to_content_even_with_visible_overflow() {
    let mut surface = Surface::new(8, 8);
    surface.clear([255; 4]);
    let rect = Rect {
        x: 2.0,
        y: 2.0,
        w: 4.0,
        h: 4.0,
    };
    let clip = ClipRect::viewport(&surface);
    paint_image(
        &mut surface,
        &rect,
        &sample(),
        ImagePresentation {
            fit: ObjectFit::Contain,
            ..Default::default()
        },
        clip,
    );
    assert_eq!(surface.get_pixel(2, 2), Some([255; 4]));
    assert_eq!(surface.get_pixel(2, 3), Some([255, 0, 0, 255]));
    assert_eq!(surface.get_pixel(5, 4), Some([0, 0, 255, 255]));
    surface.clear([255; 4]);
    paint_image(
        &mut surface,
        &rect,
        &sample(),
        ImagePresentation {
            fit: ObjectFit::Cover,
            ..Default::default()
        },
        clip,
    );
    assert_eq!(surface.get_pixel(1, 3), Some([255; 4]));
    assert_eq!(surface.get_pixel(6, 3), Some([255; 4]));
    assert_eq!(surface.get_pixel(2, 3), Some([255, 0, 0, 255]));
    assert_eq!(surface.get_pixel(5, 3), Some([0, 0, 255, 255]));
}
#[test]
fn position_moves_unscaled_object_and_preserves_transparent_layer_alpha() {
    let mut surface = Surface::new(8, 8);
    let clip = ClipRect::viewport(&surface);
    let image = amazing_media::Image {
        width: 1,
        height: 1,
        pixels: vec![255, 0, 0, 128],
    };
    let p = ImagePresentation {
        fit: ObjectFit::None,
        x: ObjectPosition {
            fraction: 1.0,
            offset: -1.0,
        },
        y: ObjectPosition {
            fraction: 1.0,
            offset: 0.0,
        },
    };
    paint_image(
        &mut surface,
        &Rect {
            x: 0.0,
            y: 0.0,
            w: 8.0,
            h: 8.0,
        },
        &image,
        p,
        clip,
    );
    assert_eq!(surface.get_pixel(6, 7), Some([255, 0, 0, 128]));
    assert_eq!(surface.get_pixel(7, 7), Some([0, 0, 0, 0]));
}
#[test]
fn fitted_pixels_obey_ancestor_clip_and_bad_buffers_do_not_panic() {
    let mut surface = Surface::new(8, 8);
    let clip = ClipRect {
        x0: 3.0,
        y0: 0.0,
        x1: 4.0,
        y1: 8.0,
    };
    paint_image(
        &mut surface,
        &Rect {
            x: 0.0,
            y: 0.0,
            w: 8.0,
            h: 8.0,
        },
        &sample(),
        ImagePresentation::default(),
        clip,
    );
    assert_eq!(surface.get_pixel(2, 2), Some([0; 4]));
    assert_eq!(surface.get_pixel(3, 2), Some([255, 0, 0, 255]));
    let hostile = amazing_media::Image {
        width: u32::MAX,
        height: u32::MAX,
        pixels: vec![],
    };
    paint_image(
        &mut surface,
        &Rect {
            x: 0.0,
            y: 0.0,
            w: 8.0,
            h: 8.0,
        },
        &hostile,
        ImagePresentation::default(),
        clip,
    );
}
