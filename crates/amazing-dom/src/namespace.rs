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

impl Dom {
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
        let (prefix, local_name) = validate_and_extract(uri, qualified_name)?;
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
fn validate_and_extract<'a>(
    uri: Option<&str>,
    qualified_name: &'a str,
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
    if !valid_element_local_name(local) {
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

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests;
