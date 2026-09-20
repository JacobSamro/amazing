//! Pinned ICANN + PRIVATE Public Suffix List. No I/O or runtime updates.
//! Inputs, licence, generator and hashes live in `data/psl` and `tools`.
use crate::RegistrableDomain;
use std::sync::OnceLock;

pub const PSL_COMMIT: &str = "645b36c9a51ff0b3718df0ba02f6570aa6921632";
pub const PSL_SHA256: &str = "d3fba7b2934b0e1b9c958baf42bef85107b53cd0115ff7e81a7e61c7119a36d0";

/// Explicit opt-in provider; the default CookieStore remains fail-closed.
/// Receives canonical lowercase ASCII hosts from the URL layer. PSL lookup on
/// A-label spelling is supported but is NOT IDNA validation for cookie setting.
#[derive(Debug, Clone, Copy, Default)]
pub struct PublicSuffixList;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainBoundary<'a> {
    Ip,
    Invalid,
    PublicSuffix,
    Registrable(&'a str),
}

fn rules() -> &'static [&'static str] {
    static RULES: OnceLock<Vec<&'static str>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            include_str!("../data/psl/rules-00.txt"),
            include_str!("../data/psl/rules-01.txt"),
            include_str!("../data/psl/rules-02.txt"),
            include_str!("../data/psl/rules-03.txt"),
            include_str!("../data/psl/rules-04.txt"),
        ]
        .into_iter()
        .flat_map(str::lines)
        .collect()
    })
}

impl PublicSuffixList {
    pub fn classify<'a>(&self, host: &'a str) -> DomainBoundary<'a> {
        let ip = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        if ip.parse::<std::net::IpAddr>().is_ok() {
            return DomainBoundary::Ip;
        }
        // Preserve a terminal root dot in the returned domain, matching PSL's
        // algorithm. Cookie syntax independently rejects terminal dots.
        let name = host.strip_suffix('.').unwrap_or(host);
        if name.is_empty()
            || name.len() > 253
            || !name.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
        {
            return DomainBoundary::Invalid;
        }
        let labels: Vec<usize> = std::iter::once(0)
            .chain(name.match_indices('.').map(|(i, _)| i + 1))
            .collect();
        let mut suffix_labels = 1; // PSL's implicit * rule, including unknown TLDs.
        for (i, &start) in labels.iter().enumerate() {
            let tail = &name[start..];
            if rules().binary_search(&format!("!{tail}").as_str()).is_ok() {
                suffix_labels = labels.len() - i - 1;
                break; // exception always takes priority
            }
            if rules().binary_search(&tail).is_ok() {
                suffix_labels = suffix_labels.max(labels.len() - i);
            }
            if i > 0 && rules().binary_search(&format!("*.{tail}").as_str()).is_ok() {
                suffix_labels = suffix_labels.max(labels.len() - i + 1);
            }
        }
        if labels.len() <= suffix_labels {
            DomainBoundary::PublicSuffix
        } else {
            DomainBoundary::Registrable(&host[labels[labels.len() - suffix_labels - 1]..])
        }
    }
}
impl RegistrableDomain for PublicSuffixList {
    fn registrable_domain(&self, host: &str) -> Option<String> {
        match self.classify(host) {
            DomainBoundary::Registrable(domain) => Some(domain.into()),
            _ => None,
        }
    }
    fn is_public_suffix(&self, host: &str) -> Option<bool> {
        match self.classify(host) {
            DomainBoundary::Invalid => None,
            DomainBoundary::PublicSuffix => Some(true),
            _ => Some(false),
        }
    }
}
