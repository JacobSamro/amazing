use crate::DomError;
use crate::{Dom, NodeIdx, NodeKind};
use crate::markup::IncrementalDocumentImport;
use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};

fn markup(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}

#[test]
fn html_fragment_serialization_escapes_text_and_attributes_but_not_raw_text() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let img = dom.create_element(root, "img");
    dom.set_attribute(img, "alt", "a&\"b").unwrap();
    dom.append_child(root, NodeKind::Text { data: "<&>\u{a0}".into() });
    let script = dom.create_element(root, "script");
    dom.append_child(script, NodeKind::Text { data: "if (a < b && c > d) x()".into() });
    assert_eq!(markup(&dom, root), "<img alt=\"a&amp;&quot;b\">&lt;&amp;&gt;&nbsp;<script>if (a < b && c > d) x()</script>");
}

#[test]
fn foreign_namespaces_and_template_children_serialize() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let svg = dom.create_element_ns(root, Some("http://www.w3.org/2000/svg"), "svg").unwrap();
    let path = dom.create_element_ns(svg, Some("http://www.w3.org/2000/svg"), "path").unwrap();
    dom.set_attribute_qualified_ns(path, Some("http://www.w3.org/1999/xlink"), "xlink:href", "a&b").unwrap();
    let math = dom.create_element_ns(root, Some("http://www.w3.org/1998/Math/MathML"), "math").unwrap();
    dom.create_element_ns(math, Some("http://www.w3.org/1998/Math/MathML"), "mi").unwrap();
    let template = dom.create_element(root, "template");
    let contents = dom.ensure_template_contents(template).unwrap();
    dom.create_element(contents, "br");
    assert_eq!(markup(&dom, root), "<svg><path xlink:href=\"a&amp;b\"></path></svg><math><mi></mi></math><template><br></template>");
}

#[test]
fn parser_arena_import_preserves_namespace_and_tree_order() {
    let mut source = Dom::new();
    let foreign = source.create_element_ns(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "svg").unwrap();
    source.create_element_ns(foreign, Some("http://www.w3.org/2000/svg"), "circle").unwrap();
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let fragment = dom.import_fragment(&source, NodeIdx(0)).unwrap();
    dom.replace_all(root, Some(fragment)).unwrap();
    assert_eq!(markup(&dom, root), "<svg><circle></circle></svg>");
}

#[test]
fn inert_document_import_preserves_own_tree_and_metadata() {
    let mut parsed = Dom::new();
    let root = parsed.create_element_ns(NodeIdx(0), Some("urn:test"), "p:root").unwrap();
    parsed.set_attribute_qualified_ns(root, Some("urn:attr"), "a:id", "one").unwrap();
    parsed.create_element_ns(root, Some("urn:test"), "child").unwrap();
    let mut dom = Dom::new();
    let inert = dom.import_inert_document(&parsed, "https://example.test/data.xml",
        "application/xml", "UTF-8").unwrap();
    let imported = dom.document_element(inert).unwrap().unwrap();
    assert_eq!(dom.owner_document(imported), Some(inert));
    assert_eq!(dom.namespace_uri(imported), Some("urn:test"));
    assert_eq!(dom.attribute_ns(imported, Some("urn:attr"), "id").unwrap().value, "one");
    assert_eq!(dom.document_metadata(inert).unwrap().unwrap().url, "https://example.test/data.xml");
    assert_eq!(dom.document_metadata(inert).unwrap().unwrap().content_type, "application/xml");
    assert_eq!(dom.document_metadata(inert).unwrap().unwrap().character_set, "UTF-8");
    assert_eq!(dom.document_ready_state(inert).unwrap().as_str(), "complete");
    assert!(dom.document_metadata(NodeIdx(0)).unwrap().is_none());
}

#[test]
fn serialization_preserves_utf16_text_units() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let text = dom.append_child(root, NodeKind::Text { data: String::new() });
    dom.replace_character_data(text, 0, 0, &[0xD800, b'&' as u16]).unwrap();
    assert_eq!(dom.serialize_children(root).unwrap(), vec![0xD800, b'&' as u16, b'a' as u16,
        b'm' as u16, b'p' as u16, b';' as u16]);
}

// HTML §13.3 at cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d and
// shadow-dom/declarative WPT at c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
#[test]
fn declarative_shadow_serialization_selects_roots_and_preserves_template_flags() {
    let mut dom = Dom::new();
    let container = dom.create_element(NodeIdx(0), "main");
    let host = dom.create_element(container, "div");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: true,
        slot_assignment: SlotAssignment::Manual, clonable: true, serializable: true };
    let root = dom.attach_declarative_shadow(host, options, true).unwrap();
    assert_eq!(dom.attach_declarative_shadow(host, options, false),
        Err(crate::DomError::NotSupportedError));
    let shadow_text = dom.create_text(root, "shadow");
    assert_eq!(dom.get(shadow_text).parent, Some(root));
    dom.create_element(host, "span");
    assert_eq!(markup(&dom, container), "<div><span></span></div>");
    let serialized = String::from_utf16(&dom.serialize_children_with_shadow(container, true, &[]).unwrap()).unwrap();
    assert_eq!(serialized, "<div><template shadowrootmode=\"closed\" shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\">shadow</template><span></span></div>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, true, &[]).unwrap()).unwrap(),
        "<template shadowrootmode=\"closed\" shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\">shadow</template><span></span>");
    let copy = dom.clone_node(host, false).unwrap();
    let copy_root = dom.shadow_root_of_host(copy).unwrap();
    assert!(dom.shadow_data(copy_root).unwrap().declarative);
    assert!(dom.shadow_data(copy_root).unwrap().keep_custom_element_registry_null);
    assert_eq!(String::from_utf16(&dom.serialize_children(copy_root).unwrap()).unwrap(), "shadow");
    let wrong_mode = ShadowOptions { mode: ShadowMode::Open, ..options };
    assert_eq!(dom.attach_shadow(host, wrong_mode), Err(crate::DomError::NotSupportedError));
    assert!(dom.shadow_data(root).unwrap().declarative);
    assert_eq!(dom.children(root).count(), 1);
    assert_eq!(dom.attach_shadow(host, options), Ok(root));
    assert!(!dom.shadow_data(root).unwrap().declarative);
    assert_eq!(dom.children(root).count(), 0);
}

#[test]
fn explicit_shadow_roots_include_nonserializable_closed_roots() {
    let mut dom = Dom::new();
    let container = dom.create_element(NodeIdx(0), "main");
    let host = dom.create_element(container, "div");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: false, serializable: false };
    let root = dom.attach_declarative_shadow(host, options, false).unwrap();
    dom.create_text(root, "inside");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(container, true, &[]).unwrap()).unwrap(), "<div></div>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(container, false, &[root]).unwrap()).unwrap(),
        "<div><template shadowrootmode=\"closed\">inside</template></div>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, false, &[root]).unwrap()).unwrap(),
        "<template shadowrootmode=\"closed\">inside</template>");
}

#[test]
fn parser_fragment_import_keeps_declarative_shadow_contents() {
    let mut source = Dom::new();
    let host = source.create_element(NodeIdx(0), "div");
    let options = ShadowOptions { mode: ShadowMode::Open, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: true, serializable: true };
    let root = source.attach_declarative_shadow(host, options, false).unwrap();
    source.create_element(root, "slot");
    let mut target = Dom::new();
    let fragment = target.import_fragment(&source, NodeIdx(0)).unwrap();
    let imported_host = target.children(fragment).next().unwrap();
    let imported_root = target.shadow_root_of_host(imported_host).unwrap();
    assert!(target.shadow_data(imported_root).unwrap().declarative);
    assert_eq!(String::from_utf16(&target.serialize_children_with_shadow(fragment, true, &[]).unwrap()).unwrap(),
        "<div><template shadowrootmode=\"open\" shadowrootserializable=\"\" shadowrootclonable=\"\"><slot></slot></template></div>");
}

#[test]
fn document_import_clones_into_target_with_clonable_shadow() {
    let mut dom = Dom::new();
    let other = dom.create_document();
    let host = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(host, "span");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: true, serializable: true };
    let root = dom.attach_declarative_shadow(host, options, true).unwrap();
    dom.create_text(root, "shadow");
    let copied = dom.import_node(other, host, true).unwrap();
    let copied_child = dom.children(copied).next().unwrap();
    let copied_root = dom.shadow_root_of_host(copied).unwrap();
    assert_ne!(copied, host);
    assert_ne!(copied_child, child);
    assert_eq!(dom.owner_document(copied), Some(other));
    assert_eq!(dom.owner_document(copied_child), Some(other));
    assert_eq!(dom.owner_document(copied_root), Some(other));
    assert_eq!(dom.owner_document(dom.children(copied_root).next().unwrap()), Some(other));
    assert!(dom.shadow_data(copied_root).unwrap().declarative);
    assert!(dom.shadow_data(copied_root).unwrap().keep_custom_element_registry_null);
    assert_eq!(dom.owner_document(host), Some(NodeIdx(0)));
    assert_eq!(dom.import_node(other, NodeIdx(0), false), Err(crate::DomError::NotSupportedError));
    assert_eq!(dom.import_node(other, root, true), Err(crate::DomError::NotSupportedError));
    assert_eq!(dom.adopt(root, other), Err(crate::DomError::HierarchyRequestError));
}

// HTML-34: `IncrementalDocumentImport` grafts a script-interleaved parser's
// private arena into an existing document node incrementally, preserving
// already-imported nodes' destination identity -- see markup.rs's doc
// comment for why this exists (the iframe counterpart of HTML-31/32's
// top-level `swap_dom` interleaving).

#[test]
fn incremental_import_preserves_identity_of_already_imported_nodes_across_syncs() {
    let mut source = Dom::new();
    let html = source.create_element(NodeIdx(0), "html");
    let body = source.create_element(html, "body");
    let first = source.create_element(body, "p");
    source.create_text(first, "one");

    let mut dest = Dom::new();
    let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document);
    import.sync(&mut dest, &source).unwrap();
    let dest_first = import.imported(first).expect("first <p> imported");
    assert_eq!(markup(&dest, document), "<html><body><p>one</p></body></html>");

    // A still-open element (here, `body`) gains a sibling of `first` after
    // the "pause" above -- the incremental import must pick it up on the
    // next `sync` without disturbing `first`'s already-assigned identity.
    let second = source.create_element(body, "p");
    source.create_text(second, "two");
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(import.imported(first), Some(dest_first), "already-imported node kept its identity");
    let dest_second = import.imported(second).expect("second <p> imported");
    assert_ne!(dest_first, dest_second);
    assert_eq!(markup(&dest, document), "<html><body><p>one</p><p>two</p></body></html>");

    // And a THIRD sync with nothing new is a clean no-op.
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(markup(&dest, document), "<html><body><p>one</p><p>two</p></body></html>");
}

#[test]
fn incremental_import_discovers_new_grandchildren_of_an_already_open_node() {
    let mut source = Dom::new();
    let div = source.create_element(NodeIdx(0), "div");
    let mut dest = Dom::new();
    let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document);
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(markup(&dest, document), "<div></div>");

    // `div` is still open (no closing tag has been seen by a real parser
    // yet); a real interleaved parse would keep appending to it across
    // several pauses before it closes. Each new grandchild must still be
    // found even though `div` itself was already imported.
    let span = source.create_element(div, "span");
    source.create_text(span, "x");
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(markup(&dest, document), "<div><span>x</span></div>");

    source.create_text(span, "y");
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(markup(&dest, document), "<div><span>xy</span></div>");
}

#[test]
fn incremental_import_carries_shadow_roots_and_template_contents() {
    // A `Document` node allows only one `Element` child (the document
    // element) -- both `host` and `template` nest under it, matching what a
    // real interleaved parse would actually produce (never two independent
    // top-level elements), unlike an early version of this test.
    let mut source = Dom::new();
    let container = source.create_element(NodeIdx(0), "div");
    let host = source.create_element(container, "div");
    let options = ShadowOptions { mode: ShadowMode::Open, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: false, serializable: true };
    let root = source.attach_declarative_shadow(host, options, false).unwrap();
    let template = source.create_element(container, "template");
    let contents = source.ensure_template_contents(template).unwrap();

    let mut dest = Dom::new();
    let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document);
    import.sync(&mut dest, &source).unwrap();
    let dest_host = import.imported(host).unwrap();
    let dest_root = dest.shadow_root_of_host(dest_host).unwrap();
    assert!(dest.shadow_data(dest_root).unwrap().declarative);

    // A pause with the shadow root/template already opened but still empty,
    // then more content arriving inside each on the next sync -- both are
    // tracked as ordinary containers, not just copied once at creation time.
    source.create_text(root, "shadow");
    let dest_template = import.imported(template).unwrap();
    source.create_element(contents, "br");
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(String::from_utf16(&dest.serialize_children(dest_root).unwrap()).unwrap(), "shadow");
    let dest_contents = dest.template_contents(dest_template).unwrap();
    assert_eq!(markup(&dest, dest_contents), "<br>");
}

#[test]
fn incremental_import_mirrors_external_document_and_preserves_adopted_identity() {
    let mut source = Dom::new(); let root = source.create_element(NodeIdx(0), "html");
    let body = source.create_element(root, "body"); let formatting = source.create_element(body, "b");
    let block = source.create_element(formatting, "div");
    let mut dest = Dom::new(); let document = dest.create_document();
    let other = dest.create_document(); dest.set_document_scripting_enabled(other, true).unwrap();
    let container = dest.create_element(other, "other");
    let mut import = IncrementalDocumentImport::new(document); import.sync(&mut dest, &source).unwrap();
    let live = import.imported(block).unwrap(); dest.pre_insert(live, container, None).unwrap();
    import.reverse_sync(&mut source, &dest).unwrap();
    let mirrored_document = source.owner_document(block).unwrap();
    assert_ne!(mirrored_document, NodeIdx(0));
    assert!(source.document_scripting_enabled(mirrored_document).unwrap());
    assert_eq!(source.local_name(source.get(block).parent.unwrap()), Some("other"));
    // The parser appends below an open node that a script adopted away.
    let child = source.create_element(block, "span");
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(dest.get(live).parent, Some(container));
    assert_eq!(dest.owner_document(import.imported(child).unwrap()), Some(other));
    // A later validated parser move comes back into its original document.
    source.pre_insert(block, body, None).unwrap(); import.sync(&mut dest, &source).unwrap();
    assert_eq!(import.imported(block), Some(live));
    assert_eq!(dest.get(live).parent, import.imported(body));
    assert_eq!(dest.owner_document(live), Some(document));
}

#[test]
fn incremental_import_mirrors_live_document_replacement_and_sibling_order() {
    let mut source = Dom::new(); let old = source.create_element(NodeIdx(0), "old");
    let mut dest = Dom::new(); let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document); import.sync(&mut dest, &source).unwrap();
    dest.detach(import.imported(old).unwrap());
    let replacement = dest.create_element(document, "replacement");
    let comment = dest.create_node_in(document, NodeKind::Comment { data: "before".into() }).unwrap();
    dest.pre_insert(comment, document, Some(replacement)).unwrap();
    import.reverse_sync(&mut source, &dest).unwrap();
    let children: Vec<_> = source.children(NodeIdx(0)).collect();
    assert_eq!(children.len(), 2); assert!(matches!(source.get(children[0]).kind, NodeKind::Comment { .. }));
    assert_eq!(source.document_element(NodeIdx(0)).unwrap(), Some(children[1]));
    let rejected = source.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "second").unwrap();
    assert_eq!(source.pre_insert(rejected, NodeIdx(0), None), Err(DomError::HierarchyRequestError));
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(dest.document_element(document).unwrap(), Some(replacement));
}

#[test]
fn incremental_import_retries_ancestor_reversal_and_reorders_siblings() {
    let mut source = Dom::new(); let root = source.create_element(NodeIdx(0), "root");
    let a = source.create_element(root, "a"); let b = source.create_element(a, "b");
    let c = source.create_element(root, "c");
    let mut dest = Dom::new(); let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document); import.sync(&mut dest, &source).unwrap();
    source.pre_insert(b, root, Some(c)).unwrap(); source.pre_insert(a, b, None).unwrap();
    source.pre_insert(c, root, Some(b)).unwrap(); import.sync(&mut dest, &source).unwrap();
    assert_eq!(markup(&dest, document), "<root><c></c><b><a></a></b></root>");
    let da = import.imported(a).unwrap(); let db = import.imported(b).unwrap(); let dr = import.imported(root).unwrap();
    dest.pre_insert(da, dr, None).unwrap(); dest.pre_insert(db, da, None).unwrap();
    import.reverse_sync(&mut source, &dest).unwrap();
    assert_eq!(source.get(b).parent, Some(a));
    assert_eq!(markup(&source, NodeIdx(0)), "<root><c></c><a><b></b></a></root>");
}

#[test]
fn incremental_import_preserves_detached_fragment_children() {
    let mut source = Dom::new(); let root = source.create_element(NodeIdx(0), "root");
    let open = source.create_element(root, "open");
    let mut dest = Dom::new(); let document = dest.create_document();
    let fragment = dest.create_document_fragment(document).unwrap();
    let mut import = IncrementalDocumentImport::new(document); import.sync(&mut dest, &source).unwrap();
    let live = import.imported(open).unwrap(); dest.pre_insert(live, fragment, None).unwrap();
    import.reverse_sync(&mut source, &dest).unwrap();
    let source_fragment = source.get(open).parent.unwrap();
    assert!(matches!(source.get(source_fragment).kind, NodeKind::DocumentFragment));
    source.create_text(open, "continued"); import.sync(&mut dest, &source).unwrap();
    assert_eq!(dest.get(live).parent, Some(fragment));
    assert_eq!(markup(&dest, fragment), "<open>continued</open>");
}

#[test]
fn incremental_import_replaces_root_without_transient_document_full_error() {
    let mut source = Dom::new(); let old = source.create_element(NodeIdx(0), "old");
    let open = source.create_element(old, "open");
    let mut dest = Dom::new(); let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document); import.sync(&mut dest, &source).unwrap();
    let identity = import.imported(open).unwrap();
    source.detach(old); let new = source.create_element(NodeIdx(0), "new");
    source.pre_insert(open, new, None).unwrap();
    import.sync(&mut dest, &source).unwrap();
    assert_eq!(import.imported(open), Some(identity));
    assert_eq!(markup(&dest, document), "<new><open></open></new>");
    assert!(dest.get(import.imported(old).unwrap()).parent.is_none());
}

#[test]
fn incremental_import_discovers_shadow_root_added_after_host_was_imported() {
    let mut source = Dom::new(); let host = source.create_element(NodeIdx(0), "div");
    let mut dest = Dom::new(); let document = dest.create_document();
    let mut import = IncrementalDocumentImport::new(document); import.sync(&mut dest, &source).unwrap();
    let options = ShadowOptions { mode: ShadowMode::Open, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: true, serializable: true };
    let root = source.attach_declarative_shadow(host, options, false).unwrap();
    source.create_text(root, "later"); import.sync(&mut dest, &source).unwrap();
    let live_root = dest.shadow_root_of_host(import.imported(host).unwrap()).unwrap();
    assert_eq!(import.imported(root), Some(live_root));
    assert!(dest.shadow_data(live_root).unwrap().declarative);
    assert_eq!(markup(&dest, live_root), "later");
}

#[test]
fn incremental_values_round_trip_preserves_parser_appends_and_live_ranges() {
    use crate::range::BoundaryPoint;
    use crate::mutation::ObserveOptions;
    let mut source=Dom::new();let root=source.create_element(NodeIdx(0),"p");
    let text=source.create_text(root,"a");
    let mut dest=Dom::new();let document=dest.create_document();
    let mut import=IncrementalDocumentImport::new(document);import.sync(&mut dest,&source).unwrap();
    let live=import.imported(text).unwrap();let live_root=import.imported(root).unwrap();
    let range=dest.create_range(document).unwrap();
    dest.range_set_start(range,BoundaryPoint::new(live,1)).unwrap();
    dest.range_set_end(range,BoundaryPoint::new(live,1)).unwrap();
    let observer=dest.create_mutation_observer(1);
    dest.observe(observer,live,ObserveOptions{character_data:Some(true),character_data_old_value:Some(true),..Default::default()}).unwrap();
    source.append_text_data(text,"b");import.sync(&mut dest,&source).unwrap();
    assert_eq!(dest.character_data_utf16(live).unwrap(),vec![97,98]);
    assert_eq!(dest.range(range).unwrap().start,BoundaryPoint::new(live,1));
    let records=dest.take_mutation_records(observer);assert_eq!(records.len(),1);
    assert_eq!(records[0].old_value_utf16,Some(vec![97]));
    dest.replace_character_data(live,0,1,&[0xd800]).unwrap();
    dest.set_attribute(live_root,"data-js","kept").unwrap();
    import.reverse_sync(&mut source,&dest).unwrap();
    assert_eq!(source.character_data_utf16(text).unwrap(),vec![0xd800,98]);
    assert_eq!(source.get_attribute_ns(root,None,"data-js"),Some("kept"));
    source.append_text_data(text,"c");import.sync(&mut dest,&source).unwrap();
    assert_eq!(dest.character_data_utf16(live).unwrap(),vec![0xd800,98,99]);
    assert_eq!(import.imported(text),Some(live));
    dest.take_mutation_records(observer);
    import.reverse_sync(&mut source,&dest).unwrap();import.sync(&mut dest,&source).unwrap();
    assert!(dest.take_mutation_records(observer).is_empty(),"unchanged values must not mutate again");
}

#[test]
fn incremental_attribute_values_preserve_expanded_identity_and_exact_units() {
    let mut source=Dom::new();let root=source.create_element(NodeIdx(0),"div");
    source.set_attribute_expanded(root,Some("urn:attr"),Some("p"),"value","initial").unwrap();
    source.set_attribute(root,"remove","old").unwrap();
    let attr=source.get_attribute_node(root,Some("urn:attr"),"value").unwrap();
    source.set_attr_value(attr,vec![0xd800,65]).unwrap();
    let mut dest=Dom::new();dest.create_element_ns_in(NodeIdx(0),Some("urn:unrelated"),"x:other").unwrap();
    let document=dest.create_document();let mut import=IncrementalDocumentImport::new(document);
    import.sync(&mut dest,&source).unwrap();let live=import.imported(root).unwrap();
    let identity=dest.get_attribute_node(live,Some("urn:attr"),"value").unwrap();
    assert_eq!(dest.attr_snapshot(identity).unwrap().value,vec![0xd800,65]);
    assert_eq!(dest.attr_snapshot(identity).unwrap().prefix.as_deref(),Some("p"));
    source.set_attr_value(attr,vec![0xdc00,66]).unwrap();source.remove_attribute(root,"remove").unwrap();
    source.set_attribute_expanded(root,Some("urn:second"),Some("q"),"value","added").unwrap();
    import.sync(&mut dest,&source).unwrap();
    assert_eq!(dest.get_attribute_node(live,Some("urn:attr"),"value"),Some(identity));
    assert_eq!(dest.attr_snapshot(identity).unwrap().value,vec![0xdc00,66]);
    assert_eq!(dest.get_attribute_ns(live,None,"remove"),None);
    assert_eq!(dest.attribute_ns(live,Some("urn:second"),"value").unwrap().prefix,Some("q"));
    dest.set_attr_value(identity,vec![0xd800,0xdc00,67]).unwrap();
    dest.remove_attribute_ns(live,Some("urn:second"),"value").unwrap();
    import.reverse_sync(&mut source,&dest).unwrap();
    assert_eq!(source.get_attribute_node(root,Some("urn:attr"),"value"),Some(attr));
    assert_eq!(source.attr_snapshot(attr).unwrap().value,vec![0xd800,0xdc00,67]);
    assert_eq!(source.attribute_ns(root,Some("urn:second"),"value"),None);
}

#[test]
fn incremental_character_data_and_special_containers_keep_raw_units() {
    let mut source=Dom::new();let root=source.create_element(NodeIdx(0),"div");
    let text=source.create_text(root,"text");let comment=source.create_comment(root,"comment");
    let cdata=source.create_node_in(NodeIdx(0),NodeKind::CDATASection{data:"cdata".into()}).unwrap();
    source.pre_insert(cdata,root,None).unwrap();
    let pi=source.create_processing_instruction_in(NodeIdx(0),"pi",&[0xd800]).unwrap();source.pre_insert(pi,root,None).unwrap();
    let template=source.create_element(root,"template");let contents=source.ensure_template_contents(template).unwrap();
    let inert=source.create_text(contents,"a");
    let shadow=source.attach_shadow(root,ShadowOptions{mode:ShadowMode::Closed,delegates_focus:false,
        slot_assignment:SlotAssignment::Named,clonable:true,serializable:true}).unwrap();
    let shadow_text=source.create_text(shadow,"s");
    let nodes=[text,comment,cdata,pi,inert,shadow_text];
    let mut dest=Dom::new();let document=dest.create_document();let mut import=IncrementalDocumentImport::new(document);
    import.sync(&mut dest,&source).unwrap();
    assert_eq!(dest.character_data_utf16(import.imported(pi).unwrap()).unwrap(),vec![0xd800]);
    for node in nodes {source.replace_character_data(node,0,source.node_length(node),&[0xdc00,88]).unwrap();}
    import.sync(&mut dest,&source).unwrap();
    for node in nodes {
        let live=import.imported(node).unwrap();assert_eq!(dest.character_data_utf16(live).unwrap(),vec![0xdc00,88]);
        dest.replace_character_data(live,0,2,&[89,0xd800]).unwrap();
    }
    import.reverse_sync(&mut source,&dest).unwrap();
    for node in nodes {assert_eq!(source.character_data_utf16(node).unwrap(),vec![89,0xd800]);}
}
