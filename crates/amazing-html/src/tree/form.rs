// Portions ported from servo/html5ever (html5ever/src/tree_builder/rules.rs)
// @ 6db7bbf1c4dfaecd28731950d427e40c8b911bef, MIT.
// Copyright 2014 The html5ever Project Developers.
//! Complete in-body form start/end rules, including template-local forms.
use super::*;

impl TreeBuilder {
    pub(super) fn start_form(&mut self, attrs: Vec<(String, String)>) {
        let in_template = self.stack_has("template");
        if self.form.is_some() && !in_template { return; }
        self.close_p_element();
        let element = self.insert_element("form", attrs);
        self.open.push(element);
        if !in_template { self.form = Some(element); }
    }

    pub(super) fn end_form(&mut self) {
        if !self.stack_has("template") {
            let Some(element) = self.form.take() else { return; };
            if !self.has_in_specific_scope_matching(&[], |node| node == element) { return; }
            self.generate_implied_end_tags(None);
            if let Some(position) = self.open.iter().position(|node| *node == element) {
                self.open.remove(position);
            }
        } else {
            if !self.has_in_scope("form") { return; }
            self.generate_implied_end_tags(None);
            self.pop_until("form");
        }
    }

    pub(super) fn insert_table_form(&mut self, attrs: Vec<(String, String)>) {
        if !self.stack_has("template") && self.form.is_none() {
            self.form = Some(self.insert_element("form", attrs));
        }
    }
}
