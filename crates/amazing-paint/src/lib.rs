//! Amazing paint — pure-Rust software rasterizer.
//!
//! **Status: Phase 4 (not started).**
//!
//! Plan:
//! - Scanline polygon fill with per-scanline coverage accumulation (anti-
//!   aliasing via 4x vertical supersampling of coverage, no float blending
//!   in the inner loop).
//! - Rgba8 pixel buffer as the single output surface. Before the GUI shell
//!   exists (Phase 8), frames dump to `.ppm` (trivial writer) — that keeps
//!   this phase's exit criteria testable headless.
//! - Glyph rasterization: quadratic Bézier flattening from TTF outlines
//!   parsed by amazing-layout's font module.

#[cfg(test)]
mod tests {
    #[test]
    fn phase4_not_started() {
        assert!(true);
    }
}
