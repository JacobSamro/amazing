//! UI-owned resource discovery and cancellable transport. No DOM/VM values cross threads.
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use amazing_net::{FetchOptions, Response, NetError};
use amazing_text_provider::{FontDisplayTimeline, FontDisplayFaceState, FontSourceLoad};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceKey {
    address: String,
    method: String,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
    max_bytes: usize,
    timeout: Duration,
    kind: ResourceKind,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ResourceKind { Session, Raw, Font { stylesheet: String, document: String } }
impl ResourceKey { pub fn is_font(&self) -> bool { matches!(self.kind, ResourceKind::Font { .. }) } }
#[derive(Clone)]
pub enum ResourceRequest {
    Session(String, FetchOptions),
    Raw(String, FetchOptions),
    Font { source: String, stylesheet: String, document: String, max_bytes: usize },
}
impl ResourceRequest {
    pub fn key(&self) -> ResourceKey {
        let (address, options, kind) = match self {
            Self::Session(address, options) => (address.clone(), options.clone(), ResourceKind::Session),
            Self::Raw(address, options) => (address.clone(), options.clone(), ResourceKind::Raw),
            Self::Font { source, stylesheet, document, max_bytes } => (source.clone(),
                FetchOptions { max_response_bytes: *max_bytes, ..FetchOptions::default() },
                ResourceKind::Font { stylesheet: stylesheet.clone(), document: document.clone() }),
        };
        ResourceKey { address, method: options.method, body: options.body, headers: options.request_headers,
            max_bytes: options.max_response_bytes, timeout: options.timeout, kind }
    }
}
#[derive(Clone)]
pub enum Acquired { Response(Response), Bytes(Vec<u8>) }
pub type AcquisitionResult = Result<Acquired, String>;
pub struct Replay {
    pub acquiring: bool,
    pub page_epoch: Instant,
    pub font_timelines: BTreeMap<String, FontDisplayTimeline>,
    pub current_font_faces: Vec<String>,
    pub font_states: BTreeMap<String, FontDisplayFaceState>,
    pub next_font_deadline: Option<Duration>,
    pub results: BTreeMap<ResourceKey, AcquisitionResult>,
    pub missing: Vec<ResourceRequest>,
}
impl Default for Replay { fn default() -> Self { Self { acquiring: true, page_epoch: Instant::now(), font_timelines: BTreeMap::new(), current_font_faces: Vec::new(), font_states: BTreeMap::new(), next_font_deadline: None, results: BTreeMap::new(), missing: Vec::new() } } }
pub fn font_trace(replay: Option<&Rc<RefCell<Replay>>>) -> String {
    let Some(replay) = replay else { return "[]".into() };
    let state = replay.borrow();
    let rows = state.font_states.iter().filter_map(|(key, face)| {
        let timeline = state.font_timelines.get(key)?;
        Some(format!("{{\"family\":{:?},\"phase\":{:?},\"visibility\":{:?},\"metrics\":{:?},\"next_ms\":{}}}",
            timeline.descriptor().family, format!("{:?}", face.display.phase), format!("{:?}", face.display.visibility),
            format!("{:?}", face.metrics), face.display.next_change.map_or("null".into(), |at| at.as_millis().to_string())))
    }).collect::<Vec<_>>();
    format!("[{}]", rows.join(","))
}
pub fn current_replay() -> Option<Rc<RefCell<Replay>>> { ACTIVE.with(|active| active.borrow().clone()) }
thread_local! { static ACTIVE: RefCell<Option<Rc<RefCell<Replay>>>> = const { RefCell::new(None) }; }
struct Scope(Option<Rc<RefCell<Replay>>>);
impl Drop for Scope { fn drop(&mut self) { ACTIVE.with(|active| *active.borrow_mut() = self.0.take()); } }
pub fn with_replay<T>(replay: Rc<RefCell<Replay>>, operation: impl FnOnce() -> T) -> T {
    let previous = ACTIVE.with(|active| active.replace(Some(replay)));
    let _scope = Scope(previous);
    operation()
}
pub fn has_missing() -> bool {
    ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|replay| !replay.borrow().missing.is_empty()))
}
enum ResourceRead { Pending, Complete(AcquisitionResult) }
fn lookup(request: ResourceRequest) -> Option<ResourceRead> {
    ACTIVE.with(|active| {
        let active = active.borrow();
        let replay = active.as_ref()?;
        let mut replay = replay.borrow_mut();
        let key = request.key();
        if let Some(result) = replay.results.get(&key) { return Some(ResourceRead::Complete(result.clone())); }
        if !replay.acquiring { return None; }
        if !replay.missing.iter().any(|other| other.key() == key) { replay.missing.push(request); }
        Some(ResourceRead::Pending)
    })
}
pub fn response(request: ResourceRequest) -> Option<Result<Response, NetError>> {
    lookup(request).map(|result| match result {
        ResourceRead::Complete(Ok(Acquired::Response(response))) => Ok(response),
        ResourceRead::Complete(Ok(_)) => Err(NetError::MalformedResponse("resource result type".into())),
        ResourceRead::Complete(Err(error)) => Err(NetError::MalformedResponse(error)),
        ResourceRead::Pending => Err(NetError::MalformedResponse("resource acquisition pending".into())),
    })
}
pub fn font(source: &str, stylesheet: &str, document: &str, max_bytes: usize) -> Option<Result<Vec<u8>, String>> {
    lookup(ResourceRequest::Font { source: source.into(), stylesheet: stylesheet.into(), document: document.into(), max_bytes })
        .map(|result| match result { ResourceRead::Complete(Ok(Acquired::Bytes(bytes))) => Ok(bytes), ResourceRead::Complete(Ok(_)) => Err("font result type".into()), ResourceRead::Complete(Err(error)) => Err(error), ResourceRead::Pending => Err("resource acquisition pending".into()) })
}

/// The producer receives a trusted source state, never a guessed warning string.
pub fn font_state(source: &str, stylesheet: &str, document: &str, max_bytes: usize) -> Option<FontSourceLoad> {
    lookup(ResourceRequest::Font { source: source.into(), stylesheet: stylesheet.into(), document: document.into(), max_bytes })
        .map(|result| match result {
            ResourceRead::Pending => FontSourceLoad::Pending,
            ResourceRead::Complete(Ok(Acquired::Bytes(bytes))) => FontSourceLoad::Ready(bytes),
            ResourceRead::Complete(_) => FontSourceLoad::Failed,
        })
}

/// Uses the engine's existing cooperative dispatcher, including decoding and TLS.
/// Cancellation retires the actual dispatcher request/socket, rather than merely hiding output.
pub fn transport(input: &str, options: &FetchOptions, cancel: &AtomicBool) -> Result<Response, String> {
    use amazing_net::dispatch::{Dispatcher, DispatchLimits, NetEvent};
    if cancel.load(Ordering::Acquire) { return Err("navigation cancelled".into()); }
    if !input.starts_with("http://") && !input.starts_with("https://") {
        let response = amazing_net::fetch(input, options).map_err(|error| error.to_string())?;
        return if cancel.load(Ordering::Acquire) { Err("navigation cancelled".into()) } else { Ok(response) };
    }
    let mut current = amazing_net::ParsedUrl::parse(input).map_err(|error| format!("{error:?}"))?;
    let mut options = options.clone();
    // Initial background acquisition has a finite browser loading budget. Keep the
    // UI-owned page options unchanged, including authored synchronous fetch semantics.
    options.timeout = options.timeout.max(Duration::from_secs(30));
    for hop in 0..=amazing_net::fetch::MAX_REDIRECTS {
        let mut dispatcher = Dispatcher::new_pending_io(DispatchLimits::default()).map_err(|error| format!("pending transport: {error}"))?;
        let id = dispatcher.submit(&current.to_string(), options.clone(), Instant::now()).map_err(|error| error.to_string())?;
        let mut response = None;
        loop {
            if cancel.load(Ordering::Acquire) { dispatcher.cancel(id); return Err("navigation cancelled".into()); }
            let mut done = false;
            for event in dispatcher.step(64 * 1024, Instant::now()) {
                match event {
                    NetEvent::Head { id: event_id, url, status, reason, headers } if event_id == id => {
                        response = Some(Response { url, status, reason, headers, body: Vec::new() });
                    },
                    NetEvent::Body { id: event_id, bytes } if event_id == id => {
                        let response = response.as_mut().ok_or("body before response head")?;
                        if response.body.len().saturating_add(bytes.len()) > options.max_response_bytes {
                            dispatcher.cancel(id); return Err("response size limit".into());
                        }
                        response.body.extend(bytes);
                    },
                    NetEvent::Done { id: event_id } if event_id == id => done = true,
                    NetEvent::Error { id: event_id, error } if event_id == id => return Err(error.to_string()),
                    NetEvent::Aborted { id: event_id } if event_id == id => return Err("navigation cancelled".into()),
                    _ => {},
                }
            }
            if done { break; }
            std::thread::sleep(Duration::from_millis(2));
        }
        let response = response.ok_or("response head missing")?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) { return Ok(response); }
        if hop == amazing_net::fetch::MAX_REDIRECTS { return Err("too many redirects".into()); }
        let location = response.headers.get("location").ok_or("redirect location missing")?;
        if matches!(response.status, 301..=303) && options.method.eq_ignore_ascii_case("POST") {
            options.method = "GET".into(); options.body.clear();
            options.request_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type") && !name.eq_ignore_ascii_case("content-length"));
        }
        let target = current.resolve(location).map_err(|error| format!("{error:?}"))?.with_inherited_fragment(&current);
        if !matches!(target.scheme(), "http" | "https") { return Err("unsupported redirect scheme".into()); }
        current = target;
    }
    Err("redirect limit".into())
}
pub fn cancellation() -> Arc<AtomicBool> { Arc::new(AtomicBool::new(false)) }
