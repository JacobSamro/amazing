//! CSS Filter Effects 1 and Compositing 1 pixel kernels. Color filters use
//! straight sRGB; spatial filters interpolate premultiplied channels.
use super::*;
use FilterOperation as F;

fn byte(v: f64) -> u8 {
    (v * 255.).round().clamp(0., 255.) as u8
}
pub(crate) fn filters(surface: &Surface, operations: &[FilterOperation]) -> Surface {
    let mut out = surface.clone();
    for op in operations {
        match *op {
            F::Blur(sigma) => out = blur(&out, sigma),
            F::DropShadow {
                offset,
                sigma,
                color,
            } => {
                let alpha = blur(&out, sigma);
                let mut shadow = Surface::new(out.width, out.height);
                for y in 0..out.height {
                    for x in 0..out.width {
                        let a = bilinear(&alpha, x as f32 - offset.0, y as f32 - offset.1)[3];
                        let mut c = color;
                        c[3] = ((u32::from(a) * u32::from(color[3]) + 127) / 255) as u8;
                        shadow.set_pixel(
                            x,
                            y,
                            amazing_paint::blend_source_over(out.get_pixel(x, y).unwrap(), c, 255),
                        );
                    }
                }
                out = shadow;
            }
            op => {
                for pixel in out.pixels.chunks_exact_mut(4) {
                    let r = f64::from(pixel[0]) / 255.;
                    let g = f64::from(pixel[1]) / 255.;
                    let b = f64::from(pixel[2]) / 255.;
                    let c = [r, g, b];
                    let result = match op {
                        F::Brightness(v) => c.map(|x| x * f64::from(v)),
                        F::Contrast(v) => c.map(|x| (x - 0.5) * f64::from(v) + 0.5),
                        F::Invert(v) => {
                            c.map(|x| x * (1. - f64::from(v)) + (1. - x) * f64::from(v))
                        }
                        F::Opacity(v) => {
                            pixel[3] = byte(f64::from(pixel[3]) / 255. * f64::from(v));
                            c
                        }
                        F::Grayscale(v) => {
                            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                            c.map(|x| x + (l - x) * f64::from(v))
                        }
                        F::Sepia(v) => {
                            let s = [
                                0.393 * r + 0.769 * g + 0.189 * b,
                                0.349 * r + 0.686 * g + 0.168 * b,
                                0.272 * r + 0.534 * g + 0.131 * b,
                            ];
                            std::array::from_fn(|i| c[i] + (s[i] - c[i]) * f64::from(v))
                        }
                        F::Saturate(v) => {
                            let s = f64::from(v);
                            [
                                (0.213 + 0.787 * s) * r
                                    + (0.715 - 0.715 * s) * g
                                    + (0.072 - 0.072 * s) * b,
                                (0.213 - 0.213 * s) * r
                                    + (0.715 + 0.285 * s) * g
                                    + (0.072 - 0.072 * s) * b,
                                (0.213 - 0.213 * s) * r
                                    + (0.715 - 0.715 * s) * g
                                    + (0.072 + 0.928 * s) * b,
                            ]
                        }
                        F::HueRotate(angle) => {
                            let (s, c) = f64::from(angle).to_radians().sin_cos();
                            [
                                (0.213 + 0.787 * c - 0.213 * s) * r
                                    + (0.715 - 0.715 * c - 0.715 * s) * g
                                    + (0.072 - 0.072 * c + 0.928 * s) * b,
                                (0.213 - 0.213 * c + 0.143 * s) * r
                                    + (0.715 + 0.285 * c + 0.140 * s) * g
                                    + (0.072 - 0.072 * c - 0.283 * s) * b,
                                (0.213 - 0.213 * c - 0.787 * s) * r
                                    + (0.715 - 0.715 * c + 0.715 * s) * g
                                    + (0.072 + 0.928 * c + 0.072 * s) * b,
                            ]
                        }
                        F::Blur(_) | F::DropShadow { .. } => unreachable!(),
                    };
                    for k in 0..3 {
                        pixel[k] = byte(result[k]);
                    }
                }
            }
        }
    }
    out
}
fn blur(surface: &Surface, sigma: f32) -> Surface {
    blur_edges(surface, sigma, None)
}
pub(crate) fn backdrop_filters(
    surface: &Surface,
    operations: &[FilterOperation],
    bounds: Rect,
) -> Surface {
    let mut out = surface.clone();
    for op in operations {
        out = match op {
            F::Blur(sigma) => blur_edges(&out, *sigma, Some(bounds)),
            _ => filters(&out, &[*op]),
        };
    }
    out
}
fn blur_edges(surface: &Surface, sigma: f32, edges: Option<Rect>) -> Surface {
    if sigma <= 0. {
        return surface.clone();
    }
    let radius = (3. * sigma).ceil() as i32;
    let mut weights: Vec<f64> = (-radius..=radius)
        .map(|x| (-f64::from(x).powi(2) / (2. * f64::from(sigma).powi(2))).exp())
        .collect();
    let sum: f64 = weights.iter().sum();
    for w in &mut weights {
        *w /= sum;
    }
    let (width, height) = (surface.width as usize, surface.height as usize);
    let mirror = |value: i32, lo: i32, hi: i32| {
        let length = hi - lo;
        if length <= 0 {
            return value;
        }
        let v = (value - lo).rem_euclid(2 * length);
        lo + if v < length { v } else { 2 * length - 1 - v }
    };
    let edge = edges.map(|r| {
        (
            r.x.floor().max(0.) as i32,
            r.y.floor().max(0.) as i32,
            (r.x + r.w).ceil().min(width as f32) as i32,
            (r.y + r.h).ceil().min(height as f32) as i32,
        )
    });
    let mut horizontal = vec![[0.; 4]; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut p = [0.; 4];
            for (i, w) in weights.iter().enumerate() {
                let sx = x as i32 + i as i32 - radius;
                let sx = edge.map_or(sx, |(x0, _, x1, _)| mirror(sx, x0, x1));
                if !(0..width as i32).contains(&sx) {
                    continue;
                }
                let c = surface.get_pixel(sx as u32, y as u32).unwrap();
                let a = f64::from(c[3]) / 255.;
                for k in 0..3 {
                    p[k] += f64::from(c[k]) / 255. * a * w;
                }
                p[3] += a * w;
            }
            horizontal[y * width + x] = p;
        }
    }
    let mut out = Surface::new(surface.width, surface.height);
    for y in 0..height {
        for x in 0..width {
            let mut p = [0.; 4];
            for (i, w) in weights.iter().enumerate() {
                let sy = y as i32 + i as i32 - radius;
                let sy = edge.map_or(sy, |(_, y0, _, y1)| mirror(sy, y0, y1));
                if !(0..height as i32).contains(&sy) {
                    continue;
                }
                for k in 0..4 {
                    p[k] += horizontal[sy as usize * width + x][k] * w;
                }
            }
            let c = if p[3] > 0. {
                [
                    byte(p[0] / p[3]),
                    byte(p[1] / p[3]),
                    byte(p[2] / p[3]),
                    byte(p[3]),
                ]
            } else {
                [0; 4]
            };
            out.set_pixel(x as u32, y as u32, c);
        }
    }
    out
}
fn bilinear(surface: &Surface, x: f32, y: f32) -> [u8; 4] {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let mut p = [0.; 4];
    for (dx, dy, w) in [
        (0, 0, (1. - fx) * (1. - fy)),
        (1, 0, fx * (1. - fy)),
        (0, 1, (1. - fx) * fy),
        (1, 1, fx * fy),
    ] {
        if ix + dx < 0 || iy + dy < 0 {
            continue;
        }
        if let Some(c) = surface.get_pixel((ix + dx) as u32, (iy + dy) as u32) {
            let a = f64::from(c[3]) / 255.;
            for k in 0..3 {
                p[k] += f64::from(c[k]) / 255. * a * f64::from(w);
            }
            p[3] += a * f64::from(w);
        }
    }
    if p[3] > 0. {
        [
            byte(p[0] / p[3]),
            byte(p[1] / p[3]),
            byte(p[2] / p[3]),
            byte(p[3]),
        ]
    } else {
        [0; 4]
    }
}
fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn sat(c: [f64; 3]) -> f64 {
    c.into_iter().fold(f64::NEG_INFINITY, f64::max) - c.into_iter().fold(f64::INFINITY, f64::min)
}
fn set_lum(mut c: [f64; 3], l: f64) -> [f64; 3] {
    let d = l - lum(c);
    for v in &mut c {
        *v += d;
    }
    let n = c.into_iter().fold(f64::INFINITY, f64::min);
    let x = c.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if n < 0. {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1. {
        for v in &mut c {
            *v = l + (*v - l) * (1. - l) / (x - l);
        }
    }
    c
}
fn set_sat(mut c: [f64; 3], s: f64) -> [f64; 3] {
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| c[a].total_cmp(&c[b]));
    let [lo, mid, hi] = order;
    if c[hi] > c[lo] {
        c[mid] = (c[mid] - c[lo]) * s / (c[hi] - c[lo]);
        c[hi] = s;
    } else {
        c[mid] = 0.;
        c[hi] = 0.;
    }
    c[lo] = 0.;
    c
}
pub(crate) fn blend(dst: [u8; 4], src: [u8; 4], opacity: f32, mode: EffectBlend) -> [u8; 4] {
    let op = amazing_paint::quantize_opacity(opacity);
    if mode == EffectBlend::Normal {
        return amazing_paint::blend_source_over(src, dst, op);
    }
    let sa = f64::from((u32::from(src[3]) * u32::from(op) + 127) / 255) / 255.;
    let da = f64::from(dst[3]) / 255.;
    if mode == EffectBlend::PlusLighter {
        // Compositing 2 section 9.1.15: add in premultiplied space, clamp
        // color and alpha independently, then return straight RGBA.
        let alpha = (sa + da).min(1.);
        if alpha == 0. { return [0; 4]; }
        let channel = |i: usize| byte(
            (sa * f64::from(src[i]) / 255. + da * f64::from(dst[i]) / 255.)
                .min(1.) / alpha);
        return [channel(0), channel(1), channel(2), byte(alpha)];
    }
    let oa = sa + da * (1. - sa);
    if oa == 0. {
        return [0; 4];
    }
    let s = std::array::from_fn(|i| f64::from(src[i]) / 255.);
    let d = std::array::from_fn(|i| f64::from(dst[i]) / 255.);
    use EffectBlend as B;
    let b = match mode {
        B::Hue => set_lum(set_sat(s, sat(d)), lum(d)),
        B::Saturation => set_lum(set_sat(d, sat(s)), lum(d)),
        B::Color => set_lum(s, lum(d)),
        B::Luminosity => set_lum(d, lum(s)),
        _ => std::array::from_fn(|i| {
            let (s, d) = (s[i], d[i]);
            match mode {
                B::Normal => s,
                B::Multiply => s * d,
                B::Screen => s + d - s * d,
                B::Overlay => {
                    if d <= 0.5 {
                        2. * s * d
                    } else {
                        1. - 2. * (1. - s) * (1. - d)
                    }
                }
                B::Darken => s.min(d),
                B::Lighten => s.max(d),
                B::ColorDodge => {
                    if d == 0. {
                        0.
                    } else if s == 1. {
                        1.
                    } else {
                        (d / (1. - s)).min(1.)
                    }
                }
                B::ColorBurn => {
                    if d == 1. {
                        1.
                    } else if s == 0. {
                        0.
                    } else {
                        1. - ((1. - d) / s).min(1.)
                    }
                }
                B::HardLight => {
                    if s <= 0.5 {
                        2. * s * d
                    } else {
                        1. - 2. * (1. - s) * (1. - d)
                    }
                }
                B::SoftLight => {
                    if s <= 0.5 {
                        d - (1. - 2. * s) * d * (1. - d)
                    } else {
                        let k = if d <= 0.25 {
                            ((16. * d - 12.) * d + 4.) * d
                        } else {
                            d.sqrt()
                        };
                        d + (2. * s - 1.) * (k - d)
                    }
                }
                B::Difference => (d - s).abs(),
                B::Exclusion => d + s - 2. * d * s,
                _ => unreachable!(),
            }
        }),
    };
    let c: [u8; 3] = std::array::from_fn(|i| {
        byte(((1. - sa) * da * d[i] + (1. - da) * sa * s[i] + sa * da * b[i]) / oa)
    });
    [c[0], c[1], c[2], byte(oa)]
}
