//! Amazing paint — Phase 4 software rasterizer: a laid-out
//! [`amazing_layout::LayoutBox`] tree in, an RGBA8 [`surface::Surface`]
//! (or `.ppm`/`.bmp` file bytes) out.
//!
//! Pipeline: layout tree → painter's-algorithm traversal
//! ([`paint::paint_document`]) → background fills, solid border strips and
//! rect-ink text ([`text`]) onto one opaque canvas, everything clipped to
//! the viewport.
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
//! - [`text`]: per-character advances from the same `FontSource::measure`
//!   numbers layout used, ink rectangles sitting on the line baseline,
//!   integer alpha compositing; [`text::GlyphProvider`] is the seam for
//!   future real glyph rasters
//!
//! # Deferred / deviations (documented at module level too)
//!
//! - REAL GLYPH OUTLINES deferred: no TTF glyf/CFF flattening, no
//!   anti-aliasing/scanline coverage yet — text renders as tidy ink bars
//!   via the default [`text::RectInkProvider`]
//! - all border styles render solid; no radius/shadows/images/markers/
//!   opacity/stacking contexts; word-level colors inside one line use the
//!   containing block's color (`WordFrag` carries no style)
//! - the canvas starts opaque white; transparent backgrounds skip filling

pub mod color;
pub mod paint;
pub mod surface;
pub mod text;

pub use color::parse_css_color;
pub use paint::{paint_document, paint_document_with};
pub use surface::Surface;
pub use text::{draw_fragment, GlyphBitmap, GlyphProvider, RectInkProvider};
