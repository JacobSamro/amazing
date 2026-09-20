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
