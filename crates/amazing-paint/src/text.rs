//! Text painting: per-character ink drawn from a pluggable glyph source.
//!
//! Advances always come from [`FontSource::measure`] (the same numbers
//! layout used to place the words), one character at a time, so painted
//! text lines up with the geometry in `WordFrag` bit-for-bit under the
//! fallback font. Ink itself comes from a [`GlyphProvider`].
//!
//! # Implemented
//!
//! - [`GlyphProvider`] trait: the seam real font rasterizers plug into
//! - [`RectInkProvider`]: each character renders as a tidy solid ink
//!   rectangle, height ≈ 0.7 × font_size, sitting ON the line baseline
//!   (spanning baseline − ink_height ..= baseline), width matching the
//!   fallback font's per-class advance estimate; whitespace draws no ink
//! - [`OutlineGlyphProvider`]: REAL TrueType letterforms. Takes a
//!   [`FontSource`]; under `FontSource::Ttf` it decomposes the glyph
//!   outline (via `amazing_layout`), flattens the quadratic B-splines
//!   (fixed 4 subdivisions per segment), scales to pixel space with the
//!   baseline at y=0 and scanline-fills with NONZERO winding and 4x4
//!   per-pixel supersampling (16 coverage levels, deterministic integer
//!   alpha). Under `FontSource::Fallback` (or an unmapped character) it
//!   delegates to [`RectInkProvider`] so the no-font path is unchanged.
//! - alpha-compositing blit (`src over dst`, integer-only, deterministic)
//!   so ink can land on colored backgrounds
//!
//! # Deferred / deviations
//!
//! - No hinting (glyph instructions ignored upstream), no subpixel
//!   positioning, no gamma-correct blending; anti-aliasing is plain 16-level
//!   box supersampling. Curves flatten at a FIXED 4 subdivisions per
//!   quadratic segment — large display sizes may show faint faceting.
//! - Winding rule is NONZERO (TrueType's native rule): outer contours and
//!   counter-oriented holes cancel correctly. Even-odd would also work for
//!   well-formed TrueType but nonzero tolerates overlapping components.
//! - Characters that map to glyph 0 (.notdef) or have zero advance draw
//!   NO ink (advances and ink must agree so words never overlap).
//! - Under a TTF [`FontSource`], RectInkProvider's bar widths will not
//!   match true advances exactly (bars are heuristic; advances are not) —
//!   use [`OutlineGlyphProvider`] for real letterforms.

use crate::surface::Surface;
use amazing_layout::font::{FontSource, GlyphOutline, OutlinePoint};

/// One rasterized glyph: RGBA8 pixels, row-major, top-left origin, plus
/// metrics relative to the pen position and the baseline.
#[derive(Debug, Clone)]
pub struct GlyphBitmap {
    /// `width * height * 4` bytes. The ALPHA channel is treated as ink
    /// coverage when compositing; RGB of the bitmap is ignored (the pen
    /// color supplies the tint).
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Horizontal offset from pen origin to bitmap left edge.
    pub bearing_x: f32,
    /// Vertical offset from baseline UP to bitmap top edge.
    pub bearing_y: f32,
    /// Informational advance for this glyph at this size; the draw loop
    /// still uses `FontSource::measure` as the single source of truth.
    pub advance: f32,
}

/// Source of glyph rasters. Implementors must be panic-free on arbitrary
/// characters and sizes; returning `None` simply skips the character's ink.
pub trait GlyphProvider {
    fn rasterize(&self, ch: char, px: f32) -> Option<GlyphBitmap>;
}

/// Default provider: solid rectangles of "ink" instead of letterforms.
///
/// Geometry: height = round(0.7 × px), clamped to ≥ 1; top edge at
/// baseline − height (so the bar SITS on the baseline). Width mirrors the
/// fallback font's width classes (0.55em letters/digits, 0.30em ASCII
/// punctuation, 1.0em anything else) so bars tile without visible gaps or
/// overlaps under [`FontSource::fallback`]. Whitespace yields no ink.
#[derive(Debug, Clone, Copy, Default)]
pub struct RectInkProvider;

impl GlyphProvider for RectInkProvider {
    fn rasterize(&self, ch: char, px: f32) -> Option<GlyphBitmap> {
        if !px.is_finite() || px <= 0.0 || ch.is_whitespace() {
            return None;
        }
        let ink_h = ((0.7 * px).round() as usize).max(1);
        let w_em = if ch.is_ascii_alphanumeric() {
            0.55
        } else if ch.is_ascii_punctuation() {
            0.30
        } else {
            1.0
        };
        let w = ((w_em * px).round() as usize).max(1);
        // Opaque white coverage mask; the blit tints it with the pen color.
        let mut pixels = vec![255u8; w * ink_h * 4];
        for px4 in pixels.chunks_exact_mut(4) {
            px4.copy_from_slice(&[255, 255, 255, 255]);
        }
        Some(GlyphBitmap {
            advance: w_em * px,
            bearing_x: 0.0,
            bearing_y: ink_h as f32, // top edge = baseline - bearing_y
            height: ink_h,
            width: w,
            pixels,
        })
    }
}

/// Real-glyph provider: TrueType outlines from an [`FontSource`], flattened
/// and scanline-filled into coverage bitmaps. Holds only a borrow of the
/// font source; the no-font fallback decision lives in [`rasterize`]
/// (delegates to [`RectInkProvider`] under `FontSource::Fallback`, and for
/// any character whose advance is 0 — unmapped glyphs must not ink over
/// neighbouring words).
#[derive(Debug, Clone, Copy)]
pub struct OutlineGlyphProvider<'a> {
    fonts: &'a FontSource,
}

impl<'a> OutlineGlyphProvider<'a> {
    pub fn new(fonts: &'a FontSource) -> Self {
        OutlineGlyphProvider { fonts }
    }
}

impl GlyphProvider for OutlineGlyphProvider<'_> {
    fn rasterize(&self, ch: char, px: f32) -> Option<GlyphBitmap> {
        if !px.is_finite() || px <= 0.0 || ch.is_whitespace() {
            return None;
        }
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        // Single source of truth for advances, same call layout makes.
        let advance = self.fonts.measure(s, px);
        if advance <= 0.0 {
            return None;
        }
        let parsed = match self.fonts {
            FontSource::Ttf(f) => f,
            FontSource::Fallback(_) => return RectInkProvider.rasterize(ch, px),
        };
        let gid = parsed.glyph_index(ch);
        if gid == 0 {
            return None; // .notdef: no letterform to show
        }
        let outline = parsed.glyph_outline(gid)?;
        rasterize_outline(&outline, px, parsed.metrics().units_per_em, advance)
    }
}

// ---------------------------------------------------------------------------
// outline flattening + scanline fill (pure functions; unit-testable without
// a font file)
// ---------------------------------------------------------------------------

/// One flattened line segment in PIXEL space (y down), baseline at y = 0.
type Edge = [f32; 4]; // x1, y1, x2, y2

const SUBDIVS: usize = 4; // fixed curve subdivisions per quadratic segment
const SUBSAMPLES: u32 = 4; // 4x4 supersamples per pixel => 16 levels
const MAX_DIM: i64 = 4096; // defensive bitmap dimension cap

/// Flattens a font-unit outline into pixel-space edges. `px_per_unit`
/// scales x; y flips sign (font y-up -> pixel y-down) with the baseline
/// at y = 0. Contours whose flagged chain is malformed (an off-curve
/// point at the wrap without a following anchor) are skipped defensively.
fn flatten_outline(outline: &GlyphOutline, s: f32) -> Vec<Edge> {
    let mut edges = Vec::new();
    for contour in &outline.contours {
        let pts = &contour.points;
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let px = |p: &OutlinePoint| (p.x * s, -p.y * s);
        let mut cur = px(&pts[0]);
        let mut i = 0usize;
        while i < n {
            let b = pts[(i + 1) % n];
            if b.on_curve {
                let (bx, by) = px(&b);
                edges.push([cur.0, cur.1, bx, by]);
                cur = (bx, by);
                i += 1;
            } else {
                // Quadratic: current anchor, control b, anchor c.
                // close_quadratic upstream guarantees the chain closes on
                // an on-curve point.
                if i + 2 > n {
                    break; // malformed tail; keep what we have
                }
                let c = pts[(i + 2) % n];
                let (cx, cy) = px(&c);
                let (bx, by) = px(&b);
                // Evaluate against the SEGMENT START p0, not the running
                // cursor -- B(t) is a Bernstein polynomial in the original
                // control points, so feeding the previous sample back in
                // would compound distortion into every later subdivision.
                let p0 = cur;
                for k in 1..=SUBDIVS {
                    let t = k as f32 / SUBDIVS as f32;
                    let mt = 1.0 - t;
                    // de Casteljau for one quadratic segment
                    let qx = mt * mt * p0.0 + 2.0 * mt * t * bx + t * t * cx;
                    let qy = mt * mt * p0.1 + 2.0 * mt * t * by + t * t * cy;
                    edges.push([cur.0, cur.1, qx, qy]);
                    cur = (qx, qy);
                }
                i += 2;
            }
        }
    }
    edges
}

/// Renders one glyph outline to a coverage bitmap.
///
/// Geometry: units scale by `px_size / upem`; y flips so the baseline is
/// pixel row boundary y = 0 and +y goes DOWN. The bitmap spans
/// floor(min)..ceil(max) of the scaled bounding box; `bearing_x` is the
/// bitmap's left edge and `bearing_y` the top edge above the baseline, so
/// blitting at `(pen + bearing_x, baseline - bearing_y)` reproduces the
/// pen-relative geometry exactly. Returns `None` for degenerate or empty
/// ink (no edges, zero-area bbox, absurd size).
///
/// Coverage: NONZERO winding rule via a horizontal ray per subsample;
/// each pixel averages a 4x4 grid of samples (16 alpha levels).
fn rasterize_outline(
    outline: &GlyphOutline,
    px_size: f32,
    upem: u16,
    advance: f32,
) -> Option<GlyphBitmap> {
    if !px_size.is_finite() || px_size <= 0.0 || upem == 0 || !advance.is_finite() {
        return None;
    }
    let edges = flatten_outline(outline, px_size / upem as f32);
    if edges.is_empty() {
        return None;
    }
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for e in &edges {
        min_x = min_x.min(e[0]).min(e[2]);
        max_x = max_x.max(e[0]).max(e[2]);
        min_y = min_y.min(e[1]).min(e[3]);
        max_y = max_y.max(e[1]).max(e[3]);
    }
    let x0 = min_x.floor();
    let y0 = min_y.floor();
    let w = (max_x.ceil() - x0) as i64;
    let h = (max_y.ceil() - y0) as i64;
    if w < 1 || h < 1 || w > MAX_DIM || h > MAX_DIM {
        return None;
    }

    let ss = SUBSAMPLES as f32;
    let step = 1.0 / ss;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let samples = SUBSAMPLES * SUBSAMPLES; // 16
    for row in 0..h {
        for col in 0..w {
            let mut hit = 0u32;
            for j in 0..SUBSAMPLES {
                let sy = y0 + row as f32 + (j as f32 + 0.5) * step;
                for i in 0..SUBSAMPLES {
                    let sx = x0 + col as f32 + (i as f32 + 0.5) * step;
                    // Nonzero winding: sum signed crossings of the ray
                    // +x from (sx, sy). Half-open span [y1, y2) keeps
                    // vertices consistent; horizontal edges never cross.
                    let mut wind = 0i32;
                    for e in &edges {
                        let (ey1, ey2) = (e[1], e[3]);
                        if ey1 <= sy {
                            if ey2 > sy {
                                let t = (sy - ey1) / (ey2 - ey1);
                                if e[0] + t * (e[2] - e[0]) > sx {
                                    wind += 1;
                                }
                            }
                        } else if ey2 <= sy {
                            let t = (sy - ey1) / (ey2 - ey1);
                            if e[0] + t * (e[2] - e[0]) > sx {
                                wind -= 1;
                            }
                        }
                    }
                    if wind != 0 {
                        hit += 1;
                    }
                }
            }
            if hit > 0 {
                let alpha = ((hit * 255) / samples) as u8;
                let p = &mut pixels[((row * w + col) as usize) * 4..][..4];
                p.copy_from_slice(&[255, 255, 255, alpha]);
            }
        }
    }

    Some(GlyphBitmap {
        bearing_x: x0,
        bearing_y: -y0, // distance of the bitmap TOP above the baseline
        width: w as usize,
        height: h as usize,
        pixels,
        advance,
    })
}

/// Draws one run of text with its pen origin at `(x, baseline_y)` using
/// per-character advances from `fonts`. Coordinates are rounded to whole
/// pixels; every glyph is clipped to the surface. Non-finite coordinates
/// are a silent no-op. Never panics.
#[allow(clippy::too_many_arguments)]
pub fn draw_fragment(
    surface: &mut Surface,
    x: f32,
    baseline_y: f32,
    text: &str,
    font_size: f32,
    color: [u8; 4],
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
) {
    draw_fragment_clipped(
        surface,
        x,
        baseline_y,
        text,
        font_size,
        color,
        fonts,
        glyphs,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_fragment_clipped(
    surface: &mut Surface,
    x: f32,
    baseline_y: f32,
    text: &str,
    font_size: f32,
    color: [u8; 4],
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
    clip: Option<[i64; 4]>,
) {
    if !x.is_finite() || !baseline_y.is_finite() || !font_size.is_finite() {
        return;
    }
    let mut buf = [0u8; 4];
    let mut pen_x = x;
    for ch in text.chars() {
        let s = ch.encode_utf8(&mut buf);
        let adv = fonts.measure(s, font_size);
        if let Some(gb) = glyphs.rasterize(ch, font_size) {
            let gx = round_i64(pen_x + gb.bearing_x);
            let gy = round_i64(baseline_y - gb.bearing_y);
            blit_glyph(surface, gx, gy, &gb, color, clip);
        }
        pen_x += adv;
    }
}

fn round_i64(v: f32) -> i64 {
    v.round() as i64 // saturating on ±inf
}

/// Composites one glyph onto the surface: integer `src over dst`, treating
/// the glyph's alpha as coverage and the destination as opaque (engine
/// surfaces always start from an opaque canvas). Deterministic, no floats.
fn blit_glyph(
    surface: &mut Surface,
    gx: i64,
    gy: i64,
    gb: &GlyphBitmap,
    color: [u8; 4],
    clip: Option<[i64; 4]>,
) {
    if gb.pixels.len() < gb.width * gb.height * 4 {
        return; // malformed provider output: skip rather than panic
    }
    let sw = surface.width as i64;
    let sh = surface.height as i64;
    let clip = clip.unwrap_or([0, 0, sw, sh]);
    let clip_x0 = clip[0].max(0).min(sw);
    let clip_y0 = clip[1].max(0).min(sh);
    let clip_x1 = clip[2].max(0).min(sw);
    let clip_y1 = clip[3].max(0).min(sh);
    for row in 0..gb.height as i64 {
        let dy = gy + row;
        if dy < clip_y0 || dy >= clip_y1 {
            continue;
        }
        for col in 0..gb.width as i64 {
            let dx = gx + col;
            if dx < clip_x0 || dx >= clip_x1 {
                continue;
            }
            let src = &gb.pixels[((row as usize) * gb.width + col as usize) * 4..][..4];
            let cov = src[3] as u32;
            if cov == 0 {
                continue;
            }
            let dst = surface
                .get_pixel(dx as u32, dy as u32)
                .unwrap_or([0, 0, 0, 255]);
            if cov >= 255 {
                surface.set_pixel(dx as u32, dy as u32, [color[0], color[1], color[2], 255]);
                continue;
            }
            let inv = 255 - cov;
            let blend =
                |sc: u8, dc: u8| -> u8 { ((sc as u32 * cov + dc as u32 * inv + 127) / 255) as u8 };
            surface.set_pixel(
                dx as u32,
                dy as u32,
                [
                    blend(color[0], dst[0]),
                    blend(color[1], dst[1]),
                    blend(color[2], dst[2]),
                    255,
                ],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::parse_css_color;

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn canvas(w: u32, h: u32) -> Surface {
        let mut s = Surface::new(w, h);
        s.clear(WHITE);
        s
    }

    #[test]
    fn rect_ink_bar_sits_on_the_baseline() {
        let mut s = canvas(40, 20);
        let fonts = FontSource::fallback();
        // size 10 -> ink height = round(7.0) = 7 -> rows 8..=14 for baseline 15.
        draw_fragment(
            &mut s,
            2.0,
            15.0,
            "W",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        assert_eq!(s.get_pixel(3, 14), Some(BLACK)); // bottom ink row (on baseline)
        assert_eq!(s.get_pixel(3, 9), Some(BLACK)); // near the top of the bar
        assert_eq!(s.get_pixel(3, 8), Some(BLACK)); // top ink row (baseline-7)
        assert_eq!(s.get_pixel(3, 7), Some(WHITE)); // strictly above the ink
        assert_eq!(s.get_pixel(3, 15), Some(WHITE)); // strictly below the baseline
    }

    #[test]
    fn ink_starts_at_pen_x_and_stops_at_advance() {
        let mut s = canvas(40, 20);
        let fonts = FontSource::fallback();
        draw_fragment(
            &mut s,
            5.0,
            10.0,
            "W",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        // Advance of 'W' at 10px under the fallback = 0.55 * 10 = 5.5px;
        // the bar is round(5.5) = 6 wide starting at x=5 -> cols 5..=10.
        assert_eq!(s.get_pixel(4, 8), Some(WHITE));
        assert_eq!(s.get_pixel(5, 8), Some(BLACK));
        assert_eq!(s.get_pixel(10, 8), Some(BLACK));
        assert_eq!(s.get_pixel(11, 8), Some(WHITE));
        assert!((fonts.measure("W", 10.0) - 5.5).abs() < 1e-4);
    }

    #[test]
    fn multiple_characters_accumulate_advances() {
        let mut s = canvas(60, 20);
        let fonts = FontSource::fallback();
        draw_fragment(
            &mut s,
            2.0,
            12.0,
            "AB",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        // First bar covers cols 2..=7 (advance 5.5); second pen starts at
        // 7.5 -> rounds to col 8, second bar covers cols 8..=13.
        assert_eq!(s.get_pixel(2, 6), Some(BLACK));
        assert_eq!(s.get_pixel(7, 6), Some(BLACK));
        assert_eq!(s.get_pixel(13, 6), Some(BLACK));
        assert_eq!(s.get_pixel(14, 6), Some(WHITE));
        assert!((fonts.measure("AB", 10.0) - 11.0).abs() < 1e-4);
    }

    #[test]
    fn whitespace_produces_no_ink_but_consumes_advance() {
        let mut s = canvas(60, 20);
        let fonts = FontSource::fallback();
        draw_fragment(
            &mut s,
            2.0,
            12.0,
            "A B",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        // 'A' bar at cols 2..=7; space advances silently; 'B' starts at
        // pen x = 2 + 5.5 + 3.0 = 10.5 -> bar around cols 10..=16.
        assert_eq!(s.get_pixel(8, 6), Some(WHITE)); // inside the gap: no ink
        assert_eq!(s.get_pixel(11, 6), Some(BLACK)); // B's bar present
        assert_eq!(s.get_pixel(16, 6), Some(BLACK));
    }

    #[test]
    fn text_is_clipped_to_the_surface_without_panicking() {
        let mut s = canvas(8, 6);
        let fonts = FontSource::fallback();
        // Baseline below the bottom edge and far right pen: everything
        // clips away; must not panic.
        draw_fragment(
            &mut s,
            6.0,
            30.0,
            "WWWWWWWWW",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        draw_fragment(
            &mut s,
            -50.0,
            3.0,
            "WWWWWW",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        draw_fragment(
            &mut s,
            0.0,
            f32::NAN,
            "W",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        assert_eq!(s.get_pixel(0, 0), Some(WHITE));
    }

    #[test]
    fn partial_coverage_blends_with_integer_math() {
        struct HalfCoverage;
        impl GlyphProvider for HalfCoverage {
            fn rasterize(&self, _ch: char, px: f32) -> Option<GlyphBitmap> {
                Some(GlyphBitmap {
                    pixels: vec![255, 255, 255, 128, 255, 255, 255, 128],
                    width: 1,
                    height: 2,
                    bearing_x: 0.0,
                    bearing_y: 2.0,
                    advance: px,
                })
            }
        }
        let mut s = canvas(4, 4);
        let fonts = FontSource::fallback();
        draw_fragment(&mut s, 1.0, 2.0, "W", 8.0, BLACK, &fonts, &HalfCoverage);
        // Hand-computed blend: (0*128 + 255*127 + 127) / 255 = 32512 / 255.
        let blended = s.get_pixel(1, 0).expect("in bounds");
        assert_eq!(blended, [127, 127, 127, 255]);
        assert_eq!(s.get_pixel(1, 1), Some([127, 127, 127, 255])); // second row too
        assert_eq!(s.get_pixel(0, 0), Some(WHITE)); // untouched column
    }

    #[test]
    fn full_coverage_ink_is_opaque_pen_color_over_background() {
        let mut s = canvas(10, 12);
        let bg = parse_css_color("steelblue").unwrap(); // [70,130,180,255]
        s.fill_rect(0.0, 0.0, 10.0, 12.0, bg);
        let fonts = FontSource::fallback();
        draw_fragment(
            &mut s,
            1.0,
            10.0,
            "W",
            10.0,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        assert_eq!(s.get_pixel(2, 9), Some(BLACK)); // fully covered pixel
        assert_eq!(s.get_pixel(9, 5), Some(bg)); // outside the bar
    }

    #[test]
    fn zero_and_negative_sizes_draw_nothing() {
        let mut s = canvas(8, 8);
        let fonts = FontSource::fallback();
        draw_fragment(&mut s, 1.0, 4.0, "W", 0.0, BLACK, &fonts, &RectInkProvider);
        draw_fragment(&mut s, 1.0, 4.0, "W", -3.0, BLACK, &fonts, &RectInkProvider);
        draw_fragment(
            &mut s,
            1.0,
            4.0,
            "W",
            f32::NAN,
            BLACK,
            &fonts,
            &RectInkProvider,
        );
        assert_eq!(s.get_pixel(1, 3), Some(WHITE));
    }

    #[test]
    fn rect_provider_rejects_whitespace_and_bad_sizes() {
        assert!(RectInkProvider.rasterize(' ', 10.0).is_none());
        assert!(RectInkProvider.rasterize('\t', 10.0).is_none());
        assert!(RectInkProvider.rasterize('\u{00a0}', 10.0).is_none());
        assert!(RectInkProvider.rasterize('W', 0.0).is_none());
        assert!(RectInkProvider.rasterize('W', -1.0).is_none());
        assert!(RectInkProvider.rasterize('W', f32::NAN).is_none());
        let g = RectInkProvider.rasterize('W', 10.0).unwrap();
        assert_eq!(g.width * g.height * 4, g.pixels.len());
        assert!((g.bearing_y - 7.0).abs() < 1e-4); // top sits 7px above baseline
        assert!((g.advance - 5.5).abs() < 1e-4);
    }

    // --- outline rasterizer math (hand-built outlines, no font file) ------

    use amazing_layout::font::Contour;

    fn poly(points: &[(f32, f32)]) -> GlyphOutline {
        GlyphOutline {
            contours: vec![Contour {
                points: points
                    .iter()
                    .map(|&(x, y)| OutlinePoint {
                        x,
                        y,
                        on_curve: true,
                    })
                    .collect(),
            }],
        }
    }

    fn alpha_of(gb: &GlyphBitmap, x: usize, y: usize) -> u8 {
        gb.pixels[(y * gb.width + x) * 4 + 3]
    }

    /// Axis-aligned square 0..500 x 0..700 font units at 10px/1000upem
    /// becomes EXACTLY the pixel rect (0..5) x (rows above baseline 7):
    /// boundaries land on integer pixel edges, so every sample is inside.
    #[test]
    fn square_outline_rasterizes_to_solid_rect() {
        let o = poly(&[(0.0, 0.0), (500.0, 0.0), (500.0, 700.0), (0.0, 700.0)]);
        let gb = rasterize_outline(&o, 10.0, 1000, 5.0).expect("bitmap");
        assert_eq!((gb.width, gb.height), (5, 7));
        assert_eq!(gb.pixels.len(), 5 * 7 * 4);
        assert_eq!(gb.bearing_x, 0.0);
        assert_eq!(gb.bearing_y, 7.0); // ink tops out 7px above the baseline
        assert_eq!(gb.advance, 5.0);
        assert!(gb.pixels.chunks_exact(4).all(|p| p == [255, 255, 255, 255]));
    }

    #[test]
    fn triangle_diagonal_coverage_is_unimodal_and_partial() {
        // Apex up: (0,0) (1000,700) (2000,0) at 10px -> (0,0)(10,-7)(20,0).
        let o = poly(&[(0.0, 0.0), (1000.0, 700.0), (2000.0, 0.0)]);
        let gb = rasterize_outline(&o, 10.0, 1000, 20.0).expect("bitmap");
        assert_eq!((gb.width, gb.height), (20, 7));
        // Topmost row crosses only the apex: coverage is partial there
        // (16-level AA), rises to a single peak near the apex column and
        // falls again; outside the sliver it is exactly 0.
        let row: Vec<u8> = (0..gb.width).map(|x| alpha_of(&gb, x, 0)).collect();
        assert_eq!(row[0], 0);
        assert_eq!(row[19], 0);
        let mut start = 0usize;
        while start < row.len() && row[start] == 0 {
            start += 1;
        }
        assert!(start < row.len(), "apex row has no ink");
        let mut peak = start;
        while peak + 1 < row.len() && row[peak + 1] >= row[peak] {
            peak += 1;
        }
        assert!((9..=11).contains(&peak), "apex column, got {}", peak);
        assert!(row[peak] >= 96, "peak too thin: {}", row[peak]);
        assert!(row[9] > 0 && row[9] < 255, "flank must be partial AA");
        for k in start..peak {
            assert!(row[k] <= row[k + 1], "not monotonically rising: {:?}", row);
        }
        for k in peak..row.len() - 1 {
            assert!(row[k] >= row[k + 1], "not monotonically falling: {:?}", row);
        }
        // Near the base the triangle is wide and solid.
        assert_eq!(alpha_of(&gb, 10, 6), 255);
    }

    #[test]
    fn hole_cancels_under_nonzero_winding() {
        // Outer contour clockwise (y-up), inner counter-clockwise: under
        // NONZERO winding the hole's winding cancels the outer's.
        let outer: Vec<OutlinePoint> = [(0.0, 0.0), (0.0, 800.0), (800.0, 800.0), (800.0, 0.0)]
            .iter()
            .map(|&(x, y)| OutlinePoint {
                x,
                y,
                on_curve: true,
            })
            .collect();
        let hole: Vec<OutlinePoint> = [
            (200.0, 200.0),
            (600.0, 200.0),
            (600.0, 600.0),
            (200.0, 600.0),
        ]
        .iter()
        .map(|&(x, y)| OutlinePoint {
            x,
            y,
            on_curve: true,
        })
        .collect();
        let o = GlyphOutline {
            contours: vec![Contour { points: outer }, Contour { points: hole }],
        };
        let gb = rasterize_outline(&o, 10.0, 1000, 8.0).expect("bitmap");
        assert_eq!((gb.width, gb.height), (8, 8));
        // All boundaries land on integer pixel edges here, so coverage is
        // strictly binary — which makes the winding check exact.
        assert_eq!(alpha_of(&gb, 1, 1), 255); // inside frame arm
        assert_eq!(alpha_of(&gb, 0, 4), 255); // left arm column
        assert_eq!(alpha_of(&gb, 7, 4), 255); // right arm column
        assert_eq!(alpha_of(&gb, 4, 1), 255); // below the hole
        assert_eq!(alpha_of(&gb, 4, 7), 255); // above the hole (y-down)
        assert_eq!(alpha_of(&gb, 4, 4), 0); // wound-cancelled hole interior
        assert_eq!(alpha_of(&gb, 3, 3), 0); // hole corner region too
    }

    #[test]
    fn empty_and_degenerate_outlines_are_none() {
        let none = GlyphOutline {
            contours: Vec::new(),
        };
        assert!(rasterize_outline(&none, 10.0, 1000, 5.0).is_none());
        // Single point: no area.
        let dot = poly(&[(100.0, 100.0)]);
        assert!(rasterize_outline(&dot, 10.0, 1000, 5.0).is_none());
        // Straight line: edges exist but zero-height bbox.
        let line = poly(&[(0.0, 0.0), (500.0, 0.0)]);
        assert!(rasterize_outline(&line, 10.0, 1000, 5.0).is_none());
        // Degenerate parameters.
        let sq = poly(&[(0.0, 0.0), (500.0, 700.0)]);
        assert!(rasterize_outline(&sq, 0.0, 1000, 5.0).is_none());
        assert!(rasterize_outline(&sq, -3.0, 1000, 5.0).is_none());
        assert!(rasterize_outline(&sq, f32::NAN, 1000, 5.0).is_none());
        assert!(rasterize_outline(&sq, 10.0, 0, 5.0).is_none());
        assert!(rasterize_outline(&sq, 10.0, 1000, f32::NAN).is_none());
    }

    #[test]
    fn flattened_curve_bbox_excludes_control_point() {
        // One quad hump: anchors (0,0) and (1000,0), control (500,1000).
        // Curve apex reaches HALF the control height (quadratic property),
        // so the bitmap is 10x5, NOT 10x10 driven by the control point.
        let o = GlyphOutline {
            contours: vec![Contour {
                points: vec![
                    OutlinePoint {
                        x: 0.0,
                        y: 0.0,
                        on_curve: true,
                    },
                    OutlinePoint {
                        x: 500.0,
                        y: 1000.0,
                        on_curve: false,
                    },
                    OutlinePoint {
                        x: 1000.0,
                        y: 0.0,
                        on_curve: true,
                    },
                ],
            }],
        };
        let gb = rasterize_outline(&o, 10.0, 1000, 10.0).expect("bitmap");
        assert_eq!((gb.width, gb.height), (10, 5));
        assert_eq!(gb.bearing_y, 5.0);
        // Baseline row is fully covered between the anchors...
        assert_eq!(alpha_of(&gb, 5, 4), 255);
        // ...and the flanks show partial coverage somewhere (curved AA).
        let any_partial = (0..gb.width).any(|x| {
            let a = alpha_of(&gb, x, 1);
            a > 0 && a < 255
        });
        assert!(any_partial, "expected anti-aliased flank pixels");
    }

    // --- provider seam ------------------------------------------------------

    fn same_bitmap(a: Option<GlyphBitmap>, b: Option<GlyphBitmap>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.width == b.width
                    && a.height == b.height
                    && a.pixels == b.pixels
                    && a.bearing_x == b.bearing_x
                    && a.bearing_y == b.bearing_y
                    && a.advance == b.advance
            }
            _ => false,
        }
    }

    #[test]
    fn provider_falls_back_to_rect_ink_without_a_real_font() {
        let src = FontSource::fallback();
        let provider = OutlineGlyphProvider::new(&src);
        for ch in ['W', 'a', '5', '!', ' ', '\t'] {
            for px in [10.0f32, 24.0, 0.0] {
                assert!(
                    same_bitmap(
                        provider.rasterize(ch, px),
                        RectInkProvider.rasterize(ch, px)
                    ),
                    "mismatch for {:?} at {}",
                    ch,
                    px
                );
            }
        }
    }

    // --- end-to-end through a synthetic TTF -------------------------------

    fn be16(v: u16) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    fn be32(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    fn assemble_sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&be32(0x0001_0000));
        buf.extend_from_slice(&be16(tables.len() as u16));
        buf.extend_from_slice(&be16(0));
        buf.extend_from_slice(&be16(0));
        buf.extend_from_slice(&be16(0));
        let mut off = (12 + tables.len() * 16) as u32;
        for (tag, body) in tables.iter() {
            buf.extend_from_slice(*tag);
            buf.extend_from_slice(&be32(0));
            buf.extend_from_slice(&be32(off));
            buf.extend_from_slice(&be32(body.len() as u32));
            off += body.len() as u32;
        }
        for (_, body) in tables.iter() {
            buf.extend_from_slice(body);
        }
        buf
    }

    /// Square glyph record 0..500 x 0..700, four on-curve points, mixed
    /// delta encodings (zero / +500 / -500 as 16-bit).
    fn square_glyf_record() -> Vec<u8> {
        let mut r = vec![0u8; 10];
        r[0..2].copy_from_slice(&be16(1)); // numberOfContours
        r[6..8].copy_from_slice(&be16(500)); // xMax
        r[8..10].copy_from_slice(&be16(700)); // yMax
        r.extend_from_slice(&be16(3)); // endPtsOfContours[0]
        r.extend_from_slice(&be16(0)); // instructionLength
                                       // Deltas from (0,0): (0,0)(+500,0)(0,+700)(-500,0); zero deltas
                                       // ride the SAME bits, +/-500 use 16-bit signed words. Every byte
                                       // carries ON_CURVE (0x01) — without it all four points decode as
                                       // off-curve controls and the outline becomes a synthesized blob.
        r.extend_from_slice(&[0x31, 0x21, 0x11, 0x21]); // flags
        r.extend_from_slice(&be16(500)); // x stream: +500 ...
        r.extend_from_slice(&be16(0xFE0C)); // ... then -500 (two's complement)
        r.extend_from_slice(&be16(700)); // y stream: +700
        r
    }

    /// cmap format 4: 65..65 ('A') -> glyph 1 via idDelta -64; terminator.
    fn cmap_a_only() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(&be16(4));
        s.extend_from_slice(&be16(32)); // length
        s.extend_from_slice(&be16(0));
        s.extend_from_slice(&be16(4)); // segCountX2 = 2
        s.extend_from_slice(&be16(0));
        s.extend_from_slice(&be16(0));
        s.extend_from_slice(&be16(0));
        for e in [65u16, 0xFFFF] {
            s.extend_from_slice(&be16(e));
        }
        s.extend_from_slice(&be16(0)); // reservedPad
        for st in [65u16, 0xFFFF] {
            s.extend_from_slice(&be16(st));
        }
        for d in [65472u16, 1] {
            s.extend_from_slice(&be16(d)); // -64; +1 wraps FFFF -> 0
        }
        for _ in 0..2 {
            s.extend_from_slice(&be16(0)); // idRangeOffset
        }
        s
    }

    fn square_ttf() -> Vec<u8> {
        let glyf = square_glyf_record();
        let loca = [be16(0), be16(0), be16(glyf.len() as u16 / 2)].concat();
        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&be16(800));
        hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
        hhea[34..36].copy_from_slice(&be16(2)); // numberOfHMetrics
        let mut maxp = vec![0u8; 32];
        maxp[0..4].copy_from_slice(&be32(0x0001_0000));
        maxp[4..6].copy_from_slice(&be16(2)); // numGlyphs
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&be16(1000)); // unitsPerEm
        head[50..52].copy_from_slice(&be16(0)); // indexToLocFormat: short
        let hmtx = [be16(600), be16(0), be16(500), be16(0)].concat();
        let cmap_sub = cmap_a_only();
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&be16(0)); // version
        cmap.extend_from_slice(&be16(1)); // numTables
        cmap.extend_from_slice(&be16(3)); // platformID
        cmap.extend_from_slice(&be16(1)); // encodingID
        cmap.extend_from_slice(&be32(12));
        cmap.extend_from_slice(&cmap_sub);
        assemble_sfnt(&[
            (b"head", head),
            (b"hhea", hhea),
            (b"maxp", maxp),
            (b"hmtx", hmtx),
            (b"cmap", cmap),
            (b"loca", loca),
            (b"glyf", glyf),
        ])
    }

    #[test]
    fn provider_rasterizes_ttf_letterform_end_to_end() {
        use amazing_layout::font::ParsedFont;
        let src = FontSource::Ttf(ParsedFont::parse(&square_ttf()).unwrap());
        let provider = OutlineGlyphProvider::new(&src);

        let gb = provider.rasterize('A', 10.0).expect("'A' bitmap");
        assert_eq!((gb.width, gb.height), (5, 7));
        assert_eq!(gb.bearing_y, 7.0);
        assert_eq!(gb.advance, 5.0); // agrees with FontSource::measure
        assert!((src.measure("A", 10.0) - gb.advance).abs() < 1e-5);
        assert!(gb.pixels.chunks_exact(4).all(|p| p == [255, 255, 255, 255]));

        // Unmapped character: zero advance -> deliberately NO ink, so
        // painted words can never overlap.
        assert!(provider.rasterize('B', 10.0).is_none());
        assert!(provider.rasterize(' ', 10.0).is_none());
        assert!(provider.rasterize('A', 0.0).is_none());
        assert!(provider.rasterize('A', f32::NAN).is_none());

        // Scales linearly: double the size, doubled bitmap.
        let big = provider.rasterize('A', 20.0).expect("big bitmap");
        assert_eq!((big.width, big.height), (10, 14));
    }
}
