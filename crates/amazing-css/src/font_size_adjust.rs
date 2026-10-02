//! CSS Fonts 5 §3.4: font-size normalization and its font/geometry handoff.
//! CSS cannot invent the first available font's metrics. `FromFont` and
//! context-dependent numeric math remain typed until the consumer resolves them.
use crate::syntax::{Token, tokenize, serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontSizeAdjustMetric { ExHeight, CapHeight, ChWidth, IcWidth, IcHeight }
impl FontSizeAdjustMetric {
    fn parse(css: &str) -> Option<Self> {
        Some(match css.to_ascii_lowercase().as_str() {
            "ex-height" => Self::ExHeight, "cap-height" => Self::CapHeight,
            "ch-width" => Self::ChWidth, "ic-width" => Self::IcWidth,
            "ic-height" => Self::IcHeight, _ => return None,
        })
    }
    pub fn css(self) -> &'static str {
        match self { Self::ExHeight => "ex-height", Self::CapHeight => "cap-height",
            Self::ChWidth => "ch-width", Self::IcWidth => "ic-width", Self::IcHeight => "ic-height" }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FontSizeAdjustAspect {
    Number(f64),
    FromFont,
    /// A valid number expression whose sign() needs contextual lengths.
    Math(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FontSizeAdjust {
    None,
    Value { metric: FontSizeAdjustMetric, aspect: FontSizeAdjustAspect },
}

fn number(css: &str) -> Option<f64> {
    if !crate::math::number_percentage(css, false) { return None }
    crate::values::alpha_value(css).filter(|n| n.is_finite()).map(|n| n.max(0.0))
}

impl FontSizeAdjust {
    pub fn parse(css: &str) -> Option<Self> {
        let tokens = tokenize(css);
        let parts = crate::computed::top_level_components(&tokens)?;
        if matches!(parts.as_slice(), [part] if matches!(crate::computed::trim_tokens(part), [Token::Ident(n)] if n.eq_ignore_ascii_case("none"))) {
            return Some(Self::None);
        }
        let (metric, value) = match parts.as_slice() {
            [value] => (FontSizeAdjustMetric::ExHeight, serialize(value)),
            [metric, value] => {
                let [Token::Ident(metric)] = crate::computed::trim_tokens(metric) else { return None };
                (FontSizeAdjustMetric::parse(metric)?, serialize(value))
            }
            _ => return None,
        };
        let aspect = if value.eq_ignore_ascii_case("from-font") { FontSizeAdjustAspect::FromFont }
            else {
                let value_tokens = tokenize(&value);
                let part = crate::computed::trim_tokens(&value_tokens);
                if matches!(part, [Token::Number(n)] if !n.is_finite() || *n < 0.0) { return None }
                if let Some(number) = number(&value) { FontSizeAdjustAspect::Number(number) }
                else {
                    let validation = crate::computed::individual_transforms::signs(&value, None)?;
                    number(&validation)?;
                    FontSizeAdjustAspect::Math(value)
                }
            };
        Some(Self::Value { metric, aspect })
    }

    /// Resolve the chosen metric from the first available font, as a ratio to
    /// its em size. No available first font means `none`; zero is valid.
    /// The provider supplies standard missing-metric fallbacks (for example,
    /// one em for unavailable ideographic metrics) before passing the ratio.
    /// Explicit numbers and numeric math are independent of this operation.
    pub fn resolve_from_font(&self, ratio: Option<f64>) -> Self {
        match self {
            Self::Value { metric, aspect: FontSizeAdjustAspect::FromFont } => {
                match ratio.filter(|ratio| ratio.is_finite() && *ratio >= 0.0) {
                    Some(ratio) => Self::Value { metric: *metric, aspect: FontSizeAdjustAspect::Number(ratio) },
                    None => Self::None,
                }
            }
            _ => self.clone(),
        }
    }

    /// A font/layout consumer supplies px for every contextual length unit,
    /// including container units; unresolved values stay typed and symbolic.
    /// This keeps the core independent of font-provider and layout crates.
    pub fn resolve_math(&self, mut length_px: impl FnMut(f64, &str) -> Option<f64>) -> Self {
        let Self::Value { metric, aspect: FontSizeAdjustAspect::Math(css) } = self else { return self.clone() };
        let tokens = tokenize(css).into_iter().map(|token| match token {
            Token::Dimension(n, unit) => {
                let px = length_px(n, &unit)?;
                px.is_finite().then(|| Token::Dimension(px, "px".to_owned()))
            }
            token => Some(token),
        }).collect::<Option<Vec<_>>>();
        let Some(tokens) = tokens else { return self.clone() };
        let Some(value) = crate::computed::individual_transforms::signs(&serialize(&tokens), Some((16.0, 16.0)))
            .and_then(|css| number(&css)) else { return self.clone() };
        Self::Value { metric: *metric, aspect: FontSizeAdjustAspect::Number(value) }
    }

    /// Default ex-height is omitted; unresolved font/geometry inputs are kept
    /// explicitly rather than serialized as a fabricated numeric metric.
    pub fn to_css(&self) -> String {
        match self {
            Self::None => "none".to_owned(),
            Self::Value { metric, aspect } => {
                let aspect = match aspect {
                    FontSizeAdjustAspect::Number(n) => n.to_string(),
                    FontSizeAdjustAspect::FromFont => "from-font".to_owned(),
                    FontSizeAdjustAspect::Math(css) => css.clone(),
                };
                if *metric == FontSizeAdjustMetric::ExHeight { aspect }
                else { format!("{} {aspect}", metric.css()) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_metric_grammar_and_range_rules() {
        for metric in ["ex-height", "cap-height", "ch-width", "ic-width", "ic-height"] {
            for value in ["0", "0.5", "from-font", "calc(-1)", "min(0.5, 0.8)"] {
                let css = format!("{metric} {value}");
                assert!(FontSizeAdjust::parse(&css).is_some());
                assert!(crate::values::parses("font-size-adjust", &css), "CSS: {css}");
                assert!(crate::svg_presentation::normalize("font-size-adjust", &css, "text").is_some(), "SVG: {css}");
            }
        }
        for value in ["", "-0.5", "0.5%", "calc(1%)", "calc(1px)", "0.5 cap-height", "em 0.5", "cap-height none", "from-font 1", "cap-height cap-height", "calc(1 + 1%)"] {
            assert!(FontSizeAdjust::parse(value).is_none(), "{value}");
        }
        assert_eq!(FontSizeAdjust::parse("ex-height 0.5").unwrap().to_css(), "0.5");
        assert_eq!(FontSizeAdjust::parse("cap-height calc(-0.5)").unwrap().to_css(), "cap-height 0");
    }
    #[test]
    fn actual_font_and_context_metrics_resolve_without_css_guesses() {
        let value = FontSizeAdjust::parse("cap-height from-font").unwrap();
        assert_eq!(value.resolve_from_font(Some(0.75)).to_css(), "cap-height 0.75");
        assert_eq!(value.resolve_from_font(None), FontSizeAdjust::None);
        assert_eq!(value.resolve_from_font(Some(f64::NAN)), FontSizeAdjust::None);
        assert_eq!(value.resolve_from_font(Some(0.0)).to_css(), "cap-height 0");
        let value = FontSizeAdjust::parse("cap-height calc(10 + sign(20cqw - 10px) * 5)").unwrap();
        assert_eq!(value.resolve_math(|n, unit| match unit { "px" => Some(n), "cqw" => Some(n * 0.1), _ => None }).to_css(), "cap-height 5");
        assert_eq!(value.resolve_math(|_, _| None), value);
    }
    #[test]
    fn variables_inheritance_and_font_reset_keep_typed_aspects() {
        let mut dom = amazing_dom::Dom::new();
        let outer = dom.create_element(amazing_dom::NodeIdx(0), "div");
        let child = dom.create_element(outer, "span");
        assert_eq!(FontSizeAdjust::parse("cap-height calc(0.5 + 1)").unwrap().to_css(), "cap-height 1.5");
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div { font-size-adjust:ch-width from-font } span { --metric:cap-height calc(0.5 + 1); font-size-adjust:var(--metric) }")]);
        assert_eq!(styles.get(child).unwrap().get("font-size-adjust"), Some("cap-height 1.5"));
        assert!(matches!(styles.get(outer).unwrap().typed("font-size-adjust"), Some(crate::values::ComputedValue::FontSizeAdjust(FontSizeAdjust::Value { metric: FontSizeAdjustMetric::ChWidth, aspect: FontSizeAdjustAspect::FromFont }))));
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div { font-size-adjust:ic-height 0.7 } span { font-size-adjust:0.5; font-size-adjust:var(--bad) }")]);
        assert_eq!(styles.get(child).unwrap().get("font-size-adjust"), Some("ic-height 0.7"));
        let styles = crate::compute(&dom, &[crate::parse_stylesheet("div { font-size-adjust:ch-width 0.4; font:20px serif }")]);
        assert_eq!(styles.get(outer).unwrap().get("font-size-adjust"), Some("none"));
    }
}

/// Resolve font/absolute numeric math at the existing computed-style boundary.
/// Container and unavailable font metrics remain symbolic for their consumer.
pub(crate) fn resolve_css_math(properties: &mut std::collections::BTreeMap<String, String>, root: f64) {
    let Some(value) = properties.get("font-size-adjust").and_then(|css| FontSizeAdjust::parse(css)) else { return };
    let font = properties.get("font-size").and_then(|css| css.strip_suffix("px"))
        .and_then(|value| value.parse().ok()).unwrap_or(16.0);
    let resolved = value.resolve_math(|n, unit| {
        // Actual glyph metrics must wait for the font context, not generic em guesses.
        if matches!(unit, "ex" | "cap" | "ch" | "ic" | "rex" | "rcap" | "rch" | "ric") { return None; }
        let length = crate::values::LengthPercentage::parse(&format!("{n}{unit}"))?;
        let mut typed = crate::values::ComputedValue::LengthPercentage(length);
        crate::values::resolve_font_units(&mut typed, font, root, false);
        match typed { crate::values::ComputedValue::LengthPercentage(crate::values::LengthPercentage::Px(px)) => Some(px), _ => None }
    });
    properties.insert("font-size-adjust".to_owned(), resolved.to_css());
}
