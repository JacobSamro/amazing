//! Ferric CSS parser, selector matching, and cascade.
//!
//! **Status: Phase 2 (not started).**
//!
//! Phase 2 will implement:
//! - Tokenizer per CSS Syntax Module Level 3 §4 ("css-syntax-3")
//! - Parser for the core grammar: qualified rules, declarations
//! - Selector matching (type/class/id/attribute/pseudo-class, all combinators)
//!   with correct specificity (§6.4 of css-selectors-4)
//! - Cascade + inheritance producing computed styles over `ferric_dom::Dom`
//!
//! Tokenizer input comes from `ferric_common::ByteReader`.

#[cfg(test)]
mod tests {
    #[test]
    fn phase2_not_started() {
        assert!(true);
    }
}
