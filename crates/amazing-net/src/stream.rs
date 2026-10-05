//! Bounded pull-style response bodies for cooperative single-threaded callers.
//! Existing `fetch` remains the buffered compatibility entry point.
use crate::{
    coding::stream::{decode_layer, CapSource, CountSource, IoSource, Source},
    fetch::{FetchOptions, ReceivedSource, MAX_REDIRECTS},
    headers::Headers,
    http, http2, NetError, ParsedUrl,
};
use std::{
    cell::Cell,
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    rc::Rc,
};

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

#[derive(Clone, Copy, Debug)]
pub struct StreamOptions {
    /// Maximum returned chunk. Defaults to 8 KiB; hard ceiling 64 KiB.
    pub chunk_bytes: usize,
    /// Maximum decoded/encoded cumulative ratio, with 64 KiB grace for
    /// headers and short streams. Defaults to 1024.
    pub max_expansion_ratio: usize,
}
impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            chunk_bytes: 8192,
            max_expansion_ratio: 1024,
        }
    }
}
impl StreamOptions {
    fn validate(self) -> Result<Self, NetError> {
        if self.chunk_bytes == 0
            || self.chunk_bytes > 65536
            || self.max_expansion_ratio == 0
            || self.max_expansion_ratio > 4096
        {
            return Err(NetError::ResponseTooLarge);
        }
        Ok(self)
    }
}

/// Metadata is available after the response headers; each call advances only
/// far enough to produce the next decoded chunk. Dropping cancels the body.
pub struct StreamingResponse {
    pub url: String,
    pub status: u16,
    pub reason: String,
    pub headers: Headers,
    body: Box<dyn Source>,
    source: ReceivedSource,
    chunk_bytes: usize,
    done: bool,
}
impl StreamingResponse {
    /// Actual successful loader route, available before body drainage.
    pub fn source(&self) -> ReceivedSource { self.source }
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if self.done {
            return Ok(None);
        }
        let mut bytes = vec![0; self.chunk_bytes];
        let n = match self.body.read(&mut bytes) {
            Ok(n) => n,
            Err(error) => {
                self.done = true;
                return Err(error);
            }
        };
        if n == 0 {
            self.done = true;
            return Ok(None);
        }
        bytes.truncate(n);
        Ok(Some(bytes))
    }
}

/// Additive pull API for HTTP(S) and file resources. Redirect bodies are
/// dropped; the final response starts yielding after its headers arrive.
pub fn fetch_stream(
    input: &str,
    options: &FetchOptions,
    stream_options: StreamOptions,
) -> Result<StreamingResponse, NetError> {
    let stream_options = stream_options.validate()?;
    let trimmed = input.trim_matches(|c: char| c <= ' ');
    let scheme_input: String = trimmed
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    if crate::url::split_scheme(&scheme_input).is_none_or(|(scheme, _)| scheme.len() < 2) {
        return file_stream(
            Path::new(trimmed),
            trimmed.to_string(),
            options,
            stream_options,
        );
    }
    let mut current = ParsedUrl::parse(trimmed)?;
    let mut request_options = options.clone();
    for redirects in 0..=MAX_REDIRECTS {
        let response = match current.scheme() {
            "http" | "https" => open_network(&current, &request_options, stream_options)?,
            "file" => {
                return file_stream(
                    &current.to_file_path()?,
                    current.to_string(),
                    options,
                    stream_options,
                )
            }
            other => return Err(NetError::UnsupportedScheme(other.into())),
        };
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if redirects == MAX_REDIRECTS {
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
        let target = current
            .resolve(&location)?
            .with_inherited_fragment(&current);
        if !matches!(target.scheme(), "http" | "https") {
            return Err(NetError::UnsupportedScheme(target.scheme().into()));
        }
        current = target;
    }
    unreachable!()
}

fn file_stream(
    path: &Path,
    url: String,
    options: &FetchOptions,
    config: StreamOptions,
) -> Result<StreamingResponse, NetError> {
    if std::fs::metadata(path)?.len() > options.max_response_bytes as u64 {
        return Err(NetError::ResponseTooLarge);
    }
    let mut headers = Headers::new();
    let content_type = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "json" => "application/json",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };
    headers.insert("Content-Type", content_type.into());
    let body: Box<dyn Source> = Box::new(CapSource {
        inner: Box::new(IoSource(std::fs::File::open(path)?)),
        total: 0,
        limit: options.max_response_bytes.min(http::MAX_BODY_BYTES),
        ratio: None,
    });
    Ok(StreamingResponse {
        source: ReceivedSource::LOCAL_FILE,
        url,
        status: 200,
        reason: "OK".into(),
        headers,
        body,
        chunk_bytes: config.chunk_bytes,
        done: false,
    })
}

fn open_network(
    url: &ParsedUrl,
    options: &FetchOptions,
    config: StreamOptions,
) -> Result<StreamingResponse, NetError> {
    let host = url.hostname();
    let connect_host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url
        .effective_port()
        .ok_or(NetError::MalformedResponse("missing port".into()))?;
    if url.scheme() == "https" {
        let io = amazing_net_tls::connect(connect_host, port, options.timeout)
            .map_err(|e| NetError::Tls(e.to_string()))?;
        return if io.protocol() == amazing_net_tls::Protocol::H2 {
            open_h2(io, url, options, config)
        } else {
            open_http1(Box::new(io), url, options, config)
        };
    }
    let io = TcpStream::connect((connect_host, port))?;
    io.set_read_timeout(Some(options.timeout))?;
    io.set_write_timeout(Some(options.timeout))?;
    open_http1(Box::new(io), url, options, config)
}

fn map_h2(error: http2::Error) -> NetError {
    match error {
        http2::Error::Limit => NetError::ResponseTooLarge,
        http2::Error::Io(ref io) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
            NetError::IncompleteBody
        }
        other => NetError::MalformedResponse(other.to_string()),
    }
}

struct H2Body {
    client: http2::Client<Box<dyn ReadWrite>>,
    id: u32,
    pending: Vec<u8>,
    pos: usize,
    done: bool,
}
impl Source for H2Body {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        loop {
            if self.pos < self.pending.len() {
                let n = output.len().min(self.pending.len() - self.pos);
                output[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
                self.pos += n;
                if self.pos == self.pending.len() {
                    self.pending.clear();
                    self.pos = 0;
                }
                return Ok(n);
            }
            match self.client.next_event(self.id).map_err(map_h2)? {
                http2::StreamEvent::Data(bytes) => {
                    self.pending = bytes;
                    self.pos = 0;
                }
                http2::StreamEvent::End { .. } => {
                    self.done = true;
                    return Ok(0);
                }
                http2::StreamEvent::Head { .. } => {
                    return Err(NetError::MalformedResponse("duplicate h2 head".into()))
                }
            }
        }
    }
}

pub(crate) fn open_h2<S: Read + Write + 'static>(
    io: S,
    url: &ParsedUrl,
    options: &FetchOptions,
    config: StreamOptions,
) -> Result<StreamingResponse, NetError> {
    let fields = http2::request_fields(url, options)?;
    let mut client =
        http2::Client::new(Box::new(io) as Box<dyn ReadWrite>, http2::Limits::default())
            .map_err(map_h2)?;
    let id = client
        .start_streaming(&fields, &options.body)
        .map_err(map_h2)?;
    let (status, fields) = match client.next_event(id).map_err(map_h2)? {
        http2::StreamEvent::Head { status, headers } => (status, headers),
        _ => {
            return Err(NetError::MalformedResponse(
                "h2 response missing head".into(),
            ))
        }
    };
    let mut headers = Headers::new();
    for (name, value) in fields {
        let name = std::str::from_utf8(&name)
            .map_err(|_| NetError::MalformedResponse("non-UTF8 header name".into()))?;
        let value = String::from_utf8(value)
            .map_err(|_| NetError::MalformedResponse("non-UTF8 header value".into()))?;
        headers.insert(name, value);
    }
    let raw = Box::new(H2Body {
        client,
        id,
        pending: Vec::new(),
        pos: 0,
        done: false,
    });
    let body = if options.method.eq_ignore_ascii_case("HEAD") || matches!(status, 204 | 304) {
        raw as Box<dyn Source>
    } else {
        pipeline(raw, &headers, options, config, true)?
    };
    Ok(StreamingResponse {
        source: ReceivedSource::NETWORK,
        url: url.to_string(),
        status,
        reason: String::new(),
        headers,
        body,
        chunk_bytes: config.chunk_bytes,
        done: false,
    })
}

fn open_http1(
    mut io: Box<dyn ReadWrite>,
    url: &ParsedUrl,
    options: &FetchOptions,
    config: StreamOptions,
) -> Result<StreamingResponse, NetError> {
    io.write_all(&http::request_bytes(url, options)?)?;
    io.flush()?;
    let mut head = Vec::new();
    loop {
        if head.len() == 65536 {
            return Err(NetError::ResponseTooLarge);
        }
        let mut b = [0];
        if io.read(&mut b)? == 0 {
            return Err(NetError::IncompleteBody);
        }
        head.push(b[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let (status, reason, headers) = http::parse_head(&head[..head.len() - 4])?;
    if (100..200).contains(&status) {
        return Err(NetError::MalformedResponse(
            "informational responses unsupported".into(),
        ));
    }
    let body = if matches!(status, 204 | 304) {
        Box::new(Http1Body::new(io, &headers, options, status)?) as Box<dyn Source>
    } else {
        pipeline(
            Box::new(Http1Body::new(io, &headers, options, status)?),
            &headers,
            options,
            config,
            false,
        )?
    };
    Ok(StreamingResponse {
        source: ReceivedSource::NETWORK,
        url: url.to_string(),
        status,
        reason,
        headers,
        body,
        chunk_bytes: config.chunk_bytes,
        done: false,
    })
}

fn pipeline(
    raw: Box<dyn Source>,
    headers: &Headers,
    options: &FetchOptions,
    config: StreamOptions,
    h2: bool,
) -> Result<Box<dyn Source>, NetError> {
    let mut transfer = http::codings(headers, "transfer-encoding")?;
    let content = http::codings(headers, "content-encoding")?;
    if content.iter().any(|c| c == "chunked") || transfer.iter().any(|c| c == "identity") {
        return Err(NetError::MalformedResponse("coding in wrong header".into()));
    }
    if !transfer.is_empty() && headers.contains("content-length") {
        return Err(NetError::MalformedResponse(
            "Transfer-Encoding with Content-Length".into(),
        ));
    }
    if h2 && !transfer.is_empty() {
        return Err(NetError::MalformedResponse(
            "HTTP/2 transfer encoding".into(),
        ));
    }
    if transfer.last().is_some_and(|c| c == "chunked") {
        transfer.pop();
    }
    if transfer.iter().any(|c| c == "chunked") || transfer.len() + content.len() > 8 {
        return Err(NetError::MalformedResponse("invalid coding layers".into()));
    }
    let limit = options.max_response_bytes.min(http::MAX_BODY_BYTES);
    let encoded = Rc::new(Cell::new(0));
    let mut source: Box<dyn Source> = Box::new(CountSource {
        inner: raw,
        count: encoded.clone(),
    });
    for coding in transfer.iter().rev().chain(content.iter().rev()) {
        source = decode_layer(source, coding, limit)?;
    }
    Ok(Box::new(CapSource {
        inner: source,
        total: 0,
        limit,
        ratio: Some((encoded, config.max_expansion_ratio)),
    }))
}

enum Framing {
    Empty {
        checked: bool,
    },
    Length(usize),
    Chunked {
        left: usize,
        need_crlf: bool,
        done: bool,
    },
    Eof,
}
struct Http1Body {
    io: Box<dyn ReadWrite>,
    framing: Framing,
    total: usize,
    limit: usize,
}
impl Http1Body {
    fn new(
        io: Box<dyn ReadWrite>,
        headers: &Headers,
        options: &FetchOptions,
        status: u16,
    ) -> Result<Self, NetError> {
        let transfer = http::codings(headers, "transfer-encoding")?;
        let content = http::codings(headers, "content-encoding")?;
        let compressed = transfer
            .iter()
            .chain(&content)
            .any(|c| c != "identity" && c != "chunked");
        let limit = if compressed {
            http::MAX_BODY_BYTES
        } else {
            options.max_response_bytes.min(http::MAX_BODY_BYTES)
        };
        let framing = if matches!(status, 204 | 304) {
            Framing::Empty { checked: false }
        } else if transfer.last().is_some_and(|c| c == "chunked") {
            Framing::Chunked {
                left: 0,
                need_crlf: false,
                done: false,
            }
        } else if let Some(length) = http::content_length(headers)? {
            if length > limit {
                return Err(NetError::ResponseTooLarge);
            }
            Framing::Length(length)
        } else {
            Framing::Eof
        };
        Ok(Self {
            io,
            framing,
            total: 0,
            limit,
        })
    }
    fn line(&mut self) -> Result<Vec<u8>, NetError> {
        let mut line = Vec::new();
        loop {
            if line.len() >= 8192 {
                return Err(NetError::ResponseTooLarge);
            }
            let mut b = [0];
            if self.io.read(&mut b)? == 0 {
                return Err(NetError::IncompleteBody);
            }
            line.push(b[0]);
            if line.ends_with(b"\r\n") {
                line.truncate(line.len() - 2);
                return Ok(line);
            }
        }
    }
    fn verify_end(&mut self, message: &'static str) -> Result<(), NetError> {
        let mut b = [0];
        match self.io.read(&mut b) {
            Ok(0) => Ok(()),
            Ok(_) => Err(NetError::MalformedResponse(message.into())),
            // Explicit framing is complete even if TLS omits close_notify.
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    fn finish_chunks(&mut self) -> Result<(), NetError> {
        let mut total = 0;
        for _ in 0..128 {
            let line = self.line()?;
            total += line.len() + 2;
            if total > 65536 {
                return Err(NetError::ResponseTooLarge);
            }
            if line.is_empty() {
                return self.verify_end("bytes after chunked message");
            }
            if !line.contains(&b':') {
                return Err(NetError::MalformedResponse("invalid trailer field".into()));
            }
        }
        Err(NetError::ResponseTooLarge)
    }
}
impl Source for Http1Body {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let framing = std::mem::replace(&mut self.framing, Framing::Empty { checked: true });
            match framing {
                Framing::Empty { checked } => {
                    if !checked {
                        self.verify_end("body on bodyless response")?;
                    }
                    return Ok(0);
                }
                Framing::Length(left) => {
                    if left == 0 {
                        self.verify_end("body exceeds Content-Length")?;
                        return Ok(0);
                    }
                    let count = output.len().min(left);
                    let n = self.io.read(&mut output[..count])?;
                    if n == 0 {
                        return Err(NetError::IncompleteBody);
                    }
                    self.total += n;
                    self.framing = Framing::Length(left - n);
                    return Ok(n);
                }
                Framing::Eof => {
                    let allowed = self.limit.saturating_sub(self.total);
                    let count = output.len().min(allowed.saturating_add(1));
                    let n = self.io.read(&mut output[..count])?;
                    if n > allowed {
                        return Err(NetError::ResponseTooLarge);
                    }
                    self.total += n;
                    self.framing = if n == 0 {
                        Framing::Empty { checked: true }
                    } else {
                        Framing::Eof
                    };
                    return Ok(n);
                }
                Framing::Chunked {
                    mut left,
                    mut need_crlf,
                    done,
                } => {
                    if done {
                        return Ok(0);
                    }
                    if left == 0 {
                        if need_crlf {
                            let mut crlf = [0; 2];
                            crate::coding::stream::read_exact(
                                &mut IoSource(&mut self.io),
                                &mut crlf,
                            )?;
                            if crlf != *b"\r\n" {
                                return Err(NetError::MalformedResponse(
                                    "missing CRLF after chunk data".into(),
                                ));
                            }
                            need_crlf = false;
                        }
                        let line = self.line()?;
                        let text = std::str::from_utf8(&line).map_err(|_| {
                            NetError::MalformedResponse("non-UTF8 chunk size".into())
                        })?;
                        let digits = text.split(';').next().unwrap_or("").trim();
                        left = usize::from_str_radix(digits, 16).map_err(|_| {
                            NetError::MalformedResponse("invalid chunk size".into())
                        })?;
                        if left == 0 {
                            self.finish_chunks()?;
                            self.framing = Framing::Chunked {
                                left: 0,
                                need_crlf: false,
                                done: true,
                            };
                            return Ok(0);
                        }
                        if left > self.limit.saturating_sub(self.total) {
                            return Err(NetError::ResponseTooLarge);
                        }
                    }
                    let count = output.len().min(left);
                    let n = self.io.read(&mut output[..count])?;
                    if n == 0 {
                        return Err(NetError::IncompleteBody);
                    }
                    self.total += n;
                    left -= n;
                    if left == 0 {
                        need_crlf = true;
                    }
                    self.framing = Framing::Chunked {
                        left,
                        need_crlf,
                        done: false,
                    };
                    return Ok(n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    struct TinyIo {
        input: Cursor<Vec<u8>>,
    }
    impl Read for TinyIo {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let count = output.len().min(1);
            self.input.read(&mut output[..count])
        }
    }
    impl Write for TinyIo {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct UncleanIo(TinyIo);
    impl Read for UncleanIo {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(output)?;
            if n == 0 {
                Err(std::io::ErrorKind::UnexpectedEof.into())
            } else {
                Ok(n)
            }
        }
    }
    impl Write for UncleanIo {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    fn unclean_body(raw: &[u8]) -> Result<Vec<u8>, NetError> {
        let url = ParsedUrl::parse("http://example.test/").unwrap();
        let mut response = open_http1(
            Box::new(UncleanIo(TinyIo {
                input: Cursor::new(raw.to_vec()),
            })),
            &url,
            &FetchOptions::default(),
            StreamOptions::default(),
        )?;
        let mut result = Vec::new();
        while let Some(chunk) = response.next_chunk()? {
            result.extend_from_slice(&chunk);
        }
        Ok(result)
    }
    fn body(
        coding: &str,
        encoded: &[u8],
        chunked: bool,
        limit: usize,
    ) -> Result<Vec<u8>, NetError> {
        let mut raw = if chunked {
            format!("HTTP/1.1 200 OK\r\nContent-Encoding: {coding}\r\nTransfer-Encoding: chunked\r\n\r\n").into_bytes()
        } else {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Encoding: {coding}\r\nContent-Length: {}\r\n\r\n",
                encoded.len()
            )
            .into_bytes()
        };
        if chunked {
            for part in encoded.chunks(17) {
                raw.extend_from_slice(format!("{:x};ext=x\r\n", part.len()).as_bytes());
                raw.extend_from_slice(part);
                raw.extend_from_slice(b"\r\n");
            }
            raw.extend_from_slice(b"0\r\nX-End: yes\r\n\r\n");
        } else {
            raw.extend_from_slice(encoded);
        }
        let url = ParsedUrl::parse("http://example.test/").unwrap();
        let mut response = open_http1(
            Box::new(TinyIo {
                input: Cursor::new(raw),
            }),
            &url,
            &FetchOptions {
                max_response_bytes: limit,
                ..FetchOptions::default()
            },
            StreamOptions {
                chunk_bytes: 3,
                ..StreamOptions::default()
            },
        )?;
        let mut result = Vec::new();
        while let Some(chunk) = response.next_chunk()? {
            assert!(chunk.len() <= 3);
            result.extend_from_slice(&chunk);
        }
        Ok(result)
    }
    #[test]
    fn one_byte_http_reads_split_chunk_size_gzip_header_and_brotli_block() {
        let (plain, gzip) = crate::coding::tests::fixture("hello-fixed-gzip");
        assert_eq!(body("gzip", &gzip, true, plain.len()).unwrap(), plain);
        assert_eq!(body("gzip", &gzip, false, plain.len()).unwrap(), plain);
        let (plain, zlib) = crate::coding::tests::fixture("hello-fixed-zlib");
        assert_eq!(body("deflate", &zlib, true, plain.len()).unwrap(), plain);
        let plain = include_bytes!("../../amazing-net-brotli/tests/vectors/hello.raw");
        let br = include_bytes!("../../amazing-net-brotli/tests/vectors/hello-11.br");
        assert_eq!(body("br", br, true, plain.len()).unwrap(), plain);
    }
    #[test]
    fn decoded_budget_rejects_before_exposing_expansion() {
        let (plain, gzip) = crate::coding::tests::fixture("overlap-default-gzip");
        assert!(plain.len() > 10);
        assert!(matches!(
            body("gzip", &gzip, true, 10),
            Err(NetError::ResponseTooLarge)
        ));
    }
    #[test]
    fn complete_framing_survives_unclean_tls_eof_but_eof_framing_does_not() {
        assert_eq!(
            unclean_body(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").unwrap(),
            b"ok"
        );
        assert_eq!(
            unclean_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\n\r\n")
                .unwrap(),
            b"ok"
        );
        assert_eq!(
            unclean_body(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap(),
            b""
        );
        assert!(matches!(
            unclean_body(b"HTTP/1.1 200 OK\r\n\r\nok"),
            Err(NetError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof
        ));
    }
}
