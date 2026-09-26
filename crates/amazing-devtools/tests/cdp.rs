//! End-to-end tests: a scripted CDP client speaks JSON-RPC over a real
//! WebSocket to a [`CdpServer`] backed by the in-memory [`MockHost`].

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use amazing_devtools::host::RemoteObject;
use amazing_devtools::json::Json;
use amazing_devtools::mock::MockHost;
use amazing_devtools::server::CdpServer;
use amazing_devtools::ws::{Message, WsConnection};

macro_rules! obj {
    ($($key:expr => $value:expr),* $(,)?) => {
        Json::Object(vec![$((String::from($key), $value)),*])
    };
}

fn s(value: impl Into<String>) -> Json {
    Json::String(value.into())
}

fn n(value: f64) -> Json {
    Json::Number(value)
}

fn empty() -> Json {
    Json::Object(Vec::new())
}

fn server(host: Arc<MockHost>) -> (SocketAddr, Arc<AtomicBool>) {
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutdown);
    thread::spawn(move || {
        let _ = server.serve(flag);
    });
    (addr, shutdown)
}

fn http_get(addr: SocketAddr, path: &str) -> Json {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read");
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("head terminator");
    let body = &raw[split + 4..];
    Json::parse(std::str::from_utf8(body).unwrap()).expect("json")
}

struct Client {
    ws: WsConnection<TcpStream>,
    events: Vec<Json>,
    next_id: u64,
}

impl Client {
    fn connect(addr: SocketAddr, path: &str) -> Self {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
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
            head.to_ascii_lowercase()
                .contains(&format!("sec-websocket-accept: {}", expected.to_ascii_lowercase())),
            "bad accept header: {head}"
        );
        Self {
            ws: WsConnection::as_client(stream),
            events: Vec::new(),
            next_id: 0,
        }
    }

    fn send_text(&mut self, text: &str) {
        self.ws.write_message(text).expect("write message");
    }

    fn recv_json(&mut self) -> Json {
        match self.ws.poll() {
            Ok(Some(Message::Text(text))) => Json::parse(&text).expect("server json"),
            Ok(Some(Message::Binary(_))) => panic!("unexpected binary frame"),
            Ok(None) => panic!("timed out waiting for a server message"),
            Err(error) => panic!("websocket error: {error}"),
        }
    }

    fn command(&mut self, method: &str, params: Json, session: Option<&str>) -> Json {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = obj! {
            "id" => n(id as f64),
            "method" => s(method),
            "params" => params,
        };
        if let Some(session) = session {
            message.insert("sessionId", s(session));
        }
        self.send_text(&message.to_string());
        loop {
            let reply = self.recv_json();
            if reply.get("id").and_then(Json::as_u64) == Some(id) {
                return reply;
            }
            self.events.push(reply);
        }
    }

    fn result(&mut self, method: &str, params: Json) -> Json {
        let reply = self.command(method, params, None);
        reply
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} returned no result: {reply}"))
    }

    fn result_session(&mut self, method: &str, params: Json, session: &str) -> Json {
        let reply = self.command(method, params, Some(session));
        assert_eq!(reply.get("sessionId").and_then(Json::as_str), Some(session));
        reply
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} returned no result: {reply}"))
    }

    fn error(&mut self, method: &str, params: Json) -> Json {
        let reply = self.command(method, params, None);
        reply
            .get("error")
            .cloned()
            .unwrap_or_else(|| panic!("{method} unexpectedly succeeded: {reply}"))
    }

    fn wait_event(&mut self, method: &str) -> Json {
        if let Some(index) = self
            .events
            .iter()
            .position(|event| event.get("method").and_then(Json::as_str) == Some(method))
        {
            return self.events.remove(index);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let event = self.recv_json();
            if event.get("method").and_then(Json::as_str) == Some(method) {
                return event;
            }
            self.events.push(event);
        }
        panic!("timed out waiting for event {method}");
    }
}

#[test]
fn discovery_endpoints_advertise_websocket_urls() {
    let (addr, shutdown) = server(Arc::new(MockHost::new()));
    let version = http_get(addr, "/json/version");
    assert_eq!(version.get("Protocol-Version").unwrap().as_str(), Some("1.3"));
    assert_eq!(version.get("Browser").unwrap().as_str(), Some("Amazing/0.0.1"));
    assert!(version
        .get("webSocketDebuggerUrl")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("/devtools/browser"));

    let list = http_get(addr, "/json/list");
    let entries = list.as_array().expect("array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].get("id").unwrap().as_str(), Some("page-1"));
    assert_eq!(entries[0].get("type").unwrap().as_str(), Some("page"));
    assert!(entries[0]
        .get("webSocketDebuggerUrl")
        .unwrap()
        .as_str()
        .unwrap()
        .ends_with("/devtools/page/page-1"));

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn page_endpoint_round_trips_every_domain() {
    let host = Arc::new(MockHost::new());
    host.script()
        .lock()
        .unwrap()
        .push_log("javascript", "error", "boom");
    let (addr, shutdown) = server(Arc::clone(&host));
    let mut client = Client::connect(addr, "/devtools/page/page-1");

    let info = client.result("Target.getTargetInfo", empty());
    assert_eq!(
        info.get("targetInfo").unwrap().get("targetId").unwrap().as_str(),
        Some("page-1")
    );

    // Page.enable emits the existing frame and an init lifecycle event.
    client.result("Page.enable", empty());
    let enabled_frame = client.wait_event("Page.frameNavigated");
    assert_eq!(
        enabled_frame
            .get("params")
            .unwrap()
            .get("frame")
            .unwrap()
            .get("url")
            .unwrap()
            .as_str(),
        Some("about:blank")
    );
    client.wait_event("Page.lifecycleEvent");

    let nav = client.result("Page.navigate", obj! { "url" => s("https://example.com/") });
    assert_eq!(nav.get("frameId").unwrap().as_str(), Some("F1"));
    let navigated = client.wait_event("Page.frameNavigated");
    assert_eq!(
        navigated
            .get("params")
            .unwrap()
            .get("frame")
            .unwrap()
            .get("url")
            .unwrap()
            .as_str(),
        Some("https://example.com/")
    );
    assert_eq!(
        host.script().lock().unwrap().navigations,
        ["https://example.com/"]
    );
    client.wait_event("Page.lifecycleEvent"); // DOMContentLoaded
    client.wait_event("Page.lifecycleEvent"); // load

    let tree = client.result("Page.getFrameTree", empty());
    assert_eq!(
        tree.get("frameTree").unwrap().get("frame").unwrap().get("id").unwrap().as_str(),
        Some("F1")
    );

    // Runtime.
    client.result("Runtime.enable", empty());
    let context = client.wait_event("Runtime.executionContextCreated");
    assert_eq!(
        context
            .get("params")
            .unwrap()
            .get("context")
            .unwrap()
            .get("origin")
            .unwrap()
            .as_str(),
        Some("https://example.com")
    );
    let evaluated = client.result(
        "Runtime.evaluate",
        obj! { "expression" => s("6*7"), "returnByValue" => Json::Bool(true) },
    );
    assert_eq!(
        evaluated.get("result").unwrap().get("value").unwrap().as_f64(),
        Some(42.0)
    );
    host.script().lock().unwrap().push_console(
        "log",
        vec![RemoteObject::string("hi")],
    );
    let console = client.wait_event("Runtime.consoleAPICalled");
    assert_eq!(
        console.get("params").unwrap().get("type").unwrap().as_str(),
        Some("log")
    );

    // DOM and CSS.
    let document = client.result("DOM.getDocument", obj! { "depth" => n(-1.0) });
    let root = document.get("root").unwrap();
    assert_eq!(root.get("nodeName").unwrap().as_str(), Some("HTML"));
    let root_id = root.get("nodeId").unwrap().as_u64().unwrap();
    let found = client.result(
        "DOM.querySelector",
        obj! { "nodeId" => n(root_id as f64), "selector" => s("#main") },
    );
    let node_id = found.get("nodeId").unwrap().as_u64().unwrap();
    assert!(node_id > 0);
    let html = client.result("DOM.getOuterHTML", obj! { "nodeId" => n(node_id as f64) });
    assert!(html.get("outerHTML").unwrap().as_str().unwrap().contains("id=\"main\""));
    let described = client.result(
        "DOM.describeNode",
        obj! { "nodeId" => n(node_id as f64), "depth" => n(1.0) },
    );
    assert_eq!(
        described.get("node").unwrap().get("localName").unwrap().as_str(),
        Some("div")
    );

    let computed = client.result(
        "CSS.getComputedStyleForNode",
        obj! { "nodeId" => n(node_id as f64) },
    );
    let style = computed.get("computedStyle").unwrap().as_array().unwrap();
    assert!(style
        .iter()
        .any(|entry| entry.get("name").unwrap().as_str() == Some("display")));

    // Network.
    client.result("Network.enable", empty());
    host.script()
        .lock()
        .unwrap()
        .push_network("r1", "https://example.com/x", 200);
    let request = client.wait_event("Network.requestWillBeSent");
    assert_eq!(
        request.get("params").unwrap().get("requestId").unwrap().as_str(),
        Some("r1")
    );
    let response = client.wait_event("Network.responseReceived");
    assert_eq!(
        response
            .get("params")
            .unwrap()
            .get("response")
            .unwrap()
            .get("status")
            .unwrap()
            .as_u64(),
        Some(200)
    );
    client.wait_event("Network.loadingFinished");

    // Log.
    client.result("Log.enable", empty());
    let log = client.wait_event("Log.entryAdded");
    assert_eq!(
        log.get("params").unwrap().get("entry").unwrap().get("text").unwrap().as_str(),
        Some("boom")
    );

    // Screenshot comes back base64-encoded from raw bytes.
    let screenshot = client.result("Page.captureScreenshot", obj! { "format" => s("png") });
    assert_eq!(
        screenshot.get("data").unwrap().as_str(),
        Some(amazing_devtools::base64::encode(b"mock-png").as_str())
    );

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn browser_endpoint_supports_flattened_sessions() {
    let (addr, shutdown) = server(Arc::new(MockHost::new()));
    let mut client = Client::connect(addr, "/devtools/browser");

    let targets = client.result("Target.getTargets", empty());
    let infos = targets.get("targetInfos").unwrap().as_array().unwrap();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].get("targetId").unwrap().as_str(), Some("page-1"));

    let attached = client.result(
        "Target.attachToTarget",
        obj! { "targetId" => s("page-1"), "flatten" => Json::Bool(true) },
    );
    let session = attached.get("sessionId").unwrap().as_str().unwrap().to_string();

    client.result_session("Runtime.enable", empty(), &session);
    let evaluated = client.result_session(
        "Runtime.evaluate",
        obj! { "expression" => s("1+1") },
        &session,
    );
    assert_eq!(
        evaluated.get("result").unwrap().get("value").unwrap().as_f64(),
        Some(42.0)
    );

    // A page-domain command without a sessionId is refused on this endpoint.
    let error = client.error("Runtime.evaluate", obj! { "expression" => s("1") });
    assert_eq!(error.get("code").unwrap().as_i64(), Some(-32000));

    let version = client.result("Browser.getVersion", empty());
    assert_eq!(version.get("protocolVersion").unwrap().as_str(), Some("1.3"));

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn errors_and_protocol_failures_are_reported() {
    let (addr, shutdown) = server(Arc::new(MockHost::new()));
    let mut client = Client::connect(addr, "/devtools/page/page-1");

    let unknown = client.error("Page.doesNotExist", empty());
    assert_eq!(unknown.get("code").unwrap().as_i64(), Some(-32601));

    let missing_param = client.error("Page.navigate", empty());
    assert_eq!(missing_param.get("code").unwrap().as_i64(), Some(-32602));

    // A syntactically broken message yields a parse error with a null id.
    client.send_text("{not json");
    let reply = client.recv_json();
    assert_eq!(reply.get("id").unwrap(), &Json::Null);
    assert_eq!(reply.get("error").unwrap().get("code").unwrap().as_i64(), Some(-32700));

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn page_endpoint_serves_inspector_read_paths() {
    let host = Arc::new(MockHost::new());
    host.script().lock().unwrap().targets[0].url = "https://example.com/".into();
    let (addr, shutdown) = server(Arc::clone(&host));
    let mut client = Client::connect(addr, "/devtools/page/page-1");

    let document = client.result("DOM.getDocument", obj! { "depth" => n(-1.0) });
    let root_id = document
        .get("root")
        .unwrap()
        .get("nodeId")
        .unwrap()
        .as_u64()
        .unwrap();
    let found = client.result(
        "DOM.querySelector",
        obj! { "nodeId" => n(root_id as f64), "selector" => s("#main") },
    );
    let node_id = found.get("nodeId").unwrap().as_u64().unwrap();

    let attributes = client.result("DOM.getAttributes", obj! { "nodeId" => n(node_id as f64) });
    let attrs = attributes.get("attributes").unwrap().as_array().unwrap();
    assert!(attrs.iter().any(|value| value.as_str() == Some("main")));

    let model = client.result("DOM.getBoxModel", obj! { "nodeId" => n(node_id as f64) });
    assert_eq!(
        model.get("model").unwrap().get("width").unwrap().as_i64(),
        Some(100)
    );
    let quads = client.result("DOM.getContentQuads", obj! { "nodeId" => n(node_id as f64) });
    assert_eq!(
        quads.get("quads").unwrap().as_array().unwrap()[0]
            .as_array()
            .unwrap()
            .len(),
        8
    );

    let search = client.result("DOM.performSearch", obj! { "query" => s("main") });
    let search_id = search.get("searchId").unwrap().as_str().unwrap().to_string();
    assert!(search.get("resultCount").unwrap().as_u64().unwrap() >= 1);
    let results = client.result(
        "DOM.getSearchResults",
        obj! {
            "searchId" => s(search_id.clone()),
            "fromIndex" => n(0.0),
            "toIndex" => n(10.0),
        },
    );
    assert!(!results.get("nodeIds").unwrap().as_array().unwrap().is_empty());
    client.result("DOM.discardSearchResults", obj! { "searchId" => s(search_id) });

    let pushed = client.result(
        "DOM.pushNodesByBackendIdsToFrontend",
        obj! { "backendNodeIds" => Json::Array(vec![n(4.0)]) },
    );
    assert_eq!(pushed.get("nodeIds").unwrap().as_array().unwrap().len(), 1);

    client.result(
        "DOM.setNodeValue",
        obj! { "nodeId" => n(node_id as f64), "value" => s("changed") },
    );
    assert!(host
        .script()
        .lock()
        .unwrap()
        .node_values
        .iter()
        .any(|(_, value)| value == "changed"));

    let sheet = client.result("CSS.getStyleSheetText", obj! { "styleSheetId" => s("sheet-1") });
    assert_eq!(sheet.get("text").unwrap().as_str(), Some("body { color: red }"));
    let set = client.result(
        "CSS.setStyleSheetText",
        obj! { "styleSheetId" => s("sheet-1"), "text" => s("body { color: blue }") },
    );
    assert_eq!(set.get("text").unwrap().as_str(), Some("body { color: blue }"));
    assert_eq!(
        host.script().lock().unwrap().style_sheet_text.as_deref(),
        Some("body { color: blue }")
    );

    let media = client.result("CSS.getMediaQueries", empty());
    assert_eq!(
        media.get("medias").unwrap().as_array().unwrap()[0]
            .get("text")
            .unwrap()
            .as_str(),
        Some("(min-width: 600px)")
    );

    let backgrounds = client.result("CSS.getBackgroundColors", obj! { "nodeId" => n(node_id as f64) });
    let colors = backgrounds.get("colors").unwrap().as_array().unwrap();
    assert_eq!(colors.len(), 2);
    assert_eq!(colors[0].as_str(), Some("rgb(255, 255, 255)"));
    assert_eq!(colors[1], Json::Null);

    let metrics = client.result("Page.getLayoutMetrics", empty());
    assert_eq!(
        metrics.get("layoutViewport").unwrap().get("clientWidth").unwrap().as_f64(),
        Some(800.0)
    );
    assert_eq!(
        metrics.get("contentSize").unwrap().get("height").unwrap().as_f64(),
        Some(1200.0)
    );

    let tree = client.result("Page.getResourceTree", empty());
    assert_eq!(
        tree.get("frameTree").unwrap().get("resources").unwrap().as_array().unwrap()[0]
            .get("url")
            .unwrap()
            .as_str(),
        Some("https://example.com/")
    );

    let content = client.result(
        "Page.getResourceContent",
        obj! { "frameId" => s("F1"), "url" => s("https://example.com/") },
    );
    assert_eq!(content.get("content").unwrap().as_str(), Some("<html>"));
    assert_eq!(content.get("base64Encoded").unwrap().as_bool(), Some(false));

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn browser_endpoint_pushes_target_lifecycle_events() {
    let host = Arc::new(MockHost::new());
    let (addr, shutdown) = server(Arc::clone(&host));
    let mut client = Client::connect(addr, "/devtools/browser");

    client.result("Target.setDiscoverTargets", obj! { "discover" => Json::Bool(true) });
    let created = client.wait_event("Target.targetCreated");
    assert_eq!(
        created.get("params").unwrap().get("targetInfo").unwrap().get("targetId").unwrap().as_str(),
        Some("page-1")
    );

    client.result(
        "Target.setAutoAttach",
        obj! { "autoAttach" => Json::Bool(true), "waitForDebuggerOnStart" => Json::Bool(false) },
    );
    host.script()
        .lock()
        .unwrap()
        .push_target_created("page-2", "https://two/", "Two");
    let created = client.wait_event("Target.targetCreated");
    assert_eq!(
        created.get("params").unwrap().get("targetInfo").unwrap().get("targetId").unwrap().as_str(),
        Some("page-2")
    );
    let attached = client.wait_event("Target.attachedToTarget");
    let session = attached
        .get("params")
        .unwrap()
        .get("sessionId")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let evaluated = client.result_session("Runtime.evaluate", obj! { "expression" => s("1") }, &session);
    assert_eq!(
        evaluated.get("result").unwrap().get("value").unwrap().as_f64(),
        Some(42.0)
    );

    host.script().lock().unwrap().push_target_destroyed("page-2");
    client.wait_event("Target.targetDestroyed");
    let detached = client.wait_event("Target.detachedFromTarget");
    assert_eq!(
        detached.get("params").unwrap().get("sessionId").unwrap().as_str(),
        Some(session.as_str())
    );

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn page_endpoint_dispatches_input_and_hit_tests() {
    let host = Arc::new(MockHost::new());
    let (addr, shutdown) = server(Arc::clone(&host));
    let mut client = Client::connect(addr, "/devtools/page/page-1");

    client.result("Input.setIgnoreInputEvents", obj! { "ignore" => Json::Bool(true) });
    assert_eq!(host.script().lock().unwrap().ignore_input, Some(true));

    client.result(
        "Input.dispatchMouseEvent",
        obj! {
            "type" => s("mousePressed"),
            "x" => n(3.0),
            "y" => n(4.0),
            "button" => s("left"),
            "buttons" => n(1.0),
            "clickCount" => n(1.0),
        },
    );
    client.result(
        "Input.dispatchKeyEvent",
        obj! {
            "type" => s("keyDown"),
            "key" => s("Enter"),
            "code" => s("Enter"),
            "windowsVirtualKeyCode" => n(13.0),
        },
    );
    client.result(
        "Input.dispatchTouchEvent",
        obj! {
            "type" => s("touchStart"),
            "touchPoints" => Json::Array(vec![obj! { "x" => n(1.0), "y" => n(2.0), "id" => n(7.0) }]),
        },
    );
    client.result("Input.insertText", obj! { "text" => s("hi") });

    let script = host.script();
    {
        let recording = script.lock().unwrap();
        assert_eq!(recording.mouse_events.len(), 1);
        assert_eq!(recording.mouse_events[0].x, 3.0);
        assert_eq!(recording.mouse_events[0].click_count, 1);
        assert_eq!(recording.key_events[0].windows_virtual_key_code, Some(13));
        assert_eq!(recording.touch_events[0].points[0].id, 7.0);
        assert_eq!(recording.inserted_text, ["hi"]);
    }

    let hit = client.result("DOM.getNodeForLocation", obj! { "x" => n(5.0), "y" => n(6.0) });
    assert_eq!(hit.get("backendNodeId").unwrap().as_u64(), Some(4));
    assert_eq!(hit.get("frameId").unwrap().as_str(), Some("F1"));
    assert!(hit.get("nodeId").unwrap().as_u64().unwrap() > 0);

    shutdown.store(true, Ordering::Relaxed);
}
