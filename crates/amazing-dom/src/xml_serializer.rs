// Portions ported from jsdom/w3c-xmlserializer (lib/serialize.js, lib/attributes.js,
// lib/constants.js) @ 265f1e1a192594708281809c8c7730b65acb36f3, MIT.
// Copyright Sebastian Mayr. License: xml_serializer.LICENSE.
//! Namespace-preserving XML serialization, independent of HTML serialization.
use crate::{Dom, DomError, NodeIdx, NodeKind};
use std::collections::{HashMap, HashSet};

const HTML: &str = "http://www.w3.org/1999/xhtml";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
type PrefixMap = HashMap<Option<String>, Vec<String>>;

#[derive(Clone)]
struct Attribute { ns: Option<String>, prefix: Option<String>, local: String, value: Vec<u16> }
fn units(s: &str) -> Vec<u16> { s.encode_utf16().collect() }
fn append(out: &mut Vec<u16>, s: &str) { out.extend(s.encode_utf16()); }
fn xml_chars(value: &[u16]) -> bool {
    char::decode_utf16(value.iter().copied()).all(|c| c.is_ok_and(|c|
        matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')))
}
fn escaped(value: &[u16], attribute: bool, strict: bool) -> Result<Vec<u16>, DomError> {
    if strict && !xml_chars(value) { return Err(DomError::InvalidStateError); }
    let mut out = Vec::new();
    for &u in value {
        let entity = match u {
            38 => Some("&amp;"), 60 => Some("&lt;"), 62 => Some("&gt;"),
            34 if attribute => Some("&quot;"), 9 if attribute => Some("&#9;"),
            10 if attribute => Some("&#xA;"), 13 if attribute => Some("&#xD;"), _ => None,
        };
        if let Some(s) = entity { append(&mut out, s); } else { out.push(u); }
    }
    Ok(out)
}
fn preferred(map: &PrefixMap, ns: &Option<String>, preference: Option<&str>) -> Option<String> {
    let candidates = map.get(ns)?;
    preference.and_then(|p| candidates.iter().find(|s| s.as_str() == p))
        .or_else(|| candidates.last()).cloned()
}
fn generated(map: &mut PrefixMap, ns: &Option<String>, index: &mut usize) -> String {
    let prefix = format!("ns{}", *index); *index += 1;
    map.insert(ns.clone(), vec![prefix.clone()]); prefix
}
fn normalized(value: &[u16]) -> Option<String> {
    if value.is_empty() { None } else { Some(String::from_utf16_lossy(value)) }
}
fn attr(out: &mut Vec<u16>, name: &str, value: &[u16], strict: bool) -> Result<(), DomError> {
    append(out, " "); append(out, name); append(out, "=\"");
    out.extend(escaped(value, true, strict)?); append(out, "\""); Ok(())
}
fn valid_local(name: &str) -> bool {
    !name.contains(':') && crate::namespace::valid_processing_instruction_target(name)
}

impl Dom {
    /// XML fragment getter boundary: serialize actual children, with one
    /// prefix counter shared across the fragment and no synthetic DOM node.
    /// HTML template children are its actual template contents. Shadow trees
    /// are traversed only when the receiver itself is that ShadowRoot.
    pub fn serialize_xml_children(&self, parent: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.check_node(parent)?;
        let map = HashMap::from([(Some(XML.into()), vec!["xml".into()])]);
        let mut serializer = Serializer { dom: self, strict: true,
            index: 1, path: HashSet::from([parent]) };
        let children = self.template_contents(parent).unwrap_or(parent);
        self.check_node(children)?;
        if children != parent && !serializer.path.insert(children) {
            return Err(DomError::InvalidStateError);
        }
        let mut out = Vec::new();
        serializer.children(children, None, &map, &mut out)?;
        Ok(out)
    }

    /// XMLSerializer uses `require_well_formed = false`; XML inner/outerHTML
    /// can use true. Both preserve DOMString code units when validation is off.
    pub fn serialize_xml(&self, node: NodeIdx, require_well_formed: bool) -> Result<Vec<u16>, DomError> {
        self.check_node(node)?;
        let map = HashMap::from([(Some(XML.into()), vec!["xml".into()])]);
        let mut serializer = Serializer { dom: self, strict: require_well_formed,
            index: 1, path: HashSet::new() };
        let mut out = Vec::new();
        serializer.node(node, None, &map, &mut out)?;
        Ok(out)
    }
}
struct Serializer<'a> { dom: &'a Dom, strict: bool, index: usize, path: HashSet<NodeIdx> }
impl Serializer<'_> {
    fn check_node(&self, node: NodeIdx) -> Result<(), DomError> {
        self.dom.check_node(node).map_err(|error|
            if self.strict { DomError::InvalidStateError } else { error })
    }
    fn node(&mut self, node: NodeIdx, inherited: Option<String>, map: &PrefixMap, out: &mut Vec<u16>) -> Result<(), DomError> {
        self.check_node(node)?;
        if !self.path.insert(node) { return Err(DomError::InvalidStateError); }
        match &self.dom.get(node).kind {
            NodeKind::Element { .. } => self.element(node, inherited, map, out)?,
            NodeKind::Document | NodeKind::DocumentFragment => {
                if self.strict && matches!(self.dom.get(node).kind, NodeKind::Document) {
                    let mut seen = HashSet::new();
                    let mut child = self.dom.get(node).first_child;
                    let mut has_element = false;
                    while let Some(child_node) = child {
                        self.check_node(child_node)?;
                        if !seen.insert(child_node) { return Err(DomError::InvalidStateError); }
                        if matches!(self.dom.get(child_node).kind, NodeKind::Element { .. }) {
                            has_element = true; break;
                        }
                        child = self.dom.get(child_node).next_sibling;
                    }
                    if !has_element { return Err(DomError::InvalidStateError); }
                }
                self.children(node,inherited.clone(),map,out)?;
            }
            NodeKind::Text { .. } => out.extend(escaped(&self.dom.character_data_utf16(node).unwrap_or_default(), false, self.strict)?),
            NodeKind::Comment { .. } => {
                let data = self.dom.character_data_utf16(node).unwrap_or_default();
                if self.strict && (!xml_chars(&data) || data.windows(2).any(|x| x == [45,45]) || data.last() == Some(&45)) {
                    return Err(DomError::InvalidStateError);
                }
                append(out,"<!--"); out.extend(data); append(out,"-->");
            }
            NodeKind::CDATASection { .. } => {
                let data = self.dom.character_data_utf16(node).unwrap_or_default();
                append(out,"<![CDATA["); out.extend(data); append(out,"]]>");
            }
            NodeKind::ProcessingInstruction { target, .. } => {
                let data = self.dom.character_data_utf16(node).unwrap_or_default();
                if self.strict && (target.contains(':') || target.eq_ignore_ascii_case("xml") || !xml_chars(&data)
                    || data.windows(2).any(|x| x == [63,62])) { return Err(DomError::InvalidStateError); }
                append(out,"<?"); append(out,target); append(out," "); out.extend(data); append(out,"?>");
            }
            NodeKind::DocumentType { name, public_id, system_id } => {
                let pubid = public_id.chars().all(|c| c.is_ascii_alphanumeric()
                    || " \r\n-'()+,./:=?;!*#@$_%".contains(c));
                if self.strict && (!pubid || !xml_chars(&units(system_id)) || (system_id.contains('\"') && system_id.contains('\''))) {
                    return Err(DomError::InvalidStateError);
                }
                append(out,"<!DOCTYPE "); append(out,name);
                if !public_id.is_empty() { append(out," PUBLIC \""); append(out,public_id); append(out,"\""); }
                else if !system_id.is_empty() { append(out," SYSTEM"); }
                if !system_id.is_empty() {
                    // Strict getters must use a legal SystemLiteral. Preserve
                    // XMLSerializer's established permissive output policy.
                    let quote = if self.strict && system_id.contains('"') { "'" } else { "\"" };
                    append(out," "); append(out,quote); append(out,system_id); append(out,quote);
                }
                append(out,">");
            }
            NodeKind::Attr { .. } => (),
        }
        self.path.remove(&node); Ok(())
    }
    fn children(&mut self, parent: NodeIdx, inherited: Option<String>, map: &PrefixMap, out: &mut Vec<u16>) -> Result<(), DomError> {
        let mut seen = HashSet::new();
        let mut child = self.dom.get(parent).first_child;
        while let Some(node) = child {
            self.check_node(node)?;
            if !seen.insert(node) { return Err(DomError::InvalidStateError); }
            let next = self.dom.get(node).next_sibling;
            self.node(node,inherited.clone(),map,out)?;
            child = next;
        }
        Ok(())
    }
    fn element(&mut self, node: NodeIdx, mut inherited: Option<String>, prefixes: &PrefixMap, out: &mut Vec<u16>) -> Result<(), DomError> {
        let local = self.dom.local_name(node).ok_or(DomError::InvalidStateError)?;
        if self.strict && !valid_local(local) { return Err(DomError::InvalidStateError); }
        let ns = self.dom.namespace_uri(node).map(str::to_owned);
        let attributes: Vec<_> = self.dom.attributes_expanded(node).unwrap_or_default().into_iter().map(|a|
            Attribute { ns: a.namespace_uri.map(str::to_owned), prefix: a.prefix.map(str::to_owned), local: a.local_name.into(),
                value: self.dom.attribute_ns_utf16(node,a.namespace_uri,a.local_name).unwrap_or_else(|| units(a.value)) }).collect();
        let mut map = prefixes.clone(); let mut declarations = HashMap::<String,Vec<u16>>::new();
        let mut default = None;
        for a in &attributes {
            if a.ns.as_deref() != Some(XMLNS) { continue; }
            if a.prefix.is_none() { default = Some(a.value.clone()); continue; }
            if a.value == units(XML) { continue; }
            let key = normalized(&a.value);
            let list = map.entry(key).or_default();
            if !list.contains(&a.local) { list.push(a.local.clone()); declarations.insert(a.local.clone(),a.value.clone()); }
        }
        let mut ignore_default = false;
        let qualified;
        append(out,"<");
        if inherited == ns {
            ignore_default = default.is_some();
            qualified = if ns.as_deref() == Some(XML) { format!("xml:{local}") } else { local.to_owned() };
            append(out,&qualified);
        } else {
            let mut prefix = self.dom.prefix(node).map(str::to_owned);
            let mut candidate = preferred(&map,&ns,prefix.as_deref());
            if prefix.as_deref() == Some("xmlns") {
                if self.strict { return Err(DomError::InvalidStateError); }
                candidate = Some("xmlns".into());
            }
            if let Some(candidate) = candidate {
                qualified = format!("{candidate}:{local}"); append(out,&qualified);
                if let Some(default) = &default { if *default != units(XML) { inherited = normalized(default); } }
            } else if let Some(p) = prefix.take() {
                let p = if declarations.contains_key(&p) { generated(&mut map,&ns,&mut self.index) } else { p };
                map.entry(ns.clone()).or_default().push(p.clone());
                qualified = format!("{p}:{local}"); append(out,&qualified);
                attr(out,&format!("xmlns:{p}"),&units(ns.as_deref().unwrap_or("")),self.strict)?;
                if let Some(default) = &default { inherited = normalized(default); }
            } else if default.as_ref().map(|d| normalized(d)) != Some(ns.clone()) {
                ignore_default = true; qualified = local.to_owned(); inherited = ns.clone(); append(out,&qualified);
                attr(out,"xmlns",&units(ns.as_deref().unwrap_or("")),self.strict)?;
            } else { qualified = local.to_owned(); inherited = ns.clone(); append(out,&qualified); }
        }
        let mut seen = HashSet::new();
        for a in &attributes {
            if self.strict && !seen.insert((a.ns.clone(),a.local.clone())) { return Err(DomError::InvalidStateError); }
            let mut candidate = if a.ns.is_some() { preferred(&map,&a.ns,a.prefix.as_deref()) } else { None };
            if a.ns.as_deref() == Some(XMLNS) {
                if a.value == units(XML) || (a.prefix.is_none() && ignore_default)
                    || (a.prefix.is_some() && declarations.get(&a.local) != Some(&a.value)
                        && map.get(&normalized(&a.value)).is_some_and(|p| p.contains(&a.local))) { continue; }
                if self.strict && (a.value == units(XMLNS) || (a.prefix.is_some() && a.value.is_empty())) { return Err(DomError::InvalidStateError); }
                if a.prefix.as_deref() == Some("xmlns") { candidate = Some("xmlns".into()); }
            } else if a.ns.is_some() && candidate.is_none() {
                // Current DOM Parsing attribute-prefix policy: retain an
                // unbound original prefix unless this element declares it.
                let p = match a.prefix.as_deref() {
                    Some(p) if !declarations.contains_key(p) => p.to_owned(),
                    _ => generated(&mut map,&a.ns,&mut self.index),
                };
                map.entry(a.ns.clone()).or_default().push(p.clone());
                attr(out,&format!("xmlns:{p}"),&units(a.ns.as_deref().unwrap()),self.strict)?;
                declarations.insert(p.clone(),units(a.ns.as_deref().unwrap()));
                candidate = Some(p);
            }
            if self.strict && (!valid_local(&a.local) || (a.local == "xmlns" && a.ns.is_none())) { return Err(DomError::InvalidStateError); }
            let name = candidate.map(|p| format!("{p}:{}",a.local)).unwrap_or_else(|| a.local.clone());
            attr(out,&name,&a.value,self.strict)?;
        }
        let empty = self.dom.children(node).next().is_none();
        let void = matches!(local,"area"|"base"|"basefont"|"bgsound"|"br"|"col"|"embed"|"frame"|"hr"|"img"|"input"|"keygen"|"link"|"menuitem"|"meta"|"param"|"source"|"track"|"wbr");
        if empty && (ns.as_deref() != Some(HTML) || void) {
            append(out,if ns.as_deref() == Some(HTML) { " />" } else { "/>" }); return Ok(());
        }
        append(out,">");
        if ns.as_deref() == Some(HTML) && local == "template" {
            if let Some(content) = self.dom.template_contents(node) { self.node(content,inherited.clone(),&map,out)?; }
        } else { self.children(node,inherited.clone(),&map,out)?; }
        append(out,"</"); append(out,&qualified); append(out,">"); Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(dom: &Dom, node: NodeIdx, strict: bool) -> String {
        String::from_utf16(&dom.serialize_xml(node,strict).unwrap()).unwrap()
    }
    #[test]
    fn xml_namespace_scope_and_prefix_generation_preserve_identity() {
        let mut dom = Dom::new();
        let root=dom.create_element_ns_in(NodeIdx(0),Some("urn:root"),"p:root").unwrap();
        dom.set_attribute_qualified_ns(root,Some(XMLNS),"xmlns:p","urn:other").unwrap();
        dom.set_attribute_qualified_ns(root,Some("urn:other"),"p:value","a\t\n\r<&>\"").unwrap();
        assert_eq!(text(&dom,root,false),"<ns1:root xmlns:ns1=\"urn:root\" xmlns:p=\"urn:other\" p:value=\"a&#9;&#xA;&#xD;&lt;&amp;&gt;&quot;\"/>");
        let child=dom.create_element_ns_in(NodeIdx(0),None,"child").unwrap();
        let parent=dom.create_element_ns_in(NodeIdx(0),Some("urn:parent"),"parent").unwrap();
        dom.pre_insert(child,parent,None).unwrap();
        assert_eq!(text(&dom,parent,true),"<parent xmlns=\"urn:parent\"><child xmlns=\"\"/></parent>");
        let unbound=dom.create_element_ns_in(NodeIdx(0),Some("urn:root"),"root").unwrap();
        dom.set_attribute_qualified_ns(unbound,Some("urn:attr"),"a:value","x").unwrap();
        dom.set_attribute_qualified_ns(unbound,Some(XML),"xml:lang","en").unwrap();
        assert_eq!(text(&dom,unbound,true),"<root xmlns=\"urn:root\" xmlns:a=\"urn:attr\" a:value=\"x\" xml:lang=\"en\"/>");
        let collision=dom.create_element_ns_in(NodeIdx(0),None,"r").unwrap();
        dom.set_attribute_qualified_ns(collision,Some("urn:first"),"p:one","a").unwrap();
        dom.set_attribute_qualified_ns(collision,Some("urn:second"),"p:two","b").unwrap();
        assert_eq!(text(&dom,collision,true),"<r xmlns:p=\"urn:first\" p:one=\"a\" xmlns:ns1=\"urn:second\" ns1:two=\"b\"/>");
    }
    #[test]
    fn xml_permissive_serialization_and_strict_well_formedness_are_distinct() {
        let mut dom=Dom::new();
        let comment=dom.create_node_in(NodeIdx(0),NodeKind::Comment { data: "bad--end-".into() }).unwrap();
        assert_eq!(text(&dom,comment,false),"<!--bad--end--->");
        assert_eq!(dom.serialize_xml(comment,true),Err(DomError::InvalidStateError));
        let text_node=dom.create_node_in(NodeIdx(0),NodeKind::Text { data: String::new() }).unwrap();
        dom.replace_character_data(text_node,0,0,&[0xd800,38,0]).unwrap();
        assert_eq!(dom.serialize_xml(text_node,false).unwrap(),vec![0xd800,38,97,109,112,59,0]);
        assert_eq!(dom.serialize_xml(text_node,true),Err(DomError::InvalidStateError));
        let element=dom.create_element_ns_in(NodeIdx(0),None,"root").unwrap();
        dom.set_attribute(element,"value","x").unwrap();
        let attr=dom.get_attribute_node(element,None,"value").unwrap();
        assert_eq!(dom.serialize_xml(attr,false).unwrap(),Vec::<u16>::new());
        dom.set_attr_value(attr,vec![0xdc00]).unwrap();
        assert!(dom.serialize_xml(element,false).unwrap().contains(&0xdc00));
        assert_eq!(dom.serialize_xml(element,true),Err(DomError::InvalidStateError));
        assert_eq!(text(&dom,NodeIdx(0),false),"");
        assert_eq!(dom.serialize_xml(NodeIdx(0),true),Err(DomError::InvalidStateError));
    }
    #[test]
    fn xml_node_kinds_html_empty_rules_template_contents_and_cycles() {
        let mut dom=Dom::new();
        let fragment=dom.create_document_fragment(NodeIdx(0)).unwrap();
        let pi=dom.create_processing_instruction_in(NodeIdx(0),"xml-stylesheet",&units("a='b'")).unwrap();
        let cdata=dom.create_node_in(NodeIdx(0),NodeKind::CDATASection { data: "<&>".into() }).unwrap();
        dom.pre_insert(pi,fragment,None).unwrap();dom.pre_insert(cdata,fragment,None).unwrap();
        assert_eq!(text(&dom,fragment,true),"<?xml-stylesheet a='b'?><![CDATA[<&>]]>");
        let doctype=dom.create_document_type_in(NodeIdx(0),"html","-//Example//EN","urn:example").unwrap();
        assert_eq!(text(&dom,doctype,true),"<!DOCTYPE html PUBLIC \"-//Example//EN\" \"urn:example\">");
        let html=dom.create_element_ns_in(NodeIdx(0),Some(HTML),"br").unwrap();
        assert_eq!(text(&dom,html,false),"<br xmlns=\"http://www.w3.org/1999/xhtml\" />");
        let template=dom.create_element_ns_in(NodeIdx(0),Some(HTML),"template").unwrap();
        let contents=dom.ensure_template_contents(template).unwrap();
        let owner=dom.owner_document(contents).unwrap();let child=dom.create_node_in(owner,NodeKind::Text { data: "&".into() }).unwrap();
        dom.pre_insert(child,contents,None).unwrap();
        assert_eq!(text(&dom,template,false),"<template xmlns=\"http://www.w3.org/1999/xhtml\">&amp;</template>");
        let cycle=dom.create_document_fragment(NodeIdx(0)).unwrap();
        dom.nodes[cycle.0 as usize].first_child=Some(cycle);
        dom.nodes[cycle.0 as usize].last_child=Some(cycle);
        assert_eq!(dom.serialize_xml(cycle,false),Err(DomError::InvalidStateError));
        let document=dom.create_document();
        let child=dom.create_node_in(document,NodeKind::Text { data: "x".into() }).unwrap();
        dom.nodes[document.0 as usize].first_child=Some(child);
        dom.nodes[document.0 as usize].last_child=Some(child);
        dom.nodes[child.0 as usize].next_sibling=Some(child);
        assert_eq!(dom.serialize_xml(document,true),Err(DomError::InvalidStateError));
        assert_eq!(dom.serialize_xml(document,false),Err(DomError::InvalidStateError));
    }
}

#[cfg(test)]
#[path = "xml_markup_getter_tests.rs"]
mod getter_tests;
