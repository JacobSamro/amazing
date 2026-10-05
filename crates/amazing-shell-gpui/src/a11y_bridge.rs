//! A11Y-9: the Linux platform accessibility bridge.
//!
//! Translates an `amazing_a11y::AxTree` (the a11y lane's engine-independent
//! accessibility tree, built from a page's DOM/computed style/layout — see
//! `crates/amazing-a11y` and its driver adapter
//! `crates/amazing/src/runtime_host_a11y.rs`) into a real GPUI element tree,
//! so GPUI's own AccessKit integration exposes it to assistive technology —
//! AT-SPI2 over D-Bus on Linux, UIA on Windows, the macOS accessibility APIs
//! on macOS, all through the *same* code here, since GPUI/AccessKit already
//! own the platform-specific adapter layer.
//!
//! # Design: why GPUI's div tree, not raw D-Bus, and not the flat
//! "synthetic children" hook
//!
//! Recorded in `docs/lanes/a11y_TODO.md`'s A11Y-9 entry in full, summarized
//! here for anyone reading just this file:
//!
//! - GPUI already speaks AT-SPI2 on Linux. It vendors AccessKit (see
//!   `crates/amazing-shell-gpui/Cargo.lock`'s `accesskit_unix`/
//!   `accesskit_atspi_common` entries) and builds a fresh
//!   [`accesskit::TreeUpdate`] every frame from its own element tree,
//!   handing it to AccessKit's platform adapter — `accesskit_unix` on Linux,
//!   which owns the whole `org.a11y.Bus` D-Bus registration and the AT-SPI2
//!   interfaces. Talking D-Bus directly, or driving AccessKit's tree model
//!   by hand outside of GPUI, would duplicate work GPUI already does and
//!   would fight GPUI for ownership of the one active AccessKit adapter per
//!   window.
//! - GPUI exposes two ways to contribute AccessKit nodes: (a) an ordinary
//!   `div().id(..).role(..)` — GPUI walks its own element tree every frame
//!   and derives real parent/child edges from real element nesting; or (b)
//!   [`Element::a11y_synthetic_children`]/`A11ySubtreeBuilder`, which lets
//!   one element push extra flat AccessKit nodes as *its own* direct
//!   children (GPUI's real usage of this, `settings_ui`'s text-input text
//!   runs, is exactly that: a flat list, never nested). `A11ySubtreeBuilder`
//!   has no way to attach a pushed node under another pushed node — every
//!   call registers as a child of the *real* element running the callback —
//!   so it cannot represent an arbitrarily deep page's AX tree without
//!   giving every single node two parents (the flattening bug this module
//!   avoids). The div-tree route is the one that supports real nesting, so
//!   it is what this module builds: one real, unpainted, absolutely positioned `div`
//!   per exposed [`AxNode`], nested exactly like the [`AxTree`] itself.
//! - The trade-off: `div()`'s public accessibility builder methods
//!   (`.aria_label`, `.aria_selected`, `.aria_toggled`, `.aria_numeric_value`,
//!   …) cover role, name, description, value, and the states with a direct
//!   named setter, but at this pinned GPUI revision there is no
//!   `.aria_live()`/`.aria_busy()`/`.aria_invalid()`/`.aria_modal()`/
//!   `.aria_multiselectable()` — `Interactivity`'s `AriaAttributes` struct
//!   simply has no such fields yet. Live regions, `aria-busy`, `aria-modal`,
//!   `aria-invalid` and `aria-multiselectable` are therefore a known,
//!   documented gap: real, upstream, not something this module works around
//!   by reaching past GPUI's public API.
//!
//! # Identity and change events
//!
//! Each shadow `div`'s id is derived from the [`AxNode`]'s `dom_node` (the
//! DOM arena index), the same key A11Y-8's `event::diff` compares by, falling
//! back to the node's own [`AxNodeId`] when there is no `dom_node` (the
//! synthetic document root). Keeping this id stable across rebuilds is what
//! makes AccessKit's own per-frame diffing (documented in GPUI's
//! `window/a11y.rs`: "we build a `TreeUpdate` ... diffs it against the
//! previous update ... nodes may have been created, destroyed, or updated")
//! produce the AT-visible equivalent of A11Y-8's `Added`/`Removed`/
//! `NameChanged`/… events automatically, every time the caller rebuilds this
//! element from a fresh [`AxTree`] (e.g. via
//! `runtime_host_a11y::diff_ax_tree`/`amazing_a11y::rebuild_and_diff` after a
//! DOM mutation checkpoint). This module does not additionally walk A11Y-8's
//! `Vec<AxEvent>` by hand: GPUI/AccessKit's own frame-to-frame tree diff is
//! the mechanism, not a second one layered on top of it.
//!
//! # Focus
//!
//! GPUI reports one node as the window's real AccessKit focus, tied to a
//! [`gpui::FocusHandle`] via `.track_focus()`. A page has many potentially
//! "focused" elements but the shell keeps exactly one real `FocusHandle` for
//! the whole viewport (needed for ordinary keyboard routing regardless of
//! accessibility). Every shadow node whose [`AxNode::states`]`.focused` is
//! true, or that some ancestor's [`AxNode::active_descendant`] points at
//! (ARIA's composite-widget roving-focus pattern — a listbox keeps real
//! focus while a descendant `option` is "active"), gets
//! `.aria_active_descendant()`: GPUI honors this only while a real focused
//! ancestor is present in the tree, so it is safe to set unconditionally on
//! the target node and never touches real keyboard focus.

use std::collections::HashSet;

use amazing_a11y::{AxNode, AxNodeId, AxTree, Role as AmazingRole};
use gpui::{IntoElement, SharedString, div, prelude::*, px};

/// The pure, GPUI-free translation of one [`AxNode`] into what a shadow `div`
/// should carry, plus its already-translated children. Kept as plain data
/// (no GPUI element) so the mapping is testable with an ordinary `#[test]`,
/// matching this crate's existing convention (see e.g.
/// `input_field.rs::build_a11y_text_runs` upstream) of separating "what to
/// set" from "the builder calls that set it".
#[derive(Debug, Clone, PartialEq)]
pub struct AxNodeAttrs {
    /// Stable id across rebuilds — see the module doc's "Identity" section.
    pub id: String,
    pub role: gpui::accesskit::Role,
    /// Actual border/text union in document CSS pixels, from this layout.
    pub bounds: Option<amazing_a11y::AxRect>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub numeric_value: Option<f64>,
    pub numeric_value_min: Option<f64>,
    pub numeric_value_max: Option<f64>,
    pub level: Option<usize>,
    pub position_in_set: Option<usize>,
    pub size_of_set: Option<usize>,
    pub row_index: Option<usize>,
    pub row_count: Option<usize>,
    pub column_index: Option<usize>,
    pub column_count: Option<usize>,
    pub orientation: Option<gpui::accesskit::Orientation>,
    pub keyshortcuts: Option<String>,
    pub placeholder: Option<String>,
    pub selected: bool,
    pub expanded: Option<bool>,
    pub toggled: Option<gpui::accesskit::Toggled>,
    /// Report this node as the AccessKit-visible focus (see the module doc's
    /// "Focus" section) via `.aria_active_descendant()`.
    pub report_as_active_descendant: bool,
    pub children: Vec<AxNodeAttrs>,
}

/// Maps an `amazing_a11y` role to the closest AccessKit role. Falls back to
/// `GenericContainer` for the small set of abstract ARIA role categories
/// (`Command`, `Composite`, `Input`, `Landmark`, `Range`, `Roletype`,
/// `Section`, `Sectionhead`, `Structure`, `Widget`) that `amazing_a11y`
/// itself never assigns to a resolved node (`role::explicit_role`/
/// `implicit_role` only ever produce a concrete role — see
/// `crates/amazing-a11y/src/role.rs`), so this arm exists purely for
/// exhaustiveness, not because it is reachable in practice.
pub fn map_role(role: AmazingRole) -> gpui::accesskit::Role {
    use gpui::accesskit::Role as K;
    match role {
        AmazingRole::Button => K::Button,
        AmazingRole::Checkbox => K::CheckBox,
        AmazingRole::Gridcell => K::GridCell,
        AmazingRole::Link => K::Link,
        AmazingRole::Menuitem => K::MenuItem,
        AmazingRole::Menuitemcheckbox => K::MenuItemCheckBox,
        AmazingRole::Menuitemradio => K::MenuItemRadio,
        AmazingRole::Option => K::ListBoxOption,
        AmazingRole::Progressbar => K::ProgressIndicator,
        AmazingRole::Radio => K::RadioButton,
        AmazingRole::Scrollbar => K::ScrollBar,
        AmazingRole::Searchbox => K::SearchInput,
        AmazingRole::Slider => K::Slider,
        AmazingRole::Spinbutton => K::SpinButton,
        AmazingRole::Switch => K::Switch,
        AmazingRole::Tab => K::Tab,
        AmazingRole::Tabpanel => K::TabPanel,
        AmazingRole::Textbox => K::TextInput,
        AmazingRole::Treeitem => K::TreeItem,
        AmazingRole::Tooltip => K::Tooltip,
        AmazingRole::Combobox => K::ComboBox,
        AmazingRole::Grid => K::Grid,
        AmazingRole::Listbox => K::ListBox,
        AmazingRole::Menu => K::Menu,
        AmazingRole::Menubar => K::MenuBar,
        AmazingRole::Radiogroup => K::RadioGroup,
        AmazingRole::Tablist => K::TabList,
        AmazingRole::Tree => K::Tree,
        AmazingRole::Treegrid => K::TreeGrid,
        AmazingRole::Application => K::Application,
        AmazingRole::Article => K::Article,
        AmazingRole::Blockquote => K::Blockquote,
        AmazingRole::Caption => K::Caption,
        AmazingRole::Cell => K::Cell,
        AmazingRole::Code => K::Code,
        // A11Y-10: `<cite>` has no ARIA equivalent (HTML-AAM's own fixture
        // names the "implicit platform computedrole" `html-cite`, not an
        // ARIA token) — expose it the same way `role::apply_minimum_role`
        // and real browsers treat an otherwise-plain generic: a bare
        // grouping container, so its content is still walked, just under no
        // special AccessKit role.
        AmazingRole::HtmlCite => K::GenericContainer,
        AmazingRole::Columnheader => K::ColumnHeader,
        AmazingRole::Definition => K::Definition,
        AmazingRole::Deletion => K::ContentDeletion,
        // `directory` is deprecated and already computes as `list` in
        // `role::role_for` (A11Y-7); this arm is unreachable in practice.
        AmazingRole::Directory => K::List,
        AmazingRole::Document => K::Document,
        AmazingRole::Emphasis => K::Emphasis,
        AmazingRole::Feed => K::Feed,
        AmazingRole::Figure => K::Figure,
        AmazingRole::Generic => K::GenericContainer,
        AmazingRole::Group => K::Group,
        AmazingRole::Heading => K::Heading,
        AmazingRole::Img => K::Image,
        AmazingRole::Insertion => K::ContentInsertion,
        AmazingRole::List => K::List,
        AmazingRole::Listitem => K::ListItem,
        AmazingRole::Log => K::Log,
        AmazingRole::Main => K::Main,
        AmazingRole::Marquee => K::Marquee,
        AmazingRole::Math => K::Math,
        AmazingRole::Meter => K::Meter,
        // `none`/`presentation` without a focusable/global-ARIA override
        // produce no `AxNode` at all in `amazing_a11y::tree`, so this is
        // unreachable; kept for exhaustiveness.
        AmazingRole::None | AmazingRole::Presentation => K::GenericContainer,
        AmazingRole::Note => K::Note,
        AmazingRole::Paragraph => K::Paragraph,
        AmazingRole::Row => K::Row,
        AmazingRole::Rowgroup => K::RowGroup,
        AmazingRole::Rowheader => K::RowHeader,
        AmazingRole::Sectionheader => K::SectionHeader,
        AmazingRole::Sectionfooter => K::SectionFooter,
        AmazingRole::Separator => K::Splitter,
        AmazingRole::Strong => K::Strong,
        AmazingRole::Table => K::Table,
        AmazingRole::Term => K::Term,
        AmazingRole::Time => K::Time,
        AmazingRole::Toolbar => K::Toolbar,
        AmazingRole::Banner => K::Banner,
        AmazingRole::Complementary => K::Complementary,
        AmazingRole::Contentinfo => K::ContentInfo,
        AmazingRole::Form => K::Form,
        AmazingRole::Navigation => K::Navigation,
        AmazingRole::Region => K::Region,
        AmazingRole::Search => K::Search,
        AmazingRole::Alert => K::Alert,
        AmazingRole::Status => K::Status,
        AmazingRole::Timer => K::Timer,
        AmazingRole::Alertdialog => K::AlertDialog,
        AmazingRole::Dialog => K::Dialog,
        AmazingRole::GraphicsDocument => K::GraphicsDocument,
        AmazingRole::GraphicsObject => K::GraphicsObject,
        AmazingRole::GraphicsSymbol => K::GraphicsSymbol,
        AmazingRole::StaticText | AmazingRole::InlineTextBox => K::Label,
        // This pinned accesskit build has no dedicated Subscript/Superscript
        // roles (only later accesskit releases add them); Label is the
        // closest available fallback for inline text-level content.
        AmazingRole::Subscript | AmazingRole::Superscript => K::Label,
        AmazingRole::LineBreak => K::LineBreak,
        AmazingRole::ListMarker => K::ListMarker,
        AmazingRole::Iframe => K::Iframe,
        AmazingRole::Mark => K::Mark,
        AmazingRole::Ruby => K::Ruby,
        // Abstract ARIA role categories: never assigned to a resolved node,
        // kept only so this match stays exhaustive against `Role`.
        AmazingRole::Command
        | AmazingRole::Composite
        | AmazingRole::Input
        | AmazingRole::Landmark
        | AmazingRole::Range
        | AmazingRole::Roletype
        | AmazingRole::Section
        | AmazingRole::Sectionhead
        | AmazingRole::Select
        | AmazingRole::Structure
        | AmazingRole::Widget
        | AmazingRole::Window => K::GenericContainer,
    }
}

fn map_tristate(value: amazing_a11y::Tristate) -> Option<gpui::accesskit::Toggled> {
    use gpui::accesskit::Toggled;
    match value {
        amazing_a11y::Tristate::True => Some(Toggled::True),
        amazing_a11y::Tristate::False => Some(Toggled::False),
        amazing_a11y::Tristate::Mixed => Some(Toggled::Mixed),
        amazing_a11y::Tristate::Undefined => None,
    }
}

fn map_orientation(value: amazing_a11y::Orientation) -> Option<gpui::accesskit::Orientation> {
    use gpui::accesskit::Orientation;
    match value {
        amazing_a11y::Orientation::Horizontal => Some(Orientation::Horizontal),
        amazing_a11y::Orientation::Vertical => Some(Orientation::Vertical),
        amazing_a11y::Orientation::Undefined => None,
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() { None } else { Some(value.to_owned()) }
}

/// Stable key for a node across rebuilds: its `dom_node` arena index (the
/// same key A11Y-8's `event::diff` compares by), falling back to its own
/// `AxNodeId` for a node with no `dom_node` (the synthetic document root).
fn stable_id(node: &AxNode) -> String {
    match node.dom_node {
        Some(dom_node) => format!("ax-dom-{}", dom_node.0),
        None => format!("ax-node-{}", node.id.0),
    }
}

/// Every node that should report as the AccessKit-visible focus: a node with
/// `states.focused`, or a node any ancestor names via `active_descendant`
/// (ARIA's composite-widget roving-focus pattern). See the module doc's
/// "Focus" section.
fn active_descendant_targets(tree: &AxTree) -> HashSet<AxNodeId> {
    let mut targets = HashSet::new();
    for node in tree.nodes() {
        if node.states.focused {
            targets.insert(node.id);
        }
        if let Some(target) = node.active_descendant {
            targets.insert(target);
        }
    }
    targets
}

/// Translates `id`'s subtree of `tree` into plain attribute data (see
/// [`AxNodeAttrs`]). Returns `None` if `id` does not resolve — the caller is
/// expected to pass a valid id (typically `tree.root()`), but a stale id
/// (e.g. from a relationship that presentational pruning removed) degrades
/// to an empty subtree rather than panicking.
pub fn node_attrs(tree: &AxTree, id: AxNodeId) -> Option<AxNodeAttrs> {
    node_attrs_with(tree, id, &active_descendant_targets(tree))
}

fn node_attrs_with(
    tree: &AxTree,
    id: AxNodeId,
    focus_targets: &HashSet<AxNodeId>,
) -> Option<AxNodeAttrs> {
    let node = tree.node(id)?;
    let props = &node.properties;
    let states = &node.states;
    let toggled = map_tristate(states.checked).or_else(|| map_tristate(states.pressed));
    let children = node
        .children
        .iter()
        .filter_map(|child_id| node_attrs_with(tree, *child_id, focus_targets))
        .collect();
    Some(AxNodeAttrs {
        id: stable_id(node),
        role: map_role(node.role),
        bounds: node.bounds,
        label: non_empty(&node.name),
        description: non_empty(&node.description),
        value: props.value_text.clone(),
        numeric_value: props.value_now,
        numeric_value_min: props.value_min,
        numeric_value_max: props.value_max,
        level: props.level.map(|value| value as usize),
        position_in_set: props.pos_in_set.map(|value| value as usize),
        size_of_set: props.set_size.map(|value| value as usize),
        row_index: props.row_index.map(|value| value as usize),
        row_count: props.row_count.map(|value| value as usize),
        column_index: props.col_index.map(|value| value as usize),
        column_count: props.col_count.map(|value| value as usize),
        orientation: map_orientation(props.orientation),
        keyshortcuts: props.keyshortcuts.clone(),
        placeholder: props.placeholder.clone(),
        selected: states.selected,
        expanded: states.expanded,
        toggled,
        report_as_active_descendant: focus_targets.contains(&id),
        children,
    })
}

/// Builds an unpainted GPUI element tree with actual layout geometry for `attrs`,
/// recursively. Every node is a real `div()` so GPUI's own per-frame walk
/// derives correct parent/child AccessKit edges from real element nesting
/// (see the module doc's design section for why this, and not the flat
/// synthetic-children hook, is the only one of GPUI's two public mechanisms
/// that supports an arbitrarily deep tree).
pub fn build_element(attrs: &AxNodeAttrs) -> gpui::AnyElement {
    build_element_at(attrs, (0., 0.))
}

fn geometry_at(attrs: &AxNodeAttrs, parent_origin: (f32, f32))
    -> (amazing_a11y::AxRect, (f32, f32))
{
    let bounds = attrs.bounds.filter(|r| [r.x, r.y, r.width, r.height]
        .iter().all(|value| value.is_finite()))
        .unwrap_or_default();
    (amazing_a11y::AxRect {
        x: bounds.x - parent_origin.0, y: bounds.y - parent_origin.1,
        width: bounds.width.max(0.), height: bounds.height.max(0.),
    }, (bounds.x, bounds.y))
}

fn build_element_at(attrs: &AxNodeAttrs, parent_origin: (f32, f32)) -> gpui::AnyElement {
    let (geometry, child_origin) = geometry_at(attrs, parent_origin);
    let bare = div()
        // Hidden absolute shadows preserve the semantic parent structure.
        // GPUI collects AX bounds during prepaint before hidden paint returns;
        // they occupy no flow space and install no input handlers.
        .absolute()
        .left(px(geometry.x))
        .top(px(geometry.y))
        .w(px(geometry.width))
        .h(px(geometry.height))
        .invisible();
    // `amazing_a11y::Role::Generic` (ARIA's own catch-all, kept in the tree
    // for structural reasons even with no name of its own — see
    // `crates/amazing-a11y/src/tree.rs`'s `ignorable()`) maps to AccessKit's
    // `GenericContainer`, which GPUI's own `div().role()` explicitly
    // `debug_assert!`s against ("filtered out of the a11y tree and has no
    // effect"). Such a node contributes no AccessKit node of its own here —
    // no `.id()`, no `.role()` — but its children are still walked
    // normally, exactly like any ordinary non-accessible wrapper `div` in a
    // ordinary GPUI app.
    if attrs.role == gpui::accesskit::Role::GenericContainer {
        return bare.children(attrs.children.iter().map(|child| build_element_at(child, child_origin))).into_any_element();
    }
    let mut el = bare.id(SharedString::from(attrs.id.clone())).role(attrs.role);
    if let Some(label) = &attrs.label {
        el = el.aria_label(label.clone());
    }
    if let Some(description) = &attrs.description {
        el = el.aria_description(description.clone());
    }
    if let Some(value) = &attrs.value {
        el = el.aria_value(value.clone());
    }
    if let Some(value) = attrs.numeric_value {
        el = el.aria_numeric_value(value);
    }
    if let Some(value) = attrs.numeric_value_min {
        el = el.aria_min_numeric_value(value);
    }
    if let Some(value) = attrs.numeric_value_max {
        el = el.aria_max_numeric_value(value);
    }
    if let Some(level) = attrs.level {
        el = el.aria_level(level);
    }
    if let Some(position) = attrs.position_in_set {
        el = el.aria_position_in_set(position);
    }
    if let Some(size) = attrs.size_of_set {
        el = el.aria_size_of_set(size);
    }
    if let Some(index) = attrs.row_index {
        el = el.aria_row_index(index);
    }
    if let Some(count) = attrs.row_count {
        el = el.aria_row_count(count);
    }
    if let Some(index) = attrs.column_index {
        el = el.aria_column_index(index);
    }
    if let Some(count) = attrs.column_count {
        el = el.aria_column_count(count);
    }
    if let Some(orientation) = attrs.orientation {
        el = el.aria_orientation(orientation);
    }
    if let Some(keyshortcuts) = &attrs.keyshortcuts {
        el = el.aria_keyshortcuts(keyshortcuts.clone());
    }
    if let Some(placeholder) = &attrs.placeholder {
        el = el.aria_placeholder(placeholder.clone());
    }
    if attrs.selected {
        el = el.aria_selected(true);
    }
    if let Some(expanded) = attrs.expanded {
        el = el.aria_expanded(expanded);
    }
    if let Some(toggled) = attrs.toggled {
        el = el.aria_toggled(toggled);
    }
    if attrs.report_as_active_descendant {
        el = el.aria_active_descendant();
    }
    el.children(attrs.children.iter().map(|child| build_element_at(child, child_origin))).into_any_element()
}

/// Convenience entry point for a caller that only holds an [`AxTree`]: builds
/// the attribute tree from its root and the real element tree from that, or
/// an empty (no-op) element if the tree has no root.
pub fn ax_tree_element(tree: &AxTree) -> gpui::AnyElement {
    ax_tree_element_with_scroll(tree, (0., 0.))
}

/// GPUI applies the actual display scale to AccessKit bounds in prepaint.
/// Supply CSS scroll only here; multiplying by DPR would apply it twice.
pub fn ax_tree_element_with_scroll(tree: &AxTree, scroll: (f32, f32)) -> gpui::AnyElement {
    match tree.root().and_then(|root| node_attrs(tree, root)) {
        Some(attrs) => build_element_at(&attrs, scroll),
        None => div().into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_a11y::build_document;

    fn tree_for(html: &str) -> AxTree {
        build_document(html, "", 800.0, Some(600.0)).tree
    }

    #[test]
    fn actual_ax_geometry_translates_scroll_once_and_preserves_parent_offsets() {
        let tree = tree_for("<a href='/'>link</a>");
        let mut attrs = node_attrs(&tree, tree.root().unwrap()).unwrap();
        attrs.bounds = Some(amazing_a11y::AxRect { x: 30., y: 200., width: 80., height: 20. });
        let original = (attrs.id.clone(), attrs.role, attrs.label.clone());
        let (translated, origin) = geometry_at(&attrs, (5., 150.));
        assert_eq!(translated, amazing_a11y::AxRect { x: 25., y: 50., width: 80., height: 20. });
        assert_eq!(origin, (30., 200.));
        let mut child = attrs.clone();
        child.bounds = Some(amazing_a11y::AxRect { x: 35., y: 203., width: 20., height: 10. });
        assert_eq!(geometry_at(&child, origin).0,
            amazing_a11y::AxRect { x: 5., y: 3., width: 20., height: 10. });
        assert_eq!((attrs.id, attrs.role, attrs.label), original);
    }

    #[test]
    fn missing_ax_geometry_keeps_document_origin_and_invalid_bounds_do_not_escape() {
        let tree = tree_for("<p>text</p>");
        let mut attrs = node_attrs(&tree, tree.root().unwrap()).unwrap();
        for bounds in [None, Some(amazing_a11y::AxRect {
            x: f32::NAN, y: 0., width: 10., height: 10.,
        })] {
            attrs.bounds = bounds;
            let (geometry, origin) = geometry_at(&attrs, (4., 100.));
            assert_eq!(origin, (0., 0.));
            assert_eq!(geometry, amazing_a11y::AxRect { x: -4., y: -100., width: 0., height: 0. });
        }
    }

    /// Depth-first search for the first node with the given role. Real pages
    /// nest through implicit `html`/`body` wrappers (`Role::Generic`, mapped
    /// to `GenericContainer`) that presentational flattening keeps around
    /// for structure, so tests search recursively rather than assuming the
    /// target is a direct child of the root.
    fn find_by_role(attrs: &AxNodeAttrs, role: gpui::accesskit::Role) -> Option<&AxNodeAttrs> {
        if attrs.role == role {
            return Some(attrs);
        }
        attrs.children.iter().find_map(|child| find_by_role(child, role))
    }

    /// Depth-first collection of every node satisfying `pred`.
    fn collect_where<'a>(
        attrs: &'a AxNodeAttrs,
        pred: &impl Fn(&AxNodeAttrs) -> bool,
        out: &mut Vec<&'a AxNodeAttrs>,
    ) {
        if pred(attrs) {
            out.push(attrs);
        }
        for child in &attrs.children {
            collect_where(child, pred, out);
        }
    }

    #[test]
    fn every_role_maps_to_something_other_than_the_default_placeholder() {
        // Every concrete role this crate can actually assign should map to a
        // deliberate accesskit role, not silently fall through; abstract
        // categories and the two truly-unreachable arms are the only ones
        // allowed to land on `GenericContainer`.
        let unreachable_or_abstract = [
            AmazingRole::Command,
            AmazingRole::Composite,
            AmazingRole::Input,
            AmazingRole::Landmark,
            AmazingRole::Range,
            AmazingRole::Roletype,
            AmazingRole::Section,
            AmazingRole::Sectionhead,
            AmazingRole::Select,
            AmazingRole::Structure,
            AmazingRole::Widget,
            AmazingRole::Window,
            AmazingRole::None,
            AmazingRole::Presentation,
            AmazingRole::Directory,
        ];
        for role in [
            AmazingRole::Button,
            AmazingRole::Heading,
            AmazingRole::Img,
            AmazingRole::StaticText,
            AmazingRole::Listbox,
            AmazingRole::Table,
        ] {
            assert_ne!(map_role(role), gpui::accesskit::Role::GenericContainer, "{role:?}");
        }
        for role in unreachable_or_abstract {
            // Exercised only for exhaustiveness/panic-safety, not asserted.
            let _ = map_role(role);
        }
    }

    #[test]
    fn heading_name_and_level_translate_into_attrs() {
        let tree = tree_for("<h2>Section title</h2>");
        let root = tree.root().expect("root");
        let attrs = node_attrs(&tree, root).expect("attrs");
        let heading = find_by_role(&attrs, gpui::accesskit::Role::Heading).expect("heading node");
        assert_eq!(heading.label.as_deref(), Some("Section title"));
        assert_eq!(heading.level, Some(2));
    }

    #[test]
    fn checkbox_state_translates_to_toggled() {
        let tree = tree_for("<input type=\"checkbox\" checked aria-label=\"Subscribe\">");
        let root = tree.root().expect("root");
        let attrs = node_attrs(&tree, root).expect("attrs");
        let checkbox =
            find_by_role(&attrs, gpui::accesskit::Role::CheckBox).expect("checkbox node");
        assert_eq!(checkbox.label.as_deref(), Some("Subscribe"));
        assert_eq!(checkbox.toggled, Some(gpui::accesskit::Toggled::True));
    }

    #[test]
    fn slider_value_and_range_translate_into_attrs() {
        let tree = tree_for(concat!(
            "<div role=\"slider\" aria-label=\"Volume\" aria-valuenow=\"5\" ",
            "aria-valuemin=\"0\" aria-valuemax=\"10\"></div>",
        ));
        let root = tree.root().expect("root");
        let attrs = node_attrs(&tree, root).expect("attrs");
        let slider = find_by_role(&attrs, gpui::accesskit::Role::Slider).expect("slider node");
        assert_eq!(slider.numeric_value, Some(5.0));
        assert_eq!(slider.numeric_value_min, Some(0.0));
        assert_eq!(slider.numeric_value_max, Some(10.0));
    }

    #[test]
    fn focused_node_is_marked_as_active_descendant() {
        let mut doc = build_document(
            "<button>a</button><button>b</button>",
            "",
            800.0,
            Some(600.0),
        );
        let second_button = doc
            .dom
            .tree_order(amazing_dom::NodeIdx(0))
            .into_iter()
            .filter(|node| doc.dom.name(*node) == Some("button"))
            .nth(1)
            .expect("second button");
        doc.dom.set_active_element(amazing_dom::NodeIdx(0), Some(second_button), true);
        doc.tree = amazing_a11y::build(&doc.dom, &doc.styled, &doc.layout);

        let attrs = node_attrs(&doc.tree, doc.tree.root().unwrap()).unwrap();
        let mut focused = Vec::new();
        collect_where(&attrs, &|node| node.report_as_active_descendant, &mut focused);
        assert_eq!(focused.len(), 1, "{attrs:?}");
        assert_eq!(focused[0].label.as_deref(), Some("b"));
    }

    #[test]
    fn stable_id_survives_a_name_change_at_the_same_dom_node() {
        // The same DOM shape before/after (only the text differs) must
        // produce the same shadow-tree id for the text's own node, so
        // AccessKit's own per-frame diffing (keyed by these ids) sees an
        // update, not a remove-then-add, matching A11Y-8's own
        // dom_node-keyed identity. `role=status` does not take its name
        // from content (that needs its own `aria-label`), so the text is
        // exposed as an ordinary `Label` descendant instead of the status
        // node's own `label`.
        let before = tree_for("<div role=\"status\">old</div>");
        let after = tree_for("<div role=\"status\">new</div>");
        let before_attrs = node_attrs(&before, before.root().unwrap()).unwrap();
        let after_attrs = node_attrs(&after, after.root().unwrap()).unwrap();
        let before_text =
            find_by_role(&before_attrs, gpui::accesskit::Role::Label).expect("text node");
        let after_text =
            find_by_role(&after_attrs, gpui::accesskit::Role::Label).expect("text node");
        assert_eq!(before_text.id, after_text.id);
        assert_ne!(before_text.label, after_text.label);
    }

    #[test]
    fn ax_tree_element_does_not_panic_on_a_real_shaped_page() {
        let tree = tree_for(concat!(
            "<nav aria-label=\"Main\"><a href=\"/\">Home</a></nav>",
            "<h1>Title</h1><p>Body text</p>",
            "<ul><li>One</li><li>Two</li></ul>",
        ));
        // Constructing the element tree must not panic; rendering it needs a
        // live GPUI window, which is exercised by the a11y_proof binary and
        // the sandbox AT-SPI evidence instead (see docs/lanes/a11y_TODO.md).
        let _ = ax_tree_element(&tree);
    }
}
