use crate::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestMode {
    SameOrigin,
    Cors,
    NoCors,
    Navigate,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialsMode {
    Omit,
    SameOrigin,
    Include,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectMode {
    Follow,
    Error,
    Manual,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseType {
    Basic,
    Cors,
    Opaque,
    OpaqueRedirect,
    Error,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Bytes(Vec<u8>),
    Unreplayable,
}
/// Identity is minted only by the native navigation constructor; it holds no
/// Window roots or author-controlled values and is never serialized as trust.
#[derive(Debug)]
pub(crate) struct NativeNavigationToken {
    pub(crate) source_reporting_claimed: std::sync::atomic::AtomicBool,
    pub(crate) source_reporting_admitted: std::sync::atomic::AtomicBool,
}
impl NativeNavigationToken {
    pub(crate) fn new() -> Self {Self {source_reporting_claimed:std::sync::atomic::AtomicBool::new(false),source_reporting_admitted:std::sync::atomic::AtomicBool::new(false)}}
}
impl PartialEq for NativeNavigationToken {
    fn eq(&self,other:&Self)->bool { std::ptr::eq(self,other) }
}
impl Eq for NativeNavigationToken {}
#[derive(Debug, Clone)]
pub struct Request {
    pub(crate) url_list: Vec<HttpUrl>,
    pub(crate) native_navigation_token: Option<std::sync::Arc<NativeNavigationToken>>,
    pub(crate) native_navigation_report_token: Option<std::sync::Arc<NativeNavigationToken>>,
    pub(crate) native_script_report_token: Option<std::sync::Arc<crate::reporting::NativeScriptCspBinding>>,
    pub origin: Origin,
    method: String,
    pub headers: HeaderList,
    body: Option<Body>,
    pub mode: RequestMode,
    pub credentials: CredentialsMode,
    pub redirect: RedirectMode,
    pub referrer: Option<HttpUrl>,
    pub referrer_policy: ReferrerPolicy,
    pub use_cors_preflight: bool,
    /// Supplied by the embedding; this crate does not derive DOM secure contexts.
    pub security: SecurityContext,
    /// Policy container and element metadata supplied by the embedding.
    pub csp: CspPolicyList,
    pub csp_self_origin: Origin,
    pub csp_destination: CspDestination,
    pub csp_nonce: Option<String>,
    pub csp_integrity: Vec<String>,
    pub csp_parser_inserted: bool,
    pub csp_violations: Vec<CspViolation>,
    /// Minted only by the native child-navigation constructor.
    pub(crate) nested_navigation: bool,
    pub(crate) embedding_owner: Option<std::sync::Arc<crate::reporting::NavigationEmbeddingOwnerSnapshot>>,
    pub(crate) csp_pre_request_checked: bool,
    pub(crate) response_body_limit: Option<usize>,
    pub integrity: String,
    pub(crate) taint: ResponseType,
}
impl Request {
    pub fn new(url: HttpUrl, origin: Origin) -> Self {
        Self {
            url_list: vec![url],
            native_navigation_token: None,
            native_navigation_report_token: None,
            native_script_report_token: None,
            csp_self_origin: origin.clone(),
            origin,
            method: "GET".into(),
            headers: HeaderList::default(),
            body: None,
            mode: RequestMode::Cors,
            credentials: CredentialsMode::SameOrigin,
            redirect: RedirectMode::Follow,
            referrer: None,
            referrer_policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
            use_cors_preflight: false,
            security: SecurityContext::default(),
            csp: CspPolicyList::default(),
            csp_destination: CspDestination::None,
            csp_nonce: None,
            csp_integrity: Vec::new(),
            csp_parser_inserted: false,
            csp_violations: Vec::new(),
            nested_navigation: false,
            embedding_owner: None,
            csp_pre_request_checked: false,
            response_body_limit: None,
            integrity: String::new(),
            taint: ResponseType::Basic,
        }
    }
    pub fn url(&self) -> &HttpUrl {
        self.url_list.last().expect("nonempty URL list")
    }
    pub fn url_list(&self) -> &[HttpUrl] {
        &self.url_list
    }
    pub fn method(&self) -> &str {
        &self.method
    }
    pub fn body(&self) -> Option<&Body> {
        self.body.as_ref()
    }
    pub fn set_method_body(&mut self, method: &str, body: Option<Body>) -> Result<(), FetchError> {
        if !crate::headers::token(method)
            || ["CONNECT", "TRACE", "TRACK"]
                .iter()
                .any(|m| method.eq_ignore_ascii_case(m))
        {
            return Err(FetchError::InvalidMethod);
        }
        let upper = method.to_ascii_uppercase();
        let method = if matches!(
            upper.as_str(),
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
        ) {
            upper
        } else {
            method.into()
        };
        if matches!(method.as_str(), "GET" | "HEAD") && body.is_some() {
            return Err(FetchError::InvalidBody);
        }
        self.method = method;
        self.body = body;
        Ok(())
    }
    /// COEP affects effective network credentials, never the observable mode.
    pub fn coep_allows_credentials(&self) -> bool {
        self.mode != RequestMode::NoCors
            || self.security.embedder_policy != EmbedderPolicy::Credentialless
            || (self.origin == *self.url().origin() && self.redirect_taint_is_same_origin())
    }
    /// The redirect-taint becomes non-same-origin only at an origin transition
    /// where the preceding URL also differs from the request origin. This uses
    /// the actual URL chain, including a cross-origin bounce back to the origin.
    pub fn redirect_taint_is_same_origin(&self) -> bool {
        !self
            .url_list
            .windows(2)
            .any(|pair| pair[0].origin() != pair[1].origin() && pair[0].origin() != &self.origin)
    }
    pub fn credentials_allowed(&self) -> bool {
        self.coep_allows_credentials()
            && (self.credentials == CredentialsMode::Include
                || (self.credentials == CredentialsMode::SameOrigin
                    && self.taint == ResponseType::Basic
                    && self.origin == *self.url().origin()))
    }
    /// Call before each transport hop. Taint is sticky across redirects.
    pub fn prepare(&mut self) -> Result<FetchPlan, FetchError> {
        upgrade_mixed_content(self)?;
        let csp = self.csp.pre_request(self);
        self.csp_pre_request_checked=true;
        self.csp_violations.extend(csp.violations.iter().cloned());
        if csp.blocked {
            return Err(FetchError::Csp(crate::CspResponseProvenance::checked(self, csp.violations).violations().to_vec()));
        }
        check_request_security(self)?;
        if self.url().as_parsed().scheme() == "data" {
            if self.mode == RequestMode::SameOrigin {
                return Err(FetchError::SameOrigin);
            }
            // Fetch main fetch selects scheme fetch before CORS for data:.
            self.taint = ResponseType::Basic;
            return Ok(FetchPlan {
                taint: ResponseType::Basic,
                credentials: false,
                preflight: false,
                referrer: None,
            });
        }
        let cross = self.origin != *self.url().origin();
        match self.mode {
            RequestMode::SameOrigin if cross => return Err(FetchError::SameOrigin),
            RequestMode::NoCors => {
                if (cross && self.redirect != RedirectMode::Follow)
                    || !safelisted_method(self.method())
                    || !unsafe_request_header_names(&self.headers).is_empty()
                {
                    return Err(FetchError::NoCors);
                }
                if cross {
                    self.taint = ResponseType::Opaque;
                }
            }
            RequestMode::Cors if cross => self.taint = ResponseType::Cors,
            _ => (),
        }
        Ok(FetchPlan {
            taint: self.taint,
            credentials: self.credentials_allowed(),
            preflight: self.taint == ResponseType::Cors
                && (self.use_cors_preflight
                    || !safelisted_method(self.method())
                    || !unsafe_request_header_names(&self.headers).is_empty()),
            referrer: determine_referrer(self.referrer.as_ref(), self.url(), self.referrer_policy),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    pub taint: ResponseType,
    pub credentials: bool,
    pub preflight: bool,
    pub referrer: Option<String>,
}
/// Internal response belongs to trusted transport/policy code, never JS.
#[derive(Debug, Clone)]
pub struct InternalResponse {
    /// Actual main-hop transport decision; retained through cache storage.
    pub request_includes_credentials: bool,
    pub status: u16,
    pub status_text: String,
    pub headers: HeaderList,
    pub body: Option<Vec<u8>>,
}
/// Native policy receipts, never exposed by the JavaScript Response interface.
/// Construction requires the actual checked request; an empty violation list
/// still retains authentic transport lineage rather than synthetic authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspResponseProvenance {
    pub(crate) native_navigation_report_token: Option<std::sync::Arc<NativeNavigationToken>>,
    pub(crate) native_script_report_token: Option<std::sync::Arc<crate::reporting::NativeScriptCspBinding>>,
    url_list: Vec<HttpUrl>,
    violations: Vec<CspViolation>,
}
impl CspResponseProvenance {
    pub(crate) fn checked(request: &Request, final_violations: Vec<CspViolation>) -> Self {
        let mut violations = Vec::new();
        for receipt in request.csp_violations.iter().cloned().chain(final_violations) {
            if !violations.contains(&receipt) { violations.push(receipt); }
        }
        Self { native_navigation_report_token:request.native_navigation_report_token.clone(),native_script_report_token:request.native_script_report_token.clone(),url_list: request.url_list().to_vec(), violations }
    }
    pub fn url_list(&self) -> &[HttpUrl] { &self.url_list }
    pub fn violations(&self) -> &[CspViolation] { &self.violations }
}
/// Native custody of the actual main-hop Fetch credential decision.
/// Synthetic responses cannot construct this receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseCredentialReceipt { includes_credentials: bool }
impl ResponseCredentialReceipt {
    pub fn includes_credentials(self) -> bool { self.includes_credentials }
    pub(crate) fn received(response: &InternalResponse) -> Self {
        Self { includes_credentials: response.request_includes_credentials }
    }
}
/// Filtered response has no pointer back to hidden headers or body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub(crate) credential_receipt: Option<ResponseCredentialReceipt>,
    pub csp_provenance: Option<CspResponseProvenance>,
    pub response_type: ResponseType,
    pub status: u16,
    pub status_text: String,
    pub headers: HeaderList,
    pub body: Option<Vec<u8>>,
    pub url_list: Vec<HttpUrl>,
}
impl Response {
    pub fn credential_receipt(&self) -> Option<ResponseCredentialReceipt> { self.credential_receipt }
    pub fn hidden(response_type: ResponseType) -> Self {
        assert!(matches!(
            response_type,
            ResponseType::Opaque | ResponseType::OpaqueRedirect | ResponseType::Error
        ));
        Self {
            response_type,
            csp_provenance: None,
            credential_receipt: None,
            status: 0,
            status_text: String::new(),
            headers: HeaderList::default(),
            body: None,
            url_list: vec![],
        }
    }
}
/// NET-2 seam: implement in the shell/driver. Perform exactly one hop; never
/// follow redirects. Caller applies prepare/preflight/CORS/filtering. The plan
/// controls credential access; partitioning uses the top-level schemeful site.
/// Cache and cookie stores stay behind this boundary and must not see JS views.
pub trait NetworkOrCache {
    fn exchange(
        &mut self,
        request: &Request,
        plan: &FetchPlan,
        top_level_site: &Site,
    ) -> Result<InternalResponse, FetchError>;
}
