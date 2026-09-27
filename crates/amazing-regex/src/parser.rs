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
    LookBehind(Box<Node>, bool),
    /// A `(?ims-ims:...)` modifier group: `[i, m, s]` bits to force on, then
    /// `[i, m, s]` bits to force off, for the inner subtree only.
    Mode([bool; 3], [bool; 3], Box<Node>),
    /// One or more capture indices: more than one only when the referenced
    /// name is declared on several mutually-exclusive alternatives, in which
    /// case exactly one of them can be bound (Some) at match time.
    Backref(Vec<usize>),
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

/// Group names in declaration order with their 1-based capture index; empty
/// when the pattern has no named capturing groups.
pub(crate) type GroupNames = Vec<(Box<str>, usize)>;

pub(crate) fn parse_utf16(
    pattern: &[u16],
    flags: Flags,
) -> Result<(Node, usize, GroupNames), Error> {
    let chars: Vec<u32> = if flags.unicode_mode() {
        char::decode_utf16(pattern.iter().copied())
            .map(|point| match point {
                Ok(ch) => ch as u32,
                Err(unpaired) => u32::from(unpaired.unpaired_surrogate()),
            })
            .collect()
    } else {
        pattern.iter().copied().map(u32::from).collect()
    };
    // DecimalEscape's backref-vs-octal disambiguation needs every capturing
    // parenthesis up front (see `count_captures`/`numeric_escape`), same as
    // `\k<name>` needs every declared name up front for a forward reference:
    // both are collected before the real parse, which resolves each
    // reference against the now-complete set.
    let total_captures = count_captures(&chars, flags);
    // `\k<name>` may reference a group declared later in the pattern, so the
    // set of names is collected with a throwaway pre-pass before the real
    // parse resolves each reference to its (stable, deterministic) index.
    let known_names = {
        let mut scan = Parser {
            chars: chars.clone(),
            total_captures,
            pos: 0,
            flags,
            captures: 0,
            depth: 0,
            refs: Vec::new(),
            scanning: true,
            names: Vec::new(),
            known_names: Vec::new(),
            disjunction_seq: 0,
            path: Vec::new(),
            name_paths: Vec::new(),
        };
        let _ = scan.disjunction()?;
        if scan.pos != scan.chars.len() {
            return scan.error("unmatched closing parenthesis");
        }
        scan.names
    };
    let mut p = Parser {
        chars,
        total_captures,
        pos: 0,
        flags,
        captures: 0,
        depth: 0,
        refs: Vec::new(),
        scanning: false,
        names: Vec::new(),
        known_names,
        disjunction_seq: 0,
        path: Vec::new(),
        name_paths: Vec::new(),
    };
    let ast = p.disjunction()?;
    if p.pos != p.chars.len() {
        return p.error("unmatched closing parenthesis");
    }
    if p.refs.iter().any(|n| *n > p.captures) {
        return p.error("invalid backreference");
    }
    // Two `(?<name>...)` declarations may share a name only if some ancestor
    // disjunction they are both nested in sent them down different
    // alternatives (they can never both be captured by the same match).
    for (i, (name_a, path_a)) in p.name_paths.iter().enumerate() {
        for (name_b, path_b) in &p.name_paths[i + 1..] {
            if name_a == name_b && !Parser::paths_diverge(path_a, path_b) {
                return p.error("duplicate capture group name");
            }
        }
    }
    Ok((ast, p.captures, p.names))
}
// DecimalEscape depends on every capturing parenthesis, including later groups.
// Escaped delimiters and brackets within classes do not introduce captures.
fn count_captures(chars: &[u32], flags: Flags) -> usize {
    let mut count = 0;
    let mut class_depth = 0usize;
    let mut pos = 0;
    while pos < chars.len() {
        match chars[pos] {
            92 => { pos += 2; continue; }
            91 if class_depth == 0 || flags.unicode_sets => class_depth += 1,
            93 if class_depth > 0 => class_depth -= 1,
            40 if class_depth == 0 => {
                let special = chars.get(pos + 1) == Some(&63);
                let named = chars.get(pos + 2) == Some(&60)
                    && !matches!(chars.get(pos + 3), Some(61 | 33));
                if !special || named { count += 1; }
            }
            _ => {}
        }
        pos += 1;
    }
    count
}

struct Parser {
    chars: Vec<u32>,
    pos: usize,
    flags: Flags,
    captures: usize,
    total_captures: usize,
    depth: usize,
    refs: Vec<usize>,
    /// True only for the throwaway pre-pass: `\k<name>` accepts any
    /// well-formed name without checking it is declared, since the point of
    /// this pass is to discover every name.
    scanning: bool,
    /// Names declared by `(?<name>...)` so far in this pass.
    names: GroupNames,
    /// Names available to `\k<name>` (from the pre-pass); empty while
    /// `scanning`.
    known_names: GroupNames,
    /// Next fresh id handed to each `disjunction()` call, for `path`.
    disjunction_seq: usize,
    /// The chain of (disjunction id, chosen alternative index) ancestors at
    /// the current parse position, used only to validate duplicate names.
    path: Vec<(usize, usize)>,
    /// Every declared name's ancestor `path` at the point it was declared.
    name_paths: Vec<(Box<str>, Vec<(usize, usize)>)>,
}
impl Parser {
    /// True unless every disjunction shared by both paths chose the same
    /// alternative (in which case the two occurrences can both be live in
    /// the same match, so a shared name between them is a real conflict).
    fn paths_diverge(a: &[(usize, usize)], b: &[(usize, usize)]) -> bool {
        let by_id: std::collections::HashMap<usize, usize> = a.iter().copied().collect();
        b.iter()
            .any(|(id, index)| by_id.get(id).is_some_and(|other| other != index))
    }
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
        let id = self.disjunction_seq;
        self.disjunction_seq += 1;
        self.path.push((id, 0));
        let mut alternatives = vec![self.sequence()?];
        while self.eat('|') {
            self.path.last_mut().unwrap().1 += 1;
            alternatives.push(self.sequence()?);
        }
        self.path.pop();
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
                // 0 = non-capturing, 1 = lookahead, 2 = negative lookahead,
                // 3 = capturing (name, if any, in `name`), 4 = lookbehind,
                // 5 = negative lookbehind, 6 = modifier group (add/remove in
                // `mode`).
                let mut name = None;
                let mut mode = None;
                let kind = if self.eat('?') {
                    if self.eat(':') {
                        0
                    } else if self.eat('=') {
                        1
                    } else if self.eat('!') {
                        2
                    } else if self.eat('<') {
                        if self.eat('=') {
                            4
                        } else if self.eat('!') {
                            5
                        } else {
                            name = Some(self.group_name()?);
                            if !self.eat('>') {
                                return self.error("unterminated group name");
                            }
                            3
                        }
                    } else if matches!(
                        self.peek().and_then(char::from_u32),
                        Some('i' | 'm' | 's' | '-')
                    ) {
                        mode = Some(self.modifiers()?);
                        if !self.eat(':') {
                            return self.error("expected ':' after RegExp modifiers");
                        }
                        6
                    } else {
                        return self.error("unsupported group modifier");
                    }
                } else {
                    3
                };
                let group = if kind == 3 {
                    self.captures += 1;
                    if let Some(name) = &name {
                        self.names.push((name.clone(), self.captures));
                        self.name_paths.push((name.clone(), self.path.clone()));
                    }
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
                    0 => Node::Seq(vec![inner]),
                    1 => Node::Look(Box::new(inner), true),
                    2 => Node::Look(Box::new(inner), false),
                    4 => Node::LookBehind(Box::new(inner), true),
                    5 => Node::LookBehind(Box::new(inner), false),
                    6 => {
                        let (add, remove) = mode.unwrap();
                        Node::Mode(add, remove, Box::new(inner))
                    }
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
                Node::Start | Node::End | Node::Boundary(_) | Node::LookBehind(..)
            ) || (self.flags.unicode_mode() && matches!(atom, Node::Look(..))) {
                // Per Annex B.1.2's extended quantifier grammar, a lookahead
                // assertion may be quantified outside Unicode mode (e.g.
                // `/(?=a)+/`); lookbehind never gained an equivalent
                // extension and stays rejected in both modes, and `^`/`$`/`\b`
                // are never quantifiable in either mode.
                return self.error("invalid quantified assertion");
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
                    (a, b) if !self.flags.unicode_mode()
                        && (!matches!(a, Member::Range(..)) || !matches!(b, Member::Range(..))) => {
                        members.extend([a, Member::Range(45, 45), b]);
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
        if !(crate::unicode_sets::supported(&name) || crate::unicode_props::supported(&name)) {
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
    /// `(?ims-ims:...)` modifiers: one or more of `i`/`m`/`s` to force on,
    /// optionally followed by `-` and one or more of `i`/`m`/`s` to force
    /// off. Each of the three flags may appear at most once across both
    /// sets, and at least one flag must be given somewhere.
    fn modifiers(&mut self) -> Result<([bool; 3], [bool; 3]), Error> {
        let mut add = [false; 3];
        let mut remove = [false; 3];
        loop {
            let slot = match self.peek().and_then(char::from_u32) {
                Some('i') => &mut add[0],
                Some('m') => &mut add[1],
                Some('s') => &mut add[2],
                _ => break,
            };
            if *slot {
                return self.error("duplicate RegExp modifier");
            }
            *slot = true;
            self.pos += 1;
        }
        if self.eat('-') {
            // The remove side may be empty (e.g. `(?s-:)`, which adds `s`
            // and removes nothing); only both sides empty is rejected below.
            loop {
                let (slot, added) = match self.peek().and_then(char::from_u32) {
                    Some('i') => (&mut remove[0], add[0]),
                    Some('m') => (&mut remove[1], add[1]),
                    Some('s') => (&mut remove[2], add[2]),
                    _ => break,
                };
                if *slot || added {
                    return self.error("duplicate RegExp modifier");
                }
                *slot = true;
                self.pos += 1;
            }
        }
        if !(add[0] || add[1] || add[2] || remove[0] || remove[1] || remove[2]) {
            return self.error("expected a RegExp modifier");
        }
        Ok((add, remove))
    }
    /// GroupName: an IdentifierStart (or `\u` escape resolving to one)
    /// followed by IdentifierPart* (`$`, `_`, ZWNJ/ZWJ, or Unicode
    /// ID_Start/ID_Continue), up to (not including) the closing `>`.
    fn group_name(&mut self) -> Result<Box<str>, Error> {
        let mut name = String::new();
        loop {
            let c = if self.eat('\\') {
                if !self.eat('u') {
                    return self.error("invalid group name escape");
                }
                self.identifier_escape()?
            } else {
                match self.peek() {
                    Some(c) if c != '>' as u32 => {
                        self.pos += 1;
                        c
                    }
                    _ => break,
                }
            };
            let start = name.is_empty();
            let ok = c == '$' as u32
                || c == '_' as u32
                || (!start && (c == 0x200C || c == 0x200D))
                || crate::unicode_props::contains(if start { "ID_Start" } else { "ID_Continue" }, c);
            if !ok {
                return self.error("invalid character in group name");
            }
            match char::from_u32(c) {
                Some(ch) => name.push(ch),
                None => return self.error("invalid character in group name"),
            }
        }
        if name.is_empty() {
            return self.error("empty group name");
        }
        Ok(name.into())
    }
    /// A RegExpUnicodeEscapeSequence for GroupName: unlike the general `\u`
    /// character escape, the braced form and surrogate-pair combining are
    /// always available here regardless of the `u`/`v` flags.
    fn identifier_escape(&mut self) -> Result<u32, Error> {
        if self.eat('{') {
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
            Ok(n)
        } else {
            let mut n = self.hex(4)?;
            if (0xD800..=0xDBFF).contains(&n)
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
            Ok(n)
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
    fn hex_or_identity(&mut self, count: usize, character: u32) -> Result<Node, Error> {
        let saved = self.pos;
        match self.hex(count) {
            Ok(value) => Ok(Node::Char(value)),
            Err(_) if !self.flags.unicode_mode() => {
                self.pos = saved;
                Ok(Node::Char(character))
            }
            Err(error) => Err(error),
        }
    }

    fn numeric_escape(&mut self, first: u32, in_class: bool) -> Result<Node, Error> {
        if first != 48 && !in_class {
            let saved = self.pos;
            let mut number = (first - 48) as usize;
            while let Some(digit @ 48..=57) = self.peek() {
                number = number.saturating_mul(10).saturating_add((digit - 48) as usize);
                self.pos += 1;
            }
            if number <= self.total_captures {
                self.refs.push(number);
                return Ok(Node::Backref(vec![number]));
            }
            if self.flags.unicode_mode() { return self.error("invalid backreference"); }
            self.pos = saved;
        }
        if self.flags.unicode_mode() {
            return if first == 48 && !self.peek().is_some_and(|c| (48..=57).contains(&c)) {
                Ok(Node::Char(0))
            } else { self.error("invalid decimal character escape") };
        }
        if first >= 56 { return Ok(Node::Char(first)); }
        let mut value = first - 48;
        let additional = if first <= 51 { 2 } else { 1 };
        for _ in 0..additional {
            let Some(digit @ 48..=55) = self.peek() else { break };
            self.pos += 1;
            value = value * 8 + digit - 48;
        }
        Ok(Node::Char(value))
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
            Some('0'..='9') => self.numeric_escape(c, in_class),
            Some('c') => {
                if let Some(next) = self.peek() {
                    if (65..=90).contains(&next) || (97..=122).contains(&next)
                        || (!self.flags.unicode_mode() && in_class
                            && ((48..=57).contains(&next) || next == 95)) {
                        self.pos += 1;
                        return Ok(Node::Char(next % 32));
                    }
                }
                if !self.flags.unicode_mode() {
                    // Annex B's '\\ [lookahead = c]' consumes only the slash.
                    self.pos -= 1;
                    Ok(Node::Char(92))
                } else { self.error("invalid control escape") }
            }
            Some('x') => self.hex_or_identity(2, c),
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
                    let saved = self.pos;
                    match self.hex(4) {
                        Ok(value) => value,
                        Err(_) if !self.flags.unicode_mode() => {
                            self.pos = saved;
                            return Ok(Node::Char(c));
                        }
                        Err(error) => return Err(error),
                    }
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
            // Outside unicode mode, `\p`/`\P` are not property-escape syntax
            // at all (that's `[+UnicodeMode]`-only per spec) and fall through
            // to the plain-identity-escape catch-all below, same as any other
            // unrecognized letter -- matching Annex B's permissive
            // SourceCharacterIdentityEscape and this crate's own
            // `annex_b_identity_control_and_class_range_fallbacks` test.
            // Outside unicode mode, `\k<...>` is only GroupName syntax when
            // the pattern has a named group somewhere (Annex B); otherwise
            // it is a plain identity escape for `k`, and `<name>` that
            // follows is ordinary literal text. The scanning pre-pass
            // always takes this branch since it doesn't yet know the
            // pattern-wide answer, but that's harmless for name discovery.
            Some('k')
                if !in_class
                    && self.peek() == Some('<' as u32)
                    && (self.scanning
                        || self.flags.unicode_mode()
                        || !self.known_names.is_empty()) =>
            {
                self.pos += 1;
                let name = self.group_name()?;
                if !self.eat('>') {
                    return self.error("unterminated group name");
                }
                // A name may be declared on several mutually-exclusive
                // alternatives (checked at the end of `parse_utf16`); collect
                // every capture index it maps to so the backref can pick out
                // whichever one is actually bound at match time.
                let indices: Vec<usize> = self
                    .known_names
                    .iter()
                    .filter(|(n, _)| **n == *name)
                    .map(|(_, i)| *i)
                    .collect();
                if !self.scanning && indices.is_empty() {
                    return self.error("undefined named backreference");
                }
                self.refs.extend(indices.iter().copied());
                Ok(Node::Backref(if indices.is_empty() {
                    vec![0]
                } else {
                    indices
                }))
            }
            // A bare `\k` not followed by `<` is a plain identity escape for
            // `k` -- unless the pattern (as a whole; `known_names` reflects
            // the completed pre-pass, not just what's been seen so far) has
            // a named group anywhere, in which case `\k` always introduces a
            // NamedBackreference and a missing `<name>` is a Syntax Error
            // (this holds even in Annex B / non-unicode mode, and even when
            // the `\k` appears before the group it doesn't name). Checked
            // only in the real pass: the scanning pre-pass must not error
            // here, since its whole job is to discover `known_names` first.
            Some('k') if !in_class && !self.scanning && !self.known_names.is_empty() => {
                self.error("\\k must be followed by a named backreference in a pattern with a named group")
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
