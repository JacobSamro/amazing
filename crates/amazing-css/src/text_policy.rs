//! Deferred CSS Text shorthands, with separate winning axes after substitution.
use super::*;
pub(super) fn expand(property: &str, value: &str) -> Option<Vec<(String, String)>> {
    let axes = crate::text_properties::axes(property)?;
    if contains_var_function(value) {
        return Some(
            axes.iter()
                .map(|axis| {
                    (
                        (*axis).into(),
                        deferred(&format!("amazing-internal-text-{property}"), value),
                    )
                })
                .collect(),
        );
    }
    crate::text_properties::expand_static(property, value)
}
pub(super) fn resolve_deferred(property: &str, value: &str) -> Option<Result<String, ()>> {
    for shorthand in ["white-space", "text-wrap", "text-spacing"] {
        if let Some(inner) = deferred_inner(value, &format!("amazing-internal-text-{shorthand}")) {
            return Some(
                crate::text_properties::expand_static(shorthand, inner)
                    .and_then(|values| {
                        values
                            .into_iter()
                            .find_map(|(name, value)| (name == property).then_some(value))
                    })
                    .ok_or(()),
            );
        }
    }
    None
}
