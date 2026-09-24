//! Amazing DOM — arena-allocated document tree.
//!
//! DOM-2: checked tree algorithms, event dispatch, and mutation observation.
//!
//! Design decisions (locked):
//! - Nodes are slots in a flat `Vec`; references are `NodeIdx` (u32) — no
//!   `Rc<RefCell<..>>`, so no reference cycles and cheap snapshotting.
//! - Single ownership: a node has exactly one parent; first-child /
//!   next-sibling / prev-sibling links give O(1) traversal.
//! - The low-level attach/detach API exists for the HTML tree constructor
//!   (foster parenting, adoption agency). It is engine-internal surface;
//!   Bindings must use checked pre-insert/pre-remove/replace operations.

#![forbid(unsafe_code)]

pub mod events;
pub mod attr;
pub mod collection;
pub mod abort;
pub mod document_surface;
pub mod render_mutation;
pub mod markup;
#[cfg(test)] mod markup_tests;
pub mod event_loop;
pub mod mutation;
pub mod shadow;
pub mod custom_elements;
pub mod forms;
mod template;
#[cfg(test)] mod template_tests;
mod namespace;
mod namespace_lookup;
mod node_mixins;
pub use node_mixins::NodeOrString;
pub use namespace::{AttributeRef, NamespaceId};
pub mod range;
pub mod traversal;
mod tree;
pub use tree::{DocumentPosition, DomError};

/// Index into a [`Dom`] arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct NodeIdx(pub u32);

/// Namespace carried by an element node. The legacy create_element constructor
/// defaults to HTML; namespace-aware constructors preserve null and arbitrary URIs.
/// Other namespace handles, like NodeIdx, belong to a single Dom arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
    /// An element with no namespace (distinct from a non-element).
    Null,
    /// URI interned in the owning Dom; handles must not cross arenas.
    Other(NamespaceId),
}

/// Node kinds. Grows in later phases (attributes live on `Element` since
/// the CSS selector engine needs them).
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Document,
    DocumentFragment,
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element {
        name: String,
        attrs: Vec<(String, String)>,
        namespace: Namespace,
    },
    Text {
        data: String,
    },
    CDATASection {
        data: String,
    },
    Attr {
        local_name: String,
        namespace_uri: Option<String>,
        prefix: Option<String>,
        value: Vec<u16>,
        owner_element: Option<NodeIdx>,
    },
    Comment {
        data: String,
    },
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

#[derive(Debug)]
pub struct Node {
    owner_document: NodeIdx,
    namespaced_attributes: Vec<namespace::NamespacedAttribute>,
    attribute_order: Vec<namespace::AttributeKey>,
    character_data_units: Option<Vec<u16>>,
    prefix: Option<String>,
    pub kind: NodeKind,
    pub parent: Option<NodeIdx>,
    pub first_child: Option<NodeIdx>,
    pub last_child: Option<NodeIdx>,
    pub next_sibling: Option<NodeIdx>,
    pub prev_sibling: Option<NodeIdx>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomExceptionData {
    pub name: Vec<u16>,
    pub message: Vec<u16>,
}

/// Arena-backed DOM.
#[derive(Debug)]
pub struct Dom {
    nodes: Vec<Node>,
    namespace_uris: Vec<String>,
    mutations: mutation::MutationObservers,
    ranges: Vec<Option<range::AbstractRange>>,
    static_ranges: Vec<Option<range::StaticRange>>,
    events: events::EventTargets,
    external_event_targets: u64,
    mutation_records: Vec<mutation::RegisteredRecord>,
    static_node_lists: Vec<Vec<NodeIdx>>,
    collections: Vec<collection::LiveCollection>,
    abort_signals: abort::AbortSignals,
    document_states: Vec<(NodeIdx, document_surface::ReadyState)>,
    document_metadata: Vec<(NodeIdx, document_surface::DocumentMetadata)>,
    token_lists: Vec<(NodeIdx, String)>,
    mutation_generation: u64,
    node_generations: Vec<u64>,
    node_slot_live: Vec<bool>,
    free_node_slots: Vec<NodeIdx>,
    render_epoch: u64,
    render_mutations: Vec<render_mutation::RenderMutation>,
    dom_exceptions: Vec<DomExceptionData>,
    traversals: traversal::Traversals,
    shadow_roots: Vec<shadow::ShadowRootData>,
    slot_states: Vec<shadow::SlotState>,
    custom_elements: custom_elements::CustomElements,
    form_states: Vec<Option<forms::ControlState>>,
    radio_lists: Vec<(NodeIdx, Vec<u16>)>,
    template_contents: Vec<(NodeIdx, NodeIdx)>,
    template_owner_documents: Vec<(NodeIdx, NodeIdx)>,
    /// Doctype name captured by the tree builder (Phase 1). Quirks-mode bits
    /// land here when the layout engine needs them.
    pub doctype: Option<String>,
}

impl Dom {
    pub fn create_dom_exception(&mut self, name: Vec<u16>, message: Vec<u16>) -> usize {
        let slot = self.dom_exceptions.len();
        self.dom_exceptions.push(DomExceptionData { name, message });
        slot
    }
    pub fn dom_exception(&self, slot: usize) -> Option<&DomExceptionData> {
        self.dom_exceptions.get(slot)
    }
    /// Listener and event registry for this document arena.
    pub fn events(&self) -> &events::EventTargets { &self.events }
    pub fn events_mut(&mut self) -> &mut events::EventTargets { &mut self.events }
    pub fn create_event_target(&mut self) -> u64 {
        let slot = self.external_event_targets;
        self.external_event_targets += 1;
        slot
    }
    pub fn has_event_target(&self, slot: u64) -> bool { slot < self.external_event_targets }

    pub fn new() -> Self {
        let mut dom = Self {
            nodes: Vec::new(),
            namespace_uris: Vec::new(),
            mutations: mutation::MutationObservers::default(),
            ranges: Vec::new(),
            static_ranges: Vec::new(),
            events: events::EventTargets::default(),
            external_event_targets: 0,
            mutation_records: Vec::new(),
            static_node_lists: Vec::new(),
            collections: Vec::new(),
            abort_signals: abort::AbortSignals::default(),
            document_states: Vec::new(),
            document_metadata: Vec::new(),
            token_lists: Vec::new(),
            mutation_generation: 0,
            node_generations: Vec::new(),
            node_slot_live: Vec::new(),
            free_node_slots: Vec::new(),
            render_epoch: 0,
            render_mutations: Vec::new(),
            dom_exceptions: Vec::new(),
            traversals: traversal::Traversals::default(),
            shadow_roots: Vec::new(),
            slot_states: Vec::new(),
            custom_elements: custom_elements::CustomElements::default(),
            form_states: Vec::new(),
            radio_lists: Vec::new(),
            template_contents: Vec::new(),
            template_owner_documents: Vec::new(),
            doctype: None,
        };
        dom.nodes.push(Node {
            owner_document: NodeIdx(0),
            namespaced_attributes: Vec::new(),
            attribute_order: Vec::new(),
            character_data_units: None,
            prefix: None,
            kind: NodeKind::Document,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
        });
        dom.node_generations.push(1);
        dom.node_slot_live.push(true);
        dom
    }

    fn alloc(&mut self, kind: NodeKind) -> NodeIdx {
        self.alloc_unattached(kind)
    }

    /// Creates an unattached node; the caller must [`attach_last`] or
    /// [`attach_before`] it. Used by the tree builder for foster-parented
    /// insertions where the final position isn't known at creation time.
    pub fn alloc_unattached(&mut self, kind: NodeKind) -> NodeIdx {
        let index = self.free_node_slots.pop().map_or_else(||
            u32::try_from(self.nodes.len()).expect("DOM arena exhausted u32 handles"), |node| node.0);
        let owner_document = if matches!(kind, NodeKind::Document) {
            NodeIdx(index)
        } else {
            NodeIdx(0)
        };
        let attribute_order = if let NodeKind::Element { attrs, .. } = &kind {
            attrs.iter().map(|(name, _)| namespace::AttributeKey {
                namespace_uri: None, local_name: name.clone(),
            }).collect()
        } else { Vec::new() };
        let node = Node {
            owner_document,
            namespaced_attributes: Vec::new(),
            attribute_order,
            character_data_units: None,
            prefix: None,
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
        };
        if (index as usize) < self.nodes.len() {
            self.nodes[index as usize] = node;
            self.node_generations[index as usize] = self.node_generations[index as usize]
                .checked_add(1).expect("DOM node generation exhausted");
            self.node_slot_live[index as usize] = true;
        } else {
            self.nodes.push(node);
            self.node_generations.push(1);
            self.node_slot_live.push(true);
        }
        NodeIdx(index)
    }

    /// GC seam: the caller must prove no DOM side table or script wrapper can
    /// still reach this unattached leaf. Detaching alone never retires a node.
    #[allow(dead_code)] // Reserved for the DOM wrapper collector; exercised by the reuse test.
    pub(crate) fn retire_unreachable_node(&mut self, node: NodeIdx) -> Result<(), DomError> {
        self.check_node(node)?;
        if node.0 == 0 || self.get(node).parent.is_some() || self.get(node).first_child.is_some() {
            return Err(DomError::InvalidNode);
        }
        self.node_slot_live[node.0 as usize] = false;
        self.free_node_slots.push(node);
        Ok(())
    }

    pub fn get(&self, idx: NodeIdx) -> &Node {
        &self.nodes[idx.0 as usize]
    }

    /// Number of arena slots, including detached nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // ---- convenience constructors ----

    /// Creates an element under `parent`, appended as the last child.
    pub fn create_element(&mut self, parent: NodeIdx, name: &str) -> NodeIdx {
        self.append_child(
            parent,
            NodeKind::Element {
                name: name.to_owned(),
                attrs: Vec::new(),
                namespace: Namespace::Html,
            },
        )
    }

    /// Creates a text node under `parent`. (Phase 1 note: tree construction
    /// requires merging into a trailing text node's data instead of creating
    /// a sibling — that policy lives in the tree constructor.)
    pub fn create_text(&mut self, parent: NodeIdx, data: &str) -> NodeIdx {
        self.append_child(
            parent,
            NodeKind::Text {
                data: data.to_owned(),
            },
        )
    }

    pub fn append_child(&mut self, parent: NodeIdx, kind: NodeKind) -> NodeIdx {
        let child = self.alloc(kind);
        self.attach_last(parent, child);
        child
    }

    // ---- structural ops used by the tree builder ----

    /// Appends an existing (unattached) node as the last child of `parent`.
    pub fn attach_last(&mut self, parent: NodeIdx, idx: NodeIdx) {
        self.attach_before(parent, None, idx);
    }

    /// Parser primitive: preserves links and observations but bypasses DOM
    /// hierarchy validity (HTML construction has its own rules).
    pub fn attach_before(&mut self, parent: NodeIdx, before: Option<NodeIdx>, idx: NodeIdx) {
        assert!(self.get(idx).parent.is_none(), "node already attached");
        assert!(!self.contains(idx, Some(parent)), "tree cycle");
        if let Some(before) = before {
            assert_eq!(self.get(before).parent, Some(parent));
        }
        let previous = before.and_then(|n| self.get(n).prev_sibling).or_else(|| {
            if before.is_none() {
                self.get(parent).last_child
            } else {
                None
            }
        });
        let document = self.get(parent).owner_document;
        self.set_subtree_document(idx, document);
        if self.is_html_template(idx) {
            self.ensure_template_contents(idx).expect("attached HTML template");
        }
        self.ranges_insert(parent, before, 1);
        self.link_before(parent, before, idx);
        self.queue_tree_record(parent, vec![idx], vec![], previous, before);
    }

    /// Parser primitive; removing a detached node is a no-op.
    pub fn detach(&mut self, idx: NodeIdx) {
        if self.get(idx).parent.is_some() {
            self.remove_internal(idx, false);
        }
    }

    /// Appends `extra` to a text node's data.
    pub fn append_text_data(&mut self, idx: NodeIdx, extra: &str) {
        assert!(self.is_text(idx), "append_text_data on non-text node");
        self.replace_character_data(
            idx,
            self.node_length(idx),
            0,
            &extra.encode_utf16().collect::<Vec<_>>(),
        )
        .expect("text node");
    }

    // ---- introspection ----

    pub fn children(&self, idx: NodeIdx) -> Children<'_> {
        Children {
            dom: self,
            next: self.get(idx).first_child,
        }
    }

    pub fn last_child(&self, idx: NodeIdx) -> Option<NodeIdx> {
        self.get(idx).last_child
    }

    /// Element name, or `None` for non-element nodes.
    pub fn name(&self, idx: NodeIdx) -> Option<&str> {
        match &self.get(idx).kind {
            NodeKind::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn attrs_of(&self, idx: NodeIdx) -> Option<&[(String, String)]> {
        match &self.get(idx).kind {
            NodeKind::Element { attrs, .. } => Some(attrs),
            _ => None,
        }
    }

    /// Element namespace, or `None` for non-element nodes.
    pub fn namespace(&self, idx: NodeIdx) -> Option<Namespace> {
        match self.get(idx).kind {
            NodeKind::Element { namespace, .. } => Some(namespace),
            _ => None,
        }
    }

    pub fn set_attrs(&mut self, idx: NodeIdx, attrs: Vec<(String, String)>) {
        if self.attrs_of(idx).is_none() {
            return;
        }
        let old = self.attrs_of(idx).unwrap().to_vec();
        for (name, _) in old {
            if !attrs.iter().any(|(n, _)| *n == name) {
                self.remove_attribute(idx, &name).expect("element");
            }
        }
        for (name, value) in &attrs {
            self.set_attribute(idx, name, value).expect("element");
        }
        // Preserve the parser primitive's exact supplied ordering. Checked
        // Element attribute operations above retain their insertion order.
        if let NodeKind::Element { attrs: slot, .. } = &mut self.nodes[idx.0 as usize].kind {
            *slot = attrs;
        }
        let namespaced: Vec<_> = self.nodes[idx.0 as usize].attribute_order.iter()
            .filter(|key| key.namespace_uri.is_some()).cloned().collect();
        self.nodes[idx.0 as usize].attribute_order = self.attrs_of(idx).unwrap().iter()
            .map(|(name, _)| namespace::AttributeKey {
                namespace_uri: None, local_name: name.clone(),
            }).chain(namespaced).collect();
    }

    pub fn is_text(&self, idx: NodeIdx) -> bool {
        matches!(self.get(idx).kind, NodeKind::Text { .. } | NodeKind::CDATASection { .. })
    }

    /// Depth-first dump in indented-text form — the Phase 1 exit-criteria output.
    pub fn dump(&self, root: NodeIdx) -> String {
        let mut out = String::new();
        self.dump_rec(root, 0, &mut out);
        out
    }

    fn dump_rec(&self, idx: NodeIdx, depth: usize, out: &mut String) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        match &self.get(idx).kind {
            NodeKind::Document => out.push_str("#document\n"),
            NodeKind::DocumentFragment => out.push_str("#document-fragment\n"),
            NodeKind::DocumentType { name, .. } => out.push_str(&format!("<!DOCTYPE {name}>\n")),
            NodeKind::Element {
                name, namespace, ..
            } => {
                out.push('<');
                match namespace {
                    Namespace::Html => {}
                    Namespace::Svg => out.push_str("svg "),
                    Namespace::MathMl => out.push_str("math "),
                    Namespace::Null => {}
                    Namespace::Other(_) => {
                        out.push_str(self.namespace_uri(idx).expect("interned namespace"));
                        out.push(' ');
                    }
                }
                if let Some(prefix) = self.prefix(idx) {
                    out.push_str(prefix);
                    out.push(':');
                }
                out.push_str(name);
                if let NodeKind::Element { attrs, .. } = &self.get(idx).kind {
                    for (key, value) in attrs {
                        out.push(' ');
                        out.push_str(key);
                        out.push_str("=\"");
                        out.push_str(value);
                        out.push('"');
                    }
                }
                out.push_str(">\n");
            }
            NodeKind::Text { data } => {
                out.push('"');
                out.push_str(data);
                out.push_str("\"\n");
            }
            NodeKind::CDATASection { data } => {
                out.push_str("<![CDATA["); out.push_str(data); out.push_str("]]>"); out.push('\n');
            }
            NodeKind::Attr { local_name, .. } => {
                out.push_str("@attribute "); out.push_str(local_name); out.push('\n');
            }
            NodeKind::Comment { data } => {
                out.push_str("<!-- ");
                out.push_str(data);
                out.push_str(" -->\n");
            }
            NodeKind::ProcessingInstruction { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                out.push(' ');
                out.push_str(data);
                out.push_str("?>\n");
            }
        }
        for child in self.children(idx) {
            self.dump_rec(child, depth + 1, out);
        }
    }
}

/// Iterator over an arena node's children.
pub struct Children<'a> {
    dom: &'a Dom,
    next: Option<NodeIdx>,
}

impl Iterator for Children<'_> {
    type Item = NodeIdx;

    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.next?;
        self.next = self.dom.get(cur).next_sibling;
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_is_root() {
        let dom = Dom::new();
        assert_eq!(dom.get(NodeIdx(0)).kind, NodeKind::Document);
    }

    #[test]
    fn append_and_iterate_children() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        dom.create_text(body, "hi");
        let kids: Vec<_> = dom.children(doc).collect();
        assert_eq!(kids, vec![html]);
        assert_eq!(dom.last_child(html), Some(body));
        assert_eq!(dom.children(body).count(), 1);
    }

    #[test]
    fn sibling_links_are_bidirectional() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let a = dom.create_element(doc, "a");
        let b = dom.create_element(doc, "b");
        assert_eq!(dom.get(a).next_sibling, Some(b));
        assert_eq!(dom.get(b).prev_sibling, Some(a));
        assert_eq!(dom.get(b).next_sibling, None);
    }

    #[test]
    fn attach_before_places_between_siblings() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let _a = dom.create_element(doc, "a");
        let c = dom.create_element(doc, "c");
        let b = dom.alloc_unattached(NodeKind::Element {
            name: "b".into(),
            attrs: vec![],
            namespace: Namespace::Html,
        });
        dom.attach_before(doc, Some(c), b);
        let names: Vec<_> = dom
            .children(doc)
            .map(|k| dom.name(k).unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn detach_repairs_links() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let a = dom.create_element(doc, "a");
        let b = dom.create_element(doc, "b");
        let c = dom.create_element(doc, "c");
        dom.detach(b);
        assert_eq!(dom.get(a).next_sibling, Some(c));
        assert_eq!(dom.get(c).prev_sibling, Some(a));
        assert_eq!(dom.get(b).parent, None);
        let names: Vec<_> = dom.children(doc).collect();
        assert_eq!(names, [a, c]);
    }

    #[test]
    fn append_text_data_merges() {
        let mut dom = Dom::new();
        let t = dom.create_text(NodeIdx(0), "hel");
        dom.append_text_data(t, "lo");
        assert_eq!(
            dom.get(t).kind,
            NodeKind::Text {
                data: "hello".into()
            }
        );
    }

    #[test]
    fn dump_shows_tree_shape() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let p = dom.create_element(html, "p");
        dom.create_text(p, "hello");
        assert_eq!(
            dom.dump(NodeIdx(0)),
            "#document\n  <html>\n    <p>\n      \"hello\"\n"
        );
    }
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}
