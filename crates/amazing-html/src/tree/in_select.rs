//! The "in select" (and "in select in table" recovery, handled by the
//! in-body rules per this module's own doc comment) insertion mode
//! (§13.2.6.4.16-17). Pure move out of `tree.rs` (no behavior change) to
//! keep that file under the workspace's 4,000-line cap; see `tree.rs`'s own
//! module doc comment and `docs/lanes/html_TODO.md` for the split.

use super::*;

impl TreeBuilder {
    /// Bounded select fragment rules. The fragment context is not on the
    /// open-elements stack, so a stray `</select>` cannot close it.
    pub(super) fn in_select(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => self.insert_text(&data),
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::StartTag { name, attrs, .. } if name == "option" => {
                if self.current().is_some_and(|idx| self.dom.name(idx) == Some("option")) {
                    self.open.pop();
                }
                self.insert_and_push("option", attrs);
            },
            Token::StartTag { name, attrs, .. } if name == "optgroup" => {
                if self.current().is_some_and(|idx| self.dom.name(idx) == Some("option")) {
                    self.open.pop();
                }
                if self.current().is_some_and(|idx| self.dom.name(idx) == Some("optgroup")) {
                    self.open.pop();
                }
                self.insert_and_push("optgroup", attrs);
            },
            Token::EndTag { name } if name == "option" => {
                if self.current().is_some_and(|idx| self.dom.name(idx) == Some("option")) {
                    self.open.pop();
                }
            },
            Token::EndTag { name } if name == "optgroup" => {
                if self.open.len() > 2
                    && self.current().is_some_and(|idx| self.dom.name(idx) == Some("option"))
                    && self.dom.name(self.open[self.open.len() - 2]) == Some("optgroup")
                {
                    self.open.pop();
                }
                if self.current().is_some_and(|idx| self.dom.name(idx) == Some("optgroup")) {
                    self.open.pop();
                }
            },
            Token::StartTag { name, attrs, .. } if name == "html" => {
                self.in_body_start_tag("html", attrs);
            },
            Token::Eof => {},
            _ => {}, // other tags and stray context end tags are ignored
        }
    }
}
