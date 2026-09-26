//! ARIA state and property parsing.
//!
//! Only typed state needed by the snapshot is given a dedicated field; the raw
//! `aria-*` attributes are also retained (normalized, non-empty only) so a
//! consumer can read anything the typed model does not cover. Native host
//! language state (`checked`, `disabled`, `required`, `readonly`, `selected`,
//! `open`) is merged in with ARIA taking precedence, matching HTML-AAM.

use amazing_dom::{Dom, NodeIdx};
use std::collections::BTreeMap;

use crate::host;
use crate::role::{self, Role};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tristate {
    #[default]
    Undefined,
    False,
    True,
    Mixed,
}

impl Tristate {
    pub fn is_defined(self) -> bool {
        !matches!(self, Tristate::Undefined)
    }
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Tristate::True => Some(true),
            Tristate::False => Some(false),
            Tristate::Mixed | Tristate::Undefined => None,
        }
    }
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Tristate::Undefined => None,
            Tristate::False => Some("false"),
            Tristate::True => Some("true"),
            Tristate::Mixed => Some("mixed"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InvalidState {
    #[default]
    False,
    True,
    Spelling,
    Grammar,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    None,
    Ascending,
    Descending,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Orientation {
    #[default]
    Undefined,
    Horizontal,
    Vertical,
}

impl Orientation {
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Orientation::Horizontal => Some("horizontal"),
            Orientation::Vertical => Some("vertical"),
            Orientation::Undefined => None,
        }
    }
}

/// Typed accessibility states of one node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxStates {
    pub checked: Tristate,
    pub pressed: Tristate,
    pub selected: bool,
    pub expanded: Option<bool>,
    pub disabled: bool,
    pub required: bool,
    pub readonly: bool,
    pub invalid: InvalidState,
    pub busy: bool,
    pub modal: bool,
    /// `aria-hidden="true"` or the `hidden` attribute.
    pub hidden: bool,
    pub inert: bool,
    pub focusable: bool,
    pub focused: bool,
    pub editable: bool,
    pub multiselectable: bool,
    pub haspopup: bool,
}

/// `aria-relevant` as a set of change kinds. ARIA's default is
/// `additions text`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Relevant {
    pub additions: bool,
    pub removals: bool,
    pub text: bool,
    pub all: bool,
}

impl Default for Relevant {
    fn default() -> Self {
        Relevant { additions: true, removals: false, text: true, all: false }
    }
}

impl Relevant {
    /// Parses a whitespace-separated `aria-relevant` token list.
    pub fn parse(value: &str) -> Self {
        let mut relevant = Relevant { additions: false, removals: false, text: false, all: false };
        for token in value.split_ascii_whitespace() {
            match token.to_ascii_lowercase().as_str() {
                "additions" => relevant.additions = true,
                "removals" => relevant.removals = true,
                "text" => relevant.text = true,
                "all" => relevant.all = true,
                _ => {}
            }
        }
        relevant
    }
}

/// Typed accessibility properties. `raw` keeps every remaining `aria-*`
/// attribute verbatim (lowercased name, original value).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxProperties {
    pub level: Option<u32>,
    pub value_now: Option<f64>,
    pub value_min: Option<f64>,
    pub value_max: Option<f64>,
    pub value_text: Option<String>,
    pub orientation: Orientation,
    pub sort: Sort,
    pub autocomplete: Option<String>,
    pub placeholder: Option<String>,
    pub roledescription: Option<String>,
    pub keyshortcuts: Option<String>,
    pub current: Option<String>,
    pub live: Option<String>,
    pub relevant: Option<String>,
    pub relevant_flags: Relevant,
    pub atomic: bool,
    pub controls: Vec<String>,
    pub describedby: Vec<String>,
    pub details: Vec<String>,
    pub errormessage: Option<String>,
    pub owns: Vec<String>,
    pub flowto: Vec<String>,
    /// Position of the item in a set (1-based), and the size of that set.
    pub pos_in_set: Option<u32>,
    pub set_size: Option<u32>,
    /// Table/grid addressing; indices and counts are 1-based per ARIA.
    pub row_index: Option<u32>,
    pub row_index_text: Option<String>,
    pub row_count: Option<u32>,
    pub col_index: Option<u32>,
    pub col_index_text: Option<String>,
    pub col_count: Option<u32>,
    pub col_span: Option<u32>,
    pub row_span: Option<u32>,
    /// `aria-activedescendant` id reference (resolved to a tree id on the node).
    pub active_descendant: Option<String>,
    pub raw: BTreeMap<String, String>,
}

/// Reads a boolean ARIA attribute: `true`/empty is true, `false` is false,
/// anything else is `None`.
pub fn aria_bool(dom: &Dom, node: NodeIdx, name: &str) -> Option<bool> {
    let value = dom.get_attribute_ns(node, None, name)?.trim();
    match value.to_ascii_lowercase().as_str() {
        "true" | "" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn aria_tristate(dom: &Dom, node: NodeIdx, name: &str) -> Tristate {
    match dom.get_attribute_ns(node, None, name).map(str::trim) {
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "true" => Tristate::True,
            "false" => Tristate::False,
            "mixed" => Tristate::Mixed,
            _ => Tristate::Undefined,
        },
        None => Tristate::Undefined,
    }
}

fn aria_number(dom: &Dom, node: NodeIdx, name: &str) -> Option<f64> {
    dom.get_attribute_ns(node, None, name)?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// A non-negative integer ARIA attribute.
fn aria_u32(dom: &Dom, node: NodeIdx, name: &str) -> Option<u32> {
    let value = aria_number(dom, node, name)?;
    if value < 0.0 {
        return None;
    }
    u32::try_from(value as i64).ok()
}

fn id_list(dom: &Dom, node: NodeIdx, name: &str) -> Vec<String> {
    dom.get_attribute_ns(node, None, name)
        .map(|v| {
            v.split_ascii_whitespace()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Computes the typed states for `node` given its resolved `role`.
pub fn compute_states(dom: &Dom, node: NodeIdx, role: Role) -> AxStates {
    let mut states = AxStates {
        disabled: host::is_disabled(dom, node),
        required: host::is_required(dom, node),
        readonly: host::is_readonly(dom, node),
        focusable: host::is_focusable(dom, node),
        focused: dom.is_focused(node),
        ..AxStates::default()
    };

    if let Some(disabled) = aria_bool(dom, node, "aria-disabled") {
        states.disabled = disabled;
    }
    if let Some(required) = aria_bool(dom, node, "aria-required") {
        states.required = required;
    }
    if let Some(readonly) = aria_bool(dom, node, "aria-readonly") {
        states.readonly = readonly;
    }

    states.checked = match aria_tristate(dom, node, "aria-checked") {
        Tristate::Undefined => host::native_checked(dom, node)
            .map(|checked| if checked { Tristate::True } else { Tristate::False })
            .unwrap_or(Tristate::Undefined),
        other => other,
    };
    states.pressed = aria_tristate(dom, node, "aria-pressed");
    states.selected = match aria_bool(dom, node, "aria-selected") {
        Some(selected) => selected,
        None => host::native_selected(dom, node).unwrap_or(false),
    };
    states.expanded = match aria_bool(dom, node, "aria-expanded") {
        Some(expanded) => Some(expanded),
        None => host::native_expanded(dom, node),
    };

    states.invalid = match dom
        .get_attribute_ns(node, None, "aria-invalid")
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("true") | Some("") => InvalidState::True,
        Some("spelling") => InvalidState::Spelling,
        Some("grammar") => InvalidState::Grammar,
        _ => InvalidState::False,
    };
    states.busy = aria_bool(dom, node, "aria-busy").unwrap_or(false);
    states.modal = aria_bool(dom, node, "aria-modal").unwrap_or(false);
    states.hidden = aria_bool(dom, node, "aria-hidden") == Some(true)
        || dom.get_attribute_ns(node, None, "hidden").is_some();
    states.inert = host::is_inert(dom, node);
    states.multiselectable = aria_bool(dom, node, "aria-multiselectable").unwrap_or(false);
    states.haspopup = match dom.get_attribute_ns(node, None, "aria-haspopup") {
        Some(value) => !value.trim().is_empty() && !value.eq_ignore_ascii_case("false"),
        None => false,
    };
    states.editable = states.focusable
        && matches!(role, Role::Textbox | Role::Searchbox | Role::Combobox)
        && !states.readonly
        && !states.disabled;
    states
}

/// Computes the typed properties for `node`.
pub fn compute_properties(dom: &Dom, node: NodeIdx, role: Role) -> AxProperties {
    let mut props = AxProperties {
        level: aria_u32(dom, node, "aria-level"),
        value_now: aria_number(dom, node, "aria-valuenow"),
        value_min: aria_number(dom, node, "aria-valuemin"),
        value_max: aria_number(dom, node, "aria-valuemax"),
        value_text: non_empty(dom.get_attribute_ns(node, None, "aria-valuetext")),
        orientation: match dom
            .get_attribute_ns(node, None, "aria-orientation")
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("horizontal") => Orientation::Horizontal,
            Some("vertical") => Orientation::Vertical,
            _ => Orientation::Undefined,
        },
        sort: match dom
            .get_attribute_ns(node, None, "aria-sort")
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ascending") => Sort::Ascending,
            Some("descending") => Sort::Descending,
            Some("none") => Sort::None,
            Some(_) => Sort::Other,
            None => Sort::None,
        },
        autocomplete: non_empty(dom.get_attribute_ns(node, None, "aria-autocomplete")),
        placeholder: non_empty(dom.get_attribute_ns(node, None, "placeholder")),
        roledescription: non_empty(dom.get_attribute_ns(node, None, "aria-roledescription")),
        keyshortcuts: non_empty(dom.get_attribute_ns(node, None, "aria-keyshortcuts")),
        current: non_empty(dom.get_attribute_ns(node, None, "aria-current")),
        live: non_empty(dom.get_attribute_ns(node, None, "aria-live")),
        relevant: non_empty(dom.get_attribute_ns(node, None, "aria-relevant")),
        relevant_flags: Relevant::default(),
        atomic: aria_bool(dom, node, "aria-atomic").unwrap_or(false),
        controls: id_list(dom, node, "aria-controls"),
        describedby: id_list(dom, node, "aria-describedby"),
        details: id_list(dom, node, "aria-details"),
        errormessage: non_empty(dom.get_attribute_ns(node, None, "aria-errormessage")),
        owns: id_list(dom, node, "aria-owns"),
        flowto: id_list(dom, node, "aria-flowto"),
        pos_in_set: aria_u32(dom, node, "aria-posinset"),
        set_size: aria_u32(dom, node, "aria-setsize"),
        row_index: aria_u32(dom, node, "aria-rowindex"),
        row_index_text: non_empty(dom.get_attribute_ns(node, None, "aria-rowindextext")),
        row_count: aria_u32(dom, node, "aria-rowcount"),
        col_index: aria_u32(dom, node, "aria-colindex"),
        col_index_text: non_empty(dom.get_attribute_ns(node, None, "aria-colindextext")),
        col_count: aria_u32(dom, node, "aria-colcount"),
        col_span: aria_u32(dom, node, "aria-colspan"),
        row_span: aria_u32(dom, node, "aria-rowspan"),
        active_descendant: non_empty(
            dom.get_attribute_ns(node, None, "aria-activedescendant"),
        ),
        raw: BTreeMap::new(),
    };
    props.relevant_flags = props
        .relevant
        .as_deref()
        .map(Relevant::parse)
        .unwrap_or_default();

    // Native set position for collection items when ARIA does not provide it.
    let (native_pos, native_size) = native_set_position(dom, node, role);
    if props.pos_in_set.is_none() {
        props.pos_in_set = native_pos;
    }
    if props.set_size.is_none() {
        props.set_size = native_size;
    }

    // Heading level defaults follow HTML-AAM: h1..h6.
    if props.level.is_none()
        && role == Role::Heading
        && let Some(name) = dom.name(node)
        && let Some(level) = name.strip_prefix('h').and_then(|d| d.parse::<u32>().ok())
        && (1..=6).contains(&level)
    {
        props.level = Some(level);
    }

    if let Some(attrs) = dom.attrs_of(node) {
        for (name, value) in attrs {
            if name.starts_with("aria-") && !value.is_empty() {
                props.raw.insert(name.clone(), value.clone());
            }
        }
    }
    props
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.map(str::to_owned).filter(|v| !v.trim().is_empty())
}

/// Native `posinset`/`setsize` for collection items: the item's 1-based index
/// among its siblings that resolve to the same role. Only reported when the set
/// has more than one member and ARIA did not supply the values.
fn native_set_position(dom: &Dom, node: NodeIdx, role: Role) -> (Option<u32>, Option<u32>) {
    if !matches!(
        role,
        Role::Listitem
            | Role::Option
            | Role::Treeitem
            | Role::Tab
            | Role::Menuitem
            | Role::Menuitemcheckbox
            | Role::Menuitemradio
            | Role::Radio
    ) {
        return (None, None);
    }
    let Some(parent) = dom.get(node).parent else { return (None, None) };
    let mut position = None;
    let mut size = 0u32;
    for child in dom.children(parent) {
        if role::role_for(dom, child) == role {
            size += 1;
            if child == node {
                position = Some(size);
            }
        }
    }
    if position.is_some() && size > 1 {
        (position, Some(size))
    } else {
        (None, None)
    }
}
