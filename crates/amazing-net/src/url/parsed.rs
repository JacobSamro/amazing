//! Owned WHATWG URL record. All construction goes through the basic parser.
use super::host::{AsciiOnly, Host, IdnaProvider};
use super::parser;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingSchemeOrBase,
    Host,
    Port,
    Ipv4,
    Ipv6,
    IdnaRequired,
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "URL parse error: {self:?}")
    }
}
impl std::error::Error for ParseError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Path {
    Segments(Vec<String>),
    Opaque(String),
}
/// Immutable URL with encoded components. `None` and an empty query/fragment
/// remain distinct. Getter strings are not decoded. No unchecked constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub(super) scheme: String,
    pub(super) username: String,
    pub(super) password: String,
    pub(super) host: Option<Host>,
    pub(super) port: Option<u16>,
    pub(super) path: Path,
    pub(super) query: Option<String>,
    pub(super) fragment: Option<String>,
}
impl ParsedUrl {
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        Self::parse_with_base(input, None)
    }
    pub fn parse_with_base(input: &str, base: Option<&Self>) -> Result<Self, ParseError> {
        Self::parse_with_idna(input, base, &AsciiOnly)
    }
    pub fn parse_with_idna(
        input: &str,
        base: Option<&Self>,
        idna: &dyn IdnaProvider,
    ) -> Result<Self, ParseError> {
        parser::parse(input, base, idna)
    }
    pub fn resolve(&self, input: &str) -> Result<Self, ParseError> {
        Self::parse_with_base(input, Some(self))
    }
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn password(&self) -> &str {
        &self.password
    }
    pub fn host(&self) -> Option<&Host> {
        self.host.as_ref()
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn effective_port(&self) -> Option<u16> {
        self.port.or_else(|| default_port(&self.scheme))
    }
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }
    pub fn has_opaque_path(&self) -> bool {
        matches!(self.path, Path::Opaque(_))
    }
    pub fn includes_credentials(&self) -> bool {
        !self.username.is_empty() || !self.password.is_empty()
    }
    pub fn is_special(&self) -> bool {
        is_special(&self.scheme)
    }
    pub fn pathname(&self) -> String {
        match &self.path {
            Path::Opaque(s) => s.clone(),
            Path::Segments(p) => p.iter().map(|s| format!("/{s}")).collect(),
        }
    }
    pub fn hostname(&self) -> String {
        self.host.as_ref().map(Host::serialize).unwrap_or_default()
    }
    pub fn host_and_port(&self) -> String {
        let mut s = self.hostname();
        if let Some(p) = self.port {
            s.push_str(&format!(":{p}"));
        }
        s
    }
    pub fn serialize(&self) -> String {
        self.serialize_with_fragment(true)
    }
    pub fn serialize_without_fragment(&self) -> String {
        self.serialize_with_fragment(false)
    }
    fn serialize_with_fragment(&self, fragment: bool) -> String {
        let mut out = format!("{}:", self.scheme);
        if self.host.is_some() {
            out.push_str("//");
            if self.includes_credentials() {
                out.push_str(&self.username);
                if !self.password.is_empty() {
                    out.push(':');
                    out.push_str(&self.password);
                }
                out.push('@');
            }
            out.push_str(&self.host_and_port());
        } else if let Path::Segments(p) = &self.path {
            if p.len() > 1 && p[0].is_empty() {
                out.push_str("/.");
            }
        }
        out.push_str(&self.pathname());
        if let Some(q) = &self.query {
            out.push('?');
            out.push_str(q);
        }
        if fragment {
            if let Some(f) = &self.fragment {
                out.push('#');
                out.push_str(f);
            }
        }
        out
    }
    /// URL-origin computation, distinct from assigning identity to an opaque
    /// security origin. `Opaque` is never a same-origin identity token.
    pub fn origin(&self) -> UrlOrigin {
        if matches!(
            self.scheme.as_str(),
            "ftp" | "http" | "https" | "ws" | "wss"
        ) {
            return UrlOrigin::Tuple(TupleOrigin {
                scheme: self.scheme.clone(),
                host: self.host.clone().expect("special host"),
                port: self.port,
            });
        }
        if self.scheme == "blob" {
            if let Ok(inner) = Self::parse(&self.pathname()) {
                if matches!(inner.scheme(), "http" | "https") {
                    return inner.origin();
                }
            }
        }
        UrlOrigin::Opaque
    }
}
impl std::fmt::Display for ParsedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.serialize())
    }
}
/// `Opaque` means the embedding must allocate a fresh opaque security origin.
/// Deliberately no Eq/PartialEq: serialized `null` is not origin identity.
#[derive(Debug, Clone)]
pub enum UrlOrigin {
    Tuple(TupleOrigin),
    Opaque,
}
impl UrlOrigin {
    pub fn serialize(&self) -> String {
        match self {
            Self::Tuple(t) => t.serialize(),
            Self::Opaque => "null".into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TupleOrigin {
    scheme: String,
    host: Host,
    port: Option<u16>,
}
impl TupleOrigin {
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    pub fn host(&self) -> &Host {
        &self.host
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn effective_port(&self) -> Option<u16> {
        self.port.or_else(|| default_port(&self.scheme))
    }
    pub fn serialize(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme,
            self.host.serialize(),
            self.port.map(|p| format!(":{p}")).unwrap_or_default()
        )
    }
}
pub fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}
pub(super) fn is_special(scheme: &str) -> bool {
    scheme == "file" || default_port(scheme).is_some()
}
