//! HTML host-language facts used by role, state and name computation:
//! focusability, labelability, native control state, and the host-language
//! text alternatives (`alt`, `placeholder`, `title`, associated `<label>`).

use amazing_dom::{Dom, Namespace, NodeIdx, NodeKind};

/// True for elements the browser can move focus to, per HTML's focusable
/// area definition (excluding disabled controls).
pub fn is_focusable(dom: &Dom, node: NodeIdx) -> bool {
    if !matches!(dom.get(node).kind, NodeKind::Element { .. }) {
        return false;
    }
    if is_disabled(dom, node) {
        return false;
    }
    if let Some(tabindex) = dom.get_attribute_ns(node, None, "tabindex") {
        if tabindex.trim().parse::<i64>().is_ok() {
            return true;
        }
    }
    if has_contenteditable(dom, node) {
        return true;
    }
    let name = dom.name(node).unwrap_or_default();
    let attr = |a: &str| dom.get_attribute_ns(node, None, a).is_some();
    match name {
        "a" | "area" => attr("href"),
        "button" | "select" | "textarea" | "iframe" | "object" | "embed" => !is_disabled(dom, node),
        "input" => {
            dom.get_attribute_ns(node, None, "type")
                .is_none_or(|t| !t.eq_ignore_ascii_case("hidden"))
                && !is_disabled(dom, node)
        }
        "audio" | "video" => attr("controls"),
        "summary" => true,
        _ => false,
    }
}

fn has_contenteditable(dom: &Dom, node: NodeIdx) -> bool {
    match dom.get_attribute_ns(node, None, "contenteditable") {
        Some(value) => !value.eq_ignore_ascii_case("false"),
        None => false,
    }
}

/// True when the element is inert: it or any ancestor carries the `inert`
/// attribute.
pub fn is_inert(dom: &Dom, node: NodeIdx) -> bool {
    let mut cursor = Some(node);
    while let Some(current) = cursor {
        if dom.get_attribute_ns(current, None, "inert").is_some() {
            return true;
        }
        cursor = dom.get(current).parent;
    }
    false
}

/// True when the element's tabindex makes it sequentially focusable (`>= 0`).
pub fn is_tab_stop(dom: &Dom, node: NodeIdx) -> bool {
    match dom
        .get_attribute_ns(node, None, "tabindex")
        .and_then(|v| v.trim().parse::<i64>().ok())
    {
        Some(value) => value >= 0,
        None => is_natively_tab_focusable(dom, node),
    }
}

fn is_natively_tab_focusable(dom: &Dom, node: NodeIdx) -> bool {
    if is_disabled(dom, node) {
        return false;
    }
    match dom.name(node).unwrap_or_default() {
        "a" | "area" => dom.get_attribute_ns(node, None, "href").is_some(),
        "button" | "select" | "textarea" | "iframe" => true,
        "input" => dom
            .get_attribute_ns(node, None, "type")
            .is_none_or(|t| !t.eq_ignore_ascii_case("hidden")),
        "audio" | "video" => dom.get_attribute_ns(node, None, "controls").is_some(),
        _ => false,
    }
}

/// True when a `<label for>` can name this element.
pub fn is_labelable(dom: &Dom, node: NodeIdx) -> bool {
    match dom.name(node).unwrap_or_default() {
        "button" | "meter" | "output" | "progress" | "select" | "textarea" => true,
        "input" => dom
            .get_attribute_ns(node, None, "type")
            .is_none_or(|t| !t.eq_ignore_ascii_case("hidden")),
        _ => false,
    }
}

/// Native (non-ARIA) disabled state, including a disabled ancestor `fieldset`
/// (its first `<legend>` child is exempt).
pub fn is_disabled(dom: &Dom, node: NodeIdx) -> bool {
    if dom.get_attribute_ns(node, None, "disabled").is_some() {
        return true;
    }
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        if dom.name(parent) == Some("fieldset")
            && dom.get_attribute_ns(parent, None, "disabled").is_some()
            && !is_first_legend(dom, node, parent)
        {
            return true;
        }
        if matches!(dom.name(parent), Some("html" | "body")) {
            break;
        }
        cursor = dom.get(parent).parent;
    }
    false
}

fn is_first_legend(dom: &Dom, node: NodeIdx, fieldset: NodeIdx) -> bool {
    let first_legend = dom
        .children(fieldset)
        .find(|child| dom.name(*child) == Some("legend"));
    first_legend.is_some_and(|legend| legend == node || dom.contains(legend, Some(node)))
}

/// Native required state.
pub fn is_required(dom: &Dom, node: NodeIdx) -> bool {
    matches!(dom.name(node), Some("input" | "select" | "textarea"))
        && dom.get_attribute_ns(node, None, "required").is_some()
}

/// Native read-only state.
pub fn is_readonly(dom: &Dom, node: NodeIdx) -> bool {
    matches!(dom.name(node), Some("input" | "textarea"))
        && dom.get_attribute_ns(node, None, "readonly").is_some()
}

/// Native checked state (`checked` on a checkbox/radio input). Returns
/// `None` when the element is not a checkbox/radio.
pub fn native_checked(dom: &Dom, node: NodeIdx) -> Option<bool> {
    let name = dom.name(node)?;
    if name != "input" {
        return None;
    }
    let ty = dom
        .get_attribute_ns(node, None, "type")
        .unwrap_or("text")
        .to_ascii_lowercase();
    if ty == "checkbox" || ty == "radio" {
        Some(dom.get_attribute_ns(node, None, "checked").is_some())
    } else {
        None
    }
}

/// Native selected state (`selected` on an `<option>`).
pub fn native_selected(dom: &Dom, node: NodeIdx) -> Option<bool> {
    if dom.name(node) == Some("option") {
        Some(dom.get_attribute_ns(node, None, "selected").is_some())
    } else {
        None
    }
}

/// Native expanded state: `<details open>` and its `<summary>`.
pub fn native_expanded(dom: &Dom, node: NodeIdx) -> Option<bool> {
    match dom.name(node)? {
        "details" => Some(dom.get_attribute_ns(node, None, "open").is_some()),
        "summary" => {
            let parent = dom.get(node).parent?;
            if dom.name(parent) == Some("details") {
                Some(dom.get_attribute_ns(parent, None, "open").is_some())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Host-language text alternatives in accname order. `alt` on `img`/`area`,
/// `placeholder`/`title` on text fields, `<figcaption>`, `<caption>` and
/// `<legend>` are handled by the accname module; this returns the raw
/// attribute alternative for the element.
pub fn host_language_alt(dom: &Dom, node: NodeIdx) -> Option<String> {
    let tag = dom.name(node)?;
    match tag {
        "img" | "area" => dom
            .get_attribute_ns(node, None, "alt")
            .map(str::to_owned)
            .filter(|v| !v.is_empty()),
        "input" => match dom.get_attribute_ns(node, None, "type").unwrap_or("") {
            "image" => dom
                .get_attribute_ns(node, None, "alt")
                .map(str::to_owned)
                .filter(|v| !v.is_empty()),
            _ => None,
        },
        _ => None,
    }
}

/// `title` attribute when non-empty.
pub fn tooltip(dom: &Dom, node: NodeIdx) -> Option<String> {
    dom.get_attribute_ns(node, None, "title")
        .map(str::to_owned)
        .filter(|v| !v.trim().is_empty())
}

/// Concatenated child text for an element (raw, uncollapsed).
pub fn child_text(dom: &Dom, node: NodeIdx) -> String {
    let mut out = String::new();
    collect_text(dom, node, &mut out);
    out
}

fn collect_text(dom: &Dom, node: NodeIdx, out: &mut String) {
    for child in dom.children(node) {
        match &dom.get(child).kind {
            NodeKind::Text { data } | NodeKind::CDATASection { data } => out.push_str(data),
            NodeKind::Element { .. } => collect_text(dom, child, out),
            _ => {}
        }
    }
}

/// True when the element is in the SVG or MathML namespace.
pub fn is_foreign(dom: &Dom, node: NodeIdx) -> bool {
    !matches!(dom.namespace(node), Some(Namespace::Html))
}

/// Name of the first ancestor in the HTML namespace, used for table context.
pub fn ancestor_named(dom: &Dom, node: NodeIdx, name: &str) -> Option<NodeIdx> {
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        if dom.name(parent) == Some(name) {
            return Some(parent);
        }
        cursor = dom.get(parent).parent;
    }
    None
}

/// True when the element has an ancestor whose computed role is `grid`/`treegrid`
/// (needed to pick `gridcell` vs `cell` for `<td>`).
pub fn has_grid_ancestor(dom: &Dom, node: NodeIdx) -> bool {
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        if dom.get_attribute_ns(parent, None, "role").is_some_and(|r| {
            r.split_ascii_whitespace()
                .any(|t| t == "grid" || t == "treegrid")
        }) {
            return true;
        }
        cursor = dom.get(parent).parent;
    }
    false
}
