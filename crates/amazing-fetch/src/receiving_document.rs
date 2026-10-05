//! Explicit native receiving-document client capture for HTML/driver adapters.
//! A final network URL cannot recover a document identity, origin or context.
use crate::{ClientContext, CspDestination, HttpUrl, Origin, ReferrerPolicy, Request};
use amazing_net::ParsedUrl;

/// `Identity` is supplied by the owning HTML/DOM lane: its actual Document and
/// generation/Window realm capability. Net defines no guessed numeric identity.
/// No Default implementation supplies an empty origin/context as authority.
#[derive(Clone)]
pub struct ReceivingDocumentClient<Identity> {
    pub identity: Identity,
    pub creation_url: ParsedUrl,
    pub document_url: ParsedUrl,
    /// Actual document base URL, which may differ from response/creation URL.
    pub base_url: ParsedUrl,
    /// Actual native origin, including inherited or opaque identity for about.
    pub origin: Origin,
    /// Policy-container self-origin may be inherited by an opaque global.
    pub csp_self_origin: Origin,
    pub context: ClientContext,
    pub referrer_policy: ReferrerPolicy,
}
/// The implementation must retrieve the actual native receiving document and
/// reject stale custody. No default URL lookup or empty-client fallback exists.
/// Capturing this value does not admit a response, insert Meta, or install policy.
pub trait ReceivingDocumentClientFactory<Identity> {
    type Error;
    fn receiving_document_client(
        &self,
        actual_receiving_document: &Identity,
    ) -> Result<ReceivingDocumentClient<Identity>, Self::Error>;
}

impl<Identity> ReceivingDocumentClient<Identity> {
    /// Build ordinary Fetch policy from this captured native client. Target URL
    /// selects only the target; it supplies no document origin or CSP self-origin.
    pub fn policy_request(&self, target: HttpUrl, destination: CspDestination) -> Request {
        let mut request = Request::new(target, self.origin.clone());
        request.csp_self_origin = self.csp_self_origin.clone();
        request.csp = self.context.csp.clone();
        request.csp_destination = destination;
        request.security = self.context.security;
        request.referrer_policy = self.referrer_policy;
        request.referrer = HttpUrl::from_parsed(self.document_url.clone())
            .ok()
            .filter(|url| matches!(url.as_parsed().scheme(), "http" | "https"));
        request
    }
}
