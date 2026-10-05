//! CSS Values math type validation. Expressions stay unresolved until the
//! consumer knows the percentage basis; this module only validates grammar.
use crate::syntax::{Token, tokenize};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Number, Length, Percentage, LengthPercentage, Angle }
impl Kind {
    fn add(self, other: Self) -> Option<Self> {
        use Kind::*;
        match (self, other) {
            (Number, Number) => Some(Number),
            (Length, Length) => Some(Length),
            (Angle, Angle) => Some(Angle),
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

struct Parser { tokens: Vec<Token>, pos: usize, allow_size: bool }
impl Parser {
    fn skip(&mut self) { while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) { self.pos += 1; } }
    fn peek(&mut self) -> Option<Token> { self.skip(); self.tokens.get(self.pos).cloned() }
    fn eat(&mut self) -> Option<Token> { self.skip(); let t = self.tokens.get(self.pos).cloned(); self.pos += usize::from(t.is_some()); t }

    /// CSS Values 5 `<calc-size-basis>`:
    /// `[ <size-keyword> | <calc-sum> | <calc-size()> | any ]`.
    /// Returns `Some(true)` for the `any` basis (whose calculation may not use
    /// the `size` keyword), `Some(false)` otherwise, `None` when invalid.
    fn calc_size_basis(&mut self) -> Option<bool> {
        match self.peek()? {
            Token::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                if lower == "any" {
                    self.eat();
                    return Some(true);
                }
                matches!(
                    lower.as_str(),
                    "auto" | "min-content" | "max-content" | "fit-content" | "stretch" | "contain"
                )
                .then(|| {
                    self.eat();
                    false
                })
            }
            Token::Function(name) if name.eq_ignore_ascii_case("calc-size") => {
                self.calc_size_function().then_some(false)
            }
            // `fit-content()` is the functional variation of the sizing keyword.
            Token::Function(name) if name.eq_ignore_ascii_case("fit-content") => {
                self.eat();
                if !matches!(self.eat(), Some(Token::OpenParen)) { return None; }
                self.expression_plain()?;
                matches!(self.eat(), Some(Token::CloseParen)).then_some(false)
            }
            _ => self.expression_plain().filter(|kind| *kind != Kind::Angle).map(|_| false),
        }
    }

    /// Parse a `<calc-sum>` while the `size` keyword is not in scope.
    fn expression_plain(&mut self) -> Option<Kind> {
        let saved = self.allow_size;
        self.allow_size = false;
        let kind = self.expression();
        self.allow_size = saved;
        kind
    }

    /// `calc-size( <calc-size-basis> , <calc-sum> )`.
    fn calc_size_function(&mut self) -> bool {
        match self.eat() {
            Some(Token::Function(name)) if name.eq_ignore_ascii_case("calc-size") => {}
            _ => return false,
        }
        if !matches!(self.eat(), Some(Token::OpenParen)) { return false; }
        self.calc_size_body()
    }

    /// `<calc-size-basis> , <calc-sum> )`, assuming the `calc-size(` function
    /// token and its opening paren are already consumed — shared by the
    /// top-level `calc_size_function` and a `calc-size()` nested inside
    /// `min()`/`max()`/`clamp()` (CSS Values 5 §11: nesting is allowed since
    /// `calc-size()` is itself a `<calc-sum>` production).
    fn calc_size_body(&mut self) -> bool {
        let Some(any) = self.calc_size_basis() else { return false; };
        if !matches!(self.eat(), Some(Token::Comma)) { return false; }
        let saved = self.allow_size;
        self.allow_size = !any;
        let calculation = self.expression();
        self.allow_size = saved;
        calculation.is_some_and(|kind| kind != Kind::Angle) && matches!(self.eat(), Some(Token::CloseParen))
    }
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
                    .or_else(|| matches!(unit.to_ascii_lowercase().as_str(),"cqw"|"cqh"|"cqi"|"cqb"|"cqmin"|"cqmax").then_some(Kind::Length))
                    .or_else(|| crate::values::Angle::parse(&css).map(|_| Kind::Angle))
            }
            // `size` names the calc-size basis inside its calculation.
            Token::Ident(name) if self.allow_size && name.eq_ignore_ascii_case("size") => {
                Some(Kind::Length)
            }
            Token::OpenParen => {
                let kind = self.expression()?;
                match self.eat()? { Token::CloseParen => Some(kind), _ => None }
            }
            Token::Function(name) => {
                if !matches!(self.eat()?, Token::OpenParen) { return None; }
                let name = name.to_ascii_lowercase();
                if matches!(name.as_str(),"sibling-index"|"sibling-count") {
                    return matches!(self.eat(),Some(Token::CloseParen)).then_some(Kind::Number);
                }
                if name == "calc-size" {
                    // `<calc-size-basis>` starts with a bare keyword (`auto`,
                    // `any`, ...) that `expression()` below cannot parse, so
                    // dispatch before it rather than through the shared
                    // `first = self.expression()?` path every other function uses.
                    return self.calc_size_body().then_some(Kind::Length);
                }
                let first = self.expression()?;
                let kind = match name.as_str() {
                    "calc" => first,
                    "sign" => Kind::Number,
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
    let mut parser = Parser { tokens, pos: 0, allow_size: false };
    let kind = parser.atom();
    parser.skip();
    parser.pos == parser.tokens.len() && match kind {
        Some(Kind::Length) => true,
        Some(Kind::Percentage | Kind::LengthPercentage) => allow_percentage,
        _ => false,
    }
}

/// A percentage-only math expression; unlike length-percentage, a length
/// dimension cannot be combined with percentage arguments here.
pub(crate) fn percentage(css: &str) -> bool {
    let mut parser = Parser { tokens: tokenize(css), pos: 0, allow_size: false };
    let kind = parser.atom();
    parser.skip();
    kind == Some(Kind::Percentage) && parser.pos == parser.tokens.len()
}

/// Validate a CSS Values 5 `calc-size( <calc-size-basis>, <calc-sum> )`
/// expression without resolving its intrinsic `size` keyword. The caller keeps
/// the canonical serialized text for the layout used-value stage.
pub(crate) fn calc_size(css: &str) -> bool {
    let tokens = tokenize(css);
    if !matches!(tokens.iter().find(|t| !matches!(t, Token::Whitespace)),
        Some(Token::Function(name)) if name.eq_ignore_ascii_case("calc-size")) {
        return false;
    }
    let mut parser = Parser { tokens, pos: 0, allow_size: false };
    parser.calc_size_function() && {
        parser.skip();
        parser.pos == parser.tokens.len()
    }
}

/// Resolve font-size math once its parent and root font sizes are known.
/// Viewport and font-metric units stay unresolved for the layout consumer.
pub(crate) fn font_size_px(css: &str, parent_px: f64, root_px: f64) -> Option<f64> {
    if !length_percentage(css, true) { return None; }
    evaluate_numeric(css, parent_px, root_px, false, false)
}

/// Pure number computation. Context-dependent dimensions remain unresolved.
pub(crate) fn computed_number(css: &str) -> Option<f64> {
    if !number_percentage(css, false) { return None; }
    evaluate_numeric(css, f64::NAN, f64::NAN, true, false)
}
/// Percentage-only math, independent of layout geometry.
pub(crate) fn computed_percentage(css:&str)->Option<f64> {
    if !percentage(css) {return None;}
    let tokens:Vec<_>=tokenize(css).into_iter().map(|t|match t {Token::Percentage(n)=>Token::Number(n),other=>other}).collect();
    computed_number(&crate::syntax::serialize(&tokens))
}
/// Computed angle serialization uses degrees directly. Routing an authored
/// degree through radians and back injects binary roundoff into its CSS text.
/// Validate dimensions with the existing angle grammar before evaluating the
/// same expression in degrees; the typed matrix path still uses radians.
pub(crate) fn computed_angle_degrees(css: &str) -> Option<f64> {
    let tokens = tokenize(css);
    let significant: Vec<_> = tokens.iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    if matches!(significant.as_slice(), [Token::Number(n)] if *n == 0.0) {
        return Some(0.0);
    }
    if !angle(css) { return None; }
    let tokens = tokens.into_iter().map(|token| match token {
        Token::Dimension(n, unit) => {
            let degrees = match unit.to_ascii_lowercase().as_str() {
                "deg" => n,
                "grad" => n * 0.9,
                "turn" => n * 360.0,
                "rad" => n.to_degrees(),
                _ => return None,
            };
            degrees.is_finite().then_some(Token::Number(degrees))
        }
        token => Some(token),
    }).collect::<Option<Vec<_>>>()?;
    computed_number(&crate::syntax::serialize(&tokens))
}
/// Pure angle computation in radians, without inventing layout/font context.
pub(crate) fn computed_angle(css: &str) -> Option<f64> {
    if !angle(css) { return None; }
    evaluate_numeric(css, f64::NAN, f64::NAN, false, true)
}
fn evaluate_numeric(css: &str, parent_px: f64, root_px: f64, number: bool, angles: bool) -> Option<f64> {
    #[derive(Clone, Copy)]
    enum Value { Number(f64), Length(f64) }
    impl Value {
        fn add(self, other: Self, subtract: bool) -> Option<Self> {
            let sign = if subtract { -1.0 } else { 1.0 };
            match (self, other) {
                (Self::Number(a), Self::Number(b)) => Some(Self::Number(a + sign * b)),
                (Self::Length(a), Self::Length(b)) => Some(Self::Length(a + sign * b)),
                _ => None,
            }
        }
        fn multiply(self, other: Self) -> Option<Self> {
            match (self, other) {
                (Self::Number(a), Self::Number(b)) => Some(Self::Number(a * b)),
                (Self::Number(a), Self::Length(b)) | (Self::Length(b), Self::Number(a)) => Some(Self::Length(a * b)),
                _ => None,
            }
        }
        fn divide(self, other: Self) -> Option<Self> {
            let Self::Number(divisor) = other else { return None; };
            if divisor == 0.0 { return None; }
            match self {
                Self::Number(value) => Some(Self::Number(value / divisor)),
                Self::Length(value) => Some(Self::Length(value / divisor)),
            }
        }
        fn length(self) -> Option<f64> {
            match self { Self::Length(value) if value.is_finite() => Some(value), _ => None }
        }
    }
    struct Eval { tokens: Vec<Token>, pos: usize, parent_px: f64, root_px: f64, angles: bool }
    impl Eval {
        fn skip(&mut self) { while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) { self.pos += 1; } }
        fn eat(&mut self) -> Option<Token> { self.skip(); let token = self.tokens.get(self.pos).cloned(); self.pos += usize::from(token.is_some()); token }
        fn peek(&mut self) -> Option<Token> { self.skip(); self.tokens.get(self.pos).cloned() }
        fn expression(&mut self) -> Option<Value> {
            let mut value = self.product()?;
            loop {
                let subtract = match self.peek() {
                    Some(Token::Delim('+')) => false,
                    Some(Token::Delim('-')) => true,
                    Some(Token::Ident(ref sign)) if sign == "-" => true,
                    _ => break,
                };
                self.eat();
                value = value.add(self.product()?, subtract)?;
            }
            Some(value)
        }
        fn product(&mut self) -> Option<Value> {
            let mut value = self.atom()?;
            loop {
                match self.peek() {
                    Some(Token::Delim('*')) => { self.eat(); value = value.multiply(self.atom()?)?; }
                    Some(Token::Delim('/')) => { self.eat(); value = value.divide(self.atom()?)?; }
                    _ => break,
                }
            }
            Some(value)
        }
        fn atom(&mut self) -> Option<Value> {
            match self.eat()? {
                Token::Number(value) if value.is_finite() => Some(Value::Number(value)),
                Token::Percentage(value) if value.is_finite() => Some(Value::Length(self.parent_px * value / 100.0)),
                Token::Dimension(value, unit) if value.is_finite() => {
                    use crate::values::{LengthPercentage as L, RelativeUnit as R};
                    if self.angles { return crate::values::Angle::parse(&format!("{value}{unit}")).map(|a|Value::Length(a.radians)); }
                    let length = L::parse(&format!("{value}{unit}"))?;
                    match length {
                        L::Px(px) => Some(Value::Length(px)),
                        L::Relative(n, R::Em) => Some(Value::Length(n * self.parent_px)),
                        L::Relative(n, R::Rem) => Some(Value::Length(n * self.root_px)),
                        _ => None,
                    }
                }
                Token::OpenParen => {
                    let value = self.expression()?;
                    matches!(self.eat(), Some(Token::CloseParen)).then_some(value)
                }
                Token::Function(name) => {
                    if !matches!(self.eat(), Some(Token::OpenParen)) { return None; }
                    let first = self.expression()?;
                    let value = match name.to_ascii_lowercase().as_str() {
                        "calc" => first,
                        "min" | "max" | "clamp" => {
                            let mut values = vec![first];
                            while matches!(self.peek(), Some(Token::Comma)) {
                                self.eat(); values.push(self.expression()?);
                            }
                            let numbers = matches!(first, Value::Number(_));
                            let values: Option<Vec<f64>> = values.into_iter().map(|v|match (numbers,v) { (true,Value::Number(n)) | (false,Value::Length(n)) if n.is_finite()=>Some(n),_=>None }).collect();
                            let values = values?;
                            let result = match name.to_ascii_lowercase().as_str() {
                                "min" => values.into_iter().reduce(f64::min)?,
                                "max" => values.into_iter().reduce(f64::max)?,
                                _ if values.len() == 3 => values[0].max(values[1].min(values[2])),
                                _ => return None,
                            };
                            if numbers { Value::Number(result) } else { Value::Length(result) }
                        }
                        _ => return None,
                    };
                    matches!(self.eat(), Some(Token::CloseParen)).then_some(value)
                }
                _ => None,
            }
        }
    }
    let mut parser = Eval { tokens: tokenize(css), pos: 0, parent_px, root_px, angles };
    let value = match (number, parser.atom()?) { (true,Value::Number(n)) if n.is_finite()=>n,(false,v)=>v.length()?,_=>return None };
    parser.skip();
    (parser.pos == parser.tokens.len()).then_some(value)
}

/// A pure number or percentage math value, without a length basis.
pub(crate) fn number_percentage(css: &str, percentage: bool) -> bool {
    let mut parser = Parser { tokens: tokenize(css), pos: 0, allow_size: false };
    let kind = parser.atom();
    parser.skip();
    parser.pos == parser.tokens.len() && (kind == Some(Kind::Number)
        || (percentage && kind == Some(Kind::Percentage)))
}

pub(crate) fn angle(css: &str) -> bool {
    let mut parser = Parser { tokens: tokenize(css), pos: 0, allow_size: false };
    let kind = parser.atom();
    parser.skip();
    parser.pos == parser.tokens.len() && kind == Some(Kind::Angle)
}
