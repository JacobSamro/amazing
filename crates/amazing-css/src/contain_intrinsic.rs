//! CSS Sizing 4 §5.2: physical and logical intrinsic-size overrides.
use super::*;

pub(crate) fn is_longhand(property: &str) -> bool {
    matches!(property, "contain-intrinsic-width" | "contain-intrinsic-height"
        | "contain-intrinsic-inline-size" | "contain-intrinsic-block-size")
}

fn axis(parts: &[&[Token]], index: &mut usize) -> Option<String> {
    let start = *index;
    if matches!(parts.get(*index).map(|part| trim_tokens(part)),
        Some([Token::Ident(word)]) if word.eq_ignore_ascii_case("auto")) {
        *index += 1;
    }
    let part = trim_tokens(parts.get(*index)?);
    let text = serialize_computed(part);
    let valid = text.eq_ignore_ascii_case("none") || {
        match values::LengthPercentage::parse(&text) {
            Some(values::LengthPercentage::Percentage(_)) => false,
            Some(length) => length.is_nonnegative(),
            None => crate::math::length_percentage(&text, false),
        }
    };
    if !valid { return None }
    *index += 1;
    let text = if text.eq_ignore_ascii_case("none") { "none".to_owned() } else { text };
    Some(if *index - start == 2 { format!("auto {text}") } else { text })
}

pub(crate) fn valid(property: &str, value: &str) -> bool {
    if css_wide_keyword(value).is_some() { return true }
    if property == "contain-intrinsic-size" { return expand(value).is_some() }
    let tokens = tokenize(value);
    let Some(parts) = top_level_components(&tokens) else { return false };
    let mut index = 0;
    axis(&parts, &mut index).is_some() && index == parts.len()
}

pub(crate) fn expand(value: &str) -> Option<Vec<(String, String)>> {
    let names = ["contain-intrinsic-width", "contain-intrinsic-height"];
    if css_wide_keyword(value).is_some() {
        return Some(names.into_iter().map(|name| (name.to_owned(), value.trim().to_owned())).collect());
    }
    if contains_var_function(value) {
        return Some(names.into_iter().enumerate().map(|(index, name)| {
            (name.to_owned(), deferred(&format!("amazing-internal-contain-intrinsic-{index}"), value))
        }).collect());
    }
    let tokens = tokenize(value);
    let parts = top_level_components(&tokens)?;
    let mut index = 0;
    let first = axis(&parts, &mut index)?;
    let second = if index == parts.len() { first.clone() } else { axis(&parts, &mut index)? };
    if index != parts.len() { return None }
    Some(vec![(names[0].to_owned(), first), (names[1].to_owned(), second)])
}

pub(super) fn resolve_deferred(value: &str) -> Option<Result<String, ()>> {
    for index in 0..2 {
        if let Some(inner) = deferred_inner(value, &format!("amazing-internal-contain-intrinsic-{index}")) {
            return Some(expand(inner).and_then(|values| values.into_iter().nth(index).map(|(_, value)| value)).ok_or(()));
        }
    }
    None
}

pub(super) fn resolve_lengths(properties: &mut BTreeMap<String, String>, root: f64) {
    let font = properties.get("font-size").and_then(|value| value.strip_suffix("px"))
        .and_then(|value| value.parse().ok()).unwrap_or(16.0);
    for name in ["contain-intrinsic-width", "contain-intrinsic-height",
        "contain-intrinsic-inline-size", "contain-intrinsic-block-size"] {
        if let Some(value) = properties.get(name).cloned() {
            let (auto, length) = value.strip_prefix("auto ").map(|s| (true, s)).unwrap_or((false, &value));
            let px = if let Some(length) = values::LengthPercentage::parse(length) {
                let mut typed = ComputedValue::LengthPercentage(length);
                values::resolve_font_units(&mut typed, font, root, false);
                match typed { ComputedValue::LengthPercentage(values::LengthPercentage::Px(px)) => Some(px), _ => None }
            } else if crate::math::length_percentage(length, false) {
                crate::math::font_size_px(length, font, root)
            } else { None };
            if let Some(px) = px {
                let value = format!("{}{}px", if auto { "auto " } else { "" }, px.max(0.0));
                properties.insert(name.to_owned(), value);
            }
        }
    }
}

pub(crate) fn serialize_shorthand(style: &mut ComputedStyle) {
    if let (Some(width), Some(height)) = (style.get("contain-intrinsic-width"), style.get("contain-intrinsic-height")) {
        let value = if width == height { width.to_owned() } else { format!("{width} {height}") };
        style.props.insert("contain-intrinsic-size".to_owned(), value);
    }
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
    fn axis_values_preserve_grouping_and_reject_invalid_lengths() {
        for value in ["", "auto", "auto auto 1px", "10%", "-1px", "none 2px 3px", "1px auto", "calc(1px + 1%)"] {
            assert!(!valid("contain-intrinsic-size", value), "{value}");
        }
        let s = style("--size:auto 10px auto none; contain-intrinsic-size:var(--size)");
        assert_eq!(s.get("contain-intrinsic-width"), Some("auto 10px"));
        assert_eq!(s.get("contain-intrinsic-height"), Some("auto none"));
        assert_eq!(s.get("contain-intrinsic-size"), Some("auto 10px auto none"));
        let s = style("contain-intrinsic-size:10px; contain-intrinsic-height:var(--missing)");
        assert_eq!(s.get("contain-intrinsic-size"), Some("10px none"));
    }
    #[test]
    fn logical_intrinsic_sizes_share_physical_cascade_order_and_defaults() {
        let s = style("writing-mode:vertical-lr; contain-intrinsic-size:10px 20px; contain-intrinsic-inline-size:30px; contain-intrinsic-width:40px");
        assert_eq!(s.get("contain-intrinsic-width"), Some("40px"));
        assert_eq!(s.get("contain-intrinsic-height"), Some("30px"));
        assert_eq!(s.get("contain-intrinsic-inline-size"), Some("30px"));
        assert_eq!(s.get("contain-intrinsic-block-size"), Some("40px"));
        let s = style("font-size:20px; contain-intrinsic-size:auto 2em 1in; contain-intrinsic-height:initial");
        assert_eq!(s.get("contain-intrinsic-size"), Some("auto 40px none"));
    }
}
