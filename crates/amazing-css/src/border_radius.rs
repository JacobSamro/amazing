//! CSS Backgrounds3 §4.1: physical corner grammar, computed radii and CSSOM.
//! Percentages retain their independent border-box axes; this module does not
//! perform the layout overlap reduction or invent unavailable font metrics.
use super::*;

pub(super) const CORNERS: [&str; 4] = [
    "border-top-left-radius", "border-top-right-radius",
    "border-bottom-right-radius", "border-bottom-left-radius",
];

pub(crate) fn supported(property: &str) -> bool {
    property == "border-radius" || CORNERS.contains(&property)
}

pub(crate) fn valid(property: &str, value: &str) -> bool {
    if css_wide_keyword(value).is_some() { return true; }
    if contains_var_function(value) { return top_level_components(&tokenize(value)).is_some(); }
    if property == "border-radius" { expand_border_radius(value).is_some() }
    else { valid_radius_pair(value) }
}

pub(super) fn component(tokens: &[Token]) -> Option<String> {
    let css = serialize_computed(trim_tokens(tokens));
    match values::LengthPercentage::parse(&css) {
        Some(length) if length.is_nonnegative() => Some(css),
        Some(_) => None,
        None => crate::math::length_percentage(&css, true).then_some(css),
    }
}

fn computed_component(tokens: &[Token], font: Option<f64>, root: f64) -> Option<String> {
    let css = component(tokens)?;
    use values::{LengthPercentage as L, RelativeUnit as R};
    if let Some(length) = L::parse(&css) {
        return Some(match length {
            L::Px(px) => format!("{px}px"),
            L::Percentage(p) => format!("{p}%"),
            L::Relative(n, R::Em) if font.is_some() => format!("{}px", n * font?),
            L::Relative(n, R::Rem) => format!("{}px", n * root),
            _ => css,
        });
    }
    // Resolve only units whose genuine computed context is already available.
    // Replacing dimensions preserves percentages and their later box basis.
    let resolved: Vec<_> = tokenize(&css).into_iter().map(|token| match token {
        Token::Dimension(n, unit) => match L::parse(&format!("{n}{unit}")) {
            Some(L::Px(px)) => Token::Dimension(px, "px".into()),
            Some(L::Relative(n, R::Em)) if font.is_some() => Token::Dimension(n * font.unwrap(), "px".into()),
            Some(L::Relative(n, R::Rem)) => Token::Dimension(n * root, "px".into()),
            _ => Token::Dimension(n, unit),
        },
        token => token,
    }).collect();
    let css = serialize_computed(&resolved);
    if crate::math::percentage(&css) {
        if let Some(p) = crate::math::computed_percentage(&css) { return Some(format!("{}%", p.max(0.0))); }
    } else if !resolved.iter().any(|t| matches!(t, Token::Percentage(_))) {
        // NaN prevents an absent context from being mistaken for a real metric.
        if let Some(px) = crate::math::font_size_px(&css, font.unwrap_or(f64::NAN), root) {
            return Some(format!("{}px", px.max(0.0)));
        }
    }
    Some(css)
}

pub(super) fn computed_pair(value: &str, font: Option<f64>, root: f64) -> Option<String> {
    let tokens = tokenize(value);
    let parts = top_level_components(&tokens)?;
    if !(1..=2).contains(&parts.len()) { return None; }
    let x = computed_component(parts[0], font, root)?;
    let y = if parts.len() == 1 { x.clone() } else { computed_component(parts[1], font, root)? };
    Some(if x == y { x } else { format!("{x} {y}") })
}

pub(super) fn resolve(props: &mut BTreeMap<String, String>, root: f64) {
    let font = props.get("font-size").and_then(|s| s.strip_suffix("px")).and_then(|s| s.parse().ok());
    for name in CORNERS {
        if let Some(value) = props.get(name).and_then(|css| computed_pair(css, font, root)) {
            props.insert(name.into(), value);
        }
    }
}

pub(super) fn shorthand(style: &ComputedStyle) -> Option<String> {
    let mut x = Vec::new();
    let mut y = Vec::new();
    for name in CORNERS {
        let tokens = tokenize(style.get(name)?);
        let parts = top_level_components(&tokens)?;
        if !(1..=2).contains(&parts.len()) { return None; }
        x.push(component(parts[0])?);
        y.push(component(parts.get(1).copied().unwrap_or(parts[0]))?);
    }
    let same = x == y;
    let compress = |mut values: Vec<String>| {
        if values[3] == values[1] { values.pop(); }
        if values.len() == 3 && values[2] == values[0] { values.pop(); }
        if values.len() == 2 && values[1] == values[0] { values.pop(); }
        values.join(" ")
    };
    let horizontal = compress(x);
    Some(if same { horizontal } else { format!("{horizontal} / {}", compress(y)) })
}

#[cfg(test)]
#[path = "border_radius_tests.rs"]
mod tests;
