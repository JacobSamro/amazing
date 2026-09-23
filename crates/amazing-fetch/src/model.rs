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
#[derive(Debug, Clone)]
pub struct Request {
    pub(crate) url_list: Vec<HttpUrl>,
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
    pub(crate) taint: ResponseType,
}
impl Request {
    pub fn new(url: HttpUrl, origin: Origin) -> Self {
        Self {
            url_list: vec![url],
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
    pub fn credentials_allowed(&self) -> bool {
        self.credentials == CredentialsMode::Include
            || (self.credentials == CredentialsMode::SameOrigin
                && self.taint == ResponseType::Basic
                && self.origin == *self.url().origin())
    }
    /// Call before each transport hop. Taint is sticky across redirects.
    pub fn prepare(&mut self) -> Result<FetchPlan, FetchError> {
        upgrade_mixed_content(self)?;
        let csp = self.csp.pre_request(self);
        self.csp_violations.extend(csp.violations.iter().cloned());
        if csp.blocked {
            return Err(FetchError::Csp(csp.violations));
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
                if self.redirect != RedirectMode::Follow
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
    pub status: u16,
    pub status_text: String,
    pub headers: HeaderList,
    pub body: Option<Vec<u8>>,
}
/// Filtered response has no pointer back to hidden headers or body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub response_type: ResponseType,
    pub status: u16,
    pub status_text: String,
    pub headers: HeaderList,
    pub body: Option<Vec<u8>>,
    pub url_list: Vec<HttpUrl>,
}
impl Response {
    pub fn hidden(response_type: ResponseType) -> Self {
        assert!(matches!(
            response_type,
            ResponseType::Opaque | ResponseType::OpaqueRedirect | ResponseType::Error
        ));
        Self {
            response_type,
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
