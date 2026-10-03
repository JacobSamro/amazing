//! CSSOM computed declaration enumeration and lower-owned serialization.
use super::*;
const LOGICAL_BORDER_LONGHANDS: [&str; 12] = [
    "border-inline-start-width",
    "border-inline-start-style",
    "border-inline-start-color",
    "border-inline-end-width",
    "border-inline-end-style",
    "border-inline-end-color",
    "border-block-start-width",
    "border-block-start-style",
    "border-block-start-color",
    "border-block-end-width",
    "border-block-end-style",
    "border-block-end-color",
];

fn alias(name: &str) -> bool {
    matches!(
        name,
        "border-image"
            | "background-position"
            | "font-variant"
            | "overflow"
            | "white-space"
            | "mask"
            | "text-decoration"
    )
}
/// Supported computed longhands, excluding internal derived shorthand aliases.
pub fn computed_longhand_names() -> impl Iterator<Item = &'static str> {
    PROPERTIES
        .iter()
        .map(|p| p.name)
        .chain(LOGICAL_BORDER_LONGHANDS)
        .filter(|name| !alias(name))
}
fn quad(mut values: Vec<String>) -> String {
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
fn serializable_shorthand(name: &str) -> bool {
    matches!(
        name,
        "margin"
            | "padding"
            | "inset"
            | "border-width"
            | "border-style"
            | "border-color"
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-inline-start"
            | "border-inline-end"
            | "border-block-start"
            | "border-block-end"
            | "margin-inline"
            | "margin-block"
            | "padding-inline"
            | "padding-block"
            | "inset-inline"
            | "inset-block"
            | "border-inline"
            | "border-block"
            | "border-inline-width"
            | "border-inline-style"
            | "border-inline-color"
            | "border-block-width"
            | "border-block-style"
            | "border-block-color"
            | "overflow"
            | "border-image"
            | "font-variant"
            | "font-synthesis"
            | "white-space"
            | "text-wrap"
            | "text-spacing"
            | "list-style"
            | "background-position"
    )
}
const SHORTHANDS: &[&str] = &[
    "animation",
    "background",
    "background-position",
    "border",
    "border-block",
    "border-block-color",
    "border-block-end",
    "border-block-start",
    "border-block-style",
    "border-block-width",
    "border-bottom",
    "border-color",
    "border-image",
    "border-inline",
    "border-inline-color",
    "border-inline-end",
    "border-inline-start",
    "border-inline-style",
    "border-inline-width",
    "border-left",
    "border-radius",
    "border-right",
    "border-style",
    "border-top",
    "border-width",
    "contain-intrinsic-size",
    "flex",
    "flex-flow",
    "font",
    "font-synthesis",
    "font-variant",
    "gap",
    "grid",
    "grid-area",
    "grid-column",
    "grid-row",
    "grid-template",
    "inset",
    "inset-block",
    "inset-inline",
    "list-style",
    "margin",
    "margin-block",
    "margin-inline",
    "outline",
    "overflow",
    "padding",
    "padding-block",
    "padding-inline",
    "place-content",
    "place-items",
    "place-self",
    "text-spacing",
    "text-wrap",
    "transition",
    "white-space",
];
/// Recognized shorthand names, using the core cascade expansion registry.
/// Serialization can still be unrepresentable or outside the implemented seam.
pub fn cssom_shorthand_names() -> impl Iterator<Item = &'static str> {
    SHORTHANDS.iter().copied()
}
/// Supported ordinary property names for CSSOM dashed/camel-case accessors.
/// Includes shorthand/legacy alias names, excludes custom and unknown names.
pub fn cssom_property_names() -> Vec<&'static str> {
    let mut names: Vec<_> = known_properties()
        .chain(LOGICAL_BORDER_LONGHANDS)
        .chain(cssom_shorthand_names())
        .chain(["word-wrap"])
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}
fn supported_shorthand(name: &str) -> bool {
    SHORTHANDS.contains(&name)
}
/// Project supported shorthand axes using the same expansion as the cascade.
/// Reset-only axes are included; unknown/unsupported names return None.
pub fn cssom_shorthand_longhands(name: &str) -> Option<Vec<String>> {
    let name = name.to_ascii_lowercase();
    if !supported_shorthand(&name) {
        return None;
    }
    let mut result: Vec<_> = expand_declaration(&name, "initial")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    if name == "background-position" {
        result = vec![
            "background-position-x".into(),
            "background-position-y".into(),
        ];
    }
    (!result.is_empty()).then_some(result)
}
impl ComputedStyle {
    /// Valid custom computed tokens. Empty values are valid; guaranteed-invalid
    /// values and cycles are omitted. Custom names retain their exact case.
    pub fn computed_custom_properties(&self) -> impl Iterator<Item = (&str, String)> + '_ {
        self.custom.iter().filter_map(|(name, value)| match value {
            CustomValue::Valid(tokens) => Some((name.as_str(), serialize_computed(tokens))),
            CustomValue::Invalid => None,
        })
    }
    /// Sorted supported longhands followed by valid custom computed values.
    /// Unknown literal passthrough and shorthand summaries are not enumerated.
    pub fn cssom_declarations(&self) -> Vec<(String, String)> {
        let mut normal: Vec<(String, String)> = computed_longhand_names()
            .filter_map(|name| self.get(name).map(|value| (name.into(), value.into())))
            .collect();
        normal.sort_by(|a, b| a.0.cmp(&b.0));
        normal.extend(
            self.computed_custom_properties()
                .map(|(name, value)| (name.into(), value)),
        );
        normal
    }
    /// Canonical computed CSS values. Geometry-dependent CSSOM resolved values
    /// belong to the driver. Unknown/unrepresentable shorthands return None.
    pub fn cssom_property_value(&self, name: &str) -> Option<String> {
        if name.starts_with("--") {
            return self
                .computed_custom_properties()
                .find_map(|(key, value)| (key == name).then_some(value));
        }
        let name = name.to_ascii_lowercase();
        if name == "font" {
            return crate::font_shorthand::computed(self);
        }
        if name == "word-wrap" {
            return self.get("overflow-wrap").map(str::to_owned);
        }
        if computed_longhand_names().any(|longhand| longhand == name) {
            return self.get(&name).map(str::to_owned);
        }
        if matches!(
            name.as_str(),
            "border-image"
                | "font-variant"
                | "font-synthesis"
                | "white-space"
                | "text-wrap"
                | "text-spacing"
        ) {
            return self.get(&name).map(str::to_owned);
        }
        if name == "background-position" {
            // This engine stores the normalized combined value as well as
            // independent axis declarations. Keep its combined serializer;
            // full cross-axis layer reconstruction is a separate area.
            if self.is_author_specified("background-position-x")
                || self.is_author_specified("background-position-y")
            {
                return None;
            }
            return self.get(&name).map(str::to_owned);
        }
        if !serializable_shorthand(&name) {
            return None;
        }
        if name == "border" {
            for (axis, initial) in crate::border_image::AXES
                .iter()
                .zip(crate::border_image::INITIAL)
            {
                if self.get(axis) != Some(initial) {
                    return None;
                }
            }
            let side = self.cssom_property_value("border-top")?;
            return ["right", "bottom", "left"]
                .into_iter()
                .all(|s| {
                    self.cssom_property_value(&format!("border-{s}")).as_deref()
                        == Some(side.as_str())
                })
                .then_some(side);
        }
        if matches!(name.as_str(), "border-inline" | "border-block") {
            let axes = cssom_shorthand_longhands(&name)?;
            let values: Option<Vec<_>> = axes
                .iter()
                .map(|axis| self.get(axis).map(str::to_owned))
                .collect();
            let v = values?;
            // Logical side shorthands expand width/style/color in each side.
            if v.len() != 6 || v[..3] != v[3..] {
                return None;
            }
            return Some(v[..3].join(" "));
        }
        let axes = cssom_shorthand_longhands(&name)?;
        let values: Option<Vec<_>> = axes
            .iter()
            .map(|axis| self.get(axis).map(str::to_owned))
            .collect();
        let values = values?;
        if name == "list-style"
            || matches!(
                name.as_str(),
                "border-top"
                    | "border-right"
                    | "border-bottom"
                    | "border-left"
                    | "border-inline-start"
                    | "border-inline-end"
                    | "border-block-start"
                    | "border-block-end"
            )
        {
            return Some(values.join(" "));
        }
        Some(quad(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> ComputedStyle {
        let mut dom = amazing_dom::Dom::new();
        let div = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))])
            .get(div)
            .unwrap()
            .clone()
    }
    #[test]
    fn custom_enumeration_preserves_case_empty_and_omits_guaranteed_invalid() {
        let s = style(
            "--Case:RED;--case:blue;--empty:;--bad:var(--missing);--cycle:var(--cycle);--copy:var(--Case);unsupported-made-up:red",
        );
        let customs: Vec<_> = s.computed_custom_properties().collect();
        assert_eq!(
            customs,
            vec![
                ("--Case", "RED".into()),
                ("--case", "blue".into()),
                ("--copy", "RED".into()),
                ("--empty", "".into())
            ]
        );
        assert_eq!(s.cssom_property_value("--empty"), Some("".into()));
        assert_eq!(s.cssom_property_value("--bad"), None);
        assert_eq!(s.cssom_property_value("--CASE"), None);
        let entries = s.cssom_declarations();
        let names: Vec<_> = entries.iter().map(|(name, _)| name.as_str()).collect();
        for excluded in [
            "border-image",
            "font-variant",
            "overflow",
            "white-space",
            "mask",
            "unsupported-made-up",
        ] {
            assert!(!names.contains(&excluded), "{excluded}");
        }
        let custom_start = names
            .iter()
            .position(|name| name.starts_with("--"))
            .unwrap();
        assert!(names[..custom_start].windows(2).all(|w| w[0] < w[1]));
        assert_eq!(names.len(), names.iter().collect::<BTreeSet<_>>().len());
    }
    #[test]
    fn independent_axes_serialize_quads_pairs_and_reset_only_borders() {
        let s = style(
            "margin:1px 2px 3px 4px;margin-left:2px;padding:5px;border:1px solid red;border-left-color:blue;overflow:clip hidden;border-image:12 round",
        );
        assert_eq!(s.cssom_property_value("MARGIN"), Some("1px 2px 3px".into()));
        assert_eq!(s.cssom_property_value("padding"), Some("5px".into()));
        assert_eq!(s.cssom_property_value("overflow"), Some("hidden".into()));
        assert_eq!(s.cssom_property_value("border"), None);
        assert!(s
            .cssom_property_value("border-top")
            .unwrap()
            .contains("solid"));
        assert!(s
            .cssom_property_value("border-image")
            .unwrap()
            .contains("12"));
        let s = style("border:1px solid red");
        assert!(s.cssom_property_value("border").unwrap().contains("solid"));
        assert_eq!(s.cssom_property_value("not-a-property"), None);
        assert_eq!(s.cssom_property_value("all"), None);
        assert_eq!(cssom_shorthand_longhands("border-image").unwrap().len(), 5);
        assert!(cssom_shorthand_longhands("color").is_none());
    }
    #[test]
    fn derived_summaries_follow_winning_longhands_and_aliases() {
        let s = style(
            "font-variant:small-caps;font-variant-numeric:tabular-nums;white-space:pre-wrap;text-wrap-mode:nowrap;word-wrap:anywhere",
        );
        let font = s.cssom_property_value("font-variant").unwrap();
        assert!(font.contains("small-caps") && font.contains("tabular-nums"));
        assert_eq!(
            s.cssom_property_value("word-wrap"),
            s.cssom_property_value("overflow-wrap")
        );
        let white = s.cssom_property_value("white-space").unwrap();
        assert_eq!(white, "pre");
        assert_eq!(
            s.cssom_property_value("white-space-collapse"),
            Some("preserve".into())
        );
        assert_eq!(
            s.cssom_property_value("text-wrap-mode"),
            Some("nowrap".into())
        );
        let names: Vec<_> = computed_longhand_names().collect();
        assert!(names.contains(&"font-variant-numeric"));
        assert!(names.contains(&"transform-origin"));
        assert!(names.contains(&"border-inline-start-color"));
        assert!(!names.contains(&"font-variant"));
        let idl = cssom_property_names();
        for name in [
            "margin",
            "background",
            "font",
            "animation",
            "border-radius",
            "font-variant",
            "border-inline-start-color",
            "word-wrap",
        ] {
            assert!(idl.contains(&name), "{name}");
        }
        assert!(idl.windows(2).all(|w| w[0] < w[1]));
        assert!(!idl.iter().any(|name| name.starts_with("--")));
        assert!(cssom_shorthand_longhands("font")
            .unwrap()
            .iter()
            .any(|name| name == "font-family"));
        assert!(cssom_shorthand_longhands("background")
            .unwrap()
            .iter()
            .any(|name| name == "background-image"));
        assert_eq!(s.cssom_property_value("font"), None); // noninitial reset-only numeric axis
    }
}
