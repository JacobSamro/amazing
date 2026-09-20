//! CSS Images 3 fitting geometry, independent of decoders and platform types.
use amazing_css::ComputedStyle;
use amazing_layout::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

/// An axis position: fraction of the leftover space, plus a CSS-pixel offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPosition {
    pub fraction: f32,
    pub offset: f32,
}
impl Default for ObjectPosition {
    fn default() -> Self {
        Self {
            fraction: 0.5,
            offset: 0.0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImagePresentation {
    pub fit: ObjectFit,
    pub x: ObjectPosition,
    pub y: ObjectPosition,
}
impl ImagePresentation {
    pub fn from_style(style: &ComputedStyle) -> Self {
        let fit = match style
            .get("object-fit")
            .unwrap_or("fill")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "contain" => ObjectFit::Contain,
            "cover" => ObjectFit::Cover,
            "none" => ObjectFit::None,
            "scale-down" => ObjectFit::ScaleDown,
            _ => ObjectFit::Fill,
        };
        let (x, y) =
            position(style.get("object-position").unwrap_or("50% 50%")).unwrap_or_default();
        Self { fit, x, y }
    }

    /// The concrete object rectangle. Caller clips it to the content box even
    /// for `overflow:visible`; positioning percentages use leftover space.
    pub fn destination(self, content: Rect, natural: (u32, u32)) -> Option<Rect> {
        if natural.0 == 0
            || natural.1 == 0
            || content.w <= 0.0
            || content.h <= 0.0
            || ![
                content.x,
                content.y,
                content.w,
                content.h,
                self.x.fraction,
                self.x.offset,
                self.y.fraction,
                self.y.offset,
            ]
            .iter()
            .all(|v| v.is_finite())
        {
            return None;
        }
        let (nw, nh) = (natural.0 as f64, natural.1 as f64);
        let (cw, ch) = (content.w as f64, content.h as f64);
        let contain = (cw / nw).min(ch / nh);
        let (w, h) = match self.fit {
            ObjectFit::Fill => (cw, ch),
            ObjectFit::None => (nw, nh),
            ObjectFit::Contain => (nw * contain, nh * contain),
            ObjectFit::Cover => {
                let scale = (cw / nw).max(ch / nh);
                (nw * scale, nh * scale)
            }
            ObjectFit::ScaleDown => (nw * contain.min(1.0), nh * contain.min(1.0)),
        };
        let x = content.x as f64 + (cw - w) * self.x.fraction as f64 + self.x.offset as f64;
        let y = content.y as f64 + (ch - h) * self.y.fraction as f64 + self.y.offset as f64;
        let rect = Rect {
            x: x as f32,
            y: y as f32,
            w: w as f32,
            h: h as f32,
        };
        [
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            rect.x + rect.w,
            rect.y + rect.h,
        ]
        .iter()
        .all(|v| v.is_finite())
        .then_some(rect)
    }
}
fn length(s: &str) -> Option<ObjectPosition> {
    let (raw, percent) = if let Some(n) = s.strip_suffix('%') {
        (n, true)
    } else if let Some(n) = s.strip_suffix("px") {
        (n, false)
    } else if s == "0" {
        (s, false)
    } else {
        return None;
    };
    let value: f32 = raw.parse().ok()?;
    value.is_finite().then_some(if percent {
        ObjectPosition {
            fraction: value / 100.0,
            offset: 0.0,
        }
    } else {
        ObjectPosition {
            fraction: 0.0,
            offset: value,
        }
    })
}
fn keyword(s: &str, horizontal: bool) -> Option<ObjectPosition> {
    let fraction = match s {
        "center" => 0.5,
        "left" if horizontal => 0.0,
        "right" if horizontal => 1.0,
        "top" if !horizontal => 0.0,
        "bottom" if !horizontal => 1.0,
        _ => return None,
    };
    Some(ObjectPosition {
        fraction,
        offset: 0.0,
    })
}
// One/two-component positions and three/four-component edge-offset syntax.
// calc() and logical-position keywords are deliberately not parsed here.
fn position(s: &str) -> Option<(ObjectPosition, ObjectPosition)> {
    let normalized = s.to_ascii_lowercase();
    let tokens: Vec<_> = normalized.split_whitespace().collect();
    let center = ObjectPosition::default();
    match tokens.as_slice() {
        [one] => {
            if matches!(*one, "top" | "bottom") {
                Some((center, keyword(one, false)?))
            } else {
                Some((keyword(one, true).or_else(|| length(one))?, center))
            }
        }
        [a, b] => {
            if matches!(*a, "top" | "bottom") || matches!(*b, "left" | "right") {
                Some((keyword(b, true)?, keyword(a, false)?))
            } else {
                Some((
                    keyword(a, true).or_else(|| length(a))?,
                    keyword(b, false).or_else(|| length(b))?,
                ))
            }
        }
        _ if (3..=4).contains(&tokens.len()) => {
            let (mut x, mut y) = (None, None);
            let mut index = 0;
            while index < tokens.len() {
                let edge = tokens[index];
                index += 1;
                let horizontal = match edge {
                    "left" | "right" => true,
                    "top" | "bottom" => false,
                    "center" => x.is_none(),
                    _ => return None,
                };
                let mut axis = keyword(edge, horizontal)?;
                if edge != "center" && index < tokens.len() {
                    if let Some(offset) = length(tokens[index]) {
                        let sign = if matches!(edge, "right" | "bottom") {
                            -1.0
                        } else {
                            1.0
                        };
                        axis.fraction += sign * offset.fraction;
                        axis.offset = sign * offset.offset;
                        index += 1;
                    }
                }
                let slot = if horizontal { &mut x } else { &mut y };
                if slot.replace(axis).is_some() {
                    return None;
                }
            }
            Some((x?, y?))
        }
        _ => None,
    }
}

/// Preserve each inline image's computed presentation at the provider seam.
/// This wrapper never changes layout or remeasures text.
pub struct StyledImages<'a> {
    pub images: &'a dyn crate::ImageProvider,
    pub styles: &'a amazing_css::StyledDom,
}
impl crate::ImageProvider for StyledImages<'_> {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        self.images.image(node)
    }
    fn presentation(&self, node: amazing_dom::NodeIdx) -> ImagePresentation {
        self.styles
            .get(node)
            .map(ImagePresentation::from_style)
            .unwrap_or_else(|| self.images.presentation(node))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_fit_modes_and_scale_down_have_distinct_geometry() {
        let c = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 100.0,
        };
        for (fit, w, h, x, y) in [
            (ObjectFit::Fill, 100.0, 100.0, 10.0, 20.0),
            (ObjectFit::Contain, 100.0, 50.0, 10.0, 45.0),
            (ObjectFit::Cover, 200.0, 100.0, -40.0, 20.0),
            (ObjectFit::None, 40.0, 20.0, 40.0, 60.0),
            (ObjectFit::ScaleDown, 40.0, 20.0, 40.0, 60.0),
        ] {
            let r = ImagePresentation {
                fit,
                ..Default::default()
            }
            .destination(c, (40, 20))
            .unwrap();
            assert_eq!((r.x, r.y, r.w, r.h), (x, y, w, h));
        }
        let r = ImagePresentation {
            fit: ObjectFit::ScaleDown,
            ..Default::default()
        }
        .destination(c, (400, 200))
        .unwrap();
        assert_eq!((r.w, r.h), (100.0, 50.0));
    }
    #[test]
    fn positions_include_negative_leftover_and_edge_offsets() {
        let (x, y) = position("bottom 1px right 2px").unwrap();
        assert_eq!(
            (x.fraction, x.offset, y.fraction, y.offset),
            (1.0, -2.0, 1.0, -1.0)
        );
        assert_eq!(position("top right"), position("right top"));
        let (x, y) = position("25% 75%").unwrap();
        let r = ImagePresentation {
            fit: ObjectFit::Cover,
            x,
            y,
        }
        .destination(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            (200, 100),
        )
        .unwrap();
        assert_eq!(r.x, -25.0);
        assert!(position("left right").is_none());
        assert!(position("NaN% 0").is_none());
    }
    #[test]
    fn invalid_geometry_fails_closed() {
        for width in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(ImagePresentation::default()
                .destination(
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        w: width,
                        h: 10.0
                    },
                    (2, 1)
                )
                .is_none());
        }
    }
}
