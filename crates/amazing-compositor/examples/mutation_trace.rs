//! Full mutation-to-frame baseline for ADR 0010. Run on win-001, never macOS.
use amazing_compositor::{PaintViewSource, Presenter, ReferenceCpuPresenter};
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_layout::{layout_dom_document_with_text_and_replaced_in_viewport, NoReplacedElements};
use amazing_paint::controls::NoControls;
use amazing_paint::display_list_view::DisplayListView;
use amazing_paint::{paint_document_with_text_resources_and_controls, NoImages};
use amazing_text::FallbackTextSystem;
use std::time::Instant;

const SIZE: u32 = 512;
const ARTICLE: &str = "<body><header>Long article</header><main><h1>Chapter one</h1><p>Text wraps through a narrow column of paragraphs.</p><p>More text follows beneath the fold.</p><p>And more lines continue for scrolling.</p><p>Final paragraph at the bottom.</p></main></body>";
const EFFECTS: &str = "<body><div class='fixed'>Fixed</div><section class='scroller'><div class='moving'>Move</div><div class='fade'>Fade</div><p>Overflow content</p><p>More overflow content</p></section></body>";
const ARTICLE_CSS: &str = "body{margin:0;background:white;color:#223}main{width:220px}main.changed{width:205px}p{margin:12px 0}";
const EFFECTS_CSS: &str = "body{margin:0;background:white}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:300px;height:180px;overflow:scroll;background:#ddd}.scroller.changed{width:280px}.moving{will-change:transform;background:blue;width:120px;height:80px}.fade{opacity:.5;background:green;width:120px;height:80px}";

fn target(dom: &Dom, tag: &str) -> NodeIdx {
    (0..dom.node_count())
        .map(|index| NodeIdx(index as u32))
        .find(|&node| matches!(&dom.get(node).kind, NodeKind::Element { name, .. } if name == tag))
        .expect("fixture target")
}
fn percentile(values: &[u128], fraction: f64) -> u128 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[((sorted.len() - 1) as f64 * fraction).ceil() as usize]
}
fn run(name: &str, html: &str, css: &str, tag: &str, base_class: &str, frames: usize) {
    let mut dom = amazing_html::parse(html);
    let node = target(&dom, tag);
    let text = FallbackTextSystem::new();
    let mut full = Vec::with_capacity(frames);
    let mut layout = Vec::with_capacity(frames);
    let mut view_build = Vec::with_capacity(frames);
    let mut frame_build = Vec::with_capacity(frames);
    println!("fixture,frame,mutation_to_frame_us,style_layout_us,view_us,frame_us,items,uploads");
    for index in 0..frames {
        let start = Instant::now();
        let class = if index % 2 == 0 {
            format!("{base_class} changed")
        } else {
            base_class.to_string()
        };
        dom.set_attribute(node, "class", &class).unwrap();
        let phase = Instant::now();
        let root = layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            css,
            SIZE as f32,
            Some(SIZE as f32),
            &text,
            &NoReplacedElements,
        );
        let layout_us = phase.elapsed().as_micros();
        let phase = Instant::now();
        let view = DisplayListView::new(&root, SIZE, SIZE, &text, &NoImages, &NoControls);
        let source = PaintViewSource::new(&view);
        let view_us = phase.elapsed().as_micros();
        let phase = Instant::now();
        let mut compositor = source.compositor(SIZE, SIZE).unwrap();
        let frame = source.build_frame(&mut compositor, SIZE, SIZE).unwrap();
        let pixels = ReferenceCpuPresenter::default().present(&frame).unwrap();
        let frame_us = phase.elapsed().as_micros();
        let total_us = start.elapsed().as_micros();
        let expected = paint_document_with_text_resources_and_controls(
            &root,
            SIZE,
            SIZE,
            &text,
            &NoImages,
            &NoControls,
        );
        assert_eq!(pixels.pixels, expected.pixels, "{name} frame {index}");
        full.push(total_us);
        layout.push(layout_us);
        view_build.push(view_us);
        frame_build.push(frame_us);
        println!(
            "{name},{index},{total_us},{layout_us},{view_us},{frame_us},{},{}",
            view.items().len(),
            frame.resources.uploads.len()
        );
    }
    eprintln!("fixture={name} frames={frames} mutation_to_frame_p50_us={} mutation_to_frame_p95_us={} style_layout_p50_us={} view_p50_us={} frame_p50_us={}",
        percentile(&full, 0.5), percentile(&full, 0.95), percentile(&layout, 0.5), percentile(&view_build, 0.5), percentile(&frame_build, 0.5));
}
fn main() {
    let frames = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20);
    run("article-class", ARTICLE, ARTICLE_CSS, "main", "", frames);
    run(
        "effects-class",
        EFFECTS,
        EFFECTS_CSS,
        "section",
        "scroller",
        frames,
    );
}
