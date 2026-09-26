//! A minimal s-expression reader for the WebAssembly text format.
//!
//! It handles the WAT lexical rules that matter for the testsuite: nested
//! block comments `(; ... ;)`, line comments `;;`, and string literals with
//! `\t \n \r \" \' \\`, `\hh` byte escapes and `\u{...}` code points. The
//! result is a generic tree; `wat` turns it into a [`crate::module::Module`]

use crate::error::{Result, WasmError};

/// One s-expression.
#[derive(Clone, PartialEq, Debug)]
pub enum Sexpr {
    /// A bare token: keyword, number, identifier, or operator.
    Atom(String),
    /// A string literal, decoded to its raw bytes.
    Str(Vec<u8>),
    /// A parenthesised list.
    List(Vec<Sexpr>),
}

impl Sexpr {
    /// The atom text, if this is an atom.
    pub fn atom(&self) -> Option<&str> {
        match self {
            Sexpr::Atom(s) => Some(s),
            _ => None,
        }
    }

    /// The list elements, if this is a list.
    pub fn list(&self) -> Option<&[Sexpr]> {
        match self {
            Sexpr::List(v) => Some(v),
            _ => None,
        }
    }

    /// The head atom of a list, if it is a list with an atom head.
    pub fn head(&self) -> Option<&str> {
        match self {
            Sexpr::List(v) => v.first().and_then(Sexpr::atom),
            _ => None,
        }
    }

    /// The decoded bytes of a string literal.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Sexpr::Str(b) => Some(b),
            _ => None,
        }
    }

    /// The decoded string, if the bytes are valid UTF-8.
    pub fn string(&self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }
}

/// Parse a whole source text into a sequence of top-level s-expressions.
pub fn parse(source: &str) -> Result<Vec<Sexpr>> {
    let bytes = source.as_bytes();
    let mut p = Parser { bytes, pos: 0, prev_word_or_str_end: None };
    let mut out = Vec::new();
    loop {
        p.skip_trivia()?;
        if p.pos >= bytes.len() {
            break;
        }
        out.push(p.parse_expr()?);
    }
    Ok(out)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// End offset of the last atom or string token emitted, used to detect
    /// a string literal glued directly onto a preceding word or string with
    /// no separating white space or comment (`data"a"`, `"a""b"`), which the
    /// text-format lexer rejects even though the boundary is unambiguous.
    prev_word_or_str_end: Option<usize>,
}

impl Parser<'_> {
    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            while self.pos < self.bytes.len() {
                match self.bytes[self.pos] {
                    b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                    _ => break,
                }
            }
            if self.pos + 1 < self.bytes.len() && self.bytes[self.pos] == b';' && self.bytes[self.pos + 1] == b';' {
                self.pos += 2;
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.pos + 1 < self.bytes.len() && self.bytes[self.pos] == b'(' && self.bytes[self.pos + 1] == b';' {
                self.pos += 2;
                let mut depth = 1usize;
                while self.pos < self.bytes.len() && depth > 0 {
                    if self.pos + 1 < self.bytes.len() && self.bytes[self.pos] == b'(' && self.bytes[self.pos + 1] == b';' {
                        depth += 1;
                        self.pos += 2;
                    } else if self.pos + 1 < self.bytes.len()
                        && self.bytes[self.pos] == b';'
                        && self.bytes[self.pos + 1] == b')'
                    {
                        depth -= 1;
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                }
                if depth > 0 {
                    return Err(WasmError::malformed("unclosed comment"));
                }
                continue;
            }
            break;
        }
        Ok(())
    }

    fn parse_expr(&mut self) -> Result<Sexpr> {
        self.skip_trivia()?;
        if self.pos >= self.bytes.len() {
            return Err(WasmError::malformed("unexpected end of input"));
        }
        // A string glued directly onto the previous word/string token with no
        // intervening white space or comment is malformed, even though the
        // `"` makes the boundary unambiguous (`data"a"`, `$l"a"`, `"a""b"`).
        if self.bytes[self.pos] == b'"' && self.prev_word_or_str_end == Some(self.pos) {
            return Err(WasmError::malformed("a string literal must be separated by white space"));
        }
        match self.bytes[self.pos] {
            b'(' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_trivia()?;
                    if self.pos >= self.bytes.len() {
                        return Err(WasmError::malformed("unexpected end of input"));
                    }
                    if self.bytes[self.pos] == b')' {
                        self.pos += 1;
                        break;
                    }
                    items.push(self.parse_expr()?);
                }
                self.prev_word_or_str_end = None;
                Ok(Sexpr::List(items))
            }
            b')' => Err(WasmError::malformed("unexpected )")),
            // A lone `;` (not `;;` or `(;`, both consumed by `skip_trivia`)
            // never starts a valid token; reject explicitly rather than
            // let `parse_atom` return an empty atom and spin in place.
            b';' => Err(WasmError::malformed("unexpected ;")),
            b'"' => {
                self.pos += 1;
                let s = self.parse_string()?;
                self.prev_word_or_str_end = Some(self.pos);
                Ok(Sexpr::Str(s))
            }
            _ => {
                let a = self.parse_atom();
                self.prev_word_or_str_end = Some(self.pos);
                Ok(Sexpr::Atom(a))
            }
        }
    }

    fn parse_atom(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                // `;` is never part of a keyword/id/number token: it only
                // ever appears as `;;` (line comment) or inside `(; ;)`
                // (block comment), neither of which requires white space
                // before it (`func;;bla` must lex as `func` then a comment).
                b' ' | b'\t' | b'\n' | b'\r' | b'(' | b')' | b'"' | b';' => break,
                _ => self.pos += 1,
            }
        }
        String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned()
    }

    fn parse_string(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    if self.pos >= self.bytes.len() {
                        return Err(WasmError::malformed("unexpected end of string"));
                    }
                    let e = self.bytes[self.pos];
                    self.pos += 1;
                    match e {
                        b't' => out.push(b'\t'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b'"' => out.push(b'"'),
                        b'\'' => out.push(b'\''),
                        b'\\' => out.push(b'\\'),
                        b'u' => {
                            if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'{' {
                                return Err(WasmError::malformed("malformed unicode escape"));
                            }
                            self.pos += 1;
                            let start = self.pos;
                            while self.pos < self.bytes.len() && self.bytes[self.pos] != b'}' {
                                self.pos += 1;
                            }
                            if self.pos >= self.bytes.len() {
                                return Err(WasmError::malformed("malformed unicode escape"));
                            }
                            let hex = std::str::from_utf8(&self.bytes[start..self.pos])
                                .map_err(|_| WasmError::malformed("malformed unicode escape"))?;
                            self.pos += 1;
                            let cp = u32::from_str_radix(hex, 16)
                                .map_err(|_| WasmError::malformed("malformed unicode escape"))?;
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| WasmError::malformed("malformed unicode escape"))?;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => {
                            // `\hh` two hex digits.
                            let hi = hex_digit(e).ok_or_else(|| {
                                WasmError::malformed("malformed string escape")
                            })?;
                            if self.pos >= self.bytes.len() {
                                return Err(WasmError::malformed("malformed string escape"));
                            }
                            let lo = hex_digit(self.bytes[self.pos]).ok_or_else(|| {
                                WasmError::malformed("malformed string escape")
                            })?;
                            self.pos += 1;
                            out.push(hi * 16 + lo);
                        }
                    }
                }
                _ => out.push(b),
            }
        }
        Err(WasmError::malformed("unexpected end of string"))
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
