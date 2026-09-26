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
