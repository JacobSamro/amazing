use amazing_compositor::*;
use amazing_paint::{controls::NoControls,display_list_view::DisplayListView,
    paint_document_with_text_resources_and_controls,NoImages};
use amazing_text::FallbackTextSystem;
pub fn run(legacy: bool) -> usize {
    let mut failures=0;
    println!("property,value,mode,exact,rounding,losses");
    for property in ["filter","backdrop-filter"] {
        for value in ["brightness(.4)","contrast(.5)","grayscale(.7)","hue-rotate(90deg)",
            "invert(.8)","opacity(.5)","saturate(.2)","sepia(.6)","blur(2px)",
            "drop-shadow(3px 2px 1px red)","invert(1) blur(1px)",
            "blur(1px) opacity(.5)"] {
            let text=FallbackTextSystem::new();
            let fill=if property=="filter" {"background:rgb(180,70,130)"}else{"background:rgba(180,70,130,.25)"};
            let css=format!("body{{margin:0;background:rgb(90,170,210)}}.outer{{padding:12px;width:40px;height:40px;background:linear-gradient(90deg,red 50%,blue 50%)}}.inner{{width:24px;height:24px;{fill};{property}:{value}}}");
            let root=amazing_layout::layout_document_with_text("<body><div class=outer><div class=inner></div></div></body>",&css,64.,&text);
            let view=DisplayListView::new(&root,64,64,&text,&NoImages,&NoControls);
            let source=PaintViewSource::new(&view);
            assert_eq!(source.mode(),PaintViewRasterMode::RawLocal);
            let mut compositor=source.compositor(64,64).unwrap();
            if legacy {
                for i in 1..view.effects().len() {
                    let id=EffectId(i);
                    let mut op=compositor.trees().effect_operations(id).cloned().unwrap_or_default();
                    op.painter_filters=None;op.filters.clear();op.backdrop_filters.clear();op.backdrop_bounds=None;
                    compositor.set_effect_operations(id,op).unwrap();
                }
            }
            let frame=source.build_frame(&mut compositor,64,64).unwrap();
            let mut presenter=ReferenceCpuPresenter::default();
            let actual=presenter.present(&frame).unwrap();
            let restored=CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
            assert_eq!(ReferenceCpuPresenter::default().present(&restored).unwrap(),actual);
            let full=paint_document_with_text_resources_and_controls(&root,64,64,&text,&NoImages,&NoControls);
            let parity=classify_pixel_parity(&presenter,&actual,&full);
            println!("{property},{value},{:?},{},{},{}",source.mode(),parity.exact,parity.layer_rounding,parity.losses);
            if parity.losses>0 {failures+=1;eprintln!("{property}/{value}: {:?}",parity.first_loss);}
            let cached=source.build_cached_frame(&mut compositor,64,64).unwrap();
            assert_eq!(presenter.present(&cached).unwrap(),actual);
        }
    }
    // RC-27 clip-path basic shapes, alone and combined with a filter, prove
    // the same real-painter import path for `EffectNode.clip`.
    for (shape,combine) in [
        ("inset(4px)",""),("inset(4px round 6px)",""),
        ("circle(40%)",""),("circle(10px at 20px 12px)",""),
        ("ellipse(30% 20%)",""),("ellipse(12px 8px at 18px 10px)",""),
        ("inset(4px round 6px)",";filter:blur(1px)"),
        ("circle(10px at 20px 12px)",";backdrop-filter:invert(1)"),
    ] {
        let text=FallbackTextSystem::new();
        let css=format!("body{{margin:0;background:rgb(90,170,210)}}.outer{{padding:12px;width:40px;height:40px;background:linear-gradient(90deg,red 50%,blue 50%)}}.inner{{width:24px;height:24px;background:rgb(180,70,130);clip-path:{shape}{combine}}}");
        let root=amazing_layout::layout_document_with_text("<body><div class=outer><div class=inner></div></div></body>",&css,64.,&text);
        let view=DisplayListView::new(&root,64,64,&text,&NoImages,&NoControls);
        let source=PaintViewSource::new(&view);
        assert_eq!(source.mode(),PaintViewRasterMode::RawLocal,"clip-path:{shape}{combine}");
        let mut compositor=source.compositor(64,64).unwrap();
        let frame=source.build_frame(&mut compositor,64,64).unwrap();
        let mut presenter=ReferenceCpuPresenter::default();
        let actual=presenter.present(&frame).unwrap();
        let restored=CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
        assert_eq!(ReferenceCpuPresenter::default().present(&restored).unwrap(),actual);
        let full=paint_document_with_text_resources_and_controls(&root,64,64,&text,&NoImages,&NoControls);
        let parity=classify_pixel_parity(&presenter,&actual,&full);
        println!("clip-path,{shape}{combine},{:?},{},{},{}",source.mode(),parity.exact,parity.layer_rounding,parity.losses);
        if parity.losses>0 {failures+=1;eprintln!("clip-path/{shape}{combine}: {:?}",parity.first_loss);}
        let cached=source.build_cached_frame(&mut compositor,64,64).unwrap();
        assert_eq!(presenter.present(&cached).unwrap(),actual);
    }
    eprintln!("TOTAL {failures}/32 failed");
    failures
}

/// COMP-16: a group whose own resolved `transform` sits directly under the
/// identity root, with no other filter/backdrop/clip group nested above or
/// below it, composites by one inverse-mapped sample of its local surface
/// (see `PainterFilterSource::from_view`, `DrawQuad::Pass`). `.outer` stays
/// plain so `.inner`'s transform is the only one in the view.
pub fn run_transformed() -> usize {
    let mut failures = 0;
    println!("transform,case,mode,exact,rounding,losses");
    let transforms = [
        "translate(4px,3px)",
        "scale(1.3)",
        "rotate(8deg)",
        "scale(.7) rotate(-6deg) translate(2px,1px)",
    ];
    let cases = [
        ("filter:invert(1)", "rgb(180,70,130)"),
        ("backdrop-filter:invert(1)", "rgba(180,70,130,.25)"),
        ("clip-path:circle(40%)", "rgb(180,70,130)"),
        ("clip-path:inset(3px round 5px)", "rgb(180,70,130)"),
        ("filter:invert(1);clip-path:ellipse(35% 25%)", "rgb(180,70,130)"),
    ];
    let total = transforms.len() * cases.len();
    for transform in transforms {
        for (effect, fill) in cases {
            let text = FallbackTextSystem::new();
            let css = format!(
                "body{{margin:0;background:rgb(90,170,210)}}\
                 .outer{{padding:12px;width:40px;height:40px;background:linear-gradient(90deg,red 50%,blue 50%)}}\
                 .inner{{width:24px;height:24px;background:{fill};{effect};transform:{transform}}}"
            );
            let root = amazing_layout::layout_document_with_text(
                "<body><div class=outer><div class=inner></div></div></body>", &css, 64., &text);
            let view = DisplayListView::new(&root, 64, 64, &text, &NoImages, &NoControls);
            let source = PaintViewSource::new(&view);
            assert_eq!(source.mode(), PaintViewRasterMode::RawLocal, "{transform} / {effect}");
            let mut compositor = source.compositor(64, 64).unwrap();
            let frame = source.build_frame(&mut compositor, 64, 64).unwrap();
            let mut presenter = ReferenceCpuPresenter::default();
            let actual = presenter.present(&frame).unwrap();
            let restored = CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
            assert_eq!(ReferenceCpuPresenter::default().present(&restored).unwrap(), actual);
            let full = paint_document_with_text_resources_and_controls(&root, 64, 64, &text, &NoImages, &NoControls);
            let parity = classify_pixel_parity(&presenter, &actual, &full);
            println!("{transform},{effect},{:?},{},{},{}", source.mode(), parity.exact, parity.layer_rounding, parity.losses);
            if parity.losses > 0 {
                failures += 1;
                eprintln!("{transform}/{effect}: {:?}", parity.first_loss);
            }
            let cached = source.build_cached_frame(&mut compositor, 64, 64).unwrap();
            assert_eq!(presenter.present(&cached).unwrap(), actual);
        }
    }
    eprintln!("TRANSFORMED TOTAL {failures}/{total} failed");
    failures
}
