//! Bounded painter reference for CSS `filter`/`backdrop-filter` functions.
//!
//! `filter` on an element applies to the element's whole rendered subtree, so
//! the painter applies the plan to its effect-group layer before compositing;
//! `backdrop-filter` applies to a snapshot of the content already painted
//! behind the element's border box, taken before the element's own content is
//! drawn (see [`crate::paint`]'s group setup). `url()` and `calc()` filter
//! functions are accepted by CSS but are not part of this reference: they are
//! skipped rather than guessed, and so are lengths this bounded parser cannot
//! resolve without layout/font context (percentages, `em`/`rem`/viewport
//! units on `blur()`/`drop-shadow()` offsets). The plan is `Copy` with a fixed
//! cap so it can live inside the copyable paint command and effect-node
//! records.

use amazing_css::effects::{FilterFunction, FilterList};
use amazing_css::values::LengthPercentage;

use crate::surface::Surface;

pub const MAX_FILTERS: usize = 8;

/// Bounds the box-blur radius so a pathological `blur()`/`drop-shadow()`
/// standard deviation cannot make one filter pass unbounded work.
const MAX_BLUR_STD: f32 = 250.0;

/// One supported filter function with its resolved amount. Lengths are
/// resolved to device pixels by the caller; colors are resolved (including
/// `currentcolor`) before the op is constructed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterOp {
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    /// Gaussian blur, standard deviation in device pixels (the function's
    /// own argument, per Filter Effects 1).
    Blur(f32),
    /// Drop shadow: offset and blur standard deviation in device pixels, and
    /// the resolved straight-alpha shadow color.
    DropShadow { dx: f32, dy: f32, std: f32, color: [u8; 4] },
}

/// A bounded, `Copy` filter list. At most `MAX_FILTERS` functions are
/// retained; unsupported functions are dropped, never approximated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPlan {
    ops: [Option<FilterOp>; MAX_FILTERS],
    len: u8,
}

impl Default for FilterPlan {
    fn default() -> Self {
        Self {
            ops: [None; MAX_FILTERS],
            len: 0,
        }
    }
}

impl FilterPlan {
    /// Parse a computed `filter`/`backdrop-filter` value. `current_color` is
    /// the element's resolved `color`, used for `drop-shadow()`'s default and
    /// explicit `currentcolor` shadow color.
    pub fn from_css(value: &str, current_color: [u8; 4]) -> Self {
        let mut plan = Self::default();
        let Some(list) = amazing_css::effects::parse_filter_list(value) else {
            return plan;
        };
        if let FilterList::Functions(functions) = list {
            for function in functions {
                if let Some(op) = op_from(&function, current_color) {
                    if (plan.len as usize) < MAX_FILTERS {
                        plan.ops[plan.len as usize] = Some(op);
                        plan.len += 1;
                    }
                }
            }
        }
        plan
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn ops(&self) -> impl Iterator<Item = FilterOp> + '_ {
        self.ops.iter().take(self.len as usize).filter_map(|op| *op)
    }

    /// Whether any retained function needs neighboring pixels (`blur`,
    /// `drop-shadow`), i.e. cannot be applied with [`apply`](Self::apply).
    pub fn has_spatial_op(&self) -> bool {
        self.ops()
            .any(|op| matches!(op, FilterOp::Blur(_) | FilterOp::DropShadow { .. }))
    }

    /// Apply every retained function in source order to one straight-alpha
    /// pixel, as if it were the only pixel of its own 1x1 surface: spatial
    /// functions have no neighbors to sample, so `blur` is a no-op and
    /// `drop-shadow` draws its shadow color coincident with the pixel.
    pub fn apply(&self, pixel: [u8; 4]) -> [u8; 4] {
        if !self.has_spatial_op() {
            let mut pixel = pixel;
            for op in self.ops() {
                pixel = apply_color_op(op, pixel);
            }
            return pixel;
        }
        let mut surface = Surface::new(1, 1);
        surface.set_pixel(0, 0, pixel);
        self.apply_to_surface(&surface).get_pixel(0, 0).unwrap_or(pixel)
    }

    /// Apply every retained function in source order across the whole
    /// surface. Color functions map every pixel independently; `blur` and
    /// `drop-shadow` need this whole-surface form because they read
    /// neighboring pixels.
    pub fn apply_to_surface(&self, source: &Surface) -> Surface {
        let mut current = source.clone();
        for op in self.ops() {
            current = apply_op_to_surface(op, &current);
        }
        current
    }
}

fn op_from(function: &FilterFunction, current_color: [u8; 4]) -> Option<FilterOp> {
    let amount = |value: f64| {
        let value = value as f32;
        value.is_finite().then_some(value)
    };
    // Only an already-resolved absolute length (the CSS parser folds
    // `in`/`cm`/`mm`/`q`/`pt`/`pc` into `Px`) is supported: percentages are
    // invalid CSS for these arguments, and `em`/`rem`/viewport units need
    // font/viewport context this bounded parser does not have.
    let px = |length: &LengthPercentage| match length {
        LengthPercentage::Px(value) if value.is_finite() => Some(*value as f32),
        _ => None,
    };
    Some(match function {
        FilterFunction::Brightness(value) => FilterOp::Brightness(amount(*value)?),
        FilterFunction::Contrast(value) => FilterOp::Contrast(amount(*value)?),
        FilterFunction::Grayscale(value) => FilterOp::Grayscale(amount(*value)?),
        FilterFunction::HueRotate(value) => FilterOp::HueRotate(amount(*value)?),
        FilterFunction::Invert(value) => FilterOp::Invert(amount(*value)?),
        FilterFunction::Opacity(value) => FilterOp::Opacity(amount(*value)?),
        FilterFunction::Saturate(value) => FilterOp::Saturate(amount(*value)?),
        FilterFunction::Sepia(value) => FilterOp::Sepia(amount(*value)?),
        FilterFunction::Blur(length) => FilterOp::Blur(px(length)?.max(0.0)),
        FilterFunction::DropShadow(shadow) => {
            let dx = px(&shadow.offset_x)?;
            let dy = px(&shadow.offset_y)?;
            let std = match &shadow.blur {
                Some(length) => px(length)?.max(0.0),
                None => 0.0,
            };
            let color = match &shadow.color {
                Some(text) if text.eq_ignore_ascii_case("currentcolor") => current_color,
                Some(text) => amazing_css::color::parse_css_color(text)?,
                None => current_color,
            };
            FilterOp::DropShadow { dx, dy, std, color }
        }
        FilterFunction::Url(_) | FilterFunction::Expression { .. } => return None,
    })
}

fn apply_op_to_surface(op: FilterOp, surface: &Surface) -> Surface {
    match op {
        FilterOp::Blur(std) => gaussian_blur(surface, std),
        FilterOp::DropShadow { dx, dy, std, color } => drop_shadow(surface, dx, dy, std, color),
        color_op => {
            let mut out = surface.clone();
            for chunk in out.pixels.chunks_exact_mut(4) {
                let pixel = apply_color_op(color_op, [chunk[0], chunk[1], chunk[2], chunk[3]]);
                chunk.copy_from_slice(&pixel);
            }
            out
        }
    }
}

fn apply_color_op(op: FilterOp, pixel: [u8; 4]) -> [u8; 4] {
    let [r, g, b, a] = pixel.map(|channel| channel as f32);
    let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
    match op {
        FilterOp::Brightness(amount) => [
            channel(r * amount),
            channel(g * amount),
            channel(b * amount),
            a as u8,
        ],
        FilterOp::Contrast(amount) => [
            channel((r - 127.5) * amount + 127.5),
            channel((g - 127.5) * amount + 127.5),
            channel((b - 127.5) * amount + 127.5),
            a as u8,
        ],
        FilterOp::Grayscale(amount) => {
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            [
                channel(r + (luma - r) * amount),
                channel(g + (luma - g) * amount),
                channel(b + (luma - b) * amount),
                a as u8,
            ]
        }
        FilterOp::Invert(amount) => [
            channel(r + (255.0 - 2.0 * r) * amount),
            channel(g + (255.0 - 2.0 * g) * amount),
            channel(b + (255.0 - 2.0 * b) * amount),
            a as u8,
        ],
        FilterOp::Opacity(amount) => [r as u8, g as u8, b as u8, channel(a * amount)],
        FilterOp::Saturate(amount) => {
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            [
                channel(luma + (r - luma) * amount),
                channel(luma + (g - luma) * amount),
                channel(luma + (b - luma) * amount),
                a as u8,
            ]
        }
        FilterOp::Sepia(amount) => {
            let sr = 0.393 * r + 0.769 * g + 0.189 * b;
            let sg = 0.349 * r + 0.686 * g + 0.168 * b;
            let sb = 0.272 * r + 0.534 * g + 0.131 * b;
            [
                channel(r + (sr - r) * amount),
                channel(g + (sg - g) * amount),
                channel(b + (sb - b) * amount),
                a as u8,
            ]
        }
        FilterOp::HueRotate(degrees) => {
            let radians = degrees.to_radians();
            let (sin, cos) = (radians.sin(), radians.cos());
            let matrix = [
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
            ];
            [
                channel(matrix[0] * r + matrix[1] * g + matrix[2] * b),
                channel(matrix[3] * r + matrix[4] * g + matrix[5] * b),
                channel(matrix[6] * r + matrix[7] * g + matrix[8] * b),
                a as u8,
            ]
        }
        // Dispatched to `apply_op_to_surface`'s spatial arms; never reached
        // through that function's `color_op` fallthrough.
        FilterOp::Blur(_) | FilterOp::DropShadow { .. } => pixel.map(|c| c as u8),
    }
}

// ---------------------------------------------------------------------------
// Spatial (whole-surface) filter functions.
// ---------------------------------------------------------------------------

/// The recommended three-box-blur approximation of a Gaussian blur (SVG
/// Filter Effects: <https://www.w3.org/TR/SVG11/filters.html#feGaussianBlurElement>),
/// box size `d = floor(std * 3 * sqrt(2*PI) / 4 + 0.5)`. An odd `d` uses
/// three centered passes of radius `(d-1)/2` either side; an even `d` cannot
/// center a whole-pixel window, so per spec its first two passes use radius
/// `d/2` biased one pixel left then one pixel right (which cancels rather
/// than compounds a shift) and the third uses the centered, always-odd size
/// `d+1`. Operates in premultiplied space so a transparent neighbor cannot
/// bleed its incidental RGB into an opaque edge, and clamps the surface edge
/// outward instead of blurring in transparent black. This is a deterministic
/// reference, not a subpixel-accurate Gaussian kernel.
fn gaussian_blur(surface: &Surface, std: f32) -> Surface {
    let d = box_size(std.min(MAX_BLUR_STD));
    if d <= 1 || surface.width == 0 || surface.height == 0 {
        return surface.clone();
    }
    let mut data = to_premultiplied(surface);
    for (radius_left, radius_right) in blur_pass_radii(d) {
        data = box_blur_2d(&data, surface.width, surface.height, radius_left, radius_right);
    }
    from_premultiplied(&data, surface.width, surface.height)
}

fn box_size(std: f32) -> u32 {
    if !std.is_finite() || std <= 0.0 {
        return 0;
    }
    let d = (std * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0 + 0.5).floor();
    if !d.is_finite() {
        return 1;
    }
    (d as i64).max(1) as u32
}

/// Per-pass `(radius_left, radius_right)` for a three-box-blur approximation
/// of box size `d`, per the SVG filter spec's odd/even split.
fn blur_pass_radii(d: u32) -> [(i64, i64); 3] {
    if d % 2 == 1 {
        let r = ((d - 1) / 2) as i64;
        [(r, r), (r, r), (r, r)]
    } else {
        let r = (d / 2) as i64;
        [(r, r - 1), (r - 1, r), (r, r)]
    }
}

fn to_premultiplied(surface: &Surface) -> Vec<[f32; 4]> {
    surface
        .pixels
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32;
            let scale = a / 255.0;
            [p[0] as f32 * scale, p[1] as f32 * scale, p[2] as f32 * scale, a]
        })
        .collect()
}

fn from_premultiplied(data: &[[f32; 4]], width: u32, height: u32) -> Surface {
    let mut surface = Surface::new(width, height);
    let unpremul = |channel: f32, alpha: f32| -> u8 {
        if alpha > 0.0 {
            (channel * 255.0 / alpha).round().clamp(0.0, 255.0) as u8
        } else {
            0
        }
    };
    for (index, pixel) in data.iter().enumerate() {
        let alpha = pixel[3].round().clamp(0.0, 255.0);
        let rgba = [
            unpremul(pixel[0], pixel[3]),
            unpremul(pixel[1], pixel[3]),
            unpremul(pixel[2], pixel[3]),
            alpha as u8,
        ];
        let x = index as u32 % width;
        let y = index as u32 / width;
        surface.set_pixel(x, y, rgba);
    }
    surface
}

/// Separable box blur of radius `(radius_left, radius_right)` over 4-plane
/// data, horizontal pass then vertical pass, with the surface edge clamped
/// (repeated) outward.
fn box_blur_2d(
    data: &[[f32; 4]],
    width: u32,
    height: u32,
    radius_left: i64,
    radius_right: i64,
) -> Vec<[f32; 4]> {
    let (w, h) = (width as usize, height as usize);
    if radius_left + radius_right < 1 || w == 0 || h == 0 {
        return data.to_vec();
    }
    let mut horizontal = vec![[0.0f32; 4]; data.len()];
    let mut row = vec![0.0f32; w];
    for y in 0..h {
        for channel in 0..4 {
            for x in 0..w {
                row[x] = data[y * w + x][channel];
            }
            let blurred = box_blur_1d(&row, radius_left, radius_right);
            for x in 0..w {
                horizontal[y * w + x][channel] = blurred[x];
            }
        }
    }
    let mut vertical = vec![[0.0f32; 4]; data.len()];
    let mut column = vec![0.0f32; h];
    for x in 0..w {
        for channel in 0..4 {
            for y in 0..h {
                column[y] = horizontal[y * w + x][channel];
            }
            let blurred = box_blur_1d(&column, radius_left, radius_right);
            for y in 0..h {
                vertical[y * w + x][channel] = blurred[y];
            }
        }
    }
    vertical
}

/// One 1D box blur pass, edge-clamped, via a running sum (`O(n)`).
fn box_blur_1d(data: &[f32], radius_left: i64, radius_right: i64) -> Vec<f32> {
    let n = data.len();
    if n == 0 || radius_left + radius_right < 1 {
        return data.to_vec();
    }
    let window = (radius_left + radius_right + 1) as f32;
    let clamp_idx = |k: i64| -> usize { k.clamp(0, n as i64 - 1) as usize };
    let mut out = vec![0.0f32; n];
    let mut sum = 0.0f32;
    for k in -radius_left..=radius_right {
        sum += data[clamp_idx(k)];
    }
    out[0] = sum / window;
    for i in 1..n {
        let add_idx = clamp_idx(i as i64 + radius_right);
        let remove_idx = clamp_idx(i as i64 - 1 - radius_left);
        sum += data[add_idx] - data[remove_idx];
        out[i] = sum / window;
    }
    out
}

/// `drop-shadow(dx dy std color)`: tint the source's alpha into `color`,
/// blur that shadow, shift it by the offset (pixels shifted past the surface
/// edge are discarded, matching the painter's existing bounded-surface
/// treatment elsewhere), then draw the original source over it.
fn drop_shadow(surface: &Surface, dx: f32, dy: f32, std: f32, color: [u8; 4]) -> Surface {
    let (width, height) = (surface.width, surface.height);
    let mut shadow = Surface::new(width, height);
    for (index, chunk) in surface.pixels.chunks_exact(4).enumerate() {
        let source_alpha = chunk[3] as f32 / 255.0;
        let color_alpha = color[3] as f32 / 255.0;
        let alpha = (source_alpha * color_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        let x = index as u32 % width;
        let y = index as u32 / width;
        shadow.set_pixel(x, y, [color[0], color[1], color[2], alpha]);
    }
    let blurred = if std > 0.0 { gaussian_blur(&shadow, std) } else { shadow };
    let mut out = shift_surface(&blurred, dx, dy);
    for (index, chunk) in surface.pixels.chunks_exact(4).enumerate() {
        let source = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let x = index as u32 % width;
        let y = index as u32 / width;
        let destination = out.get_pixel(x, y).unwrap_or([0; 4]);
        out.set_pixel(x, y, crate::blend_source_over(source, destination, 255));
    }
    out
}

/// Translate a surface by `(dx, dy)` device pixels (rounded to the nearest
/// whole pixel); content shifted past the edge is discarded and the
/// uncovered edge becomes transparent, matching this painter's bounded
/// per-surface (not per-halo) filter model.
fn shift_surface(surface: &Surface, dx: f32, dy: f32) -> Surface {
    let (width, height) = (surface.width, surface.height);
    let mut out = Surface::new(width, height);
    let dx = if dx.is_finite() { dx.round() as i64 } else { 0 };
    let dy = if dy.is_finite() { dy.round() as i64 } else { 0 };
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let source_x = x - dx;
            let source_y = y - dy;
            if source_x < 0 || source_y < 0 || source_x >= width as i64 || source_y >= height as i64 {
                continue;
            }
            if let Some(pixel) = surface.get_pixel(source_x as u32, source_y as u32) {
                out.set_pixel(x as u32, y as u32, pixel);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{FilterOp, FilterPlan};
    use crate::surface::Surface;

    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn parses_color_functions_and_now_blur_and_drop_shadow_in_order() {
        let plan = FilterPlan::from_css("grayscale(1) blur(2px) brightness(0.5)", BLACK);
        let ops: Vec<_> = plan.ops().collect();
        assert_eq!(
            ops,
            [
                FilterOp::Grayscale(1.0),
                FilterOp::Blur(2.0),
                FilterOp::Brightness(0.5)
            ]
        );
        assert!(FilterPlan::from_css("none", BLACK).is_empty());
    }

    #[test]
    fn drop_shadow_resolves_offsets_blur_and_currentcolor() {
        let plan = FilterPlan::from_css("drop-shadow(2px 3px 4px)", [10, 20, 30, 255]);
        let ops: Vec<_> = plan.ops().collect();
        assert_eq!(
            ops,
            [FilterOp::DropShadow {
                dx: 2.0,
                dy: 3.0,
                std: 4.0,
                color: [10, 20, 30, 255]
            }]
        );
        let explicit = FilterPlan::from_css("drop-shadow(rgb(4, 5, 6) 1px 2px)", BLACK);
        assert_eq!(
            explicit.ops().collect::<Vec<_>>(),
            [FilterOp::DropShadow {
                dx: 1.0,
                dy: 2.0,
                std: 0.0,
                color: [4, 5, 6, 255]
            }]
        );
    }

    #[test]
    fn relative_length_filter_functions_are_skipped_not_guessed() {
        // `em`/percentage lengths need font/box context this parser lacks.
        assert!(FilterPlan::from_css("blur(2em)", BLACK).is_empty());
        assert!(FilterPlan::from_css("drop-shadow(1em 2px)", BLACK).is_empty());
    }

    #[test]
    fn color_functions_match_their_definitions() {
        let red = [255, 0, 0, 255];
        assert_eq!(FilterPlan::from_css("grayscale(1)", BLACK).apply(red), {
            let luma = (0.2126_f32 * 255.0).round() as u8;
            [luma, luma, luma, 255]
        });
        assert_eq!(FilterPlan::from_css("brightness(0)", BLACK).apply(red), [0, 0, 0, 255]);
        assert_eq!(
            FilterPlan::from_css("invert(1)", BLACK).apply(red),
            [0, 255, 255, 255]
        );
        let half = FilterPlan::from_css("opacity(0.5)", BLACK).apply(red);
        assert_eq!(half, [255, 0, 0, 128]);
    }

    #[test]
    fn blur_of_a_uniform_opaque_field_leaves_its_interior_unchanged() {
        let mut surface = Surface::new(20, 20);
        surface.clear([200, 100, 50, 255]);
        let plan = FilterPlan::from_css("blur(4px)", BLACK);
        let blurred = plan.apply_to_surface(&surface);
        // Interior pixels, far from any edge, average a uniform field to itself.
        assert_eq!(blurred.get_pixel(10, 10), Some([200, 100, 50, 255]));
    }

    #[test]
    fn blur_spreads_a_single_opaque_pixel_into_its_transparent_neighbors() {
        let mut surface = Surface::new(11, 11);
        surface.set_pixel(5, 5, [255, 0, 0, 255]);
        // `blur(1px)`'s three-box-pass support reaches 2px either side of the
        // spike; the grid keeps a 5px margin so the far corner is untouched.
        let plan = FilterPlan::from_css("blur(1px)", BLACK);
        let blurred = plan.apply_to_surface(&surface);
        let row: Vec<u8> = (0..11)
            .map(|x| blurred.get_pixel(x, 5).expect("row pixel")[3])
            .collect();
        let center = row[5];
        // The spike dims (spreads its alpha out) but stays present.
        assert!(center > 0 && center < 255, "{row:?}");
        assert!(
            row.iter().any(|&alpha| alpha > 0 && alpha != center),
            "blur should spread alpha into some neighbor: {row:?}"
        );
        assert_eq!(
            row.iter().copied().max(),
            Some(center),
            "the original spike's column should stay the brightest: {row:?}"
        );
        let far = blurred.get_pixel(0, 0).expect("far corner");
        assert_eq!(far, [0, 0, 0, 0], "blur radius should not reach the far corner");
    }

    #[test]
    fn drop_shadow_draws_shadow_behind_and_offset_from_the_source() {
        let mut surface = Surface::new(11, 11);
        surface.set_pixel(5, 5, [255, 255, 255, 255]);
        let plan = FilterPlan::from_css("drop-shadow(2px 0px 0px black)", BLACK);
        let shadowed = plan.apply_to_surface(&surface);
        // The source pixel itself stays exactly on top of the shadow.
        assert_eq!(shadowed.get_pixel(5, 5), Some([255, 255, 255, 255]));
        // A zero-blur shadow is an opaque black pixel offset by (2, 0).
        assert_eq!(shadowed.get_pixel(7, 5), Some([0, 0, 0, 255]));
        assert_eq!(shadowed.get_pixel(6, 5), Some([0, 0, 0, 0]));
    }

    #[test]
    fn drop_shadow_defaults_to_currentcolor_and_zero_blur() {
        let mut surface = Surface::new(5, 5);
        surface.set_pixel(2, 2, [10, 20, 30, 255]);
        let plan = FilterPlan::from_css("drop-shadow(1px 0px)", [7, 8, 9, 255]);
        let shadowed = plan.apply_to_surface(&surface);
        assert_eq!(shadowed.get_pixel(3, 2), Some([7, 8, 9, 255]));
    }
}
