//! CSS Images 3/4 argument validation. Keep image text for paint, but reject
//! invalid image-list items, explicit radial radii, and cross-fade percentages.
use crate::computed::{top_level_components, trim_tokens, function_inner};
use crate::syntax::{Token, tokenize, serialize};

pub(crate) fn groups(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut depth = Vec::new();
    let mut start = 0;
    let mut out = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => depth.push(0),
            Token::OpenSquare => depth.push(1),
            Token::OpenCurly => depth.push(2),
            Token::CloseParen if depth.pop() != Some(0) => return None,
            Token::CloseSquare if depth.pop() != Some(1) => return None,
            Token::CloseCurly if depth.pop() != Some(2) => return None,
            Token::Comma if depth.is_empty() => { out.push(trim_tokens(&tokens[start..index])); start = index + 1; }
            _ => {}
        }
    }
    if !depth.is_empty() { return None }
    out.push(trim_tokens(&tokens[start..]));
    out.iter().all(|group| !group.is_empty()).then_some(out)
}

pub(crate) fn valid_image_source(value: &str) -> bool {
    groups(&tokenize(value)).is_some_and(|layers| layers.len() == 1 && layers.into_iter().all(|layer| {
        matches!(layer, [Token::Ident(name)] if name.eq_ignore_ascii_case("none")) || image(layer, 0)
    }))
}

pub(crate) fn valid_background_image(value: &str) -> bool {
    groups(&tokenize(value)).is_some_and(|layers| layers.into_iter().all(|layer| {
        matches!(layer, [Token::Ident(name)] if name.eq_ignore_ascii_case("none"))
            || image(layer, 0)
    }))
}

pub(crate) fn color(tokens: &[Token]) -> bool {
    if let Some(Token::Function(name)) = trim_tokens(tokens).first() {
        if !matches!(name.to_ascii_lowercase().as_str(), "rgb" | "rgba" | "hsl" | "hsla"
            | "hwb" | "lab" | "lch" | "oklab" | "oklch" | "color" | "color-mix"
            | "light-dark" | "alpha") { return false }
    }
    crate::values::parse("color", &serialize(tokens)).is_some()
}

fn image(tokens: &[Token], depth: usize) -> bool {
    if depth >= 64 { return false }
    let tokens = trim_tokens(tokens);
    if matches!(tokens, [Token::Url { .. }]) { return true }
    let Some(Token::Function(name)) = tokens.first() else { return false };
    let Some(inner) = function_inner(tokens, name) else { return false };
    if trim_tokens(inner).is_empty() { return false }
    match name.to_ascii_lowercase().as_str() {
        "light-dark" => groups(inner).is_some_and(|items| items.len() == 2
            && items.into_iter().all(|item| image(item, depth + 1))),
        "cross-fade" => cross_fade(inner, depth + 1),
        "radial-gradient" | "repeating-radial-gradient" | "linear-gradient" | "repeating-linear-gradient" | "conic-gradient" | "repeating-conic-gradient" => crate::gradient_values::canonical(tokens, name).is_some(),
        // These functions have distinct grammars outside this parameter area.
        // Preserve their balanced representation for the existing paint parser.
        "image" | "image-set" | "-webkit-image-set"
        | "element" | "paint" | "url" | "-webkit-gradient" => true,
        _ => false,
    }
}

fn cross_fade(tokens: &[Token], depth: usize) -> bool {
    let Some(items) = groups(tokens) else { return false };
    // <cf-image># permits a single image, and an optional percentage per item.
    items.into_iter().all(|item| {
        let Some(parts) = top_level_components(item) else { return false };
        let mut has_image = false;
        let mut has_percentage = false;
        for part in parts {
            if image(part, depth) || color(part) {
                if has_image { return false }
                has_image = true;
            } else {
                if has_percentage { return false }
                match trim_tokens(part) {
                    [Token::Percentage(n)] if n.is_finite() && (0.0..=100.0).contains(n) => {},
                    [Token::Function(_), ..] if crate::math::percentage(&serialize(part)) => {},
                    _ => return false,
                }
                has_percentage = true;
            }
        }
        has_image
    })
}

pub(crate) fn radial(tokens: &[Token]) -> bool {
    let Some(items) = groups(tokens) else { return false };
    let Some(parts) = top_level_components(items[0]) else { return false };
    // Only the first comma item may be the shape/size/position prelude.
    // A negative color-stop offset in later items is valid and stays untouched.
    if parts.first().is_some_and(|part| color(part)) { return true }
    let mut shape = None;
    let mut extent = false;
    let mut radii = Vec::new();
    for part in parts {
        match trim_tokens(part) {
            [Token::Ident(word)] if word.eq_ignore_ascii_case("at") => break,
            [Token::Ident(word)] if matches!(word.to_ascii_lowercase().as_str(), "circle" | "ellipse") => {
                if shape.replace(word.to_ascii_lowercase()).is_some() { return false }
            },
            [Token::Ident(word)] if matches!(word.to_ascii_lowercase().as_str(),
                "closest-side" | "closest-corner" | "farthest-side" | "farthest-corner") => {
                if extent { return false }
                extent = true;
            },
            // Color interpolation extends the prelude in CSS Images 4. Leave
            // that independently-defined clause to the downstream parser.
            [Token::Ident(word)] if word.eq_ignore_ascii_case("in") => break,
            _ => {
                let text = serialize(part);
                if let Some(radius) = crate::values::LengthPercentage::parse(&text) {
                    if !radius.is_nonnegative() { return false }
                    radii.push(matches!(radius, crate::values::LengthPercentage::Percentage(_)));
                } else if matches!(trim_tokens(part), [Token::Dimension(n, unit)]
                    if n.is_finite() && *n >= 0.0 && matches!(unit.to_ascii_lowercase().as_str(),
                        "vi" | "vb" | "svi" | "svb" | "lvi" | "lvb" | "dvi" | "dvb"
                        | "cqw" | "cqh" | "cqi" | "cqb" | "cqmin" | "cqmax")) {
                    // Grammar acceptance cannot depend on the downstream
                    // layout consumer already resolving this length unit.
                    radii.push(false);
                } else if crate::math::length_percentage(&text, true) {
                    radii.push(!crate::math::length_percentage(&text, false));
                } else {
                    return false;
                }
            },
        }
    }
    if extent && !radii.is_empty() { return false }
    match (shape.as_deref(), radii.len()) {
        (_, 0) => true,
        (Some("ellipse"), 2) | (None, 2) => true,
        (Some("circle") | None, 1) => !radii[0],
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_image_winners_default_after_variable_substitution() {
        let mut dom = amazing_dom::Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let sheet = crate::parse_stylesheet("div { background-image:url(fallback.png); --bad:cross-fade(101% red, blue); background-image:var(--bad) }");
        let styled = crate::compute(&dom, &[sheet]);
        assert_eq!(styled.get(div).unwrap().get("background-image"), Some("none"));
        let sheet = crate::parse_stylesheet("div { background-image:url(fallback.png); background-image:radial-gradient(circle -10px, red, blue) }");
        let styled = crate::compute(&dom, &[sheet]);
        assert!(styled.get(div).unwrap().get("background-image").unwrap().contains("fallback.png"));
    }

    #[test]
    fn image_parameters_have_distinct_types_and_ranges() {
        for value in ["none, auto", "radial-gradient(circle -10px at center, red, blue)",
            "radial-gradient(ellipse 20px -30px, red, blue)", "radial-gradient(circle 20%, red, blue)",
            "radial-gradient(circle calc(20%), red, blue)", "cross-fade(auto blue, red)", "cross-fade(1px red, green)",
            "cross-fade(calc(1% + 1px) red, green)", "cross-fade(-1% red, green)",
            "cross-fade(101% red, green)", "cross-fade(10% 20% red, blue)",
            "light-dark(url(a.png), blue)", "url(a.png),"] {
            assert!(!valid_background_image(value), "{value}");
        }
        for value in ["none, url(a.png)", "radial-gradient(ellipse 20% 30% at center, red, blue)",
            "radial-gradient(circle 10px at center, red -10px, blue)",
            "radial-gradient(circle 10cqw, red, blue)",
            "radial-gradient(ellipse calc(20%) calc(30%), red, blue)",
            "cross-fade(red 33%, white 33%, blue)", "cross-fade(calc(20% + 30%) red, blue)",
            "cross-fade(red 1%, cross-fade(red 2%, green))", "light-dark(image(blue), url(a.png))"] {
            assert!(valid_background_image(value), "{value}");
        }
    }
}
