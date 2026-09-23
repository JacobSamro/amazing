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
