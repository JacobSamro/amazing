//! Driver adoption of the CDP host seam (`amazing-devtools::host`), CDP-16.
//!
//! This is the devtools lane's own driver-adapter file (see AGENTS.md "Driver
//! adapter per lane"). integrator-assist landed the first slice
//! (`Page.navigate`/`getFrameTree`/`getLayoutMetrics`, `Runtime.evaluate`) with
//! a documented gap: `evaluate` built a *fresh* JS realm per call, because the
//! `amazing-js` heap is `Rc`-based (single-threaded execution is a locked
//! decision) while `DevToolsSession: Send` and a CDP connection owns its
//! session on its own thread.
//!
//! CDP-16 fixes that by giving the page a single owning thread instead of
//! trying to make the engine itself `Send`. [`PageOwner`] holds the live DOM
//! (`Rc<RefCell<BrowserDomHost>>`) and a persistent [`DriverScheduler`] (one JS
//! realm, kept alive across calls) and never leaves the thread
//! [`spawn_page_thread`] starts for it. [`DriverDevToolsHost`] and
//! [`DriverPageSession`] are thin, `Send`/`Sync` handles that talk to that
//! thread over an `std::sync::mpsc` channel of [`PageCommand`]s, each carrying
//! its own one-shot reply channel — the same request/response shape a
//! multi-process browser uses between its DevTools and renderer processes,
//! done here with threads and channels because Amazing is still
//! single-process (ADR "no-sandbox/no-multi-process non-goal", superseded by
//! ADR 0005 but not yet implemented).
//!
//! `Runtime.evaluate`, `Page.navigate`, `DOM.getDocument`/`querySelector(All)`
//! and `Accessibility.getFullAXTree` all now go through that one channel to
//! the one page thread, so they see the same live DOM and the same JS realm
//! (globals, closures) across calls — the regression test
//! `evaluate_persists_full_js_realm_state_across_calls` is the proof.
//!
//! ## Known limitations (follow-ups, not silently claimed as done)
//!
//! * `Page.captureScreenshot` stays `Unsupported`: `ScreenshotFormat` is
//!   Png/Jpeg/Webp and this driver has no encoder for any of them yet (only
//!   BMP, via `render`/`render-page`).
//! * `DOM.querySelector`/`querySelectorAll` use a single selector (no
//!   comma-separated selector lists) via `amazing_css::selectors`; `describeNode`,
//!   `getOuterHTML`, `setAttributeValue` and the CSS/Input domains are not
//!   wired and stay on the crate default of `HostError::Unsupported`.
//! * `Accessibility.getFullAXTree` (like `amazing axtree`) lays out with
//!   `amazing_text::FallbackTextSystem` and `amazing_layout::NoReplacedElements`,
//!   so replaced content and real glyph metrics are not reflected — the same
//!   documented gap as the a11y lane's own CLI entry point.
//! * `depth` on `Accessibility.getFullAXTree` is ignored (the whole tree is
//!   always returned), matching the CDP `-1` convention this crate already
//!   normalizes `depth` to at the domain layer.
use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use amazing_devtools::host::{
    AxNode, DevToolsHost, DevToolsSession, EngineNodeId, EvaluateParams, EvaluateResult,
    ExceptionDetails, FrameInfo, FrameTree, HostError, LayoutMetrics, NodeDescription,
    RemoteObject, TargetInfo, Viewport,
};
use amazing_devtools::server::CdpServer;

use crate::dom_host::BrowserDomHost;
use crate::js_scheduler::DriverScheduler;

/// Reply channel for one [`PageCommand`].
type Reply<T> = Sender<Result<T, HostError>>;

/// One request the session/host side sends to the page thread. Every variant
/// is `Send` (plain owned data plus a `Sender`, itself `Send` when its payload
/// is), which is what lets [`DriverPageSession`] satisfy `DevToolsSession: Send`
/// without the engine itself being `Send`.
enum PageCommand {
    Navigate(String, Reply<FrameInfo>),
    FrameTree(Reply<FrameTree>),
    LayoutMetrics(Reply<LayoutMetrics>),
    Evaluate(EvaluateParams, Reply<EvaluateResult>),
    GetDocument(u32, Reply<NodeDescription>),
    QuerySelector(EngineNodeId, String, Reply<Option<EngineNodeId>>),
    QuerySelectorAll(EngineNodeId, String, Reply<Vec<EngineNodeId>>),
    FullAxTree(Reply<Vec<AxNode>>),
    Shutdown,
}

/// Sends `make(reply_tx)` to the page thread and blocks for its answer. The
/// only failure mode besides the command's own `HostError` is the page thread
/// having already exited (e.g. the host was dropped mid-call).
fn request<T>(sender: &Sender<PageCommand>, make: impl FnOnce(Reply<T>) -> PageCommand) -> Result<T, HostError> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    sender
        .send(make(reply_tx))
        .map_err(|_| HostError::Internal("devtools page thread is gone".into()))?;
    reply_rx
        .recv()
        .map_err(|_| HostError::Internal("devtools page thread is gone".into()))?
}

fn origin_of(url: &str) -> String {
    amazing_net::Url::parse(url)
        .map(|parsed| format!("{}://{}", parsed.scheme, parsed.host))
        .unwrap_or_else(|_| url.to_owned())
}

fn frame_info(target_id: &str, url: &str) -> FrameInfo {
    FrameInfo {
        id: target_id.to_owned(),
        parent_id: None,
        url: url.to_owned(),
        name: String::new(),
        security_origin: origin_of(url),
        mime_type: "text/html".into(),
    }
}

fn js_value_to_remote_object(value: &amazing_js::value::JsValue) -> RemoteObject {
    use amazing_js::value::JsValue;
    match value {
        JsValue::Undefined => RemoteObject::undefined(),
        JsValue::Null => RemoteObject::null(),
        JsValue::Bool(b) => RemoteObject::boolean(*b),
        JsValue::Num(n) => RemoteObject::number(*n),
        JsValue::Str(s) => RemoteObject::string(s.to_utf8_lossy()),
        // No live-object bridge yet (`Runtime.getProperties`, previews, object
        // ids that outlive the call): report identity and a debug description
        // only. Real pages' `evaluate()` calls are overwhelmingly primitives;
        // this is a documented gap, not a silent wrong answer.
        JsValue::BigInt(_) | JsValue::Symbol(_) | JsValue::Object(_) => RemoteObject {
            object_type: match value {
                JsValue::BigInt(_) => "bigint",
                JsValue::Symbol(_) => "symbol",
                _ => "object",
            }
            .to_string(),
            subtype: None,
            class_name: None,
            value: None,
            unserializable_value: None,
            description: Some(format!("{value:?}")),
            object_id: None,
        },
    }
}

fn base_description(node: EngineNodeId, node_type: u16, name: &str, value: &str) -> NodeDescription {
    NodeDescription {
        node,
        parent: None,
        backend_node_id: node.0,
        node_type,
        node_name: name.to_owned(),
        local_name: String::new(),
        node_value: value.to_owned(),
        attributes: Vec::new(),
        child_node_count: 0,
        children: Vec::new(),
    }
}

/// Builds a CDP `DOM.Node` tree from the live arena DOM, expanding `depth`
/// levels of children (0 = the node itself only). `node_type` numbering
/// follows the DOM spec (`Node.ELEMENT_NODE` etc), matching what
/// `amazing-devtools`' own `NodeDescription` helpers assume.
fn describe_node(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, depth: u32) -> NodeDescription {
    use amazing_dom::NodeKind;
    let engine = EngineNodeId(node.0 as u64);
    let children: Vec<amazing_dom::NodeIdx> = dom.children(node).collect();
    let mut desc = match &dom.get(node).kind {
        NodeKind::Document => base_description(engine, 9, "#document", ""),
        NodeKind::DocumentFragment => base_description(engine, 11, "#document-fragment", ""),
        NodeKind::DocumentType { name, .. } => base_description(engine, 10, name, ""),
        NodeKind::Element { name, attrs, .. } => {
            NodeDescription::element(engine, name.clone()).with_attributes(attrs.clone())
        }
        NodeKind::Text { data } => NodeDescription::text(engine, data.clone()),
        NodeKind::Comment { data } => base_description(engine, 8, "#comment", data),
        NodeKind::CDATASection { data } => base_description(engine, 4, "#cdata-section", data),
        NodeKind::ProcessingInstruction { target, data } => base_description(engine, 7, target, data),
        NodeKind::Attr { .. } => base_description(engine, 2, "", ""),
    };
    desc.child_node_count = children.len();
    if depth > 0 {
        desc.children = children
            .into_iter()
            .map(|child| describe_node(dom, child, depth - 1))
            .collect();
    }
    desc
}

fn find_first(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selector: &amazing_css::selectors::Selector,
) -> Option<EngineNodeId> {
    if amazing_css::selectors::matches(dom, node, selector) {
        return Some(EngineNodeId(node.0 as u64));
    }
    for child in dom.children(node) {
        if let Some(found) = find_first(dom, child, selector) {
            return Some(found);
        }
    }
    None
}

fn find_all(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selector: &amazing_css::selectors::Selector,
    out: &mut Vec<EngineNodeId>,
) {
    if amazing_css::selectors::matches(dom, node, selector) {
        out.push(EngineNodeId(node.0 as u64));
    }
    for child in dom.children(node) {
        find_all(dom, child, selector, out);
    }
}

/// Converts the a11y lane's own tree into the CDP-shaped seam, matching the
/// pattern `amazing-devtools::domains::accessibility` documents: this crate
/// stays independent of `amazing-a11y`, so the conversion lives entirely on
/// the driver side.
fn convert_ax_tree(tree: &amazing_a11y::AxTree) -> Vec<AxNode> {
    tree.nodes()
        .iter()
        .map(|node| AxNode {
            ax_node_id: node.id.0.to_string(),
            ignored: false,
            role: Some(node.role.as_str().to_owned()),
            name: (!node.name.is_empty()).then(|| node.name.clone()),
            description: (!node.description.is_empty()).then(|| node.description.clone()),
            value: None,
            node: node.dom_node.map(|idx| EngineNodeId(idx.0 as u64)),
            child_ids: node.children.iter().map(|id| id.0.to_string()).collect(),
        })
        .collect()
}

/// The one piece of state that must live on a single thread: the live DOM and
/// a persistent JS realm bound to it. Not `Send` (the JS heap is `Rc`-based),
/// so it never leaves the thread [`spawn_page_thread`] gives it.
struct PageOwner {
    dom_host: Rc<RefCell<BrowserDomHost>>,
    scheduler: DriverScheduler,
    target_id: String,
    url: String,
    width: f32,
}

impl PageOwner {
    /// Builds a page owner from an already-parsed document: installs the JS
    /// realm, runs every `<script>`/inline handler and dispatches
    /// `DOMContentLoaded`/`load`, exactly as `amazing render`/`render-page` do
    /// (`force_scheduler: true`, so even a script-less page still gets a
    /// realm for `Runtime.evaluate`).
    fn from_parsed(target_id: &str, url: &str, dom: amazing_dom::Dom, width: f32) -> Result<Self, HostError> {
        let dom_host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let scheduler = crate::cmd::js::prepare_page_scripts(&dom_host, url, true)
            .map_err(HostError::Internal)?
            .expect("force_scheduler=true always yields a scheduler");
        Ok(Self { dom_host, scheduler, target_id: target_id.to_owned(), url: url.to_owned(), width })
    }

    fn fetch_and_load(target_id: &str, url: &str, width: f32) -> Result<Self, HostError> {
        let (final_url, html, content_type) =
            crate::cmd::real_pages::fetch_document(url).map_err(HostError::Internal)?;
        let document = crate::document::parse_document_bytes(&html, &final_url, content_type.as_deref())
            .map_err(|error| HostError::Internal(error.to_string()))?;
        Self::from_parsed(target_id, &final_url, document.dom, width)
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let reloaded = Self::fetch_and_load(&self.target_id, url, self.width)?;
        *self = reloaded;
        Ok(frame_info(&self.target_id, &self.url))
    }

    fn frame_tree(&self) -> FrameTree {
        FrameTree { frame: frame_info(&self.target_id, &self.url), child_frames: Vec::new() }
    }

    fn layout_metrics(&self) -> Result<LayoutMetrics, HostError> {
        let surface = crate::cmd::render::paint_host_surface(&self.dom_host, &self.url, self.width, None, "")
            .map_err(|error| HostError::Internal(error.to_string()))?;
        let viewport = Viewport {
            page_x: 0.0,
            page_y: 0.0,
            client_width: self.width as f64,
            client_height: surface.height as f64,
        };
        Ok(LayoutMetrics {
            layout_viewport: viewport,
            visual_viewport: viewport,
            content_width: surface.width as f64,
            content_height: surface.height as f64,
        })
    }

    /// Runs `expression` in this page's persistent realm. Unlike the first
    /// slice, this is the *same* `DriverScheduler` (and so the same globals
    /// and closures) across every call for this page's lifetime.
    fn evaluate(&mut self, params: &EvaluateParams) -> EvaluateResult {
        match self.scheduler.run_script(&params.expression) {
            Ok(value) => EvaluateResult::value(js_value_to_remote_object(&value)),
            Err(thrown) => EvaluateResult {
                result: js_value_to_remote_object(&thrown.0),
                exception: Some(ExceptionDetails {
                    text: thrown.describe(),
                    line_number: 0,
                    column_number: 0,
                    url: None,
                    exception: Some(js_value_to_remote_object(&thrown.0)),
                    stack_trace: None,
                }),
            },
        }
    }

    fn get_document(&self, depth: u32) -> NodeDescription {
        let host = self.dom_host.borrow();
        describe_node(&host.dom, amazing_dom::NodeIdx(0), depth)
    }

    fn query_selector(&self, node: EngineNodeId, selector: &str) -> Result<Option<EngineNodeId>, HostError> {
        let parsed = amazing_css::selectors::parse_selector(selector)
            .ok_or_else(|| HostError::InvalidParams(format!("bad selector: {selector}")))?;
        let host = self.dom_host.borrow();
        Ok(find_first(&host.dom, amazing_dom::NodeIdx(node.0 as u32), &parsed))
    }

    fn query_selector_all(&self, node: EngineNodeId, selector: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let parsed = amazing_css::selectors::parse_selector(selector)
            .ok_or_else(|| HostError::InvalidParams(format!("bad selector: {selector}")))?;
        let host = self.dom_host.borrow();
        let mut out = Vec::new();
        find_all(&host.dom, amazing_dom::NodeIdx(node.0 as u32), &parsed, &mut out);
        Ok(out)
    }

    /// Computes style + layout (the same minimal `FallbackTextSystem` /
    /// `NoReplacedElements` path `amazing axtree` uses) on the live DOM and
    /// builds the accessibility tree through the a11y lane's own driver
    /// adapter, per the ticket's "through `runtime_host_a11y::build_ax_tree`".
    fn full_ax_tree(&self) -> Result<Vec<AxNode>, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let mut sources = Vec::new();
        let _ = crate::page::collect_author_css(dom, amazing_dom::NodeIdx(0), &self.url, &mut sources);
        let sheets: Vec<amazing_css::Stylesheet> = sources.into_iter().map(|(_, _, sheet)| sheet).collect();
        let styled = amazing_css::compute_with_environment(
            dom,
            &sheets,
            &amazing_css::MediaEnvironment {
                viewport_width_px: self.width.max(1.0),
                ..amazing_css::MediaEnvironment::default()
            },
        );
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_styled_document(
            dom,
            &styled,
            &text,
            &amazing_layout::NoReplacedElements,
            self.width,
            None,
        );
        let tree = crate::runtime_host_a11y::build_ax_tree(dom, &styled, &layout);
        Ok(convert_ax_tree(&tree))
    }
}

/// What [`spawn_page_thread`] needs to build the page's [`PageOwner`] — kept
/// separate from `PageOwner` itself so the *construction inputs* (plain
/// `Send` data: strings, an `amazing_dom::Dom`, a width) cross the thread
/// boundary in the `spawn` closure, while the engine state `PageOwner` builds
/// (the `Rc`-based JS heap) never does. `std::thread::spawn` requires the
/// closure to be `Send`, which only constrains what it *captures*, not what
/// it constructs after starting — so `PageOwner::from_parsed`/`fetch_and_load`
/// run inside the spawned thread, not before it.
enum PageInit {
    Parsed { target_id: String, url: String, dom: amazing_dom::Dom, width: f32 },
    Fetch { target_id: String, url: String, width: f32 },
}

impl PageInit {
    fn load(self) -> Result<PageOwner, HostError> {
        match self {
            PageInit::Parsed { target_id, url, dom, width } => PageOwner::from_parsed(&target_id, &url, dom, width),
            PageInit::Fetch { target_id, url, width } => PageOwner::fetch_and_load(&target_id, &url, width),
        }
    }
}

fn run_page_thread(mut page: PageOwner, rx: Receiver<PageCommand>) {
    while let Ok(command) = rx.recv() {
        match command {
            PageCommand::Navigate(url, reply) => {
                let _ = reply.send(page.navigate(&url));
            }
            PageCommand::FrameTree(reply) => {
                let _ = reply.send(Ok(page.frame_tree()));
            }
            PageCommand::LayoutMetrics(reply) => {
                let _ = reply.send(page.layout_metrics());
            }
            PageCommand::Evaluate(params, reply) => {
                let _ = reply.send(Ok(page.evaluate(&params)));
            }
            PageCommand::GetDocument(depth, reply) => {
                let _ = reply.send(Ok(page.get_document(depth)));
            }
            PageCommand::QuerySelector(node, selector, reply) => {
                let _ = reply.send(page.query_selector(node, &selector));
            }
            PageCommand::QuerySelectorAll(node, selector, reply) => {
                let _ = reply.send(page.query_selector_all(node, &selector));
            }
            PageCommand::FullAxTree(reply) => {
                let _ = reply.send(page.full_ax_tree());
            }
            PageCommand::Shutdown => break,
        }
    }
}

/// Starts the dedicated page thread and returns the channel to talk to it,
/// plus the loaded page's final URL. Blocks until the initial
/// parse/script/load finishes (or fails), so a bad URL is reported to the
/// caller synchronously rather than silently wedging every later command. The
/// thread runs until it receives [`PageCommand::Shutdown`] (sent by
/// [`DriverDevToolsHost`]'s `Drop`), so it never outlives its host.
fn spawn_page_thread(init: PageInit) -> Result<(Sender<PageCommand>, String), HostError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<String, HostError>>();
    std::thread::Builder::new()
        .name("devtools-page".into())
        .spawn(move || match init.load() {
            Ok(page) => {
                let _ = ready_tx.send(Ok(page.url.clone()));
                run_page_thread(page, rx);
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })
        .expect("spawn devtools page thread");
    let url = ready_rx
        .recv()
        .map_err(|_| HostError::Internal("devtools page thread died before starting".into()))??;
    Ok((tx, url))
}

/// One attached page target: a `Send` handle to the page thread. It holds no
/// engine state itself, so it trivially satisfies `DevToolsSession: Send`
/// regardless of how many worker threads a `CdpServer` connection spawns.
pub(crate) struct DriverPageSession {
    target_id: String,
    sender: Sender<PageCommand>,
    current_url: Arc<Mutex<String>>,
}

impl DevToolsSession for DriverPageSession {
    fn target_id(&self) -> &str {
        &self.target_id
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let frame = request(&self.sender, |reply| PageCommand::Navigate(url.to_owned(), reply))?;
        if let Ok(mut current) = self.current_url.lock() {
            *current = frame.url.clone();
        }
        Ok(frame)
    }

    fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
        request(&self.sender, PageCommand::FrameTree)
    }

    fn get_layout_metrics(&mut self) -> Result<LayoutMetrics, HostError> {
        request(&self.sender, PageCommand::LayoutMetrics)
    }

    fn evaluate(&mut self, params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
        let params = params.clone();
        request(&self.sender, |reply| PageCommand::Evaluate(params, reply))
    }

    fn get_document(&mut self, depth: u32) -> Result<NodeDescription, HostError> {
        request(&self.sender, |reply| PageCommand::GetDocument(depth, reply))
    }

    fn query_selector(&mut self, node: EngineNodeId, selector: &str) -> Result<Option<EngineNodeId>, HostError> {
        let selector = selector.to_owned();
        request(&self.sender, |reply| PageCommand::QuerySelector(node, selector, reply))
    }

    fn query_selector_all(&mut self, node: EngineNodeId, selector: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let selector = selector.to_owned();
        request(&self.sender, |reply| PageCommand::QuerySelectorAll(node, selector, reply))
    }

    fn get_full_ax_tree(&mut self, _depth: Option<u32>) -> Result<Vec<AxNode>, HostError> {
        request(&self.sender, PageCommand::FullAxTree)
    }
}

/// The browser-level seam: one page target, backed by a real page (fetched or,
/// for tests, parsed directly) running on its own thread.
pub(crate) struct DriverDevToolsHost {
    target_id: String,
    sender: Mutex<Sender<PageCommand>>,
    current_url: Arc<Mutex<String>>,
}

impl DriverDevToolsHost {
    fn spawn(target_id: impl Into<String>, init: PageInit) -> Result<Self, HostError> {
        let (sender, url) = spawn_page_thread(init)?;
        Ok(Self { target_id: target_id.into(), sender: Mutex::new(sender), current_url: Arc::new(Mutex::new(url)) })
    }
}

impl Drop for DriverDevToolsHost {
    fn drop(&mut self) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(PageCommand::Shutdown);
        }
    }
}

impl DevToolsHost for DriverDevToolsHost {
    fn list_targets(&self) -> Vec<TargetInfo> {
        let url = self.current_url.lock().expect("devtools url poisoned").clone();
        vec![TargetInfo::page(self.target_id.clone(), url.clone(), url)]
    }

    fn attach(&self, target_id: &str) -> Result<Box<dyn DevToolsSession>, HostError> {
        if target_id != self.target_id {
            return Err(HostError::NotFound);
        }
        let sender = self.sender.lock().expect("devtools sender poisoned").clone();
        Ok(Box::new(DriverPageSession {
            target_id: self.target_id.clone(),
            sender,
            current_url: self.current_url.clone(),
        }))
    }
}

/// A running CDP server on its own thread, stopped on drop.
pub(crate) struct DevToolsHandle {
    pub(crate) addr: SocketAddr,
    pub(crate) browser_id: String,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl Drop for DevToolsHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl DevToolsHandle {
    /// Blocks the calling thread until the server thread exits. Nothing in
    /// this handle sets the shutdown flag itself, so this is for a foreground
    /// CLI command that should serve until the process is killed, not for
    /// code that wants to stop the server (drop the handle for that).
    pub(crate) fn wait(mut self) -> io::Result<()> {
        match self.join.take() {
            Some(join) => join.join().unwrap_or(Ok(())),
            None => Ok(()),
        }
    }
}

/// Fetches and parses `initial_url`, starts its dedicated page thread, then
/// binds and starts a CDP server for it on `addr` (`"127.0.0.1:0"` for an
/// ephemeral port) on a background thread.
pub(crate) fn spawn_devtools_server(initial_url: &str, addr: &str) -> Result<DevToolsHandle, String> {
    let init = PageInit::Fetch { target_id: "page-1".into(), url: initial_url.to_owned(), width: 1024.0 };
    let host = Arc::new(DriverDevToolsHost::spawn("page-1", init).map_err(|error| error.to_string())?);
    let server = CdpServer::bind(host, addr).map_err(|error| format!("bind {addr}: {error}"))?;
    let resolved_addr = server.local_addr().map_err(|error| error.to_string())?;
    let browser_id = server.browser_id().to_owned();
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));
    Ok(DevToolsHandle { addr: resolved_addr, browser_id, shutdown, join: Some(join) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_devtools::json::Json;
    use amazing_devtools::ws::{Message, WsConnection};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    /// Builds a page thread directly from an in-memory document, bypassing
    /// network I/O, exactly like `prepare_page_scripts` production path
    /// (`force_scheduler: true`).
    fn spawn_test_host(url: &str, html: &str) -> Arc<DriverDevToolsHost> {
        let document =
            crate::document::parse_document_bytes(html.as_bytes(), url, None).expect("parse fixture document");
        let init = PageInit::Parsed { target_id: "page-1".into(), url: url.to_owned(), dom: document.dom, width: 800.0 };
        Arc::new(DriverDevToolsHost::spawn("page-1", init).expect("spawn devtools host"))
    }

    /// A one-shot local HTTP server for exercising the real `navigate()`
    /// path (which always fetches, matching production) without reaching the
    /// network: everything stays on `127.0.0.1`.
    fn serve_once(html: &'static str) -> SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = html.as_bytes();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        addr
    }

    #[test]
    fn attach_unknown_target_is_not_found() {
        let host = spawn_test_host("http://example.test/fixture.html", "<title>fixture</title>");
        match host.attach("no-such-target") {
            Err(HostError::NotFound) => {}
            other => panic!("expected HostError::NotFound, got {}", other.is_ok()),
        }
    }

    #[test]
    fn navigate_fetches_a_real_page_and_evaluate_reads_the_new_dom() {
        let host = spawn_test_host("http://example.test/start.html", "<title>start</title><body>a</body>");
        let mut session = host.attach("page-1").expect("attach");
        let addr = serve_once("<title>second</title><body>b</body>");
        let frame = session.navigate(&format!("http://{addr}/")).expect("navigate");
        assert!(frame.url.contains(&addr.to_string()));
        let tree = session.frame_tree().expect("frame_tree");
        assert_eq!(tree.frame.url, frame.url);

        let result = session
            .evaluate(&EvaluateParams { expression: "document.title".into(), ..EvaluateParams::default() })
            .expect("evaluate");
        assert_eq!(result.result, RemoteObject::string("second"));
    }

    #[test]
    fn evaluate_persists_full_js_realm_state_across_calls() {
        // The bug CDP-16 fixes: the first slice built a fresh `Interpreter`
        // per `evaluate()` call, so `globalThis` state never survived between
        // calls even though the DOM (plain owned data) did. With one page
        // thread and one persistent `DriverScheduler`, it now does.
        let host = spawn_test_host("http://example.test/doc.html", "<title>t</title><body></body>");
        let mut session = host.attach("page-1").expect("attach");
        let expr = "globalThis.counter = (globalThis.counter || 0) + 1; globalThis.counter";
        let first = session
            .evaluate(&EvaluateParams { expression: expr.into(), ..EvaluateParams::default() })
            .expect("evaluate #1");
        assert_eq!(first.result, RemoteObject::number(1.0));
        let second = session
            .evaluate(&EvaluateParams { expression: expr.into(), ..EvaluateParams::default() })
            .expect("evaluate #2");
        assert_eq!(second.result, RemoteObject::number(2.0), "globalThis state should survive across evaluate() calls");
    }

    #[test]
    fn evaluate_reports_a_thrown_exception_instead_of_erroring_the_call() {
        let host = spawn_test_host("http://example.test/doc.html", "<title>t</title>");
        let mut session = host.attach("page-1").expect("attach");
        let result = session
            .evaluate(&EvaluateParams { expression: "undefinedGlobal.member".into(), ..EvaluateParams::default() })
            .expect("evaluate");
        assert!(result.exception.is_some());
    }

    #[test]
    fn get_document_and_query_selector_see_the_live_dom() {
        let host = spawn_test_host(
            "http://example.test/doc.html",
            "<title>t</title><body><h1 id=\"h\">Hi</h1><p class=\"x\">one</p><p class=\"x\">two</p></body>",
        );
        let mut session = host.attach("page-1").expect("attach");
        let root = session.get_document(64).expect("get_document");
        assert_eq!(root.node_type, 9, "document root should be nodeType 9");

        let found = session
            .query_selector(root.node, "#h")
            .expect("query_selector")
            .expect("#h should resolve to a node");
        let all = session.query_selector_all(root.node, ".x").expect("query_selector_all");
        assert_eq!(all.len(), 2, "both <p class=x> nodes should be found");
        assert_ne!(found, EngineNodeId(0));
    }

    #[test]
    fn get_full_ax_tree_reflects_the_live_page() {
        let host = spawn_test_host(
            "http://example.test/doc.html",
            "<title>t</title><body><h1>Hello</h1><button>Go</button></body>",
        );
        let mut session = host.attach("page-1").expect("attach");
        let nodes = session.get_full_ax_tree(None).expect("get_full_ax_tree");
        assert!(nodes.iter().any(|node| node.role.as_deref() == Some("heading")), "{nodes:?}");
        assert!(nodes.iter().any(|node| node.role.as_deref() == Some("button")), "{nodes:?}");
    }

    /// A minimal scripted CDP WebSocket client, mirroring the pattern
    /// `amazing-devtools`' own `tests/cdp.rs` uses against `MockHost`. This is
    /// the "prove it" test the ticket asks for: a real client, real JSON-RPC
    /// framing over a real `WsConnection`, against the real page thread.
    struct Client {
        ws: WsConnection<TcpStream>,
        next_id: u64,
    }

    impl Client {
        fn connect(addr: SocketAddr, path: &str) -> Self {
            let mut stream = TcpStream::connect(addr).expect("connect");
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let key = "dGhlIHNhbXBsZSBub25jZQ==";
            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).unwrap();
            stream.flush().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).expect("handshake");
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            assert!(head.starts_with("HTTP/1.1 101"), "unexpected handshake: {head}");
            let expected = amazing_devtools::sha1::websocket_accept(key);
            assert!(
                head.to_ascii_lowercase().contains(&format!("sec-websocket-accept: {}", expected.to_ascii_lowercase())),
                "bad accept header: {head}"
            );
            Self { ws: WsConnection::as_client(stream), next_id: 0 }
        }

        fn recv_json(&mut self) -> Json {
            match self.ws.poll() {
                Ok(Some(Message::Text(text))) => Json::parse(&text).expect("server json"),
                Ok(Some(Message::Binary(_))) => panic!("unexpected binary frame"),
                Ok(None) => panic!("timed out waiting for a server message"),
                Err(error) => panic!("websocket error: {error}"),
            }
        }

        fn result(&mut self, method: &str, params: Json) -> Json {
            self.next_id += 1;
            let id = self.next_id;
            let message = Json::Object(vec![
                ("id".into(), Json::Number(id as f64)),
                ("method".into(), Json::String(method.into())),
                ("params".into(), params),
            ]);
            self.ws.write_message(&message.to_string()).expect("write message");
            loop {
                let reply = self.recv_json();
                if reply.get("id").and_then(Json::as_f64) == Some(id as f64) {
                    return reply
                        .get("result")
                        .cloned()
                        .unwrap_or_else(|| panic!("{method} returned no result: {reply}"));
                }
                // Not our reply (a push event) — keep waiting.
            }
        }
    }

    #[test]
    fn end_to_end_cdp_client_evaluates_queries_dom_and_reads_the_ax_tree() {
        let host = spawn_test_host(
            "http://example.test/doc.html",
            "<title>Live</title><body><h1 id=\"h\">Hello</h1><button>Go</button></body>",
        );
        let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut client = Client::connect(addr, "/devtools/page/page-1");

        // Runtime.evaluate against the live realm.
        let evaluate = client.result(
            "Runtime.evaluate",
            Json::Object(vec![
                ("expression".into(), Json::String("21 * 2".into())),
                ("returnByValue".into(), Json::Bool(true)),
            ]),
        );
        assert_eq!(
            evaluate.get("result").and_then(|result| result.get("value")).and_then(Json::as_f64),
            Some(42.0),
            "{evaluate}"
        );

        // DOM.getDocument + DOM.querySelector/querySelectorAll on the live DOM.
        let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
        let root_node_id = document
            .get("root")
            .and_then(|root| root.get("nodeId"))
            .and_then(Json::as_f64)
            .expect("root nodeId") as u64;

        let query = client.result(
            "DOM.querySelector",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_node_id as f64)),
                ("selector".into(), Json::String("#h".into())),
            ]),
        );
        let found_node_id = query.get("nodeId").and_then(Json::as_f64).unwrap_or(0.0);
        assert!(found_node_id > 0.0, "#h should resolve to a real node: {query}");

        let heading_query = client.result(
            "DOM.querySelectorAll",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_node_id as f64)),
                ("selector".into(), Json::String("h1".into())),
            ]),
        );
        assert_eq!(
            heading_query.get("nodeIds").and_then(Json::as_array).map(<[Json]>::len),
            Some(1),
            "{heading_query}"
        );

        // Accessibility.getFullAXTree through runtime_host_a11y::build_ax_tree.
        let ax_tree = client.result("Accessibility.getFullAXTree", Json::Object(Vec::new()));
        let nodes = ax_tree.get("nodes").and_then(Json::as_array).expect("nodes array");
        let has_role = |role: &str| {
            nodes.iter().any(|node| node.get("role").and_then(|r| r.get("value")).and_then(Json::as_str) == Some(role))
        };
        assert!(has_role("heading"), "{nodes:?}");
        assert!(has_role("button"), "{nodes:?}");

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }

    #[test]
    fn spawn_devtools_server_serves_json_version_over_the_bound_port() {
        let html_host = spawn_test_host("http://example.test/wired.html", "<title>wired</title>");
        let server = CdpServer::bind(html_host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut response = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut last_error = None;
        while std::time::Instant::now() < deadline {
            match TcpStream::connect(addr) {
                Ok(mut stream) => {
                    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    let _ = stream.write_all(
                        format!("GET /json/version HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes(),
                    );
                    let _ = stream.read_to_string(&mut response);
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        assert!(response.contains("200"), "no /json/version response; last connect error: {last_error:?}, got: {response}");
        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }
}
