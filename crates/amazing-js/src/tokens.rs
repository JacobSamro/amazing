//! Token vocabulary for the ECMA-262 lexical grammar (spec §12 / §13).
//!
//! Implemented:
//! - Every reserved word of the current spec plus the literals `null`,
//!   `true`, `false` as [`Keyword`] ([`keyword_of`]).
//! - The complete punctuator set with **longest-match** semantics
//!   (`>>>= >>>= ??= &&= ||= **= ?. ... =>` etc.) as [`Punct`].
//! - Numeric literal *storage* as [`Numeric`]: the digit text with `_`
//!   separators removed, the radix, and the BigInt flag. Value conversion
//!   (`f64` / big-integer arithmetic) belongs to the interpreter half.
//! - String/template cooked values live in [`TokenKind`] variants produced
//!   by [`crate::lexer`].
//! - Per-token 1-based [`SourceLocation`] spans and the ASI-critical
//!   `newline_before` flag.
//!
//! Deferred (documented, not implemented):
//! - Contextual keywords (`let of get set static async as from accessor`)
//!   deliberately lex as plain identifiers; the parser matches them by
//!   spelling. Future-reserved words (`enum implements interface package
//!   private protected public static`) are likewise left as identifiers —
//!   strict-mode early errors are deferred wholesale.
//! - Decorators (`@`): rejected as unexpected
//!   characters.
//! - Token byte offsets; only line/column positions are kept.
//! - Raw (uncooked) template text, needed later for tagged templates.

use crate::errors::SyntaxError;

/// A 1-based position in the source text (columns count `char`s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceLocation {
    pub line: u32,
    pub column: u32,
}

impl SourceLocation {
    pub fn new(line: u32, column: u32) -> Self {
        SourceLocation { line, column }
    }
}

/// Half-open span from one source location up to (not including) another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl Span {
    pub fn new(start: SourceLocation, end: SourceLocation) -> Self {
        Span { start, end }
    }

    /// Span covering everything from `a.start` through `b.end`; both spans
    /// must come from the same file (they always do here).
    pub fn to(a: Span, b: Span) -> Span {
        Span {
            start: a.start,
            end: if b.end >= a.end { b.end } else { a.end },
        }
    }
}

/// A scanned numeric literal, normalized: no `_` separators, no radix
/// prefix, but decimal `raw` keeps its `.fraction`/`e10` spelling verbatim
/// so the interpreter can parse it losslessly with `f64::from_str`.
///
/// For radix 16/8/2, `raw` holds bare digits (`"ff"`, not `"0xff"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numeric {
    pub raw: String,
    pub radix: u8,
    pub bigint: bool,
}

impl Numeric {
    pub fn new(raw: impl Into<String>, radix: u8, bigint: bool) -> Self {
        Numeric {
            raw: raw.into(),
            radix,
            bigint,
        }
    }
}

/// Lexical token kinds. Identifiers and keywords are distinct kinds;
/// contextual keywords are [`TokenKind::Ident`] by design.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Numeric literal (`1`, `0xFF`, `1.5e3`, `10n`, ...).
    Numeric(Numeric),
    /// Selected by the parser under the RegExp lexical goal.
    RegExp {
        pattern: String,
        flags: String,
    },
    /// Deferred lexical error in parser lookahead; never emitted by tokenize.
    Invalid(SyntaxError),
    /// String literal with escapes already cooked.
    Str(String),
    /// One chunk of a template literal. `tail == true` means this part ends
    /// at the closing backtick; `tail == false` means it ends at a `${`
    /// substitution opening (whose `${` is emitted as
    /// [`Punct::DollarBrace`]). The `}` closing a substitution never
    /// surfaces as [`Punct::Rbrace`] — the lexer consumes it together with
    /// the following template chunk (see [`crate::lexer`]).
    TemplatePart {
        cooked: String,
        tail: bool,
    },
    /// Identifier (including every contextual keyword spelling).
    Ident(String),
    /// A class private identifier, including its leading `#`.
    PrivateIdent(String),
    Keyword(Keyword),
    Punct(Punct),
    Eof,
}

impl TokenKind {
    /// Short human label used in "expected X, found Y" messages.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::RegExp { .. } => "RegExp literal".into(),
            TokenKind::Invalid(e) => e.message.clone(),
            TokenKind::Numeric(n) => format!("number `{}`", n.raw),
            TokenKind::Str(_) => "string literal".into(),
            TokenKind::TemplatePart { .. } => "template literal".into(),
            TokenKind::Ident(name) => format!("identifier `{}`", name),
            TokenKind::PrivateIdent(name) => format!("private identifier `{}`", name),
            TokenKind::Keyword(k) => format!("keyword `{}`", k.spelling()),
            TokenKind::Punct(p) => format!("`{}`", p.spelling()),
            TokenKind::Eof => "end of input".into(),
        }
    }
}

/// One token with its source span and whether a line terminator (or block
/// comment containing one) preceded it — the input to automatic semicolon
/// insertion.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub newline_before: bool,
    /// Lossless cooked code units for string literals and template chunks.
    pub cooked_utf16: Option<Vec<u16>>,
    /// Source spelling of a template chunk, excluding its delimiters.
    pub raw_utf16: Option<Vec<u16>>,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span, newline_before: bool) -> Self {
        Token {
            kind,
            span,
            newline_before,
            cooked_utf16: None,
            raw_utf16: None,
        }
    }
}

/// Reserved words plus the literal keywords. Everything else that looks
/// keyword-ish in real code (`let`, `of`, `static`, `async`, `get`, ...)
/// is intentionally an identifier here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Break,
    Case,
    Catch,
    Class,
    Const,
    Continue,
    Debugger,
    Default,
    Delete,
    Do,
    Else,
    Export,
    Extends,
    Finally,
    For,
    Function,
    If,
    Import,
    In,
    InstanceOf,
    New,
    Null,
    Return,
    Super,
    Switch,
    This,
    Throw,
    True,
    False,
    Try,
    Typeof,
    Var,
    Void,
    While,
    With,
    Yield,
}

impl Keyword {
    /// Canonical spelling (used in diagnostics).
    pub fn spelling(self) -> &'static str {
        match self {
            Keyword::Break => "break",
            Keyword::Case => "case",
            Keyword::Catch => "catch",
            Keyword::Class => "class",
            Keyword::Const => "const",
            Keyword::Continue => "continue",
            Keyword::Debugger => "debugger",
            Keyword::Default => "default",
            Keyword::Delete => "delete",
            Keyword::Do => "do",
            Keyword::Else => "else",
            Keyword::Export => "export",
            Keyword::Extends => "extends",
            Keyword::Finally => "finally",
            Keyword::For => "for",
            Keyword::Function => "function",
            Keyword::If => "if",
            Keyword::Import => "import",
            Keyword::In => "in",
            Keyword::InstanceOf => "instanceof",
            Keyword::New => "new",
            Keyword::Null => "null",
            Keyword::Return => "return",
            Keyword::Super => "super",
            Keyword::Switch => "switch",
            Keyword::This => "this",
            Keyword::Throw => "throw",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Try => "try",
            Keyword::Typeof => "typeof",
            Keyword::Var => "var",
            Keyword::Void => "void",
            Keyword::While => "while",
            Keyword::With => "with",
            Keyword::Yield => "yield",
        }
    }
}

/// Maps an identifier-shaped string to its keyword, if reserved.
pub fn keyword_of(s: &str) -> Option<Keyword> {
    let k = match s {
        "break" => Keyword::Break,
        "case" => Keyword::Case,
        "catch" => Keyword::Catch,
        "class" => Keyword::Class,
        "const" => Keyword::Const,
        "continue" => Keyword::Continue,
        "debugger" => Keyword::Debugger,
        "default" => Keyword::Default,
        "delete" => Keyword::Delete,
        "do" => Keyword::Do,
        "else" => Keyword::Else,
        "export" => Keyword::Export,
        "extends" => Keyword::Extends,
        "finally" => Keyword::Finally,
        "for" => Keyword::For,
        "function" => Keyword::Function,
        "if" => Keyword::If,
        "import" => Keyword::Import,
        "in" => Keyword::In,
        "instanceof" => Keyword::InstanceOf,
        "new" => Keyword::New,
        "null" => Keyword::Null,
        "return" => Keyword::Return,
        "super" => Keyword::Super,
        "switch" => Keyword::Switch,
        "this" => Keyword::This,
        "throw" => Keyword::Throw,
        "true" => Keyword::True,
        "false" => Keyword::False,
        "try" => Keyword::Try,
        "typeof" => Keyword::Typeof,
        "var" => Keyword::Var,
        "void" => Keyword::Void,
        "while" => Keyword::While,
        "with" => Keyword::With,
        "yield" => Keyword::Yield,
        _ => return None,
    };
    Some(k)
}

/// Every punctuator in the grammar, matched longest-first by the lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punct {
    Lbrace,
    Rbrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Dot,
    Ellipsis,
    Arrow,
    Question,
    QuestionDot,
    Nullish,
    NullishEq,
    Colon,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    NotEq,
    EqEqEq,
    NotEqEq,
    Eq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Pow,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    PowEq,
    Shl,
    Shr,
    UShr,
    ShlEq,
    ShrEq,
    UShrEq,
    BitAnd,
    BitOr,
    BitXor,
    Bang,
    Tilde,
    And,
    Or,
    AndEq,
    OrEq,
    XorEq,
    AndAndEq,
    OrOrEq,
    Inc,
    Dec,
    /// The `${` that opens a template substitution (never appears in plain
    /// code; see [`TokenKind::TemplatePart`]).
    DollarBrace,
}

impl Punct {
    /// Canonical spelling (used in diagnostics).
    pub fn spelling(self) -> &'static str {
        match self {
            Punct::Lbrace => "{",
            Punct::Rbrace => "}",
            Punct::LParen => "(",
            Punct::RParen => ")",
            Punct::LBracket => "[",
            Punct::RBracket => "]",
            Punct::Semi => ";",
            Punct::Comma => ",",
            Punct::Dot => ".",
            Punct::Ellipsis => "...",
            Punct::Arrow => "=>",
            Punct::Question => "?",
            Punct::QuestionDot => "?.",
            Punct::Nullish => "??",
            Punct::NullishEq => "??=",
            Punct::Colon => ":",
            Punct::Lt => "<",
            Punct::Gt => ">",
            Punct::Le => "<=",
            Punct::Ge => ">=",
            Punct::EqEq => "==",
            Punct::NotEq => "!=",
            Punct::EqEqEq => "===",
            Punct::NotEqEq => "!==",
            Punct::Eq => "=",
            Punct::Plus => "+",
            Punct::Minus => "-",
            Punct::Star => "*",
            Punct::Slash => "/",
            Punct::Percent => "%",
            Punct::Pow => "**",
            Punct::PlusEq => "+=",
            Punct::MinusEq => "-=",
            Punct::StarEq => "*=",
            Punct::SlashEq => "/=",
            Punct::PercentEq => "%=",
            Punct::PowEq => "**=",
            Punct::Shl => "<<",
            Punct::Shr => ">>",
            Punct::UShr => ">>>",
            Punct::ShlEq => "<<=",
            Punct::ShrEq => ">>=",
            Punct::UShrEq => ">>>=",
            Punct::BitAnd => "&",
            Punct::BitOr => "|",
            Punct::BitXor => "^",
            Punct::Bang => "!",
            Punct::Tilde => "~",
            Punct::And => "&&",
            Punct::Or => "||",
            Punct::AndEq => "&=",
            Punct::OrEq => "|=",
            Punct::XorEq => "^=",
            Punct::AndAndEq => "&&=",
            Punct::OrOrEq => "||=",
            Punct::Inc => "++",
            Punct::Dec => "--",
            Punct::DollarBrace => "${",
        }
    }
}

/// Convenience constructor used by the lexer's tests and the parser.
pub(crate) fn syntax_error(message: impl Into<String>, at: SourceLocation) -> SyntaxError {
    SyntaxError::new(message, at.line, at.column)
}

#[cfg(test)]
mod tests {
    use super::{keyword_of, Keyword, Punct};

    #[test]
    fn every_reserved_word_maps_to_its_keyword() {
        assert_eq!(keyword_of("function"), Some(Keyword::Function));
        assert_eq!(keyword_of("yield"), Some(Keyword::Yield));
        assert_eq!(keyword_of("null"), Some(Keyword::Null));
        assert_eq!(keyword_of("false"), Some(Keyword::False));
        assert_eq!(keyword_of("with"), Some(Keyword::With));
    }

    #[test]
    fn contextual_keywords_are_not_reserved() {
        // Deliberate: these stay identifiers so the parser owns their
        // context sensitivity.
        for word in [
            "let", "of", "get", "set", "static", "async", "from", "accessor",
        ] {
            assert_eq!(keyword_of(word), None, "`{word}` must not be reserved");
        }
    }

    #[test]
    fn future_reserved_words_are_left_as_identifiers() {
        // Strict-mode early errors deferred; documented in module docs.
        for word in [
            "enum",
            "implements",
            "interface",
            "package",
            "private",
            "protected",
            "public",
            "static",
        ] {
            assert_eq!(keyword_of(word), None, "`{word}` must not be reserved yet");
        }
    }

    #[test]
    fn keyword_spellings_round_trip() {
        for kw in [
            Keyword::Break,
            Keyword::Case,
            Keyword::Catch,
            Keyword::Class,
            Keyword::Const,
            Keyword::Continue,
            Keyword::Debugger,
            Keyword::Default,
            Keyword::Delete,
            Keyword::Do,
            Keyword::Else,
            Keyword::Export,
            Keyword::Extends,
            Keyword::Finally,
            Keyword::For,
            Keyword::Function,
            Keyword::If,
            Keyword::Import,
            Keyword::In,
            Keyword::InstanceOf,
            Keyword::New,
            Keyword::Null,
            Keyword::Return,
            Keyword::Super,
            Keyword::Switch,
            Keyword::This,
            Keyword::Throw,
            Keyword::True,
            Keyword::False,
            Keyword::Try,
            Keyword::Typeof,
            Keyword::Var,
            Keyword::Void,
            Keyword::While,
            Keyword::With,
            Keyword::Yield,
        ] {
            assert_eq!(keyword_of(kw.spelling()), Some(kw));
        }
    }

    #[test]
    fn punctuator_spellings_are_distinct_and_canonical() {
        let all = [
            Punct::Lbrace,
            Punct::Rbrace,
            Punct::LParen,
            Punct::RParen,
            Punct::LBracket,
            Punct::RBracket,
            Punct::Semi,
            Punct::Comma,
            Punct::Dot,
            Punct::Ellipsis,
            Punct::Arrow,
            Punct::Question,
            Punct::QuestionDot,
            Punct::Nullish,
            Punct::NullishEq,
            Punct::Colon,
            Punct::Lt,
            Punct::Gt,
            Punct::Le,
            Punct::Ge,
            Punct::EqEq,
            Punct::NotEq,
            Punct::EqEqEq,
            Punct::NotEqEq,
            Punct::Eq,
            Punct::Plus,
            Punct::Minus,
            Punct::Star,
            Punct::Slash,
            Punct::Percent,
            Punct::Pow,
            Punct::PlusEq,
            Punct::MinusEq,
            Punct::StarEq,
            Punct::SlashEq,
            Punct::PercentEq,
            Punct::PowEq,
            Punct::Shl,
            Punct::Shr,
            Punct::UShr,
            Punct::ShlEq,
            Punct::ShrEq,
            Punct::UShrEq,
            Punct::BitAnd,
            Punct::BitOr,
            Punct::BitXor,
            Punct::Bang,
            Punct::Tilde,
            Punct::And,
            Punct::Or,
            Punct::AndEq,
            Punct::OrEq,
            Punct::XorEq,
            Punct::AndAndEq,
            Punct::OrOrEq,
            Punct::Inc,
            Punct::Dec,
            Punct::DollarBrace,
        ];
        let mut spellings: Vec<&str> = all.iter().map(|p| p.spelling()).collect();
        spellings.sort_unstable();
        let count = spellings.len();
        spellings.dedup();
        assert_eq!(spellings.len(), count, "two punctuators share a spelling");
    }
}
