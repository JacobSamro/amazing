//! HTML SS7.2.2.3 "Named access on the Window object" -- the DOM-side half
//! of the computation (the id/`name`-keyed named-object map, in tree
//! order). Defining the resulting properties on the JS global object is the
//! driver's job (`crates/amazing/src/runtime_binding_host.rs`'s `with_dom`),
//! since this crate has no JS binding of its own; see `named_objects_dirty`
//! on `Dom` for the dirty-tracking half of the wiring.
use crate::{Dom, NodeIdx, NodeKind};
use std::collections::HashMap;

impl Dom {
    /// Marks the named-object map stale. Called by every mutation that could
    /// change HTML SS7.2.2.3's source data: `id`/`name` attribute changes
    /// (`mutation.rs`'s `set_attribute_ns`/`remove_attribute_ns`) and node
    /// insertion/removal (`tree.rs`'s `insert_internal`/`remove_internal`) --
    /// an element with an *existing* `id`/`name` can be moved into or out of
    /// a document tree without any attribute mutation at all.
    pub fn mark_named_objects_dirty(&mut self) {
        self.named_objects_dirty = true;
    }
    /// Clears and returns the previous flag. The driver calls this once per
    /// `with_dom` closure and only recomputes/redefines window's named
    /// properties when it comes back `true`, so an ordinary read-only
    /// binding call (the overwhelming majority of them) costs one bool
    /// check, not a document rescan.
    pub fn take_named_objects_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.named_objects_dirty, false)
    }
    /// HTML SS7.2.2.3's supported-property-names/determine-the-value source
    /// data for `document`'s associated window, freshly computed in tree
    /// order: `(name, contributing elements in tree order)` pairs, keys in
    /// first-contributed order ("ignoring later duplicates" per spec). A
    /// single element contributes under two independent rules -- its `id`
    /// (any element) and its `name` content attribute (only `embed`/`form`/
    /// `img`/`object`) -- so it can appear under two different keys, or
    /// (`id`==`name`) push to the same key from both rules; the latter is
    /// deduplicated by checking the key's last entry before pushing, since
    /// both rules visit the same node back to back.
    ///
    /// Deliberately excludes the spec's third source, "window's
    /// document-tree child navigable target name property set" (nested
    /// browsing contexts addressed by name): this engine's `HTMLIFrameElement`
    /// has no `contentWindow`/content navigable yet (see
    /// `crates/amazing-bindings/evidence/range-timeouts/README.md`), so
    /// there is nothing to contribute from that source rather than an
    /// incorrect placeholder.
    pub fn window_named_objects(&self, document: NodeIdx) -> Vec<(String, Vec<NodeIdx>)> {
        let mut order: Vec<String> = Vec::new();
        let mut map: HashMap<String, Vec<NodeIdx>> = HashMap::new();
        let mut push = |map: &mut HashMap<String, Vec<NodeIdx>>, order: &mut Vec<String>, key: String, node: NodeIdx| {
            let list = map.entry(key.clone()).or_insert_with(|| {
                order.push(key);
                Vec::new()
            });
            if list.last() != Some(&node) {
                list.push(node);
            }
        };
        for node in self.tree_order(document) {
            let NodeKind::Element { name: tag, .. } = &self.get(node).kind else { continue };
            let tag = tag.clone();
            if let Some(id) = self.get_attribute_ns(node, None, "id").filter(|s| !s.is_empty()).map(str::to_owned) {
                push(&mut map, &mut order, id, node);
            }
            if matches!(tag.as_str(), "embed" | "form" | "img" | "object") {
                if let Some(name) = self.get_attribute_ns(node, None, "name").filter(|s| !s.is_empty()).map(str::to_owned) {
                    push(&mut map, &mut order, name, node);
                }
            }
        }
        order.into_iter().map(|key| {
            let elements = map.remove(&key).unwrap_or_default();
            (key, elements)
        }).collect()
    }
}
