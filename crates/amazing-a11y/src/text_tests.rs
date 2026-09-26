//! Accessible text and character-offset fixtures.

use crate::test_support::{ax_of_tag, document};

#[test]
fn paragraph_text_content() {
    let doc = document("<p>Hello world</p>");
    let p = ax_of_tag(&doc, "p").unwrap();
    assert_eq!(doc.tree.text_content(p.id), "Hello world");
}

#[test]
fn inline_runs_join_with_spaces() {
    let doc = document("<p>Hello <em>world</em> again</p>");
    let p = ax_of_tag(&doc, "p").unwrap();
    assert_eq!(doc.tree.text_content(p.id), "Hello world again");
}

#[test]
fn blocks_are_newline_separated() {
    let doc = document("<p>a</p><p>b</p>");
    assert_eq!(doc.tree.text(), "a\nb");
}

#[test]
fn line_break_is_a_newline() {
    let doc = document("<p>a<br>b</p>");
    let p = ax_of_tag(&doc, "p").unwrap();
    assert_eq!(doc.tree.text_content(p.id), "a\nb");
}

#[test]
fn button_text() {
    let doc = document("<button>Save</button>");
    let button = ax_of_tag(&doc, "button").unwrap();
    assert_eq!(doc.tree.text_content(button.id), "Save");
}

#[test]
fn list_text_spans_items() {
    let doc = document("<ul><li>one</li><li>two</li></ul>");
    assert_eq!(doc.tree.text(), "one\ntwo");
}

#[test]
fn text_range_slices_document_text() {
    let doc = document("<p>Hello world</p>");
    let p = ax_of_tag(&doc, "p").unwrap();
    let (start, end) = doc.tree.text_range(p.id).unwrap();
    let text = doc.tree.text();
    assert_eq!(&text[start..end], "Hello world");
}

#[test]
fn text_range_of_an_inner_node_is_contiguous() {
    let doc = document("<p>Hello <em>world</em> again</p>");
    let em = ax_of_tag(&doc, "em").unwrap();
    let (start, end) = doc.tree.text_range(em.id).unwrap();
    let text = doc.tree.text();
    assert!(end > start);
    assert_eq!(&text[start..end], "world");
}

#[test]
fn text_range_is_none_for_a_missing_node() {
    let doc = document("<p>x</p>");
    assert!(doc.tree.text_range(crate::AxNodeId(9999)).is_none());
    assert_eq!(doc.tree.text_content(crate::AxNodeId(9999)), "");
}
