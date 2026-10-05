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

#[test]
fn flagged_fits_draw_unscaled_or_downscaled_pixels_and_keep_both_clips() {
    let mut surface=Surface::new(12,12);let content=Rect{x:2.0,y:2.0,w:8.0,h:8.0};
    for name in ["contain scale-down","cover scale-down"] {
        surface.clear([255;4]);let style:ComputedStyle=[("object-fit".to_owned(),name.to_owned())].into_iter().collect();
        let presentation=ImagePresentation::from_style(&style);let clip=ClipRect::viewport(&surface);
        paint_image(&mut surface,&content,&sample(),presentation,clip);
        assert_eq!(surface.get_pixel(5,5),Some([255,0,0,255]));assert_eq!(surface.get_pixel(6,5),Some([0,0,255,255]));
        assert_eq!(surface.get_pixel(4,5),Some([255;4]));assert_eq!(surface.get_pixel(7,5),Some([255;4]));
    }
    // A tall image downscales with cover, rather than collapsing to contain.
    let image=amazing_media::Image{width:8,height:16,pixels:[255,0,0,255].repeat(128)};
    let style:ComputedStyle=[("object-fit".to_owned(),"cover scale-down".to_owned())].into_iter().collect();
    let presentation=ImagePresentation::from_style(&style);surface.clear([255;4]);
    let clip=ClipRect{x0:4.0,y0:0.0,x1:6.0,y1:12.0};
    paint_image(&mut surface,&Rect{x:2.0,y:2.0,w:4.0,h:4.0},&image,presentation,clip);
    assert_eq!(surface.get_pixel(3,3),Some([255;4]));assert_eq!(surface.get_pixel(4,2),Some([255,0,0,255]));assert_eq!(surface.get_pixel(5,2),Some([255,0,0,255]));
    assert_eq!(surface.get_pixel(4,1),Some([255;4]));assert_eq!(surface.get_pixel(4,6),Some([255;4]));
}

#[test]
fn density_corrected_natural_fit_keeps_decoded_sampling_and_invalid_guards() {
    let mut surface = Surface::new(8, 8);
    let rect = Rect { x: 0.0, y: 0.0, w: 8.0, h: 8.0 };
    let clip = ClipRect::viewport(&surface);
    let image = amazing_media::Image { width: 8, height: 4, pixels: [255, 0, 0, 255].repeat(32) };
    for (density, bounds) in [(1.0, (0, 2, 8, 6)), (1.5, (1, 3, 7, 5)), (2.0, (2, 3, 6, 5))] {
        for fit in [ObjectFit::None, ObjectFit::ScaleDown, ObjectFit::CoverScaleDown] {
            surface.clear([255; 4]);
            paint_image_at_density(&mut surface, &rect, &image, ImagePresentation { fit, ..Default::default() }, density, clip);
            for y in 0..8 { for x in 0..8 {
                let inside = x >= bounds.0 && x < bounds.2 && y >= bounds.1 && y < bounds.3;
                assert_eq!(surface.get_pixel(x, y), Some(if inside { [255, 0, 0, 255] } else { [255; 4] }), "density={density} fit={fit:?} ({x},{y})");
            }}
        }
    }
    for density in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        surface.clear([255; 4]);
        paint_image_at_density(&mut surface, &rect, &image, ImagePresentation::default(), density, clip);
        assert!(surface.pixels.chunks_exact(4).all(|p| p == [255; 4]));
    }
}
