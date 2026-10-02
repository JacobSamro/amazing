//! HTML fragment serialization and transfer from a parser-owned arena.
use crate::{Dom, DomError, NodeIdx, NodeKind};
use std::collections::HashMap;

/// Whether walking `node`'s host-including ancestor chain (ordinary DOM
/// parent, *or* a shadow root's hop to its host, *or* a template's
/// `content` fragment's hop to its owning `<template>` -- see `Dom::
/// host_including_parent`) reaches the arena's own document root
/// (`NodeIdx(0)`, per `ResumableParser`'s own contract -- see
/// `IncrementalDocumentImport::new`). `false` for a node that is either
/// plainly detached (no parent and no host-including hop) or attached
/// only to a floating subtree that itself never made it back into the
/// document (a formatting-element clone the adoption agency algorithm's
/// inner loop created and never attached anywhere is exactly this shape:
/// the dropped node ends up with a real `parent`, just not one that leads
/// anywhere). The host-including hop matters here as much as it does for
/// the cycle guard itself: a template's `content` fragment root, or a
/// shadow root, has no ordinary `parent` at all -- it is a root of its
/// own arena-local tree -- so a plain parent-chain walk would call *every*
/// such fragment "unreachable" even while its owning template/host is very
/// much attached, and wrongly detach its `dest` counterpart.
///
/// HTML-40 (1) follow-up: guards against revisiting a node already seen on
/// this same walk. `source` should never itself end up with a live host-
/// including cycle (`reverse_sync`'s own mirroring gives up, rather than
/// completing one, when `dest` has one -- see `mirror_dest_node_rec`), but
/// this walk is cheap to make cycle-safe on its own terms too rather than
/// lean entirely on that other function never regressing.
fn source_reaches_document_root(dom: &Dom, node: NodeIdx) -> bool {
    let mut seen = Vec::new();
    let mut cursor = Some(node);
    while let Some(id) = cursor {
        if id == NodeIdx(0) { return true; }
        if seen.contains(&id) { return false; }
        seen.push(id);
        cursor = dom.host_including_parent(id);
    }
    false
}

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
/// Known limitation: HTML tree construction can *move* an already-open node
/// to a new parent mid-parse (the adoption agency algorithm reconstructing
/// active formatting elements; table foster parenting). If such a move
/// happens to a node already imported in an earlier `sync` call, this does
/// not re-parent its destination counterpart to match -- a real but narrow
/// gap (mis-nested formatting elements or foster-parented content that also
/// straddles a parser-blocking script pause), left undone rather than
/// silently assumed away; see the module-level test covering it.
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
        Self { dest_document, map, rev }
    }

    /// Records a newly-copied pair in both directions.
    fn record(&mut self, source_node: NodeIdx, dest_node: NodeIdx) {
        self.map.insert(source_node, dest_node);
        self.rev.insert(dest_node, source_node);
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
    /// a call with nothing new to copy is a no-op.
    pub fn sync(&mut self, dest: &mut Dom, source: &Dom) -> Result<(), DomError> {
        self.sync_children(dest, source, NodeIdx(0), self.dest_document)?;
        self.detach_unreachable(dest, source);
        Ok(())
    }

    // A symmetric forward-direction "reparent an already-mapped node to
    // match source, creating any not-yet-mirrored new parent along the
    // way" pass (mirroring `reverse_sync`'s own reparent pass) was tried
    // here and reverted (HTML-40): it fixed one `adoption-agency-
    // reparenting.html` subtest (a furthest block adopted into another
    // Document, then reconstructed normally) but regressed `adoption-
    // agency-reparenting-inner-loop-cycle.html` from passing to a
    // `HierarchyRequestError` -- the two `pre_insert` validation and the
    // retry-order guarantee that make `reverse_sync`'s own version safe
    // turned out not to transfer cleanly to `dest`, which (unlike
    // `source`) is also live for every sibling frame and the parent page
    // at the same time. The explicitly-named priority for this ticket is
    // the cycle tests actually passing, so this stayed reverted rather
    // than risk that regression; the `adoption-agency-reparenting.html`
    // cross-document subtests remain unfixed (see html_TODO.md).

    /// A tree-construction step (the adoption agency algorithm's
    /// cycle/document-full guards chief among them) can decide to leave an
    /// already-imported node unreachable from the document root in
    /// `source` -- fully detached, or hanging off a floating
    /// formatting-element clone the inner loop created and never attached
    /// anywhere -- rather than move it somewhere. `sync_children`'s walk,
    /// being additions-only and reachability-driven, never visits such a
    /// node again to notice this, so `dest`'s counterpart would otherwise
    /// keep sitting wherever it last was, forever. Detach it there too.
    fn detach_unreachable(&mut self, dest: &mut Dom, source: &Dom) {
        let pairs: Vec<(NodeIdx, NodeIdx)> = self.map.iter()
            .filter(|&(&s, _)| s != NodeIdx(0)).map(|(&s, &d)| (s, d)).collect();
        for (source_node, dest_node) in pairs {
            if !source_reaches_document_root(source, source_node) && dest.get(dest_node).parent.is_some() {
                dest.detach(dest_node);
            }
        }
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
                    let created = self.import_one(dest, source, child, dest_parent)?;
                    self.record(child, created);
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
            let dest_root = *self.map.get(&root).expect("shadow root mapped at host's own import_one");
            self.sync_children(dest, source, root, dest_root)?;
        }
        if source.is_html_template(node) {
            if let Some(contents) = source.template_contents(node) {
                let dest_contents = *self.map.get(&contents)
                    .expect("template contents mapped at template's own import_one");
                self.sync_children(dest, source, contents, dest_contents)?;
            }
        }
        Ok(())
    }

    fn import_one(&mut self, dest: &mut Dom, source: &Dom, node: NodeIdx, dest_parent: NodeIdx)
        -> Result<NodeIdx, DomError> {
        let dest_document = self.dest_document;
        let created = match &source.get(node).kind {
            NodeKind::Element { .. } => {
                let created = dest.create_element_expanded_in(dest_document, source.namespace_uri(node),
                    source.prefix(node), source.local_name(node).unwrap())?;
                for attr in source.attributes_expanded(node).unwrap() {
                    dest.set_attribute_expanded(created, attr.namespace_uri, attr.prefix,
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
        dest.pre_insert(created, dest_parent, None)?;
        if let Some(root) = source.shadow_root_of_host(node) {
            let source_data = source.shadow_data(root).expect("attached parser shadow root").clone();
            let copied_root = dest.attach_shadow(created, source_data.options)?;
            let copied_data = dest.shadow_roots.iter_mut().find(|data| data.root == copied_root)
                .expect("new imported shadow root");
            copied_data.declarative = source_data.declarative;
            copied_data.available_to_element_internals = source_data.available_to_element_internals;
            copied_data.keep_custom_element_registry_null = source_data.keep_custom_element_registry_null;
            self.record(root, copied_root);
        }
        if source.is_html_template(node) {
            let destination = dest.ensure_template_contents(created)?;
            if let Some(contents) = source.template_contents(node) {
                self.record(contents, destination);
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
    /// A move to a location with no possible mirror -- script moved the
    /// node into a *different* `Document` entirely (`dest.host_including_
    /// parent` walks off the top without ever reaching something already
    /// mapped) -- has nothing in `source` it could be mirrored into, since
    /// `source` has no notion of any document but its own. The
    /// corresponding `source` node is left/made detached instead, which
    /// happens to match what the adoption agency algorithm's own "detach,
    /// then insert at the adjusted insertion location" step needs
    /// regardless of where a node used to be.
    pub fn reverse_sync(&mut self, source: &mut Dom, dest: &Dom) -> Result<(), DomError> {
        let pairs: Vec<(NodeIdx, NodeIdx)> = self.map.iter()
            .filter(|&(&s, _)| s != NodeIdx(0)).map(|(&s, &d)| (s, d)).collect();
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
        for (source_node, dest_node) in &pairs {
            self.reverse_sync_children(source, dest, *dest_node, *source_node)?;
            self.reverse_sync_special_containers(source, dest, *dest_node)?;
        }
        Ok(())
    }

    /// Resolves `node` (a `dest`-side node -- an ordinary element/text/etc,
    /// or itself a shadow root or a template's contents fragment) to the
    /// `source`-side node it should be mirrored as, creating it (and, via
    /// recursion, any not-yet-mirrored ancestor above it) if it has no
    /// mirror yet. `None` means `node` is unreachable from `dest_document`'s
    /// own tree -- it, or an ancestor of it, is not this arena's document at
    /// all -- so there is nothing to mirror it into.
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
            self.record(source_root, node);
            return Ok(Some(source_root));
        }
        if let Some(template) = dest.template_of_contents(node) {
            let Some(source_template) = self.mirror_dest_node_rec(source, dest, template, resolving)? else { return Ok(None) };
            let source_contents = source.ensure_template_contents(source_template)?;
            self.record(source_contents, node);
            return Ok(Some(source_contents));
        }
        let Some(parent) = dest.get(node).parent else { return Ok(None) };
        let Some(source_parent) = self.mirror_dest_node_rec(source, dest, parent, resolving)? else { return Ok(None) };
        self.reverse_import_one(source, dest, node, source_parent)
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
            if let Some(&source_root) = self.rev.get(&root) {
                self.reverse_sync_children(source, dest, root, source_root)?;
            }
        }
        if dest.is_html_template(node) {
            if let Some(contents) = dest.template_contents(node) {
                if let Some(&source_contents) = self.rev.get(&contents) {
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
        let source_document = NodeIdx(0);
        let created = match &dest.get(node).kind {
            NodeKind::Element { .. } => {
                let created = source.create_element_expanded_in(source_document, dest.namespace_uri(node),
                    dest.prefix(node), dest.local_name(node).unwrap())?;
                for attr in dest.attributes_expanded(node).unwrap() {
                    source.set_attribute_expanded(created, attr.namespace_uri, attr.prefix,
                        attr.local_name, attr.value)?;
                }
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
        source.attach_before(source_parent, None, created);
        self.record(created, node);
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
