//! CSS Syntax Module Level 3 tokenizer (§4, "css-syntax-3") — Phase 2 subset.
//!
//! Produces a flat component-value token stream. Deliberate simplifications
//! (all documented here and in lib.rs):
//! - Comments (`/* … */`) are dropped entirely; unclosed comment runs to EOF.
//! - Unquoted `url()` is one URL token; quoted arguments remain functions.
//! - `<EOF-token>` error recovery: unterminated strings/blocks yield what was
//!   consumed instead of the spec's full replacement logic.
//! - CRLF handling is the caller's job (HTML parser already normalizes).
//!
//! Escapes (§4.3.7-ish): `\` + 1-6 hex digits + optional single whitespace →
//! code point; `\` + any other non-newline char → that char; inside strings,
//! `\` + newline = line continuation. Bad escapes pass through literally.

/// One component value of the flat token stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    /// `name(` — the `(` is also emitted as [`Token::OpenParen`] after it.
    Function(String),
    /// Unquoted `url(...)`: decoded value and original spelling for serialization.
    Url { value: String, raw: String },
    BadUrl,
    AtKeyword(String),
    /// `#name`; only produced when a name char follows `#`.
    Hash(String),
    /// Decoded quoted string.
    Str(String),
    Delim(char),
    Number(f64),
    Percentage(f64),
    /// Numeric with unit ident, e.g. `14px` or `1e3em`.
    Dimension(f64, String),
    /// CSS Syntax 3 §4.3.9 `<urange>`: `U+41`, `U+41-5A`, `U+4??`. Stores the
    /// verbatim matched text (case, digit count and wildcards as written) —
    /// captured directly off the source so no sign/leading-zero information
    /// is lost the way it would be by re-tokenizing through `Number`/
    /// `Dimension` (see CSS-26).
    UnicodeRange(String),
    Whitespace,
    Colon,
    Semicolon,
    Comma,
    OpenCurly,
    CloseCurly,
    OpenParen,
    CloseParen,
    OpenSquare,
    CloseSquare,
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    _src: &'a str,
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '-' || !c.is_ascii()
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self { chars: src.chars().collect(), pos: 0, _src: src }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    /// CSS Syntax 3 SS4.3.7 "consume an escaped code point". Consumes an
    /// escape sequence assuming cursor sits on the `\`. Returns the decoded
    /// char, or `None` only when this is not a valid escape at all (a
    /// backslash directly followed by a newline) -- the caller then leaves
    /// the `\` unconsumed. Every other case (hex escape, EOF right after the
    /// `\`, or a literal escaped character) always yields a code point, per
    /// spec, so this must not be conflated with "not an escape": an earlier
    /// version returned `None` for a NUL/surrogate/out-of-range hex value
    /// and for EOF too, which desynced the caller's position (the hex
    /// branch had already consumed the whole escape before returning
    /// `None`) and, for EOF, decoded a trailing lone `\` as a literal
    /// backslash instead of U+FFFD (CSS-29; `querySelector`'s CSS escapes
    /// WPT selection, e.g. `#eof\` must NOT match an id of literal `eof\`).
    fn escape(&mut self) -> Option<char> {
        debug_assert_eq!(self.peek(), Some('\\'));
        match self.peek_at(1) {
            None => {
                // Backslash at EOF: a parse error, but the spec's own EOF
                // branch still returns U+FFFD, not "not an escape".
                self.pos += 1;
                Some('\u{FFFD}')
            }
            Some('\n') => None, // not a valid escape; caller leaves the `\` unconsumed.
            Some(c) if c.is_ascii_hexdigit() => {
                self.pos += 1; // past '\'
                let mut value: u32 = 0;
                let mut digits = 0;
                while digits < 6 {
                    match self.peek().and_then(|h| h.to_digit(16)) {
                        Some(d) => {
                            value = value * 16 + d;
                            self.pos += 1;
                            digits += 1;
                        },
                        None => break,
                    }
                }
                // one whitespace after the escape is consumed
                if matches!(self.peek(), Some(' ') | Some('\t') | Some('\n')) {
                    self.pos += 1;
                }
                // A NUL, a surrogate, or a value past the maximum allowed
                // code point all become U+FFFD (`char::from_u32` already
                // rejects surrogates and out-of-range values for us).
                Some(char::from_u32(value).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'))
            },
            Some(_) => {
                self.pos += 1; // past '\'
                Some(self.bump().unwrap())
            },
        }
    }

    fn ident(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(c) if is_ident_char(c) => {
                    out.push(c);
                    self.pos += 1;
                },
                Some('\\') => match self.escape() {
                    Some(c) => out.push(c),
                    None => {
                        out.push('\\');
                        self.pos += 1;
                    },
                },
                _ => break,
            }
        }
        out
    }

    /// Attempts a numeric token at the cursor. Returns None when the next
    /// chars cannot start a number.
    fn number(&mut self) -> Option<Token> {
        let start = self.pos;
        if matches!(self.peek(), Some('+') | Some('-')) {
            self.pos += 1;
        }
        let mut int_digits = 0;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
            int_digits += 1;
        }
        let mut frac_digits = 0;
        if self.peek() == Some('.')
            && self.peek_at(1).is_some_and(|c| c.is_ascii_digit())
        {
            self.pos += 1;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
                frac_digits += 1;
            }
        }
        // exponent
        if matches!(self.peek(), Some('e') | Some('E')) {
            let mut look = 1;
            if matches!(self.peek_at(1), Some('+') | Some('-')) {
                look = 2;
            }
            if self.peek_at(look).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += look;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
        }
        if int_digits + frac_digits == 0 {
            self.pos = start;
            return None;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let value: f64 = text.parse().unwrap_or(0.0);
        match self.peek() {
            Some('%') => {
                self.pos += 1;
                Some(Token::Percentage(value))
            },
            Some(c)
                if is_ident_start(c)
                    || (c == '\\' && self.peek_at(1).is_some_and(|h| h != '\n' && h.is_ascii_hexdigit()))
                    || (c == '\\' && self.peek_at(1).is_some_and(|h| !h.is_ascii_hexdigit() && h != '\n')) =>
            {
                let unit = self.ident();
                Some(Token::Dimension(value, unit))
            },
            _ => Some(Token::Number(value)),
        }
    }

    fn string(&mut self, quote: char) -> Token {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Token::Str(out), // unterminated: recovery
                Some(c) if c == quote => {
                    self.pos += 1;
                    return Token::Str(out);
                },
                Some('\n') => {
                    self.pos += 1;
                    return Token::Str(out); // spec: bad string
                },
                // escape() expects the cursor ON the backslash — peek, don't
                // bump, or the escape char would be consumed twice.
                Some('\\') => match self.escape() {
                    Some(e) => out.push(e),
                    None => {
                        // escape() leaves the cursor on the '\'; the newline
                        // (if any) is the NEXT char.
                        if self.peek_at(1) == Some('\n') {
                            self.pos += 2; // \newline continuation: drop both
                        } else {
                            self.pos += 1;
                            out.push('\\'); // lone trailing backslash
                        }
                    },
                },
                Some(_) => out.push(self.bump().unwrap()),
            }
        }
    }

    /// CSS Syntax 3 §4.3.6. Cursor starts immediately after `url(`.
    fn url(&mut self) -> Token {
        while self.peek().is_some_and(is_whitespace) { self.pos += 1; }
        let start = self.pos;
        let mut value = String::new();
        loop {
            match self.peek() {
                Some(')') => {
                    let raw = self.chars[start..self.pos].iter().collect();
                    self.pos += 1;
                    return Token::Url { value, raw };
                }
                None => return Token::Url { value, raw: self.chars[start..self.pos].iter().collect() },
                Some(c) if is_whitespace(c) => {
                    let end = self.pos;
                    while self.peek().is_some_and(is_whitespace) { self.pos += 1; }
                    if matches!(self.peek(), Some(')') | None) {
                        let raw = self.chars[start..end].iter().collect();
                        if self.peek() == Some(')') { self.pos += 1; }
                        return Token::Url { value, raw };
                    }
                    self.bad_url_remnants();
                    return Token::BadUrl;
                }
                Some('"' | '\'' | '(') => { self.bad_url_remnants(); return Token::BadUrl; }
                Some(c) if c.is_control() => { self.bad_url_remnants(); return Token::BadUrl; }
                Some('\\') => match self.escape() {
                    Some(c) => value.push(c),
                    None => { self.bad_url_remnants(); return Token::BadUrl; }
                },
                Some(c) => { value.push(c); self.pos += 1; }
            }
        }
    }

    /// CSS Syntax 3 §4.3.9 "Consume a unicode-range token". Cursor sits on
    /// the leading `u`/`U`; caller has already verified the next two code
    /// points are `+` followed by a hex digit or `?`. Captures the matched
    /// text verbatim rather than reformatting it, so `U+0131` stays
    /// `U+0131` (not `U+131`) and `U+4??`/`u+41-5a` keep their original
    /// wildcard/case spelling.
    fn unicode_range(&mut self) -> Token {
        let start = self.pos;
        self.pos += 2; // past 'u'/'U' and '+'
        let mut hex_digits = 0;
        while hex_digits < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            self.pos += 1;
            hex_digits += 1;
        }
        let mut wildcards = 0;
        while hex_digits + wildcards < 6 && self.peek() == Some('?') {
            self.pos += 1;
            wildcards += 1;
        }
        // An explicit end (`-005A`) is only valid when no wildcards were
        // consumed for the start.
        if wildcards == 0
            && self.peek() == Some('-')
            && self.peek_at(1).is_some_and(|c| c.is_ascii_hexdigit())
        {
            self.pos += 1; // past '-'
            let mut end_digits = 0;
            while end_digits < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.pos += 1;
                end_digits += 1;
            }
        }
        Token::UnicodeRange(self.chars[start..self.pos].iter().collect())
    }

    fn bad_url_remnants(&mut self) {
        loop {
            match self.peek() {
                None => return,
                Some(')') => { self.pos += 1; return; }
                Some('\\') if self.peek_at(1).is_some_and(|c| c != '\n') => {
                    let _ = self.escape();
                }
                _ => self.pos += 1,
            }
        }
    }
}

fn is_whitespace(c: char) -> bool { matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}') }

/// CSS Syntax 3 SS3.3 "Preprocessing the input stream": normalize CRLF, lone
/// CR, and FF to a single LF, and replace any NUL with U+FFFD, before
/// tokenizing. Without this, a hex escape's trailing whitespace check (which
/// only ever sees an already-normalized LF, never a CR/CRLF/FF) can eat one
/// half of a real CRLF pair and mis-tokenize whatever follows, and a literal
/// NUL byte in the source is left as-is instead of becoming the replacement
/// character real UAs show (CSS-29; `querySelector`'s CSS escapes WPT
/// selection exercises both).
fn preprocess_input(src: &str) -> String {
    if !src.contains(['\r', '\u{000C}', '\0']) {
        return src.to_owned(); // fast path: nothing to normalize
    }
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{000C}' => out.push('\n'),
            '\0' => out.push('\u{FFFD}'),
            _ => out.push(c),
        }
    }
    out
}

/// Tokenizes an entire stylesheet source into a flat token vector.
pub fn tokenize(src: &str) -> Vec<Token> {
    let preprocessed = preprocess_input(src);
    let mut lx = Lexer::new(&preprocessed);
    let mut out = Vec::new();
    loop {
        let tok = match lx.peek() {
            None => break,
            Some(' ' | '\t' | '\n' | '\r' | '\u{000C}') => {
                while lx
                    .peek()
                    .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000C}'))
                {
                    lx.pos += 1;
                }
                Token::Whitespace
            },
            Some('/') if lx.peek_at(1) == Some('*') => {
                lx.pos += 2;
                loop {
                    match lx.peek() {
                        None => break,
                        Some('*') if lx.peek_at(1) == Some('/') => {
                            lx.pos += 2;
                            break;
                        },
                        _ => lx.pos += 1,
                    }
                }
                continue; // comments vanish
            },
            Some('"') => lx.string('"'),
            Some('\'') => lx.string('\''),
            Some('#') => {
                lx.pos += 1;
                match lx.peek() {
                    Some(c) if is_ident_char(c) => Token::Hash(lx.ident()),
                    // CSS Syntax 3 SS4.3.2 "consume a hash token": a hash
                    // token also starts when the next two code points are a
                    // valid escape (a backslash not directly followed by a
                    // newline -- EOF counts as valid too, matching escape()'s
                    // own EOF branch). Without this, `#\30 foo`-style escaped
                    // id selectors tokenized as a lone `Delim('#')` and never
                    // reached `ident()`/`escape()` at all (CSS-29;
                    // `querySelector`'s CSS escapes WPT selection covers this).
                    Some('\\') if lx.peek_at(1) != Some('\n') => Token::Hash(lx.ident()),
                    _ => Token::Delim('#'),
                }
            },
            Some('@') => {
                lx.pos += 1;
                match lx.peek() {
                    Some(c) if is_ident_start(c) => {
                        Token::AtKeyword(lx.ident())
                    },
                    // Same escape-start gap as the hash token above:
                    // `@\65 dia`-style escaped at-keywords must still count.
                    Some('\\') if lx.peek_at(1) != Some('\n') => Token::AtKeyword(lx.ident()),
                    _ => Token::Delim('@'),
                }
            },
            Some(c) if c.is_ascii_digit() => lx.number().expect("digit starts number"),
            Some(c @ ('+' | '-' | '.')) => {
                let numeric = match c {
                    '.' => lx.peek_at(1).is_some_and(|d| d.is_ascii_digit()),
                    '+' => true, // handled by number(); may fail → delim
                    '-' => {
                        // '-2', '-.5', '--ident', '-ident'
                        let n1 = lx.peek_at(1);
                        let n2 = lx.peek_at(2);
                        match n1 {
                            Some(d) if d.is_ascii_digit() => true,
                            Some('.') => {
                                n2.is_some_and(|d| d.is_ascii_digit())
                            },
                            _ => false,
                        }
                    },
                    _ => false,
                };
                if numeric {
                    lx.number().unwrap_or_else(|| {
                        lx.pos += 1;
                        Token::Delim(c)
                    })
                } else if is_ident_start(c) {
                    Token::Ident(lx.ident())
                } else {
                    lx.pos += 1;
                    Token::Delim(c)
                }
            },
            Some(':') => {
                lx.pos += 1;
                Token::Colon
            },
            Some(';') => {
                lx.pos += 1;
                Token::Semicolon
            },
            Some(',') => {
                lx.pos += 1;
                Token::Comma
            },
            Some('{') => {
                lx.pos += 1;
                Token::OpenCurly
            },
            Some('}') => {
                lx.pos += 1;
                Token::CloseCurly
            },
            Some('(') => {
                lx.pos += 1;
                Token::OpenParen
            },
            Some(')') => {
                lx.pos += 1;
                Token::CloseParen
            },
            Some('[') => {
                lx.pos += 1;
                Token::OpenSquare
            },
            Some(']') => {
                lx.pos += 1;
                Token::CloseSquare
            },
            // CSS Syntax 3 §4.3.9: `u`/`U` immediately followed by `+` and a
            // hex digit or `?` starts a unicode-range token instead of an
            // ident. This is unconditional at the tokenizer level (matching
            // real UAs) — outside an `@font-face`'s `unicode-range`
            // descriptor "u+41" is not otherwise meaningful CSS, so there is
            // no ambiguity in practice.
            Some(c @ ('u' | 'U'))
                if lx.peek_at(1) == Some('+')
                    && lx.peek_at(2).is_some_and(|d| d.is_ascii_hexdigit() || d == '?') =>
            {
                let _ = c;
                lx.unicode_range()
            },
            Some(c) if is_ident_start(c) ||
                (c == '\\' && lx.peek_at(1).is_some_and(|next| next != '\n')) => {
                let name = lx.ident();
                if lx.peek() == Some('(') {
                    lx.pos += 1;
                    if name.eq_ignore_ascii_case("url") {
                        let mut offset = 0;
                        while lx.peek_at(offset).is_some_and(is_whitespace) { offset += 1; }
                        if matches!(lx.peek_at(offset), Some('"' | '\'')) {
                            out.push(Token::Function(name));
                            Token::OpenParen
                        } else {
                            lx.url()
                        }
                    } else {
                        out.push(Token::Function(name));
                        Token::OpenParen
                    }
                } else {
                    Token::Ident(name)
                }
            },
            Some(other) => {
                lx.pos += 1;
                Token::Delim(other)
            },
        };
        out.push(tok);
    }
    out
}

/// Reconstructs display text from a token slice — used to store declaration
/// values and selector preludes losslessly enough for later re-parsing.
/// Adjacent tokens concatenate directly; explicit Whitespace tokens become a
/// single space except beside structural delimiters, so values normalize the
/// way CSS tools print them (`rgba(0,0,0,.5)`).
pub fn serialize(tokens: &[Token]) -> String {
    /// Whitespace adjacent to these tokens carries no grouping meaning.
    fn glue_before(t: &Token) -> bool {
        matches!(
            t,
            Token::Comma | Token::OpenParen | Token::OpenSquare | Token::OpenCurly
                | Token::Function(_)
        )
    }
    fn glue_after(t: &Token) -> bool {
        matches!(
            t,
            Token::Comma | Token::CloseParen | Token::CloseSquare | Token::CloseCurly
        )
    }
    /// Whether the whitespace run containing index `i` renders as one space.
    fn ws_renders(tokens: &[Token], i: usize) -> bool {
        let prev = tokens[..i].iter().rev().find(|t| !matches!(t, Token::Whitespace));
        let next = tokens[i + 1..].iter().find(|t| !matches!(t, Token::Whitespace));
        !prev.is_some_and(glue_before) && !next.is_some_and(glue_after)
    }

    let mut out = String::new();
    for (i, tok) in tokens.iter().enumerate() {
        // Serialization must preserve token boundaries after comments vanish.
        // rgb(10/**/175/**/10) has three numeric channels, not one.
        if i > 0
            && !out.ends_with(' ')
            && matches!(tokens[i - 1], Token::Number(_) | Token::Percentage(_) | Token::Dimension(_, _))
            && matches!(tok, Token::Number(_) | Token::Percentage(_) | Token::Dimension(_, _))
        {
            out.push(' ');
        }
        match tok {
            Token::Ident(s) => write_ident(&mut out, s),
            // Hex colors arrive here from declaration values: keep the '#'.
            Token::Hash(s) => {
                out.push('#');
                out.push_str(s);
            },
            // The paired OpenParen token supplies the '('.
            Token::Function(s) => write_ident(&mut out, s),
            Token::Url { raw, .. } => { out.push_str("url("); out.push_str(raw); out.push(')'); },
            Token::BadUrl => out.push_str("url(\""),
            Token::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            },
            Token::Str(s) => {
                out.push('"');
                for ch in s.chars() {
                    if ch == '"' || ch == '\\' {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                out.push('"');
            },
            Token::Delim(c) => out.push(*c),
            Token::Number(n) => out.push_str(&fmt_num(*n)),
            Token::Percentage(n) => {
                out.push_str(&fmt_num(*n));
                out.push('%');
            },
            Token::Dimension(n, unit) => {
                out.push_str(&fmt_num(*n));
                out.push_str(unit);
            },
            Token::UnicodeRange(s) => out.push_str(s),
            Token::Whitespace => {
                if ws_renders(tokens, i) && !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            },
            Token::Colon => out.push(':'),
            Token::Semicolon => out.push(';'),
            // Canonical CSS serialization always separates a function's or a
            // comma-separated list's items with ", " (comma, then exactly one
            // space), regardless of the source's own whitespace -- dropped
            // entirely ("a,b"), doubled ("a  ,  b"), or on the wrong side
            // ("a ,b"). `glue_before` still suppresses any space *before* the
            // comma from the surrounding whitespace-rendering logic below.
            Token::Comma => out.push_str(", "),
            Token::OpenCurly => out.push('{'),
            Token::CloseCurly => out.push('}'),
            Token::OpenParen => out.push('('),
            Token::CloseParen => out.push(')'),
            Token::OpenSquare => out.push('['),
            Token::CloseSquare => out.push(']'),
        }
    }
    out.trim_end().to_string()
}

/// Re-escape decoded identifier characters that would change tokenization.
fn write_ident(out: &mut String, ident: &str) {
    for (index, ch) in ident.chars().enumerate() {
        if ch.is_ascii_digit() && (index == 0 || index == 1 && ident.starts_with('-'))
            || ch.is_control()
        {
            out.push_str(&format!("\\{:x} ", ch as u32));
        } else if is_ident_char(ch) {
            out.push(ch);
        } else {
            out.push('\\');
            out.push(ch);
        }
    }
}

fn fmt_num(n: f64) -> String {
    let s = if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    };
    // Canonical short fraction form: `.5`, not `0.5` (and `-.5`).
    if let Some(rest) = s.strip_prefix("0.") {
        return format!(".{rest}");
    }
    if let Some(rest) = s.strip_prefix("-0.") {
        return format!("-.{rest}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_dropped() {
        assert_eq!(
            tokenize("a/*x*/b"),
            vec![Token::Ident("a".into()), Token::Ident("b".into())]
        );
        assert_eq!(tokenize("/* unterminated"), vec![]);
        assert_eq!(serialize(&tokenize("rgb(10/**/175/**/10)")), "rgb(10 175 10)");
    }

    #[test]
    fn numbers_dimensions_percentages() {
        assert_eq!(tokenize("42"), vec![Token::Number(42.0)]);
        assert_eq!(tokenize("-3.5"), vec![Token::Number(-3.5)]);
        assert_eq!(tokenize("50%"), vec![Token::Percentage(50.0)]);
        assert_eq!(
            tokenize("14px"),
            vec![Token::Dimension(14.0, "px".into())]
        );
        assert_eq!(tokenize("."), vec![Token::Delim('.')]);
        assert_eq!(tokenize("+"), vec![Token::Delim('+')]);
    }

    #[test]
    fn strings_with_escapes() {
        // Unknown escapes decode to the escaped char itself (no C-style \n).
        assert_eq!(
            tokenize("\"a\\nb\""),
            vec![Token::Str("anb".into())]
        );
        assert_eq!(
            tokenize("'it\\'s'"),
            vec![Token::Str("it's".into())]
        );
        // \newline is a continuation, not content.
        assert_eq!(tokenize("\"a\\\nb\""), vec![Token::Str("ab".into())]);
        // hex escape + consumed trailing space
        assert_eq!(tokenize("\"\\41 \""), vec![Token::Str("A".into())]);
        // unterminated string recovers what it saw
        assert_eq!(tokenize("\"abc"), vec![Token::Str("abc".into())]);
    }

    #[test]
    fn idents_hashes_atkeywords_functions() {
        assert_eq!(
            tokenize("color red #ff0000 @media rgb(1,2,3)"),
            vec![
                Token::Ident("color".into()),
                Token::Whitespace,
                Token::Ident("red".into()),
                Token::Whitespace,
                Token::Hash("ff0000".into()),
                Token::Whitespace,
                Token::AtKeyword("media".into()),
                Token::Whitespace,
                Token::Function("rgb".into()),
                Token::OpenParen,
                Token::Number(1.0),
                Token::Comma,
                Token::Number(2.0),
                Token::Comma,
                Token::Number(3.0),
                Token::CloseParen,
            ]
        );
        // '#' without a name is a delim
        assert_eq!(tokenize("# x"), vec![Token::Delim('#'), Token::Whitespace, Token::Ident("x".into())]);
        // '@' without a name is a delim
        assert_eq!(tokenize("@ "), vec![Token::Delim('@'), Token::Whitespace]);
    }

    #[test]
    fn serialize_roundtrip_values() {
        let toks = tokenize("10px solid rgba( 0 , 0 , 0 , .5 )");
        assert_eq!(serialize(&toks), "10px solid rgba(0, 0, 0, .5)");
        let toks = tokenize("\"hello world\"");
        assert_eq!(serialize(&toks), "\"hello world\"");
    }

    #[test]
    fn unquoted_url_is_one_token_with_verbatim_data_payload() {
        for payload in ["AA+1/2==", "A+00/09=", "1234567890+/=="] {
            let css = format!("url(data:font/woff2;base64,{payload})");
            let tokens = tokenize(&css);
            assert_eq!(tokens, vec![Token::Url {
                value: format!("data:font/woff2;base64,{payload}"),
                raw: format!("data:font/woff2;base64,{payload}"),
            }]);
            assert_eq!(serialize(&tokens), css);
        }
        assert_eq!(serialize(&tokenize("url( data:a+1/2==  )")), "url(data:a+1/2==)");
        assert_eq!(tokenize("url('data:a+1/2==')")[0], Token::Function("url".into()));
        assert_eq!(tokenize("url(bad path)"), vec![Token::BadUrl]);
    }

    #[test]
    fn url_escape_has_decoded_value_and_reparseable_raw_spelling() {
        let css = r"url(a\)b)";
        let tokens = tokenize(css);
        assert_eq!(tokens, vec![Token::Url { value: "a)b".into(), raw: r"a\)b".into() }]);
        assert_eq!(serialize(&tokens), css);
    }

    #[test]
    fn escaped_identifier_keeps_its_token_boundary_when_serialized() {
        for css in [r":lang(\*-FR)", r".\31 foo", r".name\+part"] {
            let tokens = tokenize(css);
            assert_eq!(tokenize(&serialize(&tokens)), tokens, "{css}");
        }
    }

    // CSS-26: `<urange>` tokenization (CSS Syntax 3 §4.3.9 / CSS Fonts 4).
    // Regression coverage for the bug where `U+0131` reached the
    // `unicode-range` descriptor as `u131` — the `+` sign and leading zero
    // were lost because a generic `Ident("u")` + `Number(131.0)` pair
    // doesn't retain either. A dedicated `UnicodeRange` token, captured
    // verbatim off the source, fixes that at the root.
    #[test]
    fn unicode_range_single_value_preserves_sign_and_leading_zeros() {
        // The exact bug report case: a 4-digit codepoint with a leading zero.
        assert_eq!(tokenize("U+0131"), vec![Token::UnicodeRange("U+0131".into())]);
        assert_eq!(serialize(&tokenize("U+0131")), "U+0131");
        // Lowercase 'u' and lowercase hex digits are also valid and are kept
        // verbatim rather than being case-normalized.
        assert_eq!(tokenize("u+0041"), vec![Token::UnicodeRange("u+0041".into())]);
        assert_eq!(tokenize("U+f"), vec![Token::UnicodeRange("U+f".into())]);
    }

    #[test]
    fn unicode_range_hyphenated_range_and_wildcard_forms() {
        assert_eq!(tokenize("U+41-5A"), vec![Token::UnicodeRange("U+41-5A".into())]);
        assert_eq!(serialize(&tokenize("U+41-5A")), "U+41-5A");
        // Wildcards stand in for the trailing hex digits; up to 6 hex/`?`
        // code points total are consumed.
        assert_eq!(tokenize("U+4??"), vec![Token::UnicodeRange("U+4??".into())]);
        assert_eq!(tokenize("U+??????"), vec![Token::UnicodeRange("U+??????".into())]);
        // A comma-separated list, as it appears in a real descriptor value.
        assert_eq!(
            tokenize("U+41-5A, U+4??"),
            vec![
                Token::UnicodeRange("U+41-5A".into()),
                Token::Comma,
                Token::Whitespace,
                Token::UnicodeRange("U+4??".into()),
            ]
        );
    }

    #[test]
    fn unicode_range_does_not_hijack_unrelated_u_identifiers() {
        // 'u' not immediately followed by '+' stays an ordinary ident/unit.
        assert_eq!(tokenize("um"), vec![Token::Ident("um".into())]);
        assert_eq!(tokenize("14u"), vec![Token::Dimension(14.0, "u".into())]);
        // `url(...)` is unaffected (its '(' rules it out before '+' is seen).
        assert_eq!(
            tokenize("url(a.woff2)"),
            vec![Token::Url { value: "a.woff2".into(), raw: "a.woff2".into() }]
        );
        // A lone 'u+' with nothing hex/`?` after it is not a urange.
        assert_eq!(
            tokenize("u+x"),
            vec![Token::Ident("u".into()), Token::Delim('+'), Token::Ident("x".into())]
        );
    }
}
