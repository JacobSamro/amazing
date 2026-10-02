//! Actual font metrics at the computed-value boundary, before inheritance.
use crate::ComputedStyle;
use crate::font_size_adjust::{FontSizeAdjust, FontSizeAdjustAspect, FontSizeAdjustMetric};
use crate::selectors::PseudoElement;
use amazing_dom::NodeIdx;

/// The driver bridges CSS selection properties to its actual font provider.
/// Ratios describe the first available font at the unadjusted CSS font size.
/// Width and height metrics are physical, regardless of paragraph orientation.
/// Return `None` only when no first font is available; standard missing-glyph
/// metric fallbacks belong in the provider. CSS never fabricates font data.
pub trait FontMetricContext {
    fn aspect_ratio(
        &self,
        node: NodeIdx,
        pseudo: Option<PseudoElement>,
        style: &ComputedStyle,
        metric: FontSizeAdjustMetric,
    ) -> Option<f64>;

    /// Resolve contextual lengths in numeric `sign()` expressions. The driver
    /// may return `None` for geometry not available during style computation.
    fn contextual_length_px(
        &self,
        _node: NodeIdx,
        _pseudo: Option<PseudoElement>,
        _style: &ComputedStyle,
        _quantity: f64,
        _unit: &str,
    ) -> Option<f64> {
        None
    }
}

pub(crate) fn resolve(
    style: &mut ComputedStyle,
    node: NodeIdx,
    pseudo: Option<PseudoElement>,
    context: Option<&dyn FontMetricContext>,
) {
    // An inherited symbolic aspect keeps its originating font/geometry scope.
    // It must not be re-evaluated against a descendant's different font.
    if style.font_size_adjust_inherited {
        return;
    }
    let Some(context) = context else {
        return;
    };
    let Some(value) = style
        .get("font-size-adjust")
        .and_then(FontSizeAdjust::parse)
    else {
        return;
    };
    let resolved = match &value {
        FontSizeAdjust::Value {
            metric,
            aspect: FontSizeAdjustAspect::FromFont,
        } => value.resolve_from_font(context.aspect_ratio(node, pseudo, style, *metric)),
        FontSizeAdjust::Value {
            aspect: FontSizeAdjustAspect::Math(_),
            ..
        } => value.resolve_math(|quantity, unit| {
            context.contextual_length_px(node, pseudo, style, quantity, unit)
        }),
        _ => return,
    };
    style.set_contextual_value("font-size-adjust", &resolved.to_css());
}
