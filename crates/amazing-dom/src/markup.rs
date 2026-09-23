//! HTML fragment serialization and transfer from a parser-owned arena.
use crate::{Dom, DomError, NodeIdx, NodeKind};

fn append(out: &mut Vec<u16>, value: &str) { out.extend(value.encode_utf16()); }

fn escape(out: &mut Vec<u16>, value: &[u16], attribute: bool, raw: bool) {
    for unit in value {
        if raw { out.push(*unit); continue; }
        let replacement = match *unit {
            0x26 => Some("&amp;"),
            0xA0 => Some("&nbsp;"),
            0x22 if attribute => Some("&quot;"),
            0x3C if !attribute => Some("&lt;"),
            0x3E if !attribute => Some("&gt;"),
            _ => None,
        };
        if let Some(replacement) = replacement { append(out, replacement); } else { out.push(*unit); }
    }
}

fn void_element(name: &str) -> bool {
    matches!(name, "area" | "base" | "basefont" | "bgsound" | "br" | "col" | "embed"
        | "frame" | "hr" | "img" | "input" | "keygen" | "link" | "meta" | "param"
        | "source" | "track" | "wbr")
}
fn raw_text_parent(name: &str) -> bool {
    matches!(name, "style" | "script" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext")
}

impl Dom {
    /// DOM-only innerText approximation: text nodes and BR line breaks, with
    /// no style, layout, visibility, or CSS white-space computation.
    pub fn inner_text_approx(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.check_node(node)?;
        let mut out = Vec::new();
        self.inner_text_into(node, &mut out);
        Ok(out)
    }

    fn inner_text_into(&self, node: NodeIdx, out: &mut Vec<u16>) {
        match &self.get(node).kind {
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } =>
                out.extend(self.character_data_utf16(node).unwrap_or_default()),
            NodeKind::Element { .. } if self.namespace_uri(node) == Some("http://www.w3.org/1999/xhtml")
                && self.local_name(node) == Some("br") => out.push(b'\n' as u16),
            NodeKind::Element { .. } | NodeKind::Document | NodeKind::DocumentFragment =>
                for child in self.children(node) { self.inner_text_into(child, out); },
            _ => (),
        }
    }
    /// HTML fragment serialization; results preserve UTF-16 code units.
    pub fn serialize_children(&self, parent: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.serialize_children_with_shadow(parent, false, &[])
    }

    /// HTML fragment serialization used by Element/ShadowRoot.getHTML().
    /// `shadow_roots` can include closed and non-serializable roots explicitly.
    pub fn serialize_children_with_shadow(&self, parent: NodeIdx,
        serializable_shadow_roots: bool, shadow_roots: &[NodeIdx]) -> Result<Vec<u16>, DomError> {
        self.check_node(parent)?;
        let mut out = Vec::new();
        if self.namespace_uri(parent) == Some("http://www.w3.org/1999/xhtml")
            && self.local_name(parent).is_some_and(void_element) { return Ok(out); }
        self.serialize_shadow_into(parent, &mut out, serializable_shadow_roots, shadow_roots);
        let parent = self.template_contents(parent).unwrap_or(parent);
        for child in self.children(parent) {
            self.serialize_node_into(child, &mut out, serializable_shadow_roots, shadow_roots);
        }
        Ok(out)
    }

    pub fn serialize_node(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.check_node(node)?;
        let mut out = Vec::new();
        self.serialize_node_into(node, &mut out, false, &[]);
        Ok(out)
    }

    fn serialize_shadow_into(&self, host: NodeIdx, out: &mut Vec<u16>,
        serializable_shadow_roots: bool, shadow_roots: &[NodeIdx]) {
        let Some(root) = self.shadow_root_of_host(host) else { return };
        let data = self.shadow_data(root).expect("attached shadow root");
        if !((serializable_shadow_roots && data.options.serializable)
            || shadow_roots.contains(&root)) { return }
        append(out, "<template shadowrootmode=\"");
        append(out, match data.options.mode {
            crate::shadow::ShadowMode::Open => "open",
            crate::shadow::ShadowMode::Closed => "closed",
        });
        append(out, "\"");
        if data.options.delegates_focus { append(out, " shadowrootdelegatesfocus=\"\""); }
        if data.options.serializable { append(out, " shadowrootserializable=\"\""); }
        if data.options.slot_assignment == crate::shadow::SlotAssignment::Manual {
            append(out, " shadowrootslotassignment=\"manual\"");
        }
        if data.options.clonable { append(out, " shadowrootclonable=\"\""); }
        if data.keep_custom_element_registry_null {
            append(out, " shadowrootcustomelementregistry=\"\"");
        }
        append(out, ">");
        for child in self.children(root) {
            self.serialize_node_into(child, out, serializable_shadow_roots, shadow_roots);
        }
        append(out, "</template>");
    }

    fn serialize_node_into(&self, node: NodeIdx, out: &mut Vec<u16>,
        serializable_shadow_roots: bool, shadow_roots: &[NodeIdx]) {
        match &self.get(node).kind {
            NodeKind::Element { .. } => {
                let name = self.qualified_name(node).expect("element name");
                append(out, "<"); append(out, &name);
                for attr in self.attributes_expanded(node).expect("element attributes") {
                    append(out, " "); append(out, &attr.qualified_name()); append(out, "=\"");
                    escape(out, &attr.value.encode_utf16().collect::<Vec<_>>(), true, false);
                    append(out, "\"");
                }
                append(out, ">");
                let is_html = self.namespace_uri(node) == Some("http://www.w3.org/1999/xhtml");
                if is_html && void_element(self.local_name(node).unwrap()) { return; }
                self.serialize_shadow_into(node, out, serializable_shadow_roots, shadow_roots);
                let contents = self.template_contents(node).unwrap_or(node);
                for child in self.children(contents) {
                    self.serialize_node_into(child, out, serializable_shadow_roots, shadow_roots);
                }
                append(out, "</"); append(out, &name); append(out, ">");
            }
            NodeKind::Text { .. } => {
                let raw = self.get(node).parent.is_some_and(|parent|
                    self.namespace_uri(parent) == Some("http://www.w3.org/1999/xhtml")
                        && self.local_name(parent).is_some_and(raw_text_parent));
                escape(out, &self.character_data_utf16(node).unwrap_or_default(), false, raw);
            }
            NodeKind::CDATASection { .. } => {
                append(out, "<![CDATA[");
                out.extend(self.character_data_utf16(node).unwrap_or_default());
                append(out, "]]>");
            }
            NodeKind::Comment { data } => { append(out, "<!--"); append(out, data); append(out, "-->"); }
            NodeKind::ProcessingInstruction { target, data } => {
                append(out, "<?"); append(out, target); append(out, " "); append(out, data); append(out, ">");
            }
            NodeKind::DocumentType { name, .. } => { append(out, "<!DOCTYPE "); append(out, name); append(out, ">"); }
            NodeKind::Document | NodeKind::DocumentFragment => {
                for child in self.children(node) {
                    self.serialize_node_into(child, out, serializable_shadow_roots, shadow_roots);
                }
            }
            NodeKind::Attr { .. } => (),
        }
    }

    /// Copies parser results into this arena, preserving element namespace and tree order.
    /// The caller inserts the returned DocumentFragment with checked DOM operations.
    pub fn import_fragment(&mut self, source: &Dom, document: NodeIdx) -> Result<NodeIdx, DomError> {
        self.check_node(document)?;
        let fragment = self.create_document_fragment(document)?;
        for child in source.children(NodeIdx(0)) {
            self.import_node_into(source, child, document, fragment)?;
        }
        Ok(fragment)
    }

    /// Imports a parser's document into this arena without a browsing context or script execution.
    pub fn import_inert_document(&mut self, source: &Dom, url: &str, content_type: &str,
        character_set: &str) -> Result<NodeIdx, DomError> {
        let document = self.create_document();
        for child in source.children(NodeIdx(0)) {
            self.import_node_into(source, child, document, document)?;
        }
        self.set_document_metadata(document, crate::document_surface::DocumentMetadata {
            url: url.into(), content_type: content_type.into(), character_set: character_set.into(),
        })?;
        self.set_document_ready_state(document, crate::document_surface::ReadyState::Complete)?;
        Ok(document)
    }

    fn import_node_into(&mut self, source: &Dom, node: NodeIdx, document: NodeIdx, parent: NodeIdx)
        -> Result<(), DomError> {
        let created = match &source.get(node).kind {
            NodeKind::Element { .. } => {
                let created = self.create_element_expanded_in(document, source.namespace_uri(node),
                    source.prefix(node), source.local_name(node).unwrap())?;
                for attr in source.attributes_expanded(node).unwrap() {
                    self.set_attribute_expanded(created, attr.namespace_uri, attr.prefix,
                        attr.local_name, attr.value)?;
                }
                created
            }
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } | NodeKind::Comment { .. } => {
                let kind = match &source.get(node).kind {
                    NodeKind::Text { .. } => NodeKind::Text { data: String::new() },
                    NodeKind::CDATASection { .. } => NodeKind::CDATASection { data: String::new() },
                    _ => NodeKind::Comment { data: String::new() },
                };
                let created = self.create_node_in(document, kind)?;
                let units = source.character_data_utf16(node).unwrap_or_default();
                self.replace_character_data(created, 0, 0, &units)?;
                created
            }
            NodeKind::ProcessingInstruction { target, data } =>
                self.create_node_in(document, NodeKind::ProcessingInstruction { target: target.clone(), data: data.clone() })?,
            NodeKind::DocumentType { name, public_id, system_id } =>
                self.create_node_in(document, NodeKind::DocumentType {
                    name: name.clone(), public_id: public_id.clone(), system_id: system_id.clone(),
                })?,
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Attr { .. } =>
                return Err(DomError::NotSupportedError),
        };
        self.pre_insert(created, parent, None)?;
        if let Some(root) = source.shadow_root_of_host(node) {
            let source_data = source.shadow_data(root).expect("attached parser shadow root").clone();
            let copied_root = self.attach_shadow(created, source_data.options)?;
            let copied_data = self.shadow_roots.iter_mut().find(|data| data.root == copied_root)
                .expect("new imported shadow root");
            copied_data.declarative = source_data.declarative;
            copied_data.available_to_element_internals = source_data.available_to_element_internals;
            copied_data.keep_custom_element_registry_null = source_data.keep_custom_element_registry_null;
            for child in source.children(root) {
                self.import_node_into(source, child, document, copied_root)?;
            }
        }
        if source.is_html_template(node) {
            let destination = self.ensure_template_contents(created)?;
            if let Some(contents) = source.template_contents(node) {
                let inert = self.owner_document(destination).ok_or(DomError::InvalidStateError)?;
                for child in source.children(contents) {
                    self.import_node_into(source, child, inert, destination)?;
                }
            }
        } else {
            for child in source.children(node) { self.import_node_into(source, child, document, created)?; }
        }
        Ok(())
    }
}
