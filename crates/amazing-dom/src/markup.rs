//! HTML fragment serialization and transfer from a parser-owned arena.
use crate::{Dom, DomError, NodeIdx, NodeKind};
use std::collections::HashMap;

fn append(out: &mut Vec<u16>, value: &str) { out.extend(value.encode_utf16()); }

fn escape(out: &mut Vec<u16>, value: &[u16], attribute: bool, raw: bool) {
    for unit in value {
        if raw { out.push(*unit); continue; }
        let replacement = match *unit {
            0x26 => Some("&amp;"),
            0xA0 => Some("&nbsp;"),
            0x22 if attribute => Some("&quot;"),
            0x3C => Some("&lt;"),
            0x3E => Some("&gt;"),
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
                let name = if matches!(self.namespace(node), Some(crate::Namespace::Html | crate::Namespace::Svg | crate::Namespace::MathMl)) {
                    self.local_name(node).expect("element name").to_owned()
                } else { self.qualified_name(node).expect("element name") };
                append(out, "<"); append(out, &name);
                for attr in self.attributes_expanded(node).expect("element attributes") {
                    let name = match attr.namespace_uri {
                        None => attr.local_name.to_owned(),
                        Some("http://www.w3.org/XML/1998/namespace") => format!("xml:{}", attr.local_name),
                        Some("http://www.w3.org/2000/xmlns/") if attr.local_name == "xmlns" => "xmlns".into(),
                        Some("http://www.w3.org/2000/xmlns/") => format!("xmlns:{}", attr.local_name),
                        Some("http://www.w3.org/1999/xlink") => format!("xlink:{}", attr.local_name),
                        _ => attr.qualified_name(),
                    };
                    let value = self.attached_attr_slot(node, attr.namespace_uri, attr.local_name)
                        .and_then(|attr| self.attr_snapshot(attr).ok()).map(|attr| attr.value)
                        .unwrap_or_else(|| attr.value.encode_utf16().collect());
                    append(out, " "); append(out, &name); append(out, "=\"");
                    escape(out, &value, true, false);
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
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } => {
                let raw = self.get(node).parent.is_some_and(|parent|
                    self.namespace_uri(parent) == Some("http://www.w3.org/1999/xhtml")
                        && (self.local_name(parent).is_some_and(raw_text_parent)
                            || self.local_name(parent) == Some("noscript") && self.owner_document(node)
                                .is_some_and(|document| self.document_scripting_enabled(document).unwrap_or(false))));
                escape(out, &self.character_data_utf16(node).unwrap_or_default(), false, raw);
            }
            NodeKind::Comment { .. } => { append(out, "<!--"); out.extend(self.character_data_utf16(node).unwrap_or_default()); append(out, "-->"); }
            NodeKind::ProcessingInstruction { target, .. } => {
                append(out, "<?"); append(out, target); append(out, " "); out.extend(self.character_data_utf16(node).unwrap_or_default()); append(out, "?>");
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

    /// Imports `source_node`'s children (not `source_node` itself) into
    /// `target`, an already-real node living in this arena --
    /// `Element`/`ShadowRoot.setHTMLUnsafe`'s top-level declarative-shadow-
    /// root shape needs this (`markup_bindings.rs`'s `top_level_
    /// declarative_shadow_template`): the caller has already attached a
    /// real shadow root via `attach_declarative_shadow` and must move a
    /// standalone parsed fragment's `<template>`'s own `template_contents`
    /// children into it, which `import_fragment` cannot do since it always
    /// creates a fresh top-level `DocumentFragment` from `source`'s own
    /// top-level children rather than importing an arbitrary subtree into
    /// an arbitrary existing target.
    pub fn import_subtree_children(&mut self, source: &Dom, source_node: NodeIdx, target: NodeIdx)
        -> Result<(), DomError> {
        self.check_node(target)?;
        let document = self.owner_document(target).ok_or(DomError::InvalidStateError)?;
        for child in source.children(source_node) {
            self.import_node_into(source, child, document, target)?;
        }
        Ok(())
    }

    /// Imports a parser's document into this arena without a browsing context or script execution.
    pub fn import_inert_document(&mut self, source: &Dom, url: &str, content_type: &str,
        character_set: &str) -> Result<NodeIdx, DomError> {
        let document = self.create_document();
        self.set_document_allows_declarative_shadow_roots(document,
            source.document_allows_declarative_shadow_roots(NodeIdx(0))?)?;
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

/// HTML-34: incrementally imports a script-interleaved `ResumableParser`'s
/// private arena into an existing, otherwise-empty document node already
/// living in a *different* arena (an iframe's document inside its parent
/// page's shared `host.dom`), preserving the identity of every
/// already-imported node across repeated calls.
///
/// This exists because `import_inert_document`/`import_node_into` above are
/// one-shot: they assume the whole source tree is already finished and copy
/// it once. A live, interleaved parse (HTML-31/32's `ResumableParser`, which
/// pauses at each parser-blocking `<script>` so it can run against the
/// document-so-far) needs the destination side of that copy to happen
/// incrementally too, or a script would still only ever see whatever was
/// imported at the *start* of the pause -- the same "sees the finished
/// document" bug HTML-31 fixed for the top-level page, just moved one layer
/// down into the import step instead of being fixed by it.
///
/// It cannot reuse `ResumableParser::swap_dom` directly (see
/// `docs/lanes/html_TODO.md`'s HTML-32/HTML-34 notes): that swaps an entire
/// `Dom` value, which only works when the parser's target arena has nothing
/// else in it. An iframe's document has to end up inside the same arena as
/// its parent page and every sibling frame, so it is still parsed into its
/// own private, ordinary `Dom::new()`-based arena (completely unmodified
/// `ResumableParser`/`TreeBuilder`, which only knows how to build starting
/// from that shape) and *grafted* into the shared arena at each pause, this
/// time keeping already-grafted nodes' destination identity stable instead
/// of re-copying the whole tree.
///
/// Mapped nodes retain their identities when parsing moves them. Reverse
/// synchronization mirrors external documents and detached containers too, so
/// the parser validates insertion against the live document's actual children.
pub struct IncrementalDocumentImport {
    dest_document: NodeIdx,
    /// Every source node already copied, mapping to its destination
    /// counterpart -- never removed, so `sync` can always tell an
    /// already-imported node from a newly appended one. Also covers a
    /// shadow root's own source `NodeIdx` (not reachable via `children`,
    /// so `sync_special_containers` below re-descends into it explicitly)
    /// and a template's contents fragment, the same way.
    map: HashMap<NodeIdx, NodeIdx>,
    /// The reverse of `map` (dest -> source), kept in lockstep by `record`
    /// -- `reverse_sync` needs to go from a `dest` node it already knows
    /// back to the `source` node it should update, just as often as `sync`
    /// needs the other direction, so both maps are maintained together
    /// rather than rebuilding one from the other on every call.
    rev: HashMap<NodeIdx, NodeIdx>,
    generations: HashMap<NodeIdx, (u64, u64)>,
}

impl IncrementalDocumentImport {
    /// `dest_document` must be an otherwise-empty document node this arena
    /// already created (`Dom::create_document`). The source arena's own
    /// document root is always `NodeIdx(0)` (every `Dom::new()`'s first
    /// allocation, per `ResumableParser`'s own contract).
    pub fn new(dest_document: NodeIdx) -> Self {
        let mut map = HashMap::new();
        map.insert(NodeIdx(0), dest_document);
        let mut rev = HashMap::new();
        rev.insert(dest_document, NodeIdx(0));
        Self { dest_document, map, rev, generations: HashMap::new() }
    }

    /// Records a newly-copied pair in both directions.
    fn record(&mut self, source: &Dom, dest: &Dom, source_node: NodeIdx, dest_node: NodeIdx) {
        self.generations.insert(source_node, (source.node_generation(source_node).expect("live source mirror"),
            dest.node_generation(dest_node).expect("live destination mirror")));
        self.map.insert(source_node, dest_node);
        self.rev.insert(dest_node, source_node);
    }

    // A parser mapping is an identity relation, never permission to redirect a
    // recycled arena slot. Validate the complete existing relation before any
    // value/tree/state write in either direction. The root pair is captured on
    // the first use, because new() intentionally receives no arena references.
    fn validate_mappings(&mut self, source: &Dom, dest: &Dom) -> Result<(), DomError> {
        for (&s, &d) in &self.map {
            let current = (source.node_generation(s).ok_or(DomError::InvalidStateError)?,
                dest.node_generation(d).ok_or(DomError::InvalidStateError)?);
            if let Some(saved) = self.generations.get(&s) {
                if *saved != current { return Err(DomError::InvalidStateError); }
            } else if s != NodeIdx(0) || d != self.dest_document {
                return Err(DomError::InvalidStateError);
            }
        }
        if !self.generations.contains_key(&NodeIdx(0)) {
            self.generations.insert(NodeIdx(0), (source.node_generation(NodeIdx(0)).unwrap(),
                dest.node_generation(self.dest_document).unwrap()));
        }
        Ok(())
    }

    /// The destination document node passed to `new`.
    pub fn dest_document(&self) -> NodeIdx {
        self.dest_document
    }

    /// Maps an already-imported source node to its destination counterpart.
    pub fn imported(&self, source_node: NodeIdx) -> Option<NodeIdx> {
        self.map.get(&source_node).copied()
    }

    /// Imports every source node not yet copied since the last call,
    /// preserving previously imported nodes' destination identity. Safe to
    /// call repeatedly (at every parser pause, and once more after `Eof`);
    /// unchanged values do not produce repeated mutations.
    pub fn sync(&mut self, dest: &mut Dom, source: &Dom) -> Result<(), DomError> {
        self.validate_mappings(source, dest)?;
        self.sync_values(dest, source)?;
        self.sync_positions(dest, source)?;
        // Open parser nodes may have been adopted into another document or a
        // detached subtree. Visit all mapped containers, not only root zero.
        let pairs: Vec<_> = self.map.iter().filter(|&(&s, _)|
            source.host_including_parent(s).is_none()).map(|(&s, &d)| (s, d)).collect();
        for (s, d) in pairs {
            self.sync_children(dest, source, s, d)?;
            self.sync_special_containers(dest, source, s)?;
        }
        self.sync_positions(dest, source)?;
        self.sync_template_insertions(dest, source)?;
        self.sync_positions(dest, source)?;
        Ok(())
    }

    // Unlike clone/import, parser mirrors retain the SAME template's live
    // insertion identities. Open patching templates can be detached and exist
    // only on the parser stack, so include their actual native state explicitly.
    fn sync_template_insertions(&mut self, dest: &mut Dom, source: &Dom) -> Result<(), DomError> {
        let templates: Vec<_> = source.nodes.iter().enumerate()
            .filter_map(|(index, node)| {
                let id = NodeIdx(index as u32);
                (source.node_generation(id).is_some() && source.is_html_template(id) && (node.template_insertion.is_some()
                    || self.map.contains_key(&id))).then_some(id)
            }).collect();
        for template in templates {
            let mapped = self.ensure_source_node(dest, source, template)?;
            let insertion = match source.template_insertion(template) {
                Some((target, start, end)) => {
                    let target = self.ensure_source_node(dest, source, target)?;
                    let start = self.ensure_source_node(dest, source, start)?;
                    let end = end.map(|node| self.ensure_source_node(dest, source, node)).transpose()?;
                    Some((target, start, end))
                }
                None => None,
            };
            dest.set_template_insertion(mapped, insertion)?;
        }
        Ok(())
    }

    fn reverse_template_insertions(&mut self, source: &mut Dom, dest: &Dom) -> Result<(), DomError> {
        let templates: Vec<_> = dest.nodes.iter().enumerate()
            .filter_map(|(index, node)| {
                let id = NodeIdx(index as u32);
                (dest.node_generation(id).is_some() && dest.is_html_template(id) && (node.template_insertion.is_some()
                    || self.rev.contains_key(&id))).then_some(id)
            }).collect();
        for template in templates {
            let Some(mapped) = self.mirror_dest_node(source, dest, template)? else { continue; };
            let insertion = match dest.template_insertion(template) {
                Some((target, start, end)) => {
                    let target = self.mirror_dest_node(source, dest, target)?;
                    let start = self.mirror_dest_node(source, dest, start)?;
                    let end = match end {
                        Some(node) => self.mirror_dest_node(source, dest, node)?,
                        None => None,
                    };
                    target.zip(start).map(|(target, start)| (target, start, end))
                }
                None => None,
            };
            source.set_template_insertion(mapped, insertion)?;
        }
        Ok(())
    }

    fn sync_values(&self, dest: &mut Dom, source: &Dom) -> Result<(), DomError> {
        for (&s, &d) in &self.map { Self::copy_node_values(dest, d, source, s)?; }
        Ok(())
    }

    /// Mirror values through mutation APIs while retaining mapped identities.
    /// A parser continuation usually appends to an existing Text node; applying
    /// its changed middle, rather than resetting all data, preserves unaffected
    /// live Range offsets. Compare exact units, including lone surrogates.
    fn copy_node_values(dest: &mut Dom, d: NodeIdx, source: &Dom, s: NodeIdx) -> Result<(), DomError> {
        source.check_node(s)?;
        dest.check_node(d)?;
        if let Some(value) = source.character_data_utf16(s) {
            let old = dest.character_data_utf16(d).ok_or(DomError::InvalidNodeTypeError)?;
            if old != value {
                let prefix = old.iter().zip(&value).take_while(|(a,b)| a == b).count();
                let suffix = old[prefix..].iter().rev().zip(value[prefix..].iter().rev())
                    .take_while(|(a,b)| a == b).count();
                dest.replace_character_data(d, prefix, old.len()-prefix-suffix,
                    &value[prefix..value.len()-suffix])?;
            }
        } else if matches!(source.get(s).kind, NodeKind::Element { .. }) {
            let attributes = source.attributes_expanded(s).unwrap();
            let removed: Vec<_> = dest.attributes_expanded(d).ok_or(DomError::InvalidNodeTypeError)?
                .into_iter().filter(|old| !attributes.iter().any(|a|
                    a.namespace_uri == old.namespace_uri && a.local_name == old.local_name))
                .map(|a| (a.namespace_uri.map(str::to_owned), a.local_name.to_owned())).collect();
            for (uri, local) in removed { dest.remove_attribute_ns(d, uri.as_deref(), &local)?; }
            for a in attributes {
                let value = source.attribute_ns_utf16(s, a.namespace_uri, a.local_name)
                    .unwrap_or_else(|| a.value.encode_utf16().collect());
                if dest.attribute_ns_utf16(d, a.namespace_uri, a.local_name).as_ref() == Some(&value) { continue; }
                let display = String::from_utf16_lossy(&value);
                dest.set_attribute_expanded(d, a.namespace_uri, a.prefix, a.local_name, &display)?;
                if display.encode_utf16().ne(value.iter().copied()) {
                    let attr = dest.get_attribute_node(d, a.namespace_uri, a.local_name)
                        .ok_or(DomError::InvalidNode)?;
                    dest.store_attr_value_units(attr, value)?;
                }
            }
        }
        Ok(())
    }

    /// Apply only validated placements. A final tree may require moving a
    /// descendant out before its former ancestor can move underneath it, or
    /// clearing an old document element before inserting its replacement.
    /// Retry deferred operations after progress; never bypass DOM validation.
    fn sync_positions(&mut self, dest: &mut Dom, source: &Dom) -> Result<(), DomError> {
        let mut remaining: Vec<_> = self.map.iter().filter(|&(&s, _)|
            source.get(s).parent.is_some() || (!matches!(source.get(s).kind,
                NodeKind::Document | NodeKind::DocumentFragment))).map(|(&s, &d)| (s, d)).collect();
        loop {
            let mut progressed = false;
            let mut deferred = Vec::new();
            for (s, d) in remaining {
                let Some(parent) = source.get(s).parent else {
                    if dest.get(d).parent.is_some() { dest.detach(d); progressed = true; }
                    continue;
                };
                let target = self.ensure_source_node(dest, source, parent)?;
                let before = source.get(s).next_sibling.and_then(|n| self.map.get(&n).copied())
                    .filter(|&n| dest.get(n).parent == Some(target));
                if dest.get(d).parent == Some(target) { continue; }
                match dest.pre_insert(d, target, before) {
                    Ok(_) => progressed = true,
                    Err(DomError::HierarchyRequestError | DomError::NotFoundError) => deferred.push((s, d)),
                    Err(error) => return Err(error),
                }
            }
            if deferred.is_empty() || !progressed { break; }
            remaining = deferred;
        }
        // Once parent moves have settled, order each sibling list backwards.
        // Sorting arbitrary map entries can disturb an adjacency established
        // earlier in that same pass. Reverse tree order gives a fixed suffix.
        let parents: Vec<_> = self.map.iter().map(|(&s, &d)| (s, d)).collect();
        for (s, d) in parents {
            let children: Vec<_> = source.children(s).collect();
            let mut before = None;
            for child in children.into_iter().rev() {
                let Some(&mapped) = self.map.get(&child) else { continue; };
                if dest.get(mapped).parent != Some(d) { continue; }
                if dest.get(mapped).next_sibling != before {
                    match dest.pre_insert(mapped, d, before) {
                        Ok(_) => (),
                        Err(DomError::HierarchyRequestError | DomError::NotFoundError) => continue,
                        Err(error) => return Err(error),
                    }
                }
                before = Some(mapped);
            }
        }
        Ok(())
    }

    fn ensure_source_node(&mut self, dest: &mut Dom, source: &Dom, node: NodeIdx) -> Result<NodeIdx, DomError> {
        if let Some(&mapped) = self.map.get(&node) { return Ok(mapped); }
        if let Some(template) = source.template_of_contents(node) {
            self.ensure_source_node(dest, source, template)?;
            self.sync_special_containers(dest, source, template)?;
            return self.map.get(&node).copied().ok_or(DomError::InvalidNode);
        }
        if let Some(data) = source.shadow_data(node) {
            self.ensure_source_node(dest, source, data.host)?;
            self.sync_special_containers(dest, source, data.host)?;
            return self.map.get(&node).copied().ok_or(DomError::InvalidNode);
        }
        if matches!(source.get(node).kind, NodeKind::Document) {
            let document = dest.create_document();
            dest.set_document_quirks_mode(document, source.document_quirks_mode(node)?)?;
            dest.set_document_scripting_enabled(document, source.document_scripting_enabled(node)?)?;
            dest.set_document_allows_declarative_shadow_roots(document,
                source.document_allows_declarative_shadow_roots(node)?)?;
            if let Some(metadata) = source.document_metadata(node)? {
                dest.set_document_metadata(document, metadata.clone())?;
            }
            self.record(source, dest, node, document);
            self.sync_children(dest, source, node, document)?;
            return Ok(document);
        }
        if matches!(source.get(node).kind, NodeKind::DocumentFragment) {
            let document = self.ensure_source_node(dest, source,
                source.owner_document(node).ok_or(DomError::InvalidNode)?)?;
            if let Some(&mapped) = self.map.get(&node) { return Ok(mapped); }
            let created = dest.create_document_fragment(document)?;
            self.record(source, dest, node, created);
            self.sync_children(dest, source, node, created)?;
            return Ok(created);
        }
        let source_parent = source.get(node).parent;
        let parent = if let Some(parent) = source_parent {
            self.ensure_source_node(dest, source, parent)?
        } else {
            self.ensure_source_node(dest, source, source.owner_document(node).ok_or(DomError::NotSupportedError)?)?
        };
        // Resolving a new parent can synchronize its subtree and already
        // record this exact child. Never allocate a second identity.
        if let Some(&mapped) = self.map.get(&node) { return Ok(mapped); }
        // Allocate first; its placement is validated by the reconciliation
        // pass after old document children and ancestor relationships settle.
        let created = self.import_one(dest, source, node, parent, false)?;
        self.record(source, dest, node, created);
        self.sync_children(dest, source, node, created)?;
        self.sync_special_containers(dest, source, node)?;
        Ok(created)
    }

    /// Imports every not-yet-copied direct child of `source_parent`, then
    /// recurses into EVERY child (freshly imported or not) -- an
    /// already-open element can gain more children between two `sync`
    /// calls, so this must walk the whole subtree every time, not just
    /// the newly created part of it, to find them. `sync_special_containers`
    /// handles the two cases a child's own children are not enough for: a
    /// shadow root (attached to its host via a side table, not a `Dom`
    /// child at all) and a template's contents fragment (same).
    fn sync_children(&mut self, dest: &mut Dom, source: &Dom,
        source_parent: NodeIdx, dest_parent: NodeIdx) -> Result<(), DomError> {
        for child in source.children(source_parent) {
            let dest_child = match self.map.get(&child) {
                Some(existing) => *existing,
                None => {
                    let created = self.import_one(dest, source, child, dest_parent, true)?;
                    self.record(source, dest, child, created);
                    created
                }
            };
            self.sync_children(dest, source, child, dest_child)?;
            self.sync_special_containers(dest, source, child)?;
        }
        Ok(())
    }

    /// `child`/`dest_child`'s shadow root and/or (if `child` is a
    /// `<template>`) contents fragment, if it has either -- both created
    /// once, at `import_one` time (below), the FIRST time `child` itself was
    /// imported, but their own children still need the same incremental
    /// treatment `sync_children` gives every ordinary node, on every call,
    /// whether `child` was imported just now or several `sync`s ago.
    fn sync_special_containers(&mut self, dest: &mut Dom, source: &Dom,
        node: NodeIdx) -> Result<(), DomError> {
        if let Some(root) = source.shadow_root_of_host(node) {
            let dest_root = if let Some(&mapped) = self.map.get(&root) { mapped } else {
                let host = self.map[&node];
                let data = source.shadow_data(root).expect("attached source shadow root");
                let created = dest.attach_shadow(host, data.options.clone())?;
                let copied = dest.shadow_roots.iter_mut().find(|data| data.root == created)
                    .expect("new mirrored shadow root");
                copied.declarative = data.declarative;
                copied.available_to_element_internals = data.available_to_element_internals;
                copied.keep_custom_element_registry_null = data.keep_custom_element_registry_null;
                self.record(source, dest, root, created);
                created
            };
            self.sync_children(dest, source, root, dest_root)?;
        }
        if source.is_html_template(node) {
            if let Some(contents) = source.template_contents(node) {
                let dest_contents = if let Some(&mapped) = self.map.get(&contents) { mapped } else {
                    let created = dest.ensure_template_contents(self.map[&node])?;
                    self.record(source, dest, contents, created);
                    created
                };
                self.sync_children(dest, source, contents, dest_contents)?;
            }
        }
        Ok(())
    }

    fn import_one(&mut self, dest: &mut Dom, source: &Dom, node: NodeIdx, dest_parent: NodeIdx, attach: bool)
        -> Result<NodeIdx, DomError> {
        let dest_document = dest.owner_document(dest_parent).unwrap_or(dest_parent);
        let created = match &source.get(node).kind {
            NodeKind::Element { .. } => {
                let created = dest.create_element_expanded_in(dest_document, source.namespace_uri(node),
                    source.prefix(node), source.local_name(node).unwrap())?;
                created
            }
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } | NodeKind::Comment { .. } => {
                let kind = match &source.get(node).kind {
                    NodeKind::Text { .. } => NodeKind::Text { data: String::new() },
                    NodeKind::CDATASection { .. } => NodeKind::CDATASection { data: String::new() },
                    _ => NodeKind::Comment { data: String::new() },
                };
                let created = dest.create_node_in(dest_document, kind)?;
                let units = source.character_data_utf16(node).unwrap_or_default();
                dest.replace_character_data(created, 0, 0, &units)?;
                created
            }
            NodeKind::ProcessingInstruction { target, data } =>
                dest.create_node_in(dest_document, NodeKind::ProcessingInstruction { target: target.clone(), data: data.clone() })?,
            NodeKind::DocumentType { name, public_id, system_id } =>
                dest.create_node_in(dest_document, NodeKind::DocumentType {
                    name: name.clone(), public_id: public_id.clone(), system_id: system_id.clone(),
                })?,
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Attr { .. } =>
                return Err(DomError::NotSupportedError),
        };
        if attach { dest.pre_insert(created, dest_parent, None)?; }
        Self::copy_node_values(dest, created, source, node)?;
        if let Some(root) = source.shadow_root_of_host(node) {
            let source_data = source.shadow_data(root).expect("attached parser shadow root").clone();
            let copied_root = dest.attach_shadow(created, source_data.options)?;
            let copied_data = dest.shadow_roots.iter_mut().find(|data| data.root == copied_root)
                .expect("new imported shadow root");
            copied_data.declarative = source_data.declarative;
            copied_data.available_to_element_internals = source_data.available_to_element_internals;
            copied_data.keep_custom_element_registry_null = source_data.keep_custom_element_registry_null;
            self.record(source, dest, root, copied_root);
        }
        if source.is_html_template(node) {
            let destination = dest.ensure_template_contents(created)?;
            if let Some(contents) = source.template_contents(node) {
                self.record(source, dest, contents, destination);
            }
        }
        Ok(created)
    }

    /// The reverse of `sync`: makes this arena's own private `source`
    /// mirror whatever the shared, already-grafted subtree currently looks
    /// like in `dest`, for every node this import has ever copied there.
    ///
    /// `sync` only ever copies `source` forward INTO `dest`; a script that
    /// runs while a `ResumableParser` is paused mutates `dest` directly
    /// (the real, shared arena the JS host and every other frame sees), and
    /// without this, `source`'s own tree-construction algorithms -- the
    /// adoption agency algorithm's cycle/document-full checks chief among
    /// them (`amazing-html`'s `tree.rs`) -- keep reasoning about `source`'s
    /// STALE, pre-mutation shape once the parser resumes, since nothing
    /// ever told `source` about the mutation. Call this once, right after
    /// running a script and before resuming the parser.
    ///
    /// Two passes: first, every already-mapped node is re-parented in
    /// `source` to match wherever its `dest` counterpart currently lives
    /// (needed for the common "script reparents an existing node" case --
    /// the cycle tests this exists for). Second, every mapped node's
    /// current `dest` children are walked and any not-yet-mirrored one --
    /// a script-created element, or new content placed straight into a
    /// mirrored shadow root or template's contents -- gets a brand-new
    /// `source` counterpart too, so a cycle or host-including-ancestor
    /// relationship that only exists through such brand-new structure
    /// (the shadow-root/template-contents cycle tests) is visible to
    /// `source` as well, not just moves of pre-existing nodes.
    ///
    /// Other Documents and detached containers receive private mirrors with
    /// the actual context metadata. This is an arena synchronization operation,
    /// not Document.cloneNode: it must preserve active/inert context for parser
    /// validation without changing the destination's browsing context.
    pub fn reverse_sync(&mut self, source: &mut Dom, dest: &Dom) -> Result<(), DomError> {
        self.validate_mappings(source, dest)?;
        let pairs: Vec<(NodeIdx, NodeIdx)> = self.map.iter().map(|(&s, &d)| (s, d)).collect();
        for &(s, d) in &pairs {
            Self::copy_node_values(source, s, dest, d)?;
            if matches!(dest.get(d).kind, NodeKind::Document) {
                source.set_document_allows_declarative_shadow_roots(s,
                    dest.document_allows_declarative_shadow_roots(d)?)?;
            }
        }
        // `dest`'s current shape was reached by some sequence of
        // individually-valid DOM mutations (a real `Dom` can never hold an
        // actual cycle), so *some* order exists in which replaying every
        // node's move in `source` is itself never momentarily cyclic --
        // but `pairs`' own (hash-map, so arbitrary) order is not
        // necessarily that order. Retry whatever doesn't fit yet -- e.g.
        // moving A under B before B has itself been moved out from under
        // A -- until nothing more can progress.
        let mut remaining = pairs.clone();
        loop {
            let mut progressed = false;
            let mut deferred = Vec::new();
            for (source_node, dest_node) in remaining {
                if matches!(dest.get(dest_node).kind, NodeKind::Document | NodeKind::DocumentFragment)
                    && dest.get(dest_node).parent.is_none() { continue; }
                let target = match dest.get(dest_node).parent {
                    None => None,
                    Some(parent) => self.mirror_dest_node(source, dest, parent)?,
                };
                if source.get(source_node).parent == target { continue; }
                if let Some(t) = target {
                    // Host-including, not just `Dom::contains`'s ordinary
                    // parent-chain check: `shadow-cycle`/`template-cycle`
                    // construct exactly a *host-including* cycle (through a
                    // shadow root's host, or a template's owning template),
                    // which `attach_before`'s own hard cycle assert (also
                    // ordinary-only) would not catch either -- deferring
                    // here, same as an ordinary would-be cycle, is what lets
                    // the surrounding retry loop's "give up rather than
                    // loop forever" fallback apply to this case too, instead
                    // of physically constructing a live cycle in `source`
                    // that `adoption_agency`'s own cycle guard would then
                    // have to detect (or hang trying to, absent HTML-40's
                    // own fix to `host_including_ancestors`).
                    if source.is_host_including_inclusive_ancestor(source_node, t) {
                        deferred.push((source_node, dest_node));
                        continue;
                    }
                }
                source.detach(source_node);
                if let Some(parent) = target {
                    let document = source.owner_document(parent).unwrap_or(parent);
                    source.adopt(source_node, document)?;
                    source.attach_before(parent, None, source_node);
                }
                progressed = true;
            }
            if deferred.is_empty() { break; }
            if !progressed {
                // No order made progress this round -- `dest` itself would
                // have to be cyclic for that to happen, which a real `Dom`
                // cannot be; give up on the rest rather than looping
                // forever, leaving them at their last-known `source`
                // position instead.
                break;
            }
            remaining = deferred;
        }
        let containers: Vec<_> = self.map.iter().filter(|&(_, &d)|
            dest.host_including_parent(d).is_none()).map(|(&s, &d)| (s, d)).collect();
        for (source_node, dest_node) in &containers {
            self.reverse_sync_children(source, dest, *dest_node, *source_node)?;
            self.reverse_sync_special_containers(source, dest, *dest_node)?;
        }
        self.reverse_template_insertions(source, dest)?;
        Ok(())
    }

    /// Resolves `node` (a `dest`-side node -- an ordinary element/text/etc,
    /// or itself a shadow root or a template's contents fragment) to the
    /// `source`-side node it should be mirrored as, preserving document and
    /// detached-container boundaries. Host-including cycles remain unmirrored.
    fn mirror_dest_node(&mut self, source: &mut Dom, dest: &Dom, node: NodeIdx)
        -> Result<Option<NodeIdx>, DomError> {
        let mut resolving = Vec::new();
        self.mirror_dest_node_rec(source, dest, node, &mut resolving)
    }

    /// `mirror_dest_node`'s actual recursion, with `resolving` tracking the
    /// node's own currently-in-progress ancestor chain. HTML-40 (1)
    /// follow-up: a live `Dom` can never hold an *ordinary* parent-child
    /// cycle (`attach_before`'s own hard assert), but the host-including
    /// hops this walks through (shadow root -> host, template contents ->
    /// owning template) are not parent-child edges that assert ever sees --
    /// a script can construct a live *host-including* cycle at `dest`
    /// (`adoption-agency-reparenting-shadow-cycle.html` does exactly this:
    /// the shadow host ends up an ordinary descendant of a node inside its
    /// own shadow root's content). Resolving such a node's mirror walks
    /// shadow-root -> host -> ... -> the very same shadow root, forever,
    /// hanging the whole harness process with no script-level timeout ever
    /// getting a chance to fire (this is one synchronous native call, never
    /// yielding). Detect the re-entrancy and give up on this node's mirror
    /// (`None`) instead -- the same "give up rather than loop forever"
    /// choice `reverse_sync`'s own reparent-order retry loop already makes,
    /// and harmless here: `adoption_agency`'s cycle guard is exactly what
    /// will (correctly) refuse to reattach a node in this shape once the
    /// parser resumes.
    fn mirror_dest_node_rec(&mut self, source: &mut Dom, dest: &Dom, node: NodeIdx,
        resolving: &mut Vec<NodeIdx>) -> Result<Option<NodeIdx>, DomError> {
        if let Some(&mapped) = self.rev.get(&node) { return Ok(Some(mapped)); }
        if resolving.contains(&node) { return Ok(None); }
        resolving.push(node);
        let result = self.mirror_dest_node_body(source, dest, node, resolving);
        resolving.pop();
        result
    }

    fn mirror_dest_node_body(&mut self, source: &mut Dom, dest: &Dom, node: NodeIdx,
        resolving: &mut Vec<NodeIdx>) -> Result<Option<NodeIdx>, DomError> {
        if let Some(data) = dest.shadow_data(node) {
            let (host, options) = (data.host, data.options.clone());
            let (declarative, available, keep_registry_null) = (data.declarative,
                data.available_to_element_internals, data.keep_custom_element_registry_null);
            let Some(source_host) = self.mirror_dest_node_rec(source, dest, host, resolving)? else { return Ok(None) };
            let source_root = match source.shadow_root_of_host(source_host) {
                Some(existing) => existing,
                None => {
                    let created = source.attach_shadow(source_host, options)?;
                    let copied = source.shadow_roots.iter_mut().find(|d| d.root == created)
                        .expect("just-attached mirrored shadow root");
                    copied.declarative = declarative;
                    copied.available_to_element_internals = available;
                    copied.keep_custom_element_registry_null = keep_registry_null;
                    created
                }
            };
            self.record(source, dest, source_root, node);
            return Ok(Some(source_root));
        }
        if let Some(template) = dest.template_of_contents(node) {
            let Some(source_template) = self.mirror_dest_node_rec(source, dest, template, resolving)? else { return Ok(None) };
            let source_contents = source.ensure_template_contents(source_template)?;
            self.record(source, dest, source_contents, node);
            return Ok(Some(source_contents));
        }
        if matches!(dest.get(node).kind, NodeKind::Document) {
            let document = source.create_document();
            source.set_document_quirks_mode(document, dest.document_quirks_mode(node)?)?;
            source.set_document_scripting_enabled(document, dest.document_scripting_enabled(node)?)?;
            source.set_document_allows_declarative_shadow_roots(document,
                dest.document_allows_declarative_shadow_roots(node)?)?;
            if let Some(metadata) = dest.document_metadata(node)? { source.set_document_metadata(document, metadata.clone())?; }
            self.record(source, dest, document, node);
            return Ok(Some(document));
        }
        let parent = dest.get(node).parent;
        let container = parent.or_else(|| dest.owner_document(node)).ok_or(DomError::NotSupportedError)?;
        let Some(source_parent) = self.mirror_dest_node_rec(source, dest, container, resolving)? else { return Ok(None) };
        if matches!(dest.get(node).kind, NodeKind::DocumentFragment) {
            let document = source.owner_document(source_parent).unwrap_or(source_parent);
            let fragment = source.create_document_fragment(document)?;
            self.record(source, dest, fragment, node);
            return Ok(Some(fragment));
        }
        let created = self.reverse_import_one(source, dest, node, source_parent)?;
        if parent.is_none() { if let Some(created) = created { source.detach(created); } }
        Ok(created)
    }

    /// Mirrors every not-yet-mirrored direct child of `dest_parent`
    /// (recursing into every child, mirrored just now or earlier, exactly
    /// like `sync_children`'s own forward walk) into `source`, under
    /// `source_parent`.
    fn reverse_sync_children(&mut self, source: &mut Dom, dest: &Dom,
        dest_parent: NodeIdx, source_parent: NodeIdx) -> Result<(), DomError> {
        for child in dest.children(dest_parent) {
            let source_child = match self.rev.get(&child) {
                Some(&existing) => existing,
                None => match self.reverse_import_one(source, dest, child, source_parent)? {
                    Some(created) => created,
                    None => continue,
                },
            };
            if source.get(source_child).parent == Some(source_parent) {
                source.detach(source_child);
                source.attach_before(source_parent, None, source_child);
            }
            self.reverse_sync_children(source, dest, child, source_child)?;
            self.reverse_sync_special_containers(source, dest, child)?;
        }
        Ok(())
    }

    /// The reverse of `sync_special_containers`: `node`'s shadow root
    /// and/or template contents, mirrored already (by `mirror_dest_node`,
    /// as an ancestor-resolution side effect) or not at all -- either way
    /// there is nothing to *create* here, only to recurse into if it
    /// exists.
    fn reverse_sync_special_containers(&mut self, source: &mut Dom, dest: &Dom,
        node: NodeIdx) -> Result<(), DomError> {
        if let Some(root) = dest.shadow_root_of_host(node) {
            if let Some(source_root) = self.mirror_dest_node(source, dest, root)? {
                self.reverse_sync_children(source, dest, root, source_root)?;
            }
        }
        if dest.is_html_template(node) {
            if let Some(contents) = dest.template_contents(node) {
                if let Some(source_contents) = self.mirror_dest_node(source, dest, contents)? {
                    self.reverse_sync_children(source, dest, contents, source_contents)?;
                }
            }
        }
        Ok(())
    }

    /// Creates a brand-new `source` mirror of the single `dest` node
    /// `node` (an ordinary element/text/comment/etc -- never a shadow root
    /// or template-contents fragment, which `mirror_dest_node` handles on
    /// its own), as a child of `source_parent`. `None` for a `dest` node
    /// kind with no meaningful mirror (a `Document`, a `DocumentFragment`
    /// that is not a template's contents, or an `Attr`) -- its subtree, if
    /// any, simply stays invisible to `source` rather than erroring the
    /// whole reverse-sync over one such node.
    fn reverse_import_one(&mut self, source: &mut Dom, dest: &Dom, node: NodeIdx, source_parent: NodeIdx)
        -> Result<Option<NodeIdx>, DomError> {
        let source_document = source.owner_document(source_parent).unwrap_or(source_parent);
        let created = match &dest.get(node).kind {
            NodeKind::Element { .. } => {
                let created = source.create_element_expanded_in(source_document, dest.namespace_uri(node),
                    dest.prefix(node), dest.local_name(node).unwrap())?;
                created
            }
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } | NodeKind::Comment { .. } => {
                let kind = match &dest.get(node).kind {
                    NodeKind::Text { .. } => NodeKind::Text { data: String::new() },
                    NodeKind::CDATASection { .. } => NodeKind::CDATASection { data: String::new() },
                    _ => NodeKind::Comment { data: String::new() },
                };
                let created = source.create_node_in(source_document, kind)?;
                let units = dest.character_data_utf16(node).unwrap_or_default();
                source.replace_character_data(created, 0, 0, &units)?;
                created
            }
            NodeKind::ProcessingInstruction { target, data } =>
                source.create_node_in(source_document, NodeKind::ProcessingInstruction { target: target.clone(), data: data.clone() })?,
            NodeKind::DocumentType { name, public_id, system_id } =>
                source.create_node_in(source_document, NodeKind::DocumentType {
                    name: name.clone(), public_id: public_id.clone(), system_id: system_id.clone(),
                })?,
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Attr { .. } => return Ok(None),
        };
        Self::copy_node_values(source, created, dest, node)?;
        source.attach_before(source_parent, None, created);
        self.record(source, dest, created, node);
        Ok(Some(created))
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::*;
    #[test]
    fn html_serialization_normalizes_namespace_names_and_attribute_escaping() {
        let mut dom = Dom::new(); let document = NodeIdx(0);
        let svg = dom.create_element_ns_in(document,Some("http://www.w3.org/2000/svg"),"odd:svg").unwrap();
        dom.set_attribute_expanded(svg,Some("http://www.w3.org/XML/1998/namespace"),Some("other"),"lang","<&>\"\u{a0}").unwrap();
        dom.set_attribute_expanded(svg,Some("http://www.w3.org/1999/xlink"),Some("other"),"href","next").unwrap();
        dom.set_attribute_ns(svg,Some("http://www.w3.org/2000/xmlns/"),"xmlns","http://www.w3.org/2000/svg").unwrap();
        let child = dom.create_element_ns_in(document,Some("urn:other"),"p:item").unwrap();dom.pre_insert(child,svg,None).unwrap();
        assert_eq!(String::from_utf16(&dom.serialize_node(svg).unwrap()).unwrap(),
            "<svg xml:lang=\"&lt;&amp;&gt;&quot;&nbsp;\" xlink:href=\"next\" xmlns=\"http://www.w3.org/2000/svg\"><p:item></p:item></svg>");
    }
    #[test]
    fn serialization_preserves_utf16_data_and_uses_actual_document_scripting() {
        let mut dom = Dom::new();let doc = NodeIdx(0);let div = dom.create_element(doc,"div");
        dom.set_attribute(div,"value","").unwrap();let attr = dom.get_attribute_node(div,None,"value").unwrap();
        dom.set_attr_value(attr,vec![0xD800,60,62]).unwrap();
        let comment = dom.create_node_in(doc,NodeKind::Comment{data:String::new()}).unwrap();dom.pre_insert(comment,div,None).unwrap();
        dom.replace_character_data(comment,0,0,&[0xD800]).unwrap();
        let pi = dom.create_node_in(doc,NodeKind::ProcessingInstruction{target:"target".into(),data:String::new()}).unwrap();dom.pre_insert(pi,div,None).unwrap();
        dom.replace_character_data(pi,0,0,&[0xDC00]).unwrap();
        let mut expected = "<div value=\"".encode_utf16().collect::<Vec<_>>();expected.push(0xD800);
        expected.extend("&lt;&gt;\"><!--".encode_utf16());expected.push(0xD800);
        expected.extend("--><?target ".encode_utf16());expected.push(0xDC00);expected.extend("?></div>".encode_utf16());
        assert_eq!(dom.serialize_node(div).unwrap(),expected);
        let noscript = dom.create_element(doc,"noscript");dom.append_child(noscript,NodeKind::Text{data:"<&>".into()});
        assert_eq!(dom.serialize_children(noscript).unwrap(),"&lt;&amp;&gt;".encode_utf16().collect::<Vec<_>>());
        dom.set_document_scripting_enabled(doc,true).unwrap();
        assert_eq!(dom.serialize_children(noscript).unwrap(),"<&>".encode_utf16().collect::<Vec<_>>());
        let cdata = dom.create_node_in(doc,NodeKind::CDATASection{data:"<img>".into()}).unwrap();
        assert_eq!(dom.serialize_node(cdata).unwrap(),"&lt;img&gt;".encode_utf16().collect::<Vec<_>>());
    }
}
