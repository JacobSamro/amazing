//! Amazing CSS — Syntax-3 tokenizer, core parser, selector engine, cascade.
//!
//! **Status:** tokenizer + parser + selectors + cascade + specified-to-computed
//! value processing + built-in UA sheet, tracked against pinned upstream WPT.
//!
//! Modules, in dependency order:
//! - [`syntax`]: CSS Syntax Module Level 3 §4 tokenizer (see its module docs
//!   for the deviation list: simplified EOF
//!   recovery, comments dropped).
//! - [`parser`]: qualified rules + declarations (css-syntax-3 §5) with
//!   spec-style error recovery, layers, scope, and static variable-bearing `@supports`;
//!   unsupported at-rules are consumed and discarded.
//! - [`selectors`]: css-selectors-4 subset — complex selectors with all
//!   combinators, attribute operators, structural/link/interactive
//!   pseudo-classes, `:not()`/`:is()`/`:where()`, and §6.4 specificity.
//! - [`cascade`]: per-property cascade (importance → origin → inline →
//!   layer → matched-selector specificity → scope proximity → declaration order) and a minimal built-in
//!   UA stylesheet. Its private computed-value module owns custom-property
//!   inheritance/cycles, `var()` substitution, CSS-wide keywords, and
//!   supported shorthand expansion.
//!
//! Crate-level deviations (deliberate, documented per module):
//! - `@supports` handles static variable-bearing declaration conditions;
//!   `@keyframes` is retained for the compositor; unsupported at-rule bodies
//!   such as `@media` are balanced then dropped
//! - typed computed values cover primitive numeric/color values and common
//!   layout keywords; remaining compound grammars are explicitly unparsed
//! - implicit scope requires callers to preserve sheet identity and populate
//!   `Stylesheet::scope_root`; ownerless sheets use the document default
//! - CSS nesting (`&`) and layer-qualified imports remain unsupported
//! - presentational HTML attributes and `<style media>` are ignored
//!
//! This crate depends only on `amazing-dom` (+ `amazing-common`) per the
//! locked component graph; HTML-level integration lives in the driver
//! crate's tests (`crates/amazing/tests/`).
//!
//! Entry points: [`parse_stylesheet`] / [`parse_declarations`] /
//! [`selectors::parse_selector`] / [`compute`].

pub mod cascade;
pub mod animation;
pub mod color;
mod change_classification;
mod computed;
mod font_properties;
pub mod generated;
mod math;
pub mod parser;
pub mod selectors;
mod svg_presentation;
pub mod syntax;
pub mod values;

pub use cascade::{
    ElementFill, ElementPaintStyle, ElementStyleSource, ElementStyleValue, StyledDom, compute,
    compute_with_animations, AnimatedOrigin, AnimatedStyleValue, AnimatedValues,
    compute_element_paint_style,
    recompute_roots, AttributeStyleUpdate, CachedBoxSize, CompositorStyleDelta, MutationKind, RecalcRoots, RenderMutation, StyleDelta, StyleInvalidator, StyleMismatch,
};
pub use computed::ComputedStyle;
pub use change_classification::{StyleChangeKind, classify_style_property};
pub use animation::{Keyframe, KeyframesRule, keyframes_rules};
pub use parser::{
    Declaration, FontFaceRule, FontSource, Rule, Stylesheet, parse_declarations, parse_font_faces,
    parse_stylesheet,
};
