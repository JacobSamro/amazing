//! Std-only SVG document model and renderer.
//!
//! It is split out of `amazing-image` so the engine's root workspace can render
//! inline SVG without pulling in that crate's third-party raster codecs. See the
//! primary integrator's inline-SVG decision. replaced-content owns this crate
//! and moves the renderer here from `amazing-image/src/svg*.rs`;
//! `amazing-image` then re-exports it for the shell.
#![forbid(unsafe_code)]

/// Independent input, geometry, output and decoder working-allocation budgets.
#[derive(Clone, Copy, Debug)]
pub struct DecodeLimits {
    pub max_input_bytes: usize,
    pub max_dimension: u32,
    pub max_rgba_bytes: u64,
    pub max_decoder_bytes: u64,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_dimension: 8192,
            max_rgba_bytes: 64 * 1024 * 1024,
            max_decoder_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InputLimit,
    GeometryLimit,
    AllocationLimit,
    InvalidImage,
    UnsupportedSvg,
    DecoderPanic,
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DecodeError {}


mod svg;
pub use svg::{SvgDocument, decode, from_dom, render_svg};
