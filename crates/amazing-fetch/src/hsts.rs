//! RFC 6797 policy with a pinned Chromium preload snapshot.
use crate::HttpUrl;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HstsPolicy {
    pub expires: i64,
    pub include_subdomains: bool,
}

#[derive(Default)]
pub struct HstsStore {
    dynamic: BTreeMap<String, HstsPolicy>,
}

fn preload() -> &'static Vec<(String, bool)> {
    static DATA: OnceLock<Vec<(String, bool)>> = OnceLock::new();
    DATA.get_or_init(|| {
        let input = include_bytes!("../data/hsts/chromium-preload.bin");
        let mut at = 0;
        let mut result = Vec::new();
        while at < input.len() {
            let len = input[at] as usize;
            at += 1;
            let host = std::str::from_utf8(&input[at..at + len]).expect("pinned preload ASCII");
            at += len;
            let subdomains = input[at] == 1;
            at += 1;
            result.push((host.to_owned(), subdomains));
        }
        result
    })
}

fn domain_match(host: &str, policy_host: &str, include_subdomains: bool) -> bool {
    host == policy_host
        || (include_subdomains
            && host.len() > policy_host.len()
            && host.ends_with(policy_host)
            && host.as_bytes()[host.len() - policy_host.len() - 1] == b'.')
}

fn parse_header(header: &str) -> Option<(i64, bool)> {
    let mut age = None;
    let mut subdomains = false;
    for directive in header.split(';') {
        let mut parts = directive.trim().splitn(2, '=');
        let name = parts.next()?.trim();
        let value = parts.next().map(str::trim);
        if name.eq_ignore_ascii_case("max-age") {
            if age.is_some() {
                return None;
            }
            let value = value?.trim_matches('"');
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            age = Some(value.bytes().fold(0_i64, |n, b| {
                n.saturating_mul(10).saturating_add((b - b'0') as i64)
            }));
        } else if name.eq_ignore_ascii_case("includesubdomains") && value.is_none() {
            subdomains = true;
        }
    }
    age.map(|age| (age, subdomains))
}

impl HstsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, url: &HttpUrl, header: &str, now: i64) -> bool {
        if !url.is_secure() {
            return false;
        }
        let host = url.as_parsed().hostname();
        if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
            return false;
        }
        let Some((age, include_subdomains)) = parse_header(header) else {
            return false;
        };
        if age == 0 {
            self.dynamic.remove(&host);
        } else {
            self.dynamic.insert(
                host,
                HstsPolicy {
                    expires: now.saturating_add(age),
                    include_subdomains,
                },
            );
        }
        true
    }

    pub fn is_strict(&self, host: &str, now: i64) -> bool {
        if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
            return false;
        }
        for (name, policy) in &self.dynamic {
            if policy.expires > now && domain_match(host, name, policy.include_subdomains) {
                return true;
            }
        }
        let preloaded = preload();
        if preloaded
            .binary_search_by(|(name, _)| name.as_str().cmp(host))
            .is_ok()
        {
            return true;
        }
        for (dot, _) in host.match_indices('.') {
            let ancestor = &host[dot + 1..];
            if let Ok(index) = preloaded.binary_search_by(|(name, _)| name.as_str().cmp(ancestor)) {
                if preloaded[index].1 {
                    return true;
                }
            }
        }
        false
    }

    pub fn upgrade(&self, url: &HttpUrl, now: i64) -> Option<HttpUrl> {
        if url.as_parsed().scheme() != "http" || !self.is_strict(&url.as_parsed().hostname(), now) {
            return None;
        }
        let original = url.serialize();
        let mut upgraded = format!("https{}", &original[4..]);
        // RFC 6797 maps explicit port 80 to 443; other explicit ports remain.
        if url.as_parsed().port() == Some(80) {
            upgraded = upgraded.replacen(":80/", ":443/", 1);
        }
        HttpUrl::parse(&upgraded).ok()
    }

    pub fn dynamic_entries(&self, now: i64) -> Vec<(String, HstsPolicy)> {
        self.dynamic
            .iter()
            .filter(|(_, policy)| policy.expires > now)
            .map(|(host, policy)| (host.clone(), *policy))
            .collect()
    }

    pub fn restore(&mut self, host: String, policy: HstsPolicy, now: i64) {
        if policy.expires > now
            && !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        {
            self.dynamic.insert(host, policy);
        }
    }
}
