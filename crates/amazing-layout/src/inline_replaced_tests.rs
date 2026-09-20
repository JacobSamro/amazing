//! End-to-end atomic inline tests through both public layout entry points.
use crate::{font::FontSource, replaced::ReplacedKind, *};

struct Natural<'a>(Option<IntrinsicSize>, &'a amazing_dom::Dom);
impl ReplacedElementProvider for Natural<'_> {
    fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize> {
        replaced::kind(self.1, node).and(self.0)
    }
}
fn layout(html: &str, css: &str, natural: Option<IntrinsicSize>, shaped: bool) -> LayoutBox {
    let dom = amazing_html::parse(html);
    let css = format!("body{{margin:0;width:200px;font-size:10px;line-height:10px}} {css}");
    if shaped {
        layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            &css,
            200.0,
            Some(300.0),
            &amazing_text::FallbackTextSystem::new(),
            &Natural(natural, &dom),
        )
    } else {
        layout_dom_document_with_replaced_in_viewport(
            &dom,
            &css,
            200.0,
            Some(300.0),
            &FontSource::fallback(),
            &Natural(natural, &dom),
        )
    }
}
fn fragments(root: &LayoutBox) -> Vec<replaced::StyledReplacedFrag<'_>> {
    let mut result: Vec<_> = root.styled_replaced_fragments().collect();
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &root.flow {
        for child in children {
            result.extend(fragments(child));
        }
    }
    result
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "got {a}, expected {b}");
}
fn natural() -> Option<IntrinsicSize> {
    Some(IntrinsicSize {
        width: 80.0,
        height: 40.0,
    })
}

#[test]
fn direct_and_nested_controls_are_atomic_and_keep_identity() {
    for shaped in [false, true] {
        for nested in [false, true] {
            let controls = "<input><input type=search><input type=email><input type=password><input type=tel><input type=url><input type=number><input type=checkbox><input type=radio><button>label</button><select><option>choice</option></select><textarea>value</textarea>";
            let html = if nested {
                format!("<body>x<span>{controls}</span></body>")
            } else {
                format!("<body>x{controls}</body>")
            };
            let root = layout(&html, "", None, shaped);
            let fs = fragments(&root);
            assert_eq!(fs.len(), 12);
            assert_eq!(fs[7].kind, ReplacedKind::Checkbox);
            assert_eq!(fs[8].kind, ReplacedKind::Radio);
            assert_eq!(fs[9].kind, ReplacedKind::Button);
            assert_eq!(fs[10].kind, ReplacedKind::Select);
            assert_eq!(fs[11].kind, ReplacedKind::Textarea);
            assert!(
                root.lines
                    .iter()
                    .flat_map(|l| &l.words)
                    .all(|w| w.text == "x")
            );
        }
    }
}

#[test]
fn hidden_and_display_none_controls_have_no_fragment() {
    for shaped in [false, true] {
        let root = layout(
            "<body>x<input type=HIDDEN><span><input type=hidden><input style='display:none'><button style='display:none'>gone</button></span><input></body>",
            "",
            None,
            shaped,
        );
        assert_eq!(fragments(&root).len(), 1);
    }
}

#[test]
fn inline_block_controls_and_images_remain_on_the_text_line() {
    for shaped in [false, true] {
        let root = layout(
            "<body>x<button>label</button><span><img></span>y</body>",
            "button,img{display:inline-block;width:20px;height:15px}",
            natural(),
            shaped,
        );
        assert!(matches!(root.flow, Flow::Inlines(_)));
        assert_eq!(root.lines.len(), 1);
        assert_eq!(fragments(&root).len(), 2);
    }
}

#[test]
fn defaults_and_html_sizing_attributes_are_deterministic() {
    let root = layout(
        "<body><input size=5><input type=checkbox><input type=radio><button></button><select></select><textarea cols=4 rows=3></textarea></body>",
        "",
        None,
        false,
    );
    let fs = fragments(&root);
    for (f, (w, h)) in fs.iter().zip([
        (30.0, 12.0),
        (13.0, 13.0),
        (13.0, 13.0),
        (50.0, 15.0),
        (50.0, 15.0),
        (24.0, 36.0),
    ]) {
        near(f.content_box.w, w);
        near(f.content_box.h, h);
    }
}

#[test]
fn existing_provider_overrides_control_defaults_without_creating_a_ratio() {
    for shaped in [false, true] {
        let root = layout(
            "<body>x<input><span><textarea></textarea></span></body>",
            "input,textarea{width:100px}",
            natural(),
            shaped,
        );
        for f in fragments(&root) {
            near(f.content_box.w, 100.0);
            near(f.content_box.h, 40.0);
        }
    }
}

#[test]
fn image_css_dimensions_resolve_auto_axes_from_natural_ratio() {
    for shaped in [false, true] {
        for (css, w, h) in [
            ("width:120px", 120.0, 60.0),
            ("height:30px", 60.0, 30.0),
            ("width:20px;height:30px", 20.0, 30.0),
            ("width:0", 0.0, 0.0),
        ] {
            let root = layout(
                "<body>x<span><img></span></body>",
                &format!("img{{{css}}}"),
                natural(),
                shaped,
            );
            let fs = fragments(&root);
            near(fs[0].content_box.w, w);
            near(fs[0].content_box.h, h);
        }
    }
}

#[test]
fn percentages_use_containing_block_not_wrapper_or_float_band() {
    for shaped in [false, true] {
        let root = layout(
            "<body><div style='float:left;width:40px;height:80px'></div><p>x<span><img></span></p></body>",
            "p{margin:0;height:100px} span{width:10px} img{width:50%;height:50%;padding:5%}",
            natural(),
            shaped,
        );
        let fs = fragments(&root);
        near(fs[0].content_box.w, 100.0);
        near(fs[0].content_box.h, 50.0);
        near(fs[0].padding.top, 10.0);
        near(fs[0].border_box.w, 120.0);
    }
}

#[test]
fn percentage_heights_require_definite_parent_and_survive_anonymous_blocks() {
    for shaped in [false, true] {
        for (html, css, expected) in [
            (
                "<body>x<img></body>",
                "body{min-height:100px} img{height:50%;max-height:10%}",
                40.0,
            ),
            (
                "<body>x<img><div></div></body>",
                "body{height:100px} img{height:50%}",
                50.0,
            ),
            (
                "<body><div><p>x<img></p></div></body>",
                "body{height:200px} div{height:50%} p{height:50%;margin:0} img{height:50%}",
                25.0,
            ),
        ] {
            let root = layout(html, css, natural(), shaped);
            near(fragments(&root)[0].content_box.h, expected);
        }
    }
}

#[test]
fn image_min_max_preserves_ratio_or_obeys_opposing_constraints() {
    for shaped in [false, true] {
        for (css, w, h) in [
            ("max-width:40px", 40.0, 20.0),
            ("min-height:60px", 120.0, 60.0),
            ("min-width:100px;max-height:30px", 100.0, 30.0),
            ("width:100px;max-height:20px", 100.0, 20.0),
            ("min-width:100px;max-width:10px", 100.0, 50.0),
            ("max-width:25%;max-height:5%", 50.0, 25.0),
        ] {
            let root = layout(
                "<body>x<img></body>",
                &format!("img{{{css}}}"),
                natural(),
                shaped,
            );
            let fs = fragments(&root);
            near(fs[0].content_box.w, w);
            near(fs[0].content_box.h, h);
        }
    }
}

#[test]
fn chrome_contributes_to_wrapping_baseline_and_paint_metadata() {
    for shaped in [false, true] {
        let root = layout(
            "<body>x<input><input></body>",
            "body{width:100px} input{width:40px;height:20px;padding:3px;border:2px solid red;margin:4px;background:blue}",
            None,
            shaped,
        );
        assert_eq!(root.lines.len(), 2);
        let fs = fragments(&root);
        near(fs[0].border_box.w, 50.0);
        near(fs[0].border_box.h, 30.0);
        near(fs[0].content_box.x - fs[0].border_box.x, 5.0);
        near(fs[0].content_box.y - fs[0].border_box.y, 5.0);
        near(root.lines[0].baseline - fs[0].border_box.y, 34.0);
        assert!(root.lines[0].height >= 38.0);
        assert_eq!(fs[0].style.get("border-top-color"), Some("red"));
        near(
            fs[1].border_box.y - fs[0].border_box.y,
            root.lines[0].height,
        );
    }
}

#[test]
fn border_box_sizing_and_minimums_account_for_chrome_once() {
    for shaped in [false, true] {
        let root = layout(
            "<body>x<img><input></body>",
            "img,input{box-sizing:border-box;width:50px;height:40px;padding:3px;border:2px solid black;min-width:60px;max-height:30px}",
            natural(),
            shaped,
        );
        for f in fragments(&root) {
            near(f.border_box.w, 60.0);
            near(f.border_box.h, 30.0);
            near(f.content_box.w, 50.0);
            near(f.content_box.h, 20.0);
        }
    }
}

#[test]
fn repeated_layout_re_resolves_percentage_without_accumulating_chrome() {
    let dom = amazing_html::parse("<body><input></body>");
    let styled = amazing_css::compute(
        &dom,
        &[amazing_css::parse_stylesheet(
            "body{margin:0} input{width:50%;padding:2px}",
        )],
    );
    let fonts = FontSource::fallback();
    let mut root = box_tree::build_layout_tree(&dom, &styled, &fonts);
    for width in [200.0, 300.0, 200.0] {
        block::layout_root(&mut root, width, &fonts);
        let fs = fragments(&root);
        near(fs[0].content_box.w, width / 2.0);
        near(fs[0].border_box.w, width / 2.0 + 4.0);
    }
}

#[test]
fn bidi_reorder_moves_content_and_decoration_geometry_together() {
    let root = layout(
        "<body><span dir=rtl>abc<input><img>def</span></body>",
        "input,img{width:20px;height:10px;padding:3px;border:2px solid red}",
        natural(),
        true,
    );
    let fs = fragments(&root);
    assert_eq!(fs.len(), 2);
    for f in fs {
        near(f.content_box.x - f.border_box.x, 5.0);
        near(f.border_box.w, 30.0);
    }
}

#[test]
fn invalid_provider_geometry_falls_back_for_controls_and_omits_images() {
    let root = layout(
        "<body><input><img></body>",
        "",
        Some(IntrinsicSize {
            width: f32::NAN,
            height: 10.0,
        }),
        false,
    );
    let fs = fragments(&root);
    assert_eq!(fs.len(), 1);
    near(fs[0].content_box.w, 120.0);
}
