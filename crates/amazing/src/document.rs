//! Driver document-mode selection and fatal XML errors.
use std::{fmt, process::ExitCode};
use amazing_dom::{Dom, Namespace, NodeIdx};
use amazing_html::{XmlError, XmlErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentKind {
    Html,
    Xml,
}

pub(crate) struct ParsedDocument {
    pub(crate) dom: Dom,
    pub(crate) kind: DocumentKind,
}

/// Exit codes 2 and 3 are the render harness's document-error protocol.
/// Ordinary rendering/JS/IO failures retain the existing exit code 1.
#[derive(Debug)]
pub(crate) enum PageError {
    Xml(XmlError),
    Unsupported(&'static str),
    Render(String),
}

impl PageError {
    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(match self {
            Self::Xml(error) if error.kind == XmlErrorKind::Unsupported => 3,
            Self::Unsupported(_) => 3,
            Self::Xml(_) => 2,
            Self::Render(_) => 1,
        })
    }
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(error) => error.fmt(f),
            Self::Unsupported(message) => write!(f, "XML Unsupported: {message}"),
            Self::Render(message) => f.write_str(message),
        }
    }
}

impl From<String> for PageError {
    fn from(message: String) -> Self {
        Self::Render(message)
    }
}

pub(crate) fn document_kind(location: &str, content_type: Option<&str>) -> DocumentKind {
    if let Some(content_type) = content_type {
        let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if mime == "application/xml"
            || mime == "text/xml"
            || mime.split_once('/').is_some_and(|(kind, subtype)| {
                !kind.is_empty() && subtype.ends_with("+xml")
            })
        {
            return DocumentKind::Xml;
        }
    }
    // HTTP document mode comes from its MIME type, never a URL suffix.
    if location.starts_with("http://") || location.starts_with("https://") {
        return DocumentKind::Html;
    }
    let path = location.split(['?', '#']).next().unwrap_or(location);
    let filename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if filename.rsplit_once('.').is_some_and(|(_, extension)| {
        ["xml", "xhtml", "xht", "svg"].iter().any(|known| extension.eq_ignore_ascii_case(known))
    }) {
        DocumentKind::Xml
    } else {
        DocumentKind::Html
    }
}

pub(crate) fn parse_document(
    source: &str,
    location: &str,
    content_type: Option<&str>,
) -> Result<ParsedDocument, PageError> {
    let kind = document_kind(location, content_type);
    let dom = match kind {
        DocumentKind::Html => amazing_html::parse(source),
        DocumentKind::Xml => amazing_html::parse_xml(source).map_err(PageError::Xml)?,
    };
    Ok(ParsedDocument { dom, kind })
}

impl ParsedDocument {
    /// XML scripting is not implemented. Refuse script-dependent rendering
    /// instead of awarding an inert reftest pass after silently ignoring it.
    pub(crate) fn require_static_xml(&self) -> Result<(), PageError> {
        if self.kind == DocumentKind::Xml {
            for node in self.dom.tree_order(NodeIdx(0)) {
                if matches!(self.dom.namespace(node), Some(Namespace::Html | Namespace::Svg))
                    && (self.dom.name(node) == Some("script")
                        || self.dom.attrs_of(node).is_some_and(|attrs| {
                            attrs.iter().any(|(name, _)| name.starts_with("on"))
                        }))
                {
                    return Err(PageError::Unsupported(
                        "XML script elements and inline event handlers are not implemented",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_mode_uses_file_extensions_and_recognized_mime_types() {
        for path in ["page.xml", "page.xhtml", "page.xht", "C:\\page.XHTML", "file:///page.svg"] {
            assert_eq!(document_kind(path, None), DocumentKind::Xml);
        }
        for mime in ["application/xml", "text/xml", "APPLICATION/XHTML+XML; charset=utf-8", "image/svg+xml"] {
            assert_eq!(document_kind("http://example.test/no-extension", Some(mime)), DocumentKind::Xml);
        }
        assert_eq!(document_kind("https://example.test/page.xhtml", Some("text/html")), DocumentKind::Html);
        assert_eq!(document_kind("page.html", None), DocumentKind::Html);
    }
}
