//! The RGBA8 pixel buffer and its encoders — the single output type of the
//! rasterizer.
//!
//! Pixels are stored row-major with a TOP-LEFT origin (`(0,0)` is the
//! top-left corner), four bytes per pixel in R, G, B, A order. All drawing
//! is opaque overwrite (source replaces destination); alpha is carried so
//! transparent fills can be recognized by callers, not because the buffer
//! composites.
//!
//! # Implemented
//!
//! - [`Surface::new`] / [`clear`](Surface::clear) /
//!   [`get_pixel`](Surface::get_pixel) / [`set_pixel`](Surface::set_pixel)
//! - [`fill_rect`](Surface::fill_rect): float rect in, clipped integer
//!   blit out (saturating casts; NaN maps to 0, infinities saturate)
//! - PPM (P6) encoding for headless exit-criteria dumps
//! - BMP (24-bit, bottom-up) encoding with correct 4-byte row padding
//! - `save_*` wrappers over `std::fs::write`
//!
//! # Deferred / deviations
//!
//! - No alpha compositing on the surface itself (text blending lives in
//!   [`crate::text`]); no anti-aliasing, no subpixel coverage yet.

use std::fs;

/// An RGBA8 raster surface, top-left origin, opaque-overwrite semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, RGBA per pixel.
    pub pixels: Vec<u8>,
}

impl Surface {
    /// Allocates a zeroed (transparent black) surface. Panics if the
    /// dimensions cannot possibly be backed by an allocation.
    pub fn new(width: u32, height: u32) -> Surface {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .unwrap_or_else(|| panic!("surface {width}x{height} too large"));
        Surface {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    /// Overwrites every pixel with `color`.
    pub fn clear(&mut self, color: [u8; 4]) {
        for px in self.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&color);
        }
    }

    fn idx(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(((y as usize) * self.width as usize + x as usize) * 4)
    }

    /// Reads one pixel; `None` when out of bounds.
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let i = self.idx(x, y)?;
        Some([
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ])
    }

    /// Writes one pixel (opaque overwrite); silently ignores out-of-bounds
    /// coordinates so painters can draw without pre-clipping.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if let Some(i) = self.idx(x, y) {
            self.pixels[i..i + 4].copy_from_slice(&color);
        }
    }

    /// Fills an axis-aligned rectangle given in f32 CSS-pixel space.
    ///
    /// Edges are rounded to whole pixels (`f32::round`, saturating casts:
    /// NaN rounds through 0, ±inf saturate), then intersected with the
    /// surface. Degenerate rects (negative/zero size after rounding) and
    /// rects fully outside the surface are no-ops.
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        let rx = |v: f32| -> i64 {
            if v.is_nan() {
                0
            } else {
                v.round() as i64 // saturating: inf -> i64::MAX/MIN
            }
        };
        let x0 = rx(x);
        let y0 = rx(y);
        let x1 = rx(x + w);
        let y1 = rx(y + h);
        // Inverted comparison so NaN edges (e.g. +inf + -inf width) fail
        // the check and become no-ops; `<=` alone would admit NaN.
        if !(x1 > x0) || !(y1 > y0) {
            return;
        }
        let cx0 = x0.max(0).min(self.width as i64) as usize;
        let cy0 = y0.max(0).min(self.height as i64) as usize;
        let cx1 = x1.max(0).min(self.width as i64) as usize;
        let cy1 = y1.max(0).min(self.height as i64) as usize;
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        let run = (cx1 - cx0) * 4;
        let mut row = vec![0u8; run];
        for chunk in row.chunks_exact_mut(4) {
            chunk.copy_from_slice(&color);
        }
        let stride = self.width as usize * 4;
        for py in cy0..cy1 {
            let start = py * stride + cx0 * 4;
            self.pixels[start..start + run].copy_from_slice(&row);
        }
    }

    /// Encodes a binary PPM (P6, maxval 255). Rows go top-down; alpha is
    /// dropped (PPM has no alpha channel).
    pub fn encode_ppm(&self) -> Vec<u8> {
        let header = format!("P6\n{} {}\n255\n", self.width, self.height);
        let mut out = Vec::with_capacity(header.len() + (self.pixels.len() / 4) * 3);
        out.extend_from_slice(header.as_bytes());
        for px in self.pixels.chunks_exact(4) {
            out.extend_from_slice(&px[0..3]);
        }
        out
    }

    /// Encodes an uncompressed 24-bit BMP: 14-byte BITMAPFILEHEADER +
    /// 40-byte BITMAPINFOHEADER, positive biHeight (bottom-up rows), each
    /// row padded with zero bytes up to a multiple of 4, channels BGR.
    pub fn encode_bmp(&self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        // Row length padded to 4 bytes: e.g. width 3 -> 9 data bytes -> pad
        // 3 -> 12 stored bytes per row.
        let row_size = (w * 3).div_ceil(4) * 4;
        let pad = row_size - w * 3;
        let image_size = row_size * h;
        let file_size = 54 + image_size;

        let mut out = Vec::with_capacity(file_size);
        // --- BITMAPFILEHEADER (14 bytes) ---
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&54u32.to_le_bytes()); // offset to pixel data

        // --- BITMAPINFOHEADER (40 bytes) ---
        out.extend_from_slice(&40u32.to_le_bytes()); // biSize
        out.extend_from_slice(&(self.width as i32).to_le_bytes());
        out.extend_from_slice(&(self.height as i32).to_le_bytes()); // + => bottom-up
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&(image_size as u32).to_le_bytes());
        out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi x
        out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi y
        out.extend_from_slice(&0u32.to_le_bytes()); // palette colors
        out.extend_from_slice(&0u32.to_le_bytes()); // important colors

        // --- pixel rows, bottom-up, BGR + zero padding ---
        for y in (0..self.height).rev() {
            for x in 0..w {
                let p = &self.pixels[(y as usize * w + x) * 4..][..4];
                out.push(p[2]); // B
                out.push(p[1]); // G
                out.push(p[0]); // R
            }
            out.extend(std::iter::repeat_n(0u8, pad));
        }
        out
    }

    /// Encodes and writes a `.ppm` file.
    pub fn save_ppm(&self, path: &str) -> std::io::Result<()> {
        fs::write(path, self.encode_ppm())
    }

    /// Encodes and writes a `.bmp` file.
    pub fn save_bmp(&self, path: &str) -> std::io::Result<()> {
        fs::write(path, self.encode_bmp())
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    #[test]
    fn new_surface_starts_transparent_black() {
        let s = Surface::new(20, 10);
        assert_eq!(s.width, 20);
        assert_eq!(s.height, 10);
        assert_eq!(s.pixels.len(), 20 * 10 * 4);
        assert_eq!(s.get_pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(s.get_pixel(19, 9), Some([0, 0, 0, 0]));
        assert_eq!(s.get_pixel(20, 9), None); // x out of bounds
        assert_eq!(s.get_pixel(0, 10), None); // y out of bounds
    }

    #[test]
    fn set_and_get_roundtrip_with_bounds_checks() {
        let mut s = Surface::new(5, 5);
        s.set_pixel(2, 3, RED);
        assert_eq!(s.get_pixel(2, 3), Some(RED));
        // Out-of-bounds writes are ignored, not panics.
        s.set_pixel(5, 0, BLUE);
        s.set_pixel(0, 5, BLUE);
        s.set_pixel(u32::MAX, u32::MAX, BLUE);
        assert_eq!(s.get_pixel(2, 3), Some(RED)); // untouched elsewhere
    }

    #[test]
    fn clear_overwrites_everything() {
        let mut s = Surface::new(7, 3);
        s.clear(BLUE);
        for y in 0..3 {
            for x in 0..7 {
                assert_eq!(s.get_pixel(x, y), Some(BLUE), "at {x},{y}");
            }
        }
    }

    #[test]
    fn fill_rect_inside_surface_is_pixel_exact() {
        let mut s = Surface::new(20, 10);
        s.fill_rect(2.0, 1.0, 5.0, 3.0, RED);
        assert_eq!(s.get_pixel(1, 1), Some([0, 0, 0, 0])); // left of rect
        assert_eq!(s.get_pixel(2, 1), Some(RED)); // first column
        assert_eq!(s.get_pixel(6, 3), Some(RED)); // last covered pixel
        assert_eq!(s.get_pixel(7, 3), Some([0, 0, 0, 0])); // past right edge
        assert_eq!(s.get_pixel(4, 4), Some([0, 0, 0, 0])); // below bottom edge
    }

    #[test]
    fn fill_rect_clips_negative_and_overflowing_edges() {
        let mut s = Surface::new(20, 10);
        // Extends far off every side; only the visible window gets painted.
        s.fill_rect(-100.0, -100.0, 250.0, 250.0, RED);
        assert_eq!(s.get_pixel(0, 0), Some(RED));
        assert_eq!(s.get_pixel(19, 9), Some(RED));

        let mut s = Surface::new(4, 4);
        s.clear([1, 2, 3, 255]);
        // Fully outside: no-op.
        s.fill_rect(-50.0, -50.0, 10.0, 10.0, RED);
        s.fill_rect(100.0, 100.0, 10.0, 10.0, RED);
        // Degenerate sizes: no-op, must not panic.
        s.fill_rect(1.0, 1.0, 0.0, 5.0, RED);
        s.fill_rect(1.0, 1.0, 5.0, -1.0, RED);
        s.fill_rect(f32::NAN, 0.0, 5.0, 5.0, RED);
        s.fill_rect(0.0, f32::INFINITY, 5.0, 5.0, RED);
        assert_eq!(s.get_pixel(0, 0), Some([1, 2, 3, 255]));
        assert_eq!(s.get_pixel(3, 3), Some([1, 2, 3, 255]));
    }

    #[test]
    fn ppm_header_matches_hand_computed_bytes() {
        let mut s = Surface::new(20, 10);
        s.set_pixel(0, 0, RED);
        s.set_pixel(1, 0, BLUE);
        let ppm = s.encode_ppm();
        // Header: "P6\n20 10\n255\n" = 3 + 6 + 4 = 13 bytes.
        assert_eq!(&ppm[..13], b"P6\n20 10\n255\n");
        // Payload: 20*10*3 RGB bytes right after the header.
        assert_eq!(ppm.len(), 13 + 600);
        assert_eq!(&ppm[13..16], &[255, 0, 0]); // first pixel, no alpha byte
        assert_eq!(&ppm[16..19], &[0, 0, 255]);
    }

    #[test]
    fn bmp_odd_width_pads_rows_to_four_bytes() {
        // Width 3: 3*3 = 9 data bytes per row -> padded to 12 (pad = 3).
        let mut s = Surface::new(3, 2);
        s.set_pixel(0, 1, RED); // BOTTOM image row (bottom-up file order!)
        let bmp = s.encode_bmp();

        assert_eq!(&bmp[0..2], b"BM");
        // bfSize = 14 + 40 + (12 rows * 2) = 78.
        assert_eq!(&bmp[2..6], &78u32.to_le_bytes());
        assert_eq!(&bmp[6..10], &[0, 0, 0, 0]); // reserved
        assert_eq!(&bmp[10..14], &54u32.to_le_bytes()); // pixel offset
        assert_eq!(&bmp[14..18], &40u32.to_le_bytes()); // biSize
        assert_eq!(&bmp[18..22], &3i32.to_le_bytes()); // biWidth
        assert_eq!(&bmp[22..26], &2i32.to_le_bytes()); // biHeight (+ = bottom-up)
        assert_eq!(&bmp[26..28], &1u16.to_le_bytes()); // planes
        assert_eq!(&bmp[28..30], &24u16.to_le_bytes()); // bpp
        assert_eq!(&bmp[30..34], &0u32.to_le_bytes()); // BI_RGB
        assert_eq!(&bmp[34..38], &24u32.to_le_bytes()); // biSizeImage = 12*2

        // Total file size including both pads.
        assert_eq!(bmp.len(), 78);

        // First stored row is the BOTTOM image row: red pixel as BGR.
        assert_eq!(&bmp[54..57], &[0, 0, 255]);
        // ...then two opaque-black pixels, then the 3 pad bytes (zeros).
        assert_eq!(&bmp[57..63], &[0, 0, 0, 0, 0, 0]);
        assert_eq!(&bmp[63..66], &[0, 0, 0]); // padding of row 1
                                              // Second stored row is the TOP image row (all black).
        assert_eq!(&bmp[66..69], &[0, 0, 0]);
        assert_eq!(&bmp[75..78], &[0, 0, 0]); // padding of row 0
    }

    #[test]
    fn bmp_even_width_needs_no_padding() {
        // Width 20: 60 data bytes, already a multiple of 4 -> pad = 0.
        let s = Surface::new(20, 10);
        let bmp = s.encode_bmp();
        assert_eq!(bmp.len(), 54 + 60 * 10);
        assert_eq!(&bmp[34..38], &(60u32 * 10).to_le_bytes());
        // Every stored row is exactly 60 bytes: total minus header divides.
        assert_eq!((bmp.len() - 54) % 60, 0);
    }

    #[test]
    fn save_encoders_agree_with_in_memory_encoding() {
        // Windows-safe: everything stays in memory except a temp write we
        // clean up; no hardcoded paths beyond the std temp dir convention.
        let mut s = Surface::new(4, 2);
        s.fill_rect(0.0, 0.0, 4.0, 1.0, RED);
        let dir = std::env::temp_dir();
        let ppm_path = dir.join("amazing_paint_test_surface.ppm");
        let bmp_path = dir.join("amazing_paint_test_surface.bmp");
        s.save_ppm(ppm_path.to_str().expect("utf8 temp path"))
            .expect("ppm write");
        s.save_bmp(bmp_path.to_str().expect("utf8 temp path"))
            .expect("bmp write");
        assert_eq!(std::fs::read(&ppm_path).expect("read back"), s.encode_ppm());
        assert_eq!(std::fs::read(&bmp_path).expect("read back"), s.encode_bmp());
        let _ = std::fs::remove_file(&ppm_path);
        let _ = std::fs::remove_file(&bmp_path);
    }
}
