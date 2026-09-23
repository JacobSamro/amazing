use amazing_dom::Dom;
use amazing_layout::font::FontSource;
use amazing_layout::{Flow, LayoutBox, layout_document};

fn find<'a>(layout: &'a LayoutBox, dom: &Dom, tag: &str) -> Option<&'a LayoutBox> {
    if layout.node.and_then(|node| dom.name(node)) == Some(tag) {
        return Some(layout);
    }
    match &layout.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            children.iter().find_map(|child| find(child, dom, tag))
        },
        Flow::Inlines(_) | Flow::Empty => None,
    }
}

fn computed_for(html: &str, tag: &str) -> amazing_css::ComputedStyle {
    let dom = amazing_html::parse(html);
    let node = (0..dom.node_count())
        .map(|index| amazing_dom::NodeIdx(index as u32))
        .find(|node| dom.name(*node) == Some(tag))
        .expect("styled element");
    amazing_css::compute(&dom, &[])
        .get(node)
        .expect("computed style")
        .clone()
}

#[test]
fn flex_redistributes_space_after_an_item_hits_its_maximum() {
    let html = r#"
        <body>
          <main style="display:flex;width:300px">
            <a style="flex:1 1 100px;max-width:110px;height:10px"></a>
            <b style="flex:3 1 100px;height:10px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first flex item");
    let second = find(&layout, &dom, "b").expect("second flex item");

    assert_eq!(first.content_box.w, 110.0);
    assert_eq!(second.content_box.w, 190.0);
}

#[test]
fn flex_auto_minimum_preserves_unbreakable_content() {
    let html = r#"
        <body>
          <main style="display:flex;width:200px">
            <a style="flex:0 1 180px">AAAAAAAAAAAAAAAAAAAA</a>
            <b style="flex:0 1 50px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first flex item");
    let second = find(&layout, &dom, "b").expect("second flex item");
    let content_minimum = fonts.measure("AAAAAAAAAAAAAAAAAAAA", 16.0);

    assert_eq!(first.content_box.w, content_minimum);
    assert!((second.content_box.w - (200.0 - content_minimum)).abs() < 0.001);
}

#[test]
fn flex_auto_minimum_depends_on_scrollable_overflow() {
    let html = r#"
        <body>
          <main style="display:flex;width:30px">
            <a style="border:2px dotted purple;overflow-y:visible"><i style="display:block;width:80px;height:40px"></i></a>
          </main>
          <section style="display:flex;width:30px">
            <b style="border:2px dotted purple;overflow-y:hidden"><i style="display:block;width:80px;height:40px"></i></b>
          </section>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let visible = find(&layout, &dom, "a").expect("visible-overflow item");
    let hidden = find(&layout, &dom, "b").expect("hidden-overflow item");

    assert_eq!(visible.content_box.w, 80.0);
    assert_eq!(hidden.content_box.w, 26.0);
}

#[test]
fn flex_line_cross_size_counts_item_border_once() {
    let html = r#"
        <body>
          <main style="display:flex">
            <a style="border:2px solid black"><i style="display:block;width:80px;height:40px"></i></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let container = find(&layout, &dom, "main").expect("flex container");
    let item = find(&layout, &dom, "a").expect("flex item");

    assert_eq!(item.border_box.h, 44.0);
    assert_eq!(container.content_box.h, 44.0);
}

#[test]
fn flex_min_content_minimum_uses_the_child_contribution() {
    let html = r#"
        <body>
          <main style="display:flex;width:0px;height:100px">
            <a style="min-width:min-content;flex-basis:0;background:green;writing-mode:vertical-rl">
              <i style="display:block;width:100px"></i>
            </a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let item = find(&layout, &dom, "a").expect("flex item");

    assert_eq!(item.content_box.w, 100.0);
}

#[test]
fn flex_min_content_maximum_clamps_the_flex_basis() {
    let html = r#"
        <body>
          <main style="display:flex;width:200px;height:100px">
            <a style="max-width:min-content;flex-basis:200px;background:green;writing-mode:vertical-rl">
              <i style="display:block;width:100px"></i>
            </a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let item = find(&layout, &dom, "a").expect("flex item");

    assert_eq!(item.content_box.w, 100.0);
}

#[test]
fn flex_infinite_grow_factor_dominates_finite_factors() {
    let html = r#"
        <body>
          <main style="display:flex;width:100px;height:10px">
            <a style="flex:calc(infinity) 0 0px"></a>
            <b style="flex:1 0 0px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("infinite-grow item");
    let second = find(&layout, &dom, "b").expect("finite-grow item");

    assert_eq!(first.content_box.w, 100.0);
    assert_eq!(second.content_box.w, 0.0);
}

#[test]
fn grid_none_template_creates_no_phantom_explicit_track() {
    let html = r#"
        <body>
          <main style="display:grid;grid-auto-columns:30px;grid-auto-rows:30px">
            <a style="grid-column:3;grid-row:1 / span 2"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let item = find(&layout, &dom, "a").expect("grid item");

    assert_eq!(item.border_box.x, grid.content_box.x + 60.0);
    assert_eq!(item.border_box.w, 30.0);
    assert_eq!(item.border_box.h, 60.0);
}

#[test]
fn grid_auto_fill_counts_the_gap_between_repetitions() {
    let html = r#"
        <body>
          <main style="display:grid;width:300px;grid-template-columns:repeat(auto-fill,50px 50px);grid-auto-rows:25px;column-gap:100px">
            <a></a><b></b><c></c><d></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let first = find(&layout, &dom, "a").expect("first item");
    let second = find(&layout, &dom, "b").expect("second item");
    let third = find(&layout, &dom, "c").expect("third item");

    assert_eq!(first.border_box.x, grid.content_box.x);
    assert_eq!(second.border_box.x, grid.content_box.x + 150.0);
    assert_eq!(third.border_box.x, grid.content_box.x);
    assert_eq!(third.border_box.y, first.border_box.y + 25.0);
}

#[test]
fn grid_legacy_gap_aliases_separate_tracks() {
    let html = r#"
        <body>
          <main style="display:grid;grid-template-columns:50px 50px;grid-template-rows:20px 20px;grid-column-gap:100px;grid-row-gap:30px">
            <a></a><b></b><c></c>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let second = find(&layout, &dom, "b").expect("second item");
    let third = find(&layout, &dom, "c").expect("third item");

    assert_eq!(second.border_box.x, first.border_box.x + 150.0);
    assert_eq!(third.border_box.y, first.border_box.y + 50.0);
}

#[test]
fn grid_row_auto_fill_uses_the_definite_block_size() {
    let html = r#"
        <body>
          <main style="display:grid;grid-auto-flow:column;grid-template-rows:repeat(auto-fill,50px 50px);grid-auto-columns:25px;row-gap:100px;width:800px;height:300px">
            <a></a><b></b><c></c><d></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 900.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let second = find(&layout, &dom, "b").expect("second item");
    let third = find(&layout, &dom, "c").expect("third item");

    assert_eq!(second.border_box.y, first.border_box.y + 150.0);
    assert_eq!(third.border_box.y, first.border_box.y);
    assert_eq!(third.border_box.x, first.border_box.x + 25.0);
}

#[test]
fn grid_template_shorthand_populates_both_track_axes() {
    let html = r#"
        <body>
          <main style="display:grid;grid-template:20px 30px / 40px 60px">
            <a></a><b></b><c></c><d></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let fourth = find(&layout, &dom, "d").expect("fourth item");

    assert_eq!(fourth.border_box.x, grid.content_box.x + 40.0);
    assert_eq!(fourth.border_box.y, grid.content_box.y + 20.0);
    assert_eq!(fourth.border_box.w, 60.0);
    assert_eq!(fourth.border_box.h, 30.0);
}

#[test]
fn grid_minmax_floors_a_smaller_maximum_by_its_minimum() {
    let html = r#"
        <body>
          <main style="display:grid;width:100px;grid-template:50px 50px / repeat(auto-fill,minmax(50px,25px))">
            <a></a><b></b><c></c><d></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let fourth = find(&layout, &dom, "d").expect("fourth item");

    assert_eq!(fourth.border_box.x, grid.content_box.x + 50.0);
    assert_eq!(fourth.border_box.y, grid.content_box.y + 50.0);
    assert_eq!(fourth.border_box.w, 50.0);
    assert_eq!(fourth.border_box.h, 50.0);
}

#[test]
fn grid_auto_margins_center_an_item_without_sizing_the_track() {
    let html = r#"
        <body>
          <main style="display:grid;width:300px;grid-template-columns:1fr 1fr 1fr">
            <a></a><b></b><c></c>
            <d style="width:50px;margin-left:auto;margin-right:auto"></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let centered = find(&layout, &dom, "d").expect("centered item");

    assert_eq!(centered.border_box.w, 50.0);
    assert_eq!(centered.border_box.x, grid.content_box.x + 25.0);
}

#[test]
fn grid_auto_repeat_subtracts_fixed_tracks_on_both_sides() {
    let html = r#"
        <body>
          <main style="display:grid;width:300px;grid-template-columns:10px 20px repeat(auto-fill,30px 40px) 50px 60px;grid-auto-rows:20px">
            <a></a><b></b><c></c><d></d><e></e><f></f><g></g><h></h>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let eighth = find(&layout, &dom, "h").expect("eighth item");

    assert_eq!(eighth.border_box.x, grid.content_box.x + 220.0);
    assert_eq!(eighth.border_box.w, 60.0);
}

#[test]
fn grid_indefinite_row_auto_repeat_uses_one_repetition() {
    let html = r#"
        <body>
          <main style="display:grid;grid-auto-flow:column;width:800px;grid-template-rows:repeat(auto-fill,30px);grid-auto-columns:20px">
            <a></a><b></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 900.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let second = find(&layout, &dom, "b").expect("second item");

    assert_eq!(second.border_box.x, first.border_box.x + 20.0);
    assert_eq!(second.border_box.y, first.border_box.y);
}

#[test]
fn grid_named_line_places_an_item_on_the_matching_track() {
    let html = r#"
        <body>
          <main style="display:grid;grid-template-columns:[start] 40px [middle] 60px [end];grid-template-rows:20px">
            <a style="grid-column-start:middle"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let item = find(&layout, &dom, "a").expect("grid item");

    assert_eq!(item.border_box.x, grid.content_box.x + 40.0);
    assert_eq!(item.border_box.w, 60.0);
}

#[test]
fn grid_named_line_occurrences_expand_through_auto_repeat() {
    let html = r#"
        <body>
          <main style="display:grid;width:94px;column-gap:3px;grid-template-columns:[u] repeat(auto-fill,[v] 10px [w] 10px [x] 10px [y]) [z];grid-auto-rows:5px">
            <a style="grid-column-start:v 2"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let item = find(&layout, &dom, "a").expect("grid item");

    assert_eq!(item.border_box.x, grid.content_box.x + 39.0);
    assert_eq!(item.border_box.w, 10.0);
}

#[test]
fn flex_flow_shorthand_enables_wrapping() {
    let html = r#"
        <body>
          <main style="display:flex;flex-flow:row wrap;width:100px">
            <a style="width:50px;height:50px"></a>
            <b style="width:50px;height:50px"></b>
            <c style="width:50px;height:50px"></c>
            <d style="width:50px;height:50px"></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let third = find(&layout, &dom, "c").expect("third item");

    assert_eq!(third.border_box.x, first.border_box.x);
    assert_eq!(third.border_box.y, first.border_box.y + 50.0);
}

#[test]
fn flex_align_content_stretch_grows_lines_and_items() {
    let html = r#"
        <body>
          <main style="display:flex;flex-flow:row wrap;align-content:stretch;width:100px;height:100px">
            <a style="width:50px"></a><b style="width:50px"></b>
            <c style="width:50px"></c><d style="width:50px"></d>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let third = find(&layout, &dom, "c").expect("third item");

    assert_eq!(first.border_box.h, 50.0);
    assert_eq!(third.border_box.h, 50.0);
    assert_eq!(third.border_box.y, first.border_box.y + 50.0);
}

#[test]
fn flex_rtl_gap_matches_logical_inline_end_margin() {
    let html = r#"
        <body>
          <main style="display:flex;direction:rtl;width:120px;gap:20px">
            <a style="width:50px;height:10px"></a><b style="width:50px;height:10px"></b>
          </main>
          <section style="display:flex;direction:rtl;width:120px">
            <c style="width:50px;height:10px;margin-inline-end:20px"></c><d style="width:50px;height:10px"></d>
          </section>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let gap_container = find(&layout, &dom, "main").expect("gap container");
    let margin_container = find(&layout, &dom, "section").expect("margin container");
    let first_gap = find(&layout, &dom, "a").expect("first gap item");
    let second_gap = find(&layout, &dom, "b").expect("second gap item");
    let first_margin = find(&layout, &dom, "c").expect("first margin item");
    let second_margin = find(&layout, &dom, "d").expect("second margin item");

    assert_eq!(first_gap.border_box.x - gap_container.content_box.x, first_margin.border_box.x - margin_container.content_box.x);
    assert_eq!(second_gap.border_box.x - gap_container.content_box.x, second_margin.border_box.x - margin_container.content_box.x);
    assert_eq!(first_gap.border_box.h, first_margin.border_box.h);
    assert_eq!(second_gap.border_box.h, second_margin.border_box.h);
}

#[test]
fn grid_flexible_tracks_restart_when_a_base_exceeds_its_fraction() {
    let html = r#"
        <body>
          <main style="display:grid;width:300px;grid-template-columns:minmax(180px,1fr) 1fr;grid-template-rows:20px">
            <a></a><b></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first track item");
    let second = find(&layout, &dom, "b").expect("second track item");

    assert_eq!(first.border_box.w, 180.0);
    assert_eq!(second.border_box.w, 120.0);
}

#[test]
fn flex_partial_shrink_uses_unscaled_factor_sum() {
    let html = r#"
        <body>
          <main style="display:flex;width:300px">
            <a style="flex:0 .25 200px;height:10px"></a>
            <b style="flex:0 .25 200px;height:10px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 500.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let second = find(&layout, &dom, "b").expect("second item");

    assert_eq!(first.border_box.w, 175.0);
    assert_eq!(second.border_box.w, 175.0);
}

#[test]
fn flex_column_redistributes_after_a_max_height_freeze() {
    let html = r#"
        <body>
          <main style="display:flex;flex-direction:column;width:20px;height:300px">
            <a style="flex:1 1 100px;max-height:110px"></a>
            <b style="flex:3 1 100px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first item");
    let second = find(&layout, &dom, "b").expect("second item");

    assert_eq!(first.border_box.h, 110.0);
    assert_eq!(second.border_box.h, 190.0);
}

#[test]
fn logical_margins_compete_in_cascade_order_and_preserve_auto() {
    let logical_last = computed_for(
        r#"<body><a style="direction:rtl;margin-left:20px;margin-inline-end:0"></a></body>"#,
        "a",
    );
    let physical_last = computed_for(
        r#"<body><a style="direction:rtl;margin-inline-end:0;margin-left:20px"></a></body>"#,
        "a",
    );
    let automatic = computed_for(
        r#"<body><a style="direction:rtl;margin-inline-end:auto"></a></body>"#,
        "a",
    );

    assert_eq!(logical_last.get("margin-left"), Some("0"));
    assert_eq!(physical_last.get("margin-left"), Some("20px"));
    assert_eq!(automatic.get("margin-left"), Some("auto"));
}

#[test]
fn logical_margin_mapping_uses_the_computed_inherited_direction() {
    let inherited = computed_for(
        r#"<body><main style="direction:rtl"><a style="direction:inherit;margin-inline-start:7px"></a></main></body>"#,
        "a",
    );

    assert_eq!(inherited.get("margin-right"), Some("7px"));
    assert_eq!(inherited.get("margin-left"), Some("0"));
}

#[test]
fn invalid_flex_flow_does_not_override_an_earlier_valid_declaration() {
    let duplicate = computed_for(
        r#"<body><a style="flex-flow:column wrap;flex-flow:row column"></a></body>"#,
        "a",
    );
    let unknown = computed_for(
        r#"<body><a style="flex-flow:column wrap;flex-flow:row banana"></a></body>"#,
        "a",
    );

    assert_eq!(duplicate.get("flex-direction"), Some("column"));
    assert_eq!(duplicate.get("flex-wrap"), Some("wrap"));
    assert_eq!(unknown.get("flex-direction"), Some("column"));
    assert_eq!(unknown.get("flex-wrap"), Some("wrap"));
}

#[test]
fn variable_dependent_flex_and_grid_shorthands_expand_after_substitution() {
    let flex = computed_for(
        r#"<body><a style="--flow:column wrap;flex-flow:var(--flow)"></a></body>"#,
        "a",
    );
    let grid = computed_for(
        r#"<body><a style="--template:20px / 40px;grid-template:var(--template)"></a></body>"#,
        "a",
    );
    let invalid_at_computed_time = computed_for(
        r#"<body><a style="--flow:row banana;flex-flow:column wrap;flex-flow:var(--flow)"></a></body>"#,
        "a",
    );

    assert_eq!(flex.get("flex-direction"), Some("column"));
    assert_eq!(flex.get("flex-wrap"), Some("wrap"));
    assert_eq!(grid.get("grid-template-rows"), Some("20px"));
    assert_eq!(grid.get("grid-template-columns"), Some("40px"));
    assert_eq!(invalid_at_computed_time.get("flex-direction"), Some("row"));
    assert_eq!(invalid_at_computed_time.get("flex-wrap"), Some("nowrap"));
}

#[test]
fn invalid_grid_template_does_not_override_an_earlier_valid_declaration() {
    let invalid_math = computed_for(
        r#"<body><a style="grid-template:20px / 40px;grid-template:calc(banana) / 1fr"></a></body>"#,
        "a",
    );
    let invalid_auto_repeat = computed_for(
        r#"<body><a style="grid-template:20px / 40px;grid-template:repeat(auto-fill,1fr) / 30px"></a></body>"#,
        "a",
    );

    assert_eq!(invalid_math.get("grid-template-rows"), Some("20px"));
    assert_eq!(invalid_math.get("grid-template-columns"), Some("40px"));
    assert_eq!(invalid_auto_repeat.get("grid-template-rows"), Some("20px"));
    assert_eq!(invalid_auto_repeat.get("grid-template-columns"), Some("40px"));
}

#[test]
fn repeated_grid_line_names_preserve_identifier_case() {
    let html = r#"
        <body>
          <main style="display:grid;grid-template-columns:repeat(2,[Foo] 10px);grid-template-rows:10px">
            <a style="grid-column:Foo 2;height:10px"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 200.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let item = find(&layout, &dom, "a").expect("case-sensitive named-line item");

    assert_eq!(item.border_box.x, grid.content_box.x + 10.0);
}

#[test]
fn grid_maximizes_tracks_up_to_finite_growth_limits() {
    let html = r#"
        <body>
          <main style="display:grid;width:100px;grid-template-columns:minmax(10px,50px) 20px;grid-template-rows:10px">
            <a></a><b></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 200.0, &fonts);
    let first = find(&layout, &dom, "a").expect("growable track item");
    let second = find(&layout, &dom, "b").expect("fixed track item");

    assert_eq!(first.border_box.w, 50.0);
    assert_eq!(second.border_box.w, 20.0);
}

#[test]
fn grid_inline_start_is_on_the_right_in_rtl() {
    let html = r#"
        <body>
          <main style="display:grid;direction:rtl;width:100px;height:100px;grid-template-columns:100px;grid-template-rows:100px">
            <a style="width:50px;height:50px;grid-row:1 / 2"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 200.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("grid container");
    let item = find(&layout, &dom, "a").expect("grid item");

    assert_eq!(item.border_box.x, grid.content_box.x + 50.0);
}

#[test]
fn grid_rejects_unbounded_line_occurrences_before_track_allocation() {
    let html = r#"
        <body>
          <main style="display:grid;width:100px;grid-template-columns:[named] 100px;grid-template-rows:20px">
            <a style="grid-column:named 2147483647;height:20px"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 200.0, &fonts);
    let grid = find(&layout, &dom, "main").expect("bounded grid container");
    let item = find(&layout, &dom, "a").expect("auto-placed rejected occurrence");

    assert!(item.border_box.x.is_finite());
    assert_eq!(item.border_box.x, grid.content_box.x);
}

#[test]
fn flex_auto_margin_absorption_advances_following_items() {
    let html = r#"
        <body>
          <main style="display:flex;width:300px">
            <a style="width:50px;height:10px;margin-left:auto"></a>
            <b style="width:50px;height:10px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let main = find(&layout, &dom, "main").expect("row flex container");
    let first = find(&layout, &dom, "a").expect("auto-margin item");
    let second = find(&layout, &dom, "b").expect("following item");

    // The single auto side absorbs all 200px of free space, pushing both
    // items right; placement steps must include the absorbed margin.
    assert_eq!(first.border_box.x, main.content_box.x + 200.0);
    assert_eq!(second.border_box.x, main.content_box.x + 250.0);
}

#[test]
fn flex_column_auto_margin_absorbs_free_space_before_justify() {
    let html = r#"
        <body>
          <main style="display:flex;flex-direction:column;height:300px;width:60px">
            <a style="height:50px;margin-top:auto"></a>
            <b style="height:50px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let main = find(&layout, &dom, "main").expect("column flex container");
    let first = find(&layout, &dom, "a").expect("column auto-margin item");
    let second = find(&layout, &dom, "b").expect("column following item");

    // Column containers absorb free space into auto top/bottom margins
    // exactly like rows do, leaving justify-content nothing.
    assert_eq!(first.border_box.y, main.content_box.y + 200.0);
    assert_eq!(second.border_box.y, main.content_box.y + 250.0);
}

#[test]
fn grid_template_longhand_rejects_reserved_line_names() {
    let html = r#"
        <body>
          <main style="display:grid;width:100px;grid-template-columns:[default] 40px 60px">
            <a style="grid-column:default;height:10px"></a>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let item = find(&layout, &dom, "a").expect("reserved-name item");

    // `[default]` is a reserved custom-ident: the longhand winner is invalid,
    // behaves as unset, and must not create a named line to place into.
    // The item falls back to the implicit auto column, which stretches to
    // fill the 100px grid container under justify-content: normal.
    assert_eq!(item.content_box.w, 100.0);
}

#[test]
fn invalid_flex_shorthand_does_not_override_earlier_valid_longhands() {
    let html = r#"
        <body>
          <main style="display:flex;width:300px">
            <a style="flex-grow:2;flex-shrink:2;flex-basis:30px;flex:1 2 banana;height:10px"></a>
            <b style="flex:1 1 0px;height:10px"></b>
          </main>
          <section style="display:flex;width:300px">
            <i style="flex-grow:2;flex-basis:30px;flex:1 2 3;height:10px"></i>
            <em style="flex:1 1 0px;height:10px"></em>
          </section>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);

    // An invalid `flex` shorthand emits no cascade candidates, so the
    // earlier valid longhands survive: 30px basis + 2/3 of the 270px
    // remainder, with the sibling taking the rest.
    let lettered = find(&layout, &dom, "a").expect("lettered item");
    let bold = find(&layout, &dom, "b").expect("bold sibling");
    assert_eq!(lettered.content_box.w, 210.0);
    assert_eq!(bold.content_box.w, 90.0);

    // A unitless non-zero number is likewise an invalid flex-basis component.
    let italic = find(&layout, &dom, "i").expect("italic item");
    let emphasis = find(&layout, &dom, "em").expect("emphasis sibling");
    assert_eq!(italic.content_box.w, 210.0);
    assert_eq!(emphasis.content_box.w, 90.0);
}

#[test]
fn invalid_grid_gap_aliases_do_not_override_earlier_valid_gap() {
    let html = r#"
        <body>
          <main style="display:grid;gap:10px 20px;grid-row-gap:nonsense;grid-template-columns:40px 40px;width:100px">
            <a style="height:10px"></a>
            <b style="height:10px"></b>
            <c style="height:10px"></c>
          </main>
          <section style="display:grid;gap:10px 20px;grid-gap:nonsense;width:100px">
            <i style="height:10px"></i>
            <em style="height:10px"></em>
            <u style="height:10px"></u>
          </section>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let main = find(&layout, &dom, "main").expect("explicit grid container");
    let section = find(&layout, &dom, "section").expect("implicit grid container");

    // Invalid legacy gap aliases emit no cascade candidates, so the valid
    // `gap` row-gap of 10px separates the second row from the first.
    let third = find(&layout, &dom, "c").expect("third explicit-grid item");
    assert_eq!(third.border_box.y, main.content_box.y + 20.0);

    let second = find(&layout, &dom, "em").expect("second implicit-grid item");
    assert_eq!(second.border_box.y, section.content_box.y + 20.0);
}

#[test]
fn flex_basis_first_shorthand_assigns_grow_and_shrink() {
    let html = r#"
        <body>
          <main style="display:flex;width:340px">
            <a style="flex:30px 2;height:10px"></a>
            <b style="flex:10px 1;height:10px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("basis-first item");
    let second = find(&layout, &dom, "b").expect("second basis-first item");

    // §7.1.10's || combinator allows `<'flex-basis'> <'flex-grow'>`:
    // 30px basis growing by 2 and 10px basis growing by 1 share the
    // 300px remainder proportionally.
    assert_eq!(first.content_box.w, 230.0);
    assert_eq!(second.content_box.w, 110.0);
}

#[test]
fn grid_template_accepts_adjacent_line_name_blocks() {
    let html = r#"
        <body>
          <main style="display:grid;width:100px;grid-template-columns:[a][b] 10px">
            <a style="grid-column:a;height:10px"></a>
            <b style="grid-column:b;height:10px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let first = find(&layout, &dom, "a").expect("first named-line item");
    let second = find(&layout, &dom, "b").expect("second named-line item");

    // `[a][b]` registers BOTH names on line 1 without intervening
    // whitespace; both items place into the 10px first track.
    assert_eq!(first.content_box.w, 10.0);
    assert_eq!(second.content_box.w, 10.0);
}

#[test]
fn invalid_gap_shorthand_does_not_override_earlier_valid_longhands() {
    let html = r#"
        <body>
          <main style="display:grid;row-gap:15px;gap:nonsense;grid-template-columns:40px;width:100px">
            <a style="height:10px"></a>
            <b style="height:10px"></b>
          </main>
        </body>
    "#;
    let dom = amazing_html::parse(html);
    let fonts = FontSource::fallback();
    let layout = layout_document(html, "", 400.0, &fonts);
    let main = find(&layout, &dom, "main").expect("gap grid container");
    let second = find(&layout, &dom, "b").expect("second row item");

    // An invalid `gap` shorthand emits no cascade candidates, so the
    // earlier valid row-gap of 15px separates the two rows.
    assert_eq!(second.border_box.y, main.content_box.y + 25.0);
}
