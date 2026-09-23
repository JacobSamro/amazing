use crate::{Error, Flags, MAX_CAPTURES, MAX_DEPTH};

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Empty,
    Char(u32),
    Any,
    Class(Class),
    Set(SetExpr),
    Start,
    End,
    Boundary(bool),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Group(usize, Box<Node>),
    Repeat(Box<Node>, usize, Option<usize>, bool),
    Look(Box<Node>, bool),
    Backref(usize),
}
#[derive(Debug, Clone)]
pub(crate) enum Member {
    Range(u32, u32),
    Digit(bool),
    Word(bool),
    Space(bool),
}
#[derive(Debug, Clone)]
pub(crate) struct Class {
    pub inverted: bool,
    pub members: Vec<Member>,
}
#[derive(Debug, Clone)]
pub(crate) enum SetExpr {
    Char(Member),
    String(Vec<u32>),
    Property(String, bool),
    Union(Vec<SetExpr>),
    Intersect(Vec<SetExpr>),
    Subtract(Box<SetExpr>, Box<SetExpr>),
    Complement(Box<SetExpr>),
}
impl SetExpr {
    fn has_strings(&self) -> bool {
        match self {
            Self::String(s) => s.len() != 1,
            Self::Property(name, _) => crate::unicode_sets::strings(name)
                .iter()
                .any(|s| s.len() != 1),
            Self::Union(terms) | Self::Intersect(terms) => terms.iter().any(Self::has_strings),
            Self::Subtract(left, right) => left.has_strings() || right.has_strings(),
            Self::Complement(term) => term.has_strings(),
            Self::Char(_) => false,
        }
    }
}

pub(crate) fn parse_utf16(pattern: &[u16], flags: Flags) -> Result<(Node, usize), Error> {
    let chars = if flags.unicode_mode() {
        char::decode_utf16(pattern.iter().copied())
            .map(|point| match point {
                Ok(ch) => ch as u32,
                Err(unpaired) => u32::from(unpaired.unpaired_surrogate()),
            })
            .collect()
    } else {
        pattern.iter().copied().map(u32::from).collect()
    };
    let mut p = Parser {
        chars,
        pos: 0,
        flags,
        captures: 0,
        depth: 0,
        refs: Vec::new(),
    };
    let ast = p.disjunction()?;
    if p.pos != p.chars.len() {
        return p.error("unmatched closing parenthesis");
    }
    if p.refs.iter().any(|n| *n > p.captures) {
        return p.error("invalid backreference (legacy decimal/octal escapes are unsupported)");
    }
    Ok((ast, p.captures))
}
struct Parser {
    chars: Vec<u32>,
    pos: usize,
    flags: Flags,
    captures: usize,
    depth: usize,
    refs: Vec<usize>,
}
impl Parser {
    fn peek(&self) -> Option<u32> {
        self.chars.get(self.pos).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c as u32) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn take(&mut self) -> Option<u32> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }
    fn error<T>(&self, message: &str) -> Result<T, Error> {
        Err(Error::Syntax(format!(
            "{message} at pattern offset {}",
            self.pos
        )))
    }
    fn disjunction(&mut self) -> Result<Node, Error> {
        let mut alternatives = vec![self.sequence()?];
        while self.eat('|') {
            alternatives.push(self.sequence()?);
        }
        Ok(if alternatives.len() == 1 {
            alternatives.remove(0)
        } else {
            Node::Alt(alternatives)
        })
    }
    fn sequence(&mut self) -> Result<Node, Error> {
        let mut nodes = Vec::new();
        while self
            .peek()
            .is_some_and(|c| c != ')' as u32 && c != '|' as u32)
        {
            let atom = self.atom()?;
            nodes.push(self.quantifier(atom)?);
        }
        Ok(match nodes.len() {
            0 => Node::Empty,
            1 => nodes.remove(0),
            _ => Node::Seq(nodes),
        })
    }
    fn atom(&mut self) -> Result<Node, Error> {
        let Some(c) = self.take() else {
            return self.error("expected atom");
        };
        match char::from_u32(c) {
            Some('^') => Ok(Node::Start),
            Some('$') => Ok(Node::End),
            Some('.') => Ok(Node::Any),
            Some('[') => self.class(),
            Some('\\') => self.escape(false),
            Some('(') => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(Error::Limit("pattern nesting"));
                }
                let kind = if self.eat('?') {
                    if self.eat(':') {
                        0
                    } else if self.eat('=') {
                        1
                    } else if self.eat('!') {
                        2
                    } else {
                        return self
                            .error("unsupported group (named captures/lookbehind/modifiers)");
                    }
                } else {
                    3
                };
                let group = if kind == 3 {
                    self.captures += 1;
                    self.captures
                } else {
                    0
                };
                if self.captures > MAX_CAPTURES {
                    return Err(Error::Limit("captures"));
                }
                let inner = self.disjunction()?;
                if !self.eat(')') {
                    return self.error("unterminated group");
                }
                self.depth -= 1;
                Ok(match kind {
                    0 => inner,
                    1 => Node::Look(Box::new(inner), true),
                    2 => Node::Look(Box::new(inner), false),
                    _ => Node::Group(group, Box::new(inner)),
                })
            }
            Some('*' | '+' | '?') => self.error("nothing to repeat"),
            Some('{') => {
                if self.flags.unicode_mode() {
                    return self.error("unescaped syntax character");
                }
                let save = self.pos;
                if self.decimal()?.is_some() {
                    if self.eat(',') {
                        self.decimal()?;
                    }
                    if self.eat('}') {
                        return self.error("nothing to repeat");
                    }
                }
                self.pos = save;
                Ok(Node::Char(c))
            }
            Some('}' | ']') if self.flags.unicode_mode() => {
                self.error("unescaped syntax character")
            }
            _ => Ok(Node::Char(c)),
        }
    }
    fn quantifier(&mut self, atom: Node) -> Result<Node, Error> {
        let q = if self.eat('*') {
            Some((0, None))
        } else if self.eat('+') {
            Some((1, None))
        } else if self.eat('?') {
            Some((0, Some(1)))
        } else if self.peek() == Some('{' as u32) {
            let save = self.pos;
            self.pos += 1;
            if let Some(min) = self.decimal()? {
                let max = if self.eat(',') {
                    self.decimal()?
                } else {
                    Some(min)
                };
                if self.eat('}') {
                    if max.is_some_and(|m| m < min) {
                        return self.error("quantifier range out of order");
                    }
                    Some((min, max))
                } else {
                    self.pos = save;
                    None
                }
            } else {
                self.pos = save;
                None
            }
        } else {
            None
        };
        if let Some((min, max)) = q {
            if matches!(
                atom,
                Node::Start | Node::End | Node::Boundary(_) | Node::Look(..)
            ) {
                return self.error("quantified assertions are unsupported");
            }
            let greedy = !self.eat('?');
            Ok(Node::Repeat(Box::new(atom), min, max, greedy))
        } else {
            Ok(atom)
        }
    }
    fn decimal(&mut self) -> Result<Option<usize>, Error> {
        let mut n = 0usize;
        let start = self.pos;
        while let Some(c @ 48..=57) = self.peek() {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add((c - 48) as usize))
                .ok_or(Error::Limit("quantifier integer"))?;
            self.pos += 1;
        }
        Ok((self.pos != start).then_some(n))
    }
    fn class(&mut self) -> Result<Node, Error> {
        if self.flags.unicode_sets {
            return Ok(Node::Set(self.set_class()?));
        }
        let inverted = self.eat('^');
        let mut members = Vec::new();
        while self.peek() != Some(']' as u32) {
            if self.peek().is_none() {
                return self.error("unterminated character class");
            }
            let a = self.class_atom()?;
            if self.peek() == Some('-' as u32)
                && self.chars.get(self.pos + 1) != Some(&(']' as u32))
            {
                self.pos += 1;
                let b = self.class_atom()?;
                match (a, b) {
                    (Member::Range(a, _), Member::Range(b, _)) if a <= b => {
                        members.push(Member::Range(a, b))
                    }
                    _ => return self.error("invalid character class range"),
                }
            } else {
                members.push(a);
            }
        }
        self.pos += 1;
        Ok(Node::Class(Class { inverted, members }))
    }
    fn pair(&self, first: char, second: char) -> bool {
        self.chars.get(self.pos..self.pos + 2) == Some(&[first as u32, second as u32])
    }
    fn set_class(&mut self) -> Result<SetExpr, Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::Limit("pattern nesting"));
        }
        let inverted = self.eat('^');
        let mut terms = Vec::new();
        let mut operator = None;
        while self.peek() != Some(']' as u32) {
            if self.peek().is_none() {
                return self.error("unterminated character class");
            }
            let next = if self.pair('&', '&') {
                Some('&')
            } else if self.pair('-', '-') {
                Some('-')
            } else {
                None
            };
            if let Some(next) = next {
                if terms.is_empty() || operator.is_some_and(|op| op != next) {
                    return self.error("invalid class set operator");
                }
                operator = Some(next);
                self.pos += 2;
                if self.peek() == Some(']' as u32) {
                    return self.error("missing class set operand");
                }
            } else if operator.is_some() && !terms.is_empty() && terms.len() >= 2 {
                return self.error("invalid class set expression");
            }
            let mut term = self.set_operand()?;
            if operator.is_none() && self.peek() == Some('-' as u32) && !self.pair('-', '-') {
                self.pos += 1;
                let end = self.set_character()?;
                term = match (term, end) {
                    (SetExpr::Char(Member::Range(a, _)), SetExpr::Char(Member::Range(b, _)))
                        if a <= b =>
                    {
                        SetExpr::Char(Member::Range(a, b))
                    }
                    _ => return self.error("invalid class set range"),
                };
            }
            terms.push(term);
            if operator.is_some()
                && self.peek() != Some(']' as u32)
                && !self.pair('&', '&')
                && !self.pair('-', '-')
            {
                return self.error("invalid class set expression");
            }
            if operator.is_none()
                && (self.pair('&', '&') || self.pair('-', '-'))
                && terms.len() != 1
            {
                return self.error("class set operator requires one left operand");
            }
        }
        self.pos += 1;
        self.depth -= 1;
        let expr = match operator {
            Some('&') => SetExpr::Intersect(terms),
            Some('-') => {
                let mut terms = terms.into_iter();
                let mut result = terms.next().unwrap();
                for term in terms {
                    result = SetExpr::Subtract(Box::new(result), Box::new(term));
                }
                result
            }
            _ => SetExpr::Union(terms),
        };
        if inverted {
            if expr.has_strings() {
                return self.error("cannot negate a class containing strings");
            }
            Ok(SetExpr::Complement(Box::new(expr)))
        } else {
            Ok(expr)
        }
    }
    fn set_operand(&mut self) -> Result<SetExpr, Error> {
        if self.eat('[') {
            return self.set_class();
        }
        if self.pair('\\', 'q') {
            self.pos += 2;
            if !self.eat('{') {
                return self.error("invalid class string disjunction");
            }
            let mut strings = Vec::new();
            loop {
                let mut string = Vec::new();
                while self.peek() != Some('|' as u32) && self.peek() != Some('}' as u32) {
                    let SetExpr::Char(Member::Range(c, _)) = self.set_character()? else {
                        return self.error("invalid class string character");
                    };
                    string.push(c);
                }
                strings.push(SetExpr::String(string));
                if self.eat('}') {
                    break;
                }
                if !self.eat('|') {
                    return self.error("unterminated class string disjunction");
                }
            }
            return Ok(SetExpr::Union(strings));
        }
        if self.pair('\\', 'p') || self.pair('\\', 'P') {
            self.pos += 1;
            let inverse = self.take() == Some('P' as u32);
            return self.property(inverse);
        }
        if self.peek() == Some('\\' as u32)
            && self
                .chars
                .get(self.pos + 1)
                .is_some_and(|c| "dDsSwW".chars().any(|ch| *c == ch as u32))
        {
            self.pos += 1;
            let Node::Class(class) = self.escape(true)? else {
                unreachable!()
            };
            return Ok(SetExpr::Char(class.members[0].clone()));
        }
        self.set_character()
    }
    fn set_character(&mut self) -> Result<SetExpr, Error> {
        if self.eat('\\') {
            let n = self.escape(true)?;
            return match n {
                Node::Char(c) => Ok(SetExpr::Char(Member::Range(c, c))),
                _ => self.error("invalid class set character"),
            };
        }
        let Some(c) = self.take() else {
            return self.error("unterminated character class");
        };
        if "()[]{}\\/|-".chars().any(|ch| c == ch as u32)
            || "&!#$%*+,.:;<=>?@^`~"
                .chars()
                .any(|ch| c == ch as u32 && self.peek() == Some(c))
        {
            return self.error("unescaped class set character");
        }
        Ok(SetExpr::Char(Member::Range(c, c)))
    }
    fn property(&mut self, inverse: bool) -> Result<SetExpr, Error> {
        if !self.eat('{') {
            return self.error("invalid Unicode property escape");
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c != '}' as u32) {
            self.pos += 1;
        }
        if !self.eat('}') {
            return self.error("unterminated Unicode property escape");
        }
        let name: String = self.chars[start..self.pos - 1]
            .iter()
            .filter_map(|c| char::from_u32(*c))
            .collect();
        if !crate::unicode_sets::supported(&name) {
            return self.error("unsupported Unicode property");
        }
        if !self.flags.unicode_sets
            && crate::unicode_sets::strings(&name)
                .iter()
                .any(|sequence| sequence.len() != 1)
        {
            return self.error("property of strings requires the v flag");
        }
        if inverse
            && crate::unicode_sets::strings(&name)
                .iter()
                .any(|s| s.len() != 1)
        {
            return self.error("cannot negate a property of strings");
        }
        Ok(SetExpr::Property(name, inverse))
    }
    fn class_atom(&mut self) -> Result<Member, Error> {
        let n = if self.eat('\\') {
            self.escape(true)?
        } else {
            Node::Char(
                self.take()
                    .ok_or_else(|| Error::Syntax("unterminated character class".into()))?,
            )
        };
        match n {
            Node::Char(c) => Ok(Member::Range(c, c)),
            Node::Class(mut c) if c.members.len() == 1 => Ok(c.members.remove(0)),
            _ => self.error("invalid class escape"),
        }
    }
    fn hex(&mut self, count: usize) -> Result<u32, Error> {
        let mut n = 0;
        for _ in 0..count {
            let d = self
                .take()
                .and_then(char::from_u32)
                .and_then(|c| c.to_digit(16));
            let Some(d) = d else {
                return self.error("invalid hexadecimal escape");
            };
            n = n * 16 + d;
        }
        Ok(n)
    }
    fn escape(&mut self, in_class: bool) -> Result<Node, Error> {
        let Some(c) = self.take() else {
            return self.error("trailing escape");
        };
        let member = match char::from_u32(c) {
            Some('d') => Some(Member::Digit(false)),
            Some('D') => Some(Member::Digit(true)),
            Some('w') => Some(Member::Word(false)),
            Some('W') => Some(Member::Word(true)),
            Some('s') => Some(Member::Space(false)),
            Some('S') => Some(Member::Space(true)),
            _ => None,
        };
        if let Some(m) = member {
            return Ok(Node::Class(Class {
                inverted: false,
                members: vec![m],
            }));
        }
        match char::from_u32(c) {
            Some('b') if !in_class => Ok(Node::Boundary(true)),
            Some('B') if !in_class => Ok(Node::Boundary(false)),
            Some('b') => Ok(Node::Char(8)),
            Some('n') => Ok(Node::Char(10)),
            Some('r') => Ok(Node::Char(13)),
            Some('t') => Ok(Node::Char(9)),
            Some('v') => Ok(Node::Char(11)),
            Some('f') => Ok(Node::Char(12)),
            Some('0') => {
                if self.peek().is_some_and(|c| (48..=57).contains(&c)) {
                    return self.error("legacy octal escapes are unsupported");
                }
                Ok(Node::Char(0))
            }
            Some('1'..='9') if !in_class => {
                self.pos -= 1;
                let n = self.decimal()?.unwrap();
                self.refs.push(n);
                Ok(Node::Backref(n))
            }
            Some('1'..='9') => self.error("legacy decimal class escapes are unsupported"),
            Some('c') => {
                let c = self.take().unwrap_or(0);
                if !(65..=90).contains(&c) && !(97..=122).contains(&c) {
                    return self.error("invalid control escape");
                }
                Ok(Node::Char(c % 32))
            }
            Some('x') => Ok(Node::Char(self.hex(2)?)),
            Some('u') => {
                let braced = self.flags.unicode_mode() && self.eat('{');
                let mut n = if braced {
                    let mut n = 0u32;
                    let start = self.pos;
                    while self.peek() != Some('}' as u32) {
                        let Some(d) = self
                            .take()
                            .and_then(char::from_u32)
                            .and_then(|c| c.to_digit(16))
                        else {
                            return self.error("invalid Unicode escape");
                        };
                        n = n
                            .checked_mul(16)
                            .and_then(|n| n.checked_add(d))
                            .filter(|n| *n <= 0x10FFFF)
                            .ok_or_else(|| Error::Syntax("Unicode escape out of range".into()))?;
                    }
                    if self.pos == start {
                        return self.error("empty Unicode escape");
                    }
                    self.pos += 1;
                    n
                } else {
                    self.hex(4)?
                };
                if self.flags.unicode_mode()
                    && !braced
                    && (0xD800..=0xDBFF).contains(&n)
                    && self.chars.get(self.pos..self.pos + 2) == Some(&['\\' as u32, 'u' as u32])
                {
                    let save = self.pos;
                    self.pos += 2;
                    if let Ok(low @ 0xDC00..=0xDFFF) = self.hex(4) {
                        n = 0x10000 + ((n - 0xD800) << 10) + (low - 0xDC00);
                    } else {
                        self.pos = save;
                    }
                }
                Ok(Node::Char(n))
            }
            Some('p' | 'P') if self.flags.unicode_mode() => {
                Ok(Node::Set(self.property(c == 'P' as u32)?))
            }
            Some('p' | 'P' | 'k') => {
                self.error("Unicode properties/named references are unsupported")
            }
            Some(ch)
                if self.flags.unicode_mode()
                    && !"^$\\.*+?()[]{}|/".contains(ch)
                    && !(in_class && ch == '-')
                    && !(self.flags.unicode_sets && in_class && "&!#%,:;<=>@`~".contains(ch)) =>
            {
                self.error("invalid identity escape in Unicode pattern")
            }
            _ => Ok(Node::Char(c)),
        }
    }
}
