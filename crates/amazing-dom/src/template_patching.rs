//! HTML §4.12.3: native template content patching identities and preparation.
use crate::{Dom, DomError, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Insertion {
    target: (NodeIdx, u64),
    start: (NodeIdx, u64),
    end: Option<(NodeIdx, u64)>,
}
impl Dom {
    /// Stored identities survive detachment/adoption, but never alias recycled nodes.
    /// A stale end is independently null; target/start remain the actual identities.
    pub fn template_insertion(&self, template: NodeIdx) -> Option<(NodeIdx, NodeIdx, Option<NodeIdx>)> {
        self.check_node(template).ok()?;
        if !self.is_html_template(template) { return None; }
        let state = self.get(template).template_insertion?;
        let valid = |(node, generation)| self.node_generation(node) == Some(generation);
        if !valid(state.target) || !valid(state.start) { return None; }
        Some((state.target.0, state.start.0, state.end.filter(|entry| valid(*entry)).map(|entry| entry.0)))
    }
    /// Parser-only state installation. All genuine kinds are checked before any write.
    pub fn set_template_insertion(&mut self, template: NodeIdx,
        insertion: Option<(NodeIdx, NodeIdx, Option<NodeIdx>)>) -> Result<(), DomError> {
        self.check_node(template)?;
        if !self.is_html_template(template) { return Err(DomError::TypeError); }
        let state = if let Some((target, start, end)) = insertion {
            self.check_node(target)?;
            self.check_node(start)?;
            if !matches!(self.get(target).kind, NodeKind::Element { .. } | NodeKind::DocumentFragment)
                || !matches!(self.get(start).kind, NodeKind::ProcessingInstruction { .. }) {
                return Err(DomError::TypeError);
            }
            if let Some(end) = end {
                self.check_node(end)?;
                if !matches!(self.get(end).kind, NodeKind::ProcessingInstruction { .. }) {
                    return Err(DomError::TypeError);
                }
            }
            Some(Insertion { target: (target, self.node_generation(target).unwrap()),
                start: (start, self.node_generation(start).unwrap()),
                end: end.map(|node| (node, self.node_generation(node).unwrap())) })
        } else { None };
        self.nodes[template.0 as usize].template_insertion = state;
        Ok(())
    }
    fn template_markers(&self, scope: NodeIdx, name: &[u16])
        -> Result<Option<(NodeIdx, Option<NodeIdx>)>, DomError> {
        for node in self.tree_order(scope).into_iter().skip(1) {
            let NodeKind::ProcessingInstruction { target, .. } = &self.get(node).kind else { continue; };
            if !self.processing_instruction_attributes(node)?.iter()
                .any(|(key, value)| key.as_slice() == [110,97,109,101] && value.as_slice() == name) { continue; }
            if target == "marker" { return Ok(Some((node, Some(node)))); }
            if target != "start" { continue; }
            let mut nesting = 0usize;
            let mut sibling = self.get(node).next_sibling;
            while let Some(id) = sibling {
                if let NodeKind::ProcessingInstruction { target, .. } = &self.get(id).kind {
                    if target == "start" { nesting += 1; }
                    if target == "end" {
                        if nesting == 0 { return Ok(Some((node, Some(id)))); }
                        nesting -= 1;
                    }
                }
                sibling = self.get(id).next_sibling;
            }
            return Ok(Some((node, None)));
        }
        Ok(None)
    }
    /// Prepare the actual marker range, preserving ordinary removal bookkeeping.
    pub fn prepare_template_content_patching(&mut self, scope: NodeIdx, template: NodeIdx)
        -> Result<bool, DomError> {
        self.check_node(scope)?;
        self.check_node(template)?;
        if !matches!(self.get(scope).kind, NodeKind::Element { .. } | NodeKind::DocumentFragment)
            || !self.is_html_template(template) { return Err(DomError::TypeError); }
        let Some(name) = self.attribute_utf16(template, "for").filter(|name| !name.is_empty()) else { return Ok(false); };
        let Some((start, end)) = self.template_markers(scope, &name)? else { return Ok(false); };
        let target = self.get(start).parent.ok_or(DomError::InvalidStateError)?;
        self.set_template_insertion(template, Some((target, start, end)))?;
        if end == Some(start) { return Ok(true); }
        let mut removed = Vec::new();
        let mut next = self.get(start).next_sibling;
        while let Some(node) = next {
            if Some(node) == end { break; }
            removed.push(node);
            next = self.get(node).next_sibling;
        }
        // The primary algorithm removes each captured node even if it was moved.
        // There is no parent-identity shortcut; canonical removal owns its effects.
        for node in removed {
            if let Some(parent) = self.get(node).parent { self.pre_remove(node, parent)?; }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Dom, NodeIdx, NodeIdx) {
        let mut dom = Dom::new();
        let scope = dom.create_element(NodeIdx(0), "section");
        let template = dom.create_element(scope, "template");
        dom.set_attribute(template, "for", "patch").unwrap();
        (dom, scope, template)
    }
    fn pi(dom: &mut Dom, parent: NodeIdx, target: &str, name: &str) -> NodeIdx {
        let document = dom.owner_document(parent).unwrap();
        let data: Vec<_> = format!("name='{name}'").encode_utf16().collect();
        let node = dom.create_processing_instruction_in(document, target, &data).unwrap();
        dom.pre_insert(node, parent, None).unwrap();
        node
    }
    #[test]
    fn marker_state_keeps_identity_without_removing_neighbors() {
        let (mut dom, scope, template) = fixture();
        let marker = pi(&mut dom, scope, "marker", "patch");
        let text = dom.create_text(scope, "keep");
        assert_eq!(dom.prepare_template_content_patching(scope, template), Ok(true));
        assert_eq!(dom.template_insertion(template), Some((scope, marker, Some(marker))));
        assert_eq!(dom.get(text).parent, Some(scope));
        dom.set_template_insertion(template, None).unwrap();
        assert_eq!(dom.template_insertion(template), None);
    }
    #[test]
    fn range_removes_nested_markers_and_ordinary_nodes_with_observer_records() {
        let (mut dom, scope, template) = fixture();
        let start = pi(&mut dom, scope, "start", "patch");
        let text = dom.create_text(scope, "old");
        let nested = pi(&mut dom, scope, "start", "unrelated");
        let inner_end = pi(&mut dom, scope, "end", "ignored");
        let end = pi(&mut dom, scope, "end", "different-name");
        let tail = dom.create_text(scope, "keep");
        let observer = dom.create_mutation_observer(1);
        dom.observe(observer, scope, crate::mutation::ObserveOptions { child_list:true, ..Default::default() }).unwrap();
        assert_eq!(dom.prepare_template_content_patching(scope, template), Ok(true));
        assert_eq!(dom.template_insertion(template), Some((scope,start,Some(end))));
        for node in [text,nested,inner_end] { assert_eq!(dom.get(node).parent,None); }
        assert_eq!(dom.get(tail).parent,Some(scope));
        let records = dom.take_mutation_records(observer);
        assert_eq!(records.len(),3);
        assert_eq!(records.iter().flat_map(|r|r.removed_nodes.iter().copied()).collect::<Vec<_>>(),[text,nested,inner_end]);
    }
    #[test]
    fn absent_end_removes_to_parent_end_and_first_tree_marker_wins() {
        let (mut dom, scope, template) = fixture();
        let branch = dom.create_element(scope,"div");
        let start = pi(&mut dom,branch,"start","patch");
        let text = dom.create_text(branch,"old");
        let later = pi(&mut dom,scope,"marker","patch");
        assert_eq!(dom.prepare_template_content_patching(scope,template),Ok(true));
        assert_eq!(dom.template_insertion(template),Some((branch,start,None)));
        assert_eq!(dom.get(text).parent,None);
        assert_eq!(dom.get(later).parent,Some(scope));
    }
    #[test]
    fn marker_names_use_exact_units_and_scope_excludes_inert_contents() {
        let (mut dom,scope,template)=fixture();
        let inert=dom.ensure_template_contents(template).unwrap();
        pi(&mut dom,inert,"marker","patch");
        assert_eq!(dom.prepare_template_content_patching(scope,template),Ok(false));
        let marker=pi(&mut dom,scope,"marker","wrong");
        dom.set_processing_instruction_attribute(marker,&[110,97,109,101],&[0xd800]).unwrap();
        let attr=dom.get_attribute_node(template,None,"for").unwrap();
        dom.set_attr_value(attr,vec![0xfffd]).unwrap();
        assert_eq!(dom.prepare_template_content_patching(scope,template),Ok(false));
        dom.set_attr_value(attr,vec![0xd800]).unwrap();
        assert_eq!(dom.prepare_template_content_patching(scope,template),Ok(true));
    }
    #[test]
    fn insertion_generation_guards_stale_end_then_stale_start_and_recycled_template() {
        let (mut dom,scope,template)=fixture();
        let start=pi(&mut dom,scope,"start","patch");
        let end=pi(&mut dom,scope,"end","");
        dom.set_template_insertion(template,Some((scope,start,Some(end)))).unwrap();
        dom.detach(end);dom.retire_unreachable_node(end).unwrap();
        let replacement=dom.create_text(scope,"reused");assert_eq!(replacement,end);
        assert_eq!(dom.template_insertion(template),Some((scope,start,None)));
        dom.detach(start);dom.retire_unreachable_node(start).unwrap();
        assert_eq!(dom.template_insertion(template),None);
        dom.detach(template);dom.retire_unreachable_node(template).unwrap();
        let fresh=dom.create_element(scope,"template");assert_eq!(fresh,template);
        assert_eq!(dom.template_insertion(fresh),None);
    }
    #[test]
    fn validation_is_atomic_and_fresh_clone_or_import_never_copies_patch_state() {
        let (mut dom,scope,template)=fixture();
        let marker=pi(&mut dom,scope,"marker","patch");
        dom.set_template_insertion(template,Some((scope,marker,Some(marker)))).unwrap();
        let old=dom.template_insertion(template);
        for value in [(NodeIdx(0),marker,None),(scope,scope,None),(scope,marker,Some(scope))] {
            assert_eq!(dom.set_template_insertion(template,Some(value)),Err(DomError::TypeError));
            assert_eq!(dom.template_insertion(template),old);
        }
        assert!(dom.set_template_insertion(NodeIdx(u32::MAX),None).is_err());
        for deep in [false,true] {
            let copy=dom.clone_node(template,deep).unwrap();
            assert_eq!(dom.template_insertion(copy),None);
        }
        let mut target=Dom::new();
        let fragment=target.import_fragment(&dom,NodeIdx(0)).unwrap();
        let imported_scope=target.children(fragment).next().unwrap();
        let copy=target.children(imported_scope).find(|n|target.is_html_template(*n)).unwrap();
        assert_eq!(target.template_insertion(copy),None);
        let doc=dom.create_document();dom.adopt(template,doc).unwrap();
        assert_eq!(dom.template_insertion(template),old);
    }
    #[test]
    fn empty_missing_wrong_kind_and_nonmatching_requests_preserve_existing_state() {
        let (mut dom,scope,template)=fixture();
        let marker=pi(&mut dom,scope,"marker","patch");
        dom.prepare_template_content_patching(scope,template).unwrap();
        let old=dom.template_insertion(template);
        for name in ["","absent","PATCH"] {
            dom.set_attribute(template,"for",name).unwrap();
            assert_eq!(dom.prepare_template_content_patching(scope,template),Ok(false));
            assert_eq!(dom.template_insertion(template),old);
        }
        dom.remove_attribute(template,"for").unwrap();
        assert_eq!(dom.prepare_template_content_patching(scope,template),Ok(false));
        assert_eq!(dom.prepare_template_content_patching(NodeIdx(0),template),Err(DomError::TypeError));
        assert_eq!(dom.prepare_template_content_patching(scope,scope),Err(DomError::TypeError));
        assert_eq!(dom.get(marker).parent,Some(scope));
    }
}
