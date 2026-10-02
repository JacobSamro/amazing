//! Deferred image-border shorthand and reset-only global border axes.
use super::*;
pub(super) fn expand(value: &str) -> Option<Vec<(String, String)>> {
    if contains_var_function(value) {
        return Some(
            crate::border_image::AXES
                .iter()
                .map(|name| {
                    (
                        (*name).into(),
                        deferred("amazing-internal-border-image", value),
                    )
                })
                .collect(),
        );
    }
    crate::border_image::expand(value)
}
pub(super) fn reset(value: &str) -> Option<Vec<(String, String)>> {
    if contains_var_function(value) {
        return Some(
            crate::border_image::AXES
                .iter()
                .map(|name| {
                    (
                        (*name).into(),
                        deferred("amazing-internal-border-image-reset", value),
                    )
                })
                .collect(),
        );
    }
    if css_wide_keyword(value).is_some() {
        return Some(
            crate::border_image::AXES
                .iter()
                .map(|name| ((*name).into(), value.into()))
                .collect(),
        );
    }
    expand_side_border("border-top", value)?;
    Some(
        crate::border_image::AXES
            .iter()
            .zip(crate::border_image::INITIAL)
            .map(|(name, value)| ((*name).into(), value.into()))
            .collect(),
    )
}
pub(super) fn resolve_deferred(property: &str, value: &str) -> Option<Result<String, ()>> {
    let (inner, resetting) =
        if let Some(inner) = deferred_inner(value, "amazing-internal-border-image") {
            (inner, false)
        } else if let Some(inner) = deferred_inner(value, "amazing-internal-border-image-reset") {
            (inner, true)
        } else {
            return None;
        };
    Some(
        (if resetting {
            reset(inner)
        } else {
            crate::border_image::expand(inner)
        })
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
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div {{{css}}}"))])
            .get(div)
            .unwrap()
            .clone()
    }
    #[test]
    fn shorthand_groups_are_atomic_and_all_longhands_validate_types() {
        for css in [
            "auto",
            "fill",
            "1 fill 2",
            "stretch 1 repeat",
            "1 url(foo) 2",
            "1/-2px",
            "1//auto",
            "1//1%",
            "1/none/1px",
            "1 2 3 4 5",
            "url(foo),none",
            "1///2",
        ] {
            assert!(!values::parses("border-image", css), "{css}");
        }
        for (name, css) in [
            ("slice", "-1"),
            ("slice", "1px"),
            ("width", "none"),
            ("width", "-1%"),
            ("outset", "auto"),
            ("outset", "1%"),
            ("repeat", "repeat repeat repeat"),
            ("source", "none,url(foo)"),
        ] {
            assert!(
                !values::parses(&format!("border-image-{name}"), css),
                "{name}:{css}"
            );
        }
        let s = style(
            "border-image:round space linear-gradient(red,blue) fill 10% 20 / auto 2 / 3px 4",
        );
        for (name, value) in [
            ("slice", "10% 20 fill"),
            ("width", "auto 2"),
            ("outset", "3px 4"),
            ("repeat", "round space"),
        ] {
            assert_eq!(s.get(&format!("border-image-{name}")), Some(value));
        }
        assert!(values::parses(
            "border-image",
            s.get("border-image").unwrap()
        ));
    }
    #[test]
    fn cascade_defaults_global_border_reset_and_deferred_values() {
        let s = style(
            "--image:10% / auto / 2 round; border-image:var(--image); border-image-width:3; border-left:1px solid red",
        );
        assert_eq!(s.get("border-image-width"), Some("3"));
        assert_eq!(s.get("border-image-slice"), Some("10%"));
        let s = style(
            "border-image:12 fill / 3 / 4 repeat; --border:2px solid blue; border:var(--border)",
        );
        for (name, value) in crate::border_image::AXES
            .iter()
            .zip(crate::border_image::INITIAL)
        {
            assert_eq!(s.get(name), Some(value));
        }
        let s = style(
            "border-image:12 fill; --bad:1 / / auto; border-image:var(--bad); border-image-repeat:round",
        );
        assert_eq!(s.get("border-image-slice"), Some("100%"));
        assert_eq!(s.get("border-image-repeat"), Some("round"));
        let s = style(
            "font-size:40px; border-image-width:calc(.5em + 10px) calc(-.5em + 10px) 2; border-image-slice:calc(2 + 3) calc(20% - 5%) fill",
        );
        assert_eq!(s.get("border-image-width"), Some("30px 0px 2"));
        assert_eq!(s.get("border-image-slice"), Some("5 15% fill"));
    }
    #[test]
    fn explicit_inheritance_preserves_url_provenance_and_independent_axes() {
        let mut dom = amazing_dom::Dom::new();
        let parent = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let child = dom.create_element(parent, "span");
        let mut sheet =
            crate::parse_stylesheet("div{border-image:url(frame.png) 20% fill / 2 / 3px repeat}");
        sheet.base_url = Some("https://example.com/theme/main.css".into());
        let mut child_sheet = crate::parse_stylesheet(
            "span{--inherit:var(--missing,inherit);border-image:var(--inherit);border-image-width:4}",
        );
        child_sheet.base_url = Some("https://different.example/child.css".into());
        let styles = crate::compute(&dom, &[sheet, child_sheet]);
        let s = styles.get(child).unwrap();
        assert_eq!(s.get("border-image-source"), Some("url(frame.png)"));
        assert_eq!(
            s.url_base("border-image-source"),
            Some("https://example.com/theme/main.css")
        );
        assert_eq!(s.get("border-image-width"), Some("4"));
        assert_eq!(s.get("border-image-slice"), Some("20% fill"));
        assert_eq!(
            crate::classify_style_property("border-image-slice"),
            Some(crate::StyleChangeKind::PaintOnly)
        );
    }
}
