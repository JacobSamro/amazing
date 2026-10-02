//! CSS Logical Properties 1 §4.5: expand logical borders before ranking,
//! then let the cascade's existing writing-mode mapping select physical edges.
use super::*;

fn axis_component(property: &str) -> Option<(&str, &str)> {
    let rest = property.strip_prefix("border-")?;
    let (axis, component) = rest.split_once('-')?;
    (matches!(axis, "inline" | "block") && matches!(component, "width" | "style" | "color"))
        .then_some((axis, component))
}

fn is_edge(property: &str) -> bool {
    matches!(property, "border-inline-start" | "border-inline-end"
        | "border-block-start" | "border-block-end")
}

pub(crate) fn supported(property: &str) -> bool {
    is_edge(property) || matches!(property, "border-inline" | "border-block")
        || axis_component(property).is_some() || is_longhand(property)
}

fn is_longhand(property: &str) -> bool {
    ["border-inline-start-", "border-inline-end-", "border-block-start-", "border-block-end-"]
        .iter().any(|prefix| property.strip_prefix(prefix).is_some_and(|component| {
            matches!(component, "width" | "style" | "color")
        }))
}

pub(crate) fn expand(property: &str, value: &str) -> Option<Vec<(String, String)>> {
    let property = property.to_ascii_lowercase();
    if is_longhand(&property) {
        let valid = css_wide_keyword(value).is_some() || contains_var_function(value)
            || values::parse(&format!("border-top-{}", property.rsplit('-').next().unwrap()), value).is_some();
        return Some(if valid { vec![(property, value.to_owned())] } else { Vec::new() });
    }
    if is_edge(&property) {
        return Some(expand_side_border(&property, value).unwrap_or_default());
    }
    if matches!(property.as_str(), "border-inline" | "border-block") {
        let Some(start) = expand_side_border(&format!("{property}-start"), value) else {
            return Some(Vec::new());
        };
        let end = expand_side_border(&format!("{property}-end"), value).unwrap();
        return Some(start.into_iter().chain(end).collect());
    }
    let (axis, component) = axis_component(&property)?;
    let names = [format!("border-{axis}-start-{component}"), format!("border-{axis}-end-{component}")];
    if contains_var_function(value) {
        return Some(names.into_iter().enumerate().map(|(index, name)| {
            (name, deferred(&format!("amazing-internal-logical-border-{component}-{index}"), value))
        }).collect());
    }
    if css_wide_keyword(value).is_some() {
        return Some(names.into_iter().map(|name| (name, value.trim().to_owned())).collect());
    }
    let tokens = tokenize(value);
    let Some(parts) = top_level_components(&tokens) else { return Some(Vec::new()) };
    if !(1..=2).contains(&parts.len()) { return Some(Vec::new()) }
    let texts: Vec<String> = parts.into_iter().map(serialize_computed).collect();
    if !texts.iter().all(|text| values::parse(&format!("border-top-{component}"), text).is_some()) {
        return Some(Vec::new());
    }
    Some(names.into_iter().enumerate().map(|(index, name)| {
        (name, texts[index.min(texts.len() - 1)].clone())
    }).collect())
}

pub(super) fn resolve_deferred(property: &str, value: &str) -> Option<Result<String, ()>> {
    for component in ["width", "style", "color"] {
        for index in 0..2 {
            let function = format!("amazing-internal-logical-border-{component}-{index}");
            if let Some(inner) = deferred_inner(value, &function) {
                if css_wide_keyword(inner).is_some() { return Some(Ok(inner.trim().to_owned())) }
                let physical = format!("border-top-{component}");
                let tokens = tokenize(inner);
                let Some(parts) = top_level_components(&tokens) else { return Some(Err(())) };
                if !(1..=2).contains(&parts.len()) { return Some(Err(())) }
                let texts: Vec<_> = parts.into_iter().map(serialize_computed).collect();
                if !texts.iter().all(|text| values::parse(&physical, text).is_some()) { return Some(Err(())) }
                if !property.ends_with(&format!("-{component}")) { return Some(Err(())) }
                return Some(Ok(texts[index.min(texts.len() - 1)].clone()));
            }
        }
    }
    None
}

pub(crate) fn valid(property: &str, value: &str) -> Option<bool> {
    expand(property, value).map(|expanded| !expanded.is_empty())
}

/// The API exposes computed shorthand text as well as the winning longhands.
/// Reconstruct it after physical/logical cascade competition, never retain a
/// separately-ranked specified shorthand that can become stale.
pub(crate) fn serialize_shorthands(style: &mut ComputedStyle) {
    for axis in ["inline", "block"] {
        for component in ["width", "style", "color"] {
            let start = format!("border-{axis}-start-{component}");
            let end = format!("border-{axis}-end-{component}");
            if let (Some(first), Some(second)) = (style.get(&start), style.get(&end)) {
                let value = if first == second { first.to_owned() } else { format!("{first} {second}") };
                style.props.insert(format!("border-{axis}-{component}"), value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compute, parse_stylesheet};
    use amazing_dom::Dom;

    fn style(css: &str) -> ComputedStyle {
        let mut dom = Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        compute(&dom, &[parse_stylesheet(&format!("div {{ {css} }}"))]).get(div).unwrap().clone()
    }

    #[test]
    fn logical_borders_respect_axes_ranking_and_substitution() {
        for (mode, inline, block) in [
            ("horizontal-tb", ["left", "right"], ["top", "bottom"]),
            ("vertical-rl", ["top", "bottom"], ["right", "left"]),
            ("vertical-lr", ["top", "bottom"], ["left", "right"]),
        ] {
            let s = style(&format!("writing-mode:{mode}; border-inline:10px solid green; border-block:3px dashed red"));
            for side in inline { assert_eq!(s.get(&format!("border-{side}-width")), Some("10px")); }
            for side in block { assert_eq!(s.get(&format!("border-{side}-width")), Some("3px")); }
        }
        let s = style("direction:rtl; --pair:2px 7px; border-inline:10px solid green; border-inline-width:var(--pair); border-left-width:9px");
        assert_eq!(s.get("border-right-width"), Some("2px"));
        assert_eq!(s.get("border-left-width"), Some("9px"));
        assert_eq!(s.get("border-inline-width"), Some("2px 9px"));
        let s = style("border-left:5px solid red; border-inline:var(--missing)");
        assert_eq!(s.get("border-left-width"), Some("medium"));
        let s = style("--edge:4px dashed rgb(0, 128, 0); border-block-start:var(--edge); border-block-width:initial");
        assert_eq!(s.get("border-top-width"), Some("medium"));
        assert_eq!(s.get("border-top-style"), Some("dashed"));
    }

    #[test]
    fn complete_grammars_reject_atomically_and_keep_functions_whole() {
        for axis in ["inline", "block"] {
            for value in ["", "1px 2px", "solid dashed", "red blue", "-1px solid red", "solid nonsense"] {
                assert!(!valid(&format!("border-{axis}"), value).unwrap(), "{value}");
            }
            for component in ["width", "style", "color"] {
                assert!(!valid(&format!("border-{axis}-{component}"), "a b c").unwrap());
            }
            assert!(valid(&format!("border-{axis}-color"), "rgb(0, 128, 0) blue").unwrap());
            assert!(valid(&format!("border-{axis}"), "rgb(0, 128, 0) solid 1px").unwrap());
            assert_eq!(expand(&format!("border-{axis}"), "inherit").unwrap().len(), 6);
        }
        for edge in ["inline-start", "inline-end", "block-start", "block-end"] {
            assert_eq!(expand(&format!("border-{edge}"), "solid").unwrap().len(), 3);
        }
    }
}
