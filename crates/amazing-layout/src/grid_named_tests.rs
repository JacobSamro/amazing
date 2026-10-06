//! Whole named-area placement, signed lines, automatic placement and sizing controls.
use super::*;
use crate::Flow;

fn root(template: &str, items: &str, declarations: &str, width: f32) -> LayoutBox {
    crate::layout_document(&format!("<body>{items}</body>"),
        &format!("body{{display:grid;margin:0;justify-content:start;align-content:start;{template}}}{declarations}"),
        width, &FontSource::fallback())
}
fn children(root: &LayoutBox) -> &[LayoutBox] {
    let Flow::Grid(items) = &root.flow else { panic!("real Grid flow missing") }; items
}
fn rect(item: &LayoutBox, x: f32, y: f32, w: f32, h: f32) {
    let r = item.border_box;
    for (actual, expected) in [(r.x,x),(r.y,y),(r.w,w),(r.h,h)] {
        assert!((actual-expected).abs()<0.01,"actual {r:?}; expected {x},{y},{w},{h}");
    }
}
#[test]
fn named_rectangles_span_both_axes_in_real_document() {
    for gap in [0,5,10] {
        let r=root(&format!("grid-template-areas:'head head' 'main side';grid-template-columns:40px 60px;grid-template-rows:20px 30px;gap:{gap}px"),
            "<div id=h></div><div id=m></div><div id=s></div>","#h{grid-area:head}#m{grid-area:main}#s{grid-area:side}",140.0);
        let a=children(&r); rect(&a[0],0.0,0.0,(100+gap) as f32,20.0);
        rect(&a[1],0.0,(20+gap) as f32,40.0,30.0); rect(&a[2],(40+gap) as f32,(20+gap) as f32,60.0,30.0);
    }
}
#[test]
fn template_extends_explicit_grid_using_auto_track_sequence() {
    let r=root("grid-template-areas:'a b c';grid-template-columns:30px;grid-auto-columns:20px 40px;grid-auto-rows:15px",
        "<div id=a></div><div id=b></div><div id=c></div>","#a{grid-area:a}#b{grid-area:b}#c{grid-area:c}",100.0);
    for (i,(x,w)) in [(0.0,30.0),(30.0,20.0),(50.0,40.0)].into_iter().enumerate() { rect(&children(&r)[i],x,0.0,w,15.0); }
}
#[test]
fn null_cells_are_available_to_auto_placement_without_reserving_areas() {
    for flow in ["row","column","row dense","column dense"] {
        let r=root(&format!("grid-template-areas:'a .' '. b';grid-template-columns:30px 50px;grid-template-rows:20px 40px;grid-auto-flow:{flow}"),
            "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>","#a{grid-area:a}#b{grid-area:b}",100.0);
        let a=children(&r); rect(&a[0],0.0,0.0,30.0,20.0); rect(&a[1],30.0,20.0,50.0,40.0);
        let column=flow.starts_with("column");
        rect(&a[2],if column{0.0}else{30.0},if column{20.0}else{0.0},if column{30.0}else{50.0},if column{40.0}else{20.0});
        rect(&a[3],if column{30.0}else{0.0},if column{0.0}else{20.0},if column{50.0}else{30.0},if column{20.0}else{40.0});
    }
}
#[test]
fn explicit_named_lines_create_implicit_area_and_match_case() {
    for name in ["panel","Panel","écran"] {
        let r=root(&format!("grid-template-columns:20px [{name}-start] 50px [{name}-end] 30px;grid-template-rows:[{name}-start] 15px [{name}-end]"),
            "<div></div>",&format!("div{{grid-area:{name}}}"),100.0);
        rect(&children(&r)[0],20.0,0.0,50.0,15.0);
    }
}
#[test]
fn named_occurrence_union_counts_each_line_once() {
    let r=root("grid-template-areas:'a a';grid-template-columns:[a-start] 20px [a-start] 30px [a-end];grid-template-rows:15px",
        "<div id=a></div><div id=b></div>","#a{grid-column:a}#b{grid-column:2 a-start / a-end}",100.0);
    rect(&children(&r)[0],0.0,0.0,50.0,15.0); rect(&children(&r)[1],20.0,15.0,30.0,0.0);
}
#[test]
fn later_longhands_override_stale_grid_area_shorthand() {
    let r=root("grid-template-areas:'a b';grid-template-columns:20px 40px;grid-template-rows:15px",
        "<div></div>","div{grid-area:a;grid-column-start:2;grid-column-end:3}",100.0);
    rect(&children(&r)[0],20.0,0.0,40.0,15.0);
}
#[test]
fn explicit_auto_override_does_not_resurrect_area_start() {
    let mut s=ComputedStyle::default(); s.set("grid-area","a"); s.set("grid-column-start","auto"); s.set("grid-column-end","auto");
    let names=HashMap::from([("a-start".to_owned(),vec![2]),("a-end".to_owned(),vec![3])]);
    assert_eq!(named::axis(&s,"column",3,&names),(None,1));
}
#[test]
fn negative_named_occurrences_generate_leading_pattern_tracks() {
    for (value,expected) in [("-1 absent / 1",20.0),("-2 absent / 1",30.0),("-3 absent / 1",50.0)] {
        let r=root("grid-template-columns:50px;grid-template-rows:10px;grid-auto-columns:10px 20px",
            "<div id=a></div><div id=b></div>",&format!("#a{{grid-column:{value};grid-row:1}}#b{{grid-column:1;grid-row:1}}"),150.0);
        rect(&children(&r)[0],0.0,0.0,expected,10.0); rect(&children(&r)[1],expected,0.0,50.0,10.0);
    }
}
#[test]
fn named_span_search_ignores_opposite_implicit_side() {
    let r=root("grid-template-columns:50px;grid-template-rows:10px;grid-auto-columns:10px 20px",
        "<div id=a></div><div id=b></div>","#a{grid-column:span foo / 4;grid-row:1}#b{grid-column:1;grid-row:1}",150.0);
    rect(&children(&r)[0],0.0,0.0,100.0,10.0); rect(&children(&r)[1],20.0,0.0,50.0,10.0);
}
#[test]
fn named_spans_find_matching_explicit_lines_excluding_anchor() {
    for (value,x,w) in [("foo / span 2 foo",0.0,60.0),("span 2 foo / -1 foo",10.0,90.0),("2 foo / span foo",10.0,50.0)] {
        let r=root("grid-template-columns:[foo] 10px [foo] 20px 30px [foo] 40px [foo];grid-template-rows:10px",
            "<div></div>",&format!("div{{grid-column:{value}}}"),100.0);
        rect(&children(&r)[0],x,0.0,w,10.0);
    }
}
#[test]
fn spans_without_anchor_follow_conflict_rules() {
    for (value,span) in [("span 3 foo / auto",1),("auto / span 3 foo",1),("span 2 / span 3",2),("span 2 foo / span 3",1)] {
        let mut s=ComputedStyle::default(); s.set("grid-column",value);
        assert_eq!(named::axis(&s,"column",5,&HashMap::new()),(None,span));
    }
}
#[test]
fn signed_numeric_lines_keep_explicit_end_after_prepend() {
    let r=root("grid-template-columns:50px;grid-template-rows:10px;grid-auto-columns:20px",
        "<div id=a></div><div id=b></div>","#a{grid-column:-3 / -2;grid-row:1}#b{grid-column:-2 / -1;grid-row:1}",100.0);
    rect(&children(&r)[0],0.0,0.0,20.0,10.0); rect(&children(&r)[1],20.0,0.0,50.0,10.0);
}
#[test]
fn reversed_and_equal_area_edges_follow_conflict_rules() {
    for (value,x,w) in [("a-end / a-start",0.0,50.0),("a-end / a-end",50.0,20.0)] {
        let r=root("grid-template-areas:'a a';grid-template-columns:20px 30px;grid-template-rows:10px;grid-auto-columns:20px",
            "<div></div>",&format!("div{{grid-column:{value}}}"),100.0);
        rect(&children(&r)[0],x,0.0,w,10.0);
    }
}
#[test]
fn malformed_nonrectangular_rows_do_not_create_false_names() {
    for value in ["'a .' '. a'","'a a' 'a b'","'a b' 'a'"] {
        let mut s=ComputedStyle::default(); s.set("grid-template-areas",value);
        let spec=GridSpec::of(&s,None,16.0,100.0,None);
        assert!(!spec.col_line_names.contains_key("a-start"));
    }
}
#[test]
fn decoded_canonical_names_preserve_nonascii_whitespace_and_quoted_none() {
    for name in ["none","a\u{a0}b","é","7"] {
        let mut s=ComputedStyle::default(); s.set("grid-template-areas",&format!("\"{name}\""));
        let spec=GridSpec::of(&s,None,16.0,100.0,None);
        assert_eq!(spec.col_line_names[&format!("{name}-start")],vec![1]);
    }
}
#[test]
fn named_area_intrinsic_width_and_height_are_real_contributions() {
    let r=root("grid-template-areas:'a b';grid-template-columns:auto 20px;grid-template-rows:auto",
        "<div id=a></div><div id=b></div>","#a{grid-area:a;width:70px;height:35px}#b{grid-area:b;height:15px}",90.0);
    rect(&children(&r)[0],0.0,0.0,70.0,35.0); rect(&children(&r)[1],70.0,0.0,20.0,15.0);
}
#[test]
fn area_names_are_inherited_and_remapped_into_real_subgrid() {
    let r=root("grid-template-areas:'left main main';grid-template-columns:20px 30px 40px;grid-template-rows:15px",
        "<section><div id=a></div></section>","section{display:grid;grid-column:main;grid-template-columns:subgrid}#a{grid-column:main-start/main-end;height:5px}",100.0);
    let a=children(&children(&r)[0]); rect(&a[0],20.0,0.0,70.0,5.0);
}
#[test]
fn out_of_flow_named_areas_use_existing_geometry_without_adding_tracks() {
    let r=root("position:relative;grid-template-areas:'left main';grid-template-columns:20px 40px;grid-template-rows:15px",
        "<div></div>","div{position:absolute;grid-area:main;width:10px;height:5px}",100.0);
    assert_eq!(r.positioned.len(),1); rect(&r.positioned[0],20.0,0.0,10.0,5.0);
    let tracks=r.grid_tracks.as_ref().expect("resolved named geometry"); assert_eq!(tracks.col_sizes.len(),2);
    let mut s=ComputedStyle::default();s.set("grid-column","absent / span 2");
    assert_eq!(tracks.area(&s).0,None); assert_eq!(tracks.col_sizes.len(),2);
}
#[test]
fn escaped_line_identifiers_match_decoded_area_names() {
    let names=HashMap::from([("7-start".to_owned(),vec![2]),("7-end".to_owned(),vec![4])]);
    let mut s=ComputedStyle::default();s.set("grid-column",r"\37");
    assert_eq!(named::axis(&s,"column",4,&names),(Some(2),2));
}

#[test]
fn unbounded_numeric_and_named_occurrences_reject_before_allocation() {
    for value in ["named 2147483647", "-2147483648 named", "2147483647", "-2147483648"] {
        let r=root("grid-template-columns:[named] 100px;grid-template-rows:20px",
            "<div></div>",&format!("div{{grid-column:{value};height:20px}}"),200.0);
        rect(&children(&r)[0],0.0,0.0,100.0,20.0);
    }
}
