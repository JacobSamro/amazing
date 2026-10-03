//! Basic font selection grammar and context-free computed normalization.
use crate::syntax::{serialize, tokenize, Token};
pub(crate) fn supported(name: &str) -> bool {
    matches!(
        name,
        "font-weight" | "font-style" | "font-stretch" | "font-family"
    )
}
pub(crate) fn canonical(name: &str, css: &str) -> Option<String> {
    let css = css.trim();
    let lower = css.to_ascii_lowercase();
    match name {
        "font-family" => {
            // Values 5 random substitution is retained behind the existing
            // Unparsed seam; basic family validation must not discard it.
            if matches!(tokenize(css).first(),Some(Token::Function(name)) if name.eq_ignore_ascii_case("random-item"))
            {
                return Some(serialize(&tokenize(css)));
            }
            crate::font_shorthand::canonical_family(css)
        }
        "font-weight" => {
            if matches!(lower.as_str(), "normal" | "bold" | "bolder" | "lighter") {
                return Some(lower);
            }
            match tokenize(css).as_slice() {
                [Token::Number(n)] if n.is_finite() && (1.0..=1000.0).contains(n) => {
                    Some(n.to_string())
                }
                _ if matches!(tokenize(css).first(), Some(Token::Function(_)))
                    && crate::math::number_percentage(css, false) =>
                {
                    Some(serialize(&tokenize(css)))
                }
                _ => None,
            }
        }
        "font-style" => {
            if matches!(lower.as_str(), "normal" | "italic" | "oblique") {
                return Some(lower);
            }
            let t = tokenize(css);
            let Some(Token::Ident(word)) = t.first() else {
                return None;
            };
            if !word.eq_ignore_ascii_case("oblique") {
                return None;
            }
            let rest = serialize(&t[1..]);
            let rest = rest.trim();
            if let Some(a) = crate::values::Angle::parse(rest) {
                if a.radians.abs() > std::f64::consts::FRAC_PI_2 {
                    return None;
                }
            }
            if !crate::math::angle(rest) {
                return None;
            }
            Some(format!("oblique {rest}"))
        }
        "font-stretch" => {
            if matches!(
                lower.as_str(),
                "normal"
                    | "ultra-condensed"
                    | "extra-condensed"
                    | "condensed"
                    | "semi-condensed"
                    | "semi-expanded"
                    | "expanded"
                    | "extra-expanded"
                    | "ultra-expanded"
            ) {
                return Some(lower);
            }
            match tokenize(css).as_slice() {
                [Token::Percentage(n)] if n.is_finite() && *n >= 0.0 => Some(format!("{n}%")),
                _ if matches!(tokenize(css).first(), Some(Token::Function(_)))
                    && crate::math::percentage(css) =>
                {
                    Some(serialize(&tokenize(css)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
pub(crate) fn computed(name: &str, css: &str, parent: Option<&str>) -> Option<String> {
    let css = canonical(name, css)?;
    match name {
        "font-weight" => {
            let inherited = parent
                .and_then(|p| computed("font-weight", p, None))
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(400.0);
            let weight = match css.as_str() {
                "normal" => 400.0,
                "bold" => 700.0,
                "bolder" => {
                    if inherited < 350.0 {
                        400.0
                    } else if inherited < 550.0 {
                        700.0
                    } else {
                        900.0_f64.max(inherited)
                    }
                }
                "lighter" => {
                    if inherited < 550.0 {
                        100.0_f64.min(inherited)
                    } else if inherited < 750.0 {
                        400.0
                    } else {
                        700.0
                    }
                }
                _ => crate::math::computed_number(&css)?,
            };
            Some(weight.clamp(1.0, 1000.0).to_string())
        }
        "font-stretch" => {
            if let Some((_, p)) = crate::font_shorthand::WIDTHS
                .iter()
                .find(|(n, _)| *n == css)
            {
                return Some(p.to_string());
            }
            crate::math::computed_percentage(&css).map(|n| format!("{}%", n.max(0.0)))
        }
        "font-style" if css.starts_with("oblique ") => {
            let angle = crate::math::computed_angle(&css[8..])?
                .to_degrees()
                .clamp(-90.0, 90.0);
            if angle == 0.0 {
                return Some("normal".into());
            }
            let angle = (angle * 1e10).round() / 1e10;
            Some(format!("oblique {angle}deg"))
        }
        _ => Some(css),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(parent: &str, child: &str) -> crate::ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let body = dom.create_element(amazing_dom::NodeIdx(0), "body");
        let n = dom.create_element(body, "div");
        crate::compute(
            &dom,
            &[crate::parse_stylesheet(&format!(
                "body{{{parent}}}div{{{child}}}"
            ))],
        )
        .get(n)
        .unwrap()
        .clone()
    }
    #[test]
    fn selection_grammars_reject_whole_invalid_values_and_keep_math_context() {
        for (name, valid, invalid) in [
            (
                "font-weight",
                vec![
                    "normal",
                    "bold",
                    "bolder",
                    "1",
                    "550.5",
                    "1000",
                    "calc(-1)",
                    "clamp(900, 500, 100)",
                ],
                vec!["auto", "bold 900", "0", "1001", "10%", "calc(1px)"],
            ),
            (
                "font-style",
                vec![
                    "normal",
                    "italic",
                    "oblique",
                    "oblique 10grad",
                    "oblique calc(100deg)",
                ],
                vec![
                    "auto",
                    "italic oblique",
                    "oblique 91deg",
                    "oblique 0",
                    "oblique 1px",
                ],
            ),
            (
                "font-stretch",
                vec!["normal", "condensed", "0%", "105.5%", "calc(-1%)"],
                vec!["auto", "-1%", "105", "10px"],
            ),
            (
                "font-family",
                vec![
                    "Ahem, serif",
                    "\"New Century Schoolbook\", serif",
                    "\"serif\"",
                    "Menu",
                ],
                vec![
                    "cursive serif",
                    "Red/Black, sans-serif",
                    "\"Lucida\" Grande",
                    "Ahem!",
                    "#POUND",
                    "Hawaii 5-0",
                ],
            ),
        ] {
            for css in valid {
                assert!(canonical(name, css).is_some(), "{name}:{css}");
            }
            for css in invalid {
                assert!(canonical(name, css).is_none(), "{name}:{css}");
            }
        }
        assert_eq!(
            canonical("font-family", "\"New Century Schoolbook\", serif"),
            Some("New Century Schoolbook, serif".into())
        );
        assert!(canonical("font-family", "random-item(auto, serif, sans-serif)").is_some());
        assert!(canonical("font-weight", "calc(100 + (sign(20cqw - 10px) * 5))").is_some());
        assert_eq!(
            computed("font-weight", "calc(100 + (sign(20cqw - 10px) * 5))", None),
            None
        );
        assert!(canonical(
            "font-style",
            "oblique calc(30deg + (sign(2cqw - 10px) * 5deg))"
        )
        .is_some());
        assert_eq!(
            canonical("font-family", "\"serif\""),
            Some("\"serif\"".into())
        );
    }
    #[test]
    fn computed_relative_weights_and_angles_use_actual_parent_and_pure_math() {
        for (parent, bolder, lighter) in [
            (50.0, 400.0, 50.0),
            (349.5, 400.0, 100.0),
            (350.0, 700.0, 100.0),
            (549.5, 700.0, 100.0),
            (550.0, 900.0, 400.0),
            (749.5, 900.0, 400.0),
            (750.0, 900.0, 700.0),
            (950.0, 950.0, 700.0),
        ] {
            let p = parent.to_string();
            assert_eq!(
                computed("font-weight", "bolder", Some(&p)),
                Some(bolder.to_string())
            );
            assert_eq!(
                computed("font-weight", "lighter", Some(&p)),
                Some(lighter.to_string())
            );
        }
        for (css, result) in [
            ("normal", "400"),
            ("bold", "700"),
            ("calc(0)", "1"),
            ("calc(100 + 100)", "200"),
            ("clamp(900, 500, 100)", "900"),
        ] {
            assert_eq!(computed("font-weight", css, None), Some(result.into()));
        }
        for (css, result) in [
            ("oblique 0deg", "normal"),
            ("oblique 10grad", "oblique 9deg"),
            ("oblique calc(30deg * 2)", "oblique 60deg"),
            ("oblique calc(100deg)", "oblique 90deg"),
        ] {
            assert_eq!(computed("font-style", css, None), Some(result.into()));
        }
        let s = style("font-weight:800", "font: bolder 20px serif");
        assert_eq!(s.get("font-weight"), Some("900"));
        assert_eq!(
            s.cssom_property_value("font"),
            Some("900 20px serif".into())
        );
        let s = style(
            "font-weight:800",
            "font-weight:lighter;font-weight:bold 900",
        );
        assert_eq!(s.get("font-weight"), Some("700"));
    }
    #[test]
    fn scalar_math_and_used_position_keep_separate_dimension_types() {
        assert_eq!(
            crate::math::computed_number("max(10, calc(5 * 3))"),
            Some(15.0)
        );
        assert_eq!(crate::math::computed_number("calc(1px)"), None);
        assert_eq!(
            crate::used_position_axis("clamp(100px, 20px, 10px)", 50.0),
            Some(100.0)
        );
        assert!(
            (crate::used_position_axis("calc(10px + 25%)", 80.0).unwrap() - 30.0).abs() < 1e-10
        );
        assert_eq!(crate::math::computed_angle("calc(1deg + 1px)"), None);
    }
}
