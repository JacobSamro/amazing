//! Document tree-derived HTML/SVG surface and driver-controlled readiness.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadyState { Loading, Interactive, Complete }
impl ReadyState {
    pub fn as_str(self) -> &'static str {
        match self { Self::Loading => "loading", Self::Interactive => "interactive", Self::Complete => "complete" }
    }
}
fn html_space(unit: u16) -> bool { matches!(unit, 9 | 10 | 12 | 13 | 32) }
fn collapsed(value: &[u16]) -> Vec<u16> {
    let mut result = Vec::new();
    let mut pending = false;
    for &unit in value {
        if html_space(unit) { pending = !result.is_empty(); }
        else { if pending { result.push(32); pending = false; } result.push(unit); }
    }
    result
}
impl Dom {
    fn document_checked(&self, document: NodeIdx) -> Result<(), DomError> {
        self.check_node(document)?;
        if matches!(self.get(document).kind, NodeKind::Document) { Ok(()) }
        else { Err(DomError::TypeError) }
    }
    pub fn document_ready_state(&self, document: NodeIdx) -> Result<ReadyState, DomError> {
        self.document_checked(document)?;
        Ok(self.document_states.iter().find(|(node, _)| *node == document)
            .map_or(ReadyState::Loading, |(_, state)| *state))
    }
    /// The parser/driver advances readiness; DOM getters never infer it.
    pub fn set_document_ready_state(&mut self, document: NodeIdx, state: ReadyState) -> Result<(), DomError> {
        self.document_checked(document)?;
        if let Some((_, current)) = self.document_states.iter_mut().find(|(node, _)| *node == document) {
            *current = state;
        } else { self.document_states.push((document, state)); }
        Ok(())
    }
    pub fn document_element(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        self.document_checked(document)?;
        Ok(self.children(document).find(|node| matches!(self.get(*node).kind, NodeKind::Element { .. })))
    }
    pub fn document_doctype(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        self.document_checked(document)?;
        Ok(self.children(document).find(|node| matches!(self.get(*node).kind, NodeKind::DocumentType { .. })))
    }
    fn html_root(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        Ok(self.document_element(document)?.filter(|root| self.namespace(*root) == Some(Namespace::Html)
            && self.local_name(*root) == Some("html")))
    }
    pub fn document_head(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        Ok(self.html_root(document)?.and_then(|root| self.children(root).find(|node|
            self.namespace(*node) == Some(Namespace::Html) && self.local_name(*node) == Some("head"))))
    }
    pub fn document_body(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        Ok(self.html_root(document)?.and_then(|root| self.children(root).find(|node|
            self.namespace(*node) == Some(Namespace::Html) &&
                matches!(self.local_name(*node), Some("body" | "frameset")))))
    }
    pub fn set_document_body(&mut self, document: NodeIdx, body: NodeIdx) -> Result<(), DomError> {
        self.document_checked(document)?;
        self.check_node(body)?;
        if self.namespace(body) != Some(Namespace::Html) ||
            !matches!(self.local_name(body), Some("body" | "frameset")) {
            return Err(DomError::HierarchyRequestError);
        }
        match self.document_body(document)? {
            Some(old) if old == body => Ok(()),
            Some(old) => self.replace(old, body, self.get(old).parent.unwrap()).map(|_| ()),
            None => {
                let root = self.document_element(document)?.ok_or(DomError::HierarchyRequestError)?;
                self.pre_insert(body, root, None).map(|_| ())
            }
        }
    }
    fn title_element(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        let Some(root) = self.document_element(document)? else { return Ok(None) };
        if self.namespace(root) == Some(Namespace::Svg) {
            return Ok(self.children(root).find(|node| self.namespace(*node) == Some(Namespace::Svg)
                && self.local_name(*node) == Some("title")));
        }
        Ok(self.tree_order(document).into_iter().skip(1).find(|node| self.namespace(*node) == Some(Namespace::Html)
            && self.local_name(*node) == Some("title")))
    }
    pub fn document_title(&self, document: NodeIdx) -> Result<Vec<u16>, DomError> {
        let Some(title) = self.title_element(document)? else { return Ok(Vec::new()) };
        let mut text = Vec::new();
        for node in self.tree_order(title).into_iter().skip(1) {
            if matches!(self.get(node).kind, NodeKind::Text { .. } | NodeKind::CDATASection { .. }) {
                text.extend(self.character_data_utf16(node).unwrap_or_default());
            }
        }
        Ok(collapsed(&text))
    }
    pub fn set_document_title(&mut self, document: NodeIdx, value: &[u16]) -> Result<(), DomError> {
        self.document_checked(document)?;
        let title = if let Some(title) = self.title_element(document)? { title } else {
            let Some(root) = self.document_element(document)? else { return Ok(()) };
            let parent = if self.namespace(root) == Some(Namespace::Svg) { root }
                else { match self.document_head(document)? { Some(head) => head, None => return Ok(()) } };
            let namespace = self.namespace_uri(parent).map(str::to_owned);
            let title = self.create_element_expanded_in(document, namespace.as_deref(), None, "title")?;
            let before = if parent == root && self.namespace(root) == Some(Namespace::Svg) {
                self.get(root).first_child
            } else { None };
            self.pre_insert(title, parent, before)?;
            title
        };
        let text = if value.is_empty() { None } else {
            let node = self.create_node_in(document, NodeKind::Text { data: String::new() })?;
            self.replace_character_data(node, 0, 0, value)?;
            Some(node)
        };
        self.replace_all(title, text)
    }
    pub fn get_element_by_id(&self, root: NodeIdx, id: &[u16]) -> Result<Option<NodeIdx>, DomError> {
        self.check_node(root)?;
        if !matches!(self.get(root).kind, NodeKind::Document | NodeKind::DocumentFragment) {
            return Err(DomError::TypeError);
        }
        if id.is_empty() { return Ok(None); }
        Ok(self.tree_order(root).into_iter().skip(1).find(|node|
            matches!(self.get(*node).kind, NodeKind::Element { .. }) &&
            self.attribute_utf16(*node, "id").as_deref() == Some(id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_title_body_and_readiness_follow_tree() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let head = dom.create_element(root, "head");
        let title = dom.create_element(head, "title");
        dom.append_child(title, NodeKind::Text { data: "  A \n B  ".into() });
        let old = dom.create_element(root, "body");
        assert_eq!(dom.document_title(NodeIdx(0)).unwrap(), "A B".encode_utf16().collect::<Vec<_>>());
        assert_eq!(dom.document_body(NodeIdx(0)).unwrap(), Some(old));
        let new = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "body").unwrap();
        dom.set_document_body(NodeIdx(0), new).unwrap();
        assert_eq!(dom.document_body(NodeIdx(0)).unwrap(), Some(new));
        assert_eq!(dom.set_document_body(NodeIdx(0), head), Err(DomError::HierarchyRequestError));
        dom.set_document_title(NodeIdx(0), &" X ".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(dom.document_title(NodeIdx(0)).unwrap(), "X".encode_utf16().collect::<Vec<_>>());
        assert_eq!(dom.document_ready_state(NodeIdx(0)), Ok(ReadyState::Loading));
        dom.set_document_ready_state(NodeIdx(0), ReadyState::Complete).unwrap();
        assert_eq!(dom.document_ready_state(NodeIdx(0)), Ok(ReadyState::Complete));
    }
    #[test]
    fn svg_title_and_fragment_id() {
        let mut dom = Dom::new();
        let svg = dom.create_element_ns(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "svg").unwrap();
        let title = dom.create_element_ns(svg, Some("http://www.w3.org/2000/svg"), "title").unwrap();
        dom.append_child(title, NodeKind::Text { data: " SVG ".into() });
        assert_eq!(dom.document_title(NodeIdx(0)).unwrap(), "SVG".encode_utf16().collect::<Vec<_>>());
        dom.set_document_title(NodeIdx(0), &"Changed".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(dom.document_title(NodeIdx(0)).unwrap(), "Changed".encode_utf16().collect::<Vec<_>>());
        let body = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "body").unwrap();
        dom.set_document_body(NodeIdx(0), body).unwrap();
        assert_eq!(dom.document_body(NodeIdx(0)).unwrap(), None);
        let fragment = dom.create_document_fragment(NodeIdx(0)).unwrap();
        let child = dom.create_element_ns(fragment, Some("http://www.w3.org/1999/xhtml"), "span").unwrap();
        dom.set_attribute(child, "id", "needle").unwrap();
        assert_eq!(dom.get_element_by_id(fragment, &"needle".encode_utf16().collect::<Vec<_>>()), Ok(Some(child)));
    }
}
