//! HTML Canvas 2D rendering engine: paths, fill/stroke, compositing, text and images.
#![forbid(unsafe_code)]

mod context;
mod geometry;
mod image;
mod paint;
mod path;
mod png;
mod text;

pub use context::*;
pub use geometry::*;
pub use image::*;
pub use paint::*;
pub use path::*;
pub use png::encode_png;
pub use text::*;
