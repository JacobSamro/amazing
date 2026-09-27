//! WHATWG HTML tokenizer (§13.2.5 "Tokenization").
//!
//! State names mirror the spec (`Data state`, `Tag open state`, …) because
//! the spec's error-recovery behavior is the interop contract. Deviations
//! from full spec (all deliberate, all listed in lib.rs):
//! - encoding sniffing happens in the caller; input is lossy UTF-8 `char`s
//!
//! Input is a `Vec<char>` — one `char` = one scalar value, so cursor
//! arithmetic is per-character and entity names (ASCII) match 1 byte = 1 char.

use std::collections::VecDeque;

use crate::entities::MAX_NAME_LEN;

/// A tokenizer token, per §13.2.5 "Emitting tokens".
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
        /// `true` when the doctype was malformed enough to force quirks mode.
        force_quirks: bool,
    },
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
    /// One or more characters, already entity-decoded.
    Characters(String),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Data,
    PlainText,
    EndTagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValueDouble,
    AttrValueSingle,
    AttrValueUnquoted,
    AfterAttrValueQuoted,
    SelfClosingStart,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    BogusComment,
    CdataSection,
    CdataBracket,
    CdataEnd,
    ProcessingInstructionOpen,
    ProcessingInstructionTarget,
    AfterProcessingInstructionTarget,
    ProcessingInstructionData,
    ProcessingInstructionQuestionable,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDouble,
    DoctypePublicIdentifierSingle,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDouble,
    DoctypeSystemIdentifierSingle,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    RawText,
    RawTextLessThan,
    RawTextEndTagOpen,
    RawTextEndTagName,
    ScriptData,
    ScriptLessThan,
    ScriptEndTagOpen,
    ScriptEndTagName,
    ScriptEscapeStart,
    ScriptEscapeStartDash,
    ScriptEscaped,
    ScriptEscapedDash,
    ScriptEscapedDashDash,
    ScriptEscapedLessThan,
    ScriptEscapedEndTagOpen,
    ScriptEscapedEndTagName,
    ScriptDoubleEscapeStart,
    ScriptDoubleEscaped,
    ScriptDoubleEscapedDash,
    ScriptDoubleEscapedDashDash,
    ScriptDoubleEscapedLessThan,
    ScriptDoubleEscapeEnd,
    RcData,
    RcLessThan,
    RcEndTagOpen,
    RcEndTagName,
    /// Terminal state after EOF has been fully processed.
    Done,
}

/// Attribute under construction; committed into the tag by
/// [`Tokenizer::finish_attr`] at attribute/tag boundaries.
#[derive(Default)]
struct PendingAttr {
    name: String,
    value: String,
}

impl PendingAttr {
    fn push_name(&mut self, c: char) {
        self.name.push(c);
    }

    fn push_value(&mut self, c: char) {
        self.value.push(c);
    }

    fn push_value_str(&mut self, s: &str) {
        self.value.push_str(s);
    }
}

/// One builder shared by start and end tags: end tags drop their attrs at
/// emit time (§13.2.5.32 attributes on end tokens are a parse error).
struct TagBuilder {
    name: String,
    attrs: Vec<(String, String)>,
}

impl TagBuilder {
    fn new() -> Self {
        Self { name: String::new(), attrs: Vec::new() }
    }

    /// WHATWG §13.2.5.33: duplicate attribute = parse error; first wins.
    fn push_attr(&mut self, name: String, value: String) {
        if !self.attrs.iter().any(|(n, _)| *n == name) {
            self.attrs.push((name, value));
        }
    }
}

pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    /// Position of the `<` that opened the tag currently being scanned;
    /// used to reconstruct literal source text when EOF hits mid-tag.
    tag_start: usize,
    state: State,
    queue: VecDeque<Token>,
    /// Character accumulation for Data/RawText/RcData states; flushed as a
    /// single `Characters` token whenever any other token is emitted.
    text_buf: String,
    tag: Option<TagBuilder>,
    is_end_tag: bool,
    pending: PendingAttr,
    comment_buf: String,
    processing_target: String,
    processing_data: String,
    doctype_name: Option<String>,
    doctype_public_id: Option<String>,
    doctype_system_id: Option<String>,
    doctype_force_quirks: bool,
    /// End-tag name we're looking for while in a text state.
    raw_end: String,
    /// The script double-escape delimiter's case-folded name.
    script_temporary: String,
    eof_sent: bool,
    input_finished: bool,
    /// Updated by the tree builder after each token from its current node.
    allow_cdata: bool,
}

fn is_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{000C}' | ' ')
}

fn is_alpha(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// Windows-1252 replacement table for numeric references in the C1 range
/// (§13.2.5.72). Index = code point − 0x80; identity entries included so the
/// lookup is total over 0x80..=0x9F (32 entries).
const C1_REPLACEMENTS: [u32; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, // 80-87
    0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, // 88-8F
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, // 90-97
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178, // 98-9F
];

impl Tokenizer {
    pub fn new(input: Vec<char>) -> Self {
        Self {
            input,
            pos: 0,
            tag_start: 0,
            state: State::Data,
            queue: VecDeque::new(),
            text_buf: String::new(),
            tag: None,
            is_end_tag: false,
            pending: PendingAttr::default(),
            comment_buf: String::new(),
            processing_target: String::new(),
            processing_data: String::new(),
            doctype_name: None,
            doctype_public_id: None,
            doctype_system_id: None,
            doctype_force_quirks: false,
            raw_end: String::new(),
            script_temporary: String::new(),
            eof_sent: false,
            input_finished: true,
            allow_cdata: false,
        }
    }

    pub(crate) fn streaming() -> Self {
        let mut tokenizer = Self::new(Vec::new());
        tokenizer.input_finished = false;
        tokenizer
    }

    pub(crate) fn push_chars(&mut self, chars: impl IntoIterator<Item = char>) {
        assert!(!self.input_finished, "cannot feed a finished tokenizer");
        self.input.extend(chars);
    }

    pub(crate) fn finish_input(&mut self) {
        self.input_finished = true;
    }

    /// HTML-31: `document.write`'s real behavior while the parser is paused
    /// at a blocking script -- insert text into the input stream just before
    /// the current position (the WHATWG "insertion point"), not appended at
    /// the end like `push_chars` (which is for network-streamed input that
    /// always arrives after what is already buffered). Safe to call between
    /// `next_token`/`next_available` calls only; this engine only pauses
    /// right after a tag closes, in `State::Data`, so there is never a
    /// half-built tag/comment/reference straddling the splice point.
    pub(crate) fn insert_chars_at_cursor(&mut self, chars: impl IntoIterator<Item = char>) {
        let tail = self.input.split_off(self.pos);
        self.input.extend(chars);
        self.input.extend(tail);
    }

    pub(crate) fn set_allow_cdata(&mut self, allow: bool) {
        self.allow_cdata = allow;
    }

    pub(crate) fn buffered_input_scalars(&self) -> usize {
        self.input.len()
    }

    /// Drop consumed scalars, retaining the source of a tag still being
    /// constructed because EOF recovery replays it literally.
    pub(crate) fn compact_input(&mut self) {
        let replaying_tag = matches!(
            self.state,
            State::EndTagOpen | State::TagName | State::BeforeAttrName
                | State::AttrName | State::AfterAttrName | State::BeforeAttrValue
                | State::AttrValueDouble | State::AttrValueSingle
                | State::AttrValueUnquoted | State::AfterAttrValueQuoted
                | State::SelfClosingStart | State::RawTextLessThan
                | State::RawTextEndTagOpen | State::RawTextEndTagName
                | State::RcLessThan | State::RcEndTagOpen | State::RcEndTagName
                | State::ScriptLessThan | State::ScriptEndTagOpen | State::ScriptEndTagName
                | State::ScriptEscapedLessThan | State::ScriptEscapedEndTagOpen
                | State::ScriptEscapedEndTagName
        );
        let drop = if replaying_tag { self.tag_start.min(self.pos) } else { self.pos };
        if drop > 0 {
            self.input = self.input.split_off(drop);
            self.pos -= drop;
            self.tag_start = self.tag_start.saturating_sub(drop);
        }
    }

    // ---- cursor helpers ----

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.input.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    /// Case-insensitive ASCII match of `s` starting `off` chars past the
    /// cursor (no consumption).
    fn ci_at(&self, off: usize, s: &str) -> bool {
        for (i, want) in s.chars().enumerate() {
            match self.peek_at(off + i) {
                Some(have) if have.is_ascii() && want.eq_ignore_ascii_case(&have) => {},
                _ => return false,
            }
        }
        true
    }

    // ---- token queue ----

    fn flush_text(&mut self) {
        if !self.text_buf.is_empty() {
            let data = std::mem::take(&mut self.text_buf);
            self.queue.push_back(Token::Characters(data));
        }
    }

    fn emit(&mut self, tok: Token) {
        self.flush_text();
        self.queue.push_back(tok);
    }

    /// Enters the tokenizer state requested by tree construction after it
    /// has inserted an HTML text element. Foreign elements with the same
    /// local name never call this hook.
    pub(crate) fn enter_text_element(&mut self, name: &str) {
        match name {
            "plaintext" => self.state = State::PlainText,
            "textarea" | "title" => {
                self.raw_end = name.to_owned();
                self.state = State::RcData;
            },
            "script" => {
                self.raw_end = name.to_owned();
                self.state = State::ScriptData;
            },
            "style" | "xmp" | "iframe" | "noembed"
            | "noframes" | "noscript" => {
                self.raw_end = name.to_owned();
                self.state = State::RawText;
            },
            _ => {},
        }
    }

    /// A fragment context changes the initial state without creating an
    /// appropriate end tag: the context start tag was never tokenized.
    pub(crate) fn enter_fragment_text_context(&mut self, name: &str) {
        self.raw_end.clear();
        self.state = match name {
            "plaintext" => State::PlainText,
            "textarea" | "title" => State::RcData,
            "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => State::RawText,
            "script" => State::ScriptData,
            _ => return,
        };
    }

    fn processing_target_disallowed(&self) -> bool {
        self.processing_target.eq_ignore_ascii_case("xml")
            || self
                .processing_target
                .eq_ignore_ascii_case("xml-stylesheet")
    }

    fn processing_to_bogus_comment(&mut self) {
        self.comment_buf.clear();
        self.comment_buf.push('?');
        self.comment_buf.push_str(&self.processing_target);
        self.state = State::BogusComment;
    }

    fn emit_processing_instruction(&mut self) {
        let target = std::mem::take(&mut self.processing_target);
        let data = std::mem::take(&mut self.processing_data);
        self.emit(Token::ProcessingInstruction { target, data });
        self.state = State::Data;
    }

    /// Public entry point per §13.2.5 "Tokenization". Runs states until a
    /// token is available or EOF is fully processed; repeated calls after
    /// EOF keep returning [`Token::Eof`].
    pub fn next_token(&mut self) -> Token {
        self.next_available().unwrap_or(Token::Eof)
    }

    /// Returns `None` while a chunk is exhausted but more input may arrive.
    /// Unlike `next_token`, this emits EOF only once after `finish_input`.
    pub(crate) fn next_available(&mut self) -> Option<Token> {
        if let Some(t) = self.queue.pop_front() {
            return Some(t);
        }
        if !self.eof_sent {
            self.run();
            if let Some(t) = self.queue.pop_front() {
                return Some(t);
            }
            if self.input_finished && self.state == State::Done {
                self.eof_sent = true;
                return Some(Token::Eof);
            }
        }
        None
    }

    fn needs_more_input(&self) -> bool {
        match self.state {
            State::Data | State::RcData => match self.peek() {
                Some('&') => self.reference_needs_more(),
                Some('<') if self.state == State::Data => self.tag_open_needs_more(),
                _ => false,
            },
            State::AttrValueDouble | State::AttrValueSingle | State::AttrValueUnquoted => {
                self.peek() == Some('&') && self.reference_needs_more()
            },
            State::AfterDoctypeName if self.peek().is_some_and(is_alpha) => {
                let tail = &self.input[self.pos..];
                ["public", "system"].iter().any(|keyword| {
                    tail.len() < keyword.len()
                        && tail.iter().zip(keyword.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b))
                })
            },
            _ => false,
        }
    }

    fn tag_open_needs_more(&self) -> bool {
        let tail = &self.input[self.pos + 1..];
        let Some(&first) = tail.first() else { return true; };
        if first != '!' { return false; }
        let markup = &tail[1..];
        let comment = ['-', '-'];
        let doctype = ['d', 'o', 'c', 't', 'y', 'p', 'e'];
        let cdata = ['[', 'C', 'D', 'A', 'T', 'A', '['];
        (markup.len() < comment.len() && markup == &comment[..markup.len()])
            || (markup.len() < doctype.len()
                && markup.iter().zip(&doctype).all(|(a, b)| a.eq_ignore_ascii_case(b)))
            || (markup.len() < cdata.len() && markup == &cdata[..markup.len()])
    }

    fn reference_needs_more(&self) -> bool {
        let tail = &self.input[self.pos + 1..];
        if tail.is_empty() { return true; }
        if tail[0] == '#' {
            let mut i = 1;
            let hex = tail.get(i).is_some_and(|c| *c == 'x' || *c == 'X');
            if hex { i += 1; }
            if i == tail.len() { return true; }
            let base = if hex { 16 } else { 10 };
            while i < tail.len() && tail[i].to_digit(base).is_some() { i += 1; }
            return i == tail.len();
        }
        let mut i = 0;
        while i < tail.len() && i < MAX_NAME_LEN && tail[i].is_ascii_alphanumeric() { i += 1; }
        i == tail.len() && i < MAX_NAME_LEN
    }

    /// Drives states until input is exhausted and everything is emitted.
    fn run(&mut self) {
        loop {
            if !self.queue.is_empty() {
                return;
            }
            if self.pos >= self.input.len() {
                if self.input_finished { self.finish_eof(); }
                else { self.flush_text(); }
                return;
            }
            if !self.input_finished && self.needs_more_input() {
                self.flush_text();
                return;
            }
            match self.state {
                State::Done => return,

                State::Data => match self.peek() {
                    Some('&') => {
                        self.bump();
                        match self.consume_reference(false) {
                            Some(s) => self.text_buf.push_str(&s),
                            None => self.text_buf.push('&'),
                        }
                    },
                    Some('<') => self.tag_open_dispatch(),
                    Some('\0') => {
                        self.pos += 1;
                        self.text_buf.push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.text_buf.push(c);
                    },
                    None => unreachable!("run() checked EOF"),
                },

                State::PlainText => match self.bump() {
                    Some('\0') => self.text_buf.push('\u{FFFD}'),
                    Some(c) => self.text_buf.push(c),
                    None => {},
                },

                State::CdataSection => match self.peek() {
                    Some(']') => { self.pos += 1; self.state = State::CdataBracket; },
                    Some('\0') => { self.pos += 1; self.text_buf.push('\u{FFFD}'); },
                    Some(c) => { self.pos += 1; self.text_buf.push(c); },
                    None => {},
                },
                State::CdataBracket => {
                    if self.peek() == Some(']') {
                        self.pos += 1;
                        self.state = State::CdataEnd;
                    } else {
                        self.text_buf.push(']');
                        self.state = State::CdataSection;
                    }
                },
                State::CdataEnd => match self.peek() {
                    Some(']') => { self.pos += 1; self.text_buf.push(']'); },
                    Some('>') => { self.pos += 1; self.state = State::Data; },
                    _ => { self.text_buf.push_str("]]"); self.state = State::CdataSection; },
                },

                State::EndTagOpen => match self.peek() {
                    None => {}, // finish_eof handles it via slice
                    Some(c) if is_alpha(c) => {
                        self.tag = Some(TagBuilder::new());
                        self.is_end_tag = true;
                        self.state = State::TagName;
                        // reconsume the letter in TagName
                    },
                    Some('>') => {
                        // "</>": parse error, whole thing ignored.
                        self.pos += 1;
                        self.state = State::Data;
                    },
                    Some(_) => {
                        // parse error: bogus comment, reconsume current char.
                        self.comment_buf.clear();
                        self.state = State::BogusComment;
                    },
                },

                State::TagName => match self.peek() {
                    None => {}, // finish_eof reconstructs the literal tag
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                        self.state = State::BeforeAttrName;
                    },
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::SelfClosingStart;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.tag.as_mut().unwrap().name.push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.tagname_push(c.to_ascii_lowercase());
                    },
                },

                State::BeforeAttrName => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                    },
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::SelfClosingStart;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some('=') => {
                        // parse error: attr name starting with '='
                        self.attr_start(Some('='));
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.attr_start(Some(c.to_ascii_lowercase()));
                    },
                },

                State::AttrName => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                        self.state = State::AfterAttrName;
                    },
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::SelfClosingStart;
                    },
                    Some('=') => {
                        self.pos += 1;
                        self.state = State::BeforeAttrValue;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.pending.push_name('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.pending.push_name(c.to_ascii_lowercase());
                    },
                },

                State::AfterAttrName => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                    },
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::SelfClosingStart;
                    },
                    Some('=') => {
                        self.pos += 1;
                        self.state = State::BeforeAttrValue;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.finish_attr();
                        self.pending.push_name(c.to_ascii_lowercase());
                        self.state = State::AttrName;
                    },
                },

                State::BeforeAttrValue => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                    },
                    Some('"') => {
                        self.pos += 1;
                        self.state = State::AttrValueDouble;
                    },
                    Some('\'') => {
                        self.pos += 1;
                        self.state = State::AttrValueSingle;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.pending.push_value(c);
                        self.state = State::AttrValueUnquoted;
                    },
                },

                State::AttrValueDouble => match self.peek() {
                    None => {},
                    Some('"') => {
                        self.pos += 1;
                        self.state = State::AfterAttrValueQuoted;
                    },
                    Some('&') => self.attr_value_entity(),
                    Some('\0') => {
                        self.pos += 1;
                        self.pending.push_value('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.pending.push_value(c);
                    },
                },

                State::AttrValueSingle => match self.peek() {
                    None => {},
                    Some('\'') => {
                        self.pos += 1;
                        self.state = State::AfterAttrValueQuoted;
                    },
                    Some('&') => self.attr_value_entity(),
                    Some('\0') => {
                        self.pos += 1;
                        self.pending.push_value('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.pending.push_value(c);
                    },
                },

                State::AttrValueUnquoted => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                        self.finish_attr();
                        self.state = State::BeforeAttrName;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some('&') => self.attr_value_entity(),
                    Some('\0') => {
                        self.pos += 1;
                        self.pending.push_value('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.pending.push_value(c);
                    },
                },

                State::AfterAttrValueQuoted => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                        self.finish_attr();
                        self.state = State::BeforeAttrName;
                    },
                    Some('/') => {
                        self.pos += 1;
                        self.finish_attr();
                        self.state = State::SelfClosingStart;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(false);
                    },
                    Some(_) => {
                        // missing whitespace between attributes: reconsume
                        // as a new attribute name.
                        let c = self.bump().unwrap();
                        self.finish_attr();
                        self.pending.push_name(c.to_ascii_lowercase());
                        self.state = State::AttrName;
                    },
                },

                State::SelfClosingStart => match self.peek() {
                    None => {},
                    Some('>') => {
                        self.pos += 1;
                        self.emit_tag(true);
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.finish_attr();
                        self.pending.push_name(c.to_ascii_lowercase());
                        self.state = State::AttrName;
                    },
                },

                State::CommentStart => match self.peek() {
                    None => {}, // finish_eof emits what we have
                    Some('-') => {
                        self.pos += 1;
                        self.state = State::CommentStartDash;
                    },
                    Some('>') => {
                        // abruptly closed comment: "<!-->"
                        self.pos += 1;
                        let data = std::mem::take(&mut self.comment_buf);
                        self.emit(Token::Comment(data));
                        self.state = State::Data;
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push(c);
                        self.state = State::Comment;
                    },
                },

                State::CommentStartDash => match self.peek() {
                    None => {}, // finish_eof appends nothing here
                    Some('-') => {
                        self.pos += 1;
                        self.state = State::CommentEnd;
                    },
                    Some('>') => {
                        self.pos += 1;
                        let data = std::mem::take(&mut self.comment_buf);
                        self.emit(Token::Comment(data));
                        self.state = State::Data;
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push('-');
                        self.comment_buf.push(c);
                        self.state = State::Comment;
                    },
                },

                State::Comment => match self.peek() {
                    None => {},
                    Some('-') => {
                        self.pos += 1;
                        self.state = State::CommentEndDash;
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push(c);
                    },
                },

                State::CommentEndDash => match self.peek() {
                    None => {}, // finish_eof re-adds the '-'
                    Some('-') => {
                        self.pos += 1;
                        self.state = State::CommentEnd;
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push('-');
                        self.comment_buf.push(c);
                        self.state = State::Comment;
                    },
                },

                State::CommentEnd => match self.peek() {
                    None => {}, // finish_eof re-adds "--"
                    Some('>') => {
                        self.pos += 1;
                        let data = std::mem::take(&mut self.comment_buf);
                        self.emit(Token::Comment(data));
                        self.state = State::Data;
                    },
                    Some('!') => {
                        self.pos += 1;
                        self.state = State::CommentEndBang;
                    },
                    Some('-') => {
                        self.pos += 1;
                        self.comment_buf.push('-');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push_str("--");
                        self.comment_buf.push(c);
                        self.state = State::Comment;
                    },
                },

                State::CommentEndBang => match self.peek() {
                    None => {}, // finish_eof re-adds "--!"
                    Some('-') => {
                        self.pos += 1;
                        self.state = State::CommentEndDash;
                    },
                    Some('>') => {
                        // "--!>" closes an incorrectly-closed comment.
                        self.pos += 1;
                        let data = std::mem::take(&mut self.comment_buf);
                        self.emit(Token::Comment(data));
                        self.state = State::Data;
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push_str("--!");
                        self.comment_buf.push(c);
                        self.state = State::Comment;
                    },
                },

                State::BogusComment => match self.peek() {
                    None => {},
                    Some('>') => {
                        self.pos += 1;
                        let data = std::mem::take(&mut self.comment_buf);
                        self.emit(Token::Comment(data));
                        self.state = State::Data;
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.comment_buf.push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.comment_buf.push(c);
                    },
                },

                State::ProcessingInstructionOpen => match self.peek() {
                    None => {},
                    Some(c) if is_alpha(c) || c == '_' => {
                        self.processing_target.push(c);
                        self.pos += 1;
                        self.state = State::ProcessingInstructionTarget;
                    },
                    Some(_) => self.processing_to_bogus_comment(),
                },

                State::ProcessingInstructionTarget => match self.peek() {
                    None => {},
                    Some(c) if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') => {
                        self.processing_target.push(c);
                        self.pos += 1;
                    },
                    Some(c) if is_ws(c) => {
                        if self.processing_target_disallowed() {
                            self.processing_to_bogus_comment();
                        } else {
                            self.pos += 1;
                            self.state = State::AfterProcessingInstructionTarget;
                        }
                    },
                    Some('?') => {
                        if self.processing_target_disallowed() {
                            self.processing_to_bogus_comment();
                        } else {
                            self.pos += 1;
                            self.state = State::ProcessingInstructionQuestionable;
                        }
                    },
                    Some('>') => {
                        if self.processing_target_disallowed() {
                            self.processing_to_bogus_comment();
                        } else {
                            self.pos += 1;
                            self.emit_processing_instruction();
                        }
                    },
                    Some(_) => self.processing_to_bogus_comment(),
                },

                State::AfterProcessingInstructionTarget => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => self.pos += 1,
                    Some('?') => {
                        self.pos += 1;
                        self.state = State::ProcessingInstructionQuestionable;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_processing_instruction();
                    },
                    Some(_) => self.state = State::ProcessingInstructionData,
                },

                State::ProcessingInstructionData => match self.peek() {
                    None => {},
                    Some('?') => {
                        self.pos += 1;
                        self.state = State::ProcessingInstructionQuestionable;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_processing_instruction();
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.processing_data.push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.processing_data.push(c);
                    },
                },

                State::ProcessingInstructionQuestionable => match self.peek() {
                    None => {},
                    Some('>') => {
                        self.pos += 1;
                        self.emit_processing_instruction();
                    },
                    Some(_) => {
                        self.processing_data.push('?');
                        self.state = State::ProcessingInstructionData;
                    },
                },

                State::BeforeDoctypeName => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_doctype(true);
                        self.state = State::Data;
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.doctype_name =
                            Some(c.to_ascii_lowercase().to_string());
                        self.state = State::DoctypeName;
                    },
                },

                State::DoctypeName => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                        self.state = State::AfterDoctypeName;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_doctype(false);
                        self.state = State::Data;
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.doctype_name.as_mut().unwrap().push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.doctype_name
                            .as_mut()
                            .unwrap()
                            .push(c.to_ascii_lowercase());
                    },
                },

                State::AfterDoctypeName => match self.peek() {
                    None => {},
                    Some(c) if is_ws(c) => {
                        self.pos += 1;
                    },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_doctype(false);
                        self.state = State::Data;
                    },
                    Some(c) if is_alpha(c) && self.ci_at(0, "public") => {
                        self.pos += 6;
                        self.state = State::AfterDoctypePublicKeyword;
                    },
                    Some(c) if is_alpha(c) && self.ci_at(0, "system") => {
                        self.pos += 6;
                        self.state = State::AfterDoctypeSystemKeyword;
                    },
                    Some(_) => {
                        self.doctype_force_quirks = true;
                        self.state = State::BogusDoctype;
                    },
                },

                State::AfterDoctypePublicKeyword | State::BeforeDoctypePublicIdentifier => {
                    match self.peek() {
                        Some(c) if is_ws(c) => {
                            self.pos += 1;
                            self.state = State::BeforeDoctypePublicIdentifier;
                        },
                        Some('"') | Some('\'') => {
                            let quote = self.bump().unwrap();
                            self.doctype_public_id = Some(String::new());
                            self.state = if quote == '"' { State::DoctypePublicIdentifierDouble }
                                else { State::DoctypePublicIdentifierSingle };
                        },
                        Some('>') => {
                            self.pos += 1;
                            self.emit_doctype(true);
                            self.state = State::Data;
                        },
                        Some(_) => {
                            self.doctype_force_quirks = true;
                            self.state = State::BogusDoctype;
                        },
                        None => {},
                    }
                },
                State::DoctypePublicIdentifierDouble | State::DoctypePublicIdentifierSingle => {
                    let quote = if self.state == State::DoctypePublicIdentifierDouble { '"' } else { '\'' };
                    match self.bump() {
                        Some(c) if c == quote => self.state = State::AfterDoctypePublicIdentifier,
                        Some('>') => { self.emit_doctype(true); self.state = State::Data; },
                        Some('\0') => self.doctype_public_id.as_mut().unwrap().push('\u{FFFD}'),
                        Some(c) => self.doctype_public_id.as_mut().unwrap().push(c),
                        None => {},
                    }
                },
                State::AfterDoctypePublicIdentifier | State::BetweenDoctypePublicAndSystemIdentifiers => {
                    match self.peek() {
                        Some(c) if is_ws(c) => {
                            self.pos += 1;
                            self.state = State::BetweenDoctypePublicAndSystemIdentifiers;
                        },
                        Some('>') => {
                            self.pos += 1;
                            self.emit_doctype(false);
                            self.state = State::Data;
                        },
                        Some('"') | Some('\'') => {
                            let quote = self.bump().unwrap();
                            self.doctype_system_id = Some(String::new());
                            self.state = if quote == '"' { State::DoctypeSystemIdentifierDouble }
                                else { State::DoctypeSystemIdentifierSingle };
                        },
                        Some(_) => {
                            self.doctype_force_quirks = true;
                            self.state = State::BogusDoctype;
                        },
                        None => {},
                    }
                },
                State::AfterDoctypeSystemKeyword | State::BeforeDoctypeSystemIdentifier => {
                    match self.peek() {
                        Some(c) if is_ws(c) => {
                            self.pos += 1;
                            self.state = State::BeforeDoctypeSystemIdentifier;
                        },
                        Some('"') | Some('\'') => {
                            let quote = self.bump().unwrap();
                            self.doctype_system_id = Some(String::new());
                            self.state = if quote == '"' { State::DoctypeSystemIdentifierDouble }
                                else { State::DoctypeSystemIdentifierSingle };
                        },
                        Some('>') => {
                            self.pos += 1;
                            self.emit_doctype(true);
                            self.state = State::Data;
                        },
                        Some(_) => {
                            self.doctype_force_quirks = true;
                            self.state = State::BogusDoctype;
                        },
                        None => {},
                    }
                },
                State::DoctypeSystemIdentifierDouble | State::DoctypeSystemIdentifierSingle => {
                    let quote = if self.state == State::DoctypeSystemIdentifierDouble { '"' } else { '\'' };
                    match self.bump() {
                        Some(c) if c == quote => self.state = State::AfterDoctypeSystemIdentifier,
                        Some('>') => { self.emit_doctype(true); self.state = State::Data; },
                        Some('\0') => self.doctype_system_id.as_mut().unwrap().push('\u{FFFD}'),
                        Some(c) => self.doctype_system_id.as_mut().unwrap().push(c),
                        None => {},
                    }
                },
                State::AfterDoctypeSystemIdentifier => match self.peek() {
                    Some(c) if is_ws(c) => { self.pos += 1; },
                    Some('>') => {
                        self.pos += 1;
                        self.emit_doctype(false);
                        self.state = State::Data;
                    },
                    Some(_) => self.state = State::BogusDoctype,
                    None => {},
                },

                State::BogusDoctype => match self.peek() {
                    None => {},
                    Some('>') => {
                        self.pos += 1;
                        self.emit_doctype(false);
                        self.state = State::Data;
                    },
                    Some(_) => {
                        self.pos += 1;
                    },
                },

                State::RawText => match self.peek() {
                    None => {},
                    Some('<') => {
                        self.tag_start = self.pos;
                        self.pos += 1;
                        self.state = State::RawTextLessThan;
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.text_buf.push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.text_buf.push(c);
                    },
                },

                State::RawTextLessThan => match self.peek() {
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::RawTextEndTagOpen;
                    },
                    _ => {
                        self.text_buf.push('<');
                        self.state = State::RawText;
                    },
                },

                State::RawTextEndTagOpen | State::RcEndTagOpen
                | State::ScriptEndTagOpen | State::ScriptEscapedEndTagOpen => {
                    match self.peek() {
                        Some(c) if is_alpha(c) => {
                            self.tag = Some(TagBuilder::new());
                            self.is_end_tag = true;
                            self.state = match self.state {
                                State::RawTextEndTagOpen => State::RawTextEndTagName,
                                State::RcEndTagOpen => State::RcEndTagName,
                                State::ScriptEndTagOpen => State::ScriptEndTagName,
                                _ => State::ScriptEscapedEndTagName,
                            };
                            // reconsume the letter
                        },
                        _ => {
                            self.text_buf.push_str("</");
                            self.state = self.rawtext_back();
                        },
                    }
                },

                State::RawTextEndTagName | State::RcEndTagName
                | State::ScriptEndTagName | State::ScriptEscapedEndTagName => {
                    self.rawtext_end_tag_name_state();
                },

                State::ScriptData => match self.bump() {
                    Some('<') => { self.tag_start = self.pos - 1; self.state = State::ScriptLessThan; },
                    Some('\0') => self.text_buf.push('\u{FFFD}'),
                    Some(c) => self.text_buf.push(c),
                    None => {},
                },
                State::ScriptLessThan => match self.peek() {
                    Some('/') => { self.pos += 1; self.state = State::ScriptEndTagOpen; },
                    Some('!') => {
                        self.pos += 1;
                        self.text_buf.push_str("<!");
                        self.state = State::ScriptEscapeStart;
                    },
                    _ => { self.text_buf.push('<'); self.state = State::ScriptData; },
                },
                State::ScriptEscapeStart => {
                    if self.peek() == Some('-') {
                        self.pos += 1;
                        self.text_buf.push('-');
                        self.state = State::ScriptEscapeStartDash;
                    } else { self.state = State::ScriptData; }
                },
                State::ScriptEscapeStartDash => {
                    if self.peek() == Some('-') {
                        self.pos += 1;
                        self.text_buf.push('-');
                        self.state = State::ScriptEscapedDashDash;
                    } else { self.state = State::ScriptData; }
                },
                State::ScriptEscaped | State::ScriptEscapedDash | State::ScriptEscapedDashDash => {
                    let previous = self.state;
                    match self.peek() {
                        Some('-') => {
                            self.pos += 1;
                            self.text_buf.push('-');
                            self.state = if previous == State::ScriptEscaped {
                                State::ScriptEscapedDash
                            } else { State::ScriptEscapedDashDash };
                        },
                        Some('<') => {
                            self.tag_start = self.pos;
                            self.pos += 1;
                            self.state = State::ScriptEscapedLessThan;
                        },
                        Some('>') if previous == State::ScriptEscapedDashDash => {
                            self.pos += 1;
                            self.text_buf.push('>');
                            self.state = State::ScriptData;
                        },
                        Some(c) => {
                            self.pos += 1;
                            self.text_buf.push(if c == '\0' { '\u{FFFD}' } else { c });
                            self.state = State::ScriptEscaped;
                        },
                        None => {},
                    }
                },
                State::ScriptEscapedLessThan => match self.peek() {
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::ScriptEscapedEndTagOpen;
                    },
                    Some(c) if is_alpha(c) => {
                        self.text_buf.push('<');
                        self.script_temporary.clear();
                        self.state = State::ScriptDoubleEscapeStart;
                    },
                    _ => {
                        self.text_buf.push('<');
                        self.state = State::ScriptEscaped;
                    },
                },
                State::ScriptDoubleEscapeStart | State::ScriptDoubleEscapeEnd => {
                    let starting = self.state == State::ScriptDoubleEscapeStart;
                    match self.peek() {
                        Some(c) if is_alpha(c) => {
                            self.pos += 1;
                            self.script_temporary.push(c.to_ascii_lowercase());
                            self.text_buf.push(c);
                        },
                        Some(c) if is_ws(c) || c == '/' || c == '>' => {
                            self.pos += 1;
                            self.text_buf.push(c);
                            self.state = match (starting, self.script_temporary == "script") {
                                (true, true) => State::ScriptDoubleEscaped,
                                (true, false) => State::ScriptEscaped,
                                (false, true) => State::ScriptEscaped,
                                (false, false) => State::ScriptDoubleEscaped,
                            };
                        },
                        _ => self.state = if starting { State::ScriptEscaped } else { State::ScriptDoubleEscaped },
                    }
                },
                State::ScriptDoubleEscaped | State::ScriptDoubleEscapedDash
                | State::ScriptDoubleEscapedDashDash => {
                    let previous = self.state;
                    match self.peek() {
                        Some('-') => {
                            self.pos += 1;
                            self.text_buf.push('-');
                            self.state = if previous == State::ScriptDoubleEscaped {
                                State::ScriptDoubleEscapedDash
                            } else { State::ScriptDoubleEscapedDashDash };
                        },
                        Some('<') => {
                            self.pos += 1;
                            self.text_buf.push('<');
                            self.state = State::ScriptDoubleEscapedLessThan;
                        },
                        Some('>') if previous == State::ScriptDoubleEscapedDashDash => {
                            self.pos += 1;
                            self.text_buf.push('>');
                            self.state = State::ScriptData;
                        },
                        Some(c) => {
                            self.pos += 1;
                            self.text_buf.push(if c == '\0' { '\u{FFFD}' } else { c });
                            self.state = State::ScriptDoubleEscaped;
                        },
                        None => {},
                    }
                },
                State::ScriptDoubleEscapedLessThan => {
                    if self.peek() == Some('/') {
                        self.pos += 1;
                        self.text_buf.push('/');
                        self.script_temporary.clear();
                        self.state = State::ScriptDoubleEscapeEnd;
                    } else { self.state = State::ScriptDoubleEscaped; }
                },

                State::RcData => match self.peek() {
                    None => {},
                    Some('&') => {
                        self.bump();
                        match self.consume_reference(false) {
                            Some(s) => self.text_buf.push_str(&s),
                            None => self.text_buf.push('&'),
                        }
                    },
                    Some('<') => {
                        self.tag_start = self.pos;
                        self.pos += 1;
                        self.state = State::RcLessThan;
                    },
                    Some('\0') => {
                        self.pos += 1;
                        self.text_buf.push('\u{FFFD}');
                    },
                    Some(_) => {
                        let c = self.bump().unwrap();
                        self.text_buf.push(c);
                    },
                },

                State::RcLessThan => match self.peek() {
                    Some('/') => {
                        self.pos += 1;
                        self.state = State::RcEndTagOpen;
                    },
                    _ => {
                        self.text_buf.push('<');
                        self.state = State::RcData;
                    },
                },
            }
        }
    }

    /// Dispatches on whatever follows `<` while in Data state. Handles
    /// `<!--` comments, `<!doctype`, other `<!…` markup, alpha tags, `<?`
    /// bogus comments, plain `<` text and EOF-after-`<`.
    fn tag_open_dispatch(&mut self) {
        match self.peek_at(1) {
            Some('!') if self.peek_at(2) == Some('-')
                && self.peek_at(3) == Some('-') =>
            {
                self.pos += 4;
                self.comment_buf.clear();
                self.state = State::CommentStart;
            },
            Some('!') if self.ci_at(2, "doctype") => {
                self.pos += 9; // "<!" + "doctype"
                self.doctype_name = None;
                self.doctype_public_id = None;
                self.doctype_system_id = None;
                self.doctype_force_quirks = false;
                self.state = State::BeforeDoctypeName;
            },
            Some('!') if self.allow_cdata && self.input[self.pos + 2..]
                .starts_with(&['[', 'C', 'D', 'A', 'T', 'A', '[']) => {
                self.pos += 9;
                self.state = State::CdataSection;
            },
            Some('!') => {
                // CDATA in HTML content, or another unknown declaration.
                self.pos += 2;
                self.comment_buf.clear();
                self.state = State::BogusComment;
            },
            Some(c) if is_alpha(c) => {
                self.tag_start = self.pos;
                self.pos += 1; // consume '<'
                self.tag = Some(TagBuilder::new());
                self.is_end_tag = false;
                self.state = State::TagName;
            },
            Some('/') => {
                self.tag_start = self.pos;
                self.pos += 2; // consume "</"
                self.state = State::EndTagOpen;
            },
            Some('?') => {
                self.pos += 2; // consume "<?"
                self.processing_target.clear();
                self.processing_data.clear();
                self.state = State::ProcessingInstructionOpen;
            },
            _ => {
                // Not a tag open: '<' is literal text.
                self.pos += 1;
                self.text_buf.push('<');
            },
        }
    }

    /// The RCDATA/RAWTEXT appropriate-end-tag-name states (they share shape;
    /// only their "return to" raw mode differs).
    fn rawtext_end_tag_name_state(&mut self) {
        let return_state = self.rawtext_back();
        match self.peek() {
            None => {
                // EOF mid-end-tag: dump it back as text (finish_eof also
                // covers this, but keep behaviour explicit here).
                self.text_buf.extend(self.input[self.tag_start..self.pos].iter());
                self.tag = None;
                self.state = self.rawtext_back();
            },
            Some(c) if is_ws(c) => {
                self.pos += 1;
                if self.raw_end_matches() {
                    self.state = State::BeforeAttrName;
                } else {
                    self.dump_mismatched_end_tag();
                }
            },
            Some('/') => {
                self.pos += 1;
                if self.raw_end_matches() {
                    self.state = State::SelfClosingStart;
                } else {
                    self.dump_mismatched_end_tag();
                }
            },
            Some('>') => {
                self.pos += 1;
                if self.raw_end_matches() && self.is_end_tag {
                    let name = self.raw_end.clone();
                    self.tag = None;
                    self.raw_end.clear();
                    self.emit(Token::EndTag { name });
                    self.state = State::Data;
                } else {
                    self.dump_mismatched_end_tag();
                }
            },
            Some(c) if matches!(return_state, State::ScriptData | State::ScriptEscaped)
                && !is_alpha(c) => {
                // Script end-tag-name's anything-else branch reconsumes the
                // character in script data; in particular a null becomes
                // U+FFFD there rather than entering the candidate tag name.
                self.dump_mismatched_end_tag();
            },
            Some(_) => {
                let c = self.bump().unwrap();
                self.tagname_push(c.to_ascii_lowercase());
                // stay in this end-tag-name state
                self.state = match return_state {
                    State::RcData => State::RcEndTagName,
                    State::ScriptData => State::ScriptEndTagName,
                    State::ScriptEscaped => State::ScriptEscapedEndTagName,
                    _ => State::RawTextEndTagName,
                };
            },
        }
    }

    fn rawtext_back(&self) -> State {
        match self.state {
            State::RcEndTagOpen | State::RcEndTagName | State::RcLessThan => {
                State::RcData
            },
            State::ScriptEndTagOpen | State::ScriptEndTagName | State::ScriptLessThan => State::ScriptData,
            State::ScriptEscapedEndTagOpen | State::ScriptEscapedEndTagName
            | State::ScriptEscapedLessThan => State::ScriptEscaped,
            _ => State::RawText,
        }
    }

    /// True when the accumulated end-tag name equals the RAWTEXT/RCDATA tag
    /// we're inside — i.e. this is the "appropriate end tag".
    fn raw_end_matches(&self) -> bool {
        self.tag
            .as_ref()
            .map(|t| t.name == self.raw_end && self.is_end_tag)
            .unwrap_or(false)
    }

    /// Replay the original source rather than the lowercased tag builder.
    fn dump_mismatched_end_tag(&mut self) {
        self.text_buf.extend(self.input[self.tag_start..self.pos].iter());
        self.tag = None;
        self.state = self.rawtext_back();
    }

    fn tagname_push(&mut self, c: char) {
        if let Some(t) = &mut self.tag {
            t.name.push(c);
        }
    }

    /// Begins a new attribute whose name starts with `first`; commits any
    /// attribute still pending.
    fn attr_start(&mut self, first: Option<char>) {
        self.finish_attr();
        if let Some(c) = first {
            self.pending.push_name(c);
        }
        self.state = State::AttrName;
    }

    /// Commits the pending attribute into the tag (empty value when only a
    /// name was seen). No-op when no name accumulated.
    fn finish_attr(&mut self) {
        if !self.pending.name.is_empty() {
            let name = std::mem::take(&mut self.pending.name);
            let value = std::mem::take(&mut self.pending.value);
            if let Some(tag) = &mut self.tag {
                tag.push_attr(name, value);
            }
        } else {
            self.pending.value.clear();
        }
    }

    /// Handles '&' inside an attribute value (in_attr = true).
    fn attr_value_entity(&mut self) {
        self.bump(); // consume '&'
        match self.consume_reference(true) {
            Some(s) => self.pending.push_value_str(&s),
            None => self.pending.push_value('&'),
        }
    }

    /// Emits a start/end tag from [`self.tag`]. Tree construction requests
    /// RAWTEXT/RCDATA after namespace-aware insertion via
    /// [`enter_text_element`](Self::enter_text_element).
    fn emit_tag(&mut self, self_closing: bool) {
        self.finish_attr();
        let tag = self.tag.take().expect("tag builder present");
        let name = tag.name;

        if self.is_end_tag {
            if !self.raw_end.is_empty() {
                // Appropriate end tag closing a RAWTEXT/RCDATA element:
                // leave the raw mode (attrs on end tags are dropped).
                self.raw_end.clear();
            }
            self.state = State::Data;
            self.emit(Token::EndTag { name });
        } else {
            self.state = State::Data;
            self.emit(Token::StartTag {
                name,
                attrs: tag.attrs,
                self_closing,
            });
        }
    }

    fn emit_doctype(&mut self, force_quirks: bool) {
        let name = self.doctype_name.take();
        let public_id = self.doctype_public_id.take();
        let system_id = self.doctype_system_id.take();
        let force_quirks = force_quirks || std::mem::take(&mut self.doctype_force_quirks);
        self.emit(Token::Doctype { name, public_id, system_id, force_quirks });
    }

    /// Central EOF handling: emit whatever token the current state was in
    /// the middle of, flush buffered characters, and park in `Done`.
    fn finish_eof(&mut self) {
        match self.state {
            State::Done => {
                self.flush_text();
                return;
            },
            State::Data | State::PlainText | State::RawText | State::RcData | State::ScriptData
            | State::CdataSection
            | State::ScriptEscaped | State::ScriptEscapedDash | State::ScriptEscapedDashDash
            | State::ScriptDoubleEscapeStart | State::ScriptDoubleEscaped
            | State::ScriptDoubleEscapedDash | State::ScriptDoubleEscapedDashDash
            | State::ScriptDoubleEscapeEnd | State::ScriptEscapeStart
            | State::ScriptEscapeStartDash => {
                // just flush text below
            },
            State::CdataBracket => self.text_buf.push(']'),
            State::CdataEnd => self.text_buf.push_str("]]"),
            State::EndTagOpen
            | State::TagName
            | State::BeforeAttrName
            | State::AttrName
            | State::AfterAttrName
            | State::BeforeAttrValue
            | State::AttrValueDouble
            | State::AttrValueSingle
            | State::AttrValueUnquoted
            | State::AfterAttrValueQuoted
            | State::SelfClosingStart => {
                // EOF in tag: emit the literal source text back. Attributes
                // survive because we replay the source slice from '<'. An
                // appropriate RAWTEXT/RCDATA/script end tag has already
                // entered tag parsing after its name; at EOF the partial end
                // tag is discarded, rather than becoming text in its element.
                if !(self.is_end_tag && !self.raw_end.is_empty() && self.raw_end_matches()) {
                    let start = self.tag_start.min(self.pos);
                    let lit: String = self.input[start..self.pos].iter().collect();
                    self.text_buf.push_str(&lit);
                }
            },
            State::RawTextLessThan | State::RcLessThan | State::ScriptLessThan
            | State::ScriptEscapedLessThan => {
                self.text_buf.push('<');
            },
            State::RawTextEndTagOpen | State::RcEndTagOpen | State::ScriptEndTagOpen
            | State::ScriptEscapedEndTagOpen => {
                self.text_buf.push_str("</");
            },
            State::RawTextEndTagName | State::RcEndTagName
            | State::ScriptEndTagName | State::ScriptEscapedEndTagName => {
                self.text_buf.extend(self.input[self.tag_start..self.pos].iter());
            },
            State::ScriptDoubleEscapedLessThan => {},
            State::CommentStartDash => {
                // dash not yet added
                let data = std::mem::take(&mut self.comment_buf);
                self.emit(Token::Comment(data));
            },
            State::CommentEndDash => {
                let mut data = std::mem::take(&mut self.comment_buf);
                data.push('-');
                self.emit(Token::Comment(data));
            },
            State::CommentEnd => {
                let mut data = std::mem::take(&mut self.comment_buf);
                data.push_str("--");
                self.emit(Token::Comment(data));
            },
            State::CommentEndBang => {
                let mut data = std::mem::take(&mut self.comment_buf);
                data.push_str("--!");
                self.emit(Token::Comment(data));
            },
            State::CommentStart | State::Comment | State::BogusComment => {
                let data = std::mem::take(&mut self.comment_buf);
                self.emit(Token::Comment(data));
            },
            State::ProcessingInstructionOpen
            | State::ProcessingInstructionTarget
            | State::AfterProcessingInstructionTarget
            | State::ProcessingInstructionData
            | State::ProcessingInstructionQuestionable => {},
            State::BeforeDoctypeName
            | State::DoctypeName
            | State::AfterDoctypeName
            | State::AfterDoctypePublicKeyword
            | State::BeforeDoctypePublicIdentifier
            | State::DoctypePublicIdentifierDouble
            | State::DoctypePublicIdentifierSingle
            | State::AfterDoctypePublicIdentifier
            | State::BetweenDoctypePublicAndSystemIdentifiers
            | State::AfterDoctypeSystemKeyword
            | State::BeforeDoctypeSystemIdentifier
            | State::DoctypeSystemIdentifierDouble
            | State::DoctypeSystemIdentifierSingle
            | State::AfterDoctypeSystemIdentifier
            | State::BogusDoctype => {
                let force = self.state != State::BogusDoctype;
                self.emit_doctype(force);
            },
        }
        self.flush_text();
        self.state = State::Done;
    }

    // ---- character references (§13.2.5 "Character reference states") ----

    /// Consumes a character reference assuming the cursor sits just past
    /// '&'. Returns `None` when nothing decodes (caller emits a literal '&').
    ///
    /// `in_attr` selects attribute-value rules: legacy semicolon-less refs
    /// (named *and* numeric) followed by '=' or an ASCII alphanumeric must
    /// NOT decode (§13.2.5 "attribute value reference consumed without ;").
    pub(crate) fn consume_reference(&mut self, in_attr: bool) -> Option<String> {
        match self.peek() {
            Some('#') => self.numeric_reference(in_attr),
            _ => self.named_reference(in_attr),
        }
    }

    /// `&#123;` / `&#x7B;` — digits then optional ';' (missing ';' is a
    /// parse error but still decodes, unless blocked by the attribute rule).
    fn numeric_reference(&mut self, in_attr: bool) -> Option<String> {
        let hex = self.peek_at(1) == Some('x') || self.peek_at(1) == Some('X');
        let mut i = if hex { 2 } else { 1 };
        let base = if hex { 16u32 } else { 10 };
        let mut value: u32 = 0;
        let mut any_digit = false;
        while let Some(c) = self.peek_at(i) {
            match c.to_digit(base) {
                Some(d) => {
                    value = value
                        .checked_mul(base)
                        .and_then(|v| v.checked_add(d))
                        .unwrap_or(0x11_0000); // clamp above valid range
                    any_digit = true;
                    i += 1;
                },
                None => break,
            }
        }
        if !any_digit {
            // With no digits, the attempted reference remains literal.
            // Leave '#' (and a possible 'x') for the data/attribute state.
            return None;
        }
        let had_semi = self.peek_at(i) == Some(';');
        if !had_semi && in_attr {
            if let Some(next) = self.peek_at(i) {
                if next == '=' || next.is_ascii_alphanumeric() {
                    return None; // attribute rule: don't decode
                }
            }
        }
        self.pos += i + usize::from(had_semi);
        Some(numeric_ref_char(value))
    }

    /// Longest-match against the named tables. Full names (with ';') always
    /// decode; legacy semicolon-less names follow the attribute rule.
    ///
    /// Probes candidate lengths longest-first via `entities::named_lookup`/
    /// `legacy_lookup` (O(1) hashmap lookups) rather than scanning all 2,231
    /// table entries per candidate — see `entities.rs`'s module doc (HTML-34)
    /// for why the linear scan stopped being viable once the table grew from
    /// ~85 entries to the full spec table.
    fn named_reference(&mut self, in_attr: bool) -> Option<String> {
        // Every table name is pure ASCII, so a non-ASCII char can never be
        // part of one — truncating the window there (rather than at
        // MAX_NAME_LEN unconditionally) keeps `window`'s byte indices equal
        // to its char indices, so every candidate length below can be a
        // cheap byte-range slice instead of a fresh per-candidate `String`.
        let window: String = self
            .input
            .iter()
            .skip(self.pos)
            .take_while(|c| c.is_ascii())
            .take(MAX_NAME_LEN)
            .collect();
        if window.is_empty() {
            return None;
        }

        for len in (1..=window.len()).rev() {
            if let Some(replacement) = crate::entities::named_lookup(&window[..len]) {
                self.pos += len; // ASCII: bytes == chars == positions consumed
                return Some(replacement.to_string());
            }
        }

        // Legacy semicolon-less names: only the single longest match is ever
        // considered (matching the table's own no-overlapping-prefix
        // invariant), and the attribute rule can only block that one match —
        // it never falls back to a shorter legacy candidate.
        for len in (1..=window.len()).rev() {
            if let Some(replacement) = crate::entities::legacy_lookup(&window[..len]) {
                let next = window.as_bytes().get(len).map(|b| *b as char);
                // Attribute rule: legacy ref followed by '=' or alphanumeric
                // must not decode.
                let blocked = in_attr
                    && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric());
                if !blocked {
                    self.pos += len;
                    return Some(replacement.to_string());
                }
                break;
            }
        }
        None
    }
}

/// Maps a numeric reference value per §13.2.5.72 (numeric character
/// reference end state): out-of-range, surrogates and zero become U+FFFD;
/// C1 controls get the Windows-1252 replacement table treatment.
fn numeric_ref_char(value: u32) -> String {
    let adjusted: u32 = if (0x80..=0x9F).contains(&value) {
        C1_REPLACEMENTS[(value - 0x80) as usize]
    } else {
        value
    };
    let ch = match adjusted {
        0 | 0xD800..=0xDFFF | 0x11_0000..=u32::MAX => '\u{FFFD}',
        v => char::from_u32(v).unwrap_or('\u{FFFD}'),
    };
    ch.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(src: &str) -> Vec<Token> {
        let mut t = Tokenizer::new(src.chars().collect());
        let mut out = Vec::new();
        loop {
            let tok = t.next_token();
            if tok == Token::Eof {
                break;
            }
            if let Token::StartTag { name, self_closing: false, .. } = &tok {
                t.enter_text_element(name);
            }
            out.push(tok);
        }
        out
    }

    #[test]
    fn cdata_tokenization_requires_foreign_namespace_feedback() {
        let mut html = Tokenizer::new("<![CDATA[x<y]]>".chars().collect());
        assert_eq!(html.next_token(), Token::Comment("[CDATA[x<y]]".into()));
        assert_eq!(html.next_token(), Token::Eof);

        let mut foreign = Tokenizer::new("<![CDATA[x<y &amp;]]]>".chars().collect());
        foreign.set_allow_cdata(true);
        assert_eq!(foreign.next_token(), Token::Characters("x<y &amp;]".into()));
        assert_eq!(foreign.next_token(), Token::Eof);
    }

    #[test]
    fn script_document_end_tags_follow_escape_depth() {
        fn names_and_text(src: &str) -> Vec<Token> { tokenize(src) }
        assert_eq!(names_and_text("<script>a<b>&amp;</script><p>z"), vec![
            Token::StartTag { name: "script".into(), attrs: vec![], self_closing: false },
            Token::Characters("a<b>&amp;".into()),
            Token::EndTag { name: "script".into() },
            Token::StartTag { name: "p".into(), attrs: vec![], self_closing: false },
            Token::Characters("z".into()),
        ]);
        assert_eq!(names_and_text("<script><!-- <script>inner</script> --> tail </script><p>z"), vec![
            Token::StartTag { name: "script".into(), attrs: vec![], self_closing: false },
            Token::Characters("<!-- <script>inner</script> --> tail ".into()),
            Token::EndTag { name: "script".into() },
            Token::StartTag { name: "p".into(), attrs: vec![], self_closing: false },
            Token::Characters("z".into()),
        ]);
        assert_eq!(names_and_text("<script>a</ScRiPt><p>z"), vec![
            Token::StartTag { name: "script".into(), attrs: vec![], self_closing: false },
            Token::Characters("a".into()),
            Token::EndTag { name: "script".into() },
            Token::StartTag { name: "p".into(), attrs: vec![], self_closing: false },
            Token::Characters("z".into()),
        ]);
    }

    #[test]
    fn simple_tag_stream() {
        let toks = tokenize("<div class=\"a\">hi</div>");
        assert_eq!(
            toks,
            vec![
                Token::StartTag {
                    name: "div".into(),
                    attrs: vec![("class".into(), "a".into())],
                    self_closing: false,
                },
                Token::Characters("hi".into()),
                Token::EndTag { name: "div".into() },
            ]
        );
    }

    #[test]
    fn attr_quoting_styles_and_dedupe() {
        let toks = tokenize("<a x=1 y='two' z=\"3\" X=dup>");
        assert_eq!(
            toks,
            vec![Token::StartTag {
                name: "a".into(),
                attrs: vec![
                    ("x".into(), "1".into()),
                    ("y".into(), "two".into()),
                    ("z".into(), "3".into()),
                ],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn comment_forms_incl_abrupt_close() {
        assert_eq!(tokenize("<!-- ok -->"), vec![Token::Comment(" ok ".into())]);
        assert_eq!(tokenize("<!-->"), vec![Token::Comment(String::new())]);
        assert_eq!(tokenize("<!--x--!y-->"), vec![Token::Comment("x--!y".into())]);
        assert_eq!(
            tokenize("<?php echo 1 ?>"),
            vec![Token::ProcessingInstruction {
                target: "php".into(),
                data: "echo 1 ".into(),
            }]
        );
        assert!(tokenize("<?").is_empty());
        assert_eq!(
            tokenize("<?xml version=\"1.0\">Hi"),
            vec![
                Token::Comment("?xml version=\"1.0\"".into()),
                Token::Characters("Hi".into()),
            ]
        );
        assert_eq!(tokenize("<?#"), vec![Token::Comment("?#".into())]);
        assert_eq!(
            tokenize("<?foo><?foo bar><?foo?x?>"),
            vec![
                Token::ProcessingInstruction {
                    target: "foo".into(),
                    data: String::new(),
                },
                Token::ProcessingInstruction {
                    target: "foo".into(),
                    data: "bar".into(),
                },
                Token::ProcessingInstruction {
                    target: "foo".into(),
                    data: "?x".into(),
                },
            ]
        );
        assert_eq!(
            tokenize("<?a:b?>"),
            vec![Token::Comment("?a:b?".into())]
        );
        for abrupt in ["<?foo", "<?foo ", "<?foo bar", "<?foo?"] {
            assert!(tokenize(abrupt).is_empty(), "abrupt PI {abrupt:?}");
        }
        assert_eq!(
            tokenize("FOO<!-- BAR"),
            vec![Token::Characters("FOO".into()), Token::Comment(" BAR".into())]
        );
    }

    #[test]
    fn doctype_case_insensitive_and_missing_name() {
        assert_eq!(
            tokenize("<!DOCTYPE html>"),
            vec![Token::Doctype { name: Some("html".into()), public_id: None, system_id: None, force_quirks: false }]
        );
        assert_eq!(
            tokenize("<!doctype Html>"),
            vec![Token::Doctype { name: Some("html".into()), public_id: None, system_id: None, force_quirks: false }]
        );
        assert_eq!(
            tokenize("<!DOCTYPE >"),
            vec![Token::Doctype { name: None, public_id: None, system_id: None, force_quirks: true }]
        );
    }

    #[test]
    fn doctype_identifiers_and_recovery() {
        let cases = [
            ("<!DOCTYPE html PUBLIC \"pub\">", Some("pub"), None, false),
            ("<!DOCTYPE html SYSTEM 'sys'>", None, Some("sys"), false),
            ("<!DOCTYPE Html PUBLIC 'p' \"s\"><p>x", Some("p"), Some("s"), false),
            ("<!DOCTYPE html PUBLIC '' \"\">", Some(""), Some(""), false),
            ("<!DOCTYPE html PUBLIC \"p\">", Some("p"), None, false),
            ("<!DOCTYPE html PUBLIC\"p\">", Some("p"), None, false),
            ("<!DOCTYPE html SYSTEM's'>", None, Some("s"), false),
            ("<!DOCTYPE html PUBLIC nope>", None, None, true),
            ("<!DOCTYPE html SYSTEM nope>", None, None, true),
            ("<!DOCTYPE html PUBLIC \"partial", Some("partial"), None, true),
            ("<!DOCTYPE html SYSTEM 'partial", None, Some("partial"), true),
            ("<!DOCTYPE html PUBLIC \"p>tail", Some("p"), None, true),
            ("<!DOCTYPE html SYSTEM \"s\"junk>", None, Some("s"), false),
            ("<!DOCTYPE html PUBLIC 'a\0b'>", Some("a\u{FFFD}b"), None, false),
        ];
        for (input, public, system, force_quirks) in cases {
            let tokens = tokenize(input);
            assert_eq!(tokens.first(), Some(&Token::Doctype {
                name: Some("html".into()),
                public_id: public.map(str::to_owned),
                system_id: system.map(str::to_owned),
                force_quirks,
            }), "{input:?}");
        }
        assert_eq!(tokenize("<!DOCTYPE html PUBLIC \"p>tail")[1], Token::Characters("tail".into()));
    }

    #[test]
    fn entities_named_legacy_numeric() {
        assert_eq!(
            tokenize("a&amp;b&nbsp;c&thetasym;d"),
            vec![Token::Characters("a&b\u{00A0}c\u{03D1}d".into())]
        );
        // legacy without semicolon
        assert_eq!(tokenize("a&amp;b"), vec![Token::Characters("a&b".into())]);
        // numeric decimal + hex
        assert_eq!(tokenize("&#65;&#x42;"), vec![Token::Characters("AB".into())]);
        // C1 Windows-1252 replacements
        assert_eq!(
            tokenize("&#128;&#151;"),
            vec![Token::Characters("\u{20AC}\u{2014}".into())]
        );
        // zero → FFFD
        assert_eq!(tokenize("&#0;"), vec![Token::Characters("\u{FFFD}".into())]);
    }

    #[test]
    fn shaping_control_entities_decode_to_unicode_scalars() {
        let tokens = tokenize("a&zwj;b&zwnj;c");
        assert!(tokens.iter().any(|token| {
            matches!(token, Token::Characters(data) if data.contains("a\u{200d}b\u{200c}c"))
        }), "tokens were {tokens:?}");
    }

    #[test]
    fn whitespace_control_entities_decode_for_css_text_processing() {
        let tokens = tokenize("a&Tab;b&NewLine;c");
        assert_eq!(tokens, vec![Token::Characters("a\tb\nc".into())]);
    }

    #[test]
    fn soft_hyphen_entity_decodes_to_the_discretionary_character() {
        let tokens = tokenize("a&shy;b");
        assert_eq!(tokens, vec![Token::Characters("a\u{00ad}b".into())]);
    }

    #[test]
    fn entities_in_attr_blocked_by_equals() {
        // "?a=&ampx" — legacy "&amp" followed by '=' must not decode.
        let toks = tokenize("<a href=\"?a=&ampx\">");
        assert_eq!(
            toks,
            vec![Token::StartTag {
                name: "a".into(),
                attrs: vec![("href".into(), "?a=&ampx".into())],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn html23_named_references_use_longest_match_and_legacy_context_rules() {
        assert_eq!(
            tokenize("&notit; &notin; &pound_id &AElig= &prod;_id"),
            vec![Token::Characters("\u{00ac}it; \u{2209} \u{00a3}_id \u{00c6}= \u{220f}_id".into())]
        );
        assert_eq!(
            tokenize("<div a='&pound_id' b='&pound=23' c='&prod;_id'>"),
            vec![Token::StartTag {
                name: "div".into(),
                attrs: vec![
                    ("a".into(), "\u{00a3}_id".into()),
                    ("b".into(), "&pound=23".into()),
                    ("c".into(), "\u{220f}_id".into()),
                ],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn script_keeps_inner_markup_as_raw_text() {
        let toks = tokenize("<script>if(a<b){x=\"</p>\"}</script>");
        assert_eq!(
            toks,
            vec![
                Token::StartTag { name: "script".into(), attrs: vec![], self_closing: false },
                Token::Characters("if(a<b){x=\"</p>\"}".into()),
                Token::EndTag { name: "script".into() },
            ]
        );
    }

    #[test]
    fn textarea_decodes_entities_but_not_tags() {
        let toks = tokenize("<textarea>&lt;i&gt;</textarea>");
        assert_eq!(
            toks,
            vec![
                Token::StartTag { name: "textarea".into(), attrs: vec![], self_closing: false },
                Token::Characters("<i>".into()),
                Token::EndTag { name: "textarea".into() },
            ]
        );
    }

    #[test]
    fn self_closing_flag_survives() {
        assert_eq!(
            tokenize("<br/>"),
            vec![Token::StartTag {
                name: "br".into(),
                attrs: vec![],
                self_closing: true,
            }]
        );
    }

    #[test]
    fn eof_mid_tag_falls_back_to_literal_text() {
        assert_eq!(
            tokenize("hello<div class='x"),
            vec![Token::Characters("hello<div class='x".into())]
        );
        assert_eq!(tokenize("</scr"), vec![Token::Characters("</scr".into())]);
    }

    #[test]
    fn mismatched_rawtext_end_tag_dumps_back_to_text() {
        // "</scrip" is not the appropriate end tag → literal text.
        let toks = tokenize("<style>a</scrip b></style>");
        assert_eq!(
            toks,
            vec![
                Token::StartTag { name: "style".into(), attrs: vec![], self_closing: false },
                Token::Characters("a</scrip b>".into()),
                Token::EndTag { name: "style".into() },
            ]
        );
    }
}
