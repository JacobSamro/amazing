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
        near(c[0].border_box.w, 200.0 / 3.0);
        near(c[1].border_box.w, 400.0 / 3.0);
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
fn collapsed_border_joins_cover_short_empty_rows() {
    let b = layout(
        "<body><table><tr><td>x</td></tr><tr style='height:2px'></tr><tr><td>y</td></tr></table>",
        "table{border-collapse:collapse}td{border:10px solid black;line-height:0;padding:0}",
        false,
    );
    let t = collect(&b, Role::Table)[0];
    let x = t
        .table_decoration()
        .borders
        .iter()
        .filter(|edge| !edge.horizontal && edge.rect.w == 10.0)
        .map(|edge| edge.rect.x)
        .fold(f32::INFINITY, f32::min);
    let mut left_edges = t
        .table_decoration()
        .borders
        .iter()
        .filter(|edge| !edge.horizontal && edge.rect.w == 10.0 && (edge.rect.x - x).abs() < 0.02)
        .collect::<Vec<_>>();
    left_edges.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y));
    assert!(left_edges.len() >= 2);
    assert!(left_edges[0].rect.y + left_edges[0].rect.h >= left_edges[1].rect.y);
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
fn table_min_height_grows_past_intrinsic_row_height() {
    // css-tables/#computing-the-table-height: `min-height` applies to the
    // table's WRAPPER box (padding/border edge), not its grid box, so the
    // grid must grow by exactly enough to make the wrapper reach it.
    let b = layout(
        "<body><table><tr><td><div></div></td></tr></table>",
        "table{min-height:312px;border:1px solid black;padding:5px}div{width:300px;height:5px}",
        false,
    );
    let table = &kids(&b)[0];
    near(table.border_box.h, 312.0);
}

#[test]
fn table_min_height_wins_over_a_smaller_max_height() {
    // Same rule as every other min/max clamp in this codebase: min always
    // wins when it exceeds max, even though that means max-height did not
    // actually cap anything here.
    let b = layout(
        "<body><table><tr><td><div></div></td></tr></table>",
        "table{min-height:150px;max-height:40px;border:1px solid black;padding:5px}div{width:300px;height:5px}",
        false,
    );
    let table = &kids(&b)[0];
    near(table.border_box.h, 150.0);
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

#[test]
fn fixed_table_distributes_excess_by_column_kind_and_width() {
    for shaped in [false, true] {
        for (columns, expected) in [
            (
                "<col style='width:20px'><col style='width:60px'>",
                [40.0, 120.0],
            ),
            (
                "<col style='width:25%'><col style='width:25%'>",
                [80.0, 80.0],
            ),
            ("<col style='width:40px'><col>", [40.0, 120.0]),
            (
                "<col style='width:0px'><col style='width:0px'>",
                [80.0, 80.0],
            ),
        ] {
            let b = layout(
                &format!("<body><table>{columns}<tr><td></td><td></td></tr></table>"),
                "table{table-layout:fixed;width:160px}",
                shaped,
            );
            let cells = collect(&b, Role::Cell);
            near(cells[0].border_box.w, expected[0]);
            near(cells[1].border_box.w, expected[1]);
        }
    }
    // The pinned scripted fixed-layout-excess-width-distribution fixture:
    // excess goes to absolute columns before percentage columns.
    let b = layout(
        "<body><table><tr><td style='width:20px'></td><td style='width:10px'></td><td style='width:10%'></td></tr></table>",
        "table{table-layout:fixed;width:300px}",
        false,
    );
    for (cell, width) in collect(&b, Role::Cell).iter().zip([180.0, 90.0, 30.0]) {
        near(cell.border_box.w, width);
    }
}

#[test]
fn fixed_table_preserves_lengths_when_percentages_overcommit() {
    for columns in [
        "<col style='width:100px'><col style='width:80%'>",
        "<col style='width:calc(50px + 40%)'><col style='width:calc(50px + 40%)'>",
    ] {
        let b = layout(
            &format!("<body><table>{columns}<tr><td></td><td></td></tr></table>"),
            "table{table-layout:fixed;width:200px}",
            false,
        );
        let cells = collect(&b, Role::Cell);
        near(cells[0].border_box.w, 100.0);
        near(cells[1].border_box.w, 100.0);
        near(collect(&b, Role::Table)[0].border_box.w, 200.0);
    }
}

#[test]
fn fixed_table_colgroup_fallback_and_spanning_cell_use_unassigned_columns() {
    let b = layout(
        "<body><table><colgroup style='width:40px'><col><col style='width:60px'></colgroup><col><tr><td colspan='2' style='width:200px'></td><td></td></tr></table>",
        "table{table-layout:fixed;width:160px}",
        false,
    );
    let cols = collect(&b, Role::Column);
    near(cols[0].border_box.w, 40.0);
    near(cols[1].border_box.w, 60.0);
    near(cols[2].border_box.w, 60.0);
    let b = layout(
        "<body><table><col style='width:40px'><col><col><tr><td colspan='2' style='width:100px'></td><td></td></tr></table>",
        "table{table-layout:fixed;width:160px}",
        false,
    );
    let cols = collect(&b, Role::Column);
    near(cols[0].border_box.w, 40.0);
    near(cols[1].border_box.w, 60.0);
    near(cols[2].border_box.w, 60.0);
}

#[test]
fn table_cell_box_sizing_and_fixed_percentage_offsets() {
    for algorithm in ["auto", "fixed"] {
        for (sizing, expected) in [("border-box", 100.0), ("content-box", 124.0)] {
            let b = layout(
                "<body><table><tr><td></td><td></td></tr></table>",
                &format!(
                    "table{{table-layout:{algorithm};width:200px}}td:first-child{{width:100px;padding:10px;border:2px solid;box-sizing:{sizing}}}"
                ),
                false,
            );
            // The second, unconstrained column absorbs remaining space in fixed
            // mode. In auto mode isolate the outer width with shrink-to-fit.
            if algorithm == "fixed" {
                near(collect(&b, Role::Cell)[0].border_box.w, expected);
            }
            let b = layout(
                "<body><table><tr><td></td></tr></table>",
                &format!("td{{width:100px;padding:10px;border:2px solid;box-sizing:{sizing}}}"),
                false,
            );
            near(collect(&b, Role::Table)[0].border_box.w, expected);
        }
    }
    let b = layout(
        "<body><table><tr><td></td><td></td></tr></table>",
        "table{table-layout:fixed;width:200px}td:first-child{width:50%;padding:10px;border:2px solid}",
        false,
    );
    near(collect(&b, Role::Cell)[0].border_box.w, 124.0);
    near(collect(&b, Role::Cell)[1].border_box.w, 76.0);
}

#[test]
fn auto_table_prioritizes_percent_then_unconstrained_columns() {
    for shaped in [false, true] {
        let b = layout(
            "<body><table><tr><td style='width:25%'></td><td style='width:40px'></td><td></td></tr></table>",
            "table{width:200px}",
            shaped,
        );
        let c = collect(&b, Role::Cell);
        near(c[0].border_box.w, 50.0);
        near(c[1].border_box.w, 40.0);
        near(c[2].border_box.w, 110.0);
        let b = layout(
            "<body><table><tr><td style='width:80%'></td><td style='width:80%'></td></tr></table>",
            "table{width:200px}",
            shaped,
        );
        let c = collect(&b, Role::Cell);
        near(c[0].border_box.w, 160.0);
        near(c[1].border_box.w, 40.0);
    }
}

#[test]
fn auto_table_spanning_percentage_preserves_existing_column_shares() {
    let b = layout(
        "<body><table><col style='width:30%'><col><col><tr><td colspan='2' style='width:70%'></td><td></td></tr></table>",
        "table{width:200px}",
        false,
    );
    let cols = collect(&b, Role::Column);
    near(cols[0].border_box.w, 60.0);
    near(cols[1].border_box.w, 80.0);
    near(cols[2].border_box.w, 60.0);
}

#[test]
fn table_min_width_participates_in_column_distribution() {
    for algorithm in ["auto", "fixed"] {
        let b = layout(
            "<body><table><tr><td></td><td></td></tr></table>",
            &format!("table{{table-layout:{algorithm};width:100px;min-width:200px}}"),
            false,
        );
        near(collect(&b, Role::Table)[0].border_box.w, 200.0);
        for cell in collect(&b, Role::Cell) {
            near(cell.border_box.w, 100.0);
        }
    }
}

#[test]
fn fixed_spans_subtract_existing_columns_across_length_and_percentage_terms() {
    for (column, cell, expected) in [
        ("40px", "50%", [40.0, 60.0, 100.0]),
        ("30%", "100px", [60.0, 40.0, 100.0]),
    ] {
        let b = layout(
            &format!(
                "<body><table><col style='width:{column}'><col><col><tr><td colspan='2' style='width:{cell}'></td><td></td></tr></table>"
            ),
            "table{table-layout:fixed;width:200px}",
            false,
        );
        let cols = collect(&b, Role::Column);
        for (col, width) in cols.iter().zip(expected) {
            near(col.border_box.w, width);
        }
    }
}

#[test]
fn auto_table_cell_min_max_constraints_use_outer_box_sizes() {
    for shaped in [false, true] {
        for (sizing, expected) in [("content-box", 60.0), ("border-box", 40.0)] {
            let b = layout(
                "<body><table><tr><td></td></tr></table>",
                &format!("td{{min-width:40px;padding:10px;box-sizing:{sizing}}}"),
                shaped,
            );
            near(collect(&b, Role::Table)[0].border_box.w, expected);
        }
        let b = layout(
            "<body><table><tr><td>x x x x x x x</td></tr></table>",
            "td{max-width:30px}",
            shaped,
        );
        near(collect(&b, Role::Table)[0].border_box.w, 30.0);
    }
}

#[test]
fn auto_table_percentage_max_width_caps_share_not_intrinsic_text() {
    let b = layout(
        "<body><table><tr><td style='width:80%;max-width:25%'></td><td></td></tr></table>",
        "table{width:200px}",
        false,
    );
    let cells = collect(&b, Role::Cell);
    near(cells[0].border_box.w, 50.0);
    near(cells[1].border_box.w, 150.0);
    let b = layout(
        "<body><table><tr><td>x x x x</td></tr></table>",
        "td{min-width:90%;max-width:10%}",
        false,
    );
    let reference = layout("<body><table><tr><td>x x x x</td></tr></table>", "", false);
    near(
        collect(&b, Role::Table)[0].border_box.w,
        collect(&reference, Role::Table)[0].border_box.w,
    );
}

#[test]
fn auto_table_scroll_cell_max_width_limits_track_without_shrinking_contents() {
    for shaped in [false, true] {
        for overflow in ["auto", "scroll", "hidden"] {
            let b = layout(
                "<body><table><tr><td><div></div></td></tr></table>",
                &format!(
                    "td{{max-width:100px;overflow-x:{overflow}}}td div{{width:120px;height:50px}}"
                ),
                shaped,
            );
            near(collect(&b, Role::Table)[0].border_box.w, 100.0);
            let cell = collect(&b, Role::Cell)[0];
            near(cell.border_box.w, 100.0);
            near(kids(cell)[0].border_box.w, 120.0);
        }
    }
}

#[test]
fn fixed_table_decoration_stays_in_tracks_narrower_than_cell_offsets() {
    for shaped in [false, true] {
        for (padding, borders) in [
            ("50px 25px", ""),
            (
                "50px 10px",
                "border-left:15px solid;border-right:15px solid",
            ),
            ("50px 25px 50px 0", "border-left:25px solid"),
            ("50px 0 50px 25px", "border-right:25px solid"),
        ] {
            let b = layout(
                "<body><table><tr><td></td><td id='middle'></td><td></td></tr></table>",
                &format!(
                    "table{{table-layout:fixed;width:100px}}td{{padding:{padding}}}#middle{{width:50%;{borders}}}"
                ),
                shaped,
            );
            let cells = collect(&b, Role::Cell);
            for (cell, (x, width)) in cells.iter().zip([(0.0, 0.0), (0.0, 100.0), (100.0, 0.0)]) {
                near(cell.border_box.x, x);
                near(cell.border_box.w, width);
                near(cell.border_box.h, 100.0);
                near(cell.content_box.w, if width > 0.0 { 50.0 } else { 0.0 });
            }
        }
    }
}

#[test]
fn fixed_table_column_percentages_exclude_table_borders_and_spacing() {
    for (width, border, spacing, columns, expected) in [
        (
            "533px",
            29,
            15,
            "13% 22% 31% auto",
            [52.0, 88.0, 124.0, 136.0],
        ),
        (
            "533px",
            29,
            15,
            "13% 100px 31% auto",
            [52.0, 100.0, 124.0, 124.0],
        ),
        (
            "80%",
            11,
            18,
            "13% 100px 31% auto",
            [52.0, 100.0, 124.0, 124.0],
        ),
    ] {
        let cols = columns
            .split_whitespace()
            .map(|w| format!("<col style='width:{w}'>"))
            .collect::<String>();
        let b = layout(
            &format!("<body><table>{cols}<tr><td></td><td></td><td></td><td></td></tr></table>"),
            &format!(
                "body{{width:640px}}table{{table-layout:fixed;width:{width};border:solid;border-width:0 {border}px;border-spacing:{spacing}px 4px}}td{{padding:1px 10px}}"
            ),
            false,
        );
        let mut x = (border + spacing) as f32;
        for (cell, width) in collect(&b, Role::Cell).iter().zip(expected) {
            near(cell.border_box.x, x);
            near(cell.border_box.w, width);
            x += width + spacing as f32;
        }
    }
}
