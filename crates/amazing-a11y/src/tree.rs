//! Accessibility tree construction.
//!
//! The builder walks the DOM once. It decides, per node, whether the node is
//! exposed (Include), removed while keeping its children (Promote), or removed
//! with its whole subtree (Skip). Promotion implements presentational-node
//! pruning; skipping implements `display:none`, `hidden`, `aria-hidden` and
//! non-rendered elements.

use amazing_css::StyledDom;
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_layout::LayoutBox;
use std::collections::{HashMap, HashSet};

use crate::accname::{self, NameContext};
use crate::aria;
use crate::bounds::{self, AxRect, BoundsMap};
use crate::host;
use crate::role::{self, Role};

/// Stable identifier of a node in an [`AxTree`]. Ids are assigned in preorder
/// after pruning, so two trees built from the same document agree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct AxNodeId(pub u32);

/// Resolved id-reference relationships of one node. Each id is looked up in the
/// same document and only retained when it maps to a node in this tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AxRelations {
    pub labelledby: Vec<AxNodeId>,
    pub describedby: Vec<AxNodeId>,
    pub details: Vec<AxNodeId>,
    pub errormessage: Option<AxNodeId>,
    pub controls: Vec<AxNodeId>,
    pub flowto: Vec<AxNodeId>,
    pub owns: Vec<AxNodeId>,
}

/// One node of the accessibility tree.
#[derive(Clone, Debug)]
pub struct AxNode {
    pub id: AxNodeId,
    /// Originating DOM arena node; `None` for the synthetic document root.
    pub dom_node: Option<NodeIdx>,
    pub role: Role,
    pub name: String,
    pub description: String,
    pub states: aria::AxStates,
    pub properties: aria::AxProperties,
    pub bounds: Option<AxRect>,
    pub children: Vec<AxNodeId>,
    pub parent: Option<AxNodeId>,
    /// Resolved `aria-activedescendant` target, when present in this tree.
    pub active_descendant: Option<AxNodeId>,
    /// Resolved id-reference relationships.
    pub relations: AxRelations,
}

impl AxNode {
    pub fn is_focusable(&self) -> bool {
        self.states.focusable
    }
}

/// A built accessibility tree.
#[derive(Clone, Debug, Default)]
pub struct AxTree {
    nodes: Vec<AxNode>,
    root: Option<AxNodeId>,
}

impl AxTree {
    pub fn root(&self) -> Option<AxNodeId> {
        self.root
    }

    pub fn node(&self, id: AxNodeId) -> Option<&AxNode> {
        self.nodes.get(id.0 as usize)
    }

    pub fn nodes(&self) -> &[AxNode] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn children(&self, id: AxNodeId) -> &[AxNodeId] {
        self.nodes.get(id.0 as usize).map_or(&[], |node| node.children.as_slice())
    }

    /// Resolved id-reference relationships of a node.
    pub fn relations(&self, id: AxNodeId) -> Option<&AxRelations> {
        self.node(id).map(|node| &node.relations)
    }

    /// The exposed node for a DOM arena index, when one exists.
    pub fn find_dom(&self, node: NodeIdx) -> Option<AxNodeId> {
        self.nodes
            .iter()
            .find(|entry| entry.dom_node == Some(node))
            .map(|entry| entry.id)
    }

    /// The node carrying focus, if any.
    pub fn focused(&self) -> Option<AxNodeId> {
        self.nodes.iter().find(|node| node.states.focused).map(|node| node.id)
    }

    /// Root-to-focused node path (empty when nothing is focused).
    pub fn focus_chain(&self) -> Vec<AxNodeId> {
        let Some(mut current) = self.focused() else { return Vec::new() };
        let mut chain = Vec::new();
        loop {
            chain.push(current);
            match self.nodes.get(current.0 as usize).and_then(|node| node.parent) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        chain.reverse();
        chain
    }

    /// Accessible text of a subtree in reading order. Inline runs are joined
    /// with spaces; line breaks and block-level boundaries become newlines.
    pub fn text_content(&self, id: AxNodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out.trim_end().to_owned()
    }

    /// The full document text, in reading order.
    pub fn text(&self) -> String {
        match self.root {
            Some(root) => self.text_content(root),
            None => String::new(),
        }
    }

    /// Character range of a node's text inside [`AxTree::text`]. Leading
    /// separators are excluded, so the range always slices the node's own text.
    pub fn text_range(&self, id: AxNodeId) -> Option<(usize, usize)> {
        let root = self.root?;
        let mut out = String::new();
        let mut start = None;
        let mut end = None;
        self.record_range(root, id, &mut out, &mut start, &mut end);
        let (mut start, end) = (start?, end?);
        let chars: Vec<char> = out.chars().collect();
        while start < end && chars.get(start).is_some_and(|c| c.is_whitespace()) {
            start += 1;
        }
        Some((start, end))
    }

    fn collect_text(&self, id: AxNodeId, out: &mut String) {
        self.append_text(id, out);
        for child in self.children(id) {
            self.collect_text(*child, out);
        }
    }

    fn append_text(&self, id: AxNodeId, out: &mut String) {
        let Some(node) = self.node(id) else { return };
        match node.role {
            Role::StaticText | Role::InlineTextBox => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(&node.name);
            }
            Role::LineBreak => out.push('\n'),
            role if breaks_line(role) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }

    fn record_range(
        &self,
        current: AxNodeId,
        target: AxNodeId,
        out: &mut String,
        start: &mut Option<usize>,
        end: &mut Option<usize>,
    ) {
        if current == target {
            *start = Some(out.chars().count());
        }
        self.append_text(current, out);
        for child in self.children(current) {
            self.record_range(*child, target, out, start, end);
        }
        if current == target {
            *end = Some(out.chars().count());
        }
    }

    /// Depth-first preorder walk starting at `root` (defaults to the tree root).
    pub fn walk(&self, start: Option<AxNodeId>) -> Vec<AxNodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<AxNodeId> = start.or(self.root).into_iter().collect();
        while let Some(id) = stack.pop() {
            out.push(id);
            let mut children = self.children(id).to_vec();
            children.reverse();
            stack.extend(children);
        }
        out
    }

    /// A flat, detached snapshot of the tree.
    pub fn snapshot(&self) -> SchemaSnapshot {
        SchemaSnapshot {
            root: self.root,
            nodes: self.nodes.iter().map(SchemaNode::from).collect(),
        }
    }
}

/// Flat snapshot of one accessibility node.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaNode {
    pub id: AxNodeId,
    pub dom_node: Option<u32>,
    pub role: Role,
    pub name: String,
    pub description: String,
    pub states: aria::AxStates,
    pub properties: aria::AxProperties,
    pub bounds: Option<AxRect>,
    pub children: Vec<AxNodeId>,
    pub parent: Option<AxNodeId>,
    pub active_descendant: Option<AxNodeId>,
    /// Resolved id-reference relationships.
    pub relations: AxRelations,
}

impl From<&AxNode> for SchemaNode {
    fn from(node: &AxNode) -> Self {
        Self {
            id: node.id,
            dom_node: node.dom_node.map(|n| n.0),
            role: node.role,
            name: node.name.clone(),
            description: node.description.clone(),
            states: node.states.clone(),
            properties: node.properties.clone(),
            bounds: node.bounds,
            children: node.children.clone(),
            parent: node.parent,
            active_descendant: node.active_descendant,
            relations: node.relations.clone(),
        }
    }
}

/// Flat snapshot of a whole tree.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaSnapshot {
    pub root: Option<AxNodeId>,
    pub nodes: Vec<SchemaNode>,
}

impl SchemaSnapshot {
    pub fn node(&self, id: AxNodeId) -> Option<&SchemaNode> {
        self.nodes.get(id.0 as usize).filter(|node| node.id == id)
    }

    /// Text rendering used by tests and the CLI: one indented line per node.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        if let Some(root) = self.root {
            self.dump_rec(root, 0, &mut out);
        }
        out
    }

    fn dump_rec(&self, id: AxNodeId, depth: usize, out: &mut String) {
        let Some(node) = self.node(id) else { return };
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(node.role.as_str());
        if !node.name.is_empty() {
            out.push_str(" \"");
            out.push_str(&node.name);
            out.push('"');
        }
        if !node.description.is_empty() {
            out.push_str(" (");
            out.push_str(&node.description);
            out.push(')');
        }
        let states = &node.states;
        if states.checked.is_defined() {
            out.push_str(" checked=");
            out.push_str(states.checked.as_str().unwrap_or("?"));
        }
        if states.disabled {
            out.push_str(" disabled");
        }
        if let Some(expanded) = states.expanded {
            out.push_str(if expanded { " expanded" } else { " collapsed" });
        }
        if states.selected {
            out.push_str(" selected");
        }
        if states.required {
            out.push_str(" required");
        }
        if states.invalid != aria::InvalidState::False {
            out.push_str(" invalid");
        }
        if states.hidden {
            out.push_str(" hidden");
        }
        if states.inert {
            out.push_str(" inert");
        }
        if states.focusable {
            out.push_str(" focusable");
        }
        if states.focused {
            out.push_str(" focused");
        }
        out.push('\n');
        for child in &node.children {
            self.dump_rec(*child, depth + 1, out);
        }
    }
}

enum Decision {
    Include,
    Promote,
    Skip,
}

/// Builds the accessibility tree for a document whose styles and layout are
/// already computed.
pub fn build(dom: &Dom, styled: &StyledDom, layout: &LayoutBox) -> AxTree {
    let (ids, labels_by_target) = collect_indices(dom);
    let bounds = bounds::collect_bounds(dom, layout);
    let mut builder =
        Builder { dom, styled, ids: ids.clone(), labels_by_target, bounds, nodes: Vec::new() };
    let root = builder.build_root();
    builder.apply_owns();
    let mut tree = builder.finish(root);
    // Id-reference resolution runs on the final, pruned tree so a reference to
    // a node that presentational flattening removed is dropped rather than
    // left pointing at a stale id.
    resolve_references(dom, &ids, &mut tree);
    tree
}

/// Convenience entry point: parse HTML, compute styles, lay out, and build the
/// tree with the fallback text system.
pub fn build_from_html(html: &str, author_css: &str, viewport_width: f32) -> AxTree {
    build_from_html_in_viewport(html, author_css, viewport_width, None)
}

/// Like [`build_from_html`] with an explicit viewport height.
pub fn build_from_html_in_viewport(
    html: &str,
    author_css: &str,
    viewport_width: f32,
    viewport_height: Option<f32>,
) -> AxTree {
    build_document(html, author_css, viewport_width, viewport_height).tree
}

/// The parsed DOM, computed styles, layout tree and accessibility tree for one
/// document, retained together so a driver can map nodes across layers.
#[derive(Debug)]
pub struct AxDocument {
    pub dom: Dom,
    pub styled: StyledDom,
    pub layout: LayoutBox,
    pub tree: AxTree,
}

/// Parses HTML, computes styles, lays out, and builds the accessibility tree,
/// returning every intermediate artifact.
pub fn build_document(
    html: &str,
    author_css: &str,
    viewport_width: f32,
    viewport_height: Option<f32>,
) -> AxDocument {
    let dom = amazing_html::parse(html);
    let sheet = amazing_css::parse_stylesheet(author_css);
    let styled = amazing_css::compute(&dom, &[sheet]);
    let text = amazing_text::FallbackTextSystem::new();
    let layout = amazing_layout::layout_styled_document(
        &dom,
        &styled,
        &text,
        &amazing_layout::NoReplacedElements,
        viewport_width,
        viewport_height,
    );
    let tree = build(&dom, &styled, &layout);
    AxDocument { dom, styled, layout, tree }
}

/// Full HTML entry point returning the accessibility tree for an already-parsed
/// document pipeline.
pub fn build_from_parsed_html(
    html: &str,
    author_css: &str,
    viewport_width: f32,
    viewport_height: Option<f32>,
) -> AxTree {
    build_document(html, author_css, viewport_width, viewport_height).tree
}

/// Resolves `aria-activedescendant` and the id-reference relationships
/// (`aria-labelledby`, `aria-describedby`, `aria-details`,
/// `aria-errormessage`, `aria-controls`, `aria-flowto`, `aria-owns`) against
/// the final, pruned tree. Ids that do not resolve to a node exposed in
/// `tree` are dropped. `aria-owns` is resolved here for explicit exposure of
/// the relationship, independent of the reparenting `Builder::apply_owns`
/// already applied to the tree structure.
fn resolve_references(dom: &Dom, ids: &HashMap<String, NodeIdx>, tree: &mut AxTree) {
    let by_dom: HashMap<NodeIdx, AxNodeId> = tree
        .nodes
        .iter()
        .filter_map(|node| node.dom_node.map(|dom_node| (dom_node, node.id)))
        .collect();
    for index in 0..tree.nodes.len() {
        let Some(dom_node) = tree.nodes[index].dom_node else { continue };
        let single = |attr: &str| -> Option<AxNodeId> {
            dom.get_attribute_ns(dom_node, None, attr)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .and_then(|value| ids.get(value))
                .and_then(|target| by_dom.get(target).copied())
        };
        let active_descendant = single("aria-activedescendant");
        let errormessage = single("aria-errormessage");
        tree.nodes[index].active_descendant = active_descendant;
        tree.nodes[index].relations = AxRelations {
            labelledby: resolve_ids(dom, ids, &by_dom, dom_node, "aria-labelledby"),
            describedby: resolve_ids(dom, ids, &by_dom, dom_node, "aria-describedby"),
            details: resolve_ids(dom, ids, &by_dom, dom_node, "aria-details"),
            errormessage,
            controls: resolve_ids(dom, ids, &by_dom, dom_node, "aria-controls"),
            flowto: resolve_ids(dom, ids, &by_dom, dom_node, "aria-flowto"),
            owns: resolve_ids(dom, ids, &by_dom, dom_node, "aria-owns"),
        };
    }
}

fn collect_indices(dom: &Dom) -> (HashMap<String, NodeIdx>, HashMap<String, Vec<NodeIdx>>) {
    let mut ids = HashMap::new();
    let mut labels: HashMap<String, Vec<NodeIdx>> = HashMap::new();
    for node in dom.tree_order(NodeIdx(0)) {
        if let Some(id) = dom.get_attribute_ns(node, None, "id")
            && !id.is_empty()
        {
            ids.entry(id.to_owned()).or_insert(node);
        }
        if dom.name(node) == Some("label")
            && let Some(target) = dom.get_attribute_ns(node, None, "for")
            && !target.is_empty()
        {
            labels.entry(target.to_owned()).or_default().push(node);
        }
    }
    (ids, labels)
}

struct Builder<'a> {
    dom: &'a Dom,
    styled: &'a StyledDom,
    ids: HashMap<String, NodeIdx>,
    labels_by_target: HashMap<String, Vec<NodeIdx>>,
    bounds: BoundsMap,
    nodes: Vec<AxNode>,
}

impl<'a> Builder<'a> {
    fn build_root(&mut self) -> Option<AxNodeId> {
        let document = NodeIdx(0);
        if let Some(html) = self
            .dom
            .children(document)
            .find(|child| self.dom.name(*child) == Some("html"))
        {
            let id = self.push_node(html, None)?;
            let children: Vec<NodeIdx> = self.dom.children(html).collect();
            for child in children {
                self.visit(child, Some(id));
            }
            return Some(id);
        }
        let id = self.push_synthetic(Role::Document, None);
        let children: Vec<NodeIdx> = self.dom.children(document).collect();
        for child in children {
            self.visit(child, Some(id));
        }
        Some(id)
    }

    /// Reparents nodes named by `aria-owns` under their owner, in the order
    /// listed. The first owner wins; moves that would create a cycle are
    /// ignored. Owned children are appended after the owner's DOM children.
    fn apply_owns(&mut self) {
        let by_dom: HashMap<NodeIdx, AxNodeId> = self
            .nodes
            .iter()
            .filter_map(|node| node.dom_node.map(|dom_node| (dom_node, node.id)))
            .collect();
        let owners: Vec<(AxNodeId, NodeIdx)> = self
            .nodes
            .iter()
            .filter_map(|node| node.dom_node.map(|dom_node| (node.id, dom_node)))
            .collect();
        let mut owned: HashSet<AxNodeId> = HashSet::new();
        for (owner, owner_dom) in owners {
            let Some(references) = self.dom.get_attribute_ns(owner_dom, None, "aria-owns") else {
                continue;
            };
            let targets: Vec<AxNodeId> = references
                .split_ascii_whitespace()
                .filter_map(|id| self.ids.get(id))
                .filter_map(|dom_node| by_dom.get(dom_node).copied())
                .collect();
            for target in targets {
                // The first owner wins; a node can be owned at most once.
                if target == owner
                    || owned.contains(&target)
                    || self.nodes[owner.0 as usize].children.contains(&target)
                {
                    continue;
                }
                // The owner must not already be inside the target.
                if self.is_under(owner, target) {
                    continue;
                }
                if let Some(parent) = self.nodes[target.0 as usize].parent {
                    self.nodes[parent.0 as usize].children.retain(|child| *child != target);
                }
                self.nodes[target.0 as usize].parent = Some(owner);
                self.nodes[owner.0 as usize].children.push(target);
                owned.insert(target);
            }
        }
    }

    /// True when `node` is `ancestor` or inside it.
    fn is_under(&self, node: AxNodeId, ancestor: AxNodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.nodes.get(id.0 as usize).and_then(|entry| entry.parent);
        }
        false
    }

    fn visit(&mut self, node: NodeIdx, parent: Option<AxNodeId>) {
        match self.classify(node) {
            Decision::Skip => {}
            Decision::Promote => {
                let children: Vec<NodeIdx> = self.dom.children(node).collect();
                for child in children {
                    self.visit(child, parent);
                }
            }
            Decision::Include => {
                let Some(id) = self.push_node(node, parent) else { return };
                let children: Vec<NodeIdx> = self.dom.children(node).collect();
                for child in children {
                    self.visit(child, Some(id));
                }
            }
        }
    }

    fn classify(&self, node: NodeIdx) -> Decision {
        match &self.dom.get(node).kind {
            NodeKind::Document | NodeKind::DocumentFragment => Decision::Promote,
            NodeKind::DocumentType { .. }
            | NodeKind::Comment { .. }
            | NodeKind::ProcessingInstruction { .. }
            | NodeKind::Attr { .. } => Decision::Skip,
            NodeKind::Text { data } | NodeKind::CDATASection { data } => {
                if accname::flatten(data).is_empty() || self.text_visibility_hidden(node) {
                    Decision::Skip
                } else {
                    Decision::Include
                }
            }
            NodeKind::Element { name, .. } => self.classify_element(node, name),
        }
    }

    /// A text node whose containing element computes `visibility:hidden` is not
    /// rendered; a descendant that resets `visibility:visible` still exposes its
    /// own text, so only the immediate element is consulted.
    fn text_visibility_hidden(&self, node: NodeIdx) -> bool {
        let Some(parent) = self.dom.get(node).parent else { return false };
        style_visibility_hidden(self.styled, parent)
    }

    fn classify_element(&self, node: NodeIdx, tag: &str) -> Decision {
        if is_non_rendered_tag(tag) {
            return Decision::Skip;
        }
        let element = node;
        if style_display_none(self.styled, element)
            || self.dom.get_attribute_ns(element, None, "hidden").is_some()
            || aria_hidden(self.dom, element)
        {
            return Decision::Skip;
        }
        let role = self.resolved_role(node);
        if matches!(role, Role::None | Role::Presentation) {
            return Decision::Promote;
        }
        // `visibility:hidden` prunes this node but a descendant that overrides
        // it back to `visible` is still exposed.
        if style_visibility_hidden(self.styled, element) {
            return Decision::Promote;
        }
        Decision::Include
    }

    /// Role after presentational conflict resolution: `none`/`presentation` is
    /// ignored on a focusable element or one with a global ARIA attribute, in
    /// which case the implicit role applies.
    fn resolved_role(&self, node: NodeIdx) -> Role {
        let explicit = role::explicit_role(self.dom, node);
        let Some(explicit) = explicit else {
            return role::implicit_role(self.dom, node);
        };
        if explicit.is_presentational()
            && (host::is_focusable(self.dom, node) || role::has_global_aria(self.dom, node))
        {
            let implicit = role::implicit_role(self.dom, node);
            if implicit.is_presentational() || implicit == Role::None {
                return Role::Generic;
            }
            return implicit;
        }
        explicit
    }

    fn push_node(&mut self, node: NodeIdx, parent: Option<AxNodeId>) -> Option<AxNodeId> {
        let role = match &self.dom.get(node).kind {
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } => Role::StaticText,
            NodeKind::Element { .. } => self.resolved_role(node),
            _ => return None,
        };
        let is_text = matches!(
            self.dom.get(node).kind,
            NodeKind::Text { .. } | NodeKind::CDATASection { .. }
        );
        let (name, description, states, properties) = if is_text {
            let data = match &self.dom.get(node).kind {
                NodeKind::Text { data } | NodeKind::CDATASection { data } => data.clone(),
                _ => String::new(),
            };
            (accname::flatten(&data), String::new(), aria::AxStates::default(), aria::AxProperties::default())
        } else {
            let (name, description) = {
                let ctx = NameContext::new(
                    self.dom,
                    self.styled,
                    &self.ids,
                    &self.labels_by_target,
                );
                (
                    accname::accessible_name(&ctx, node),
                    accname::accessible_description(&ctx, node),
                )
            };
            let states = aria::compute_states(self.dom, node, role);
            let properties = aria::compute_properties(self.dom, node, role);
            (name, description, states, properties)
        };
        let bounds = self.bounds.get(node);
        let id = AxNodeId(self.nodes.len() as u32);
        self.nodes.push(AxNode {
            id,
            dom_node: Some(node),
            role,
            name,
            description,
            states,
            properties,
            bounds,
            children: Vec::new(),
            parent,
            active_descendant: None,
            relations: AxRelations::default(),
        });
        if let Some(parent) = parent
            && let Some(entry) = self.nodes.get_mut(parent.0 as usize)
        {
            entry.children.push(id);
        }
        Some(id)
    }

    fn push_synthetic(&mut self, role: Role, parent: Option<AxNodeId>) -> AxNodeId {
        let id = AxNodeId(self.nodes.len() as u32);
        self.nodes.push(AxNode {
            id,
            dom_node: None,
            role,
            name: String::new(),
            description: String::new(),
            states: aria::AxStates::default(),
            properties: aria::AxProperties::default(),
            bounds: None,
            children: Vec::new(),
            parent,
            active_descendant: None,
            relations: AxRelations::default(),
        });
        if let Some(parent) = parent
            && let Some(entry) = self.nodes.get_mut(parent.0 as usize)
        {
            entry.children.push(id);
        }
        id
    }

    fn finish(self, root: Option<AxNodeId>) -> AxTree {
        let drop = self.ignorable();
        let mut out: Vec<AxNode> = Vec::with_capacity(self.nodes.len());
        let new_root = root.and_then(|root| self.emit(root, None, &drop, &mut out));
        AxTree { nodes: out, root: new_root }
    }

    /// Generic containers with no name whose children are only static text (or
    /// other flattened containers, or no children at all) are not exposed.
    /// Focusable or ARIA-bearing generics are always kept. Children are pushed
    /// before their parents, so a reverse pass reaches a fixpoint in one sweep.
    fn ignorable(&self) -> HashSet<AxNodeId> {
        let mut drop = HashSet::new();
        for node in self.nodes.iter().rev() {
            if node.role != Role::Generic
                || !node.name.is_empty()
                || node.states.focusable
                || !node.properties.raw.is_empty()
                // An explicit (even invalid/unrecognized) `role` attribute
                // must still surface an AxNode, falling back to `generic`:
                // ARIA 1.2 conflict resolution never removes the node, only
                // reassigns its role. Without this, an invalid-role element
                // like `<span role="foo">` would be silently pruned away.
                || node
                    .dom_node
                    .is_some_and(|dom_node| self.dom.get_attribute_ns(dom_node, None, "role").is_some())
            {
                continue;
            }
            let all_flattened = node.children.iter().all(|child| {
                drop.contains(child)
                    || matches!(
                        self.nodes.get(child.0 as usize).map(|n| n.role),
                        Some(Role::StaticText | Role::InlineTextBox | Role::LineBreak)
                    )
            });
            if node.children.is_empty() || all_flattened {
                drop.insert(node.id);
            }
        }
        drop
    }

    fn emit(
        &self,
        id: AxNodeId,
        parent: Option<AxNodeId>,
        drop: &HashSet<AxNodeId>,
        out: &mut Vec<AxNode>,
    ) -> Option<AxNodeId> {
        let source = self.nodes.get(id.0 as usize)?;
        if drop.contains(&id) {
            for child in &source.children {
                self.emit(*child, parent, drop, out);
            }
            return None;
        }
        let new_id = AxNodeId(out.len() as u32);
        let mut node = source.clone();
        node.id = new_id;
        node.parent = parent;
        node.children = Vec::new();
        let children = source.children.clone();
        out.push(node);
        let mut new_children = Vec::new();
        for child in children {
            if let Some(child_id) = self.emit(child, Some(new_id), drop, out) {
                new_children.push(child_id);
            }
        }
        out[new_id.0 as usize].children = new_children;
        Some(new_id)
    }
}

fn is_non_rendered_tag(tag: &str) -> bool {
    matches!(
        tag,
        "base"
            | "head"
            | "link"
            | "meta"
            | "noscript"
            | "param"
            | "script"
            | "slot"
            | "source"
            | "style"
            | "template"
            | "title"
            | "track"
    )
}

fn style_display_none(styled: &StyledDom, node: NodeIdx) -> bool {
    styled
        .get(node)
        .and_then(|style| style.get("display"))
        .is_some_and(|value| value.eq_ignore_ascii_case("none"))
}

fn style_visibility_hidden(styled: &StyledDom, node: NodeIdx) -> bool {
    styled
        .get(node)
        .and_then(|style| style.get("visibility"))
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")
        })
}

fn aria_hidden(dom: &Dom, node: NodeIdx) -> bool {
    dom.get_attribute_ns(node, None, "aria-hidden")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

/// Splits `attr` on `node` as a whitespace-separated id list and resolves each
/// id to a node in this tree, dropping ids that do not resolve to an id in
/// `ids` or a node exposed in `by_dom`.
fn resolve_ids(
    dom: &Dom,
    ids: &HashMap<String, NodeIdx>,
    by_dom: &HashMap<NodeIdx, AxNodeId>,
    node: NodeIdx,
    attr: &str,
) -> Vec<AxNodeId> {
    dom.get_attribute_ns(node, None, attr)
        .map(|value| {
            value
                .split_ascii_whitespace()
                .filter_map(|id| ids.get(id))
                .filter_map(|target| by_dom.get(target).copied())
                .collect()
        })
        .unwrap_or_default()
}

/// Roles that end the current line of accessible text.
fn breaks_line(role: Role) -> bool {
    use Role::*;
    matches!(
        role,
        Document
            | Paragraph
            | Heading
            | Blockquote
            | Caption
            | Definition
            | Term
            | Listitem
            | Cell
            | Columnheader
            | Rowheader
            | Gridcell
            | Row
            | Table
            | Article
            | Banner
            | Complementary
            | Contentinfo
            | Form
            | Main
            | Navigation
            | Region
            | Search
            | Figure
            | Feed
            | Log
            | Alert
            | Status
            | Dialog
            | Alertdialog
            | Group
    )
}
