//! CSS Fonts 4 font shorthand grammar, reset axes and CSSOM serialization.
use crate::computed::ComputedStyle;
use crate::syntax::{serialize, tokenize, Token};

pub(crate) const LONGHANDS: &[&str] = &[
    "font-family",
    "font-feature-settings",
    "font-size",
    "font-size-adjust",
    "font-stretch",
    "font-style",
    "font-variant",
    "font-variant-ligatures",
    "font-weight",
    "font-kerning",
    "font-language-override",
    "font-optical-sizing",
    "font-variant-alternates",
    "font-variant-caps",
    "font-variant-east-asian",
    "font-variant-emoji",
    "font-variant-numeric",
    "font-variant-position",
    "font-variation-settings",
    "line-height",
];
const WIDTHS: &[(&str, &str)] = &[
    ("ultra-condensed", "50%"),
    ("extra-condensed", "62.5%"),
    ("condensed", "75%"),
    ("semi-condensed", "87.5%"),
    ("normal", "100%"),
    ("semi-expanded", "112.5%"),
    ("expanded", "125%"),
    ("extra-expanded", "150%"),
    ("ultra-expanded", "200%"),
];
fn skip(t: &[Token], p: &mut usize) {
    while matches!(t.get(*p), Some(Token::Whitespace)) {
        *p += 1;
    }
}
/// Consume a complete component, including nested math functions.
fn component(t: &[Token], p: &mut usize) -> Option<String> {
    skip(t, p);
    let start = *p;
    let first = t.get(*p)?;
    *p += 1;
    if matches!(first, Token::Function(_)) {
        if !matches!(t.get(*p), Some(Token::OpenParen)) {
            return None;
        }
        let mut depth = 0;
        loop {
            match t.get(*p)? {
                Token::OpenParen => depth += 1,
                Token::CloseParen => {
                    depth -= 1;
                    if depth == 0 {
                        *p += 1;
                        break;
                    }
                }
                _ => {}
            };
            *p += 1;
        }
    }
    Some(serialize(&t[start..*p]))
}
fn length(css: &str) -> bool {
    crate::values::LengthPercentage::parse(css).is_some_and(|v| v.is_nonnegative())
        || crate::math::length_percentage(css, true)
}
fn size(css: &str) -> bool {
    length(css)
        || matches!(
            css.to_ascii_lowercase().as_str(),
            "xx-small"
                | "x-small"
                | "small"
                | "medium"
                | "large"
                | "x-large"
                | "xx-large"
                | "xxx-large"
                | "smaller"
                | "larger"
                | "math"
        )
}
fn line(css: &str) -> bool {
    length(css)
        || css.eq_ignore_ascii_case("normal")
        || matches!(tokenize(css).as_slice(),[Token::Number(n)] if n.is_finite()&&*n>=0.0)
        || crate::math::number_percentage(css, false)
}
fn family(t: &[Token]) -> bool {
    let mut names = 0;
    let mut quoted = false;
    let mut generic = false;
    for token in t {
        match token {
            Token::Whitespace => {}
            Token::Comma if names > 0 => {
                names = 0;
                quoted = false;
                generic = false;
            }
            Token::Str(_) if names == 0 => {
                names = 1;
                quoted = true;
            }
            Token::Ident(s)
                if !quoted
                    && !generic
                    && !matches!(
                        s.to_ascii_lowercase().as_str(),
                        "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "default"
                    ) =>
            {
                let is_generic = matches!(
                    s.to_ascii_lowercase().as_str(),
                    "serif"
                        | "sans-serif"
                        | "cursive"
                        | "fantasy"
                        | "monospace"
                        | "system-ui"
                        | "ui-serif"
                        | "ui-sans-serif"
                        | "ui-monospace"
                        | "ui-rounded"
                        | "emoji"
                        | "math"
                        | "fangsong"
                );
                if is_generic && names > 0 {
                    return false;
                }
                generic = is_generic;
                names += 1;
            }
            _ => return false,
        }
    }
    names > 0
}

fn initial(name: &str) -> &'static str {
    match name {
        "font-family" => "serif",
        "font-size" => "medium",
        "font-size-adjust" => "none",
        "font-kerning" | "font-optical-sizing" => "auto",
        _ => "normal",
    }
}
pub(crate) fn expand(css: &str) -> Option<Vec<(String, String)>> {
    let css = css.trim();
    let lower = css.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    ) {
        return Some(
            LONGHANDS
                .iter()
                .map(|n| (n.to_string(), lower.clone()))
                .collect(),
        );
    }
    // No OS preference query is installed: use the permitted UA default font.
    if matches!(
        lower.as_str(),
        "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar"
    ) {
        return Some(
            LONGHANDS
                .iter()
                .map(|n| (n.to_string(), initial(n).to_owned()))
                .collect(),
        );
    }
    let t = tokenize(css);
    let mut p = 0;
    let mut slots: [Option<String>; 4] = std::array::from_fn(|_| None);
    let mut normals = 0;
    let font_size = loop {
        let value = component(&t, &mut p)?;
        let lower = value.to_ascii_lowercase();
        if size(&value) {
            break value;
        }
        if lower == "normal" {
            normals += 1;
            if normals + slots.iter().flatten().count() > 4 {
                return None;
            }
            continue;
        }
        let (index, value) = if matches!(lower.as_str(), "italic" | "oblique") {
            let mut value = lower.clone();
            if lower == "oblique" {
                let mut next = p;
                if let Some(angle) = component(&t, &mut next) {
                    if crate::math::angle(&angle) {
                        if let Some(a) = crate::values::Angle::parse(&angle) {
                            if a.radians.abs() > std::f64::consts::FRAC_PI_2 {
                                return None;
                            }
                        }
                        value.push(' ');
                        value.push_str(&angle);
                        p = next;
                    }
                }
            };
            (0, value)
        } else if lower == "small-caps" {
            (1, lower)
        } else if matches!(lower.as_str(), "bold" | "bolder" | "lighter")
            || (matches!(tokenize(&value).as_slice(),[Token::Number(n)] if n.is_finite()&&(1.0..=1000.0).contains(n)))
            || crate::math::number_percentage(&value, false)
        {
            (2, lower)
        } else if WIDTHS.iter().any(|(n, _)| *n == lower) {
            (3, lower)
        } else {
            return None;
        };
        if slots[index].is_some() {
            return None;
        }
        slots[index] = Some(value);
        if normals + slots.iter().flatten().count() > 4 {
            return None;
        }
    };
    skip(&t, &mut p);
    let line_height = if matches!(t.get(p), Some(Token::Delim('/'))) {
        p += 1;
        let v = component(&t, &mut p)?;
        if !line(&v) {
            return None;
        }
        v
    } else {
        "normal".into()
    };
    skip(&t, &mut p);
    let families = t.get(p..)?;
    if !family(families) {
        return None;
    }
    let mut out: Vec<_> = LONGHANDS
        .iter()
        .map(|n| (n.to_string(), initial(n).to_string()))
        .collect();
    for (name, value) in [
        ("font-style", slots[0].clone().unwrap_or("normal".into())),
        ("font-variant", slots[1].clone().unwrap_or("normal".into())),
        (
            "font-variant-caps",
            slots[1].clone().unwrap_or("normal".into()),
        ),
        ("font-weight", slots[2].clone().unwrap_or("normal".into())),
        ("font-stretch", slots[3].clone().unwrap_or("normal".into())),
        ("font-size", font_size),
        ("line-height", line_height),
        ("font-family", serialize(families)),
    ] {
        out.iter_mut().find(|(n, _)| n == name)?.1 = value;
    }
    Some(out)
}
/// Return a font shorthand only if every reset-only axis is representable.
pub(crate) fn computed(style: &ComputedStyle) -> Option<String> {
    for name in LONGHANDS {
        if matches!(
            *name,
            "font-family"
                | "font-size"
                | "font-style"
                | "font-weight"
                | "font-stretch"
                | "line-height"
                | "font-variant"
                | "font-variant-caps"
        ) {
            continue;
        }
        if style.get(name)? != initial(name) {
            return None;
        }
    }
    let caps = style.get("font-variant-caps")?;
    if !matches!(caps, "normal" | "small-caps") {
        return None;
    }
    let stretch = style.get("font-stretch")?;
    let width = WIDTHS
        .iter()
        .find(|(n, p)| stretch == *n || stretch == *p)?
        .0;
    let weight = match style.get("font-weight")? {
        "normal" => "400",
        "bold" => "700",
        "bolder" | "lighter" => return None,
        other => other,
    };
    let mut values = Vec::new();
    for value in [style.get("font-style")?, caps, weight, width] {
        if value != "normal" && value != "400" {
            values.push(value.to_string());
        }
    }
    let mut size = style.get("font-size")?.to_owned();
    let height = style.get("line-height")?;
    if height != "normal" {
        size.push_str(" / ");
        size.push_str(height);
    }
    values.push(size);
    values.push(style.get("font-family")?.to_owned());
    let result = values.join(" ");
    expand(&result).map(|_| result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let n = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))])
            .get(n)
            .unwrap()
            .clone()
    }
    #[test]
    fn grammar_covers_optional_categories_math_and_family_boundaries() {
        for css in [
            "16px serif",
            "italic small-caps bold condensed 12pt/1.5 Times New Roman, serif",
            "normal normal normal normal 20%/1.2 \"FB Armada\"",
            "oblique 25deg 753.5 calc(10px + 1em)/calc(120% + 1.2em) fantasy",
            "menu",
            "large menu",
            "0/0 serif",
        ] {
            assert!(expand(css).is_some(), "{css}");
            assert!(crate::values::parses("font", css), "{css}");
        }
        for css in [
            "menu icon",
            "italic oblique xx-small serif",
            "small-caps small-caps medium/normal sans-serif",
            "bold bolder xx-large/1.2 cursive",
            "ultra-condensed extra-condensed larger/calc(120% + 1.2em) fantasy",
            "normal normal normal normal normal 20% serif",
            "italic small-caps lighter condensed smaller",
            "100% larger / 2 fantasy",
            "oldstyle-nums medium serif",
            "20deg serif",
            "12bogus serif",
            "-1px serif",
            "16px/1deg serif",
            "16px serif,",
            "16px \"A\" B",
            "16px inherit",
            "oblique 91deg 16px serif",
        ] {
            assert!(expand(css).is_none(), "{css}");
            assert!(!crate::values::parses("font", css), "{css}");
        }
    }
    #[test]
    fn cascade_rejection_substitution_resets_and_inheritance_are_atomic() {
        let s = style("font:italic small-caps bold 20px serif;font:italic oblique 10px sans-serif");
        assert_eq!(s.get("font-size"), Some("20px"));
        assert_eq!(s.get("font-variant-caps"), Some("small-caps"));
        let s=style("font-variant-numeric:oldstyle-nums;font-size-adjust:0.5;font-synthesis:none;font-palette:dark;--f:oblique 12deg 550.5 20px/2 serif;font:var(--f)");
        assert_eq!(s.get("font-variant-numeric"), Some("normal"));
        assert_eq!(s.get("font-size-adjust"), Some("none"));
        assert_eq!(s.get("font-synthesis-style"), Some("none"));
        assert_eq!(s.get("font-palette"), Some("dark"));
        assert_eq!(s.get("font-size"), Some("20px"));
        let s = style(
            "font-weight:bold;font-size:20px;--bad:italic oblique 10px serif;font:var(--bad)",
        );
        assert_eq!(s.get("font-size"), Some("16px"));
    }
    #[test]
    fn cssom_uses_actual_axes_and_refuses_unrepresentable_reset_values() {
        let s = style("font:italic small-caps 550 condensed 20px/2 serif");
        assert_eq!(
            s.cssom_property_value("font"),
            Some("italic small-caps 550 condensed 20px / 2 serif".into())
        );
        let s = style("font:20px serif;font-size-adjust:0.5");
        assert_eq!(s.cssom_property_value("font"), None);
        let s = style("font:20px serif;font-variant-caps:petite-caps");
        assert_eq!(s.cssom_property_value("font"), None);
        let s = style("font:20px serif;font-stretch:80%");
        assert_eq!(s.cssom_property_value("font"), None);
        let s = style("font:20px serif;font-weight:700");
        assert_eq!(
            s.cssom_property_value("font"),
            Some("700 20px serif".into())
        );
    }
}
