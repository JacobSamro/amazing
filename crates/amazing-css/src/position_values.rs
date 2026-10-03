//! CSS Values4 position grammar; Transforms1 origins and reference boxes.
//! Generic positions reject the background-only three-component edge syntax.
use super::*;

const PROPERTIES: [&str; 4] = [
    "object-position",
    "transform-origin",
    "perspective-origin",
    "transform-box",
];
pub(crate) fn supported(property: &str) -> bool {
    PROPERTIES.contains(&property)
}
fn parts(value: &str) -> Option<Vec<String>> {
    let mut tokens = tokenize(value);
    let mut depth = 0usize;
    for token in &tokens {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    tokens.extend(std::iter::repeat_n(Token::CloseParen, depth));
    Some(
        top_level_components(&tokens)?
            .into_iter()
            .map(serialize_computed)
            .collect(),
    )
}
fn keyword(value: &str) -> Option<String> {
    let tokens = tokenize(value);
    let [Token::Ident(word)] = trim_tokens(&tokens) else {
        return None;
    };
    Some(word.to_ascii_lowercase())
}
fn axis_keyword(value: &str, x: bool) -> bool {
    keyword(value).is_some_and(|word| {
        if x {
            matches!(word.as_str(), "left" | "center" | "right")
        } else {
            matches!(word.as_str(), "top" | "center" | "bottom")
        }
    })
}
fn length(value: &str, percentage: bool) -> bool {
    let Some(validated) = individual_transforms::signs(value, None) else {
        return false;
    };
    values::LengthPercentage::parse(value).is_some_and(|length| {
        percentage || !matches!(length, values::LengthPercentage::Percentage(_))
    }) || crate::math::length_percentage(&validated, percentage)
}
fn position(parts: &[String]) -> Option<String> {
    let valid = match parts {
        [one] => axis_keyword(one, true) || axis_keyword(one, false) || length(one, true),
        [a, b] => {
            if keyword(a).is_some() && keyword(b).is_some() {
                (axis_keyword(a, true) && axis_keyword(b, false))
                    || (axis_keyword(a, false) && axis_keyword(b, true))
            } else {
                (axis_keyword(a, true) || length(a, true))
                    && (axis_keyword(b, false) || length(b, true))
            }
        }
        [a, b, c, d] => {
            let edge = |s: &str, x| {
                keyword(s).is_some_and(|w| {
                    if x {
                        matches!(w.as_str(), "left" | "right")
                    } else {
                        matches!(w.as_str(), "top" | "bottom")
                    }
                })
            };
            ((edge(a, true) && edge(c, false)) || (edge(a, false) && edge(c, true)))
                && length(b, true)
                && length(d, true)
        }
        _ => false,
    };
    if !valid {
        return None;
    }
    background_position_computed(&parts.join(" "))
}
pub(crate) fn canonical(property: &str, value: &str) -> Option<String> {
    if css_wide_keyword(value).is_some() {
        return Some(value.to_ascii_lowercase());
    }
    if property == "transform-box" {
        let word = keyword(value)?;
        return matches!(
            word.as_str(),
            "content-box" | "border-box" | "fill-box" | "stroke-box" | "view-box"
        )
        .then_some(word);
    }
    let parts = parts(value)?;
    if property == "transform-origin" {
        match parts.as_slice() {
            [_, _, z] if length(z, false) => {
                let xy = position(&parts[..2])?;
                Some(format!("{xy} {z}"))
            }
            [_, _, ..] if parts.len() > 2 => None,
            _ => position(&parts),
        }
    } else {
        position(&parts)
    }
}
pub(super) fn resolve(properties: &mut BTreeMap<String, String>, root: f64) {
    let font = properties
        .get("font-size")
        .and_then(|s| s.strip_suffix("px"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(16.0);
    for property in PROPERTIES.into_iter().filter(|p| *p != "transform-box") {
        let Some(value) = properties
            .get(property)
            .and_then(|v| canonical(property, v))
        else {
            continue;
        };
        let Some(parts) = parts(&value) else { continue };
        let mut resolved:Vec<String>=parts.into_iter().map(|part| {
            let tokens=tokenize(&part);
            if tokens.iter().any(|t|matches!(t,Token::Dimension(_,u) if !matches!(u.to_ascii_lowercase().as_str(),"px"|"in"|"cm"|"mm"|"q"|"pt"|"pc"|"em"|"rem"))) { return part; }
            let Some(value)=individual_transforms::signs(&part,Some((font,root))) else { return part };
            individual_transforms::length(&value,font,root)
        }).collect();
        if property == "transform-origin" && resolved.len() == 3 && resolved[2] == "0px" {
            resolved.pop();
        }
        properties.insert(property.into(), resolved.join(" "));
    }
}

/// A computed affine offset. Percentages use the caller's actual reference
/// dimension (object leftover space may be negative), never a guessed box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionAxis {
    pub fraction: f64,
    pub offset_px: f64,
}
impl PositionAxis {
    pub fn resolve(self, basis: f64) -> Option<f64> {
        let value = self.fraction * basis + self.offset_px;
        (basis.is_finite() && value.is_finite()).then_some(value)
    }
}
/// Extract a linear computed px/percentage expression without resolving its
/// percentage basis. Context units and nonlinear functions remain unsupported.
pub fn affine_position_axis(css: &str) -> Option<PositionAxis> {
    let tokens = tokenize(css);
    let significant: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    for (i, token) in significant.iter().enumerate() {
        match token {
            Token::Dimension(_, unit) if !unit.eq_ignore_ascii_case("px") => return None,
            Token::Function(name) if !name.eq_ignore_ascii_case("calc") => return None,
            Token::Delim('/') if !matches!(significant.get(i+1),Some(Token::Number(n)) if n.is_finite() && *n!=0.0) => {
                return None;
            }
            _ => {}
        }
    }
    let offset_px = crate::math::font_size_px(css, 0.0, 0.0)?;
    let fraction = crate::math::font_size_px(css, 100.0, 0.0)? / 100.0 - offset_px / 100.0;
    (offset_px.is_finite() && fraction.is_finite()).then_some(PositionAxis {
        fraction,
        offset_px,
    })
}
/// Normalize a generic physical position and extract its two affine axes.
pub fn affine_position_pair(css: &str) -> Option<(PositionAxis, PositionAxis)> {
    let canonical = canonical("object-position", css)?;
    let parts = parts(&canonical)?;
    let [x, y] = parts.as_slice() else {
        return None;
    };
    Some((affine_position_axis(x)?, affine_position_axis(y)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))])
            .get(div)
            .unwrap()
            .clone()
    }
    #[test]
    fn generic_and_three_dimensional_grammars_are_distinct() {
        for p in ["object-position", "perspective-origin"] {
            for value in [
                "bottom 10%",
                "10px left",
                "left right",
                "top bottom",
                "1px 2px 3px",
                "center left 1px",
                "left 4px top",
                "right top 5px",
                "right 1px bottom 2px,center",
                "auto",
            ] {
                assert!(!values::parses(p, value), "{p}:{value}");
            }
            for value in [
                "left",
                "top center",
                "center left",
                "10% bottom",
                "right 40%",
                "bottom 10% right 20%",
                "calc(2em + 3ex) 10px",
            ] {
                assert!(values::parses(p, value), "{p}:{value}");
            }
        }
        for value in [
            "1px 2px 3%",
            "1px 2px left",
            "1px 2px 3px 4px",
            "1px left",
            "top 1px",
            "right left",
            "bottom 10% right 20%",
        ] {
            assert!(!values::parses("transform-origin", value), "{value}");
        }
        for value in [
            "bottom right -7px",
            "-1px bottom 5px",
            "center left 6px",
            "calc(2em + 3ex)",
        ] {
            assert!(values::parses("transform-origin", value), "{value}");
        }
        for value in [
            "content-box",
            "border-box",
            "fill-box",
            "stroke-box",
            "view-box",
        ] {
            assert!(values::parses("transform-box", value));
        }
        for value in ["padding-box", "auto", "view-box fill-box"] {
            assert!(!values::parses("transform-box", value));
        }
    }
    #[test]
    fn computed_axes_preserve_geometry_and_resolve_actual_font_lengths() {
        let s = style(
            "font-size:40px;object-position:bottom 10% right 2em;transform-origin:calc(25% + .5em) bottom -1em;perspective-origin:top center;transform-box:content-box",
        );
        assert_eq!(s.get("object-position"), Some("calc(100% - 80px) 90%"));
        assert_eq!(
            s.get("transform-origin"),
            Some("calc(25% + 20px) 100% -40px")
        );
        assert_eq!(s.get("perspective-origin"), Some("50% 0%"));
        assert_eq!(s.get("transform-box"), Some("content-box"));
        let s = style("transform-origin:center 2em 0;perspective-origin:calc(2em + 3ex) 20%");
        assert_eq!(s.get("transform-origin"), Some("50% 32px"));
        assert_eq!(s.get("perspective-origin"), Some("calc(2em + 3ex) 20%"));
    }
    #[test]
    fn affine_used_axes_preserve_actual_negative_basis_and_reject_guesses() {
        let (x, y) = affine_position_pair("calc(100% - 10px) calc(25% + 3px)").unwrap();
        assert_eq!(x.resolve(-100.0), Some(-110.0));
        assert_eq!(y.resolve(200.0), Some(53.0));
        assert_eq!(
            affine_position_axis("calc((20% + 6px) / 2)")
                .unwrap()
                .resolve(100.0),
            Some(13.0)
        );
        for css in [
            "1em",
            "1ex",
            "1cqw",
            "min(10%,20px)",
            "calc(10px / (10% + 1px) * 1px)",
            "calc(1px / 0)",
        ] {
            assert!(affine_position_axis(css).is_none(), "{css}");
        }
        assert!(x.resolve(f64::NAN).is_none());
    }
    #[test]
    fn cascade_var_invalid_defaults_inheritance_and_invalidation() {
        let mut dom = amazing_dom::Dom::new();
        let parent = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let child = dom.create_element(parent, "span");
        let styles = crate::compute(
            &dom,
            &[crate::parse_stylesheet(
                "div{font-size:40px;object-position:left;transform-origin:1em 20%;transform-box:content-box}span{--bad:top 10px;transform-origin:var(--bad);object-position:inherit;transform-box:inherit;perspective-origin:bottom 10% right 2em}",
            )],
        );
        let s = styles.get(child).unwrap();
        assert_eq!(s.get("transform-origin"), Some("50% 50%"));
        assert_eq!(s.get("object-position"), Some("0% 50%"));
        assert_eq!(s.get("transform-box"), Some("content-box"));
        assert_eq!(s.get("perspective-origin"), Some("calc(100% - 80px) 90%"));
        let s = style(
            "transform-origin:left;transform-origin:top 10px;perspective-origin:right top 5px",
        );
        assert_eq!(s.get("transform-origin"), Some("0% 50%"));
        assert_eq!(s.get("perspective-origin"), Some("50% 50%"));
        for p in ["transform-origin", "perspective-origin", "transform-box"] {
            assert_eq!(
                crate::classify_style_property(p),
                Some(crate::StyleChangeKind::CompositorOnly)
            );
        }
    }
}
