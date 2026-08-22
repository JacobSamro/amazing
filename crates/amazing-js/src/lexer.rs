//! The ECMA-262 lexical scanner (spec §11–§12): source text → [`Token`]s.
//!
//! Implemented:
//! - Panic-free entry point [`tokenize`] returning every token (terminated
//!   by [`TokenKind::Eof`]) or a [`SyntaxError`]; CRLF and lone CR inputs
//!   are normalized to LF up front (mirroring `amazing_html`).
//! - Whitespace including NBSP (`U+00A0`) and ZWNBSP/BOM (`U+FEFF`);
//!   `//` and `/* */` comments (block comments set `newline_before` when
//!   they contain a line terminator, as ASI requires).
//! - Line terminators `\n`, `U+2028`, `U+2029` feed both position tracking
//!   and the per-token `newline_before` flag that drives ASI.
//! - Numeric literals: decimal (with fraction/exponent), `0x`/`0o`/`0b`,
//!   `_` separators validated to sit *between* digits, and BigInt `n`.
//!   Stored as [`Numeric`] text; value conversion is the interpreter's job.
//! - String literals with escapes: `\n \t \r \b \v \f \0`, `\xHH`,
//!   `\uHHHH`, `\u{H+}`, `\' \" \\ \` `, and line continuations. Unknown
//!   escapes yield the escaped character itself (sloppy-mode web compat;
//!   strict-mode octal-escape errors deferred).
//! - Template literals via an explicit mode stack: chunks come out as
//!   [`TokenKind::TemplatePart`], substitutions open with
//!   [`Punct::DollarBrace`] and their closing `}` is absorbed by the lexer
//!   (tracked with a brace-depth counter so nested `{}` inside
//!   substitutions works) together with the next chunk.
//! - All punctuators, matched longest-first (`>>>= >>>= ??= &&= ||= **= ?.
//!   ... =>` etc.). `/` is **always division**; see the division-vs-regex
//!   note at the bottom of this list.
//!
//! Deferred (documented, not implemented):
//! - Regex literals entirely. The classic `/` ambiguity (division vs
//!   regex) is resolved in favor of division because regexes are out of
//!   scope; when they land, the parser must pass a "regex allowed here?"
//!   feedback flag into the scanner at every operand boundary — that is
//!   where the ASI/division ambiguity will live.
//! - Full Unicode identifier classes: identifiers use the simplified rule
//!   `char::is_alphabetic | $ | _` (start) and `+ is_numeric | digits`
//!   (continuation). `\u{…}` escapes inside identifier names, ID_Start /
//!   ID_Continue property data, and ZWNJ/ZWJ joining are deferred.
//! - Unicode Space_Separator category beyond NBSP/BOM (e.g. U+2000..U+200A).
//! - HTML-like comments (`<!--` … `-->`, Annex B).
//! - Legacy (Annex B) octal literals: `0123` lexes as *decimal* 123, and
//!   strict-mode early errors around them are deferred with all strict
//!   checks. Non-octal decimals like `089` behave correctly.
//! - Tagged templates need raw (uncooked) chunk text, which is not kept.
//! - Private names (`#x`) and decorators (`@`) are rejected.

use crate::errors::SyntaxError;
use crate::tokens::{
    keyword_of, syntax_error, Numeric, Punct, SourceLocation, Span, Token, TokenKind,
};

/// Tokenizes a whole script. Never panics; all failures are [`SyntaxError`]
/// positioned at the first offending character.
pub fn tokenize(src: &str) -> Result<Vec<Token>, SyntaxError> {
    // Newline normalization (the decoding step browsers do): CRLF and lone
    // CR become LF before scanning.
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        src
    };
    Lexer::new(src).run()
}

struct Lexer {
    chars: Vec<char>,
    idx: usize,
    line: u32,
    col: u32,
    /// Open `{` seen in plain code; distinguishes an object/block `}` from
    /// the `}` that closes a template substitution.
    brace_depth: usize,
    /// Number of `${ … }` substitutions currently open.
    open_templates: usize,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Lexer {
            chars: src.chars().collect(),
            idx: 0,
            line: 1,
            col: 1,
            brace_depth: 0,
            open_templates: 0,
        }
    }

    fn run(mut self) -> Result<Vec<Token>, SyntaxError> {
        let mut toks = Vec::new();
        loop {
            let newline_before = self.skip_trivia()?;
            let start = self.loc();
            if self.at_end() {
                if self.open_templates > 0 {
                    return Err(syntax_error("unterminated template literal", start));
                }
                toks.push(Token::new(TokenKind::Eof, Span::new(start, start), newline_before));
                return Ok(toks);
            }
            let c = self.cur().expect("not at end");
            match c {
                '\'' | '"' => toks.push(self.string_token(start, newline_before)?),
                '`' => self.template_open(start, newline_before, &mut toks)?,
                '}' => self.rbrace_token(start, newline_before, &mut toks)?,
                c if c.is_ascii_digit() => toks.push(self.number_token(start, newline_before)?),
                '.' if self.peek(1).is_some_and(|c| c.is_ascii_digit()) => {
                    toks.push(self.number_token(start, newline_before)?)
                }
                c if is_ident_start(c) => toks.push(self.ident_token(start, newline_before)),
                _ => toks.push(self.punct_token(start, newline_before)?),
            }
        }
    }

    // ----- cursor helpers -------------------------------------------------

    fn loc(&self) -> SourceLocation {
        SourceLocation::new(self.line, self.col)
    }

    fn at_end(&self) -> bool {
        self.idx >= self.chars.len()
    }

    fn cur(&self) -> Option<char> {
        self.peek(0)
    }

    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.idx + n).copied()
    }

    fn advance(&mut self) {
        if self.at_end() {
            return;
        }
        let c = self.chars[self.idx];
        self.idx += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
    }

    fn mk(&self, kind: TokenKind, start: SourceLocation, newline_before: bool) -> Token {
        Token::new(kind, Span::new(start, self.loc()), newline_before)
    }

    fn punct_tok(&mut self, p: Punct) -> Punct {
        self.advance();
        p
    }

    /// Consumes exactly `pat` if the following characters match it.
    fn eats(&mut self, pat: &str) -> bool {
        for (k, want) in pat.chars().enumerate() {
            if self.peek(k) != Some(want) {
                return false;
            }
        }
        for _ in pat.chars() {
            self.advance();
        }
        true
    }

    // ----- trivia ---------------------------------------------------------

    fn skip_trivia(&mut self) -> Result<bool, SyntaxError> {
        let mut newline = false;
        loop {
            match self.cur() {
                Some(' ') | Some('\t') | Some('\u{000b}') | Some('\u{000c}')
                | Some('\u{00a0}') | Some('\u{feff}') => self.advance(),
                Some('\n') | Some('\u{2028}') | Some('\u{2029}') => {
                    newline = true;
                    self.advance();
                }
                Some('/') if self.peek(1) == Some('/') => {
                    while !self.at_end() && self.cur() != Some('\n') {
                        self.advance();
                    }
                }
                Some('/') if self.peek(1) == Some('*') => {
                    self.advance();
                    self.advance();
                    let mut closed = false;
                    while !self.at_end() {
                        match self.cur() {
                            Some('\n') | Some('\u{2028}') | Some('\u{2029}') => {
                                newline = true;
                                self.advance();
                            }
                            Some('*') if self.peek(1) == Some('/') => {
                                self.advance();
                                self.advance();
                                closed = true;
                                break;
                            }
                            _ => self.advance(),
                        }
                    }
                    if !closed {
                        return Err(syntax_error("unterminated block comment", self.loc()));
                    }
                }
                _ => return Ok(newline),
            }
        }
    }

    // ----- strings & templates ---------------------------------------------

    fn string_token(&mut self, start: SourceLocation, newline_before: bool) -> Result<Token, SyntaxError> {
        let quote = self.cur().expect("caller checked");
        self.advance();
        let mut value = String::new();
        loop {
            match self.cur() {
                None | Some('\n') => {
                    return Err(syntax_error("unterminated string literal", start));
                }
                Some(c) if c == quote => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    if let Some(c) = self.read_escape()? {
                        value.push(c);
                    }
                }
                Some(c) => {
                    value.push(c);
                    self.advance();
                }
            }
        }
        Ok(self.mk(TokenKind::Str(value), start, newline_before))
    }

    fn template_open(
        &mut self,
        start: SourceLocation,
        newline_before: bool,
        toks: &mut Vec<Token>,
    ) -> Result<(), SyntaxError> {
        self.advance(); // opening backtick
        let (cooked, tail) = self.scan_template_part()?;
        toks.push(self.mk(TokenKind::TemplatePart { cooked, tail }, start, newline_before));
        if !tail {
            toks.push(self.mk(TokenKind::Punct(Punct::DollarBrace), start, false));
            self.open_templates += 1;
        }
        Ok(())
    }

    fn rbrace_token(
        &mut self,
        start: SourceLocation,
        newline_before: bool,
        toks: &mut Vec<Token>,
    ) -> Result<(), SyntaxError> {
        self.advance();
        if self.brace_depth > 0 {
            self.brace_depth -= 1;
            toks.push(self.mk(TokenKind::Punct(Punct::Rbrace), start, newline_before));
            return Ok(());
        }
        if self.open_templates > 0 {
            // Closes a `${` substitution; the following template chunk is
            // scanned right away so the parser never sees this `}`.
            let (cooked, tail) = self.scan_template_part()?;
            toks.push(self.mk(TokenKind::TemplatePart { cooked, tail }, start, newline_before));
            if tail {
                self.open_templates -= 1;
            } else {
                toks.push(self.mk(TokenKind::Punct(Punct::DollarBrace), start, false));
            }
            return Ok(());
        }
        // Unmatched `}` in plain code: emit it and let the parser produce a
        // positioned "unexpected token" diagnostic.
        toks.push(self.mk(TokenKind::Punct(Punct::Rbrace), start, newline_before));
        Ok(())
    }

    /// Scans cooked template text up to the closing backtick (returns
    /// `tail = true`) or up to `${` (consumes it, returns `tail = false`).
    fn scan_template_part(&mut self) -> Result<(String, bool), SyntaxError> {
        let mut cooked = String::new();
        loop {
            match self.cur() {
                None => return Err(syntax_error("unterminated template literal", self.loc())),
                Some('`') => {
                    self.advance();
                    return Ok((cooked, true));
                }
                Some('$') if self.peek(1) == Some('{') => {
                    self.advance();
                    self.advance();
                    return Ok((cooked, false));
                }
                Some('\\') => {
                    self.advance();
                    if let Some(c) = self.read_escape()? {
                        cooked.push(c);
                    }
                }
                Some(c) => {
                    cooked.push(c);
                    self.advance();
                }
            }
        }
    }

    /// Reads the character(s) after a `\`. `Ok(None)` = escape sequence
    /// producing nothing (line continuation). Caller has consumed the `\`.
    fn read_escape(&mut self) -> Result<Option<char>, SyntaxError> {
        let c = match self.cur() {
            Some(c) => c,
            None => return Err(syntax_error("unterminated escape sequence", self.loc())),
        };
        self.advance();
        let ch = match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{0008}',
            'v' => '\u{000b}',
            'f' => '\u{000c}',
            '0' if !self.cur().is_some_and(|c| c.is_ascii_digit()) => '\0',
            'x' => {
                let v = self.take_hex(2)?;
                char_at(v, self.loc())?
            }
            'u' => {
                if self.cur() == Some('{') {
                    self.advance();
                    let v = self.take_hex_braced()?;
                    char_at(v, self.loc())?
                } else {
                    let v = self.take_hex(4)?;
                    char_at(v, self.loc())?
                }
            }
            // Line continuation: backslash-newline yields nothing. Lone CR
            // cannot occur (normalized up front).
            '\n' => return Ok(None),
            // Everything else (including unknown escapes and sloppy-mode
            // octal lookalikes like `\1`) is the character itself.
            other => other,
        };
        Ok(Some(ch))
    }

    fn take_hex(&mut self, n: usize) -> Result<u32, SyntaxError> {
        let mut v: u32 = 0;
        for _ in 0..n {
            let d = match self.cur().and_then(hex_digit) {
                Some(d) => d,
                None => return Err(syntax_error("invalid hexadecimal escape", self.loc())),
            };
            self.advance();
            v = v * 16 + d;
        }
        Ok(v)
    }

    fn take_hex_braced(&mut self) -> Result<u32, SyntaxError> {
        let mut v: u32 = 0;
        let mut count = 0;
        while let Some(d) = self.cur().and_then(hex_digit) {
            self.advance();
            v = v.saturating_mul(16).saturating_add(d);
            count += 1;
            if count > 6 {
                return Err(syntax_error("too many digits in \\u{...} escape", self.loc()));
            }
        }
        if count == 0 || self.cur() != Some('}') {
            return Err(syntax_error("invalid \\u{...} escape", self.loc()));
        }
        self.advance();
        Ok(v)
    }

    // ----- numbers ----------------------------------------------------------

    fn number_token(&mut self, start: SourceLocation, newline_before: bool) -> Result<Token, SyntaxError> {
        let numeric = self.scan_number()?;
        if self.cur().is_some_and(is_ident_part) {
            return Err(syntax_error(
                "identifier character immediately after numeric literal",
                self.loc(),
            ));
        }
        Ok(self.mk(TokenKind::Numeric(numeric), start, newline_before))
    }

    fn scan_number(&mut self) -> Result<Numeric, SyntaxError> {
        // Radix prefixes: 0x 0o 0b (and uppercase), BigInt suffix allowed.
        if self.cur() == Some('0')
            && matches!(
                self.peek(1),
                Some('x') | Some('X') | Some('o') | Some('O') | Some('b') | Some('B')
            )
        {
            self.advance(); // '0'
            let radix = match self.cur() {
                Some('x') | Some('X') => 16u8,
                Some('o') | Some('O') => 8,
                _ => 2,
            };
            self.advance();
            let digits = self.digit_run(radix)?;
            if digits.is_empty() {
                return Err(syntax_error("missing digits after radix prefix", self.loc()));
            }
            let bigint = self.take_bigint_suffix();
            return Ok(Numeric::new(digits, radix, bigint));
        }

        // Decimal: integer part, optional fraction, optional exponent.
        let mut raw = self.digit_run(10)?;
        if self.cur() == Some('.')
            && !self.peek(1).is_some_and(|c| c == '_' || is_ident_start(c))
        {
            raw.push('.');
            self.advance();
            if self.cur().is_some_and(|c| c.is_ascii_digit()) {
                let frac = self.digit_run(10)?;
                raw.push_str(&frac);
            }
        }
        // Exponent only counts when well-formed: `1e3` yes, `1ex` no (the
        // trailing-character check then rejects the stray identifier).
        if matches!(self.cur(), Some('e') | Some('E')) {
            let sign = self.peek(1);
            let digit_after = match sign {
                Some('+') | Some('-') => self.peek(2).is_some_and(|c| c.is_ascii_digit()),
                Some(c) => c.is_ascii_digit(),
                None => false,
            };
            if digit_after {
                raw.push('e');
                self.advance();
                if matches!(self.cur(), Some('+') | Some('-')) {
                    raw.push(self.cur().unwrap());
                    self.advance();
                }
                raw.push_str(&self.digit_run(10)?);
            }
        }
        let bigint = self.take_bigint_suffix();
        Ok(Numeric::new(raw, 10, bigint))
    }

    fn take_bigint_suffix(&mut self) -> bool {
        if self.cur() == Some('n') {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Reads a maximal digit run in `radix`, allowing `_` separators only
    /// strictly between two digits. Separators are stripped from output.
    fn digit_run(&mut self, radix: u8) -> Result<String, SyntaxError> {
        let mut out = String::new();
        loop {
            match self.cur() {
                Some('_') => {
                    let ok = !out.is_empty() && self.peek(1).is_some_and(|c| is_radix_digit(c, radix));
                    if !ok {
                        return Err(syntax_error(
                            "numeric separator must be between digits",
                            self.loc(),
                        ));
                    }
                    self.advance();
                }
                Some(c) if is_radix_digit(c, radix) => {
                    out.push(c);
                    self.advance();
                }
                _ => return Ok(out),
            }
        }
    }

    // ----- identifiers ------------------------------------------------------

    fn ident_token(&mut self, start: SourceLocation, newline_before: bool) -> Token {
        let mut name = String::new();
        while self.cur().is_some_and(is_ident_part) {
            name.push(self.cur().expect("checked"));
            self.advance();
        }
        let kind = match keyword_of(&name) {
            Some(k) => TokenKind::Keyword(k),
            None => TokenKind::Ident(name),
        };
        self.mk(kind, start, newline_before)
    }

    // ----- punctuators --------------------------------------------------------

    fn punct_token(&mut self, start: SourceLocation, newline_before: bool) -> Result<Token, SyntaxError> {
        let p = self.match_punct()?;
        Ok(self.mk(TokenKind::Punct(p), start, newline_before))
    }

    fn match_punct(&mut self) -> Result<Punct, SyntaxError> {
        let c = match self.cur() {
            Some(c) => c,
            None => return Err(syntax_error("unexpected end of input", self.loc())),
        };
        let p = match c {
            '(' => self.punct_tok(Punct::LParen),
            ')' => self.punct_tok(Punct::RParen),
            '[' => self.punct_tok(Punct::LBracket),
            ']' => self.punct_tok(Punct::RBracket),
            ';' => self.punct_tok(Punct::Semi),
            ',' => self.punct_tok(Punct::Comma),
            '~' => self.punct_tok(Punct::Tilde),
            ':' => self.punct_tok(Punct::Colon),
            '{' => {
                self.advance();
                self.brace_depth += 1;
                Punct::Lbrace
            }
            '}' => self.punct_tok(Punct::Rbrace), // unreachable via run(); kept total
            '+' => {
                if self.eats("++") {
                    Punct::Inc
                } else if self.eats("+=") {
                    Punct::PlusEq
                } else {
                    self.advance();
                    Punct::Plus
                }
            }
            '-' => {
                if self.eats("--") {
                    Punct::Dec
                } else if self.eats("-=") {
                    Punct::MinusEq
                } else {
                    self.advance();
                    Punct::Minus
                }
            }
            '*' => {
                if self.eats("**=") {
                    Punct::PowEq
                } else if self.eats("**") {
                    Punct::Pow
                } else if self.eats("*=") {
                    Punct::StarEq
                } else {
                    self.advance();
                    Punct::Star
                }
            }
            '/' => {
                if self.eats("/=") {
                    Punct::SlashEq
                } else {
                    self.advance();
                    Punct::Slash // always division; regex deferred
                }
            }
            '%' => {
                if self.eats("%=") {
                    Punct::PercentEq
                } else {
                    self.advance();
                    Punct::Percent
                }
            }
            '&' => {
                if self.eats("&&=") {
                    Punct::AndAndEq
                } else if self.eats("&&") {
                    Punct::And
                } else if self.eats("&=") {
                    Punct::AndEq
                } else {
                    self.advance();
                    Punct::BitAnd
                }
            }
            '|' => {
                if self.eats("||=") {
                    Punct::OrOrEq
                } else if self.eats("||") {
                    Punct::Or
                } else if self.eats("|=") {
                    Punct::OrEq
                } else {
                    self.advance();
                    Punct::BitOr
                }
            }
            '^' => {
                if self.eats("^=") {
                    Punct::XorEq
                } else {
                    self.advance();
                    Punct::BitXor
                }
            }
            '<' => {
                if self.eats("<<=") {
                    Punct::ShlEq
                } else if self.eats("<<") {
                    Punct::Shl
                } else if self.eats("<=") {
                    Punct::Le
                } else {
                    self.advance();
                    Punct::Lt
                }
            }
            '>' => {
                if self.eats(">>>=") {
                    Punct::UShrEq
                } else if self.eats(">>>") {
                    Punct::UShr
                } else if self.eats(">>=") {
                    Punct::ShrEq
                } else if self.eats(">>") {
                    Punct::Shr
                } else if self.eats(">=") {
                    Punct::Ge
                } else {
                    self.advance();
                    Punct::Gt
                }
            }
            '=' => {
                if self.eats("===") {
                    Punct::EqEqEq
                } else if self.eats("==") {
                    Punct::EqEq
                } else if self.eats("=>") {
                    Punct::Arrow
                } else {
                    self.advance();
                    Punct::Eq
                }
            }
            '!' => {
                if self.eats("!==") {
                    Punct::NotEqEq
                } else if self.eats("!=") {
                    Punct::NotEq
                } else {
                    self.advance();
                    Punct::Bang
                }
            }
            '?' => {
                if self.eats("??=") {
                    Punct::NullishEq
                } else if self.eats("??") {
                    Punct::Nullish
                } else if self.peek(1) == Some('.')
                    && !self.peek(2).is_some_and(|c| c.is_ascii_digit())
                {
                    // `?.` but NOT `?.5` (which is ternary over `.5`).
                    self.advance(); // '?'
                    self.advance(); // '.'
                    Punct::QuestionDot
                } else {
                    self.advance();
                    Punct::Question
                }
            }
            '.' => {
                if self.eats("...") {
                    Punct::Ellipsis
                } else {
                    self.advance();
                    Punct::Dot
                }
            }
            other => {
                return Err(syntax_error(
                    format!("unexpected character `{other}`"),
                    self.loc(),
                ));
            }
        };
        Ok(p)
    }
}

// ----- character classification ------------------------------------------

/// Simplified identifier rules (see module docs for the deferral).
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '$' || c == '_'
}

fn is_ident_part(c: char) -> bool {
    is_ident_start(c) || c.is_numeric() || c.is_ascii_digit()
}

fn is_radix_digit(c: char, radix: u8) -> bool {
    match radix {
        16 => c.is_ascii_hexdigit(),
        10 => c.is_ascii_digit(),
        8 => ('0'..='7').contains(&c),
        2 => c == '0' || c == '1',
        _ => false,
    }
}

fn hex_digit(c: char) -> Option<u32> {
    c.to_digit(16)
}

/// Converts a scanned code point, rejecting out-of-range values.
fn char_at(v: u32, at: SourceLocation) -> Result<char, SyntaxError> {
    char::from_u32(v).ok_or_else(|| syntax_error("invalid Unicode code point in escape", at))
}

#[cfg(test)]
mod tests {
    use super::tokenize;
    use crate::errors::SyntaxError;
    use crate::tokens::{Keyword, Punct, TokenKind};

    fn kinds(src: &str) -> Vec<TokenKind> {
        let mut toks = tokenize(src)
            .expect("tokenization should succeed")
            .into_iter()
            .map(|t| t.kind)
            .collect::<Vec<_>>();
        // Drop the synthetic EOF token; tests compare real tokens.
        toks.pop();
        toks
    }

    fn p(variants: &[Punct]) -> Vec<TokenKind> {
        variants.iter().copied().map(TokenKind::Punct).collect()
    }

    fn err_of(src: &str) -> SyntaxError {
        tokenize(src).expect_err("tokenization should fail")
    }

    #[test]
    fn punctuators_match_longest_first() {
        let got = kinds(">>>= >>> >>= >> >= > <<= << <= === !== == != => &&= && ||= || ??= ?? ?.");
        assert_eq!(
            got,
            p(&[
                Punct::UShrEq, Punct::UShr, Punct::ShrEq, Punct::Shr,
                Punct::Ge, Punct::Gt, Punct::ShlEq, Punct::Shl, Punct::Le,
                Punct::EqEqEq, Punct::NotEqEq, Punct::EqEq, Punct::NotEq,
                Punct::Arrow, Punct::AndAndEq, Punct::And, Punct::OrOrEq,
                Punct::Or, Punct::NullishEq, Punct::Nullish, Punct::QuestionDot,
            ])
        );
    }

    #[test]
    fn ternary_question_survives_before_dot_number() {
        // `?.5` is `? .5`, not optional chaining.
        assert_eq!(kinds("a?.5:b"), {
            let mut v = vec![TokenKind::Ident("a".into())];
            v.extend(p(&[Punct::Question]));
            v.push(TokenKind::Numeric(crate::tokens::Numeric::new(".5", 10, false)));
            v.extend(p(&[Punct::Colon]));
            v.push(TokenKind::Ident("b".into()));
            v
        });
    }

    #[test]
    fn slash_is_always_division() {
        assert_eq!(kinds("a / b /= c"), {
            let mut v = vec![TokenKind::Ident("a".into())];
            v.extend(p(&[Punct::Slash]));
            v.push(TokenKind::Ident("b".into()));
            v.extend(p(&[Punct::SlashEq]));
            v.push(TokenKind::Ident("c".into()));
            v
        });
    }

    #[test]
    fn numeric_literals_across_radixes_separators_and_bigints() {
        use crate::tokens::Numeric;
        let n = |raw: &str, radix: u8, big: bool| TokenKind::Numeric(Numeric::new(raw, radix, big));
        assert_eq!(
            kinds("42 0xFF 0o17 0b101 1_000.5 .5e3 6.02e23 10n 0xDEAD_beefn 089"),
            vec![
                n("42", 10, false),
                n("FF", 16, false),
                n("17", 8, false),
                n("101", 2, false),
                n("1000.5", 10, false),
                n(".5e3", 10, false),
                n("6.02e23", 10, false),
                n("10", 10, true),
                n("DEADbeef", 16, true),
                n("089", 10, false),
            ]
        );
    }

    #[test]
    fn string_escapes_are_cooked() {
        assert_eq!(
            kinds("'a\\n\\t\\\\\\'\\x41\\u{1F600}\\u0042\\q'"),
            vec![TokenKind::Str("a\n\t\\'A\u{1F600}Bq".into())]
        );
        // Line continuation produces nothing.
        assert_eq!(kinds("\"one\\\n  two\""), vec![TokenKind::Str("one  two".into())]);
        // U+2028/U+2029 are legal inside strings.
        assert_eq!(
            kinds("'a b'"),
            vec![TokenKind::Str("a b".into())]
        );
    }

    #[test]
    fn nbsp_and_zwnbsp_are_whitespace() {
        assert_eq!(kinds("a\u{00a0}\u{feff}b"), vec![
            TokenKind::Ident("a".into()),
            TokenKind::Ident("b".into()),
        ]);
    }

    #[test]
    fn comments_feed_newline_flag_for_asi() {
        let toks = tokenize("a /* one\ntwo */ b // eol\nc").expect("ok");
        let flags: Vec<bool> = toks.iter().map(|t| t.newline_before).collect();
        //              a      b      c     Eof (no trailing newline)
        assert_eq!(flags, vec![false, true, true, false]);
    }

    #[test]
    fn positions_track_lines_and_columns() {
        let toks = tokenize("let x = 1;\nx").expect("ok");
        let first_x = &toks[1];
        assert_eq!(first_x.span.start.line, 1);
        assert_eq!(first_x.span.start.column, 5);
        let second_x = &toks[5];
        assert_eq!(second_x.span.start.line, 2);
        assert_eq!(second_x.span.start.column, 1);
    }

    #[test]
    fn crlf_is_normalized_before_scanning() {
        let toks = tokenize("a\r\nb").expect("ok");
        assert_eq!(toks[1].span.start.line, 2, "CRLF counts as one line break");
    }

    #[test]
    fn nested_templates_with_object_braces_tokenize_as_parts_and_transitions() {
        let got = kinds("`a${ {x: 1} }b${`inner${q}`}c`");
        let tp = |cooked: &str, tail: bool| {
            TokenKind::TemplatePart { cooked: cooked.into(), tail }
        };
        assert_eq!(
            got,
            vec![
                tp("a", false),
                TokenKind::Punct(Punct::DollarBrace),
                TokenKind::Punct(Punct::Lbrace),
                TokenKind::Ident("x".into()),
                TokenKind::Punct(Punct::Colon),
                TokenKind::Numeric(crate::tokens::Numeric::new("1", 10, false)),
                TokenKind::Punct(Punct::Rbrace),
                tp("b", false),
                TokenKind::Punct(Punct::DollarBrace),
                tp("inner", false),
                TokenKind::Punct(Punct::DollarBrace),
                TokenKind::Ident("q".into()),
                tp("", true),
                tp("c", true),
            ]
        );
    }

    #[test]
    fn template_escapes_and_multiline_chunks_cook_correctly() {
        let got = kinds("`line1\\n${x}`");
        match &got[0] {
            TokenKind::TemplatePart { cooked, tail } => {
                assert_eq!(cooked, "line1\n");
                assert!(!tail);
            }
            other => panic!("expected template part, got {other:?}"),
        }
        assert_eq!(got[1], TokenKind::Punct(Punct::DollarBrace));
        assert_eq!(got[2], TokenKind::Ident("x".into()));
        match &got[3] {
            TokenKind::TemplatePart { cooked, tail } => {
                assert_eq!(cooked, "");
                assert!(*tail);
            }
            other => panic!("expected tail part, got {other:?}"),
        }
    }

    #[test]
    fn keywords_become_keyword_tokens_contextuals_stay_idents() {
        let got = kinds("function let static async of yield");
        assert_eq!(
            got,
            vec![
                TokenKind::Keyword(Keyword::Function),
                TokenKind::Ident("let".into()),
                TokenKind::Ident("static".into()),
                TokenKind::Ident("async".into()),
                TokenKind::Ident("of".into()),
                TokenKind::Keyword(Keyword::Yield),
            ]
        );
    }

    #[test]
    fn every_input_terminates_with_eof_token() {
        for src in ["", "   ", "x", "/* c */"] {
            let toks = tokenize(src).expect("ok");
            assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
        }
        // Token-free sources produce exactly the EOF token.
        assert_eq!(tokenize("").unwrap().len(), 1);
        assert_eq!(tokenize("   ").unwrap().len(), 1);
        assert_eq!(tokenize("/* c */").unwrap().len(), 1);
        assert_eq!(tokenize("x").unwrap().len(), 2);
    }

    #[test]
    fn unterminated_string_is_a_positioned_error() {
        let e = err_of("'abc");
        assert_eq!(e.line, 1);
        assert_eq!(e.column, 1);
        assert!(e.message.contains("unterminated string"));
    }

    #[test]
    fn bad_unicode_escape_is_rejected() {
        assert!(err_of("'\\u{110000}'").message.contains("Unicode"));
        assert!(err_of("'\\u00'").message.contains("hexadecimal"));
    }

    #[test]
    fn misplaced_numeric_separator_is_rejected() {
        assert!(err_of("1__0").message.contains("separator"));
        assert!(err_of("1_").message.contains("separator"));
    }

    #[test]
    fn missing_digits_after_radix_prefix_is_rejected() {
        assert!(err_of("0x").message.contains("missing digits"));
        assert!(err_of("0b").message.contains("missing digits"));
    }

    #[test]
    fn identifier_jammed_after_number_is_rejected() {
        assert!(err_of("5ex").message.contains("numeric"));
        // `5n.toString()` is *valid* (BigInt method access); only an
        // identifier-start character jammed against the literal errors.
        assert!(err_of("5nq").message.contains("numeric"));
        assert!(tokenize("5n.toString()").is_ok());
    }

    #[test]
    fn unterminated_template_is_rejected() {
        assert!(err_of("`abc").message.contains("template"));
        assert!(err_of("`${x").message.contains("template"));
        assert!(err_of("`a${`b`").message.contains("template"));
    }

    #[test]
    fn unterminated_block_comment_is_rejected() {
        assert!(err_of("/* nope").message.contains("comment"));
    }

    #[test]
    fn unknown_characters_are_rejected_with_position() {
        let e = err_of("a @ b");
        assert_eq!(e.column, 3);
        assert!(e.message.contains('@'));
    }
}

