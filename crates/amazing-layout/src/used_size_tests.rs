//! Actual border-box, anonymous context and generated source geometry.
use crate::*;
use amazing_css::{ComputedStyle, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};
use amazing_text::FallbackTextSystem;
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
}
fn node(dom: &Dom, id: &str) -> NodeIdx {
    (0..dom.node_count())
        .map(|n| NodeIdx(n as u32))
        .find(|&n| {
            dom.attrs_of(n)
                .is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "id" && value == id))
        })
        .unwrap()
}
fn layout(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = amazing_html::parse(&format!("<body>{html}"));
    let root = layout_dom_document_with_text_and_replaced_in_viewport(
        &dom,
        &format!("body{{margin:0;font:10px/10px serif}}{css}"),
        300.0,
        Some(200.0),
        &FallbackTextSystem::new(),
        &NoReplacedElements,
    );
    (dom, root)
}
fn geometry(root: &LayoutBox, dom: &Dom, id: &str) -> ResolvedStyleGeometry {
    resolved_style_geometry(root, LayoutSource::Element(node(dom, id))).unwrap()
}
#[test]
fn declared_border_box_dimensions_and_limits_use_actual_content_edges() {
    for zoom in [1.0, 2.0] {
        for (width, min, max, expected) in [
            ("50px", "0", "none", 50.0),
            ("50%", "0", "none", 50.0),
            ("calc(40px + 10%)", "0", "none", 50.0),
            ("20px", "50px", "none", 50.0),
            ("80px", "0", "50px", 50.0),
            ("5px", "0", "none", 10.0),
        ] {
            let (dom, root) = layout(
                "<div id='parent'><div id='box'></div></div>",
                &format!(
                    "#parent{{width:100px;zoom:{zoom}}}#box{{box-sizing:border-box;width:{width};min-width:{min};max-width:{max};height:30px;padding:3px;border:2px solid}}"
                ),
            );
            let b = geometry(&root, &dom, "box");
            near(b.border_box.w, expected * zoom);
            near(b.content_box.w, (expected - 10.0) * zoom);
            near(b.border_box.h, 30.0 * zoom);
            near(b.content_box.h, 20.0 * zoom);
        }
    }
}
#[test]
fn parent_content_assignment_preserves_declared_constraints_and_style() {
    let mut style = ComputedStyle::default();
    for (key, value) in [
        ("box-sizing", "border-box"),
        ("width", "80px"),
        ("height", "auto"),
        ("min-width", "40px"),
        ("max-width", "50px"),
        ("padding-left", "3px"),
        ("padding-right", "3px"),
        ("border-left-width", "2px"),
        ("border-right-width", "2px"),
        ("border-left-style", "solid"),
        ("border-right-style", "solid"),
    ] {
        style.set(key, value);
    }
    let mut b = LayoutBox::new(None, Display::Block, style.clone(), 10.0, 10.0);
    block::layout_block_with_used_content_size(
        &mut b,
        0.0,
        0.0,
        100.0,
        &font::FontSource::fallback(),
        None,
        UsedContentSize {
            width: Some(70.0),
            height: Some(25.0),
        },
    );
    near(b.content_box.w, 40.0);
    near(b.border_box.w, 50.0);
    near(b.content_box.h, 25.0);
    for key in ["width", "height", "box-sizing", "min-width", "max-width"] {
        assert_eq!(b.style.get(key), style.get(key));
    }
    let before = resolved_geometry::capture_geometry_snapshot(&mut b);
    b.used_content_size.height = Some(30.0);
    assert_ne!(resolved_geometry::capture_geometry_snapshot(&mut b), before);
}
#[test]
fn anonymous_table_wrapper_uses_accumulated_zoom_and_actual_grid_dimensions() {
    for zoom in [1.0, 2.0, 0.5] {
        let (dom, root) = layout(
            "<table id='table'><caption id='caption'>C</caption><tr><td>X</td></tr></table>",
            &format!(
                "#table{{zoom:{zoom};margin-left:5px;width:50px}}#caption{{height:7px;margin:0}}"
            ),
        );
        let t = geometry(&root, &dom, "table");
        assert_eq!(t.effective_zoom, zoom as f64);
        near(t.margin.left.unwrap(), 5.0 * zoom);
        near(t.border_box.w, 50.0 * zoom);
        let c = geometry(&root, &dom, "caption");
        near(c.content_box.h, 7.0 * zoom);
        assert!(t.table_wrapper_box.unwrap().h >= t.border_box.h + 7.0 * zoom);
    }
}
#[test]
fn contents_pseudos_are_actual_generated_children_in_block_flex_and_grid() {
    for display in ["block", "flex", "grid"] {
        let (dom, root) = layout(
            "<div id='parent'><div id='contents'><div id='middle'></div></div></div>",
            &format!(
                "#parent{{display:{display};width:100px;align-items:start;justify-items:start;grid-template-columns:100px}}#contents{{display:contents}}#contents::before{{content:'C';display:block;width:50%;height:7px}}#contents::after{{content:'D';display:block;width:20px;height:9px}}#middle{{width:10px;height:8px}}"
            ),
        );
        assert!(
            resolved_style_geometry(&root, LayoutSource::Element(node(&dom, "contents"))).is_none()
        );
        let before = resolved_style_geometry(
            &root,
            LayoutSource::Pseudo(node(&dom, "contents"), PseudoElement::Before),
        )
        .unwrap();
        let after = resolved_style_geometry(
            &root,
            LayoutSource::Pseudo(node(&dom, "contents"), PseudoElement::After),
        )
        .unwrap();
        near(before.content_box.w, 50.0);
        near(before.content_box.h, 7.0);
        near(after.content_box.w, 20.0);
        near(after.content_box.h, 9.0);
        if display == "block" {
            let middle = geometry(&root, &dom, "middle");
            assert!(
                before.border_box.y < middle.border_box.y
                    && middle.border_box.y < after.border_box.y
            );
        }
    }
}
#[test]
fn contents_generated_positioned_and_nested_sources_keep_real_owner_and_context() {
    let (dom, root) = layout(
        "<div id='parent'><span><div id='outer'><div id='inner'></div></div></span></div>",
        "#parent{position:relative;width:100px;height:80px}#outer,#inner{display:contents}#inner::before{content:'P';position:absolute;left:10%;top:5px;width:50%;height:7px}#outer::after{content:'A';display:block;width:20px;height:9px}",
    );
    let p = resolved_style_geometry(
        &root,
        LayoutSource::Pseudo(node(&dom, "inner"), PseudoElement::Before),
    )
    .unwrap();
    near(p.containing_block.w, 100.0);
    near(p.content_box.w, 50.0);
    near(p.content_box.h, 7.0);
    near(p.border_box.x, 10.0);
    near(p.border_box.y, 5.0);
    assert!(
        resolved_style_geometry(
            &root,
            LayoutSource::Pseudo(node(&dom, "outer"), PseudoElement::After)
        )
        .is_some()
    );
}
#[test]
fn flex_grid_border_box_and_stretch_keep_original_style_and_used_height() {
    for display in ["flex", "grid"] {
        for zoom in [1.0, 2.0] {
            let (dom, root) = layout(
                "<div id='parent'><div id='box'>text</div></div>",
                &format!(
                    "#parent{{display:{display};zoom:{zoom};width:100px;height:40px;grid-template-columns:100px;grid-template-rows:40px}}#box{{box-sizing:border-box;width:50px;min-width:40px;max-width:60px;padding:3px;border:2px solid;flex:none}}"
                ),
            );
            let b = geometry(&root, &dom, "box");
            near(b.border_box.w, 50.0 * zoom);
            near(b.content_box.w, 40.0 * zoom);
            near(b.border_box.h, 40.0 * zoom);
        }
    }
}
fn find_mut(root: &mut LayoutBox, target: NodeIdx) -> Option<&mut LayoutBox> {
    if root.node == Some(target) {
        return Some(root);
    }
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &mut root.flow {
        for child in children {
            if let Some(found) = find_mut(child, target) {
                return Some(found);
            }
        }
    }
    for child in &mut root.positioned {
        if let Some(found) = find_mut(child, target) {
            return Some(found);
        }
    }
    None
}
#[test]
fn assigned_sizes_survive_shaping_and_invalidate_retained_geometry_queries() {
    let (dom, mut root) = layout(
        "<div id='parent'><div id='box'>a b c d</div></div>",
        "#parent{display:grid;width:100px;height:60px;grid-template-columns:100px;grid-template-rows:60px}#box{padding:3px;border:2px solid}",
    );
    let id = node(&dom, "box");
    let before = geometry(&root, &dom, "box");
    near(before.border_box.h, 60.0);
    apply_shaped_text(&mut root, &FallbackTextSystem::new());
    resolved_geometry::capture_geometry_snapshot(&mut root);
    let after = geometry(&root, &dom, "box");
    near(after.border_box.h, 60.0);
    find_mut(&mut root, id).unwrap().used_content_size.height = Some(30.0);
    assert!(resolved_style_geometry(&root, LayoutSource::Element(id)).is_none());
}
#[test]
fn declared_border_box_height_limits_and_ratio_keep_selected_box_edges() {
    for (height, min, max, expected) in [
        ("10px", "30px", "none", 30.0),
        ("50px", "0", "30px", 30.0),
        ("5px", "0", "none", 10.0),
        ("auto", "0", "none", 25.0),
    ] {
        let (dom, root) = layout(
            "<div id='box'></div>",
            &format!(
                "#box{{box-sizing:border-box;width:50px;height:{height};min-height:{min};max-height:{max};padding:3px;border:2px solid;aspect-ratio:2;overflow:hidden}}"
            ),
        );
        let b = geometry(&root, &dom, "box");
        near(b.border_box.w, 50.0);
        near(b.border_box.h, expected);
    }
}
#[test]
fn parent_table_content_assignment_measures_real_caption_before_grid_height() {
    let (dom, mut root) = layout(
        "<table id='table'><caption>C</caption><tr><td>X</td></tr></table>",
        "table{width:50px;table-layout:fixed}caption{height:7px;margin:0}td{height:10px;padding:0}",
    );
    let table = find_mut(&mut root, node(&dom, "table")).unwrap();
    block::layout_block_with_used_content_size(
        table,
        0.0,
        0.0,
        100.0,
        &font::FontSource::fallback(),
        Some(&FallbackTextSystem::new()),
        UsedContentSize {
            width: Some(80.0),
            height: Some(50.0),
        },
    );
    near(table.border_box.w, 80.0);
    near(table.border_box.h, 50.0);
}
#[test]
fn column_flex_used_cross_content_and_main_border_box_basis_are_independent() {
    for zoom in [1.0, 2.0] {
        for width in ["auto", "50px"] {
            let (dom, root) = layout(
                "<div id='parent'><div id='box'>A</div></div>",
                &format!(
                    "#parent{{display:flex;flex-direction:column;width:100px;height:60px;zoom:{zoom}}}#box{{box-sizing:border-box;width:{width};height:30px;padding:3px;border:2px solid;flex:none}}"
                ),
            );
            let b = geometry(&root, &dom, "box");
            near(
                b.border_box.w,
                if width == "auto" {
                    100.0 * zoom
                } else {
                    50.0 * zoom
                },
            );
            near(b.border_box.h, 30.0 * zoom);
        }
    }
}
#[test]
fn table_parent_sizing_reads_original_grid_preferred_width_and_percentage_rows() {
    for display in ["flex", "grid"] {
        for zoom in [1.0, 2.0] {
            let (dom, root) = layout(
                "<div id='parent'><table id='table'><thead><tr><td></td></tr></thead><tbody><tr><td></td></tr></tbody></table></div>",
                &format!(
                    "#parent{{display:{display};width:200px;zoom:{zoom}}}table{{border-spacing:0;width:100px}}tr{{height:50px}}td{{height:100%;padding:0}}"
                ),
            );
            let t = geometry(&root, &dom, "table");
            near(t.border_box.w, 100.0 * zoom);
            near(t.border_box.h, 100.0 * zoom);
        }
    }
}
#[test]
fn nested_zoom_table_wrapper_keeps_root_font_unit_context_without_local_zoom() {
    for (parent_zoom, table_zoom, zoom) in [(0.5, 2.0, 1.0), (1.0, 2.0, 2.0), (2.0, 0.5, 1.0)] {
        for margin in ["1rem", "calc(.5rem + 10px)"] {
            let (dom, root) = layout(
                "<div id='parent'><table id='table'><caption>C</caption><tr><td>X</td></tr></table></div>",
                &format!(
                    "html{{font-size:20px}}#parent{{zoom:{parent_zoom}}}#table{{zoom:{table_zoom};width:calc(4rem + 20px);margin-left:{margin}}}caption{{height:7px;margin:0}}"
                ),
            );
            let t = geometry(&root, &dom, "table");
            assert_eq!(t.effective_zoom, zoom as f64);
            near(t.margin.left.unwrap(), 20.0 * zoom);
            near(t.border_box.w, 100.0 * zoom);
        }
    }
}
