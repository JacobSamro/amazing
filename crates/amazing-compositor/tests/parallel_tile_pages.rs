use amazing_compositor::{
    classify_pixel_parity, PaintViewSource, Presenter, ReferenceCpuPresenter, ScrollId,
    TileRasterMode, TileRasterRequest,
};
use amazing_layout::layout_document_with_text;
use amazing_paint::controls::NoControls;
use amazing_paint::display_list_view::{DisplayListView, RetainedPaintView};
use amazing_paint::{paint_document_with_text_resources_and_controls, NoImages};
use amazing_text::FallbackTextSystem;
use std::time::Instant;

const REFTEST_RED_BLUE: &str =
    include_str!("../../../tests/wpt-replaced-content/rc10-fixtures/z-equivalence.html");
const REFTEST_COLORS: &str =
    include_str!("../../../tests/wpt-replaced-content/rc6-fixtures/inherited-variable.html");

fn percentile(values: &mut [u128], p: f64) -> u128 {
    values.sort_unstable();
    values[((values.len() - 1) as f64 * p).ceil() as usize]
}

fn measure(name: &str, html: &str, css: &str, scroll: bool) {
    const SIZE: u32 = 512;
    let text = FallbackTextSystem::new();
    let root = layout_document_with_text(html, css, SIZE as f32, &text);
    let view = DisplayListView::with_node_generations(
        &root,
        SIZE,
        SIZE,
        &text,
        &NoImages,
        &NoControls,
        &|_| Some(1),
    );
    let retained = RetainedPaintView::from_view(&view).expect("owned RC-20 paint snapshot");
    let source = PaintViewSource::new(&view);
    let mut serial = source.compositor(SIZE, SIZE).unwrap();
    let mut parallel = source.compositor(SIZE, SIZE).unwrap();
    serial.set_tile_budget(32);
    parallel.set_tile_budget(32);
    let mut serial_presenter = ReferenceCpuPresenter::default();
    let mut parallel_presenter = ReferenceCpuPresenter::default();
    let mut times = Vec::new();
    let mut serial_times = Vec::new();
    let mut rastered = Vec::new();
    for frame in 0..30 {
        if scroll {
            let offset = (frame * 128) as f32;
            serial.set_scroll(ScrollId(1), (0., offset)).unwrap();
            parallel.set_scroll(ScrollId(1), (0., offset)).unwrap();
        }
        let request = |mode| TileRasterRequest {
            width: SIZE,
            height: SIZE,
            scroll_direction: if scroll { (0., 1.) } else { (0., 0.) },
            mode,
        };
        let serial_start = Instant::now();
        let (a, _) = source
            .build_parallel_frame(&mut serial, &retained, request(TileRasterMode::Deterministic))
            .unwrap();
        serial_times.push(serial_start.elapsed().as_micros());
        let start = Instant::now();
        let (b, batch) = source
            .build_parallel_frame(&mut parallel, &retained, request(TileRasterMode::Parallel { workers: 0 }))
            .unwrap();
        times.push(start.elapsed().as_micros());
        rastered.push(batch.rasterized_tiles as u128);
        let actual = serial_presenter.present(&a).unwrap();
        assert_eq!(actual, parallel_presenter.present(&b).unwrap(), "{name} frame {frame}");
        if frame == 0 {
            let full = paint_document_with_text_resources_and_controls(
                &root, SIZE, SIZE, &text, &NoImages, &NoControls);
            let parity = classify_pixel_parity(&serial_presenter, &actual, &full);
            assert_eq!(parity.losses, 0, "{name} retained paint/full painter parity: {parity:?}");
        }
    }
    let p95 = percentile(&mut times, 0.95);
    let serial_p95 = percentile(&mut serial_times, 0.95);
    let tiles_p95 = percentile(&mut rastered, 0.95);
    let total: u128 = rastered.iter().sum();
    println!("COMP14 {name}: serial_frame_p95_us={serial_p95} parallel_frame_p95_us={p95} tiles_total={total} tiles_p95={tiles_p95} frames=30");
}

#[test]
fn ia_oracle_pages_and_long_article_scroll_trace() {
    measure("z-equivalence", REFTEST_RED_BLUE, "", false);
    measure("inherited-variable", REFTEST_COLORS, "", false);
    let paragraphs = (0..20).map(|i| format!("<p>Chapter {i}: This is a long article paragraph with text that wraps in the viewport.</p>"))
        .collect::<String>();
    let html = format!("<body><div class='scroller'><article>{paragraphs}</article></div></body>");
    measure("long-article-scroll", &html,
        "body{margin:0}.scroller{width:512px;height:512px;overflow:scroll}article{width:480px}p{height:300px;margin:8px 0;color:#234}", true);
}
