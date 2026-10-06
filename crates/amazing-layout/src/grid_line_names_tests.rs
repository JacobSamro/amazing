//! Complete explicit named-line collection: decoded names and unique actual boundaries.
use super::*;
use crate::Flow;

fn doc(template: &str, declaration: &str, width: f32) -> LayoutBox {
    crate::layout_document("<body><div></div></body>",
        &format!("body{{display:grid;margin:0;justify-content:start;grid-template-rows:10px;grid-template-columns:{template};grid-auto-columns:10px}}div{{height:10px;grid-column:{declaration}}}"),
        width,&FontSource::fallback())
}
fn child(root: &LayoutBox) -> &LayoutBox {
    let Flow::Grid(items)=&root.flow else {panic!("real Grid flow missing")}; &items[0]
}
fn geometry(root: &LayoutBox, x: f32, width: f32) {
    let r=child(root).border_box;assert!((r.x-x).abs()<0.01 && (r.w-width).abs()<0.01,"actual {r:?}; expected x={x},width={width}");
}
fn names(template: &str) -> HashMap<String,Vec<i32>> {
    collect_line_names(template,&RepeatCtx{font_px:16.0,content_w:100.0,zoom:1.0,repeat_available:Some(100.0),gap:0.0},true)
}
#[test]
fn repeated_boundaries_count_distinct_lines_once_in_actual_placement() {
    for (value,x,w) in [("2 foo / 3 foo",20.0,20.0),("3 foo / span 1",40.0,10.0),("-3 foo / -2 foo",0.0,20.0)] {
        geometry(&doc("repeat(2, [foo] 20px [foo])",value,100.0),x,w);
    }
}
#[test]
fn duplicated_words_on_one_line_do_not_create_occurrences() {
    for block in ["[foo foo]","[foo foo foo]"] {
        geometry(&doc(&format!("{block} 20px [foo] 20px [foo]"),"2 foo / 3 foo",100.0),20.0,20.0);
    }
}
#[test]
fn outer_groups_union_with_the_repeated_fragment_edges() {
    let template="[foo] repeat(3, [foo] 20px [foo]) [foo]";
    assert_eq!(names(template)["foo"],vec![1,2,3,4]);
    for (value,x) in [("2 foo / 3 foo",20.0),("3 foo / 4 foo",40.0),("-3 foo / -2 foo",20.0)] {
        geometry(&doc(template,value,100.0),x,20.0);
    }
}
#[test]
fn escaped_explicit_names_share_the_decoded_placement_namespace() {
    for spelling in [r"\66 oo",r"f\6f o",r"\000066oo"] {
        geometry(&doc(&format!("[{spelling}] 30px [foo-end]"),"foo / foo-end",100.0),0.0,30.0);
    }
}
#[test]
fn escaped_and_literal_spellings_on_the_same_line_are_one_name() {
    geometry(&doc(r"[\66 oo foo] 20px [foo] 30px [foo]","2 foo / 3 foo",100.0),20.0,30.0);
    assert_eq!(names(r"[\66 oo foo] 20px [foo]")["foo"],vec![1,2]);
}
#[test]
fn nonascii_names_decode_without_case_folding() {
    for spelling in [r"\e9 cran",r"\0000e9cran","écran"] {
        geometry(&doc(&format!("[{spelling}] 40px [end]"),"écran / end",100.0),0.0,40.0);
    }
    geometry(&doc(r"[foo] 20px [\46 oo] 30px [end]","Foo / end",100.0),20.0,30.0);
}
#[test]
fn decoded_identifier_codepoints_are_not_resplit_as_whitespace_or_syntax() {
    for (spelling,key) in [(r"a\20 b","a b"),(r"bracket\5d","bracket]"),(r"\37","7"),("a\u{a0}b","a\u{a0}b")] {
        let n=names(&format!("[{spelling}] 20px [end]"));assert_eq!(n[key],vec![1]);assert_eq!(n["end"],vec![2]);
    }
}
#[test]
fn independent_names_on_one_line_keep_their_own_occurrence_sequences() {
    let n=names("[a a b] 20px [a c c] 30px [b c]");
    assert_eq!(n["a"],vec![1,2]);assert_eq!(n["b"],vec![1,3]);assert_eq!(n["c"],vec![2,3]);
    geometry(&doc("[a a b] 20px [a c c] 30px [b c]","2 a / 2 b",100.0),20.0,30.0);
}
#[test]
fn empty_groups_do_not_add_tracks_or_names() {
    // One optional name group per track boundary (Grid 2 section 7.2.1).
    let n=names("[] [foo] 20px [] [foo] 30px [] [foo]");assert_eq!(n.len(),1);assert_eq!(n["foo"],vec![1,2,3]);
    let legal=names("[] 20px [foo] 30px []");assert_eq!(legal.len(),1);assert_eq!(legal["foo"],vec![2]);
    geometry(&doc("[] 20px [foo] 30px []","foo / 3",100.0),20.0,30.0);
}
#[test]
fn original_consecutive_block_fixture_is_retained_as_invalid_css() {
    let original="[] [foo] 20px [] [foo] 30px [] [foo]";
    let r=doc(original,"2 foo / 3 foo",100.0);
    assert_eq!(r.style.get("grid-template-columns"),Some("none"));
    assert!(r.grid_tracks.as_ref().unwrap().col_line_names.is_empty());
    // With no explicit named lines, occurrences 2 and 3 refer to implicit
    // lines 3 and 4, using the declared 10px implicit track pattern.
    geometry(&r,20.0,10.0);
}
#[test]
fn automatic_repetition_uses_unique_boundary_names_in_layout() {
    for width in [40.0,60.0,80.0] {
        let r=doc("repeat(auto-fill, [foo] 20px [foo])","2 foo / 3 foo",width);geometry(&r,20.0,20.0);
        let count=(width/20.0) as usize;assert_eq!(r.grid_tracks.as_ref().unwrap().col_line_names["foo"],(1..=count as i32+1).collect::<Vec<_>>());
    }
}
#[test]
fn auto_fit_retains_explicit_line_numbers_while_collapsing_unoccupied_tracks() {
    let r=doc("repeat(auto-fit, [foo] 20px [foo])","3 foo / span 1",60.0);geometry(&r,0.0,20.0);
    let tracks=r.grid_tracks.as_ref().unwrap();assert_eq!(tracks.col_sizes,vec![0.0,0.0,20.0]);assert_eq!(tracks.col_line_names["foo"],vec![1,2,3,4]);
}
#[test]
fn subgrid_inheritance_preserves_the_collected_line_namespace() {
    let r=crate::layout_document("<body><section><div></div></section></body>",
        "body{display:grid;margin:0;grid-template-columns:repeat(3,[foo] 20px [foo]);grid-template-rows:10px}section{display:grid;grid-column:1/span 3;grid-template-columns:subgrid}div{grid-column:-3 foo/-2 foo;height:5px}",
        60.0,&FontSource::fallback());
    let Flow::Grid(items)=&r.flow else{panic!("parent Grid missing")};let Flow::Grid(children)=&items[0].flow else{panic!("subgrid missing")};
    assert!((children[0].border_box.x-20.0).abs()<0.01);assert!((children[0].border_box.w-20.0).abs()<0.01);
}
#[test]
fn original_nested_repeat_rejection_and_explicit_line_bound_are_preserved() {
    assert!(names("repeat(2, repeat(2,[foo]20px[foo]))").is_empty());
    let n=names("repeat(2000,[foo] 1px [foo])");let positions=&n["foo"];assert_eq!(positions.len(),MAX_GRID_TRACKS+1);assert_eq!(positions.first(),Some(&1));assert_eq!(positions.last(),Some(&(MAX_GRID_TRACKS as i32+1)));
}
