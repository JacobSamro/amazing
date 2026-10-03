// Portions ported from servo/html5ever (html5ever/src/tree_builder/rules.rs)
// @ 6db7bbf1c4dfaecd28731950d427e40c8b911bef, MIT.
// Copyright 2014 The html5ever Project Developers.
//! Complete in-body form start/end rules, including template-local forms.
use super::*;

impl TreeBuilder {
    /// HTML §13.2.4.4: a template fragment context also means parsing
    /// template contents even though it is absent from the open stack.
    fn parsing_template_contents(&self) -> bool {
        self.stack_has("template") || (self.fragment_context_namespace == Namespace::Html
            && self.fragment_context.as_deref() == Some("template"))
    }

    pub(super) fn start_form(&mut self, attrs: Vec<(String, String)>) {
        let in_template = self.parsing_template_contents();
        if self.form.is_some() && !in_template { return; }
        self.close_p_element();
        let element = self.insert_element("form", attrs);
        self.open.push(element);
        if !in_template { self.form = Some(element); }
    }

    pub(super) fn end_form(&mut self) {
        if !self.parsing_template_contents() {
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
        let in_template = self.parsing_template_contents();
        if self.form.is_some() && !in_template { return; }
        let element = self.insert_element("form", attrs);
        if !in_template { self.form = Some(element); }
    }
}
