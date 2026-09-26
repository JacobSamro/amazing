//! A minimal HTTP/1.1 request reader and response writer.
//!
//! The CDP endpoint needs exactly three things from HTTP: read a request line
//! and headers, answer `/json/version` and `/json/list` with JSON, and perform
//! the RFC 6455 upgrade. This is not a general server: no keep-alive pipelining,
//! no chunked bodies, no routing beyond the paths [`crate::server`] matches.
//!
//! Headers are read one byte at a time so that no byte after the terminating
//! blank line is consumed; the WebSocket layer then owns the raw stream.

use std::io::{self, Read, Write};

/// Default cap on the request line plus headers.
pub const DEFAULT_MAX_HEADERS: usize = 16 * 1024;
/// Maximum number of header fields accepted.
pub const MAX_HEADER_COUNT: usize = 128;

/// A parsed HTTP request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub target: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
}

impl HttpRequest {
    /// Returns the first value of `name`, compared case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Whether any value of `name` contains `token` as a comma-separated item.
    pub fn header_has_token(&self, name: &str, token: &str) -> bool {
        self.headers
            .iter()
            .filter(|(field, _)| field.eq_ignore_ascii_case(name))
            .any(|(_, value)| {
                value
                    .split(',')
                    .any(|item| item.trim_matches([' ', '\t']).eq_ignore_ascii_case(token))
            })
    }

    /// The path portion of the request target, without a query string.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }

    /// The query string, without the leading `?`, if present.
    pub fn query(&self) -> Option<&str> {
        self.target.split_once('?').map(|(_, query)| query)
    }
}

/// Why an HTTP request could not be read.
#[derive(Debug)]
pub enum HttpError {
    Io(io::Error),
    Protocol(&'static str),
    Limit,
}

impl From<io::Error> for HttpError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "http i/o error: {e}"),
            Self::Protocol(m) => write!(f, "http protocol error: {m}"),
            Self::Limit => f.write_str("http request headers exceeded the limit"),
        }
    }
}

impl std::error::Error for HttpError {}

/// Reads one request head. `Ok(None)` means the peer closed cleanly before
/// sending anything.
pub fn read_request<R: Read>(
    reader: &mut R,
    max_headers: usize,
) -> Result<Option<HttpRequest>, HttpError> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= max_headers {
            return Err(HttpError::Limit);
        }
        match reader.read(&mut byte) {
            Ok(0) => {
                if bytes.is_empty() {
                    return Ok(None);
                }
                return Err(HttpError::Protocol("truncated request head"));
            }
            Ok(_) => bytes.push(byte[0]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(HttpError::Io(e)),
        }
    }
    parse_head(&bytes).map(Some)
}

/// Parses a fully-buffered request head (used by `read_request` and tests).
pub fn parse_head(bytes: &[u8]) -> Result<HttpRequest, HttpError> {
    let text = std::str::from_utf8(bytes).map_err(|_| HttpError::Protocol("non-UTF-8 head"))?;
    let text = text
        .strip_suffix("\r\n\r\n")
        .ok_or(HttpError::Protocol("missing head terminator"))?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(HttpError::Protocol("empty request"))?;
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if parts.next().is_some() || !is_token(method) {
        return Err(HttpError::Protocol("malformed request line"));
    }
    if target.is_empty()
        || !target.starts_with('/')
        || !target.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return Err(HttpError::Protocol("invalid request target"));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(HttpError::Protocol("unsupported HTTP version"));
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            return Err(HttpError::Protocol("stray blank line"));
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(HttpError::Protocol("obsolete line folding"));
        }
        let (name, raw) = line
            .split_once(':')
            .ok_or(HttpError::Protocol("header without colon"))?;
        if !is_token(name) {
            return Err(HttpError::Protocol("invalid header name"));
        }
        if !raw
            .bytes()
            .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
        {
            return Err(HttpError::Protocol("invalid header value"));
        }
        if headers.len() >= MAX_HEADER_COUNT {
            return Err(HttpError::Limit);
        }
        headers.push((name.to_string(), raw.trim_matches([' ', '\t']).to_string()));
    }
    let request = HttpRequest {
        method: method.to_string(),
        target: target.to_string(),
        version: version.to_string(),
        headers,
    };
    // The upgrade and discovery paths carry no body; refuse one rather than
    // silently treat it as WebSocket bytes.
    if request
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > 0)
        || request.header("transfer-encoding").is_some()
    {
        return Err(HttpError::Protocol("request body not supported"));
    }
    Ok(request)
}

/// An HTTP response with a reason phrase and headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// A `200 OK` with a JSON body and `Cache-Control: no-cache`.
    pub fn json(body: String) -> Self {
        Self {
            status: 200,
            reason: "OK",
            headers: vec![
                ("Content-Type".into(), "application/json; charset=UTF-8".into()),
                ("Cache-Control".into(), "no-cache".into()),
            ],
            body: body.into_bytes(),
        }
    }

    /// A bodyless response such as `404 Not Found` or `405 Method Not Allowed`.
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// A `text/plain` response, used for HTTP-level errors.
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            headers: vec![("Content-Type".into(), "text/plain; charset=UTF-8".into())],
            body: body.into().into_bytes(),
        }
    }
}

/// Writes a response with `Content-Length` and a closing connection.
pub fn write_response<W: Write>(writer: &mut W, response: &HttpResponse) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, response.reason);
    for (name, value) in &response.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    head.push_str("Connection: close\r\n\r\n");
    writer.write_all(head.as_bytes())?;
    writer.write_all(&response.body)?;
    writer.flush()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        101 => "Switching Protocols",
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        426 => "Upgrade Required",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "Unknown",
    }
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_get_request() {
        let raw = b"GET /json/list?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n";
        let request = parse_head(raw).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path(), "/json/list");
        assert_eq!(request.query(), Some("x=1"));
        assert_eq!(request.header("host"), Some("127.0.0.1"));
        assert!(request.header_has_token("connection", "upgrade"));
        assert!(!request.header_has_token("connection", "close"));
    }

    #[test]
    fn reads_a_request_and_stops_at_the_blank_line() {
        let raw = b"GET / HTTP/1.1\r\nA: b\r\n\r\nTRAILING";
        let mut cursor = io::Cursor::new(raw.to_vec());
        let request = read_request(&mut cursor, DEFAULT_MAX_HEADERS).unwrap().unwrap();
        assert_eq!(request.target, "/");
        let rest = &raw[cursor.position() as usize..];
        assert_eq!(rest, b"TRAILING");
    }

    #[test]
    fn clean_eof_is_none() {
        let mut cursor = io::Cursor::new(Vec::new());
        assert!(read_request(&mut cursor, DEFAULT_MAX_HEADERS).unwrap().is_none());
    }

    #[test]
    fn rejects_malformed_requests() {
        for bad in [
            &b"GET / HTTP/1.1\r\nbad header\r\n\r\n"[..],
            &b"GET / HTTP/2.0\r\n\r\n"[..],
            &b"G@T / HTTP/1.1\r\n\r\n"[..],
            &b"GET relative HTTP/1.1\r\n\r\n"[..],
            &b"GET / HTTP/1.1\r\nA: b\r\n A: c\r\n\r\n"[..],
            &b"GET / HTTP/1.1\r\nContent-Length: 5\r\n\r\n"[..],
        ] {
            assert!(parse_head(bad).is_err(), "expected rejection: {bad:?}");
        }
    }

    #[test]
    fn enforces_header_limit() {
        let mut raw = b"GET / HTTP/1.1\r\n".to_vec();
        raw.extend_from_slice(&vec![b'x'; 64]);
        raw.extend_from_slice(b"\r\n\r\n");
        let mut cursor = io::Cursor::new(raw);
        assert!(matches!(
            read_request(&mut cursor, 32),
            Err(HttpError::Limit)
        ));
    }

    #[test]
    fn writes_a_response() {
        let mut out = Vec::new();
        write_response(&mut out, &HttpResponse::json("{}".into())).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
        assert!(text.contains("Content-Length: 2\r\n"));
    }
}
