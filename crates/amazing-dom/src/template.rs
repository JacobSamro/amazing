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
        self.template_contents.push((template, contents));
        Ok(contents)
    }

    pub fn template_contents(&self, template: NodeIdx) -> Option<NodeIdx> {
        self.template_contents.iter().find(|(node, _)| *node == template).map(|(_, contents)| *contents)
    }

    /// Reverse of `template_contents`: the `<template>` element that owns
    /// `contents`, if `contents` is itself a template-contents fragment.
    pub fn template_of_contents(&self, contents: NodeIdx) -> Option<NodeIdx> {
        self.template_contents.iter().find(|(_, c)| *c == contents).map(|(template, _)| *template)
    }

    pub(crate) fn appropriate_template_document(&mut self, document: NodeIdx) -> NodeIdx {
        if self.template_owner_documents.iter().any(|(_, inert)| *inert == document) {
            return document;
        }
        if let Some((_, inert)) = self.template_owner_documents.iter().find(|(source, _)| *source == document) {
            return *inert;
        }
        let inert = self.create_document();
        // DOM Standard "appropriate template contents owner document": the
        // new document has `document`'s type, so an HTML source document's
        // inert template contents document is also an HTML document.
        // Without this, every element built into `<template>` content (this
        // owner document's whole purpose) reports a foreign-case `tagName`,
        // since `is_html_document` requires explicit `text/html` metadata
        // rather than defaulting to it for any document but the main one.
        // The main document (`NodeIdx(0)`) often has no explicit metadata
        // set either, so default to HTML there too, matching that same
        // main-document special case.
        let metadata = self.document_metadata(document).ok().flatten().cloned()
            .unwrap_or_else(|| DocumentMetadata {
                url: String::new(), content_type: "text/html".into(), character_set: "UTF-8".into(),
            });
        let _ = self.set_document_metadata(inert, metadata);
        self.template_owner_documents.push((document, inert));
        inert
    }

    pub(crate) fn adopt_template_contents(&mut self, template: NodeIdx, document: NodeIdx) {
        if let Some(contents) = self.template_contents(template) {
            let inert = self.appropriate_template_document(document);
            self.set_subtree_document(contents, inert);
        }
    }
}
