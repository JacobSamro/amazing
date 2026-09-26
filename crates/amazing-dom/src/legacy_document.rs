//! Obsolete document and HTML body color reflection retained for compatibility.
use crate::{Dom, DomError, Namespace, NodeIdx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegacyColor { Text, Background, Link, VisitedLink, ActiveLink }
impl LegacyColor {
    pub const fn attribute(self) -> &'static str {
        match self { Self::Text => "text", Self::Background => "bgcolor", Self::Link => "link",
            Self::VisitedLink => "vlink", Self::ActiveLink => "alink" }
    }
}
impl Dom {
    fn legacy_color_body(&self, document: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        Ok(self.document_body(document)?.filter(|body| self.local_name(*body) == Some("body")))
    }
    pub fn body_legacy_color(&self, body: NodeIdx, color: LegacyColor) -> Result<Vec<u16>, DomError> {
        self.check_node(body)?;
        if self.namespace(body) != Some(Namespace::Html) || self.local_name(body) != Some("body") {
            return Err(DomError::TypeError);
        }
        Ok(self.attribute_utf16(body, color.attribute()).unwrap_or_default())
    }
    pub fn set_body_legacy_color(&mut self, body: NodeIdx, color: LegacyColor, value: Vec<u16>) -> Result<(), DomError> {
        self.body_legacy_color(body, color)?;
        let name = color.attribute();
        self.set_attribute(body, name, &String::from_utf16_lossy(&value))?;
        let attribute = self.get_attribute_node(body, None, name).ok_or(DomError::InvalidStateError)?;
        self.store_attr_value_units(attribute, value)
    }
    pub fn document_legacy_color(&self, document: NodeIdx, color: LegacyColor) -> Result<Vec<u16>, DomError> {
        match self.legacy_color_body(document)? {
            Some(body) => self.body_legacy_color(body, color), None => Ok(Vec::new()),
        }
    }
    pub fn set_document_legacy_color(&mut self, document: NodeIdx, color: LegacyColor, value: Vec<u16>) -> Result<(), DomError> {
        if let Some(body) = self.legacy_color_body(document)? { self.set_body_legacy_color(body, color, value)?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_colors_follow_the_current_body_and_ignore_framesets() {
        let mut dom = Dom::new();
        let document = NodeIdx(0);
        let root = dom.create_element(document, "html");
        for color in [LegacyColor::Text, LegacyColor::Background, LegacyColor::Link,
            LegacyColor::VisitedLink, LegacyColor::ActiveLink] {
            dom.set_document_legacy_color(document, color, vec![65]).unwrap();
            assert!(dom.document_legacy_color(document, color).unwrap().is_empty());
        }
        let body = dom.create_element(root, "body");
        let value = vec![0xd800, 65, 0xdc00];
        dom.set_document_legacy_color(document, LegacyColor::Text, value.clone()).unwrap();
        assert_eq!(dom.body_legacy_color(body, LegacyColor::Text).unwrap(), value);
        let frameset = dom.create_element_ns_in(document, Some("http://www.w3.org/1999/xhtml"), "frameset").unwrap();
        dom.set_document_body(document, frameset).unwrap();
        dom.set_document_legacy_color(document, LegacyColor::Text, vec![66]).unwrap();
        assert!(dom.document_legacy_color(document, LegacyColor::Text).unwrap().is_empty());
        assert_eq!(dom.get_attribute_ns(frameset, None, "text"), None);
        dom.set_document_body(document, body).unwrap();
        assert_eq!(dom.document_legacy_color(document, LegacyColor::Text).unwrap(), value);
    }
}
