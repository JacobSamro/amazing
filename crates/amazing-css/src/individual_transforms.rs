//! CSS Transforms 2 §5: individual transform grammars and computed values.
//! These properties preserve `none` separately from identity transforms.
use super::*;

fn parts(value: &str) -> Option<Vec<String>> {
    let mut tokens = tokenize(value);
    // CSS Syntax consumes an unfinished function through EOF and supplies its
    // closing delimiter. Keep this recovery distinct from stray close tokens.
    let mut depth = 0usize;
    for token in &tokens {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    tokens.extend(std::iter::repeat_n(Token::CloseParen, depth));
    Some(top_level_components(&tokens)?.into_iter().map(serialize_computed).collect())
}

fn numeric(value: &str, percentage: bool) -> Option<f64> {
    if !crate::math::number_percentage(value, percentage) { return None }
    values::alpha_value(value).filter(|value| value.is_finite())
}

/// `sign()` has no percentage basis here. Validate its dimension before
/// replacing it with a number; at computation resolve font lengths first.
pub(crate) fn signs(value: &str, font: Option<(f64, f64)>) -> Option<String> {
    let mut tokens = tokenize(value);
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], Token::Function(name) if name.eq_ignore_ascii_case("sign")) {
            if !matches!(tokens.get(i + 1), Some(Token::OpenParen)) { return None }
            let mut depth = 1;
            let mut end = i + 2;
            while end < tokens.len() {
                match tokens[end] { Token::OpenParen => depth += 1, Token::CloseParen => depth -= 1, _ => {} }
                if depth == 0 { break }
                end += 1;
            }
            if depth != 0 { return None }
            let inner = format!("calc({})", serialize_computed(&tokens[i + 2..end]));
            let mut inner = signs(&inner, font)?;
            if font.is_none() {
                let tokens = tokenize(&inner).into_iter().map(|token| match token {
                    Token::Dimension(n, unit) if matches!(unit.to_ascii_lowercase().as_str(),
                        "vi" | "vb" | "svi" | "svb" | "lvi" | "lvb" | "dvi" | "dvb"
                        | "cqw" | "cqh" | "cqi" | "cqb" | "cqmin" | "cqmax") => Token::Dimension(n, "px".to_owned()),
                    token => token,
                }).collect::<Vec<_>>();
                inner = serialize_computed(&tokens);
            }
            let number = if let Some(number) = numeric(&inner, false) { number }
                else if crate::math::length_percentage(&inner, false) {
                    // Validation establishes the dimension without assuming a
                    // font metric; computed evaluation needs its actual value.
                    match font {
                        Some((font, root)) => crate::math::font_size_px(&inner, font, root)?,
                        None => 1.0,
                    }
                } else { return None };
            let sign = if number == 0.0 { 0.0 } else { number.signum() };
            tokens.splice(i..=end, [Token::Number(sign)]);
        }
        i += 1;
    }
    Some(serialize_computed(&tokens))
}

fn angle(value: &str) -> Option<f64> {
    if !crate::math::angle(value) { return None }
    let tokens = tokenize(value).into_iter().map(|token| match token {
        Token::Dimension(n, unit) => values::Angle::parse(&format!("{n}{unit}"))
            .map(|angle| Token::Number(angle.radians.to_degrees())),
        token => Some(token),
    }).collect::<Option<Vec<_>>>()?;
    values::alpha_value(&serialize_computed(&tokens)).filter(|n| n.is_finite())
}

fn rotation(parts: &[String]) -> Option<([f64; 3], f64)> {
    let (angle_index, angle) = parts.iter().enumerate().find_map(|(i, value)| angle(value).map(|a| (i, a)))?;
    let axis: Vec<&str> = parts.iter().enumerate().filter(|(i, _)| *i != angle_index).map(|(_, s)| s.as_str()).collect();
    let axis = match axis.as_slice() {
        [] => [0.0, 0.0, 1.0],
        [axis] => match axis.to_ascii_lowercase().as_str() {
            "x" => [1.0, 0.0, 0.0], "y" => [0.0, 1.0, 0.0], "z" => [0.0, 0.0, 1.0], _ => return None,
        },
        [x, y, z] if angle_index == 0 || angle_index == 3 => [numeric(x, false)?, numeric(y, false)?, numeric(z, false)?],
        _ => return None,
    };
    Some((axis, angle))
}

pub(crate) fn valid(property: &str, value: &str) -> bool {
    if value.trim().eq_ignore_ascii_case("none") { return true }
    let Some(parts) = parts(value) else { return false };
    match property {
        "scale" => (1..=3).contains(&parts.len()) && parts.iter().all(|p| signs(p, None).and_then(|p| numeric(&p, true)).is_some()),
        "rotate" => rotation(&parts).is_some(),
        "translate" => (1..=3).contains(&parts.len()) && parts.iter().enumerate().all(|(i, p)| {
            match values::LengthPercentage::parse(p) {
                Some(values::LengthPercentage::Percentage(_)) => i < 2,
                Some(_) => true,
                None => crate::math::length_percentage(p, i < 2),
            }
        }),
        _ => false,
    }
}

fn number(n: f64) -> String {
    let n = if n == 0.0 { 0.0 } else if (n - n.round()).abs() < 1e-10 { n.round() } else { n };
    n.to_string()
}

fn length(css: &str, font: f64, root: f64) -> String {
    let mut tokens = tokenize(css);
    for token in &mut tokens {
        if let Token::Dimension(n, unit) = token {
            if let Some(length) = values::LengthPercentage::parse(&format!("{n}{unit}")) {
                let mut value = ComputedValue::LengthPercentage(length);
                values::resolve_font_units(&mut value, font, root, false);
                if let ComputedValue::LengthPercentage(values::LengthPercentage::Px(px)) = value {
                    *token = Token::Dimension(px, "px".to_owned());
                }
            }
        }
    }
    let text = serialize_computed(&tokens);
    match values::LengthPercentage::parse(&text) {
        Some(values::LengthPercentage::Px(px)) => format!("{}px", number(px)),
        Some(values::LengthPercentage::Percentage(percent)) => format!("{}%", number(percent)),
        _ if crate::math::length_percentage(&text, false) => crate::math::font_size_px(&text, font, root)
            .map(|px| format!("{}px", number(px))).unwrap_or(text),
        // A linear calc length-percentage preserves its percentage, including
        // 0%, rather than resolving against an arbitrary box. Comparison and
        // piecewise math stays symbolic for the used-value consumer.
        _ if text.to_ascii_lowercase().starts_with("calc(")
            && !tokens.iter().any(|t| matches!(t, Token::Function(n) if !n.eq_ignore_ascii_case("calc"))) => {
            match (crate::math::font_size_px(&text, 0.0, root), crate::math::font_size_px(&text, 100.0, root)) {
                (Some(px), Some(at100)) => {
                    let percent = at100 - px;
                    if px == 0.0 { format!("{}%", number(percent)) }
                    else { format!("calc({}% {} {}px)", number(percent), if px < 0.0 { "-" } else { "+" }, number(px.abs())) }
                }
                _ => text,
            }
        }
        _ => text,
    }
}

pub(super) fn resolve(properties: &mut BTreeMap<String, String>, root: f64) {
    let font = properties.get("font-size").and_then(|v| v.strip_suffix("px")).and_then(|v| v.parse().ok()).unwrap_or(16.0);
    for property in ["scale", "rotate", "translate"] {
        let Some(value) = properties.get(property).cloned() else { continue };
        if value.eq_ignore_ascii_case("none") {
            properties.insert(property.to_owned(), "none".to_owned());
            continue
        }
        let Some(parts) = parts(&value) else { continue };
        let computed = match property {
            "scale" => {
                let numbers: Option<Vec<f64>> = parts.iter().map(|p| numeric(&signs(p, Some((font, root)))?, true)).collect();
                let Some(mut numbers) = numbers else { continue };
                if numbers.len() == 3 && numbers[2] == 1.0 { numbers.pop(); }
                if numbers.len() == 2 && numbers[0] == numbers[1] { numbers.pop(); }
                numbers.into_iter().map(number).collect::<Vec<_>>().join(" ")
            }
            "rotate" => {
                let Some((axis, mut angle)) = rotation(&parts) else { continue };
                let nonzero: Vec<usize> = (0..3).filter(|i| axis[*i] != 0.0).collect();
                let axis = if nonzero.len() == 1 {
                    let i = nonzero[0];
                    if axis[i] < 0.0 { angle = -angle; }
                    match i { 0 => "x ".to_owned(), 1 => "y ".to_owned(), _ => String::new() }
                } else { format!("{} {} {} ", number(axis[0]), number(axis[1]), number(axis[2])) };
                format!("{axis}{}deg", number(angle))
            }
            _ => {
                let mut lengths: Vec<String> = parts.iter().map(|p| length(p, font, root)).collect();
                if lengths.len() == 3 && lengths[2] == "0px" { lengths.pop(); }
                if lengths.len() == 2 && lengths[1] == "0px" { lengths.pop(); }
                lengths.join(" ")
            }
        };
        properties.insert(property.to_owned(), computed);
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
    fn axis_grammars_and_type_checks() {
        for (property, values) in [
            ("scale", vec!["", "none 1", "1px", "1, 2", "1 2 3 4", "calc(1px)", "calc(1 + 1%)"]),
            ("rotate", vec!["0", "1deg 2deg", "x y 1deg", "1 2 1deg", "1 1deg 2 3", "calc(1deg + 1)"]),
            ("translate", vec!["1", "1deg", "1px 2px 3%", "1px 2px calc(3px + 1%)", "none 1px"]),
        ] { for value in values { assert!(!valid(property, value), "{property}: {value}"); } }
        for (property, value) in [("scale", "calc(200%) 3"), ("scale", "2 calc(300% * sign(1em - 1px)"), ("rotate", "calc(1turn / 2) x"), ("translate", "1em calc(2% + 3px) -4px")] {
            assert!(valid(property, value), "{property}: {value}");
        }
    }
    #[test]
    fn computed_identity_axis_and_percentage_serialization() {
        let s = style("font-size:20px; translate:1em 0 0; rotate:-2 0 0 1turn; scale:200% calc(2) 1");
        assert_eq!(s.get("translate"), Some("20px"));
        assert_eq!(s.get("rotate"), Some("x -360deg"));
        assert_eq!(s.get("scale"), Some("2"));
        let s = style("translate:0% 0% 1px; rotate:0 0 0 0deg; scale:1 1 2");
        assert_eq!(s.get("translate"), Some("0% 0% 1px"));
        assert_eq!(s.get("rotate"), Some("0 0 0 0deg"));
        assert_eq!(s.get("scale"), Some("1 1 2"));
        let s = style("translate:0; rotate:0deg; scale:1");
        assert_eq!(s.get("translate"), Some("0px"));
        assert_eq!(s.get("rotate"), Some("0deg"));
        assert_eq!(s.get("scale"), Some("1"));
    }
    #[test]
    fn variables_and_invalid_winners_obey_cascade() {
        let s = style("scale:2; scale:1px; rotate:20deg; --axis:y 1turn; rotate:var(--axis); translate:10px; --bad:20%; translate:1px 2px var(--bad)");
        assert_eq!(s.get("scale"), Some("2"));
        assert_eq!(s.get("rotate"), Some("y 360deg"));
        assert_eq!(s.get("translate"), Some("none"));
        let s = style("scale:2; --bad:1px; scale:var(--bad); rotate:NONE; translate:NONE");
        assert_eq!(s.get("scale"), Some("none"));
        assert_eq!(s.get("rotate"), Some("none"));
        assert_eq!(s.get("translate"), Some("none"));
    }
}
