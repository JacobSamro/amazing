use amazing_compositor::{
    baked_paint_view_compositor, paint_view_metadata, CompositingReasons, Compositor, PaintChunk,
    PaintViewRasterMode, PaintViewSource, Positioning, Presenter, Rect, ReferenceCpuPresenter,
    ScrollId, StickyConstraint,
};
use amazing_paint::controls::NoControls;
use amazing_paint::display_list_view::DisplayListView;
use amazing_paint::{paint_document_with_text_resources_and_controls, NoImages};
use amazing_text::FallbackTextSystem;
use std::time::Instant;

const VIEWPORT: u32 = 96;
const REFTEST_RED_BLUE: &str =
    include_str!("../../../tests/wpt-replaced-content/rc10-fixtures/z-equivalence.html");
const REFTEST_COLORS: &str =
    include_str!("../../../tests/wpt-replaced-content/rc6-fixtures/inherited-variable.html");
const LONG_ARTICLE: &str = r#"<body><header>Long article</header><main><h1>Chapter one</h1><p>Text wraps through a narrow column of paragraphs.</p><p>More text follows beneath the fold.</p><p>And more lines continue for scrolling.</p><p>Final paragraph at the bottom.</p></main></body>"#;
const EFFECTS_PAGE: &str = r#"<body><div class='fixed'>Fixed</div><section class='scroller'><div class='moving'>Move</div><div class='fade'>Fade</div><p>Overflow content</p><p>More overflow content</p></section></body>"#;

fn compare(name: &str, html: &str, css: &str) {
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text(html, css, VIEWPORT as f32, &text);
    let view = DisplayListView::new(&root, VIEWPORT, VIEWPORT, &text, &NoImages, &NoControls);
    let (trees, chunks) = paint_view_metadata(&view).unwrap();
    assert_eq!(
        chunks.len(),
        view.items().len(),
        "{name} metadata item count"
    );
    for chunk in &chunks {
        assert!(trees.transform(chunk.state.transform).is_some());
    }
    let mut compositor = baked_paint_view_compositor(&view, VIEWPORT, VIEWPORT).unwrap();
    let frame = compositor.build_frame(&view, VIEWPORT, VIEWPORT).unwrap();
    let actual = ReferenceCpuPresenter::default().present(&frame).unwrap();
    let expected = paint_document_with_text_resources_and_controls(
        &root,
        VIEWPORT,
        VIEWPORT,
        &text,
        &NoImages,
        &NoControls,
    );
    assert_eq!(actual.pixels, expected.pixels, "{name} pixel parity");
}

#[test]
fn pinned_real_pages_match_full_painter() {
    compare("z-equivalence", REFTEST_RED_BLUE, "");
    compare("inherited-variable", REFTEST_COLORS, "");
    compare("long-article", LONG_ARTICLE,
        "body{margin:0;background:white;color:#223}header{position:fixed;top:0;background:#aaf}main{width:72px;padding-top:18px}p{margin:12px 0}");
    compare("effects-scroller", EFFECTS_PAGE,
        "body{margin:0;background:white}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:80px;height:52px;overflow:scroll;background:#ddd}.moving{will-change:transform;transform:translate(3px,4px);background:blue;width:30px;height:22px}.fade{opacity:.5;background:green;width:30px;height:22px}");
}

#[test]
fn raw_real_pages_match_full_painter() {
    for (name, html, css) in [
        ("z-equivalence", REFTEST_RED_BLUE, ""),
        ("inherited-variable", REFTEST_COLORS, ""),
        ("article", LONG_ARTICLE, "body{margin:0;background:white;color:#223}main{width:72px}p{margin:12px 0}"),
        ("effects", EFFECTS_PAGE, "body{margin:0;background:white}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:80px;height:52px;overflow:scroll;background:#ddd}.moving{will-change:transform;transform:translate(3px,4px);background:blue;width:30px;height:22px}.fade{opacity:.5;background:green;width:30px;height:22px}"),
    ] {
        let text = FallbackTextSystem::new();
        let root = amazing_layout::layout_document_with_text(html, css, VIEWPORT as f32, &text);
        let view = DisplayListView::new(&root, VIEWPORT, VIEWPORT, &text, &NoImages, &NoControls);
        let source = PaintViewSource::new(&view);
        assert_eq!(source.mode(), PaintViewRasterMode::RawLocal, "{name}");
        let mut compositor = source.compositor(VIEWPORT, VIEWPORT).unwrap();
        let frame = source.build_frame(&mut compositor, VIEWPORT, VIEWPORT).unwrap();
        let actual = ReferenceCpuPresenter::default().present(&frame).unwrap();
        let expected = paint_document_with_text_resources_and_controls(&root, VIEWPORT, VIEWPORT, &text, &NoImages, &NoControls);
        if actual.pixels != expected.pixels {
            let mismatch = actual.pixels.chunks_exact(4).zip(expected.pixels.chunks_exact(4)).enumerate().find(|(_, (a, b))| a != b).unwrap();
            panic!("{name} raw parity at ({}, {}): {:?} vs {:?}", mismatch.0 % VIEWPORT as usize, mismatch.0 / VIEWPORT as usize, mismatch.1.0, mismatch.1.1);
        }
    }
}

#[test]
fn retained_real_page_property_trace() {
    const SIZE: u32 = 512;
    let html = format!("{EFFECTS_PAGE}<main>{LONG_ARTICLE}</main>");
    let css = "body{margin:0;background:white;color:#223}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:300px;height:180px;overflow:scroll;background:#ddd}.moving{will-change:transform;transform:translate(3px,4px);background:blue;width:120px;height:80px}.fade{opacity:.5;background:green;width:120px;height:80px}main{width:220px}p{margin:12px 0}";
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text(&html, css, SIZE as f32, &text);
    let view = DisplayListView::new(&root, SIZE, SIZE, &text, &NoImages, &NoControls);
    let source = PaintViewSource::new(&view);
    assert_eq!(source.mode(), PaintViewRasterMode::RawLocal);
    let scroll = ScrollId(1);
    let transform = amazing_compositor::TransformId(1);
    let effect = amazing_compositor::EffectId(2);
    let mut compositor = source.compositor(SIZE, SIZE).unwrap();
    let bounds = compositor.trees().scroll(scroll).unwrap().container;
    compositor
        .set_scroll_contents(
            scroll,
            Rect::new(bounds.x, bounds.y, bounds.w, bounds.h + 100.),
        )
        .unwrap();
    let initial = source.build_frame(&mut compositor, SIZE, SIZE).unwrap();
    assert!(!initial.resources.uploads.is_empty());
    println!("scenario,frames,full_paint_us,retained_build_us,uploads");
    for scenario in ["scroll", "transform", "opacity"] {
        let mut full_us = 0;
        let mut retained_us = 0;
        let mut uploads = 0;
        for index in 0..20 {
            let start = Instant::now();
            let _ = paint_document_with_text_resources_and_controls(
                &root,
                SIZE,
                SIZE,
                &text,
                &NoImages,
                &NoControls,
            );
            full_us += start.elapsed().as_micros();
            match scenario {
                "scroll" => compositor
                    .set_scroll(scroll, (0., (index + 1) as f32))
                    .unwrap(),
                "transform" => compositor
                    .set_transform(
                        transform,
                        amazing_compositor::Matrix4::translation((index + 1) as f32, 4., 0.),
                    )
                    .unwrap(),
                "opacity" => compositor
                    .set_opacity(effect, 0.5 + index as f32 / 100.)
                    .unwrap(),
                _ => unreachable!(),
            }
            let start = Instant::now();
            let frame = source.build_frame(&mut compositor, SIZE, SIZE).unwrap();
            retained_us += start.elapsed().as_micros();
            uploads += frame.resources.uploads.len();
        }
        println!("{scenario},20,{full_us},{retained_us},{uploads}");
        assert_eq!(uploads, 0, "{scenario} rerasterized retained tiles");
        assert!(
            retained_us * 10 <= full_us,
            "{scenario} retained frame exceeded 10% of full paint"
        );
    }
    compositor.invalidate_item(0);
    assert!(compositor.dirty_tiles() > 0);
    let _damaged = source.build_frame(&mut compositor, SIZE, SIZE).unwrap();
    assert_eq!(compositor.dirty_tiles(), 0);
}

#[test]
fn control_uses_baked_fallback_with_parity() {
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text(
        "<body><input type='checkbox'></body>",
        "body{margin:0;background:white}",
        VIEWPORT as f32,
        &text,
    );
    let view = DisplayListView::new(&root, VIEWPORT, VIEWPORT, &text, &NoImages, &NoControls);
    let source = PaintViewSource::new(&view);
    assert_eq!(source.mode(), PaintViewRasterMode::BakedFallback);
    let mut compositor = source.compositor(VIEWPORT, VIEWPORT).unwrap();
    let frame = source
        .build_frame(&mut compositor, VIEWPORT, VIEWPORT)
        .unwrap();
    let actual = ReferenceCpuPresenter::default().present(&frame).unwrap();
    let expected = paint_document_with_text_resources_and_controls(
        &root,
        VIEWPORT,
        VIEWPORT,
        &text,
        &NoImages,
        &NoControls,
    );
    assert_eq!(actual.pixels, expected.pixels);
}

#[test]
fn real_nested_overflow_fixed_sticky_and_snap_metadata() {
    let html = "<body><section class='outer'><div class='fixed'>Fixed</div><section class='inner'><div class='sticky'>Sticky</div><p>Scrollable paragraph</p><p>Second paragraph</p></section></section></body>";
    let css = "body{margin:0}.outer{width:80px;height:75px;overflow:scroll}.inner{width:65px;height:40px;overflow:scroll}.fixed{position:fixed;top:0;left:0;background:red}.sticky{position:sticky;top:4px;background:blue}p{margin:10px 0}";
    let text = FallbackTextSystem::new();
    let root = amazing_layout::layout_document_with_text(html, css, VIEWPORT as f32, &text);
    let view = DisplayListView::new(&root, VIEWPORT, VIEWPORT, &text, &NoImages, &NoControls);
    let (mut trees, chunks) = paint_view_metadata(&view).unwrap();
    assert!(view.scrolls().len() >= 3, "nested overflow nodes");
    let fixed = view.items().iter().find(|item| item.hints.fixed).unwrap();
    let sticky = view.items().iter().find(|item| item.hints.sticky).unwrap();
    assert_eq!(chunks[fixed.index].state.positioning, Positioning::Fixed);
    let inner = ScrollId((view.scrolls().len() - 1) as usize);
    let outer = ScrollId(view.scrolls()[inner.0].parent as usize);
    assert_ne!(outer, ScrollId(0));
    let bounds = trees.scroll(inner).unwrap().container;
    trees
        .set_scroll_contents(
            inner,
            Rect::new(bounds.x, bounds.y, bounds.w, bounds.h + 80.),
        )
        .unwrap();
    trees
        .set_snap_points(inner, vec![], vec![0., 20., 40.])
        .unwrap();
    let fixed_state = chunks[fixed.index].state;
    let fixed_before = trees.matrix(fixed_state).unwrap();
    let sticky_bounds = chunks[sticky.index].bounds;
    let sticky_id = trees
        .add_sticky(StickyConstraint {
            scroll: inner,
            normal: sticky_bounds,
            top: Some(4.),
            right: None,
            bottom: None,
            left: None,
        })
        .unwrap();
    let mut selected = vec![PaintChunk {
        items: fixed.index..fixed.index + 1,
        state: fixed_state,
        bounds: chunks[fixed.index].bounds,
        reasons: CompositingReasons::FIXED,
    }];
    selected.push(PaintChunk {
        items: sticky.index..sticky.index + 1,
        state: amazing_compositor::PropertyTreeState {
            positioning: Positioning::Sticky(sticky_id),
            ..chunks[sticky.index].state
        },
        bounds: sticky_bounds,
        reasons: CompositingReasons::SCROLLER,
    });
    // Source item ranges must stay in paint order.
    selected.sort_by_key(|chunk| chunk.items.start);
    trees.set_scroll(inner, (0., 27.)).unwrap();
    assert_eq!(trees.snap_offset(inner, (0., 27.)), Some((0., 20.)));
    assert_eq!(trees.matrix(fixed_state).unwrap(), fixed_before);
    let compositor = Compositor::new(trees, selected, view.items().len()).unwrap();
    assert!(compositor
        .trees()
        .matrix(
            compositor
                .chunks()
                .iter()
                .find(|c| c.state.positioning == Positioning::Sticky(sticky_id))
                .unwrap()
                .state
        )
        .is_some());
}

#[test]
fn real_page_style_change_trace_and_parity() {
    println!("scenario,frame,full_repaint_us,baked_quad_build_us,quad_count,uploads");
    for scenario in ["scroll_layout_shift", "transform", "opacity"] {
        for frame_index in 0..12 {
            let value = frame_index + 1;
            let css = match scenario {
                "scroll_layout_shift" => format!(
                    "body{{margin:0;background:white}}main{{width:72px;margin-top:-{value}px}}p{{margin:12px 0}}"
                ),
                "transform" => format!(
                    "body{{margin:0;background:white}}.moving{{width:30px;height:22px;background:blue;will-change:transform;transform:rotate({value}deg)}}"
                ),
                "opacity" => format!(
                    "body{{margin:0;background:white}}.fade{{width:30px;height:22px;background:green;opacity:{}}}",
                    1.0 - value as f32 / 24.0
                ),
                _ => unreachable!(),
            };
            let html = if scenario == "scroll_layout_shift" {
                LONG_ARTICLE
            } else {
                EFFECTS_PAGE
            };
            let text = FallbackTextSystem::new();
            let root =
                amazing_layout::layout_document_with_text(html, &css, VIEWPORT as f32, &text);
            let view =
                DisplayListView::new(&root, VIEWPORT, VIEWPORT, &text, &NoImages, &NoControls);
            let full_start = Instant::now();
            let expected = paint_document_with_text_resources_and_controls(
                &root,
                VIEWPORT,
                VIEWPORT,
                &text,
                &NoImages,
                &NoControls,
            );
            let full_us = full_start.elapsed().as_micros();
            let quad_start = Instant::now();
            let mut compositor = baked_paint_view_compositor(&view, VIEWPORT, VIEWPORT).unwrap();
            let frame = compositor.build_frame(&view, VIEWPORT, VIEWPORT).unwrap();
            let quad_us = quad_start.elapsed().as_micros();
            let actual = ReferenceCpuPresenter::default().present(&frame).unwrap();
            assert_eq!(
                actual.pixels, expected.pixels,
                "{scenario} frame {frame_index}"
            );
            println!(
                "{scenario},{frame_index},{full_us},{quad_us},{},{}",
                frame.quad_count(),
                frame.resources.uploads.len()
            );
        }
    }
}
