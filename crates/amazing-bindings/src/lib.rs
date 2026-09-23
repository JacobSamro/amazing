//! DOM interfaces installed into the JS runtime.
//!
//! Today every JS-facing DOM wrapper lives in `amazing-js/src/interp.rs` and in
//! driver adapters, which means a DOM API added in `amazing-dom` cannot become
//! visible to a WPT test without editing another lane's crate. This crate is
//! that seam: `amazing-dom` owns the algorithms, `amazing-js` owns execution,
//! and the binding layer installs one onto the other.
//!
//! Depends on the native DOM, URL, and Fetch body models. The install direction is
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
pub mod storage_interfaces;
pub mod dom_exception;

mod interfaces;
mod invoke;
mod range_bindings;
mod factories;
mod collections;
mod traversal_bindings;
mod event_bindings;
mod abort_bindings;
mod document_bindings;
mod element_reflection;
pub use abort_bindings::fire_abort_timeout;
mod mutation_bindings;
mod shadow_bindings;
mod custom_element_bindings;
mod form_bindings;
mod form_reflection;
mod web_bindings;
mod attr_bindings;
mod character_data_bindings;
mod tokenlist;
mod html_reflection;
mod markup_bindings;
pub use mutation_bindings::deliver_mutation_checkpoint;
pub use amazing_dom::{Dom, DomError, NodeIdx};
pub use amazing_fetch::api::{Blob, BlobPart, File, FormData};
pub use interfaces::{html_element_interface, install, InstallReport, OptionalCapability, INTERFACES};
pub use invoke::{invoke, supported_index_count, supported_names};
pub use custom_element_bindings::{deliver_custom_element_reactions, registry_for_document};

#[cfg(test)]
mod tests;
