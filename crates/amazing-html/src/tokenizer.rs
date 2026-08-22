//! WHATWG HTML tokenizer (§13.2.5 "Tokenization").
//!
//! State names mirror the spec (`Data state`, `Tag open state`, …) because
//! the spec's error-recovery behavior is the interop contract. Deviations
//! from full spec (all deliberate, all listed in lib.rs):
//! - encoding sniffing happens in the caller; input is lossy UTF-8 `char`s
//! - PUBLIC/SYSTEM identifiers are consumed but not captured
//! - CDATA (`<![CDATA[`) is treated as a bogus comment
//!
//! Input is a `Vec<char>` — one `char` = one scalar value, so cursor
//! arithmetic is per-character and entity names (ASCII) match 1 byte = 1 char.

use std::collections::VecDeque;

use crate::entities::{LEGACY_NO_SEMI, MAX_NAME_LEN, NAMED};

/// A tokenizer token, per §13.2.5 "Emitting tokens".
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Doctype {
        name: Option<String>,
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
    /// One or more characters, already entity-decoded.
    Characters(String),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Data,
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
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    BogusDoctype,
    RawText,
    RawTextLessThan,
    RawTextEndTagOpen,
    RawTextEndTagName,
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
    doctype_name: Option<String>,
    /// End-tag name we're looking for while in RAWTEXT/RCDATA.
    raw_end: String,
    eof_sent: bool,
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
            doctype_name: None,
            raw_end: String::new(),
            eof_sent: false,
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

    /// Public entry point per §13.2.5 "Tokenization". Runs states until a
    /// token is available or EOF is fully processed; repeated calls after
    /// EOF keep returning [`Token::Eof`].
    pub fn next_token(&mut self) -> Token {
        if let Some(t) = self.queue.pop_front() {
            return t;
        }
        if !self.eof_sent {
            self.run();
            if let Some(t) = self.queue.pop_front() {
                return t;
            }
            self.eof_sent = true;
        }
        Token::Eof
    }

    /// Drives states until input is exhausted and everything is emitted.
    fn run(&mut self) {
        loop {
            if self.pos >= self.input.len() {
                self.finish_eof();
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

                // PUBLIC/SYSTEM identifiers are consumed but not captured —
                // only their presence matters for quirks-mode detection.
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
            Some(c)
                if is_alpha(c) && (self.ci_at(0, "public") || self.ci_at(0, "system")) =>
            {
                self.doctype_consume_external(self.ci_at(0, "public"));
            },
                    Some(_) => {
                        // Anything unrecognised after the name: bogus doctype
                        // (force-quirks on close). Approximation of the
                        // before/between/after-public/system-id states.
                        self.state = State::BogusDoctype;
                    },
                },

                State::BogusDoctype => match self.peek() {
                    None => {},
                    Some('>') => {
                        self.pos += 1;
                        self.emit_doctype(true);
                        self.state = State::Data;
                    },
                    Some(_) => {
                        self.pos += 1;
                    },
                },

                State::RawText => match self.peek() {
                    None => {},
                    Some('<') => {
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

                State::RawTextEndTagOpen | State::RcEndTagOpen => {
                    match self.peek() {
                        Some(c) if is_alpha(c) => {
                            self.tag = Some(TagBuilder::new());
                            self.is_end_tag = true;
                            self.tag_start = self.pos.saturating_sub(2);
                            self.state = if self.state
                                == State::RawTextEndTagOpen
                            {
                                State::RawTextEndTagName
                            } else {
                                State::RcEndTagName
                            };
                            // reconsume the letter
                        },
                        _ => {
                            self.text_buf.push_str("</");
                            self.state = self.rawtext_back();
                        },
                    }
                },

                State::RawTextEndTagName | State::RcEndTagName => {
                    self.rawtext_end_tag_name_state();
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
                self.state = State::BeforeDoctypeName;
            },
            Some('!') => {
                // e.g. CDATA / declarations: bogus comment (deviation note
                // in lib.rs covers CDATA specifically).
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
                // Empty comment token; the '?' itself is reconsumed by
                // BogusComment (spec §13.2.5.6 tag open state).
                self.pos += 1;
                self.comment_buf.clear();
                self.state = State::BogusComment;
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
        let in_rc = matches!(
            self.state,
            State::RcEndTagName | State::RcData | State::RcLessThan
        );
        match self.peek() {
            None => {
                // EOF mid-end-tag: dump it back as text (finish_eof also
                // covers this, but keep behaviour explicit here).
                self.text_buf.push_str("</");
                if let Some(t) = self.tag.take() {
                    self.text_buf.push_str(&t.name);
                }
                self.state = self.rawtext_back();
            },
            Some(c) if is_ws(c) => {
                self.pos += 1;
                if self.raw_end_matches() {
                    self.state = State::BeforeAttrName;
                } else {
                    let mut term = String::new();
                    term.push(c);
                    self.dump_mismatched_end_tag(&term);
                }
            },
            Some('/') => {
                self.pos += 1;
                if self.raw_end_matches() {
                    self.state = State::SelfClosingStart;
                } else {
                    self.dump_mismatched_end_tag("/");
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
                    self.dump_mismatched_end_tag(">");
                }
            },
            Some(_) => {
                let c = self.bump().unwrap();
                self.tagname_push(c.to_ascii_lowercase());
                // stay in this end-tag-name state
                self.state = if in_rc {
                    State::RcEndTagName
                } else {
                    State::RawTextEndTagName
                };
            },
        }
    }

    fn rawtext_back(&self) -> State {
        match self.state {
            State::RcEndTagOpen | State::RcEndTagName | State::RcLessThan => {
                State::RcData
            },
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

    /// The matched-end-tag shortcut failed: dump "</name…" back into text
    /// and resume the surrounding raw-text mode.
    fn dump_mismatched_end_tag(&mut self, terminator: &str) {
        self.text_buf.push_str("</");
        if let Some(t) = self.tag.take() {
            self.text_buf.push_str(&t.name);
        }
        self.text_buf.push_str(terminator);
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

    /// Emits a start/end tag from [`self.tag`]. Start tags named
    /// script/style switch to RAWTEXT and textarea/title to RCDATA (the
    /// tree builder would request this anyway; doing it here keeps the two
    /// components decoupled — documented deviation in lib.rs).
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
            match name.as_str() {
                "script" | "style" => {
                    self.raw_end = name.clone();
                    self.state = State::RawText;
                },
                "textarea" | "title" => {
                    self.raw_end = name.clone();
                    self.state = State::RcData;
                },
                _ => self.state = State::Data,
            }
            self.emit(Token::StartTag {
                name,
                attrs: tag.attrs,
                self_closing,
            });
        }
    }

    fn emit_doctype(&mut self, force_quirks: bool) {
        let name = self.doctype_name.take();
        self.emit(Token::Doctype { name, force_quirks });
    }

    /// Consumes a PUBLIC/SYSTEM keyword plus its quoted identifier(s),
    /// leaving the cursor on the terminator (usually '>' or EOF). The id
    /// strings themselves are discarded. `two_ids` is true for PUBLIC
    /// (formal identifier + system URI) and false for SYSTEM.
    fn doctype_consume_external(&mut self, two_ids: bool) {
        self.pos += 6; // skip "public"/"system"
        let mut strings_closed = 0usize;
        loop {
            match self.peek() {
                None => return,
                Some('>') => return, // AfterDoctypeName will close it out
                Some(q @ ('"' | '\'')) => {
                    self.pos += 1;
                    loop {
                        match self.peek() {
                            None => return,
                            Some('>') => break, // unterminated string
                            Some(c) => {
                                self.pos += 1;
                                if c == q {
                                    strings_closed += 1;
                                    break;
                                }
                            },
                        }
                    }
                    if strings_closed >= if two_ids { 2 } else { 1 } {
                        return;
                    }
                },
                Some(_) => {
                    self.pos += 1;
                },
            }
        }
    }

    /// Central EOF handling: emit whatever token the current state was in
    /// the middle of, flush buffered characters, and park in `Done`.
    fn finish_eof(&mut self) {
        match self.state {
            State::Done => {
                self.flush_text();
                return;
            },
            State::Data | State::RawText | State::RcData => {
                // just flush text below
            },
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
                // survive because we replay the source slice from '<'.
                let start = self.tag_start.min(self.pos);
                let lit: String = self.input[start..self.pos].iter().collect();
                self.text_buf.push_str(&lit);
            },
            State::RawTextLessThan | State::RcLessThan => {
                self.text_buf.push('<');
            },
            State::RawTextEndTagOpen | State::RcEndTagOpen => {
                self.text_buf.push_str("</");
            },
            State::RawTextEndTagName | State::RcEndTagName => {
                self.text_buf.push_str("</");
                if let Some(t) = self.tag.take() {
                    self.text_buf.push_str(&t.name);
                }
            },
            State::CommentStartDash => {
                // dash not yet added
                let data = std::mem::take(&mut self.comment_buf);
                self.queue.push_back(Token::Comment(data));
            },
            State::CommentEndDash => {
                let mut data = std::mem::take(&mut self.comment_buf);
                data.push('-');
                self.queue.push_back(Token::Comment(data));
            },
            State::CommentEnd => {
                let mut data = std::mem::take(&mut self.comment_buf);
                data.push_str("--");
                self.queue.push_back(Token::Comment(data));
            },
            State::CommentEndBang => {
                let mut data = std::mem::take(&mut self.comment_buf);
                data.push_str("--!");
                self.queue.push_back(Token::Comment(data));
            },
            State::CommentStart | State::Comment | State::BogusComment => {
                let data = std::mem::take(&mut self.comment_buf);
                self.queue.push_back(Token::Comment(data));
            },
            State::BeforeDoctypeName
            | State::DoctypeName
            | State::AfterDoctypeName
            | State::BogusDoctype => {
                let name = self.doctype_name.take();
                self.queue.push_back(Token::Doctype {
                    name,
                    force_quirks: true,
                });
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
            // Parse error: emit U+FFFD, drop "#"/"#x", continue normally.
            self.pos += i;
            return Some("\u{FFFD}".to_string());
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
    fn named_reference(&mut self, in_attr: bool) -> Option<String> {
        let window: String = self
            .input
            .iter()
            .skip(self.pos)
            .take(MAX_NAME_LEN)
            .collect::<String>();
        if window.is_empty() {
            return None;
        }

        // Names in the tables include the leading '&'; the window does not.
        let mut best: Option<(&str, &str)> = None;
        for (name, replacement) in NAMED {
            let bare = &name[1..]; // drop '&'
            if window.len() >= bare.len() && window.starts_with(bare) {
                if best.map_or(true, |(b, _)| bare.len() > b.len()) {
                    best = Some((bare, replacement));
                }
            }
        }
        if let Some((bare, replacement)) = best {
            self.pos += bare.len(); // ASCII: bytes == chars
            return Some(replacement.to_string());
        }

        // Legacy semicolon-less names. `window[name.len()-1]` is the char
        // immediately after the name (index shifts by the skipped '&').
        let mut best_legacy: Option<(&str, &str)> = None;
        for (name, replacement) in LEGACY_NO_SEMI {
            let bare = &name[1..];
            if window.len() >= bare.len() && window.starts_with(bare) {
                if best_legacy.map_or(true, |(b, _)| bare.len() > b.len()) {
                    best_legacy = Some((bare, replacement));
                }
            }
        }
        if let Some((bare, replacement)) = best_legacy {
            let next = window.as_bytes().get(bare.len()).map(|b| *b as char);
            // Attribute rule: legacy ref followed by '=' or alphanumeric
            // must not decode.
            let blocked = in_attr
                && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric());
            if !blocked {
                self.pos += bare.len();
                return Some(replacement.to_string());
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
            out.push(tok);
        }
        out
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
        assert_eq!(tokenize("<?php echo 1 ?>"), vec![Token::Comment("?php echo 1 ?".into())]);
    }

    #[test]
    fn doctype_case_insensitive_and_missing_name() {
        assert_eq!(
            tokenize("<!DOCTYPE html>"),
            vec![Token::Doctype { name: Some("html".into()), force_quirks: false }]
        );
        assert_eq!(
            tokenize("<!doctype Html>"),
            vec![Token::Doctype { name: Some("html".into()), force_quirks: false }]
        );
        assert_eq!(
            tokenize("<!DOCTYPE >"),
            vec![Token::Doctype { name: None, force_quirks: true }]
        );
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
