use crate::HttpUrl;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferrerPolicy {
    NoReferrer,
    NoReferrerWhenDowngrade,
    SameOrigin,
    Origin,
    StrictOrigin,
    OriginWhenCrossOrigin,
    StrictOriginWhenCrossOrigin,
    UnsafeUrl,
}
impl ReferrerPolicy {
    /// Last recognized token wins, including comma-separated response fields.
    pub fn parse_header(value: &str) -> Option<Self> {
        value
            .split(',')
            .filter_map(|s| match s.trim() {
                "no-referrer" => Some(Self::NoReferrer),
                "no-referrer-when-downgrade" => Some(Self::NoReferrerWhenDowngrade),
                "same-origin" => Some(Self::SameOrigin),
                "origin" => Some(Self::Origin),
                "strict-origin" => Some(Self::StrictOrigin),
                "origin-when-cross-origin" => Some(Self::OriginWhenCrossOrigin),
                "strict-origin-when-cross-origin" => Some(Self::StrictOriginWhenCrossOrigin),
                "unsafe-url" => Some(Self::UnsafeUrl),
                _ => None,
            })
            .next_back()
    }
}
pub fn determine_referrer(
    source: Option<&HttpUrl>,
    target: &HttpUrl,
    policy: ReferrerPolicy,
) -> Option<String> {
    let source = source?;
    let origin = format!("{}/", source.origin().serialize());
    let full = source.without_fragment();
    let full = if full.len() > 4096 {
        origin.clone()
    } else {
        full
    };
    let same = source.origin() == target.origin();
    // Conservative subset: all HTTPS -> HTTP is a downgrade. Trustworthy
    // localhost exceptions await the Secure Contexts / URL integration gate.
    let downgrade = source.is_secure() && !target.is_secure();
    use ReferrerPolicy::*;
    match policy {
        NoReferrer => None,
        SameOrigin => same.then_some(full),
        Origin => Some(origin),
        UnsafeUrl => Some(full),
        NoReferrerWhenDowngrade => (!downgrade).then_some(full),
        StrictOrigin => (!downgrade).then_some(origin),
        OriginWhenCrossOrigin => Some(if same { full } else { origin }),
        StrictOriginWhenCrossOrigin => {
            if same {
                Some(full)
            } else {
                (!downgrade).then_some(origin)
            }
        }
    }
}
