//! URL parsing, relative resolution, and percent-decoding.
//!
//! [`ParsedUrl`] is the immutable WHATWG basic-parser API (UTF-8; Unicode
//! domains require an [`IdnaProvider`]). It is the live network transport
//! and Fetch policy parser. The public-field [`Url`] below is the deprecated legacy
//! API; the following implementation notes describe that legacy API only.
//!
//! Implements the slice of WHATWG URL / RFC 3986 a v1 engine needs:
//!
//! Implemented:
//! - absolute `http:`/`https:` URLs with authority parsing (IPv6 bracket
//!   literals, default ports, userinfo dropped), path, query, fragment
//! - `file:` URLs mapped to filesystem paths (Windows drive-letter form
//!   `file:///C:/...` included; `file://localhost/...` tolerated)
//! - relative resolution against a base: scheme-relative `//host/path`,
//!   path-absolute `/x`, query-only `?x`, fragment-only `#x`, empty
//!   reference, and bare relative paths with RFC 3986 §5.2.4
//!   dot-segment removal (`..` clamps at the root)
//! - percent-decoding for paths ([`Url::decoded_path`]); the stored
//!   [`Url::path`] keeps the raw (still-encoded) form so HTTP request
//!   targets are forwarded byte-for-byte
//!
//! Deferred (documented limits, not bugs):
//! - no IDNA/punycode host normalization; hosts are lowercased ASCII-wise
//!   and otherwise compared as written
//! - `%2E` is *not* treated as a dot segment (RFC 3986 literal matching
//!   only); no backslash-as-slash quirk, no non-special-scheme handling
//! - userinfo is dropped, never interpreted or re-sent
//! - `file://host/path` with a non-local host is accepted and the host is
//!   ignored (UNC paths are out of scope); prefer the three-slash form
//! - query strings and fragments are stored raw (no `+`-as-space, which is
//!   form-encoding semantics, not URL syntax)
//!
//! Parsing never panics on malformed input; every failure is an
//! [`UrlError`].

// The immutable standards parser is live in transport and Fetch policy.
// The legacy public-field Url below is only an external-consumer shim.
mod encoding;
mod host;
mod parsed;
mod parser;
pub use encoding::{percent_encode, PercentEncodeSet};
pub use host::{AsciiOnly, Host, IdnaProvider};
pub use parsed::{ParseError, ParsedUrl, TupleOrigin, UrlOrigin};

use std::fmt;
use std::path::PathBuf;

/// Why a URL could not be parsed or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    /// Catch-all for structurally invalid input, with a detail string.
    InvalidUrl(String),
    /// A relative reference was parsed with no base URL to resolve against.
    RelativeWithoutBase,
    /// `http`/`https` require a non-empty host.
    EmptyHost,
    /// Port was not empty, all-digits, and in `0..=65535`.
    InvalidPort(String),
    /// Scheme is recognized syntactically but not supported by this crate.
    UnsupportedScheme(String),
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::InvalidUrl(d) => write!(f, "invalid URL: {d}"),
            UrlError::RelativeWithoutBase => write!(f, "relative URL without a base"),
            UrlError::EmptyHost => write!(f, "empty host"),
            UrlError::InvalidPort(p) => write!(f, "invalid port: {p}"),
            UrlError::UnsupportedScheme(s) => write!(f, "unsupported scheme: {s}"),
        }
    }
}

impl std::error::Error for UrlError {}

/// Compatibility-only legacy URL; new code must use [`ParsedUrl`].
///
/// This public-field API is retained for shell/driver consumers. It is no
/// longer used by network I/O or Fetch policy, and does not have the new
/// parser's validation guarantees. Removal requires coordinated migration.
///
/// A parsed URL. `path` is stored percent-encoded exactly as written (it
/// always starts with `/`); use [`Url::decoded_path`] for the readable form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// Lowercased scheme without the trailing `:` (`"http"`, `"file"`, …).
    pub scheme: String,
    /// Lowercased host; IPv6 literals keep their brackets (`"[::1]"`).
    /// Empty for `file:` URLs.
    pub host: String,
    /// Port with the scheme default already applied (http 80, https 443,
    /// file 0).
    pub port: u16,
    /// Raw (still percent-encoded) path, always starting with `/`.
    pub path: String,
    /// Query without the leading `?`, stored raw; `None` when absent.
    pub query: Option<String>,
    /// Fragment without the leading `#`, stored raw; `None` when absent.
    pub fragment: Option<String>,
}

impl Url {
    /// Parses an absolute URL (`http:`/`https:`/`file:`).
    pub fn parse(input: &str) -> Result<Url, UrlError> {
        parse_with_base(input, None)
    }

    /// Path with `%XX` escapes decoded. Invalid escapes are left as-is;
    /// invalid UTF-8 after decoding becomes U+FFFD (lossy). `+` is *not*
    /// decoded to a space (that is form encoding, not URL syntax).
    pub fn decoded_path(&self) -> String {
        percent_decode(&self.path)
    }

    /// The origin-form request target for the HTTP/1.1 request line:
    /// raw path plus `?query` when present.
    pub fn request_target(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{}", self.path, q),
            None => self.path.clone(),
        }
    }

    /// Value for the `Host` header: host, with the port appended only when
    /// it differs from the scheme default.
    pub fn host_header(&self) -> String {
        if Some(self.port) == default_port(&self.scheme) {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Maps a `file:` URL to a local filesystem path.
    ///
    /// On Windows, the drive-letter form `/C:/...` loses its leading slash
    /// (`C:/...`); elsewhere the decoded path is used as-is.
    pub fn to_file_path(&self) -> Result<PathBuf, UrlError> {
        if self.scheme != "file" {
            return Err(UrlError::InvalidUrl("not a file:// URL".into()));
        }
        let decoded = self.decoded_path();
        let bytes = decoded.as_bytes();
        let windows_drive = cfg!(windows)
            && bytes.len() >= 3
            && decoded.starts_with('/')
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        let path = if windows_drive {
            decoded[1..].to_string()
        } else {
            decoded
        };
        Ok(PathBuf::from(path))
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://", self.scheme)?;
        if !self.host.is_empty() {
            write!(f, "{}", self.host)?;
            if Some(self.port) != default_port(&self.scheme) {
                write!(f, ":{}", self.port)?;
            }
        }
        if self.path != "/" {
            write!(f, "{}", self.path)?;
        }
        if let Some(q) = &self.query {
            write!(f, "?{q}")?;
        }
        if let Some(fr) = &self.fragment {
            write!(f, "#{fr}")?;
        }
        Ok(())
    }
}

/// Resolves `input` against `base`. Absolute inputs win; otherwise the
/// scheme-relative, path-absolute, query-only, fragment-only, empty, and
/// bare-relative forms are handled (see the module docs).
pub fn resolve(input: &str, base: &Url) -> Result<Url, UrlError> {
    parse_with_base(input, Some(base))
}

/// Scheme extractor shared with the fetch dispatcher: returns the lowercased
/// scheme and the remainder after `:` when `input` starts with a valid
/// scheme (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ": "` per RFC 3986).
pub(crate) fn split_scheme(input: &str) -> Option<(String, &str)> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let mut end = first.len_utf8();
    for (i, c) in chars {
        if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.' {
            end = i + c.len_utf8();
        } else if c == ':' {
            return Some((input[..end].to_ascii_lowercase(), &input[i + 1..]));
        } else {
            return None;
        }
    }
    None // ran out of input before any ':'
}

fn parse_with_base(input: &str, base: Option<&Url>) -> Result<Url, UrlError> {
    // Strip leading/trailing C0 controls and spaces (lenient WHATWG trim).
    let trimmed = input.trim_matches(|c: char| c.is_ascii() && c <= ' ');
    // The fragment splits off first and survives any kind of resolution.
    let (rest, fragment) = match trimmed.find('#') {
        Some(i) => (&trimmed[..i], Some(trimmed[i + 1..].to_string())),
        None => (trimmed, None),
    };
    // An empty reference resolves to the base itself (RFC 3986 §5.3); with
    // no base it is simply invalid input.
    if rest.is_empty() {
        return match base {
            Some(b) => Ok(Url {
                scheme: b.scheme.clone(),
                host: b.host.clone(),
                port: b.port,
                path: b.path.clone(),
                query: b.query.clone(),
                fragment,
            }),
            None => Err(UrlError::InvalidUrl("empty input".into())),
        };
    }
    match split_scheme(rest) {
        Some((scheme, after)) => parse_absolute(&scheme, after, fragment),
        None => {
            let base = base.ok_or(UrlError::RelativeWithoutBase)?;
            resolve_relative(rest, base, fragment)
        }
    }
}

fn parse_absolute(scheme: &str, after: &str, fragment: Option<String>) -> Result<Url, UrlError> {
    match scheme {
        "http" | "https" => {
            let rest = after.strip_prefix("//").ok_or_else(|| {
                UrlError::InvalidUrl(format!("{scheme}: URL requires a // authority"))
            })?;
            let (authority, path_and_query) = split_authority_end(rest);
            let (host, port) = parse_authority(authority, scheme)?;
            if host.is_empty() {
                return Err(UrlError::EmptyHost);
            }
            let (path, query) = split_path_query(path_and_query);
            Ok(Url {
                scheme: scheme.to_string(),
                host,
                port,
                path: normalize_path(&path),
                query,
                fragment,
            })
        }
        "file" => parse_file(after, fragment),
        other => Err(UrlError::UnsupportedScheme(other.to_string())),
    }
}

/// Splits `//authority/rest` at the first `/` or `?` (fragment already gone).
fn split_authority_end(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    }
}

fn parse_authority(authority: &str, scheme: &str) -> Result<(String, u16), UrlError> {
    // Userinfo (`user:pass@host`) is dropped, never interpreted.
    let hostport = match authority.rfind('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    };
    let fallback = default_port(scheme).unwrap_or(0);
    if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 literal: brackets belong to the host, an optional port
        // follows `]` behind its own colon.
        let close = rest
            .find(']')
            .ok_or_else(|| UrlError::InvalidUrl("unterminated [ IPv6 literal".into()))?;
        let host = format!("[{}]", &rest[..close]).to_ascii_lowercase();
        let after_bracket = &rest[close + 1..];
        let port = parse_port(after_bracket.strip_prefix(':').unwrap_or(""), fallback)?;
        Ok((host, port))
    } else {
        // Splitting at the *first* ':' keeps the host colon-free by
        // construction; anything left of it that still looks like a second
        // port fails digit validation below ("a:b:c" -> port "b:c").
        let (host, port) = match hostport.find(':') {
            Some(i) => (&hostport[..i], parse_port(&hostport[i + 1..], fallback)?),
            None => (hostport, fallback),
        };
        Ok((host.to_ascii_lowercase(), port))
    }
}

fn parse_port(s: &str, fallback: u16) -> Result<u16, UrlError> {
    if s.is_empty() {
        return Ok(fallback); // "host:" is leniently treated as the default
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(UrlError::InvalidPort(s.to_string()));
    }
    s.parse::<u16>().map_err(|_| UrlError::InvalidPort(s.to_string()))
}

fn parse_file(after: &str, fragment: Option<String>) -> Result<Url, UrlError> {
    // Lenient forms: file:///p, file://localhost/p (host ignored), file:/p,
    // file:p. The three-slash form is the documented, fully-supported one.
    let raw = if let Some(rest) = after.strip_prefix("//") {
        let (_, tail) = split_authority_end(rest);
        tail.to_string()
    } else {
        after.to_string()
    };
    let raw = if raw.is_empty() {
        "/".to_string()
    } else if raw.starts_with('/') {
        raw
    } else {
        format!("/{raw}")
    };
    let (path, query) = split_path_query(&raw);
    Ok(Url {
        scheme: "file".to_string(),
        host: String::new(),
        port: 0,
        path: normalize_path(&path),
        query,
        fragment,
    })
}

fn resolve_relative(rest: &str, base: &Url, fragment: Option<String>) -> Result<Url, UrlError> {
    // Empty references never reach here (handled in parse_with_base).
    // Scheme-relative: "//host/path" borrows the base scheme.
    if let Some(after) = rest.strip_prefix("//") {
        return parse_absolute(&base.scheme, &format!("//{after}"), fragment);
    }
    // Path-absolute: "/x" keeps the authority, replaces path + query.
    if let Some(path_and_query) = rest.strip_prefix('/') {
        let (path, query) = split_path_query(&format!("/{path_and_query}"));
        return Ok(Url {
            scheme: base.scheme.clone(),
            host: base.host.clone(),
            port: base.port,
            path: normalize_path(&path),
            query,
            fragment,
        });
    }
    // Query-only: "?x" keeps the base path verbatim.
    if let Some(q) = rest.strip_prefix('?') {
        return Ok(Url {
            scheme: base.scheme.clone(),
            host: base.host.clone(),
            port: base.port,
            path: base.path.clone(),
            query: if q.is_empty() { None } else { Some(q.to_string()) },
            fragment,
        });
    }
    // Bare relative: merge onto the base directory, then remove dot segments.
    let merged = merge_paths(&base.path, rest);
    let (path, query) = split_path_query(&merged);
    Ok(Url {
        scheme: base.scheme.clone(),
        host: base.host.clone(),
        port: base.port,
        path: normalize_path(&path),
        query,
        fragment,
    })
}

/// Base directory (up to and including the last `/`) plus the relative part.
fn merge_paths(base_path: &str, rel: &str) -> String {
    let dir = match base_path.rfind('/') {
        Some(i) => &base_path[..=i],
        None => "/",
    };
    format!("{dir}{rel}")
}

/// RFC 3986 §5.2.4 remove_dot_segments, stack-style. `path` must start with
/// `/`; `..` above the root clamps to `/`. Empty segments (hence `//`) and
/// trailing slashes are preserved; `%2E` is not treated as a dot.
fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect(); // segments[0] == ""
    let last = segments.len().saturating_sub(1);
    let mut out: Vec<&str> = Vec::new();
    for (i, seg) in segments.iter().enumerate().skip(1) {
        match *seg {
            "." => {
                if i == last {
                    out.push(""); // "/a/." keeps its trailing slash
                }
            }
            ".." => {
                out.pop();
                if i == last {
                    out.push(""); // "/a/.." becomes "/"
                }
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

/// Splits `path?query`; an empty path becomes `/`. A bare `?` (empty query)
/// counts as absent.
fn split_path_query(s: &str) -> (String, Option<String>) {
    match s.find('?') {
        Some(i) => {
            let path = if s[..i].is_empty() { "/" } else { &s[..i] };
            let q = &s[i + 1..];
            let query = if q.is_empty() { None } else { Some(q.to_string()) };
            (path.to_string(), query)
        }
        None => {
            let path = if s.is_empty() { "/" } else { s };
            (path.to_string(), None)
        }
    }
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

/// Decodes `%XX` escapes; invalid escapes pass through byte-for-byte, and
/// the result is lossily reinterpreted as UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hi = (bytes[i + 1] as char).to_digit(16).unwrap_or(0) as u8;
            let lo = (bytes[i + 2] as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn err(s: &str) -> UrlError {
        Url::parse(s).expect_err(s)
    }

    #[test]
    fn parses_minimal_http_url_with_default_port_and_root_path() {
        let url = u("http://example.com");
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 80);
        assert_eq!(url.path, "/");
        assert_eq!(url.query, None);
        assert_eq!(url.fragment, None);
        assert_eq!(url.host_header(), "example.com");
    }

    #[test]
    fn parses_full_url_with_port_query_and_fragment() {
        let url = u("https://example.com:8443/a/b?x=1&y=2#frag");
        assert_eq!(url.scheme, "https");
        assert_eq!(url.port, 8443);
        assert_eq!(url.path, "/a/b");
        assert_eq!(url.query.as_deref(), Some("x=1&y=2"));
        assert_eq!(url.fragment.as_deref(), Some("frag"));
        assert_eq!(url.host_header(), "example.com:8443");
    }

    #[test]
    fn lowercases_scheme_and_host_but_preserves_path_case() {
        let url = u("HTTP://EXAMPLE.COM/Path?Q=1");
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.path, "/Path");
        assert_eq!(url.query.as_deref(), Some("Q=1"));
    }

    #[test]
    fn parses_ipv6_literal_host_and_port() {
        let url = u("http://[2001:DB8::1]:8080/x");
        assert_eq!(url.host, "[2001:db8::1]");
        assert_eq!(url.port, 8080);
        assert_eq!(url.host_header(), "[2001:db8::1]:8080");
        // No port at all: default must apply, not error.
        assert_eq!(u("http://[::1]/x").port, 80);
        assert_eq!(u("https://[::1]/x").port, 443);
    }

    #[test]
    fn drops_userinfo_and_keeps_host() {
        let url = u("http://alice:s3cret@example.com/x");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.path, "/x");
    }

    #[test]
    fn empty_port_falls_back_to_default() {
        assert_eq!(u("http://example.com:/x").port, 80);
        assert_eq!(u("https://example.com:/").port, 443);
    }

    #[test]
    fn query_only_url_gets_root_path() {
        let url = u("http://example.com?q");
        assert_eq!(url.path, "/");
        assert_eq!(url.query.as_deref(), Some("q"));
    }

    #[test]
    fn first_hash_splits_the_fragment() {
        let url = u("http://example.com/p#x#y");
        assert_eq!(url.path, "/p");
        assert_eq!(url.fragment.as_deref(), Some("x#y"));
    }

    #[test]
    fn request_target_keeps_raw_encoding_and_appends_query() {
        let url = u("http://example.com/a%20b/c?d=e");
        assert_eq!(url.request_target(), "/a%20b/c?d=e");
    }

    #[test]
    fn percent_decoding_of_paths() {
        assert_eq!(u("http://x/a%20b").decoded_path(), "/a b");
        assert_eq!(u("http://x/a%2Fb").decoded_path(), "/a/b");
        assert_eq!(u("http://x/caf%C3%A9").decoded_path(), "/café");
        // Invalid escapes survive untouched.
        assert_eq!(u("http://x/a%2").decoded_path(), "/a%2");
        assert_eq!(u("http://x/a%zz").decoded_path(), "/a%zz");
        assert_eq!(u("http://x/100%").decoded_path(), "/100%");
    }

    #[test]
    fn file_url_unix_path_roundtrip() {
        let url = u("file:///usr/share/page.html");
        assert_eq!(url.scheme, "file");
        assert_eq!(url.host, "");
        assert_eq!(url.path, "/usr/share/page.html");
        assert_eq!(url.to_file_path().unwrap(), PathBuf::from("/usr/share/page.html"));
    }

    #[test]
    fn file_url_windows_drive_letter() {
        let url = u("file:///C:/Users/x/page.html");
        // Platform-dependent mapping; both branches must parse cleanly.
        if cfg!(windows) {
            assert_eq!(url.to_file_path().unwrap(), PathBuf::from("C:/Users/x/page.html"));
        } else {
            assert_eq!(url.to_file_path().unwrap(), PathBuf::from("/C:/Users/x/page.html"));
        }
    }

    #[test]
    fn file_url_percent_decoding_reaches_the_path() {
        let url = u("file:///dir/with%20space/f.txt");
        assert_eq!(url.decoded_path(), "/dir/with space/f.txt");
    }

    #[test]
    fn file_url_localhost_authority_is_tolerated() {
        let url = u("file://localhost/tmp/x");
        assert_eq!(url.path, "/tmp/x");
    }

    #[test]
    fn to_file_path_rejects_non_file_schemes() {
        assert!(u("http://example.com/").to_file_path().is_err());
    }

    #[test]
    fn scheme_relative_resolution_borrows_base_scheme() {
        let base = u("http://a/b/c");
        let r = resolve("//other.host/x?q", &base).unwrap();
        assert_eq!(r.scheme, "http");
        assert_eq!(r.host, "other.host");
        assert_eq!(r.path, "/x");
        assert_eq!(r.query.as_deref(), Some("q"));
    }

    #[test]
    fn path_absolute_resolution_keeps_authority() {
        let base = u("http://user@host.example:8000/b/c");
        let r = resolve("/top", &base).unwrap();
        assert_eq!(r.host, "host.example");
        assert_eq!(r.port, 8000);
        assert_eq!(r.path, "/top");
    }

    #[test]
    fn bare_relative_resolution_merges_with_base_directory() {
        let base = u("http://a/b/c");
        let r = resolve("g/h?y#s", &base).unwrap();
        assert_eq!(r.host, "a");
        assert_eq!(r.path, "/b/g/h");
        assert_eq!(r.query.as_deref(), Some("y"));
        assert_eq!(r.fragment.as_deref(), Some("s"));
    }

    #[test]
    fn dot_segment_removal_during_resolution() {
        let base = u("http://a/b/c");
        assert_eq!(resolve("../g", &base).unwrap().path, "/g");
        assert_eq!(resolve("./", &base).unwrap().path, "/b/");
        assert_eq!(resolve("../../../g", &base).unwrap().path, "/g"); // clamps
        assert_eq!(resolve("/../x", &base).unwrap().path, "/x");
        assert_eq!(resolve("g/./h", &base).unwrap().path, "/b/g/h");
        assert_eq!(resolve("g/../h", &base).unwrap().path, "/b/h");
    }

    #[test]
    fn query_only_and_fragment_only_resolution() {
        let base = u("http://a/b/c?old=1");
        let q = resolve("?new=2", &base).unwrap();
        assert_eq!(q.path, "/b/c");
        assert_eq!(q.query.as_deref(), Some("new=2"));
        let f = resolve("#anchor", &base).unwrap();
        assert_eq!(f.path, "/b/c");
        assert_eq!(f.query.as_deref(), Some("old=1"));
        assert_eq!(f.fragment.as_deref(), Some("anchor"));
    }

    #[test]
    fn empty_reference_resolves_to_the_base_itself() {
        let base = u("http://a/b/c?x#y");
        let r = resolve("", &base).unwrap();
        assert_eq!(r.path, "/b/c");
        assert_eq!(r.query.as_deref(), Some("x"));
        assert_eq!(r.fragment, None);
    }

    #[test]
    fn absolute_input_ignores_the_base() {
        let base = u("http://a/b");
        let r = resolve("https://z:9/x", &base).unwrap();
        assert_eq!(r.scheme, "https");
        assert_eq!(r.host, "z");
        assert_eq!(r.port, 9);
    }

    #[test]
    fn resolution_works_against_file_bases() {
        let base = u("file:///site/pages/index.html");
        let r = resolve("../style/site.css", &base).unwrap();
        assert_eq!(r.scheme, "file");
        assert_eq!(r.path, "/site/style/site.css");
    }

    #[test]
    fn display_roundtrips_a_reconstructed_url() {
        assert_eq!(u("http://example.com").to_string(), "http://example.com");
        assert_eq!(
            u("https://example.com:8443/a?b#c").to_string(),
            "https://example.com:8443/a?b#c"
        );
        // Default port is elided, explicit non-default port is kept.
        assert_eq!(u("http://example.com:80/x").to_string(), "http://example.com/x");
    }

    #[test]
    fn normalize_path_table() {
        assert_eq!(normalize_path("/a/b/../c"), "/a/c");
        assert_eq!(normalize_path("/a/./b/"), "/a/b/");
        assert_eq!(normalize_path("/.."), "/");
        assert_eq!(normalize_path("/a/.."), "/");
        assert_eq!(normalize_path("/a//b"), "/a//b"); // empty segments kept
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn rejects_malformed_inputs_without_panicking() {
        // Payload-bearing variants need `matches!`; unit variants can use
        // plain equality.
        let e = |s: &str| err(s);
        assert!(matches!(e(""), UrlError::InvalidUrl(_)));
        assert!(matches!(e("   "), UrlError::InvalidUrl(_)));
        assert_eq!(err("http://"), UrlError::EmptyHost);
        assert_eq!(err("https:///x"), UrlError::EmptyHost);
        assert!(matches!(e("http://x:abc/"), UrlError::InvalidPort(_)));
        assert!(matches!(e("http://x:99999/"), UrlError::InvalidPort(_)));
        assert!(matches!(e("http://x:-1/"), UrlError::InvalidPort(_)));
        assert!(matches!(e("http://[::1/x"), UrlError::InvalidUrl(_)));
        assert!(matches!(e("ftp://example.com/"), UrlError::UnsupportedScheme(_)));
        assert!(matches!(e("gopher://x"), UrlError::UnsupportedScheme(_)));
        assert_eq!(err("1http://x"), UrlError::RelativeWithoutBase);
        assert_eq!(err("relative/thing"), UrlError::RelativeWithoutBase);
        assert!(matches!(e("http:x"), UrlError::InvalidUrl(_))); // missing authority
        assert!(matches!(e("http://a:b:c/"), UrlError::InvalidPort(_))); // "b:c"
    }

    #[test]
    fn surrounding_control_characters_are_trimmed() {
        let url = u("  \n http://example.com/x \r");
        assert_eq!(url.path, "/x");
    }
}
