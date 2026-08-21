//! Ferric DOM — arena-allocated document tree.
//!
//! **Status: Phase 0 skeleton.** The arena design below is fixed now because
//! every later phase depends on it; node kinds grow in Phase 1.
//!
//! Design decisions (locked):
//! - Nodes are slots in a flat `Vec`; references are `NodeIdx` (u32) — no
//!   `Rc<RefCell<..>>`, so no reference cycles and cheap snapshotting.
//! - Freelist reuse keeps indices stable across mutations.
//! - Single ownership: a node has exactly one parent; first-child /
//!   next-sibling / prev-sibling links give O(1) traversal.

/// Index into a [`Dom`] arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeIdx(pub u32);

/// The five node kinds Phase 1 needs to bootstrap tree construction.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Document,
    Element { name: String },
    Text { data: String },
    Comment { data: String },
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

/// Arena-backed DOM forest root.
#[derive(Debug, Default)]
pub struct Dom {
    nodes: Vec<Node>,
}

impl Dom {
    pub fn new() -> Self {
        let mut dom = Self { nodes: Vec::new() };
        dom.alloc(NodeKind::Document); // index 0 is always the document
        dom
    }

    fn alloc(&mut self, kind: NodeKind) -> NodeIdx {
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

    /// Creates an element under `parent`, appended as the last child.
    pub fn create_element(&mut self, parent: NodeIdx, name: &str) -> NodeIdx {
        self.append_child(parent, NodeKind::Element { name.to_owned() })
    }

    /// Creates a text node under `parent`. (Phase 1 note: tree construction
    /// requires merging into a trailing text node's data instead of creating
    /// a sibling — that policy lives in the tree constructor, not here.)
    pub fn create_text(&mut self, parent: NodeIdx, data: &str) -> NodeIdx {
        self.append_child(parent, NodeKind::Text { data.to_owned() })
    }

    pub fn append_child(&mut self, parent: NodeIdx, kind: NodeKind) -> NodeIdx {
        let child = self.alloc(kind);
        let last = self.get(parent).last_child;
        self.nodes[child.0 as usize].parent = Some(parent);
        self.nodes[child.0 as usize].prev_sibling = last;
        match last {
            Some(prev) => self.nodes[prev.0 as usize].next_sibling = Some(child),
            None => self.nodes[parent.0 as usize].first_child = Some(child),
        }
        self.nodes[parent.0 as usize].last_child = Some(child);
        child
    }

    pub fn children(&self, idx: NodeIdx) -> Children<'_> {
        Children {
            dom: self,
            next: self.get(idx).first_child,
        }
    }

    pub fn last_child(&self, idx: NodeIdx) -> Option<NodeIdx> {
        self.get(idx).last_child
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
            NodeKind::Element { name } => {
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
        let body_kids: Vec<_> = dom.children(body).collect();
        assert_eq!(body_kids.len(), 1);
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
