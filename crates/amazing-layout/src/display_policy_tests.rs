//! Actual box/glyph geometry for compound outer/inner/marker policies.
use crate::*;
use amazing_dom::Dom;
use amazing_text::{FallbackTextSystem, GlyphId};

fn layout(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = amazing_html::parse(&format!("<body>{html}"));
    let root = layout_dom_document_with_text_and_replaced_in_viewport(&dom,
        &format!("body{{margin:0;font:20px/20px serif}}{css}"),
        300.0, Some(300.0), &FallbackTextSystem::new(), &NoReplacedElements);
    (dom,root)
}
fn find<'a>(root: &'a LayoutBox, dom: &Dom, id: &str) -> Option<&'a LayoutBox> {
    if root.node.is_some_and(|node| dom.attrs_of(node).is_some_and(|attrs|
        attrs.iter().any(|(key,value)| key == "id" && value == id))) { return Some(root); }
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &root.flow {
        for child in children { if let Some(found) = find(child,dom,id) { return Some(found); } }
    }
    root.positioned.iter().find_map(|child| find(child,dom,id))
}
fn glyph_count(root: &LayoutBox, glyph: char) -> usize {
    let mut count = root.lines.iter().flat_map(|line| &line.glyphs)
        .filter(|frag| frag.glyph == GlyphId(glyph as u32)).count();
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &root.flow {
        count += children.iter().map(|child| glyph_count(child,glyph)).sum::<usize>();
    }
    count + root.positioned.iter().map(|child| glyph_count(child,glyph)).sum::<usize>()
}

#[test]
fn compound_marker_flag_preserves_block_inline_and_atomic_outer_geometry() {
    for (display,block,atomic) in [("list-item",true,false),("flow-root list-item",true,false),
        ("list-item flow-root",true,false),("inline list-item",false,false),
        ("list-item inline",false,false),("inline flow-root list-item",false,true),
        ("list-item inline flow-root",false,true)] {
        let (dom,root) = layout("A<span id='target'>B</span>C",&format!(
            "#target{{display:{display};width:40px;height:20px;list-style-position:inside}}#target::marker{{content:'X'}}"));
        assert_eq!(glyph_count(&root,'X'),1,"marker {display}");
        assert_eq!(glyph_count(&root,'B'),1,"principal {display}");
        if block {
            let target = find(&root,&dom,"target").unwrap();
            assert_eq!(target.display,Display::Block);
            assert!(!target.inline_atomic);
            assert!(matches!(&root.flow,Flow::Blocks(_)));
        } else if atomic {
            let target = find(&root,&dom,"target").unwrap();
            assert!(target.inline_atomic);
            assert!((target.border_box.w-40.0).abs()<0.01);
            assert!((target.border_box.h-20.0).abs()<0.01);
            let Flow::Inlines(items) = &root.flow else { panic!("inline atomic {display}") };
            assert!(items.iter().any(|item| matches!(item,InlineNode::Atomic{..})));
        } else {
            assert!(find(&root,&dom,"target").is_none(),"non-atomic inline {display}");
            assert!(matches!(&root.flow,Flow::Inlines(_)));
        }
    }
}

#[test]
fn flow_root_marker_boxes_trap_float_height_and_child_margin() {
    for display in ["flow-root list-item","list-item flow-root","block list-item flow-root"] {
        let (dom,root) = layout("<span id='target'><div id='float'></div><div id='normal'></div></span>",&format!(
            "body{{font-size:0;line-height:0}}#target{{display:{display};list-style-type:none}}#float{{float:left;width:30px;height:40px}}#normal{{height:10px;margin-top:20px}}"));
        let target = find(&root,&dom,"target").unwrap();
        let normal = find(&root,&dom,"normal").unwrap();
        assert!(crate::block::establishes_bfc(&target.style));
        assert!((target.content_box.h-40.0).abs()<0.01,"{display}: {:?}",target.content_box);
        assert!((normal.border_box.y-target.content_box.y-20.0).abs()<0.01,"{display}: {:?}",normal.border_box);
    }
    let (dom,root) = layout("<span id='target'><div id='float'></div><div id='normal'></div></span>",
        "body{font-size:0;line-height:0}#target{display:list-item;list-style-type:none}#float{float:left;width:30px;height:40px}#normal{height:10px;margin-top:20px}");
    let target = find(&root,&dom,"target").unwrap();
    assert!(!crate::block::establishes_bfc(&target.style));
    assert!(target.content_box.h<40.0,"ordinary list flow must not enclose float");
}

#[test]
fn inline_marker_flags_do_not_change_containment_applicability() {
    for display in ["inline list-item","inline flow-root list-item"] {
        let (dom,root) = layout("A<span id='target'>BBBB</span>C",&format!(
            "#target{{display:{display};contain:size layout;contain-intrinsic-size:55px 25px;list-style-type:none}}"));
        if display == "inline list-item" {
            assert!(find(&root,&dom,"target").is_none());
            assert_eq!(glyph_count(&root,'B'),4);
        } else {
            let target = find(&root,&dom,"target").unwrap();
            assert!(target.inline_atomic);
            assert!((target.content_box.w-55.0).abs()<0.01,"{:?}",target.content_box);
            assert!((target.content_box.h-25.0).abs()<0.01,"{:?}",target.content_box);
        }
    }
}

#[test]
fn expanded_original_types_preserve_static_slot_flags_and_inner_layout() {
    for display in ["inline list-item","inline flow-root list-item","run-in list-item",
        "run-in flow-root list-item","flow-root list-item","run-in flex","run-in grid",
        "run-in table","run-in ruby"] {
        for position in ["absolute","fixed"] {
            let (dom,root) = layout("A<div id='target'>B</div>C",&format!(
                "#target{{display:{display};position:{position};list-style-position:inside}}#target::marker{{content:'X'}}"));
            let target = find(&root,&dom,"target").unwrap();
            assert_eq!(target.originally_inline_level,display != "flow-root list-item","{display};{position}");
            if display.ends_with("flex") { assert!(matches!(&target.flow,Flow::Flex(_))); }
            if display.ends_with("grid") { assert!(matches!(&target.flow,Flow::Grid(_))); }
            if display.ends_with("table") { assert!(target.table_wrapper); }
            if display.ends_with("ruby") { assert!(matches!(&target.flow,Flow::Inlines(items) if items.iter().any(|item| matches!(item,InlineNode::Ruby{..})))); }
            assert_eq!(glyph_count(&root,'X'),usize::from(display.contains("list-item")),"{display};{position}");
        }
    }
    // Type decomposition preserves run-in inner contexts; sequence insertion
    // into a following sibling still requires the separate box-tree algorithm.
    let (dom,table) = layout("A<div id='target'><div class='row'><span class='cell'>B</span></div></div>C",
        "#target{display:run-in table}.row{display:table-row}.cell{display:table-cell}");
    let target = find(&table,&dom,"target").unwrap();
    assert!(target.table_wrapper && target.inline_atomic);
    let (_,ruby) = layout("A<div id='target'><rb>B</rb><rt>C</rt></div>D","#target{display:run-in ruby}");
    let Flow::Inlines(items) = &ruby.flow else { panic!("ruby outer") };
    assert!(items.iter().any(|item| matches!(item,InlineNode::Ruby{columns,..}
        if columns.iter().any(|column| column.annotation.as_ref().is_some_and(|note| note.text == "C")))));
}
