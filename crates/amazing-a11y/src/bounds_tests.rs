//! Bounding-box fixtures: blocks, inline unions and the viewport root.

use crate::Role;
use crate::test_support::{ax_of_tag, document, document_with_css, first_role};

#[test]
fn button_bounds_are_positive() {
    let doc = document_with_css(
        "<button class=\"b\">Go</button>",
        ".b{width:120px;height:40px}",
    );
    let button = ax_of_tag(&doc, "button").unwrap();
    let rect = button.bounds.expect("button has a border box");
    assert!(rect.width > 0.0 && rect.height > 0.0, "{rect:?}");
}

#[test]
fn stacked_blocks_increase_y() {
    let doc = document_with_css(
        "<p class=\"b\">A</p><p class=\"b\">B</p>",
        ".b{height:20px;margin:0}",
    );
    let paragraphs: Vec<_> = doc
        .tree
        .nodes()
        .iter()
        .filter(|node| node.role == Role::Paragraph)
        .collect();
    assert_eq!(paragraphs.len(), 2);
    let first = paragraphs[0].bounds.unwrap();
    let second = paragraphs[1].bounds.unwrap();
    assert!(second.y >= first.y + first.height - 0.5, "{first:?} {second:?}");
}

#[test]
fn inline_element_union_of_its_text() {
    let doc = document("<p>Hello <em>world</em> again</p>");
    let em = ax_of_tag(&doc, "em").unwrap();
    let rect = em.bounds.expect("inline element inherits its text rects");
    assert!(rect.width > 0.0 && rect.height > 0.0, "{rect:?}");
}

#[test]
fn text_node_has_bounds() {
    let doc = document("<p>Hi</p>");
    let text = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::StaticText)
        .unwrap();
    assert!(text.bounds.is_some());
}

#[test]
fn document_root_spans_the_viewport() {
    let doc = document_with_css("<p>x</p>", "p{height:100px}");
    let root = doc.tree.root().unwrap();
    let rect = doc.tree.node(root).unwrap().bounds.expect("html has bounds");
    assert!((700.0..=900.0).contains(&rect.width), "viewport width {rect:?}");
}

#[test]
fn out_of_flow_box_is_positioned_absolutely() {
    let doc = document_with_css(
        "<p class=\"spacer\">spacer</p><div class=\"abs\" role=\"group\">abs</div>",
        ".spacer{height:200px;margin:0}.abs{position:absolute;top:10px;left:15px;width:50px;height:25px}",
    );
    let group = first_role(&doc, Role::Group).unwrap();
    let rect = group.bounds.expect("absolutely positioned box has bounds");
    assert!((rect.x - 15.0).abs() < 1.0, "{rect:?}");
    assert!((rect.y - 10.0).abs() < 1.0, "{rect:?}");
}

#[test]
fn zero_sized_boxes_are_still_reported() {
    let doc = document_with_css("<div role=\"group\" class=\"z\"></div>", ".z{width:0;height:0}");
    let group = first_role(&doc, Role::Group).unwrap();
    let rect = group.bounds.expect("empty box still has a rect");
    assert!(rect.is_empty(), "{rect:?}");
}
