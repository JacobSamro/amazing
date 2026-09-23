//! Driver-owned Fetch state. The embedding advances this once per event-loop turn.
use crate::{
    api::{ApiCache, ApiError, ApiRequest, ApiResponse, PendingFetchApi},
    cache::{CacheKind, CacheLimits, CacheLookup, CacheRequest, CacheResponse, CacheStore},
    cookies::{CookieAccess, CookieSite, CookieStore},
    filter_response, CredentialsMode, CspDestination, CspPolicyList, FetchError, HeaderList,
    HttpUrl, InternalResponse, Origin, SecurityContext,
};
use amazing_net::dispatch::{DispatchLimits, Dispatcher, NetEvent, WebSocketEvent, WebSocketId};
use std::{collections::BTreeMap, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceId(pub u64);

#[derive(Clone)]
pub struct ClientContext {
    pub cookie_site: CookieSite,
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
impl Default for ClientContext {
    fn default() -> Self {
        Self {
            cookie_site: CookieSite::NoClient,
            security: SecurityContext::default(),
            csp: CspPolicyList::default(),
            csp_destination: CspDestination::None,
            now_unix: 0,
        }
    }
}

#[derive(Debug)]
pub enum ServiceEvent {
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
    body: Vec<u8>,
}

pub struct NetworkService {
    pub dispatcher: Dispatcher,
    pub cookies: CookieStore,
    pub cache: CacheStore,
    pub preflight_cache: crate::PreflightCache,
    next_id: u64,
    active: BTreeMap<ServiceId, Task>,
    websockets: BTreeMap<ServiceId, WebSocketId>,
    prefer_websocket: bool,
    ready: Vec<ServiceEvent>,
}
impl NetworkService {
    pub fn new(limits: DispatchLimits) -> Self {
        Self {
            dispatcher: Dispatcher::new(limits),
            cookies: CookieStore::default(),
            cache: CacheStore::new(CacheKind::Private, CacheLimits::default())
                .expect("valid cache limits"),
            preflight_cache: crate::PreflightCache::default(),
            next_id: 1,
            active: BTreeMap::new(),
            websockets: BTreeMap::new(),
            prefer_websocket: false,
            ready: Vec::new(),
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
    pub fn start(
        &mut self,
        request: ApiRequest,
        context: ClientContext,
    ) -> Result<ServiceId, ApiError> {
        self.start_at(request, context, Instant::now())
    }
    /// Deterministic clock entry point for the event-loop driver and tests.
    pub fn start_at(
        &mut self,
        mut request: ApiRequest,
        context: ClientContext,
        now: Instant,
    ) -> Result<ServiceId, ApiError> {
        if request.signal_aborted {
            return Err(ApiError::Aborted);
        }
        if request.body.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        let id = ServiceId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if request.url().as_parsed().scheme() == "data" {
            let mut policy = request.policy_request()?;
            policy.security = context.security;
            policy.csp = context.csp;
            policy.csp_destination = context.csp_destination;
            let result = policy
                .prepare()
                .and_then(|_| crate::data_url::data_response(policy.url().as_parsed()))
                .and_then(|internal| filter_response(&policy, &internal))
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
            };
            let bytes = self
                .cookies
                .cookie_header(&access, context.now_unix)
                .map_err(|_| ApiError::Network(FetchError::Transport))?;
            String::from_utf8(bytes).map_err(|_| ApiError::Network(FetchError::Transport))?
        } else {
            String::new()
        };
        let headers = request.headers.to_header_list()?;
        let cacheable = method == "GET" && request.cache == ApiCache::Default;
        let mut revalidating = false;
        let mut conditional_headers = HeaderList::default();
        if cacheable {
            let cache_request = CacheRequest {
                uri: &url,
                method: &method,
                headers: &headers,
                cookie: &cookie,
            };
            match self
                .cache
                .lookup(&cache_request, context.now_unix)
                .map_err(|_| ApiError::Network(FetchError::Transport))?
            {
                CacheLookup::Fresh(entry) => {
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
                    headers: validators,
                    ..
                } => {
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
        let pending = PendingFetchApi::start_with_cache(
            &mut request,
            &mut self.dispatcher,
            now,
            context.security,
            context.csp.clone(),
            context.csp_destination,
            (!cookie.is_empty()).then(|| cookie.clone()),
            revalidating.then_some(conditional_headers),
            Some(&mut self.preflight_cache),
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
                body: Vec::new(),
            },
        );
        Ok(id)
    }
    pub fn step(&mut self, budget: usize, now: Instant) -> Vec<ServiceEvent> {
        let mut result = std::mem::take(&mut self.ready);
        // A close event can precede the nonblocking write of its reply. Keep
        // stepping dispatcher-owned sockets after removing the service id.
        let wire_websockets = self.dispatcher.websockets_in_flight() != 0;
        let both = wire_websockets && !self.active.is_empty();
        let http_budget = if !wire_websockets {
            budget
        } else if self.active.is_empty() {
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
            let Some(id) = self
                .active
                .iter()
                .find_map(|(id, task)| (task.pending.id() == wire_id).then_some(*id))
            else {
                continue;
            };
            let mut finished = None;
            let cookies = &mut self.cookies;
            if let Some(task) = self.active.get_mut(&id) {
                if task.pending.is_main() {
                    match &event {
                        NetEvent::Head {
                            url,
                            status,
                            headers,
                            ..
                        } => {
                            let mut fields = HeaderList::default();
                            for (name, value) in headers.iter() {
                                let _ = fields.append(name, value);
                            }
                            if let Ok(parsed) = HttpUrl::parse(url) {
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
                                    };
                                    for cookie in headers.get_all("set-cookie") {
                                        let _ = cookies.set_cookie(
                                            cookie.as_bytes(),
                                            &access,
                                            task.context.now_unix,
                                        );
                                    }
                                }
                            }
                            task.head = Some((*status, fields));
                            task.body.clear();
                        }
                        NetEvent::Body { bytes, .. } if task.cacheable => {
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
                        };
                        cookies
                            .cookie_header(&access, wall_time)
                            .ok()
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                            .filter(|value| !value.is_empty())
                    },
                    Some(&mut self.preflight_cache),
                );
            }
            let wire_done = matches!(&event, NetEvent::Done { .. });
            result.push(ServiceEvent::Network { id, event });
            if let Some(mut outcome) = finished {
                if let Some(task) = self.active.remove(&id) {
                    if task.revalidating && wire_done {
                        if let Some((304, response_headers)) = &task.head {
                            let request = CacheRequest {
                                uri: &task.url,
                                method: &task.method,
                                headers: &task.headers,
                                cookie: &task.cookie,
                            };
                            outcome = self
                                .cache
                                .update_304(
                                    &request,
                                    response_headers,
                                    task.context.now_unix,
                                    task.context.now_unix,
                                )
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
                            let _ = self.cache.store(
                                &request,
                                response,
                                task.context.now_unix,
                                task.context.now_unix,
                            );
                        }
                    }
                }
                result.push(ServiceEvent::Finished {
                    id,
                    result: outcome,
                });
            }
        }
        for event in self.dispatcher.step_websockets(budget - http_budget, now) {
            let wire_id = match &event {
                WebSocketEvent::Open { id, .. }
                | WebSocketEvent::Message { id, .. }
                | WebSocketEvent::Close { id, .. }
                | WebSocketEvent::Error { id, .. } => *id,
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
        result
    }
    pub fn abort(&mut self, id: ServiceId) -> bool {
        if let Some(task) = self.active.remove(&id) {
            task.pending.cancel(&mut self.dispatcher);
            self.ready.push(ServiceEvent::Finished {
                id,
                result: Err(ApiError::Aborted),
            });
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
