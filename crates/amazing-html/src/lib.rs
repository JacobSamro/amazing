//! Amazing HTML parser — WHATWG HTML Standard tokenizer + tree construction.
//!
//! **Status: Phase 1 (not started).**
//!
//! Phase 1 will implement the tokenizer state machine from
//! <https://html.spec.whatwg.org/multipage/parsing.html#tokenization> with
//! states named exactly as in the spec (`Data state`, `Tag open state`, …)
//! because the spec's error-recovery behavior is the interop contract.
//!
//! Tree construction follows §13.2.4, including the "in body" insertion mode
//! rules that produce browser-compatible handling of mismatched tags,
//! unclosed `<li>`/`<p>`, and implied `<tbody>`.
//!
//! Output is a `amazing_dom::Dom` tree built through arena indices —
//! never `Rc<RefCell<Node>>`.

#[cfg(test)]
mod tests {
    /// Placeholder gate: proves the crate links and runs under `cargo test`
    /// until the real tokenizer lands in Phase 1.
    #[test]
    fn phase1_not_started() {
        assert!(true);
    }
}
