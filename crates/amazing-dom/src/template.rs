//! Inert, persistent contents for HTML template elements.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};
use crate::document_surface::DocumentMetadata;

impl Dom {
    pub fn is_html_template(&self, node: NodeIdx) -> bool {
        matches!(self.get(node).kind, NodeKind::Element { namespace: Namespace::Html, .. })
            && self.local_name(node) == Some("template")
    }

    /// The fragment is separate from the template's light DOM and keeps its
    /// identity across reads. Its owner is the document's inert template owner.
    pub fn ensure_template_contents(&mut self, template: NodeIdx) -> Result<NodeIdx, DomError> {
        self.check_node(template)?;
        if !self.is_html_template(template) { return Err(DomError::TypeError); }
        if let Some(contents) = self.template_contents(template) { return Ok(contents); }
        let document = self.get(template).owner_document;
        let owner = self.appropriate_template_document(document);
        let contents = self.create_document_fragment(owner)?;
        self.template_contents.push((template, self.node_generation(template).unwrap(),
            contents, self.node_generation(contents).unwrap()));
        Ok(contents)
    }

    pub fn template_contents(&self, template: NodeIdx) -> Option<NodeIdx> {
        self.template_contents.iter().find(|(node, version, contents, contents_version)|
            *node == template && self.node_generation(*node) == Some(*version)
                && self.node_generation(*contents) == Some(*contents_version))
            .map(|(_, _, contents, _)| *contents)
    }

    /// Reverse of `template_contents`: the `<template>` element that owns
    /// `contents`, if `contents` is itself a template-contents fragment.
    pub fn template_of_contents(&self, contents: NodeIdx) -> Option<NodeIdx> {
        self.template_contents.iter().find(|(template, version, node, node_version)|
            *node == contents && self.node_generation(*template) == Some(*version)
                && self.node_generation(*node) == Some(*node_version))
            .map(|(template, _, _, _)| *template)
    }

    pub(crate) fn appropriate_template_document(&mut self, document: NodeIdx) -> NodeIdx {
        let valid = |&(source, version, inert, inert_version): &(NodeIdx, u64, NodeIdx, u64)|
            self.node_generation(source) == Some(version)
                && self.node_generation(inert) == Some(inert_version);
        if self.template_owner_documents.iter().filter(|entry| valid(entry)).any(|(_, _, inert, _)| *inert == document) {
            return document;
        }
        if let Some((_, _, inert, _)) = self.template_owner_documents.iter()
            .filter(|entry| valid(entry)).find(|(source, _, _, _)| *source == document) {
            return *inert;
        }
        let html = self.document_metadata(document).ok().flatten()
            .map_or(document == NodeIdx(0), |metadata| metadata.content_type == "text/html");
        let inert = self.create_document();
        // A fresh document inherits only the HTML document type. URL,
        // encoding, mode and browsing context keep their creation defaults.
        let metadata = DocumentMetadata { url: "about:blank".into(),
            content_type: if html { "text/html" } else { "application/xml" }.into(),
            character_set: "UTF-8".into() };
        self.set_document_metadata(inert, metadata).expect("new template owner document");
        self.template_owner_documents.push((document, self.node_generation(document).unwrap(),
            inert, self.node_generation(inert).unwrap()));
        inert
    }

    pub(crate) fn adopt_template_contents(&mut self, template: NodeIdx, document: NodeIdx) {
        if let Some(contents) = self.template_contents(template) {
            let inert = self.appropriate_template_document(document);
            self.set_subtree_document(contents, inert);
        }
    }
}
