//! Syntax-error type shared by the lexer and parser.
//!
//! Implemented:
//! - [`SyntaxError`] with a message plus 1-based line/column of the offending
//!   token (columns count *characters*, not grapheme clusters or bytes).
//! - `Display` in the conventional `line:column: message` shape and
//!   `std::error::Error` so the crate composes with `Result` pipelines.
//!
//! Deferred (documented, not implemented):
//! - Byte/column offsets into the original source string (only line/column).
//! - Multi-error recovery: the lexer/parser stop at the first error.
//! - Error "kinds" taxonomy (early vs reference vs syntax); everything the
//!   front half rejects is a flat [`SyntaxError`].

use std::fmt;

/// A lexical or syntactic error, positioned at the 1-based source location
/// where it was detected.
///
/// Constructed only by the lexer and parser; public APIs in this crate are
/// panic-free and surface every rejection as this type via `Result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Human-readable description, e.g. `"unterminated string literal"`.
    pub message: String,
    /// 1-based line of the offending token.
    pub line: u32,
    /// 1-based column (in characters) of the offending token.
    pub column: u32,
}

impl SyntaxError {
    /// Builds an error at the given position from any message-like value.
    pub fn new(message: impl Into<String>, line: u32, column: u32) -> Self {
        SyntaxError {
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for SyntaxError {}

#[cfg(test)]
mod tests {
    use super::SyntaxError;

    #[test]
    fn display_includes_position_and_message() {
        let e = SyntaxError::new("unterminated template literal", 3, 17);
        assert_eq!(e.to_string(), "3:17: unterminated template literal");
    }

    #[test]
    fn fields_are_public_and_preserved() {
        let e = SyntaxError::new(String::from("boom"), 1, 1);
        assert_eq!(e.message, "boom");
        assert_eq!(e.line, 1);
        assert_eq!(e.column, 1);
    }

    #[test]
    fn implements_std_error() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        assert_error(&SyntaxError::new("x", 0, 0));
    }
}
