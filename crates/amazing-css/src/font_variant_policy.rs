//! Deferred font-variant expansion after custom-property substitution.
use super::*;
pub(super) fn expand(value: &str) -> Option<Vec<(String, String)>> {
    if contains_var_function(value) {
        return Some(
            crate::font_variant::AXES
                .iter()
                .map(|axis| {
                    (
                        (*axis).into(),
                        deferred("amazing-internal-font-variant", value),
                    )
                })
                .collect(),
        );
    }
    crate::font_variant::expand(value)
}
pub(super) fn resolve_deferred(property: &str, value: &str) -> Option<Result<String, ()>> {
    let inner = deferred_inner(value, "amazing-internal-font-variant")?;
    Some(
        crate::font_variant::expand(inner)
            .and_then(|values| {
                values
                    .into_iter()
                    .find_map(|(name, value)| (name == property).then_some(value))
            })
            .ok_or(()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(
            &dom,
            &[crate::parse_stylesheet(&format!("div {{ {css} }}"))],
        )
        .get(div)
        .unwrap()
        .clone()
    }
    #[test]
    fn whole_feature_family_rejects_conflicts_and_distributes_axes() {
        for value in [
            "",
            "none small-caps",
            "normal sub",
            "small-caps all-small-caps",
            "sub super",
            "text emoji",
            "lining-nums oldstyle-nums",
            "jis78 jis90",
            "contextual no-contextual",
            "stylistic(foo) stylistic(bar)",
            "styleset()",
            "swash(foo, bar)",
            "annotation(12)",
            "historical-forms historical-forms",
            "styleset(foo,) ",
            "banana",
        ] {
            assert!(!values::parses("font-variant", value), "{value}");
        }
        for value in [
            "stylistic(inherit)",
            "styleset(default)",
            "character-variant(foo, unset)",
        ] {
            assert!(!values::parses("font-variant", value), "{value}");
        }
        assert_eq!(style("font-variant-numeric:slashed-zero ordinal tabular-nums oldstyle-nums; font-variant-east-asian:ruby full-width jis78; font-variant-ligatures:no-contextual common-ligatures; font-variant-alternates:annotation(Note) historical-forms stylistic(Foo)").get("font-variant"), Some("common-ligatures no-contextual stylistic(Foo) historical-forms annotation(Note) oldstyle-nums tabular-nums ordinal slashed-zero jis78 full-width ruby"));
        let css = "font-variant:emoji super ruby tabular-nums historical-forms styleset(Foo, Bar) small-caps no-contextual";
        let s = style(css);
        for (axis, expected) in [
            ("ligatures", "no-contextual"),
            ("caps", "small-caps"),
            ("alternates", "historical-forms styleset(Foo, Bar)"),
            ("numeric", "tabular-nums"),
            ("east-asian", "ruby"),
            ("position", "super"),
            ("emoji", "emoji"),
        ] {
            assert_eq!(s.get(&format!("font-variant-{axis}")), Some(expected));
        }
        assert_eq!(
            s.get("font-variant"),
            Some(
                "no-contextual small-caps historical-forms styleset(Foo, Bar) tabular-nums ruby super emoji"
            )
        );
    }
    #[test]
    fn independent_winners_reset_and_reconstruct_a_reparseable_summary() {
        let mut s = style(
            "font-feature-settings:'test' 2; font-language-override:'TRK'; font-variation-settings:'wght' 500; font-variant:none; font-variant-caps:small-caps",
        );
        let summary = s.get("font-variant").unwrap().to_owned();
        assert_eq!(
            summary,
            "no-common-ligatures no-discretionary-ligatures no-historical-ligatures no-contextual small-caps"
        );
        assert!(values::parses("font-variant", &summary));
        assert_eq!(
            style(&format!("font-variant:{summary}")).get("font-variant"),
            Some(summary.as_str())
        );
        assert_eq!(s.get("font-feature-settings"), Some("\"test\" 2"));
        assert_eq!(s.get("font-language-override"), Some("\"TRK\""));
        assert_eq!(s.get("font-variation-settings"), Some("\"wght\" 500"));
        s.set("font-variant", "normal");
        assert_eq!(s.get("font-variant"), Some("normal"));
        for axis in crate::font_variant::AXES {
            assert_eq!(s.get(axis), Some("normal"));
        }
        assert_eq!(
            style("font-variant:super ruby; font:small-caps 20px serif").get("font-variant"),
            Some("small-caps")
        );
    }
    #[test]
    fn deferred_variables_and_inheritance_keep_independent_axes() {
        let mut dom = amazing_dom::Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let span = dom.create_element(div, "span");
        let sheets = [crate::parse_stylesheet(
            "div {font-variant:small-caps ruby super} span { --fv:tabular-nums emoji; font-variant:var(--fv); font-variant-caps:inherit }",
        )];
        let styles = crate::compute(&dom, &sheets);
        assert_eq!(
            styles.get(span).unwrap().get("font-variant"),
            Some("small-caps tabular-nums emoji")
        );
        let styles = crate::compute(
            &dom,
            &[crate::parse_stylesheet(
                "div {font-variant:small-caps ruby super} span {font-variant:none; font-variant:var(--missing); font-variant-position:normal}",
            )],
        );
        assert_eq!(
            styles.get(span).unwrap().get("font-variant"),
            Some("small-caps ruby")
        );
        let s = style(
            "--fv:small-caps all-small-caps; font-variant:var(--fv); font-variant-numeric:ordinal",
        );
        assert_eq!(s.get("font-variant"), Some("ordinal"));
    }
}
