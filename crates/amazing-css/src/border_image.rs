//! CSS Backgrounds 3 §5: border-image grammar and independent cascade axes.
use crate::computed::{top_level_components, trim_tokens};
use crate::syntax::{Token, serialize, tokenize};
use std::collections::BTreeMap;
pub(crate) const AXES: [&str; 5] = [
    "border-image-source",
    "border-image-slice",
    "border-image-width",
    "border-image-outset",
    "border-image-repeat",
];
pub(crate) const INITIAL: [&str; 5] = ["none", "100%", "1", "0", "stretch"];
pub(crate) fn supported(name: &str) -> bool {
    name == "border-image" || AXES.contains(&name)
}
fn parts(value: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    for token in tokenize(value) {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.checked_sub(1)?,
            Token::Delim('/') if depth == 0 => {
                tokens.push(Token::Whitespace);
                tokens.push(token);
                tokens.push(Token::Whitespace);
                continue;
            }
            _ => {}
        }
        tokens.push(token);
    }
    tokens.extend(std::iter::repeat_n(Token::CloseParen, depth));
    Some(
        top_level_components(&tokens)?
            .into_iter()
            .map(serialize)
            .collect(),
    )
}
fn keyword(css: &str, allowed: &[&str]) -> Option<String> {
    let tokens = tokenize(css);
    let [Token::Ident(word)] = trim_tokens(&tokens) else {
        return None;
    };
    allowed
        .contains(&word.to_ascii_lowercase().as_str())
        .then(|| word.to_ascii_lowercase())
}
fn source(css: &str) -> bool {
    crate::image_values::valid_image_source(css)
}

fn numeric(css: &str, index: usize) -> bool {
    let tokens = tokenize(css);
    match trim_tokens(&tokens) {
        [Token::Number(n)] => n.is_finite() && *n >= 0.0,
        [Token::Percentage(n)] => matches!(index, 1 | 2) && n.is_finite() && *n >= 0.0,
        [Token::Dimension(n, _)] => {
            matches!(index, 2 | 3)
                && n.is_finite()
                && *n >= 0.0
                && crate::values::LengthPercentage::parse(css).is_some()
        }
        [Token::Ident(word)] => index == 2 && word.eq_ignore_ascii_case("auto"),
        _ => {
            let Some(validation) = crate::computed::individual_transforms::signs(css, None) else {
                return false;
            };
            crate::math::number_percentage(&validation, matches!(index, 1 | 2))
                || (matches!(index, 2 | 3)
                    && crate::math::length_percentage(&validation, index == 2))
        }
    }
}
fn compress(mut values: Vec<String>) -> String {
    if values.len() == 4 && values[3] == values[1] {
        values.pop();
    }
    if values.len() == 3 && values[2] == values[0] {
        values.pop();
    }
    if values.len() == 2 && values[1] == values[0] {
        values.pop();
    }
    values.join(" ")
}
pub(crate) fn canonical(name: &str, value: &str) -> Option<String> {
    if let Some(wide) = keyword(
        value,
        &["initial", "inherit", "unset", "revert", "revert-layer"],
    ) {
        return Some(wide);
    }
    if name == "border-image" {
        return expand(value).map(|v| summary(&v.into_iter().collect()));
    }
    let index = AXES.iter().position(|name2| *name2 == name)?;
    let mut parts = parts(value)?;
    if index == 0 {
        return source(value).then(|| serialize(&tokenize(value)));
    }
    if index == 4 {
        if !(1..=2).contains(&parts.len()) {
            return None;
        }
        return parts
            .into_iter()
            .map(|v| keyword(&v, &["stretch", "repeat", "round", "space"]))
            .collect::<Option<Vec<_>>>()
            .map(compress);
    }
    let mut fill = false;
    if index == 1 {
        if parts
            .first()
            .is_some_and(|part| part.eq_ignore_ascii_case("fill"))
        {
            fill = true;
            parts.remove(0);
        } else if parts
            .last()
            .is_some_and(|part| part.eq_ignore_ascii_case("fill"))
        {
            fill = true;
            parts.pop();
        }
    }
    if !(1..=4).contains(&parts.len()) || !parts.iter().all(|part| numeric(part, index)) {
        return None;
    }
    let mut css = compress(
        parts
            .into_iter()
            .map(|part| serialize(&tokenize(&part)))
            .collect(),
    );
    if fill {
        css.push_str(" fill");
    }
    Some(css)
}
pub(crate) fn expand(value: &str) -> Option<Vec<(String, String)>> {
    if let Some(wide) = keyword(
        value,
        &["initial", "inherit", "unset", "revert", "revert-layer"],
    ) {
        return Some(
            AXES.iter()
                .map(|name| ((*name).into(), wide.clone()))
                .collect(),
        );
    }
    let parts = parts(value)?;
    if parts.is_empty() {
        return None;
    }
    let mut values: [Option<String>; 5] = std::array::from_fn(|_| None);
    let mut numeric_group = Vec::new();
    let mut repeat_group = Vec::new();
    let mut current_group = None;
    let mut closed_groups = 0u8;
    for part in parts {
        let group = if source(&part) {
            0
        } else if keyword(&part, &["stretch", "repeat", "round", "space"]).is_some() {
            4
        } else {
            1
        };
        if current_group != Some(group) {
            if closed_groups & (1 << group) != 0 {
                return None;
            }
            if let Some(old) = current_group {
                closed_groups |= 1 << old;
            }
            current_group = Some(group);
        }
        match group {
            0 => {
                if values[0].replace(part).is_some() {
                    return None;
                }
            }
            4 => repeat_group.push(part),
            _ => numeric_group.push(part),
        }
    }
    if !repeat_group.is_empty() {
        values[4] = Some(canonical(AXES[4], &repeat_group.join(" "))?);
    }
    if !numeric_group.is_empty() {
        let sections: Vec<_> = numeric_group.split(|part| part == "/").collect();
        if sections.len() > 3 || sections[0].is_empty() {
            return None;
        }
        values[1] = Some(canonical(AXES[1], &sections[0].join(" "))?);
        if sections.len() > 1 {
            if !sections[1].is_empty() {
                values[2] = Some(canonical(AXES[2], &sections[1].join(" "))?);
            } else if sections.len() != 3 {
                return None;
            }
        }
        if sections.len() == 3 {
            values[3] = Some(canonical(AXES[3], &sections[2].join(" "))?);
        }
    }
    Some(
        AXES.iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    (*name).into(),
                    values[index]
                        .take()
                        .unwrap_or_else(|| INITIAL[index].into()),
                )
            })
            .collect(),
    )
}
fn summary(properties: &BTreeMap<String, String>) -> String {
    let v: Vec<_> = AXES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            properties
                .get(*name)
                .map(String::as_str)
                .unwrap_or(INITIAL[i])
        })
        .collect();
    format!("{} {} / {} / {} {}", v[0], v[1], v[2], v[3], v[4])
}
pub(crate) fn serialize_properties(properties: &mut BTreeMap<String, String>) {
    properties.insert("border-image".into(), summary(properties));
}

/// Resolve em/rem and absolute lengths and pure arithmetic at computed time.
/// Context-dependent font/container/viewport metrics remain explicit math.
pub(crate) fn resolve_properties(properties: &mut BTreeMap<String, String>, root: f64) {
    let font = properties
        .get("font-size")
        .and_then(|css| css.strip_suffix("px"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(16.0);
    for (index, name) in AXES
        .iter()
        .enumerate()
        .filter(|(index, _)| matches!(index, 1 | 2 | 3))
    {
        let Some(value) = properties.get(*name).cloned() else {
            continue;
        };
        let Some(parts) = parts(&value) else {
            continue;
        };
        let resolved=parts.into_iter().map(|part| {
            let tokens=tokenize(&part);
            if tokens.iter().any(|token|matches!(token,Token::Dimension(_,unit) if !matches!(unit.to_ascii_lowercase().as_str(),"px"|"in"|"cm"|"mm"|"q"|"pt"|"pc"|"em"|"rem"))) {return part;}
            let Some(css)=crate::computed::individual_transforms::signs(&part,Some((font,root))) else{return part;};
            if crate::math::number_percentage(&css,false) {
                return crate::values::alpha_value(&css).filter(|n|n.is_finite()).map(|n|n.max(0.0).to_string()).unwrap_or(part);
            }
            if matches!(index,1|2) && crate::math::percentage(&css) {
                let numbers=tokenize(&css).into_iter().map(|token|match token {Token::Percentage(n)=>Token::Number(n),token=>token}).collect::<Vec<_>>();
                return crate::values::alpha_value(&serialize(&numbers)).filter(|n|n.is_finite()).map(|n|format!("{}%",n.max(0.0))).unwrap_or(part);
            }
            if matches!(index,2|3) {
                let px=if let Some(length)=crate::values::LengthPercentage::parse(&css) {
                    let mut typed=crate::values::ComputedValue::LengthPercentage(length);
                    crate::values::resolve_font_units(&mut typed,font,root,false);
                    match typed {crate::values::ComputedValue::LengthPercentage(crate::values::LengthPercentage::Px(px))=>Some(px),_=>None}
                }else if crate::math::length_percentage(&css,false){crate::math::font_size_px(&css,font,root)}else{None};
                if let Some(px)=px {return format!("{}px",px.max(0.0));}
            }
            part
        }).collect::<Vec<_>>().join(" ");
        if let Some(css) = canonical(name, &resolved) {
            properties.insert((*name).into(), css);
        }
    }
    serialize_properties(properties);
}
