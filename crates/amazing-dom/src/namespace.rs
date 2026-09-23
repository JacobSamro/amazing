//! Element namespace identity and DOM validate-and-extract (element context).
//! XML parsers must additionally enforce XML's stricter QName grammar themselves.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};

/// An interned URI in one Dom arena. Like NodeIdx, never transfer across arenas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespaceId(usize);

const HTML: &str = "http://www.w3.org/1999/xhtml";
const SVG: &str = "http://www.w3.org/2000/svg";
const MATHML: &str = "http://www.w3.org/1998/Math/MathML";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Clone, Debug)]
pub(crate) struct NamespacedAttribute {
    pub(crate) namespace: String,
    pub(crate) local_name: String,
    pub(crate) prefix: Option<String>,
    pub(crate) value: String,
}

/// Borrowed expanded attribute identity from the element's attribute storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributeRef<'a> {
    pub namespace_uri: Option<&'a str>,
    pub prefix: Option<&'a str>,
    pub local_name: &'a str,
    pub value: &'a str,
}
impl AttributeRef<'_> {
    pub fn qualified_name(&self) -> String {
        match self.prefix {
            Some(prefix) => format!("{prefix}:{}", self.local_name),
            None => self.local_name.to_owned(),
        }
    }
}

impl Dom {
    pub fn create_processing_instruction_in(
        &mut self, document: NodeIdx, target: &str, data: &[u16],
    ) -> Result<NodeIdx, DomError> {
        if !valid_element_local_name(target) || target.eq_ignore_ascii_case("xml") {
            return Err(DomError::InvalidCharacterError);
        }
        if data.windows(2).any(|pair| pair == [b'?' as u16, b'>' as u16]) {
            return Err(DomError::InvalidCharacterError);
        }
        let node = self.create_node_in(document, NodeKind::ProcessingInstruction {
            target: target.into(), data: String::new(),
        })?;
        self.store_character_data(node, data.to_vec());
        Ok(node)
    }
    /// Create a detached doctype in the associated document's arena.
    pub fn create_document_type_in(
        &mut self, document: NodeIdx, name: &str, public_id: &str, system_id: &str,
    ) -> Result<NodeIdx, DomError> {
        if !valid_element_local_name(name) {
            return Err(DomError::InvalidCharacterError);
        }
        self.create_node_in(document, NodeKind::DocumentType {
            name: name.into(), public_id: public_id.into(), system_id: system_id.into(),
        })
    }
    /// Creates and appends an element with an explicit namespace URI.
    /// Empty URI means null; name case and prefix are preserved. Name/namespace
    /// validation follows DOM validate-and-extract, not XML parser tokenization.
    /// Uses checked insertion (including document child validity).
    pub fn create_element_ns(
        &mut self,
        parent: NodeIdx,
        namespace_uri: Option<&str>,
        qualified_name: &str,
    ) -> Result<NodeIdx, DomError> {
        self.check_node(parent)?;
        let document = self.owner_document(parent).unwrap_or(parent);
        let node = self.create_element_ns_in(document, namespace_uri, qualified_name)?;
        self.pre_insert(node, parent, None)?;
        Ok(node)
    }

    /// Detached counterpart for bindings and tree sinks that attach later.
    /// `document` must be a Document in this arena.
    pub fn create_element_ns_in(
        &mut self,
        document: NodeIdx,
        namespace_uri: Option<&str>,
        qualified_name: &str,
    ) -> Result<NodeIdx, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document) {
            return Err(DomError::NotSupportedError);
        }
        let uri = namespace_uri.filter(|uri| !uri.is_empty());
        let (prefix, local_name) = validate_and_extract(uri, qualified_name, false)?;
        self.create_element_expanded_in(document, uri, prefix, local_name)
    }

    /// Parser primitive: append an element from already-validated expanded names.
    /// The caller owns XML QName grammar, prefix binding/reserved-name checks and
    /// namespace resolution. Deliberately does not impose DOM createElementNS
    /// validation: an unprefixed XML element named xmlns is valid here.
    /// Checked tree insertion and document ownership still apply.
    pub fn create_element_expanded(
        &mut self,
        parent: NodeIdx,
        namespace_uri: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
    ) -> Result<NodeIdx, DomError> {
        self.check_node(parent)?;
        let document = self.owner_document(parent).unwrap_or(parent);
        let node = self.create_element_expanded_in(document, namespace_uri, prefix, local_name)?;
        self.pre_insert(node, parent, None)?;
        Ok(node)
    }

    /// Detached parser counterpart. The same expanded-name preconditions apply.
    pub fn create_element_expanded_in(
        &mut self,
        document: NodeIdx,
        namespace_uri: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
    ) -> Result<NodeIdx, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document) {
            return Err(DomError::NotSupportedError);
        }
        let uri = namespace_uri.filter(|uri| !uri.is_empty());
        let namespace = match uri {
            None => Namespace::Null,
            Some(HTML) => Namespace::Html,
            Some(SVG) => Namespace::Svg,
            Some(MATHML) => Namespace::MathMl,
            Some(uri) => {
                let index = self
                    .namespace_uris
                    .iter()
                    .position(|entry| entry == uri)
                    .unwrap_or_else(|| {
                        self.namespace_uris.push(uri.to_owned());
                        self.namespace_uris.len() - 1
                    });
                Namespace::Other(NamespaceId(index))
            }
        };
        let node = self.create_node_in(
            document,
            NodeKind::Element {
                name: local_name.to_owned(),
                attrs: Vec::new(),
                namespace,
            },
        )?;
        self.nodes[node.0 as usize].prefix = prefix.map(str::to_owned);
        Ok(node)
    }

    /// DOM API qualified-name seam; keeps get_attribute_ns keyed by URI/local name.
    /// Updating an existing expanded attribute changes its value, preserving its
    /// existing prefix, as required by DOM's set-an-attribute-value algorithm.
    pub fn set_attribute_qualified_ns(
        &mut self,
        node: NodeIdx,
        namespace_uri: Option<&str>,
        qualified_name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        let uri = namespace_uri.filter(|uri| !uri.is_empty());
        let (prefix, local_name) = validate_and_extract(uri, qualified_name, true)?;
        self.set_attribute_expanded(node, uri, prefix, local_name, value)
    }

    /// Parser/value-setting primitive for already-validated expanded attributes.
    /// Parser owns QName, reserved namespace, and duplicate-expanded-name checks.
    /// Existing attribute identity/prefix is retained when replacing its value.
    pub fn set_attribute_expanded(
        &mut self,
        node: NodeIdx,
        namespace_uri: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::Element { .. }) {
            return Err(DomError::NotSupportedError);
        }
        let uri = namespace_uri.filter(|uri| !uri.is_empty());
        // The parser must not pass a prefix with a null namespace.
        if prefix.is_some() && uri.is_none() {
            return Err(DomError::NamespaceError);
        }
        let exists = self.get_attribute_ns(node, uri, local_name).is_some();
        self.set_attribute_ns(node, uri, local_name, value)?;
        if !exists {
            if let Some(uri) = uri {
                let attr = self.nodes[node.0 as usize]
                    .namespaced_attributes
                    .iter_mut()
                    .find(|attr| attr.namespace == uri && attr.local_name == local_name)
                    .unwrap();
                attr.prefix = prefix.map(str::to_owned);
            }
            self.sync_attached_attr_prefix(node, uri, local_name, prefix);
        }
        Ok(())
    }

    /// Lookup by expanded identity; null namespace never inherits from its element.
    pub fn attribute_ns(
        &self,
        node: NodeIdx,
        namespace_uri: Option<&str>,
        local_name: &str,
    ) -> Option<AttributeRef<'_>> {
        if let Some(uri) = namespace_uri.filter(|uri| !uri.is_empty()) {
            let attr = self
                .get(node)
                .namespaced_attributes
                .iter()
                .find(|attr| attr.namespace == uri && attr.local_name == local_name)?;
            Some(AttributeRef {
                namespace_uri: Some(&attr.namespace),
                prefix: attr.prefix.as_deref(),
                local_name: &attr.local_name,
                value: &attr.value,
            })
        } else {
            let (local_name, value) = self
                .attrs_of(node)?
                .iter()
                .find(|(local, _)| local == local_name)?;
            Some(AttributeRef {
                namespace_uri: None,
                prefix: None,
                local_name,
                value,
            })
        }
    }

    /// URI of an element namespace. Null namespaces and non-elements return None;
    /// namespace(node) distinguishes Some(Namespace::Null) from non-elements.
    pub fn namespace_uri(&self, node: NodeIdx) -> Option<&str> {
        match self.namespace(node)? {
            Namespace::Html => Some(HTML),
            Namespace::Svg => Some(SVG),
            Namespace::MathMl => Some(MATHML),
            Namespace::Null => None,
            Namespace::Other(NamespaceId(index)) => Some(&self.namespace_uris[index]),
        }
    }

    /// Element prefix, or None for unprefixed elements and non-elements.
    pub fn prefix(&self, node: NodeIdx) -> Option<&str> {
        self.name(node)?;
        self.get(node).prefix.as_deref()
    }

    /// Element local name, preserving case. Alias for the existing name accessor.
    pub fn local_name(&self, node: NodeIdx) -> Option<&str> {
        self.name(node)
    }

    /// Element qualified name, preserving its prefix and local name.
    pub fn qualified_name(&self, node: NodeIdx) -> Option<String> {
        let local_name = self.local_name(node)?;
        Some(match self.prefix(node) {
            Some(prefix) => format!("{prefix}:{local_name}"),
            None => local_name.to_owned(),
        })
    }
}

fn forbidden(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\x0c' | '\r' | ' ' | '/' | '>')
}
fn valid_element_local_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_alphabetic() {
        return !chars.any(forbidden);
    }
    if !matches!(first, ':' | '_') && first.is_ascii() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_') || !c.is_ascii())
}
pub(crate) fn validate_and_extract<'a>(
    uri: Option<&str>,
    qualified_name: &'a str,
    attribute: bool,
) -> Result<(Option<&'a str>, &'a str), DomError> {
    let (prefix, local) = match qualified_name.split_once(':') {
        Some((prefix, local)) => {
            if prefix.is_empty() || prefix.chars().any(forbidden) {
                return Err(DomError::InvalidCharacterError);
            }
            (Some(prefix), local)
        }
        None => (None, qualified_name),
    };
    let valid = if attribute {
        !local.is_empty() && !local.chars().any(|c| forbidden(c) || c == '=')
    } else {
        valid_element_local_name(local)
    };
    if !valid {
        return Err(DomError::InvalidCharacterError);
    }
    if (prefix.is_some() && uri.is_none())
        || (prefix == Some("xml") && uri != Some(XML))
        || ((qualified_name == "xmlns" || prefix == Some("xmlns")) && uri != Some(XMLNS))
        || (uri == Some(XMLNS) && qualified_name != "xmlns" && prefix != Some("xmlns"))
    {
        return Err(DomError::NamespaceError);
    }
    Ok((prefix, local))
}
pub(crate) fn valid_attribute_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| forbidden(c) || c == '=')
}

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests;
