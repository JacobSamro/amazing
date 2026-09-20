//! ECMA-262 RegExp: pattern parsing, compilation, and matching.
//!
//! Hand-written against ECMA-262 §22.2, like every other parser in this engine —
//! no `regex` crate, no `regex-syntax`. Engine cores are **std-only**.
//!
//! This is on the critical path: `amazing-js` lexes `/` as always-division and
//! defers regex literals, which makes the unmodified upstream
//! `resources/testharness.js` unparseable (SyntaxError at 581:43) and blocks the
//! WPT testharness runner, and with it every script-driven conformance suite.
//!
//! Skeleton registered by the integrator so the workspace resolves; the
//! `js-lang` lane implements it under ticket JS-2.

#![forbid(unsafe_code)]
