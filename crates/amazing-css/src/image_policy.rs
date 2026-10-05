//! CSS Images property-value producers. Raster scaling and object geometry
//! remain owned by the image, layout and paint consumers.
use crate::syntax::{Token, tokenize};

pub(crate) fn supported(property: &str) -> bool {
    matches!(property, "object-fit" | "image-rendering")
}
pub(crate) fn canonical(property: &str, css: &str) -> Option<String> {
    let words = tokenize(css).into_iter().filter(|t| !matches!(t, Token::Whitespace))
        .map(|t| match t { Token::Ident(s) => Some(s.to_ascii_lowercase()), _ => None })
        .collect::<Option<Vec<_>>>()?;
    let words: Vec<_> = words.iter().map(String::as_str).collect();
    if let [word] = words.as_slice() {
        if matches!(*word, "initial" | "inherit" | "unset" | "revert" | "revert-layer") { return Some((*word).into()); }
    }
    match (property, words.as_slice()) {
        ("image-rendering", [word]) if matches!(*word, "auto" | "smooth" | "high-quality" | "crisp-edges" | "pixelated") => Some((*word).into()),
        ("object-fit", [word]) if matches!(*word, "fill" | "none" | "contain" | "cover" | "scale-down") => Some((*word).into()),
        ("object-fit", [fit, "scale-down"] | ["scale-down", fit]) if matches!(*fit, "contain" | "cover") => Some(format!("{fit} scale-down")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> crate::ComputedStyle {
        let mut dom=amazing_dom::Dom::new();let node=dom.create_element(amazing_dom::NodeIdx(0),"div");
        crate::compute(&dom,&[crate::parse_stylesheet(&format!("div{{{css}}}"))]).get(node).unwrap().clone()
    }
    #[test]
    fn every_object_fit_alternative_and_flag_order_canonicalizes() {
        for value in ["fill","none","contain","cover","scale-down"] {
            assert_eq!(canonical("object-fit",value).as_deref(),Some(value));
            assert_eq!(style(&format!("object-fit:{value}")).get("object-fit"),Some(value));
        }
        for fit in ["contain","cover"] {
            for value in [format!("{fit} scale-down"),format!("scale-down {fit}"),format!("SCALE-DOWN /**/ {}",fit.to_uppercase())] {
                let expected=format!("{fit} scale-down");assert_eq!(canonical("object-fit",&value).as_deref(),Some(expected.as_str()));
                assert_eq!(style(&format!("object-fit:{value}")).get("object-fit"),Some(expected.as_str()));
            }
        }
        assert_eq!(canonical("object-fit","co\\76 er scale-down").as_deref(),Some("cover scale-down"));
    }
    #[test]
    fn every_rendering_keyword_is_an_inherited_canonical_singleton() {
        for value in ["auto","smooth","high-quality","crisp-edges","pixelated"] {
            assert_eq!(canonical("image-rendering",value).as_deref(),Some(value));
            assert_eq!(style(&format!("image-rendering:{}",value.to_uppercase())).get("image-rendering"),Some(value));
        }
        let mut dom=amazing_dom::Dom::new();let parent=dom.create_element(amazing_dom::NodeIdx(0),"section");let child=dom.create_element(parent,"div");
        let styles=crate::compute(&dom,&[crate::parse_stylesheet("section{image-rendering:pixelated;object-fit:cover scale-down}")]);
        assert_eq!(styles.get(child).unwrap().get("image-rendering"),Some("pixelated"));assert_eq!(styles.get(child).unwrap().get("object-fit"),Some("fill"));
    }
    #[test]
    fn malformed_and_duplicate_components_retain_prior_declarations_atomically() {
        for property in ["object-fit","image-rendering"] {
            let valid=if property=="object-fit" {"cover"} else {"smooth"};
            for invalid in ["","invalid","1","30deg","'cover'","url(a)","fill none","fill scale-down","none scale-down","cover contain","cover cover","scale-down scale-down","cover scale-down scale-down","initial cover","smooth pixelated","high-quality crisp-edges","none cover"] {
                assert_eq!(canonical(property,invalid),None,"{property}:{invalid}");assert!(!crate::values::parses(property,invalid));
                assert_eq!(style(&format!("{property}:{valid};{property}:{invalid}")).get(property),Some(valid));
            }
        }
        assert!(!crate::values::parses("image-rendering","none"));
    }
    #[test]
    fn actual_substitution_css_wide_reset_importance_and_change_classification() {
        assert_eq!(style("--fit:scale-down cover;object-fit:var(--fit)").get("object-fit"),Some("cover scale-down"));
        assert_eq!(style("--kind:pixelated;image-rendering:var(--kind)").get("image-rendering"),Some("pixelated"));
        assert_eq!(style("object-fit:cover;object-fit:var(--absent)").get("object-fit"),Some("fill"));
        assert_eq!(style("image-rendering:pixelated;image-rendering:var(--absent)").get("image-rendering"),Some("auto"));
        assert_eq!(style("object-fit:cover!important;object-fit:none").get("object-fit"),Some("cover"));
        assert_eq!(style("image-rendering:pixelated;object-fit:cover;all:initial").get("image-rendering"),Some("auto"));
        assert_eq!(style("object-fit:attr(missing type(<custom-ident>), cover) scale-down").get("object-fit"),Some("cover scale-down"));
        let mut dom=amazing_dom::Dom::new();let target=dom.create_element(amazing_dom::NodeIdx(0),"div");
        let styles=crate::compute(&dom,&[crate::parse_stylesheet("@supports (image-rendering:pixelated) and (object-fit:scale-down cover){div{image-rendering:pixelated;object-fit:cover scale-down}}")]);
        assert_eq!(styles.get(target).unwrap().get("image-rendering"),Some("pixelated"));assert_eq!(styles.get(target).unwrap().get("object-fit"),Some("cover scale-down"));
        for property in ["object-fit","image-rendering"] {
            assert_eq!(crate::classify_style_property(property),Some(crate::StyleChangeKind::PaintOnly));
            for keyword in ["inherit","initial","unset","revert","revert-layer"] {assert!(crate::values::parses(property,keyword));}
        }
    }
}
