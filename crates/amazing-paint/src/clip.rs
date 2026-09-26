//! Bounded painter reference for CSS `clip-path` basic shapes.
//!
//! `inset()`, `circle()`, `ellipse()`, `polygon()` and straight-edged
//! `path()` resolve to a rasterizable mask; a `url()` reference and a bare
//! `<geometry-box>` with no shape are skipped rather than guessed. Every
//! shape resolves its percentages, keywords (`closest-side`,
//! `farthest-side`, position keywords) and `at <position>` against the
//! box's own border box, regardless of an explicit `<geometry-box>`: this
//! bounded resolver does not track a separate padding/margin reference box.
//! `em`/`rem`/viewport-relative lengths need font/viewport context this
//! parser does not have and are likewise skipped. `polygon()`/`path()`
//! vertices are capped at [`MAX_CLIP_POINTS`] and stored in a fixed array so
//! the mask stays `Copy`, like the paint command and effect-node records it
//! lives in; a shape with more vertices, or a `path()` using a curve or arc
//! command, is skipped rather than approximated.

use amazing_css::effects::{BasicShape, ClipPath, FillRule, ShapeRadius};
use amazing_css::values::LengthPercentage;

/// The vertex cap for `polygon()`/`path()` masks (see the module doc).
pub const MAX_CLIP_POINTS: usize = 32;

/// A resolved clip mask.
///
/// `Rect` is an axis-aligned rect with an independent `(rx, ry)` radius per
/// corner, in top-left, top-right, bottom-right, bottom-left order.
/// `inset()` maps directly; `circle()`/`ellipse()` map onto their own
/// bounding box with every corner sharing the one radius pair, which
/// reproduces an exact ellipse through the same per-corner elliptical test
/// used for rounded rectangles elsewhere in this painter.
///
/// `Polygon` is a closed vertex loop tested by winding rule (`polygon()`'s
/// own `fill_rule`, or `path()`'s), used for both `polygon()` and a
/// straight-edged `path()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipMask {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radii: [(f32, f32); 4],
    },
    Polygon {
        points: [(f32, f32); MAX_CLIP_POINTS],
        len: u8,
        even_odd: bool,
    },
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
        match *self {
            ClipMask::Rect { x: rx, y: ry, width, height, radii } => {
                rect_contains(rx, ry, width, height, radii, x, y)
            }
            ClipMask::Polygon { points, len, even_odd } => {
                point_in_polygon(&points[..len as usize], x, y, even_odd)
            }
        }
    }
}

fn rect_contains(
    self_x: f32,
    self_y: f32,
    width: f32,
    height: f32,
    radii: [(f32, f32); 4],
    x: f32,
    y: f32,
) -> bool {
    if x < self_x || y < self_y || x >= self_x + width || y >= self_y + height {
        return false;
    }
    let corners = [
        (
            self_x,
            self_y,
            radii[0],
            x < self_x + radii[0].0 && y < self_y + radii[0].1,
        ),
        (
            self_x + width,
            self_y,
            radii[1],
            x >= self_x + width - radii[1].0 && y < self_y + radii[1].1,
        ),
        (
            self_x + width,
            self_y + height,
            radii[2],
            x >= self_x + width - radii[2].0 && y >= self_y + height - radii[2].1,
        ),
        (
            self_x,
            self_y + height,
            radii[3],
            x < self_x + radii[3].0 && y >= self_y + height - radii[3].1,
        ),
    ];
    for (corner_x, corner_y, (radius_x, radius_y), applies) in corners {
        if !applies || radius_x <= 0.0 || radius_y <= 0.0 {
            continue;
        }
        let center_x = if corner_x == self_x { corner_x + radius_x } else { corner_x - radius_x };
        let center_y = if corner_y == self_y { corner_y + radius_y } else { corner_y - radius_y };
        let dx = (x - center_x) / radius_x;
        let dy = (y - center_y) / radius_y;
        return dx * dx + dy * dy <= 1.0;
    }
    true
}

/// Point-in-polygon by winding number. `even_odd` selects CSS Shapes'
/// `evenodd` rule (parity of crossings); otherwise `nonzero` (a nonzero
/// signed winding count) is used, matching `polygon()`/`path()`'s default.
/// Shared with `amazing_paint`'s raster mask so `clip-path` composites
/// through the same tested containment check regardless of shape.
pub(crate) fn point_in_polygon(points: &[(f32, f32)], x: f32, y: f32, even_odd: bool) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut winding = 0i32;
    let mut crossings = 0u32;
    for i in 0..points.len() {
        let (x0, y0) = points[i];
        let (x1, y1) = points[(i + 1) % points.len()];
        if (y0 <= y) != (y1 <= y) {
            let t = (y - y0) / (y1 - y0);
            let cross_x = x0 + t * (x1 - x0);
            if cross_x > x {
                crossings += 1;
                winding += if y1 > y0 { 1 } else { -1 };
            }
        }
    }
    if even_odd { crossings % 2 == 1 } else { winding != 0 }
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
            Some(ClipMask::Rect { x: rect_x, y: rect_y, width: rect_w, height: rect_h, radii })
        }
        BasicShape::Circle { radius, position } => {
            let (cx, cy) = resolve_position(*position, reference);
            let r = resolve_circle_radius(radius.as_ref(), (cx, cy), reference)?;
            (r.is_finite() && r >= 0.0).then(|| ClipMask::Rect {
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
            (rx.is_finite() && ry.is_finite() && rx >= 0.0 && ry >= 0.0).then(|| ClipMask::Rect {
                x: cx - rx,
                y: cy - ry,
                width: rx * 2.0,
                height: ry * 2.0,
                radii: [(rx, ry); 4],
            })
        }
        BasicShape::Polygon { fill_rule, vertices } => {
            if vertices.len() < 3 || vertices.len() > MAX_CLIP_POINTS {
                return None;
            }
            let mut points = [(0.0, 0.0); MAX_CLIP_POINTS];
            for (slot, (vx, vy)) in points.iter_mut().zip(vertices) {
                *slot = (x0 + resolve(*vx, w)?, y0 + resolve(*vy, h)?);
            }
            Some(ClipMask::Polygon {
                points,
                len: vertices.len() as u8,
                even_odd: matches!(fill_rule, Some(FillRule::EvenOdd)),
            })
        }
        BasicShape::Path { fill_rule, path } => {
            let points = parse_straight_path(path)?;
            Some(ClipMask::Polygon {
                points: {
                    let mut array = [(0.0, 0.0); MAX_CLIP_POINTS];
                    for (slot, (px, py)) in array.iter_mut().zip(&points) {
                        *slot = (x0 + px, y0 + py);
                    }
                    array
                },
                len: points.len() as u8,
                even_odd: matches!(fill_rule, Some(FillRule::EvenOdd)),
            })
        }
    }
}

/// A bounded `path()` data parser: only the straight-edged commands
/// (`M`/`L`/`H`/`V`/`Z`, absolute or relative) are supported, capped at
/// [`MAX_CLIP_POINTS`] vertices. A curve/arc command (`C`/`S`/`Q`/`T`/`A`),
/// more than one subpath, too many vertices, or malformed syntax is skipped
/// rather than approximated, matching this module's existing policy for
/// unsupported filter functions and shapes.
fn parse_straight_path(data: &str) -> Option<Vec<(f32, f32)>> {
    let mut chars = data.trim().chars().peekable();
    let mut points: Vec<(f32, f32)> = Vec::new();
    let mut cursor = (0.0f32, 0.0f32);
    let mut start = None;
    let mut closed = false;
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let Some(&command) = chars.peek() else { break };
        if !command.is_ascii_alphabetic() {
            return None;
        }
        chars.next();
        let relative = command.is_ascii_lowercase();
        let take_number = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| -> Option<f32> {
            while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
                chars.next();
            }
            let mut text = String::new();
            if chars.peek() == Some(&'-') || chars.peek() == Some(&'+') {
                text.push(chars.next().unwrap());
            }
            let mut seen_dot = false;
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || (c == '.' && !seen_dot) {
                    seen_dot |= c == '.';
                    text.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            text.parse::<f32>().ok().filter(|value| value.is_finite())
        };
        match command.to_ascii_uppercase() {
            'M' => {
                let x = take_number(&mut chars)?;
                let y = take_number(&mut chars)?;
                cursor = if relative { (cursor.0 + x, cursor.1 + y) } else { (x, y) };
                if start.is_some() && !closed {
                    // A second, unclosed subpath is a shape this bounded
                    // resolver does not represent (only one closed loop).
                    return None;
                }
                start = Some(cursor);
                closed = false;
                points.push(cursor);
            }
            'L' => {
                let x = take_number(&mut chars)?;
                let y = take_number(&mut chars)?;
                cursor = if relative { (cursor.0 + x, cursor.1 + y) } else { (x, y) };
                points.push(cursor);
            }
            'H' => {
                let x = take_number(&mut chars)?;
                cursor = (if relative { cursor.0 + x } else { x }, cursor.1);
                points.push(cursor);
            }
            'V' => {
                let y = take_number(&mut chars)?;
                cursor = (cursor.0, if relative { cursor.1 + y } else { y });
                points.push(cursor);
            }
            'Z' => {
                closed = true;
                if let Some(first) = start {
                    cursor = first;
                }
            }
            _ => return None,
        }
        if points.len() > MAX_CLIP_POINTS {
            return None;
        }
    }
    if points.len() < 3 {
        return None;
    }
    Some(points)
}

#[cfg(test)]
mod tests {
    use super::ClipMask;

    /// Unwraps the `Rect` variant for the inset/circle/ellipse tests, which
    /// predate `polygon()`/`path()` and were written against plain fields.
    fn rect(mask: ClipMask) -> (f32, f32, f32, f32, [(f32, f32); 4]) {
        match mask {
            ClipMask::Rect { x, y, width, height, radii } => (x, y, width, height, radii),
            ClipMask::Polygon { .. } => panic!("expected a Rect mask, got a Polygon"),
        }
    }

    #[test]
    fn inset_resolves_offsets_and_percentage_round_radius() {
        let mask = ClipMask::from_css("inset(1px 2px 3px 4px round 10%)", (0.0, 0.0, 20.0, 20.0))
            .expect("valid inset");
        let (x, y, width, height, radii) = rect(mask);
        assert_eq!(x, 4.0);
        assert_eq!(y, 1.0);
        assert_eq!(width, 20.0 - 4.0 - 2.0);
        assert_eq!(height, 20.0 - 1.0 - 3.0);
        // `round 10%` resolves against the reference box's own width (20px).
        assert_eq!(radii[0], (2.0, 2.0));
    }

    #[test]
    fn inset_single_offset_and_round_value_apply_to_all_four() {
        let mask = ClipMask::from_css("inset(2px round 1px)", (0.0, 0.0, 10.0, 10.0)).expect("valid inset");
        let (x, y, width, height, radii) = rect(mask);
        assert_eq!((x, y, width, height), (2.0, 2.0, 6.0, 6.0));
        assert!(radii.iter().all(|&r| r == (1.0, 1.0)));
    }

    #[test]
    fn circle_without_radius_defaults_to_closest_side_and_centers_on_the_box() {
        // A 10x20 box: closest-side from the center is min(5, 10) = 5.
        let mask = ClipMask::from_css("circle()", (0.0, 0.0, 10.0, 20.0)).expect("valid circle");
        let (x, y, width, height, radii) = rect(mask);
        assert_eq!((x, y, width, height), (0.0, 5.0, 10.0, 10.0));
        assert_eq!(radii[0], (5.0, 5.0));
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
    fn path_and_a_bare_geometry_box_are_skipped_not_guessed() {
        assert!(ClipMask::from_css("border-box", (0.0, 0.0, 10.0, 10.0)).is_none());
        assert!(ClipMask::from_css("none", (0.0, 0.0, 10.0, 10.0)).is_none());
        // A curve command makes this bounded straight-edge-only path() parser skip it.
        assert!(ClipMask::from_css("path(\"M 0 0 C 1 1 2 2 3 3 Z\")", (0.0, 0.0, 10.0, 10.0)).is_none());
        // More vertices than MAX_CLIP_POINTS is skipped, not truncated.
        let many = (0..super::MAX_CLIP_POINTS + 1).map(|i| format!("{i}px {i}px")).collect::<Vec<_>>().join(", ");
        assert!(ClipMask::from_css(&format!("polygon({many})"), (0.0, 0.0, 10.0, 10.0)).is_none());
    }

    #[test]
    fn polygon_clips_to_the_triangle_by_default_nonzero_winding() {
        // A right triangle spanning the box's diagonal half.
        let mask = ClipMask::from_css("polygon(0 0, 10px 0, 0 10px)", (0.0, 0.0, 10.0, 10.0))
            .expect("valid polygon");
        assert!(mask.contains(1.0, 1.0), "near the right-angle corner, inside");
        assert!(!mask.contains(9.0, 9.0), "opposite corner, outside the triangle");
        assert!(mask.contains(4.0, 4.0), "near the hypotenuse, still inside");
        assert!(!mask.contains(6.0, 6.0), "past the hypotenuse, outside");
    }

    #[test]
    fn polygon_resolves_percentages_against_the_reference_box() {
        let mask = ClipMask::from_css("polygon(0% 0%, 100% 0%, 100% 100%, 0% 100%)", (10.0, 20.0, 40.0, 30.0))
            .expect("valid polygon");
        // The box's whole border box, offset by the reference box's origin.
        assert!(mask.contains(30.0, 35.0), "box center is inside");
        assert!(!mask.contains(5.0, 35.0), "left of the box is outside");
    }

    #[test]
    fn path_with_only_straight_commands_clips_to_the_closed_outline() {
        let mask = ClipMask::from_css("path(\"M 0 0 L 10 0 L 10 10 L 0 10 Z\")", (0.0, 0.0, 10.0, 10.0))
            .expect("valid straight-edged path");
        assert!(mask.contains(5.0, 5.0));
        assert!(!mask.contains(15.0, 15.0));
    }

    #[test]
    fn path_evenodd_fill_rule_carries_through() {
        // `evenodd` on a simple non-self-intersecting square behaves the same
        // as `nonzero`; this only proves the fill-rule keyword parses and is
        // threaded onto the mask (winding-rule divergence needs a
        // self-intersecting path, out of scope for this straight-edge subset).
        let mask = ClipMask::from_css("path(evenodd, \"M 0 0 L 10 0 L 10 10 L 0 10 Z\")", (0.0, 0.0, 10.0, 10.0))
            .expect("valid path");
        assert!(matches!(mask, ClipMask::Polygon { even_odd: true, .. }));
    }
}
