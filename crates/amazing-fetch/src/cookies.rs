//! In-memory RFC6265bis-22 cookie algorithms. No persistence or live dispatcher.
//! Time is caller-supplied UTC seconds; the embedding supplies audited browsing
//! context (including ancestors, redirects and reload rules), not merely the
//! address-bar URL. Cookie octets remain bytes, never percent-decoded or UTF-8 replaced.
pub(crate) mod date;
mod parse;
mod snapshot;
use crate::{HttpUrl, Origin, RegistrableDomain, Site};

pub const MAX_LIFETIME: i64 = 400 * 86400;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Default,
    Strict,
    Lax,
    None,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookieError {
    Empty,
    ControlCharacter,
    TooLarge,
    InvalidHost,
    Domain,
    UnknownPublicSuffix,
    Secure,
    HttpOnly,
    SameSite,
    Prefix,
    Partition,
    Snapshot,
    Limits,
    HeaderTooLarge,
}
impl std::fmt::Display for CookieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cookie rejected: {self:?}")
    }
}
impl std::error::Error for CookieError {}
/// Explicitly distinguishes no client (same-site) from an opaque/cross-site client.
#[derive(Debug, Clone)]
pub enum CookieSite {
    NoClient,
    Origin(Origin),
    CrossSite,
}
#[derive(Debug, Clone)]
pub struct CookieAccess<'a> {
    pub url: &'a HttpUrl,
    pub site_for_cookies: CookieSite,
    pub http_api: bool,
    pub top_level_navigation: bool,
    /// Current hop's method, after any redirect rewrite. Safe methods are case-sensitive.
    pub method: &'a str,
    /// Schemeful site of the top-level document, minted by the browser.
    pub partition_key: Option<Site>,
}
impl CookieAccess<'_> {
    fn same_site(&self, domains: &impl RegistrableDomain) -> bool {
        match &self.site_for_cookies {
            CookieSite::NoClient => true,
            CookieSite::CrossSite => false,
            CookieSite::Origin(origin) => origin.site(domains) == self.url.origin().site(domains),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct CookieLimits {
    pub total: usize,
    pub per_domain: usize,
    pub header_bytes: usize,
    /// Enable the optional 120-second grace for Default cookies only.
    pub lax_allowing_unsafe: bool,
}
impl Default for CookieLimits {
    fn default() -> Self {
        Self {
            total: 3000,
            per_domain: 180,
            header_bytes: 65536,
            lax_allowing_unsafe: true,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Cookie {
    name: Vec<u8>,
    value: Vec<u8>,
    domain: String,
    path: Vec<u8>,
    expires: Option<i64>,
    created: i64,
    accessed: i64,
    host_only: bool,
    secure: bool,
    http_only: bool,
    same_site: SameSite,
    partition_key: Option<Site>,
}
impl Cookie {
    pub fn name(&self) -> &[u8] {
        &self.name
    }
    pub fn value(&self) -> &[u8] {
        &self.value
    }
    pub fn domain(&self) -> &str {
        &self.domain
    }
    pub fn path(&self) -> &[u8] {
        &self.path
    }
    pub fn expires(&self) -> Option<i64> {
        self.expires
    }
    pub fn creation_time(&self) -> i64 {
        self.created
    }
    pub fn last_access_time(&self) -> i64 {
        self.accessed
    }
    pub fn host_only(&self) -> bool {
        self.host_only
    }
    pub fn secure(&self) -> bool {
        self.secure
    }
    pub fn http_only(&self) -> bool {
        self.http_only
    }
    pub fn same_site(&self) -> SameSite {
        self.same_site
    }
    pub fn partition_key(&self) -> Option<&Site> {
        self.partition_key.as_ref()
    }
    pub fn persistent(&self) -> bool {
        self.expires.is_some()
    }
}
/// No bundled suffix data: all nonempty Domain attributes fail closed.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConservativeDomains;
impl RegistrableDomain for ConservativeDomains {
    fn registrable_domain(&self, _: &str) -> Option<String> {
        None
    }
}
/// The profile owns this store and its clock. Persistence/import are intentionally
/// absent until the shell's on-disk migration is designed.
pub struct CookieStore<P = ConservativeDomains> {
    domains: P,
    limits: CookieLimits,
    cookies: Vec<Cookie>,
}
impl Default for CookieStore<ConservativeDomains> {
    fn default() -> Self {
        Self::new(ConservativeDomains, CookieLimits::default()).unwrap()
    }
}
impl<P: RegistrableDomain> CookieStore<P> {
    pub fn new(domains: P, limits: CookieLimits) -> Result<Self, CookieError> {
        if limits.total == 0
            || limits.total > 3000
            || limits.per_domain == 0
            || limits.per_domain > limits.total
            || limits.header_bytes > 16 * 1024 * 1024
        {
            return Err(CookieError::Limits);
        }
        Ok(Self {
            domains,
            limits,
            cookies: Vec::new(),
        })
    }
    pub fn cookies(&mut self, now: i64) -> &[Cookie] {
        self.expire(now);
        &self.cookies
    }
    pub fn end_session(&mut self, now: i64) {
        self.expire(now);
        self.cookies.retain(Cookie::persistent);
    }
    pub fn expire(&mut self, now: i64) {
        self.cookies.retain(|c| c.expires.is_none_or(|t| t > now));
    }
    pub fn set_cookie(
        &mut self,
        input: &[u8],
        context: &CookieAccess<'_>,
        now: i64,
    ) -> Result<(), CookieError> {
        self.expire(now);
        let host = context.url.as_parsed().hostname();
        if !valid_host(&host, &self.domains) {
            return Err(CookieError::InvalidHost);
        }
        let path = context.url.as_parsed().pathname();
        let default = default_path(path.as_bytes());
        // Conservative storage policy: avoid copying an unbounded URL-derived
        // default path, including through an invalid Path attribute fallback.
        if default.len() > 1024 {
            return Err(CookieError::TooLarge);
        }
        let parsed = parse::parse(input, now, default)?;
        let mut domain = parsed.domain.as_deref().unwrap_or_default();
        if !domain.is_ascii() {
            return Err(CookieError::Domain);
        }
        if !domain.is_empty() {
            let candidate = std::str::from_utf8(domain).map_err(|_| CookieError::Domain)?;
            if !valid_host(candidate, &self.domains) || !domain_matches(&host, candidate) {
                return Err(CookieError::Domain);
            }
            match self.domains.is_public_suffix(candidate) {
                Some(true) if candidate == host => domain = b"",
                Some(true) => return Err(CookieError::Domain),
                None => return Err(CookieError::UnknownPublicSuffix),
                Some(false) => (),
            }
        }
        let host_only = domain.is_empty();
        let domain = if host_only {
            host
        } else {
            std::str::from_utf8(domain).unwrap().to_owned()
        };
        let cookie = Cookie {
            name: parsed.name,
            value: parsed.value,
            domain,
            path: parsed.path.clone().unwrap_or_else(|| default.to_vec()),
            expires: parsed.expires,
            created: now,
            accessed: now,
            host_only,
            secure: parsed.secure,
            http_only: parsed.http_only,
            same_site: parsed.same_site,
            partition_key: if parsed.partitioned {
                Some(context.partition_key.clone().ok_or(CookieError::Partition)?)
            } else {
                None
            },
        };
        if parsed.partitioned && (!cookie.secure || !context.url.is_secure()) {
            return Err(CookieError::Partition);
        }
        if cookie.secure && !context.url.is_secure() {
            return Err(CookieError::Secure);
        }
        if cookie.http_only && !context.http_api {
            return Err(CookieError::HttpOnly);
        }
        if !cookie.secure
            && !context.url.is_secure()
            && self.cookies.iter().any(|old| {
                old.secure
                    && old.name == cookie.name
                    && (domain_matches(&old.domain, &cookie.domain)
                        || domain_matches(&cookie.domain, &old.domain))
                    && path_matches(&cookie.path, &old.path)
            })
        {
            return Err(CookieError::Secure);
        }
        if cookie.same_site != SameSite::None
            && !context.same_site(&self.domains)
            && (!context.http_api || !context.top_level_navigation)
        {
            return Err(CookieError::SameSite);
        }
        if cookie.same_site == SameSite::None && !cookie.secure {
            return Err(CookieError::Secure);
        }
        if starts(&cookie.name, b"__Secure-") && !cookie.secure {
            return Err(CookieError::Prefix);
        }
        if starts(&cookie.name, b"__Host-")
            && (!cookie.secure || !cookie.host_only || parsed.path.is_none() || cookie.path != b"/")
        {
            return Err(CookieError::Prefix);
        }
        if cookie.name.is_empty()
            && (starts(&cookie.value, b"__Secure-") || starts(&cookie.value, b"__Host-"))
        {
            return Err(CookieError::Prefix);
        }
        let existing = self.cookies.iter().position(|old| {
            old.name == cookie.name
                && old.domain == cookie.domain
                && old.host_only == cookie.host_only
                && old.path == cookie.path
                && old.partition_key == cookie.partition_key
        });
        let mut cookie = cookie;
        if let Some(index) = existing {
            if !context.http_api && self.cookies[index].http_only {
                return Err(CookieError::HttpOnly);
            }
            cookie.created = self.cookies[index].created;
            // Keep stable ordering among cookies with identical creation times.
            self.cookies[index] = cookie;
        } else {
            self.cookies.push(cookie);
        }
        self.expire(now);
        self.evict();
        Ok(())
    }
    pub fn cookie_header(
        &mut self,
        context: &CookieAccess<'_>,
        now: i64,
    ) -> Result<Vec<u8>, CookieError> {
        self.expire(now);
        let host = context.url.as_parsed().hostname();
        if !valid_host(&host, &self.domains) {
            return Err(CookieError::InvalidHost);
        }
        let path = context.url.as_parsed().pathname();
        let same = context.same_site(&self.domains);
        let mut selected: Vec<usize> = self
            .cookies
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let domain = if c.host_only {
                    c.domain == host
                } else {
                    domain_matches(&host, &c.domain)
                        && self.domains.is_public_suffix(&c.domain) == Some(false)
                };
                let safe = matches!(context.method, "GET" | "HEAD" | "OPTIONS" | "TRACE");
                let grace = self.limits.lax_allowing_unsafe
                    && c.same_site == SameSite::Default
                    && now >= c.created
                    && now.saturating_sub(c.created) <= 120;
                let lax = context.http_api
                    && context.top_level_navigation
                    && matches!(c.same_site, SameSite::Lax | SameSite::Default)
                    && (safe || grace);
                (domain
                    && path_matches(path.as_bytes(), &c.path)
                    && (!c.secure || context.url.is_secure())
                    && (!c.http_only || context.http_api)
                    && (c.partition_key.is_none() || c.partition_key == context.partition_key)
                    && (same || c.same_site == SameSite::None || lax))
                    .then_some(i)
            })
            .collect();
        selected.sort_by(|a, b| {
            self.cookies[*b]
                .path
                .len()
                .cmp(&self.cookies[*a].path.len())
                .then(self.cookies[*a].created.cmp(&self.cookies[*b].created))
        });
        let size = selected
            .iter()
            .enumerate()
            .try_fold(0usize, |total, (position, index)| {
                let c = &self.cookies[*index];
                let n = c.name.len()
                    + c.value.len()
                    + usize::from(!c.name.is_empty())
                    + if position == 0 { 0 } else { 2 };
                total
                    .checked_add(n)
                    .filter(|v| *v <= self.limits.header_bytes)
                    .ok_or(CookieError::HeaderTooLarge)
            })?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(size)
            .map_err(|_| CookieError::HeaderTooLarge)?;
        for index in selected {
            if !result.is_empty() {
                result.extend_from_slice(b"; ");
            }
            let c = &mut self.cookies[index];
            if !c.name.is_empty() {
                result.extend_from_slice(&c.name);
                result.push(b'=');
            }
            result.extend_from_slice(&c.value);
            c.accessed = now;
        }
        Ok(result)
    }
    fn evict(&mut self) {
        loop {
            let mut counts = std::collections::BTreeMap::new();
            for cookie in &self.cookies {
                *counts.entry(cookie.domain.as_str()).or_insert(0usize) += 1;
            }
            let over_domain = counts.values().any(|count| *count > self.limits.per_domain);
            if !over_domain && self.cookies.len() <= self.limits.total {
                break;
            }
            let victim = self
                .cookies
                .iter()
                .enumerate()
                .min_by_key(|(_, c)| {
                    let over = counts[c.domain.as_str()] > self.limits.per_domain;
                    (
                        if over && !c.secure {
                            0
                        } else if over {
                            1
                        } else {
                            2
                        },
                        c.accessed,
                    )
                })
                .map(|(i, _)| i)
                .unwrap();
            self.cookies.remove(victim);
        }
    }
}
fn starts(value: &[u8], prefix: &[u8]) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|v| v.eq_ignore_ascii_case(prefix))
}
fn ip(host: &str) -> bool {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok()
}
fn valid_host(host: &str, domains: &impl RegistrableDomain) -> bool {
    if ip(host) {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!label.starts_with("xn--") && label.as_bytes().get(2..4) != Some(b"--")
                    || domains.validated_a_label(label))
        })
}
/// Canonical, lowercase inputs; a suffix must begin at a label boundary and
/// IP addresses only match exactly. PSL authorization is separate.
pub fn domain_matches(host: &str, domain: &str) -> bool {
    !host.is_empty()
        && !domain.is_empty()
        && (host == domain
            || (!ip(host)
                && host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))))
}
pub fn default_path(path: &[u8]) -> &[u8] {
    if !path.starts_with(b"/") {
        return b"/";
    }
    match path.iter().rposition(|b| *b == b'/') {
        Some(i) if i > 0 => &path[..i],
        _ => b"/",
    }
}
pub fn path_matches(request: &[u8], cookie: &[u8]) -> bool {
    request == cookie
        || request
            .strip_prefix(cookie)
            .is_some_and(|rest| cookie.ends_with(b"/") || rest.first() == Some(&b'/'))
}
