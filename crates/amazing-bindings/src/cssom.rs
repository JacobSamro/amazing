//! Live, read-only computed declaration blocks, independent of a CSS engine.
//! CSSOM https://drafts.csswg.org/cssom/#the-cssstyledeclaration-interface
use crate::host::{BindingHost, PlatformKey};
use amazing_dom::DomError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComputedPseudo {
    Element, Before, After, Marker, FirstLine, FirstLetter, Placeholder,
    Selection, Backdrop, FileSelectorButton, Empty,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComputedStyleTarget {
    /// Actual arena generation of the element, independent of wrapper slots.
    pub element: PlatformKey,
    pub pseudo: ComputedPseudo,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssDeclaration {
    pub name: Vec<u16>,
    pub value: Vec<u16>,
}

/// Callbacks run after WebIDL conversion, without a borrowed native arena.
/// Enumeration contains sorted supported longhands followed by valid custom
/// properties. Values come from the host's current cascade/layout, never a
/// snapshot stored in the wrapper. Unsupported layout must not invent sizes.
pub trait CssomHost: BindingHost {
    fn cssom_parse_pseudo(&mut self, units: &[u16]) -> ComputedPseudo { parse_pseudo(units) }
    fn cssom_element_key(&self, value: &Self::Value) -> Option<PlatformKey>;
    fn cssom_wrap_computed(&mut self, target: ComputedStyleTarget) -> Result<Self::Value, Self::Error>;
    fn cssom_target(&self, value: &Self::Value) -> Option<ComputedStyleTarget>;
    fn cssom_declarations(&mut self, target: ComputedStyleTarget) -> Result<Vec<CssDeclaration>, Self::Error>;
    /// Also serializes supported shorthands, which are absent from enumeration.
    fn cssom_property_value(&mut self, target: ComputedStyleTarget, property: &[u16]) -> Result<Vec<u16>, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssomOp {
    CssText, SetCssText, Length, Item, GetPropertyValue, GetPropertyPriority,
    SetProperty, RemoveProperty, ParentRule, CssFloat, SetCssFloat,
}

pub fn parse_pseudo(units: &[u16]) -> ComputedPseudo {
    if units.first() != Some(&58) { return ComputedPseudo::Element; }
    let Ok(text) = String::from_utf16(units) else { return ComputedPseudo::Empty };
    // A non-colon argument does not request a pseudo-element. Do not trim
    // leading whitespace before testing the CSSOM starts-with-colon step.
    if !text.starts_with(':') { return ComputedPseudo::Element; }
    match text.to_ascii_lowercase().as_str() {
        ":before" | "::before" => ComputedPseudo::Before,
        ":after" | "::after" => ComputedPseudo::After,
        ":first-line" | "::first-line" => ComputedPseudo::FirstLine,
        ":first-letter" | "::first-letter" => ComputedPseudo::FirstLetter,
        "::marker" => ComputedPseudo::Marker,
        "::placeholder" => ComputedPseudo::Placeholder,
        "::selection" => ComputedPseudo::Selection,
        "::backdrop" => ComputedPseudo::Backdrop,
        "::file-selector-button" => ComputedPseudo::FileSelectorButton,
        _ => ComputedPseudo::Empty,
    }
}

pub fn get_computed_style<H: CssomHost>(host: &mut H, arguments: &[H::Value]) -> Result<H::Value, H::Error> {
    let element = arguments.first().and_then(|value| host.cssom_element_key(value))
        .ok_or_else(|| host.type_error("getComputedStyle requires an Element"))?;
    let pseudo = match arguments.get(1) {
        None => ComputedPseudo::Element,
        Some(value) if host.is_nullish(value) => ComputedPseudo::Element,
        Some(value) => {
            let units = host.to_dom_string(value)?;
            host.cssom_parse_pseudo(&units)
        }
    };
    host.cssom_wrap_computed(ComputedStyleTarget { element, pseudo })
}

pub fn property_name(units: &[u16]) -> Vec<u16> {
    if units.starts_with(&[45, 45]) { return units.to_vec(); }
    units.iter().map(|unit| if (65..=90).contains(unit) { unit + 32 } else { *unit }).collect()
}
fn argument<H: BindingHost>(host: &mut H, arguments: &[H::Value], index: usize) -> Result<Vec<u16>, H::Error> {
    let value = arguments.get(index).ok_or_else(|| host.type_error("Missing CSSStyleDeclaration argument"))?;
    host.to_dom_string(value)
}
fn legacy_null_string<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<Vec<u16>, H::Error> {
    if host.same_value(value, &host.null()) { Ok(Vec::new()) } else { host.to_dom_string(value) }
}
fn readonly<H: BindingHost>(host: &mut H) -> H::Error {
    crate::dom_exception::throw_dom_error(host, DomError::NoModificationAllowedError)
}
pub fn invoke<H: CssomHost>(host: &mut H, operation: CssomOp, receiver: H::Value,
    arguments: &[H::Value]) -> Result<H::Value, H::Error> {
    let target = host.cssom_target(&receiver)
        .ok_or_else(|| host.type_error("Expected a CSSStyleDeclaration"))?;
    match operation {
        CssomOp::CssText => host.string(&[]),
        CssomOp::ParentRule => Ok(host.null()),
        CssomOp::Length => {
            let count = host.cssom_declarations(target)?.len();
            Ok(host.number(count as f64))
        }
        CssomOp::Item => {
            let value = arguments.first().ok_or_else(|| host.type_error("Missing CSSStyleDeclaration index"))?;
            let number = host.to_number(value)?;
            let index = if number.is_finite() { number.trunc().rem_euclid(4294967296.0) as u32 } else { 0 };
            let declarations = host.cssom_declarations(target)?;
            host.string(declarations.get(index as usize).map_or(&[], |declaration| declaration.name.as_slice()))
        }
        CssomOp::GetPropertyValue => {
            let name = property_name(&argument(host, arguments, 0)?);
            let value = host.cssom_property_value(target, &name)?;
            host.string(&value)
        }
        CssomOp::GetPropertyPriority => {
            argument(host, arguments, 0)?;
            // A computed declaration never carries an author !important flag.
            host.string(&[])
        }
        CssomOp::CssFloat => {
            let value = host.cssom_property_value(target, &[102,108,111,97,116])?;
            host.string(&value)
        }
        CssomOp::SetCssText => {
            argument(host, arguments, 0)?;
            Err(readonly(host))
        }
        CssomOp::SetCssFloat => {
            let value = arguments.first().ok_or_else(|| host.type_error("Missing CSS property value"))?;
            legacy_null_string(host, value)?;
            Err(readonly(host))
        }
        CssomOp::SetProperty => {
            argument(host, arguments, 0)?;
            let value = arguments.get(1).ok_or_else(|| host.type_error("Missing CSS property value"))?;
            legacy_null_string(host, value)?;
            if let Some(priority) = arguments.get(2) { legacy_null_string(host, priority)?; }
            Err(readonly(host))
        }
        CssomOp::RemoveProperty => {
            argument(host, arguments, 0)?;
            Err(readonly(host))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pseudo_and_property_conversion_preserve_cssom_distinctions() {
        for (source, expected) in [("",ComputedPseudo::Element),("before",ComputedPseudo::Element),
            (" :before",ComputedPseudo::Element),(":BeFoRe",ComputedPseudo::Before),
            ("::after",ComputedPseudo::After),("::after ",ComputedPseudo::Empty),("::slotted(*)",ComputedPseudo::Empty),
            ("::part(a)",ComputedPseudo::Empty),(":marker",ComputedPseudo::Empty),
            ("::unknown",ComputedPseudo::Empty)] {
            assert_eq!(parse_pseudo(&source.encode_utf16().collect::<Vec<_>>()), expected);
        }
        assert_eq!(parse_pseudo(&[0xd800]), ComputedPseudo::Element);
        assert_eq!(parse_pseudo(&[58,0xd800]), ComputedPseudo::Empty);
        assert_eq!(property_name(&"COLOR".encode_utf16().collect::<Vec<_>>()), "color".encode_utf16().collect::<Vec<_>>());
        assert_eq!(property_name(&"--Case".encode_utf16().collect::<Vec<_>>()), "--Case".encode_utf16().collect::<Vec<_>>());
    }
}
