//! CSS Fonts synthetic-face controls: deferred shorthand and CSSOM summary.
use super::*;
const AXES: [&str; 4] = ["weight", "style", "small-caps", "position"];

pub(super) fn expand(value: &str) -> Option<Vec<(String, String)>> {
    if contains_var_function(value) {
        return Some(AXES.into_iter().enumerate().map(|(index, axis)| {
            (format!("font-synthesis-{axis}"), deferred(&format!("amazing-internal-font-synthesis-{index}"), value))
        }).collect());
    }
    crate::font_properties::expand_synthesis(value)
}

pub(super) fn resolve_deferred(value: &str) -> Option<Result<String, ()>> {
    for index in 0..AXES.len() {
        if let Some(inner) = deferred_inner(value, &format!("amazing-internal-font-synthesis-{index}")) {
            return Some(crate::font_properties::expand_synthesis(inner)
                .and_then(|values| values.into_iter().nth(index).map(|(_, value)| value)).ok_or(()));
        }
    }
    None
}

/// Reconstruct from winning inherited longhands, never the original shorthand.
/// `style` and `oblique-only` are mutually exclusive values of one axis.
pub(super) fn serialize(properties: &mut BTreeMap<String, String>) {
    let mut enabled = Vec::new();
    for axis in AXES {
        let Some(value) = properties.get(&format!("font-synthesis-{axis}")) else { return };
        match value.as_str() {
            "none" => {},
            "auto" => enabled.push(axis),
            "oblique-only" if axis == "style" => enabled.push("oblique-only"),
            _ => return,
        }
    }
    properties.insert("font-synthesis".to_owned(), if enabled.is_empty() { "none".to_owned() } else { enabled.join(" ") });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div {{ {css} }}"))]).get(div).unwrap().clone()
    }
    #[test]
    fn shorthand_axis_conflicts_and_canonical_order() {
        for value in ["", "auto", "none weight", "style oblique-only", "oblique-only style", "weight weight", "small-caps position position", "weight, style"] {
            assert!(crate::font_properties::expand_synthesis(value).is_none(), "{value}");
            assert!(!values::parses("font-synthesis", value), "{value}");
        }
        let mut s = style("font-synthesis:position small-caps oblique-only weight");
        assert_eq!(s.get("font-synthesis"), Some("weight oblique-only small-caps position"));
        assert_eq!(s.get("font-synthesis-style"), Some("oblique-only"));
        s.set("font-synthesis-style", "none");
        assert_eq!(s.get("font-synthesis"), Some("weight small-caps position"));
        assert_eq!(style("font-synthesis:none").get("font-synthesis"), Some("none"));
        assert_eq!(style("").get("font-synthesis"), Some("weight style small-caps position"));
    }
    #[test]
    fn variables_defaults_and_font_shorthand_independence() {
        let s = style("--faces:small-caps oblique-only; font-synthesis:var(--faces); font-synthesis-weight:auto; font:italic 20px serif");
        assert_eq!(s.get("font-synthesis"), Some("weight oblique-only small-caps"));
        let s = style("font-synthesis:none; font-synthesis:weight unknown");
        assert_eq!(s.get("font-synthesis"), Some("none"));
        let s = style("font-synthesis:none; --bad:style oblique-only; font-synthesis:var(--bad)");
        assert_eq!(s.get("font-synthesis"), Some("weight style small-caps position"));
        let s = style("font-synthesis:oblique-only; --bad:auto none; font-synthesis-style:var(--bad)");
        assert_eq!(s.get("font-synthesis"), Some("style"));
    }
    #[test]
    fn inherited_axes_and_css_wide_shorthand_keep_independent_winners() {
        let mut dom = amazing_dom::Dom::new();
        let outer = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let child = dom.create_element(outer, "span");
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div { font-synthesis:weight oblique-only } span { font-synthesis:inherit; font-synthesis-position:auto; font-synthesis-weight:none }")]);
        let s = styles.get(child).unwrap();
        assert_eq!(s.get("font-synthesis"), Some("oblique-only position"));
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div { font-synthesis:small-caps } span { font-synthesis:none; font-synthesis:var(--missing) }")]);
        assert_eq!(styles.get(child).unwrap().get("font-synthesis"), Some("small-caps"));
    }
}
