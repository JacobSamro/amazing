//! Bounded, blocking RFC9113 client with a hand-written RFC7541 codec.
//! One Client can multiplex requests; the current fetch dispatcher creates one
//! connection per fetch. No h2c, server push, pooling, prioritization or retry.
mod connection;
mod frame;
mod hpack;
mod semantics;
mod tables;
pub use connection::{Client, FrameIo, StreamEvent};
pub type Field = (Vec<u8>, Vec<u8>);
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
pub const MAX_FRAME: usize = 16384;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Protocol,
    FrameSize,
    Compression,
    FlowControl,
    Limit,
    Closed,
    StreamReset { stream: u32, code: u32 },
    Unprocessed(u32),
    GoAway { last_stream: u32, code: u32 },
}
impl Error {
    pub(super) fn code(&self) -> u32 {
        match self {
            Self::FrameSize => 6,
            Self::Compression => 9,
            Self::FlowControl => 3,
            Self::Limit => 11,
            Self::StreamReset { code, .. } => *code,
            _ => 1,
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP/2: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub header_list_bytes: usize,
    pub dynamic_table_bytes: usize,
    pub concurrent_streams: usize,
    pub body_bytes: usize,
    pub buffered_body_bytes: usize,
    /// Total incoming frames per connection, including unknown/empty/control frames.
    pub frames: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            header_list_bytes: 65536,
            dynamic_table_bytes: 4096,
            concurrent_streams: 32,
            body_bytes: 32 * 1024 * 1024,
            buffered_body_bytes: 32 * 1024 * 1024,
            frames: 100000,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<Self, Error> {
        if self.header_list_bytes == 0
            || self.header_list_bytes > 65536
            || self.dynamic_table_bytes > 4096
            || self.concurrent_streams == 0
            || self.concurrent_streams > 32
            || self.body_bytes > 32 * 1024 * 1024
            || self.buffered_body_bytes > 32 * 1024 * 1024
            || self.frames == 0
            || self.frames > 1000000
        {
            Err(Error::Limit)
        } else {
            Ok(self)
        }
    }
}
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<Field>,
    pub trailers: Vec<Field>,
    pub body: Vec<u8>,
}

/// Live fetch path and tests use this same protocol adapter.
pub(crate) fn fetch<S: std::io::Read + std::io::Write>(
    io: S,
    url: &crate::ParsedUrl,
    options: &crate::FetchOptions,
) -> Result<crate::Response, crate::NetError> {
    let map = |e: Error| match e {
        Error::Limit => crate::NetError::ResponseTooLarge,
        Error::Io(ref err) if matches!(err.kind(), std::io::ErrorKind::UnexpectedEof) => {
            crate::NetError::IncompleteBody
        }
        _ => crate::NetError::MalformedResponse(e.to_string()),
    };
    let headers = request_fields(url, options)?;
    let mut client = Client::new(io, Limits::default()).map_err(map)?;
    let id = client.start(&headers, &options.body).map_err(map)?;
    let reply = loop {
        if let Some(reply) = client.take_reply(id).map_err(map)? {
            break reply;
        }
        client.poll().map_err(map)?;
    };
    let mut fields = crate::Headers::new();
    for (name, value) in reply.headers {
        let name = std::str::from_utf8(&name)
            .map_err(|_| crate::NetError::MalformedResponse("non-UTF8 header name".into()))?;
        let value = String::from_utf8(value)
            .map_err(|_| crate::NetError::MalformedResponse("non-UTF8 header value".into()))?;
        fields.insert(name, value);
    }
    let body = if options.method.eq_ignore_ascii_case("HEAD") || matches!(reply.status, 204 | 304) {
        Vec::new()
    } else {
        crate::http::decode_body(
            &reply.body,
            &fields,
            false,
            options.max_response_bytes.min(crate::http::MAX_BODY_BYTES),
        )?
    };
    Ok(crate::Response {
        url: url.to_string(),
        status: reply.status,
        reason: String::new(),
        headers: fields,
        body,
    })
}

pub(crate) fn request_fields(
    url: &crate::ParsedUrl,
    options: &crate::FetchOptions,
) -> Result<Vec<Field>, crate::NetError> {
    let mut headers = vec![
        (
            b":method".to_vec(),
            crate::headers::normalize_method(&options.method).into_bytes(),
        ),
        (b":scheme".to_vec(), url.scheme().as_bytes().to_vec()),
        (b":authority".to_vec(), url.host_and_port().into_bytes()),
        (b":path".to_vec(), url.http_request_target()?.into_bytes()),
        (b"user-agent".to_vec(), crate::http::USER_AGENT.as_bytes().to_vec()),
        (b"accept".to_vec(), b"*/*".to_vec()),
        (b"accept-encoding".to_vec(), b"gzip, deflate, br".to_vec()),
    ];
    let mut header_bytes = headers
        .iter()
        .map(|(n, v)| n.len() + v.len() + 32)
        .sum::<usize>();
    for (name, value) in &options.request_headers {
        if name.len() > 65536 || value.len() > 65536 {
            return Err(crate::NetError::ResponseTooLarge);
        }
        let name = name.to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "host"
                | "connection"
                | "proxy-connection"
                | "keep-alive"
                | "transfer-encoding"
                | "upgrade"
                | "content-length"
                | "accept-encoding"
        ) {
            continue;
        }
        if !semantics::valid_name(name.as_bytes())
            || name.starts_with(':')
            || !semantics::valid_value(value.as_bytes())
        {
            continue;
        }
        header_bytes = header_bytes
            .checked_add(name.len())
            .and_then(|n| n.checked_add(value.len() + 32))
            .ok_or(crate::NetError::ResponseTooLarge)?;
        if header_bytes > 65500 {
            return Err(crate::NetError::ResponseTooLarge);
        }
        headers.push((name.into_bytes(), value.as_bytes().to_vec()));
    }
    if !options.body.is_empty() {
        headers.push((
            b"content-length".to_vec(),
            options.body.len().to_string().into_bytes(),
        ));
    }
    Ok(headers)
}
#[cfg(test)]
mod tests;
