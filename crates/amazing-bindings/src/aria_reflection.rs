//! ARIAMixin reflection (DOM-B-REFLECT): `role` plus the `aria-*` string
//! attribute set defined by <https://w3c.github.io/aria/#ARIAMixin>. Every one
//! of these IDL attributes is a plain nullable-string (`DOMString?`)
//! reflection of the identically-named content attribute, and -- unlike
//! `element_reflection.rs`'s `LegacyReflectGet`/`Set`, which gate on a
//! specific tag or interface -- ARIAMixin applies to every `Element`
//! regardless of namespace or tag. That is different enough from the
//! existing tag-gated machinery to warrant its own small dispatch here
//! rather than bending that table's type check.
//!
//! Scope cut: the element-reference attributes (`ariaLabelledByElements` and
//! friends, `FrozenArray<Element>?`/`Element?`) are not implemented. Per
//! spec they need an explicitly-set-element override that the accessible
//! name/description computation (`amazing-a11y`, owned by the a11y lane)
//! would have to consult ahead of any ID-ref fallback -- an ID-ref-only
//! getter would silently diverge from that algorithm and from what
//! `Element.getAttribute("aria-labelledby")` already exposes. Routed to
//! a11y coordination rather than half-implemented.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, NodeKind};

pub(crate) fn handles(op: O) -> bool { matches!(op, O::AriaGet(_) | O::AriaSet(_)) }

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an Element"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    match op {
        O::AriaSet(attribute) => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if host.is_nullish(value) {
                host.with_dom(|dom| dom.remove_attribute(node, attribute))?;
            } else {
                let units = host.to_dom_string(value)?;
                host.with_dom(|dom| crate::html_reflection::set(dom, node, attribute, units))?;
            }
            Ok(host.undefined())
        }
        O::AriaGet(attribute) => {
            let value = host.with_dom(|dom| Ok(dom.attribute_utf16(node, attribute)))?;
            if let Some(spec) = ENUMERATED.iter().find(|spec| spec.attribute == attribute) {
                return match spec.resolve(value.as_deref()) {
                    Some(keyword) => { let units: Vec<u16> = keyword.encode_utf16().collect(); host.string(&units) }
                    None => Ok(host.null()),
                };
            }
            match value { Some(units) => host.string(&units), None => Ok(host.null()) }
        }
        _ => unreachable!("routed by handles()"),
    }
}

/// Enumerated ARIA attributes (w3c/aria#2484): the getter canonicalises to a keyword.
struct Enumerated {
    attribute: &'static str,
    keywords: &'static [&'static str],
    /// Result for the empty string when it differs from the invalid value default.
    empty: Option<Option<&'static str>>,
    invalid: Option<&'static str>,
}

impl Enumerated {
    fn resolve(&self, value: Option<&[u16]>) -> Option<&'static str> {
        let Some(value) = value else { return None };
        let text = String::from_utf16_lossy(value);
        if let Some(keyword) = self.keywords.iter().find(|keyword| keyword.eq_ignore_ascii_case(&text)) {
            if text.len() == keyword.len() && value.iter().all(|unit| *unit < 0x80) { return Some(keyword); }
        }
        if text.is_empty() { if let Some(result) = self.empty { return result; } }
        self.invalid
    }
}

const BOOL: &[&str] = &["true", "false"];
const TRISTATE: &[&str] = &["true", "false", "mixed"];
const fn e(attribute: &'static str, keywords: &'static [&'static str], empty: Option<Option<&'static str>>,
    invalid: Option<&'static str>) -> Enumerated {
    Enumerated { attribute, keywords, empty, invalid }
}
static ENUMERATED: [Enumerated; 20] = [
    e("aria-atomic", BOOL, Some(Some("false")), Some("false")),
    e("aria-autocomplete", &["inline", "list", "both", "none"], None, Some("none")),
    e("aria-busy", BOOL, Some(Some("false")), Some("false")),
    e("aria-checked", TRISTATE, Some(None), None),
    e("aria-current", &["page", "step", "location", "date", "time", "true", "false"], Some(Some("false")), Some("true")),
    e("aria-disabled", BOOL, Some(Some("false")), Some("false")),
    e("aria-expanded", BOOL, Some(None), None),
    e("aria-haspopup", &["true", "false", "menu", "dialog", "listbox", "tree", "grid"], None, Some("false")),
    e("aria-hidden", BOOL, Some(Some("false")), Some("false")),
    e("aria-invalid", &["true", "false", "spelling", "grammar"], Some(Some("false")), Some("true")),
    e("aria-live", &["polite", "assertive", "off"], None, Some("off")),
    e("aria-modal", BOOL, Some(Some("false")), Some("false")),
    e("aria-multiline", BOOL, Some(Some("false")), Some("false")),
    e("aria-multiselectable", BOOL, Some(Some("false")), Some("false")),
    e("aria-orientation", &["horizontal", "vertical"], Some(None), None),
    e("aria-pressed", TRISTATE, Some(None), None),
    e("aria-readonly", BOOL, Some(Some("false")), Some("false")),
    e("aria-required", BOOL, Some(Some("false")), Some("false")),
    e("aria-selected", BOOL, Some(None), None),
    e("aria-sort", &["ascending", "descending", "other", "none"], None, Some("none")),
];
