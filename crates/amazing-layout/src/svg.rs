//! CSS box sizing for an outermost inline SVG. SVG descendants belong to the
//! renderer; only the root participates in CSS layout.
use amazing_css::ComputedStyle;
use amazing_dom::{Dom, Namespace, NodeIdx};

use crate::{IntrinsicSize, values};

#[derive(Debug, Clone)]
pub(crate) struct SvgSizing {
    width: Option<String>,
    height: Option<String>,
    viewbox_ratio: Option<f32>,
}

pub(crate) fn is_outer(dom: &Dom, node: NodeIdx) -> bool {
    dom.namespace(node) == Some(Namespace::Svg)
        && dom.name(node) == Some("svg")
        && dom
            .get(node)
            .parent
            .is_some_and(|parent| dom.namespace(parent) != Some(Namespace::Svg))
}

fn attr<'a>(dom: &'a Dom, node: NodeIdx, name: &str) -> Option<&'a str> {
    dom.attrs_of(node)?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn svg_length(style: &ComputedStyle, raw: &str, font: f32, basis: Option<f32>) -> Option<f32> {
    let raw = raw.trim();
    // SVG presentation lengths permit unitless user units, unlike CSS lengths.
    let unitless = raw.parse::<f32>().ok();
    let value = if let Some(number) = unitless {
        style.scale_used_length(number as f64) as f32
    } else {
        values::used_length_percentage(style, raw, font, font * 0.6)?.resolve(basis)?
    };
    (value.is_finite() && value > 0.0).then_some(value)
}

fn css_length(style: &ComputedStyle, raw: &str, font: f32, basis: Option<f32>) -> Option<f32> {
    values::used_length_percentage(style, raw, font, font * 0.6)?
        .resolve(basis)
        .filter(|value| value.is_finite() && *value >= 0.0)
}

impl SvgSizing {
    pub(crate) fn from_dom(dom: &Dom, node: NodeIdx) -> Self {
        let width = attr(dom, node, "width").map(str::to_owned);
        let height = attr(dom, node, "height").map(str::to_owned);
        let viewbox_ratio = attr(dom, node, "viewBox").and_then(|raw| {
            let dimensions: Vec<_> = raw
                .split(|c: char| c.is_ascii_whitespace() || c == ',')
                .filter(|part| !part.is_empty())
                .map(str::parse::<f32>)
                .collect::<Result<_, _>>()
                .ok()?;
            if dimensions.len() != 4 {
                return None;
            }
            let (w, h) = (dimensions[2], dimensions[3]);
            (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some(w / h)
        });
        Self {
            width,
            height,
            viewbox_ratio,
        }
    }

    /// CSS width/height win over the SVG presentation attributes. A percentage
    /// attribute needs the containing block; an unresolved value is unsized.
    pub(crate) fn used_size(
        &self,
        style: &ComputedStyle,
        font: f32,
        width_basis: Option<f32>,
        height_basis: Option<f32>,
    ) -> IntrinsicSize {
        let author_width = style.is_author_specified("width");
        let author_height = style.is_author_specified("height");
        let width = match style.get("width").filter(|_| author_width) {
            Some(css) => css_length(style, css, font, width_basis),
            None => self
                .width
                .as_deref()
                .and_then(|raw| svg_length(style, raw, font, width_basis)),
        };
        let height = match style.get("height").filter(|_| author_height) {
            Some(css) => css_length(style, css, font, height_basis),
            None => self
                .height
                .as_deref()
                .and_then(|raw| svg_length(style, raw, font, height_basis)),
        };
        let dimensions_ratio = (!author_width && !author_height)
            .then(|| {
                self.width
                    .as_deref()
                    .and_then(|raw| svg_length(style, raw, font, None))
                    .zip(
                        self.height
                            .as_deref()
                            .and_then(|raw| svg_length(style, raw, font, None)),
                    )
                    .map(|(w, h)| w / h)
            })
            .flatten();
        let ratio = dimensions_ratio.or(self.viewbox_ratio);
        let (width, height) = match (width, height, ratio) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some(r)) => (w, w / r),
            (None, Some(h), Some(r)) => (h * r, h),
            (Some(w), None, None) => (w, style.scale_used_length(150.0) as f32),
            (None, Some(h), None) => (style.scale_used_length(300.0) as f32, h),
            (None, None, Some(r)) => {
                let w = width_basis.unwrap_or(style.scale_used_length(300.0) as f32);
                (w, w / r)
            }
            (None, None, None) => (
                style.scale_used_length(300.0) as f32,
                style.scale_used_length(150.0) as f32,
            ),
        };
        IntrinsicSize {
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }
}
