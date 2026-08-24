//! CSS Syntax Module Level 3 tokenizer (§4, "css-syntax-3") — Phase 2 subset.
//!
//! Produces a flat component-value token stream. Deliberate simplifications
//! (all documented here and in lib.rs):
//! - Comments (`/* … */`) are dropped entirely; unclosed comment runs to EOF.
//! - `url()` is not a dedicated token; it tokenizes as `Function("url")`
//!   followed by its contents (good enough until images exist).
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

    /// Consumes an escape sequence assuming cursor sits on the `\`.
    /// Returns the decoded char, or `None` when the escape is invalid
    /// (caller emits the backslash as a literal).
    fn escape(&mut self) -> Option<char> {
        debug_assert_eq!(self.peek(), Some('\\'));
        match self.peek_at(1) {
            None => None, // trailing backslash at EOF
            Some('\n') => None,
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
                char::from_u32(value).filter(|&c| c != '\0')
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
}

/// Tokenizes an entire stylesheet source into a flat token vector.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut lx = Lexer::new(src);
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
                    _ => Token::Delim('#'),
                }
            },
            Some('@') => {
                lx.pos += 1;
                match lx.peek() {
                    Some(c) if is_ident_start(c) => {
                        Token::AtKeyword(lx.ident())
                    },
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
            Some(c) if is_ident_start(c) => {
                let name = lx.ident();
                if lx.peek() == Some('(') {
                    lx.pos += 1;
                    out.push(Token::Function(name));
                    Token::OpenParen
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
        match tok {
            Token::Ident(s) => out.push_str(s),
            // Hex colors arrive here from declaration values: keep the '#'.
            Token::Hash(s) => {
                out.push('#');
                out.push_str(s);
            },
            // The paired OpenParen token supplies the '('.
            Token::Function(s) => out.push_str(s),
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
            Token::Whitespace => {
                if ws_renders(tokens, i) && !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            },
            Token::Colon => out.push(':'),
            Token::Semicolon => out.push(';'),
            Token::Comma => out.push(','),
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
        assert_eq!(serialize(&toks), "10px solid rgba(0,0,0,.5)");
        let toks = tokenize("\"hello world\"");
        assert_eq!(serialize(&toks), "\"hello world\"");
    }
}
