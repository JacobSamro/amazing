//! Accessible name and description computation (accname-1.2) and the
//! HTML-AAM host-language text alternatives.
//!
//! The algorithm is the recursive `name from content` walk from the spec:
//!
//! 1. hidden nodes are skipped unless directly referenced,
//! 2. `aria-labelledby` then `aria-label`,
//! 3. HTML host-language alternatives (`label`, `alt`, `caption`, `legend`,
//!    `figcaption`, `title`, `placeholder`),
//! 4. name from content for roles that support it, or any directly referenced
//!    node,
//! 5. `title` as the last resort.
//!
//! Cycles are broken with a visited set; the result is whitespace-flattened.

use amazing_css::StyledDom;
use amazing_css::selectors::PseudoElement;
use amazing_dom::{Dom, NodeIdx, NodeKind};
use std::collections::HashMap;

use crate::host;
use crate::role::{self, Role};

/// Shared context for name computation: the arena, the computed styles (for
/// hidden state and CSS generated content), an id → node index, and the labels
/// targeting each id (`<label for=…>`).
pub struct NameContext<'a> {
    pub dom: &'a Dom,
    pub styled: &'a StyledDom,
    pub ids: &'a HashMap<String, NodeIdx>,
    pub labels_by_target: &'a HashMap<String, Vec<NodeIdx>>,
}

impl<'a> NameContext<'a> {
    pub fn new(
        dom: &'a Dom,
        styled: &'a StyledDom,
        ids: &'a HashMap<String, NodeIdx>,
        labels_by_target: &'a HashMap<String, Vec<NodeIdx>>,
    ) -> Self {
        Self { dom, styled, ids, labels_by_target }
    }
}

/// Accessible name of an element, per accname-1.2.
pub fn accessible_name(ctx: &NameContext<'_>, node: NodeIdx) -> String {
    let mut visited = Vec::new();
    compute_name(ctx, node, &mut visited, false)
}

/// Accessible description of an element, per accname-1.2 §4.3.
pub fn accessible_description(ctx: &NameContext<'_>, node: NodeIdx) -> String {
    let name = accessible_name(ctx, node);
    if let Some(ids) = ctx.dom.get_attribute_ns(node, None, "aria-describedby") {
        let mut parts = Vec::new();
        for id in ids.split_ascii_whitespace() {
            if let Some(target) = ctx.ids.get(id) {
                let mut branch = vec![node];
                let text = compute_name(ctx, *target, &mut branch, true);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
        if !parts.is_empty() {
            return flatten(&parts.join(" "));
        }
    }
    if let Some(description) = non_empty(ctx.dom.get_attribute_ns(node, None, "aria-description")) {
        return flatten(&description);
    }
    if let Some(title) = host::tooltip(ctx.dom, node) {
        let title = flatten(&title);
        if !title.is_empty() && title != name {
            return title;
        }
    }
    String::new()
}

fn compute_name(
    ctx: &NameContext<'_>,
    node: NodeIdx,
    visited: &mut Vec<NodeIdx>,
    referenced: bool,
) -> String {
    if visited.contains(&node) {
        return String::new();
    }
    match &ctx.dom.get(node).kind {
        NodeKind::Text { data } | NodeKind::CDATASection { data } => return flatten(data),
        NodeKind::Element { .. } => {}
        _ => return String::new(),
    }
    // Step 2A: hidden nodes produce no name unless directly referenced.
    if !referenced && is_hidden(ctx, node) {
        return String::new();
    }
    visited.push(node);

    // Step 2B: aria-labelledby (referenced nodes may be hidden).
    if let Some(ids) = ctx.dom.get_attribute_ns(node, None, "aria-labelledby") {
        let mut parts = Vec::new();
        for id in ids.split_ascii_whitespace() {
            if let Some(target) = ctx.ids.get(id)
                && *target != node
            {
                let mut branch = visited.clone();
                let text = compute_name(ctx, *target, &mut branch, true);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
        if !parts.is_empty() {
            visited.pop();
            return flatten(&parts.join(" "));
        }
    }

    // Step 2D: aria-label.
    if let Some(label) = non_empty(ctx.dom.get_attribute_ns(node, None, "aria-label")) {
        visited.pop();
        return flatten(&label);
    }

    // Step 2E: HTML host-language text alternative.
    if let Some(alt) = host_language_name(ctx, node) {
        visited.pop();
        return flatten(&alt);
    }

    // Steps 2F/2G: name from content, for roles that allow it or when the node
    // is directly referenced by aria-labelledby/aria-describedby.
    let element_role = role::role_for(ctx.dom, node);
    if referenced || element_role.name_from_content() {
        let mut parts = Vec::new();
        collect_content(ctx, node, visited, &mut parts);
        if !parts.is_empty() {
            visited.pop();
            return flatten(&parts.join(" "));
        }
    }

    // Step 2I: title, except for roles that prohibit naming.
    if !matches!(element_role, Role::Generic | Role::None | Role::Presentation)
        && let Some(title) = host::tooltip(ctx.dom, node)
    {
        visited.pop();
        return flatten(&title);
    }

    visited.pop();
    String::new()
}

/// HTML-AAM host language alternatives, in precedence order.
fn host_language_name(ctx: &NameContext<'_>, node: NodeIdx) -> Option<String> {
    let dom = ctx.dom;
    let tag = dom.name(node)?;
    match tag {
        "img" | "area" => match dom.get_attribute_ns(node, None, "alt") {
            // An `alt` attribute present (even empty) is the name; otherwise
            // fall back to `title`.
            Some(alt) => Some(alt.to_owned()),
            None => non_empty(dom.get_attribute_ns(node, None, "title")),
        },
        "input" => {
            let ty = dom
                .get_attribute_ns(node, None, "type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if ty == "image" {
                return non_empty(dom.get_attribute_ns(node, None, "alt"))
                    .or_else(|| non_empty(dom.get_attribute_ns(node, None, "title")));
            }
            if ty == "hidden" {
                return Option::None;
            }
            if matches!(ty.as_str(), "button" | "submit" | "reset") {
                if let Some(value) = non_empty(dom.get_attribute_ns(node, None, "value")) {
                    return Some(value);
                }
                return match ty.as_str() {
                    "submit" => Some("Submit".to_owned()),
                    "reset" => Some("Reset".to_owned()),
                    _ => Option::None,
                };
            }
            if let Some(label) = associated_label_text(ctx, node) {
                return Some(label);
            }
            if let Some(title) = non_empty(dom.get_attribute_ns(node, None, "title")) {
                return Some(title);
            }
            non_empty(dom.get_attribute_ns(node, None, "placeholder"))
        }
        "textarea" => {
            if let Some(label) = associated_label_text(ctx, node) {
                return Some(label);
            }
            if let Some(title) = non_empty(dom.get_attribute_ns(node, None, "title")) {
                return Some(title);
            }
            non_empty(dom.get_attribute_ns(node, None, "placeholder"))
        }
        "select" => {
            if let Some(label) = associated_label_text(ctx, node) {
                return Some(label);
            }
            non_empty(dom.get_attribute_ns(node, None, "title"))
        }
        "table" => child_text_by_tag(dom, node, "caption"),
        "fieldset" => child_text_by_tag(dom, node, "legend"),
        "figure" => child_text_by_tag(dom, node, "figcaption"),
        "svg" => descendant_text_by_tag(dom, node, "title"),
        "abbr" => non_empty(dom.get_attribute_ns(node, None, "title")),
        "progress" | "meter" | "output" => associated_label_text(ctx, node),
        _ => None,
    }
}

/// Appends the name-from-content pieces of `node`: its text, embedded-control
/// values, CSS `::before`/`::after` and descendants.
fn collect_content(
    ctx: &NameContext<'_>,
    node: NodeIdx,
    visited: &mut Vec<NodeIdx>,
    parts: &mut Vec<String>,
) {
    if let Some(before) = ctx.styled.generated_content(node, PseudoElement::Before) {
        push_generated(parts, before);
    }
    for child in ctx.dom.children(node) {
        match &ctx.dom.get(child).kind {
            NodeKind::Text { data } | NodeKind::CDATASection { data } => {
                let text = flatten(data);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            NodeKind::Element { .. } => {
                if !visited.contains(&child) && is_hidden(ctx, child) {
                    continue;
                }
                if let Some(own) = own_name(ctx, child, visited) {
                    if !own.is_empty() {
                        parts.push(own);
                    }
                    continue;
                }
                // Form controls are not traversed for text; their text is not
                // part of the container's name.
                if is_form_control(ctx.dom, child) {
                    continue;
                }
                let mut branch = visited.clone();
                branch.push(child);
                collect_content(ctx, child, &mut branch, parts);
            }
            _ => {}
        }
    }
    // A label with no other text takes the value of its embedded control.
    if parts.is_empty() && ctx.dom.name(node) == Some("label") {
        if let Some(value) = first_embedded_value(ctx.dom, node) {
            parts.push(value);
        }
    }
    if let Some(after) = ctx.styled.generated_content(node, PseudoElement::After) {
        push_generated(parts, after);
    }
}

/// The name an element contributes *without* exposing its subtree text: its
/// own `aria-label`/`aria-labelledby`, `<img alt>`, SVG title, and so on.
fn own_name(ctx: &NameContext<'_>, node: NodeIdx, visited: &mut Vec<NodeIdx>) -> Option<String> {
    if visited.contains(&node) {
        return None;
    }
    if ctx.dom.get_attribute_ns(node, None, "aria-labelledby").is_some()
        || non_empty(ctx.dom.get_attribute_ns(node, None, "aria-label")).is_some()
    {
        let mut branch = visited.clone();
        branch.push(node);
        return Some(compute_name(ctx, node, &mut branch, false));
    }
    let tag = ctx.dom.name(node)?;
    match tag {
        "img" | "area" | "abbr" => host_language_name(ctx, node),
        "input"
            if ctx
                .dom
                .get_attribute_ns(node, None, "type")
                .is_some_and(|t| t.eq_ignore_ascii_case("image")) =>
        {
            host_language_name(ctx, node)
        }
        _ => None,
    }
}

fn push_generated(parts: &mut Vec<String>, content: &amazing_css::generated::GeneratedContent) {
    let text = match &content.alt_text {
        Some(alt) => flatten(alt),
        None => flatten(&content.text()),
    };
    if !text.is_empty() {
        parts.push(text);
    }
}

fn is_form_control(dom: &Dom, node: NodeIdx) -> bool {
    matches!(dom.name(node), Some("input" | "select" | "textarea" | "button"))
}

fn first_embedded_value(dom: &Dom, node: NodeIdx) -> Option<String> {
    for id in dom.tree_order(node) {
        if id == node {
            continue;
        }
        if is_form_control(dom, id) && host::is_focusable(dom, id) {
            if let Some(value) = control_value(dom, id) {
                return Some(value);
            }
        }
    }
    None
}

fn control_value(dom: &Dom, node: NodeIdx) -> Option<String> {
    let role = role::role_for(dom, node);
    if matches!(role, Role::Checkbox | Role::Radio | Role::Switch) {
        return None;
    }
    non_empty(dom.get_attribute_ns(node, None, "aria-valuetext"))
        .or_else(|| non_empty(dom.get_attribute_ns(node, None, "value")))
}

/// Text of all `<label>` elements associated with a labelable control
/// (`for` attribute, or an ancestor `<label>`), concatenated.
fn associated_label_text(ctx: &NameContext<'_>, node: NodeIdx) -> Option<String> {
    if !host::is_labelable(ctx.dom, node) {
        return None;
    }
    let mut labels: Vec<NodeIdx> = Vec::new();
    if let Some(id) = ctx.dom.get_attribute_ns(node, None, "id")
        && let Some(found) = ctx.labels_by_target.get(id)
    {
        labels.extend(found.iter().copied());
    }
    let mut cursor = ctx.dom.get(node).parent;
    while let Some(parent) = cursor {
        if ctx.dom.name(parent) == Some("label") {
            labels.push(parent);
            break;
        }
        cursor = ctx.dom.get(parent).parent;
    }
    labels.sort_unstable();
    labels.dedup();

    let mut parts = Vec::new();
    for label in labels {
        let mut branch = Vec::new();
        collect_content(ctx, label, &mut branch, &mut parts);
    }
    if parts.is_empty() { None } else { Some(parts.join(" ")) }
}

fn child_text_by_tag(dom: &Dom, node: NodeIdx, tag: &str) -> Option<String> {
    dom.children(node)
        .find(|child| dom.name(*child) == Some(tag))
        .map(|child| host::child_text(dom, child))
        .filter(|text| !text.trim().is_empty())
}

fn descendant_text_by_tag(dom: &Dom, node: NodeIdx, tag: &str) -> Option<String> {
    for id in dom.tree_order(node) {
        if id != node && dom.name(id) == Some(tag) {
            let text = host::child_text(dom, id);
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// A node is hidden when it has `hidden`/`aria-hidden="true"`, or its computed
/// style is `display:none` / `visibility:hidden|collapse`.
pub fn is_hidden(ctx: &NameContext<'_>, node: NodeIdx) -> bool {
    let element = match ctx.dom.get(node).kind {
        NodeKind::Element { .. } => node,
        NodeKind::Text { .. } | NodeKind::CDATASection { .. } => match ctx.dom.get(node).parent {
            Some(parent) => parent,
            None => return false,
        },
        _ => return false,
    };
    if ctx.dom.get_attribute_ns(element, None, "hidden").is_some() {
        return true;
    }
    if ctx
        .dom
        .get_attribute_ns(element, None, "aria-hidden")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    {
        return true;
    }
    if let Some(style) = ctx.styled.get(element) {
        if style
            .get("display")
            .is_some_and(|v| v.eq_ignore_ascii_case("none"))
        {
            return true;
        }
        if style.get("visibility").is_some_and(|v| {
            v.eq_ignore_ascii_case("hidden") || v.eq_ignore_ascii_case("collapse")
        }) {
            return true;
        }
    }
    false
}

/// Per accname: collapse runs of whitespace and trim.
pub fn flatten(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.map(flatten).filter(|v| !v.is_empty())
}
