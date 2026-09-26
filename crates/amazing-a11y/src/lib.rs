//! Accessibility tree for Amazing: HTML-AAM roles, accname-1.2 names, ARIA
//! states and layout bounds, computed from the DOM, computed style and the
//! layout tree.
//!
//! The public seam is [`AxTree`], built by [`build`] (or the HTML convenience
//! entry point [`build_from_html`]). [`AxTree::snapshot`] returns a flat,
//! detached [`SchemaSnapshot`] of plain data: node id, role, name, description,
//! states, children and bounds.
//!
//! ```no_run
//! use amazing_a11y::{build_from_html, Role};
//!
//! let tree = build_from_html(
//!     r#"<button aria-pressed="false">Play</button>"#,
//!     "",
//!     800.0,
//! );
//! let snapshot = tree.snapshot();
//! let root = snapshot.root.unwrap();
//! let button = snapshot.node(root).unwrap();
//! assert_eq!(button.role, Role::Button);
//! assert_eq!(button.name, "Play");
//! ```
//!
//! Engine-only roles (`statictext`, `linebreak`, …) carry lowercase names like
//! ARIA roles. `dom_node` on a snapshot node is the originating arena index, so
//! a driver can map a tree node back to the live document.

#![forbid(unsafe_code)]

pub mod accname;
pub mod aria;
pub mod bounds;
pub mod event;
pub mod host;
pub mod role;
pub mod tree;

pub use accname::NameContext;
pub use aria::{AxProperties, AxStates, InvalidState, Orientation, Relevant, Sort, Tristate};
pub use bounds::{AxRect, BoundsMap};
pub use event::{AxEvent, AxEventKind, Live, LiveRegionInfo, diff, live_region};
pub use role::Role;
pub use tree::{
    AxDocument, AxNode, AxNodeId, AxRelations, AxTree, SchemaNode, SchemaSnapshot, build,
    build_document, build_from_html, build_from_html_in_viewport, build_from_parsed_html,
};

#[cfg(test)]
mod accname_tests;
#[cfg(test)]
mod aria_tests;
#[cfg(test)]
mod bounds_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod relationships_tests;
#[cfg(test)]
mod role_tests;
#[cfg(test)]
mod text_tests;
#[cfg(test)]
mod tree_tests;
#[cfg(test)]
pub(crate) mod test_support;
