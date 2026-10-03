//! Lazy scanner snapshots let the parser choose the slash lexical goal.
//! Speculative lookahead is discarded when an operand starts with a slash.
use crate::errors::SyntaxError;
use crate::lexer::Lexer;
use crate::tokens::{SourceLocation, Span, Token, TokenKind};

pub(crate) struct TokenStream {
    lexer: Lexer,
    tokens: Vec<Token>,
    before: Vec<Lexer>,
}
impl TokenStream {
    pub(crate) fn new(source: &str) -> Self {
        Self {
            lexer: Lexer::new(source),
            tokens: Vec::new(),
            before: Vec::new(),
        }
    }
    pub(crate) fn set_module_goal(&mut self) {
        self.lexer.set_module_goal();
    }
    pub(crate) fn legacy_escape_in(&self, span: Span) -> Option<SourceLocation> {
        self.tokens.iter().find(|token| token.legacy_escape
            && token.span.start >= span.start && token.span.end <= span.end)
            .map(|token| token.span.start)
    }
    pub(crate) fn legacy_numeric_at(&self, index: usize) -> bool {
        matches!(self.tokens.get(index).map(|token| &token.kind), Some(TokenKind::Numeric(_)))
            && self.before[index].legacy_numeric_ahead()
    }
    pub(crate) fn legacy_numeric_in(&self, span: Span) -> Option<SourceLocation> {
        self.tokens.iter().enumerate().find(|(index, token)|
            token.span.start >= span.start && token.span.end <= span.end
                && self.legacy_numeric_at(*index))
            .map(|(_, token)| token.span.start)
    }
    pub(crate) fn get(&mut self, index: usize) -> Token {
        while self.tokens.len() <= index {
            if let Some(t) = self.tokens.last() {
                if matches!(t.kind, TokenKind::Eof | TokenKind::Invalid(_)) {
                    return t.clone();
                }
            }
            self.before.push(self.lexer.clone());
            let t = self.lexer.next_token().unwrap_or_else(|error| {
                let at = SourceLocation::new(error.line, error.column);
                Token::new(TokenKind::Invalid(error), Span::new(at, at), false)
            });
            self.tokens.push(t);
        }
        self.tokens[index].clone()
    }
    pub(crate) fn regex(&mut self, index: usize) -> Result<(), SyntaxError> {
        self.get(index);
        self.lexer = self.before[index].clone();
        let token = self.lexer.regex_token()?;
        self.tokens.truncate(index);
        self.before.truncate(index + 1);
        self.tokens.push(token);
        Ok(())
    }
}
