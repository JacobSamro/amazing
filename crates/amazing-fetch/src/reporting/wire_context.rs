//! Reporting §3.5.1–2: endpoint ownership stays with the original global,
//! while the upload origin is the immutable serialized report URL's origin.
//! A null request client is represented directly, never by a default client.
use super::*;
use crate::{Destination, EmbedderPolicy, NoCorsMediaState, SecurityContext, Site};
use crate::cookies::CookieSite;

#[derive(Clone)]
pub(super) enum DeliveryContext {
    /// CSP's legacy report-uri path still uses its actual original client.
    Legacy(ClientContext),
    /// Reporting requests have no client or reserved client. The scalar clock
    /// is retained from genuine report admission, without a Window root.
    NoClient { now_unix: i64 },
}
impl DeliveryContext {
    pub(super) fn now_unix(&self) -> i64 {
        match self { Self::Legacy(client) => client.now_unix, Self::NoClient { now_unix } => *now_unix }
    }
    pub(super) fn cookie_site(&self) -> CookieSite {
        match self { Self::Legacy(client) => client.cookie_site.clone(), Self::NoClient { .. } => CookieSite::NoClient }
    }
    pub(super) fn partition_key(&self) -> Option<Site> {
        // Fetch §2.7 returns null when both client and reserved client are null.
        match self { Self::Legacy(client) => client.partition_key.clone(), Self::NoClient { .. } => None }
    }
}
/// Private native construction only after registered endpoint/settings custody.
/// About/literal globals have empty endpoint lists, so cannot reach this path.
pub(super) fn modern_request(endpoint: HttpUrl, report_url: &str, now_unix: i64)
    -> Result<(Request, DeliveryContext), ReportError>
{
    let report_url = HttpUrl::parse(report_url).map_err(|_| ReportError::InvalidReport)?;
    if !matches!(report_url.as_parsed().scheme(), "http" | "https") {
        return Err(ReportError::InvalidReport);
    }
    let mut request = Request::new(endpoint, report_url.origin().clone());
    request.security = SecurityContext {
        client_restricts_mixed_content: false,
        destination: Destination::Report,
        image_set_initiator: false,
        top_level_navigation: false,
        embedder_policy: EmbedderPolicy::UnsafeNone,
        no_cors_media_state: NoCorsMediaState::NotMedia,
    };
    request.credentials = CredentialsMode::SameOrigin;
    request.mode = RequestMode::Cors;
    request.redirect = RedirectMode::Follow;
    Ok((request, DeliveryContext::NoClient { now_unix }))
}
