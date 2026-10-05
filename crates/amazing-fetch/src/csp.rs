//! CSP3 fetch directives, pinned to w3c/webappsec-csp b1bad631b091cdca904138d6947a16b89edfd2b4.
//! The policy container and element metadata are supplied by the embedder.
use crate::{HeaderList, HttpUrl, Origin, Request};
use amazing_net::ParsedUrl;
mod inline;
mod request_report;
pub use request_report::*;
mod meta_utf16;
mod report_snapshot;
pub use inline::*;
pub use report_snapshot::*;

const MAX_POLICY_BYTES: usize = 65_536;
const MAX_DIRECTIVES: usize = 256;
const MAX_POLICIES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspDisposition {
    Enforce,
    ReportOnly,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspViolation {
    pub directive: String,
    pub disposition: CspDisposition,
    pub blocked_url: String,
    // Native immutable custody; public legacy diagnostics do not authorize
    // event/report enrichment or identify a policy by fallback name.
    policy_index: usize,
    effective_directive: String,
    source_directive: String,
    phase: CspRequestPhase,
    policy_snapshot: CspPolicy,
    request_urls: Vec<HttpUrl>,
    resource_url: ParsedUrl,
    request_origin: Origin,
    csp_self_origin: Origin,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspCheck {
    pub blocked: bool,
    pub violations: Vec<CspViolation>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CspParseError {
    TooLong,
    TooManyDirectives,
    TooManyPolicies,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspDirective {
    pub name: String,
    pub sources: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CspPolicySource {
    Header,
    Meta,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspPolicy {
    pub source: CspPolicySource,
    /// Existing decoded/trimmed String serialization retained for compatibility.
    /// Meta DOMString event/report authority is original_policy_units, not this diagnostic.
    pub original_policy: String,
    /// Authoritative DOMString for original-policy event/report fields. Header
    /// policies retain the existing decoded/trimmed header serialization; Meta
    /// policies retain every native content attribute code unit without loss.
    pub original_policy_units: Vec<u16>,
    pub disposition: CspDisposition,
    pub directives: Vec<CspDirective>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CspPolicyList {
    pub policies: Vec<CspPolicy>,
}

impl CspPolicyList {
    /// Delivery provenance survives cloning/inheritance. Response parsing drops
    /// empty directive sets (CSP3 2.2.2); meta policies never count for hiding.
    pub fn contains_header_delivered_policy(&self) -> bool {
        self.policies
            .iter()
            .any(|p| p.source == CspPolicySource::Header)
    }
    /// A comma separates serialized policies; a semicolon separates directives.
    /// Limits are checked on borrowed header values before constructing strings.
    pub fn from_headers(headers: &HeaderList) -> Result<Self, CspParseError> {
        let mut bytes = 0usize;
        let mut directives = 0usize;
        let mut policies = 0usize;
        for (name, value) in headers.iter() {
            if !is_policy_header(name) {
                continue;
            }
            bytes = bytes
                .checked_add(value.len())
                .ok_or(CspParseError::TooLong)?;
            if bytes > MAX_POLICY_BYTES {
                return Err(CspParseError::TooLong);
            }
            policies = policies
                .checked_add(value.split(',').count())
                .ok_or(CspParseError::TooManyPolicies)?;
            directives = directives
                .checked_add(value.matches(';').count() + value.split(',').count())
                .ok_or(CspParseError::TooManyDirectives)?;
            if policies > MAX_POLICIES {
                return Err(CspParseError::TooManyPolicies);
            }
            if directives > MAX_DIRECTIVES {
                return Err(CspParseError::TooManyDirectives);
            }
        }
        let mut list = Self::default();
        // CSP3 2.2.2 collects enforcing headers first, then report-only headers,
        // regardless of their ordering in the HTTP header list.
        for (header_name, disposition) in [
            ("content-security-policy", CspDisposition::Enforce),
            (
                "content-security-policy-report-only",
                CspDisposition::ReportOnly,
            ),
        ] {
            for (_, value) in headers.iter().filter(|(name, _)| *name == header_name) {
                for serialized in value.split(',') {
                    let mut policy = CspPolicy {
                        source: CspPolicySource::Header,
                        original_policy: serialized.trim_matches(is_ascii_whitespace).to_owned(),
                        original_policy_units: serialized
                            .trim_matches(is_ascii_whitespace)
                            .encode_utf16()
                            .collect(),
                        disposition,
                        directives: Vec::new(),
                    };
                    for part in serialized.split(';') {
                        if !part.is_ascii() {
                            continue;
                        }
                        let mut tokens = part.split(is_ascii_whitespace).filter(|s| !s.is_empty());
                        let Some(name) = tokens.next() else {
                            continue;
                        };
                        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                            continue;
                        }
                        let name = name.to_ascii_lowercase();
                        if policy.directives.iter().any(|d| d.name == name) {
                            continue;
                        }
                        policy.directives.push(CspDirective {
                            name,
                            sources: tokens
                                .filter(|s| valid_source(s))
                                .map(str::to_owned)
                                .collect(),
                        });
                    }
                    if !policy.directives.is_empty() {
                        list.policies.push(policy);
                    }
                }
            }
        }
        Ok(list)
    }
    /// Parse an enforcing meta-delivered policy. The HTML owner must call this
    /// only for a valid, connected CSP meta element in the document head; policies
    /// apply prospectively. Report-only meta delivery is not supported by CSP.
    pub fn from_meta(serialized: &str) -> Result<Self, CspParseError> {
        // Preserve the legacy byte limit before any UTF16 allocation.
        if serialized.len() > MAX_POLICY_BYTES {
            return Err(CspParseError::TooLong);
        }
        Self::from_meta_utf16(&serialized.encode_utf16().collect::<Vec<_>>())
    }
    pub fn pre_request(&self, request: &Request) -> CspCheck {
        self.check(request, request.url(), CspRequestPhase::PreRequest)
    }
    pub fn post_request(&self, request: &Request, response_url: &HttpUrl) -> CspCheck {
        self.check(request, response_url, CspRequestPhase::PostRequest)
    }
    fn check(&self, request: &Request, url: &HttpUrl, phase: CspRequestPhase) -> CspCheck {
        self.check_parsed(request, url.as_parsed(), phase)
    }
    /// Apply connect-src to the original ws/wss URL, preserving scheme sources.
    pub fn pre_websocket(&self, request: &Request, url: &ParsedUrl) -> CspCheck {
        self.check_parsed(request, url, CspRequestPhase::WebSocketPreRequest)
    }
    fn check_parsed(&self, request: &Request, url: &ParsedUrl, phase: CspRequestPhase) -> CspCheck {
        let mut result = CspCheck {
            blocked: false,
            violations: Vec::new(),
        };
        let destination = if request.csp_destination == CspDestination::None {
            match request.security.destination {
                crate::Destination::Script => CspDestination::ScriptElement,
                crate::Destination::Worker
                | crate::Destination::SharedWorker
                | crate::Destination::ServiceWorker => CspDestination::Worker,
                crate::Destination::Style => CspDestination::StyleElement,
                crate::Destination::Image => CspDestination::Image,
                crate::Destination::Audio | crate::Destination::Video => CspDestination::Media,
                crate::Destination::Empty => CspDestination::Connect,
                _ => CspDestination::None,
            }
        } else {
            request.csp_destination
        };
        let Some(effective) = destination.directive() else {
            return result;
        };
        let redirected = request.url_list().len() > 1;
        for (policy_index, policy) in self.policies.iter().enumerate() {
            let Some(directive) = fallback(effective)
                .iter()
                .find_map(|name| policy.directives.iter().find(|d| d.name == *name))
            else {
                continue;
            };
            if source_list_matches(&directive.sources, request, destination, url, redirected) {
                continue;
            }
            result.blocked |= policy.disposition == CspDisposition::Enforce;
            result.violations.push(CspViolation {
                directive: directive.name.clone(),
                disposition: policy.disposition,
                blocked_url: url.serialize_without_fragment(),
                policy_index,
                effective_directive: effective.into(),
                source_directive: directive.name.clone(),
                phase,
                policy_snapshot: policy.clone(),
                request_urls: request.url_list().to_vec(),
                resource_url: if phase == CspRequestPhase::WebSocketPreRequest {
                    url.clone()
                } else {
                    request.url_list()[0].as_parsed().clone()
                },
                request_origin: request.origin.clone(),
                csp_self_origin: request.csp_self_origin.clone(),
            });
        }
        result
    }
}
fn is_policy_header(name: &str) -> bool {
    matches!(
        name,
        "content-security-policy" | "content-security-policy-report-only"
    )
}
fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' ')
}
fn valid_source(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && b != b';' && b != b',')
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CspDestination {
    #[default]
    None,
    Script,
    ScriptElement,
    Style,
    StyleElement,
    Image,
    Connect,
    Font,
    Media,
    Object,
    Frame,
    Child,
    Worker,
    Manifest,
}
impl CspDestination {
    fn directive(self) -> Option<&'static str> {
        Some(match self {
            Self::None => return None,
            Self::Script => "script-src",
            Self::ScriptElement => "script-src-elem",
            Self::Style => "style-src",
            Self::StyleElement => "style-src-elem",
            Self::Image => "img-src",
            Self::Connect => "connect-src",
            Self::Font => "font-src",
            Self::Media => "media-src",
            Self::Object => "object-src",
            Self::Frame => "frame-src",
            Self::Child => "child-src",
            Self::Worker => "worker-src",
            Self::Manifest => "manifest-src",
        })
    }
}
fn fallback(name: &str) -> &'static [&'static str] {
    match name {
        "script-src-elem" => &["script-src-elem", "script-src", "default-src"],
        "style-src-elem" => &["style-src-elem", "style-src", "default-src"],
        "worker-src" => &["worker-src", "child-src", "script-src", "default-src"],
        "frame-src" => &["frame-src", "child-src", "default-src"],
        "script-src" => &["script-src", "default-src"],
        "style-src" => &["style-src", "default-src"],
        "child-src" => &["child-src", "default-src"],
        "img-src" => &["img-src", "default-src"],
        "connect-src" => &["connect-src", "default-src"],
        "font-src" => &["font-src", "default-src"],
        "media-src" => &["media-src", "default-src"],
        "object-src" => &["object-src", "default-src"],
        "manifest-src" => &["manifest-src", "default-src"],
        _ => &[],
    }
}
fn source_list_matches(
    sources: &[String],
    request: &Request,
    destination: CspDestination,
    url: &ParsedUrl,
    redirected: bool,
) -> bool {
    let script = matches!(
        destination,
        CspDestination::Script | CspDestination::ScriptElement | CspDestination::Worker
    );
    if script {
        if let Some(nonce) = request.csp_nonce.as_deref() {
            if !nonce.is_empty()
                && sources.iter().any(|s| {
                    s.strip_prefix("'nonce-").and_then(|s| s.strip_suffix('\'')) == Some(nonce)
                })
            {
                return true;
            }
        }
        if !request.csp_integrity.is_empty()
            && request
                .csp_integrity
                .iter()
                .all(|hash| sources.iter().any(|s| s == hash))
        {
            return true;
        }
        if sources
            .iter()
            .any(|s| s.eq_ignore_ascii_case("'strict-dynamic'"))
        {
            return !request.csp_parser_inserted;
        }
    }
    sources
        .iter()
        .any(|s| expression_matches(s, url, &request.csp_self_origin, redirected))
}
fn scheme_matches(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
        || matches!(
            (a, b),
            ("http", "https" | "ws" | "wss")
                | ("https", "wss")
                | ("ws", "wss" | "http" | "https")
                | ("wss", "https")
        )
}
/// Match a fetch source against any parsed URL, including ws/wss URLs used by
/// the embedding's WebSocket path.
pub fn csp_source_matches(
    source: &str,
    url: &ParsedUrl,
    origin: &Origin,
    redirected: bool,
) -> bool {
    expression_matches(source, url, origin, redirected)
}
fn expression_matches(source: &str, parsed: &ParsedUrl, origin: &Origin, redirected: bool) -> bool {
    if source == "*" {
        return matches!(parsed.scheme(), "http" | "https" | "ws" | "wss");
    }
    if source.eq_ignore_ascii_case("'self'") {
        let Ok(self_url) = HttpUrl::parse(&origin.serialize()) else {
            return false;
        };
        let own = self_url.as_parsed();
        if own.scheme() == parsed.scheme()
            && own.hostname() == parsed.hostname()
            && own.effective_port() == parsed.effective_port()
        {
            return true;
        }
        return own.hostname() == parsed.hostname()
            && (own.effective_port() == parsed.effective_port()
                || (own.port().is_none() && parsed.port().is_none()))
            && scheme_matches(own.scheme(), parsed.scheme());
    }
    if let Some(scheme) = source.strip_suffix(':') {
        if !scheme.contains('/') && scheme.bytes().all(|b| b.is_ascii_alphabetic()) {
            return scheme_matches(scheme, parsed.scheme());
        }
    }
    if source.starts_with('\'') {
        return false;
    }
    let (scheme, rest) = if let Some((scheme, rest)) = source.split_once("://") {
        (scheme, rest)
    } else {
        ("", source)
    };
    if !scheme.is_empty() {
        if !scheme_matches(scheme, parsed.scheme()) {
            return false;
        }
    } else {
        let Ok(self_url) = HttpUrl::parse(&origin.serialize()) else {
            return false;
        };
        if !scheme_matches(self_url.as_parsed().scheme(), parsed.scheme()) {
            return false;
        }
    }
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let (host, port) = if authority.starts_with('[') {
        match authority.find(']') {
            Some(i) => (&authority[..=i], authority[i + 1..].strip_prefix(':')),
            None => return false,
        }
    } else {
        authority
            .rsplit_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)))
    };
    let target_host = parsed.hostname();
    let host_ok = host == "*"
        || host.strip_prefix("*.").is_some_and(|suffix| {
            target_host.ends_with(&format!(".{}", suffix.to_ascii_lowercase()))
        })
        || host.eq_ignore_ascii_case(&target_host);
    if !host_ok {
        return false;
    }
    if let Some(port) = port {
        if port != "*" && port.parse::<u16>().ok() != parsed.effective_port() {
            return false;
        }
    } else if parsed.port().is_some() {
        return false;
    }
    if !path.is_empty() && !redirected {
        let expected = format!("/{path}");
        let actual = parsed.pathname();
        if expected.ends_with('/') {
            if !actual.starts_with(&expected) {
                return false;
            }
        } else if actual != expected {
            return false;
        }
    }
    true
}
