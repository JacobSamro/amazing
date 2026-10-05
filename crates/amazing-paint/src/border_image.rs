//! CSS Backgrounds 3 §5: nine-piece border images from actual decoded pixels.
//! No fetching or invented intrinsic dimensions; unsupported values fall back.
use crate::{Surface, paint::ImageProvider};
use amazing_css::ComputedStyle;
use amazing_layout::{LayoutBox, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Repeat {
    Stretch,
    Repeat,
    Round,
    Space,
}
#[derive(Debug, Clone)]
pub struct BorderImagePlan {
    pub url: Option<String>,
    pub gradient: Option<String>,
    pub source_base: Option<String>,
    slices: [String; 4],
    widths: [String; 4],
    outsets: [String; 4],
    fill: bool,
    repeat: [Repeat; 2],
}
/// Split computed axes without splitting math function arguments.
fn tokens(s: &str) -> Option<Vec<&str>> {
    if s.len() > 65536 {
        return None;
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() && depth == 0 {
            if let Some(a) = start.take() {
                out.push(&s[a..i]);
            }
        } else {
            start.get_or_insert(i);
            if c == '(' {
                depth += 1;
                if depth > 32 {
                    return None;
                }
            }
            if c == ')' {
                depth = depth.checked_sub(1)?;
            }
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(a) = start {
        out.push(&s[a..]);
    }
    Some(out)
}
fn sides(v: &[&str]) -> Option<[String; 4]> {
    let a = *v.first()?;
    Some(
        match v.len() {
            1 => [a, a, a, a],
            2 => [a, v[1], a, v[1]],
            3 => [a, v[1], v[2], v[1]],
            4 => [a, v[1], v[2], v[3]],
            _ => return None,
        }
        .map(str::to_owned),
    )
}
impl BorderImagePlan {
    pub fn from_style(style: &ComputedStyle) -> Option<Self> {
        let source = style.get("border-image-source")?;
        if source.len() > 65536 {
            return None;
        }
        let url = crate::background::url(source);
        let gradient = crate::background::gradient(source);
        if url.is_none() && gradient.is_none() {
            return None;
        }
        let mut slice = tokens(style.get("border-image-slice").unwrap_or("100%"))?;
        let fill = slice.contains(&"fill");
        slice.retain(|s| *s != "fill");
        let r = tokens(style.get("border-image-repeat").unwrap_or("stretch"))?;
        if r.is_empty() || r.len() > 2 {
            return None;
        }
        let mode = |s: &str| {
            Some(match s {
                "stretch" => Repeat::Stretch,
                "repeat" => Repeat::Repeat,
                "round" => Repeat::Round,
                "space" => Repeat::Space,
                _ => return None,
            })
        };
        Some(Self {
            url,
            gradient,
            source_base: style.url_base("border-image-source").map(str::to_owned),
            slices: sides(&slice)?,
            widths: sides(&tokens(style.get("border-image-width").unwrap_or("1"))?)?,
            outsets: sides(&tokens(style.get("border-image-outset").unwrap_or("0"))?)?,
            fill,
            repeat: [mode(r[0])?, mode(*r.get(1).unwrap_or(&r[0]))?],
        })
    }
    pub fn ink_bounds(&self, b: &LayoutBox) -> Option<Rect> {
        let z = b.style.effective_zoom();
        if !z.is_finite() || z <= 0.0 {
            return None;
        }
        let borders = [b.border.top, b.border.right, b.border.bottom, b.border.left];
        let mut o = [0.0; 4];
        for i in 0..4 {
            o[i] = if let Ok(n) = self.outsets[i].parse::<f64>() {
                n * borders[i] as f64
            } else {
                amazing_css::used_position_axis(&self.outsets[i], 0.0)? * z
            };
            if !o[i].is_finite() || o[i] < 0.0 {
                return None;
            }
        }
        let r = b.border_box;
        let out = Rect {
            x: r.x - o[3] as f32,
            y: r.y - o[0] as f32,
            w: r.w + (o[1] + o[3]) as f32,
            h: r.h + (o[0] + o[2]) as f32,
        };
        [out.x, out.y, out.w, out.h]
            .iter()
            .all(|v| v.is_finite())
            .then_some(out)
    }
    fn geometry(&self, b: &LayoutBox, natural: (u32, u32)) -> Option<Geometry> {
        let area = self.ink_bounds(b)?;
        if area.w <= 0.0 || area.h <= 0.0 {
            return None;
        }
        let nw = natural.0 as f64;
        let nh = natural.1 as f64;
        if nw <= 0.0 || nh <= 0.0 {
            return None;
        }
        let z = b.style.effective_zoom();
        let mut s = [0.0; 4];
        let mut w = [0.0; 4];
        let borders = [b.border.top, b.border.right, b.border.bottom, b.border.left];
        for i in 0..4 {
            let dim = if i % 2 == 0 { nh } else { nw };
            s[i] = if let Ok(n) = self.slices[i].parse::<f64>() {
                if self.gradient.is_some() {
                    n * z
                } else {
                    n
                }
            } else {
                amazing_css::used_position_axis(&self.slices[i], dim)?
            };
            if !s[i].is_finite() || s[i] < 0.0 {
                return None;
            }
            s[i] = s[i].min(dim);
            let basis = if i % 2 == 0 { area.h } else { area.w } as f64;
            w[i] = if self.widths[i] == "auto" {
                if self.gradient.is_some() {
                    borders[i] as f64
                } else {
                    s[i] * z
                }
            } else if let Ok(n) = self.widths[i].parse::<f64>() {
                n * borders[i] as f64
            } else {
                amazing_css::used_position_axis(&self.widths[i], basis / z)? * z
            };
            if !w[i].is_finite() || w[i] < 0.0 {
                return None;
            }
        }
        let factor = (area.w as f64 / (w[1] + w[3]))
            .min(area.h as f64 / (w[0] + w[2]))
            .min(1.0);
        if factor.is_finite() {
            for n in &mut w {
                *n *= factor;
            }
        }
        Some(Geometry {
            area,
            sx: [0.0, s[3], nw - s[1], nw],
            sy: [0.0, s[0], nh - s[2], nh],
            dx: [
                area.x as f64,
                area.x as f64 + w[3],
                (area.x + area.w) as f64 - w[1],
                (area.x + area.w) as f64,
            ],
            dy: [
                area.y as f64,
                area.y as f64 + w[0],
                (area.y + area.h) as f64 - w[2],
                (area.y + area.h) as f64,
            ],
            w,
        })
    }
}
struct Geometry {
    area: Rect,
    sx: [f64; 4],
    sy: [f64; 4],
    dx: [f64; 4],
    dy: [f64; 4],
    w: [f64; 4],
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Outcome {
    Unavailable,
    Painted,
    BudgetExceeded,
    RasterFailure,
}
#[derive(Clone, Copy)]
struct Axis {
    extent: f64,
    tile: f64,
    start: f64,
    period: f64,
    mode: Repeat,
}
impl Axis {
    fn new(extent: f64, tile: f64, mode: Repeat) -> Self {
        let (tile, start, period) = match mode {
            Repeat::Stretch => (extent, 0.0, extent),
            Repeat::Round => {
                let t = extent / (extent / tile).round().max(1.0);
                (t, 0.0, t)
            }
            Repeat::Repeat => (tile, (extent - tile) / 2.0, tile),
            Repeat::Space => {
                let n = (extent / tile).floor();
                if n >= 1.0 {
                    let gap = (extent - n * tile) / (n + 1.0);
                    (tile, gap, tile + gap)
                } else {
                    (0.0, 0.0, 0.0)
                }
            }
        };
        Self {
            extent,
            tile,
            start,
            period,
            mode,
        }
    }
    fn sample(self, p: f64) -> Option<f64> {
        if !self.tile.is_finite()
            || self.tile <= 0.0
            || self.period <= 0.0
            || p < 0.0
            || p >= self.extent
        {
            return None;
        }
        let local = match self.mode {
            Repeat::Repeat => (p - self.start).rem_euclid(self.period),
            Repeat::Round | Repeat::Stretch => p.rem_euclid(self.period),
            Repeat::Space => {
                if p < self.start {
                    return None;
                }
                (p - self.start).rem_euclid(self.period)
            }
        };
        (local < self.tile).then_some(local / self.tile)
    }
}
pub(crate) fn paint(
    surface: &mut Surface,
    b: &LayoutBox,
    images: &dyn ImageProvider,
    clip: [i64; 4],
    budget: &mut usize,
) -> Outcome {
    paint_at_lattice(
        surface,
        b,
        images,
        clip,
        budget,
        crate::raster_viewport::RasterLattice::UNIT,
    )
}
pub(crate) fn paint_at_lattice(
    surface: &mut Surface,
    b: &LayoutBox,
    images: &dyn ImageProvider,
    clip: [i64; 4],
    budget: &mut usize,
    lattice: crate::raster_viewport::RasterLattice,
) -> Outcome {
    if b.table_decoration().suppress_border {
        return Outcome::Unavailable;
    }
    let Some(plan) = BorderImagePlan::from_style(&b.style) else {
        return Outcome::Unavailable;
    };
    let generated;
    let image = if let Some(css) = &plan.gradient {
        let Some(area) = plan.ink_bounds(b) else {
            return Outcome::Unavailable;
        };
        let (w, h) = (
            (f64::from(area.w) * lattice.scale).ceil().max(1.0) as u32,
            (f64::from(area.h) * lattice.scale).ceil().max(1.0) as u32,
        );
        let Some(cost) = (w as usize).checked_mul(h as usize) else {
            return Outcome::BudgetExceeded;
        };
        if cost > *budget {
            return Outcome::BudgetExceeded;
        }
        *budget -= cost;
        let Some(img) = crate::gradient::raster_at_scale(css, w, h, if lattice.scale==1.0 {(w as f32,h as f32)} else {(area.w,area.h)}, b.font_size,lattice.scale)
        else {
            // Unit entrypoints keep their established ordinary-border fallback.
            // A checked device raster must not certify an omitted generated image.
            return if lattice.scale == 1.0 {
                Outcome::Unavailable
            } else {
                Outcome::RasterFailure
            };
        };
        generated = img;
        &generated
    } else {
        let Some(img) = images.border_image(
            b.node,
            plan.url.as_deref().unwrap_or(""),
            plan.source_base.as_deref(),
        ) else {
            return Outcome::Unavailable;
        };
        img
    };
    if (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(4))
        != Some(image.pixels.len())
    {
        return Outcome::Unavailable;
    }
    let density = if plan.gradient.is_some() {
        lattice.scale
    } else {
        1.0
    };
    let Some(g) = plan.geometry(
        b,
        (
            (f64::from(image.width) / density).ceil() as u32,
            (f64::from(image.height) / density).ceil() as u32,
        ),
    ) else {
        return Outcome::Unavailable;
    };
    let bounds = lattice.bounds([g.area.x, g.area.y, g.area.x + g.area.w, g.area.y + g.area.h]);
    let x0 = clip[0].max(0).max(bounds[0]);
    let y0 = clip[1].max(0).max(bounds[1]);
    let x1 = clip[2].min(i64::from(surface.width)).min(bounds[2]);
    let y1 = clip[3].min(i64::from(surface.height)).min(bounds[3]);
    let cost = (x1 - x0).max(0) as usize * (y1 - y0).max(0) as usize;
    if cost > *budget {
        return Outcome::BudgetExceeded;
    }
    *budget -= cost;
    for row in 0..3 {
        for col in 0..3 {
            if row == 1 && col == 1 && !plan.fill {
                continue;
            }
            let sw = g.sx[col + 1] - g.sx[col];
            let sh = g.sy[row + 1] - g.sy[row];
            let dw = g.dx[col + 1] - g.dx[col];
            let dh = g.dy[row + 1] - g.dy[row];
            if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
                continue;
            }
            let scale_x = if row != 1 {
                dh / sh
            } else if g.sy[1] > 0.0 && g.w[0] > 0.0 {
                g.w[0] / g.sy[1]
            } else if image.height as f64 - g.sy[2] > 0.0 && g.w[2] > 0.0 {
                g.w[2] / (image.height as f64 - g.sy[2])
            } else {
                1.0
            };
            let scale_y = if col != 1 {
                dw / sw
            } else if g.sx[1] > 0.0 && g.w[3] > 0.0 {
                g.w[3] / g.sx[1]
            } else if image.width as f64 - g.sx[2] > 0.0 && g.w[1] > 0.0 {
                g.w[1] / (image.width as f64 - g.sx[2])
            } else {
                1.0
            };
            let ax = Axis::new(
                dw,
                sw * scale_x,
                if col == 1 {
                    plan.repeat[0]
                } else {
                    Repeat::Stretch
                },
            );
            let ay = Axis::new(
                dh,
                sh * scale_y,
                if row == 1 {
                    plan.repeat[1]
                } else {
                    Repeat::Stretch
                },
            );
            for y in y0..y1 {
                let py = lattice.css_point(0.0, y as f64 + 0.5).1 - g.dy[row];
                let Some(v) = ay.sample(py) else {
                    continue;
                };
                for x in x0..x1 {
                    let px = lattice.css_point(x as f64 + 0.5, 0.0).0 - g.dx[col];
                    let Some(u) = ax.sample(px) else {
                        continue;
                    };
                    let sx = ((g.sx[col] + u * sw) * density)
                        .floor()
                        .min(image.width as f64 - 1.0) as usize;
                    let sy = ((g.sy[row] + v * sh) * density)
                        .floor()
                        .min(image.height as f64 - 1.0) as usize;
                    let i = (sy * image.width as usize + sx) * 4;
                    let src = image.pixels[i..i + 4].try_into().unwrap();
                    let dst = surface.get_pixel(x as u32, y as u32).unwrap();
                    surface.set_pixel(x as u32, y as u32, crate::blend_source_over(src, dst, 255));
                }
            }
        }
    }
    Outcome::Painted
}
#[cfg(test)]
#[path = "border_image_tests.rs"]
mod tests;
