use crate::{Dom, DomError, NodeIdx};

#[test]
fn template_contents_are_persistent_inert_and_outside_light_dom() {
    let mut dom = Dom::new();
    let template = dom.create_element(NodeIdx(0), "template");
    let contents = dom.ensure_template_contents(template).unwrap();
    assert_eq!(dom.ensure_template_contents(template), Ok(contents));
    assert_eq!(dom.template_contents(template), Some(contents));
    assert_eq!(dom.children(template).count(), 0);
    assert_ne!(dom.owner_document(contents), Some(NodeIdx(0)));
    let inert = dom.owner_document(contents).unwrap();
    dom.create_text(contents, "inert");
    assert_eq!(dom.children(template).count(), 0);
    assert_eq!(dom.owner_document(dom.children(contents).next().unwrap()), Some(inert));
    assert_eq!(String::from_utf16(&dom.serialize_node(template).unwrap()).unwrap(),
        "<template>inert</template>");
    assert_eq!(dom.ensure_template_contents(NodeIdx(0)), Err(DomError::TypeError));
}

#[test]
fn clone_and_adopt_copy_or_move_template_contents() {
    let mut dom = Dom::new();
    let template = dom.create_element(NodeIdx(0), "template");
    let contents = dom.ensure_template_contents(template).unwrap();
    dom.create_element(contents, "b");
    let shallow = dom.clone_node(template, false).unwrap();
    let deep = dom.clone_node(template, true).unwrap();
    let shallow_contents = dom.template_contents(shallow).unwrap();
    let deep_contents = dom.template_contents(deep).unwrap();
    assert_ne!(contents, shallow_contents);
    assert_ne!(contents, deep_contents);
    assert_eq!(dom.children(shallow_contents).count(), 0);
    assert_eq!(String::from_utf16(&dom.serialize_node(deep).unwrap()).unwrap(),
        "<template><b></b></template>");
    let new_document = dom.create_document();
    dom.adopt(template, new_document).unwrap();
    assert_eq!(dom.owner_document(template), Some(new_document));
    let new_inert = dom.owner_document(contents).unwrap();
    assert_ne!(new_inert, new_document);
    assert_ne!(new_inert, dom.owner_document(deep_contents).unwrap());
    assert_eq!(dom.owner_document(dom.children(contents).next().unwrap()), Some(new_inert));
    assert_eq!(dom.ensure_template_contents(template), Ok(contents));
}

#[test]
fn import_preserves_template_fragment_and_nested_template_ownership() {
    let mut source = Dom::new();
    let template = source.create_element(NodeIdx(0), "template");
    let contents = source.ensure_template_contents(template).unwrap();
    let nested = source.create_element(contents, "template");
    let nested_contents = source.ensure_template_contents(nested).unwrap();
    source.create_text(nested_contents, "text");
    let mut target = Dom::new();
    let imported = target.import_fragment(&source, NodeIdx(0)).unwrap();
    assert_eq!(String::from_utf16(&target.serialize_children(imported).unwrap()).unwrap(),
        "<template><template>text</template></template>");
    let outer = target.children(imported).next().unwrap();
    let inner = target.children(target.template_contents(outer).unwrap()).next().unwrap();
    let outer_doc = target.owner_document(target.template_contents(outer).unwrap()).unwrap();
    assert_eq!(target.owner_document(target.template_contents(inner).unwrap()), Some(outer_doc));
}

#[test]
fn template_owner_inherits_only_document_type_and_is_shared() {
    use crate::document_surface::{DocumentMetadata, QuirksMode};
    let mut dom = Dom::new();
    dom.set_document_metadata(NodeIdx(0), DocumentMetadata {
        url: "https://example.test/active".into(), content_type: "text/html".into(),
        character_set: "Shift_JIS".into(),
    }).unwrap();
    dom.set_document_scripting_enabled(NodeIdx(0), true).unwrap();
    dom.set_document_quirks_mode(NodeIdx(0), QuirksMode::Quirks).unwrap();
    let first = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "template").unwrap();
    let fragment = dom.ensure_template_contents(first).unwrap();
    let inert = dom.owner_document(fragment).unwrap();
    let metadata = dom.document_metadata(inert).unwrap().unwrap();
    assert_eq!(metadata.url, "about:blank");
    assert_eq!(metadata.character_set, "UTF-8");
    assert_eq!(metadata.content_type, "text/html");
    assert!(!dom.document_scripting_enabled(inert).unwrap());
    assert_eq!(dom.document_quirks_mode(inert), Ok(QuirksMode::NoQuirks));
    let second = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "template").unwrap();
    let second_contents = dom.ensure_template_contents(second).unwrap();
    assert_eq!(dom.owner_document(second_contents), Some(inert));
    let nested = dom.create_element(fragment, "template");
    let nested_contents = dom.ensure_template_contents(nested).unwrap();
    assert_eq!(dom.owner_document(nested_contents), Some(inert));
    let clone = dom.clone_node(first, true).unwrap();
    assert_eq!(dom.owner_document(dom.template_contents(clone).unwrap()), Some(inert));
}

#[test]
fn template_in_xml_document_has_xml_inert_owner_even_at_root_slot() {
    use crate::document_surface::DocumentMetadata;
    let mut dom = Dom::new();
    dom.set_document_metadata(NodeIdx(0), DocumentMetadata {
        url: "https://example.test/document.xml".into(), content_type: "application/xhtml+xml".into(),
        character_set: "UTF-16".into(),
    }).unwrap();
    let template = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "template").unwrap();
    let fragment = dom.ensure_template_contents(template).unwrap();
    let inert = dom.owner_document(fragment).unwrap();
    let metadata = dom.document_metadata(inert).unwrap().unwrap();
    assert_eq!(metadata.content_type, "application/xml");
    assert_eq!(metadata.url, "about:blank");
    assert_eq!(metadata.character_set, "UTF-8");
}

#[test]
fn template_associations_do_not_alias_recycled_node_slots() {
    let mut dom = Dom::new();
    let document = dom.create_document();
    let template = dom.create_element_ns_in(document, Some("http://www.w3.org/1999/xhtml"), "template").unwrap();
    let contents = dom.ensure_template_contents(template).unwrap();
    let inert = dom.owner_document(contents).unwrap();
    // The complete unreachable association is retired together; none of these
    // slots retains a script wrapper, child or external DOM reference.
    for node in [contents, template, inert, document] { dom.retire_unreachable_node(node).unwrap(); }
    let replacement_document = dom.create_document();
    assert_eq!(replacement_document, document);
    let replacement = dom.create_element_ns_in(replacement_document, Some("http://www.w3.org/1999/xhtml"), "template").unwrap();
    assert_eq!(dom.template_contents(replacement), None);
    let new_contents = dom.ensure_template_contents(replacement).unwrap();
    assert_eq!(dom.template_of_contents(new_contents), Some(replacement));
    let new_inert = dom.owner_document(new_contents).unwrap();
    assert_ne!(new_inert, replacement_document);
    assert!(matches!(dom.get(new_inert).kind, crate::NodeKind::Document));
    assert!(!dom.document_scripting_enabled(new_inert).unwrap());
}
