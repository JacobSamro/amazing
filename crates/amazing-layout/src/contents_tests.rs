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
