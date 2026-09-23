//! CSS Values math type validation. Expressions stay unresolved until the
//! consumer knows the percentage basis; this module only validates grammar.
use crate::syntax::{Token, tokenize};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Number, Length, Percentage, LengthPercentage }
impl Kind {
    fn add(self, other: Self) -> Option<Self> {
        use Kind::*;
        match (self, other) {
            (Number, Number) => Some(Number),
            (Length, Length) => Some(Length),
            (Percentage, Percentage) => Some(Percentage),
            (Length | Percentage | LengthPercentage, Length | Percentage | LengthPercentage) => Some(LengthPercentage),
            _ => None,
        }
    }
    fn multiply(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Number, kind) | (kind, Self::Number) => Some(kind),
            _ => None,
        }
    }
}

struct Parser { tokens: Vec<Token>, pos: usize }
impl Parser {
    fn skip(&mut self) { while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) { self.pos += 1; } }
    fn peek(&mut self) -> Option<Token> { self.skip(); self.tokens.get(self.pos).cloned() }
    fn eat(&mut self) -> Option<Token> { self.skip(); let t = self.tokens.get(self.pos).cloned(); self.pos += usize::from(t.is_some()); t }
    fn expression(&mut self) -> Option<Kind> {
        let mut left = self.product()?;
        loop {
            self.skip();
            if !matches!(self.tokens.get(self.pos), Some(Token::Delim('+' | '-')))
                && !matches!(self.tokens.get(self.pos), Some(Token::Ident(name)) if name == "-") { break; }
            let operator = self.pos;
            // CSS requires whitespace on both sides of binary + and -.
            if operator == 0 || !matches!(self.tokens.get(operator - 1), Some(Token::Whitespace))
                || !matches!(self.tokens.get(operator + 1), Some(Token::Whitespace)) { return None; }
            self.pos += 1;
            left = left.add(self.product()?)?;
        }
        Some(left)
    }
    fn product(&mut self) -> Option<Kind> {
        let mut left = self.atom()?;
        loop {
            match self.peek() {
                Some(Token::Delim('*')) => { self.eat(); left = left.multiply(self.atom()?)?; }
                Some(Token::Delim('/')) => {
                    self.eat();
                    if self.atom()? != Kind::Number { return None; }
                }
                _ => break,
            }
        }
        Some(left)
    }
    fn atom(&mut self) -> Option<Kind> {
        match self.eat()? {
            Token::Number(n) if n.is_finite() => Some(Kind::Number),
            Token::Percentage(n) if n.is_finite() => Some(Kind::Percentage),
            Token::Dimension(n, unit) if n.is_finite() => {
                let css = format!("{n}{unit}");
                crate::values::LengthPercentage::parse(&css).map(|_| Kind::Length)
            }
            Token::OpenParen => {
                let kind = self.expression()?;
                match self.eat()? { Token::CloseParen => Some(kind), _ => None }
            }
            Token::Function(name) => {
                if !matches!(self.eat()?, Token::OpenParen) { return None; }
                let name = name.to_ascii_lowercase();
                let first = self.expression()?;
                let kind = match name.as_str() {
                    "calc" => first,
                    "min" | "max" | "clamp" => {
                        let mut kind = first;
                        let mut count = 1;
                        while matches!(self.peek(), Some(Token::Comma)) {
                            self.eat();
                            kind = kind.add(self.expression()?)?;
                            count += 1;
                        }
                        if (name == "clamp" && count != 3) || (name != "clamp" && count < 1) { return None; }
                        kind
                    }
                    _ => return None,
                };
                match self.eat()? { Token::CloseParen => Some(kind), _ => None }
            }
            _ => None,
        }
    }
}

/// Validate a complete CSS math expression as a length or length-percentage.
/// The caller retains the canonical serialized tokens in ComputedStyle::get.
pub(crate) fn length_percentage(css: &str, allow_percentage: bool) -> bool {
    let tokens = tokenize(css);
    if !matches!(tokens.iter().find(|t| !matches!(t, Token::Whitespace)),
        Some(Token::Function(name)) if matches!(name.to_ascii_lowercase().as_str(), "calc" | "min" | "max" | "clamp")) { return false; }
    let mut parser = Parser { tokens, pos: 0 };
    let kind = parser.atom();
    parser.skip();
    parser.pos == parser.tokens.len() && match kind {
        Some(Kind::Length) => true,
        Some(Kind::Percentage | Kind::LengthPercentage) => allow_percentage,
        _ => false,
    }
}
