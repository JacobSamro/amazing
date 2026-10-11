//! Typed projection of the existing CSS Text 4 autospace grammar.
//! Grammar, shorthand expansion and canonical spelling remain in text_properties.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutospaceMode {
    Insert,
    Replace,
}

/// The native shaping policy does not need author keyword spelling. CSS keeps
/// that spelling separately because normal, auto and explicit insert differ
/// in computed serialization even when the selected policy is equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutospacePolicy {
    pub ideograph_alpha: bool,
    pub ideograph_numeric: bool,
    pub punctuation: bool,
    pub mode: AutospaceMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAutospace {
    #[default]
    Normal,
    Auto,
    NoAutospace,
    Explicit {
        ideograph_alpha: bool,
        ideograph_numeric: bool,
        punctuation: bool,
        /// Omitted insert remains omitted in CSS computed serialization.
        mode: Option<AutospaceMode>,
    },
}

impl TextAutospace {
    pub fn parse(css: &str) -> Option<Self> {
        let canonical = crate::text_properties::canonical("text-autospace", css)?;
        match canonical.as_str() {
            "normal" => Some(Self::Normal),
            "auto" => Some(Self::Auto),
            "no-autospace" => Some(Self::NoAutospace),
            "initial" | "inherit" | "unset" | "revert" | "revert-layer" => None,
            _ => {
                // The existing grammar has already authenticated these keywords,
                // order, exclusivity and multiplicity. Do not parse it twice.
                let words = canonical.split_ascii_whitespace().collect::<Vec<_>>();
                Some(Self::Explicit {
                    ideograph_alpha: words.contains(&"ideograph-alpha"),
                    ideograph_numeric: words.contains(&"ideograph-numeric"),
                    punctuation: words.contains(&"punctuation"),
                    mode: if words.contains(&"replace") { Some(AutospaceMode::Replace) }
                        else if words.contains(&"insert") { Some(AutospaceMode::Insert) }
                        else { None },
                })
            }
        }
    }

    pub fn to_css(self) -> String {
        match self {
            Self::Normal => "normal".into(),
            Self::Auto => "auto".into(),
            Self::NoAutospace => "no-autospace".into(),
            Self::Explicit { ideograph_alpha, ideograph_numeric, punctuation, mode } => {
                let mut words = Vec::new();
                if ideograph_alpha { words.push("ideograph-alpha"); }
                if ideograph_numeric { words.push("ideograph-numeric"); }
                if punctuation { words.push("punctuation"); }
                if let Some(mode) = mode {
                    words.push(match mode { AutospaceMode::Insert => "insert", AutospaceMode::Replace => "replace" });
                }
                words.join(" ")
            }
        }
    }

    pub fn policy(self) -> AutospacePolicy {
        match self {
            // CSS permits the UA to choose auto. Amazing selects normal's
            // conservative alpha+numeric insertion policy for that keyword.
            Self::Normal | Self::Auto => AutospacePolicy {
                ideograph_alpha: true, ideograph_numeric: true,
                punctuation: false, mode: AutospaceMode::Insert,
            },
            Self::NoAutospace => AutospacePolicy {
                ideograph_alpha: false, ideograph_numeric: false,
                punctuation: false, mode: AutospaceMode::Insert,
            },
            Self::Explicit { ideograph_alpha, ideograph_numeric, punctuation, mode } => AutospacePolicy {
                ideograph_alpha, ideograph_numeric, punctuation,
                mode: mode.unwrap_or(AutospaceMode::Insert),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::ComputedValue;

    #[test]
    fn existing_whole_grammar_projects_without_losing_keyword_serialization() {
        for css in ["normal", "auto", "no-autospace", "ideograph-alpha", "ideograph-numeric",
            "punctuation", "insert", "replace", "ideograph-alpha ideograph-numeric",
            "punctuation insert", "ideograph-numeric replace", "insert punctuation ideograph-alpha",
            "replace punctuation ideograph-alpha ideograph-numeric"] {
            let value = TextAutospace::parse(css).unwrap();
            assert_eq!(value.to_css(), crate::text_properties::canonical("text-autospace", css).unwrap());
            assert_eq!(TextAutospace::parse(&value.to_css()), Some(value));
            assert!(matches!(crate::values::parse("text-autospace", css), Some(ComputedValue::TextAutospace(v)) if v == value));
        }
        assert_ne!(TextAutospace::Normal, TextAutospace::Auto);
        assert_eq!(TextAutospace::Normal.policy(), TextAutospace::Auto.policy());
        assert_eq!(TextAutospace::parse("ideograph-alpha").unwrap().policy(), TextAutospace::parse("ideograph-alpha insert").unwrap().policy());
        assert_ne!(TextAutospace::parse("ideograph-alpha"), TextAutospace::parse("ideograph-alpha insert"));
    }

    #[test]
    fn all_independent_flag_mode_combinations_keep_the_existing_canonical_order() {
        for mask in 1..8 {
            for mode in ["", "insert", "replace"] {
                let mut words = Vec::new();
                if mask & 4 != 0 { words.push("punctuation"); }
                if !mode.is_empty() { words.push(mode); }
                if mask & 2 != 0 { words.push("ideograph-numeric"); }
                if mask & 1 != 0 { words.push("ideograph-alpha"); }
                let value = TextAutospace::parse(&words.join(" ")).unwrap();
                let policy = value.policy();
                assert_eq!(policy.ideograph_alpha, mask & 1 != 0);
                assert_eq!(policy.ideograph_numeric, mask & 2 != 0);
                assert_eq!(policy.punctuation, mask & 4 != 0);
                assert_eq!(policy.mode, if mode == "replace" { AutospaceMode::Replace } else { AutospaceMode::Insert });
                assert_eq!(TextAutospace::parse(&value.to_css()), Some(value));
            }
        }
        for invalid in ["", "none", "normal insert", "auto replace", "punctuation normal",
            "no-autospace punctuation", "insert replace", "replace replace", "ideograph-alpha ideograph-alpha",
            "ideograph-numeric 1", "punctuation, insert", "var(--missing)"] {
            assert_eq!(TextAutospace::parse(invalid), None, "{invalid}");
        }
        assert_eq!(TextAutospace::parse("i\\64 eograph-alpha/**/replace").unwrap().to_css(), "ideograph-alpha replace");
    }

    #[test]
    fn typed_policy_follows_inheritance_variables_invalid_at_computed_time_and_shorthand() {
        let mut dom = amazing_dom::Dom::new();
        let parent = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let child = dom.create_element(parent, "span");
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div{text-autospace:punctuation ideograph-alpha replace}span{--space:ideograph-numeric insert;text-autospace:var(--space)}")]);
        assert_eq!(styles.get(parent).unwrap().get("text-autospace"), Some("ideograph-alpha punctuation replace"));
        assert_eq!(styles.get(child).unwrap().typed("text-autospace"), Some(&ComputedValue::TextAutospace(TextAutospace::parse("ideograph-numeric insert").unwrap())));
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div{text-autospace:ideograph-alpha replace}span{text-autospace:no-autospace;text-autospace:var(--missing)}")]);
        assert_eq!(styles.get(child).unwrap().typed("text-autospace"), styles.get(parent).unwrap().typed("text-autospace"));
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div{text-autospace:punctuation}span{text-spacing:none}")]);
        assert_eq!(styles.get(child).unwrap().typed("text-autospace"), Some(&ComputedValue::TextAutospace(TextAutospace::NoAutospace)));
    }

    #[test]
    fn setters_keep_typed_policy_and_computed_spelling_in_sync() {
        let mut style = crate::ComputedStyle::default();
        for css in ["normal", "auto", "no-autospace", "replace ideograph-alpha", "ideograph-numeric punctuation insert"] {
            style.set("text-autospace", css);
            let value = TextAutospace::parse(css).unwrap();
            assert_eq!(style.get("text-autospace"), Some(value.to_css().as_str()));
            assert_eq!(style.typed("text-autospace"), Some(&ComputedValue::TextAutospace(value)));
        }
        let before = style.get("text-autospace").unwrap().to_owned();
        style.set("text-autospace", "normal replace");
        assert_eq!(style.get("text-autospace"), Some(before.as_str()));
        style.set("text-spacing", "none");
        assert_eq!(style.typed("text-autospace"), Some(&ComputedValue::TextAutospace(TextAutospace::NoAutospace)));
    }
    #[test]
    fn ua_spacing_policy_uses_element_semantics_and_allows_author_override() {
        let mut dom = amazing_dom::Dom::new();
        let paragraph = dom.create_element(amazing_dom::NodeIdx(0), "p");
        let tags = ["pre", "code", "kbd", "samp", "tt", "listing", "xmp", "plaintext"];
        let nodes = tags.map(|tag| dom.create_element(amazing_dom::NodeIdx(0), tag));
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("p{font-family:monospace}")]);
        assert_eq!(styles.get(paragraph).unwrap().typed("text-autospace"), Some(&ComputedValue::TextAutospace(TextAutospace::Normal)));
        for node in nodes {
            assert_eq!(styles.get(node).unwrap().typed("text-autospace"), Some(&ComputedValue::TextAutospace(TextAutospace::NoAutospace)));
        }
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("pre,code,kbd,samp,tt,listing,xmp,plaintext{text-autospace:ideograph-alpha replace}")]);
        for node in nodes {
            assert_eq!(styles.get(node).unwrap().typed("text-autospace"), Some(&ComputedValue::TextAutospace(TextAutospace::parse("ideograph-alpha replace").unwrap())));
        }
    }

}
