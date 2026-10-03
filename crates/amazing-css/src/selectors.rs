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
#[path = "shadow_selectors.rs"]
mod shadow_selectors;
pub use shadow_selectors::{matches_in_tree, matches_pseudo_in_tree};
pub(crate) use shadow_selectors::{selector_origins,selector_parent};
#[cfg(test)]
#[path = "shadow_selector_tests.rs"]
mod shadow_selector_tests;

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
    /// CSS Shadow 1: only meaningful in an explicit shadow tree context.
    Host(Option<Box<Compound>>),
    HostContext(Box<Compound>),
    /// An alias for actual elements assigned to an originating slot.
    Slotted(Box<Compound>),
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
    HasSlotted,
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
    /// Interactive pointer states have no input in a static render: never match.
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Enabled,
    Disabled,
    Checked,
    Required,
    Optional,
    Open,
    /// Constraint validation (Phase 2 subset, CSS-29): attribute-heuristic
    /// only -- see `is_invalid`/`is_validity_candidate`.
    Invalid,
    Valid,
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
        Simple::PseudoElement(_) | Simple::Slotted(_) => true,
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
        Simple::Host(argument) => {
            spec.b += 1;
            if let Some(argument) = argument { spec = spec + compound_specificity(argument); }
        }
        Simple::HostContext(argument) => { spec.b += 1; spec = spec + compound_specificity(argument); }
        Simple::Slotted(argument) => { spec.c += 1; spec = spec + compound_specificity(argument); }
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
    let mut sel = p.complex()?;
    p.skip_ws();
    if p.failed || p.pos != p.toks.len() {
        return None; // trailing junk
    }
    let rightmost = sel.compounds.len() - 1;
    for (index, compound) in sel.compounds.iter_mut().enumerate() {
        let mut pseudo = None;
        let mut slotted = false;
        for simple in &mut compound.simple {
            if matches!(simple, Simple::Slotted(_)) {
                if index != rightmost || slotted || pseudo.is_some() { return None; }
                slotted = true;
                continue;
            }
            if slotted && !slotted_tail(simple) { return None; }
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
                        Pseudo::Hover
                            | Pseudo::Active
                            | Pseudo::Focus
                            | Pseudo::FocusVisible
                            | Pseudo::FocusWithin
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

fn slotted_tail(simple:&mut Simple)->bool {
    match simple {
        Simple::PseudoElement(PseudoElement::Before | PseudoElement::After | PseudoElement::Marker | PseudoElement::Placeholder | PseudoElement::FileSelectorButton) => true,
        _=>user_action_selector(simple),
    }
}
fn user_action_selector(simple:&mut Simple)->bool {
    match simple {
        Simple::Pseudo(Pseudo::Hover | Pseudo::Active | Pseudo::Focus | Pseudo::FocusVisible | Pseudo::FocusWithin)=>true,
        Simple::Negation(args)=>args.iter_mut().all(|compound|compound.simple.iter_mut().all(user_action_selector)),
        Simple::Matches(args) | Simple::Where(args)=> {
            args.retain_mut(|compound|compound.simple.iter_mut().all(user_action_selector));
            true
        }
        Simple::ComplexMatches(args) | Simple::ComplexWhere(args)=> {
            args.retain_mut(|selector|selector.compounds.len()==1 && selector.compounds[0].simple.iter_mut().all(user_action_selector));
            true
        }
        _=>false,
    }
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
                    if !f.eq_ignore_ascii_case("slotted") { return None; }
                    return self.shadow_argument("slotted");
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
        if lower == "host" { return Some(Simple::Host(None)); }
        let pseudo = match lower.as_str() {
            "has-slotted" => Pseudo::HasSlotted,
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
            "focus-visible" => Pseudo::FocusVisible,
            "focus-within" => Pseudo::FocusWithin,
            "enabled" => Pseudo::Enabled,
            "disabled" => Pseudo::Disabled,
            "checked" => Pseudo::Checked,
            "required" => Pseudo::Required,
            "optional" => Pseudo::Optional,
            "open" => Pseudo::Open,
            "invalid" => Pseudo::Invalid,
            "valid" => Pseudo::Valid,
            _ => return None, // unknown pseudo-class invalidates the selector
        };
        Some(Simple::Pseudo(pseudo))
    }

    /// Parses `(selector-list)` after `:not(` / `:is(` / `:matches(` /
    /// `:where(`, and `(an+b)` after `:nth-child(`/`:nth-last-child(`.
    fn functional_args(&mut self, name: String) -> Option<Simple> {
        if matches!(name.as_str(), "host" | "host-context") { return self.shadow_argument(&name); }
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

    fn shadow_argument(&mut self, name: &str) -> Option<Simple> {
        if self.bump()? != Token::OpenParen { return None; }
        self.skip_ws();
        let argument = self.compound()?;
        if compound_contains_pseudo_element(&argument) { return None; }
        self.skip_ws();
        if self.bump()? != Token::CloseParen { return None; }
        Some(match name {
            "host" => Simple::Host(Some(Box::new(argument))),
            "host-context" => Simple::HostContext(Box::new(argument)),
            _ => Simple::Slotted(Box::new(argument)),
        })
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
///
/// CSS-29: made crate-external `pub` (was `pub(crate)`) so a caller outside
/// `amazing-css` can bind `:scope` to something other than the document root
/// -- e.g. `Element.closest()`, whose DOM algorithm matches each inclusive
/// ancestor with `:scope` bound to the ORIGINAL context element throughout
/// the walk, not to whichever ancestor is currently being tested. Wiring
/// that into `closest()` itself is outside this crate (see
/// `.agents/css/questions.md`), but the primitive it needs now exists here.
pub fn matches_in_scope(dom: &Dom, el: NodeIdx, sel: &Selector, scope: Option<NodeIdx>) -> bool {
    if sel.pseudo_element().is_some() {
        return false;
    }
    match_rightward(dom, el, sel, sel.compounds.len() - 1, scope)
}

pub fn matches_pseudo_in_scope(
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
        Simple::Host(_) | Simple::HostContext(_) | Simple::Slotted(_) => false,
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
        Pseudo::HasSlotted => dom.is_html_slot(el)
            && dom.assigned_nodes(el, false, false).is_ok_and(|nodes| !nodes.is_empty())
            && dom.assigned_nodes(el, true, false).is_ok_and(|nodes| !nodes.is_empty()),
        Pseudo::Root | Pseudo::Scope => dom
            .get(el)
            .parent
            .is_some_and(|par| matches!(dom.get(par).kind, amazing_dom::NodeKind::Document)),
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
        // Privacy: `:visited` never matches in this engine. Pointer states
        // have no input in a static render.
        Pseudo::Visited | Pseudo::Hover | Pseudo::Active => false,
        // Focus states read the DOM's programmatic focus record. `:focus-visible`
        // follows the DOM's visibility flag, with the text-field exception: an
        // editable text control is always focus-visible while focused.
        Pseudo::Focus => dom.is_focused(el),
        Pseudo::FocusWithin => dom.is_focus_within(el),
        Pseudo::FocusVisible => {
            dom.is_focused(el)
                && (dom.is_focus_visible(el) || is_text_control(dom, el))
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
        // Constraint validation, Phase 2 subset: no live form/widget state,
        // so validity is read from attributes only -- `required` plus an
        // empty effective value for input/select/textarea, or (per the HTML
        // Standard's own `:invalid`/`:valid` matching rule for `fieldset`
        // and `form`) whether any non-barred descendant control is invalid.
        // Pattern/type-format/min-max mismatches are out of scope here.
        Pseudo::Invalid => is_invalid(dom, el),
        Pseudo::Valid => is_validity_candidate(dom, el) && !is_invalid(dom, el),
    }
}

fn is_invalid(dom: &Dom, el: NodeIdx) -> bool {
    match dom.name(el) {
        Some("input") => {
            supports_required(dom, el)
                && attr_of(dom, el, "disabled").is_none()
                && attr_of(dom, el, "required").is_some()
                && attr_of(dom, el, "value").unwrap_or("").is_empty()
        }
        Some("select") => {
            attr_of(dom, el, "disabled").is_none()
                && attr_of(dom, el, "required").is_some()
                && select_effective_value(dom, el).is_empty()
        }
        Some("textarea") => {
            attr_of(dom, el, "disabled").is_none()
                && attr_of(dom, el, "required").is_some()
                && element_text_content(dom, el).is_empty()
        }
        // The HTML Standard: a `fieldset`/`form` element matches `:invalid`
        // when it has a descendant that is both a candidate for constraint
        // validation and itself invalid -- it has no value of its own.
        Some("fieldset") | Some("form") => has_invalid_descendant(dom, el),
        _ => false,
    }
}

fn has_invalid_descendant(dom: &Dom, el: NodeIdx) -> bool {
    let mut pending: Vec<NodeIdx> = dom.children(el).collect();
    while let Some(node) = pending.pop() {
        if matches!(dom.name(node), Some("input" | "select" | "textarea")) && is_invalid(dom, node)
        {
            return true;
        }
        pending.extend(dom.children(node));
    }
    false
}

/// Elements `:valid`/`:invalid` apply to at all (candidates for constraint
/// validation, minus the always-barred input types and disabled controls).
fn is_validity_candidate(dom: &Dom, el: NodeIdx) -> bool {
    match dom.name(el) {
        Some("input") => {
            !attr_of(dom, el, "type").is_some_and(|kind| {
                ["hidden", "reset", "button"]
                    .iter()
                    .any(|excluded| kind.eq_ignore_ascii_case(excluded))
            }) && attr_of(dom, el, "disabled").is_none()
        }
        Some("select" | "textarea" | "button") => attr_of(dom, el, "disabled").is_none(),
        Some("output" | "fieldset" | "form") => true,
        _ => false,
    }
}

/// A `<select>`'s effective value: the `value` attribute (or else text
/// content) of its `selected` option, or of its first `<option>` descendant
/// if none is marked `selected`, or the empty string if it has none.
fn select_effective_value(dom: &Dom, el: NodeIdx) -> String {
    let mut options = Vec::new();
    collect_options(dom, el, &mut options);
    let chosen = options
        .iter()
        .find(|&&o| attr_of(dom, o, "selected").is_some())
        .or_else(|| options.first())
        .copied();
    match chosen {
        Some(opt) => attr_of(dom, opt, "value")
            .map(str::to_owned)
            .unwrap_or_else(|| element_text_content(dom, opt)),
        None => String::new(),
    }
}

fn collect_options(dom: &Dom, node: NodeIdx, out: &mut Vec<NodeIdx>) {
    for child in dom.children(node) {
        if dom.name(child) == Some("option") {
            out.push(child);
        } else {
            collect_options(dom, child, out);
        }
    }
}

fn element_text_content(dom: &Dom, el: NodeIdx) -> String {
    let mut out = String::new();
    collect_text(dom, el, &mut out);
    out
}

fn collect_text(dom: &Dom, node: NodeIdx, out: &mut String) {
    match &dom.get(node).kind {
        amazing_dom::NodeKind::Text { data } | amazing_dom::NodeKind::CDATASection { data } => {
            out.push_str(data)
        }
        _ => {}
    }
    for child in dom.children(node) {
        collect_text(dom, child, out);
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

/// Editable text controls are always `:focus-visible` when focused
/// (Selectors 4 §8.4.3): text-ish `<input>`, `<textarea>`, and contenteditable.
fn is_text_control(dom: &Dom, el: NodeIdx) -> bool {
    match dom.name(el) {
        Some("textarea") => true,
        Some("input") => !attr_of(dom, el, "type").is_some_and(|kind| {
            [
                "hidden", "range", "color", "submit", "image", "reset", "button",
                "checkbox", "radio", "file",
            ]
            .iter()
            .any(|excluded| kind.eq_ignore_ascii_case(excluded))
        }),
        _ => attr_of(dom, el, "contenteditable").is_some_and(|value| {
            value.is_empty()
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("plaintext-only")
        }),
    }
}

// DOM attributes are DOMStrings and can contain unpaired UTF16 surrogates.
// CSS escapes normalize invalid scalar values to U+FFFD, but that does not
// make a raw DOM surrogate equal to an actual replacement character.
fn attr_is(dom: &Dom, el: NodeIdx, name: &str, want: &str) -> bool {
    dom.attribute_ns_utf16(el,None,name).is_some_and(|actual|
        actual.iter().copied().eq(want.encode_utf16()))
}
fn selector_space(unit: &u16) -> bool { matches!(*unit,0x09|0x0a|0x0c|0x0d|0x20) }
fn class_list_contains(dom: &Dom, el: NodeIdx, class: &str) -> bool {
    let want:Vec<_>=class.encode_utf16().collect();
    !want.is_empty()&&dom.attribute_ns_utf16(el,None,"class").is_some_and(|actual|
        actual.split(selector_space).any(|word|word==want.as_slice()))
}
fn match_attr(dom: &Dom, el: NodeIdx, name: &str, op: AttrOp, value: &str, ci: bool) -> bool {
    let Some(mut actual)=dom.attribute_ns_utf16(el,None,name) else {return false;};
    let mut want:Vec<_>=value.encode_utf16().collect();
    if ci {
        for unit in actual.iter_mut().chain(want.iter_mut()) {
            if (0x41..=0x5a).contains(unit){*unit+=0x20;}
        }
    }
    match op {
        AttrOp::Exists=>true,
        AttrOp::Equals=>actual==want,
        AttrOp::Includes=>!want.is_empty()&&actual.split(selector_space).any(|word|word==want.as_slice()),
        AttrOp::DashMatch=>actual==want||(actual.starts_with(&want)&&actual.get(want.len())==Some(&0x2d)),
        AttrOp::Prefix=>!want.is_empty()&&actual.starts_with(&want),
        AttrOp::Suffix=>!want.is_empty()&&actual.ends_with(&want),
        AttrOp::Substring=>!want.is_empty()&&actual.windows(want.len()).any(|window|window==want.as_slice()),
    }
}

#[cfg(test)]
#[path = "selector_utf16_tests.rs"]
mod utf16_tests;

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
    fn focus_pseudo_classes_follow_dom_focus_state() {
        let mut dom = Dom::new();
        let doc = NodeIdx(0);
        let html = dom.create_element(doc, "html");
        let body = dom.create_element(html, "body");
        let div = dom.create_element(body, "div");
        let button = dom.create_element(div, "button");
        let input = dom.create_element(div, "input");
        let check = |dom: &Dom, node, selector| matches(dom, node, &parse_selector(selector).unwrap());

        // No focus recorded yet: none of the focus states match.
        assert!(!check(&dom, button, ":focus"));
        assert!(!check(&dom, body, ":focus-within"));

        // Programmatic focus without keyboard use: `:focus` matches, `:focus-visible`
        // does not, and ancestors match `:focus-within`.
        dom.set_active_element(doc, Some(button), false);
        assert!(check(&dom, button, ":focus"));
        assert!(!check(&dom, button, ":focus-visible"));
        assert!(check(&dom, div, ":focus-within"));
        assert!(check(&dom, body, ":focus-within"));
        assert!(!check(&dom, input, ":focus"));

        // Keyboard focus makes `:focus-visible` match. `set_active_element`
        // no-ops when the same element is already focused, so blur first.
        dom.set_active_element(doc, None, false);
        dom.set_active_element(doc, Some(button), true);
        assert!(check(&dom, button, ":focus-visible"));

        // A focused text control is always `:focus-visible`, even without the
        // keyboard-visibility flag.
        dom.set_active_element(doc, Some(input), false);
        assert!(check(&dom, input, ":focus"));
        assert!(check(&dom, input, ":focus-visible"));
        assert!(!check(&dom, button, ":focus-within"));
        assert!(check(&dom, div, ":focus-within"));

        // Moving focus off the subtree clears the ancestor state.
        dom.set_active_element(doc, None, false);
        assert!(!check(&dom, input, ":focus"));
        assert!(!check(&dom, div, ":focus-within"));
        assert!(!check(&dom, body, ":focus-within"));
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

    /// CSS-29: CSS Syntax 3 SS4.3.7 "consume an escaped code point", exercised
    /// through `#id` selectors the way WPT's own
    /// `dom/nodes/ParentNode-querySelector-escapes.html` does -- give an
    /// element the literal (already-decoded) `id`, then check whether an
    /// escaped selector spelling decodes to that same string.
    #[test]
    fn id_selector_escapes_decode_per_css_syntax_4_3_7() {
        fn el_with_id(id: &str) -> (Dom, NodeIdx) {
            let mut dom = Dom::new();
            let e = dom.create_element(NodeIdx(0), "span");
            dom.set_attrs(e, vec![("id".into(), id.into())]);
            (dom, e)
        }
        fn matches_selector(id: &str, selector: &str) -> bool {
            let (dom, e) = el_with_id(id);
            let parsed = parse_selector(selector)
                .unwrap_or_else(|| panic!("failed to parse selector: {selector:?}"));
            matches(&dom, e, &parsed)
        }

        // Hex escape terminated by a consumed trailing whitespace, or by a
        // non-hex character, or capped at 6 digits.
        assert!(matches_selector("0nextIsWhiteSpace", "#\\30 nextIsWhiteSpace"));
        assert!(matches_selector("0nextIsNotHexLetters", "#\\30nextIsNotHexLetters"));
        assert!(matches_selector("0connectHexMoreThan6Hex", "#\\000030connectHexMoreThan6Hex"));
        assert!(matches_selector("0spaceMoreThan6Hex", "#\\000030 spaceMoreThan6Hex"));

        // Special-replacement hex values: NUL, surrogates, out-of-range --
        // all become U+FFFD, never the raw value and never "not an escape".
        assert!(matches_selector("zero\u{fffd}", "#zero\\0"));
        assert!(!matches_selector("zero\u{0}", "#zero\\0"));
        assert!(matches_selector("zero\u{fffd}", "#zero\\000000"));
        // (There is no companion "does NOT match the raw surrogate" assertion:
        // a lone surrogate is not a valid Unicode scalar value, so Rust
        // cannot even express that `char`/`&str` literal to compare against.)
        assert!(matches_selector("\u{fffd}surrogateFirst", "#\\d83d surrogateFirst"));
        assert!(matches_selector("outOfRange\u{fffd}", "#outOfRange\\110000"));
        assert!(matches_selector("outOfRange\u{fffd}", "#outOfRange\\110030"));
        assert!(!matches_selector("outOfRange\u{30}", "#outOfRange\\110030"));
        // Exactly the maximum allowed code point is valid, not replaced.
        assert!(matches_selector("\u{10ffff}", "#\\10fFfF"));
        assert!(matches_selector("\u{10ffff}0", "#\\10fFfF0"));

        // A lone backslash right at EOF decodes to U+FFFD (a parse error,
        // but still a code point per spec), never a literal `\`: this must
        // NOT match an id containing a real trailing backslash.
        assert!(!matches_selector("eof\\", "#eof\\"));

        // Escaping any other character just yields that character literally.
        assert!(matches_selector(".comma", "#\\.comma"));
        assert!(matches_selector("hello", "#hel\\6Co"));
        assert!(matches_selector("&B", "#\\26 B"));

        // CRLF and form-feed normalize to one consumed whitespace after a
        // hex escape (CSS Syntax 3 SS3.3 input preprocessing), not two
        // separate characters that would otherwise truncate the ident.
        assert!(matches_selector("spaces", "#spac\\65\r\ns"));
        assert!(matches_selector("spaces", "#sp\\61\tc\\65\u{c}s"));

        // A literal NUL byte in the selector source itself (not an escape)
        // also becomes U+FFFD via the same preprocessing step.
        assert!(matches_selector("\u{fffd}", "#\u{0}"));
    }

    /// CSS-29: `:invalid`/`:valid`, Phase 2 attribute-only subset (no live
    /// form/widget state). Mirrors the shape of `dom/nodes/Element-closest.html`'s
    /// `do_test(":invalid", "test11", "test2")` fixture: a form containing a
    /// required-and-empty `<input>` sibling to a `<fieldset>` that itself
    /// contains a required `<select>` (validly filled) and a required-and-empty
    /// `<input>` -- both the form and the fieldset must report `:invalid`
    /// because they have an invalid descendant, even though neither has a
    /// "value" of its own.
    #[test]
    fn invalid_and_valid_pseudo_classes_read_required_and_empty_value() {
        let invalid = parse_selector(":invalid").unwrap();
        let valid = parse_selector(":valid").unwrap();

        // A plain required-and-empty input is :invalid, not :valid.
        {
            let mut dom = Dom::new();
            let input = dom.create_element(NodeIdx(0), "input");
            dom.set_attrs(input, vec![("required".into(), "".into())]);
            assert!(matches(&dom, input, &invalid));
            assert!(!matches(&dom, input, &valid));
        }
        // A required input WITH a value is :valid.
        {
            let mut dom = Dom::new();
            let input = dom.create_element(NodeIdx(0), "input");
            dom.set_attrs(
                input,
                vec![("required".into(), "".into()), ("value".into(), "x".into())],
            );
            assert!(matches(&dom, input, &valid));
            assert!(!matches(&dom, input, &invalid));
        }
        // A non-required input is always :valid (no pattern/type checks here).
        {
            let mut dom = Dom::new();
            let input = dom.create_element(NodeIdx(0), "input");
            assert!(matches(&dom, input, &valid));
            assert!(!matches(&dom, input, &invalid));
        }
        // A required select whose selected option has a non-empty implied
        // value (text content, no `value` attribute) is :valid.
        {
            let mut dom = Dom::new();
            let select = dom.create_element(NodeIdx(0), "select");
            dom.set_attrs(select, vec![("required".into(), "".into())]);
            let option = dom.create_element(select, "option");
            dom.set_attrs(option, vec![("selected".into(), "".into())]);
            dom.create_text(option, "Test11");
            assert!(matches(&dom, select, &valid));
            assert!(!matches(&dom, select, &invalid));
        }
        // fieldset/form propagate invalidity from a descendant control, per
        // the HTML Standard's own :valid/:invalid rule for those two
        // elements (they have no value of their own).
        {
            let mut dom = Dom::new();
            let form = dom.create_element(NodeIdx(0), "form");
            let required_input = dom.create_element(form, "input");
            dom.set_attrs(required_input, vec![("required".into(), "".into())]);
            let fieldset = dom.create_element(form, "fieldset");
            let select = dom.create_element(fieldset, "select");
            dom.set_attrs(select, vec![("required".into(), "".into())]);
            let option = dom.create_element(select, "option");
            dom.set_attrs(option, vec![("selected".into(), "".into())]);
            dom.create_text(option, "ok");
            let empty_input = dom.create_element(fieldset, "input");
            dom.set_attrs(empty_input, vec![("required".into(), "".into())]);

            assert!(matches(&dom, form, &invalid), "form has an invalid descendant");
            assert!(!matches(&dom, form, &valid));
            assert!(
                matches(&dom, fieldset, &invalid),
                "fieldset has an invalid descendant (the empty required input)"
            );
            assert!(!matches(&dom, fieldset, &valid));
            // The select itself is validly filled.
            assert!(matches(&dom, select, &valid));
            assert!(!matches(&dom, select, &invalid));
        }
    }
}
