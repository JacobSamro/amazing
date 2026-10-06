use crate::{LayoutBox,Flow};
use crate::font::FontSource;
use amazing_dom::Dom;

fn find<'a>(root: &'a LayoutBox, dom: &Dom, id: &str) -> &'a LayoutBox {
    fn walk<'a>(root: &'a LayoutBox, dom: &Dom, id: &str) -> Option<&'a LayoutBox> {
        if root.node.and_then(|node|dom.attrs_of(node)).is_some_and(|attrs|attrs.iter().any(|(name,value)|name=="id" && value==id)) {return Some(root);}
        match &root.flow {Flow::Blocks(k)|Flow::Flex(k)|Flow::Grid(k)=>k.iter().find_map(|c|walk(c,dom,id)),_=>None}
    }
    walk(root,dom,id).unwrap()
}
fn document(markup: &str, css: &str) -> (Dom,LayoutBox) {
    let dom=amazing_html::parse(&format!("<body><div id=g>{markup}</div></body>"));
    let css=format!("body{{margin:0}}#g{{display:grid;width:200px;grid-template-columns:100px 100px;{css}}}#a{{font-size:12px}}#b{{font-size:30px}}");
    let root=crate::layout_dom_document(&dom,&css,800.0,&FontSource::fallback());(dom,root)
}
fn close(a:f32,b:f32){assert!((a-b).abs()<0.02,"{a} != {b}");}
fn first(b:&LayoutBox)->f32 {super::first_baseline(b)}
fn last(b:&LayoutBox)->f32 {super::last_baseline(b)}
const PAIR:&str="<div id=a>small</div><div id=b>large</div>";
#[test]
fn native_first_baselines_share_a_row_across_font_sizes(){
    let (dom,root)=document(PAIR,"align-items:first baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_baseline_alias_inherits_per_item(){
    let (dom,root)=document(PAIR,"align-items:baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_explicit_self_baseline_overrides_items(){
    let markup="<div id=a style='align-self:first baseline'>small</div><div id=b style='align-self:baseline'>large</div>";
    let (dom,root)=document(markup,"align-items:end;grid-template-rows:120px");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_actual_nested_text_baselines_participate(){
    let (dom,root)=document("<div id=a><section>small</section></div><div id=b><section>large</section></div>","align-items:baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_real_padding_border_and_asymmetric_margins_participate(){
    let markup="<div id=a style='padding:7px;border:3px solid;margin-top:11px;margin-bottom:5px'>small</div><div id=b style='padding:2px;border:1px solid;margin-top:3px;margin-bottom:9px'>large</div>";
    let (dom,root)=document(markup,"align-items:baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_baseline_shim_grows_an_intrinsic_row(){
    let markup="<div id=a style='height:120px'>small</div><div id=b style='font-size:80px'>large</div>";
    let (dom,root)=document(markup,"align-items:baseline;grid-template-rows:auto");let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");let g=find(&root,&dom,"g");
    close(first(a),first(b));assert!(g.content_box.h>120.0);assert!(g.content_box.h+0.02>=a.border_box.y+a.border_box.h-g.content_box.y);
}
#[test]
fn native_last_baselines_use_the_actual_final_text_line(){
    let markup="<div id=a>small<br>second</div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:last baseline;grid-template-rows:120px");let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");close(last(a),last(b));assert!((first(a)-first(b)).abs()>1.0);
}
#[test]
fn native_last_shim_grows_intrinsic_rows_from_real_descent(){
    let markup="<div id=a style='height:120px'>small</div><div id=b style='font-size:80px'>large</div>";
    let (dom,root)=document(markup,"align-items:last baseline;grid-template-rows:auto");let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");close(last(a),last(b));assert!(find(&root,&dom,"g").content_box.h>120.0);
}
#[test]
fn native_singleton_last_baseline_has_end_fallback(){
    let (dom,root)=document("<div id=a>small</div>","align-items:last baseline;grid-template-rows:100px");let a=find(&root,&dom,"a");let g=find(&root,&dom,"g");close(a.border_box.y+a.border_box.h,g.content_box.y+100.0);
}
#[test]
fn native_auto_block_margin_excludes_baseline_sharing(){
    let markup="<div id=a style='margin-top:auto'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:baseline;grid-template-rows:100px");let a=find(&root,&dom,"a");let g=find(&root,&dom,"g");close(a.border_box.y+a.border_box.h,g.content_box.y+100.0);assert!((first(a)-first(find(&root,&dom,"b"))).abs()>1.0);
}
#[test]
fn native_first_baseline_multirow_span_shares_its_start_row(){
    let markup="<div id=a style='grid-row:1 / 3;grid-column:1'>small</div><div id=b style='grid-row:1;grid-column:2'>large</div>";
    let (dom,root)=document(markup,"align-items:baseline;grid-template-rows:80px 80px;row-gap:10px");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_last_baseline_multirow_span_shares_its_end_row(){
    let markup="<div id=a style='grid-row:1 / 3;grid-column:1'>small</div><div id=b style='grid-row:2;grid-column:2'>large</div>";
    let (dom,root)=document(markup,"align-items:last baseline;grid-template-rows:80px 80px;row-gap:10px");close(last(find(&root,&dom,"a")),last(find(&root,&dom,"b")));
}
#[test]
fn native_separate_rows_have_separate_baseline_groups(){
    let markup="<div id=a>small</div><div id=b>large</div><div id=c style='font-size:40px'>next</div><div id=d style='font-size:18px'>row</div>";
    let (dom,root)=document(markup,"align-items:baseline;grid-template-rows:80px 80px;row-gap:10px");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));close(first(find(&root,&dom,"c")),first(find(&root,&dom,"d")));assert!(first(find(&root,&dom,"c"))>first(find(&root,&dom,"a"))+20.0);
}
#[test]
fn native_empty_item_synthesizes_a_border_end_baseline(){
    let markup="<div id=a style='height:50px'></div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:baseline");let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");close(a.border_box.y+a.border_box.h,first(b));
}
#[test]
fn native_layout_containment_suppresses_a_descendant_text_baseline(){
    let markup="<div id=a style='contain:layout;height:50px'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:baseline");let a=find(&root,&dom,"a");close(first(a),a.border_box.y+a.border_box.h);close(first(a),first(find(&root,&dom,"b")));
}
#[test]
fn native_last_baseline_of_an_empty_box_is_its_border_end(){
    let markup="<div id=a style='height:50px'></div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:last baseline;grid-template-rows:100px");let a=find(&root,&dom,"a");close(a.border_box.y+a.border_box.h,last(find(&root,&dom,"b")));
}
#[test]
fn native_out_of_flow_text_does_not_supply_first_or_last_baselines(){
    let markup="<div id=a><section style='position:absolute;font-size:80px'>outside</section><section>small</section><section style='position:fixed;font-size:100px'>fixed</section></div><div id=b>large</div>";
    for position in ["first baseline","last baseline"] {
        let (dom,root)=document(markup,&format!("align-items:{position}"));let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");if position=="first baseline" {close(first(a),first(b));}else {close(last(a),last(b));}
    }
}
#[test]
fn native_scroll_container_baseline_is_clamped_to_border_box(){
    for overflow in ["hidden","scroll","auto"] {
        let markup=format!("<div id=a style='height:5px;overflow:{overflow}'>small</div><div id=b>large</div>");
        let (dom,root)=document(&markup,"align-items:baseline");let a=find(&root,&dom,"a");close(first(a),a.border_box.y+a.border_box.h);close(first(a),first(find(&root,&dom,"b")));
    }
}
#[test]
fn native_inline_auto_margins_do_not_exclude_block_baseline_sharing(){
    let markup="<div id=a style='width:20px;margin-left:auto;margin-right:auto'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_last_and_first_preferences_have_separate_groups(){
    let markup="<div id=a style='align-self:first baseline'>small</div><div id=b style='align-self:last baseline'>large</div>";
    let (dom,root)=document(markup,"grid-template-rows:100px");let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");let g=find(&root,&dom,"g");close(a.border_box.y,g.content_box.y);close(b.border_box.y+b.border_box.h,g.content_box.y+100.0);
}
#[test]
fn native_row_gap_does_not_enter_the_shared_baseline_shim(){
    for gap in [0,5,30] {
        let (dom,root)=document(PAIR,&format!("align-items:baseline;row-gap:{gap}px"));close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
    }
}
#[test]
fn native_percentage_size_in_intrinsic_rows_uses_baseline_fallback(){
    let markup="<div id=a style='height:50%'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"height:100px;grid-template-rows:auto;align-items:first baseline");assert!((first(find(&root,&dom,"a"))-first(find(&root,&dom,"b"))).abs()>1.0);
}
#[test]
fn native_percentage_size_in_flexible_auto_minimum_rows_uses_baseline_fallback(){
    let markup="<div id=a style='height:50%'>small</div><div id=b>large</div>";
    for tracks in ["1fr","minmax(auto,1fr)"] {
        let (dom,root)=document(markup,&format!("height:100px;grid-template-rows:{tracks};align-items:first baseline"));assert!((first(find(&root,&dom,"a"))-first(find(&root,&dom,"b"))).abs()>1.0);
    }
}
#[test]
fn native_percentage_size_in_definite_fixed_minimum_flexible_rows_can_share_baselines(){
    let markup="<div id=a style='height:50%'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"height:100px;grid-template-rows:minmax(0px,1fr);align-items:first baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}
#[test]
fn native_percentage_size_in_indefinite_flexible_rows_uses_baseline_fallback(){
    let markup="<div id=a style='height:50%'>small</div><div id=b>large</div>";
    for tracks in ["1fr","minmax(auto,1fr)","minmax(0px,1fr)"] {
        let (dom,root)=document(markup,&format!("grid-template-rows:{tracks};align-items:first baseline"));assert!((first(find(&root,&dom,"a"))-first(find(&root,&dom,"b"))).abs()>1.0);
    }
}
#[test]
fn native_percentage_calc_retains_its_cyclic_row_dependency(){
    let markup="<div id=a style='height:calc(50% + 1px)'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"height:100px;grid-template-rows:minmax(auto,1fr);align-items:first baseline");assert!((first(find(&root,&dom,"a"))-first(find(&root,&dom,"b"))).abs()>1.0);
}
#[test]
fn native_percentage_size_in_a_fixed_track_can_share_baselines(){
    let markup="<div id=a style='height:50%'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"height:100px;grid-template-rows:100px;align-items:first baseline");close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
}

#[test]
fn native_baseline_keywords_are_ascii_case_insensitive(){
    for keyword in ["FIRST BASELINE","BASELINE FIRST","BASELINE","LAST BASELINE","BASELINE LAST"] {
        let (dom,root)=document(PAIR,&format!("grid-template-rows:100px;align-items:{keyword}"));let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");if keyword.contains("LAST") {close(last(a),last(b));} else {close(first(a),first(b));}
    }
}
#[test]
fn native_last_group_overflow_uses_safe_start_without_breaking_baselines(){
    let markup="<div id=a style='height:120px'>small</div><div id=b>large</div>";
    let (dom,root)=document(markup,"align-items:last baseline;grid-template-rows:40px");let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");let g=find(&root,&dom,"g");close(last(a),last(b));assert!(a.border_box.y+0.02>=g.content_box.y);assert!(b.border_box.y+0.02>=g.content_box.y);
}
#[test]
fn native_each_auto_block_margin_combination_has_priority(){
    for (margin,edge) in [("margin-top:auto",0),("margin-bottom:auto",1),("margin-top:auto;margin-bottom:auto",2)] {
        let markup=format!("<div id=a style='{margin}'>small</div><div id=b>large</div>");
        let (dom,root)=document(&markup,"align-items:last baseline;grid-template-rows:100px");let a=find(&root,&dom,"a");let g=find(&root,&dom,"g");let offset=match edge {0=>100.0-a.border_box.h,1=>0.0,_=>(100.0-a.border_box.h)/2.0};close(a.border_box.y,g.content_box.y+offset);
    }
}


fn row_lanes_document(markup: &str, rows: &str, css: &str) -> (Dom, LayoutBox) {
    document(markup, &format!("display:grid-lanes;grid-template-columns:none;width:500px;grid-template-rows:{rows};{css}"))
}
#[test]
fn native_row_lanes_first_groups_use_unstretched_actual_item_heights() {
    let markup="<div id=a style='width:100px;min-height:30px;grid-row:1'>small</div><div id=b style='width:100px;min-height:50px;grid-row:1'>large</div>";
    let (dom,root)=row_lanes_document(markup,"100px 100px","align-items:first baseline;gap:3px");
    let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");
    close(first(a),first(b));close(a.border_box.h,30.0);close(b.border_box.h,50.0);
    close(b.border_box.x-a.border_box.x,103.0);
}
#[test]
fn native_row_lanes_last_groups_preserve_multiline_chrome_and_margins() {
    let markup="<div id=a style='width:100px;min-height:60px;padding:5px;margin-top:11px;margin-bottom:5px;grid-row:1'>small<br>second</div><div id=b style='width:100px;min-height:50px;padding:3px;margin-top:3px;margin-bottom:9px;grid-row:1'>large</div>";
    let (dom,root)=row_lanes_document(markup,"120px","align-items:last baseline;gap:3px");
    let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");close(last(a),last(b));
    assert!(a.border_box.h<100.0 && b.border_box.h<100.0);
    assert!((first(a)-first(b)).abs()>1.0);
}
#[test]
fn native_row_lanes_intrinsic_shims_match_real_ordinary_grid_references() {
    let markup="<div id=a style='height:60px;width:40px;grid-row:1;font-size:0'><span style='display:inline-block;width:10px;height:60px;vertical-align:bottom'></span></div><div id=b style='height:30px;width:40px;grid-row:1;font-size:0'><span style='display:inline-block;width:10px;height:10px;vertical-align:top'></span></div>";
    for rows in ["auto","minmax(auto,0px)","minmax(auto,75px)","minmax(auto,150px)","minmax(auto,auto)"] {
        let (dom,root)=row_lanes_document(markup,rows,"align-items:baseline;width:100px;font-size:0");
        let (reference_dom,reference)=document(markup,&format!("width:100px;grid-template-columns:40px 60px;grid-template-rows:{rows};align-items:baseline;font-size:0"));
        for id in ["a","b"] {
            let item=find(&root,&dom,id);let expected=find(&reference,&reference_dom,id);
            close(item.border_box.y-find(&root,&dom,"g").content_box.y,
                expected.border_box.y-find(&reference,&reference_dom,"g").content_box.y);
            close(item.border_box.h,expected.border_box.h);
        }
        close(first(find(&root,&dom,"a")),first(find(&root,&dom,"b")));
        close(find(&root,&dom,"g").content_box.h,find(&reference,&reference_dom,"g").content_box.h);
    }
}
#[test]
fn native_row_lanes_automatic_groups_follow_actual_placed_rows() {
    let markup="<div id=a style='width:50px;min-height:40px'>small</div><div id=b style='width:50px;min-height:50px'>large</div><div id=c style='width:50px;font-size:26px;min-height:40px'>third</div>";
    for alignment in ["first baseline","last baseline"] {
        let (dom,root)=row_lanes_document(markup,"100px 100px",&format!("align-items:{alignment};gap:3px;flow-tolerance:0"));
        let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");let c=find(&root,&dom,"c");let g=find(&root,&dom,"g");
        close(a.border_box.x,g.content_box.x);close(b.border_box.x,g.content_box.x);close(c.border_box.x,g.content_box.x+53.0);
        if alignment=="first baseline" {close(first(a),first(c));}else {close(last(a),last(c));}
        assert!(b.border_box.y>=g.content_box.y+103.0-0.02);
    }
}
#[test]
fn native_row_lanes_auto_margins_keep_priority_over_sharing() {
    for margin in ["margin-top:auto","margin-top:auto;margin-bottom:auto"] {
        for alignment in ["first baseline","last baseline"] {
            let markup=format!("<div id=a style='width:50px;min-height:30px;grid-row:1;{margin}'>small</div><div id=b style='width:50px;min-height:50px;grid-row:1'>large</div>");
            let (dom,root)=row_lanes_document(&markup,"100px",&format!("align-items:{alignment}"));
            let a=find(&root,&dom,"a");let g=find(&root,&dom,"g");close(a.border_box.h,30.0);
            let lead=if margin.contains("margin-bottom") {35.0}else {70.0};close(a.border_box.y,g.content_box.y+lead);
        }
    }
}
#[test]
fn native_row_lanes_first_and_last_span_groups_use_the_matching_edge() {
    for alignment in ["first baseline","last baseline"] {
        let row=if alignment=="first baseline" {1}else {2};
        let markup=format!("<div id=a style='width:50px;min-height:40px;grid-row:1/span 2'>small</div><div id=b style='width:50px;min-height:50px;grid-row:{row}'>large</div>");
        let (dom,root)=row_lanes_document(&markup,"100px 100px",&format!("align-items:{alignment};gap:3px"));
        let a=find(&root,&dom,"a");let b=find(&root,&dom,"b");
        if alignment=="first baseline" {close(first(a),first(b));}else {close(last(a),last(b));}
        close(a.border_box.h,40.0);close(b.border_box.h,50.0);
    }
}
