//! Amazing CSS — Syntax-3 tokenizer, core parser, selector engine, cascade.
//!
//! **Status: Phase 2 implemented** (tokenizer + parser + selectors +
//! cascade/inheritance + built-in UA sheet), pending WPT-style tracking
//! which begins in Phase 3 per plan.
//!
//! Modules, in dependency order:
//! - [`syntax`]: CSS Syntax Module Level 3 §4 tokenizer (see its module docs
//!   for the deviation list: no dedicated `url()` token, simplified EOF
//!   recovery, comments dropped).
//! - [`parser`]: qualified rules + declarations (css-syntax-3 §5) with
//!   spec-style error recovery; at-rules are consumed and discarded.
//! - [`selectors`]: css-selectors-4 subset — complex selectors with all
//!   combinators, attribute operators, structural/link/interactive
//!   pseudo-classes, `:not()`/`:is()`/`:where()`, and §6.4 specificity.
//! - [`cascade`]: per-property cascade (importance → origin → inline →
//!   specificity → order), standard inheritance, initial values for the
//!   tracked property set, and a minimal built-in UA stylesheet.
//!
//! Crate-level deviations (deliberate, documented per module):
//! - shorthand↔longhand interaction is not modeled (`margin` vs
//!   `margin-top` cascade independently until layout needs expansion)
//! - at-rule bodies (@media/@supports/@keyframes) are parsed for balance
//!   then dropped — no conditional rule processing
//! - pseudo-elements invalidate their rule (nothing can paint them yet)
//! - presentational HTML attributes and `<style media>` are ignored
//!
//! This crate depends only on `amazing-dom` (+ `amazing-common`) per the
//! locked component graph; HTML-level integration lives in the driver
//! crate's tests (`crates/amazing/tests/`).
//!
//! Entry points: [`parse_stylesheet`] / [`parse_declarations`] /
//! [`selectors::parse_selector`] / [`cascade::compute`].

pub mod cascade;
pub mod parser;
pub mod selectors;
pub mod syntax;

pub use cascade::{compute, ComputedStyle, StyledDom};
pub use parser::{parse_declarations, parse_stylesheet, Declaration, Rule, Stylesheet};
