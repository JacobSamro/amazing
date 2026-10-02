//! Selectedcontent mirroring for customizable select elements. Current
//! select parsing uses the in-body rules, including fragment contexts.

use super::*;

impl TreeBuilder {
    /// The `<selectedcontent>` element mirrors its `<select>`'s selected
    /// `<option>` (a newer HTML feature -- "customizable select";
    /// html5lib/WPT webkit02.dat cases 45-48). Its spec form is a live
    /// "update selectedcontent" step run from option "insertion steps" and
    /// from attribute changes, driven by script; none of these fixtures run
    /// script, so a single post-parse pass over the finished tree (called
    /// once from `into_dom`) produces the same result: for every `<select>`
    /// whose first child is a `<button>`, find the descendant
    /// `<selectedcontent>` (if any) and mirror a deep clone of the selected
    /// option's children into it. "Selected" follows the same default-
    /// selectedness rule script-driven selects use: the last `<option>`
    /// carrying a `selected` attribute, or else the first `<option>` at all.
    pub(super) fn apply_selectedcontent_mirroring(&mut self) {
        let mut selects = Vec::new();
        Self::collect_by_name(&self.dom, NodeIdx(0), "select", &mut selects);
        for select in selects {
            let mut options = Vec::new();
            Self::collect_select_options(&self.dom, select, &mut options);
            let selected = options
                .iter()
                .rev()
                .copied()
                .find(|&o| {
                    self.dom
                        .attrs_of(o)
                        .is_some_and(|attrs| attrs.iter().any(|(name, _)| name == "selected"))
                })
                .or_else(|| options.first().copied());
            let Some(selected) = selected else { continue };
            let Some(button) = self.dom.children(select).next().filter(|&c| {
                self.dom.namespace(c) == Some(Namespace::Html) && self.dom.name(c) == Some("button")
            }) else {
                continue;
            };
            let mut target = None;
            Self::first_by_name(&self.dom, button, "selectedcontent", &mut target);
            let Some(target) = target else { continue };
            for child in self.dom.children(target).collect::<Vec<_>>() {
                self.dom.detach(child);
            }
            for child in self.dom.children(selected).collect::<Vec<_>>() {
                Self::clone_node_within(&mut self.dom, child, target);
            }
        }
    }

    fn collect_by_name(dom: &Dom, root: NodeIdx, name: &str, out: &mut Vec<NodeIdx>) {
        for child in dom.children(root) {
            if dom.namespace(child) == Some(Namespace::Html) && dom.name(child) == Some(name) {
                out.push(child);
            }
            Self::collect_by_name(dom, child, name, out);
        }
    }

    fn first_by_name(dom: &Dom, root: NodeIdx, name: &str, out: &mut Option<NodeIdx>) {
        if out.is_some() {
            return;
        }
        for child in dom.children(root) {
            if dom.namespace(child) == Some(Namespace::Html) && dom.name(child) == Some(name) {
                *out = Some(child);
                return;
            }
            Self::first_by_name(dom, child, name, out);
            if out.is_some() {
                return;
            }
        }
    }

    /// Every `<option>` descendant of `root` (a `<select>`), not descending
    /// into a nested `<select>` (disallowed by parsing, but guarded anyway).
    fn collect_select_options(dom: &Dom, root: NodeIdx, out: &mut Vec<NodeIdx>) {
        for child in dom.children(root) {
            if dom.namespace(child) != Some(Namespace::Html) {
                continue;
            }
            match dom.name(child) {
                Some("option") => out.push(child),
                Some("select") => {},
                _ => Self::collect_select_options(dom, child, out),
            }
        }
    }

    /// Deep-clones `source` (and its descendants) as a new last child of
    /// `parent`, within the same `Dom` -- `amazing_html::lib.rs`'s own
    /// `clone_subtree` clones across two separate `Dom`s (fragment parsing
    /// into a caller's document) and so borrows source and target
    /// independently; this variant is for cloning within one `Dom`.
    fn clone_node_within(dom: &mut Dom, source: NodeIdx, parent: NodeIdx) {
        let kind = dom.get(source).kind.clone();
        let created = dom.append_child(parent, kind);
        let children: Vec<NodeIdx> = dom.children(source).collect();
        for child in children {
            Self::clone_node_within(dom, child, created);
        }
    }
}
