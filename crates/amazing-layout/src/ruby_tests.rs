//! Ruby pairing and geometry contracts independent of raster fonts.
use crate::*;
fn layout(html: &str, css: &str) -> LayoutBox {
    let dom = amazing_html::parse(&format!("<body>{html}"));
    layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        &format!("body{{margin:0;font:20px/20px serif}}ruby{{ruby-align:center}}{css}"),
        500.0,
        Some(500.0),
        &amazing_text::FallbackTextSystem::new(),
        &NoReplacedElements,
    )
}
fn columns(b: &LayoutBox) -> &[RubyColumn] {
    let Flow::Inlines(items) = &b.flow else {
        panic!("inline flow")
    };
    items
        .iter()
        .find_map(|i| {
            if let InlineNode::Ruby { columns, .. } = i {
                Some(columns.as_slice())
            } else {
                None
            }
        })
        .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.05, "{a} != {b}");
}
fn end(b: &LayoutBox) -> f32 {
    b.lines[0]
        .words
        .iter()
        .map(|w| w.x + w.width)
        .fold(0.0, f32::max)
}

#[test]
fn ruby_segments_restart_pairing_after_annotations() {
    let b = layout("<ruby>A<rt>one</rt>B<rt>two</rt></ruby>", "");
    let c = columns(&b);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].base.text, "A");
    assert_eq!(c[1].base.text, "B");
    assert_eq!(c[0].annotation.as_ref().unwrap().text, "one");
    assert_eq!(c[1].annotation.as_ref().unwrap().text, "two");
}
#[test]
fn multiple_levels_keep_independent_pairing_and_spans() {
    let b = layout(
        "<ruby><rb>A</rb><rb>B</rb><rt>x</rt><rt>y</rt><rtc>shared</rtc></ruby>",
        "",
    );
    let c = columns(&b);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].annotation.as_ref().unwrap().text, "x");
    assert_eq!(c[1].annotation.as_ref().unwrap().text, "y");
    let note = &c[0].additional_annotations[0];
    assert_eq!(note.span, 2);
    assert_eq!(note.level, 1);
    assert_eq!(note.text.text, "shared");
}
#[test]
fn intra_level_spaces_pair_with_empty_space_on_missing_levels() {
    let b = layout("<ruby><rb>A</rb> <rb>B</rb><rt>x</rt><rt>y</rt></ruby>", "");
    let c = columns(&b);
    assert_eq!(c.len(), 3);
    assert!(c[1].intra_level_whitespace);
    assert_eq!(c[1].base.text, " ");
    assert!(c[1].annotation.is_none());
    assert_eq!(c[2].annotation.as_ref().unwrap().text, "y");
    let b = layout("<ruby><rb>A</rb><rb>B</rb><rt>x</rt> <rt>y</rt></ruby>", "");
    let c = columns(&b);
    assert_eq!(c.len(), 3);
    assert!(c[1].base.text.is_empty());
    assert_eq!(c[1].annotation.as_ref().unwrap().text, " ");
}
#[test]
fn nested_ruby_keeps_annotations_out_of_outer_base_text() {
    let b = layout(
        "<ruby><ruby>A<rt>x</rt>B<rt>y</rt></ruby><rt>outer</rt></ruby>",
        "ruby{ruby-position:over}",
    );
    let c = columns(&b);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].base.raw_text, "AB");
    assert_eq!(c[0].base.nested[0].columns.len(), 2);
    let reference = layout(
        "<ruby><rb>A</rb><rb>B</rb><rt>x</rt><rt>y</rt><rtc>outer</rtc></ruby>",
        "ruby{ruby-position:over}",
    );
    near(end(&b), end(&reference));
    near(b.lines[0].height, reference.lines[0].height);
}
#[test]
fn orphan_ruby_parts_form_one_anonymous_container() {
    let b = layout("<rb>A</rb><rb>B</rb><rt>x</rt><rt>y</rt>", "");
    assert_eq!(columns(&b).len(), 2);
    let reference = layout("<ruby><rb>A</rb><rb>B</rb><rt>x</rt><rt>y</rt></ruby>", "");
    near(end(&b), end(&reference));
    near(b.lines[0].height, reference.lines[0].height);
}
#[test]
fn annotation_under_and_alternate_have_distinct_sides() {
    let over = layout("<ruby>A<rt>note</rt></ruby>", "ruby{ruby-position:over}");
    let under = layout("<ruby>A<rt>note</rt></ruby>", "ruby{ruby-position:under}");
    assert!(
        over.lines[0]
            .glyphs
            .iter()
            .filter(|g| g.font_size < 20.0)
            .all(|g| g.y < 0.0)
    );
    assert!(
        under.lines[0]
            .glyphs
            .iter()
            .filter(|g| g.font_size < 20.0)
            .all(|g| g.y > 0.0)
    );
    near(over.lines[0].height, under.lines[0].height);
    let alternate = layout("<ruby>A<rt>x</rt><rtc>y</rtc></ruby>", "");
    let notes: Vec<_> = alternate.lines[0]
        .glyphs
        .iter()
        .filter(|g| g.font_size < 20.0)
        .collect();
    assert!(notes.iter().any(|g| g.y < 0.0));
    assert!(notes.iter().any(|g| g.y > 0.0));
}
#[test]
fn identical_raw_annotation_auto_hides_but_visibility_hidden_keeps_measure() {
    let same = layout("<ruby>A<rt>A</rt></ruby>", "");
    assert!(columns(&same)[0].annotation_hidden);
    let note = layout("<ruby>A<rt>long note</rt></ruby>", "");
    let hidden = layout("<ruby>A<rt>long note</rt></ruby>", "rt{visibility:hidden}");
    near(end(&note), end(&hidden));
    assert_eq!(hidden.lines[0].glyphs.len(), 1);
    let collapsed = layout(
        "<ruby>A<rt>long note</rt></ruby>",
        "rt{visibility:collapse}",
    );
    assert!(end(&collapsed) < end(&hidden));
}
#[test]
fn ruby_alignment_stretches_word_spaces_not_latin_letters() {
    let start = layout(
        "<ruby><rb>A B C</rb><rt>xxxxxxxxxxxxxxxx</rt></ruby>",
        "rb{ruby-align:start}",
    );
    let between = layout(
        "<ruby><rb>A B C</rb><rt>xxxxxxxxxxxxxxxx</rt></ruby>",
        "rb{ruby-align:space-between}",
    );
    let last = |b: &LayoutBox| {
        b.lines[0]
            .glyphs
            .iter()
            .filter(|g| g.font_size == 20.0)
            .map(|g| g.x)
            .fold(0.0, f32::max)
    };
    assert!(last(&between) > last(&start));
    near(end(&between), end(&start));
}
#[test]
fn ruby_max_content_width_includes_annotation_and_contextual_overhang() {
    let b = layout(
        "<ruby>A<rt>xxxxxxxxxx</rt></ruby>",
        "body{width:max-content}",
    );
    near(b.content_box.w, end(&b));
    assert!(b.content_box.w > 20.0);
    let b = layout(
        "A　<ruby>A<rt>xxxxxxxxxx</rt></ruby>　A",
        "body{width:max-content}ruby{ruby-overhang:spaces}",
    );
    let r = layout(
        "A<ruby>　A　<rt>xxxxxxxxxx</rt></ruby>A",
        "body{width:max-content}ruby{ruby-overhang:spaces}",
    );
    near(b.content_box.w, r.content_box.w);
}
#[test]
fn over_and_under_vertical_ruby_use_right_and_left_sides_in_both_flows() {
    for mode in ["vertical-rl", "vertical-lr"] {
        for (position, positive) in [("over", true), ("under", false)] {
            let b = layout(
                "<ruby>A<rt>note</rt></ruby>",
                &format!("body{{writing-mode:{mode}}}ruby{{ruby-position:{position}}}"),
            );
            let base = b.lines[0]
                .glyphs
                .iter()
                .find(|g| g.font_size == 20.0)
                .unwrap()
                .x;
            assert!(
                b.lines[0]
                    .glyphs
                    .iter()
                    .filter(|g| g.font_size < 20.0)
                    .all(|g| (g.x > base) == positive)
            );
        }
    }
}

#[test]
fn empty_annotation_container_keeps_its_level_and_font_metrics() {
    let a = layout(
        "a<ruby>b<rtc></rtc><rt>c</rt></ruby>d",
        "body{line-height:5}",
    );
    let b = layout(
        "a<ruby>b<rtc><rt></rt></rtc><rt>c</rt></ruby>d",
        "body{line-height:5}",
    );
    near(a.lines[0].height, b.lines[0].height);
    for (a, b) in a.lines[0].glyphs.iter().zip(&b.lines[0].glyphs) {
        near(a.x, b.x);
        near(a.y, b.y);
    }
}
#[test]
fn first_line_ruby_position_is_applied_before_placement() {
    let a = layout(
        "<div>foo <ruby>base<rt>note</rt></ruby><br>bar <ruby>base<rt>note</rt></ruby></div>",
        "div::first-line{ruby-position:under}",
    );
    let Flow::Blocks(c) = &a.flow else {
        panic!("blocks")
    };
    assert!(
        c[0].lines[0]
            .glyphs
            .iter()
            .filter(|g| g.font_size < 20.0)
            .all(|g| g.y > 0.0)
    );
    assert!(
        c[0].lines[1]
            .glyphs
            .iter()
            .filter(|g| g.font_size < 20.0)
            .all(|g| g.y < 0.0)
    );
}
#[test]
fn surplus_line_height_already_accommodates_annotations() {
    let a = layout("<ruby>A<rt>note</rt></ruby>", "body{line-height:60px}");
    near(a.lines[0].height, 60.0);
}
#[test]
fn float_boundary_does_not_supply_collapsed_space_for_overhang() {
    let a = layout(
        "<div style='float:left;width:50px;height:50px'></div>\n<ruby>base<rt>annotation</rt></ruby>",
        "",
    );
    let mut glyphs = Vec::new();
    fn visit<'a>(b: &'a LayoutBox, out: &mut Vec<&'a GlyphFrag>) {
        out.extend(b.lines.iter().flat_map(|l| &l.glyphs));
        if let Flow::Blocks(c) = &b.flow {
            for c in c {
                visit(c, out)
            }
        }
    }
    visit(&a, &mut glyphs);
    assert!(glyphs.iter().all(|g| g.x >= 49.99));
}

#[test]
fn nested_ruby_distributes_outer_spanning_width_into_inner_columns() {
    let a = layout(
        "<ruby><ruby>東<rt>とう</rt>南<rt>なん</rt></ruby><rt>Southeast long</rt></ruby>",
        "ruby{ruby-position:over;ruby-align:space-around}",
    );
    let b = layout(
        "<ruby><rb>東</rb><rb>南</rb><rt>とう</rt><rt>なん</rt><rtc>Southeast long</rtc></ruby>",
        "ruby{ruby-position:over;ruby-align:space-around}",
    );
    let coordinates = |b: &LayoutBox| {
        let mut v: Vec<_> = b.lines[0]
            .glyphs
            .iter()
            .map(|g| (g.glyph, g.x, g.y))
            .collect();
        v.sort_by_key(|g| g.0.0);
        v
    };
    let a = coordinates(&a);
    let b = coordinates(&b);
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(a.0, b.0);
        near(a.1, b.1);
        near(a.2, b.2);
    }
}
#[test]
fn specified_tab_stops_affect_intrinsic_width_and_visible_glyph_positions() {
    let b = layout(
        "X\tY",
        "body{white-space:pre;tab-size:100px;width:max-content}",
    );
    near(b.lines[0].glyphs.last().unwrap().x, 100.0);
    let b = layout(
        "X\t<ruby>A<rt>abcdefgh</rt></ruby>\tY",
        "body{white-space:pre;tab-size:100px;width:max-content}ruby{ruby-overhang:spaces}",
    );
    assert!(b.content_box.w >= 111.0);
}

#[test]
fn generated_ruby_parts_participate_in_pairing_without_whitespace_reclassification() {
    let b = layout(
        "<ruby><rbc><rb>base</rb></rbc><rtc><rt>text</rt></rtc></ruby>",
        "rbc::before{display:ruby-base;content:'before'}rbc::after{display:ruby-base;content:' '}rtc::before{display:ruby-text;content:' '}rtc::after{display:ruby-text;content:'after'}",
    );
    let c = columns(&b);
    assert_eq!(c.len(), 3);
    assert_eq!(c[0].base.text, "before");
    assert_eq!(c[1].base.text, "base");
    assert_eq!(c[2].base.text, " ");
    assert_eq!(c[0].annotation.as_ref().unwrap().text, " ");
    assert_eq!(c[2].annotation.as_ref().unwrap().text, "after");
}
#[test]
fn whitespace_collapses_across_base_boundaries_but_keeps_pairing() {
    let b = layout(
        "<ruby><rb>b</rb><rb> </rb><rb> c</rb><rt>x</rt><rt></rt><rt>z</rt></ruby>",
        "",
    );
    let c = columns(&b);
    assert_eq!(c.len(), 3);
    assert_eq!(c[1].base.text, " ");
    assert_eq!(c[2].base.text, "c");
    assert_eq!(c[2].annotation.as_ref().unwrap().text, "z");
}
