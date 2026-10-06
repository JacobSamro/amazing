//! A tab navigation owns acquisition, cancellation, and an unpublished UI-thread page.
use std::{cell::RefCell, collections::VecDeque, rc::Rc,
    sync::{Arc, OnceLock, atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError}}};
use amazing_net::{FetchOptions, Response};
use crate::{engine_viewport::{EngineViewport, LoadedPage, PreparedDocument}, session_state::{BrowserSession, FetchPreparation, PreparedFetch}, viewport_metrics::ViewportMetrics,
    navigation_acquisition::{self as acquisition, Acquired, AcquisitionResult, Replay, ResourceKey, ResourceRequest}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryCommit { Push, Traverse(usize), Reload }
struct WorkerJob { request: ResourceRequest, cancel: Arc<AtomicBool>, result: mpsc::Sender<AcquisitionResult> }
mod workers;
fn workers() -> &'static SyncSender<WorkerJob> {
    static WORKERS: OnceLock<SyncSender<WorkerJob>> = OnceLock::new();
    WORKERS.get_or_init(workers::start)
}
struct InFlight {
    key: ResourceKey,
    prepared: Option<PreparedFetch>,
    receiver: Receiver<AcquisitionResult>,
}
struct Queued { key: ResourceKey, prepared: Option<PreparedFetch>, job: WorkerJob, receiver: Receiver<AcquisitionResult> }
impl Queued {
    fn prepare(request: ResourceRequest, session: &Rc<RefCell<BrowserSession>>, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let key = request.key();
        let (sender, receiver) = mpsc::channel();
        let (request, prepared) = match request {
            ResourceRequest::Session(address, options) => match session.borrow_mut().prepare_fetch(&address, &options).map_err(|error| error.to_string())? {
                FetchPreparation::Ready(response) => {
                    let _ = sender.send(Ok(Acquired::Response(response)));
                    return Ok(Self { key, prepared: None, receiver, job: WorkerJob { request: ResourceRequest::Raw(address, options), cancel, result: sender } });
                },
                FetchPreparation::Transport(prepared) =>
                    (ResourceRequest::Raw(prepared.input.clone(), prepared.options.clone()), Some(prepared)),
            },
            request => (request, None),
        };
        Ok(Self { key, prepared, receiver, job: WorkerJob { request, cancel, result: sender } })
    }
}

pub struct PendingNavigation {
    pub id: u64,
    pub address: String,
    pub history: HistoryCommit,
    pub committed: bool,
    cancel: Arc<AtomicBool>,
    options: FetchOptions,
    document_key: ResourceKey,
    document: Option<Response>,
    prepared_document: Option<PreparedDocument>,
    replay: Rc<RefCell<Replay>>,
    queued: VecDeque<Queued>,
    flights: Vec<InFlight>,
    page: Option<LoadedPage>,
    retained_bytes: usize,
    failure: Option<String>,
    needs_resource_refresh: bool,
}
impl Drop for PendingNavigation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        let mut replay = self.replay.borrow_mut();
        // Freeze every unfinished visual request of this original document.
        // Stop reflows the same page; it must not reopen a cancelled image/style
        // synchronously while a font was keeping the loading identity alive.
        let unfinished = !self.flights.is_empty() || !self.queued.is_empty() || !replay.missing.is_empty();
        let keys = self.flights.iter().map(|flight| flight.key.clone())
            .chain(self.queued.iter().map(|queued| queued.key.clone()))
            .chain(replay.missing.iter().map(ResourceRequest::key)).collect::<Vec<_>>();
        for key in keys { replay.results.entry(key).or_insert_with(|| Err("navigation cancelled".into())); }
        if !unfinished {
            // Completed pages retain existing synchronous authored-network and
            // session/cache behavior; failures remain stable until a new request.
            replay.results.retain(|key, result| key.is_font() || result.is_err());
        }
        replay.missing.clear(); replay.acquiring = false;
    }
}
impl PendingNavigation {
    pub fn new(id: u64, address: String, method: &str, body: Vec<u8>, history: HistoryCommit,
        session: &Rc<RefCell<BrowserSession>>) -> Self {
        let options = FetchOptions { timeout: std::time::Duration::from_secs(3), method: method.into(), body,
            request_headers: if method.eq_ignore_ascii_case("POST") { vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())] } else { vec![] },
            ..FetchOptions::default() };
        let request = ResourceRequest::Session(address.clone(), options.clone());
        let document_key = request.key();
        let cancel = acquisition::cancellation();
        let mut queued = VecDeque::new();
        let failure = match Queued::prepare(request, session, cancel.clone()) {
            Ok(request) => { queued.push_back(request); None }, Err(error) => Some(error),
        };
        Self { id, address, history, committed: false, cancel, options, document_key, document: None, prepared_document: None,
            replay: Rc::new(RefCell::new(Replay::default())), queued, flights: vec![], page: None, retained_bytes: 0, failure, needs_resource_refresh: false }
    }
    /// The published page/runtime stays owned on this UI thread. It is lent back
    /// only for resource reflow; neither the DOM nor author execution is recreated.
    pub fn lend_visible_page(&mut self, page: LoadedPage) { assert!(self.committed && self.page.is_none()); self.page = Some(page); }
    pub fn take_visible_page(&mut self) -> Option<LoadedPage> { assert!(self.committed); self.page.take() }
    fn only_fonts_pending(&self) -> bool {
        self.flights.iter().all(|flight| flight.key.is_font()) && self.queued.iter().all(|queued| queued.key.is_font())
    }
    pub fn request_data(&self) -> (String, Vec<u8>) { (self.options.method.clone(), self.options.body.clone()) }
    fn queue_missing(&mut self, session: &Rc<RefCell<BrowserSession>>) -> Result<(), String> {
        let missing = std::mem::take(&mut self.replay.borrow_mut().missing);
        if self.replay.borrow().results.len().saturating_add(missing.len()) > 256 { return Err("navigation resource count limit".into()); }
        for request in missing {
            let key = request.key();
            if self.flights.iter().any(|flight| flight.key == key) || self.queued.iter().any(|queued| queued.key == key) { continue; }
            match Queued::prepare(request, session, self.cancel.clone()) {
                Ok(request) => self.queued.push_back(request),
                Err(error) => { self.replay.borrow_mut().results.insert(key, Err(error)); self.needs_resource_refresh = true; },
            }
        }
        Ok(())
    }
    pub fn poll(&mut self, session: &Rc<RefCell<BrowserSession>>, metrics: ViewportMetrics) -> Option<Result<LoadedPage, String>> {
        if let Some(error) = self.failure.take() { return Some(Err(error)); }
        for index in (0..self.flights.len()).rev() {
            let result = match self.flights[index].receiver.try_recv() {
                Ok(result) => result, Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => Err("navigation transport exited without result".into()),
            };
            let mut flight = self.flights.swap_remove(index);
            let result = match (result, flight.prepared.take()) {
                (Ok(Acquired::Response(response)), Some(prepared)) => session.borrow_mut().finish_fetch(prepared, response)
                    .map(Acquired::Response).map_err(|error| error.to_string()),
                (result, _) => result,
            };
            if let Ok(acquired) = &result {
                self.retained_bytes = self.retained_bytes.saturating_add(match acquired { Acquired::Response(response) => response.body.len(), Acquired::Bytes(bytes) => bytes.len() });
                if self.retained_bytes > 128 * 1024 * 1024 { return Some(Err("navigation resource byte limit".into())); }
            }
            if flight.key == self.document_key {
                match result { Ok(Acquired::Response(response)) => self.document = Some(response),
                    Err(error) => return Some(Err(error)), _ => return Some(Err("document result type".into())) }
            } else { self.replay.borrow_mut().results.insert(flight.key, result); self.needs_resource_refresh = true; }
        }
        // Author callbacks can create new visual misses after a font-only
        // commit. Admit those on every UI tick, independently of font deadlines.
        if let Err(error) = self.queue_missing(session) { return Some(Err(error)); }
        while self.flights.len() < 4 {
            let Some(queued) = self.queued.pop_front() else { break };
            // A fresh cache hit is already complete and needs no transport admission.
            match queued.receiver.try_recv() {
                Ok(result) => {
                    let (sender, receiver) = mpsc::channel(); let _ = sender.send(result);
                    self.flights.push(InFlight { key: queued.key, prepared: queued.prepared, receiver });
                    continue;
                },
                Err(TryRecvError::Disconnected) => return Some(Err("navigation queue result closed".into())),
                Err(TryRecvError::Empty) => {},
            }
            match workers().try_send(queued.job) {
                Ok(()) => self.flights.push(InFlight { key: queued.key, prepared: queued.prepared, receiver: queued.receiver }),
                Err(TrySendError::Full(job)) => {
                    self.queued.push_front(Queued { key: queued.key, prepared: queued.prepared, receiver: queued.receiver, job }); break;
                },
                Err(TrySendError::Disconnected(_)) => return Some(Err("navigation workers unavailable".into())),
            }
        }
        if (!self.flights.is_empty() || !self.queued.is_empty()) && (self.committed || !self.only_fonts_pending()) {
            let due = { let state = self.replay.borrow(); self.committed && state.next_font_deadline.is_some_and(|at| state.page_epoch.elapsed() >= at) };
            if self.committed && (due || self.needs_resource_refresh) {
                self.needs_resource_refresh = false;
                let result = acquisition::with_replay(self.replay.clone(), ||
                    EngineViewport::refresh_initial_page(self.page.as_mut().expect("published loading page"), metrics));
                if let Err(error) = result { return Some(Err(error)); }
                if let Err(error) = self.queue_missing(session) { return Some(Err(error)); }
            }
            return None;
        }
        let response = self.document.as_ref()?;
        let content_type = response.content_type().unwrap_or("application/octet-stream");
        if content_type != "text/html" && content_type != "application/xhtml+xml" {
            return Some(Err(match session.borrow_mut().save_download(response) {
                Ok(download) => format!("downloaded {} to {}", download.name, download.path),
                Err(error) => format!("cannot render response type {content_type}; download failed: {error}"),
            }));
        }
        self.needs_resource_refresh = false;
        if self.page.is_none() {
            if self.prepared_document.is_none() {
                match PreparedDocument::new(response, self.options.clone()) {
                    Ok(document) => self.prepared_document = Some(document),
                    Err(error) => return Some(Err(error)),
                }
            }
            let prepared = acquisition::with_replay(self.replay.clone(), ||
                self.prepared_document.as_ref().expect("original navigation document").discover(session.clone(), metrics));
            let blocking_missing = self.replay.borrow().missing.iter().any(|request| !request.key().is_font());
            if blocking_missing {
                if let Err(error) = self.queue_missing(session) { return Some(Err(error)); }
                return None;
            }
            match prepared {
                Err(error) => return Some(Err(error)),
                Ok(prepared) => {
                    let document = self.prepared_document.take().expect("original navigation document");
                    let page = acquisition::with_replay(self.replay.clone(), || document.finish(prepared, session.clone(), metrics));
                    match page { Ok(page) => self.page = Some(page), Err(error) => return Some(Err(error)) }
                },
            }
        } else {
            let result = acquisition::with_replay(self.replay.clone(), ||
                EngineViewport::refresh_initial_page(self.page.as_mut().expect("pending page"), metrics));
            if let Err(error) = result { return Some(Err(error)); }
        }
        if !self.replay.borrow().missing.is_empty() {
            if let Err(error) = self.queue_missing(session) { return Some(Err(error)); }
        }
        // A preparation failure just resolved a miss after this snapshot. Give
        // the original runtime one more reflow before final commit/retirement.
        if self.needs_resource_refresh { return None; }
        if !self.flights.is_empty() || !self.queued.is_empty() {
            if self.committed || !self.only_fonts_pending() { return None; }
            // A font alone does not keep the fully acquired page on the previous URL.
            // Keep loading/stop identity while publishing the actual fallback layout.
            self.committed = true;
        }
        self.page.take().map(Ok)
    }
}
