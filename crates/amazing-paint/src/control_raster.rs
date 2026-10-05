//! Rasterization of flattened control commands. No layout traversal or text metrics.
use super::*;
use crate::controls::{ControlContent, ControlPaint};
use amazing_layout::replaced::ReplacedKind;

pub(super) fn paint(
    surface: &mut Surface,
    control: &ControlPaint,
    content: &ControlContent<'_>,
    images: &dyn ImageProvider,
    clip: ClipRect,
    budget: &mut usize,
) -> bool {
    let b = &control.geometry;
    let color = resolve_text_color(&b.style);
    let appearance = b
        .style
        .get("appearance")
        .or_else(|| b.style.get("-webkit-appearance"));
    let native = control.kind != ReplacedKind::Svg
        && !appearance.is_some_and(|v| v.eq_ignore_ascii_case("none"));
    let author_background = ["background-color", "background-image"]
        .iter()
        .any(|p| b.style.is_author_specified(p))
        || resolve_color(&b.style, "background-color", color).is_some_and(|c| c[3] != 0)
        || control.background.layers.iter().any(|l| l.url.is_some());
    let author_border = ["top", "right", "bottom", "left"].iter().any(|side| {
        ["width", "style", "color"].iter().any(|part| {
            b.style
                .is_author_specified(&format!("border-{side}-{part}"))
        })
    }) || [b.border.top, b.border.right, b.border.bottom, b.border.left]
        .iter()
        .any(|v| *v > 0.0);
    paint_box_shadow(surface, b, color, clip);
    if native
        && !native_chrome(
            surface,
            control,
            content,
            clip,
            budget,
            !author_background,
            !author_border,
            false,
        )
    {
        return false;
    }
    paint_background(surface, b, color, control.background.color_clip, clip);
    if !background_raster::paint(surface, b, &control.background, images, clip, budget) {
        return false;
    }
    paint_inset_box_shadow(surface, b, color, clip);
    paint_borders(surface, b, color, clip);
    if native && !native_chrome(surface, control, content, clip, budget, false, false, true) {
        return false;
    }
    true
}

fn native_chrome(
    surface: &mut Surface,
    control: &ControlPaint,
    state: &ControlContent<'_>,
    clip: ClipRect,
    budget: &mut usize,
    fill: bool,
    border: bool,
    indicator: bool,
) -> bool {
    let b = &control.geometry;
    let r = b.border_box;
    if ![r.x, r.y, r.w, r.h].iter().all(|n| n.is_finite()) || r.w <= 0.0 || r.h <= 0.0 {
        return true;
    }
    let bounds = clip
        .intersect(r.x, r.y, r.w, r.h)
        .surface_bounds(surface)
        .pixels();
    let [x0, y0, x1, y1] = bounds;
    if x1 <= x0 || y1 <= y0 {
        return true;
    }
    let count = (x1 - x0) as usize * (y1 - y0) as usize;
    let Some(remaining) = budget.checked_sub(count) else {
        return false;
    };
    *budget = remaining;
    let disabled = state.disabled;
    let edge = if disabled {
        [160, 160, 160, 255]
    } else {
        [100, 100, 100, 255]
    };
    let accent = b
        .style
        .get("accent-color")
        .and_then(parse_css_color)
        .unwrap_or([0, 120, 212, 255]);
    let accent = if disabled {
        [160, 160, 160, 255]
    } else {
        accent
    };
    let background = if disabled {
        [240, 240, 240, 255]
    } else if matches!(control.kind, ReplacedKind::Button | ReplacedKind::Select) {
        [239, 239, 239, 255]
    } else {
        [255; 4]
    };
    let circle = control.kind == ReplacedKind::Radio;
    let side = r.w.min(r.h);
    let cx = r.x + r.w * 0.5;
    let cy = r.y + r.h * 0.5;
    let outer = if circle {
        RoundedRect {
            x: cx - side * 0.5,
            y: cy - side * 0.5,
            width: side,
            height: side,
            radii: radius(side * 0.5),
        }
    } else {
        let authored = resolve_corner_radii(b, &r);
        RoundedRect {
            x: r.x,
            y: r.y,
            width: r.w,
            height: r.h,
            radii: if authored == CornerRadii::default() {
                radius(2.0_f32.min(side * 0.5))
            } else {
                authored
            },
        }
    };
    let inner = RoundedRect {
        x: outer.x + 1.0,
        y: outer.y + 1.0,
        width: (outer.width - 2.0).max(0.0),
        height: (outer.height - 2.0).max(0.0),
        radii: inset_radii(
            outer.radii,
            1.0,
            1.0,
            1.0,
            1.0,
            (outer.width - 2.0).max(0.0),
            (outer.height - 2.0).max(0.0),
        ),
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = clip.point(x, y);
            if !outer.contains(px, py) {
                continue;
            }
            let ink = if indicator {
                match control.kind {
                    ReplacedKind::Checkbox if state.checked => {
                        let u = (px - r.x) / r.w;
                        let v = (py - r.y) / r.h;
                        (segment(u, v, 0.20, 0.50, 0.43, 0.72) < 0.085
                            || segment(u, v, 0.43, 0.72, 0.80, 0.27) < 0.085)
                            .then_some(accent)
                    }
                    ReplacedKind::Radio if state.checked => {
                        (((px - cx).powi(2) + (py - cy).powi(2)) < (side * 0.25).powi(2))
                            .then_some(accent)
                    }
                    ReplacedKind::Select => {
                        let cb = b.content_box;
                        let size = (cb.h * 0.22).min(4.0).min(cb.w * 0.15);
                        let rtl = b.style.get("direction").is_some_and(|v| v == "rtl");
                        let arrow_x = if rtl {
                            cb.x + size + 3.0
                        } else {
                            cb.x + cb.w - size - 3.0
                        };
                        let arrow_y = cb.y + cb.h * 0.5;
                        let dy = py - (arrow_y - size * 0.5);
                        (dy >= 0.0 && dy < size && (px - arrow_x).abs() <= size - dy)
                            .then_some(edge)
                    }
                    _ => None,
                }
            } else if border && !inner.contains(px, py) {
                Some(edge)
            } else if fill {
                Some(background)
            } else {
                None
            };
            if let Some(ink) = ink {
                let dest = surface.get_pixel(x as u32, y as u32).unwrap_or([0; 4]);
                surface.set_pixel(x as u32, y as u32, source_over(ink, dest, 1.0));
            }
        }
    }
    true
}
fn radius(r: f32) -> CornerRadii {
    CornerRadii {
        top_left: (r, r),
        top_right: (r, r),
        bottom_left: (r, r),
        bottom_right: (r, r),
    }
}
fn segment(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let t = ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy);
    let t = t.clamp(0.0, 1.0);
    ((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt()
}
