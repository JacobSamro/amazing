//! Amazing net — Phase 5: URL parsing + HTTP/1.1 client over `std::net::TcpStream`.
//!
//! **Status: Phase 5 implemented** (URL parser/resolver, HTTP/1.1 GET with
//! Content-Length and chunked bodies, redirects, `file://` fetching).
//!
//! Modules, in dependency order:
//! - [`url`]: live [`ParsedUrl`] WHATWG parser plus a legacy compatibility URL; relative
//!   resolution against a base, percent-decoding. Never panics on malformed
//!   input; parsing failures are [`ParseError`] (legacy APIs use [`UrlError`]).
//! - [`headers`]: case-insensitive multi-value header map.
//! - [`response`]: final [`Response`] value with `text()`/`content_type()`.
//! - [`http`]: request writing + status-line/header parsing + body framing
//!   (Content-Length, chunked incl. trailer section, read-to-EOF).
//! - [`fetch`]: the [`fetch`] dispatcher used by earlier phases for local
//!   pages (`file://` and bare filesystem paths) and by Phase 5+ for `http`.
//!
//! # TLS decision (recorded per ARCHITECTURE.md §6)
//!
//! Hand-rolling TLS 1.3 is a security-critical, multi-month sub-project of
//! its own (record layers, AEAD ciphers, X.509 chain validation, revocation…)
//! and a mistake there is silently exploitable in a way a rendering bug never
//! is. The working assumption stands: Amazing will adopt a *vetted* TLS
//! implementation as infrastructure — the same call every serious
//! from-scratch engine makes — rather than hand-roll one. HTTPS crosses the
//! safe `amazing-net-tls` adapter, which pins rustls, verifies certificates
//! against Mozilla roots, and negotiates h2 with HTTP/1.1 fallback. Provider
//! types never cross into this crate's public API.
//!
//! # Implemented vs. deferred (deliberate, documented)
//!
//! Implemented:
//! - `GET` over plain HTTP/1.1 with `Host`, `User-Agent: Ferric/0.1`,
//!   `Accept: */*`, `Connection: close`
//! - status line + header parsing (case-insensitive names, repeated headers)
//! - body framing: `Content-Length`, `Transfer-Encoding: chunked` (with
//!   trailer section), and read-to-EOF when neither is present
//! - bounded gzip, zlib-wrapped deflate and isolated Brotli decoding, advertised
//!   as `Accept-Encoding: gzip, deflate, br`; checksum and framing validation
//! - negotiated HTTP/2, HPACK and bounded multiplexed streams
//! - redirect following (301/302/303/307/308) with `Location` resolved
//!   against the response URL, capped at 5 hops
//! - configurable socket timeout ([`FetchOptions::timeout`], default 10 s)
//! - relative URL resolution: scheme-relative `//`, absolute path `/`,
//!   query-only `?`, fragment-only `#`, bare relative paths with
//!   dot-segment removal
//! - `file://` fetch mapped to a filesystem read (Windows drive-letter
//!   paths included); bare paths treated as files for earlier phases
//!
//! Deferred (each is its own later work item, not an oversight):
//! - keep-alive / connection pooling (v1 sends `Connection: close`)
//! - methods other than GET/POST
//! - cookies, caching, conditional requests, authentication
//! - HTTP/3, h2c, server push and cross-fetch HTTP/2 connection pooling
//! - obsolete line folding and full obs-fold handling in headers
//! - IDNA/punycode: Unicode special hosts fail via the deferred provider seam

mod coding;
pub mod dispatch;
pub mod fetch;
pub mod headers;
pub mod http;
pub mod http2;
pub mod response;
pub mod stream;
pub mod url;

pub use fetch::{fetch, FetchOptions};
pub use headers::Headers;
pub use response::Response;
pub use stream::{fetch_stream, StreamOptions, StreamingResponse};
pub use url::{resolve, ParseError, ParsedUrl, Url, UrlError, UrlOrigin};

/// Top-level error type for network operations.
///
/// URL-syntax failures surface as [`NetError::Url`] wrapping the finer-grained
/// [`UrlError`]; everything transport-or-protocol-shaped lives here so callers
/// match on one enum.
#[derive(Debug)]
pub enum NetError {
    /// Underlying socket/filesystem I/O failure.
    Io(std::io::Error),
    /// The input was not a usable URL (wraps [`url::UrlError`] detail).
    Url(UrlError),
    /// Scheme known to the parser but outside `http`/`https`/`file`.
    UnsupportedScheme(String),
    /// HTTP method outside the current GET/POST browser surface.
    UnsupportedMethod(String),
    /// Certificate validation, handshake, or encrypted transport failure.
    Tls(String),
    /// More than [`fetch::MAX_REDIRECTS`] hops, or a bad hop along the way.
    TooManyRedirects,
    /// Redirect target had no resolvable `Location` header.
    MissingLocation,
    /// Peer spoke something that is not HTTP/1.x, or truncated framing.
    MalformedResponse(String),
    /// Connection closed before the promised body bytes arrived.
    IncompleteBody,
    /// Body exceeded the safety cap before framing completed.
    ResponseTooLarge,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Io(e) => write!(f, "i/o error: {e}"),
            NetError::Url(e) => write!(f, "invalid url: {e}"),
            NetError::UnsupportedScheme(s) => write!(f, "unsupported scheme: {s}"),
            NetError::UnsupportedMethod(method) => write!(f, "unsupported method: {method}"),
            NetError::Tls(error) => write!(f, "{error}"),
            NetError::TooManyRedirects => write!(f, "too many redirects"),
            NetError::MissingLocation => write!(f, "redirect without Location header"),
            NetError::MalformedResponse(d) => write!(f, "malformed response: {d}"),
            NetError::IncompleteBody => write!(f, "connection closed mid-body"),
            NetError::ResponseTooLarge => write!(f, "response body too large"),
        }
    }
}

impl std::error::Error for NetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NetError::Io(e) => Some(e),
            NetError::Url(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e)
    }
}

impl From<UrlError> for NetError {
    fn from(e: UrlError) -> Self {
        NetError::Url(e)
    }
}

// Keep the existing public NetError::Url shape for downstream error matches.
// The new parser's typed error remains available directly from ParsedUrl.
impl From<ParseError> for NetError {
    fn from(error: ParseError) -> Self {
        Self::Url(UrlError::InvalidUrl(error.to_string()))
    }
}

/// Bounded RFC6455 client over TCP or verified HTTP/1.1 TLS.
pub mod websocket;
