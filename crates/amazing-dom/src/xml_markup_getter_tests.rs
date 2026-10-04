//! Getter-specific fragment boundaries and strict native serialization states.
use super::*;
fn output(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_xml_children(node).unwrap()).unwrap()
}
#[test]
fn xml_getter_children_exclude_receiver_and_share_sibling_prefix_counter() {
    let mut dom = Dom::new();
    let parent = dom.create_element_ns_in(NodeIdx(0), Some("urn:excluded"), "p:parent").unwrap();
    for ns in ["urn:first", "urn:second"] {
        let child = dom.create_element_ns_in(NodeIdx(0), None, "child").unwrap();
        dom.set_attribute_qualified_ns(child, Some(ns), "value", "&\t").unwrap();
        dom.pre_insert(child, parent, None).unwrap();
    }
    assert_eq!(output(&dom, parent), "<child xmlns:ns1=\"urn:first\" ns1:value=\"&amp;&#9;\"/><child xmlns:ns2=\"urn:second\" ns2:value=\"&amp;&#9;\"/>");
    assert!(!output(&dom, parent).contains("urn:excluded"));
    assert!(String::from_utf16(&dom.serialize_xml(parent, true).unwrap()).unwrap().starts_with("<p:parent xmlns:p=\"urn:excluded\""));
}
#[test]
fn xml_getter_default_undeclaration_templates_and_real_shadow_root() {
    let mut dom = Dom::new();
    let parent = dom.create_element_ns_in(NodeIdx(0), Some("urn:p"), "p:parent").unwrap();
    dom.set_attribute_qualified_ns(parent, Some(XMLNS), "xmlns", "").unwrap();
    let child = dom.create_element_ns_in(NodeIdx(0), None, "child").unwrap();
    dom.pre_insert(child, parent, None).unwrap();
    assert_eq!(String::from_utf16(&dom.serialize_xml(parent,true).unwrap()).unwrap(), "<p:parent xmlns:p=\"urn:p\" xmlns=\"\"><child/></p:parent>");
    let template = dom.create_element_ns_in(NodeIdx(0), Some(HTML), "template").unwrap();
    let contents = dom.ensure_template_contents(template).unwrap();
    let text = dom.create_node_in(dom.owner_document(contents).unwrap(),NodeKind::Text{data:"&".into()}).unwrap();
    dom.pre_insert(text,contents,None).unwrap();
    assert_eq!(output(&dom,template),"&amp;");
    let host=dom.create_element(NodeIdx(0),"div");
    let shadow=dom.attach_shadow(host,crate::shadow::ShadowOptions{mode:crate::shadow::ShadowMode::Closed,delegates_focus:false,slot_assignment:crate::shadow::SlotAssignment::Named,clonable:false,serializable:false}).unwrap();
    let text=dom.create_node_in(NodeIdx(0),NodeKind::Text{data:"<".into()}).unwrap();
    dom.pre_insert(text,shadow,None).unwrap();
    assert_eq!(output(&dom,shadow),"&lt;");
    assert_eq!(output(&dom,host),"");
}
#[test]
fn xml_getter_strict_character_kinds_and_doctype_quote_policy() {
    let mut dom=Dom::new();
    let fragment=dom.create_document_fragment(NodeIdx(0)).unwrap();
    for forbidden in [0,1,0xb,0xc,0xfffe,0xffff,0xd800,0xdc00] {
        let text=dom.create_node_in(NodeIdx(0),NodeKind::Text{data:String::new()}).unwrap();
        dom.replace_character_data(text,0,0,&[forbidden]).unwrap();
        dom.replace_all(fragment,Some(text)).unwrap();
        assert_eq!(dom.serialize_xml_children(fragment),Err(DomError::InvalidStateError));
        assert_eq!(dom.serialize_xml(text,false).unwrap(),vec![forbidden]);
    }
    let comment=dom.create_node_in(NodeIdx(0),NodeKind::Comment{data:"--".into()}).unwrap();
    dom.replace_all(fragment,Some(comment)).unwrap();
    assert_eq!(dom.serialize_xml_children(fragment),Err(DomError::InvalidStateError));
    let pi=dom.create_processing_instruction_in(NodeIdx(0),"target",&[]).unwrap();
    dom.replace_character_data(pi,0,0,&units("bad?>data")).unwrap();
    dom.replace_all(fragment,Some(pi)).unwrap();
    assert_eq!(dom.serialize_xml_children(fragment),Err(DomError::InvalidStateError));
    let cdata=dom.create_node_in(NodeIdx(0),NodeKind::CDATASection{data:"]]>".into()}).unwrap();
    dom.replace_all(fragment,Some(cdata)).unwrap();
    assert_eq!(output(&dom,fragment),"<![CDATA[]]>]]>");
    let doctype=dom.create_document_type_in(NodeIdx(0),"r","","a\"b").unwrap();
    assert_eq!(String::from_utf16(&dom.serialize_xml(doctype,true).unwrap()).unwrap(),"<!DOCTYPE r SYSTEM 'a\"b'>");
    assert_eq!(String::from_utf16(&dom.serialize_xml(doctype,false).unwrap()).unwrap(),"<!DOCTYPE r SYSTEM \"a\"b\">");
}
#[test]
fn xml_getter_native_cycle_and_exact_attribute_validation() {
    let mut dom=Dom::new();
    let root=dom.create_element_ns_in(NodeIdx(0),None,"r").unwrap();
    dom.set_attribute(root,"value","x").unwrap();
    let attr=dom.get_attribute_node(root,None,"value").unwrap();
    dom.set_attr_value(attr,vec![0xd800]).unwrap();
    assert_eq!(dom.serialize_xml(root,true),Err(DomError::InvalidStateError));
    dom.set_attr_value(attr,units("valid")).unwrap();
    let duplicate=dom.get(root).attribute_order[0].clone();
    dom.nodes[root.0 as usize].attribute_order.push(duplicate);
    assert_eq!(dom.serialize_xml(root,true),Err(DomError::InvalidStateError));
    let fragment=dom.create_document_fragment(NodeIdx(0)).unwrap();
    dom.nodes[fragment.0 as usize].first_child=Some(fragment);
    assert_eq!(dom.serialize_xml_children(fragment),Err(DomError::InvalidStateError));
    let stale=dom.create_node_in(NodeIdx(0),NodeKind::Text{data:"retired".into()}).unwrap();
    dom.retire_unreachable_node(stale).unwrap();
    dom.nodes[fragment.0 as usize].first_child=Some(stale);
    assert_eq!(dom.serialize_xml_children(fragment),Err(DomError::InvalidStateError));
    assert_eq!(dom.serialize_xml(fragment,false),Err(DomError::InvalidNode));
    let child=dom.create_element_ns_in(NodeIdx(0),None,"child").unwrap();
    let parent=dom.create_document_fragment(NodeIdx(0)).unwrap();
    dom.nodes[parent.0 as usize].first_child=Some(child);
    dom.nodes[child.0 as usize].next_sibling=Some(child);
    assert_eq!(dom.serialize_xml_children(parent),Err(DomError::InvalidStateError));
}
