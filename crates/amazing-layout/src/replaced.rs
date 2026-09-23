//! Atomic replaced geometry. Natural dimensions cross the existing resource
//! seam; CSS dimensions are resolved once the containing block is known.
use crate::{
    Flow, InlineNode, IntrinsicSize, LayoutBox, LineBox, Rect, ReplacedElementProvider,
    ReplacedFrag,
    values::{self, Sides},
};
use amazing_css::ComputedStyle;
use amazing_dom::{Dom, NodeIdx};

/// Control identity only; live values, selection and checked state belong to DOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacedKind {
    Image,
    Svg,
    TextInput,
    Checkbox,
    Radio,
    Button,
    Select,
    Textarea,
}

/// Style and natural geometry retained through flattening and repeated layout.
#[derive(Debug, Clone)]
pub struct InlineReplaced {
    pub kind: ReplacedKind,
    pub style: ComputedStyle,
    pub font_size: f32,
    pub natural: IntrinsicSize,
    pub(crate) svg_sizing: Option<crate::svg::SvgSizing>,
    pub(crate) geometry: Geometry,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Geometry {
    content: Rect,
    padding: Sides,
    border: Sides,
    margin: Sides,
}

/// Paint-facing view of a positioned atomic inline. Existing `ReplacedFrag`
/// rectangles remain content boxes; all decoration geometry is supplied here.
#[derive(Debug)]
pub struct StyledReplacedFrag<'a> {
    pub node: NodeIdx,
    pub kind: ReplacedKind,
    pub style: &'a ComputedStyle,
    pub font_size: f32,
    pub content_box: Rect,
    pub border_box: Rect,
    pub padding: Sides,
    pub border: Sides,
    pub margin: Sides,
}

impl LayoutBox {
    /// Styled atomic fragments directly owned by this formatting context.
    /// Tree consumers recurse into normal and positioned children as usual.
    pub fn styled_replaced_fragments(&self) -> impl Iterator<Item = StyledReplacedFrag<'_>> {
        let items = match &self.flow {
            Flow::Inlines(items) => items.as_slice(),
            _ => &[],
        };
        let boxes: std::collections::HashMap<_, _> = items
            .iter()
            .filter_map(|item| match item {
                InlineNode::Replaced { node, replaced, .. } => Some((node.0, replaced.as_ref())),
                _ => None,
            })
            .collect();
        self.lines
            .iter()
            .flat_map(|line| &line.replaced)
            .filter_map(move |fragment| {
                let info = boxes.get(&fragment.node.0)?;
                let g = info.geometry;
                Some(StyledReplacedFrag {
                    node: fragment.node,
                    kind: info.kind,
                    style: &info.style,
                    font_size: info.font_size,
                    content_box: Rect {
                        x: fragment.x,
                        y: fragment.y,
                        w: fragment.width,
                        h: fragment.height,
                    },
                    border_box: Rect {
                        x: fragment.x - g.padding.left - g.border.left,
                        y: fragment.y - g.padding.top - g.border.top,
                        w: fragment.width
                            + g.padding.left
                            + g.padding.right
                            + g.border.left
                            + g.border.right,
                        h: fragment.height
                            + g.padding.top
                            + g.padding.bottom
                            + g.border.top
                            + g.border.bottom,
                    },
                    padding: g.padding,
                    border: g.border,
                    margin: g.margin,
                })
            })
    }
}

fn attr<'a>(dom: &'a Dom, node: NodeIdx, name: &str) -> Option<&'a str> {
    dom.attrs_of(node)?
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

pub(crate) fn kind(dom: &Dom, node: NodeIdx) -> Option<ReplacedKind> {
    if crate::svg::is_outer(dom, node) {
        return Some(ReplacedKind::Svg);
    }
    if dom.namespace(node) != Some(amazing_dom::Namespace::Html) {
        return None;
    }
    Some(match dom.name(node)? {
        "img" => ReplacedKind::Image,
        "input" => match attr(dom, node, "type")
            .unwrap_or("text")
            .to_ascii_lowercase()
            .as_str()
        {
            "hidden" => return None,
            "checkbox" => ReplacedKind::Checkbox,
            "radio" => ReplacedKind::Radio,
            "button" | "submit" | "reset" => ReplacedKind::Button,
            "image" => ReplacedKind::Image,
            _ => ReplacedKind::TextInput,
        },
        "button" => ReplacedKind::Button,
        "select" => ReplacedKind::Select,
        "textarea" => ReplacedKind::Textarea,
        _ => return None,
    })
}

pub(crate) fn hidden(dom: &Dom, node: NodeIdx) -> bool {
    dom.name(node) == Some("input")
        && attr(dom, node, "type").is_some_and(|s| s.eq_ignore_ascii_case("hidden"))
}

pub(crate) fn inline_atomic(dom: &Dom, node: NodeIdx, style: &ComputedStyle) -> bool {
    kind(dom, node).is_some()
        && crate::floats::side(style).is_none()
        && style.get("display").is_none_or(|display| {
            display.eq_ignore_ascii_case("inline") || display.eq_ignore_ascii_case("inline-block")
        })
}

pub(crate) fn construct(
    dom: &Dom,
    node: NodeIdx,
    style: &ComputedStyle,
    parent_font: f32,
    provider: &dyn ReplacedElementProvider,
) -> Option<InlineNode> {
    let kind = kind(dom, node)?;
    let font_size =
        values::resolve_font_size(style.get("font-size").unwrap_or("medium"), parent_font);
    let count = |name, fallback| {
        attr(dom, node, name)
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(fallback)
            .min(10000) as f32
    };
    let natural = provider
        .intrinsic_size(node)
        .filter(|size| {
            size.width.is_finite()
                && size.height.is_finite()
                && size.width >= 0.0
                && size.height >= 0.0
        })
        .or_else(|| {
            Some(match kind {
                // Undecoded images retain the existing no-fragment behavior.
                ReplacedKind::Image => return None,
                ReplacedKind::Svg => IntrinsicSize { width: 300.0, height: 150.0 },
                ReplacedKind::Checkbox | ReplacedKind::Radio => IntrinsicSize {
                    width: 13.0,
                    height: 13.0,
                },
                ReplacedKind::TextInput => IntrinsicSize {
                    width: count("size", 20) * font_size * 0.6,
                    height: font_size * 1.2,
                },
                ReplacedKind::Textarea => IntrinsicSize {
                    width: count("cols", 20) * font_size * 0.6,
                    height: count("rows", 2) * font_size * 1.2,
                },
                ReplacedKind::Button | ReplacedKind::Select => IntrinsicSize {
                    width: font_size * 5.0,
                    height: font_size * 1.5,
                },
            })
        })?;
    let mut replaced = Box::new(InlineReplaced {
        kind,
        style: style.clone(),
        font_size,
        natural,
        svg_sizing: (kind == ReplacedKind::Svg).then(|| crate::svg::SvgSizing::from_dom(dom, node)),
        geometry: Geometry::default(),
    });
    // Useful for intrinsic-width consumers before block placement; percentages
    // have no definite basis during this query.
    let size = replaced.resolve(None, None);
    Some(InlineNode::Replaced {
        node,
        size,
        replaced,
    })
}

fn length(style: &ComputedStyle, name: &str, font: f32, basis: Option<f32>) -> Option<f32> {
    values::style_length_percentage(style, name, font, font * 0.6)?
        .resolve(basis)
        .filter(|v| v.is_finite() && *v >= 0.0)
}

impl InlineReplaced {
    fn resolve(&mut self, width: Option<f32>, height: Option<f32>) -> IntrinsicSize {
        let style = &self.style;
        let padding = values::resolve_padding(style, self.font_size, width.unwrap_or(0.0));
        let border = values::resolve_borders(style, self.font_size);
        let margin =
            values::resolve_margins(style, self.font_size, width.unwrap_or(0.0)).unwrap_or(0.0);
        let chrome_w = padding.left + padding.right + border.left + border.right;
        let chrome_h = padding.top + padding.bottom + border.top + border.bottom;
        let border_box = style
            .get("box-sizing")
            .is_some_and(|s| s.eq_ignore_ascii_case("border-box"));
        let extent = |name: &str, basis: Option<f32>, chrome: f32| {
            length(style, name, self.font_size, basis)
                .map(|n| if border_box { (n - chrome).max(0.0) } else { n })
        };
        let w = extent("width", width, chrome_w);
        let h = extent("height", height, chrome_h);
        let min_w = extent("min-width", width, chrome_w).unwrap_or(0.0);
        let min_h = extent("min-height", height, chrome_h).unwrap_or(0.0);
        let max_w = extent("max-width", width, chrome_w)
            .unwrap_or(f32::INFINITY)
            .max(min_w);
        let max_h = extent("max-height", height, chrome_h)
            .unwrap_or(f32::INFINITY)
            .max(min_h);
        // Controls have natural preferred dimensions, not an image aspect ratio.
        let natural = self.svg_sizing.as_ref().map_or(self.natural, |svg| {
            svg.used_size(style, self.font_size, width, height)
        });
        let ratio = (matches!(self.kind, ReplacedKind::Image | ReplacedKind::Svg)
            && natural.width > 0.0
            && natural.height > 0.0)
            .then_some(natural.width / natural.height);
        let mut used_w =
            w.unwrap_or_else(|| h.zip(ratio).map_or(natural.width, |(h, r)| h * r));
        let mut used_h = h.unwrap_or_else(|| ratio.map_or(natural.height, |r| used_w / r));
        if w.is_none() && h.is_none() && ratio.is_some() {
            // CSS2.2 10.4: preserve ratio unless opposing constraints require
            // distortion. The feasible scale interval handles all table rows.
            let lower = (min_w / used_w).max(min_h / used_h);
            let upper = (max_w / used_w).min(max_h / used_h);
            let scale = 1.0_f32.max(lower).min(upper);
            used_w = (used_w * scale).clamp(min_w, max_w);
            used_h = (used_h * scale).clamp(min_h, max_h);
        } else {
            used_w = used_w.clamp(min_w, max_w);
            if h.is_none()
                && let Some(r) = ratio
            {
                used_h = used_w / r;
            }
            used_h = used_h.clamp(min_h, max_h);
        }
        self.geometry = Geometry {
            content: Rect {
                x: 0.0,
                y: 0.0,
                w: used_w,
                h: used_h,
            },
            padding,
            border,
            margin,
        };
        IntrinsicSize {
            width: (used_w + chrome_w + margin.left + margin.right).max(0.0),
            height: (used_h + chrome_h + margin.top + margin.bottom).max(0.0),
        }
    }
}

pub(crate) fn resolve(items: &mut [InlineNode], width: f32, height: Option<f32>) {
    for item in items {
        if let InlineNode::Replaced { size, replaced, .. } = item {
            *size = replaced.resolve(Some(width), height);
        }
    }
}

/// Line fitting, baselines, floats and bidi operate on the outer geometry.
/// Convert to the paint content rectangle only after final visual placement.
pub(crate) fn finish_lines(lines: &mut [LineBox], items: &[InlineNode]) {
    let geometry: std::collections::HashMap<_, _> = items
        .iter()
        .filter_map(|item| match item {
            InlineNode::Replaced { node, replaced, .. } => Some((node.0, replaced.geometry)),
            _ => None,
        })
        .collect();
    for ReplacedFrag {
        node,
        x,
        y,
        width,
        height,
    } in lines.iter_mut().flat_map(|line| &mut line.replaced)
    {
        if let Some(g) = geometry.get(&node.0) {
            *x += g.margin.left + g.border.left + g.padding.left;
            *y += g.margin.top + g.border.top + g.padding.top;
            *width = g.content.w;
            *height = g.content.h;
        }
    }
}
