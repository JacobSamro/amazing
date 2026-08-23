//! Shared byte-stream and text utilities for the Amazing engine.
//!
//! This crate is the foundation for the hand-written HTML (Phase 1) and CSS
//! (Phase 2) tokenizers. It provides a `ByteReader` over an in-memory byte
//! slice with position tracking, plus ASCII/UTF-8 helpers that mirror the
//! predicates used throughout the WHATWG and CSS Syntax specs.

/// A cursor over an in-memory byte stream.
///
/// Tokenizers need cheap lookahead, backtracking to saved positions, and
/// line/column reconstruction for diagnostics. `ByteReader` provides exactly
/// that and nothing more — no allocation, no I/O, no dependencies.
pub struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Current offset in bytes from the start of the stream.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Restores a previously saved position (for tokenizer backtracking).
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.bytes.len());
    }

    /// Number of unconsumed bytes.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// Returns the next byte without consuming it. EOF yields `None`.
    pub fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Returns the byte at `offset` ahead of the cursor without consuming it.
    /// `peek_at(0)` equals [`peek`](Self::peek).
    pub fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    /// Consumes and returns the next byte. EOF yields `None`.
    pub fn next_byte(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Consumes the next byte if it satisfies `pred`; returns whether it did.
    pub fn eat_if(&mut self, pred: impl FnOnce(u8) -> bool) -> bool {
        match self.peek() {
            Some(b) if pred(b) => {
                self.pos += 1;
                true
            },
            _ => false,
        }
    }

    /// Consumes the exact byte sequence `s` if it is next; returns whether it was.
    pub fn eat_str(&mut self, s: &[u8]) -> bool {
        if self.bytes.get(self.pos..self.pos + s.len()) == Some(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    /// Case-insensitive variant of [`eat_str`](Self::eat_str), ASCII-only,
    /// as used by HTML tag names ("<!DOCTYPE", "<!--") per WHATWG §13.2.5.
    pub fn eat_str_ascii_ci(&mut self, s: &[u8]) -> bool {
        let end = (self.pos + s.len()).min(self.bytes.len());
        let window = &self.bytes[self.pos.min(self.bytes.len())..end];
        if window.len() == s.len() && window.eq_ignore_ascii_case(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    /// Skips past one UTF-8 scalar starting at the current position.
    /// Invalid sequences advance exactly one byte (replacement-char policy,
    /// matching the WHATWG input preprocessing behavior tokenizers expect).
    pub fn skip_utf8_scalar(&mut self) {
        match self.peek() {
            None => {},
            Some(b) if b < 0x80 => self.pos += 1,
            Some(b) => {
                let len = utf8_len(b);
                self.pos += len.min(self.remaining());
            },
        }
    }

    /// Converts a byte offset into a 1-based (line, column) pair for
    /// diagnostics. Columns count Unicode scalars by walking from the start —
    /// fine for error paths, not for hot loops.
    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.bytes.len());
        let mut line = 1usize;
        let mut col = 1usize;
        for &b in &self.bytes[..pos] {
            if b == b'\n' {
                line += 1;
                col = 1;
            } else if !(0x80..0xC0).contains(&b) {
                // Continuation bytes (10xxxxxx) belong to their scalar.
                col += 1;
            }
        }
        (line, col)
    }
}

/// Length in bytes of the UTF-8 sequence whose lead byte is `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        // Stray continuation or invalid lead byte: one broken byte.
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_and_consume() {
        let mut r = ByteReader::new(b"ab");
        assert_eq!(r.peek(), Some(b'a'));
        assert_eq!(r.peek_at(1), Some(b'b'));
        assert_eq!(r.next_byte(), Some(b'a'));
        assert_eq!(r.position(), 1);
        assert_eq!(r.next_byte(), Some(b'b'));
        assert_eq!(r.next_byte(), None);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn backtrack_with_seek() {
        let mut r = ByteReader::new(b"hello");
        r.seek(3);
        assert_eq!(r.peek(), Some(b'l'));
        r.seek(999); // clamps, does not panic
        assert_eq!(r.peek(), None);
    }

    #[test]
    fn eat_if_and_eat_str() {
        let mut r = ByteReader::new(b"<div>");
        assert!(r.eat_if(|b| b == b'<'));
        assert!(!r.eat_str(b"DIV"));
        assert!(!r.eat_str_ascii_ci(b"span"));
        assert!(r.eat_str_ascii_ci(b"div"));
        assert!(r.eat_str(b">"));
    }

    #[test]
    fn eat_str_ascii_ci_matches_whatwg_doctype_case() {
        for probe in ["<!doctype", "<!DOCTYPE", "<!DocTypE"] {
            let mut r = ByteReader::new(probe.as_bytes());
            assert!(r.eat_str_ascii_ci(b"<!doctype"), "{probe}");
        }
    }

    #[test]
    fn eat_str_ascii_ci_near_eof_is_safe() {
        let mut r = ByteReader::new(b"<di");
        assert!(!r.eat_str_ascii_ci(b"<div")); // short window must not panic or match
        assert_eq!(r.position(), 0);
    }

    #[test]
    fn utf8_scalar_skipping() {
        let mut r = ByteReader::new("aé中".as_bytes());
        r.skip_utf8_scalar(); // 'a'  (1 byte)
        r.skip_utf8_scalar(); // 'é'  (2 bytes)
        assert_eq!(r.position(), 3);
        r.skip_utf8_scalar(); // '中' (3 bytes)
        assert_eq!(r.position(), 6);
        r.skip_utf8_scalar(); // EOF: no-op
        assert_eq!(r.position(), 6);
    }

    #[test]
    fn truncated_utf8_does_not_overrun() {
        // Truncated 3-byte sequence at end of buffer.
        let mut r = ByteReader::new(&[0xE4, 0xB8]);
        r.skip_utf8_scalar();
        assert_eq!(r.position(), 2);
    }

    #[test]
    fn line_col_counts_lines_and_scalars() {
        let src = "ab\ncdé\nf".as_bytes();
        let r = ByteReader::new(src);
        assert_eq!(r.line_col(0), (1, 1));
        assert_eq!(r.line_col(2), (1, 3));
        assert_eq!(r.line_col(3), (2, 1)); // offset of '\n' starts the new line
        assert_eq!(r.line_col(src.len()), (3, 2));
    }
}
