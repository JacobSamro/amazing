//! HTML §4.12.3 content patching and §13.2.6 appropriate insertion location.
//! Native template insertion identities are shared with the DOM producer;
//! the parser deliberately does not maintain a second association table.
use super::{NodeIdx, TreeBuilder};

impl TreeBuilder {
    // HTML's thorough variant is specific to template closure; other stack
    // checkpoints keep their existing ordinary implied-end-tag algorithm.
    pub(super) fn generate_template_implied_end_tags(&mut self) {
        while self.open.last().is_some_and(|node| {
            self.dom.namespace(*node) == Some(super::Namespace::Html)
                && matches!(self.dom.name(*node), Some("caption" | "colgroup" | "dd" | "dt" |
                    "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc" |
                    "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"))
        }) { self.open.pop(); }
    }

    pub(super) fn template_insertion_place(&self, target: NodeIdx) -> (NodeIdx, Option<NodeIdx>) {
        if let Some((parent, _, end)) = self.dom.template_insertion(target) {
            let before = end.filter(|end| self.dom.get(*end).parent == Some(parent));
            return (parent, before);
        }
        let parent = self.shadow_templates.iter().find(|(template, _)| *template == target)
            .map(|(_, root)| *root).or_else(|| self.dom.template_contents(target)).unwrap_or(target);
        (parent, None)
    }

    pub(super) fn prepare_content_patching(&mut self, scope: NodeIdx, template: NodeIdx) -> bool {
        self.dom.prepare_template_content_patching(scope, template)
            .expect("valid parser patch scope and template")
    }

    pub(super) fn finish_content_patching(&mut self, template: NodeIdx) {
        if let Some((_, start, end)) = self.dom.template_insertion(template) {
            if self.dom.get(start).parent.is_some() { self.dom.detach(start); }
            if let Some(end) = end.filter(|end| *end != start) {
                if self.dom.get(end).parent.is_some() { self.dom.detach(end); }
            }
        }
        // Clear captured state even if its target/start has since retired;
        // a generation-invalid identity must never survive parser completion.
        self.dom.set_template_insertion(template, None).expect("valid parser template");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Token, Tokenizer, feed_with_tokenizer_feedback};

    fn prepared() -> TreeBuilder {
        let mut builder = TreeBuilder::new();
        let mut tokenizer = Tokenizer::new("<body><div><?start name='x'>old<?end>tail</div><section></section><template for=x>".chars().collect());
        loop {
            let token = tokenizer.next_token();
            assert_ne!(token, Token::Eof);
            let stop = matches!(&token, Token::StartTag { name, .. } if name == "template");
            feed_with_tokenizer_feedback(&mut builder, &mut tokenizer, token);
            if stop { break; }
        }
        builder
    }

    #[test]
    fn live_moved_end_appends_and_closure_removes_actual_moved_markers() {
        for move_start in [false, true] {
            let mut builder = prepared();
            let template = *builder.open.last().unwrap();
            let (target, start, end) = builder.dom.template_insertion(template).unwrap();
            let end = end.unwrap();
            let section = builder.dom.tree_order(NodeIdx(0)).into_iter()
                .find(|node| builder.dom.name(*node) == Some("section")).unwrap();
            builder.dom.detach(end);
            builder.dom.attach_last(section, end);
            if move_start {
                builder.dom.detach(start);
                builder.dom.attach_last(section, start);
            }
            builder.feed(Token::Characters("new".into()));
            assert_eq!(String::from_utf16(&builder.dom.serialize_children(target).unwrap()).unwrap(),
                if move_start { "tailnew" } else { "<?start name='x'?>tailnew" });
            builder.feed(Token::EndTag { name: "template".into() });
            assert_eq!(builder.dom.template_insertion(template), None);
            assert_eq!(builder.dom.get(start).parent, None);
            assert_eq!(builder.dom.get(end).parent, None);
            assert_eq!(String::from_utf16(&builder.dom.serialize_children(target).unwrap()).unwrap(), "tailnew");
            assert_eq!(builder.dom.children(section).count(), 0);
        }
    }
}
