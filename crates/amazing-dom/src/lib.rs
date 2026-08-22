//! Amazing DOM — arena-allocated document tree.
//!
//! **Status: Phase 0 skeleton, extended in Phase 1.**
//!
//! Design decisions (locked):
//! - Nodes are slots in a flat `Vec`; references are `NodeIdx` (u32) — no
//!   `Rc<RefCell<..>>`, so no reference cycles and cheap snapshotting.
//! - Single ownership: a node has exactly one parent; first-child /
//!   next-sibling / prev-sibling links give O(1) traversal.
//! - The low-level attach/detach API exists for the HTML tree constructor
//!   (foster parenting, adoption agency). It is engine-internal surface;
//!   Phase 7 will encapsulate it behind the DOM/JS bridge.

/// Index into a [`Dom`] arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeIdx(pub u32);

/// Node kinds. Grows in later phases (attributes live on `Element` since
/// the CSS selector engine needs them).
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Document,
    Element {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Text {
        data: String,
    },
    Comment {
        data: String,
    },
}

#[derive(Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeIdx>,
    pub first_child: Option<NodeIdx>,
    pub last_child: Option<NodeIdx>,
    pub next_sibling: Option<NodeIdx>,
    pub prev_sibling: Option<NodeIdx>,
}

/// Arena-backed DOM.
#[derive(Debug, Default)]
pub struct Dom {
    nodes: Vec<Node>,
    /// Doctype name captured by the tree builder (Phase 1). Quirks-mode bits
    /// land here when the layout engine needs them.
    pub doctype: Option<String>,
}

impl Dom {
    pub fn new() -> Self {
        let mut dom = Self {
            nodes: Vec::new(),
            doctype: None,
        };
        dom.nodes.push(Node {
            kind: NodeKind::Document,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
        });
        dom
    }

    fn alloc(&mut self, kind: NodeKind) -> NodeIdx {
        self.alloc_unattached(kind)
    }

    /// Creates an unattached node; the caller must [`attach_last`] or
    /// [`attach_before`] it. Used by the tree builder for foster-parented
    /// insertions where the final position isn't known at creation time.
    pub fn alloc_unattached(&mut self, kind: NodeKind) -> NodeIdx {
        self.nodes.push(Node {
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
        });
        NodeIdx((self.nodes.len() - 1) as u32)
    }

    pub fn get(&self, idx: NodeIdx) -> &Node {
        &self.nodes[idx.0 as usize]
    }

    // ---- convenience constructors ----

    /// Creates an element under `parent`, appended as the last child.
    pub fn create_element(&mut self, parent: NodeIdx, name: &str) -> NodeIdx {
        self.append_child(parent, NodeKind::Element {
            name: name.to_owned(),
            attrs: Vec::new(),
        })
    }

    /// Creates a text node under `parent`. (Phase 1 note: tree construction
    /// requires merging into a trailing text node's data instead of creating
    /// a sibling — that policy lives in the tree constructor.)
    pub fn create_text(&mut self, parent: NodeIdx, data: &str) -> NodeIdx {
        self.append_child(parent, NodeKind::Text { data: data.to_owned() })
    }

    pub fn append_child(&mut self, parent: NodeIdx, kind: NodeKind) -> NodeIdx {
        let child = self.alloc(kind);
        self.attach_last(parent, child);
        child
    }

    // ---- structural ops used by the tree builder ----

    /// Appends an existing (unattached) node as the last child of `parent`.
    pub fn attach_last(&mut self, parent: NodeIdx, idx: NodeIdx) {
        debug_assert!(self.get(idx).parent.is_none(), "node already attached");
        let last = self.get(parent).last_child;
        self.nodes[idx.0 as usize].parent = Some(parent);
        self.nodes[idx.0 as usize].prev_sibling = last;
        self.nodes[idx.0 as usize].next_sibling = None;
        match last {
            Some(prev) => self.nodes[prev.0 as usize].next_sibling = Some(idx),
            None => self.nodes[parent.0 as usize].first_child = Some(idx),
        }
        self.nodes[parent.0 as usize].last_child = Some(idx);
    }

    /// Inserts an existing (unattached) node under `parent`, immediately
    /// before sibling `before` (or as last child when `before` is `None`).
    /// Foster parenting and adoption-agency relocation use this.
    pub fn attach_before(
        &mut self,
        parent: NodeIdx,
        before: Option<NodeIdx>,
        idx: NodeIdx,
    ) {
        match before {
            None => self.attach_last(parent, idx),
            Some(before) => {
                debug_assert_eq!(self.get(before).parent, Some(parent));
                let prev = self.get(before).prev_sibling;
                self.nodes[idx.0 as usize].parent = Some(parent);
                self.nodes[idx.0 as usize].prev_sibling = prev;
                self.nodes[idx.0 as usize].next_sibling = Some(before);
                self.nodes[before.0 as usize].prev_sibling = Some(idx);
                match prev {
                    Some(p) => self.nodes[p.0 as usize].next_sibling = Some(idx),
                    None => self.nodes[parent.0 as usize].first_child = Some(idx),
                }
            },
        }
    }

    /// Removes a node from its parent's child list, leaving it unattached.
    pub fn detach(&mut self, idx: NodeIdx) {
        let (parent, prev, next) = {
            let n = self.get(idx);
            (n.parent, n.prev_sibling, n.next_sibling)
        };
        if let Some(p) = prev {
            self.nodes[p.0 as usize].next_sibling = next;
        }
        if let Some(n) = next {
            self.nodes[n.0 as usize].prev_sibling = prev;
        }
        if let Some(par) = parent {
            if self.nodes[par.0 as usize].first_child == Some(idx) {
                self.nodes[par.0 as usize].first_child = next;
            }
            if self.nodes[par.0 as usize].last_child == Some(idx) {
                self.nodes[par.0 as usize].last_child = prev;
            }
        }
        let n = &mut self.nodes[idx.0 as usize];
        n.parent = None;
        n.prev_sibling = None;
        n.next_sibling = None;
    }

    /// Appends `extra` to a text node's data.
    pub fn append_text_data(&mut self, idx: NodeIdx, extra: &str) {
        if let NodeKind::Text { data } = &mut self.nodes[idx.0 as usize].kind {
            data.push_str(extra);
        } else {
            unreachable!("append_text_data on non-text node");
        }
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

    pub fn set_attrs(&mut self, idx: NodeIdx, attrs: Vec<(String, String)>) {
        if let NodeKind::Element { attrs: slot, .. } =
            &mut self.nodes[idx.0 as usize].kind
        {
            *slot = attrs;
        }
    }

    pub fn is_text(&self, idx: NodeIdx) -> bool {
        matches!(self.get(idx).kind, NodeKind::Text { .. })
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
            NodeKind::Element { name, .. } => {
                out.push('<');
                out.push_str(name);
                out.push_str(">\n");
            },
            NodeKind::Text { data } => {
                out.push('"');
                out.push_str(data);
                out.push_str("\"\n");
            },
            NodeKind::Comment { data } => {
                out.push_str("<!-- ");
                out.push_str(data);
                out.push_str(" -->\n");
            },
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
        });
        dom.attach_before(doc, Some(c), b);
        let names: Vec<_> = dom.children(doc).map(|k| dom.name(k).unwrap().to_owned()).collect();
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
            NodeKind::Text { data: "hello".into() }
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
