//! Ferric layout — box model, block/inline formatting contexts.
//!
//! **Status: Phase 3 (not started); flexbox/grid explicitly deferred to 3.5.**
//!
//! Phase 3 scope (in order):
//! - CSS box model: margin/border/padding/content, containing-block resolution
//! - Block formatting context: vertical stacking, margin collapsing
//! - Inline formatting context: line boxes, inline-level boxes, line breaking
//! - Text: hand-written TTF/OTF parser for glyph metrics (advance widths,
//!   ascender/descender) — no font crates; shaping limited to simple scripts v1
//!
//! Output is a layout tree: absolute device-pixel rectangles per box.

#[cfg(test)]
mod tests {
    #[test]
    fn phase3_not_started() {
        assert!(true);
    }
}
