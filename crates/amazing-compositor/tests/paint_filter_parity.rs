#[path="support/paint_filter_parity.rs"]
mod corpus;
#[test]
fn real_painter_filter_directory(){assert_eq!(corpus::run(false),0);}

#[test]
fn transformed_filter_clip_and_backdrop_groups_match_full_paint() {
    assert_eq!(corpus::run_transformed(), 0);
}

/// COMP-16 gave a group with its own transform, sitting directly under the
/// identity root and unnested with any other filter/backdrop/clip group, a
/// single inverse-mapped composite (see the corpus above). These remaining
/// cases still need the exact full-paint fallback: an oversized filter is
/// rejected on its own terms; the rest would need composing two groups'
/// local surfaces (or a second, different transform) rather than one
/// inverse-mapped sample, which the retained compositor does not do.
#[test]
fn transformed_clipped_and_oversize_filters_use_exact_full_paint_fallback() {
    use amazing_compositor::*;
    use amazing_paint::{controls::NoControls,display_list_view::DisplayListView,
        paint_document_with_text_resources_and_controls,NoImages};
    use amazing_text::FallbackTextSystem;
    for (html, css) in [
        ("<body><div></div></body>",
         "body{margin:0;background:blue}div{width:16px;height:16px;background:red;filter:blur(129px)}"),
        // A transformed group's own blur reads neighboring source pixels
        // outward by its outset; that read is not yet transform-sampled
        // (see `PainterFilterSource::from_view`'s outset check).
        ("<body><div></div></body>",
         "body{margin:0;background:blue}div{width:16px;height:16px;background:red;filter:blur(2px);transform:rotate(5deg)}"),
        // Two qualifying groups, nested, the outer one transformed.
        ("<body><div class=outer><div class=inner></div></div></body>",
         "body{margin:0;background:blue}.outer{width:24px;height:24px;background:red;filter:blur(2px);transform:rotate(5deg)}.inner{width:12px;height:12px;background:green;clip-path:inset(2px)}"),
        // The filtered group's own transform sits under an ancestor
        // transform, not directly under the identity root.
        ("<body><div class=outer><div class=inner></div></div></body>",
         "body{margin:0;background:blue}.outer{width:24px;height:24px;background:red;transform:translate(2px,2px)}.inner{width:12px;height:12px;background:green;filter:blur(2px);transform:rotate(5deg)}"),
        // A plain, further transform nested inside a transformed filter
        // group: its content would need a second, different sample.
        ("<body><div class=outer><div class=inner></div></div></body>",
         "body{margin:0;background:blue}.outer{width:24px;height:24px;background:red;filter:blur(2px);transform:translate(2px,2px)}.inner{width:12px;height:12px;background:green;transform:rotate(5deg)}"),
    ] {
        let text=FallbackTextSystem::new();
        let root=amazing_layout::layout_document_with_text(html,css,32.,&text);
        let view=DisplayListView::new(&root,32,32,&text,&NoImages,&NoControls);
        let source=PaintViewSource::new(&view);
        assert_eq!(source.mode(),PaintViewRasterMode::BakedFallback,"{css}");
        assert!(source.property_bindings().is_none());
        let mut compositor=source.compositor(32,32).unwrap();
        let frame=source.build_frame(&mut compositor,32,32).unwrap();
        let actual=ReferenceCpuPresenter::default().present(&frame).unwrap();
        let expected=paint_document_with_text_resources_and_controls(&root,32,32,&text,&NoImages,&NoControls);
        assert_eq!(actual,expected,"{css}");
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
