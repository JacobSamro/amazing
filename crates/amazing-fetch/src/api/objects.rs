use super::{usv_string, ApiBody, ApiError, ApiHeaders, BodyInput, HeaderGuard};
use crate::{
    check_preflight, filter_response, preflight, redirect_location, Body, CredentialsMode,
    FetchError, HttpUrl, NetworkOrCache, Origin, RedirectAction, RedirectMode, ReferrerPolicy,
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
    url: HttpUrl,
    method: String,
    pub headers: ApiHeaders,
    pub body: ApiBody,
    pub mode: RequestMode,
    pub credentials: CredentialsMode,
    pub cache: ApiCache,
    pub redirect: RedirectMode,
    pub referrer: String,
    pub referrer_policy: ReferrerPolicy,
    pub integrity: String,
    pub keepalive: bool,
    pub priority: ApiPriority,
    pub signal_aborted: bool,
    stream_body: bool,
    force_preflight: bool,
    origin: Origin,
}
impl ApiRequest {
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
                    method: "GET".into(),
                    headers: ApiHeaders::new(HeaderGuard::Request),
                    body: ApiBody::empty(),
                    mode: RequestMode::Cors,
                    credentials: CredentialsMode::SameOrigin,
                    cache: ApiCache::Default,
                    redirect: RedirectMode::Follow,
                    referrer: "about:client".into(),
                    referrer_policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
                    integrity: String::new(),
                    keepalive: false,
                    priority: ApiPriority::Auto,
                    signal_aborted: false,
                    stream_body: false,
                    force_preflight: false,
                    origin: origin.clone(),
                }
            }
            ApiRequestInput::Request(other) => {
                if other.body.is_unusable() {
                    return Err(ApiError::BodyUsed);
                }
                Self {
                    url: other.url.clone(),
                    method: other.method.clone(),
                    headers: other.headers.clone(),
                    body: other.body.clone_body()?,
                    mode: other.mode,
                    credentials: other.credentials,
                    cache: other.cache,
                    redirect: other.redirect,
                    referrer: other.referrer.clone(),
                    referrer_policy: other.referrer_policy,
                    integrity: other.integrity.clone(),
                    keepalive: other.keepalive,
                    priority: other.priority,
                    signal_aborted: other.signal_aborted,
                    stream_body: other.stream_body,
                    force_preflight: other.force_preflight,
                    origin: other.origin.clone(),
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
                headers.append_string(name, value)?;
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
                    request.headers.append_string("content-type", mime)?;
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
    pub fn clone_request(&self) -> Result<Self, ApiError> {
        if self.body.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        Ok(Self {
            url: self.url.clone(),
            method: self.method.clone(),
            headers: self.headers.clone(),
            body: self.body.clone_body()?,
            mode: self.mode,
            credentials: self.credentials,
            cache: self.cache,
            redirect: self.redirect,
            referrer: self.referrer.clone(),
            referrer_policy: self.referrer_policy,
            integrity: self.integrity.clone(),
            keepalive: self.keepalive,
            priority: self.priority,
            signal_aborted: self.signal_aborted,
            stream_body: self.stream_body,
            force_preflight: self.force_preflight,
            origin: self.origin.clone(),
        })
    }
    pub(crate) fn policy_request(&self) -> Result<Request, ApiError> {
        let mut out = Request::new(self.url.clone(), self.origin.clone());
        out.mode = self.mode;
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
#[derive(Debug)]
pub struct ApiResponse {
    pub response_type: ApiResponseType,
    pub url: String,
    pub redirected: bool,
    pub status: u16,
    pub status_text: String,
    pub headers: ApiHeaders,
    pub body: ApiBody,
}
impl ApiResponse {
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
                headers.append_string(n, v)?;
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
            response_type: ApiResponseType::Default,
            url: String::new(),
            redirected: false,
            status,
            status_text,
            headers,
            body,
        })
    }
    pub fn error() -> Self {
        Self {
            response_type: ApiResponseType::Error,
            url: String::new(),
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
            response_type: self.response_type,
            url: self.url.clone(),
            redirected: self.redirected,
            status: self.status,
            status_text: self.status_text.clone(),
            headers: self.headers.clone(),
            body: self.body.clone_body()?,
        })
    }
    pub(crate) fn from_policy(response: crate::Response) -> Result<Self, ApiError> {
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
            response_type,
            url,
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
                let mut probe = Request::new(policy.url().clone(), policy.origin.clone());
                probe.mode = RequestMode::Cors;
                probe.credentials = CredentialsMode::Omit;
                probe.headers = preflight.headers;
                probe
                    .set_method_body("OPTIONS", None)
                    .map_err(ApiError::Network)?;
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
                return Err(ApiError::Network(FetchError::Redirect))
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
    policy: Request,
    wire_id: amazing_net::dispatch::RequestId,
    stage: DispatchStage,
    current: Option<crate::InternalResponse>,
    redirects: usize,
    stream_body: bool,
    cookie_header: Option<String>,
    conditional_headers: Option<crate::HeaderList>,
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
    pub(crate) fn is_main(&self) -> bool {
        matches!(self.stage, DispatchStage::Main)
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
        let (wire_id, stage) = Self::start_hop(
            &mut policy,
            dispatcher,
            now,
            cookie_header.as_deref(),
            conditional_headers.as_ref(),
            cache,
        )?;
        Ok(Self {
            policy,
            wire_id,
            stage,
            current: None,
            redirects: 0,
            stream_body: request.stream_body,
            cookie_header,
            conditional_headers,
        })
    }
    fn start_hop<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
        policy: &mut Request,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        cookie_header: Option<&str>,
        conditional_headers: Option<&crate::HeaderList>,
        mut cache: Option<&mut crate::PreflightCache>,
    ) -> Result<(amazing_net::dispatch::RequestId, DispatchStage), ApiError> {
        let plan = policy.prepare().map_err(ApiError::Network)?;
        if plan.preflight
            && !cache
                .as_mut()
                .is_some_and(|cache| cache.contains(policy, now))
        {
            if let Some(preflight) = preflight(policy).map_err(ApiError::Network)? {
                let mut probe = Request::new(policy.url().clone(), policy.origin.clone());
                probe.mode = RequestMode::Cors;
                probe.credentials = CredentialsMode::Omit;
                probe.headers = preflight.headers;
                probe
                    .set_method_body("OPTIONS", None)
                    .map_err(ApiError::Network)?;
                let probe_plan = probe.prepare().map_err(ApiError::Network)?;
                let id = submit_policy(&probe, &probe_plan, dispatcher, now, None, None)?;
                return Ok((id, DispatchStage::Preflight));
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
        self.on_event_with_cache(event, dispatcher, now, |_| None, None)
    }
    pub(crate) fn on_event_with_cache<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
        F: FnMut(&Request) -> Option<String>,
    >(
        &mut self,
        event: &amazing_net::dispatch::NetEvent,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now: std::time::Instant,
        mut cookie_for: F,
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
            NetEvent::Error { .. } => Some(Err(ApiError::Network(FetchError::Transport))),
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
                            cache.insert(&self.policy, &response, now);
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
                        let location = response.headers.get("location");
                        match redirect_location(&mut self.policy, &response, location.as_deref()) {
                            Ok(RedirectAction::Follow) => {
                                // A new hop gets credentials for its own URL only.
                                self.cookie_header = cookie_for(&self.policy);
                                self.conditional_headers = None;
                                self.redirects += 1;
                                if self.redirects > 20 {
                                    return Some(Err(ApiError::Network(FetchError::RedirectLimit)));
                                }
                                match Self::start_hop(
                                    &mut self.policy,
                                    dispatcher,
                                    now,
                                    self.cookie_header.as_deref(),
                                    None,
                                    cache,
                                ) {
                                    Ok((id, stage)) => {
                                        self.wire_id = id;
                                        self.stage = stage;
                                        None
                                    }
                                    Err(e) => Some(Err(e)),
                                }
                            }
                            Ok(RedirectAction::Manual(r)) => Some(ApiResponse::from_policy(r)),
                            Ok(RedirectAction::NavigationPending) => {
                                Some(Err(ApiError::Network(FetchError::Redirect)))
                            }
                            Ok(RedirectAction::NotRedirect) => Some(
                                filter_response(&self.policy, &response)
                                    .map_err(ApiError::Network)
                                    .and_then(ApiResponse::from_policy),
                            ),
                            Err(e) => Some(Err(ApiError::Network(e))),
                        }
                    }
                }
            }
        }
    }
    pub fn cancel<R: amazing_net::dispatch::Resolver, C: amazing_net::dispatch::Connector>(
        &self,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
    ) {
        dispatcher.cancel(self.wire_id);
    }
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
    options.method = request.method().to_owned();
    options.request_headers = request
        .headers
        .iter()
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
    if let Some(referrer) = &plan.referrer {
        options
            .request_headers
            .push(("referer".into(), referrer.clone()));
    }
    if let Some(cookie) = cookie_header {
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
    dispatcher
        .submit(&request.url().serialize(), options, now)
        .map_err(|_| ApiError::Network(FetchError::Transport))
}
