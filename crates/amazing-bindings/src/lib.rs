//! DOM interfaces installed into the JS runtime.
//!
//! Today every JS-facing DOM wrapper lives in `amazing-js/src/interp.rs` and in
//! driver adapters, which means a DOM API added in `amazing-dom` cannot become
//! visible to a WPT test without editing another lane's crate. This crate is
//! that seam: `amazing-dom` owns the algorithms, `amazing-js` owns execution,
//! and the binding layer installs one onto the other.
//!
//! Deliberately depends on `amazing-dom` only. The install direction is
//! inverted through the host trait declared in this crate and implemented by
//! the driver using runtime capabilities, so this crate never imports the
//! interpreter and the dependency graph keeps pointing downward
//! per the component graph in ARCHITECTURE.md.
//!
//! std-only, like every engine core. Skeleton registered by the integrator;
//! the `dom-bindings` lane implements it, coordinating the runtime-side host
//! trait with `js-runtime`.

#![forbid(unsafe_code)]

pub mod host;

mod interfaces;
mod invoke;
pub use amazing_dom::{Dom, DomError, NodeIdx};
pub use interfaces::{install, INTERFACES};
pub use invoke::invoke;

#[cfg(test)]
mod tests;
