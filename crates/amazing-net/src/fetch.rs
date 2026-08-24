//! The [`fetch`] dispatcher: URL-or-path in, fully-buffered [`Response`]
//! out.
//!
//! Routing:
//! - `http://…`      → HTTP/1.1 GET ([`crate::http`]) with redirect following
//! - `https://…`     → HTTP/1.1 through the isolated vetted-rustls adapter
//! - `file://…`      → filesystem read of the decoded URL path
//! - anything else without an `http`/`https`/`file` scheme → treated as a
//!   bare filesystem path, because earlier phases load local pages that way
//!   (a one-letter pseudo-scheme like `C:` counts as a drive path, not a URL)
//!
//! Redirects (RFC 9110 §15.4): 301, 302, 303, 307, 308 are followed up to
//! [`MAX_REDIRECTS`] hops, each `Location` resolved against the *response*
//! URL (so relative locations work through chains). Method semantics are
//! trivially satisfied for v1: we only ever issue GET, which is exactly what
//! 302/303 demand, and what 301/307/308 preserve. Redirect response bodies
//! are discarded. Exceeding the hop cap or a redirect without a resolvable
//! `Location` is an error, never an infinite loop.
//!
//! Deferred: cross-origin cookie/state tracking across redirects, method
//! rewriting rules beyond GET, `Refresh`-style meta redirection.

use std::path::Path;
use std::io::Read;
use std::time::Duration;

use crate::headers::Headers;
use crate::http;
use crate::response::Response;
use crate::url::{self, resolve, Url, UrlError};
use crate::NetError;

/// Maximum number of redirect hops after the initial request.
pub const MAX_REDIRECTS: usize = 5;

/// Knobs for a single fetch. Defaults suit interactive browsing.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Socket read/write timeout applied per connection. Default: 10 s.
    /// Note this bounds each blocking read, not total transfer time.
    pub timeout: Duration,
    /// Additional request headers supplied by the embedding browser session.
    /// Names/values containing HTTP control characters are ignored.
    pub request_headers: Vec<(String, String)>,
    /// Request method. The current browser surface supports GET and POST.
    pub method: String,
    /// Fully-buffered request body, bounded by the embedding caller.
    pub body: Vec<u8>,
    /// Maximum response-body bytes. Defaults to the transport-wide 32 MiB
    /// cap; resource loaders may select a smaller bound before any read.
    pub max_response_bytes: usize,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            request_headers: Vec::new(),
            method: "GET".into(),
            body: Vec::new(),
            max_response_bytes: crate::http::MAX_BODY_BYTES,
        }
    }
}

/// Fetches `input` (an `http`/`https`/`file` URL, or a bare filesystem
/// path) into a buffered [`Response`].
pub fn fetch(input: &str, options: &FetchOptions) -> Result<Response, NetError> {
    let trimmed = input.trim();
    if !looks_like_url(trimmed) {
        return read_file_response(
            Path::new(trimmed),
            trimmed.to_string(),
            options.max_response_bytes,
        );
    }

    // Surface unknown-scheme errors directly (not wrapped in NetError::Url)
    // so callers can distinguish "no such scheme" from malformed URLs.
    let mut current = Url::parse(trimmed).map_err(|e| match e {
        UrlError::UnsupportedScheme(s) => NetError::UnsupportedScheme(s),
        other => NetError::Url(other),
    })?;
    let mut redirects = 0usize;
    let mut request_options = options.clone();
    loop {
        let response = match current.scheme.as_str() {
            "http" | "https" => http::get(&current, &request_options)?,
            "file" => {
                let fs_path = current.to_file_path()?;
                let reported = current.to_string();
                return read_file_response(&fs_path, reported, options.max_response_bytes);
            }
            other => return Err(NetError::UnsupportedScheme(other.to_string())),
        };

        if !is_redirect(response.status) {
            return Ok(response);
        }
        redirects += 1;
        if redirects > MAX_REDIRECTS {
            return Err(NetError::TooManyRedirects);
        }
        let location = response
            .headers
            .get("location")
            .ok_or(NetError::MissingLocation)?
            .to_string();
        if matches!(response.status, 301..=303)
            && request_options.method.eq_ignore_ascii_case("POST")
        {
            request_options.method = "GET".into();
            request_options.body.clear();
            request_options.request_headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("content-type")
                    && !name.eq_ignore_ascii_case("content-length")
            });
        }
        current = resolve(&location, &current)?;
    }
}

/// True when `input` should go down the URL parser instead of the
/// filesystem. A valid scheme of length ≥ 2 (`http:`, `ftp:`, …) marks a
/// URL; a single letter before `:` is a Windows drive path.
fn looks_like_url(input: &str) -> bool {
    match url::split_scheme(input) {
        Some((scheme, _)) => scheme.len() >= 2,
        None => false,
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn read_file_response(
    fs_path: &Path,
    reported_url: String,
    max_response_bytes: usize,
) -> Result<Response, NetError> {
    if std::fs::metadata(fs_path)?.len() > max_response_bytes as u64 {
        return Err(NetError::ResponseTooLarge);
    }
    let mut body = Vec::with_capacity(max_response_bytes.min(8192));
    std::fs::File::open(fs_path)?
        .take(max_response_bytes.saturating_add(1) as u64)
        .read_to_end(&mut body)?;
    if body.len() > max_response_bytes {
        return Err(NetError::ResponseTooLarge);
    }
    let mut headers = Headers::new();
    headers.insert("Content-Type", content_type_for(fs_path).to_string());
    Ok(Response {
        url: reported_url,
        status: 200,
        reason: "OK".into(),
        headers,
        body,
    })
}

/// Minimal extension sniff so later phases can pick a parser for local
/// files without server help. Unknown extensions stay octet-stream.
fn content_type_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::process;

    use crate::http::tests::serve;

    /// Builds a `file:///` URL from a local path on any platform.
    fn file_url_of(path: &Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        if s.starts_with('/') {
            format!("file://{s}")
        } else {
            format!("file:///{s}")
        }
    }

    fn temp_file(label: &str, ext: &str, contents: &[u8]) -> std::path::PathBuf {
        let mut path = env::temp_dir();
        path.push(format!(
            "ferric-net-{label}-{}-{}.{ext}",
            process::id(),
            line!(),
        ));
        fs::write(&path, contents).expect("write temp file");
        path
    }

    #[test]
    fn follows_a_302_then_301_chain_to_the_final_url_and_body() {
        let script = vec![
            b"HTTP/1.1 302 Found\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 301 Moved Permanently\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n"
                .to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\n\r\nfinal!"
                .to_vec(),
        ];
        let (port, rx) = serve(script);
        let resp = fetch(
            &format!("http://127.0.0.1:{port}/start"),
            &FetchOptions::default(),
        )
        .expect("fetch");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text().as_deref(), Some("final!"));
        assert_eq!(resp.content_type(), Some("text/html"));
        assert_eq!(resp.url, format!("http://127.0.0.1:{port}/final"));
        // Three sequential connections (Connection: close), three served
        // request heads seen by the harness.
        for expected_path in ["/start", "/b", "/final"] {
            let req = String::from_utf8(rx.recv_timeout(Duration::from_secs(5)).expect("request"))
                .expect("utf-8");
            assert!(req.starts_with(&format!("GET {expected_path} HTTP/1.1\r\n")));
        }
    }

    #[test]
    fn exceeding_five_redirect_hops_is_an_error() {
        let script: Vec<Vec<u8>> = (0..7)
            .map(|i| {
                format!("HTTP/1.1 302 Found\r\nLocation: /hop-{i}\r\nContent-Length: 0\r\n\r\n")
                    .into_bytes()
            })
            .collect();
        let (port, _rx) = serve(script);
        match fetch(
            &format!("http://127.0.0.1:{port}/start"),
            &FetchOptions::default(),
        ) {
            Err(NetError::TooManyRedirects) => {}
            other => panic!("expected TooManyRedirects, got {other:?}"),
        }
    }

    #[test]
    fn redirect_without_location_header_is_an_error() {
        let script = vec![b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n".to_vec()];
        let (port, _rx) = serve(script);
        match fetch(
            &format!("http://127.0.0.1:{port}/nowhere"),
            &FetchOptions::default(),
        ) {
            Err(NetError::MissingLocation) => {}
            other => panic!("expected MissingLocation, got {other:?}"),
        }
    }

    #[test]
    fn five_redirects_are_still_followed() {
        // Exactly 5 hops then success must NOT trip the cap.
        let mut script: Vec<Vec<u8>> = (0..5)
            .map(|i| {
                format!("HTTP/1.1 302 Found\r\nLocation: /h{i}\r\nContent-Length: 0\r\n\r\n")
                    .into_bytes()
            })
            .collect();
        script.push(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec());
        let (port, _rx) = serve(script);
        let resp = fetch(
            &format!("http://127.0.0.1:{port}/start"),
            &FetchOptions::default(),
        )
        .expect("five hops must succeed");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.url, format!("http://127.0.0.1:{port}/h4"));
    }

    #[test]
    fn https_urls_use_the_tls_default_port() {
        let url = Url::parse("https://example.com/x?q#f").expect("https parses");
        assert_eq!(url.port, 443);
    }

    #[test]
    fn unknown_network_schemes_are_rejected_not_mistaken_for_paths() {
        match fetch("ftp://example.com/file", &FetchOptions::default()) {
            Err(NetError::UnsupportedScheme(s)) => assert_eq!(s, "ftp"),
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn file_url_roundtrip_through_temp_dir() {
        let path = temp_file("fileurl", "html", b"<p>hi</p>");
        let resp = fetch(&file_url_of(&path), &FetchOptions::default()).expect("fetch");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.reason, "OK");
        assert_eq!(resp.text().as_deref(), Some("<p>hi</p>"));
        assert_eq!(resp.content_type(), Some("text/html"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn bare_filesystem_path_is_read_as_a_local_file() {
        let path = temp_file("bare", "txt", b"plain");
        let reported = path.to_string_lossy().into_owned();
        let resp = fetch(&reported, &FetchOptions::default()).expect("fetch");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text().as_deref(), Some("plain"));
        assert_eq!(resp.url, reported);
        assert_eq!(resp.content_type(), Some("text/plain"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn caller_response_limit_is_enforced_before_a_local_file_read() {
        let path = temp_file("bounded", "woff2", &[7; 65]);
        let options = FetchOptions {
            max_response_bytes: 64,
            ..FetchOptions::default()
        };

        assert!(matches!(
            fetch(path.to_str().unwrap(), &options),
            Err(NetError::ResponseTooLarge)
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_bare_path_surfaces_io_error_not_url_error() {
        let mut path = env::temp_dir();
        path.push(format!("ferric-net-missing-{}", process::id()));
        match fetch(
            path.to_str().expect("temp path utf-8"),
            &FetchOptions::default(),
        ) {
            Err(NetError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    #[test]
    fn windows_drive_paths_are_dispatched_to_the_filesystem() {
        if !cfg!(windows) {
            return; // meaningful only on win-001; harmless elsewhere
        }
        match fetch(
            r"C:\ferric-net-definitely-missing.txt",
            &FetchOptions::default(),
        ) {
            Err(NetError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io error (not UnsupportedScheme), got {other:?}"),
        }
    }

    #[test]
    fn empty_input_fails_without_panicking() {
        assert!(fetch("", &FetchOptions::default()).is_err());
        assert!(fetch("   ", &FetchOptions::default()).is_err());
    }
}
