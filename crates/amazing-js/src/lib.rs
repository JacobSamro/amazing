//! Amazing JS — ECMA-262 lexer, parser, and tree-walking interpreter.
//!
//! **Status: Phase 6, milestone 1 — lexical + syntactic front half.**
//! Largest single sub-project in Amazing; comparable in scope to Phases 0–5
//! combined. Strategy (locked): v1 is a tree-walking interpreter — no JIT,
//! per Ladybird's LibJS precedent — and Test262 is the conformance gate
//! from the first milestone onward.
//!
//! # What this milestone delivers
//!
//! A panic-free script front end over `std` only:
//!
//! ```
//! use amazing_js::parse_script;
//!
//! let program = parse_script("const add = (a, b) => a + b;").expect("parses");
//! assert_eq!(program.body.len(), 1);
//! ```
//!
//! - [`tokens`] — the full ES token vocabulary with longest-match
//!   punctuators, numeric/string literal storage, and the `newline_before`
//!   flag that drives ASI.
//! - [`lexer`] — the scanner: template-literal modes, escape sequences,
//!   positioned [`SyntaxError`]s. Regex literals are deferred (`/` is
//!   always division); see its module docs for where the division/regex
//!   ambiguity will be resolved.
//! - [`ast`] — the syntactic grammar as data (statements, expressions,
//!   functions/classes), with implemented-vs-deferred lists.
//! - [`parser`] — recursive descent + precedence climbing with correct
//!   automatic semicolon insertion, arrow cover-grammar speculation, and
//!   the `??`-mixing restriction.
//! - [`errors`] — [`SyntaxError`] with line/column positions.
//!
//! Deferred to later Phase 6 milestones (each module documents its own
//! list in detail): regex/tagged templates, destructuring everywhere,
//! modules (`import`/`export`), private class members, strict-mode early
//! errors, and then the interpreter itself (objects/prototypes, built-ins,
//! async/Promise, Map/Set).
//!
//! The interpreter back half will live alongside these modules and consume
//! [`ast::Program`]; nothing in this crate allocates unsafely or panics on
//! any input.

pub mod ast;
pub mod errors;
pub mod lexer;
pub mod parser;
pub mod tokens;

pub use ast::Program;
pub use errors::SyntaxError;
pub use parser::parse_script;

#[cfg(test)]
mod tests {
    use super::parse_script;

    #[test]
    fn parse_script_is_the_stable_public_entry_point() {
        let p = parse_script("let x = 1; x++;").expect("should parse");
        assert_eq!(p.body.len(), 2);
    }

    #[test]
    fn errors_surface_through_the_public_api() {
        let err = parse_script("let = ;").expect_err("should reject");
        assert!(err.line >= 1 && err.column >= 1);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn empty_input_yields_an_empty_program() {
        let p = parse_script("").expect("empty script parses");
        assert!(p.body.is_empty());
        let p = parse_script("// only a comment\n").expect("comment-only parses");
        assert!(p.body.is_empty());
    }
}
