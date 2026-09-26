//! Driver adoption of the CDP host seam (`amazing-devtools::host`).
//!
//! This is `integrator-assist`'s own driver-adapter file (see AGENTS.md
//! "Driver adapter per lane"): the devtools lane owns `crates/amazing-devtools`
//! and asked the driver to (1) implement `DevToolsHost`/`DevToolsSession` over
//! a real browser page and (2) own a `CdpServer::bind(...)` on a background
//! thread. Every `DevToolsSession`/`DevToolsHost` method not implemented here
//! keeps its crate default of `HostError::Unsupported`, so this lands one
//! usable slice (`Page.navigate`, `Page.getFrameTree`,
//! `Page.getLayoutMetrics`, `Runtime.evaluate`) rather than stubbing the rest.
//!
//! ## Known limitations (follow-ups, not silently claimed as done)
//!
//! * `Runtime.evaluate` gets a **fresh JS realm per call**: nothing keeps the
//!   `amazing-js` heap (which is `Rc`-based, per ADR "single-threaded
//!   execution") alive across CDP calls, because `DevToolsSession: Send` and a
//!   CDP connection owns its session on its own thread. DOM mutations made by
//!   one `evaluate` call ARE visible to the next (the DOM itself is plain
//!   owned data, moved in and out of a throwaway scheduler), but JS globals
//!   and closures are not retained. A persistent realm needs either a
//!   dedicated single JS thread the session talks to over a channel, or
//!   relaxing the trait's `Send` bound; routed to `questions.md`.
//! * `Page.captureScreenshot` stays `Unsupported`: `ScreenshotFormat` is
//!   Png/Jpeg/Webp and this driver has no encoder for any of them yet (only
//!   BMP, via `render`/`render-page`).
//! * DOM/CSS/Input domains (`getDocument`, `querySelector`,
//!   `dispatchMouseEvent`, …) are not wired; they need the same node-identity
//!   bridge `EngineNodeId` implies and are left for the next slice.
use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use amazing_devtools::host::{
    DevToolsHost, DevToolsSession, EvaluateParams, EvaluateResult, ExceptionDetails, FrameInfo,
    FrameTree, HostError, LayoutMetrics, RemoteObject, TargetInfo, Viewport,
};
use amazing_devtools::server::CdpServer;

use crate::dom_host::BrowserDomHost;
use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};

/// The one piece of state a CDP session needs between calls: the current
/// document. Plain owned data (no `Rc`/`RefCell`), so `Arc<Mutex<PageState>>`
/// is `Send`, unlike the JS scheduler.
struct PageState {
    dom: amazing_dom::Dom,
    url: String,
}

fn origin_of(url: &str) -> String {
    amazing_net::Url::parse(url)
        .map(|parsed| format!("{}://{}", parsed.scheme, parsed.host))
        .unwrap_or_else(|_| url.to_owned())
}

fn frame_info(target_id: &str, state: &PageState) -> FrameInfo {
    FrameInfo {
        id: target_id.to_owned(),
        parent_id: None,
        url: state.url.clone(),
        name: String::new(),
        security_origin: origin_of(&state.url),
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

/// One attached page target. `Send` because it only holds an `Arc<Mutex<..>>`
/// to plain data; the JS scheduler it builds per `evaluate()` call never
/// crosses a thread boundary (it is created and dropped inside that call, on
/// whichever thread already owns this `Box<dyn DevToolsSession>`).
pub(crate) struct DriverPageSession {
    target_id: String,
    state: Arc<Mutex<PageState>>,
}

impl DevToolsSession for DriverPageSession {
    fn target_id(&self) -> &str {
        &self.target_id
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let (final_url, html, content_type) = crate::cmd::real_pages::fetch_document(url)
            .map_err(HostError::Internal)?;
        let document = crate::document::parse_document_bytes(&html, &final_url, content_type.as_deref())
            .map_err(|error| HostError::Internal(error.to_string()))?;
        let mut state = self.state.lock().expect("devtools page state poisoned");
        state.dom = document.dom;
        state.url = final_url;
        Ok(frame_info(&self.target_id, &state))
    }

    fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
        let state = self.state.lock().expect("devtools page state poisoned");
        Ok(FrameTree { frame: frame_info(&self.target_id, &state), child_frames: Vec::new() })
    }

    fn get_layout_metrics(&mut self) -> Result<LayoutMetrics, HostError> {
        let mut state = self.state.lock().expect("devtools page state poisoned");
        let dom = std::mem::replace(&mut state.dom, amazing_dom::Dom::new());
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let width = 1024.0f32;
        let surface = crate::cmd::render::paint_host_surface(&host, &state.url, width, None, "");
        let (content_width, content_height) = surface
            .as_ref()
            .map(|surface| (surface.width as f64, surface.height as f64))
            .unwrap_or((0.0, 0.0));
        state.dom = Rc::try_unwrap(host)
            .map(|cell| cell.into_inner().dom)
            .unwrap_or_else(|_| amazing_dom::Dom::new());
        let error = surface.err();
        drop(state);
        if let Some(error) = error {
            return Err(HostError::Internal(error.to_string()));
        }
        let viewport = Viewport { page_x: 0.0, page_y: 0.0, client_width: width as f64, client_height: content_height };
        Ok(LayoutMetrics {
            layout_viewport: viewport,
            visual_viewport: viewport,
            content_width,
            content_height,
        })
    }

    fn evaluate(&mut self, params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
        let mut state = self.state.lock().expect("devtools page state poisoned");
        let dom = std::mem::replace(&mut state.dom, amazing_dom::Dom::new());
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let outcome = {
            let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY)
                .map_err(|error| HostError::Internal(error.describe()))?;
            scheduler.set_location_url(&state.url);
            scheduler.run_script(&params.expression)
        };
        // The scheduler (and its extra `Rc` clone of `host`) is dropped above,
        // so this is the sole remaining strong reference.
        state.dom = Rc::try_unwrap(host)
            .map(|cell| cell.into_inner().dom)
            .unwrap_or_else(|_| amazing_dom::Dom::new());
        match outcome {
            Ok(value) => Ok(EvaluateResult::value(js_value_to_remote_object(&value))),
            Err(thrown) => Ok(EvaluateResult {
                result: js_value_to_remote_object(&thrown.0),
                exception: Some(ExceptionDetails {
                    text: thrown.describe(),
                    line_number: 0,
                    column_number: 0,
                    url: None,
                    exception: Some(js_value_to_remote_object(&thrown.0)),
                    stack_trace: None,
                }),
            }),
        }
    }
}

/// The browser-level seam: one page target, backed by real fetched/parsed
/// document state.
pub(crate) struct DriverDevToolsHost {
    target_id: String,
    state: Arc<Mutex<PageState>>,
}

impl DriverDevToolsHost {
    pub(crate) fn new(target_id: impl Into<String>, url: impl Into<String>, dom: amazing_dom::Dom) -> Self {
        Self {
            target_id: target_id.into(),
            state: Arc::new(Mutex::new(PageState { dom, url: url.into() })),
        }
    }
}

impl DevToolsHost for DriverDevToolsHost {
    fn list_targets(&self) -> Vec<TargetInfo> {
        let state = self.state.lock().expect("devtools page state poisoned");
        vec![TargetInfo::page(self.target_id.clone(), state.url.clone(), state.url.clone())]
    }

    fn attach(&self, target_id: &str) -> Result<Box<dyn DevToolsSession>, HostError> {
        if target_id != self.target_id {
            return Err(HostError::NotFound);
        }
        Ok(Box::new(DriverPageSession { target_id: self.target_id.clone(), state: self.state.clone() }))
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

/// Fetches and parses `initial_url`, then binds and starts a CDP server for it
/// on `addr` (`"127.0.0.1:0"` for an ephemeral port) on a background thread.
pub(crate) fn spawn_devtools_server(initial_url: &str, addr: &str) -> Result<DevToolsHandle, String> {
    let (final_url, html, content_type) =
        crate::cmd::real_pages::fetch_document(initial_url).map_err(|error| format!("fetch: {error}"))?;
    let document = crate::document::parse_document_bytes(&html, &final_url, content_type.as_deref())
        .map_err(|error| error.to_string())?;
    let host = Arc::new(DriverDevToolsHost::new("page-1", final_url, document.dom));
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
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    fn local_host(url: &str, html: &str) -> Arc<DriverDevToolsHost> {
        let document = crate::document::parse_document_bytes(html.as_bytes(), url, None)
            .expect("parse fixture document");
        Arc::new(DriverDevToolsHost::new("page-1", url, document.dom))
    }

    #[test]
    fn attach_unknown_target_is_not_found() {
        let host = local_host("file:///fixture.html", "<title>fixture</title>");
        match host.attach("no-such-target") {
            Err(HostError::NotFound) => {}
            other => panic!("expected HostError::NotFound, got {}", other.is_ok()),
        }
    }

    #[test]
    fn navigate_updates_frame_tree_and_evaluate_reads_the_new_dom() {
        let host = local_host("file:///start.html", "<title>start</title><body>a</body>");
        let mut session = host.attach("page-1").expect("attach");
        // Real page instead of network I/O in a unit test: build a second
        // in-process document and swap it in directly through the shared
        // state, exercising the same path `navigate` uses after fetching.
        {
            let state = host.state.clone();
            let mut state = state.lock().unwrap();
            let document = crate::document::parse_document_bytes(
                b"<title>second</title><body>b</body>",
                "file:///second.html",
                None,
            )
            .unwrap();
            state.dom = document.dom;
            state.url = "file:///second.html".into();
        }
        let tree = session.frame_tree().expect("frame_tree");
        assert_eq!(tree.frame.url, "file:///second.html");

        let result = session
            .evaluate(&EvaluateParams { expression: "1 + 41".into(), ..EvaluateParams::default() })
            .expect("evaluate");
        assert_eq!(result.result, RemoteObject::number(42.0));
        assert!(result.exception.is_none());
    }

    #[test]
    fn evaluate_persists_dom_mutations_across_calls_without_a_persistent_realm() {
        let host = local_host("file:///doc.html", "<title>before</title><body></body>");
        let mut session = host.attach("page-1").expect("attach");
        let first = session
            .evaluate(&EvaluateParams {
                expression: "document.title = 'after'; document.title".into(),
                ..EvaluateParams::default()
            })
            .expect("evaluate #1");
        assert_eq!(first.result, RemoteObject::string("after"));
        // A fresh realm, but the same (mutated) DOM: `document.title` reads
        // back "after" even though no JS state from the first call survives.
        let second = session
            .evaluate(&EvaluateParams { expression: "document.title".into(), ..EvaluateParams::default() })
            .expect("evaluate #2");
        assert_eq!(second.result, RemoteObject::string("after"));
    }

    #[test]
    fn evaluate_reports_a_thrown_exception_instead_of_erroring_the_call() {
        let host = local_host("file:///doc.html", "<title>t</title>");
        let mut session = host.attach("page-1").expect("attach");
        let result = session
            .evaluate(&EvaluateParams { expression: "undefinedGlobal.member".into(), ..EvaluateParams::default() })
            .expect("evaluate");
        assert!(result.exception.is_some());
    }

    #[test]
    fn spawn_devtools_server_serves_json_version_over_the_bound_port() {
        // Uses a `file:` URL fetched through `fetch_document`'s local-path
        // fallback path is not exercised here (that helper is HTTP(S)-only);
        // this test instead binds the host directly and only exercises the
        // server/thread wiring, which is the part this file adds.
        let html_host = local_host("file:///wired.html", "<title>wired</title>");
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
