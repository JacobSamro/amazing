//! Contents changes formatting ancestry, not the DOM or style ancestry.
use crate::*;
use amazing_dom::{Dom, NodeKind};

fn page(html: &str, css: &str, shaped: bool) -> (Dom, LayoutBox) {
    let dom = amazing_html::parse(html);
    let css = format!("body{{margin:0;font-size:10px;line-height:10px}}{css}");
    let root = if shaped {
        layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            &css,
            200.0,
            Some(300.0),
            &amazing_text::FallbackTextSystem::new(),
            &NoReplacedElements,
        )
    } else {
        layout_dom_document_with_replaced_in_viewport(
            &dom,
            &css,
            200.0,
            Some(300.0),
            &font::FontSource::fallback(),
            &NoReplacedElements,
        )
    };
    (dom, root)
}
fn id(dom: &Dom, value: &str) -> NodeIdx {
    let mut pending = vec![NodeIdx(0)];
    while let Some(node) = pending.pop() {
        if let NodeKind::Element { attrs, .. } = &dom.get(node).kind {
            if attrs.iter().any(|(name, v)| name == "id" && v == value) {
                return node;
            }
        }
        pending.extend(dom.children(node));
    }
    panic!("missing id {value}")
}
fn boxes(b: &LayoutBox) -> Vec<&LayoutBox> {
    let mut out = vec![b];
    if let Flow::Blocks(c) | Flow::Flex(c) | Flow::Grid(c) = &b.flow {
        for child in c {
            out.extend(boxes(child));
        }
    }
    for child in &b.positioned {
        out.extend(boxes(child));
    }
    out
}
fn get<'a>(root: &'a LayoutBox, node: NodeIdx) -> &'a LayoutBox {
    boxes(root)
        .into_iter()
        .find(|b| b.node == Some(node))
        .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "got {a}, expected {b}");
}

#[test]
fn inline_flex_and_grid_boxes_share_one_parent_line() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><span id=a><i></i></span><span id=b><i></i></span></body>",
            "span{display:inline-flex;width:20px;height:10px}#b{display:inline-grid}i{display:block;width:20px;height:10px}",
            shaped,
        );
        assert!(matches!(root.flow, Flow::Inlines(_)));
        assert_eq!(root.lines.len(), 1);
        let a = get(&root, id(&dom, "a"));
        let b = get(&root, id(&dom, "b"));
        assert!(matches!(a.flow, Flow::Flex(_)));
        assert!(matches!(b.flow, Flow::Grid(_)));
        near(a.border_box.x, 0.0);
        near(b.border_box.x, 20.0);
        near(a.border_box.y, b.border_box.y);
    }
}

#[test]
fn inline_flex_uses_its_first_item_baseline_in_parent_line() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body>x<span id=flex><span id=inner>y</span></span>z</body>",
            "#flex{display:inline-flex}#inner{font-size:20px;line-height:20px}",
            shaped,
        );
        let flex = get(&root, id(&dom, "flex"));
        assert_eq!(root.lines.len(), 1);
        near(crate::flex::first_baseline(flex), root.lines[0].baseline);
    }
}

#[test]
fn inline_block_is_one_unbreakable_box_with_shrink_to_fit_width() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><span id=a>hello</span><span id=b>world</span></body>",
            "span{display:inline-block;margin:0 3px;padding:0 2px}",
            shaped,
        );
        assert_eq!(root.lines.len(), 1);
        let a = get(&root, id(&dom, "a"));
        let b = get(&root, id(&dom, "b"));
        assert!(matches!(a.flow, Flow::Inlines(_)));
        assert!(a.border_box.w < root.content_box.w / 2.0);
        near(b.border_box.x, a.border_box.x + a.border_box.w + 6.0);
        assert!(!a.lines.is_empty());
    }
}

#[test]
fn inline_grid_lanes_uses_combined_track_intrinsic_width() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><span id=lanes><i></i><i></i><i></i><i></i></span></body>",
            "#lanes{display:inline grid-lanes;grid-template-columns:repeat(4,auto);column-gap:5px}i{width:20px;height:10px}",
            shaped,
        );
        let lanes = get(&root, id(&dom, "lanes"));
        assert!(matches!(lanes.flow, Flow::Grid(_)));
        near(lanes.border_box.w, 95.0);
        assert_eq!(root.lines.len(), 1);
    }
}

#[test]
fn inline_block_uses_last_line_baseline_or_bottom_margin_edge() {
    for shaped in [false, true] {
        for overflow in ["visible", "hidden"] {
            let (dom, root) = page(
                "<body>x<span id=a>first<br>last</span>y</body>",
                &format!("#a{{display:inline-block;overflow:{overflow};margin-bottom:4px}}"),
                shaped,
            );
            let a = get(&root, id(&dom, "a"));
            assert_eq!(root.lines.len(), 1);
            assert_eq!(a.lines.len(), 2);
            let expected = if overflow == "visible" {
                a.lines.last().unwrap().baseline
            } else {
                a.border_box.y + a.border_box.h + 4.0
            };
            assert!((root.lines[0].baseline - expected).abs() < 0.02,
                "shaped={shaped} overflow={overflow}: outer baseline {}, inner baseline {}",
                root.lines[0].baseline, expected);
        }
    }
}

#[test]
fn inline_block_contains_its_float_and_child_margins() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><span id=a><b id=float></b><i id=child></i></span>end</body>",
            "#a{display:inline-block}#float{display:block;float:left;width:20px;height:25px}#child{display:block;margin:10px 0;width:10px;height:5px}",
            shaped,
        );
        let a = get(&root, id(&dom, "a"));
        assert!(a.border_box.h >= 25.0);
        assert!(a.border_box.w >= 20.0);
        assert!(root.lines[0].replaced.iter().any(|frag| frag.node == id(&dom, "a")));
    }
}

#[test]
fn inline_block_top_and_bottom_vertical_align_share_line_edges() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><span id=top></span><span id=bottom></span></body>",
            "span{display:inline-block;width:10px}#top{height:24px;vertical-align:top}#bottom{height:8px;vertical-align:bottom}",
            shaped,
        );
        assert_eq!(root.lines.len(), 1);
        let top = get(&root, id(&dom, "top"));
        let bottom = get(&root, id(&dom, "bottom"));
        near(top.border_box.y, root.lines[0].y_top);
        near(bottom.border_box.y + bottom.border_box.h, root.lines[0].y_top + root.lines[0].height);
    }
}

#[test]
fn anonymous_line_owns_its_atomic_subtrees_after_block_sibling() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><p>intro</p><span id=a><i id=child></i></span><span id=b>next</span></body>",
            "p{margin:0}span{display:inline-block;width:30px;height:25px}i{display:block;width:20px;height:20px}",
            shaped,
        );
        let Flow::Blocks(blocks) = &root.flow else { panic!("block sibling") };
        let line = blocks.last().unwrap();
        assert_eq!(line.positioned.len(), 2);
        assert!(root.positioned.is_empty());
        assert_eq!(line.lines.len(), 1);
        let a = get(&root, id(&dom, "a"));
        let b = get(&root, id(&dom, "b"));
        let child = get(&root, id(&dom, "child"));
        near(a.border_box.w, 30.0);
        near(b.border_box.x, a.border_box.x + 30.0);
        near(child.border_box.x, a.content_box.x);
        near(child.border_box.y, a.content_box.y);
    }
}

#[test]
fn reshaped_atomic_line_moves_absolute_but_not_fixed_descendants() {
    for shaped in [false, true] {
        let (dom, mut root) = page(
            "<body><p>intro</p><span id=parent><i id=absolute></i><b id=fixed></b></span></body>",
            "p{margin:0}#parent{display:inline-block;width:100px;height:100px}#absolute{position:absolute;width:100px;height:100px}#fixed{position:fixed;left:17px;top:3px;width:5px;height:5px}",
            shaped,
        );
        let parent_node = id(&dom, "parent");
        let fixed_node = id(&dom, "fixed");
        let fixed_x = get(&root, fixed_node).border_box.x;
        let Flow::Blocks(blocks) = &mut root.flow else { panic!("anonymous line") };
        let anon = blocks.last_mut().unwrap();
        let fragment = anon.lines[0].replaced.iter_mut()
            .find(|fragment| fragment.node == parent_node).unwrap();
        fragment.x -= 0.8;
        block::place_atomic_inlines(anon);
        let parent = get(&root, parent_node);
        let absolute = get(&root, id(&dom, "absolute"));
        near(absolute.border_box.x, parent.border_box.x);
        near(get(&root, fixed_node).border_box.x, fixed_x);
    }
}

#[test]
fn inline_flex_intrinsic_width_includes_row_items_and_gaps() {
    for shaped in [false, true] {
        let (dom, root) = page(
            "<body><span id=flex><b></b><i></i></span></body>",
            "#flex{display:inline-flex;gap:20px}b{width:30px;height:10px}i{width:40px;height:10px}",
            shaped,
        );
        near(get(&root, id(&dom, "flex")).border_box.w, 90.0);
    }
}

#[test]
fn contents_blocks_ignore_wrapper_size_margin_float_and_position() {
    for shaped in [false, true] {
        for position in ["static", "relative", "absolute", "fixed"] {
            let (dom, b) = page(
                "<body><article id=w><div id=a></div><section><div id=b></div></section></article>",
                &format!("article,section{{display:contents;width:9px;height:99px;margin:40px;padding:30px;border:5px solid red;float:right;position:{position};left:80px}}div{{width:20px;height:12px}}"), shaped,
            );
            assert!(boxes(&b).iter().all(|b| b.node != Some(id(&dom, "w"))));
            near(get(&b, id(&dom, "a")).border_box.x, 0.0);
            near(get(&b, id(&dom, "a")).border_box.y, 0.0);
            near(get(&b, id(&dom, "b")).border_box.y, 12.0);
            near(b.content_box.h, 24.0);
            assert!(b.positioned.is_empty());
        }
    }
}

#[test]
fn contents_promotes_flex_and_grid_items_in_source_order() {
    for shaped in [false, true] {
        for display in ["flex", "grid"] {
            let (dom,b)=page("<body><main id=host><section id=w><div id=a></div><article><div id=b></div></article></section></main>",
                &format!("main{{display:{display};width:100px;grid-template-columns:20px 20px;align-items:start}}section,article{{display:contents;border:20px solid red}}div{{width:20px;height:10px}}"),shaped);
            let host = get(&b, id(&dom, "host"));
            let children = match &host.flow {
                Flow::Flex(c) | Flow::Grid(c) => c,
                _ => panic!("item context"),
            };
            assert_eq!(children.len(), 2);
            assert_eq!(children[0].node, Some(id(&dom, "a")));
            assert_eq!(children[1].node, Some(id(&dom, "b")));
            near(children[0].border_box.x, 0.0);
            near(children[1].border_box.x, 20.0);
        }
    }
}

#[test]
fn contents_text_stays_one_anonymous_flex_item_across_wrappers() {
    for shaped in [false, true] {
        let (dom,b)=page("<body><main id=host>A<section>B<article>C</article></section>D<div id=box></div></main>",
            "main{display:flex}section,article{display:contents}div{width:10px;height:10px}",shaped);
        let Flow::Flex(c) = &get(&b, id(&dom, "host")).flow else {
            panic!("flex")
        };
        assert_eq!(c.len(), 2);
        assert!(c[0].node.is_none());
        let Flow::Inlines(items) = &c[0].flow else {
            panic!("text item")
        };
        let text: String = items
            .iter()
            .filter_map(|n| match n {
                InlineNode::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "ABCD");
        assert_eq!(c[1].node, Some(id(&dom, "box")));
    }
}

#[test]
fn contents_keeps_text_inheritance_but_not_its_decoration_or_bidi_box() {
    for shaped in [false, true] {
        let (_,b)=page("<body><section>A<span>B</span></section>",
            "section{display:contents;font-size:20px;color:green;background:red;padding:30px;unicode-bidi:isolate}span{font-size:50%}",shaped);
        let Flow::Inlines(items) = &b.flow else {
            panic!("inline")
        };
        let texts: Vec<_> = items
            .iter()
            .filter_map(|n| match n {
                InlineNode::Text {
                    font_size,
                    color,
                    background_color,
                    inline_padding,
                    source_mappings,
                    ..
                } => Some((
                    *font_size,
                    color,
                    background_color,
                    inline_padding,
                    source_mappings,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2);
        near(texts[0].0, 20.0);
        near(texts[1].0, 10.0);
        for (_, color, bg, padding, mappings) in texts {
            assert_eq!(color, "green");
            assert_eq!(bg, "transparent");
            near(padding.left, 0.0);
            near(padding.right, 0.0);
            assert!(!mappings.is_empty());
        }
        assert!(!items
            .iter()
            .any(|n| matches!(n, InlineNode::BidiStart { .. } | InlineNode::BidiEnd)));
    }
}

#[test]
fn contents_absolute_descendants_use_nearest_real_positioned_box() {
    for shaped in [false, true] {
        let css="main{position:relative;margin:20px;padding:5px;border:2px solid blue;width:100px;height:50px}section{display:contents;position:relative;left:80px;padding:70px}div{position:absolute;left:7px;top:9px;width:10px;height:10px}";
        let (dom, b) = page(
            "<body><main><section id=w><div id=abs></div></section></main>",
            css,
            shaped,
        );
        let (control_dom, control) = page("<body><main><div id=abs></div></main>", css, shaped);
        let actual = get(&b, id(&dom, "abs"));
        let expected = get(&control, id(&control_dom, "abs"));
        near(actual.border_box.x, expected.border_box.x);
        near(actual.border_box.y, expected.border_box.y);
        assert!(boxes(&b).iter().all(|b| b.node != Some(id(&dom, "w"))));
    }
}

#[test]
fn contents_cells_join_before_anonymous_table_repair() {
    for shaped in [false, true] {
        let (_, b) = page(
            "<body><table><tr><td>P</td><td>A</td><td>S</td><td>S</td></tr></table>",
            "table{border-spacing:0}td{display:contents;padding:30px;border:10px solid red}",
            shaped,
        );
        let cells: Vec<_> = boxes(&b)
            .into_iter()
            .filter(|b| crate::table_tree::role(b) == crate::table_tree::Role::Cell)
            .collect();
        assert_eq!(cells.len(), 1);
        assert!(cells[0].node.is_none());
        near(cells[0].padding.left, 0.0);
        near(cells[0].border.left, 0.0);
        let words: Vec<_> = boxes(cells[0])
            .into_iter()
            .flat_map(|b| &b.lines)
            .flat_map(|l| &l.words)
            .collect();
        assert_eq!(
            words.iter().map(|w| w.text.as_str()).collect::<String>(),
            "PASS"
        );
    }
}

#[test]
fn contents_unusual_elements_suppress_subtrees_but_buttons_unbox() {
    for shaped in [false, true] {
        let (_,b)=page("<body>P<br><object>FAIL</object><input><textarea>FAIL</textarea><select><option>FAIL</option></select><button>A<span>S</span></button>S",
            "br,object,input,textarea,select,button{display:contents;border:10px solid red;padding:20px}",shaped);
        assert!(b.styled_replaced_fragments().next().is_none());
        assert_eq!(b.lines.len(), 1);
        assert_eq!(
            b.lines
                .iter()
                .flat_map(|l| &l.words)
                .map(|w| w.text.as_str())
                .collect::<String>(),
            "PASS"
        );
    }
}

#[test]
fn contents_body_has_only_anonymous_viewport_host() {
    for shaped in [false, true] {
        let (_,b)=page("<body>PASS", "body{display:contents;width:30px;margin:40px;padding:20px;border:10px solid red;background:red;position:relative;left:90px;color:green}",shaped);
        assert!(b.node.is_none());
        near(b.content_box.x, 0.0);
        near(b.content_box.w, 200.0);
        near(b.padding.left, 0.0);
        near(b.border.left, 0.0);
        let Flow::Inlines(items) = &b.flow else {
            panic!("text")
        };
        assert!(matches!(&items[0],InlineNode::Text{color,..} if color=="green"));
    }
}

#[test]
fn contents_does_not_resurrect_display_none_descendants() {
    for shaped in [false, true] {
        let (_, b) = page(
            "<body>P<section><article>FAIL</article>A</section>SS",
            "section{display:contents}article{display:none}",
            shaped,
        );
        assert_eq!(
            b.lines
                .iter()
                .flat_map(|l| &l.words)
                .map(|w| w.text.as_str())
                .collect::<String>(),
            "PASS"
        );
    }
}

#[test]
fn contents_pre_text_does_not_preserve_a_collapsible_space_from_its_sibling() {
    let (_, b) = page(
        "<body><p>heading</p>\n<section>Two\nlines</section>",
        "p{margin:0}section{display:contents;white-space:pre}",
        true,
    );
    let lines: Vec<_> = boxes(&b).into_iter().flat_map(|b| &b.lines).collect();
    let two = lines
        .iter()
        .flat_map(|l| &l.words)
        .find(|w| w.text == "Two")
        .unwrap();
    near(two.x, 0.0);
    let tail = lines
        .iter()
        .find(|l| l.words.iter().any(|w| w.text == "lines"))
        .unwrap();
    let first = lines
        .iter()
        .find(|l| l.words.iter().any(|w| w.text == "Two"))
        .unwrap();
    assert!(tail.y_top >= first.y_top + first.height);
}

#[test]
fn contents_empty_boxes_and_style_edges_do_not_add_inline_space() {
    for shaped in [false, true] {
        let (_, b) = page(
            "<body>A<span><section></section>B</span>C",
            "section{display:contents;margin:80px;padding:60px;border:20px solid red}",
            shaped,
        );
        let (_, reference) = page("<body>A<span>B</span>C", "", shaped);
        near(
            b.lines[0].words.last().unwrap().x + b.lines[0].words.last().unwrap().width,
            reference.lines[0].words.last().unwrap().x
                + reference.lines[0].words.last().unwrap().width,
        );
        assert_eq!(b.lines.len(), reference.lines.len());
    }
}

#[test]
fn contents_pre_spaces_keep_their_own_style_at_line_start() {
    for (prefix, pre, expected) in [("", "  Two", 6.0), (" ", "  Two", 6.0), (" ", "Two", 0.0)] {
        let (_, b) = page(
            &format!("<body><p>heading</p>{prefix}<section>{pre}</section>"),
            "p{margin:0}section{display:contents;white-space:pre}",
            true,
        );
        let line = boxes(&b)
            .into_iter()
            .flat_map(|b| &b.lines)
            .find(|line| line.words.iter().any(|word| word.text.ends_with("Two")))
            .unwrap();
        let word = line
            .words
            .iter()
            .find(|word| word.text.ends_with("Two"))
            .unwrap();
        let first_letter = line
            .glyphs
            .iter()
            .find(|glyph| glyph.node == word.node && glyph.source_range.start == pre.len() - 3)
            .unwrap();
        near(first_letter.x, expected);
    }
}


#[test]
fn rtl_column_wrapping_combines_direction_and_wrap_reverse() {
    for wrap in ["wrap", "wrap-reverse"] {
        let (dom, root) = page(
            "<body><div id=c><i id=a></i><i></i><i id=b></i></div></body>",
            &format!("#c{{display:flex;direction:rtl;flex-flow:column {wrap};width:100px;height:40px}}i{{width:30px;height:20px}}"), false);
        let a = get(&root, id(&dom, "a"));
        let b = get(&root, id(&dom, "b"));
        assert_eq!(a.border_box.x > b.border_box.x, wrap == "wrap");
    }
}

#[test]
fn lanes_auto_fit_collapses_internal_empty_tracks_in_both_axes() {
    for axis in ["columns", "rows"] {
        let place = if axis == "columns" { "column" } else { "row" };
        let (dom, root) = page(
            "<body><div id=c><i id=a></i><i id=b></i></div></body>",
            &format!("#c{{display:grid-lanes;grid-template-{axis}:repeat(auto-fit,50px);width:150px;height:150px}}i{{width:50px;height:50px}}#a{{grid-{place}:1}}#b{{grid-{place}:3}}"), false);
        let a = get(&root, id(&dom, "a"));
        let b = get(&root, id(&dom, "b"));
        near(if axis == "columns" { b.border_box.x-a.border_box.x } else { b.border_box.y-a.border_box.y }, 50.0);
    }
}

#[test]
fn positioned_stretch_is_definite_for_percentage_descendants() {
    let dom = amazing_html::parse("<body><div id=modal><div id=dialog><div id=wrapper><div id=content><div id=inner></div></div></div></div></div></body>");
    let sheet = amazing_css::parse_stylesheet("#modal{position:fixed;top:10px;bottom:20px;left:0;width:100px}#dialog,#wrapper,#inner{height:100%;width:100px}#wrapper{position:relative}#content{position:absolute;top:0;bottom:0;width:100px}");
    let styled = amazing_css::compute(&dom, &[sheet]);
    let mut root = box_tree::build_layout_tree(&dom, &styled, &font::FontSource::fallback());
    // CSS currently rejects stretch at declaration parsing (routed to CSS).
    // Exercise layout's computed-style seam without changing that ownership.
    let modal = &mut root.positioned[0];
    modal.style = modal.style.iter().map(|(name, value)|
        (name.to_owned(), if name == "height" { "stretch".to_owned() } else { value.to_owned() }))
        .collect();
    block::layout_root_in_viewport(&mut root, 200.0, Some(300.0), &font::FontSource::fallback());
    near(get(&root, id(&dom, "inner")).border_box.h, 270.0);
}

#[test]
fn row_lanes_subgrid_chrome_matches_grid_sizing() {
    let html = "<body><div id=outer><div id=c><section id=s><i>A</i><i>B</i></section></div></div></body>";
    let css = "#outer{display:flex;width:200px}#c{grid-template-rows:repeat(2,60px);gap:10px}#s{display:grid;grid-template-rows:subgrid;grid-row:1/-1;grid-auto-flow:column;border:9px solid;padding:6px}i{width:80px;padding:8px;border:1px solid}";
    let (grid_dom, grid) = page(html, &format!("{css}#c{{display:grid}}"), false);
    let (lanes_dom, lanes) = page(html, &format!("{css}#c{{display:grid-lanes}}"), false);
    near(get(&lanes, id(&lanes_dom, "s")).border_box.w, get(&grid, id(&grid_dom, "s")).border_box.w);
}
