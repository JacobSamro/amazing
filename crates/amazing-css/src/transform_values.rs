//! CSS Transforms 1/2 complete transform-list grammar and used matrix handoff.
//! Reference-box percentages and unresolved font/viewport dimensions retain their basis.
use crate::syntax::{Token, serialize, tokenize};
use crate::values::{Angle, LengthPercentage, Transform};
use std::collections::BTreeMap;

pub(crate) fn functions(css: &str) -> Option<Vec<(String, Vec<String>)>> {
    let mut tokens = tokenize(css);
    let mut depth = 0usize;
    for token in &tokens {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    tokens.extend(std::iter::repeat_n(Token::CloseParen, depth));
    let mut result = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], Token::Whitespace) {
            i += 1;
            continue;
        }
        let Token::Function(name) = &tokens[i] else {
            return None;
        };
        let name = name.to_ascii_lowercase();
        i += 1;
        if !matches!(tokens.get(i), Some(Token::OpenParen)) {
            return None;
        }
        i += 1;
        let start = i;
        let mut depth = 1usize;
        while i < tokens.len() {
            match tokens[i] {
                Token::OpenParen => depth += 1,
                Token::CloseParen => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            i += 1;
        }
        if depth != 0 {
            return None;
        }
        let inner = &tokens[start..i];
        let mut args = Vec::new();
        let mut start = 0;
        let mut nested = 0usize;
        for (j, token) in inner.iter().enumerate() {
            match token {
                Token::OpenParen => nested += 1,
                Token::CloseParen => nested = nested.checked_sub(1)?,
                Token::Comma if nested == 0 => {
                    let text = serialize(&inner[start..j]);
                    if text.trim().is_empty() {
                        return None;
                    }
                    args.push(text.trim().to_owned());
                    start = j + 1;
                }
                _ => {}
            }
        }
        let tail = serialize(&inner[start..]);
        if tail.trim().is_empty() {
            return None;
        }
        args.push(tail.trim().to_owned());
        result.push((name, args));
        i += 1;
    }
    (!result.is_empty()).then_some(result)
}
fn numeric(css: &str, percentage: bool) -> Option<f64> {
    if !crate::math::number_percentage(css, percentage) {
        return None;
    }
    let n = if percentage {
        crate::values::alpha_value(css)?
    } else {
        crate::math::computed_number(css)?
    };
    n.is_finite().then_some(n)
}
fn angle(css: &str) -> Option<Angle> {
    Angle::parse(css).or_else(|| {
        crate::math::computed_angle(css)
            .filter(|n| n.is_finite())
            .map(|radians| Angle { radians })
    })
}
fn valid_length(css: &str, percentage: bool) -> bool {
    if let Some(value) = LengthPercentage::parse(css) {
        return percentage || !matches!(value, LengthPercentage::Percentage(_));
    }
    crate::math::length_percentage(css, percentage)
}
fn translation(args: [&str; 3], three_d: bool) -> Option<Transform> {
    if !valid_length(args[0], true) || !valid_length(args[1], true) || !valid_length(args[2], false)
    {
        return None;
    }
    match (
        LengthPercentage::parse(args[0]),
        LengthPercentage::parse(args[1]),
        LengthPercentage::parse(args[2]),
    ) {
        (Some(x), Some(y), Some(z)) => Some(if three_d {
            Transform::Translate3D(x, y, z)
        } else {
            Transform::Translate(x, y)
        }),
        _ => Some(Transform::TranslateExpression(args.map(str::to_owned))),
    }
}
pub(crate) fn parse(css: &str) -> Option<Vec<Transform>> {
    if css.trim().eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for (name, args) in functions(css)? {
        let a = |i: usize| args.get(i).map(String::as_str);
        let op = match name.as_str() {
            "translate" if (1..=2).contains(&args.len()) => {
                translation([a(0)?, a(1).unwrap_or("0"), "0"], false)?
            }
            "translatex" if args.len() == 1 => translation([a(0)?, "0", "0"], false)?,
            "translatey" if args.len() == 1 => translation(["0", a(0)?, "0"], false)?,
            "translatez" if args.len() == 1 => translation(["0", "0", a(0)?], true)?,
            "translate3d" if args.len() == 3 => translation([a(0)?, a(1)?, a(2)?], true)?,
            "scale" if (1..=2).contains(&args.len()) => {
                Transform::Scale(numeric(a(0)?, true)?, numeric(a(1).unwrap_or(a(0)?), true)?)
            }
            "scalex" if args.len() == 1 => Transform::Scale(numeric(a(0)?, true)?, 1.0),
            "scaley" if args.len() == 1 => Transform::Scale(1.0, numeric(a(0)?, true)?),
            "scalez" if args.len() == 1 => Transform::Scale3D([1.0, 1.0, numeric(a(0)?, true)?]),
            "scale3d" if args.len() == 3 => Transform::Scale3D([
                numeric(a(0)?, true)?,
                numeric(a(1)?, true)?,
                numeric(a(2)?, true)?,
            ]),
            "rotate" | "rotatez" if args.len() == 1 => Transform::Rotate(angle(a(0)?)?),
            "rotatex" | "rotatey" if args.len() == 1 => Transform::Rotate3D(
                if name == "rotatex" {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                },
                angle(a(0)?)?,
            ),
            "rotate3d" if args.len() == 4 => Transform::Rotate3D(
                [
                    numeric(a(0)?, false)?,
                    numeric(a(1)?, false)?,
                    numeric(a(2)?, false)?,
                ],
                angle(a(3)?)?,
            ),
            "skew" if (1..=2).contains(&args.len()) => {
                Transform::Skew(angle(a(0)?)?, angle(a(1).unwrap_or("0"))?)
            }
            "skewx" if args.len() == 1 => Transform::SkewX(angle(a(0)?)?),
            "skewy" if args.len() == 1 => Transform::SkewY(angle(a(0)?)?),
            "matrix" if args.len() == 6 => {
                let mut v = [0.0; 6];
                for (i, n) in v.iter_mut().enumerate() {
                    *n = numeric(a(i)?, false)?;
                }
                Transform::Matrix(v)
            }
            "matrix3d" if args.len() == 16 => {
                let mut v = [0.0; 16];
                for (i, n) in v.iter_mut().enumerate() {
                    *n = numeric(a(i)?, false)?;
                }
                Transform::Matrix3D(v)
            }
            "perspective" if args.len() == 1 => {
                if a(0)?.eq_ignore_ascii_case("none") {
                    Transform::Perspective(None)
                } else {
                    if !valid_length(a(0)?, false) {
                        return None;
                    }
                    if let Some(LengthPercentage::Px(n) | LengthPercentage::Relative(n, _)) =
                        LengthPercentage::parse(a(0)?)
                    {
                        if n < 0.0 {
                            return None;
                        }
                    }
                    Transform::Perspective(Some(a(0)?.to_owned()))
                }
            }
            _ => return None,
        };
        out.push(op);
    }
    Some(out)
}
/// Resolve only CSS-owned units; reference-box percentages remain symbolic.
pub(crate) fn resolve(properties: &mut BTreeMap<String, String>, root: f64) {
    let Some(source) = properties.get("transform").cloned() else {
        return;
    };
    if source.eq_ignore_ascii_case("none") {
        return;
    }
    let Some(mut functions) = functions(&source) else {
        return;
    };
    let font = properties
        .get("font-size")
        .and_then(|s| s.strip_suffix("px"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(16.0);
    for (name, args) in &mut functions {
        for (i, arg) in args.iter_mut().enumerate() {
            let length = name.starts_with("translate")
                || name == "perspective" && !arg.eq_ignore_ascii_case("none");
            if length {
                *arg = crate::computed::individual_transforms::length(arg, font, root);
            }
            if name == "perspective"
                && let Some(LengthPercentage::Px(n)) = LengthPercentage::parse(arg)
            {
                *arg = format!("{}px", n.max(0.0));
            }
            let angle_arg = name.starts_with("rotate") && (name != "rotate3d" || i == 3)
                || name.starts_with("skew");
            if angle_arg && let Some(degrees) = crate::math::computed_angle_degrees(arg) {
                *arg = format!("{degrees}deg");
            }
        }
    }
    let css = functions
        .into_iter()
        .map(|(name, args)| {
            let name = match name.as_str() {
                "translatex" => "translateX",
                "translatey" => "translateY",
                "translatez" => "translateZ",
                "scalex" => "scaleX",
                "scaley" => "scaleY",
                "scalez" => "scaleZ",
                "rotatex" => "rotateX",
                "rotatey" => "rotateY",
                "rotatez" => "rotateZ",
                "skewx" => "skewX",
                "skewy" => "skewY",
                _ => &name,
            };
            format!("{name}({})", args.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" ");
    if parse(&css).is_some() {
        properties.insert("transform".into(), css);
    }
}

#[cfg(test)]
#[path = "transform_values_tests.rs"]
mod tests;
