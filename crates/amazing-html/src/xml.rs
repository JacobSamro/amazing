//! Fatal-error XML 1.0 Fifth Edition / Namespaces 1.0 parser.
//!
//! Input is already decoded Unicode; CR/CRLF are normalized before parsing.
//! DTD internal subsets, external entity loading, scripting, and document.write
//! are unsupported. External doctype identifiers are retained but never fetched.
//! No HTML recovery, implicit elements, case folding, or HTML named entities.

use amazing_dom::{Dom, Namespace, NodeIdx, NodeKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlErrorKind {
    Malformed,
    Unsupported,
    Limit,
    Sink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    /// UTF-8 byte offset in the newline-normalized input (after optional BOM).
    pub offset: usize,
    pub kind: XmlErrorKind,
    pub message: String,
}
impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XML {:?} at byte {}: {}",
            self.kind, self.offset, self.message
        )
    }
}
impl std::error::Error for XmlError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlDeclaration {
    pub encoding: Option<String>,
    pub standalone: Option<bool>,
}

/// Resolved XML name, not a second DOM namespace enum. Null namespace is None.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlName {
    pub qualified: String,
    pub local: String,
    pub prefix: Option<String>,
    pub namespace_uri: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlAttribute {
    pub name: XmlName,
    pub value: String,
}

/// Lower-crate boundary for namespace-aware DOM creation. Implementations must
/// discard their partial document if parsing fails. No callbacks follow failure.
pub trait XmlTreeSink {
    type Node: Copy;
    fn document(&self) -> Self::Node;
    fn element(
        &mut self,
        parent: Self::Node,
        name: &XmlName,
        attrs: &[XmlAttribute],
    ) -> Result<Self::Node, String>;
    fn text(&mut self, parent: Self::Node, data: &str) -> Result<(), String>;
    fn comment(&mut self, parent: Self::Node, data: &str) -> Result<(), String>;
    fn processing_instruction(
        &mut self,
        parent: Self::Node,
        target: &str,
        data: &str,
    ) -> Result<(), String>;
    fn doctype(&mut self, name: &str, public_id: &str, system_id: &str) -> Result<(), String>;
}

/// Parses XML/XHTML into the engine DOM. A failure never exposes a partial DOM.
/// XHTML must declare its HTML namespace, as required by XML namespace rules.
pub fn parse_xml(src: &str) -> Result<Dom, XmlError> {
    let mut dom = Dom::new();
    parse_xml_into(src, &mut dom)?;
    Ok(dom)
}

impl XmlTreeSink for Dom {
    type Node = NodeIdx;
    fn document(&self) -> NodeIdx {
        NodeIdx(0)
    }
    fn element(
        &mut self,
        parent: NodeIdx,
        name: &XmlName,
        attrs: &[XmlAttribute],
    ) -> Result<NodeIdx, String> {
        // Arbitrary/null element namespaces require the DOM-owned creation seam.
        // Never silently label an unknown XML vocabulary as HTML.
        let namespace = match name.namespace_uri.as_deref() {
            Some(HTML_NAMESPACE) => Namespace::Html,
            Some(SVG_NAMESPACE) => Namespace::Svg,
            Some(MATHML_NAMESPACE) => Namespace::MathMl,
            _ => return Err("DOM cannot yet represent this element namespace".into()),
        };
        let node = self.append_child(
            parent,
            NodeKind::Element {
                name: name.local.clone(),
                attrs: Vec::new(),
                namespace,
            },
        );
        for attr in attrs {
            self.set_attribute_ns(
                node,
                attr.name.namespace_uri.as_deref(),
                &attr.name.local,
                &attr.value,
            )
            .map_err(|e| format!("{e:?}"))?;
        }
        Ok(node)
    }
    fn text(&mut self, parent: NodeIdx, data: &str) -> Result<(), String> {
        self.create_text(parent, data);
        Ok(())
    }
    fn comment(&mut self, parent: NodeIdx, data: &str) -> Result<(), String> {
        self.append_child(parent, NodeKind::Comment { data: data.into() });
        Ok(())
    }
    fn processing_instruction(
        &mut self,
        parent: NodeIdx,
        target: &str,
        data: &str,
    ) -> Result<(), String> {
        self.append_child(
            parent,
            NodeKind::ProcessingInstruction {
                target: target.into(),
                data: data.into(),
            },
        );
        Ok(())
    }
    fn doctype(&mut self, name: &str, public_id: &str, system_id: &str) -> Result<(), String> {
        self.doctype = Some(name.into());
        self.append_child(
            NodeIdx(0),
            NodeKind::DocumentType {
                name: name.into(),
                public_id: public_id.into(),
                system_id: system_id.into(),
            },
        );
        Ok(())
    }
}

struct Frame<N> {
    node: N,
    qualified: String,
    previous_bindings: Vec<(String, Option<String>)>,
}
struct Parser<'a> {
    src: &'a str,
    pos: usize,
    external_subset: bool,
    standalone: bool,
}
type ResultXml<T> = Result<T, XmlError>;

/// Parses into an owned consumer. The declaration is returned only on success.
/// Limits: 16 MiB input, 512 open elements, 1024 attributes per element.
pub fn parse_xml_into<S: XmlTreeSink>(
    src: &str,
    sink: &mut S,
) -> ResultXml<Option<XmlDeclaration>> {
    if src.len() > 16 * 1024 * 1024 {
        return Err(XmlError {
            offset: 0,
            kind: XmlErrorKind::Limit,
            message: "document exceeds 16 MiB".into(),
        });
    }
    let normalized = src
        .strip_prefix('\u{feff}')
        .unwrap_or(src)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut p = Parser {
        src: &normalized,
        pos: 0,
        external_subset: false,
        standalone: false,
    };
    if let Some((offset, _)) = normalized.char_indices().find(|(_, c)| !xml_char(*c)) {
        p.pos = offset;
        return Err(p.error("forbidden XML character"));
    }
    let declaration = if p.rest().starts_with("<?xml") && p.rest()[5..].starts_with(xml_space) {
        Some(p.declaration()?)
    } else {
        None
    };
    p.standalone = declaration
        .as_ref()
        .is_some_and(|d| d.standalone == Some(true));
    let mut stack: Vec<Frame<S::Node>> = Vec::new();
    let mut bindings = BTreeMap::from([("xml".into(), XML_NAMESPACE.into())]);
    let mut root_seen = false;
    let mut doctype_seen = false;
    while !p.rest().is_empty() {
        let parent = stack.last().map_or(sink.document(), |f| f.node);
        if p.take("<!--") {
            let data = p.until("-->")?;
            if data.contains("--") || data.ends_with('-') {
                return Err(p.error("invalid comment"));
            }
            p.sink(sink.comment(parent, data))?;
        } else if p.take("<?") {
            let target = p.name()?;
            if target.eq_ignore_ascii_case("xml") || target.contains(':') {
                return Err(p.error("reserved or invalid PI target"));
            }
            let data = if p.take("?>") {
                ""
            } else {
                p.require_space()?;
                p.until("?>")?
            };
            p.sink(sink.processing_instruction(parent, &target, data))?;
        } else if p.take("<!DOCTYPE") {
            if root_seen || doctype_seen {
                return Err(p.error("doctype must precede the sole root"));
            }
            doctype_seen = true;
            p.require_space()?;
            let name = p.qname()?;
            let separated = p.space();
            let mut public_id = String::new();
            let mut system_id = String::new();
            if separated && p.take("SYSTEM") {
                p.external_subset = true;
                p.require_space()?;
                system_id = p.literal()?.into();
                p.space();
            } else if separated && p.take("PUBLIC") {
                p.external_subset = true;
                p.require_space()?;
                public_id = p.literal()?.into();
                if !public_id.chars().all(pubid_char) {
                    return Err(p.error("invalid public identifier"));
                }
                p.require_space()?;
                system_id = p.literal()?.into();
                p.space();
            }
            if p.rest().starts_with('[') {
                return Err(p.special(
                    XmlErrorKind::Unsupported,
                    "DTD internal subsets are unsupported",
                ));
            }
            p.expect(">")?;
            p.sink(sink.doctype(&name, &public_id, &system_id))?;
        } else if p.take("<![CDATA[") {
            if stack.is_empty() {
                return Err(p.error("CDATA outside root"));
            }
            let data = p.until("]]>")?;
            p.sink(sink.text(parent, data))?;
        } else if p.take("</") {
            let name = p.qname()?;
            p.space();
            p.expect(">")?;
            let frame = stack
                .pop()
                .ok_or_else(|| p.error("end tag without start tag"))?;
            if frame.qualified != name {
                return Err(p.error("mismatched end tag"));
            }
            restore_bindings(&mut bindings, frame.previous_bindings);
        } else if p.take("<") {
            if stack.is_empty() && root_seen {
                return Err(p.error("multiple document elements"));
            }
            if stack.len() >= 512 {
                return Err(p.special(XmlErrorKind::Limit, "element depth exceeds 512"));
            }
            let qualified = p.qname()?;
            let mut attrs = Vec::new();
            let mut raw_names = BTreeSet::new();
            let empty;
            loop {
                let separated = p.space();
                if p.take("/>") {
                    empty = true;
                    break;
                }
                if p.take(">") {
                    empty = false;
                    break;
                }
                if !separated {
                    return Err(p.error("attributes require whitespace"));
                }
                if attrs.len() >= 1024 {
                    return Err(p.special(XmlErrorKind::Limit, "attribute count exceeds 1024"));
                }
                let key = p.qname()?;
                if !raw_names.insert(key.clone()) {
                    return Err(p.error("duplicate attribute name"));
                }
                p.space();
                p.expect("=")?;
                p.space();
                let raw = p.literal()?;
                if raw.contains('<') {
                    return Err(p.error("literal < in attribute"));
                }
                let value = p.references(raw, true)?;
                attrs.push((key, value));
            }
            let mut previous_bindings = Vec::new();
            for (key, uri) in &attrs {
                let prefix = if key == "xmlns" {
                    Some("")
                } else {
                    key.strip_prefix("xmlns:")
                };
                if let Some(prefix) = prefix {
                    if prefix == "xmlns"
                        || uri == XMLNS_NAMESPACE
                        || (prefix == "xml") != (uri == XML_NAMESPACE)
                        || (!prefix.is_empty() && uri.is_empty())
                    {
                        return Err(p.error("illegal namespace binding"));
                    }
                    let previous = bindings.insert(prefix.into(), uri.clone());
                    previous_bindings.push((prefix.to_owned(), previous));
                }
            }
            let name = p.resolve(&qualified, &bindings, false)?;
            let mut expanded = BTreeSet::new();
            let mut resolved = Vec::new();
            for (key, value) in attrs {
                let name = if key == "xmlns" || key.starts_with("xmlns:") {
                    let (prefix, local) = split_name(&key);
                    XmlName {
                        qualified: key.clone(),
                        local: local.into(),
                        prefix: prefix.map(str::to_owned),
                        namespace_uri: Some(XMLNS_NAMESPACE.into()),
                    }
                } else {
                    p.resolve(&key, &bindings, true)?
                };
                if !expanded.insert((name.namespace_uri.clone(), name.local.clone())) {
                    return Err(p.error("duplicate expanded attribute name"));
                }
                resolved.push(XmlAttribute { name, value });
            }
            let node = p.sink(sink.element(parent, &name, &resolved))?;
            root_seen = true;
            if !empty {
                stack.push(Frame {
                    node,
                    qualified,
                    previous_bindings,
                });
            } else {
                restore_bindings(&mut bindings, previous_bindings);
            }
        } else {
            let end = p.rest().find('<').unwrap_or(p.rest().len());
            let raw = &p.rest()[..end];
            p.pos += end;
            if stack.is_empty() {
                if !raw.chars().all(xml_space) {
                    return Err(p.error("character data outside root"));
                }
            } else {
                if raw.contains("]]>") {
                    return Err(p.error("]]> in character data"));
                }
                let data = p.references(raw, false)?;
                p.sink(sink.text(parent, &data))?;
            }
        }
    }
    if !stack.is_empty() {
        return Err(p.error("unclosed element"));
    }
    if !root_seen {
        return Err(p.error("missing document element"));
    }
    Ok(declaration)
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }
    fn error(&self, message: &str) -> XmlError {
        self.special(XmlErrorKind::Malformed, message)
    }
    fn special(&self, kind: XmlErrorKind, message: &str) -> XmlError {
        XmlError {
            offset: self.pos,
            kind,
            message: message.into(),
        }
    }
    fn sink<T>(&self, result: Result<T, String>) -> ResultXml<T> {
        result.map_err(|s| self.special(XmlErrorKind::Sink, &s))
    }
    fn take(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> ResultXml<()> {
        if self.take(s) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {s}")))
        }
    }
    fn space(&mut self) -> bool {
        let before = self.pos;
        while let Some(c) = self.rest().chars().next().filter(|c| xml_space(*c)) {
            self.pos += c.len_utf8();
        }
        self.pos != before
    }
    fn require_space(&mut self) -> ResultXml<()> {
        if self.space() {
            Ok(())
        } else {
            Err(self.error("expected XML whitespace"))
        }
    }
    fn until(&mut self, delimiter: &str) -> ResultXml<&'a str> {
        let end = self
            .rest()
            .find(delimiter)
            .ok_or_else(|| self.error("unterminated markup"))?;
        let result = &self.rest()[..end];
        self.pos += end + delimiter.len();
        Ok(result)
    }
    fn literal(&mut self) -> ResultXml<&'a str> {
        let quote = self
            .rest()
            .chars()
            .next()
            .filter(|c| *c == '\'' || *c == '"')
            .ok_or_else(|| self.error("expected quoted value"))?;
        self.pos += 1;
        let end = self
            .rest()
            .find(quote)
            .ok_or_else(|| self.error("unterminated quoted value"))?;
        let result = &self.rest()[..end];
        self.pos += end + 1;
        Ok(result)
    }
    fn name(&mut self) -> ResultXml<String> {
        let first = self
            .rest()
            .chars()
            .next()
            .ok_or_else(|| self.error("expected name"))?;
        if !name_start(first) {
            return Err(self.error("invalid name start"));
        }
        let begin = self.pos;
        self.pos += first.len_utf8();
        while let Some(c) = self.rest().chars().next().filter(|c| name_char(*c)) {
            self.pos += c.len_utf8();
        }
        Ok(self.src[begin..self.pos].into())
    }
    fn qname(&mut self) -> ResultXml<String> {
        let name = self.name()?;
        let parts: Vec<_> = name.split(':').collect();
        if parts.len() > 2
            || parts.iter().any(|part| {
                part.is_empty()
                    || !part
                        .chars()
                        .next()
                        .is_some_and(|c| c != ':' && name_start(c))
            })
        {
            return Err(self.error("invalid qualified name"));
        }
        Ok(name)
    }
    fn resolve(
        &self,
        name: &str,
        bindings: &BTreeMap<String, String>,
        attribute: bool,
    ) -> ResultXml<XmlName> {
        let (prefix, local) = split_name(name);
        let uri = if let Some(prefix) = prefix {
            if prefix == "xmlns" {
                return Err(self.error("xmlns cannot prefix an element"));
            }
            Some(
                bindings
                    .get(prefix)
                    .ok_or_else(|| self.error("unbound namespace prefix"))?
                    .clone(),
            )
        } else if attribute {
            None
        } else {
            bindings.get("").filter(|s| !s.is_empty()).cloned()
        };
        Ok(XmlName {
            qualified: name.into(),
            local: local.into(),
            prefix: prefix.map(str::to_owned),
            namespace_uri: uri,
        })
    }
    fn references(&self, raw: &str, attribute: bool) -> ResultXml<String> {
        let mut result = String::new();
        let mut rest = raw;
        while !rest.is_empty() {
            if let Some(tail) = rest.strip_prefix('&') {
                let end = tail
                    .find(';')
                    .ok_or_else(|| self.error("unterminated reference"))?;
                let entity = &tail[..end];
                let c = match entity {
                    "lt" => '<',
                    "gt" => '>',
                    "amp" => '&',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => {
                        let (digits, radix) = if let Some(s) = entity.strip_prefix("#x") {
                            (s, 16)
                        } else if let Some(s) = entity.strip_prefix('#') {
                            (s, 10)
                        } else {
                            if !entity
                                .chars()
                                .next()
                                .is_some_and(|c| c != ':' && name_start(c))
                                || !entity.chars().all(|c| c != ':' && name_char(c))
                            {
                                return Err(self.error("invalid entity reference name"));
                            }
                            return Err(if self.external_subset && !self.standalone {
                                self.special(XmlErrorKind::Unsupported, "entity may require an external DTD; external entities are not loaded")
                            } else {
                                self.error("undeclared entity")
                            });
                        };
                        if digits.is_empty()
                            || !digits.chars().all(|c| {
                                if radix == 16 {
                                    c.is_ascii_hexdigit()
                                } else {
                                    c.is_ascii_digit()
                                }
                            })
                        {
                            return Err(self.error("invalid character reference"));
                        }
                        u32::from_str_radix(digits, radix)
                            .ok()
                            .and_then(char::from_u32)
                            .filter(|c| xml_char(*c))
                            .ok_or_else(|| self.error("forbidden character reference"))?
                    }
                };
                result.push(c);
                rest = &tail[end + 1..];
            } else {
                let c = rest.chars().next().unwrap();
                rest = &rest[c.len_utf8()..];
                result.push(if attribute && xml_space(c) { ' ' } else { c });
            }
        }
        Ok(result)
    }
    fn declaration(&mut self) -> ResultXml<XmlDeclaration> {
        self.expect("<?xml")?;
        self.require_space()?;
        self.expect("version")?;
        self.space();
        self.expect("=")?;
        self.space();
        let version = self.literal()?;
        if version != "1.0" {
            return Err(self.special(XmlErrorKind::Unsupported, "only XML 1.0 is supported"));
        }
        let mut result = XmlDeclaration::default();
        let mut separated = self.space();
        if separated && self.take("encoding") {
            self.space();
            self.expect("=")?;
            self.space();
            let value = self.literal()?;
            if !value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            {
                return Err(self.error("invalid encoding name"));
            }
            result.encoding = Some(value.into());
            separated = self.space();
        }
        if separated && self.take("standalone") {
            self.space();
            self.expect("=")?;
            self.space();
            result.standalone = Some(match self.literal()? {
                "yes" => true,
                "no" => false,
                _ => return Err(self.error("invalid standalone value")),
            });
            self.space();
        }
        self.expect("?>")?;
        Ok(result)
    }
}

// Only declarations on this element are saved, so nesting does not copy the
// entire ancestor environment or amplify large inherited namespace strings.
fn restore_bindings(
    bindings: &mut BTreeMap<String, String>,
    previous: Vec<(String, Option<String>)>,
) {
    for (prefix, value) in previous.into_iter().rev() {
        if let Some(value) = value {
            bindings.insert(prefix, value);
        } else {
            bindings.remove(&prefix);
        }
    }
}

fn split_name(name: &str) -> (Option<&str>, &str) {
    name.split_once(':')
        .map_or((None, name), |(prefix, local)| (Some(prefix), local))
}
fn xml_space(c: char) -> bool {
    matches!(c, '\x20' | '\t' | '\n' | '\r')
}
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}
fn name_start(c: char) -> bool {
    matches!(c, ':' | 'A'..='Z' | '_' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}
fn name_char(c: char) -> bool {
    name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}
fn pubid_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || " \r\n-'()+,./:=?;!*#@$_%".contains(c)
}

#[cfg(test)]
#[path = "xml_tests.rs"]
mod tests;
