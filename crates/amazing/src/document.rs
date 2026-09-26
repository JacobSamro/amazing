//! Driver document-mode selection and fatal XML errors.
use amazing_dom::{Dom, Namespace, NodeIdx};
use amazing_html::{XmlError, XmlErrorKind};
use amazing_net::StreamingResponse;
use std::{fmt, process::ExitCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentKind {
    Html,
    Xml,
}

pub(crate) struct ParsedDocument {
    pub(crate) dom: Dom,
    pub(crate) kind: DocumentKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StreamMetrics {
    pub(crate) bytes: usize,
    pub(crate) chunks: usize,
    /// Largest caller-owned chunk plus the parser's retained byte input.
    pub(crate) peak_driver_bytes: usize,
}

/// Exit code 2 is a document error; 3 is an unsupported render condition.
/// Ordinary rendering/JS/IO failures retain the existing exit code 1.
#[derive(Debug)]
pub(crate) enum PageError {
    Xml(XmlError),
    Unsupported(&'static str),
    ReftestWaitTimeout,
    Render(String),
}

impl PageError {
    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(match self {
            Self::Xml(error) if error.kind == XmlErrorKind::Unsupported => 3,
            Self::Unsupported(_) => 3,
            Self::ReftestWaitTimeout => 3,
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
            Self::ReftestWaitTimeout => write!(f, "Reftest Unsupported: reftest-wait remained after 120 deterministic frame checkpoints"),
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
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime == "application/xml"
            || mime == "text/xml"
            || mime
                .split_once('/')
                .is_some_and(|(kind, subtype)| !kind.is_empty() && subtype.ends_with("+xml"))
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
        ["xml", "xhtml", "xht", "svg"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    }) {
        DocumentKind::Xml
    } else {
        DocumentKind::Html
    }
}

/// The Content-Type media type chooses document mode; only its charset
/// parameter is passed to the bounded HTML byte decoder.
pub(crate) fn transport_label(content_type: Option<&str>) -> Option<&str> {
    content_type?.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|quoted| quoted.strip_suffix('"'))
            .unwrap_or(value);
        (!value.is_empty()).then_some(value)
    })
}

/// HTML-32: decodes (never parses) an HTML byte stream, for a caller that
/// wants to drive `amazing_html::ResumableParser`/`cmd::js::
/// run_interleaved_parse` directly instead of a pre-built `Dom` --
/// `parse_document_bytes`'s own decode-then-`tree::build` split, minus the
/// build. Only meaningful for `DocumentKind::Html`; XML keeps its existing
/// lossy-UTF-8, whole-source `parse_xml` path (`parse_document_bytes`),
/// since interleaved parsing is an HTML-only, script-execution-timing
/// concept with no XML equivalent here.
pub(crate) fn decode_html_bytes(bytes: &[u8], content_type: Option<&str>) -> String {
    amazing_html::encoding::decode(bytes, transport_label(content_type)).text
}

pub(crate) fn parse_document_bytes(
    bytes: &[u8],
    location: &str,
    content_type: Option<&str>,
) -> Result<ParsedDocument, PageError> {
    let kind = document_kind(location, content_type);
    let dom = match kind {
        DocumentKind::Html => amazing_html::parse_bytes(bytes, transport_label(content_type)),
        // XML currently accepts caller-decoded Unicode only. Retain the
        // previous lossy UTF-8 boundary; HTML's four decoders are not XML.
        DocumentKind::Xml => {
            let source = String::from_utf8_lossy(bytes);
            amazing_html::parse_xml(&source).map_err(PageError::Xml)?
        }
    };
    Ok(ParsedDocument { dom, kind })
}

/// Consume each decoded network chunk before requesting another. HTML never
/// collects a response body; XML keeps its existing whole-source parser.
pub(crate) fn parse_streaming_document(
    response: StreamingResponse,
) -> Result<(ParsedDocument, StreamMetrics), PageError> {
    parse_streaming_document_with(response, |_| {})
}

fn parse_streaming_document_with(
    mut response: StreamingResponse,
    mut on_chunk: impl FnMut(StreamMetrics),
) -> Result<(ParsedDocument, StreamMetrics), PageError> {
    let content_type = response.headers.get("content-type");
    let kind = document_kind(&response.url, content_type);
    let transport = if response.url.starts_with("http://") || response.url.starts_with("https://") {
        transport_label(content_type)
    } else {
        None
    };
    let mut metrics = StreamMetrics::default();
    let dom = match kind {
        DocumentKind::Html => {
            let mut parser = amazing_html::IncrementalByteParser::new(transport);
            while let Some(chunk) = response
                .next_chunk()
                .map_err(|error| PageError::Render(error.to_string()))?
            {
                parser.push_chunk(&chunk);
                metrics.bytes += chunk.len();
                metrics.chunks += 1;
                metrics.peak_driver_bytes = metrics
                    .peak_driver_bytes
                    .max(chunk.len() + parser.buffered_input_bytes());
                on_chunk(metrics);
            }
            parser.finish()
        }
        DocumentKind::Xml => {
            let mut body = Vec::new();
            while let Some(chunk) = response
                .next_chunk()
                .map_err(|error| PageError::Render(error.to_string()))?
            {
                body.extend_from_slice(&chunk);
                metrics.bytes += chunk.len();
                metrics.chunks += 1;
                metrics.peak_driver_bytes = metrics.peak_driver_bytes.max(body.len() + chunk.len());
                on_chunk(metrics);
            }
            let source = String::from_utf8_lossy(&body);
            amazing_html::parse_xml(&source).map_err(PageError::Xml)?
        }
    };
    Ok((ParsedDocument { dom, kind }, metrics))
}

impl ParsedDocument {
    /// XML scripting is not implemented. Refuse script-dependent rendering
    /// instead of awarding an inert reftest pass after silently ignoring it.
    pub(crate) fn require_static_xml(&self) -> Result<(), PageError> {
        if self.kind == DocumentKind::Xml {
            for node in self.dom.tree_order(NodeIdx(0)) {
                if matches!(
                    self.dom.namespace(node),
                    Some(Namespace::Html | Namespace::Svg)
                ) && (self.dom.name(node) == Some("script")
                    || self
                        .dom
                        .attrs_of(node)
                        .is_some_and(|attrs| attrs.iter().any(|(name, _)| name.starts_with("on"))))
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    #[test]
    fn xml_mode_uses_file_extensions_and_recognized_mime_types() {
        for path in [
            "page.xml",
            "page.xhtml",
            "page.xht",
            "C:\\page.XHTML",
            "file:///page.svg",
        ] {
            assert_eq!(document_kind(path, None), DocumentKind::Xml);
        }
        for mime in [
            "application/xml",
            "text/xml",
            "APPLICATION/XHTML+XML; charset=utf-8",
            "image/svg+xml",
        ] {
            assert_eq!(
                document_kind("http://example.test/no-extension", Some(mime)),
                DocumentKind::Xml
            );
        }
        assert_eq!(
            document_kind("https://example.test/page.xhtml", Some("text/html")),
            DocumentKind::Html
        );
        assert_eq!(document_kind("page.html", None), DocumentKind::Html);
    }

    #[test]
    fn transport_charset_reads_raw_content_type_parameters() {
        assert_eq!(
            transport_label(Some("text/html; charset=windows-1252")),
            Some("windows-1252")
        );
        assert_eq!(
            transport_label(Some("TEXT/HTML; boundary=x; CHARSET=\"utf-16be\"")),
            Some("utf-16be")
        );
        assert_eq!(transport_label(Some("text/html")), None);
        assert_eq!(transport_label(Some("text/html; charset=")), None);
    }

    #[test]
    fn stream_consumes_before_final_read_and_matches_buffered_decoding() {
        let mut utf8_bom = vec![0xef, 0xbb, 0xbf];
        utf8_bom.extend_from_slice(b"<meta charset=windows-1252><p>");
        utf8_bom.extend_from_slice("é".as_bytes());
        utf8_bom.extend_from_slice(b"</p>");
        let utf8_split = "<p>é雪</p>".as_bytes().to_vec();
        let mut meta = b"<meta charset=windows-1252><p>".to_vec();
        meta.extend_from_slice(&[0x93, b'x', 0x94]);
        meta.extend_from_slice(b"</p>");
        let mut utf16 = vec![0xff, 0xfe];
        for unit in "<p>雪</p>".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let large = format!("<p>{}</p>", "a".repeat(100_000)).into_bytes();
        for (label, mime, body, first_bytes) in [
            ("bom", "text/html; charset=windows-1252", utf8_bom, 1),
            ("meta", "text/html", meta, 10),
            ("utf8", "text/html; charset=utf-8", utf8_split, 4),
            ("utf16", "text/html", utf16, 1),
            ("large", "text/html", large, 1),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let url = format!("http://{address}/{label}.html");
            let expected = parse_document_bytes(&body, &url, Some(mime)).unwrap();
            let expected_dump = expected.dom.dump(NodeIdx(0));
            let expected_kind = expected.kind;
            let length = body.len();
            let (consumed_tx, consumed_rx) = mpsc::channel();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_nodelay(true).unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").unwrap();
                socket.write_all(&body[..first_bytes]).unwrap();
                consumed_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("first part was not consumed before final network write");
                let chunk_bytes = if body.len() > 10_000 { 4096 } else { 7 };
                for chunk in body[first_bytes..].chunks(chunk_bytes) {
                    socket.write_all(chunk).unwrap();
                }
            });
            let response = amazing_net::fetch_stream(
                &url,
                &amazing_net::FetchOptions::default(),
                amazing_net::StreamOptions::default(),
            )
            .unwrap();
            let mut first = true;
            let (actual, metrics) = parse_streaming_document_with(response, |metrics| {
                if first {
                    assert_eq!(
                        metrics.bytes, first_bytes,
                        "{label}: first read should be observable"
                    );
                    consumed_tx.send(()).unwrap();
                    first = false;
                }
            })
            .unwrap();
            server.join().unwrap();
            assert_eq!(actual.kind, expected_kind, "{label}");
            assert_eq!(actual.dom.dump(NodeIdx(0)), expected_dump, "{label}");
            assert_eq!(metrics.bytes, length, "{label}");
            assert!(metrics.chunks >= 2, "{label}: {metrics:?}");
            assert!(
                metrics.peak_driver_bytes <= 8192 + 1024,
                "{label}: {metrics:?}"
            );
            eprintln!(
                "IA-STREAM {label}: bytes={} chunks={} peak_driver_bytes={}",
                metrics.bytes, metrics.chunks, metrics.peak_driver_bytes
            );
        }
    }

    #[test]
    fn streamed_xml_keeps_buffered_mode_dump_and_fatal_errors() {
        for source in ["<Root><Item/></Root>", "<Root><Item></Root>"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let url = format!("http://{address}/document.html");
            let expected = parse_document_bytes(source.as_bytes(), &url, Some("application/xml"));
            let body = source.as_bytes().to_vec();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                socket.write_all(&body).unwrap();
            });
            let response = amazing_net::fetch_stream(
                &url,
                &amazing_net::FetchOptions::default(),
                amazing_net::StreamOptions {
                    chunk_bytes: 3,
                    ..Default::default()
                },
            )
            .unwrap();
            let actual = parse_streaming_document(response);
            server.join().unwrap();
            match (expected, actual) {
                (Ok(expected), Ok((actual, metrics))) => {
                    assert_eq!(actual.kind, DocumentKind::Xml);
                    assert_eq!(actual.kind, expected.kind);
                    assert_eq!(actual.dom.dump(NodeIdx(0)), expected.dom.dump(NodeIdx(0)));
                    assert_eq!(metrics.bytes, source.len());
                    assert!(metrics.chunks > 1);
                }
                (Err(expected), Err(actual)) => {
                    assert_eq!(actual.exit_code(), expected.exit_code());
                    assert_eq!(actual.to_string(), expected.to_string());
                }
                _ => panic!("streamed XML differed from buffered XML"),
            }
        }
    }
}
