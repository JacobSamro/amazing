use amazing_compositor::{
    baked_paint_view_compositor, paint_view_metadata, Presenter, ReferenceCpuPresenter,
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
