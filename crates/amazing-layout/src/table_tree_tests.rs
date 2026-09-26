//! Anonymous table repair through both public layout pipelines.
use crate::table_tree::{Role, role};
use crate::*;

fn layout(html: &str, css: &str, shaped: bool) -> LayoutBox {
    let dom = amazing_html::parse(html);
    let css = format!(
        "body{{margin:0;font:10px/10px serif}}table{{border-spacing:0}}td{{padding:0}}{css}"
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
fn collect(b: &LayoutBox, wanted: Role) -> Vec<&LayoutBox> {
    let mut out = Vec::new();
    if role(b) == wanted {
        out.push(b);
    }
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &b.flow {
        for c in children {
            out.extend(collect(c, wanted));
        }
    }
    for c in &b.positioned {
        out.extend(collect(c, wanted));
    }
    out
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "got {a}, expected {b}");
}

#[test]
fn orphan_cells_in_inline_parent_share_an_inline_table_and_surrounding_line() {
    for shaped in [false, true] {
        for separator in ["", " ", "\n <!-- comment --> \t"] {
            let b = layout(
                &format!(
                    "<body><span>a <span class='cell'>b</span>{separator}<span class='cell'>c</span> d</span>"
                ),
                ".cell{display:table-cell}",
                shaped,
            );
            let reference = layout("<body>a bc d", "", shaped);
            assert_eq!(b.lines.len(), 1);
            assert_eq!(collect(&b, Role::Table).len(), 1);
            let cells = collect(&b, Role::Cell);
            assert_eq!(cells.len(), 2);
            near(
                cells[1].border_box.x,
                cells[0].border_box.x + cells[0].border_box.w,
            );
            near(cells[0].lines[0].baseline, b.lines[0].baseline);
            near(cells[1].lines[0].baseline, b.lines[0].baseline);
            near(line_end(&b.lines[0]), line_end(&reference.lines[0]));
        }
    }
}

#[test]
fn generated_text_remains_outside_anonymous_inline_table() {
    for shaped in [false, true] {
        for (html, css) in [
            (
                "<body><span id='t'>a <span class='cell'>b</span> <span class='cell'>c</span> </span>",
                "#t:after{content:'d'}",
            ),
            (
                "<body><span id='t'> <span class='cell'>b</span> <span class='cell'>c</span> d</span>",
                "#t:before{content:'a'}",
            ),
        ] {
            let b = layout(html, &format!(".cell{{display:table-cell}}{css}"), shaped);
            let reference = layout("<body>a bc d", "", shaped);
            assert_eq!(b.lines.len(), 1);
            near(line_end(&b.lines[0]), line_end(&reference.lines[0]));
            assert_eq!(collect(&b, Role::Cell).len(), 2);
        }
    }
}

#[test]
fn non_whitespace_inline_content_separates_anonymous_tables() {
    for separator in ["x", "&nbsp;", "<span> </span>"] {
        let b = layout(
            &format!("<body><span><i>b</i>{separator}<i>c</i></span>"),
            "i{display:table-cell}",
            false,
        );
        assert_eq!(collect(&b, Role::Table).len(), 2);
        assert_eq!(b.lines.len(), 1);
    }
    let b = layout(
        "<body><span><i>b</i><span style='display:none'>x</span><i>c</i></span>",
        "i{display:table-cell}",
        false,
    );
    assert_eq!(collect(&b, Role::Table).len(), 1);
}

#[test]
fn explicit_inline_table_baseline_uses_first_row_below_caption() {
    for shaped in [false, true] {
        let b = layout(
            "<body>a<table><caption>cap</caption><tr><td>b</td></tr><tr><td>c</td></tr></table>d",
            "table{display:inline-table}",
            shaped,
        );
        assert_eq!(b.lines.len(), 1);
        let cells = collect(&b, Role::Cell);
        near(cells[0].lines[0].baseline, b.lines[0].baseline);
        assert!(cells[1].lines[0].baseline > b.lines[0].baseline);
        let caption = collect(&b, Role::Caption)[0];
        assert!(caption.lines[0].baseline < b.lines[0].baseline);
    }
}

#[test]
fn anonymous_cell_owns_atomic_inline_children_and_their_whitespace() {
    for shaped in [false, true] {
        let b = layout(
            "<body><div class='table'><div class='half'>A</div> <div class='half'>B</div></div>",
            ".table{display:table;width:500px}.half{display:inline-block;width:50%}",
            shaped,
        );
        let cell = collect(&b, Role::Cell)[0];
        let Flow::Blocks(children) = &cell.flow else {
            panic!("anonymous inline content host")
        };
        let host = &children[0];
        assert_eq!(host.positioned.len(), 2);
        assert_eq!(
            host.lines.len(),
            2,
            "intervening whitespace forces wrapping"
        );
        for atom in &host.positioned {
            near(atom.border_box.w, 250.0);
        }
        assert!(host.positioned[1].border_box.y > host.positioned[0].border_box.y);
    }
}

#[test]
fn anonymous_cell_keeps_collapsing_child_margins_inside_its_bfc() {
    let b = layout(
        "<body><div class='table'><div class='row'><div style='width:100px;margin:50px 0'><div style='margin:50px 0'></div></div><div style='margin:50px 0'></div></div><div class='row'><div style='margin:50px 0'></div></div><div class='row'></div></div>",
        ".table{display:table;height:100px}.row{display:table-row}",
        false,
    );
    let rows = collect(&b, Role::Row);
    near(rows[0].border_box.h, 50.0);
    near(rows[1].border_box.h, 50.0);
    near(rows[2].border_box.h, 0.0);
    near(collect(&b, Role::Table)[0].border_box.h, 100.0);
}

#[test]
fn preserved_whitespace_between_table_parts_does_not_create_cells() {
    let b = layout(
        "<body><div class='table'>\n<div class='row'>\n<span>A</span>\n<span>B</span>\n</div>\n</div>",
        ".table{display:table;white-space:pre}.row{display:table-row}.row span{display:table-cell}",
        false,
    );
    assert_eq!(collect(&b, Role::Row).len(), 1);
    assert_eq!(collect(&b, Role::Cell).len(), 2);
}

fn line_end(line: &LineBox) -> f32 {
    line.words
        .iter()
        .map(|w| w.x + w.width)
        .chain(line.glyphs.iter().map(|g| g.x + g.advance_x))
        .chain(line.replaced.iter().map(|r| r.x + r.width))
        .fold(0.0, f32::max)
}

#[test]
fn inline_table_baseline_ignores_other_cells_vertical_alignment() {
    for shaped in [false, true] {
        let b = layout(
            "<body>a<table><tr><td>b</td><td style='vertical-align:bottom;padding-top:30px'>c</td></tr></table>d",
            "table{display:inline-table}td{vertical-align:baseline}",
            shaped,
        );
        let cells = collect(&b, Role::Cell);
        near(b.lines[0].baseline, cells[0].lines[0].baseline);
        assert!(cells[1].lines[0].baseline > b.lines[0].baseline);
        let b = layout(
            "<body>a<table><tr><td>b<br>c</td></tr></table>d",
            "table{display:inline-table}td{vertical-align:top}",
            shaped,
        );
        let cell = collect(&b, Role::Cell)[0];
        near(b.lines[0].baseline, cell.content_box.y + cell.content_box.h);
    }
}

#[test]
fn nested_inline_table_contributes_to_the_outer_column_intrinsic_size() {
    for shaped in [false, true] {
        let nested = layout(
            "<body><table><tr><td>x</td><td>y</td></tr><tr><td><span class='inner'>longest first column</span></td><td>z</td></tr></table>",
            ".inner{display:inline-table}",
            shaped,
        );
        let plain = layout(
            "<body><table><tr><td>x</td><td>y</td></tr><tr><td>longest first column</td><td>z</td></tr></table>",
            "",
            shaped,
        );
        let actual = collect(&nested, Role::Table)[0];
        let expected = collect(&plain, Role::Table)[0];
        near(actual.border_box.w, expected.border_box.w);
        near(
            collect(&nested, Role::Cell)[1].border_box.x,
            collect(&plain, Role::Cell)[1].border_box.x,
        );
    }
}

#[test]
fn empty_inline_table_row_synthesizes_baseline_from_its_minimum_height() {
    for shaped in [false, true] {
        let b = layout(
            "<body><div class='table'><div class='caption'>caption</div><div class='cell'></div></div>",
            ".table{display:inline-table;caption-side:bottom}.caption{display:table-caption}.cell{display:table-cell;width:200px;height:100px}",
            shaped,
        );
        let table = collect(&b, Role::Table)[0];
        near(table.border_box.y, 0.0);
        near(table.border_box.h, 100.0);
        near(b.lines[0].baseline, 100.0);
        near(collect(&b, Role::Caption)[0].border_box.y, 100.0);
    }
}

#[test]
fn css_table_box_sizing_distinguishes_content_and_border_edges() {
    for shaped in [false, true] {
        for (sizing, expected) in [("content-box", 224.0), ("border-box", 200.0)] {
            let b = layout(
                "<body><div class='table'><div class='cell'></div></div>",
                &format!(
                    ".table{{display:table;width:200px;padding:10px;border:2px solid;box-sizing:{sizing}}}.cell{{display:table-cell}}"
                ),
                shaped,
            );
            near(collect(&b, Role::Table)[0].border_box.w, expected);
        }
        let b = layout(
            "<body><div class='table'><span>A</span> <span>B</span></div>",
            ".table{display:table;width:500px;border:1px solid}span{display:inline-block;width:50%}",
            shaped,
        );
        near(collect(&b, Role::Table)[0].border_box.w, 502.0);
        let cell = collect(&b, Role::Cell)[0];
        near(cell.content_box.w, 500.0);
    }
}

#[test]
fn inline_table_caption_shaping_matches_block_at_nonzero_origin() {
    let actual = layout(
        "<body><p>intro</p><div class='table'><div class='caption'>Filler Text</div><div class='cell'></div></div>",
        "body{margin:8px;font:16px/normal serif}.table{display:inline-table;caption-side:bottom}.caption{display:table-caption}.cell{display:table-cell;width:192px;height:96px}",
        true,
    );
    let expected = layout(
        "<body><p>intro</p><div style='width:192px;height:96px'></div><div>Filler Text</div>",
        "body{margin:8px;font:16px/normal serif}",
        true,
    );
    let caption = collect(&actual, Role::Caption)[0];
    let Flow::Blocks(children) = &expected.flow else {
        panic!("blocks")
    };
    let reference = &children[2];
    assert_eq!(
        caption.lines[0]
            .glyphs
            .iter()
            .map(|g| (g.x, g.y, g.advance_x))
            .collect::<Vec<_>>(),
        reference.lines[0]
            .glyphs
            .iter()
            .map(|g| (g.x, g.y, g.advance_x))
            .collect::<Vec<_>>()
    );
}
