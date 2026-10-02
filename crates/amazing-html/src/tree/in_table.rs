//! The table-family insertion modes (§13.2.6.4.9-15): "in table", "in table
//! text", "in table body", "in row", "in cell", "in caption" and "in column
//! group". Pure move out of `tree.rs` (no behavior change) to keep that file
//! under the workspace's 4,000-line cap; see `tree.rs`'s own module doc
//! comment and `docs/lanes/html_TODO.md` for the split.

use super::*;

impl TreeBuilder {
    pub(super) fn in_table(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                self.table_text_return_mode = self.mode;
                self.pending_table_text.clear();
                self.pending_table_text.push_str(&data);
                self.mode = Mode::InTableText;
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing: _ } => {
                self.in_table_start_tag(&name, attrs)
            },
            Token::EndTag { name } => self.in_table_end_tag(&name),
            Token::Eof => {},
            Token::ProcessingInstruction { .. } => unreachable!("handled by dispatcher"),
        }
    }

    /// Accumulate adjacent character tokens before deciding their insertion
    /// place. A non-whitespace scalar makes the entire run foster-parented;
    /// otherwise it belongs under the current table-family element.
    pub(super) fn in_table_text(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => self.pending_table_text.push_str(&data),
            other => {
                let data = std::mem::take(&mut self.pending_table_text);
                self.mode = self.table_text_return_mode;
                if !data.is_empty() {
                    if data.chars().all(is_ws_char) {
                        self.insert_text(&data);
                    } else {
                        self.foster = true;
                        self.in_body(Token::Characters(data));
                        self.foster = false;
                    }
                }
                self.process_token(other);
            },
        }
    }

    fn in_table_start_tag(&mut self, name: &str, attrs: Vec<(String, String)>) {
        match name {
            "caption" => {
                self.clear_back_to_table();
                self.fmt.push(FmtEntry::Marker);
                self.insert_and_push("caption", attrs);
                self.mode = Mode::InCaption;
            },
            "colgroup" => {
                self.clear_back_to_table();
                self.insert_and_push("colgroup", attrs);
                self.mode = Mode::InColumnGroup;
            },
            "col" => {
                self.clear_back_to_table();
                self.insert_and_push("colgroup", Vec::new());
                self.mode = Mode::InColumnGroup;
                self.in_column_group(Token::StartTag {
                    name: "col".to_owned(),
                    attrs,
                    self_closing: false,
                });
            },
            "tbody" | "tfoot" | "thead" => {
                self.clear_back_to_table();
                self.insert_and_push(name, attrs);
                self.mode = Mode::InTableBody;
            },
            "td" | "th" | "tr" => {
                // Implicit tbody (§13.2.6.4.9).
                self.clear_back_to_table();
                self.insert_and_push("tbody", Vec::new());
                self.mode = Mode::InTableBody;
                self.in_table_body_start_tag(name, attrs);
            },
            "table" => {
                if self.has_in_table_scope("table") {
                    self.pop_until("table");
                    self.reset_insertion_mode();
                    self.process_token(Token::StartTag {
                        name: name.to_owned(),
                        attrs,
                        self_closing: false,
                    });
                }
            },
            "style" | "script" => {
                self.start_generic_text_element(name, attrs);
            },
            "input" => {
                let hidden = attrs.iter().any(|(n, v)| {
                    n == "type" && v.eq_ignore_ascii_case("hidden")
                });
                if hidden {
                    self.insert_void("input", attrs);
                } else {
                    self.foster = true;
                    self.in_body_start_tag(name, attrs);
                }
            },
            "form" => {
                self.insert_table_form(attrs);
            },
            _ => {
                self.foster = true;
                self.in_body_start_tag(name, attrs);
            },
        }
    }

    fn in_table_end_tag(&mut self, name: &str) {
        match name {
            "table" => {
                if self.has_in_table_scope("table") {
                    self.pop_until("table");
                    self.reset_insertion_mode();
                }
            },
            "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td"
            | "tfoot" | "th" | "thead" | "tr" => {},
            _ => {
                self.foster = true;
                self.in_body_end_tag(name);
            },
        }
    }

    pub(super) fn in_table_body(&mut self, tok: Token) {
        match tok {
            Token::StartTag { name, attrs, self_closing: _ } => {
                self.in_table_body_start_tag(&name, attrs)
            },
            Token::EndTag { name } => self.in_table_body_end_tag(&name),
            other => {
                self.in_table(other);
            },
        }
    }

    fn in_table_body_start_tag(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) {
        match name {
            "tr" => {
                self.clear_back_to_section();
                self.insert_and_push("tr", attrs);
                self.mode = Mode::InRow;
            },
            "td" | "th" => {
                // Parse error: synthesize the missing row, then reprocess
                // the cell token using the in-row rules.
                self.clear_back_to_section();
                self.insert_and_push("tr", Vec::new());
                self.mode = Mode::InRow;
                self.in_row(Token::StartTag {
                    name: name.to_owned(),
                    attrs,
                    self_closing: false,
                });
            },
            "caption" | "colgroup" | "col" | "tbody" | "tfoot" | "thead" => {
                if self.close_section() {
                    self.reset_insertion_mode();
                    self.in_table_start_tag(name, attrs);
                }
            },
            _ => {
                self.in_table_start_tag(name, attrs);
            },
        }
    }

    fn in_table_body_end_tag(&mut self, name: &str) {
        match name {
            "tbody" | "tfoot" | "thead" => {
                if self.stack_has(name) {
                    self.generate_implied_end_tags(Some(name));
                    self.pop_until(name);
                    self.mode = Mode::InTable;
                }
            },
            "td" | "th" | "tr" => {
                // These end tags are ignored in the table-body mode.
            },
            _ => {
                self.in_table_end_tag(name);
            },
        }
    }

    pub(super) fn in_row(&mut self, tok: Token) {
        match tok {
            Token::StartTag { name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "td" | "th" => {
                        self.clear_back_to_row();
                        self.fmt.push(FmtEntry::Marker);
                        self.insert_and_push(&name, attrs);
                        self.mode = Mode::InCell;
                    },
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot"
                    | "thead" | "tr" => {
                        if self.close_tr() {
                            self.mode = Mode::InTableBody;
                            self.in_table_body_start_tag(&name, attrs);
                        }
                    },
                    _ => {
                        self.in_table_start_tag(&name, attrs);
                    },
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "tr" => {
                    if self.close_tr() {
                        self.mode = Mode::InTableBody;
                    }
                },
                "td" | "th" => {
                    if self.stack_has(&name) {
                        self.close_cell();
                    }
                },
                "tbody" | "tfoot" | "thead" => {
                    if self.close_tr() {
                        self.mode = Mode::InTableBody;
                        self.in_table_body_end_tag(&name);
                    }
                },
                _ => {
                    self.in_table_end_tag(&name);
                },
            },
            other => {
                self.in_table(other);
            },
        }
    }

    pub(super) fn in_cell(&mut self, tok: Token) {
        match tok {
            Token::StartTag { name, attrs, self_closing: _ } => {
                let cell_open = self.stack_has("td") || self.stack_has("th");
                let is_boundary = matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot"
                        | "th" | "thead" | "tr"
                );
                if is_boundary && cell_open {
                    self.close_cell();
                    self.in_row(Token::StartTag {
                        name,
                        attrs,
                        self_closing: false,
                    });
                } else {
                    self.in_body_start_tag(&name, attrs);
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "td" | "th" => {
                    if self.stack_has(&name) {
                        self.close_cell();
                    }
                },
                "body" | "caption" | "col" | "colgroup" | "html" => {},
                "table" | "tbody" | "tfoot" | "thead" | "tr" => {
                    if self.has_in_table_scope(&name) {
                        self.close_cell();
                        self.in_row(Token::EndTag { name });
                    }
                },
                _ => self.in_body_end_tag(&name),
            },
            other => self.in_body(other),
        }
    }

    pub(super) fn in_caption(&mut self, tok: Token) {
        match tok {
            Token::StartTag { ref name, .. }
                if matches!(name.as_str(), "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr") =>
            {
                if self.has_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_fmt_to_marker();
                    self.mode = Mode::InTable;
                    self.in_table(tok);
                }
            },
            Token::EndTag { ref name } => match name.as_str() {
                "caption" => {
                    if self.stack_has("caption") {
                        self.generate_implied_end_tags(None);
                        self.pop_until("caption");
                        self.clear_fmt_to_marker();
                        self.mode = Mode::InTable;
                    }
                },
                "table" => {
                    if self.has_in_table_scope("table") {
                        self.generate_implied_end_tags(None);
                        self.pop_until("caption");
                        self.clear_fmt_to_marker();
                        self.mode = Mode::InTable;
                        self.in_table(Token::EndTag { name: name.clone() });
                    }
                },
                "body" | "col" | "colgroup" | "html" | "tbody" | "td"
                | "tfoot" | "th" | "thead" | "tr" => {},
                _ => self.in_body(tok.clone()),
            },
            other => self.in_body(other),
        }
    }

    pub(super) fn in_column_group(&mut self, tok: Token) {
        let in_colgroup = self
            .current()
            .map(|c| self.dom.name(c))
            == Some(Some("colgroup"));
        match tok {
            Token::Characters(data) => {
                if data.chars().all(is_ws_char) {
                    self.insert_text(&data);
                } else if in_colgroup {
                    // §13.2.6.4.13 "in column group" processes character
                    // tokens one at a time in the spec's abstract model: each
                    // leading whitespace character is inserted into the
                    // still-open `colgroup` before the first non-whitespace
                    // character closes it and reprocesses. This batched
                    // implementation receives the whole run as one token, so
                    // it must split it at that boundary itself -- inserting
                    // only the leading whitespace *prefix* here and
                    // reprocessing just the remainder under "in table" (which
                    // foster-parents it), rather than reprocessing the whole
                    // run including the whitespace it already committed to
                    // `colgroup` (html5lib domjs-unsafe.dat case 37:
                    // `<colgroup> foo</colgroup>` put the leading space back
                    // into `" foo"` ahead of the table instead of leaving it
                    // inside `colgroup` and foster-parenting only `"foo"`).
                    let split = data.char_indices()
                        .find_map(|(index, c)| (!is_ws_char(c)).then_some(index))
                        .unwrap_or(data.len());
                    if split > 0 {
                        self.insert_text(&data[..split]);
                    }
                    self.open.pop();
                    self.mode = Mode::InTable;
                    self.in_table(Token::Characters(data[split..].to_owned()));
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "html" => self.in_body_start_tag("html", attrs),
                    "col" => {
                        self.insert_void("col", attrs);
                    },
                    _ => {
                        if in_colgroup {
                            self.open.pop();
                            self.mode = Mode::InTable;
                            self.in_table_start_tag(name, attrs);
                        }
                    },
                }
            },
            Token::EndTag { ref name } => match name.as_str() {
                "colgroup" => {
                    if in_colgroup {
                        self.open.pop();
                        self.mode = Mode::InTable;
                    }
                },
                "col" => {},
                _ => {
                    if in_colgroup {
                        self.open.pop();
                        self.mode = Mode::InTable;
                        self.in_table_end_tag(name);
                    }
                },
            },
            _ => {},
        }
    }
}
