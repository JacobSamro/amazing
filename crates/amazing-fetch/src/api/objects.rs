use super::{usv_string, ApiBody, ApiError, ApiHeaders, BodyInput, HeaderGuard};
use crate::{
    check_preflight, filter_response, preflight, redirect_location, Body, CredentialsMode,
    Destination, FetchError, HttpUrl, NetworkOrCache, Origin, RedirectAction, RedirectMode, ReferrerPolicy,
    Request, RequestMode, ResponseType, Site,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiCache {
    Default,
    NoStore,
    Reload,
    NoCache,
    ForceCache,
    OnlyIfCached,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiPriority {
    Auto,
    High,
    Low,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiResponseType {
    Default,
    Basic,
    Cors,
    Opaque,
    OpaqueRedirect,
    Error,
}

#[derive(Default)]
pub struct ApiRequestInit {
    pub method: Option<Vec<u16>>, // ByteString
    pub headers: Option<ApiHeaders>,
    pub body: Option<Option<BodyInput>>, // present null differs from missing
    pub referrer: Option<Vec<u16>>,      // USVString
    pub referrer_policy: Option<String>,
    pub mode: Option<String>,
    pub credentials: Option<String>,
    pub cache: Option<String>,
    pub redirect: Option<String>,
    pub integrity: Option<String>, // DOMString
    pub keepalive: Option<bool>,
    pub signal_aborted: Option<bool>,
    pub duplex: Option<String>,
    pub priority: Option<String>,
    pub window_non_null: Option<bool>,
    /// XHR permits URL userinfo; ordinary Request construction does not.
    pub(crate) allow_url_credentials: bool,
}
impl ApiRequestInit {
    fn present(&self) -> bool {
        self.method.is_some()
            || self.headers.is_some()
            || self.body.is_some()
            || self.referrer.is_some()
            || self.referrer_policy.is_some()
            || self.mode.is_some()
            || self.credentials.is_some()
            || self.cache.is_some()
            || self.redirect.is_some()
            || self.integrity.is_some()
            || self.keepalive.is_some()
            || self.signal_aborted.is_some()
            || self.duplex.is_some()
            || self.priority.is_some()
            || self.window_non_null.is_some()
    }
}
pub enum ApiRequestInput<'a> {
    Url(&'a [u16]),
    Request(&'a ApiRequest),
}

#[derive(Debug)]
pub struct ApiRequest {
    pub(super) url: HttpUrl,
    pub(super) native_navigation_token: Option<std::sync::Arc<crate::model::NativeNavigationToken>>,
    native_navigation_report_token: Option<std::sync::Arc<crate::model::NativeNavigationToken>>,
    native_script_report_token: Option<std::sync::Arc<crate::reporting::NativeScriptCspBinding>>,
    native_script_reporting_state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    native_script_service_admitted: bool,
    pub(super) navigation_urls: Vec<HttpUrl>,
    pub(super) navigation_csp_violations: Vec<crate::CspViolation>,
    pub(super) method: String,
    pub headers: ApiHeaders,
    pub body: ApiBody,
    pub mode: RequestMode,
    pub destination: Destination,
    pub credentials: CredentialsMode,
    pub cache: ApiCache,
    pub redirect: RedirectMode,
    pub referrer: String,
    pub referrer_policy: ReferrerPolicy,
    pub integrity: String,
    pub(crate) response_body_limit: Option<usize>,
    pub keepalive: bool,
    pub priority: ApiPriority,
    pub signal_aborted: bool,
    pub reload_or_history: bool,
    pub client_reserved_id: Option<u64>,
    pub(super) stream_body: bool,
    force_preflight: bool,
    pub(super) origin: Origin,
    pub(super) nested_navigation: bool,
    pub(crate) embedding_owner: Option<std::sync::Arc<crate::reporting::NavigationEmbeddingOwnerSnapshot>>,
    // Ordinary browser-owned element/policy metadata; no WebIDL setter exists.
    csp_nonce: Option<String>,
    csp_parser_inserted: bool,
    csp_self_origin: Option<Origin>,
    /// False until `init.referrerPolicy` (or a copied Request) fixed a policy;
    /// the client's document policy applies otherwise.
    referrer_policy_set: bool,
}
/// Trusted browser navigation inputs. This constructor is outside WebIDL;
/// JavaScript `new Request(..., { mode: "navigate" })` remains invalid.
pub struct NavigationRequestInit {
    pub method: String,
    pub body: Vec<u8>,
    pub referrer: String,
    pub referrer_policy: String,
    pub initiator_origin: Origin,
    pub reload_or_history: bool,
    pub client_reserved_id: Option<u64>,
}
/// Browser-minted parent browsing context for a child-document navigation.
#[derive(Clone)]
pub struct NavigableContext {
    pub top_level_origin: Origin,
    pub parent_origin: Origin,
    /// Top-level through immediate parent, in browsing-context order.
    pub ancestor_origins: Vec<Origin>,
    pub sandbox_flags: u32,
    pub frame_csp: crate::CspPolicyList,
    pub referrer: String,
    pub referrer_policy: String,
    pub now_unix: i64,
}
impl ApiRequest {
    pub fn child_navigation(target: HttpUrl, parent: &NavigableContext) -> Result<Self, ApiError> {
        Self::child_navigation_with_initiator(target, parent, &parent.parent_origin)
    }
    /// The authentic incumbent initiator is separate from the embedding parent.
    /// Parent custody still validates native ancestor/sandbox constraints.
    pub fn child_navigation_with_initiator(target: HttpUrl, parent: &NavigableContext,
        actual_initiator: &Origin) -> Result<Self, ApiError> {
        if parent.ancestor_origins.is_empty() || parent.ancestor_origins.len() > 32
            || parent.ancestor_origins.first() != Some(&parent.top_level_origin)
            || parent.ancestor_origins.last() != Some(&parent.parent_origin)
            || parent.sandbox_flags & !0xffff != 0 {
            return Err(ApiError::Network(FetchError::FrameEmbedding));
        }
        let mut request = Self::navigation(target, NavigationRequestInit {
            method: "GET".into(), body: Vec::new(),
            referrer: parent.referrer.clone(),
            referrer_policy: parent.referrer_policy.clone(),
            initiator_origin: actual_initiator.clone(),
            reload_or_history: false, client_reserved_id: None,
        })?;
        request.destination = Destination::Other; // wire destination: frame
        request.nested_navigation = true;
        Ok(request)
    }
    pub fn child_navigation_with_embedding_owner(target:HttpUrl,parent:&NavigableContext,
        actual_initiator:&Origin,owner:crate::reporting::PreparedNavigationEmbeddingOwner)->Result<Self,ApiError> {
        let snapshot=owner.into_snapshot(parent)?;
        let mut request=Self::child_navigation_with_initiator(target,parent,actual_initiator)?;
        request.embedding_owner=Some(snapshot);
        Ok(request)
    }
    pub fn navigation(target: HttpUrl, init: NavigationRequestInit) -> Result<Self, ApiError> {
        let units: Vec<u16> = target.serialize().encode_utf16().collect();
        let body = if init.body.is_empty() && matches!(init.method.as_str(), "GET" | "HEAD") {
            None
        } else {
            Some(Some(BodyInput::Bytes(init.body)))
        };
        let mut request = Self::new(
            ApiRequestInput::Url(&units),
            ApiRequestInit {
                method: Some(init.method.encode_utf16().collect()),
                body,
                referrer: Some(init.referrer.encode_utf16().collect()),
                referrer_policy: Some(init.referrer_policy),
                ..ApiRequestInit::default()
            },
            &target,
            init.initiator_origin,
        )?;
        request.native_navigation_token = Some(std::sync::Arc::new(crate::model::NativeNavigationToken::new()));
        request.navigation_urls = vec![request.url.clone()];
        request.mode = RequestMode::Navigate;
        request.destination = Destination::Document;
        request.credentials = CredentialsMode::Include;
        request.redirect = RedirectMode::Manual;
        request.reload_or_history = init.reload_or_history;
        request.client_reserved_id = init.client_reserved_id;
        Ok(request)
    }
    /// Bound the response body before dispatch. Zero permits an empty body.
    pub fn set_response_body_limit(&mut self, bytes: u64) -> Result<(), FetchError> {
        let limit = usize::try_from(bytes).map_err(|_| FetchError::ResponseTooLarge)?;
        if limit > super::MAX_API_BODY_BYTES {
            return Err(FetchError::ResponseTooLarge);
        }
        self.response_body_limit = Some(limit);
        Ok(())
    }
    /// `base` and `origin` come from the embedding's relevant settings object.
    pub fn new(
        input: ApiRequestInput<'_>,
        init: ApiRequestInit,
        base: &HttpUrl,
        origin: Origin,
    ) -> Result<Self, ApiError> {
        if init.window_non_null == Some(true) {
            return Err(ApiError::InvalidWindow);
        }
        let present = init.present();
        let mut request = match input {
            ApiRequestInput::Url(units) => {
                let url = base
                    .resolve(&usv_string(units))
                    .map_err(|_| ApiError::InvalidUrl)?;
                if url.includes_credentials() && !init.allow_url_credentials {
                    return Err(ApiError::UrlCredentials);
                }
                Self {
                    url,
                    native_navigation_token: None,
                    native_navigation_report_token: None,
                    native_script_report_token: None,
                    native_script_reporting_state: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
                    native_script_service_admitted: false,
                    navigation_urls: Vec::new(),
                    navigation_csp_violations: Vec::new(),
                    method: "GET".into(),
                    headers: ApiHeaders::new(HeaderGuard::Request),
                    body: ApiBody::empty(),
                    mode: RequestMode::Cors,
                    destination: Destination::Empty,
                    credentials: CredentialsMode::SameOrigin,
                    cache: ApiCache::Default,
                    redirect: RedirectMode::Follow,
                    referrer: "about:client".into(),
                    referrer_policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
                    integrity: String::new(),
                    response_body_limit: None,
                    keepalive: false,
                    priority: ApiPriority::Auto,
                    signal_aborted: false,
                    reload_or_history: false,
                    client_reserved_id: None,
                    stream_body: false,
                    force_preflight: false,
                    origin: origin.clone(),
                    nested_navigation: false,
                    embedding_owner: None,
                    csp_nonce: None, csp_parser_inserted: false, csp_self_origin: None,
                    referrer_policy_set: false,
                }
            }
            ApiRequestInput::Request(other) => {
                if other.body.is_unusable() {
                    return Err(ApiError::BodyUsed);
                }
                Self {
                    url: other.url.clone(),
                    native_navigation_token: other.native_navigation_token.clone(),
                    native_navigation_report_token: other.native_navigation_report_token.clone(),
                    native_script_report_token: other.native_script_report_token.clone(),
                    native_script_reporting_state: other.native_script_reporting_state.clone(),
                    native_script_service_admitted: false,
                    navigation_urls: other.navigation_urls.clone(),
                    navigation_csp_violations: other.navigation_csp_violations.clone(),
                    method: other.method.clone(),
                    headers: other.headers.clone(),
                    body: other.body.clone_body()?,
                    mode: other.mode,
                    destination: other.destination,
                    credentials: other.credentials,
                    cache: other.cache,
                    redirect: other.redirect,
                    referrer: other.referrer.clone(),
                    referrer_policy: other.referrer_policy,
                    integrity: other.integrity.clone(),
                    response_body_limit: other.response_body_limit,
                    keepalive: other.keepalive,
                    priority: other.priority,
                    signal_aborted: other.signal_aborted,
                    reload_or_history: other.reload_or_history,
                    client_reserved_id: other.client_reserved_id,
                    stream_body: other.stream_body,
                    force_preflight: other.force_preflight,
                    origin: other.origin.clone(),
                    nested_navigation: other.nested_navigation,
                    embedding_owner: other.embedding_owner.clone(),
                    csp_nonce: other.csp_nonce.clone(), csp_parser_inserted: other.csp_parser_inserted, csp_self_origin: other.csp_self_origin.clone(),
                    referrer_policy_set: other.referrer_policy_set,
                }
            }
        };
        if present {
            if request.mode == RequestMode::Navigate {
                request.mode = RequestMode::SameOrigin;
            }
            request.origin = origin.clone();
            request.referrer = "about:client".into();
        }
        if let Some(referrer) = init.referrer {
            let referrer = usv_string(&referrer);
            request.referrer = if referrer.is_empty() {
                String::new()
            } else if referrer == "about:client" {
                referrer
            } else {
                let parsed = base
                    .resolve(&referrer)
                    .map_err(|_| ApiError::InvalidReferrer)?;
                if parsed.origin() != &origin {
                    "about:client".into()
                } else {
                    parsed.serialize()
                }
            };
        }
        if let Some(policy) = init.referrer_policy {
            request.referrer_policy = parse_referrer_policy(&policy)?;
            request.referrer_policy_set = true;
        }
        if let Some(mode) = init.mode {
            request.mode = parse_mode(&mode)?;
        }
        if let Some(credentials) = init.credentials {
            request.credentials = parse_credentials(&credentials)?;
        }
        if let Some(cache) = init.cache {
            request.cache = parse_cache(&cache)?;
        }
        if request.cache == ApiCache::OnlyIfCached && request.mode != RequestMode::SameOrigin {
            return Err(ApiError::OnlyIfCachedMode);
        }
        if let Some(redirect) = init.redirect {
            request.redirect = parse_redirect(&redirect)?;
        }
        if let Some(integrity) = init.integrity {
            request.integrity = integrity;
        }
        if let Some(keepalive) = init.keepalive {
            request.keepalive = keepalive;
        }
        if let Some(signal) = init.signal_aborted {
            request.signal_aborted = signal;
        }
        if let Some(priority) = init.priority {
            request.priority = parse_priority(&priority)?;
        }
        if let Some(duplex) = &init.duplex {
            if duplex != "half" {
                return Err(ApiError::InvalidDuplex);
            }
        }
        if let Some(method) = init.method {
            let method = super::byte_string(&method)?;
            if !crate::headers::token(&method) {
                return Err(ApiError::InvalidMethod);
            }
            if ["CONNECT", "TRACE", "TRACK"]
                .iter()
                .any(|m| method.eq_ignore_ascii_case(m))
            {
                return Err(ApiError::ForbiddenMethod);
            }
            let uppercase = method.to_ascii_uppercase();
            request.method = if matches!(
                uppercase.as_str(),
                "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
            ) {
                uppercase
            } else {
                method
            };
        }
        request
            .headers
            .set_guard(if request.mode == RequestMode::NoCors {
                HeaderGuard::RequestNoCors
            } else {
                HeaderGuard::Request
            });
        if request.mode == RequestMode::NoCors && !crate::safelisted_method(&request.method) {
            return Err(ApiError::NoCorsMethod);
        }
        if present {
            let source = init.headers.unwrap_or_else(|| request.headers.clone());
            let mut headers = ApiHeaders::new(request.headers.guard());
            for (name, value) in source.raw_entries() {
                headers.append_existing(source.wire_name(name), value)?;
            }
            request.headers = headers;
        }
        let input_body = request.body.shared_present();
        let init_body = init.body.flatten();
        if (input_body || init_body.is_some()) && matches!(request.method.as_str(), "GET" | "HEAD")
        {
            return Err(ApiError::GetHeadBody);
        }
        if let Some(body) = init_body {
            if body.is_stream() {
                if request.keepalive {
                    return Err(ApiError::KeepaliveStream);
                }
                if init.duplex.is_none() {
                    return Err(ApiError::StreamNeedsDuplex);
                }
                if !matches!(request.mode, RequestMode::Cors | RequestMode::SameOrigin) {
                    return Err(ApiError::StreamMode);
                }
            }
            request.stream_body = body.is_stream();
            let extracted = ApiBody::from_input(body)?;
            if let Some(mime) = extracted.mime_type() {
                if !request.headers.has_string("content-type")? {
                    request.headers.append_string("Content-Type", mime)?;
                }
            }
            request.body = extracted;
        }
        if request.stream_body
            && !matches!(request.mode, RequestMode::Cors | RequestMode::SameOrigin)
        {
            return Err(ApiError::StreamMode);
        }
        Ok(request)
    }
    pub fn url(&self) -> &HttpUrl {
        &self.url
    }
    /// Network-owned HSTS rewrite before Fetch policy and cache lookup.
    pub fn upgrade_hsts(&mut self, url: HttpUrl) {
        self.url = url;
    }
    pub fn method(&self) -> &str {
        &self.method
    }
    pub(crate) fn origin(&self) -> &Origin {
        &self.origin
    }
    pub fn body_used(&self) -> bool {
        self.body.body_used()
    }
    /// XHR upload listeners set Fetch's use-CORS-preflight flag even for a
    /// safelisted method and header set.
    pub fn set_use_cors_preflight(&mut self, value: bool) {
        self.force_preflight = value;
    }
    pub fn clone_request(&self) -> Result<Self, ApiError> { self.clone_request_inner(false) }
    pub(super) fn clone_request_inner(&self,drop_body:bool) -> Result<Self,ApiError> {
        if !drop_body && self.body.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        Ok(Self {
            url: self.url.clone(),
            native_navigation_token: self.native_navigation_token.clone(),
            native_navigation_report_token: self.native_navigation_report_token.clone(),
            native_script_report_token: self.native_script_report_token.clone(),
            native_script_reporting_state: self.native_script_reporting_state.clone(),
            native_script_service_admitted: false,
            navigation_urls: self.navigation_urls.clone(),
            navigation_csp_violations: self.navigation_csp_violations.clone(),
            method: self.method.clone(),
            headers: self.headers.clone(),
            body: if drop_body { ApiBody::empty() } else { self.body.clone_body()? },
            mode: self.mode,
            destination: self.destination,
            credentials: self.credentials,
            cache: self.cache,
            redirect: self.redirect,
            referrer: self.referrer.clone(),
            referrer_policy: self.referrer_policy,
            integrity: self.integrity.clone(),
            response_body_limit: self.response_body_limit,
            keepalive: self.keepalive,
            priority: self.priority,
            signal_aborted: self.signal_aborted,
            reload_or_history: self.reload_or_history,
            client_reserved_id: self.client_reserved_id,
            stream_body: self.stream_body,
            force_preflight: self.force_preflight,
            origin: self.origin.clone(),
            nested_navigation: self.nested_navigation,
            embedding_owner: self.embedding_owner.clone(),
            csp_nonce: self.csp_nonce.clone(), csp_parser_inserted: self.csp_parser_inserted, csp_self_origin: self.csp_self_origin.clone(),
            referrer_policy_set: self.referrer_policy_set,
        })
    }
    /// Native external HTML element preparation before ordinary Fetch dispatch.
    /// The loader supplies its actual element kind/internal nonce/attributes.
    /// Request constructors and clones retain this private native metadata.
    pub fn apply_csp_request_element(&mut self, kind: crate::CspExternalElementKind, element: &crate::CspRequestElement<'_>) {
        self.destination = match kind {crate::CspExternalElementKind::Script=>Destination::Script,crate::CspExternalElementKind::Style=>Destination::Style};
        let mut policy = Request::new(self.url.clone(), self.origin.clone());
        policy.security.destination = self.destination;
        policy.apply_csp_request_element(element);
        self.csp_nonce = policy.csp_nonce;
        self.csp_parser_inserted = policy.csp_parser_inserted;
    }
    /// Preserve actual receiving origin separately from its protected policy
    /// self-origin, including sandbox-opaque documents and inherited about.
    pub fn apply_receiving_csp_client<Identity>(&mut self, receiving: &crate::ReceivingDocumentClient<Identity>) -> Result<(),crate::CspRequestEnrichmentError> {
        if self.origin != receiving.origin { return Err(crate::CspRequestEnrichmentError::ReceivingClientMismatch); }
        self.csp_self_origin = Some(receiving.csp_self_origin.clone());
        Ok(())
    }
    pub(crate) fn apply_navigation_receiving_client<Identity>(&mut self,
        receiving:&crate::ReceivingDocumentClient<Identity>) -> Result<(),crate::CspRequestEnrichmentError> {
        self.apply_receiving_csp_client(receiving)?;
        if self.navigation_urls.len() <= 1 {
            self.referrer_policy=receiving.referrer_policy;
            self.referrer=HttpUrl::from_parsed(receiving.document_url.clone()).ok()
                .filter(|url|matches!(url.as_parsed().scheme(),"http"|"https"))
                .map(|url|url.serialize()).unwrap_or_default();
        }
        Ok(())
    }
    /// `fetch()` main-fetch step: a request whose referrer is still the `client`
    /// takes the client's document URL (http/https only) and, unless the
    /// caller fixed one, the client's referrer policy.
    pub fn apply_fetch_receiving_client<Identity>(&mut self, receiving: &crate::ReceivingDocumentClient<Identity>) {
        if self.referrer == "about:client" {
            self.referrer = HttpUrl::from_parsed(receiving.document_url.clone()).ok()
                .filter(|url| matches!(url.as_parsed().scheme(), "http" | "https"))
                .map(|url| url.serialize()).unwrap_or_default();
        }
        if !self.referrer_policy_set {
            self.referrer_policy = receiving.referrer_policy;
        }
    }
    pub(crate) fn can_claim_script_csp_reporting(&self)->bool {
        self.mode!=RequestMode::Navigate && self.embedding_owner.is_none()
            && matches!(self.destination,Destination::Empty|Destination::Script)
            && self.native_script_report_token.is_none()
            && self.native_script_reporting_state.load(std::sync::atomic::Ordering::Acquire)==0
    }
    pub(crate) fn claim_script_csp_reporting(&mut self,
        binding:std::sync::Arc<crate::reporting::NativeScriptCspBinding>) -> bool {
        if !self.can_claim_script_csp_reporting() {return false;}
        if self.native_script_reporting_state.compare_exchange(0,2,std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire).is_err() {return false;}
        self.destination=Destination::Script;
        self.native_script_report_token=Some(binding);true
    }
    pub(crate) fn claim_script_csp_admission(&mut self,service:u64,context:&crate::ClientContext)->bool {
        use std::sync::atomic::Ordering::{Acquire,AcqRel};
        if let Some(binding)=&self.native_script_report_token {
            if !binding.matches_admission(service,&self.origin,self.csp_self_origin.as_ref(),self.mode,
                self.destination,self.embedding_owner.is_some(),context) {return false;}
            if self.native_script_reporting_state.compare_exchange(2,3,AcqRel,Acquire).is_err() {return false;}
            binding.mark_admitted();self.native_script_service_admitted=true;true
        } else {
            // Existing ordinary requests may be reused. A clone predating a
            // source claim cannot bypass or consume that claim's one-shot loan.
            loop {match self.native_script_reporting_state.load(Acquire) {
                0=>if self.native_script_reporting_state.compare_exchange(0,1,AcqRel,Acquire).is_ok(){return true;},
                1=>return true,
                _=>return false,
            }}
        }
    }
    pub(crate) fn claim_navigation_csp_reporting(&mut self)
        -> Option<std::sync::Arc<crate::model::NativeNavigationToken>> {
        if self.mode!=RequestMode::Navigate || self.redirect!=RedirectMode::Manual
            || self.embedding_owner.is_none() || self.native_navigation_report_token.is_some() {return None;}
        let token=self.native_navigation_token.as_ref()?;
        if token.source_reporting_admitted.load(std::sync::atomic::Ordering::Acquire) {return None;}
        token.source_reporting_claimed.compare_exchange(false,true,std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire).ok()?;
        self.native_navigation_report_token=Some(token.clone());Some(token.clone())
    }
    pub(crate) fn claim_navigation_csp_admission(&self) -> bool {
        if self.native_navigation_report_token.is_none() {
            if let Some(token)=&self.native_navigation_token {token.source_reporting_admitted.store(true,std::sync::atomic::Ordering::Release);}
            return true;
        }
        self.native_navigation_token.as_ref().is_some_and(|token|
            token.source_reporting_admitted.compare_exchange(false,true,std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire).is_ok())
    }
    pub(crate) fn policy_request(&self) -> Result<Request, ApiError> {
        let mut out = Request::new(self.url.clone(), self.origin.clone());
        out.native_navigation_token=self.native_navigation_token.clone();
        out.native_navigation_report_token=self.native_navigation_report_token.clone();
        // Only the actual service-owned admitted object can project reporting
        // authority. Public clones and legacy synchronous calls cannot bypass
        // the one-shot native service admission through policy projection.
        out.native_script_report_token=self.native_script_service_admitted.then(||self.native_script_report_token.clone()).flatten();
        if !self.navigation_urls.is_empty() {
            out.url_list=self.navigation_urls.clone();
            *out.url_list.last_mut().unwrap()=self.url.clone();
            out.csp_violations=self.navigation_csp_violations.clone();
        }
        out.nested_navigation = self.nested_navigation;
        out.embedding_owner = self.embedding_owner.clone();
        out.csp_nonce = self.csp_nonce.clone();
        out.csp_parser_inserted = self.csp_parser_inserted;
        if let Some(origin) = &self.csp_self_origin { out.csp_self_origin = origin.clone(); }
        out.mode = self.mode;
        out.security.destination = self.destination;
        out.security.top_level_navigation = self.mode == RequestMode::Navigate;
        out.credentials = self.credentials;
        out.redirect = self.redirect;
        out.referrer_policy = self.referrer_policy;
        out.referrer = if self.referrer == "about:client" || self.referrer.is_empty() {
            None
        } else {
            Some(HttpUrl::parse(&self.referrer).map_err(|_| ApiError::InvalidReferrer)?)
        };
        out.headers = self.headers.to_header_list()?;
        out.use_cors_preflight = self.stream_body || self.force_preflight;
        out.response_body_limit = self.response_body_limit;
        out.integrity = self.integrity.clone();
        let body = if self.body.shared_present() {
            Some(Body::Unreplayable)
        } else {
            None
        };
        out.set_method_body(&self.method, body)
            .map_err(ApiError::Network)?;
        Ok(out)
    }
}
fn parse_mode(s: &str) -> Result<RequestMode, ApiError> {
    match s {
        "cors" => Ok(RequestMode::Cors),
        "no-cors" => Ok(RequestMode::NoCors),
        "same-origin" => Ok(RequestMode::SameOrigin),
        "navigate" => Err(ApiError::NavigateMode),
        _ => Err(ApiError::InvalidMode),
    }
}
fn parse_credentials(s: &str) -> Result<CredentialsMode, ApiError> {
    match s {
        "omit" => Ok(CredentialsMode::Omit),
        "same-origin" => Ok(CredentialsMode::SameOrigin),
        "include" => Ok(CredentialsMode::Include),
        _ => Err(ApiError::InvalidCredentials),
    }
}
fn parse_cache(s: &str) -> Result<ApiCache, ApiError> {
    match s {
        "default" => Ok(ApiCache::Default),
        "no-store" => Ok(ApiCache::NoStore),
        "reload" => Ok(ApiCache::Reload),
        "no-cache" => Ok(ApiCache::NoCache),
        "force-cache" => Ok(ApiCache::ForceCache),
        "only-if-cached" => Ok(ApiCache::OnlyIfCached),
        _ => Err(ApiError::InvalidCache),
    }
}
fn parse_redirect(s: &str) -> Result<RedirectMode, ApiError> {
    match s {
        "follow" => Ok(RedirectMode::Follow),
        "error" => Ok(RedirectMode::Error),
        "manual" => Ok(RedirectMode::Manual),
        _ => Err(ApiError::InvalidRedirect),
    }
}
fn parse_priority(s: &str) -> Result<ApiPriority, ApiError> {
    match s {
        "auto" => Ok(ApiPriority::Auto),
        "high" => Ok(ApiPriority::High),
        "low" => Ok(ApiPriority::Low),
        _ => Err(ApiError::InvalidPriority),
    }
}
fn parse_referrer_policy(s: &str) -> Result<ReferrerPolicy, ApiError> {
    if s.is_empty() {
        return Ok(ReferrerPolicy::StrictOriginWhenCrossOrigin);
    }
    ReferrerPolicy::parse_header(s).ok_or(ApiError::InvalidReferrerPolicy)
}

#[derive(Default)]
pub struct ApiResponseInit {
    pub status: Option<u16>,
    pub status_text: Option<Vec<u16>>,
    pub headers: Option<ApiHeaders>,
}
/// Browser-only metadata from an accepted navigation response. Script-visible
/// Response headers retain Fetch filtering; this snapshot is never initialized
/// by Response constructors or renderer-supplied policy data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationResponseMetadata {
    authenticated_fields: (u16, String, String, Vec<(String, String)>, String),
    native_csp_receipt: Option<crate::CspResponseProvenance>,
    pub(super) native_navigation_token: Option<std::sync::Arc<crate::model::NativeNavigationToken>>,
    pub(super) request_urls: Vec<HttpUrl>,
    pub(super) request_origin: Origin,
    pub(super) owner: Option<crate::reporting::NavigationEmbeddingOwnerId>,
    pub(super) request_includes_credentials: bool,
    pub status: u16,
    pub final_url: String,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub referrer: String,
}
impl NavigationResponseMetadata {
    pub(crate) fn with_csp_receipt(mut self, receipt: Option<&crate::CspResponseProvenance>) -> Self {
        self.native_csp_receipt = receipt.cloned(); self
    }
    pub(super) fn authentic_received_fields(&self, receipt: Option<&crate::CspResponseProvenance>) -> bool {
        self.authenticated_fields.0 == self.status
            && self.authenticated_fields.1 == self.final_url
            && self.authenticated_fields.2 == self.reason
            && self.authenticated_fields.3 == self.headers
            && self.authenticated_fields.4 == self.referrer
            && self.native_csp_receipt.as_ref() == receipt && receipt.is_some()
    }
    pub(crate) fn capture(policy: &Request, response: &crate::InternalResponse,
        referrer: Option<&str>) -> Option<Self> {
        if policy.mode != RequestMode::Navigate
            || !matches!(policy.url().as_parsed().scheme(), "http" | "https") {
            return None;
        }
        let authenticated_fields = (response.status, policy.url().without_fragment(),
            response.status_text.clone(), response.headers.iter().map(|(n,v)|(n.to_owned(),v.to_owned())).collect(),
            referrer.unwrap_or_default().to_owned());
        Some(Self {
            authenticated_fields,
            native_csp_receipt: None,
            native_navigation_token: policy.native_navigation_token.clone(),
            request_urls: policy.url_list().to_vec(),
            request_origin: policy.origin.clone(),
            owner: policy.embedding_owner.as_ref().map(|owner|owner.id()),
            request_includes_credentials: response.request_includes_credentials,
            status: response.status,
            final_url: policy.url().without_fragment(),
            reason: response.status_text.clone(),
            headers: response.headers.iter().map(|(n, v)| (n.to_owned(), v.to_owned())).collect(),
            referrer: referrer.unwrap_or_default().to_owned(),
        })
    }
}
#[derive(Debug)]
pub struct ApiResponse {
    pub(crate) credential_receipt: Option<crate::ResponseCredentialReceipt>,
    /// Trusted native-only CSP receipts and transport lineage; not a JS property.
    /// Synthetic Response constructors do not mint checked-request provenance.
    pub csp_provenance: Option<crate::CspResponseProvenance>,
    pub(crate) navigation_metadata: Option<NavigationResponseMetadata>,
    pub response_type: ApiResponseType,
    pub url: String,
    /// Trusted URL chain; never exposed through the JavaScript Response interface.
    pub url_list: Vec<String>,
    pub redirected: bool,
    pub status: u16,
    pub status_text: String,
    pub headers: ApiHeaders,
    pub body: ApiBody,
}
impl ApiResponse {
    pub fn credential_receipt(&self) -> Option<crate::ResponseCredentialReceipt> { self.credential_receipt }
    /// Actual accepted native navigation fields, independent of Fetch-visible headers.
    pub fn navigation_metadata(&self) -> Option<&NavigationResponseMetadata> { self.navigation_metadata.as_ref() }
    pub fn new(input: Option<BodyInput>, init: ApiResponseInit) -> Result<Self, ApiError> {
        let status = init.status.unwrap_or(200);
        if !(200..=599).contains(&status) {
            return Err(ApiError::InvalidStatus);
        }
        let status_text = match init.status_text {
            Some(v) => super::byte_string(&v)?,
            None => String::new(),
        };
        if !status_text
            .chars()
            .all(|c| c == '\t' || (c >= ' ' && c <= '~') || (c as u32 >= 128 && c as u32 <= 255))
        {
            return Err(ApiError::InvalidStatusText);
        }
        if input.is_some() && matches!(status, 204 | 205 | 304) {
            return Err(ApiError::NullBodyStatus);
        }
        let mut headers = ApiHeaders::new(HeaderGuard::Response);
        if let Some(source) = init.headers {
            for (n, v) in source.raw_entries() {
                headers.append_existing(source.wire_name(n), v)?;
            }
        }
        let mut body = match input {
            Some(body) => ApiBody::from_input(body)?,
            None => ApiBody::empty(),
        };
        if let Some(mime) = body.mime_type() {
            if !headers.has_string("content-type")? {
                headers.append_string("content-type", mime)?;
            }
        }
        if let Some(mime) = headers.get_string("content-type")? {
            body.set_mime_type(Some(mime));
        }
        Ok(Self {
            csp_provenance: None,
            credential_receipt: None,
            navigation_metadata: None,
            response_type: ApiResponseType::Default,
            url: String::new(),
            url_list: Vec::new(),
            redirected: false,
            status,
            status_text,
            headers,
            body,
        })
    }
    pub fn error() -> Self {
        Self {
            csp_provenance: None,
            credential_receipt: None,
            navigation_metadata: None,
            response_type: ApiResponseType::Error,
            url: String::new(),
            url_list: Vec::new(),
            redirected: false,
            status: 0,
            status_text: String::new(),
            headers: ApiHeaders::new(HeaderGuard::Immutable),
            body: ApiBody::empty(),
        }
    }
    pub fn redirect(url: &[u16], status: u16, base: &HttpUrl) -> Result<Self, ApiError> {
        let target = base
            .resolve(&usv_string(url))
            .map_err(|_| ApiError::InvalidUrl)?;
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Err(ApiError::InvalidRedirectStatus);
        }
        let mut response = Self::new(None, ApiResponseInit::default())?;
        response.status = status;
        response
            .headers
            .append_string("location", &target.serialize())?;
        response.headers.set_guard(HeaderGuard::Immutable);
        Ok(response)
    }
    /// The embedder performs JSON.stringify and passes its UTF-8 output.
    pub fn json(serialized: Vec<u8>, init: ApiResponseInit) -> Result<Self, ApiError> {
        let mut response = Self::new(Some(BodyInput::Bytes(serialized)), init)?;
        if !response.headers.has_string("content-type")? {
            response
                .headers
                .append_string("content-type", "application/json")?;
        }
        Ok(response)
    }
    pub fn ok(&self) -> bool {
        (200..=299).contains(&self.status)
    }
    /// Consume using the current response Content-Type, including header mutations.
    pub fn form_data(&mut self) -> Result<super::FormData, ApiError> {
        self.body
            .set_mime_type(self.headers.get_string("content-type")?);
        self.body.form_data()
    }
    pub fn blob(&mut self) -> Result<super::Blob, ApiError> {
        self.body
            .set_mime_type(self.headers.get_string("content-type")?);
        self.body.blob()
    }
    pub fn body_used(&self) -> bool {
        self.body.body_used()
    }
    pub fn clone_response(&self) -> Result<Self, ApiError> {
        if self.body.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        Ok(Self {
            csp_provenance: self.csp_provenance.clone(),
            credential_receipt: self.credential_receipt,
            navigation_metadata: self.navigation_metadata.clone(),
            response_type: self.response_type,
            url: self.url.clone(),
            url_list: self.url_list.clone(),
            redirected: self.redirected,
            status: self.status,
            status_text: self.status_text.clone(),
            headers: self.headers.clone(),
            body: self.body.clone_body()?,
        })
    }
    pub(crate) fn from_policy(response: crate::Response) -> Result<Self, ApiError> {
        let credential_receipt = response.credential_receipt();
        let url_list = response.url_list.iter().map(HttpUrl::serialize).collect();
        let url = response
            .url_list
            .last()
            .map(HttpUrl::without_fragment)
            .unwrap_or_default();
        let redirected = response.url_list.len() > 1;
        let response_type = match response.response_type {
            ResponseType::Basic => ApiResponseType::Basic,
            ResponseType::Cors => ApiResponseType::Cors,
            ResponseType::Opaque => ApiResponseType::Opaque,
            ResponseType::OpaqueRedirect => ApiResponseType::OpaqueRedirect,
            ResponseType::Error => ApiResponseType::Error,
        };
        let mime = response.headers.get("content-type");
        let mut body = match response.body {
            Some(bytes) => ApiBody::from_input(BodyInput::Bytes(bytes))?,
            None => ApiBody::empty(),
        };
        body.set_mime_type(mime);
        Ok(Self {
            credential_receipt,
            csp_provenance: response.csp_provenance,
            navigation_metadata: None,
            response_type,
            url,
            url_list,
            redirected,
            status: response.status,
            status_text: response.status_text,
            headers: ApiHeaders::from_header_list(HeaderGuard::Response, &response.headers),
            body,
        })
    }
}

pub trait AbortCheck {
    fn aborted(&self) -> bool;
}
impl AbortCheck for bool {
    fn aborted(&self) -> bool {
        *self
    }
}

/// Single-hop transport/cookie/cache remains behind NetworkOrCache. The glue
/// applies Fetch policy at every hop and checks cancellation between hops.
pub fn fetch_api(
    request: &mut ApiRequest,
    network: &mut impl NetworkOrCache,
    site: &Site,
    abort: &impl AbortCheck,
) -> Result<ApiResponse, ApiError> {
    if request.embedding_owner.is_some() { return Err(ApiError::Network(FetchError::FrameEmbedding)); }
    if abort.aborted() || request.signal_aborted {
        return Err(ApiError::Aborted);
    }
    if request.body.is_unusable() {
        return Err(ApiError::BodyUsed);
    }
    let mut policy = request.policy_request()?;
    for _ in 0..=20 {
        if abort.aborted() {
            return Err(ApiError::Aborted);
        }
        let plan = policy.prepare().map_err(ApiError::Network)?;
        if request.body.shared_present() && !request.body.body_used() {
            let bytes = request.body.bytes()?;
            policy
                .set_method_body(request.method(), Some(Body::Bytes(bytes)))
                .map_err(ApiError::Network)?;
        }
        if policy.url().as_parsed().scheme() == "data" {
            let response = crate::data_url::data_response(policy.url().as_parsed())
                .map_err(ApiError::Network)?;
            return ApiResponse::from_policy(
                filter_response(&policy, &response).map_err(ApiError::Network)?,
            );
        }
        if plan.preflight {
            if let Some(preflight) = preflight(&policy).map_err(ApiError::Network)? {
                let mut probe = preflight_policy(&policy, preflight.headers)?;
                let probe_plan = probe.prepare().map_err(ApiError::Network)?;
                let answer = network
                    .exchange(&probe, &probe_plan, site)
                    .map_err(ApiError::Network)?;
                check_preflight(&policy, &answer).map_err(ApiError::Network)?;
            }
        }
        let response = network
            .exchange(&policy, &plan, site)
            .map_err(ApiError::Network)?;
        if abort.aborted() {
            return Err(ApiError::Aborted);
        }
        // Buffering a pull source for the transport does not make the original
        // streaming request body replayable across a redirect.
        if request.stream_body && matches!(policy.body(), Some(Body::Bytes(_))) {
            let method = policy.method().to_owned();
            policy
                .set_method_body(&method, Some(Body::Unreplayable))
                .map_err(ApiError::Network)?;
        }
        let location = response.headers.get("location");
        match redirect_location(&mut policy, &response, location.as_deref())
            .map_err(ApiError::Network)?
        {
            RedirectAction::Follow => continue,
            RedirectAction::Manual(response) => return ApiResponse::from_policy(response),
            RedirectAction::NavigationPending => {
                let filtered = filter_response(&policy, &response).map_err(ApiError::Network)?;
                return ApiResponse::from_policy(filtered);
            }
            RedirectAction::NotRedirect => {}
        }
        let filtered = filter_response(&policy, &response).map_err(ApiError::Network)?;
        return ApiResponse::from_policy(filtered);
    }
    Err(ApiError::Network(FetchError::RedirectLimit))
}

/// Fetch policy state driven by `amazing_net::dispatch::NetEvent`. The driver
/// calls `Dispatcher::step` and routes matching events here; each hop remains
/// asynchronous. The existing `fetch_api` entry point is unchanged.
pub struct PendingFetchApi {
    preflight_partition: Option<crate::Site>,
    cors_preflight_path: bool,
    sent_referrer: Option<String>,
    policy: Request,
    wire_id: amazing_net::dispatch::RequestId,
    stage: DispatchStage,
    current: Option<crate::InternalResponse>,
    redirects: usize,
    stream_body: bool,
    cookie_header: Option<String>,
    conditional_headers: Option<crate::HeaderList>,
    request_includes_credentials: bool,
    failure_provenance: Option<crate::CspResponseProvenance>,
}
#[derive(Clone, Copy)]
enum DispatchStage {
    Preflight,
    Main,
}
impl PendingFetchApi {
    pub fn id(&self) -> amazing_net::dispatch::RequestId {
        self.wire_id
    }
    /// The exact plan used by the successful main-hop dispatch, retained before
    /// response/redirect processing mutates the policy. Preflight is separate.
    pub(crate) fn sent_referrer(&self) -> Option<&str> {
        self.sent_referrer.as_deref()
    }
    pub(crate) fn is_main(&self) -> bool {
        matches!(self.stage, DispatchStage::Main)
    }
    pub(crate) fn request_includes_credentials(&self) -> bool {
        self.is_main() && self.request_includes_credentials
    }
    pub(crate) fn policy(&self) -> &Request {
        &self.policy
    }
    pub fn start<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
        request: &mut ApiRequest,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
    ) -> Result<Self, ApiError> {
        Self::start_with_context(
            request,
            dispatcher,
            now,
            crate::SecurityContext::default(),
            crate::CspPolicyList::default(),
            crate::CspDestination::None,
            None,
            None,
        )
    }
    pub(crate) fn start_with_context<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
    >(
        request: &mut ApiRequest,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        security: crate::SecurityContext,
        csp: crate::CspPolicyList,
        csp_destination: crate::CspDestination,
        cookie_header: Option<String>,
        conditional_headers: Option<crate::HeaderList>,
    ) -> Result<Self, ApiError> {
        Self::start_with_cache(
            request,
            dispatcher,
            now,
            security,
            csp,
            csp_destination,
            cookie_header,
            conditional_headers,
            None,
        )
    }
    pub(crate) fn start_with_cache<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
    >(
        request: &mut ApiRequest,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        security: crate::SecurityContext,
        csp: crate::CspPolicyList,
        csp_destination: crate::CspDestination,
        cookie_header: Option<String>,
        conditional_headers: Option<crate::HeaderList>,
        cache: Option<&mut crate::PreflightCache>,
    ) -> Result<Self, ApiError> {
        Self::start_with_cache_progress(request,dispatcher,now,security,csp,csp_destination,cookie_header,conditional_headers,cache,&mut None,None,None)
    }
    pub(crate) fn start_with_cache_progress<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
    >(
        request: &mut ApiRequest,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        security: crate::SecurityContext,
        csp: crate::CspPolicyList,
        csp_destination: crate::CspDestination,
        cookie_header: Option<String>,
        conditional_headers: Option<crate::HeaderList>,
        mut cache: Option<&mut crate::PreflightCache>,
        progress:&mut Option<crate::CspResponseProvenance>,
        owner_service:Option<u64>,
        preflight_partition: Option<crate::Site>,
    ) -> Result<Self, ApiError> {
        if let Some(owner)=&request.embedding_owner {
            owner.validate_service(owner_service.ok_or(ApiError::Network(FetchError::FrameEmbedding))?)
                .map_err(|_|ApiError::Network(FetchError::FrameEmbedding))?;
        }
        if request.signal_aborted {
            return Err(ApiError::Aborted);
        }
        if request.body.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        let mut policy = request.policy_request()?;
        policy.security = security;
        policy.csp = csp;
        policy.csp_destination = csp_destination;
        if request.body.shared_present() && !request.body.body_used() {
            let bytes = request.body.bytes()?;
            policy
                .set_method_body(request.method(), Some(Body::Bytes(bytes)))
                .map_err(ApiError::Network)?;
        }
        let started = Self::start_hop(
            &mut policy,
            dispatcher,
            now,
            cookie_header.as_deref(),
            conditional_headers.as_ref(),
            cache.as_deref_mut(),
            preflight_partition.as_ref(),
        );
        *progress=policy.csp_pre_request_checked.then(||crate::CspResponseProvenance::checked(&policy,Vec::new()));
        if started.is_err() && policy.taint == ResponseType::Cors
            && (policy.use_cors_preflight || !crate::safelisted_method(policy.method())
                || !crate::unsafe_request_header_names(&policy.headers).is_empty()) {
            if let Some(cache) = cache {cache.clear_partitioned(&policy, preflight_partition.as_ref());}
        }
        let (wire_id,stage,sent_referrer,request_includes_credentials)=started?;
        Ok(Self {
            preflight_partition,
            cors_preflight_path: policy.taint == ResponseType::Cors && (policy.use_cors_preflight || !crate::safelisted_method(policy.method()) || !crate::unsafe_request_header_names(&policy.headers).is_empty()),
            failure_provenance:progress.clone(),
            sent_referrer,
            policy,
            wire_id,
            stage,
            current: None,
            redirects: 0,
            stream_body: request.stream_body,
            cookie_header,
            conditional_headers,
            request_includes_credentials,
        })
    }
    fn start_hop<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
        policy: &mut Request,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        cookie_header: Option<&str>,
        conditional_headers: Option<&crate::HeaderList>,
        mut cache: Option<&mut crate::PreflightCache>,
        partition: Option<&crate::Site>,
    ) -> Result<(amazing_net::dispatch::RequestId, DispatchStage, Option<String>, bool), ApiError> {
        let plan = policy.prepare().map_err(ApiError::Network)?;
        if plan.preflight
            && !cache
                .as_mut()
                .is_some_and(|cache| cache.contains_partitioned(policy, partition, now))
        {
            if let Some(preflight) = preflight(policy).map_err(ApiError::Network)? {
                let mut probe = preflight_policy(policy, preflight.headers)?;
                let probe_plan = probe.prepare().map_err(ApiError::Network)?;
                let id = submit_policy(&probe, &probe_plan, dispatcher, now, None, None)?;
                return Ok((id, DispatchStage::Preflight, None, false));
            }
        }
        Ok((
            submit_policy(
                policy,
                &plan,
                dispatcher,
                now,
                cookie_header,
                conditional_headers,
            )?,
            DispatchStage::Main,
            plan.referrer,
            plan.credentials,
        ))
    }
    /// Returns `None` until the current hop completes. A redirect or
    /// preflight schedules another hop without blocking this call.
    pub fn on_event<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
        &mut self,
        event: &amazing_net::dispatch::NetEvent,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
    ) -> Option<Result<ApiResponse, ApiError>> {
        self.on_event_with_cache(event, dispatcher, now, |_| None, |_| None, None)
    }
    pub(crate) fn on_event_with_cache<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
        F: FnMut(&Request) -> Option<String>,
        U: FnMut(&HttpUrl) -> Option<HttpUrl>,
    >(
        &mut self,
        event: &amazing_net::dispatch::NetEvent,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        cookie_for: F,
        hsts_upgrade: U,
        mut cache: Option<&mut crate::PreflightCache>,
    ) -> Option<Result<ApiResponse, ApiError>> {
        let result = self.on_event_with_cache_inner(event, dispatcher, now, cookie_for, hsts_upgrade, cache.as_deref_mut());
        if result.as_ref().is_some_and(Result::is_err) {
            if let Some(cache) = cache { self.clear_preflight_cache(cache); }
        }
        result
    }
    pub(crate) fn clear_preflight_cache(&self, cache: &mut crate::PreflightCache) {
        if self.cors_preflight_path {
            cache.clear_partitioned(&self.policy, self.preflight_partition.as_ref());
        }
    }
    fn on_event_with_cache_inner<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
        F: FnMut(&Request) -> Option<String>,
        U: FnMut(&HttpUrl) -> Option<HttpUrl>,
    >(
        &mut self,
        event: &amazing_net::dispatch::NetEvent,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        mut cookie_for: F,
        mut hsts_upgrade: U,
        mut cache: Option<&mut crate::PreflightCache>,
    ) -> Option<Result<ApiResponse, ApiError>> {
        use amazing_net::dispatch::NetEvent;
        let id = match event {
            NetEvent::Head { id, .. }
            | NetEvent::Body { id, .. }
            | NetEvent::Done { id }
            | NetEvent::Error { id, .. }
            | NetEvent::Aborted { id } => *id,
        };
        if id != self.wire_id {
            return None;
        }
        match event {
            NetEvent::Head {
                status,
                reason,
                headers,
                ..
            } => {
                let mut fields = crate::HeaderList::default();
                for (name, value) in headers.iter() {
                    if let Err(e) = fields.append(name, value) {
                        return Some(Err(ApiError::Network(e)));
                    }
                }
                self.current = Some(crate::InternalResponse {
                    request_includes_credentials: self.request_includes_credentials(),
                    status: *status,
                    status_text: reason.clone(),
                    headers: fields,
                    body: Some(Vec::new()),
                });
                None
            }
            NetEvent::Body { bytes, .. } => {
                if let Some(current) = &mut self.current {
                    current.body.as_mut().unwrap().extend_from_slice(bytes);
                }
                None
            }
            NetEvent::Error { error, .. } => {
                let mapped = match error {
                    amazing_net::dispatch::DispatchError::Network(
                        amazing_net::NetError::ResponseTooLarge,
                    ) => FetchError::ResponseTooLarge,
                    _ => FetchError::Transport,
                };
                Some(Err(ApiError::Network(mapped)))
            }
            NetEvent::Aborted { .. } => Some(Err(ApiError::Aborted)),
            NetEvent::Done { .. } => {
                let response = match self.current.take() {
                    Some(r) => r,
                    None => return Some(Err(ApiError::Network(FetchError::Transport))),
                };
                match self.stage {
                    DispatchStage::Preflight => {
                        if let Err(e) = check_preflight(&self.policy, &response) {
                            return Some(Err(ApiError::Network(e)));
                        }
                        if let Some(cache) = cache.as_mut() {
                            cache.insert_partitioned(&self.policy, &response, self.preflight_partition.as_ref(), now);
                        }
                        let plan = match self.policy.prepare() {
                            Ok(p) => p,
                            Err(e) => return Some(Err(ApiError::Network(e))),
                        };
                        match submit_policy(
                            &self.policy,
                            &plan,
                            dispatcher,
                            now,
                            self.cookie_header.as_deref(),
                            self.conditional_headers.as_ref(),
                        ) {
                            Ok(id) => {
                                self.wire_id = id;
                                self.stage = DispatchStage::Main;
                                self.request_includes_credentials = plan.credentials;
                                self.sent_referrer = plan.referrer;
                                None
                            }
                            Err(e) => Some(Err(e)),
                        }
                    }
                    DispatchStage::Main => {
                        if self.stream_body && matches!(self.policy.body(), Some(Body::Bytes(_))) {
                            let method = self.policy.method().to_owned();
                            if let Err(e) = self
                                .policy
                                .set_method_body(&method, Some(Body::Unreplayable))
                            {
                                return Some(Err(ApiError::Network(e)));
                            }
                        }
                        let metadata = NavigationResponseMetadata::capture(
                            &self.policy, &response, self.sent_referrer());
                        let location = response.headers.get("location");
                        let redirect=crate::redirect_location_with_upgrade(
                            &mut self.policy, &response, location.as_deref(),
                            |url| hsts_upgrade(&url).unwrap_or(url),
                        );
                        self.failure_provenance=Some(crate::CspResponseProvenance::checked(&self.policy,Vec::new()));
                        let outcome = match redirect {
                            Ok(RedirectAction::Follow) => {
                                // A new hop gets credentials for its own URL only.
                                self.cookie_header = cookie_for(&self.policy);
                                self.conditional_headers = None;
                                self.redirects += 1;
                                if self.redirects > 20 {
                                    return Some(Err(ApiError::Network(FetchError::RedirectLimit)));
                                }
                                self.cors_preflight_path |= self.policy.taint == ResponseType::Cors
                                    && (self.policy.use_cors_preflight || !crate::safelisted_method(self.policy.method())
                                        || !crate::unsafe_request_header_names(&self.policy.headers).is_empty());
                                match Self::start_hop(
                                    &mut self.policy,
                                    dispatcher,
                                    now,
                                    self.cookie_header.as_deref(),
                                    None,
                                    cache,
                                    self.preflight_partition.as_ref(),
                                ) {
                                    Ok((id, stage, referrer, request_includes_credentials)) => {
                                        self.wire_id = id;
                                        self.stage = stage;
                                        self.sent_referrer = referrer;
                                        self.request_includes_credentials = request_includes_credentials;
                                        None
                                    }
                                    Err(e) => Some(Err(e)),
                                }
                            }
                            Ok(RedirectAction::Manual(r)) => Some(ApiResponse::from_policy(r)),
                            Ok(RedirectAction::NavigationPending) => {
                                Some(crate::cors::filter_response_with_csp_progress(&self.policy, &response, &mut self.failure_provenance)
                                    .map_err(ApiError::Network)
                                    .and_then(ApiResponse::from_policy))
                            }
                            Ok(RedirectAction::NotRedirect) => Some(
                                crate::cors::filter_response_with_csp_progress(&self.policy, &response, &mut self.failure_provenance)
                                    .map_err(ApiError::Network)
                                    .and_then(ApiResponse::from_policy),
                            ),
                            Err(e) => Some(Err(ApiError::Network(e))),
                        };
                        outcome.map(|result| result.map(|mut response| {
                            response.navigation_metadata = metadata.map(|head| head.with_csp_receipt(response.csp_provenance.as_ref()));
                            response
                        }))
                    }
                }
            }
        }
    }
    pub(crate) fn failure_csp_provenance(&self)->Option<crate::CspResponseProvenance> {
        let final_receipts=self.failure_provenance.as_ref().map(|p|p.violations().to_vec()).unwrap_or_default();
        self.policy.csp_pre_request_checked.then(||crate::CspResponseProvenance::checked(&self.policy,final_receipts))
    }
    pub fn cancel<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
        &self,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
    ) {
        dispatcher.cancel(self.wire_id);
    }
}
/// Fetch §4.8 copies source/lineage and excludes credentials/body. Preflight
/// goes directly to HTTP-network fetch: main CSP already checked the original
/// request. Preserve that fact instead of rechecking an OPTIONS-shaped request.
fn preflight_policy(policy:&Request,headers:crate::HeaderList)->Result<Request,ApiError> {
    let mut probe=Request::new(policy.url().clone(),policy.origin.clone());
    probe.url_list=policy.url_list.clone();probe.mode=RequestMode::Cors;
    probe.credentials=CredentialsMode::Omit;probe.taint=ResponseType::Cors;
    probe.headers=headers;probe.referrer=policy.referrer.clone();probe.referrer_policy=policy.referrer_policy;
    probe.security=policy.security;probe.csp_destination=policy.csp_destination;
    probe.csp_pre_request_checked=policy.csp_pre_request_checked;
    probe.response_body_limit=policy.response_body_limit;
    probe.set_method_body("OPTIONS",None).map_err(ApiError::Network)?;Ok(probe)
}
fn submit_policy<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
    request: &Request,
    plan: &crate::FetchPlan,
    dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
    now: std::time::Instant,
    cookie_header: Option<&str>,
    conditional_headers: Option<&crate::HeaderList>,
) -> Result<amazing_net::dispatch::RequestId, ApiError> {
    let mut options = amazing_net::FetchOptions::default();
    if let Some(limit) = request.response_body_limit {
        options.max_response_bytes = limit;
    }
    options.method = request.method().to_owned();
    options.request_headers = request
        .headers
        .wire_iter()
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
    if plan.taint == ResponseType::Cors || !matches!(request.method(), "GET" | "HEAD") {
        // Origin is a singleton UA field. Preflight already contains its
        // generated value; replace it once, using the actual redirect lineage.
        options.request_headers.retain(|(name,_)|!name.eq_ignore_ascii_case("origin"));
        options.request_headers.push(("origin".into(),crate::cors_origin(request)));
    }
    if let Some(referrer) = &plan.referrer {
        options
            .request_headers
            .push(("referer".into(), referrer.clone()));
    }
    if let Some(cookie) = cookie_header.filter(|_| plan.credentials) {
        options
            .request_headers
            .push(("cookie".into(), cookie.into()));
    }
    if let Some(conditional) = conditional_headers {
        options.request_headers.extend(
            conditional
                .iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned())),
        );
    }
    if let Some(Body::Bytes(bytes)) = request.body() {
        options.body = bytes.clone();
    }
    if matches!(request.method(), "POST" | "PUT")
        && options.body.is_empty()
        && !options
            .request_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        options
            .request_headers
            .push(("content-length".into(), "0".into()));
    }
    dispatcher
        .submit(&request.url().serialize(), options, now)
        .map_err(|_| ApiError::Network(FetchError::Transport))
}

#[cfg(test)]
mod child_initiator_tests {
    use super::*;
    #[test]
    fn explicit_initiator_retains_parent_validation_and_nested_marker() {
        let parent=HttpUrl::parse("https://parent.test/").unwrap();
        let target=HttpUrl::parse("https://child.test/").unwrap();
        let initiator=Origin::opaque_with_id(211);
        let mut context=NavigableContext {top_level_origin:parent.origin().clone(),parent_origin:parent.origin().clone(),ancestor_origins:vec![parent.origin().clone()],sandbox_flags:1,frame_csp:Default::default(),referrer:parent.serialize(),referrer_policy:"strict-origin-when-cross-origin".into(),now_unix:10};
        let request=ApiRequest::child_navigation_with_initiator(target.clone(),&context,&initiator).unwrap();
        assert_eq!(request.origin(),&initiator);
        assert!(request.policy_request().unwrap().nested_navigation);
        assert_eq!(request.clone_request().unwrap().origin(),&initiator);
        assert_eq!(ApiRequest::child_navigation(target.clone(),&context).unwrap().origin(),parent.origin());
        context.ancestor_origins.clear();
        assert!(ApiRequest::child_navigation_with_initiator(target,&context,&initiator).is_err());
    }
}
