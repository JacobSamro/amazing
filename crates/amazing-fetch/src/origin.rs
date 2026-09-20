use crate::FetchError;
use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque origins carry identity, even though each serializes as `null`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin(OriginKind);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum OriginKind {
    Tuple {
        scheme: String,
        host: String,
        port: u16,
    },
    Opaque(u64),
}
impl Origin {
    pub fn opaque() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("opaque origin identity exhausted");
        Self(OriginKind::Opaque(id))
    }
    pub fn serialize(&self) -> String {
        match &self.0 {
            OriginKind::Opaque(_) => "null".into(),
            OriginKind::Tuple { scheme, host, port } => {
                let suffix =
                    if (*port == 443 && scheme == "https") || (*port == 80 && scheme == "http") {
                        String::new()
                    } else {
                        format!(":{port}")
                    };
                format!("{scheme}://{host}{suffix}")
            }
        }
    }
    /// No last-two-label heuristic: registrable domains require a PSL provider.
    pub fn site(&self, domains: &impl RegistrableDomain) -> Site {
        match &self.0 {
            OriginKind::Opaque(id) => Site::Opaque(*id),
            OriginKind::Tuple { scheme, host, .. } => Site::Tuple {
                scheme: scheme.clone(),
                domain: domains
                    .registrable_domain(host)
                    .unwrap_or_else(|| host.clone()),
            },
        }
    }
}
/// Trusted provider contract: input is a normalized ASCII host; return its
/// normalized registrable domain using a pinned public suffix list, or None
/// for IP addresses / hosts with no registrable domain.
pub trait RegistrableDomain {
    fn registrable_domain(&self, host: &str) -> Option<String>;
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Site {
    Tuple { scheme: String, domain: String },
    Opaque(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl {
    origin: Origin,
    path_query: String,
    fragment: Option<String>,
}
impl HttpUrl {
    /// Strict canonical ASCII subset. Rejects userinfo, backslashes, controls,
    /// non-ASCII, legacy IPv4 spellings, and percent-encoded hosts.
    pub fn parse(input: &str) -> Result<Self, FetchError> {
        if !input.is_ascii() || input.bytes().any(|b| b <= 32 || b == 127 || b == b'\\') {
            return Err(FetchError::InvalidUrl);
        }
        let (scheme, rest) = input.split_once("://").ok_or(FetchError::InvalidUrl)?;
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https") {
            return Err(FetchError::InvalidUrl);
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        if authority.contains('@') {
            return Err(FetchError::InvalidUrl);
        }
        let (host, port) = if authority.starts_with('[') {
            let end = authority.find(']').ok_or(FetchError::InvalidUrl)?;
            let ip: std::net::Ipv6Addr = authority[1..end]
                .parse()
                .map_err(|_| FetchError::InvalidUrl)?;
            let tail = &authority[end + 1..];
            (
                format!("[{ip}]"),
                if tail.is_empty() {
                    None
                } else {
                    Some(tail.strip_prefix(':').ok_or(FetchError::InvalidUrl)?)
                },
            )
        } else {
            let (host, port) = authority
                .split_once(':')
                .map_or((authority, None), |(h, p)| (h, Some(p)));
            if host.is_empty()
                || host.split('.').any(|label| {
                    label.is_empty()
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return Err(FetchError::InvalidUrl);
            }
            let lower = host.to_ascii_lowercase();
            let last = lower.rsplit('.').next().unwrap();
            if last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x") {
                let ip: std::net::Ipv4Addr = host.parse().map_err(|_| FetchError::InvalidUrl)?;
                if ip.to_string() != host {
                    return Err(FetchError::InvalidUrl);
                }
            }
            (host.to_ascii_lowercase(), port)
        };
        let port = match port {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                p.parse().map_err(|_| FetchError::InvalidUrl)?
            }
            Some(_) => return Err(FetchError::InvalidUrl),
            None => {
                if scheme == "https" {
                    443
                } else {
                    80
                }
            }
        };
        let (path, fragment) = rest[end..]
            .split_once('#')
            .map_or((&rest[end..], None), |(p, f)| (p, Some(f.to_owned())));
        let path_query = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        Ok(Self {
            origin: Origin(OriginKind::Tuple { scheme, host, port }),
            path_query,
            fragment,
        })
    }
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
    pub fn is_secure(&self) -> bool {
        matches!(&self.origin.0, OriginKind::Tuple { scheme, .. } if scheme == "https")
    }
    pub fn without_fragment(&self) -> String {
        format!("{}{}", self.origin.serialize(), self.path_query)
    }
    pub fn serialize(&self) -> String {
        let mut s = self.without_fragment();
        if let Some(f) = &self.fragment {
            s.push('#');
            s.push_str(f);
        }
        s
    }
    /// Redirect targets must already be resolved by the URL layer. Inherit
    /// source fragment only if Location did not contain one (including empty).
    pub(crate) fn inherit_fragment(&mut self, source: &Self) {
        if self.fragment.is_none() {
            self.fragment = source.fragment.clone();
        }
    }
}
