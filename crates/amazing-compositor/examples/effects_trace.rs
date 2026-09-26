//! IA-page layer accounting and existing full-painter parity. This does not
//! claim CSS filter parity until the painter exports those effects.
use amazing_compositor::{
    classify_pixel_parity, PaintViewSource, Presenter, ReferenceCpuPresenter,
};
use amazing_paint::{
    controls::NoControls, display_list_view::DisplayListView,
    paint_document_with_text_resources_and_controls, NoImages,
};
use amazing_text::FallbackTextSystem;
fn main() {
    println!("fixture,mode,items,chunks,layers,promoted,squashed,exact,layer_rounding,losses");
    for (name,html,css) in [
        ("ia-reflow","<body><div id=box class=changed></div></body>","body{margin:0}.changed{background:#008000;width:48px;height:32px}"),
        ("ia-effects-scroller","<body><div class=fixed>Fixed</div><section class='scroller changed'><div class=moving>Move</div><div class=fade>Fade</div><p>Overflow content</p><p>More overflow content</p></section></body>","body{margin:0;background:white}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:300px;height:180px;overflow:scroll;background:#ddd}.scroller.changed{width:280px}.moving{will-change:transform;background:blue;width:120px;height:80px}.fade{opacity:.5;background:green;width:120px;height:80px}"),
        ("article","<body><header>Long article</header><main><h1>Chapter one</h1><p>Text wraps through a narrow column of paragraphs.</p><p>More text follows beneath the fold.</p><p>And more lines continue for scrolling.</p><p>Final paragraph at the bottom.</p></main></body>","body{margin:0;background:white;color:#223}main{width:220px}p{margin:12px 0}"),
    ] {
        let text=FallbackTextSystem::new();
        let root=amazing_layout::layout_document_with_text(html,css,512.,&text);
        let view=DisplayListView::new(&root,512,512,&text,&NoImages,&NoControls);
        let source=PaintViewSource::new(&view);let mut compositor=source.compositor(512,512).unwrap();
        let counts=compositor.layer_counts();
        let frame=source.build_frame(&mut compositor,512,512).unwrap();
        let mut presenter=ReferenceCpuPresenter::default();let candidate=presenter.present(&frame).unwrap();
        let full=paint_document_with_text_resources_and_controls(&root,512,512,&text,&NoImages,&NoControls);
        let parity=classify_pixel_parity(&presenter,&candidate,&full);
        println!("{name},{:?},{},{},{},{},{},{},{},{}",source.mode(),view.items().len(),counts.chunks,counts.layers,counts.promoted,counts.squashed_chunks,parity.exact,parity.layer_rounding,parity.losses);
        assert_eq!(parity.losses,0,"{name}: {:?}",parity.first_loss);
    }
}
