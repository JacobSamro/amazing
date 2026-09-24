//! Selector parsing, matching, and specificity (css-selectors-4).
//!
//! Phase 2 subset — implemented:
//! - complex selectors: compound sequences joined by descendant / child
//!   (`>`) / next-sibling (`+`) / later-sibling (`~`) combinators
//! - simple selectors: type, universal, `#id`, `.class`, attribute tests
//!   (`[a]`, `=`, `~=`, `|=`, `^=`, `$=`, `*=`, plus the `i` case flag),
//!   pseudo-classes (structural set incl. full `an+b` forms, `:root`,
//!   `:empty`, link state, and the never-matching interactive set), `:not()`,
//!   `:is()`/`:matches()`, `:where()`
//! - specificity per css-selectors-4 §6.4: ids in slot A, classes/
//!   attributes/pseudo-classes in B, types/pseudo-elements in C; `:not()` /
//!   `:is()` contribute their most specific argument, `:where()` contributes
//!   zero
//!
//! Deferred/deviations:
//! - namespace prefixes (`svg|rect`) are not supported
//! - `:has()` currently accepts single-compound relative arguments; complex
//!   relative selectors remain unsupported

use crate::syntax::{Token, serialize, tokenize};
use amazing_dom::{Dom, Namespace, NodeIdx};
use std::fmt;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// Combinator joining two adjacent compounds of a complex selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

/// One complex selector. `compounds` is leftmost-first;
/// `combinators[i]` joins `compounds[i]` to `compounds[i + 1]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound {
    pub simple: Vec<Simple>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Simple {
    /// Element type selector (lowercased); compared case-insensitively
    /// since the HTML tree builder lowercases element names anyway.
    Type(String),
    Universal,
    Id(String),
    Class(String),
    Attr {
        name: String,
        op: AttrOp,
        value: String,
        /// selectors-4 case-insensitivity flag (`[x=y i]`).
        ci: bool,
    },
    Pseudo(Pseudo),
    PseudoElement(PseudoElement),
    /// `:not(compound, …)` — never matches if any argument matches.
    Negation(Vec<Compound>),
    /// `:is(compound, …)` / legacy alias `:matches(…)` — matches if any
    /// argument does.
    Matches(Vec<Compound>),
    /// `:where(compound, …)` — same matching as [`Simple::Matches`], zero
    /// specificity.
    Where(Vec<Compound>),
    /// Complex selector-list arguments, including combinators.
    ComplexNegation(Vec<Selector>),
    ComplexMatches(Vec<Selector>),
    ComplexWhere(Vec<Selector>),
    /// Relative one-step selectors, with `>` for child and whitespace for descendant.
    Has(Vec<(Combinator, Compound)>),
    Lang(Vec<String>),
    Dir(bool),
    NthChildOf {
        a: i32,
        b: i32,
        from_last: bool,
        selectors: Vec<Selector>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
    Marker,
    Placeholder,
    Selection,
    FirstLine,
    FirstLetter,
    Backdrop,
    FileSelectorButton,
}

impl PseudoElement {
    pub const STYLED: [Self; 5] = [
        Self::Before,
        Self::After,
        Self::Marker,
        Self::FirstLine,
        Self::FirstLetter,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Equals,
    /// Whitespace-separated word list contains value.
    Includes,
    /// Dash-separated language-style prefix: equals or starts `value-`.
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pseudo {
    Root,
    Scope,
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    Empty,
    /// `nth-child(an+b)` with parsed coefficients.
    NthChild {
        a: i32,
        b: i32,
    },
    NthLastChild {
        a: i32,
        b: i32,
    },
    NthOfType { a: i32, b: i32 },
    NthLastOfType { a: i32, b: i32 },
    /// Matches `a`/`area` elements with an `href` attribute (unvisited).
    Link,
    /// Privacy: never matches anything in this engine.
    Visited,
    /// Interactive states have no input in a static render: never match.
    Hover,
    Active,
    Focus,
    FocusWithin,
    Enabled,
    Disabled,
    Checked,
    Required,
    Optional,
    Open,
}

// ---------------------------------------------------------------------------
// Specificity
// ---------------------------------------------------------------------------

/// Specificity triple (A=id count, B=class/attr/pseudo-class count,
/// C=type/pseudo-element count), ordered for comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity {
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

impl fmt::Display for Specificity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.a, self.b, self.c)
    }
}

impl std::ops::Add for Specificity {
    type Output = Specificity;

    fn add(self, other: Self) -> Self {
        Specificity {
            a: self.a + other.a,
            b: self.b + other.b,
            c: self.c + other.c,
        }
    }
}

impl Selector {
    /// Specificity of the whole complex selector (css-selectors-4 §6.4):
    /// sum over compounds; functional pseudos contribute their most
    /// specific argument (`:where()` always zero).
    pub fn pseudo_element(&self) -> Option<PseudoElement> {
        self.compounds
            .last()?
            .simple
            .iter()
            .rev()
            .find_map(|simple| match simple {
                Simple::PseudoElement(kind) => Some(*kind),
                _ => None,
            })
    }

    pub fn specificity(&self) -> Specificity {
        let mut total = Specificity::default();
        for c in &self.compounds {
            total = total + compound_specificity(c);
        }
        total
    }
}

fn compound_contains_pseudo_element(compound: &Compound) -> bool {
    compound.simple.iter().any(|simple| match simple {
        Simple::PseudoElement(_) => true,
        Simple::Negation(args) | Simple::Matches(args) | Simple::Where(args) => {
            args.iter().any(compound_contains_pseudo_element)
        }
        Simple::ComplexNegation(args) | Simple::ComplexMatches(args) | Simple::ComplexWhere(args) => {
            args.iter().any(|selector| selector.compounds.iter().any(compound_contains_pseudo_element))
        }
        Simple::Has(args) => args
            .iter()
            .any(|(_, compound)| compound_contains_pseudo_element(compound)),
        Simple::NthChildOf { selectors, .. } => selectors.iter().any(|selector| {
            selector
                .compounds
                .iter()
                .any(compound_contains_pseudo_element)
        }),
        _ => false,
    })
}

fn compound_specificity(c: &Compound) -> Specificity {
    let mut spec = Specificity::default();
    for s in &c.simple {
        spec = spec + simple_specificity(s);
    }
    spec
}

fn simple_specificity(s: &Simple) -> Specificity {
    let mut spec = Specificity::default();
    match s {
        Simple::Id(_) => spec.a += 1,
        Simple::Class(_) | Simple::Attr { .. } | Simple::Pseudo(_) => spec.b += 1,
        Simple::Type(_) | Simple::PseudoElement(_) => spec.c += 1,
        Simple::Negation(args) | Simple::Matches(args) => {
            // Most specific argument (compounds sum internally); an empty
            // list counts as zero.
            let max = args
                .iter()
                .map(compound_specificity)
                .max()
                .unwrap_or_default();
            spec = spec + max;
        }
        Simple::ComplexNegation(args) | Simple::ComplexMatches(args) => {
            spec = spec + args.iter().map(Selector::specificity).max().unwrap_or_default();
        }
        Simple::Has(args) => {
            spec = spec
                + args
                    .iter()
                    .map(|(_, c)| compound_specificity(c))
                    .max()
                    .unwrap_or_default();
        }
        Simple::NthChildOf { selectors, .. } => {
            spec.b += 1;
            spec = spec
                + selectors
                    .iter()
                    .map(Selector::specificity)
                    .max()
                    .unwrap_or_default();
        }
        Simple::Lang(_) | Simple::Dir(_) => spec.b += 1,
        Simple::Universal | Simple::Where(_) | Simple::ComplexWhere(_) => {}
    }
    spec
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses one complex selector (already comma-split by the rule parser).
/// Returns `None` on any construct outside the implemented subset —
/// callers drop such selectors (and per spec, any rule containing one).
pub fn parse_selector(src: &str) -> Option<Selector> {
    let toks = tokenize(src);
    let mut p = SelParser {
        toks: &toks,
        pos: 0,
        failed: false,
    };
    p.skip_ws();
    let sel = p.complex()?;
    p.skip_ws();
    if p.failed || p.pos != p.toks.len() {
        return None; // trailing junk
    }
    let rightmost = sel.compounds.len() - 1;
    for (index, compound) in sel.compounds.iter().enumerate() {
        let mut pseudo = None;
        for simple in &compound.simple {
            if let Simple::PseudoElement(kind) = simple {
                if index != rightmost {
                    return None;
                }
                if pseudo.is_some_and(|previous| {
                    !matches!(previous, PseudoElement::Before | PseudoElement::After)
                        || *kind != PseudoElement::Marker
                }) {
                    return None;
                }
                pseudo = Some(*kind);
            } else if pseudo.is_some()
                && !matches!(
                    simple,
                    Simple::Pseudo(
                        Pseudo::Hover | Pseudo::Active | Pseudo::Focus | Pseudo::FocusWithin
                    ) | Simple::Matches(_)
                        | Simple::Where(_)
                        | Simple::Negation(_)
                )
            {
                return None;
            }
        }
    }
    Some(sel)
}

struct SelParser<'t> {
    toks: &'t [Token],
    pos: usize,
    failed: bool,
}

impl<'t> SelParser<'t> {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.peek()?.clone();
        self.pos += 1;
        Some(t)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
    }

    /// complex := compound (combinator? compound)*
    fn complex(&mut self) -> Option<Selector> {
        let mut compounds = vec![self.compound()?];
        let mut combinators = Vec::new();
        loop {
            // Detect what follows: EOF/comma ends the selector; whitespace is
            // padding that may hide a descendant combinator or precede an
            // explicit one.
            let mut saw_ws = false;
            while matches!(self.peek(), Some(Token::Whitespace)) {
                saw_ws = true;
                self.pos += 1;
            }
            let comb = match self.peek() {
                None | Some(Token::Comma) => break,
                Some(Token::Delim('>')) => {
                    self.pos += 1;
                    Combinator::Child
                }
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    Combinator::NextSibling
                }
                Some(Token::Delim('~')) => {
                    self.pos += 1;
                    Combinator::LaterSibling
                }
                _ if saw_ws => Combinator::Descendant,
                _ => return None, // two compounds butted together without ws
            };
            self.skip_ws();
            let next = self.compound()?;
            combinators.push(comb);
            compounds.push(next);
        }
        Some(Selector {
            compounds,
            combinators,
        })
    }

    /// compound := simple+
    fn compound(&mut self) -> Option<Compound> {
        let first = self.simple()?;
        Some(self.compound_tail(first))
    }

    /// Parses the next simple selector if one begins at the cursor.
    fn try_simple(&mut self) -> Option<Simple> {
        match self.peek()? {
            Token::Ident(_)
            | Token::Hash(_)
            | Token::Delim('*' | '.')
            | Token::OpenSquare
            | Token::Colon => {
                let parsed = self.simple();
                if parsed.is_none() {
                    self.failed = true;
                }
                parsed
            }
            _ => None,
        }
    }

    fn simple(&mut self) -> Option<Simple> {
        match self.bump()? {
            Token::Ident(name) => Some(Simple::Type(name)),
            Token::Delim('*') => Some(Simple::Universal),
            Token::Hash(id) => Some(Simple::Id(id)),
            Token::Delim('.') => match self.bump()? {
                Token::Ident(name) => Some(Simple::Class(name)),
                _ => None,
            },
            Token::OpenSquare => self.attribute(),
            Token::Colon => self.pseudo(),
            _ => None,
        }
    }

    /// Like [`compound`](Self::compound) but reuses an already-parsed first
    /// simple selector (used inside functional argument lists).
    fn compound_tail(&mut self, first: Simple) -> Compound {
        let mut simple = vec![first];
        while let Some(s) = self.try_simple() {
            simple.push(s);
        }
        Compound { simple }
    }

    /// `[name]`, `[name=value]`, `[name~=value]`, … with optional trailing
    /// `i`/`s` flag.
    fn attribute(&mut self) -> Option<Simple> {
        self.skip_ws();
        // `[|name]` explicitly selects a no-namespace attribute. Whitespace
        // between the namespace separator and name is invalid grammar.
        if matches!(self.peek(), Some(Token::Delim('|'))) {
            self.pos += 1;
            if !matches!(self.peek(), Some(Token::Ident(_))) { return None; }
        }
        let name = match self.bump()? {
            Token::Ident(n) => n.to_ascii_lowercase(),
            _ => return None,
        };
        self.skip_ws();
        // Match on an owned token so the cursor can advance inside arms
        // (Token borrows from the stream would outlive the mutation).
        let op = match self.peek().cloned() {
            Some(Token::CloseSquare) => {
                self.pos += 1;
                return Some(Simple::Attr {
                    name,
                    op: AttrOp::Exists,
                    value: String::new(),
                    ci: false,
                });
            }
            Some(Token::Delim('=')) => {
                self.pos += 1;
                AttrOp::Equals
            }
            Some(Token::Delim(c @ ('~' | '^' | '$' | '*' | '|'))) => {
                self.pos += 1;
                if self.bump()? != Token::Delim('=') {
                    return None;
                }
                match c {
                    '~' => AttrOp::Includes,
                    '^' => AttrOp::Prefix,
                    '$' => AttrOp::Suffix,
                    '*' => AttrOp::Substring,
                    _ => AttrOp::DashMatch,
                }
            }
            _ => return None,
        };
        self.skip_ws();
        let value = match self.bump()? {
            Token::Str(s) => s,
            Token::Ident(i) => i,
            _ => return None,
        };
        self.skip_ws();
        // Optional case-sensitivity flag. Classified through an owned token
        // so the cursor can advance without holding a borrow of the stream.
        let mut ci = false;
        let flag_ci = match self.peek().cloned() {
            Some(Token::Ident(f)) if f.eq_ignore_ascii_case("i") || f.eq_ignore_ascii_case("s") => {
                Some(f.eq_ignore_ascii_case("i"))
            }
            _ => None,
        };
        if let Some(is_i) = flag_ci {
            ci = is_i;
            self.pos += 1;
            self.skip_ws();
        }
        if self.bump()? != Token::CloseSquare {
            return None;
        }
        Some(Simple::Attr {
            name,
            op,
            value,
            ci,
        })
    }

    fn pseudo(&mut self) -> Option<Simple> {
        let double_colon = matches!(self.peek(), Some(Token::Colon));
        if double_colon {
            self.pos += 1;
        }
        let name = match self.bump()? {
            Token::Ident(n) => n,
            Token::Function(f) => {
                // Functional pseudo-classes; contents parsed below.
                if double_colon {
                    return None;
                }
                return self.functional_args(f.to_ascii_lowercase());
            }
            _ => return None,
        };
        let lower = name.to_ascii_lowercase();
        let element = match lower.as_str() {
            "before" => Some(PseudoElement::Before),
            "after" => Some(PseudoElement::After),
            "marker" => Some(PseudoElement::Marker),
            "placeholder" => Some(PseudoElement::Placeholder),
            "selection" => Some(PseudoElement::Selection),
            "first-line" => Some(PseudoElement::FirstLine),
            "first-letter" => Some(PseudoElement::FirstLetter),
            "backdrop" => Some(PseudoElement::Backdrop),
            "file-selector-button" => Some(PseudoElement::FileSelectorButton),
            _ => None,
        };
        if let Some(element) = element {
            if double_colon
                || matches!(
                    element,
                    PseudoElement::Before
                        | PseudoElement::After
                        | PseudoElement::FirstLine
                        | PseudoElement::FirstLetter
                )
            {
                return Some(Simple::PseudoElement(element));
            }
            return None;
        }
        if double_colon {
            return None;
        }
        let pseudo = match lower.as_str() {
            "root" => Pseudo::Root,
            "scope" => Pseudo::Scope,
            "first-child" => Pseudo::FirstChild,
            "last-child" => Pseudo::LastChild,
            "only-child" => Pseudo::OnlyChild,
            "first-of-type" => Pseudo::FirstOfType,
            "last-of-type" => Pseudo::LastOfType,
            "only-of-type" => Pseudo::OnlyOfType,
            "empty" => Pseudo::Empty,
            "link" | "any-link" => Pseudo::Link,
            "visited" => Pseudo::Visited,
            "hover" => Pseudo::Hover,
            "active" => Pseudo::Active,
            "focus" => Pseudo::Focus,
            "focus-within" => Pseudo::FocusWithin,
            "enabled" => Pseudo::Enabled,
            "disabled" => Pseudo::Disabled,
            "checked" => Pseudo::Checked,
            "required" => Pseudo::Required,
            "optional" => Pseudo::Optional,
            "open" => Pseudo::Open,
            _ => return None, // unknown pseudo-class invalidates the selector
        };
        Some(Simple::Pseudo(pseudo))
    }

    /// Parses `(selector-list)` after `:not(` / `:is(` / `:matches(` /
    /// `:where(`, and `(an+b)` after `:nth-child(`/`:nth-last-child(`.
    fn functional_args(&mut self, name: String) -> Option<Simple> {
        debug_assert!(matches!(self.peek(), Some(Token::OpenParen)));
        // Function tokens are always followed by an explicit OpenParen
        // token; every functional form consumes it before its arguments.
        self.pos += 1;
        match name.as_str() {
            "nth-child" => self.nth_args(false),
            "nth-last-child" => self.nth_args(true),
            "nth-of-type" => self.anb().map(|(a, b)| Simple::Pseudo(Pseudo::NthOfType { a, b })),
            "nth-last-of-type" => self.anb().map(|(a, b)| Simple::Pseudo(Pseudo::NthLastOfType { a, b })),
            "lang" => {
                let mut ranges = Vec::new();
                loop {
                    self.skip_ws();
                    let range = match self.bump()? {
                        Token::Ident(value) | Token::Str(value) => value,
                        _ => return None,
                    };
                    ranges.push(range);
                    self.skip_ws();
                    match self.bump()? {
                        Token::Comma => continue,
                        Token::CloseParen => break,
                        _ => return None,
                    }
                }
                Some(Simple::Lang(ranges))
            }
            "dir" => {
                self.skip_ws();
                let direction = match self.bump()? {
                    Token::Ident(value) if value.eq_ignore_ascii_case("ltr") => false,
                    Token::Ident(value) if value.eq_ignore_ascii_case("rtl") => true,
                    _ => return None,
                };
                self.skip_ws();
                (self.bump()? == Token::CloseParen).then_some(Simple::Dir(direction))
            }
            "has" => {
                let mut args = Vec::new();
                loop {
                    self.skip_ws();
                    let relation = if matches!(self.peek(), Some(Token::Delim('>'))) {
                        self.pos += 1;
                        self.skip_ws();
                        Combinator::Child
                    } else {
                        Combinator::Descendant
                    };
                    let compound = self.compound()?;
                    if compound_contains_pseudo_element(&compound) {
                        return None;
                    }
                    args.push((relation, compound));
                    self.skip_ws();
                    match self.bump()? {
                        Token::Comma => continue,
                        Token::CloseParen => break,
                        _ => return None,
                    }
                }
                Some(Simple::Has(args))
            }
            "not" | "is" | "matches" | "where" => {
                let forgiving = name != "not";
                let mut selectors = self.selector_list_until_close(forgiving)?;
                if !forgiving && selectors.iter().any(|selector| selector.compounds.iter().any(compound_contains_pseudo_element)) {
                    return None;
                }
                selectors.retain(|selector| !selector.compounds.iter().any(compound_contains_pseudo_element));
                if selectors.is_empty() {
                    return match name.as_str() {
                        "where" => Some(Simple::Where(Vec::new())),
                        "not" => None,
                        _ => Some(Simple::Matches(Vec::new())),
                    };
                }
                if selectors.iter().all(|selector| selector.compounds.len() == 1) {
                    let args: Vec<_> = selectors.into_iter().map(|selector| selector.compounds.into_iter().next().unwrap()).collect();
                    return match name.as_str() {
                        "not" => Some(Simple::Negation(args)),
                        "where" => Some(Simple::Where(args)),
                        _ => Some(Simple::Matches(args)),
                    };
                }
                match name.as_str() {
                    "not" => Some(Simple::ComplexNegation(selectors)),
                    "where" => Some(Simple::ComplexWhere(selectors)),
                    _ => Some(Simple::ComplexMatches(selectors)),
                }
            }
            _ => None,
        }
    }

    fn nth_args(&mut self, from_last: bool) -> Option<Simple> {
        let mut depth = 0usize;
        let mut of = None;
        for index in self.pos..self.toks.len() {
            match &self.toks[index] {
                Token::OpenParen => depth += 1,
                Token::CloseParen if depth == 0 => break,
                Token::CloseParen => depth -= 1,
                Token::Ident(value) if depth == 0 && value.eq_ignore_ascii_case("of") => {
                    of = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let Some(of_index) = of else {
            let (a, b) = self.anb()?;
            return Some(Simple::Pseudo(if from_last {
                Pseudo::NthLastChild { a, b }
            } else {
                Pseudo::NthChild { a, b }
            }));
        };
        let formula_end = (self.pos..of_index)
            .rev()
            .find(|&i| !matches!(self.toks[i], Token::Whitespace))
            .map(|i| i + 1)?;
        let mut formula = self.toks[self.pos..formula_end].to_vec();
        formula.push(Token::CloseParen);
        let (a, b) = SelParser {
            toks: &formula,
            pos: 0,
            failed: false,
        }
        .anb()?;
        self.pos = of_index + 1;
        let selectors = self.selector_list_until_close(false)?;
        if selectors.iter().any(|selector| {
            selector
                .compounds
                .iter()
                .any(compound_contains_pseudo_element)
        }) {
            return None;
        }
        Some(Simple::NthChildOf {
            a,
            b,
            from_last,
            selectors,
        })
    }

    fn selector_list_until_close(&mut self, forgiving: bool) -> Option<Vec<Selector>> {
        let mut selectors = Vec::new();
        let mut start = self.pos;
        let mut depth = 0usize;
        loop {
            let end = self.pos;
            match self.peek()? {
                Token::OpenParen => depth += 1,
                Token::CloseParen if depth > 0 => depth -= 1,
                Token::Comma | Token::CloseParen if depth == 0 => {
                    let parsed = parse_selector(&serialize(&self.toks[start..end]));
                    if let Some(selector) = parsed {
                        selectors.push(selector);
                    } else if !forgiving {
                        return None;
                    }
                    let closing = matches!(self.peek(), Some(Token::CloseParen));
                    self.pos += 1;
                    if closing {
                        return (forgiving || !selectors.is_empty()).then_some(selectors);
                    }
                    start = self.pos;
                    continue;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// `an+b` micro-syntax: `odd`, `even`, integers, `n`, `2n±k`, `-n+k`.
    ///
    /// Accumulates the raw token text (the tokenizer splits `2n` into a
    /// Dimension token and `-1` into a signed Number, so the pieces are
    /// re-serialized before the standalone `an+b` parser runs).
    fn anb(&mut self) -> Option<(i32, i32)> {
        self.skip_ws();
        let mut text = String::new();
        while let Some(tok) = self.peek().cloned() {
            // Work on an owned token each round so `self.pos` can advance
            // without borrow conflicts (Token is Clone but not Copy).
            let consumed = match tok {
                Token::Dimension(..) => {
                    // `2n` arrives as one Dimension token; re-serialize it so
                    // the numeric coefficient isn't lost.
                    text.push_str(&serialize(&[tok]));
                    true
                }
                Token::Ident(s) if !text.contains('n') => {
                    text.push_str(&s);
                    true
                }
                // CSS Syntax tokenizes a separated sign (`2n - 2`) as an
                // identifier, while an adjacent sign is a delimiter or part
                // of the following number.
                Token::Ident(s) if matches!(s.as_str(), "+" | "-") => {
                    text.push_str(&s);
                    true
                }
                Token::Number(n) => {
                    // The tokenizer folds the sign into the number (`2n+1`
                    // arrives as Dimension "2n" then Number +1), so an
                    // explicit '+' must be restored for the b term.
                    if n >= 0.0 && text.ends_with('n') {
                        text.push('+');
                    }
                    if n < 0.0 && text.ends_with('-') {
                        text.pop(); // "2n-" + Number(-1) would double the sign
                    }
                    text.push_str(&serialize(&[tok]));
                    true
                }
                Token::Delim(c @ ('+' | '-')) => {
                    text.push(c);
                    true
                }
                Token::Whitespace if !text.is_empty() => {
                    // Whitespace is legal only around '+'/'-' in an+b; any
                    // other gap terminates the expression.
                    matches!(
                        self.toks.get(self.pos + 1),
                        Some(Token::Delim('+') | Token::Delim('-'))
                    )
                }
                _ => break,
            };
            if !consumed {
                break;
            }
            self.pos += 1;
            self.skip_ws();
        }
        // Validate before consuming the ')': a rejected expression must
        // leave the cursor alone, or the failure silently degrades the
        // selector (`p:nth-child(n+)` would otherwise parse as just `p`).
        let parsed = parse_anb(&text)?;
        if self.bump()? != Token::CloseParen {
            return None;
        }
        Some(parsed)
    }
}

/// Standalone `an+b` parser over the accumulated token text.
fn parse_anb(text: &str) -> Option<(i32, i32)> {
    let t = text.trim().to_ascii_lowercase();
    match t.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        "" => return None,
        _ => {}
    }
    // Only digits, 'n', and signs may appear; at most one 'n'.
    if t.contains("nn")
        || !t
            .chars()
            .all(|c| c.is_ascii_digit() || c == 'n' || c == '+' || c == '-')
    {
        return None;
    }
    match t.find('n') {
        None => Some((0, t.parse::<i32>().ok()?)),
        Some(n_pos) => {
            let (a_part, b_raw) = t.split_at(n_pos);
            let b_raw = &b_raw[1..]; // past 'n'
            // `2n3`-style junk: 'n' directly followed by a digit is invalid.
            if b_raw.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            let a = match a_part {
                "" | "+" => 1,
                "-" => -1,
                other => other.parse::<i32>().ok()?,
            };
            let b = match b_raw.trim() {
                "" => 0,
                s => s.parse::<i32>().ok()?,
            };
            Some((a, b))
        }
    }
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Does element `el` match the whole complex selector?
pub fn matches(dom: &Dom, el: NodeIdx, sel: &Selector) -> bool {
    matches_in_scope(dom, el, sel, None)
}

/// Match with an explicit scoping root; absent roots use the document element.
pub(crate) fn matches_in_scope(
    dom: &Dom,
    el: NodeIdx,
    sel: &Selector,
    scope: Option<NodeIdx>,
) -> bool {
    if sel.pseudo_element().is_some() {
        return false;
    }
    match_rightward(dom, el, sel, sel.compounds.len() - 1, scope)
}

pub(crate) fn matches_pseudo_in_scope(
    dom: &Dom,
    el: NodeIdx,
    sel: &Selector,
    pseudo: PseudoElement,
    scope: Option<NodeIdx>,
) -> bool {
    sel.pseudo_element() == Some(pseudo)
        && match_rightward(dom, el, sel, sel.compounds.len() - 1, scope)
}

/// Right-to-left matching: compound at position `pos` must match `el`, then
/// the combinator at `pos - 1` determines which candidate nodes the previous
/// compound is tried against.
fn match_rightward(
    dom: &Dom,
    el: NodeIdx,
    sel: &Selector,
    pos: usize,
    scope: Option<NodeIdx>,
) -> bool {
    if !compound_in_scope(dom, el, &sel.compounds[pos], scope) {
        return false;
    }
    if pos == 0 {
        return true;
    }
    match dom.get(el).parent {
        None => false,
        Some(parent) => match sel.combinators[pos - 1] {
            Combinator::Child => {
                parent_is_element(dom, parent) && match_rightward(dom, parent, sel, pos - 1, scope)
            }
            Combinator::Descendant => {
                let mut anc = Some(parent);
                while let Some(a) = anc {
                    if parent_is_element(dom, a) && match_rightward(dom, a, sel, pos - 1, scope) {
                        return true;
                    }
                    anc = dom.get(a).parent;
                }
                false
            }
            Combinator::NextSibling => {
                let mut sibling = dom.get(el).prev_sibling;
                while let Some(node) = sibling {
                    if dom.name(node).is_some() {
                        return match_rightward(dom, node, sel, pos - 1, scope);
                    }
                    sibling = dom.get(node).prev_sibling;
                }
                false
            }
            Combinator::LaterSibling => {
                let mut sib = dom.get(el).prev_sibling;
                while let Some(s) = sib {
                    if match_rightward(dom, s, sel, pos - 1, scope) {
                        return true;
                    }
                    sib = dom.get(s).prev_sibling;
                }
                false
            }
        },
    }
}

/// The document node is not an element: `* > html` must not match through it.
fn parent_is_element(dom: &Dom, idx: NodeIdx) -> bool {
    dom.name(idx).is_some()
}

/// True when element `el` matches every simple selector in `comp`.
pub fn matches_compound(dom: &Dom, el: NodeIdx, comp: &Compound) -> bool {
    compound_in_scope(dom, el, comp, None)
}

fn compound_in_scope(dom: &Dom, el: NodeIdx, comp: &Compound, scope: Option<NodeIdx>) -> bool {
    comp.simple
        .iter()
        .all(|s| simple_in_scope(dom, el, s, scope))
}

pub fn matches_simple(dom: &Dom, el: NodeIdx, s: &Simple) -> bool {
    simple_in_scope(dom, el, s, None)
}

fn simple_in_scope(dom: &Dom, el: NodeIdx, s: &Simple, scope: Option<NodeIdx>) -> bool {
    match s {
        Simple::Type(name) => dom.name(el).is_some_and(|local| {
            if dom.namespace(el) == Some(Namespace::Html) {
                local.eq_ignore_ascii_case(name)
            } else {
                local == name
            }
        }),
        Simple::Universal => true,
        Simple::Id(id) => attr_is(dom, el, "id", id),
        Simple::Class(class) => class_list_contains(dom, el, class),
        Simple::Attr {
            name,
            op,
            value,
            ci,
        } => match_attr(dom, el, name, *op, value, *ci),
        Simple::PseudoElement(_) => true,
        Simple::Pseudo(Pseudo::Scope) => {
            scope.map_or_else(|| matches_pseudo(dom, el, Pseudo::Root), |root| root == el)
        }
        Simple::Pseudo(p) => matches_pseudo(dom, el, *p),
        Simple::Negation(args) => !args.iter().any(|c| compound_in_scope(dom, el, c, scope)),
        Simple::Matches(args) => args.iter().any(|c| compound_in_scope(dom, el, c, scope)),
        Simple::Where(args) => args.iter().any(|c| compound_in_scope(dom, el, c, scope)),
        Simple::ComplexNegation(args) => !args.iter().any(|s| match_rightward(dom, el, s, s.compounds.len() - 1, scope)),
        Simple::ComplexMatches(args) | Simple::ComplexWhere(args) =>
            args.iter().any(|s| match_rightward(dom, el, s, s.compounds.len() - 1, scope)),
        Simple::Has(args) => args.iter().any(|(relation, compound)| match relation {
            Combinator::Child => dom.children(el).any(|child| {
                dom.name(child).is_some() && compound_in_scope(dom, child, compound, scope)
            }),
            Combinator::Descendant => {
                let mut pending: Vec<_> = dom.children(el).collect();
                while let Some(node) = pending.pop() {
                    if dom.name(node).is_some() && compound_in_scope(dom, node, compound, scope) {
                        return true;
                    }
                    pending.extend(dom.children(node));
                }
                false
            }
            _ => false,
        }),
        Simple::Lang(ranges) => ranges.iter().any(|range| {
            let language = element_language(dom, el);
            if range.is_empty() {
                return language.is_none();
            }
            language.is_some_and(|lang| extended_language_match(range, lang))
        }),
        Simple::Dir(rtl) => element_direction(dom, el) == *rtl,
        Simple::NthChildOf {
            a,
            b,
            from_last,
            selectors,
        } => nth_index_of(dom, el, *from_last, selectors)
            .is_some_and(|index| anb_matches(*a, *b, index)),
    }
}

fn element_language<'a>(dom: &'a Dom, el: NodeIdx) -> Option<&'a str> {
    let mut current = Some(el);
    while let Some(node) = current {
        if let Some(language) =
            attr_of(dom, node, "lang").or_else(|| attr_of(dom, node, "xml:lang"))
        {
            return (!language.is_empty()).then_some(language);
        }
        current = dom.get(node).parent;
    }
    None
}

/// RFC 4647 §3.3.2 extended filtering. Non-initial range wildcards skip
/// positions; a tag singleton starts an extension and cannot be skipped.
fn extended_language_match(range: &str, language: &str) -> bool {
    let ranges: Vec<_> = range.split('-').collect();
    let tags: Vec<_> = language.split('-').collect();
    let valid_range = ranges.iter().enumerate().all(|(index, part)| {
        *part == "*"
            || (!part.is_empty()
                && part.len() <= 8
                && part.bytes().all(|c| {
                    if index == 0 {
                        c.is_ascii_alphabetic()
                    } else {
                        c.is_ascii_alphanumeric()
                    }
                }))
    });
    let valid_tag = tags.iter().all(|part| {
        !part.is_empty() && part.len() <= 8 && part.bytes().all(|c| c.is_ascii_alphanumeric())
    });
    if !valid_range || !valid_tag || ranges.is_empty() || tags.is_empty() {
        return false;
    }
    if ranges[0] != "*" && !ranges[0].eq_ignore_ascii_case(tags[0]) {
        return false;
    }
    let (mut r, mut t) = (1, 1);
    while r < ranges.len() {
        if ranges[r] == "*" {
            r += 1;
            continue;
        }
        if t == tags.len() {
            return false;
        }
        if ranges[r].eq_ignore_ascii_case(tags[t]) {
            r += 1;
        } else if tags[t].len() == 1 {
            return false;
        }
        t += 1;
    }
    true
}

fn element_direction(dom: &Dom, el: NodeIdx) -> bool {
    let mut current = Some(el);
    while let Some(node) = current {
        if let Some(direction) = attr_of(dom, node, "dir") {
            if direction.eq_ignore_ascii_case("rtl") {
                return true;
            }
            if direction.eq_ignore_ascii_case("ltr") {
                return false;
            }
        }
        current = dom.get(node).parent;
    }
    false
}

fn nth_index_of(dom: &Dom, el: NodeIdx, from_last: bool, selectors: &[Selector]) -> Option<u32> {
    let parent = dom.get(el).parent?;
    let siblings: Vec<_> = dom
        .children(parent)
        .filter(|node| {
            dom.name(*node).is_some()
                && selectors
                    .iter()
                    .any(|selector| matches(dom, *node, selector))
        })
        .collect();
    let position = siblings.iter().position(|&node| node == el)? as u32 + 1;
    Some(if from_last {
        siblings.len() as u32 + 1 - position
    } else {
        position
    })
}

fn matches_pseudo(dom: &Dom, el: NodeIdx, p: Pseudo) -> bool {
    match p {
        Pseudo::Root | Pseudo::Scope => dom
            .get(el)
            .parent
            .is_some_and(|par| dom.name(par).is_none()),
        Pseudo::FirstChild => nth_index(dom, el, false) == Some(1),
        Pseudo::LastChild => nth_index(dom, el, true) == Some(1),
        Pseudo::OnlyChild => {
            nth_index(dom, el, false) == Some(1) && nth_index(dom, el, true) == Some(1)
        }
        Pseudo::FirstOfType => type_position(dom, el, false) == Some(1),
        Pseudo::LastOfType => type_position(dom, el, true) == Some(1),
        Pseudo::OnlyOfType => {
            type_position(dom, el, false) == Some(1) && type_position(dom, el, true) == Some(1)
        }
        Pseudo::Empty => is_empty(dom, el),
        Pseudo::NthChild { a, b } => {
            nth_index(dom, el, false).is_some_and(|i| anb_matches(a, b, i))
        }
        Pseudo::NthLastChild { a, b } => {
            nth_index(dom, el, true).is_some_and(|i| anb_matches(a, b, i))
        }
        Pseudo::NthOfType { a, b } => {
            type_position(dom, el, false).is_some_and(|i| anb_matches(a, b, i))
        }
        Pseudo::NthLastOfType { a, b } => {
            type_position(dom, el, true).is_some_and(|i| anb_matches(a, b, i))
        }
        Pseudo::Link => {
            matches!(dom.name(el), Some("a") | Some("area")) && attr_of(dom, el, "href").is_some()
        }
        // Never matches (privacy) / no interaction state in static rendering.
        Pseudo::Visited | Pseudo::Hover | Pseudo::Active | Pseudo::Focus | Pseudo::FocusWithin => {
            false
        }
        // Form-control state: our DOM has no widget model, so only the
        // explicit `disabled`/`checked` attributes are observable.
        Pseudo::Enabled => attr_of(dom, el, "disabled").is_none(),
        Pseudo::Disabled => attr_of(dom, el, "disabled").is_some(),
        Pseudo::Checked => attr_of(dom, el, "checked").is_some(),
        Pseudo::Required => supports_required(dom, el) && attr_of(dom, el, "required").is_some(),
        Pseudo::Optional => supports_required(dom, el) && attr_of(dom, el, "required").is_none(),
        Pseudo::Open => matches!(dom.name(el), Some("details" | "dialog"))
            && attr_of(dom, el, "open").is_some(),
    }
}

fn supports_required(dom: &Dom, el: NodeIdx) -> bool {
    match dom.name(el) {
        Some("select" | "textarea") => true,
        Some("input") => !attr_of(dom, el, "type").is_some_and(|kind| {
            ["hidden", "range", "color", "submit", "image", "reset", "button"]
                .iter().any(|excluded| kind.eq_ignore_ascii_case(excluded))
        }),
        _ => false,
    }
}

/// 1-based position among element siblings, counted forward (`false`) or
/// backward (`true`). `None` when `el` has no parent context.
fn nth_index(dom: &Dom, el: NodeIdx, from_last: bool) -> Option<u32> {
    let parent = dom.get(el).parent?;
    let siblings: Vec<NodeIdx> = dom
        .children(parent)
        .filter(|c| dom.name(*c).is_some())
        .collect();
    let pos = siblings.iter().position(|&c| c == el)? as u32 + 1;
    Some(if from_last {
        siblings.len() as u32 + 1 - pos
    } else {
        pos
    })
}

/// Position among same-type element siblings.
fn type_position(dom: &Dom, el: NodeIdx, from_last: bool) -> Option<u32> {
    let my_name = dom.name(el)?.to_owned();
    let my_namespace = dom.namespace(el)?;
    let parent = dom.get(el).parent?;
    let same_type: Vec<NodeIdx> = dom
        .children(parent)
        .filter(|c| {
            dom.namespace(*c) == Some(my_namespace)
                && dom.name(*c).is_some_and(|name| {
                    if my_namespace == Namespace::Html {
                        name.eq_ignore_ascii_case(&my_name)
                    } else {
                        name == my_name
                    }
                })
        })
        .collect();
    let pos = same_type.iter().position(|&c| c == el)? as u32 + 1;
    Some(if from_last {
        same_type.len() as u32 + 1 - pos
    } else {
        pos
    })
}

fn is_empty(dom: &Dom, el: NodeIdx) -> bool {
    // Selectors-4: children consisting solely of whitespace still count as
    // empty; comments do not affect emptiness.
    dom.children(el).all(|c| match &dom.get(c).kind {
        amazing_dom::NodeKind::Comment { .. }
        | amazing_dom::NodeKind::ProcessingInstruction { .. } => true,
        amazing_dom::NodeKind::Text { data } => data.chars().all(char::is_whitespace),
        amazing_dom::NodeKind::CDATASection { data } => data.chars().all(char::is_whitespace),
        _ => false,
    })
}

/// ∃ k ≥ 0 such that a·k + b == i.
fn anb_matches(a: i32, b: i32, i: u32) -> bool {
    let i = i as i32;
    if a == 0 {
        return i == b;
    }
    let diff = i - b;
    diff % a == 0 && diff / a >= 0
}

fn attr_of<'d>(dom: &'d Dom, el: NodeIdx, name: &str) -> Option<&'d str> {
    dom.attrs_of(el)?
        .iter()
        .find(|kv| kv.0 == name)
        .map(|(_, v)| v.as_str())
}

fn attr_is(dom: &Dom, el: NodeIdx, name: &str, want: &str) -> bool {
    attr_of(dom, el, name) == Some(want)
}

fn class_list_contains(dom: &Dom, el: NodeIdx, class: &str) -> bool {
    attr_of(dom, el, "class").is_some_and(|v| v.split_ascii_whitespace().any(|w| w == class))
}

fn match_attr(dom: &Dom, el: NodeIdx, name: &str, op: AttrOp, value: &str, ci: bool) -> bool {
    let actual = match attr_of(dom, el, name) {
        Some(v) => v,
        None => return false,
    };
    let norm = |s: &str| {
        if ci {
            s.to_ascii_lowercase()
        } else {
            s.to_owned()
        }
    };
    let want = norm(value);
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => norm(actual) == want,
        AttrOp::Includes => {
            !want.is_empty() && actual.split_ascii_whitespace().any(|w| norm(w) == want)
        }
        AttrOp::DashMatch => {
            let a = norm(actual);
            a == want || a.starts_with(&format!("{want}-"))
        }
        AttrOp::Prefix => !want.is_empty() && norm(actual).starts_with(&want),
        AttrOp::Suffix => !want.is_empty() && norm(actual).ends_with(&want),
        AttrOp::Substring => !want.is_empty() && norm(actual).contains(&want),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a tiny tree: html > body > (p#intro.red, ul > li*3, div.empty)
    fn fixture() -> Dom {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let p = dom.create_element(body, "p");
        dom.set_attrs(
            p,
            vec![
                ("id".into(), "intro".into()),
                ("class".into(), "red lead".into()),
                ("data-lang".into(), "en-US".into()),
            ],
        );
        dom.create_text(p, "hi"); // keeps p out of the :empty set
        let ul = dom.create_element(body, "ul");
        for label in ["one", "two", "three"] {
            let li = dom.create_element(ul, "li");
            dom.create_text(li, label);
        }
        let div = dom.create_element(body, "div");
        dom.set_attrs(
            div,
            vec![
                ("class".into(), "empty-box".into()),
                ("id".into(), String::new()),
            ],
        );
        dom
    }

    /// Walks element-only child indices from the document node and reports
    /// whether the node at that position matches `src`.
    fn m(dom: &Dom, path: &[usize], src: &str) -> bool {
        let mut cur = NodeIdx(0);
        for &i in path {
            let kids: Vec<NodeIdx> = dom
                .children(cur)
                .filter(|c| dom.name(*c).is_some())
                .collect();
            cur = kids[i];
        }
        matches(
            dom,
            cur,
            &parse_selector(src).unwrap_or_else(|| panic!("parse fail: {src}")),
        )
    }

    #[test]
    fn language_direction_and_relative_has_match_dom_state() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        dom.set_attrs(html, vec![("lang".into(), "en-US".into())]);
        let body = dom.create_element(html, "body");
        let first = dom.create_element(body, "div");
        let second = dom.create_element(body, "div");
        let span = dom.create_element(second, "span");
        dom.set_attrs(span, vec![("lang".into(), "fr-FR-x-foobar".into())]);
        let rtl = dom.create_element(body, "div");
        dom.set_attrs(rtl, vec![("dir".into(), "rtl".into())]);
        let child = dom.create_element(rtl, "div");
        let check = |el, text| matches(&dom, el, &parse_selector(text).unwrap());
        assert!(check(first, ":lang(en-US)"));
        assert!(check(span, ":lang(fr)"));
        assert!(check(span, ":lang(\"FR\")"));
        assert!(!check(first, ":lang(fr)"));
        assert!(check(second, ":has(> span)"));
        assert!(!check(first, ":has(> span)"));
        assert!(check(second, ":not(:dir(rtl)) + div"));
        assert!(check(child, ":dir(rtl)"));
        dom.create_text(body, "\n");
        let next = dom.create_element(body, "div");
        assert!(matches(
            &dom,
            next,
            &parse_selector(":not(:dir(ltr)) + div").unwrap()
        ));
    }

    #[test]
    fn required_and_optional_match_only_eligible_form_controls() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let required = dom.create_element(body, "input");
        dom.set_attrs(required, vec![("required".into(), "".into())]);
        let optional = dom.create_element(body, "textarea");
        let hidden = dom.create_element(body, "input");
        dom.set_attrs(hidden, vec![("type".into(), "hidden".into())]);
        let unrelated = dom.create_element(body, "p");
        let check = |node, selector| matches(&dom, node, &parse_selector(selector).unwrap());
        assert!(check(required, ":required"));
        assert!(!check(required, ":optional"));
        assert!(check(optional, ":optional"));
        assert!(!check(optional, ":required"));
        assert!(!check(hidden, ":optional"));
        assert!(!check(unrelated, ":optional"));
    }

    #[test]
    fn open_state_and_no_namespace_attribute_selector() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let details = dom.create_element(html, "details");
        dom.set_attrs(details, vec![("open".into(), "".into()), ("data-test".into(), "".into())]);
        let other = dom.create_element(html, "div");
        dom.set_attrs(other, vec![("open".into(), "".into())]);
        assert!(matches(&dom, details, &parse_selector(":open").unwrap()));
        assert!(!matches(&dom, other, &parse_selector(":open").unwrap()));
        assert!(matches(&dom, details, &parse_selector("[|data-test]").unwrap()));
        assert!(parse_selector("[| data-test]").is_none());
    }

    #[test]
    fn nth_of_filters_siblings_and_counts_from_either_end() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let mut elements = Vec::new();
        for name in ["target", "div", "target", "target", "div"] {
            elements.push(dom.create_element(body, name));
        }
        let matches_text = |el, text| matches(&dom, el, &parse_selector(text).unwrap());
        assert!(matches_text(
            elements[3],
            ":nth-child(3 of/*comment*/target)"
        ));
        assert!(matches_text(
            elements[3],
            ":nth-child(3 of/* my comment */target)"
        ));
        assert!(!matches_text(elements[2], ":nth-child(3 of target)"));
        dom.set_attrs(
            elements[0],
            vec![
                ("class".into(), "target".into()),
                ("target".into(), "".into()),
            ],
        );
        dom.set_attrs(
            elements[2],
            vec![
                ("class".into(), "target".into()),
                ("target".into(), "".into()),
            ],
        );
        dom.set_attrs(
            elements[3],
            vec![
                ("class".into(), "target".into()),
                ("target".into(), "".into()),
            ],
        );
        let matches_text = |el, text| matches(&dom, el, &parse_selector(text).unwrap());
        assert!(matches_text(elements[3], ":nth-child(3 of.target)"));
        assert!(matches_text(elements[3], ":nth-child(3 of[target])"));
        assert!(matches_text(
            elements[3],
            ":nth-last-child(1 of target, .missing)"
        ));
        assert_eq!(
            parse_selector(":nth-child(3 of target)")
                .unwrap()
                .specificity(),
            Specificity { a: 0, b: 1, c: 1 }
        );
    }

    #[test]
    fn nth_last_child_of_complex_selector_list() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let paragraphs: Vec<_> = (0..9)
            .map(|_| {
                let paragraph = dom.create_element(body, "p");
                dom.create_text(body, "\n");
                paragraph
            })
            .collect();
        let selector = parse_selector("p:nth-last-child(4n of html:root>body>p+p:not(empty), :not(*), p:not(p), span, .notthere)").unwrap();
        for (index, paragraph) in paragraphs.into_iter().enumerate() {
            assert_eq!(
                matches(&dom, paragraph, &selector),
                index == 1 || index == 5,
                "p {index}"
            );
        }
    }

    #[test]
    fn nth_last_child_of_nested_nth_last_child() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let paragraphs: Vec<_> = (0..12)
            .map(|index| {
                let paragraph = dom.create_element(body, "p");
                if index % 2 == 1 {
                    dom.set_attrs(paragraph, vec![("class".into(), "target".into())]);
                }
                paragraph
            })
            .collect();
        assert!(parse_selector(":nth-last-child(n of .target)").is_some());
        assert!(parse_selector(":nth-last-child(2n - 2 of .target)").is_some());
        assert!(parse_selector(":nth-child(2n + 2 of .target)").is_some());
        let selector = parse_selector(
            ":nth-last-child(2n - 2 of :nth-last-child(n of .target))",
        ).expect("nested nth syntax");
        for (index, paragraph) in paragraphs.into_iter().enumerate() {
            assert_eq!(matches(&dom, paragraph, &selector), [1, 5, 9].contains(&index), "p {index}");
        }
    }

    #[test]
    fn parses_and_matches_basic_selectors() {
        let d = fixture();
        // Paths resolve document > html > body > (p, ul, div).
        assert!(m(&d, &[0, 0, 0], "p"));
        assert!(m(&d, &[0, 0, 0], "body p"));
        assert!(m(&d, &[0, 0, 0], "#intro"));
        assert!(m(&d, &[0, 0, 0], ".lead"));
        assert!(m(&d, &[0, 0, 0], "p.lead.red#intro"));
        assert!(!m(&d, &[0, 0, 0], ".empty-box"));
        assert!(m(&d, &[0, 0, 1], "*"));
        assert!(m(&d, &[0, 0], "html > body"));
        assert!(!m(&d, &[0, 0, 0], "html > p"));
        assert!(m(&d, &[0, 0, 1], "p + ul"));
        assert!(m(&d, &[0, 0, 2], "p ~ div"));
        assert!(!m(&d, &[0, 0, 1], "ul ~ p"));
        assert!(m(&d, &[0, 0, 2], "ul + div"));
    }

    #[test]
    fn attribute_operators() {
        let d = fixture();
        let p = [0, 0, 0];
        let div = [0, 0, 2];
        assert!(m(&d, &p, "[data-lang]"));
        assert!(m(&d, &p, "[data-lang=en-US]"));
        assert!(!m(&d, &p, "[data-lang=en-us]")); // values are case-sensitive
        assert!(m(&d, &p, "[data-lang=en-us i]")); // …unless flagged
        assert!(m(&d, &p, "[data-lang=en-US i]"));
        assert!(m(&d, &p, "[class~=red]"));
        assert!(!m(&d, &p, "[class~=redbox]"));
        assert!(m(&d, &p, "[data-lang|=en]"));
        assert!(!m(&d, &p, "[data-lang|=US]"));
        assert!(m(&d, &p, "[id^=intr]"));
        assert!(m(&d, &p, "[id$=tro]"));
        assert!(m(&d, &p, "[id*=ntr]"));
        // Empty ^=/$=/*=/~= values never match (selectors-4); the div carries
        // an empty id attribute so the Exists case still has an attr to see.
        assert!(m(&d, &div, "[id]"));
        assert!(!m(&d, &div, "[id^=\"\"]"));
        assert!(!m(&d, &div, "[id$=\"\"]"));
        assert!(!m(&d, &div, "[id*=\"\"]"));
        assert!(!m(&d, &div, "[id~=\"\"]"));
    }

    #[test]
    fn structural_pseudos() {
        let d = fixture();
        // body children: p, ul, div (element-only counting ignores nothing
        // here; li text nodes don't affect ul's position).
        let p = [0, 0, 0];
        let ul = [0, 0, 1];
        let div = [0, 0, 2];
        assert!(m(&d, &p, ":first-child"));
        assert!(m(&d, &div, ":last-child"));
        assert!(!m(&d, &ul, ":first-child"));
        assert!(!m(&d, &ul, ":only-child"));
        assert!(m(&d, &ul, ":nth-child(2)"));
        assert!(m(&d, &ul, ":nth-child(even)"));
        assert!(m(&d, &p, ":nth-child(odd)"));
        assert!(m(&d, &ul, ":nth-child(2n+2)")); // position 2
        assert!(!m(&d, &div, ":nth-child(2n+2)")); // position 3 misses
        assert!(m(&d, &ul, ":nth-child(-n+2)")); // positions 1,2
        assert!(!m(&d, &div, ":nth-child(-n+2)"));
        assert!(m(&d, &div, ":nth-last-child(1)"));
        assert!(m(&d, &[0, 0, 1, 1], "li:nth-child(2)"));
        // text content makes li non-empty; div has none
        assert!(m(&d, &div, ":empty"));
        assert!(!m(&d, &p, ":empty"));
        // of-type positions among the single p / single div
        assert!(m(&d, &p, "p:first-of-type"));
        assert!(m(&d, &p, "p:only-of-type"));
        // A lone p is first, last, AND only of its type at once; the three
        // li siblings exercise real of-type positions.
        assert!(m(&d, &p, "p:last-of-type"));
        assert!(!m(&d, &[0, 0, 1, 0], "li:last-of-type"));
        assert!(m(&d, &[0, 0, 1, 2], "li:last-of-type"));
    }

    #[test]
    fn foreign_type_matching_uses_namespace_and_case_sensitive_local_names() {
        let mut dom = Dom::new();
        let body = dom.create_element(NodeIdx(0), "body");
        let _html_rect = dom.create_element(body, "rect");
        let svg_rect = dom.append_child(
            body,
            amazing_dom::NodeKind::Element {
                name: "rect".into(),
                attrs: Vec::new(),
                namespace: Namespace::Svg,
            },
        );
        let gradient = dom.append_child(
            body,
            amazing_dom::NodeKind::Element {
                name: "linearGradient".into(),
                attrs: Vec::new(),
                namespace: Namespace::Svg,
            },
        );
        assert!(matches(
            &dom,
            svg_rect,
            &parse_selector("rect:first-of-type").unwrap()
        ));
        assert!(matches(
            &dom,
            gradient,
            &parse_selector("linearGradient").unwrap()
        ));
        assert!(!matches(
            &dom,
            gradient,
            &parse_selector("lineargradient").unwrap()
        ));
    }

    #[test]
    fn processing_instructions_do_not_break_empty() {
        let mut dom = Dom::new();
        let element = dom.create_element(NodeIdx(0), "div");
        dom.append_child(
            element,
            amazing_dom::NodeKind::ProcessingInstruction {
                target: "test".into(),
                data: String::new(),
            },
        );
        assert!(is_empty(&dom, element));
    }

    #[test]
    fn functional_pseudos_and_root() {
        let d = fixture();
        let p = [0, 0, 0];
        let ul = [0, 0, 1];
        assert!(m(&d, &[0], ":root"));
        assert!(!m(&d, &p, ":root"));
        assert!(m(&d, &p, "p:not(.empty-box)"));
        assert!(!m(&d, &p, "p:not(#intro)"));
        assert!(m(&d, &p, "p:not(div):not(.zzz)"));
        assert!(m(&d, &ul, ":is(ul, ol)"));
        assert!(!m(&d, &p, ":is(ul, ol)"));
        assert!(m(&d, &ul, ":where(ul)")); // matches like :is
        assert!(m(&d, &ul, ":matches(ul)")); // legacy alias
    }

    #[test]
    fn specificity_values() {
        let s = |src: &str| parse_selector(src).unwrap().specificity();
        assert_eq!(s("*"), Specificity { a: 0, b: 0, c: 0 });
        assert_eq!(s("li"), Specificity { a: 0, b: 0, c: 1 });
        assert_eq!(s("ul li"), Specificity { a: 0, b: 0, c: 2 });
        assert_eq!(s(".a"), Specificity { a: 0, b: 1, c: 0 });
        assert_eq!(s("ul.foo"), Specificity { a: 0, b: 1, c: 1 });
        assert_eq!(s("#x"), Specificity { a: 1, b: 0, c: 0 });
        assert_eq!(s("#x.y.z"), Specificity { a: 1, b: 2, c: 0 });
        assert_eq!(s("[href]"), Specificity { a: 0, b: 1, c: 0 });
        assert_eq!(s(":hover"), Specificity { a: 0, b: 1, c: 0 });
        // :not/:is take the most specific argument; :where takes zero.
        assert_eq!(s(":not(#a, .b)"), Specificity { a: 1, b: 0, c: 0 });
        assert_eq!(s(":is(em, #a.b)"), Specificity { a: 1, b: 1, c: 0 });
        assert_eq!(s(":where(#a)"), Specificity { a: 0, b: 0, c: 0 });
        assert_eq!(s(":is(#a + div, main > .b)"), Specificity { a: 1, b: 0, c: 1 });
        assert_eq!(s(":not(main > #a, .b + em)"), Specificity { a: 1, b: 0, c: 1 });
        assert_eq!(s("div:where(main > #a, .b + em)"), Specificity { a: 0, b: 0, c: 1 });
        // FOO is a type argument: :not contributes its (0,0,1).
        assert_eq!(s("#s12:not(FOO)#s34"), Specificity { a: 2, b: 0, c: 1 });
        assert_eq!(
            s("ul > li:nth-child(2n+1) a:hover"),
            Specificity { a: 0, b: 2, c: 3 }
        );
        // Compound arguments sum internally before the max is taken
        // (.a.b is two classes → (0,2,0)).
        assert_eq!(s("p:not(.a.b)"), Specificity { a: 0, b: 2, c: 1 });
    }

    #[test]
    fn anb_parser_forms() {
        assert_eq!(parse_anb("odd"), Some((2, 1)));
        assert_eq!(parse_anb("even"), Some((2, 0)));
        assert_eq!(parse_anb("5"), Some((0, 5)));
        assert_eq!(parse_anb("-3"), Some((0, -3)));
        assert_eq!(parse_anb("n"), Some((1, 0)));
        assert_eq!(parse_anb("-n+3"), Some((-1, 3)));
        assert_eq!(parse_anb("2n-1"), Some((2, -1)));
        assert_eq!(parse_anb("+2n+1"), Some((2, 1)));
        assert_eq!(parse_anb("2n+1"), Some((2, 1)));
        assert_eq!(parse_anb(""), None);
        assert_eq!(parse_anb("nn"), None);
        assert_eq!(parse_anb("2n3"), None); // digit glued after n
        assert_eq!(parse_anb("2em"), None); // wrong unit
        assert_eq!(parse_anb("n+b"), None); // non-numeric b
    }

    #[test]
    fn pseudo_element_syntax_target_and_specificity() {
        let d = fixture();
        let p = d.children(NodeIdx(0)).next().unwrap();
        let p = d.children(p).next().unwrap();
        let p = d.children(p).next().unwrap();
        let before = parse_selector("p.lead::before").unwrap();
        assert_eq!(before.specificity(), Specificity { a: 0, b: 1, c: 2 });
        assert!(!matches(&d, p, &before));
        assert!(matches_pseudo_in_scope(
            &d,
            p,
            &before,
            PseudoElement::Before,
            None
        ));
        assert!(parse_selector("p:before").is_some());
        for name in [
            "marker",
            "placeholder",
            "selection",
            "first-line",
            "first-letter",
            "backdrop",
            "file-selector-button",
        ] {
            assert!(parse_selector(&format!("p::{name}")).is_some(), "{name}");
        }
        assert!(parse_selector("p:marker").is_none());
        assert!(parse_selector("p::after:hover").is_some());
        assert_eq!(
            parse_selector("p::before::marker")
                .unwrap()
                .pseudo_element(),
            Some(PseudoElement::Marker)
        );
        assert!(parse_selector("p::before::after").is_none());
    }

    #[test]
    fn invalid_selectors_rejected() {
        for bad in [
            "",
            "..a",
            "p >",
            "> p",
            "p >> q",
            "[unclosed",
            "[a==b]",
            ":unknown",
            "p::unknown",
            "p::before.class",
            "p::before span",
            "p:has(::before)",
            "p:nth-child(1 of ::before)",
            "p:not()",
            "p:nth-child(n+)",
            "p:hover!",
            "#",
            ".",
            "[]",
        ] {
            assert!(parse_selector(bad).is_none(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn complex_is_where_not_and_escaped_language_ranges() {
        let mut dom = Dom::new();
        let main = dom.create_element(NodeIdx(0), "main");
        let a = dom.create_element(main, "div");
        dom.set_attrs(a, vec![("id".into(), "a".into())]);
        let b = dom.create_element(main, "div");
        dom.set_attrs(b, vec![("id".into(), "b".into()), ("lang".into(), "fr-Latn-FR".into())]);
        let d = dom.create_element(main, "div");
        dom.set_attrs(d, vec![("id".into(), "d".into())]);
        let e = dom.create_element(main, "div");
        let before = parse_selector(":is(#a, #c) + :is(main :is(#b, #d))::before").unwrap();
        assert!(matches_pseudo_in_scope(&dom, b, &before, PseudoElement::Before, None));
        let after = parse_selector(":is(#d + div, #d ~ #h)::after").unwrap();
        assert!(matches_pseudo_in_scope(&dom, e, &after, PseudoElement::After, None));
        assert!(matches_in_scope(&dom, e, &parse_selector(":where(main > #d + div)").unwrap(), None));
        assert!(!matches_in_scope(&dom, e, &parse_selector(":not(#d + div)").unwrap(), None));
        assert!(matches_in_scope(&dom, b, &parse_selector(r":lang(\*-FR)").unwrap(), None));
        let sheet = crate::parse_stylesheet(r":lang(\*-FR) { color: green }");
        assert_eq!(sheet.rules[0].selector_srcs, [r":lang(\*-FR)"]);
        assert_eq!(crate::compute(&dom, &[sheet]).get(b).unwrap().get("color"), Some("green"));
        assert!(parse_selector(":is(::before, #b)").is_some());
        let empty = parse_selector(":is(::before)").unwrap();
        assert!(!matches(&dom, b, &empty));
        assert!(parse_selector(":where()").is_some());
    }

    #[test]
    fn link_state_and_disabled() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let a = dom.create_element(body, "a");
        dom.set_attrs(
            a,
            vec![
                ("href".into(), "/x".into()),
                ("disabled".into(), String::new()),
            ],
        );
        let plain = dom.create_element(body, "button");
        let sel = |s: &str| parse_selector(s).unwrap();
        assert!(matches(&dom, a, &sel(":link")));
        assert!(!matches(&dom, plain, &sel(":link"))); // no href → not a link
        assert!(!matches(&dom, a, &sel(":visited")));
        assert!(!matches(&dom, a, &sel(":hover")));
        assert!(matches(&dom, a, &sel(":disabled")));
        assert!(matches(&dom, plain, &sel(":enabled")));
    }

    #[test]
    fn document_node_is_not_an_element_for_child_combinator() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        // `* > html` must NOT match: the parent is the document node.
        assert!(!matches(&dom, html, &parse_selector("* > html").unwrap()));
        assert!(matches(&dom, html, &parse_selector("html").unwrap()));
        // …but a descendant walk may pass through it.
        let p = dom.create_element(html, "p");
        assert!(matches(&dom, p, &parse_selector("* p").unwrap()));
    }
}
