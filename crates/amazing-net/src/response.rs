//! The fetched-resource value handed back by [`crate::fetch`].

use crate::headers::Headers;

/// A fully-buffered response: final URL after redirects, status, reason
/// phrase, headers, and body bytes.
///
/// Bodies are buffered whole because every consumer so far (the Phase 1 HTML
/// parser, file dumps) wants a complete byte slice; streaming would complicate
/// the decoder boundary for no current benefit. The [`crate::http`] module
/// caps buffering at 32 MiB as a safety valve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// URL the response ultimately came from (after redirect resolution);
    /// for bare filesystem paths this is the path string as given.
    pub url: String,
    pub status: u16,
    /// Reason phrase from the status line (`"OK"`, `"Not Found"`, …); may be
    /// empty when the server omitted it.
    pub reason: String,
    /// Original wire headers. Content-Length/Content-Encoding describe the
    /// encoded representation, while `body` is already decoded; do not decode
    /// it again or compare its length against the wire Content-Length.
    pub headers: Headers,
    /// Body after transfer and content decoding, bounded by FetchOptions.
    pub body: Vec<u8>,
}

impl Response {
    /// Body decoded as UTF-8, replacing invalid sequences with U+FFFD.
    ///
    /// Returns `Option` for forward compatibility: charset sniffing
    /// (BOM / `<meta charset>`) may later refuse to guess and yield `None`.
    /// In v1 this is always `Some`.
    pub fn text(&self) -> Option<String> {
        Some(String::from_utf8_lossy(&self.body).into_owned())
    }

    /// Media type from `Content-Type`, with any parameters (`; charset=…`)
    /// stripped and surrounding whitespace trimmed. `None` when the server
    /// sent no such header.
    pub fn content_type(&self) -> Option<&str> {
        let raw = self.headers.get("content-type")?;
        let base = raw.split(';').next().unwrap_or(raw);
        Some(base.trim())
    }

    /// True for 2xx statuses.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(body: &[u8], content_type: Option<&str>) -> Response {
        let mut headers = Headers::new();
        if let Some(ct) = content_type {
            headers.insert("Content-Type", ct.to_string());
        }
        Response {
            url: "http://example.com/x".into(),
            status: 200,
            reason: "OK".into(),
            headers,
            body: body.to_vec(),
        }
    }

    #[test]
    fn text_decodes_utf8_and_replaces_invalid_bytes() {
        assert_eq!(resp("héllo".as_bytes(), None).text().as_deref(), Some("héllo"));
        // 0xFF is never valid UTF-8; must become U+FFFD, not panic.
        let bad = resp(&[b'a', 0xFF, b'b'], None);
        assert_eq!(bad.text().as_deref(), Some("a\u{FFFD}b"));
    }

    #[test]
    fn content_type_strips_parameters_and_reports_none_when_absent() {
        assert_eq!(resp(b"", Some("text/html; charset=utf-8")).content_type(), Some("text/html"));
        assert_eq!(resp(b"", Some("  image/png ")).content_type(), Some("image/png"));
        assert_eq!(resp(b"", None).content_type(), None);
    }

    #[test]
    fn is_success_covers_the_2xx_band_only() {
        let mut r = resp(b"", None);
        r.status = 204;
        assert!(r.is_success());
        r.status = 301;
        assert!(!r.is_success());
        r.status = 404;
        assert!(!r.is_success());
    }
}
