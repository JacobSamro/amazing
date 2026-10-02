//! Bounded, std-only ECMA-262 RegExp matching (§22.2).
//!
//! Supports g/i/m/s/u/v/y, UTF-16 indices, classes, captures, backreferences,
//! greedy/lazy repetition, alternatives, boundaries and lookahead. The matcher
//! uses an explicit backtracking stack and a shared work budget, including
//! lookaheads and search attempts. Unsupported syntax is rejected explicitly.
//! See README.md for the deliberately incomplete ECMAScript surface.
#![forbid(unsafe_code)]

mod parser;
mod unicode_casefold;
mod unicode_props;
mod unicode_sets;
mod vm;

use std::fmt;
use std::ops::Range;

const MAX_PATTERN: usize = 16_384;
// A complete Unicode code-point corpus is 2,162,688 UTF-16 units. Keep
// enough bounded input space for it, including property complements.
const MAX_INPUT: usize = 4_194_304;
// Recursive-descent parser nesting depth (parser.rs's `atom`/`set_class`,
// one increment per `(` or nested `[...]` class): a real, finite guard
// against native stack overflow from a maliciously deep pattern, not a
// content-size proxy like MAX_INPUT/MAX_SLOTS. 64 was too tight for
// legitimate patterns -- built-ins/RegExp/S15.10.2.8_A3_T15.js and _T16.js
// nest 200 plain capturing groups (`(((...hello...)))`, a real-world-shaped
// stress pattern from a 2009 Mozilla bug report, not adversarial input) and
// threw "pattern nesting" under the old limit despite matching in a single
// linear pass with no backtracking. Raised to comfortably clear that with
// headroom while staying a small, bounded number of recursive call frames
// (a few hundred bytes each) well inside a normal thread's stack.
const MAX_DEPTH: usize = 512;
const MAX_CAPTURES: usize = 128;
const MAX_CODE: usize = 32_768;
const MAX_STEPS: usize = 8_388_608;
// Ambiguous repetitions still share bounded frame and slot storage.
// Deterministic greedy atoms represent all retry endpoints in one frame.
const MAX_STACK: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Syntax(String),
    Limit(&'static str),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(s) => f.write_str(s),
            Self::Limit(s) => write!(f, "RegExp resource limit: {s}"),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub has_indices: bool,
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub unicode_sets: bool,
    pub sticky: bool,
}
impl Flags {
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::parse_inner(text)
    }
    fn parse_utf16(text: &str) -> Result<Self, Error> {
        Self::parse_inner(text)
    }
    fn parse_inner(text: &str) -> Result<Self, Error> {
        let mut f = Self::default();
        for c in text.chars() {
            let slot = match c {
                'd' => &mut f.has_indices,
                'g' => &mut f.global,
                'i' => &mut f.ignore_case,
                'm' => &mut f.multiline,
                's' => &mut f.dot_all,
                'u' => &mut f.unicode,
                'v' => &mut f.unicode_sets,
                'y' => &mut f.sticky,
                _ => return Err(Error::Syntax(format!("unsupported RegExp flag {c:?}"))),
            };
            if *slot {
                return Err(Error::Syntax(format!("duplicate RegExp flag {c:?}")));
            }
            *slot = true;
        }
        if f.unicode && f.unicode_sets {
            return Err(Error::Syntax(
                "RegExp flags u and v cannot be combined".into(),
            ));
        }
        Ok(f)
    }
    fn unicode_mode(self) -> bool {
        self.unicode || self.unicode_sets
    }
    pub fn as_string(self) -> String {
        [
            (self.has_indices, 'd'),
            (self.global, 'g'),
            (self.ignore_case, 'i'),
            (self.multiline, 'm'),
            (self.dot_all, 's'),
            (self.unicode, 'u'),
            (self.unicode_sets, 'v'),
            (self.sticky, 'y'),
        ]
        .into_iter()
        .filter_map(|(yes, c)| yes.then_some(c))
        .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Regex {
    pattern: String,
    pattern_utf16: Box<[u16]>,
    flags: Flags,
    program: vm::Program,
    captures: usize,
    group_names: Box<[(Box<str>, usize)]>,
}
impl PartialEq for Regex {
    fn eq(&self, other: &Self) -> bool {
        self.pattern_utf16 == other.pattern_utf16 && self.flags == other.flags
    }
}
impl Eq for Regex {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// UTF-16 code-unit range of the entire match.
    pub range: Range<usize>,
    /// Capture 0 is the entire match; nonparticipating captures are None.
    pub captures: Vec<Option<Range<usize>>>,
}
impl Regex {
    pub fn new(pattern: &str, flags: &str) -> Result<Self, Error> {
        if pattern.len() > MAX_PATTERN {
            return Err(Error::Limit("pattern bytes"));
        }
        let units: Vec<u16> = pattern.encode_utf16().collect();
        Self::compile_utf16(&units, Flags::parse(flags)?)
    }
    /// Compile a pattern from its exact UTF-16 code units, including lone surrogates.
    pub fn new_utf16(pattern: &[u16], flags: &str) -> Result<Self, Error> {
        if pattern.len() > MAX_PATTERN {
            return Err(Error::Limit("pattern code units"));
        }
        Self::compile_utf16(pattern, Flags::parse_utf16(flags)?)
    }
    fn compile_utf16(pattern: &[u16], flags: Flags) -> Result<Self, Error> {
        let (ast, captures, group_names) = parser::parse_utf16(pattern, flags)?;
        let program = vm::compile(&ast, captures, flags)?;
        Ok(Self {
            pattern: String::from_utf16_lossy(pattern),
            pattern_utf16: pattern.into(),
            flags,
            program,
            captures,
            group_names: group_names.into(),
        })
    }
    /// Named capturing groups in declaration order, each with its 1-based
    /// capture index; empty when the pattern has none.
    pub fn group_names(&self) -> &[(Box<str>, usize)] {
        &self.group_names
    }
    pub fn flags(&self) -> Flags {
        self.flags
    }
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
    /// The exact pattern. Use this when it may contain lone UTF-16 surrogates.
    pub fn pattern_utf16(&self) -> &[u16] {
        &self.pattern_utf16
    }
    pub fn capture_count(&self) -> usize {
        self.captures
    }
    /// Search from a UTF-16 index. `y` restricts matching to exactly that index.
    /// The JS binding owns lastIndex and global iteration; this object is pure.
    pub fn exec(&self, input: &str, start: usize) -> Result<Option<Match>, Error> {
        if input.len() > MAX_INPUT * 3 {
            return Err(Error::Limit("input bytes"));
        }
        let units: Vec<u16> = input.encode_utf16().collect();
        self.exec_utf16(&units, start)
    }
    /// Lossless entry point for hosts with UTF-16 strings, including lone surrogates.
    pub fn exec_utf16(&self, input: &[u16], start: usize) -> Result<Option<Match>, Error> {
        if input.len() > MAX_INPUT {
            return Err(Error::Limit("input code units"));
        }
        self.matcher_utf16(input)?.exec(start)
    }
    /// A single operation's input and shared budget, reused by global iteration.
    pub fn matcher_utf16<'a>(&'a self, input: &'a [u16]) -> Result<Matcher<'a>, Error> {
        if input.len() > MAX_INPUT {
            return Err(Error::Limit("input code units"));
        }
        Ok(Matcher {
            regex: self,
            input,
            budget: vm::Budget::new(),
            failed: false,
        })
    }
}

pub struct Matcher<'a> {
    regex: &'a Regex,
    input: &'a [u16],
    budget: vm::Budget,
    failed: bool,
}
impl Matcher<'_> {
    pub fn exec(&mut self, start: usize) -> Result<Option<Match>, Error> {
        if self.failed {
            return Err(Error::Limit("matcher previously exhausted"));
        }
        let result = vm::search(
            &self.regex.program,
            self.input,
            start,
            self.regex.flags,
            self.regex.captures,
            &mut self.budget,
        );
        self.failed = result.is_err();
        result
    }
}

pub fn advance_index(input: &[u16], index: usize, unicode: bool) -> usize {
    index.saturating_add(
        if unicode
            && input
                .get(index)
                .is_some_and(|u| (0xD800..=0xDBFF).contains(u))
            && input
                .get(index + 1)
                .is_some_and(|u| (0xDC00..=0xDFFF).contains(u))
        {
            2
        } else {
            1
        },
    )
}
fn point(input: &[u16], index: usize, unicode: bool) -> Option<(u32, usize)> {
    let first = *input.get(index)? as u32;
    let end = advance_index(input, index, unicode);
    Some((
        if end == index + 2 {
            0x10000 + ((first - 0xD800) << 10) + (input[index + 1] as u32 - 0xDC00)
        } else {
            first
        },
        end,
    ))
}
fn line(c: u32) -> bool {
    matches!(c, 10 | 13 | 0x2028 | 0x2029)
}
fn space(c: u32) -> bool {
    matches!(c, 9..=13|32|0xA0|0x1680|0x2000..=0x200A|0x2028|0x2029|0x202F|0x205F|0x3000|0xFEFF)
}
fn canonical(c: u32, flags: Flags) -> u32 {
    if !flags.ignore_case {
        return c;
    }
    if flags.unicode_mode() {
        return unicode_casefold::PAIRS
            .binary_search_by_key(&c, |p| p.0)
            .map(|i| unicode_casefold::PAIRS[i].1)
            .unwrap_or(c);
    }
    let Some(ch) = char::from_u32(c) else {
        return c;
    };
    let mut upper = ch.to_uppercase();
    let u = upper.next().unwrap_or(ch) as u32;
    if upper.next().is_some() || u > 0xFFFF || (c >= 128 && u < 128) {
        c
    } else {
        u
    }
}
fn word(c: u32, flags: Flags) -> bool {
    let c = if flags.unicode_mode() && flags.ignore_case {
        canonical(c, flags)
    } else {
        c
    };
    matches!(c, 48..=57|65..=90|95|97..=122)
}

#[cfg(test)]
mod tests;
