#[path="support/paint_filter_parity.rs"]
mod corpus;
#[test]
fn real_painter_filter_directory(){assert_eq!(corpus::run(false),0);}

#[test]
fn transformed_clipped_and_oversize_filters_use_exact_full_paint_fallback() {
    use amazing_compositor::*;
    use amazing_paint::{controls::NoControls,display_list_view::DisplayListView,
        paint_document_with_text_resources_and_controls,NoImages};
    use amazing_text::FallbackTextSystem;
    for declaration in [
        "filter:blur(2px);transform:translate(3px,4px)",
        "backdrop-filter:blur(2px);transform:scale(1.1)",
        "filter:blur(2px);clip-path:circle(40%);transform:translate(1px,1px)",
        "filter:invert(1);clip-path:inset(3px round 2px);transform:translate(1px,1px)",
        "clip-path:ellipse(40% 30%);transform:scale(1.1)",
        "filter:blur(129px)",
    ] {
        let text=FallbackTextSystem::new();
        let css=format!("body{{margin:0;background:blue}}div{{width:16px;height:16px;background:red;{declaration}}}");
        let root=amazing_layout::layout_document_with_text("<body><div></div></body>",&css,32.,&text);
        let view=DisplayListView::new(&root,32,32,&text,&NoImages,&NoControls);
        let source=PaintViewSource::new(&view);
        assert_eq!(source.mode(),PaintViewRasterMode::BakedFallback,"{declaration}");
        assert!(source.property_bindings().is_none());
        let mut compositor=source.compositor(32,32).unwrap();
        let frame=source.build_frame(&mut compositor,32,32).unwrap();
        let actual=ReferenceCpuPresenter::default().present(&frame).unwrap();
        let expected=paint_document_with_text_resources_and_controls(&root,32,32,&text,&NoImages,&NoControls);
        assert_eq!(actual,expected,"{declaration}");
    }
}
#[test]
fn imported_program_cannot_silently_survive_a_changed_filter_list() {
    use amazing_compositor::*;
    let plan=amazing_paint::filter::FilterPlan::from_css("blur(2px)",[0;4]);
    let mut op=EffectOperations {
        filters:vec![FilterOperation::Blur(2.)],
        painter_filters:Some((plan,Default::default())),
        ..Default::default()
    };
    assert!(op.validate().is_ok());
    op.filters=vec![FilterOperation::Blur(4.)];
    assert_eq!(op.validate(),Err(EffectError::UnresolvedFilter));
}
