//! CSS Containment's complete keyword grammar and canonical serialization.
//! Used containment effects remain owned by Layout; no geometry is inferred here.
use crate::syntax::{Token, tokenize};

fn bits(css: &str) -> Option<u8> {
    let mut result = 0;
    for token in tokenize(css) {
        let name = match token {
            Token::Whitespace => continue,
            Token::Ident(name) => name.to_ascii_lowercase(),
            _ => return None,
        };
        let bit = match name.as_str() {
            "size" => 1,
            "inline-size" => 2,
            "layout" => 4,
            "style" => 8,
            "paint" => 16,
            _ => return None,
        };
        if result & bit != 0 {
            return None;
        }
        result |= bit;
    }
    (result != 0 && result & 3 != 3).then_some(result)
}

fn singleton(css: &str) -> Option<String> {
    let tokens = tokenize(css);
    let mut tokens = tokens.iter().filter(|t| !matches!(t, Token::Whitespace));
    let Token::Ident(name) = tokens.next()? else {
        return None;
    };
    tokens.next().is_none().then(|| name.to_ascii_lowercase())
}

pub(crate) fn specified(css: &str) -> Option<String> {
    if let Some(name) = singleton(css) {
        if matches!(
            name.as_str(),
            "none"
                | "strict"
                | "content"
                | "inherit"
                | "initial"
                | "unset"
                | "revert"
                | "revert-layer"
        ) {
            return Some(name);
        }
    }
    let bits = bits(css)?;
    Some(
        [
            (1, "size"),
            (2, "inline-size"),
            (4, "layout"),
            (8, "style"),
            (16, "paint"),
        ]
        .iter()
        .filter_map(|(bit, name)| (bits & bit != 0).then_some(*name))
        .collect::<Vec<_>>()
        .join(" "),
    )
}

pub(crate) fn computed(css: &str) -> Option<String> {
    let canonical = specified(css)?;
    // Canonical CSSOM serialization uses the equivalent compact keyword.
    // inline-size never becomes strict: it does not imply block-size containment.
    Some(match bits(&canonical) {
        Some(29) => "strict".into(),
        Some(28) => "content".into(),
        _ => canonical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> crate::ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let node = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))])
            .get(node)
            .unwrap()
            .clone()
    }
    fn permutations(values: &mut [&str], at: usize, output: &mut Vec<String>) {
        if at == values.len() {
            output.push(values.join(" "));
            return;
        }
        for i in at..values.len() {
            values.swap(at, i);
            permutations(values, at + 1, output);
            values.swap(at, i);
        }
    }
    #[test]
    fn every_valid_combination_and_order_has_one_canonical_value() {
        for mask in 1u8..32 {
            if mask & 3 == 3 {
                continue;
            }
            let mut names = [
                (1, "size"),
                (2, "inline-size"),
                (4, "layout"),
                (8, "style"),
                (16, "paint"),
            ]
            .iter()
            .filter_map(|(bit, name)| (mask & bit != 0).then_some(*name))
            .collect::<Vec<_>>();
            let canonical = names.join(" ");
            let mut inputs = Vec::new();
            permutations(&mut names, 0, &mut inputs);
            for input in inputs {
                assert_eq!(
                    specified(&input).as_deref(),
                    Some(canonical.as_str()),
                    "{input}"
                );
                assert!(crate::values::parses("contain", &input));
                let expected = match mask {
                    29 => "strict",
                    28 => "content",
                    _ => canonical.as_str(),
                };
                assert_eq!(
                    style(&format!("contain:{input}")).get("contain"),
                    Some(expected)
                );
            }
        }
    }
    #[test]
    fn invalid_keywords_duplicates_exclusivity_and_tokens_reject_atomically() {
        for input in [
            "",
            "auto",
            "inline-size inline-size",
            "layout layout",
            "none none",
            "none strict",
            "paint content",
            "paint layout style paint",
            "paint strict",
            "size inline-size",
            "size layout size",
            "strict content",
            "strict layout",
            "strict none",
            "strict strict",
            "style style",
            "paint paint",
            "none size",
            "layout,paint",
            "'paint'",
            "1",
            "paint()",
            "paint / size",
        ] {
            assert_eq!(specified(input), None, "{input}");
            assert!(!crate::values::parses("contain", input), "{input}");
            assert_eq!(
                style(&format!("contain:layout;contain:{input}")).get("contain"),
                Some("layout"),
                "{input}"
            );
        }
    }
    #[test]
    fn escaped_identifiers_comments_case_and_aliases_preserve_both_serialization_stages() {
        for input in ["PAINT /**/ LaYoUt STYLE", "\\70 aint layout style"] {
            assert_eq!(specified(input).as_deref(), Some("layout style paint"));
            assert_eq!(computed(input).as_deref(), Some("content"));
        }
        for input in ["size paint layout style", "STRICT"] {
            assert_eq!(computed(input).as_deref(), Some("strict"));
        }
        assert_eq!(
            computed("paint style layout inline-size").as_deref(),
            Some("inline-size layout style paint")
        );
        assert_eq!(specified("none/**/none"), None);
    }
    #[test]
    fn actual_cascade_custom_property_inheritance_and_css_wide_values_are_complete() {
        let mut dom = amazing_dom::Dom::new();
        let parent = dom.create_element(amazing_dom::NodeIdx(0), "section");
        let a = dom.create_element(parent, "div");
        let b = dom.create_element(parent, "span");
        let sheet = crate::parse_stylesheet(
            "section{contain:strict;--contain:paint layout style}div{contain:inherit}span{contain:var(--contain)}",
        );
        let styles = crate::compute(&dom, &[sheet]);
        assert_eq!(styles.get(parent).unwrap().get("contain"), Some("strict"));
        assert_eq!(styles.get(a).unwrap().get("contain"), Some("strict"));
        assert_eq!(styles.get(b).unwrap().get("contain"), Some("content"));
        let styles = crate::compute(
            &dom,
            &[crate::parse_stylesheet(
                "section{contain:strict}div{contain:unset}span{contain:initial}",
            )],
        );
        assert_eq!(styles.get(a).unwrap().get("contain"), Some("none"));
        assert_eq!(styles.get(b).unwrap().get("contain"), Some("none"));
        for css in [
            "contain:layout;contain:var(--missing)",
            "--bad:size inline-size;contain:layout;contain:var(--bad)",
        ] {
            assert_eq!(style(css).get("contain"), Some("none"), "{css}");
        }
        assert_eq!(
            style("--x:paint;contain:var(--missing,var(--x))").get("contain"),
            Some("paint")
        );
        assert_eq!(
            style("contain:paint!important;contain:layout").get("contain"),
            Some("paint")
        );
    }
    #[test]
    fn property_registry_cssom_and_all_reset_retain_the_real_default() {
        let s = style("contain:size style paint layout");
        assert_eq!(s.cssom_property_value("contain").as_deref(), Some("strict"));
        assert_eq!(
            style("contain:paint;all:initial").get("contain"),
            Some("none")
        );
        assert_eq!(style("").get("contain"), Some("none"));
        assert!(crate::computed::is_supported_property("contain"));
        assert_eq!(crate::classify_style_property("contain"), Some(crate::StyleChangeKind::Rebuild));
        for keyword in ["initial", "inherit", "unset", "revert", "revert-layer"] {
            assert_eq!(specified(keyword).as_deref(), Some(keyword));
            assert!(crate::values::parses("contain", keyword));
        }
    }
}
