//! Errors surfaced by the WebAssembly decoder, validator and interpreter.
//!
//! The four kinds mirror the specification's failure taxonomy so the `.wast`
//! runner can classify an `assert_malformed`, `assert_invalid`,
//! `assert_unlinkable` and `assert_trap` without guessing:
//!
//! - [`ErrorKind::Malformed`] — the byte stream is not a well-formed module
//!   (bad section order, truncated LEB, bad UTF-8, trailing bytes, ...).
//! - [`ErrorKind::Invalid`] — well-formed but fails type checking.
//! - [`ErrorKind::Unlinkable`] — valid but cannot be instantiated (missing
//!   import, incompatible import type, out-of-range active segment, ...).
//! - [`ErrorKind::Trap`] — a runtime trap.
//! - [`ErrorKind::Exhaustion`] — the call stack or the value/table/memory
//!   resource is exhausted at runtime.

use core::fmt;

/// Classification of a WebAssembly failure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// Not a syntactically well-formed module.
    Malformed,
    /// Well-formed but not type-correct.
    Invalid,
    /// Cannot be linked or instantiated.
    Unlinkable,
    /// Runtime trap.
    Trap,
    /// Runtime resource exhaustion (call stack, table, memory).
    Exhaustion,
    /// A feature outside the WASM-1 scope (SIMD, threads, GC, ...).
    Unsupported,
}

impl ErrorKind {
    /// The `.wast` assertion category this kind satisfies.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Malformed => "malformed",
            ErrorKind::Invalid => "invalid",
            ErrorKind::Unlinkable => "unlinkable",
            ErrorKind::Trap => "trap",
            ErrorKind::Exhaustion => "exhaustion",
            ErrorKind::Unsupported => "unsupported",
        }
    }
}

/// A WebAssembly error with its classification and a human-readable message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WasmError {
    /// Failure category.
    pub kind: ErrorKind,
    /// Description, ideally matching the upstream testsuite expectation.
    pub message: String,
}

impl WasmError {
    /// Build an error of `kind`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        WasmError { kind, message: message.into() }
    }

    /// A malformed-module error.
    pub fn malformed(message: impl Into<String>) -> Self {
        WasmError::new(ErrorKind::Malformed, message)
    }

    /// A validation error.
    pub fn invalid(message: impl Into<String>) -> Self {
        WasmError::new(ErrorKind::Invalid, message)
    }

    /// A link/instantiation error.
    pub fn unlinkable(message: impl Into<String>) -> Self {
        WasmError::new(ErrorKind::Unlinkable, message)
    }

    /// A runtime trap.
    pub fn trap(message: impl Into<String>) -> Self {
        WasmError::new(ErrorKind::Trap, message)
    }

    /// A runtime exhaustion.
    pub fn exhaustion(message: impl Into<String>) -> Self {
        WasmError::new(ErrorKind::Exhaustion, message)
    }

    /// A feature outside the lane's scope.
    pub fn unsupported(message: impl Into<String>) -> Self {
        WasmError::new(ErrorKind::Unsupported, message)
    }
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for WasmError {}

/// Convenience result alias used throughout the crate.
pub type Result<T> = core::result::Result<T, WasmError>;
