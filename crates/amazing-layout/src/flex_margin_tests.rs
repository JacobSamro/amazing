use super::*;
use crate::{block, Flow};

fn layout(css: &str, html: &str) -> LayoutBox {
    let dom = amazing_html::parse(&format!("<body><section>{html}</section></body>"));
    let sheet = amazing_css::parse_stylesheet(&format!("body{{margin:0}}section{{display:flex;width:100px;height:100px}}section>*{{flex:none;min-width:0;width:20px;height:20px}}{css}"));
    let styled = amazing_css::compute(&dom, &[sheet]);
    let fonts = FontSource::fallback();
    let mut root = crate::box_tree::build_layout_tree(&dom, &styled, &fonts);
    block::layout_root(&mut root, 300.0, &fonts);
    root
}
fn items(root: &LayoutBox) -> &[LayoutBox] {
    let Flow::Blocks(children) = &root.flow else { panic!("body blocks") };
    let Flow::Flex(items) = &children[0].flow else { panic!("section flex") };
    items
}
fn near(actual: f32, expected: f32) { assert!((actual - expected).abs() < 0.01, "actual {actual}, expected {expected}"); }

#[test]
fn main_auto_margins_share_space_without_rewriting_declarations() {
    for (direction, coordinate) in [("row", true), ("row-reverse", true), ("column", false), ("column-reverse", false)] {
        let sides = if coordinate { "margin-left:auto;margin-right:auto" } else { "margin-top:auto;margin-bottom:auto" };
        let mut root = layout(&format!("section{{flex-direction:{direction}}}div{{{sides}}}"), "<div></div><div></div>");
        let children = items(&root);
        let positions = if coordinate { [children[0].border_box.x, children[1].border_box.x] } else { [children[0].border_box.y, children[1].border_box.y] };
        let expected = if direction.ends_with("reverse") { [65.0, 15.0] } else { [15.0, 65.0] };
        for (a,b) in positions.into_iter().zip(expected) { near(a,b); }
        for child in children { assert_eq!(child.style.get(if coordinate { "margin-left" } else { "margin-top" }), Some("auto")); }
        // Reflow the very same boxes. Used margin values from the previous
        // solve must never masquerade as new specified fixed lengths.
        block::layout_root(&mut root, 400.0, &FontSource::fallback());
        let again = items(&root);
        near(if coordinate { again[0].border_box.x } else { again[0].border_box.y }, expected[0]);
    }
}

#[test]
fn cross_auto_sides_override_item_alignment_in_both_axes() {
    for (direction, start, end, row) in [("row","top","bottom",true),("column","left","right",false)] {
        for (declarations, offset) in [(format!("margin-{start}:auto"),80.0),(format!("margin-{end}:auto"),0.0),(format!("margin-{start}:auto;margin-{end}:auto"),40.0)] {
            for wrap in ["nowrap", "wrap-reverse"] {
                let root = layout(&format!("section{{flex-direction:{direction};flex-wrap:{wrap};align-items:center}}div{{{declarations}}}"),"<div></div>");
                let item=&items(&root)[0];
                near(if row {item.border_box.y}else{item.border_box.x},offset);
                near(if row {item.border_box.h}else{item.border_box.w},20.0);
                if declarations.contains(&format!("margin-{start}")) { assert_eq!(item.style.get(&format!("margin-{start}")),Some("auto")); }
            }
        }
    }
}

#[test]
fn cross_overflow_keeps_start_and_resolves_opposite_signed_margin() {
    for (direction, row, sides) in [("row",true,"margin-top:auto;margin-bottom:auto;height:120px"),("column",false,"margin-left:auto;margin-right:auto;width:120px")] {
        for wrap in ["nowrap","wrap-reverse"] {
            let root=layout(&format!("section{{flex-direction:{direction};flex-wrap:{wrap};align-content:stretch}}div{{{sides}}}"),"<div></div>");
            let item=&items(&root)[0];
            near(if row{item.border_box.h}else{item.border_box.w},120.0);
            near(if row{item.border_box.y}else{item.border_box.x},if wrap=="nowrap"{0.0}else{-20.0});
            if wrap=="nowrap" { near(if row{item.margin.bottom.unwrap()}else{item.margin.right.unwrap()},-20.0); }
        }
    }
}

#[test]
fn row_center_and_end_use_margin_box_without_double_counting_edges() {
    for (alignment, expected) in [("center",36.0),("flex-end",65.0)] {
        let root=layout(&format!("section{{align-items:{alignment}}}div{{padding:5px;border:2px solid;margin-top:7px;margin-bottom:1px}}"),"<div></div>");
        let item=&items(&root)[0];
        near(item.border_box.h,34.0);
        near(item.border_box.y,expected);
    }
}

#[test]
fn stretched_wrapped_lines_realigned_after_cross_space_distribution() {
    for wrap in ["wrap","wrap-reverse"] {
        let root=layout(&format!("section{{width:30px;flex-wrap:{wrap};align-content:stretch}}div{{margin-top:auto}}"),"<div></div><div></div>");
        let children=items(&root);
        if wrap=="wrap" {near(children[0].border_box.y,30.0);near(children[1].border_box.y,80.0);}
        else {near(children[0].border_box.y,80.0);near(children[1].border_box.y,30.0);}
        for child in children {near(child.margin.top.unwrap(),30.0);assert_eq!(child.style.get("margin-top"),Some("auto"));}
    }
}

#[test]
fn auto_cross_margins_disable_stretch_and_preserve_nested_source_geometry() {
    let root=layout("section{align-items:stretch}section>div{height:auto;margin-top:auto;margin-bottom:auto}span{display:block;height:10px}","<div><span></span></div>");
    let item=&items(&root)[0];
    near(item.border_box.h,10.0);near(item.border_box.y,45.0);
    let Flow::Blocks(children)=&item.flow else {panic!("real descendant")};
    near(children[0].border_box.y,45.0);
    assert!(children[0].node.is_some());
}

#[test]
fn signed_margins_and_safe_overflow_align_actual_margin_box() {
    for (direction, row, size) in [("row",true,"height"),("column",false,"width")] {
        for (alignment,offset) in [("unsafe center",-10.0),("safe center",0.0),("unsafe flex-end",-20.0),("safe flex-end",0.0)] {
            let root=layout(&format!("section{{flex-direction:{direction};align-items:{alignment}}}div{{{size}:120px}}"),"<div></div>");
            let item=&items(&root)[0];near(if row{item.border_box.h}else{item.border_box.w},120.0);near(if row{item.border_box.y}else{item.border_box.x},offset);
        }
    }
    let root=layout("section{align-items:center}div{margin-top:-10px;margin-bottom:10px}","<div></div>");
    near(items(&root)[0].border_box.y,30.0);
}

#[test]
fn wrapped_column_line_packing_and_auto_cross_margins_use_final_line_width() {
    let root=layout("section{flex-direction:column;flex-wrap:wrap;align-content:space-between}div{height:60px;margin-left:auto}","<div></div><div></div>");
    let children=items(&root);near(children[0].border_box.x,0.0);near(children[1].border_box.x,80.0);
    for child in children{near(child.border_box.h,60.0);near(child.border_box.w,20.0);assert_eq!(child.style.get("margin-left"),Some("auto"));}
}

#[test]
fn overflow_auto_margin_start_follows_direction_not_wrap_reversal() {
    for (flow, mode) in [("column","direction:rtl"),("row","writing-mode:vertical-rl")] {
        let root=layout(&format!("section{{flex-direction:{flow};{mode}}}div{{width:120px;margin-left:auto;margin-right:auto}}"),"<div></div>");
        let item=&items(&root)[0];near(item.border_box.w,120.0);near(item.border_box.x,-20.0);
        near(item.margin.left.unwrap(),-20.0);near(item.margin.right.unwrap(),0.0);
        assert_eq!(item.style.get("margin-left"),Some("auto"));assert_eq!(item.style.get("margin-right"),Some("auto"));
    }
    // Positive free space still follows the authored physical auto side.
    let root=layout("section{flex-direction:column;direction:rtl}div{margin-left:auto}","<div></div>");
    near(items(&root)[0].border_box.x,80.0);
}

#[test]
fn definite_line_stretch_respects_author_limits_and_natural_overflow() {
    for (rule,expected) in [("max-height:30px",30.0),("min-height:130px",130.0),("box-sizing:border-box;max-height:40px;padding:5px;border:2px solid",40.0)] {
        let root=layout(&format!("section{{align-items:stretch}}section>div{{height:auto;{rule}}}"),"<div></div>");
        let item=&items(&root)[0];near(item.border_box.h,expected);near(item.border_box.y,0.0);
        assert_eq!(item.style.get("height"),Some("auto"));
    }
    let root=layout("section{align-items:stretch}section>div{height:auto}span{display:block;height:120px}","<div><span></span></div>");
    let item=&items(&root)[0];near(item.border_box.h,100.0);
    let Flow::Blocks(children)=&item.flow else{panic!("actual overflowing content")};
    near(children[0].border_box.h,120.0);near(children[0].border_box.y,item.content_box.y);
}
