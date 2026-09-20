//! Public-pipeline table regressions; use CSS table roles on ordinary elements
//! as well as HTML tables so HTML parser repair cannot hide layout repair bugs.
use crate::table_tree::{Role, role};
use crate::*;
fn layout(html: &str, css: &str, shaped: bool) -> LayoutBox {
    let dom = amazing_html::parse(html);
    let css = format!(
        "body{{margin:0;font-size:10px;line-height:10px}}td,th{{padding:0}}table{{border-spacing:0}}{css}"
    );
    if shaped {
        layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            &css,
            500.0,
            Some(500.0),
            &amazing_text::FallbackTextSystem::new(),
            &NoReplacedElements,
        )
    } else {
        layout_dom_document_with_replaced_in_viewport(
            &dom,
            &css,
            500.0,
            Some(500.0),
            &font::FontSource::fallback(),
            &NoReplacedElements,
        )
    }
}
fn kids(b: &LayoutBox) -> &[LayoutBox] {
    match &b.flow {
        Flow::Blocks(c) => c,
        _ => &[],
    }
}
fn collect<'a>(b: &'a LayoutBox, r: Role) -> Vec<&'a LayoutBox> {
    let mut out = Vec::new();
    if role(b) == r {
        out.push(b)
    }
    for c in kids(b) {
        out.extend(collect(c, r))
    }
    out
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "got {a}, expected {b}");
}
#[test]
fn table_orphan_cells_share_a_row_and_table() {
    let b = layout(
        "<body><div>A</div><div>B</div>",
        "div{display:table-cell;width:30px}",
        false,
    );
    let tables = collect(&b, Role::Table);
    assert_eq!(tables.len(), 1);
    let rows = collect(&b, Role::Row);
    assert_eq!(rows.len(), 1);
    assert!(tables[0].node.is_none() && rows[0].node.is_none());
    let cells = collect(&b, Role::Cell);
    near(cells[0].border_box.x, 0.0);
    near(cells[1].border_box.x - cells[0].border_box.x, 30.0);
}
#[test]
fn table_missing_child_wrappers_group_consecutive_content() {
    let b = layout(
        "<body><section><div>A</div><div>B</div><article><span>C</span><b>D</b></article></section>",
        "section{display:table}article{display:table-row}",
        false,
    );
    let rows = collect(&b, Role::Row);
    assert_eq!(rows.len(), 2);
    assert_eq!(collect(rows[0], Role::Cell).len(), 1);
    assert_eq!(kids(collect(rows[0], Role::Cell)[0]).len(), 2);
    assert_eq!(collect(rows[1], Role::Cell).len(), 1);
}
#[test]
fn table_columns_discard_irrelevant_children() {
    let b = layout(
        "<body><section><aside><div>ignored</div><i><b>ignored too</b></i></aside><article><p>x</p></article></section>",
        "section{display:table}aside{display:table-column-group}i{display:table-column}article{display:table-row}p{display:table-cell}",
        false,
    );
    let cols = collect(&b, Role::Column);
    assert_eq!(cols.len(), 1);
    assert!(matches!(cols[0].flow, Flow::Empty));
    assert_eq!(kids(collect(&b, Role::ColumnGroup)[0]).len(), 1);
}
#[test]
fn table_fixed_columns_then_first_row_then_equal_remainder() {
    for shaped in [false, true] {
        let b = layout(
            "<body><table><col style='width:60px'><col><col><tr><td>A</td><td style='width:90px'>B</td><td>C</td></tr><tr><td style='width:400px'>long content</td><td>D</td><td>E</td></tr></table>",
            "table{width:300px;table-layout:fixed}",
            shaped,
        );
        let c = collect(&b, Role::Cell);
        near(c[0].border_box.w, 60.0);
        near(c[1].border_box.w, 90.0);
        near(c[2].border_box.w, 150.0);
        near(c[3].border_box.w, 60.0);
    }
}
#[test]
fn table_auto_uses_content_and_distributes_extra_width() {
    for shaped in [false, true] {
        let b = layout(
            "<body><table><tr><td><div style='width:40px;height:10px'></div></td><td><div style='width:80px;height:10px'></div></td></tr></table>",
            "",
            shaped,
        );
        near(collect(&b, Role::Table)[0].border_box.w, 120.0);
        let b = layout(
            "<body><table><tr><td><div style='width:40px;height:10px'></div></td><td><div style='width:80px;height:10px'></div></td></tr></table>",
            "table{width:200px}",
            shaped,
        );
        let c = collect(&b, Role::Cell);
        near(c[0].border_box.w, 80.0);
        near(c[1].border_box.w, 120.0);
    }
}
#[test]
fn table_spans_allocate_grid_and_clamp_at_row_group_boundary() {
    let b = layout(
        "<body><table><tbody><tr><td rowspan=99>A</td><td>B</td></tr><tr><td>C</td></tr></tbody><tbody><tr><td colspan=2>D</td></tr></tbody></table>",
        "table{width:100px;table-layout:fixed}td{height:20px}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[0].border_box.h, 40.0);
    near(c[2].border_box.x, 50.0);
    near(c[3].border_box.w, 100.0);
    near(c[3].border_box.y, 40.0);
}
#[test]
fn table_zero_rowspan_grows_to_end_of_group_and_overlap_keeps_source_order() {
    let b = layout(
        "<body><table><tr><td>A</td><td rowspan=0>B</td><td>C</td></tr><tr><td colspan=2>D</td><td>E</td></tr></table>",
        "table{width:150px;table-layout:fixed}td{height:20px}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[1].border_box.h, 40.0);
    near(c[3].border_box.x, 0.0);
    near(c[3].border_box.w, 100.0);
    near(c[4].border_box.x, 100.0);
}
#[test]
fn table_separate_spacing_padding_and_centering() {
    let b = layout(
        "<body><table><tr><td>A</td><td>B</td></tr></table>",
        "table{width:120px;table-layout:fixed;border-spacing:10px 5px;margin:auto}td{padding:3px;height:10px}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    near(t.border_box.x, 190.0);
    near(t.border_box.h, 26.0);
    let c = collect(&b, Role::Cell);
    near(c[0].border_box.x, 200.0);
    near(c[0].border_box.w, 45.0);
    near(c[0].content_box.w, 39.0);
    near(c[1].border_box.x, 255.0);
}
#[test]
fn table_vertical_alignment_preserves_natural_content_height() {
    for shaped in [false, true] {
        let b = layout(
            "<body><table><tr><td><div></div></td><td class=mid><div></div></td><td class=bot><div></div></td></tr></table>",
            "td{height:60px;vertical-align:top}.mid{vertical-align:middle}.bot{vertical-align:bottom}td div{width:10px;height:20px}",
            shaped,
        );
        let c = collect(&b, Role::Cell);
        near(kids(c[0])[0].border_box.y, 0.0);
        near(kids(c[1])[0].border_box.y, 20.0);
        near(kids(c[2])[0].border_box.y, 40.0);
    }
}
#[test]
fn table_header_footer_and_caption_order() {
    let b = layout(
        "<body><table><caption>top</caption><tfoot><tr><td>foot</td></tr></tfoot><tbody><tr><td>body</td></tr></tbody><thead><tr><td>head</td></tr></thead></table>",
        "td{height:20px}",
        false,
    );
    let c = collect(&b, Role::Cell);
    assert!(c[2].border_box.y < c[1].border_box.y && c[1].border_box.y < c[0].border_box.y);
    near(collect(&b, Role::Caption)[0].border_box.y, 0.0);
}
#[test]
fn table_collapsed_winner_controls_half_border_geometry() {
    let b = layout(
        "<body><table><tr><td>A</td><td>B</td></tr></table>",
        "table{border-collapse:collapse;width:110px;table-layout:fixed}td{border:4px solid red}td+td{border-left-width:10px;border-left-style:solid;border-left-color:blue}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    assert!(t.table_decoration().suppress_border);
    let c = collect(&b, Role::Cell);
    near(c[0].border.right, 5.0);
    near(c[1].border.left, 5.0);
    let winner = t
        .table_decoration()
        .borders
        .iter()
        .find(|e| !e.horizontal && e.rect.w == 10.0)
        .unwrap();
    assert_eq!(winner.color, [0, 0, 255, 255]);
    near(winner.rect.x + 5.0, c[1].border_box.x);
    assert!(c.iter().all(|c| c.table_decoration().suppress_border));
}
#[test]
fn table_collapsed_hidden_wins_and_none_loses() {
    let b = layout(
        "<body><table><tr><td>A</td><td>B</td></tr></table>",
        "table{border-collapse:collapse}td{border:12px double red}td+td{border-left-style:hidden}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[0].border.right, 0.0);
    near(c[1].border.left, 0.0);
    let x = c[1].border_box.x;
    assert!(
        !collect(&b, Role::Table)[0]
            .table_decoration()
            .borders
            .iter()
            .any(|e| !e.horizontal && (e.rect.x + e.rect.w / 2.0 - x).abs() < 0.01)
    );
}
#[test]
fn table_collapsed_ties_prefer_style_then_cell_then_logical_start() {
    let b = layout(
        "<body><table><tr><td>A</td><td>B</td></tr></table>",
        "table{border-collapse:collapse;border:4px solid green}tr{border:4px solid blue}td{border:4px dashed red}td+td{border-left-width:4px;border-left-style:double;border-left-color:purple}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    assert!(
        t.table_decoration()
            .borders
            .iter()
            .any(|e| e.style == "double" && e.color == [128, 0, 128, 255])
    );
    // Solid row border wins over dashed cell regardless of origin priority.
    assert!(
        t.table_decoration()
            .borders
            .iter()
            .any(|e| e.horizontal && e.color == [0, 0, 255, 255])
    );
}
#[test]
fn table_spanning_cells_have_no_internal_border_segments() {
    let b = layout(
        "<body><table><tr><td rowspan=2>A</td><td>B</td></tr><tr><td>C</td></tr></table>",
        "table{border-collapse:collapse}td,tr{border:2px solid red}td{height:20px}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    let c = collect(&b, Role::Cell);
    let middle = c[2].border_box.y;
    assert!(!t.table_decoration().borders.iter().any(|e| e.horizontal
        && (e.rect.y + e.rect.h / 2.0 - middle).abs() < 0.01
        && e.rect.x < c[1].border_box.x - 0.01));
}
#[test]
fn table_empty_cells_hide_decoration_without_losing_layout() {
    let b = layout(
        "<body><table><tr><td></td><td>x</td></tr></table>",
        "table{width:100px;table-layout:fixed;empty-cells:hide}tr{background:red}td{height:20px;border:2px solid blue}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[0].border_box.w, 50.0);
    assert!(c[0].table_decoration().suppress_border && c[0].table_decoration().suppress_background);
    assert!(!c[1].table_decoration().suppress_background);
    assert_eq!(
        collect(&b, Role::Row)[0]
            .table_decoration()
            .background_holes,
        [c[0].border_box]
    );
    let b = layout(
        "<body><table><tr><td></td></tr></table>",
        "table{border-collapse:collapse;empty-cells:hide}td{height:20px;border:2px solid blue}",
        false,
    );
    assert!(
        !collect(&b, Role::Cell)[0]
            .table_decoration()
            .suppress_background
    );
}
#[test]
fn table_caption_is_outside_grid_decoration_and_moves_with_wrapper() {
    let b = layout(
        "<body><table><caption>caption</caption><tr><td>x</td></tr></table>",
        "table{width:100px;position:relative;left:7px;top:3px;border:2px solid blue}caption{height:20px}td{height:30px}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    let cap = collect(&b, Role::Caption)[0];
    near(cap.border_box.x, 7.0);
    near(cap.border_box.y, 3.0);
    near(t.border_box.y, 23.0);
    near(t.border_box.h, 34.0);
    assert_eq!(t.style.get("position"), Some("static"));
    assert!(kids(&b)[0].table_wrapper);
}
#[test]
fn table_baseline_aligns_first_lines_of_different_font_sizes() {
    for shaped in [false, true] {
        let b = layout(
            "<body><table><tr><td>A</td><td class=large>B</td></tr></table>",
            "td{vertical-align:baseline}.large{font-size:30px;line-height:40px}",
            shaped,
        );
        let c = collect(&b, Role::Cell);
        near(c[0].lines[0].baseline, c[1].lines[0].baseline);
    }
}
#[test]
fn table_rtl_columns_and_winning_physical_borders() {
    let b = layout(
        "<body><table><tr><td>A</td><td>B</td></tr></table>",
        "table{direction:rtl;table-layout:fixed;width:100px;border-collapse:collapse}td{border:2px solid red}td+td{border-right-width:8px;border-right-color:blue}",
        false,
    );
    let c = collect(&b, Role::Cell);
    assert!(c[0].border_box.x > c[1].border_box.x);
    near(c[0].border.left, 4.0);
    near(c[1].border.right, 4.0);
    let segment = collect(&b, Role::Table)[0]
        .table_decoration()
        .borders
        .iter()
        .find(|e| !e.horizontal && e.rect.w == 8.0)
        .unwrap();
    assert_eq!(segment.color, [0, 0, 255, 255]);
}
#[test]
fn table_span_attribute_clamping_is_bounded_and_css_cells_ignore_html_spans() {
    let b = layout(
        "<body><table><tr><td colspan='99999999999999999999999999' rowspan=0>x</td></tr></table>",
        "",
        false,
    );
    let c = collect(&b, Role::Cell);
    assert_eq!(c[0].table_colspan, 1000);
    assert!(c[0].border_box.w.is_finite());
    let b = layout(
        "<body><section><div colspan=3>A</div><div>B</div></section>",
        "section{display:table-row}div{display:table-cell;width:20px}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[1].border_box.x - c[0].border_box.x, 20.0);
}
#[test]
fn table_relayout_does_not_accumulate_alignment_or_border_offsets() {
    let mut b = layout(
        "<body><table><caption>top</caption><tr><td>A</td><td><div></div></td></tr></table>",
        "table{width:100px;border-collapse:collapse}td{border:4px solid red;vertical-align:bottom}div{height:40px;width:20px}",
        false,
    );
    let before: Vec<_> = collect(&b, Role::Cell)
        .iter()
        .map(|c| (c.border_box, c.content_box, c.padding))
        .collect();
    let edges = collect(&b, Role::Table)[0]
        .table_decoration()
        .borders
        .clone();
    block::layout_root(&mut b, 500.0, &font::FontSource::fallback());
    let after: Vec<_> = collect(&b, Role::Cell)
        .iter()
        .map(|c| (c.border_box, c.content_box, c.padding))
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        edges,
        collect(&b, Role::Table)[0].table_decoration().borders
    );
}
#[test]
fn table_empty_collapsed_grid_keeps_declared_height_and_ignores_padding() {
    let b = layout(
        "<body><table></table>",
        "table{border-collapse:collapse;width:100px;height:100px;padding:100px}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    near(t.border_box.w, 100.0);
    near(t.border_box.h, 100.0);
    near(t.padding.left, 0.0);
}

#[test]
fn table_empty_cells_does_not_hide_preserved_spaces_or_in_flow_boxes() {
    let b = layout(
        "<body><table><tr><td style='white-space:pre'> </td><td><div></div></td></tr></table>",
        "table{empty-cells:hide}td{height:20px}div{width:10px;height:10px}",
        false,
    );
    assert!(
        collect(&b, Role::Cell)
            .iter()
            .all(|c| !c.table_decoration().suppress_background)
    );
}

#[test]
fn table_direct_row_span_stops_before_an_explicit_group() {
    let b = layout(
        "<body><table><tbody style='display:contents'><tr><td rowspan=99>A</td><td>B</td></tr></tbody><tbody><tr><td>C</td><td>D</td></tr></tbody></table>",
        "table{width:100px;table-layout:fixed}td{height:20px}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[0].border_box.h, 20.0);
    near(c[2].border_box.x, 0.0);
    near(c[2].border_box.y, 20.0);
}

#[test]
fn table_rowspan_explicit_height_is_a_minimum_across_owned_rows() {
    let b = layout(
        "<body><table><tr><td rowspan=2 style='height:80px'>A</td><td>B</td></tr><tr><td>C</td></tr></table>",
        "td{vertical-align:top}table{border-spacing:0}",
        false,
    );
    let c = collect(&b, Role::Cell);
    near(c[0].border_box.h, 80.0);
    near(c[1].border_box.h + c[2].border_box.h, 80.0);
}

#[test]
fn table_intrinsic_width_in_a_float_honors_width_and_collapsed_padding() {
    for model in ["content-box", "border-box"] {
        let b = layout(
            "<body><div><table></table></div>",
            &format!(
                "div{{float:left}}table{{width:100px;height:100px;border-collapse:collapse;padding:100px;box-sizing:{model}}}"
            ),
            false,
        );
        near(kids(&b)[0].border_box.w, 100.0);
        near(kids(&b)[0].border_box.h, 100.0);
    }
}
