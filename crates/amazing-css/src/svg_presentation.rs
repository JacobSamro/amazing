//! SVG 2 presentation attributes. Values enter the ordinary CSS cascade as
//! low-priority author hints after SVG's attribute-specific grammar is checked.
use amazing_dom::{Dom, Namespace, NodeIdx};
use crate::syntax::{Token, tokenize, serialize};
use crate::values::{self, ComputedValue, LengthPercentage, SvgPaint, SvgResource};

pub(crate) fn collect(dom: &Dom, el: NodeIdx) -> Vec<(String, String)> {
    if dom.namespace(el) != Some(Namespace::Svg) { return Vec::new(); }
    let element = dom.local_name(el).unwrap_or("");
    dom.attributes_expanded(el).unwrap_or_default().into_iter()
        .filter(|attribute| attribute.namespace_uri.is_none())
        .filter_map(|attribute| normalize(attribute.local_name, attribute.value, element))
        .collect()
}

pub(crate) fn normalize(attribute: &str, raw: &str, element: &str) -> Option<(String, String)> {
    let property = match attribute {
        "patternTransform" if element == "pattern" => "transform",
        "gradientTransform" if matches!(element, "linearGradient" | "radialGradient") => "transform",
        "transform" if !matches!(element, "pattern" | "linearGradient" | "radialGradient") => "transform",
        "fill" if !matches!(element, "animate" | "animateMotion" | "animateTransform" | "set") => "fill",
        "fill-opacity" | "fill-rule" | "stroke" | "stroke-width" | "stroke-opacity"
        | "stroke-linecap" | "stroke-linejoin" | "stroke-miterlimit" | "stroke-dasharray"
        | "stroke-dashoffset" | "opacity" | "color" | "display" | "visibility"
        | "font-family" | "font-size" | "font-size-adjust" | "font-stretch"
        | "font-style" | "font-variant" | "font-weight" | "text-anchor"
        | "dominant-baseline" | "clip-path" | "mask" | "clip-rule"
        | "vector-effect" => attribute,
        _ => return None,
    };
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('!') || raw.contains(';') { return None; }
    let value = if property == "transform" { normalize_transform(raw)? }
        else if matches!(property, "stroke-width" | "stroke-dashoffset" | "font-size") {
            svg_length(raw, property != "stroke-dashoffset")?
        } else if property == "stroke-dasharray" { normalize_dasharray(raw)? }
        else { raw.to_owned() };
    if matches!(value.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset" | "revert" | "revert-layer") {
        return Some((property.to_owned(), value));
    }
    let valid = match property {
        "fill" | "stroke" | "fill-rule" | "stroke-width" | "stroke-opacity"
        | "stroke-linecap" | "stroke-linejoin" | "stroke-miterlimit"
        | "stroke-dasharray" | "stroke-dashoffset" | "text-anchor"
        | "dominant-baseline" | "clip-path" | "mask" | "clip-rule"
        | "vector-effect" | "font-size-adjust" => parse_property(property, &value).is_some(),
        "fill-opacity" | "opacity" => opacity(&value).is_some(),
        "transform" => values::parse("transform", &value).is_some(),
        "color" | "display" | "visibility" | "font-size" => values::parse(property, &value).is_some(),
        "font-family" => !value.contains('{') && !value.contains('}'),
        "font-style" => matches!(value.to_ascii_lowercase().as_str(), "normal" | "italic" | "oblique"),
        "font-variant" => matches!(value.to_ascii_lowercase().as_str(), "normal" | "small-caps"),
        "font-weight" => matches!(value.to_ascii_lowercase().as_str(), "normal" | "bold" | "bolder" | "lighter" | "100" | "200" | "300" | "400" | "500" | "600" | "700" | "800" | "900"),
        "font-stretch" => matches!(value.to_ascii_lowercase().as_str(), "normal" | "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed" | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded"),
        _ => false,
    };
    valid.then_some((property.to_owned(), value))
}

fn opacity(value: &str) -> Option<f64> {
    match crate::syntax::tokenize(value).as_slice() {
        [Token::Number(n)] if n.is_finite() => Some(n.clamp(0.0, 1.0)),
        [Token::Percentage(n)] if n.is_finite() => Some((n / 100.0).clamp(0.0, 1.0)),
        _ => None,
    }
}
fn svg_length(value: &str, nonnegative: bool) -> Option<String> {
    let tokens = tokenize(value);
    if let [Token::Number(n)] = tokens.as_slice() {
        if !n.is_finite() || (nonnegative && *n < 0.0) { return None; }
        return Some(format!("{n}px"));
    }
    let length = LengthPercentage::parse(value)?;
    let negative = match length { LengthPercentage::Px(n) | LengthPercentage::Percentage(n) | LengthPercentage::Relative(n, _) => n < 0.0 };
    if nonnegative && negative { return None; }
    Some(value.to_owned())
}
fn normalize_dasharray(value: &str) -> Option<String> {
    if value.eq_ignore_ascii_case("none") { return Some("none".into()); }
    let parts: Vec<_> = value.replace(',', " ").split_whitespace().map(str::to_owned).collect();
    if parts.is_empty() { return None; }
    parts.iter().map(|part| svg_length(part, true)).collect::<Option<Vec<_>>>().map(|parts| parts.join(", "))
}
fn url(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.to_ascii_lowercase().starts_with("url(") || !value.ends_with(')') { return None; }
    let inner = value[4..value.len()-1].trim();
    (!inner.is_empty() && !inner.contains(')')).then_some(value.to_owned())
}

pub(crate) fn is_svg_property(property: &str) -> bool {
    matches!(property, "fill" | "stroke" | "fill-opacity" | "stroke-opacity"
        | "fill-rule" | "clip-rule" | "stroke-linecap" | "stroke-linejoin"
        | "vector-effect" | "text-anchor" | "dominant-baseline"
        | "stroke-width" | "stroke-dashoffset" | "stroke-miterlimit"
        | "stroke-dasharray" | "clip-path" | "mask" | "font-size-adjust")
}

/// Typed grammars for SVG-specific longhands. Other properties reuse CSS types.
pub(crate) fn parse_property(property: &str, value: &str) -> Option<ComputedValue> {
    use ComputedValue as V;
    let lower = value.trim().to_ascii_lowercase();
    let s = lower.as_str();
    match property {
        "fill" | "stroke" => {
            if s == "none" { return Some(V::SvgPaint(SvgPaint::None)); }
            if s == "currentcolor" { return Some(V::SvgPaint(SvgPaint::CurrentColor)); }
            if matches!(s, "context-fill" | "context-stroke") { return Some(V::SvgKeyword(value.into())); }
            if let Some(url) = url(value) { return Some(V::SvgPaint(SvgPaint::Url(url))); }
            crate::color::parse_css_color(value).map(|color| V::SvgPaint(SvgPaint::Color(color)))
        }
        "fill-opacity" | "stroke-opacity" => opacity(value).map(V::Number),
        "fill-rule" | "clip-rule" if matches!(s, "nonzero" | "evenodd") => Some(V::SvgKeyword(s.into())),
        "stroke-linecap" if matches!(s, "butt" | "round" | "square") => Some(V::SvgKeyword(s.into())),
        "stroke-linejoin" if matches!(s, "miter" | "round" | "bevel") => Some(V::SvgKeyword(s.into())),
        "vector-effect" if matches!(s, "none" | "non-scaling-stroke") => Some(V::SvgKeyword(s.into())),
        "text-anchor" if matches!(s, "start" | "middle" | "end") => Some(V::SvgKeyword(s.into())),
        "dominant-baseline" if matches!(s, "auto" | "text-bottom" | "alphabetic" | "ideographic" | "middle" | "central" | "mathematical" | "hanging" | "text-top" | "text-before-edge" | "text-after-edge") => Some(V::SvgKeyword(s.into())),
        "stroke-width" => LengthPercentage::parse(value).filter(|n| nonnegative(*n)).map(V::LengthPercentage),
        "stroke-dashoffset" => LengthPercentage::parse(value).map(V::LengthPercentage),
        "stroke-miterlimit" => value.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0).map(V::Number),
        "stroke-dasharray" if s == "none" => Some(V::SvgDasharray(Vec::new())),
        "stroke-dasharray" => {
            let parts: Vec<_> = value.replace(',', " ").split_whitespace().map(str::to_owned).collect();
            if parts.is_empty() { return None; }
            parts.iter().map(|part| LengthPercentage::parse(part).filter(|n| nonnegative(*n))).collect::<Option<Vec<_>>>().map(V::SvgDasharray)
        }
        "clip-path" | "mask" if s == "none" => Some(V::SvgResource(SvgResource::None)),
        "clip-path" | "mask" => url(value).map(|url| V::SvgResource(SvgResource::Url(url))),
        "font-size-adjust" => crate::font_size_adjust::FontSizeAdjust::parse(value).map(V::FontSizeAdjust),
        _ => None,
    }
}
fn nonnegative(value: LengthPercentage) -> bool {
    match value { LengthPercentage::Px(n) | LengthPercentage::Percentage(n) | LengthPercentage::Relative(n, _) => n >= 0.0 }
}

fn normalize_transform(raw: &str) -> Option<String> {
    if raw.eq_ignore_ascii_case("none") { return Some("none".into()); }
    let tokens = tokenize(raw);
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < tokens.len() {
        while matches!(tokens.get(pos), Some(Token::Whitespace | Token::Comma)) { pos += 1; }
        if pos == tokens.len() { break; }
        let Token::Function(name) = tokens.get(pos)? else { return None; };
        let name = name.to_ascii_lowercase();
        pos += 1;
        if !matches!(tokens.get(pos), Some(Token::OpenParen)) { return None; }
        pos += 1;
        let start = pos;
        let mut depth = 1;
        while pos < tokens.len() && depth > 0 {
            match tokens[pos] { Token::OpenParen => depth += 1, Token::CloseParen => depth -= 1, _ => {} }
            pos += 1;
        }
        if depth != 0 { return None; }
        let inner = &tokens[start..pos-1];
        let args = crate::computed::transform_arguments(inner)?;
        let args: Vec<_> = args.iter().map(|arg| serialize(arg)).collect();
        let number = |index: usize| args.get(index).and_then(|arg| arg.parse::<f64>().ok()).filter(|n| n.is_finite());
        let css = match name.as_str() {
            "translate" if (1..=2).contains(&args.len()) => {
                let x = svg_length(&args[0], false)?;
                let y = if args.len() == 2 { svg_length(&args[1], false)? } else { "0px".into() };
                format!("translate({x}, {y})")
            }
            "rotate" if args.len() == 1 => format!("rotate({}deg)", number(0)?),
            "rotate" if args.len() == 3 => {
                let (angle, x, y) = (number(0)?, number(1)?, number(2)?);
                format!("translate({x}px, {y}px) rotate({angle}deg) translate({}px, {}px)", -x, -y)
            }
            "skewx" | "skewy" if args.len() == 1 => format!("{}({}deg)", name, number(0)?),
            "scale" if (1..=2).contains(&args.len()) && args.iter().all(|a| a.parse::<f64>().is_ok_and(|n| n.is_finite())) => format!("scale({})", args.join(", ")),
            "matrix" if args.len() == 6 && args.iter().all(|a| a.parse::<f64>().is_ok_and(|n| n.is_finite())) => format!("matrix({})", args.join(", ")),
            _ => return None,
        };
        out.push(css);
    }
    (!out.is_empty()).then(|| out.join(" "))
}
