use crate::FetchError;
use amazing_net::url::TupleOrigin;
use amazing_net::{ParsedUrl, UrlOrigin};
use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque origins carry identity, even though each serializes as `null`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin(OriginKind);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum OriginKind {
    Tuple(TupleOrigin),
    Opaque(u128),
}
impl Origin {
    pub fn opaque() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("opaque origin identity exhausted");
        Self(OriginKind::Opaque((1_u128 << 127) | u128::from(id)))
    }
    /// Restore an opaque origin whose identity was minted by the browser.
    /// The same ID denotes the same origin across process boundaries.
    pub fn opaque_with_id(id: u128) -> Self {
        Self(OriginKind::Opaque(id))
    }
    pub fn opaque_id(&self) -> Option<u128> {
        match &self.0 {
            OriginKind::Opaque(id) => Some(*id),
            OriginKind::Tuple(_) => None,
        }
    }
    pub fn serialize(&self) -> String {
        match &self.0 {
            OriginKind::Opaque(_) => "null".into(),
            OriginKind::Tuple(origin) => origin.serialize(),
        }
    }
    /// No last-two-label heuristic: registrable domains require a PSL provider.
    pub fn site(&self, domains: &impl RegistrableDomain) -> Site {
        match &self.0 {
            OriginKind::Opaque(id) => Site::Opaque(*id),
            OriginKind::Tuple(origin) => {
                let host = origin.host().serialize();
                Site::Tuple {
                    scheme: origin.scheme().into(),
                    domain: domains.registrable_domain(&host).unwrap_or(host),
                }
            }
        }
    }
}
/// Trusted provider contract: input is a normalized ASCII host; return its
/// normalized registrable domain using a pinned public suffix list, or None
/// for IP addresses / hosts with no registrable domain.
pub trait RegistrableDomain {
    fn registrable_domain(&self, host: &str) -> Option<String>;
    /// Exact public-suffix classification from the same pinned data/policy.
    /// None means unknown, NOT "not a public suffix". Domain cookies require
    /// Some(false), except an exact-host public suffix becomes host-only.
    /// Implementations cover ICANN/PRIVATE wildcard and exception rules and
    /// classify IPs as Some(false). Rechecked on retrieval after data updates.
    fn is_public_suffix(&self, _host: &str) -> Option<bool> {
        None
    }
    /// Optional IDNA validation seam for RFC6265bis canonical A-labels.
    /// The default rejects unproven A-labels; PSL data alone cannot prove them.
    fn validated_a_label(&self, _label: &str) -> bool {
        false
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Site {
    Tuple { scheme: String, domain: String },
    Opaque(u128),
}

/// Validated Fetch URL for HTTP(S), data:, and about:. Syntax, canonicalization and
/// origin projection come from amazing-net's ParsedUrl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl {
    parsed: ParsedUrl,
    origin: Origin,
}
impl HttpUrl {
    /// WHATWG parsing with UTF-8 encoding and the default deferred-IDNA seam.
    /// Non-ASCII special hosts still fail; Unicode paths and recovery forms work.
    pub fn parse(input: &str) -> Result<Self, FetchError> {
        Self::from_parsed(ParsedUrl::parse(input).map_err(|_| FetchError::InvalidUrl)?)
    }
    /// Adopt an already validated URL without reparsing or losing userinfo.
    pub fn from_parsed(parsed: ParsedUrl) -> Result<Self, FetchError> {
        if matches!(parsed.scheme(), "data" | "about") {
            return Ok(Self {
                parsed,
                origin: Origin::opaque(),
            });
        }
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(FetchError::InvalidUrl);
        }
        let UrlOrigin::Tuple(origin) = parsed.origin() else {
            return Err(FetchError::InvalidUrl);
        };
        Ok(Self {
            parsed,
            origin: Origin(OriginKind::Tuple(origin)),
        })
    }
    pub fn as_parsed(&self) -> &ParsedUrl {
        &self.parsed
    }
    pub fn resolve(&self, reference: &str) -> Result<Self, FetchError> {
        Self::from_parsed(
            self.parsed
                .resolve(reference)
                .map_err(|_| FetchError::InvalidUrl)?,
        )
    }
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
    pub fn is_secure(&self) -> bool {
        self.parsed.scheme() == "https"
    }
    pub fn includes_credentials(&self) -> bool {
        self.parsed.includes_credentials()
    }
    pub fn without_fragment(&self) -> String {
        self.parsed.serialize_without_fragment()
    }
    pub fn serialize(&self) -> String {
        self.parsed.serialize()
    }
    pub(crate) fn referrer_url(&self) -> String {
        self.parsed.serialize_referrer()
    }
    pub(crate) fn inherit_fragment(&mut self, source: &Self) {
        self.parsed = self.parsed.with_inherited_fragment(&source.parsed);
    }
}
