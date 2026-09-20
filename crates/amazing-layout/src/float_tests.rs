//! Public-interface float geometry checks, without inspecting the solver.
use crate::{Flow, LayoutBox};
use amazing_dom::Dom;

fn layout(html: &str, css: &str, shaped: bool) -> (Dom, LayoutBox) {
    let dom = amazing_html::parse(html);
    let css = format!("body{{margin:0;width:200px;font-size:10px;line-height:10px}} {css}");
    let root = if shaped {
        crate::layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            &css,
            200.0,
            None,
            &amazing_text::FallbackTextSystem::new(),
            &crate::NoReplacedElements,
        )
    } else {
        crate::layout_dom_document(&dom, &css, 200.0, &crate::font::FontSource::fallback())
    };
    (dom, root)
}
fn find<'a>(b: &'a LayoutBox, dom: &Dom, id: &str) -> &'a LayoutBox {
    fn visit<'a>(b: &'a LayoutBox, dom: &Dom, id: &str) -> Option<&'a LayoutBox> {
        if b.node
            .and_then(|n| dom.attrs_of(n))
            .is_some_and(|a| a.iter().any(|(k, v)| k == "id" && v == id))
        {
            return Some(b);
        }
        match &b.flow {
            Flow::Blocks(c) | Flow::Flex(c) | Flow::Grid(c) => {
                c.iter().find_map(|b| visit(b, dom, id))
            }
            _ => None,
        }
    }
    visit(b, dom, id).unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.01, "got {a}, expected {b}");
}

#[test]
fn opposing_floats_pack_and_drop_without_advancing_normal_blocks() {
    let (dom, root) = layout(
        "<body><div id=a></div><div id=b></div><div id=c></div><div id=normal></div></body>",
        "#a{float:left;width:80px;height:30px} #b{float:right;width:70px;height:20px} #c{float:left;width:60px;height:10px} #normal{height:5px}",
        false,
    );
    let a = find(&root, &dom, "a");
    let b = find(&root, &dom, "b");
    let c = find(&root, &dom, "c");
    near(a.border_box.x, 0.0);
    near(b.border_box.x, 130.0);
    near(b.border_box.y, 0.0);
    near(c.border_box.x, 80.0);
    near(c.border_box.y, 20.0);
    near(find(&root, &dom, "normal").border_box.y, 0.0);
    near(root.content_box.h, 30.0);
}

#[test]
fn float_margins_do_not_absorb_unused_width_or_collapse() {
    let (dom, root) = layout(
        "<body><div id=a></div><div id=b></div></body>",
        "div{float:left;width:40px;height:20px;margin:5px 10px}",
        false,
    );
    near(find(&root, &dom, "a").border_box.x, 10.0);
    near(find(&root, &dom, "b").border_box.x, 70.0);
    near(find(&root, &dom, "b").border_box.y, 5.0);
    near(root.content_box.h, 30.0);
}

#[test]
fn clear_selects_side_and_preserves_trailing_margin_contract() {
    for (clear, expected) in [("left", 30.0), ("right", 50.0), ("both", 50.0)] {
        let (dom, root) = layout(
            "<body><div id=a></div><div id=b></div><div id=c></div><div id=d></div></body>",
            &format!(
                "#a{{float:left;width:40px;height:30px}} #b{{float:right;width:40px;height:50px}} #c{{clear:{clear};height:10px;margin-top:5px;margin-bottom:20px}} #d{{height:10px;margin-top:12px}}"
            ),
            false,
        );
        near(find(&root, &dom, "c").border_box.y, expected);
        near(find(&root, &dom, "d").border_box.y, expected + 30.0);
    }
}

#[test]
fn nested_normal_blocks_share_floats_but_bfc_contains_and_isolates_them() {
    let (dom, root) = layout(
        "<body><div id=outer><div id=a></div></div><div id=bfc><div id=b></div></div><div id=after></div></body>",
        "#a{float:left;width:60px;height:50px} #bfc{display:flow-root} #b{float:left;width:30px;height:20px} #after{clear:both;height:10px}",
        false,
    );
    near(find(&root, &dom, "outer").content_box.h, 0.0);
    near(find(&root, &dom, "bfc").border_box.x, 60.0);
    near(find(&root, &dom, "bfc").content_box.h, 20.0);
    near(find(&root, &dom, "after").border_box.y, 50.0);
}

#[test]
fn inline_float_is_blockified_and_lines_recover_full_width_below_it() {
    for shaped in [false, true] {
        let (dom, root) = layout(
            "<body><span id=f></span><div id=text>aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa</div></body>",
            "#f{float:left;width:100px;height:20px}",
            shaped,
        );
        near(find(&root, &dom, "f").border_box.w, 100.0);
        let lines = &find(&root, &dom, "text").lines;
        assert!(lines.len() > 2);
        near(lines[0].words[0].x, 100.0);
        near(lines[1].words[0].x, 100.0);
        near(lines[2].words[0].x, 0.0);
    }
}

#[test]
fn unbreakable_first_word_moves_below_an_obstructing_float() {
    for shaped in [false, true] {
        let (dom, root) = layout(
            "<body><div id=f></div><div id=text>abcdefghij</div></body>",
            "#f{float:right;width:180px;height:40px}",
            shaped,
        );
        let text = find(&root, &dom, "text");
        near(text.lines[0].y_top, 40.0);
        near(text.lines[0].words[0].x, 0.0);
        near(text.content_box.h, 50.0);
    }
}

#[test]
fn float_auto_width_uses_shrink_to_fit_and_contains_its_own_floats() {
    let (dom, root) = layout(
        "<body><div id=f>abc def</div><div id=outer><div id=inner></div></div></body>",
        "#f{float:left} #outer{float:right;width:60px} #inner{float:left;width:30px;height:40px}",
        false,
    );
    assert!(find(&root, &dom, "f").border_box.w < 100.0);
    near(find(&root, &dom, "outer").content_box.h, 40.0);
}

#[test]
fn bfc_height_takes_maximum_of_float_bottom_and_normal_trailing_margin() {
    let (dom, root) = layout(
        "<body><div id=bfc><div id=f></div><div id=n></div></div></body>",
        "#bfc{overflow:hidden} #f{float:left;width:30px;height:100px} #n{height:10px;margin-bottom:20px}",
        false,
    );
    near(find(&root, &dom, "bfc").content_box.h, 100.0);
}

#[test]
fn bfc_border_box_avoids_a_float_starting_below_its_top() {
    let (dom, root) = layout(
        "<body><div id=f1></div><div id=f2></div><div id=bfc></div></body>",
        "#f1{float:left;width:30px;height:20px} #f2{float:left;clear:left;width:100px;height:40px} #bfc{overflow:hidden;width:150px;height:50px}",
        false,
    );
    near(find(&root, &dom, "bfc").border_box.y, 60.0);
    near(find(&root, &dom, "bfc").border_box.x, 0.0);
}

#[test]
fn empty_float_does_not_increase_explicit_zero_line_height() {
    let (dom, root) = layout(
        "<body><p id=p><span style='float:left'></span>text</p></body>",
        "p{line-height:0;margin:0}",
        true,
    );
    near(find(&root, &dom, "p").content_box.h, 0.0);
}

#[test]
fn cleared_empty_margin_group_stays_inside_its_parent() {
    let (dom, root) = layout(
        "<body><div id=parent><div id=f></div><div id=clear></div><div id=empty></div></div><div id=after></div></body>",
        "#f{float:left;width:30px;height:100px} #clear{clear:both;margin-bottom:20px} #empty{margin-bottom:30px} #after{height:10px}",
        false,
    );
    near(find(&root, &dom, "clear").border_box.y, 100.0);
    near(find(&root, &dom, "parent").content_box.h, 130.0);
    near(find(&root, &dom, "after").border_box.y, 130.0);
}

#[test]
fn floated_inline_flex_keeps_its_inner_formatting_context() {
    let (dom, root) = layout(
        "<body><div id=f><div id=a></div><div id=b></div></div></body>",
        "#f{float:left;display:inline-flex;width:100px} #a,#b{width:30px;height:10px}", false);
    assert!(matches!(find(&root, &dom, "f").flow, Flow::Flex(_)));
    near(find(&root, &dom, "a").border_box.y, find(&root, &dom, "b").border_box.y);
    near(find(&root, &dom, "b").border_box.x, 30.0);
}
