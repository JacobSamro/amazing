use super::*;
use amazing_layout::{Display, LayoutBox, Rect};
use amazing_text::FallbackTextSystem;
fn root(pairs: &[(&str, &str)], rect: Rect) -> LayoutBox {
    let style = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut b = LayoutBox::new(None, Display::Block, style, 16.0, 20.0);
    b.content_box = rect;
    b.border_box = rect;
    b
}
fn draw(b: &LayoutBox, scale: f64) -> Surface {
    paint_document_at_viewport(
        b,
        RasterViewport::new((12.0, 8.0), (12.0, 8.0), scale).unwrap(),
        &FallbackTextSystem,
        &NoImages,
        &controls::NoControls,
    )
    .unwrap()
}
#[test]
fn raster_extent_is_checked_and_distinct_from_configured_css_viewport() {
    let v = RasterViewport::new((10.25, 5.25), (10.25, 40.25), 1.5).unwrap();
    assert_eq!(v.physical_size(), (16, 61));
    assert_eq!(v.css_viewport_size(), (10.25, 5.25));
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(RasterViewport::new((10.0, 5.0), (10.0, 5.0), scale).is_err());
    }
    assert_eq!(
        RasterViewport::new((1.0, 1.0), (1e12, 1e12), 2.0),
        Err(RasterError::ResourceLimit)
    );
    assert_eq!(v.css_scroll_to_device((2.0, -4.0)), Some((3.0, -6.0)));
    assert_eq!(
        v.css_affine_to_device([2.0, 0.0, 0.0, 3.0, 4.0, 5.0]),
        Some([2.0, 0.0, 0.0, 3.0, 6.0, 7.5])
    );
}
#[test]
fn full_lattice_samples_css_geometry_at_actual_device_centers() {
    let b = root(
        &[("background-color", "red")],
        Rect {
            x: 1.25,
            y: 1.25,
            w: 3.0,
            h: 2.0,
        },
    );
    for scale in [1.25, 1.5, 2.0, 3.0] {
        let image = draw(&b, scale);
        for y in 0..image.height {
            for x in 0..image.width {
                let cx = (f64::from(x) + 0.5) / scale;
                let cy = (f64::from(y) + 0.5) / scale;
                let color = if cx >= 1.25 && cx < 4.25 && cy >= 1.25 && cy < 3.25 {
                    [255, 0, 0, 255]
                } else {
                    [255; 4]
                };
                assert_eq!(
                    image.get_pixel(x, y),
                    Some(color),
                    "density {scale} ({x},{y})"
                );
            }
        }
    }
    let legacy = paint_document_with_text_resources_and_controls(
        &b,
        12,
        8,
        &FallbackTextSystem,
        &NoImages,
        &controls::NoControls,
    );
    assert_eq!(legacy, draw(&b, 1.0));
}
#[test]
fn physical_raw_and_baked_tiles_match_actual_full_raster() {
    for scale in [1.25, 1.5, 2.0, 3.0] {
        let b = root(
            &[("background-color", "blue")],
            Rect {
                x: 1.0,
                y: 1.0,
                w: 8.0,
                h: 5.0,
            },
        );
        let full = draw(&b, scale);
        let viewport = RasterViewport::new((12.0, 8.0), (12.0, 8.0), scale).unwrap();
        let text = FallbackTextSystem;
        let controls = controls::NoControls;
        let view = display_list_view::DisplayListView::new(&b, 12, 8, &text, &NoImages, &controls)
            .with_raster_viewport(viewport)
            .unwrap();
        let mut tile = Surface::new(7, 5);
        tile.clear([255; 4]);
        view.raster_raw_local(0..view.items().len(), &mut tile, (3, 2))
            .unwrap();
        for y in 0..5 {
            for x in 0..7 {
                assert_eq!(tile.get_pixel(x, y), full.get_pixel(x + 3, y + 2));
            }
        }
        let mut baked = Surface::new(7, 5);
        baked.clear([255; 4]);
        assert!(view.raster_range(
            0..view.items().len(),
            &mut baked,
            (-3, -2),
            display_list_view::PaintBounds {
                x: 0.0,
                y: 0.0,
                width: 7.0,
                height: 5.0
            }
        ));
        assert_eq!(tile, baked);
    }
}
#[test]
fn generated_images_evaluate_original_css_stops_at_device_centers() {
    let image = crate::gradient::raster_at_scale(
        "linear-gradient(to right, red 0px, blue 10px)",
        20,
        4,
        (10.0, 2.0),
        16.0,
        2.0,
    )
    .unwrap();
    let logical =
        crate::gradient::raster("linear-gradient(to right, red 0px, blue 10px)", 10, 2, 16.0)
            .unwrap();
    assert_ne!(&image.pixels[0..4], &logical.pixels[0..4]);
    assert_ne!(&image.pixels[0..4], &image.pixels[4..8]);
    assert_eq!(image.width, 20);
    assert_eq!(image.height, 4);
}
#[test]
fn authentic_shaped_text_uses_device_raster_and_preserves_css_layout() {
    let text = FallbackTextSystem;
    let root = amazing_layout::layout_document_with_text(
        "<body>A</body>",
        "body{margin:0;color:black;font-size:10px}",
        40.0,
        &text,
    );
    let before = format!("{root:?}");
    let unit = paint_document_at_viewport(
        &root,
        RasterViewport::new((40.0, 30.0), (40.0, 30.0), 1.0).unwrap(),
        &text,
        &NoImages,
        &controls::NoControls,
    )
    .unwrap();
    let black = |image: &Surface| image.pixels.chunks_exact(4).filter(|p| p[0] < 128).count();
    assert!(black(&unit) > 0);
    for scale in [1.25, 1.5, 2.0, 3.0] {
        let image = paint_document_at_viewport(
            &root,
            RasterViewport::new((40.0, 30.0), (40.0, 30.0), scale).unwrap(),
            &text,
            &NoImages,
            &controls::NoControls,
        )
        .unwrap();
        assert!(black(&image) > black(&unit));
    }
    assert_eq!(before, format!("{root:?}"));
}
#[test]
fn changed_density_invalidates_cache_even_with_same_ceiled_extent() {
    let text=FallbackTextSystem;let controls=controls::NoControls;
    let mut dom = amazing_dom::Dom::new();
    let html = dom.create_element(amazing_dom::NodeIdx(0), "html");
    dom.create_element(html, "head");
    let body = dom.create_element(html, "body");
    dom.create_element(body, "div");
    let root=amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(&dom,"body{margin:0}div{width:1px;height:1px;background:red}",1.0,Some(1.0),&text,&amazing_layout::NoReplacedElements);
    let make=|scale| display_list_view::DisplayListView::with_dom_generations(&root,1,1,&text,&NoImages,&controls,&dom).with_raster_viewport(RasterViewport::new((1.0,1.0),(1.0,1.0),scale).unwrap()).unwrap();
    let first=make(1.1);let second=make(1.2);
    assert_eq!(first.raster_viewport().unwrap().physical_size(),second.raster_viewport().unwrap().physical_size());
    assert_ne!(first.raster_viewport(),second.raster_viewport());
    assert!(matches!(first.update(&second),display_list_view::PaintViewUpdate::RequiresFullRebuild{reason:display_list_view::PaintFallbackReason::ViewportChanged,..}));
}
#[test]
fn actual_nested_transform_rounded_clip_and_opacity_share_device_lattice() {
    let text = FallbackTextSystem;
    let root = amazing_layout::layout_document_with_text(
        "<body><div><span></span></div></body>",
        "body{margin:0}div{width:8px;height:6px;overflow:hidden;border-radius:2px;transform:translate(1px,1px);opacity:.5}span{display:block;width:10px;height:8px;background:red}",
        12.0,
        &text,
    );
    for scale in [1.25, 1.5, 2.0, 3.0] {
        let v = RasterViewport::new((12.0, 8.0), (12.0, 8.0), scale).unwrap();
        let full =
            paint_document_at_viewport(&root, v, &text, &NoImages, &controls::NoControls).unwrap();
        let controls = controls::NoControls;
        let view =
            display_list_view::DisplayListView::new(&root, 12, 8, &text, &NoImages, &controls)
                .with_raster_viewport(v)
                .unwrap();
        let mut replay = Surface::new(full.width, full.height);
        replay.clear([255; 4]);
        assert!(view.raster_range(
            0..view.items().len(),
            &mut replay,
            (0, 0),
            display_list_view::PaintBounds {
                x: 0.0,
                y: 0.0,
                width: full.width as f32,
                height: full.height as f32
            }
        ));
        assert_eq!(full, replay, "density {scale}");
        assert_eq!(full.get_pixel(0, 0), Some([255; 4]));
        assert!(
            full.pixels
                .chunks_exact(4)
                .any(|p| p[0] == 255 && p[1] < 255)
        );
    }
}
#[test]
fn fractional_gradient_size_keeps_authored_stop_geometry_independent_of_ceil() {
    let css="linear-gradient(to right, red 0px, blue 10px)";
    let prepared=crate::gradient::PreparedGradient::new(css,(6.3,2.1),16.0).unwrap();
    let raster=crate::gradient::raster_at_scale(css,13,5,(6.3,2.1),16.0,2.0).unwrap();
    for x in 0..13 {assert_eq!(&raster.pixels[x*4..x*4+4],&prepared.sample((x as f32+0.5)/2.0,0.25).unwrap());}
}
#[test]
fn decoded_background_repetition_keeps_css_phase_at_each_device_density() {
    struct Stripes(amazing_media::Image);
    impl paint::ImageProvider for Stripes {
        fn image(&self,_:amazing_dom::NodeIdx)->Option<&amazing_media::Image>{None}
        fn background_image(&self,_:amazing_dom::NodeIdx,url:&str)->Option<&amazing_media::Image>{(url=="stripes").then_some(&self.0)}
    }
    let images=Stripes(amazing_media::Image{width:2,height:1,pixels:vec![255,0,0,255,0,0,255,255]});
    let text=FallbackTextSystem;
    let root=amazing_layout::layout_document_with_text("<body><div></div></body>","body{margin:0}div{width:6.3px;height:2.1px;background-image:url(stripes);background-size:2px 1px;background-position:-1px 0}",12.0,&text);
    for scale in [1.25,1.5,2.0,3.0] {
        let image=paint_document_at_viewport(&root,RasterViewport::new((12.0,8.0),(12.0,8.0),scale).unwrap(),&text,&images,&controls::NoControls).unwrap();
        for y in 0..image.height {for x in 0..image.width {
            let cx=(f64::from(x)+0.5)/scale;let cy=(f64::from(y)+0.5)/scale;
            if cx<6.3 && cy<2.1 {
                let expected=if (cx+1.0).rem_euclid(2.0)<1.0 {[255,0,0,255]} else {[0,0,255,255]};
                assert_eq!(image.get_pixel(x,y),Some(expected),"density {scale} ({x},{y})");
            }
        }}
    }
}
#[test]
fn missing_scaled_text_support_is_checked_and_leaves_partial_targets_unchanged() {
    struct LegacyText;
    impl amazing_text::TextSystem for LegacyText {
        fn generation(&self)->u64{0}
        fn metrics(&self,style:&amazing_text::TextStyle)->Result<amazing_text::FontMetrics,amazing_text::TextError>{FallbackTextSystem.metrics(style)}
        fn shape(&self,p:amazing_text::Paragraph<'_>)->Result<amazing_text::ShapedParagraph,amazing_text::TextError>{FallbackTextSystem.shape(p)}
        fn shape_line(&self,r:amazing_text::LineRequest<'_>)->Result<amazing_text::ShapedLine,amazing_text::TextError>{FallbackTextSystem.shape_line(r)}
        fn rasterize(&self,r:amazing_text::GlyphRasterRequest<'_>)->Result<Option<amazing_text::GlyphImage>,amazing_text::TextError>{FallbackTextSystem.rasterize(r)}
    }
    use amazing_text::TextSystem;
    let text=LegacyText;
    let root=amazing_layout::layout_document_with_text("<body>A</body>","body{margin:0;font-size:10px}",40.0,&text);
    let controls=controls::NoControls;
    let v=RasterViewport::new((40.0,30.0),(40.0,30.0),2.0).unwrap();
    assert_eq!(paint_document_at_viewport(&root,v,&text,&NoImages,&controls),Err(RasterError::UnsupportedText));
    assert!(paint_document_at_viewport(&root,RasterViewport::new((40.0,30.0),(40.0,30.0),1.0).unwrap(),&text,&NoImages,&controls).is_ok());
    let view=display_list_view::DisplayListView::new(&root,40,30,&text,&NoImages,&controls).with_raster_viewport(v).unwrap();
    let mut target=Surface::new(80,60);target.clear([17,23,41,255]);let original=target.clone();
    assert!(view.raster_raw_local(0..view.items().len(),&mut target,(0,0)).is_err());
    assert_eq!(target,original);
    assert!(!view.raster_range(0..view.items().len(),&mut target,(0,0),display_list_view::PaintBounds{x:0.0,y:0.0,width:80.0,height:60.0}));
    assert_eq!(target,original);
}
