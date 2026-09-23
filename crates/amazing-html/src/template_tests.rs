use amazing_dom::{Dom, Namespace, NodeIdx};
use crate::{parse, parse_fragment, IncrementalParser};

fn templates(dom: &Dom, root: NodeIdx, found: &mut Vec<NodeIdx>) {
    for child in dom.children(root) {
        if dom.namespace(child) == Some(Namespace::Html) && dom.name(child) == Some("template") {
            found.push(child);
            templates(dom, dom.template_contents(child).expect("template contents"), found);
        } else { templates(dom, child, found); }
    }
}
fn markup(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}

#[test]
fn parsed_template_content_is_inert_and_serializes() {
    let mut dom = parse("<body><template><p>A</p>tail</template><p>B</p>");
    let mut found = Vec::new();
    templates(&dom, NodeIdx(0), &mut found);
    assert_eq!(found.len(), 1);
    let template = found[0];
    assert_eq!(dom.children(template).count(), 0);
    let content = dom.template_contents(template).unwrap();
    assert_eq!(markup(&dom, content), "<p>A</p>tail");
    assert_ne!(dom.owner_document(content), dom.owner_document(template));
    let clone = dom.clone_node(template, true).unwrap();
    assert_eq!(markup(&dom, dom.template_contents(clone).unwrap()), "<p>A</p>tail");
    let new_document = dom.create_document();
    dom.adopt(template, new_document).unwrap();
    assert_eq!(markup(&dom, content), "<p>A</p>tail");
    assert_ne!(dom.owner_document(content), Some(new_document));
}

#[test]
fn nested_templates_and_eof_close_without_light_children() {
    for source in ["<template><template><span>deep</span></template><b>outer</b></template>",
                   "<template><p>A"] {
        let dom = parse(source);
        let mut found = Vec::new();
        templates(&dom, NodeIdx(0), &mut found);
        assert!(found.iter().all(|&t| dom.children(t).count() == 0));
        assert!(markup(&dom, NodeIdx(0)).contains("<template>"));
        if source.contains("deep") { assert_eq!(found.len(), 2); }
        else { assert_eq!(found.len(), 1); }
    }
}

#[test]
fn fragment_template_context_and_table_foster_parenting() {
    let fragment = parse_fragment("template", "<p>A</p><table><tr><td>x</td></tr></table>").unwrap();
    assert!(markup(&fragment, NodeIdx(0)).contains("<p>A</p><table>"));
    let dom = parse("<table><template><div>A</div><tr><td>x</td></tr></template><tr><td>after</td></tr></table>");
    let mut found = Vec::new();
    templates(&dom, NodeIdx(0), &mut found);
    assert_eq!(found.len(), 1);
    assert!(markup(&dom, dom.template_contents(found[0]).unwrap()).contains("<div>A</div>"));
}

#[test]
fn incremental_template_matches_one_shot() {
    let source = "<template><p>A</p><template><b>B</b></template></template><p>C</p>";
    let mut incremental = IncrementalParser::new();
    for chunk in source.as_bytes().chunks(3) {
        incremental.push_chunk(std::str::from_utf8(chunk).unwrap());
    }
    assert_eq!(markup(&incremental.finish(), NodeIdx(0)), markup(&parse(source), NodeIdx(0)));
}
