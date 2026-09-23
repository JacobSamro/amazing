//! Reusable HTML content-attribute reflection conversions.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{Dom, DomError, Namespace, NodeIdx, NodeKind};

pub(crate) fn handles(op: O) -> bool { matches!(op,
    O::ElementId | O::ElementSetId | O::HTMLTitle | O::HTMLSetTitle |
    O::HTMLLang | O::HTMLSetLang | O::HTMLDir | O::HTMLSetDir |
    O::HTMLHidden | O::HTMLSetHidden | O::HTMLTabIndex | O::HTMLSetTabIndex |
    O::HTMLAccessKey | O::HTMLSetAccessKey | O::HTMLTranslate | O::HTMLSetTranslate) }

fn ascii_lower(value: &[u16]) -> Vec<u16> {
    value.iter().map(|&unit| if (65..=90).contains(&unit) { unit + 32 } else { unit }).collect()
}
pub(crate) fn keyword(value: &[u16], candidates: &[&str]) -> Option<Vec<u16>> {
    let lower = ascii_lower(value);
    candidates.iter().find(|candidate| lower == candidate.encode_utf16().collect::<Vec<_>>())
        .map(|candidate| candidate.encode_utf16().collect())
}
/// HTML integer parsing consumes a leading digit run and ignores trailing
/// characters; the reflected long getter then rejects out-of-range results.
fn parsed_integer(value: &[u16]) -> Option<i64> {
    let mut input = value.iter().copied().skip_while(|c| matches!(*c, 9 | 10 | 12 | 13 | 32)).peekable();
    let negative = match input.peek() { Some(45) => { input.next(); true }, Some(43) => { input.next(); false }, _ => false };
    let mut number = 0i64;
    let mut any = false;
    for unit in input.take_while(|c| (48..=57).contains(c)) {
        any = true;
        number = number.checked_mul(10)?.checked_add(i64::from(unit - 48))?;
    }
    if !any { return None; }
    Some(if negative { -number } else { number })
}
pub(crate) fn signed_long(value: &[u16]) -> Option<i32> {
    i32::try_from(parsed_integer(value)?).ok()
}
/// Shared for future reflected unsigned-long attributes.
pub(crate) fn unsigned_long(value: &[u16]) -> Option<u32> {
    u32::try_from(parsed_integer(value)?).ok()
}
pub(crate) fn value(dom: &Dom, node: NodeIdx, name: &str) -> Vec<u16> {
    dom.attribute_utf16(node, name).unwrap_or_default()
}
pub(crate) fn set(dom: &mut Dom, node: NodeIdx, name: &str, units: Vec<u16>) -> Result<(), DomError> {
    if dom.get_attribute_ns(node, None, name).is_none() { dom.set_attribute(node, name, "")?; }
    let attr = dom.get_attribute_node(node, None, name).ok_or(DomError::InvalidStateError)?;
    dom.set_attr_value(attr, units)
}
fn default_tab_index(dom: &Dom, node: NodeIdx) -> i32 {
    let name = dom.name(node).unwrap_or("");
    match name {
        "a" | "area" | "button" | "frame" | "input" | "select" | "textarea" | "iframe" | "object" => 0,
        "summary" if dom.get(node).parent.is_some_and(|parent| dom.name(parent) == Some("details") &&
            dom.children(parent).filter(|child| dom.name(*child) == Some("summary")).next() == Some(node)) => 0,
        _ => -1,
    }
}
fn translation_enabled(dom: &Dom, mut node: NodeIdx) -> bool {
    loop {
        if let Some(mode) = (dom.namespace(node) == Some(Namespace::Html))
            .then(|| dom.attribute_utf16(node, "translate")).flatten() {
            let lower = ascii_lower(&mode);
            if lower == "no".encode_utf16().collect::<Vec<_>>() { return false; }
            if lower.is_empty() || lower == "yes".encode_utf16().collect::<Vec<_>>() { return true; }
        }
        match dom.get(node).parent { Some(parent) => node = parent, None => return true }
    }
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an Element"))?;
    let html_only = !matches!(op, O::ElementId | O::ElementSetId);
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { namespace: Namespace::Html, .. }) ||
        (!html_only && matches!(dom.get(node).kind, NodeKind::Element { .. })) { Ok(()) } else { Err(DomError::TypeError) })?;
    let name = match op {
        O::ElementId | O::ElementSetId => "id",
        O::HTMLTitle | O::HTMLSetTitle => "title",
        O::HTMLLang | O::HTMLSetLang => "lang",
        O::HTMLDir | O::HTMLSetDir => "dir",
        O::HTMLHidden | O::HTMLSetHidden => "hidden",
        O::HTMLTabIndex | O::HTMLSetTabIndex => "tabindex",
        O::HTMLAccessKey | O::HTMLSetAccessKey => "accesskey",
        O::HTMLTranslate | O::HTMLSetTranslate => "translate",
        _ => unreachable!(),
    };
    if matches!(op, O::ElementSetId | O::HTMLSetTitle | O::HTMLSetLang | O::HTMLSetDir | O::HTMLSetAccessKey) {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let units = host.to_dom_string(arg)?;
        host.with_dom(|dom| set(dom, node, name, units))?;
        return Ok(host.undefined());
    }
    if op == O::HTMLSetHidden {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let string = host.string_units(arg);
        let until_found = string.as_ref().is_some_and(|s| ascii_lower(s) == "until-found".encode_utf16().collect::<Vec<_>>());
        let hidden = until_found || host.to_boolean(arg);
        host.with_dom(|dom| if until_found { set(dom, node, name, "until-found".encode_utf16().collect()) }
            else if hidden { set(dom, node, name, Vec::new()) } else { dom.remove_attribute(node, name) })?;
        return Ok(host.undefined());
    }
    if op == O::HTMLSetTranslate {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let value = if host.to_boolean(arg) { "yes" } else { "no" };
        host.with_dom(|dom| set(dom, node, name, value.encode_utf16().collect()))?;
        return Ok(host.undefined());
    }
    if op == O::HTMLSetTabIndex {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let number = host.to_number(arg)?;
        let signed = if number.is_finite() { let modulo = number.trunc().rem_euclid(4_294_967_296.0) as u32; modulo as i32 } else { 0 };
        host.with_dom(|dom| set(dom, node, name, signed.to_string().encode_utf16().collect()))?;
        return Ok(host.undefined());
    }
    if op == O::HTMLTranslate {
        let enabled = host.with_dom(|dom| Ok(translation_enabled(dom, node)))?;
        return Ok(host.boolean(enabled));
    }
    if op == O::HTMLTabIndex {
        let index = host.with_dom(|dom| Ok(dom.attribute_utf16(node, "tabindex").and_then(|v| signed_long(&v))
            .unwrap_or_else(|| default_tab_index(dom, node))))?;
        return Ok(host.number(f64::from(index)));
    }
    if op == O::HTMLHidden {
        let state = host.with_dom(|dom| Ok(dom.attribute_utf16(node, "hidden")))?;
        if state.as_ref().is_some_and(|s| ascii_lower(s) == "until-found".encode_utf16().collect::<Vec<_>>()) {
            return host.string(&"until-found".encode_utf16().collect::<Vec<_>>());
        }
        return Ok(host.boolean(state.is_some()));
    }
    let mut units = host.with_dom(|dom| Ok(value(dom, node, name)))?;
    if op == O::HTMLDir { units = keyword(&units, &["ltr", "rtl", "auto"]).unwrap_or_default(); }
    host.string(&units)
}
