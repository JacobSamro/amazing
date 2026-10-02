// Portions ported from servo/html5ever (html5ever/src/tree_builder/rules.rs)
// @ 6db7bbf1c4dfaecd28731950d427e40c8b911bef, MIT.
// Copyright 2014 The html5ever Project Developers.
//! Complete scripting-disabled "in head noscript" insertion mode.

use super::*;

impl TreeBuilder {
    pub(super) fn in_head_noscript(&mut self, tok: Token) {
        match tok {
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, .. } if name == "html" => self.in_body(tok),
            Token::EndTag { ref name } if name == "noscript" => {
                self.open.pop();
                self.mode = Mode::InHead;
            },
            Token::Characters(data) => {
                let non_whitespace = data.char_indices().find_map(|(i, c)| (!is_ws_char(c)).then_some(i));
                if let Some(i) = non_whitespace {
                    if i != 0 { self.in_head(Token::Characters(data[..i].to_owned())); }
                    self.open.pop();
                    self.mode = Mode::InHead;
                    self.process_token(Token::Characters(data[i..].to_owned()));
                } else {
                    self.in_head(Token::Characters(data));
                }
            },
            Token::Comment(_) => self.in_head(tok),
            Token::StartTag { ref name, .. } if matches!(name.as_str(),
                "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style") => self.in_head(tok),
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "head" | "noscript") => {},
            Token::EndTag { ref name } if name != "br" => {},
            other => {
                self.open.pop();
                self.mode = Mode::InHead;
                self.process_token(other);
            },
        }
    }
}
