//! The CDP server: a localhost TCP listener, HTTP discovery, and the WebSocket
//! connection that routes commands and streams events.
//!
//! One thread handles each connection. A connection owns either the browser
//! endpoint (`/devtools/browser/<id>`, for `Target.*`/`Browser.*` and flattened
//! sessions created by `Target.attachToTarget`) or a page endpoint
//! (`/devtools/page/<id>`, bound directly to one [`DevToolsSession`]).
//!
//! Discovery follows Chrome's shape: `GET /json/version` and `GET /json/list`
//! advertise the WebSocket URLs, and `GET /json/list` reports one entry per
//! page target.

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domains::target;
use crate::host::{DevToolsHost, TargetEventKind, TargetInfo};
use crate::http::{self, HttpRequest, HttpResponse};
use crate::json::Json;
use crate::protocol::{self, CdpError, CdpRequest};
use crate::session::{self, DispatchOutcome, SessionState};
use crate::ws::{Message, WsConnection, WsError};

/// Tunables for the listener and each connection.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Cap on the HTTP request head.
    pub max_headers: usize,
    /// Cap on one WebSocket frame.
    pub max_frame: usize,
    /// Cap on a reassembled WebSocket message.
    pub max_message: usize,
    /// How long a connection waits for the next inbound byte before polling
    /// engine events.
    pub read_timeout: Duration,
    /// Cap on simultaneously handled connections.
    pub max_connections: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_headers: http::DEFAULT_MAX_HEADERS,
            max_frame: crate::ws::DEFAULT_MAX_FRAME,
            max_message: crate::ws::DEFAULT_MAX_MESSAGE,
            read_timeout: Duration::from_millis(25),
            max_connections: 64,
        }
    }
}

/// A bound CDP server. Call [`CdpServer::serve`] on its own thread.
pub struct CdpServer {
    listener: TcpListener,
    host: Arc<dyn DevToolsHost>,
    config: ServerConfig,
    browser_id: String,
}

impl CdpServer {
    /// Binds `addr` (for example `"127.0.0.1:0"` for an ephemeral port).
    pub fn bind(host: Arc<dyn DevToolsHost>, addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        Ok(Self::from_listener(listener, host))
    }

    /// Binds an explicit address.
    pub fn bind_addr(host: Arc<dyn DevToolsHost>, addr: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        Ok(Self::from_listener(listener, host))
    }

    fn from_listener(listener: TcpListener, host: Arc<dyn DevToolsHost>) -> Self {
        let browser_id = format!(
            "{:016x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        );
        Self {
            listener,
            host,
            config: ServerConfig::default(),
            browser_id,
        }
    }

    /// Replaces the default configuration.
    pub fn with_config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// The bound address, with the ephemeral port resolved.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The id advertised in `/json/version`'s browser WebSocket URL.
    pub fn browser_id(&self) -> &str {
        &self.browser_id
    }

    /// Accepts connections until `shutdown` is set.
    pub fn serve(self, shutdown: Arc<AtomicBool>) -> io::Result<()> {
        self.listener.set_nonblocking(true)?;
        let active = Arc::new(AtomicUsize::new(0));
        let mut next_thread = 0u64;
        loop {
            if shutdown.load(Ordering::Relaxed) {
                return Ok(());
            }
            match self.listener.accept() {
                Ok((stream, _peer)) => {
                    if active.fetch_add(1, Ordering::AcqRel) >= self.config.max_connections {
                        active.fetch_sub(1, Ordering::AcqRel);
                        continue;
                    }
                    next_thread += 1;
                    let host = Arc::clone(&self.host);
                    let config = self.config.clone();
                    let worker_active = Arc::clone(&active);
                    let name = format!("cdp-{next_thread}");
                    let spawned = thread::Builder::new().name(name).spawn(move || {
                        let _ = handle_connection(stream, host, config);
                        worker_active.fetch_sub(1, Ordering::AcqRel);
                    });
                    if spawned.is_err() {
                        active.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    host: Arc<dyn DevToolsHost>,
    config: ServerConfig,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(config.read_timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut stream = stream;

    let request = match http::read_request(&mut stream, config.max_headers) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(http::HttpError::Limit) => {
            return http::write_response(&mut stream, &HttpResponse::text(400, "request headers too large"));
        }
        Err(_) => {
            return http::write_response(&mut stream, &HttpResponse::text(400, "bad request"));
        }
    };
    if request.method != "GET" {
        return http::write_response(&mut stream, &HttpResponse::empty(405));
    }
    if request.header_has_token("upgrade", "websocket") {
        return handle_websocket(stream, request, host, config);
    }
    handle_discovery(&mut stream, &request, host.as_ref())
}

fn handle_discovery(
    stream: &mut TcpStream,
    request: &HttpRequest,
    host: &dyn DevToolsHost,
) -> io::Result<()> {
    let base = base_authority(request, stream.local_addr().ok());
    let response = match request.path() {
        "/json/version" => HttpResponse::json(version_body(&host.info(), &base)),
        "/json" | "/json/" | "/json/list" | "/json/list/" => {
            HttpResponse::json(list_body(&host.list_targets(), &base))
        }
        _ => HttpResponse::text(404, "not found"),
    };
    http::write_response(stream, &response)
}

fn handle_websocket(
    mut stream: TcpStream,
    request: HttpRequest,
    host: Arc<dyn DevToolsHost>,
    config: ServerConfig,
) -> io::Result<()> {
    if !request.header_has_token("connection", "upgrade")
        || !request
            .header("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    {
        return http::write_response(&mut stream, &HttpResponse::text(400, "not a websocket upgrade"));
    }
    let key = match websocket_key(&request) {
        Ok(key) => key,
        Err(reason) => return http::write_response(&mut stream, &HttpResponse::text(400, reason)),
    };

    // Resolve the endpoint before completing the handshake so a bad path gets
    // an honest HTTP error rather than an upgrade that immediately closes.
    let path = request.path();
    let (default_session, starting_target) = if let Some(target_id) = path.strip_prefix("/devtools/page/") {
        if target_id.is_empty() {
            return http::write_response(&mut stream, &HttpResponse::empty(404));
        }
        let info = host
            .list_targets()
            .into_iter()
            .find(|target| target.id == target_id)
            .unwrap_or_else(|| TargetInfo::page(target_id.to_string(), "", ""));
        (true, Some(info))
    } else if path == "/devtools/browser" || path.starts_with("/devtools/browser/") {
        (false, None)
    } else {
        return http::write_response(&mut stream, &HttpResponse::empty(404));
    };

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        crate::sha1::websocket_accept(&key)
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;

    let ws = WsConnection::with_limits(stream, config.max_frame, config.max_message);
    let mut connection = Connection::new(Arc::clone(&host), ws);
    if default_session {
        let info = starting_target.unwrap_or_else(|| TargetInfo::page("", "", ""));
        match host.attach(&info.id) {
            Ok(dev) => {
                let session_id = "default".to_string();
                let session = SessionState::new(session_id.clone(), info, dev);
                connection.insert_session(session, true);
            }
            Err(_) => {
                // Attach failed: let the client's first command produce the
                // error. This mirrors Chrome attaching lazily.
            }
        }
    }
    connection.run();
    Ok(())
}

/// Validates and returns the `Sec-WebSocket-Key`.
pub fn websocket_key(request: &HttpRequest) -> Result<String, &'static str> {
    if request.header("sec-websocket-version") != Some("13") {
        return Err("unsupported websocket version");
    }
    let key = request
        .header("sec-websocket-key")
        .ok_or("missing Sec-WebSocket-Key")?;
    if key.len() != 24 {
        return Err("invalid Sec-WebSocket-Key");
    }
    match crate::base64::decode(key) {
        Ok(bytes) if bytes.len() == 16 => Ok(key.to_string()),
        _ => Err("invalid Sec-WebSocket-Key"),
    }
}

fn base_authority(request: &HttpRequest, fallback: Option<SocketAddr>) -> String {
    if let Some(host) = request.header("host") {
        if !host.is_empty() {
            return host.to_string();
        }
    }
    fallback
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

fn version_body(info: &crate::host::HostInfo, base: &str) -> String {
    Json::Object(vec![
        ("Browser".into(), Json::String(info.product.clone())),
        (
            "Protocol-Version".into(),
            Json::String(info.protocol_version.clone()),
        ),
        ("User-Agent".into(), Json::String(info.user_agent.clone())),
        ("V8-Version".into(), Json::String(info.js_version.clone())),
        ("WebKit-Version".into(), Json::String(info.revision.clone())),
        (
            "webSocketDebuggerUrl".into(),
            Json::String(format!("ws://{base}/devtools/browser")),
        ),
    ])
    .to_string()
}

fn list_body(targets: &[TargetInfo], base: &str) -> String {
    let entries: Vec<Json> = targets
        .iter()
        .filter(|target| target.target_type != "browser")
        .map(|target| {
            Json::Object(vec![
                ("description".into(), Json::String(String::new())),
                (
                    "devtoolsFrontendUrl".into(),
                    Json::String(format!("/devtools/inspector.html?ws={base}/devtools/page/{}", target.id)),
                ),
                ("id".into(), Json::String(target.id.clone())),
                ("title".into(), Json::String(target.title.clone())),
                ("type".into(), Json::String(target.target_type.clone())),
                ("url".into(), Json::String(target.url.clone())),
                (
                    "webSocketDebuggerUrl".into(),
                    Json::String(format!("ws://{base}/devtools/page/{}", target.id)),
                ),
            ])
        })
        .collect();
    Json::Array(entries).to_string()
}

/// A live WebSocket connection and the sessions attached to it.
struct Connection {
    host: Arc<dyn DevToolsHost>,
    ws: WsConnection<TcpStream>,
    sessions: HashMap<String, SessionState>,
    order: Vec<String>,
    default_session: Option<String>,
    next_session: u64,
    /// `Target.setDiscoverTargets` state.
    discover_targets: bool,
    /// `Target.setAutoAttach` state: attach a flattened session per new target.
    auto_attach: bool,
}

impl Connection {
    fn new(host: Arc<dyn DevToolsHost>, ws: WsConnection<TcpStream>) -> Self {
        Self {
            host,
            ws,
            sessions: HashMap::new(),
            order: Vec::new(),
            default_session: None,
            next_session: 0,
            discover_targets: false,
            auto_attach: false,
        }
    }

    fn insert_session(&mut self, session: SessionState, is_default: bool) {
        if is_default {
            self.default_session = Some(session.session_id.clone());
        }
        self.order.push(session.session_id.clone());
        self.sessions.insert(session.session_id.clone(), session);
    }

    fn run(&mut self) {
        loop {
            match self.ws.poll() {
                Ok(Some(Message::Text(text))) => {
                    self.dispatch_message(&text);
                    self.pump();
                }
                Ok(Some(Message::Binary(_))) => {
                    // CDP is text-only; ignore binary payloads.
                    self.pump();
                }
                Ok(None) => self.pump(),
                Err(WsError::Closed { .. }) | Err(WsError::Eof) => break,
                Err(_) => {
                    let _ = self.ws.write_close(1002, "protocol error");
                    break;
                }
            }
        }
    }

    /// Emits pending target-lifecycle and engine events.
    fn pump(&mut self) {
        self.emit_target_events();
        self.emit_events();
    }

    fn dispatch_message(&mut self, text: &str) {
        match CdpRequest::parse(text) {
            Ok(request) => self.execute(request),
            Err(error) => {
                let id = extract_id(text);
                let value = protocol::error_response(&id, &error);
                self.write_json(&value);
            }
        }
    }

    fn execute(&mut self, request: CdpRequest) {
        let response_tag = request.session_id.clone();
        let outcome = self.route(&request);
        if request.id.is_none() {
            if let Ok(outcome) = outcome {
                for event in outcome.events {
                    self.write_tagged(event, response_tag.as_deref());
                }
            }
            return;
        }
        let id = request.id.clone().unwrap_or(Json::Null);
        match outcome {
            Ok(outcome) => {
                self.write_tagged(
                    protocol::response(&id, outcome.result),
                    response_tag.as_deref(),
                );
                for event in outcome.events {
                    self.write_tagged(event, response_tag.as_deref());
                }
            }
            Err(error) => {
                self.write_tagged(protocol::error_response(&id, &error), response_tag.as_deref());
            }
        }
    }

    fn route(&mut self, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
        if request.method.starts_with("Target.") || request.method.starts_with("Browser.") {
            return self.host_method(request);
        }
        let session_id = request
            .session_id
            .clone()
            .or_else(|| self.default_session.clone())
            .ok_or_else(|| {
                CdpError::server(
                    "This connection is not attached to a page; use /devtools/page/<targetId>",
                )
            })?;
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| CdpError::server("No session with given sessionId"))?;
        session::dispatch(session, request)
    }

    fn host_method(&mut self, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
        match request.method.as_str() {
            "Target.getTargets" => {
                let targets = self.host.list_targets();
                let infos: Vec<Json> = targets
                    .iter()
                    .map(|info| {
                        let mut info = info.clone();
                        info.attached = self
                            .sessions
                            .values()
                            .any(|session| session.target.id == info.id);
                        target::target_info(&info)
                    })
                    .collect();
                Ok(DispatchOutcome::result(crate::domains::object(vec![(
                    "targetInfos",
                    Json::Array(infos),
                )])))
            }
            "Target.getTargetInfo" => {
                let info = self.resolve_target(request.optional_string("targetId")?)?;
                Ok(DispatchOutcome::result(crate::domains::object(vec![(
                    "targetInfo",
                    target::target_info(&info),
                )])))
            }
            "Target.attachToTarget" => {
                let target_id = request.string_param("targetId")?.to_string();
                let flatten = request.optional_bool("flatten", false)?;
                if !flatten {
                    return Err(CdpError::invalid_params(
                        "only flattened sessions are supported (pass flatten: true)",
                    ));
                }
                let info = self.resolve_target(Some(&target_id))?;
                let dev = self.host.attach(&target_id).map_err(CdpError::from)?;
                self.next_session += 1;
                let session_id = format!("S{:x}", self.next_session);
                let session = SessionState::new(session_id.clone(), info, dev);
                self.insert_session(session, false);
                Ok(DispatchOutcome::result(crate::domains::object(vec![(
                    "sessionId",
                    Json::String(session_id),
                )])))
            }
            "Target.activateTarget" => {
                let _ = self.resolve_target(request.optional_string("targetId")?)?;
                Ok(DispatchOutcome::empty())
            }
            "Target.closeTarget" => Ok(DispatchOutcome::result(crate::domains::object(vec![(
                "success",
                Json::Bool(false),
            )]))),
            "Target.setDiscoverTargets" => {
                let discover = request.optional_bool("discover", false)?;
                self.discover_targets = discover;
                let mut events = Vec::new();
                if discover {
                    for target in self.host.list_targets() {
                        events.push(target::created_event(&target));
                    }
                }
                Ok(DispatchOutcome::empty().with_events(events))
            }
            "Target.getBrowserContexts" => Ok(DispatchOutcome::result(crate::domains::object(vec![(
                "browserContextIds",
                Json::Array(Vec::new()),
            )]))),
            "Target.setAutoAttach" => {
                let auto_attach = request.optional_bool("autoAttach", false)?;
                let _wait = request.optional_bool("waitForDebuggerOnStart", false)?;
                self.auto_attach = auto_attach;
                Ok(DispatchOutcome::empty())
            }
            "Browser.getVersion" => Ok(DispatchOutcome::result(target::browser_version(
                &self.host.info(),
            ))),
            "Browser.getBrowserCommandLine" => Ok(DispatchOutcome::result(
                crate::domains::object(vec![("arguments", Json::Array(Vec::new()))]),
            )),
            other => Err(CdpError::method_not_found(other)),
        }
    }

    fn resolve_target(&self, target_id: Option<&str>) -> Result<TargetInfo, CdpError> {
        let targets = self.host.list_targets();
        if let Some(id) = target_id {
            return targets
                .into_iter()
                .find(|target| target.id == id)
                .ok_or_else(|| CdpError::server(format!("No target with id '{id}'")));
        }
        // No id given: prefer the default session's target, then the first page.
        if let Some(default) = &self.default_session {
            if let Some(session) = self.sessions.get(default) {
                return Ok(session.target.clone());
            }
        }
        targets
            .into_iter()
            .find(|target| target.target_type == "page")
            .ok_or_else(|| CdpError::server("No page target available"))
    }

    /// Emits `Target.*` lifecycle events from the host and auto-attaches
    /// flattened sessions when `Target.setAutoAttach(true)` is active.
    fn emit_target_events(&mut self) {
        let events = self.host.drain_target_events();
        for event in events {
            match event.kind {
                TargetEventKind::Created => {
                    if self.discover_targets {
                        self.write_json(&target::created_event(&event.target));
                    }
                    if self.auto_attach && self.session_for_target(&event.target.id).is_none() {
                        if let Ok(dev) = self.host.attach(&event.target.id) {
                            self.next_session += 1;
                            let session_id = format!("S{:x}", self.next_session);
                            let session =
                                SessionState::new(session_id.clone(), event.target.clone(), dev);
                            self.insert_session(session, false);
                            self.write_json(&target::attached_event(
                                &session_id,
                                &event.target,
                                false,
                            ));
                        }
                    }
                }
                TargetEventKind::InfoChanged => {
                    if self.discover_targets {
                        self.write_json(&target::info_changed_event(&event.target));
                    }
                }
                TargetEventKind::Destroyed => {
                    let removed = self.remove_target_sessions(&event.target.id);
                    if self.discover_targets {
                        self.write_json(&target::destroyed_event(&event.target));
                    }
                    for session_id in removed {
                        self.write_json(&target::detached_event(&session_id));
                    }
                }
                TargetEventKind::Attached => {
                    let session_id = event.session_id.clone().unwrap_or_default();
                    self.write_json(&target::attached_event(&session_id, &event.target, false));
                }
                TargetEventKind::Detached => {
                    let session_id = event.session_id.clone().unwrap_or_default();
                    self.write_json(&target::detached_event(&session_id));
                }
            }
        }
    }

    fn session_for_target(&self, target_id: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|(_, session)| session.target.id == target_id)
            .map(|(id, _)| id.clone())
    }

    fn remove_target_sessions(&mut self, target_id: &str) -> Vec<String> {
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.target.id == target_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            self.sessions.remove(id);
        }
        self.order.retain(|id| !ids.contains(id));
        if let Some(default) = self.default_session.clone() {
            if ids.contains(&default) {
                self.default_session = None;
            }
        }
        ids
    }

    fn emit_events(&mut self) {
        let default = self.default_session.clone();
        let mut batches: Vec<(Option<String>, Vec<Json>)> = Vec::new();
        for id in &self.order {
            let Some(session) = self.sessions.get_mut(id) else {
                continue;
            };
            let events = session.poll_events();
            if events.is_empty() {
                continue;
            }
            let tag = if Some(id) == default.as_ref() {
                None
            } else {
                Some(id.clone())
            };
            batches.push((tag, events));
        }
        for (tag, events) in batches {
            for event in events {
                self.write_tagged(event, tag.as_deref());
            }
        }
    }

    fn write_tagged(&mut self, value: Json, tag: Option<&str>) {
        let value = match tag {
            Some(tag) => protocol::with_session(value, tag),
            None => value,
        };
        self.write_json(&value);
    }

    fn write_json(&mut self, value: &Json) {
        let _ = self.ws.write_message(&value.to_string());
    }
}

/// Recovers a request id from a message that failed to parse.
fn extract_id(text: &str) -> Json {
    Json::parse(text)
        .ok()
        .and_then(|value| value.get("id").cloned())
        .unwrap_or(Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::parse_head;

    fn upgrade_request(path: &str, key: &str) -> HttpRequest {
        let raw = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:9\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n"
        );
        parse_head(raw.as_bytes()).unwrap()
    }

    #[test]
    fn validates_websocket_keys() {
        let request = upgrade_request("/devtools/page/T1", "dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(
            websocket_key(&request).unwrap(),
            "dGhlIHNhbXBsZSBub25jZQ=="
        );
        let bad = upgrade_request("/devtools/page/T1", "short");
        assert!(websocket_key(&bad).is_err());
    }

    #[test]
    fn discovery_bodies_advertise_urls() {
        let info = crate::host::HostInfo::default();
        let version = version_body(&info, "127.0.0.1:9222");
        assert!(version.contains("ws://127.0.0.1:9222/devtools/browser"));
        assert!(version.contains("\"Protocol-Version\":\"1.3\""));

        let targets = vec![TargetInfo::page("T1", "https://a/", "A")];
        let list = list_body(&targets, "127.0.0.1:9222");
        assert!(list.contains("\"webSocketDebuggerUrl\":\"ws://127.0.0.1:9222/devtools/page/T1\""));
        assert!(list.contains("\"type\":\"page\""));
    }

    #[test]
    fn host_uses_request_authority() {
        let request = upgrade_request("/devtools/page/T1", "dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(base_authority(&request, None), "127.0.0.1:9");
    }
}
