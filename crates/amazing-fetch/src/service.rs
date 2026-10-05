//! Driver-owned Fetch state. The embedding advances this once per event-loop turn.
use crate::{
    api::{
        ApiCache, ApiError, ApiRequest, ApiRequestInit, ApiRequestInput, ApiResponse,
        PendingFetchApi,
    },
    cache::{CacheKind, CacheLimits, CacheLookup, CacheRequest, CacheResponse, CacheStore},
    hsts::HstsStore,
    cookies::{CookieAccess, CookieSite, CookieStore},
    filter_response, CredentialsMode, CspDestination, CspPolicyList, Destination, FetchError,
    HeaderList, HttpUrl, InternalResponse, Origin, SecurityContext, Site,
};
use amazing_net::dispatch::{DispatchLimits, Dispatcher, NetEvent, WebSocketEvent, WebSocketId};
use std::{collections::{BTreeMap, VecDeque}, io, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceId(pub u64);
/// Browser request identifier returned by `start_subresource`.
pub type RequestId = ServiceId;

/// Script-visible response metadata after Fetch policy has accepted the head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponseHead {
    /// Native policy receipts, including report-only and actual redirect lineage.
    /// Independent of opaque/script-visible filtering.
    pub csp_provenance: Option<crate::CspResponseProvenance>,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub final_url: String,
    /// Trusted redirect chain for the navigation controller.
    pub url_list: Vec<String>,
    /// An origin transition within the chain or to this response's Location.
    pub redirect_tainted: bool,
    pub response_type: crate::api::ApiResponseType,
    pub redirected: bool,
}
impl ApiResponseHead {
    fn from_response(response: &ApiResponse) -> Self {
        let chain_tainted = url_chain_tainted(&response.url_list);
        let location_tainted = response.headers.get_string("location").ok().flatten()
            .and_then(|location| HttpUrl::parse(&response.url).ok()
                .and_then(|base| base.resolve(&location).ok()
                    .map(|target| base.origin() != target.origin())))
            .unwrap_or(false);
        Self {
            csp_provenance: response.csp_provenance.clone(),
            status: response.status,
            reason: response.status_text.clone(),
            headers: response.headers.raw_entries().to_vec(),
            final_url: response.url.clone(),
            url_list: response.url_list.clone(),
            redirect_tainted: chain_tainted || location_tainted,
            response_type: response.response_type,
            redirected: response.redirected,
        }
    }
}

fn url_chain_tainted(urls: &[String]) -> bool {
    urls.windows(2).any(|pair| match (HttpUrl::parse(&pair[0]), HttpUrl::parse(&pair[1])) {
        (Ok(left), Ok(right)) => left.origin() != right.origin(),
        _ => false,
    })
}

fn policy_response_type(kind: crate::api::ApiResponseType) -> crate::ResponseType {
    match kind {
        crate::api::ApiResponseType::Default | crate::api::ApiResponseType::Basic => crate::ResponseType::Basic,
        crate::api::ApiResponseType::Cors => crate::ResponseType::Cors,
        crate::api::ApiResponseType::Opaque => crate::ResponseType::Opaque,
        crate::api::ApiResponseType::OpaqueRedirect => crate::ResponseType::OpaqueRedirect,
        crate::api::ApiResponseType::Error => crate::ResponseType::Error,
    }
}

#[derive(Clone)]
pub struct ClientContext {
    pub cookie_site: CookieSite,
    pub partition_key: Option<Site>,
    pub frame_ancestors: Vec<Origin>,
    pub security: SecurityContext,
    pub csp: CspPolicyList,
    pub csp_destination: CspDestination,
    /// UTC seconds supplied by the embedding for cookie and cache decisions.
    pub now_unix: i64,
}
#[derive(Clone)]
pub struct WebSocketClient {
    pub origin: Origin,
    pub context: ClientContext,
}
/// A CSS Fonts 4 anonymous font fetch. Relative URLs resolve against the
/// stylesheet, while the request origin remains the containing document's.
pub struct FontRequest {
    request: ApiRequest,
}
impl FontRequest {
    pub fn new(
        font_url: &str,
        stylesheet_url: &HttpUrl,
        document_origin: Origin,
        max_body_bytes: usize,
    ) -> Result<Self, ApiError> {
        let input: Vec<u16> = font_url.encode_utf16().collect();
        let referrer: Vec<u16> = stylesheet_url.serialize().encode_utf16().collect();
        let mut request = ApiRequest::new(
            ApiRequestInput::Url(&input),
            ApiRequestInit {
                mode: Some("cors".into()),
                credentials: Some("same-origin".into()),
                referrer: Some(referrer),
                ..Default::default()
            },
            stylesheet_url,
            document_origin,
        )?;
        request.response_body_limit = Some(max_body_bytes);
        Ok(Self { request })
    }
    pub fn request(&self) -> &ApiRequest {
        &self.request
    }
}
impl Default for ClientContext {
    fn default() -> Self {
        Self {
            cookie_site: CookieSite::NoClient,
            partition_key: None,
            frame_ancestors: Vec::new(),
            security: SecurityContext::default(),
            csp: CspPolicyList::default(),
            csp_destination: CspDestination::None,
            now_unix: 0,
        }
    }
}

#[derive(Debug)]
pub enum ServiceEvent {
    ReportFinished(crate::reporting::ReportFinished),
    ResponseHead { id: ServiceId, head: ApiResponseHead },
    BodyChunk { id: ServiceId, bytes: Vec<u8> },
    StreamFinished { id: ServiceId },
    StreamFailed { id: ServiceId, error: ApiError },
    /// Dispatcher event for an embedding-owned consumer such as `Xhr`.
    /// The consumer advances its pending handle with `on_dispatch_event`.
    ExternalNetwork(NetEvent),
    WebSocket {
        id: ServiceId,
        event: WebSocketEvent,
    },
    Network {
        id: ServiceId,
        event: NetEvent,
    },
    Finished {
        id: ServiceId,
        result: Result<ApiResponse, ApiError>,
    },
    /// Browser-internal result; never convert this body into a JS Response.
    SubresourceFinished { id: RequestId, response: SubresourceResponse },
    SubresourceFailed { id: RequestId, failure: SubresourceFailure },
}

/// Native terminal error and actual CSP processing receipts. None means no
/// native CSP check occurred; it is not an empty receiving-policy authority.
#[derive(Debug,Clone)]
pub struct SubresourceFailure {
    pub error: ApiError,
    pub csp_provenance: Option<crate::CspResponseProvenance>,
}
/// Validated network bytes for a trusted renderer consumer.
#[derive(Debug)]
pub struct SubresourceResponse {
    pub response_type: crate::ResponseType,
    pub head: ApiResponseHead,
    pub body: Vec<u8>,
}

struct Task {
    pending: PendingFetchApi,
    context: ClientContext,
    url: String,
    method: String,
    headers: HeaderList,
    cookie: String,
    cacheable: bool,
    revalidating: bool,
    credentials: CredentialsMode,
    client_origin: Origin,
    head: Option<(u16, HeaderList)>,
    head_reason: String,
    body: Vec<u8>,
    trusted_subresource: bool,
}

pub const INITIAL_STREAM_CREDIT: u64 = 64 * 1024;
struct StreamTask {
    credit: usize,
    head_emitted: bool,
    body_visible: bool,
    sniff_prefix: Vec<u8>,
    deferred_head: Option<(crate::Request, InternalResponse)>,
    chunks: VecDeque<Vec<u8>>,
    terminal: Option<Result<(), ApiError>>,
}
impl StreamTask {
    fn new(credit: usize) -> Self {
        Self { credit, head_emitted: false, body_visible: true,
            sniff_prefix: Vec::new(), deferred_head: None,
            chunks: VecDeque::new(), terminal: None }
    }
    fn flush(&mut self, id: ServiceId, output: &mut Vec<ServiceEvent>) -> bool {
        while self.credit > 0 {
            let Some(mut chunk) = self.chunks.pop_front() else { break; };
            let count = self.credit.min(chunk.len());
            let rest = chunk.split_off(count);
            self.credit -= count;
            output.push(ServiceEvent::BodyChunk { id, bytes: chunk });
            if !rest.is_empty() { self.chunks.push_front(rest); }
        }
        if self.chunks.is_empty() {
            if let Some(terminal) = self.terminal.take() {
                match terminal {
                    Ok(()) => output.push(ServiceEvent::StreamFinished { id }),
                    Err(error) => output.push(ServiceEvent::StreamFailed { id, error }),
                }
                return true;
            }
        }
        false
    }
}

fn validated_stream_head(task: &Task, event: &NetEvent) -> Option<ApiResponseHead> {
    let NetEvent::Head { status, reason, headers, .. } = event else { return None; };
    if !task.pending.is_main() || task.revalidating || !task.pending.policy().integrity.is_empty() {
        return None;
    }
    let navigation_redirect = (300..400).contains(status) && headers.get("location").is_some()
        && task.pending.policy().mode == crate::RequestMode::Navigate;
    if (300..400).contains(status) && headers.get("location").is_some() && !navigation_redirect {
        return None;
    }
    let mut policy = task.pending.policy().clone();
    if policy.prepare().ok()?.taint == crate::ResponseType::Opaque { return None; }
    let mut fields = HeaderList::default();
    for (name, value) in headers.iter() { fields.append(name, value).ok()?; }
    let internal = InternalResponse { status: *status, status_text: reason.clone(),
        headers: fields, body: Some(Vec::new()) };
    if navigation_redirect {
        crate::redirect_location(&mut policy, &internal, internal.headers.get("location").as_deref())
            .ok()?;
    }
    let filtered = filter_response(&policy, &internal).ok()?;
    let response = ApiResponse::from_policy(filtered).ok()?;
    Some(ApiResponseHead::from_response(&response))
}

fn opaque_stream_head(task: &Task, event: &NetEvent)
    -> Option<(crate::Request, InternalResponse)> {
    let NetEvent::Head { status, reason, headers, .. } = event else { return None; };
    if !task.pending.is_main() || task.revalidating || !task.pending.policy().integrity.is_empty() {
        return None;
    }
    if (300..400).contains(status) && headers.get("location").is_some() { return None; }
    let mut policy = task.pending.policy().clone();
    if policy.prepare().ok()?.taint != crate::ResponseType::Opaque { return None; }
    let mut fields = HeaderList::default();
    for (name, value) in headers.iter() { fields.append(name, value).ok()?; }
    Some((policy, InternalResponse { status: *status, status_text: reason.clone(),
        headers: fields, body: Some(Vec::new()) }))
}

/// The network process owns durable storage; Fetch only asks it to persist
/// the bounded policy store after a mutation.
pub trait CachePersistence {
    fn save(&self, cache: &CacheStore) -> io::Result<()>;
}
pub trait HstsPersistence {
    fn save(&self, hsts: &HstsStore, now: i64) -> io::Result<()>;
}
pub trait CookiePersistence {
    fn save(&self, cookies: &mut CookieStore<crate::psl::PublicSuffixList>, now: i64) -> io::Result<()>;
}

pub struct NetworkService {
    pub dispatcher: Dispatcher,
    pub cookies: CookieStore<crate::psl::PublicSuffixList>,
    cookie_persistence: Option<Box<dyn CookiePersistence>>,
    pub cache: CacheStore,
    pub hsts: HstsStore,
    hsts_persistence: Option<Box<dyn HstsPersistence>>,
    cache_persistence: Option<Box<dyn CachePersistence>>,
    pub preflight_cache: crate::PreflightCache,
    next_id: u64,
    active: BTreeMap<ServiceId, Task>,
    streams: BTreeMap<ServiceId, StreamTask>,
    websockets: BTreeMap<ServiceId, WebSocketId>,
    prefer_websocket: bool,
    ready: Vec<ServiceEvent>,
    pub(crate) reporting: crate::reporting::ReportingState,
}
impl NetworkService {
    /// A service result already queued for delivery is not pending I/O.
    pub fn has_pending_request_io(&self, id: ServiceId) -> bool {
        self.active.get(&id).is_some_and(|task| self.dispatcher.has_pending_request_io(task.pending.id()))
    }
    pub fn start_child_navigation(&mut self, target: HttpUrl,
        parent: &crate::api::NavigableContext) -> Result<ServiceId, ApiError> {
        let request = ApiRequest::child_navigation(target, parent)?;
        let top = parent.top_level_origin.site(&crate::psl::PublicSuffixList);
        let same_site = parent.ancestor_origins.iter()
            .all(|origin| origin.site(&crate::psl::PublicSuffixList) == top);
        let context = ClientContext {
            cookie_site: if same_site {
                CookieSite::Origin(parent.top_level_origin.clone())
            } else { CookieSite::CrossSite },
            partition_key: Some(top),
            frame_ancestors: parent.ancestor_origins.clone(),
            security: SecurityContext {
                destination: Destination::Other,
                client_restricts_mixed_content: parent.top_level_origin.serialize().starts_with("https://")
                    || parent.parent_origin.serialize().starts_with("https://"),
                top_level_navigation: false,
                ..SecurityContext::default()
            },
            csp: parent.frame_csp.clone(),
            csp_destination: CspDestination::Frame,
            now_unix: parent.now_unix,
        };
        self.start(request, context)
    }
    pub fn start_font(
        &mut self,
        font: FontRequest,
        mut context: ClientContext,
    ) -> Result<ServiceId, ApiError> {
        context.security.destination = Destination::Font;
        context.csp_destination = CspDestination::Font;
        self.start(font.request, context)
    }
    /// Browser-only subresource path. The ordinary buffered and JS Fetch paths
    /// continue to receive filtered (possibly opaque) bodies.
    pub fn start_subresource(&mut self, mut request: ApiRequest,
        mut context: ClientContext, destination: Destination) -> RequestId {
        request.destination = destination;
        request.cache = ApiCache::NoStore;
        context.security.destination = destination;
        context.csp_destination = match destination {
            Destination::Script => CspDestination::Script,
            Destination::Style => CspDestination::Style,
            Destination::Image => CspDestination::Image,
            Destination::Font => CspDestination::Font,
            Destination::Audio | Destination::Video => CspDestination::Media,
            Destination::Worker | Destination::SharedWorker | Destination::ServiceWorker => CspDestination::Worker,
            Destination::Document => CspDestination::Frame,
            Destination::Empty | Destination::Other => CspDestination::Connect,
        };
        let mut failure_provenance=None;
        let id = match self.start_at_with_csp_progress(request, context, Instant::now(),&mut failure_provenance) {
            Ok(id) => id,
            Err(error) => {
                let id = ServiceId(self.next_id);
                self.next_id = self.next_id.wrapping_add(1).max(1);
                self.ready.push(ServiceEvent::SubresourceFailed { id, failure:SubresourceFailure{error,csp_provenance:failure_provenance} });
                return id;
            }
        };
        if let Some(task) = self.active.get_mut(&id) {
            task.trusted_subresource = true;
        } else if let Some(event) = self.ready.iter_mut().find(|event|
            matches!(event, ServiceEvent::Finished { id: found, .. } if *found == id)) {
            let ServiceEvent::Finished { result, .. } = std::mem::replace(event,
                ServiceEvent::StreamFinished { id }) else {
                unreachable!()
            };
            let result = result.and_then(|mut response| {
                failure_provenance=response.csp_provenance.clone();
                let head = ApiResponseHead::from_response(&response);
                let body = response.body.bytes()?;
                Ok(SubresourceResponse { response_type: policy_response_type(response.response_type), head, body })
            });
            *event = match result {
                Ok(response) => ServiceEvent::SubresourceFinished { id, response },
                Err(error) => ServiceEvent::SubresourceFailed { id, failure:SubresourceFailure{error,csp_provenance:failure_provenance} },
            };
        }
        id
    }
    pub fn new(limits: DispatchLimits) -> Self {
        Self {
            dispatcher: Dispatcher::new(limits),
            cookies: CookieStore::new(crate::psl::PublicSuffixList, Default::default())
                .expect("valid cookie limits"),
            cookie_persistence: None,
            cache: CacheStore::new(CacheKind::Private, CacheLimits::default())
                .expect("valid cache limits"),
            hsts: HstsStore::new(),
            hsts_persistence: None,
            cache_persistence: None,
            preflight_cache: crate::PreflightCache::default(),
            next_id: 1,
            active: BTreeMap::new(),
            streams: BTreeMap::new(),
            websockets: BTreeMap::new(),
            prefer_websocket: false,
            ready: Vec::new(),
            reporting: crate::reporting::ReportingState::default(),
        }
    }
    pub(crate) fn observe_reporting_hsts(&mut self, url: &HttpUrl, headers: &HeaderList, now: i64) {
        let mut changed = false;
        for (_, value) in headers.iter().filter(|(name, _)| *name == "strict-transport-security") {
            changed |= self.hsts.observe(url, value, now);
        }
        if changed {
            if let Some(disk) = &self.hsts_persistence { let _ = disk.save(&self.hsts, now); }
        }
    }
    /// Install a restored cache and its persistence hook before requests start.
    pub fn install_cache(&mut self, cache: CacheStore, persistence: Box<dyn CachePersistence>) {
        self.cache = cache;
        self.cache_persistence = Some(persistence);
    }
    pub fn install_hsts(&mut self, hsts: HstsStore, persistence: Box<dyn HstsPersistence>) {
        self.hsts = hsts;
        self.hsts_persistence = Some(persistence);
    }
    pub fn install_cookies(&mut self,
        cookies: CookieStore<crate::psl::PublicSuffixList>, persistence: Box<dyn CookiePersistence>) {
        self.cookies = cookies;
        self.cookie_persistence = Some(persistence);
    }
    pub fn persist_cookies(&mut self, now: i64) {
        if let Some(disk) = &self.cookie_persistence {
            if let Err(error) = disk.save(&mut self.cookies, now) {
                eprintln!("cookie persistence failed: {error}");
            }
        }
    }
    fn persist_cache(&self) {
        if let Some(persistence) = &self.cache_persistence {
            if let Err(error) = persistence.save(&self.cache) {
                eprintln!("HTTP cache persistence failed: {error}");
            }
        }
    }
    pub fn open_websocket(
        &mut self,
        url: &str,
        protocols: Vec<String>,
        client: WebSocketClient,
    ) -> Result<ServiceId, FetchError> {
        self.open_websocket_at(url, protocols, client, Instant::now())
    }
    pub fn open_websocket_at(
        &mut self,
        url: &str,
        protocols: Vec<String>,
        client: WebSocketClient,
        now: Instant,
    ) -> Result<ServiceId, FetchError> {
        let parsed = amazing_net::ParsedUrl::parse(url).map_err(|_| FetchError::InvalidUrl)?;
        let scheme = match parsed.scheme() {
            "ws" => "http",
            "wss" => "https",
            _ => return Err(FetchError::InvalidUrl),
        };
        let mapped = format!("{scheme}{}", &parsed.serialize()[parsed.scheme().len()..]);
        let http_url = HttpUrl::parse(&mapped)?;
        let mut policy = crate::Request::new(http_url.clone(), client.origin.clone());
        policy.credentials = CredentialsMode::Include;
        policy.security = client.context.security;
        policy.csp = client.context.csp.clone();
        policy.csp_destination = CspDestination::Connect;
        crate::check_request_security(&policy)?;
        let csp = policy.csp.pre_websocket(&policy, &parsed);
        if csp.blocked {
            return Err(FetchError::Csp(csp.violations));
        }
        let access = CookieAccess {
            url: &http_url,
            site_for_cookies: client.context.cookie_site,
            http_api: true,
            top_level_navigation: false,
            method: "GET",
            partition_key: client.context.partition_key,
        };
        let cookie = self
            .cookies
            .cookie_header(&access, client.context.now_unix)
            .map_err(|_| FetchError::Transport)?;
        let cookie = String::from_utf8(cookie).map_err(|_| FetchError::Transport)?;
        let options = amazing_net::websocket::Options {
            origin: Some(client.origin.serialize()),
            protocols,
            ..Default::default()
        };
        let wire_id = self
            .dispatcher
            .open_websocket(
                url,
                options,
                (!cookie.is_empty()).then_some(cookie.as_str()),
                now,
            )
            .map_err(|_| FetchError::Transport)?;
        let id = ServiceId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.websockets.insert(id, wire_id);
        Ok(id)
    }
    pub fn send_websocket(
        &mut self,
        id: ServiceId,
        message: amazing_net::websocket::Event,
    ) -> Result<(), amazing_net::websocket::Error> {
        let wire = *self
            .websockets
            .get(&id)
            .ok_or(amazing_net::websocket::Error::Closed)?;
        self.dispatcher.send_websocket(wire, message)
    }
    pub fn close_websocket(
        &mut self,
        id: ServiceId,
        close: Option<amazing_net::websocket::Close>,
    ) -> Result<(), amazing_net::websocket::Error> {
        let wire = *self
            .websockets
            .get(&id)
            .ok_or(amazing_net::websocket::Error::Closed)?;
        self.dispatcher.close_websocket(wire, close)
    }
    /// Fail a WebSocket that has not yet reached `Open` (JS `close()` during
    /// CONNECTING) without a close handshake. Returns false if `id` is
    /// unknown or already terminal.
    pub fn forget_websocket(&mut self, id: ServiceId) -> bool {
        let Some(wire) = self.websockets.remove(&id) else {
            return false;
        };
        self.dispatcher.forget_websocket(wire);
        true
    }
    pub fn start(
        &mut self,
        request: ApiRequest,
        context: ClientContext,
    ) -> Result<ServiceId, ApiError> {
        self.start_at(request, context, Instant::now())
    }
    /// Start a streamed response. The first 64 KiB of decoded body has credit;
    /// callers grant more after consuming chunks. The buffered `start`/`step`
    /// contract remains available to existing consumers.
    pub fn start_stream(&mut self, request: ApiRequest, context: ClientContext)
        -> Result<ServiceId, ApiError> {
        self.start_stream_with_credit(request, context, INITIAL_STREAM_CREDIT)
    }
    /// Initial credit may be zero; response metadata and the terminal event
    /// remain visible, while body chunks wait for `grant_credit`.
    pub fn start_stream_with_credit(&mut self, request: ApiRequest,
        context: ClientContext, initial_credit: u64) -> Result<ServiceId, ApiError> {
        self.start_stream_with_credit_at(request, context, initial_credit, Instant::now())
    }
    pub fn start_stream_at(&mut self, request: ApiRequest, context: ClientContext,
        now: Instant) -> Result<ServiceId, ApiError> {
        self.start_stream_with_credit_at(request, context, INITIAL_STREAM_CREDIT, now)
    }
    pub fn start_stream_with_credit_at(&mut self, request: ApiRequest, context: ClientContext,
        initial_credit: u64, now: Instant) -> Result<ServiceId, ApiError> {
        let credit = usize::try_from(initial_credit)
            .map_err(|_| ApiError::Network(FetchError::ResponseTooLarge))?;
        let id = self.start_at(request, context, now)?;
        self.streams.insert(id, StreamTask::new(credit));
        Ok(id)
    }
    pub fn grant_credit(&mut self, id: ServiceId, bytes: usize) -> bool {
        let Some(stream) = self.streams.get_mut(&id) else { return false; };
        stream.credit = stream.credit.saturating_add(bytes);
        true
    }
    pub fn cancel(&mut self, id: ServiceId) -> bool {
        if !self.streams.contains_key(&id) { return self.abort(id); }
        if self.abort(id) { return true; }
        let Some(stream) = self.streams.get_mut(&id) else { return false; };
        stream.chunks.clear();
        stream.terminal = Some(Err(ApiError::Aborted));
        true
    }
    fn finish_stream(&mut self, id: ServiceId, result: Result<ApiResponse, ApiError>,
        output: &mut Vec<ServiceEvent>) {
        let Some(stream) = self.streams.get_mut(&id) else { return; };
        match result {
            Ok(mut response) => {
                if !stream.head_emitted {
                    output.push(ServiceEvent::ResponseHead { id,
                        head: ApiResponseHead::from_response(&response) });
                    stream.head_emitted = true;
                    match response.body.bytes() {
                        Ok(bytes) if !bytes.is_empty() => stream.chunks.push_back(bytes),
                        Ok(_) => {},
                        Err(error) => { stream.terminal = Some(Err(error)); return; }
                    }
                }
                stream.terminal = Some(Ok(()));
            }
            Err(error) => {
                stream.chunks.clear();
                stream.terminal = Some(Err(error));
            }
        }
    }
    fn flush_streams(&mut self, output: &mut Vec<ServiceEvent>) {
        let mut completed = Vec::new();
        for (id, stream) in &mut self.streams {
            if stream.flush(*id, output) { completed.push(*id); }
        }
        for id in completed { self.streams.remove(&id); }
        for (id, task) in &self.active {
            if let Some(stream) = self.streams.get(id) {
                self.dispatcher.set_paused(task.pending.id(), stream.head_emitted && stream.credit == 0);
            }
        }
    }
    /// Deterministic clock entry point for the event-loop driver and tests.
    pub fn start_at(&mut self,request:ApiRequest,context:ClientContext,now:Instant)->Result<ServiceId,ApiError> {
        self.start_at_with_csp_progress(request,context,now,&mut None)
    }
    fn start_at_with_csp_progress(
        &mut self,
        mut request: ApiRequest,
        mut context: ClientContext,
        now: Instant,
        progress:&mut Option<crate::CspResponseProvenance>,
    ) -> Result<ServiceId, ApiError> {
        if request.mode == crate::RequestMode::Navigate && context.frame_ancestors.is_empty() {
            context.security.destination = Destination::Document;
            context.security.top_level_navigation = true;
        }
        if request.signal_aborted {
            return Err(ApiError::Aborted);
        }
        if request.body.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        if let Some(upgraded) = self.hsts.upgrade(request.url(), context.now_unix) {
            request.upgrade_hsts(upgraded);
        }
        let id = ServiceId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if request.url().as_parsed().scheme() == "data" {
            let mut policy = request.policy_request()?;
            policy.security = context.security;
            policy.csp = context.csp;
            policy.csp_destination = context.csp_destination;
            let prepared=policy.prepare();
            *progress=policy.csp_pre_request_checked.then(||crate::CspResponseProvenance::checked(&policy,Vec::new()));
            let result = prepared.and_then(|_| {
                    crate::data_url::data_response_limited(
                        policy.url().as_parsed(),
                        request
                            .response_body_limit
                            .unwrap_or(crate::api::MAX_API_BODY_BYTES),
                    )
                })
                .and_then(|internal|crate::cors::filter_response_with_csp_progress(&policy,&internal,progress))
                .map_err(ApiError::Network)
                .and_then(ApiResponse::from_policy);
            self.ready.push(ServiceEvent::Finished { id, result });
            return Ok(id);
        }
        let url = request.url().serialize();
        let method = request.method().to_owned();
        let client_origin = request.origin().clone();
        let credentials = request.credentials != CredentialsMode::Omit
            && (request.credentials == CredentialsMode::Include
                || request.url().origin() == request.origin());
        let cookie = if credentials {
            let access = CookieAccess {
                url: request.url(),
                site_for_cookies: context.cookie_site.clone(),
                http_api: true,
                top_level_navigation: context.security.top_level_navigation,
                method: &method,
                partition_key: context.partition_key.clone(),
            };
            let bytes = self
                .cookies
                .cookie_header(&access, context.now_unix)
                .map_err(|_| ApiError::Network(FetchError::Transport))?;
            String::from_utf8(bytes).map_err(|_| ApiError::Network(FetchError::Transport))?
        } else {
            String::new()
        };
        if matches!(request.cache, ApiCache::NoCache | ApiCache::NoStore) {
            if request.headers.get_string("cache-control")?.is_none() {
                let value = if request.cache == ApiCache::NoCache { "max-age=0" } else { "no-cache" };
                request.headers.append_string("Cache-Control", value)?;
            }
            if request.cache == ApiCache::NoStore && request.headers.get_string("pragma")?.is_none() {
                request.headers.append_string("Pragma", "no-cache")?;
            }
        }
        let headers = request.headers.to_header_list()?;
        let cacheable = method == "GET" && matches!(request.cache,
            ApiCache::Default | ApiCache::NoCache | ApiCache::Reload);
        let mut revalidating = false;
        let mut conditional_headers = HeaderList::default();
        if cacheable && request.cache != ApiCache::Reload {
            let mut lookup_headers = headers.clone();
            if request.cache == ApiCache::NoCache {
                lookup_headers.append("Cache-Control", "no-cache").map_err(ApiError::Network)?;
            }
            let cache_request = CacheRequest {
                uri: &url,
                method: &method,
                headers: &lookup_headers,
                cookie: &cookie,
            };
            match self
                .cache
                .lookup(&cache_request, context.now_unix)
                .map_err(|_| ApiError::Network(FetchError::Transport))?
            {
                CacheLookup::Fresh(entry) => {
                    if request
                        .response_body_limit
                        .is_some_and(|limit| entry.response.body.len() > limit)
                    {
                        return Err(ApiError::Network(FetchError::ResponseTooLarge));
                    }
                    let cached = entry.response_at(context.now_unix);
                    let mut policy = request.policy_request()?;
                    policy.security = context.security;
                    policy.csp = context.csp.clone();
                    policy.csp_destination = context.csp_destination;
                    policy.prepare().map_err(ApiError::Network)?;
                    let internal = InternalResponse {
                        status: cached.status,
                        status_text: String::new(),
                        headers: cached.headers,
                        body: Some(cached.body),
                    };
                    let result = filter_response(&policy, &internal)
                        .map_err(ApiError::Network)
                        .and_then(ApiResponse::from_policy);
                    self.ready.push(ServiceEvent::Finished { id, result });
                    return Ok(id);
                }
                CacheLookup::Validate {
                    entry,
                    headers: validators,
                } => {
                    if request
                        .response_body_limit
                        .is_some_and(|limit| entry.response.body.len() > limit)
                    {
                        return Err(ApiError::Network(FetchError::ResponseTooLarge));
                    }
                    for (name, value) in validators.iter() {
                        if headers.get(name).is_none() {
                            conditional_headers
                                .append(name, value)
                                .map_err(ApiError::Network)?;
                            revalidating = true;
                        }
                    }
                }
                CacheLookup::Miss => {}
            }
        }
        let pending = PendingFetchApi::start_with_cache_progress(
            &mut request,
            &mut self.dispatcher,
            now,
            context.security,
            context.csp.clone(),
            context.csp_destination,
            (!cookie.is_empty()).then(|| cookie.clone()),
            revalidating.then_some(conditional_headers),
            Some(&mut self.preflight_cache),
            progress,
        )?;
        self.active.insert(
            id,
            Task {
                pending,
                context,
                url,
                method,
                headers,
                cookie,
                cacheable,
                revalidating,
                credentials: request.credentials,
                client_origin,
                head: None,
                head_reason: String::new(),
                body: Vec::new(),
                trusted_subresource: false,
            },
        );
        Ok(id)
    }
    pub fn step(&mut self, budget: usize, now: Instant) -> Vec<ServiceEvent> {
        let mut result = Vec::new();
        for event in std::mem::take(&mut self.ready) {
            match event {
                ServiceEvent::Finished { id, result: outcome } if self.streams.contains_key(&id) =>
                    self.finish_stream(id, outcome, &mut result),
                other => result.push(other),
            }
        }
        self.flush_streams(&mut result);
        // A close event can precede the nonblocking write of its reply. Keep
        // stepping dispatcher-owned sockets after removing the service id.
        let wire_websockets = self.dispatcher.websockets_in_flight() != 0;
        let has_http = self.dispatcher.in_flight() != 0;
        let both = wire_websockets && has_http;
        let http_budget = if !wire_websockets {
            budget
        } else if !has_http {
            0
        } else if budget == 1 {
            usize::from(!self.prefer_websocket)
        } else {
            budget / 2
        };
        if both {
            self.prefer_websocket = !self.prefer_websocket;
        }
        for event in self.dispatcher.step(http_budget, now) {
            let wire_id = match &event {
                NetEvent::Head { id, .. }
                | NetEvent::Body { id, .. }
                | NetEvent::Done { id }
                | NetEvent::Error { id, .. }
                | NetEvent::Aborted { id } => *id,
            };
            if self.on_reporting_event(&event, now, &mut result) {
                continue;
            }
            let Some(id) = self
                .active
                .iter()
                .find_map(|(id, task)| (task.pending.id() == wire_id).then_some(*id))
            else {
                if !matches!(event, NetEvent::Aborted { .. }) {
                    result.push(ServiceEvent::ExternalNetwork(event));
                }
                continue;
            };
            if let NetEvent::Head { url, status, headers, .. } = &event {
                let ancestors = &self.active[&id].context.frame_ancestors;
                let redirects = matches!(status, 301 | 302 | 303 | 307 | 308)
                    && headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("location") && !value.is_empty());
                if self.active[&id].pending.policy().mode == crate::RequestMode::Navigate
                    && !ancestors.is_empty() && !redirects {
                    let mut fields = HeaderList::default();
                    let check = headers.iter().try_for_each(|(name, value)| fields.append(name, value))
                        .and_then(|_| HttpUrl::parse(url)
                            .and_then(|parsed| crate::check_frame_embedding(&parsed, &fields, ancestors)));
                    if check.is_err() {
                        self.dispatcher.cancel(wire_id);
                        let task=self.active.remove(&id).unwrap();
                        let error = ApiError::Network(FetchError::FrameEmbedding);
                        if task.trusted_subresource {
                            result.push(ServiceEvent::SubresourceFailed{id,failure:SubresourceFailure{error,csp_provenance:task.pending.failure_csp_provenance()}});
                        } else if self.streams.contains_key(&id) {
                            self.finish_stream(id, Err(error), &mut result);
                        } else {
                            result.push(ServiceEvent::Finished { id, result: Err(error) });
                        }
                        continue;
                    }
                }
            }
            let mut finished = None;
            let cookies = &mut self.cookies;
            if let Some(task) = self.active.get_mut(&id) {
                if let Some(stream) = self.streams.get_mut(&id) {
                    if !stream.head_emitted {
                        if let Some(head) = validated_stream_head(task, &event) {
                            stream.head_emitted = true;
                            result.push(ServiceEvent::ResponseHead { id, head });
                        } else if stream.deferred_head.is_none() {
                            stream.deferred_head = opaque_stream_head(task, &event);
                        }
                    }
                    if !stream.head_emitted {
                        if let (Some((policy, internal)), NetEvent::Body { bytes, .. }) =
                            (&stream.deferred_head, &event) {
                            let needed = 1024_usize.saturating_sub(stream.sniff_prefix.len());
                            stream.sniff_prefix.extend_from_slice(&bytes[..bytes.len().min(needed)]);
                            if stream.sniff_prefix.len() == 1024 {
                                let mut probe = internal.clone();
                                probe.body = Some(stream.sniff_prefix.clone());
                                if let Ok(response) = filter_response(policy, &probe)
                                    .map_err(ApiError::Network).and_then(ApiResponse::from_policy) {
                                    let head = ApiResponseHead::from_response(&response);
                                    stream.body_visible = head.response_type != crate::api::ApiResponseType::Opaque;
                                    stream.head_emitted = true;
                                    result.push(ServiceEvent::ResponseHead { id, head });
                                }
                            }
                        }
                    }
                    if stream.head_emitted && stream.body_visible {
                        if let NetEvent::Body { bytes, .. } = &event {
                            if !bytes.is_empty() { stream.chunks.push_back(bytes.clone()); }
                        }
                    }
                }
                if task.pending.is_main() {
                    match &event {
                        NetEvent::Head {
                            url,
                            status,
                            reason,
                            headers,
                            ..
                        } => {
                            let mut fields = HeaderList::default();
                            for (name, value) in headers.iter() {
                                let _ = fields.append(name, value);
                            }
                            if let Ok(parsed) = HttpUrl::parse(url) {
                                if let Some(hsts) = headers.get_all("strict-transport-security").into_iter().next() {
                                    if self.hsts.observe(&parsed, hsts, task.context.now_unix) {
                                        if let Some(disk) = &self.hsts_persistence {
                                            let _ = disk.save(&self.hsts, task.context.now_unix);
                                        }
                                    }
                                }
                                let accept = task.credentials == CredentialsMode::Include
                                    || (task.credentials == CredentialsMode::SameOrigin
                                        && parsed.origin() == &task.client_origin);
                                if accept {
                                    let access = CookieAccess {
                                        url: &parsed,
                                        site_for_cookies: task.context.cookie_site.clone(),
                                        http_api: true,
                                        top_level_navigation: task
                                            .context
                                            .security
                                            .top_level_navigation,
                                        method: &task.method,
                                        partition_key: task.context.partition_key.clone(),
                                    };
                                    let mut changed = false;
                                    for cookie in headers.get_all("set-cookie") {
                                        changed |= cookies.set_cookie(
                                            cookie.as_bytes(),
                                            &access,
                                            task.context.now_unix,
                                        ).is_ok();
                                    }
                                    if changed {
                                        if let Some(disk) = &self.cookie_persistence {
                                            let _ = disk.save(cookies, task.context.now_unix);
                                        }
                                    }
                                }
                            }
                            task.head = Some((*status, fields));
                            task.head_reason = reason.clone();
                            task.body.clear();
                        }
                        NetEvent::Body { bytes, .. } if task.cacheable || task.trusted_subresource => {
                            task.body.extend_from_slice(bytes)
                        }
                        _ => {}
                    }
                }
                let credentials = task.credentials;
                let client_origin = task.client_origin.clone();
                let site = task.context.cookie_site.clone();
                let top_level = task.context.security.top_level_navigation;
                let wall_time = task.context.now_unix;
                let partition_key = task.context.partition_key.clone();
                finished = task.pending.on_event_with_cache(
                    &event,
                    &mut self.dispatcher,
                    now,
                    |policy| {
                        if credentials == CredentialsMode::Omit
                            || (credentials == CredentialsMode::SameOrigin
                                && policy.url().origin() != &client_origin)
                        {
                            return None;
                        }
                        let access = CookieAccess {
                            url: policy.url(),
                            site_for_cookies: site.clone(),
                            http_api: true,
                            top_level_navigation: top_level,
                            method: policy.method(),
                            partition_key: partition_key.clone(),
                        };
                        cookies
                            .cookie_header(&access, wall_time)
                            .ok()
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                            .filter(|value| !value.is_empty())
                    },
                    |url| self.hsts.upgrade(url, wall_time),
                    Some(&mut self.preflight_cache),
                );
            }
            let wire_done = matches!(&event, NetEvent::Done { .. });
            if !self.streams.contains_key(&id) { result.push(ServiceEvent::Network { id, event }); }
            if let Some(mut outcome) = finished {
                let mut subresource = None;
                let mut failure_provenance=None;
                if let Some(task) = self.active.remove(&id) {
                    failure_provenance=task.pending.failure_csp_provenance();
                    if outcome.is_ok() {
                        if let Some((status, response_headers)) = &task.head {
                            let before = self.cache.len();
                            self.cache.invalidate_unsafe(&task.method, &task.url, *status);
                            if let Ok(base) = HttpUrl::parse(&task.url) {
                                for field in ["location", "content-location"] {
                                    if let Some(value) = response_headers.get(field) {
                                        if let Ok(target) = base.resolve(&value) {
                                            if target.origin() == base.origin() {
                                                self.cache.invalidate_unsafe(
                                                    &task.method, &target.serialize(), *status);
                                            }
                                        }
                                    }
                                }
                            }
                            if self.cache.len() != before { self.persist_cache(); }
                        }
                    }
                    if task.revalidating && wire_done {
                        if let Some((304, response_headers)) = &task.head {
                            let request = CacheRequest {
                                uri: &task.url,
                                method: &task.method,
                                headers: &task.headers,
                                cookie: &task.cookie,
                            };
                            let updated = self.cache.update_304(
                                    &request,
                                    response_headers,
                                    task.context.now_unix,
                                    task.context.now_unix,
                                );
                            if updated.as_ref().is_ok_and(Option::is_some) {
                                self.persist_cache();
                            }
                            outcome = updated
                                .ok()
                                .flatten()
                                .ok_or(ApiError::Network(FetchError::Transport))
                                .and_then(|merged| {
                                    let internal = InternalResponse {
                                        status: merged.status,
                                        status_text: String::new(),
                                        headers: merged.headers,
                                        body: Some(merged.body),
                                    };
                                    filter_response(task.pending.policy(), &internal)
                                        .map_err(ApiError::Network)
                                        .and_then(ApiResponse::from_policy)
                                });
                        }
                    }
                    if task.trusted_subresource {
                        subresource = Some(outcome.as_ref().map(|response| {
                            let mut head = ApiResponseHead::from_response(response);
                            if let Some((status, fields)) = &task.head {
                                head.status = *status;
                                head.reason = task.head_reason.clone();
                                head.headers = fields.iter().filter(|(name, _)|
                                    !crate::headers::forbidden_response(name))
                                    .map(|(name, value)| (name.to_owned(), value.to_owned())).collect();
                            }
                            head.final_url = task.pending.policy().url().serialize();
                            head.url_list = task.pending.policy().url_list().iter()
                                .map(HttpUrl::serialize).collect();
                            head.redirected = head.url_list.len() > 1;
                            head.redirect_tainted = url_chain_tainted(&head.url_list);
                            SubresourceResponse {
                                response_type: policy_response_type(response.response_type),
                                head,
                                body: task.body.clone(),
                            }
                        }).map_err(Clone::clone));
                    }
                    if task.cacheable && outcome.as_ref().is_ok_and(|r| !r.redirected) {
                        if let Some((status, response_headers)) = task.head {
                            let request = CacheRequest {
                                uri: &task.url,
                                method: &task.method,
                                headers: &task.headers,
                                cookie: &task.cookie,
                            };
                            let response = CacheResponse {
                                status,
                                headers: response_headers,
                                body: task.body,
                            };
                            if self.cache.store(
                                &request,
                                response,
                                task.context.now_unix,
                                task.context.now_unix,
                            ).unwrap_or(false) {
                                self.persist_cache();
                            }
                        }
                    }
                }
                if let Some(subresource) = subresource {
                    match subresource {
                        Ok(response) => result.push(ServiceEvent::SubresourceFinished { id, response }),
                        Err(error) => result.push(ServiceEvent::SubresourceFailed { id, failure:SubresourceFailure{error,csp_provenance:failure_provenance} }),
                    }
                } else if self.streams.contains_key(&id) {
                    self.finish_stream(id, outcome, &mut result);
                } else {
                    result.push(ServiceEvent::Finished { id, result: outcome });
                }
            }
        }
        for event in self.dispatcher.step_websockets(budget - http_budget, now) {
            let wire_id = match &event {
                WebSocketEvent::Open { id, .. }
                | WebSocketEvent::Message { id, .. }
                | WebSocketEvent::Close { id, .. }
                | WebSocketEvent::Error { id, .. }
                | WebSocketEvent::Sent { id, .. } => *id,
            };
            if let Some(id) = self
                .websockets
                .iter()
                .find_map(|(id, wire)| (*wire == wire_id).then_some(*id))
            {
                if matches!(
                    event,
                    WebSocketEvent::Close { .. } | WebSocketEvent::Error { .. }
                ) {
                    self.websockets.remove(&id);
                }
                result.push(ServiceEvent::WebSocket { id, event });
            }
        }
        self.flush_streams(&mut result);
        result
    }
    pub fn abort(&mut self, id: ServiceId) -> bool {
        if let Some(task) = self.active.remove(&id) {
            task.pending.cancel(&mut self.dispatcher);
            if task.trusted_subresource {
                self.ready.push(ServiceEvent::SubresourceFailed{id,failure:SubresourceFailure{error:ApiError::Aborted,csp_provenance:task.pending.failure_csp_provenance()}});
            } else {
                self.ready.push(ServiceEvent::Finished {id,result:Err(ApiError::Aborted)});
            }
            true
        } else if let Some(index) = self.ready.iter().position(
            |event| matches!(event, ServiceEvent::Finished { id: found, .. } if *found == id),
        ) {
            self.ready.remove(index);
            self.ready.push(ServiceEvent::Finished {
                id,
                result: Err(ApiError::Aborted),
            });
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiRequestInit, ApiRequestInput};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    fn request(url: &HttpUrl) -> ApiRequest {
        let units: Vec<u16> = url.serialize().encode_utf16().collect();
        ApiRequest::new(
            ApiRequestInput::Url(&units),
            ApiRequestInit::default(),
            url,
            url.origin().clone(),
        )
        .unwrap()
    }

    #[test]
    fn embedding_headers_apply_to_navigation_not_resources_or_fetch_and_skip_redirect_heads() {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
        let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());
        let server=thread::spawn(move || {
            let deadline=Instant::now()+Duration::from_secs(8);let mut received=0;
            while received<17 && Instant::now()<deadline {
                let (mut socket,_)=match listener.accept() {
                    Ok(value)=>value,Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{thread::sleep(Duration::from_millis(1));continue;},
                    Err(error)=>panic!("accept: {error}"),
                };
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes=Vec::new();while !bytes.ends_with(b"\r\n\r\n") {let mut byte=[0];socket.read_exact(&mut byte).unwrap();bytes.push(byte[0]);}
                let line=String::from_utf8(bytes).unwrap();let path=line.split_whitespace().nth(1).unwrap();
                let response=if let Some(tail)=path.strip_prefix("/redirect/") {
                    format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: /resource/{tail}\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                } else if let Some(tail)=path.strip_prefix("/redirect-allow/") {
                    format!("HTTP/1.1 302 Found\r\nLocation: /allow/{tail}\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                } else {
                    let policy=if path.starts_with("/allow/") {""} else {"X-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\n"};
                    format!("HTTP/1.1 200 OK\r\n{policy}Content-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                };
                socket.write_all(response.as_bytes()).unwrap();received+=1;
            }
            received
        });
        let mut service=NetworkService::new(DispatchLimits::default());let mut pending=BTreeMap::new();
        for redirect in [false,true] {for (kind,destination) in [None,None,Some(Destination::Script),Some(Destination::Image),Some(Destination::Font)].into_iter().enumerate() {
            let url=HttpUrl::parse(&format!("{base}/{}/case{kind}",if redirect {"redirect"}else {"resource"})).unwrap();
            let origin=url.origin().clone();let mut request=if kind==0 {
                ApiRequest::child_navigation(url.clone(),&crate::api::NavigableContext {top_level_origin:origin.clone(),parent_origin:origin.clone(),ancestor_origins:vec![origin.clone()],
                    sandbox_flags:0,frame_csp:Default::default(),referrer:url.serialize(),referrer_policy:String::new(),now_unix:0}).unwrap()
            } else {request(&url)};
            // Child navigation is manual by default; this group explicitly
            // exercises automatic follow while preserving navigation mode.
            request.redirect=crate::RedirectMode::Follow;
            request.cache=ApiCache::NoStore;
            let context=ClientContext {frame_ancestors:vec![origin],csp_destination:CspDestination::Frame,..Default::default()};
            let id=match destination {Some(destination)=>service.start_subresource(request,context,destination),None=>service.start(request,context).unwrap()};
            pending.insert(id,kind!=0);
        }}
        let url=HttpUrl::parse(&format!("{base}/redirect-allow/navigation")).unwrap();let mut allowed=request(&url);allowed.mode=crate::RequestMode::Navigate;allowed.cache=ApiCache::NoStore;
        let id=service.start(allowed,ClientContext {frame_ancestors:vec![url.origin().clone()],..Default::default()}).unwrap();pending.insert(id,true);
        let deadline=Instant::now()+Duration::from_secs(6);
        while !pending.is_empty() && Instant::now()<deadline {
            for event in service.step(8192,Instant::now()) {
                let (id,result)=match event {
                    ServiceEvent::Finished {id,result}=>(id,result.and_then(|mut response|response.body.bytes())),
                    ServiceEvent::SubresourceFinished {id,response}=>(id,Ok(response.body)),
                    ServiceEvent::SubresourceFailed {id,failure}=>(id,Err(failure.error)),_=>continue,
                };
                let expected=pending.remove(&id).expect("one terminal result per request");
                if expected {assert_eq!(result.unwrap(),b"ok");}
                else {assert!(matches!(result,Err(ApiError::Network(FetchError::FrameEmbedding))),"{result:?}");}
            }
            if !pending.is_empty() {thread::sleep(Duration::from_millis(1));}
        }
        assert!(pending.is_empty(),"pending: {pending:?}");assert_eq!(server.join().unwrap(),17);
    }
    #[test]
    fn websocket_checks_connect_src_and_mixed_content_before_dispatch() {
        let origin = HttpUrl::parse("https://client.example/")
            .unwrap()
            .origin()
            .clone();
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut headers = HeaderList::default();
        headers
            .append("content-security-policy", "connect-src 'none'")
            .unwrap();
        let blocked = WebSocketClient {
            origin: origin.clone(),
            context: ClientContext {
                csp: CspPolicyList::from_headers(&headers).unwrap(),
                ..ClientContext::default()
            },
        };
        assert!(matches!(
            service.open_websocket("wss://example.com/", vec![], blocked),
            Err(FetchError::Csp(_))
        ));
        let mixed = WebSocketClient {
            origin: origin.clone(),
            context: ClientContext {
                security: SecurityContext {
                    client_restricts_mixed_content: true,
                    ..SecurityContext::default()
                },
                ..ClientContext::default()
            },
        };
        assert!(matches!(
            service.open_websocket("ws://example.com/", vec![], mixed),
            Err(FetchError::MixedContent)
        ));
        headers = HeaderList::default();
        headers
            .append("content-security-policy", "connect-src wss:")
            .unwrap();
        let allowed = WebSocketClient {
            origin,
            context: ClientContext {
                csp: CspPolicyList::from_headers(&headers).unwrap(),
                ..ClientContext::default()
            },
        };
        assert!(service
            .open_websocket("wss://example.com/", vec![], allowed)
            .is_ok());
    }

    #[test]
    fn websocket_handshake_uses_cookie_jar() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            socket
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let url = HttpUrl::parse(&format!("http://{address}/socket")).unwrap();
        let mut service = NetworkService::new(DispatchLimits::default());
        service
            .cookies
            .set_cookie(
                b"sid=abc; Path=/",
                &CookieAccess {
                    url: &url,
                    site_for_cookies: CookieSite::NoClient,
                    http_api: true,
                    top_level_navigation: false,
                    method: "GET",
                    partition_key: None,
                },
                100,
            )
            .unwrap();
        let client = WebSocketClient {
            origin: url.origin().clone(),
            context: ClientContext {
                now_unix: 100,
                ..ClientContext::default()
            },
        };
        let now = Instant::now();
        let id = service
            .open_websocket_at(&format!("ws://{address}/socket"), vec![], client, now)
            .unwrap();
        let mut errored = false;
        for tick in 0..1000 {
            if service.step(4096, now + Duration::from_millis(tick)).iter().any(|event| matches!(event, ServiceEvent::WebSocket { id: found, event: WebSocketEvent::Error { .. } } if *found == id)) {
                errored = true; break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(errored);
        let request = server.join().unwrap();
        assert!(request.contains("Cookie: sid=abc\r\n"));
    }

    #[test]
    fn service_drives_dispatcher_socket_after_service_id_is_gone() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let now = Instant::now();
        service
            .dispatcher
            .open_websocket(
                &format!("ws://127.0.0.1:{port}/"),
                amazing_net::websocket::Options::default(),
                None,
                now,
            )
            .unwrap();
        for tick in 0..1000 {
            service.step(1, now + Duration::from_millis(tick));
            if service.dispatcher.websockets_in_flight() == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(service.dispatcher.websockets_in_flight(), 0);
        server.join().unwrap();
    }

    #[test]
    fn service_uses_preflight_grants_until_expiry_and_separates_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!(
            "http://127.0.0.1:{}/resource",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let origin = HttpUrl::parse("http://client.example/")
            .unwrap()
            .origin()
            .clone();
        let server = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut methods = Vec::new();
            while methods.len() < 7 && Instant::now() < deadline {
                let (mut socket, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("accept: {error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let text = String::from_utf8(request).unwrap();
                let method = text.split_once(' ').unwrap().0.to_owned();
                let response = if method == "OPTIONS" {
                    "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: http://client.example\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: x-test\r\nAccess-Control-Max-Age: 2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: http://client.example\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                socket.write_all(response.as_bytes()).unwrap();
                methods.push(method);
            }
            methods
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let initial = Instant::now();
        for (index, (credentials, offset)) in [
            (CredentialsMode::Omit, 0),
            (CredentialsMode::Omit, 1),
            (CredentialsMode::Include, 1),
            (CredentialsMode::Include, 4),
        ]
        .into_iter()
        .enumerate()
        {
            let units: Vec<u16> = url.serialize().encode_utf16().collect();
            let mut request = ApiRequest::new(
                ApiRequestInput::Url(&units),
                ApiRequestInit {
                    method: Some("PUT".encode_utf16().collect()),
                    ..ApiRequestInit::default()
                },
                &url,
                origin.clone(),
            )
            .unwrap();
            request.credentials = credentials;
            request
                .headers
                .append(
                    &"x-test".encode_utf16().collect::<Vec<_>>(),
                    &"yes".encode_utf16().collect::<Vec<_>>(),
                )
                .unwrap();
            let now = initial + Duration::from_secs(offset);
            let id = service
                .start_at(request, ClientContext::default(), now)
                .unwrap();
            let mut finished = false;
            for tick in 0..2000 {
                for event in service.step(8192, now + Duration::from_millis(tick)) {
                    if let ServiceEvent::Finished { id: found, result } = event {
                        assert_eq!(found, id, "request {index}");
                        assert_eq!(result.unwrap().status, 200);
                        finished = true;
                    }
                }
                if finished {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            assert!(finished, "request {index}");
        }
        assert_eq!(
            server.join().unwrap(),
            ["OPTIONS", "PUT", "PUT", "OPTIONS", "PUT", "OPTIONS", "PUT"]
        );
    }

    #[test]
    fn service_drives_network_then_serves_fresh_cache_entry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (release, hold) = std::sync::mpsc::channel();
        let url = HttpUrl::parse(&format!(
            "http://127.0.0.1:{}/cached",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = [0; 2048];
            socket.read(&mut bytes).unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nCache-Control: max-age=60\r\n\r\nok",
                )
                .unwrap();
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let context = ClientContext {
            now_unix: 100,
            ..ClientContext::default()
        };
        let now = Instant::now();
        let first = service
            .start_at(request(&url), context.clone(), now)
            .unwrap();
        let mut finished = false;
        for tick in 0..2000 {
            for event in service.step(8192, now + Duration::from_millis(tick)) {
                if let ServiceEvent::Finished { id, result } = event {
                    assert_eq!(id, first);
                    assert_eq!(result.unwrap().status, 200);
                    finished = true;
                }
            }
            if finished {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(finished);
        release.send(()).unwrap();
        server.join().unwrap();
        let second = service.start_at(request(&url), context, now).unwrap();
        assert!(service.step(1, now).into_iter().any(|event| matches!(event, ServiceEvent::Finished { id, result: Ok(response) } if id == second && response.status == 200)));
        let mut aborted = request(&url);
        aborted.signal_aborted = true;
        assert!(matches!(
            service.start_at(
                aborted,
                ClientContext {
                    now_unix: 100,
                    ..ClientContext::default()
                },
                now
            ),
            Err(ApiError::Aborted)
        ));
    }

    #[test]
    fn service_recomputes_cookie_for_redirect_hop() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!(
            "http://127.0.0.1:{}/first",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 2048];
            socket.read(&mut bytes).unwrap();
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /second\r\nSet-Cookie: x=y; Path=/\r\nContent-Length: 0\r\n\r\n").unwrap();
            let n = socket.read(&mut bytes).unwrap();
            let second = String::from_utf8_lossy(&bytes[..n]).to_ascii_lowercase();
            assert!(second.starts_with("get /second "));
            assert!(second.contains("cookie: x=y\r\n"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let now = Instant::now();
        let id = service
            .start_at(
                request(&url),
                ClientContext {
                    now_unix: 100,
                    ..ClientContext::default()
                },
                now,
            )
            .unwrap();
        let mut finished = false;
        for tick in 0..5000 {
            for event in service.step(8192, now + Duration::from_millis(tick)) {
                if let ServiceEvent::Finished { id: found, result } = event {
                    assert_eq!(found, id);
                    assert_eq!(result.unwrap().status, 200);
                    finished = true;
                }
            }
            if finished {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(finished);
        server.join().unwrap();
    }

    #[test]
    fn service_revalidates_304_and_reuses_stored_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!(
            "http://127.0.0.1:{}/etag",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let (release, hold) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 2048];
            let first = socket.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..first]).starts_with("GET /etag "));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nCache-Control: max-age=0\r\nETag: \"v1\"\r\n\r\nbody").unwrap();
            let second = socket.read(&mut bytes).unwrap();
            let conditional = String::from_utf8_lossy(&bytes[..second]).to_ascii_lowercase();
            assert!(
                conditional.contains("if-none-match: \"v1\"\r\n"),
                "{conditional}"
            );
            socket.write_all(b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nETag: \"v1\"\r\n\r\n").unwrap();
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let now = Instant::now();
        for (wall, expected_body) in [
            (100, b"body".as_slice()),
            (101, b"body".as_slice()),
            (102, b"body".as_slice()),
        ] {
            let mut incoming = request(&url);
            if wall == 101 {
                incoming.mode = crate::RequestMode::NoCors;
            }
            let id = service
                .start_at(
                    incoming,
                    ClientContext {
                        now_unix: wall,
                        ..ClientContext::default()
                    },
                    now,
                )
                .unwrap();
            let mut finished = false;
            for tick in 0..5000 {
                for event in service.step(8192, now + Duration::from_millis(tick)) {
                    if let ServiceEvent::Finished { id: found, result } = event {
                        assert_eq!(found, id);
                        let mut response = result.unwrap_or_else(|error| {
                            panic!("wall={wall} cache={} error={error:?}", service.cache.len())
                        });
                        assert_eq!(response.status, 200);
                        assert_eq!(response.body.bytes().unwrap(), expected_body);
                        finished = true;
                    }
                }
                if finished {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            assert!(finished);
        }
        release.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn service_serves_data_without_dispatch() {
        let base = HttpUrl::parse("https://example.test/").unwrap();
        let input: Vec<u16> = "data:text/plain,hello".encode_utf16().collect();
        let request = ApiRequest::new(
            ApiRequestInput::Url(&input),
            ApiRequestInit {
                mode: Some("cors".into()),
                ..ApiRequestInit::default()
            },
            &base,
            base.origin().clone(),
        )
        .unwrap();
        let mut service = NetworkService::new(DispatchLimits::default());
        let id = service
            .start_at(request, ClientContext::default(), Instant::now())
            .unwrap();
        let events = service.step(1, Instant::now());
        assert_eq!(events.len(), 1);
        match events.into_iter().next().unwrap() {
            ServiceEvent::Finished { id: found, result } => {
                assert_eq!(found, id);
                let mut response = result.unwrap();
                assert_eq!(response.status, 200);
                assert_eq!(response.body.bytes().unwrap(), b"hello");
            }
            _ => panic!("unexpected wire event"),
        }
    }
}
