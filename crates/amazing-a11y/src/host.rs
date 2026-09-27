//! HTML host-language facts used by role, state and name computation:
//! focusability, labelability, native control state, and the host-language
//! text alternatives (`alt`, `placeholder`, `title`, associated `<label>`).
//!
//! A11Y-15: the native-state readers below (`native_checked`,
//! `native_indeterminate`, `native_selected`, `native_range_value`) read
//! `amazing-dom`'s *live* form-control state (`Dom::input_checked`,
//! `input_indeterminate`, `option_selected`, `input_value_as_number`, ...),
//! not the parse-time content attribute — `checked`/`selected`/an input's
//! `value` each have a "dirty" IDL state distinct from their content
//! attribute per spec, which a real click/selection/edit updates without
//! ever touching the attribute. `is_disabled`/`is_required`/`is_readonly`/
//! `native_expanded` were audited too and confirmed to have no such
//! divergence (see their own doc comments) — no fix needed for those.

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
///
/// A11Y-15: `disabled` has no separate IDL/dirty-value state distinct from
/// the content attribute (unlike `checked`/`selected`/`value` below) — per
/// spec it is a plain reflected boolean, so reading the attribute here was
/// never actually stale. Delegates to `Dom::form_control_disabled` (the
/// engine's own canonical live check, used by the real click-activation
/// path) rather than a second, hand-rolled copy of the same rule, so this
/// can't drift from whatever click/activation actually enforces. Unlike
/// `form_control_disabled`, this is asked about *any* element (not just the
/// tags disabled is valid on), so it falls back to a plain own-attribute
/// check for anything `form_control_disabled` always reports `false` for by
/// construction (e.g. a non-form-control element carrying a stray
/// `disabled` attribute) — matches this function's pre-existing, broader
/// contract.
pub fn is_disabled(dom: &Dom, node: NodeIdx) -> bool {
    if dom.form_control_disabled(node) {
        return true;
    }
    dom.get_attribute_ns(node, None, "disabled").is_some()
}

/// Native required state. A11Y-15: audited — like `disabled`, `required` has
/// no separate live/dirty state, only the reflected content attribute, so
/// this was already a live read.
pub fn is_required(dom: &Dom, node: NodeIdx) -> bool {
    matches!(dom.name(node), Some("input" | "select" | "textarea"))
        && dom.get_attribute_ns(node, None, "required").is_some()
}

/// Native read-only state. A11Y-15: audited — same as `required`, `readonly`
/// is a plain reflected attribute with no separate live state.
pub fn is_readonly(dom: &Dom, node: NodeIdx) -> bool {
    matches!(dom.name(node), Some("input" | "textarea"))
        && dom.get_attribute_ns(node, None, "readonly").is_some()
}

/// Native checked state (checkbox/radio). Returns `None` when the element
/// is not a checkbox/radio.
///
/// A11Y-15: reads `Dom::input_checked` — the live IDL checkedness (current
/// checkedness, `Dom::FormState::checked` once the dirty-checkedness flag is
/// set by a real click/activation) — instead of the `checked` **content**
/// attribute, which per spec only ever reflects *default* checkedness from
/// parse time and is never touched by a real click. Before this fix, a
/// click that toggled a checkbox produced no `StateChanged` accessibility
/// event at all (filed by integrator-assist, IA-REAL-3,
/// `.agents/integrator-assist/questions.md`).
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
        dom.input_checked(node).ok()
    } else {
        None
    }
}

/// Native indeterminate state (checkbox only — `.indeterminate` is an
/// IDL-only property with no content-attribute form at all, so there is
/// nothing for a static-markup read to have gotten wrong; it simply didn't
/// exist as a signal here before A11Y-15). HTML-AAM maps a checkbox's
/// `indeterminate` to `aria-checked="mixed"` when no explicit `aria-checked`
/// overrides it.
pub fn native_indeterminate(dom: &Dom, node: NodeIdx) -> bool {
    if dom.name(node) != Some("input") {
        return false;
    }
    let ty = dom
        .get_attribute_ns(node, None, "type")
        .unwrap_or("text")
        .to_ascii_lowercase();
    ty == "checkbox" && dom.input_indeterminate(node).unwrap_or(false)
}

/// Native selected state (`<option>`).
///
/// A11Y-15: reads `Dom::option_selected` — the live selectedness (which also
/// resolves a single `<select>`'s implicit "first non-disabled option"
/// default when nothing is explicitly selected) — instead of the `selected`
/// **content** attribute, which per spec only reflects *default*
/// selectedness and is never touched by a real selection change (e.g.
/// picking a different option in a dropdown, or `select.value = ...`).
pub fn native_selected(dom: &Dom, node: NodeIdx) -> Option<bool> {
    if dom.name(node) == Some("option") {
        dom.option_selected(node).ok()
    } else {
        None
    }
}

/// Native numeric value/min/max for a range-like control (`<input
/// type=range>`, `<progress>`, `<meter>`), the fallback for ARIA
/// `valuenow`/`valuemin`/`valuemax` when no explicit `aria-value*` override
/// is present (HTML-AAM).
///
/// A11Y-15: `<input type=range>`'s current value has the same dirty-value-
/// flag semantics as any other input value — a live IDL/form-state read via
/// `Dom::input_value_as_number`, not the `value` content attribute, which
/// stays at its parse-time default once the user drags the slider (or
/// `stepUp()`/`stepDown()`/`.valueAsNumber = ...` runs). `<progress>`/
/// `<meter>` have no such distinction — their `value`/`min`/`max` attributes
/// *are* the live value (setting `.value` reflects straight back to the
/// attribute per spec) — so those stay plain attribute reads; defaults per
/// spec (range: min 0/max 100/value the midpoint; progress: min 0, max 1,
/// no native value when the `value` attribute is absent at all — an
/// indeterminate progress bar, correctly leaving `value_now` `None` rather
/// than fabricating one; meter: min 0/max 1/value clamped into range).
pub fn native_range_value(dom: &Dom, node: NodeIdx) -> Option<(f64, f64, f64)> {
    fn numeric_attr(dom: &Dom, node: NodeIdx, name: &str) -> Option<f64> {
        dom.get_attribute_ns(node, None, name)?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
    }
    match dom.name(node)? {
        "input" => {
            let ty = dom
                .get_attribute_ns(node, None, "type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if ty != "range" {
                return None;
            }
            let min = numeric_attr(dom, node, "min").unwrap_or(0.0);
            let max = numeric_attr(dom, node, "max").unwrap_or(100.0).max(min);
            let value = dom
                .input_value_as_number(node)
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or((min + max) / 2.0);
            Some((value, min, max))
        }
        "progress" => {
            let max = numeric_attr(dom, node, "max").filter(|v| *v > 0.0).unwrap_or(1.0);
            let value = numeric_attr(dom, node, "value")?;
            Some((value.clamp(0.0, max), 0.0, max))
        }
        "meter" => {
            let min = numeric_attr(dom, node, "min").unwrap_or(0.0);
            let max = numeric_attr(dom, node, "max").unwrap_or(1.0).max(min);
            let value = numeric_attr(dom, node, "value").unwrap_or(min).clamp(min, max);
            Some((value, min, max))
        }
        _ => None,
    }
}

/// Native expanded state: `<details open>` and its `<summary>`.
///
/// A11Y-15: audited — `open` has no separate IDL/dirty-value state either.
/// `amazing-bindings` has no `.open` reflection installed yet
/// (`interfaces.rs` has zero hits for it), and `Dom::prepare_input_
/// activation` (the real click-activation path) only ever returns `Some`
/// for `<input>`, so no click-driven `<summary>` toggle exists in this
/// engine yet — nothing currently sets `open` other than the content
/// attribute itself, so this read was never stale. Filed as a future gap
/// (a `<summary>` click-activation implementation), not this ticket's live-
/// state fix, since there is no live state distinct from the attribute to
/// switch to yet.
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
