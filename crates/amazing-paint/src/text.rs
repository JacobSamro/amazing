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

use amazing_layout::font::FontSource;
use crate::surface::Surface;

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

/// Draws one run of text with its pen origin at `(x, baseline_y)` using
/// per-character advances from `fonts`. Coordinates are rounded to whole
/// pixels; every glyph is clipped to the surface. Non-finite coordinates
/// are a silent no-op. Never panics.
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
            blit_glyph(surface, gx, gy, &gb, color);
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
fn blit_glyph(surface: &mut Surface, gx: i64, gy: i64, gb: &GlyphBitmap, color: [u8; 4]) {
    if gb.pixels.len() < gb.width * gb.height * 4 {
        return; // malformed provider output: skip rather than panic
    }
    let sw = surface.width as i64;
    let sh = surface.height as i64;
    for row in 0..gb.height as i64 {
        let dy = gy + row;
        if dy < 0 || dy >= sh {
            continue;
        }
        for col in 0..gb.width as i64 {
            let dx = gx + col;
            if dx < 0 || dx >= sw {
                continue;
            }
            let src = &gb.pixels[((row as usize) * gb.width + col as usize) * 4..][..4];
            let cov = src[3] as u32;
            if cov == 0 {
                continue;
            }
            let dst = surface.get_pixel(dx as u32, dy as u32).unwrap_or([0, 0, 0, 255]);
            if cov >= 255 {
                surface.set_pixel(dx as u32, dy as u32, [
                    color[0], color[1], color[2], 255,
                ]);
                continue;
            }
            let inv = 255 - cov;
            let blend = |sc: u8, dc: u8| -> u8 {
                ((sc as u32 * cov + dc as u32 * inv + 127) / 255) as u8
            };
            surface.set_pixel(dx as u32, dy as u32, [
                blend(color[0], dst[0]),
                blend(color[1], dst[1]),
                blend(color[2], dst[2]),
                255,
            ]);
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
        draw_fragment(&mut s, 2.0, 15.0, "W", 10.0, BLACK, &fonts, &RectInkProvider);
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
        draw_fragment(&mut s, 5.0, 10.0, "W", 10.0, BLACK, &fonts, &RectInkProvider);
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
        draw_fragment(&mut s, 2.0, 12.0, "AB", 10.0, BLACK, &fonts, &RectInkProvider);
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
        draw_fragment(&mut s, 2.0, 12.0, "A B", 10.0, BLACK, &fonts, &RectInkProvider);
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
        draw_fragment(&mut s, 6.0, 30.0, "WWWWWWWWW", 10.0, BLACK, &fonts, &RectInkProvider);
        draw_fragment(&mut s, -50.0, 3.0, "WWWWWW", 10.0, BLACK, &fonts, &RectInkProvider);
        draw_fragment(&mut s, 0.0, f32::NAN, "W", 10.0, BLACK, &fonts, &RectInkProvider);
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
        draw_fragment(&mut s, 1.0, 10.0, "W", 10.0, BLACK, &fonts, &RectInkProvider);
        assert_eq!(s.get_pixel(2, 9), Some(BLACK)); // fully covered pixel
        assert_eq!(s.get_pixel(9, 5), Some(bg)); // outside the bar
    }

    #[test]
    fn zero_and_negative_sizes_draw_nothing() {
        let mut s = canvas(8, 8);
        let fonts = FontSource::fallback();
        draw_fragment(&mut s, 1.0, 4.0, "W", 0.0, BLACK, &fonts, &RectInkProvider);
        draw_fragment(&mut s, 1.0, 4.0, "W", -3.0, BLACK, &fonts, &RectInkProvider);
        draw_fragment(&mut s, 1.0, 4.0, "W", f32::NAN, BLACK, &fonts, &RectInkProvider);
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
}
