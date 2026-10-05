//! Used transform/perspective origins from computed CSS and actual reference geometry.
//! The caller chooses the reference box under CSS Transforms1 section5. Its offset
//! is local to the element, never a document-space or guessed fixture coordinate.
use crate::{ComputedStyle, syntax::{serialize, tokenize}, used_position_axis};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OriginReferenceBox {
    pub offset: [f64; 2],
    pub size: [f64; 2],
}
impl OriginReferenceBox {
    fn valid(self) -> bool {
        self.offset.iter().all(|n| n.is_finite())
            && self.size.iter().all(|n| n.is_finite() && *n >= 0.0)
    }
}

/// Resolve the whole computed origin against the caller's actual box. Existing
/// CSS computed values retain percentages; this is the later used-value handoff.
/// Unknown geometry or unresolved font/viewport units decline rather than guess.
pub fn used_origin(style: &ComputedStyle, property: &str, reference: OriginReferenceBox) -> Option<[f64; 3]> {
    if !matches!(property, "transform-origin" | "perspective-origin") || !reference.valid() {
        return None;
    }
    let css = crate::computed::position_values::canonical(property, style.get(property)?)?;
    let tokens = tokenize(&css);
    let parts = crate::computed::top_level_components(&tokens)?;
    let text = parts.iter().map(|part| serialize(part)).collect::<Vec<_>>();
    let [x, y, rest @ ..] = text.as_slice() else { return None };
    if rest.len() > 1 || property == "perspective-origin" && !rest.is_empty() {
        return None;
    }
    let z = if let Some(z) = rest.first() {
        used_position_axis(z, 0.0)?
    } else { 0.0 };
    let result = [
        reference.offset[0] + used_position_axis(x, reference.size[0])?,
        reference.offset[1] + used_position_axis(y, reference.size[1])?,
        z,
    ];
    result.iter().all(|n| n.is_finite()).then_some(result)
}

#[cfg(test)]
#[path = "transform_origins_tests.rs"]
mod tests;
