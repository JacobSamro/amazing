//! Consumes display-list background descriptors; never walks a layout tree.
use super::*;
use crate::background::{BackgroundPlan, BackgroundSize, BoxArea, Repeat, SizeValue};
use amazing_layout::Rect;

pub(super) fn area(b: &LayoutBox, which: BoxArea) -> RoundedRect {
    let cb = b.content_box;
    let outer = resolve_corner_radii(b, &b.border_box);
    let (rect, top, right, bottom, left) = match which {
        BoxArea::Border => (b.border_box, 0.0, 0.0, 0.0, 0.0),
        BoxArea::Padding => (
            Rect {
                x: cb.x - b.padding.left,
                y: cb.y - b.padding.top,
                w: cb.w + b.padding.left + b.padding.right,
                h: cb.h + b.padding.top + b.padding.bottom,
            },
            b.border.top,
            b.border.right,
            b.border.bottom,
            b.border.left,
        ),
        BoxArea::Content => (
            cb,
            b.border.top + b.padding.top,
            b.border.right + b.padding.right,
            b.border.bottom + b.padding.bottom,
            b.border.left + b.padding.left,
        ),
    };
    RoundedRect {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
        radii: inset_radii(outer, top, right, bottom, left, rect.w, rect.h),
    }
}
fn length(value: SizeValue, reference: f64, font: f64) -> Option<f64> {
    match value {
        SizeValue::Auto => None,
        SizeValue::Px(n) => Some(n as f64),
        SizeValue::Percent(n) => Some(n as f64 * reference),
        SizeValue::Em(n) => Some(n as f64 * font),
        SizeValue::Rem(n) => Some(n as f64 * 16.0),
    }
}
fn dimensions(
    size: BackgroundSize,
    origin: RoundedRect,
    natural: (u32, u32),
    font: f32,
) -> (f64, f64) {
    let (nw, nh) = (natural.0 as f64, natural.1 as f64);
    let (ow, oh) = (origin.width as f64, origin.height as f64);
    match size {
        BackgroundSize::Cover | BackgroundSize::Contain => {
            let factor = if size == BackgroundSize::Cover {
                (ow / nw).max(oh / nh)
            } else {
                (ow / nw).min(oh / nh)
            };
            (nw * factor, nh * factor)
        }
        BackgroundSize::Explicit(w, h) => {
            match (length(w, ow, font as f64), length(h, oh, font as f64)) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, w * nh / nw),
                (None, Some(h)) => (h * nw / nh, h),
                (None, None) => (nw, nh),
            }
        }
    }
}
#[derive(Clone, Copy)]
struct Axis {
    start: f64,
    tile: f64,
    period: f64,
    repeat: bool,
}
impl Axis {
    fn new(
        origin: f64,
        extent: f64,
        tile: f64,
        position: crate::image::ObjectPosition,
        mode: Repeat,
    ) -> Self {
        let positioned =
            origin + (extent - tile) * position.fraction as f64 + position.offset as f64;
        match mode {
            Repeat::Space if (extent / tile).floor() >= 2.0 => {
                let count = (extent / tile).floor();
                Self {
                    start: origin,
                    tile,
                    period: tile + (extent - count * tile) / (count - 1.0),
                    repeat: true,
                }
            }
            Repeat::NoRepeat | Repeat::Space => Self {
                start: positioned,
                tile,
                period: tile,
                repeat: false,
            },
            _ => Self {
                start: positioned,
                tile,
                period: tile,
                repeat: true,
            },
        }
    }
    fn sample(self, coordinate: f64) -> Option<f64> {
        if !self.start.is_finite()
            || !self.tile.is_finite()
            || !self.period.is_finite()
            || self.tile <= 0.0
            || self.period <= 0.0
        {
            return None;
        }
        let offset = coordinate - self.start;
        let local = if self.repeat {
            offset.rem_euclid(self.period)
        } else {
            offset
        };
        (local >= 0.0 && local < self.tile).then_some(local / self.tile)
    }
}

pub(super) fn paint(
    surface: &mut Surface,
    b: &LayoutBox,
    plan: &BackgroundPlan,
    images: &dyn ImageProvider,
    clip: ClipRect,
    budget: &mut usize,
) -> bool {
    if b.table_decoration().suppress_background {
        return true;
    }
    let Some(node) = b.node else {
        return true;
    };
    // First declared image is topmost; the color was already painted beneath all.
    for layer in plan.layers.iter().rev() {
        let origin = area(b, layer.origin);
        let painting = area(b, layer.clip);
        if origin.width <= 0.0 || origin.height <= 0.0 {
            continue;
        }
        // A gradient has no intrinsic dimensions: `auto` resolves to the
        // positioning area and the raster is produced at its used size. A URL
        // layer keeps the provider's decoded pixels and its natural size.
        let gradient_image;
        let tile;
        let image: &amazing_media::Image = if let Some(gradient) = layer.gradient.as_deref() {
            let natural = (
                origin.width.max(1.0).ceil() as u32,
                origin.height.max(1.0).ceil() as u32,
            );
            let (w, h) = dimensions(layer.size, origin, natural, b.font_size);
            if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
                continue;
            }
            let (rw, rh) = (
                (w * clip.lattice.scale).ceil().max(1.0) as u32,
                (h * clip.lattice.scale).ceil().max(1.0) as u32,
            );
            let used = if clip.lattice.scale == 1.0 {
                (rw as f32, rh as f32)
            } else {
                (w as f32, h as f32)
            };
            let Some(rastered) =
                crate::gradient::raster_at_scale(gradient, rw, rh, used, b.font_size,clip.lattice.scale)
            else {
                if clip.lattice.scale != 1.0 {
                    return false;
                }
                continue;
            };
            tile = (f64::from(used.0), f64::from(used.1));
            gradient_image = rastered;
            &gradient_image
        } else if let Some(url) = layer.url.as_deref() {
            let Some(resolved) = images.background_image_with_base(node, url,layer.source_base.as_deref()) else {
                continue;
            };
            tile = (resolved.width as f64, resolved.height as f64);
            resolved
        } else {
            continue;
        };
        let Some(expected) = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|n| n.checked_mul(4))
        else {
            continue;
        };
        if image.width == 0 || image.height == 0 || image.pixels.len() != expected {
            continue;
        }
        let (mut w, mut h) = if layer.gradient.is_some() {
            tile
        } else {
            dimensions(layer.size, origin, (image.width, image.height), b.font_size)
        };
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            continue;
        }
        let (old_w, old_h) = (w, h);
        if layer.repeat_x == Repeat::Round {
            w = origin.width as f64 / (origin.width as f64 / w).round().max(1.0);
        }
        if layer.repeat_y == Repeat::Round {
            h = origin.height as f64 / (origin.height as f64 / h).round().max(1.0);
        }
        if layer.repeat_x == Repeat::Round
            && layer.repeat_y != Repeat::Round
            && matches!(layer.size, BackgroundSize::Explicit(_, SizeValue::Auto))
        {
            h *= w / old_w;
        }
        if layer.repeat_y == Repeat::Round
            && layer.repeat_x != Repeat::Round
            && matches!(layer.size, BackgroundSize::Explicit(SizeValue::Auto, _))
        {
            w *= h / old_h;
        }
        let x_axis = Axis::new(
            origin.x as f64,
            origin.width as f64,
            w,
            layer.position.0,
            layer.repeat_x,
        );
        let y_axis = Axis::new(
            origin.y as f64,
            origin.height as f64,
            h,
            layer.position.1,
            layer.repeat_y,
        );
        let precise_gradient=if clip.lattice.scale!=1.0 {
            if let Some(css)=layer.gradient.as_deref() {
                let Some(prepared)=crate::gradient::PreparedGradient::new(css,(w as f32,h as f32),b.font_size) else {return false;};
                Some(prepared)
            } else {None}
        } else {None};
        let bounds = clip
            .intersect(painting.x, painting.y, painting.width, painting.height)
            .surface_bounds(surface)
            .pixels();
        let [x0, y0, x1, y1] = bounds;
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let count = (x1 - x0) as usize * (y1 - y0) as usize;
        let Some(remaining) = budget.checked_sub(count) else {
            return false;
        };
        *budget = remaining;
        for y in y0..y1 {
            let Some(v) = y_axis.sample(clip.lattice.css_point(0.0, y as f64 + 0.5).1) else {
                continue;
            };
            let iy = (v * image.height as f64)
                .floor()
                .min(image.height as f64 - 1.0) as usize;
            for x in x0..x1 {
                if !{
                    let (cx, cy) = clip.point(x, y);
                    painting.contains(cx, cy)
                } {
                    continue;
                }
                if b.table_decoration().background_holes.iter().any(|hole| {
                    let (px, py) = clip.point(x, y);
                    px >= hole.x && px < hole.x + hole.w && py >= hole.y && py < hole.y + hole.h
                }) {
                    continue;
                }
                let Some(u) = x_axis.sample(clip.lattice.css_point(x as f64 + 0.5, 0.0).0) else {
                    continue;
                };
                let ix = (u * image.width as f64)
                    .floor()
                    .min(image.width as f64 - 1.0) as usize;
                let source=if let Some(gradient)=&precise_gradient {
                    let Some(color)=gradient.sample((u*w) as f32,(v*h) as f32) else {return false;};color
                } else {
                    let offset=(iy*image.width as usize+ix)*4;
                    let source=&image.pixels[offset..offset+4];[source[0],source[1],source[2],source[3]]
                };
                let dest = surface.get_pixel(x as u32, y as u32).unwrap_or([0; 4]);
                surface.set_pixel(x as u32, y as u32, source_over(source, dest, 1.0));
            }
        }
    }
    true
}
