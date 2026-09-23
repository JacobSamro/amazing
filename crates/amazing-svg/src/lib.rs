//! Std-only SVG document model and renderer.
//!
//! It is split out of `amazing-image` so the engine's root workspace can render
//! inline SVG without pulling in that crate's third-party raster codecs. See the
//! primary integrator's inline-SVG decision. replaced-content owns this crate
//! and moves the renderer here from `amazing-image/src/svg*.rs`;
//! `amazing-image` then re-exports it for the shell.
#![forbid(unsafe_code)]
