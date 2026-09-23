//! Driver-owned Fetch state. The embedding advances this once per event-loop turn.
use crate::{
    api::{ApiCache, ApiError, ApiRequest, ApiResponse, PendingFetchApi},
    cache::{CacheKind, CacheLimits, CacheLookup, CacheRequest, CacheResponse, CacheStore},
    cookies::{CookieAccess, CookieSite, CookieStore},
    filter_response, CredentialsMode, CspDestination, CspPolicyList, FetchError, HeaderList,
    HttpUrl, InternalResponse, Origin, SecurityContext,
};
use amazing_net::dispatch::{DispatchLimits, Dispatcher, NetEvent};
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
    credentials: CredentialsMode,
    client_origin: Origin,
    head: Option<(u16, HeaderList)>,
    body: Vec<u8>,
}

pub struct NetworkService {
    pub dispatcher: Dispatcher,
    pub cookies: CookieStore,
    pub cache: CacheStore,
    next_id: u64,
    active: BTreeMap<ServiceId, Task>,
    ready: Vec<ServiceEvent>,
}
impl NetworkService {
    pub fn new(limits: DispatchLimits) -> Self {
        Self {
            dispatcher: Dispatcher::new(limits),
            cookies: CookieStore::default(),
            cache: CacheStore::new(CacheKind::Private, CacheLimits::default())
                .expect("valid cache limits"),
            next_id: 1,
            active: BTreeMap::new(),
            ready: Vec::new(),
        }
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
        if cacheable {
            let cache_request = CacheRequest {
                uri: &url,
                method: &method,
                headers: &headers,
                cookie: &cookie,
            };
            if let CacheLookup::Fresh(entry) =
                self.cache
                    .lookup(&cache_request, context.now_unix)
                    .map_err(|_| ApiError::Network(FetchError::Transport))?
            {
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
        }
        let pending = PendingFetchApi::start_with_context(
            &mut request,
            &mut self.dispatcher,
            now,
            context.security,
            context.csp.clone(),
            context.csp_destination,
            (!cookie.is_empty()).then(|| cookie.clone()),
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
        for event in self.dispatcher.step(budget, now) {
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
                finished = task.pending.on_event_with_cookie(
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
                );
            }
            result.push(ServiceEvent::Network { id, event });
            if let Some(outcome) = finished {
                if let Some(task) = self.active.remove(&id) {
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
    fn service_drives_network_then_serves_fresh_cache_entry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
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
            thread::yield_now();
        }
        assert!(finished);
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
            thread::yield_now();
        }
        assert!(finished);
        server.join().unwrap();
    }
}
