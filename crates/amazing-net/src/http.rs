//! HTTP/1.1 GET over plain `std::net::TcpStream`.
//!
//! One connection per request (`Connection: close`), fully-buffered
//! response. Request line + four fixed headers; response parsing covers the
//! status line, the header block, and all three body framings.
//!
//! Implemented:
//! - `GET` with `Host`, `User-Agent: Ferric/0.1`, `Accept: */*`,
//!   `Connection: close`
//! - status-line validation (`HTTP/1.x NNN [reason]`), case-insensitive
//!   header names via [`crate::headers::Headers`]
//! - body framing priority: `Transfer-Encoding: chunked` (chunk extensions
//!   and trailer section consumed, contents ignored) > `Content-Length`
//!   (both short and overlong bodies reject) > read-to-EOF
//! - read/write timeouts from [`crate::FetchOptions::timeout`]
//! - 32 MiB buffering cap so a hostile peer cannot exhaust memory
//!
//! Deferred: keep-alive/pipelining, other methods, `Expect: 100-continue`,
//! obs-fold header lines, bare-LF line endings in
//! the framing layer (header *values* tolerate them), HTTP/3.

use std::io::{Read, Write};
use std::net::TcpStream;

use crate::fetch::FetchOptions;
use crate::headers::Headers;
use crate::response::Response;
use crate::url::ParsedUrl;
use crate::NetError;

/// Upper bound on buffered response bytes, enforced during both raw reads
/// and dechunking.
pub(crate) const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

const USER_AGENT: &str = "Ferric/0.1";

/// Performs one GET against an HTTP or HTTPS URL. HTTPS crosses the isolated
/// vetted-rustls adapter; response parsing and safety limits remain here.
pub(crate) fn get(url: &ParsedUrl, options: &FetchOptions) -> Result<Response, NetError> {
    let body_limit = options.max_response_bytes.min(MAX_BODY_BYTES);
    let wire_limit = body_limit.saturating_add(MAX_HEADER_BYTES);
    let request = request_bytes(url, options)?;
    // ToSocketAddrs for (&str, u16) parses IP literals itself, but bracketed
    // IPv6 ("[::1]") does not parse — hand it the bare address instead.
    let host = url.hostname();
    let connect_host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url
        .effective_port()
        .expect("validated HTTP(S) URL has a port");
    let (raw, require_explicit_body_length) = match url.scheme() {
        "http" => {
            let mut stream = TcpStream::connect((connect_host, port))?;
            stream.set_read_timeout(Some(options.timeout))?;
            stream.set_write_timeout(Some(options.timeout))?;
            stream.write_all(&request)?;
            stream.flush()?;
            (read_to_eof(&stream, wire_limit)?, false)
        }
        "https" => {
            let stream = amazing_net_tls::connect(connect_host, port, options.timeout)
                .map_err(|e| NetError::Tls(e.to_string()))?;
            return tls_response(stream, url, options, &request);
        }
        other => return Err(NetError::UnsupportedScheme(other.to_string())),
    };
    response_from_raw_with_limit(url, &raw, require_explicit_body_length, body_limit)
}

/// Shared request serialization for buffered and pull-style HTTP/1.1 paths.
pub(crate) fn request_bytes(url: &ParsedUrl, options: &FetchOptions) -> Result<Vec<u8>, NetError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(NetError::UnsupportedScheme(url.scheme().into()));
    }
    let method = options.method.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST") {
        return Err(NetError::UnsupportedMethod(method));
    }
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\n",
        method,
        url.http_request_target()?,
        url.host_and_port(),
    );
    if !options
        .request_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        request.push_str(&format!("User-Agent: {USER_AGENT}\r\n"));
    }
    if !options
        .request_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
    {
        request.push_str("Accept: */*\r\n");
    }
    request.push_str("Accept-Encoding: gzip, deflate, br\r\nConnection: close\r\n");
    for (name, value) in &options.request_headers {
        if !name.eq_ignore_ascii_case("accept-encoding")
            && !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            && !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
        {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
    }
    if !options.body.is_empty()
        && !options
            .request_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        request.push_str(&format!("Content-Length: {}\r\n", options.body.len()));
    }
    request.push_str("\r\n");
    let mut request = request.into_bytes();
    request.extend_from_slice(&options.body);
    Ok(request)
}

/// Shared post-handshake dispatch; loopback TLS proof uses this exact path.
pub(crate) fn tls_response(
    stream: amazing_net_tls::TlsStream,
    url: &ParsedUrl,
    options: &FetchOptions,
    request: &[u8],
) -> Result<Response, NetError> {
    if stream.protocol() == amazing_net_tls::Protocol::H2 {
        return crate::http2::fetch(stream, url, options);
    }
    let body_limit = options.max_response_bytes.min(MAX_BODY_BYTES);
    let response = stream
        .exchange_http1(request, body_limit.saturating_add(MAX_HEADER_BYTES))
        .map_err(|error| match error {
            amazing_net_tls::TlsError::ResponseTooLarge => NetError::ResponseTooLarge,
            other => NetError::Tls(other.to_string()),
        })?;
    response_from_raw_with_limit(
        url,
        &response.bytes,
        !response.peer_closed_cleanly,
        body_limit,
    )
}

#[cfg(test)]
fn response_from_raw(
    url: &ParsedUrl,
    raw: &[u8],
    require_explicit_body_length: bool,
) -> Result<Response, NetError> {
    response_from_raw_with_limit(url, raw, require_explicit_body_length, MAX_BODY_BYTES)
}

fn response_from_raw_with_limit(
    url: &ParsedUrl,
    raw: &[u8],
    require_explicit_body_length: bool,
    body_limit: usize,
) -> Result<Response, NetError> {
    let head_end = find_header_end(raw)
        .ok_or_else(|| NetError::MalformedResponse("no \\r\\n\\r\\n header terminator".into()))?;
    if head_end > MAX_HEADER_BYTES {
        return Err(NetError::ResponseTooLarge);
    }
    let (status, reason, headers) = parse_head(&raw[..head_end])?;
    // Metadata on bodyless responses describes a representation, not bytes to
    // decompress. This GET/POST transport does not implement protocol upgrades.
    if (100..200).contains(&status) {
        return Err(NetError::MalformedResponse(
            "informational responses unsupported".into(),
        ));
    }
    let body = if matches!(status, 204 | 304) {
        if raw.len() != head_end + 4 {
            return Err(NetError::MalformedResponse(
                "body on bodyless response".into(),
            ));
        }
        Vec::new()
    } else {
        decode_body(
            &raw[head_end + 4..],
            &headers,
            require_explicit_body_length,
            body_limit,
        )?
    };

    Ok(Response {
        url: url.to_string(),
        status,
        reason,
        headers,
        body,
    })
}

/// Reads until the peer closes (we always send `Connection: close`), with
/// the size cap applied incrementally. A read-timeout fires here as a plain
/// I/O error — partial data is discarded rather than served truncated.
fn read_to_eof(mut stream: &TcpStream, limit: usize) -> Result<Vec<u8>, NetError> {
    let mut buf = [0u8; 8192];
    let mut raw: Vec<u8> = Vec::with_capacity(8 * 1024);
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(raw);
        }
        if raw.len().saturating_add(n) > limit {
            return Err(NetError::ResponseTooLarge);
        }
        raw.extend_from_slice(&buf[..n]);
    }
}

/// Byte offset of the `\r\n\r\n` separator ending the head.
fn find_header_end(raw: &[u8]) -> Option<usize> {
    if raw.len() < 4 {
        return None;
    }
    raw.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parses the status line and header block (the bytes before `\r\n\r\n`).
/// Header lines may use bare `\n`; their trailing `\r` is trimmed.
pub(crate) fn parse_head(head: &[u8]) -> Result<(u16, String, Headers), NetError> {
    let head_str = std::str::from_utf8(head)
        .map_err(|_| NetError::MalformedResponse("non-UTF-8 header block".into()))?;
    let mut lines = head_str.split('\n');
    let status_line = lines.next().unwrap_or("").trim_end_matches('\r');
    let (status, reason) = parse_status_line(status_line)?;

    let mut headers = Headers::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (name, value) = match line.split_once(':') {
            Some((n, v)) => (n, v),
            None => {
                return Err(NetError::MalformedResponse(format!(
                    "header line without ':': {line:?}"
                )))
            }
        };
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(NetError::MalformedResponse(format!(
                "invalid header name: {name:?}"
            )));
        }
        headers.insert(
            name,
            value.trim_matches(|c| c == ' ' || c == '\t').to_string(),
        );
    }
    Ok((status, reason, headers))
}

/// Accepts exactly `HTTP/1.<digit> <3 digits>[ reason]`.
fn parse_status_line(line: &str) -> Result<(u16, String), NetError> {
    let malformed = || NetError::MalformedResponse(format!("bad status line: {line:?}"));
    let mut parts = line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let minor_ok = version.len() == 8 && version.as_bytes()[7].is_ascii_digit();
    if !version.starts_with("HTTP/1.") || !minor_ok {
        return Err(malformed());
    }
    let code = parts.next().unwrap_or("");
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    let status: u16 = code.parse().map_err(|_| malformed())?;
    let reason = parts.next().unwrap_or("").trim().to_string();
    Ok((status, reason))
}

/// Header order is coding order; decoding runs in reverse. Bound layering as
/// well as bytes so a header cannot request unbounded repeated work.
pub(crate) fn codings(headers: &Headers, name: &str) -> Result<Vec<String>, NetError> {
    let mut result = Vec::new();
    for field in headers.get_all(name) {
        for item in field.split(',') {
            let item = item.trim().to_ascii_lowercase();
            if !matches!(
                item.as_str(),
                "gzip" | "x-gzip" | "deflate" | "br" | "identity" | "chunked"
            ) {
                return Err(NetError::MalformedResponse(format!(
                    "unsupported {name}: {item}"
                )));
            }
            if result.len() == 8 {
                return Err(NetError::MalformedResponse("too many coding layers".into()));
            }
            result.push(item);
        }
    }
    Ok(result)
}

pub(crate) fn content_length(headers: &Headers) -> Result<Option<usize>, NetError> {
    let mut expected = None;
    for field in headers.get_all("content-length") {
        for value in field.split(',') {
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(NetError::MalformedResponse("invalid Content-Length".into()));
            }
            let length = value
                .parse::<usize>()
                .map_err(|_| NetError::MalformedResponse("Content-Length overflow".into()))?;
            if expected.is_some_and(|old| old != length) {
                return Err(NetError::MalformedResponse(
                    "conflicting Content-Length".into(),
                ));
            }
            expected = Some(length);
        }
    }
    Ok(expected)
}

/// Validate wire framing before applying transfer codings, then content
/// codings. Content-Length counts encoded wire bytes, never decoded bytes.
pub(crate) fn decode_body(
    body: &[u8],
    headers: &Headers,
    require_explicit_length: bool,
    limit: usize,
) -> Result<Vec<u8>, NetError> {
    let mut transfer = codings(headers, "transfer-encoding")?;
    let content = codings(headers, "content-encoding")?;
    if content.iter().any(|c| c == "chunked") || transfer.iter().any(|c| c == "identity") {
        return Err(NetError::MalformedResponse("coding in wrong header".into()));
    }
    if !transfer.is_empty() && headers.contains("content-length") {
        return Err(NetError::MalformedResponse(
            "Transfer-Encoding with Content-Length".into(),
        ));
    }
    let chunked = transfer.last().is_some_and(|c| c == "chunked");
    if chunked {
        transfer.pop();
    }
    if transfer.iter().any(|c| c == "chunked") {
        return Err(NetError::MalformedResponse(
            "chunked must occur once, last".into(),
        ));
    }
    if transfer.len() + content.len() > 8 {
        return Err(NetError::MalformedResponse("too many coding layers".into()));
    }
    // Encoded bytes have their own transport-wide cap. A tiny decoded budget
    // must still permit gzip headers/trailers that exceed the decoded size.
    let compressed = transfer.iter().chain(&content).any(|c| c != "identity");
    let framed_limit = if compressed { MAX_BODY_BYTES } else { limit };
    let mut decoded = if chunked {
        dechunk_with_limit(body, framed_limit)?.0
    } else {
        if body.len() > framed_limit {
            return Err(NetError::ResponseTooLarge);
        }
        if let Some(length) = content_length(headers)? {
            if length > framed_limit {
                return Err(NetError::ResponseTooLarge);
            }
            if body.len() < length {
                return Err(NetError::IncompleteBody);
            }
            if body.len() > length {
                return Err(NetError::MalformedResponse(
                    "body exceeds Content-Length".into(),
                ));
            }
        } else if require_explicit_length {
            return Err(NetError::IncompleteBody);
        }
        body.to_vec()
    };
    for coding in transfer.iter().rev().chain(content.iter().rev()) {
        if coding != "identity" {
            decoded = crate::coding::decode(&decoded, coding, limit)?;
        }
    }
    if decoded.len() > limit {
        return Err(NetError::ResponseTooLarge);
    }
    Ok(decoded)
}

/// Decoded chunks plus whatever trailer section followed the last-chunk.
///
/// Grammar enforced: `size [;ext] CRLF data CRLF` repeated, then
/// `0 CRLF trailer-lines…`. Chunk extensions are skipped; trailer field
/// contents are ignored (connection closes anyway, so there is nothing to
/// keep alive about them). Bare-LF size lines are rejected deliberately —
/// real-world servers that dechunk correctly always send CRLF.
#[cfg(test)]
fn dechunk(input: &[u8]) -> Result<(Vec<u8>, &[u8]), NetError> {
    dechunk_with_limit(input, MAX_BODY_BYTES)
}

fn dechunk_with_limit(input: &[u8], limit: usize) -> Result<(Vec<u8>, &[u8]), NetError> {
    let malformed = |d: &str| NetError::MalformedResponse(format!("chunked: {d}"));
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let line_end =
            find_crlf(input, pos).ok_or_else(|| malformed("unterminated chunk-size line"))?;
        let size_field = std::str::from_utf8(&input[pos..line_end])
            .map_err(|_| malformed("non-UTF-8 chunk-size line"))?;
        // Everything after ';' is a chunk extension; ignore it.
        let size_digits = size_field.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_digits, 16)
            .map_err(|_| malformed(&format!("invalid chunk size {size_digits:?}")))?;
        pos = line_end + 2;
        if size == 0 {
            break;
        }
        if out.len().saturating_add(size) > limit {
            return Err(NetError::ResponseTooLarge);
        }
        let data_end = pos.saturating_add(size);
        let data = input
            .get(pos..data_end)
            .ok_or_else(|| malformed("truncated chunk data"))?;
        out.extend_from_slice(data);
        pos = data_end;
        if input.get(pos..pos.saturating_add(2)) != Some(&b"\r\n"[..]) {
            return Err(malformed("missing CRLF after chunk data"));
        }
        pos += 2;
    }
    // Require the complete trailer terminator even for unclean TLS EOF, and
    // reject extra bytes (this transport never pipelines another response).
    let trailer_start = pos;
    loop {
        let end = find_crlf(input, pos).ok_or_else(|| malformed("unterminated trailer section"))?;
        if end == pos {
            if end + 2 != input.len() {
                return Err(malformed("bytes after chunked message"));
            }
            return Ok((out, &input[trailer_start..]));
        }
        if !input[pos..end].contains(&b':') {
            return Err(malformed("invalid trailer field"));
        }
        pos = end + 2;
    }
}

fn find_crlf(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    /// Binds an ephemeral loopback listener and spawns a thread that serves
    /// one canned response per incoming connection (draining each request
    /// first so the client's write can never block). Every received request
    /// byte-block is forwarded on the returned receiver. Port 0 only, so
    /// tests never clash; no traffic leaves loopback.
    pub(crate) fn serve(script: Vec<Vec<u8>>) -> (u16, Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let port = listener.local_addr().expect("local_addr").port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (i, incoming) in listener.incoming().enumerate() {
                let Ok(mut stream) = incoming else { continue };
                if let Some(resp) = script.get(i) {
                    let mut buf = [0u8; 4096];
                    let mut seen: Vec<u8> = Vec::new();
                    loop {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                seen.extend_from_slice(&buf[..n]);
                                if seen.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = tx.send(seen);
                    if stream.write_all(resp).and_then(|_| stream.flush()).is_err() {
                        break;
                    }
                }
                // Dropping `stream` closes the socket → client sees EOF.
            }
        });
        (port, rx)
    }

    fn fetch_loopback(script: Vec<Vec<u8>>, path: &str) -> Result<Response, NetError> {
        let (port, _rx) = serve(script);
        let url = format!("http://127.0.0.1:{port}{path}");
        crate::fetch::fetch(&url, &FetchOptions::default())
    }

    #[test]
    fn fetches_content_length_body_and_sends_required_headers() {
        let script = vec![
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello"
                .to_vec(),
        ];
        let (port, rx) = serve(script);
        let resp = crate::fetch::fetch(
            &format!("http://127.0.0.1:{port}/a"),
            &FetchOptions::default(),
        )
        .expect("fetch");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.reason, "OK");
        assert_eq!(resp.text().as_deref(), Some("hello"));
        assert_eq!(resp.content_type(), Some("text/plain"));
        assert_eq!(resp.url, format!("http://127.0.0.1:{port}/a"));

        let request = String::from_utf8(rx.recv_timeout(Duration::from_secs(5)).expect("request"))
            .expect("utf-8 request");
        assert!(request.starts_with("GET /a HTTP/1.1\r\n"));
        assert!(request.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
        assert!(request.contains("User-Agent: Ferric/0.1\r\n"));
        assert!(request.contains("Accept: */*\r\n"));
        assert!(request.contains("Connection: close\r\n"));
    }

    #[test]
    fn embedding_headers_are_sent_and_control_injection_is_ignored() {
        let (port, requests) = serve(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        ]);
        let options = FetchOptions {
            timeout: Duration::from_secs(5),
            request_headers: vec![
                ("Cookie".into(), "sid=abc".into()),
                ("Bad\r\nInjected".into(), "yes".into()),
                ("X-Test".into(), "bad\nvalue".into()),
            ],
            method: "GET".into(),
            body: Vec::new(),
            ..FetchOptions::default()
        };
        crate::fetch::fetch(&format!("http://127.0.0.1:{port}/headers"), &options).unwrap();
        let request =
            String::from_utf8(requests.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert!(request.contains("Cookie: sid=abc\r\n"));
        assert!(!request.contains("Injected"));
        assert!(!request.contains("bad\nvalue"));
    }

    #[test]
    fn post_request_sends_content_type_length_and_body() {
        let (port, requests) = serve(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec(),
        ]);
        let options = FetchOptions {
            timeout: Duration::from_secs(5),
            request_headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            method: "POST".into(),
            body: b"q=rust+browser".to_vec(),
            ..FetchOptions::default()
        };
        let response =
            crate::fetch::fetch(&format!("http://127.0.0.1:{port}/submit"), &options).unwrap();
        assert_eq!(response.text().as_deref(), Some("ok"));
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(request.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(request.contains("Content-Length: 14\r\n"));
        assert!(request.ends_with("\r\n\r\nq=rust+browser"));
    }

    #[test]
    fn header_lookup_is_case_insensitive_across_the_wire() {
        let resp = fetch_loopback(
            vec![
                b"HTTP/1.1 204 No Content\r\nX-WeIrD-NaMe: v\r\nContent-Length: 0\r\n\r\n".to_vec(),
            ],
            "/",
        )
        .expect("fetch");
        assert_eq!(resp.status, 204);
        assert_eq!(resp.headers.get("x-weird-name"), Some("v"));
        assert_eq!(resp.body.len(), 0);
        assert_eq!(resp.content_type(), None);
    }

    #[test]
    fn repeated_set_cookie_headers_survive_parsing() {
        let resp = fetch_loopback(
            vec![b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Length: 0\r\n\r\n"
                .to_vec()],
            "/",
        )
        .expect("fetch");
        assert_eq!(resp.headers.get_all("set-cookie"), vec!["a=1", "b=2"]);
    }

    #[test]
    fn reassembles_chunked_body_with_extensions_and_trailer() {
        // Three chunks (one carrying a chunk extension), then the 0-chunk
        // with a trailer field before the final blank line.
        let raw = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Transfer-Encoding: chunked\r\n",
            "\r\n",
            "4\r\nWiki\r\n",
            "5\r\npedia\r\n",
            "E;ext=1\r\n in\r\n\r\nchunks.\r\n",
            "0\r\n",
            "X-Trailer: ignored\r\n",
            "\r\n",
        );
        let resp = fetch_loopback(vec![raw.as_bytes().to_vec()], "/").expect("fetch");
        assert_eq!(resp.text().as_deref(), Some("Wikipedia in\r\n\r\nchunks."));
        assert!(!resp.headers.contains("content-length"));
    }

    #[test]
    fn content_length_rejects_overlong_stream_data() {
        // NET-4 intentional change: reject a size mismatch rather than silently
        // truncating the extra byte as the old transport did.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nabZ";
        assert!(matches!(
            fetch_loopback(vec![raw.to_vec()], "/"),
            Err(NetError::MalformedResponse(_))
        ));
    }

    #[test]
    fn short_content_length_body_is_an_error_not_a_truncation() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc";
        match fetch_loopback(vec![raw.to_vec()], "/") {
            Err(NetError::IncompleteBody) => {}
            other => panic!("expected IncompleteBody, got {other:?}"),
        }
    }

    #[test]
    fn eof_delimited_body_is_accepted_without_cl_or_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<body>to eof";
        let resp = fetch_loopback(vec![raw.to_vec()], "/").expect("fetch");
        assert_eq!(resp.text().as_deref(), Some("<body>to eof"));
    }

    #[test]
    fn unclean_tls_eof_requires_http_body_framing() {
        let url = ParsedUrl::parse("https://example.test/").unwrap();
        let unframed = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\npossibly truncated";
        assert!(matches!(
            response_from_raw(&url, unframed, true),
            Err(NetError::IncompleteBody)
        ));

        let framed =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 8\r\n\r\ncomplete";
        assert_eq!(
            response_from_raw(&url, framed, true)
                .unwrap()
                .text()
                .as_deref(),
            Some("complete")
        );
    }

    #[test]
    fn malformed_status_line_is_rejected() {
        let raw = b"NOT-HTTP AT ALL\r\n\r\n";
        match fetch_loopback(vec![raw.to_vec()], "/") {
            Err(NetError::MalformedResponse(_)) => {}
            other => panic!("expected MalformedResponse, got {other:?}"),
        }
    }

    #[test]
    fn missing_header_terminator_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nnever-terminated";
        match fetch_loopback(vec![raw.to_vec()], "/") {
            Err(NetError::MalformedResponse(_)) => {}
            other => panic!("expected MalformedResponse, got {other:?}"),
        }
    }

    #[test]
    fn header_line_without_colon_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nBadHeaderLine\r\nContent-Length: 0\r\n\r\n";
        match fetch_loopback(vec![raw.to_vec()], "/") {
            Err(NetError::MalformedResponse(_)) => {}
            other => panic!("expected MalformedResponse, got {other:?}"),
        }
    }

    #[test]
    fn status_line_without_reason_phrase_is_accepted() {
        let resp = fetch_loopback(
            vec![b"HTTP/1.1 404\r\nContent-Length: 0\r\n\r\n".to_vec()],
            "/",
        )
        .expect("fetch");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.reason, "");
    }

    // ---- direct dechunk unit tests (no sockets involved) ----

    #[test]
    fn dechunk_handles_multiple_chunks_and_returns_trailer_slice() {
        let input = b"3\r\none\r\n4\r\ntwo!\r\n0\r\nX-T: v\r\n\r\n";
        let (body, trailer) = dechunk(input).expect("dechunk");
        assert_eq!(body, b"onetwo!");
        assert_eq!(trailer, b"X-T: v\r\n\r\n");
    }

    #[test]
    fn dechunk_skips_chunk_extensions() {
        let (body, _) = dechunk(b"2;name=value\r\nhi\r\n0\r\n\r\n").expect("dechunk");
        assert_eq!(body, b"hi");
    }

    #[test]
    fn dechunk_rejects_bad_sizes_and_truncation() {
        assert!(dechunk(b"zz\r\nab\r\n0\r\n\r\n").is_err()); // not hex
        assert!(dechunk(b"5\r\nabc").is_err()); // truncated data
        assert!(dechunk(b"1\r\na").is_err()); // missing final CRLF + last-chunk
        assert!(dechunk(b"\r\n").is_err()); // empty size field
        let (body, trailer) = dechunk(b"0\r\n\r\n").expect("empty chunked body");
        assert!(body.is_empty());
        assert_eq!(trailer, b"\r\n");
    }

    #[test]
    fn oversized_chunk_is_refused_by_the_cap() {
        // A 4 GiB chunk claim must hit the cap long before any allocation.
        let input = b"FFFFFFFF\r\nx".to_vec();
        match dechunk(&input) {
            Err(NetError::ResponseTooLarge) => {}
            other => panic!("expected ResponseTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn connect_refused_maps_to_io_error() {
        // Port 1 on loopback: reserved, virtually guaranteed closed.
        let url = ParsedUrl::parse("http://127.0.0.1:1/x").expect("url");
        let err = get(&url, &FetchOptions::default()).expect_err("must fail");
        assert!(matches!(err, NetError::Io(_)), "got {err:?}");
    }
}

#[cfg(test)]
#[path = "http_coding_tests.rs"]
mod coding_tests;
