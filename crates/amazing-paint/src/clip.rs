//! Bounded painter reference for CSS `clip-path` basic shapes.
//!
//! Only `inset()`, `circle()` and `ellipse()` resolve to a rasterizable
//! mask; `polygon()`, `path()`, a `url()` reference and a bare
//! `<geometry-box>` with no shape are skipped rather than guessed. Every
//! shape resolves its percentages, keywords (`closest-side`,
//! `farthest-side`, position keywords) and `at <position>` against the
//! box's own border box, regardless of an explicit `<geometry-box>`: this
//! bounded resolver does not track a separate padding/margin reference box.
//! `em`/`rem`/viewport-relative lengths need font/viewport context this
//! parser does not have and are likewise skipped.

use amazing_css::effects::{BasicShape, ClipPath, ShapeRadius};
use amazing_css::values::LengthPercentage;

/// A resolved clip mask: an axis-aligned rect with an independent `(rx, ry)`
/// radius per corner, in top-left, top-right, bottom-right, bottom-left
/// order. `inset()` maps directly; `circle()`/`ellipse()` map onto their own
/// bounding box with every corner sharing the one radius pair, which
/// reproduces an exact ellipse through the same per-corner elliptical test
/// used for rounded rectangles elsewhere in this painter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipMask {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radii: [(f32, f32); 4],
}

impl ClipMask {
    /// `reference` is the box this shape's percentages/keywords resolve
    /// against, in the same untransformed surface pixel space as the
    /// group's own decorations: `(x, y, width, height)`.
    pub fn from_css(value: &str, reference: (f32, f32, f32, f32)) -> Option<Self> {
        let clip_path = amazing_css::effects::parse_clip_path(value)?;
        let shape = match clip_path {
            ClipPath::Shape(shape) | ClipPath::ShapeWithBox(shape, _) => shape,
            ClipPath::None | ClipPath::Url(_) | ClipPath::Geometry(_) => return None,
        };
        from_shape(&shape, reference)
    }

    /// Whether `(x, y)`, in the same coordinate space as the mask, is
    /// inside the shape.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if x < self.x || y < self.y || x >= self.x + self.width || y >= self.y + self.height {
            return false;
        }
        let corners = [
            (
                self.x,
                self.y,
                self.radii[0],
                x < self.x + self.radii[0].0 && y < self.y + self.radii[0].1,
            ),
            (
                self.x + self.width,
                self.y,
                self.radii[1],
                x >= self.x + self.width - self.radii[1].0 && y < self.y + self.radii[1].1,
            ),
            (
                self.x + self.width,
                self.y + self.height,
                self.radii[2],
                x >= self.x + self.width - self.radii[2].0
                    && y >= self.y + self.height - self.radii[2].1,
            ),
            (
                self.x,
                self.y + self.height,
                self.radii[3],
                x < self.x + self.radii[3].0 && y >= self.y + self.height - self.radii[3].1,
            ),
        ];
        for (corner_x, corner_y, (radius_x, radius_y), applies) in corners {
            if !applies || radius_x <= 0.0 || radius_y <= 0.0 {
                continue;
            }
            let center_x = if corner_x == self.x { corner_x + radius_x } else { corner_x - radius_x };
            let center_y = if corner_y == self.y { corner_y + radius_y } else { corner_y - radius_y };
            let dx = (x - center_x) / radius_x;
            let dy = (y - center_y) / radius_y;
            return dx * dx + dy * dy <= 1.0;
        }
        true
    }
}

fn resolve(length: LengthPercentage, reference: f32) -> Option<f32> {
    match length {
        LengthPercentage::Px(value) if value.is_finite() => Some(value as f32),
        LengthPercentage::Percentage(value) if value.is_finite() => {
            Some((value / 100.0) as f32 * reference)
        }
        _ => None,
    }
}

/// The CSS four-value shorthand expansion (`1224` rule) shared by `inset()`'s
/// `top right bottom left` offsets and its `round` corner radii: one value
/// applies to all four, two alternate, three repeat the second, four are
/// used as given.
fn expand_four(values: &[LengthPercentage]) -> Option<[LengthPercentage; 4]> {
    match values {
        [a] => Some([*a, *a, *a, *a]),
        [a, b] => Some([*a, *b, *a, *b]),
        [a, b, c] => Some([*a, *b, *c, *b]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

fn resolve_position(
    position: Option<(LengthPercentage, LengthPercentage)>,
    (x0, y0, w, h): (f32, f32, f32, f32),
) -> (f32, f32) {
    let (px, py) = position.unwrap_or((LengthPercentage::Percentage(50.0), LengthPercentage::Percentage(50.0)));
    let cx = resolve(px, w).unwrap_or(w * 0.5);
    let cy = resolve(py, h).unwrap_or(h * 0.5);
    (x0 + cx, y0 + cy)
}

/// `circle()`'s single radius: per CSS Shapes 1, a keyword resolves against
/// the closest/farthest of all four sides (not per axis), and a
/// `<length-percentage>` percentage resolves against
/// `sqrt(width^2 + height^2) / sqrt(2)`.
fn resolve_circle_radius(
    radius: Option<&ShapeRadius>,
    (cx, cy): (f32, f32),
    (x0, y0, w, h): (f32, f32, f32, f32),
) -> Option<f32> {
    let distances = [cx - x0, (x0 + w) - cx, cy - y0, (y0 + h) - cy];
    match radius {
        None | Some(ShapeRadius::ClosestSide) => distances.into_iter().reduce(f32::min),
        Some(ShapeRadius::FarthestSide) => distances.into_iter().reduce(f32::max),
        Some(ShapeRadius::Length(length)) => {
            resolve(*length, (w * w + h * h).sqrt() / std::f32::consts::SQRT_2)
        }
    }
}

/// `ellipse()`'s per-axis radius: a keyword resolves against that axis's own
/// two sides only (horizontal radius against left/right, vertical against
/// top/bottom), independent of the other axis.
fn resolve_ellipse_axis_radius(
    radius: Option<&ShapeRadius>,
    center: f32,
    near_edge: f32,
    far_edge: f32,
) -> Option<f32> {
    let near = (center - near_edge).abs();
    let far = (far_edge - center).abs();
    match radius {
        None | Some(ShapeRadius::ClosestSide) => Some(near.min(far)),
        Some(ShapeRadius::FarthestSide) => Some(near.max(far)),
        Some(ShapeRadius::Length(length)) => resolve(*length, far_edge - near_edge),
    }
}

fn from_shape(shape: &BasicShape, reference @ (x0, y0, w, h): (f32, f32, f32, f32)) -> Option<ClipMask> {
    match shape {
        BasicShape::Inset { offsets, round } => {
            let [top, right, bottom, left] = expand_four(offsets)?;
            let top = resolve(top, h)?;
            let right = resolve(right, w)?;
            let bottom = resolve(bottom, h)?;
            let left = resolve(left, w)?;
            let rect_x = x0 + left;
            let rect_y = y0 + top;
            let rect_w = (w - left - right).max(0.0);
            let rect_h = (h - top - bottom).max(0.0);
            let radii = match round {
                Some(values) => {
                    let [tl, tr, br, bl] = expand_four(values)?;
                    // The parser does not yet split a corner into separate
                    // horizontal/vertical radii (no `/` clause support), so
                    // each corner's one resolved value is shared by both axes.
                    [resolve(tl, w)?, resolve(tr, w)?, resolve(br, w)?, resolve(bl, w)?].map(|r| (r, r))
                }
                None => [(0.0, 0.0); 4],
            };
            Some(ClipMask { x: rect_x, y: rect_y, width: rect_w, height: rect_h, radii })
        }
        BasicShape::Circle { radius, position } => {
            let (cx, cy) = resolve_position(*position, reference);
            let r = resolve_circle_radius(radius.as_ref(), (cx, cy), reference)?;
            (r.is_finite() && r >= 0.0).then(|| ClipMask {
                x: cx - r,
                y: cy - r,
                width: r * 2.0,
                height: r * 2.0,
                radii: [(r, r); 4],
            })
        }
        BasicShape::Ellipse { radii, position } => {
            let (cx, cy) = resolve_position(*position, reference);
            let (horizontal, vertical) = match radii {
                Some((rx, ry)) => (Some(rx), Some(ry)),
                None => (None, None),
            };
            let rx = resolve_ellipse_axis_radius(horizontal, cx, x0, x0 + w)?;
            let ry = resolve_ellipse_axis_radius(vertical, cy, y0, y0 + h)?;
            (rx.is_finite() && ry.is_finite() && rx >= 0.0 && ry >= 0.0).then(|| ClipMask {
                x: cx - rx,
                y: cy - ry,
                width: rx * 2.0,
                height: ry * 2.0,
                radii: [(rx, ry); 4],
            })
        }
        BasicShape::Polygon { .. } | BasicShape::Path { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::ClipMask;

    #[test]
    fn inset_resolves_offsets_and_percentage_round_radius() {
        let mask = ClipMask::from_css("inset(1px 2px 3px 4px round 10%)", (0.0, 0.0, 20.0, 20.0))
            .expect("valid inset");
        assert_eq!(mask.x, 4.0);
        assert_eq!(mask.y, 1.0);
        assert_eq!(mask.width, 20.0 - 4.0 - 2.0);
        assert_eq!(mask.height, 20.0 - 1.0 - 3.0);
        // `round 10%` resolves against the reference box's own width (20px).
        assert_eq!(mask.radii[0], (2.0, 2.0));
    }

    #[test]
    fn inset_single_offset_and_round_value_apply_to_all_four() {
        let mask = ClipMask::from_css("inset(2px round 1px)", (0.0, 0.0, 10.0, 10.0)).expect("valid inset");
        assert_eq!((mask.x, mask.y, mask.width, mask.height), (2.0, 2.0, 6.0, 6.0));
        assert!(mask.radii.iter().all(|&r| r == (1.0, 1.0)));
    }

    #[test]
    fn circle_without_radius_defaults_to_closest_side_and_centers_on_the_box() {
        // A 10x20 box: closest-side from the center is min(5, 10) = 5.
        let mask = ClipMask::from_css("circle()", (0.0, 0.0, 10.0, 20.0)).expect("valid circle");
        assert_eq!((mask.x, mask.y, mask.width, mask.height), (0.0, 5.0, 10.0, 10.0));
        assert_eq!(mask.radii[0], (5.0, 5.0));
    }

    #[test]
    fn circle_with_explicit_radius_and_position_is_an_exact_disc() {
        let mask = ClipMask::from_css("circle(3px at 4px 5px)", (0.0, 0.0, 20.0, 20.0))
            .expect("valid circle");
        assert!(mask.contains(4.0, 5.0), "center is inside");
        assert!(mask.contains(4.0, 5.0 + 2.9), "just inside the radius");
        assert!(!mask.contains(4.0, 5.0 + 3.1), "just outside the radius");
        // The bounding box corners, outside the inscribed circle, are excluded.
        assert!(!mask.contains(4.0 - 3.0, 5.0 - 3.0));
    }

    #[test]
    fn ellipse_with_explicit_radii_uses_independent_axes() {
        let mask = ClipMask::from_css("ellipse(4px 2px at center)", (0.0, 0.0, 20.0, 20.0))
            .expect("valid ellipse");
        assert!(mask.contains(10.0, 10.0));
        assert!(mask.contains(10.0 + 3.9, 10.0));
        assert!(!mask.contains(10.0 + 4.1, 10.0));
        assert!(mask.contains(10.0, 10.0 + 1.9));
        assert!(!mask.contains(10.0, 10.0 + 2.1));
    }

    #[test]
    fn polygon_path_and_bare_geometry_box_are_skipped_not_guessed() {
        assert!(ClipMask::from_css("polygon(0 0, 1px 1px, 2px 0)", (0.0, 0.0, 10.0, 10.0)).is_none());
        assert!(ClipMask::from_css("path(\"M 0 0 L 1 1\")", (0.0, 0.0, 10.0, 10.0)).is_none());
        assert!(ClipMask::from_css("border-box", (0.0, 0.0, 10.0, 10.0)).is_none());
        assert!(ClipMask::from_css("none", (0.0, 0.0, 10.0, 10.0)).is_none());
    }
}
