//! Amazing CSS — Syntax-3 tokenizer, core parser, selector engine, cascade.
//!
//! **Status:** tokenizer + parser + selectors + cascade + specified-to-computed
//! value processing + built-in UA sheet, tracked against pinned upstream WPT.
//!
//! Modules, in dependency order:
//! - [`syntax`]: CSS Syntax Module Level 3 §4 tokenizer (see its module docs
//!   for the deviation list: no dedicated `url()` token, simplified EOF
//!   recovery, comments dropped).
//! - [`parser`]: qualified rules + declarations (css-syntax-3 §5) with
//!   spec-style error recovery and static variable-bearing `@supports`;
//!   unsupported at-rules are consumed and discarded.
//! - [`selectors`]: css-selectors-4 subset — complex selectors with all
//!   combinators, attribute operators, structural/link/interactive
//!   pseudo-classes, `:not()`/`:is()`/`:where()`, and §6.4 specificity.
//! - [`cascade`]: per-property cascade (importance → origin → inline →
//!   matched-selector specificity → declaration order) and a minimal built-in
//!   UA stylesheet. Its private computed-value module owns custom-property
//!   inheritance/cycles, `var()` substitution, CSS-wide keywords, and
//!   supported shorthand expansion.
//!
//! Crate-level deviations (deliberate, documented per module):
//! - `@supports` handles static variable-bearing declaration conditions;
//!   other at-rule bodies (`@media`, `@layer`, `@scope`, `@keyframes`, …) are
//!   balanced then dropped
//! - property-specific computed grammars remain string-backed outside custom
//!   properties and CSS-wide defaulting
//! - pseudo-elements invalidate their rule (nothing can paint them yet)
//! - presentational HTML attributes and `<style media>` are ignored
//!
//! This crate depends only on `amazing-dom` (+ `amazing-common`) per the
//! locked component graph; HTML-level integration lives in the driver
//! crate's tests (`crates/amazing/tests/`).
//!
//! Entry points: [`parse_stylesheet`] / [`parse_declarations`] /
//! [`selectors::parse_selector`] / [`compute`].

pub mod cascade;
mod computed;
pub mod parser;
pub mod selectors;
pub mod syntax;

pub use cascade::{compute, StyledDom};
pub use computed::ComputedStyle;
pub use parser::{
    parse_declarations, parse_font_faces, parse_stylesheet, Declaration, FontFaceRule,
    FontSource, Rule, Stylesheet,
};
