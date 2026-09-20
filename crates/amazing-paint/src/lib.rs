//! Amazing paint — Phase 4 software rasterizer: a laid-out
//! [`amazing_layout::LayoutBox`] tree in, an RGBA8 [`surface::Surface`]
//! (or `.ppm`/`.bmp` file bytes) out.
//!
//! Pipeline: layout tree → bounded provider-neutral display list → software
//! layer rasterization/compositing ([`paint::paint_document`]) → RGBA surface.
//!
//! # Implemented
//!
//! - [`surface::Surface`]: RGBA8 pixel buffer, top-left origin, opaque
//!   overwrites; clipped `fill_rect`; PPM (P6) and 24-bit BMP encoders
//!   (bottom-up rows with correct 4-byte row padding) plus save wrappers
//! - [`color::parse_css_color`]: hex (`#rgb/#rgba/#rrggbb/#rrggbbaa`),
//!   `rgb()`/`rgba()` in comma and space syntax, percentages, ~50 named
//!   colors including `transparent`; case-insensitive, panic-free
//! - [`paint::paint_document`]: recursive top-down painting — each box's
//!   padding-box background, then its four border strips (all non-none
//!   styles painted solid), then its line-box text fragments, then its
//!   children; flex children like block children
//! - decoded replaced images from an [`paint::ImageProvider`], scaled with
//!   clipped nearest-neighbor sampling and source-over RGBA compositing
//! - stacking-phase ordering, atomic nested contexts, full-subtree opacity,
//!   2D affine transforms, rectangular/rounded overflow clips, rounded
//!   backgrounds/borders, background-clip geometry, zero-blur shadows, and
//!   bounded transparent effect layers
//! - provider-neutral shaped text consumes layout's exact font/glyph ids,
//!   advances, offsets, colors, and inline decoration geometry, with bounded
//!   alpha/RGBA glyph images supplied through `amazing-text`
//! - [`text`]: legacy per-character advances from `FontSource::measure`,
//!   integer alpha compositing, and two compatibility
//!   [`text::GlyphProvider`] implementations: [`text::RectInkProvider`]
//!   (tidy ink bars on the baseline, the no-font default) and
//!   [`text::OutlineGlyphProvider`] (real TrueType letterforms — outline
//!   decomposition via `amazing-layout`, quadratic flattening, nonzero-
//!   winding scanline fill with 4x4 supersampled coverage)
//!
//! # Deferred / deviations (documented at module level too)
//!
//! - Outline text has no hinting and flattens curves at a fixed 4
//!   subdivisions per segment; coverage is 16-level box supersampling.
//! - border styles other than `double` render solid; blurred/multiple shadows,
//!   3D transforms, blend modes, filters, outlines, and list markers remain
//!   deferred; legacy word painting still uses the containing box's color
//!   while the shaped path retains per-inline color/background
//! - the canvas starts opaque white; transparent backgrounds skip filling

pub mod color;
pub mod image;
mod display_list;
pub mod paint;
pub mod surface;
pub mod text;

pub use color::parse_css_color;
pub use paint::{
    paint_document, paint_document_with, paint_document_with_resources,
    paint_document_with_text_resources, ImageProvider, NoImages,
};
pub use surface::Surface;
pub use text::{draw_fragment, GlyphBitmap, GlyphProvider, OutlineGlyphProvider, RectInkProvider};
