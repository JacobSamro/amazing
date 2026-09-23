use crate::{Flow, LayoutBox, Rect, font::FontSource, replaced::ReplacedKind};

fn layout(markup: &str, css: &str) -> LayoutBox {
    let dom = amazing_html::parse(&format!("<body>{markup}</body>"));
    let css = format!("body{{margin:0;width:500px;height:200px;line-height:0}} {css}");
    crate::layout_dom_document_with_replaced_in_viewport(
        &dom,
        &css,
        800.0,
        Some(300.0),
        &FontSource::fallback(),
        &crate::NoReplacedElements,
    )
}

fn inline_svgs(box_: &LayoutBox, out: &mut Vec<Rect>) {
    out.extend(
        box_.styled_replaced_fragments()
            .filter(|fragment| fragment.kind == ReplacedKind::Svg)
            .map(|fragment| fragment.content_box),
    );
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
        for child in children {
            inline_svgs(child, out);
        }
    }
}

fn single_inline(markup: &str, css: &str) -> Rect {
    let root = layout(markup, css);
    let mut rects = Vec::new();
    inline_svgs(&root, &mut rects);
    assert_eq!(rects.len(), 1);
    rects[0]
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.05, "{actual} != {expected}");
}

#[test]
fn wpt_outer_svg_sizing_matrix() {
    // Static snapshots of svg/coordinate-systems/outer-svg-intrinsic-size-001
    // and css/css-sizing/svg-intrinsic-size-002 at WPT c2a2fce69e5c.
    for (markup, width, height) in [
        ("<svg></svg>", 300.0, 150.0),
        ("<svg width='100'></svg>", 100.0, 150.0),
        ("<svg height='200'></svg>", 300.0, 200.0),
        ("<svg width='400' height='100'></svg>", 400.0, 100.0),
        ("<svg viewBox='0 0 10 8'></svg>", 500.0, 400.0),
        ("<svg width='100' viewBox='0 0 50 10'></svg>", 100.0, 20.0),
        ("<svg height='40' viewBox='0 0 4 1'></svg>", 160.0, 40.0),
        ("<svg width='40' viewBox='0 0 4 1'></svg>", 40.0, 10.0),
        (
            "<svg width='40' height='40' viewBox='0 0 4 1'></svg>",
            40.0,
            40.0,
        ),
    ] {
        let rect = single_inline(markup, "");
        near(rect.w, width);
        near(rect.h, height);
    }
}

#[test]
fn css_dimensions_and_percentages_override_svg_attributes() {
    let rect = single_inline(
        "<svg width='100' height='40' viewBox='0 0 4 1' style='width:50%;height:25%'></svg>",
        "",
    );
    near(rect.w, 250.0);
    near(rect.h, 50.0);
    let rect = single_inline("<svg width='50%' height='40'></svg>", "");
    near(rect.w, 250.0);
    near(rect.h, 40.0);
    let rect = single_inline("<svg width='-1' height='0'></svg>", "");
    near(rect.w, 300.0);
    near(rect.h, 150.0);
    let rect = single_inline("<svg width='100' height='50' style='width:auto'></svg>", "");
    near(rect.w, 300.0);
    near(rect.h, 50.0);
}

#[test]
fn svg_subtrees_never_make_css_boxes() {
    let root = layout(
        "<svg width='80' height='40'><rect width='900' height='900'/><svg width='500' height='500'><text>wrong</text></svg></svg>",
        "",
    );
    let mut rects = Vec::new();
    inline_svgs(&root, &mut rects);
    assert_eq!(rects.len(), 1);
    near(rects[0].w, 80.0);
    near(rects[0].h, 40.0);
    assert!(root.lines.iter().all(|line| line.words.is_empty()));
}

#[test]
fn svg_display_none_and_contents_suppress_the_entire_subtree() {
    for display in ["none", "contents"] {
        let root = layout(
            &format!("<svg style='display:{display}'><svg width='90'/><text>wrong</text></svg>"),
            "",
        );
        let mut rects = Vec::new();
        inline_svgs(&root, &mut rects);
        assert!(rects.is_empty());
        assert!(matches!(root.flow, Flow::Empty));
    }
}

#[test]
fn block_svg_exposes_root_and_content_viewport() {
    let root = layout(
        "<svg width='50%' viewBox='0 0 4 1' style='display:block'><rect width='900'/></svg>",
        "",
    );
    let Flow::Blocks(children) = &root.flow else {
        panic!("expected block SVG")
    };
    let svg = &children[0];
    assert!(svg.svg_root.is_some());
    assert!(matches!(svg.flow, Flow::Empty));
    near(svg.content_box.w, 250.0);
    near(svg.content_box.h, 62.5);
}
