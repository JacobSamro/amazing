//! Ferric JS — ECMA-262 lexer, parser, tree-walking interpreter.
//!
//! **Status: Phase 6 (not started).** Largest single sub-project in Ferric;
//! comparable in scope to Phases 0–5 combined.
//!
//! Strategy (locked):
//! - v1 is a tree-walking interpreter — no JIT. Precedent: Ladybird's LibJS.
//! - Test262 is the conformance gate from the first milestone onward; report
//!   pass rates per language-area milestone, never estimates.
//! - Milestone order: lexical grammar → expressions → statements/functions →
//!   objects/prototypes → built-ins (Object/Array/String/Number) →
//!   Map/Set/Promise+async → strict mode → remaining built-ins.

#[cfg(test)]
mod tests {
    #[test]
    fn phase6_not_started() {
        assert!(true);
    }
}
