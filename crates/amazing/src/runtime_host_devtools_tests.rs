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

/// A one-shot local HTTP server that answers `routes.len()` requests, one
/// per `(path, content_type, body)` entry, matching each incoming
/// request's path exactly. Lets a fixture exercise real document +
/// subresource fetches (`<script src>`) in one page load, so
/// `Network.requestWillBeSent`/`responseReceived`/`loadingFinished` fire
/// for more than just the top-level document.
fn serve_multi(routes: Vec<(&'static str, &'static str, &'static str)>) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let addr = listener.local_addr().expect("local_addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().take(routes.len()) {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/");
            if let Some((_, content_type, body)) = routes.iter().find(|(route, _, _)| *route == path) {
                let body = body.as_bytes();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            } else {
                let body = b"not found";
                let head = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            }
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

    /// Like [`Self::result`], but also returns every push event (a
    /// message with no matching `id`) seen while waiting for the reply,
    /// in arrival order. CDP-18's engine-driven events (`Network.*`,
    /// `Runtime.consoleAPICalled`/`exceptionThrown`, `Log.entryAdded`)
    /// are pushed asynchronously by the server's own poll loop, not
    /// attached to any one command's reply, so a test proving they
    /// arrived must collect across every command it sends rather than
    /// only inspecting one reply.
    fn result_and_events(&mut self, method: &str, params: Json) -> (Json, Vec<Json>) {
        self.next_id += 1;
        let id = self.next_id;
        let message = Json::Object(vec![
            ("id".into(), Json::Number(id as f64)),
            ("method".into(), Json::String(method.into())),
            ("params".into(), params),
        ]);
        self.ws.write_message(&message.to_string()).expect("write message");
        let mut events = Vec::new();
        loop {
            let reply = self.recv_json();
            if reply.get("id").and_then(Json::as_f64) == Some(id as f64) {
                let result = reply
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| panic!("{method} returned no result: {reply}"));
                return (result, events);
            }
            events.push(reply);
        }
    }

    /// Sends a command and returns the whole reply message, including an
    /// error response (`result()`/`result_and_events()` both assume
    /// success and panic on an `"error"` reply). For tests that expect a
    /// command to fail.
    fn raw(&mut self, method: &str, params: Json) -> Json {
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
                return reply;
            }
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

/// CDP-17: `DOM.describeNode`/`getOuterHTML`/`setAttributeValue`/
/// `setOuterHTML`/`removeNode`, plus comma-separated selector lists in
/// `querySelectorAll`, all through a real WebSocket client against the
/// live DOM.
#[test]
fn end_to_end_dom_mutation_and_selector_lists_work_through_the_real_websocket_client() {
    let host = spawn_test_host(
        "http://example.test/dom.html",
        "<title>Dom</title><body><div id=\"container\"><span id=\"target\" class=\"x\">old</span><p class=\"x\">p</p></div></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
    let root_id = document
        .get("root")
        .and_then(|root| root.get("nodeId"))
        .and_then(Json::as_f64)
        .expect("root nodeId") as u64;

    // Comma-separated selector list: the <span class=x> and <p class=x>.
    let all = client.result(
        "DOM.querySelectorAll",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("span.x, p.x".into())),
        ]),
    );
    assert_eq!(
        all.get("nodeIds").and_then(Json::as_array).map(<[Json]>::len),
        Some(2),
        "{all}"
    );

    let target = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#target".into())),
        ]),
    );
    let target_id = target.get("nodeId").and_then(Json::as_f64).expect("target nodeId") as u64;
    assert!(target_id > 0, "{target}");

    let describe = client.result(
        "DOM.describeNode",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    assert_eq!(
        describe.get("node").and_then(|node| node.get("nodeName")).and_then(Json::as_str),
        Some("SPAN"),
        "{describe}"
    );

    let outer = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let outer_html = outer.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
    assert!(outer_html.contains("old"), "{outer_html}");

    client.result(
        "DOM.setAttributeValue",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(target_id as f64)),
            ("name".into(), Json::String("data-x".into())),
            ("value".into(), Json::String("1".into())),
        ]),
    );
    let outer_after_attr = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let outer_after_attr = outer_after_attr.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
    assert!(outer_after_attr.contains("data-x=\"1\""), "{outer_after_attr}");

    client.result(
        "DOM.setOuterHTML",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(target_id as f64)),
            ("outerHTML".into(), Json::String("<span id=\"target\">new</span>".into())),
        ]),
    );
    let requeried = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#target".into())),
        ]),
    );
    let requeried_id = requeried.get("nodeId").and_then(Json::as_f64).expect("requeried nodeId") as u64;
    assert!(requeried_id > 0, "{requeried}");
    let new_outer = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(requeried_id as f64))]),
    );
    let new_outer = new_outer.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
    assert!(new_outer.contains("new") && !new_outer.contains("old"), "{new_outer}");

    client.result(
        "DOM.removeNode",
        Json::Object(vec![("nodeId".into(), Json::Number(requeried_id as f64))]),
    );
    let gone = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#target".into())),
        ]),
    );
    assert_eq!(gone.get("nodeId").and_then(Json::as_f64), Some(0.0), "{gone}");

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-17: `CSS.getComputedStyleForNode`, `getMatchedStylesForNode` (the
/// matched rules from `amazing-css`) and `getStyleSheetText`.
#[test]
fn end_to_end_css_computed_matched_styles_and_stylesheet_text() {
    let host = spawn_test_host(
        "http://example.test/css.html",
        "<title>Css</title><style>#head { color: red; }</style><body><h1 id=\"head\">Hi</h1></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
    let root_id = document
        .get("root")
        .and_then(|root| root.get("nodeId"))
        .and_then(Json::as_f64)
        .expect("root nodeId") as u64;
    let head = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#head".into())),
        ]),
    );
    let head_id = head.get("nodeId").and_then(Json::as_f64).expect("head nodeId") as u64;
    assert!(head_id > 0, "{head}");

    let computed = client.result(
        "CSS.getComputedStyleForNode",
        Json::Object(vec![("nodeId".into(), Json::Number(head_id as f64))]),
    );
    let computed = computed.get("computedStyle").and_then(Json::as_array).expect("computedStyle array");
    let color = computed
        .iter()
        .find(|entry| entry.get("name").and_then(Json::as_str) == Some("color"))
        .unwrap_or_else(|| panic!("no color property in {computed:?}"));
    assert!(
        color.get("value").and_then(Json::as_str).unwrap_or_default().contains("red"),
        "{color}"
    );

    let matched = client.result(
        "CSS.getMatchedStylesForNode",
        Json::Object(vec![("nodeId".into(), Json::Number(head_id as f64))]),
    );
    let rules = matched.get("matchedCSSRules").and_then(Json::as_array).expect("matchedCSSRules array");
    assert!(!rules.is_empty(), "{rules:?}");
    let first_rule = rules[0].get("rule").expect("rule field");
    assert_eq!(first_rule.get("origin").and_then(Json::as_str), Some("author"), "{first_rule}");
    assert!(
        first_rule
            .get("selectorList")
            .and_then(|list| list.get("text"))
            .and_then(Json::as_str)
            .unwrap_or_default()
            .contains("head"),
        "{first_rule}"
    );

    let sheet_text = client.result(
        "CSS.getStyleSheetText",
        Json::Object(vec![("styleSheetId".into(), Json::String("0".into()))]),
    );
    let text = sheet_text.get("text").and_then(Json::as_str).unwrap_or_default();
    assert!(text.contains("color") && text.contains("red"), "{text}");

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-17: `Input.dispatchMouseEvent`/`dispatchKeyEvent` reach real
/// `addEventListener` handlers on the live page (hit-tested for the mouse
/// event, `document.activeElement` for the key event).
#[test]
fn end_to_end_input_dispatch_reaches_real_dom_event_listeners() {
    let host = spawn_test_host(
        "http://example.test/input.html",
        "<title>Input</title><body>\
         <button id=\"btn\" style=\"position:absolute;left:0;top:0;width:200px;height:200px\">Go</button>\
         <input id=\"field\">\
         <script>\
           document.getElementById('btn').addEventListener('mousedown', () => { window.mouseHit = true; });\
           document.getElementById('field').addEventListener('keydown', () => { window.keyHit = true; });\
         </script>\
         </body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");

    client.result(
        "Input.dispatchMouseEvent",
        Json::Object(vec![
            ("type".into(), Json::String("mousePressed".into())),
            ("x".into(), Json::Number(50.0)),
            ("y".into(), Json::Number(50.0)),
            ("button".into(), Json::String("left".into())),
            ("buttons".into(), Json::Number(1.0)),
            ("clickCount".into(), Json::Number(1.0)),
        ]),
    );
    let mouse_hit = client.result(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("window.mouseHit === true".into())),
            ("returnByValue".into(), Json::Bool(true)),
        ]),
    );
    assert_eq!(
        mouse_hit.get("result").and_then(|result| result.get("value")).and_then(Json::as_bool),
        Some(true),
        "{mouse_hit}"
    );

    client.result(
        "Runtime.evaluate",
        Json::Object(vec![(
            "expression".into(),
            Json::String("document.getElementById('field').focus()".into()),
        )]),
    );
    client.result(
        "Input.dispatchKeyEvent",
        Json::Object(vec![
            ("type".into(), Json::String("keyDown".into())),
            ("key".into(), Json::String("a".into())),
            ("code".into(), Json::String("KeyA".into())),
        ]),
    );
    let key_hit = client.result(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("window.keyHit === true".into())),
            ("returnByValue".into(), Json::Bool(true)),
        ]),
    );
    assert_eq!(
        key_hit.get("result").and_then(|result| result.get("value")).and_then(Json::as_bool),
        Some(true),
        "{key_hit}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-17: `Page.captureScreenshot` returns a real, decodable PNG through
/// the existing raster path.
#[test]
fn end_to_end_capture_screenshot_returns_a_valid_png() {
    let host = spawn_test_host(
        "http://example.test/shot.html",
        "<title>Shot</title><body style=\"background:white\"><h1>Hello</h1></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let shot = client.result(
        "Page.captureScreenshot",
        Json::Object(vec![("format".into(), Json::String("png".into()))]),
    );
    let data = shot.get("data").and_then(Json::as_str).expect("data field");
    let bytes = amazing_devtools::base64::decode(data).expect("valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG signature: {bytes:?}");
    assert!(bytes.len() > 100, "suspiciously small PNG: {} bytes", bytes.len());

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-18's "prove it end to end" test: a real page that fetches an
/// external script and logs to the console, driven through a real
/// WebSocket client against a real fixture HTTP server (document +
/// `<script src>`, both real `amazing_net::fetch` calls, not the
/// in-memory `spawn_test_host` shortcut). Proves `Network.*`,
/// `Runtime.consoleAPICalled`, `Runtime.getProperties`,
/// `Runtime.callFunctionOn` and `Network.getResponseBody` together.
#[test]
fn end_to_end_network_console_and_object_inspection_from_a_real_fetched_page() {
    static DOC: &str = "<title>Live</title><script src=\"/app.js\"></script><body><h1>hi</h1></body>";
    static SCRIPT: &str =
        "console.log('hello from app.js'); globalThis.pageObject = { greeting: 'hi', num: 7 };";
    let addr = serve_multi(vec![("/", "text/html", DOC), ("/app.js", "application/javascript", SCRIPT)]);
    let url = format!("http://{addr}/");
    let init = PageInit::Fetch { target_id: "page-1".into(), url, width: 800.0 };
    let host = Arc::new(DriverDevToolsHost::spawn("page-1", init).expect("spawn devtools host"));
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let server_addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(server_addr, "/devtools/page/page-1");
    let mut events = Vec::new();
    let (_, more) = client.result_and_events("Network.enable", Json::Object(Vec::new()));
    events.extend(more);
    let (_, more) = client.result_and_events("Runtime.enable", Json::Object(Vec::new()));
    events.extend(more);
    let (_, more) = client.result_and_events("Log.enable", Json::Object(Vec::new()));
    events.extend(more);
    // Nudges another poll cycle so anything still queued after the
    // enables above is flushed before this test starts asserting.
    let (_, more) = client.result_and_events(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("1 + 1".into()))]),
    );
    events.extend(more);

    let find_all = |method: &str| -> Vec<Json> {
        events.iter().filter(|event| event.get("method").and_then(Json::as_str) == Some(method)).cloned().collect()
    };
    let request_type = |event: &Json| -> String {
        event.get("params").and_then(|params| params.get("type")).and_then(Json::as_str).unwrap_or_default().to_owned()
    };
    let request_id_of = |event: &Json| -> String {
        event.get("params").and_then(|params| params.get("requestId")).and_then(Json::as_str).unwrap_or_default().to_owned()
    };

    let requests = find_all("Network.requestWillBeSent");
    assert_eq!(requests.len(), 2, "document + script requests: {events:?}");
    let document_request_id = requests
        .iter()
        .find(|event| request_type(event) == "Document")
        .map(request_id_of)
        .expect("a Document requestWillBeSent");
    let script_request_id = requests
        .iter()
        .find(|event| request_type(event) == "Script")
        .map(request_id_of)
        .expect("a Script requestWillBeSent");

    let responses = find_all("Network.responseReceived");
    assert!(
        responses.iter().any(|event| request_id_of(event) == document_request_id),
        "no responseReceived for the document: {events:?}"
    );
    assert!(
        responses.iter().any(|event| request_id_of(event) == script_request_id),
        "no responseReceived for the script: {events:?}"
    );

    let finished = find_all("Network.loadingFinished");
    assert!(finished.iter().any(|event| request_id_of(event) == document_request_id));
    assert!(finished.iter().any(|event| request_id_of(event) == script_request_id));

    let console = find_all("Runtime.consoleAPICalled");
    assert!(
        console.iter().any(|event| {
            event
                .get("params")
                .and_then(|params| params.get("args"))
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .any(|arg| arg.get("value").and_then(Json::as_str).unwrap_or_default().contains("hello from app.js"))
        }),
        "no consoleAPICalled with the script's console.log line: {events:?}"
    );

    // Runtime.getProperties on the object app.js created.
    let evaluated = client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("globalThis.pageObject".into()))]),
    );
    let object_id = evaluated
        .get("result")
        .and_then(|result| result.get("objectId"))
        .and_then(Json::as_str)
        .expect("pageObject should be a live object with an objectId")
        .to_owned();
    let properties = client.result(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(object_id.clone())),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    let descriptors = properties.get("result").and_then(Json::as_array).expect("result array");
    let greeting = descriptors
        .iter()
        .find(|descriptor| descriptor.get("name").and_then(Json::as_str) == Some("greeting"))
        .unwrap_or_else(|| panic!("no greeting property in {descriptors:?}"));
    assert_eq!(
        greeting.get("value").and_then(|value| value.get("value")).and_then(Json::as_str),
        Some("hi")
    );

    // Runtime.callFunctionOn against that same object.
    let called = client.result(
        "Runtime.callFunctionOn",
        Json::Object(vec![
            ("objectId".into(), Json::String(object_id)),
            (
                "functionDeclaration".into(),
                Json::String("function() { return this.num * 2; }".into()),
            ),
            ("returnByValue".into(), Json::Bool(true)),
        ]),
    );
    assert_eq!(
        called.get("result").and_then(|result| result.get("value")).and_then(Json::as_f64),
        Some(14.0),
        "{called}"
    );

    // Network.getResponseBody for the script request.
    let body = client.result(
        "Network.getResponseBody",
        Json::Object(vec![("requestId".into(), Json::String(script_request_id))]),
    );
    assert!(
        body.get("body").and_then(Json::as_str).unwrap_or_default().contains("hello from app.js"),
        "{body}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-18: a page whose own script throws (uncaught) still loads and
/// keeps its realm — `Runtime.exceptionThrown` and `Log.entryAdded`
/// report the bug instead of failing the whole page load.
#[test]
fn end_to_end_uncaught_script_error_is_reported_not_fatal() {
    let host = spawn_test_host(
        "http://example.test/broken.html",
        "<title>Broken</title><script>undefinedGlobal.member;</script><body>still here</body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let mut events = Vec::new();
    let (_, more) = client.result_and_events("Runtime.enable", Json::Object(Vec::new()));
    events.extend(more);
    let (_, more) = client.result_and_events("Log.enable", Json::Object(Vec::new()));
    events.extend(more);
    let (title, more) = client.result_and_events(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("document.title".into())),
            ("returnByValue".into(), Json::Bool(true)),
        ]),
    );
    events.extend(more);
    assert_eq!(title.get("result").and_then(|result| result.get("value")).and_then(Json::as_str), Some("Broken"));

    assert!(
        events.iter().any(|event| event.get("method").and_then(Json::as_str) == Some("Runtime.exceptionThrown")),
        "no Runtime.exceptionThrown for the page's own uncaught error: {events:?}"
    );
    assert!(
        events.iter().any(|event| {
            event.get("method").and_then(Json::as_str) == Some("Log.entryAdded")
                && event.get("params").and_then(|params| params.get("entry")).and_then(|entry| entry.get("source")).and_then(Json::as_str)
                    == Some("javascript")
        }),
        "no Log.entryAdded for the page's own uncaught error: {events:?}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-19's "prove it end to end" test: `DOM.getDocument`/`querySelector`/
/// `getOuterHTML`/`setAttributeValue` and `CSS.getComputedStyleForNode`
/// (already implemented from CDP-16/17) chained onto the *new* work —
/// `DOM.getBoxModel`/`getContentQuads` and `Overlay.highlightNode`/
/// `hideHighlight` — against one live node, over a real WebSocket client.
#[test]
fn end_to_end_box_model_and_overlay_highlight_on_the_live_page() {
    let host = spawn_test_host(
        "http://example.test/box.html",
        "<title>Box</title><body>\
         <div id=\"target\" style=\"position:absolute;left:10px;top:20px;width:100px;\
         height:50px;border:5px solid black;margin:3px;padding:2px\">hi</div>\
         <span id=\"hidden\" style=\"display:none\">gone</span>\
         </body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
    let root_id = document
        .get("root")
        .and_then(|root| root.get("nodeId"))
        .and_then(Json::as_f64)
        .expect("root nodeId") as u64;

    let target = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#target".into())),
        ]),
    );
    let target_id = target.get("nodeId").and_then(Json::as_f64).expect("target nodeId") as u64;
    assert!(target_id > 0, "{target}");

    let outer = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    assert!(
        outer.get("outerHTML").and_then(Json::as_str).unwrap_or_default().contains("hi"),
        "{outer}"
    );

    client.result(
        "DOM.setAttributeValue",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(target_id as f64)),
            ("name".into(), Json::String("data-cdp19".into())),
            ("value".into(), Json::String("1".into())),
        ]),
    );
    let outer_after = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    assert!(
        outer_after.get("outerHTML").and_then(Json::as_str).unwrap_or_default().contains("data-cdp19=\"1\""),
        "{outer_after}"
    );

    let computed = client.result(
        "CSS.getComputedStyleForNode",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let computed = computed.get("computedStyle").and_then(Json::as_array).expect("computedStyle array");
    assert!(
        computed.iter().any(|entry| entry.get("name").and_then(Json::as_str) == Some("width")),
        "{computed:?}"
    );

    // DOM.getBoxModel: content is the CSS content box (100x50, unaffected
    // by position), the border box adds the 2px padding + 5px border on
    // every side (114x64), and the margin box adds the 3px margin
    // (120x70) — pure box-model arithmetic, independent of where the
    // absolutely-positioned box actually landed on the page.
    let box_model = client.result(
        "DOM.getBoxModel",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let model = box_model.get("model").expect("model field");
    assert_eq!(model.get("width").and_then(Json::as_f64), Some(114.0), "{model}");
    assert_eq!(model.get("height").and_then(Json::as_f64), Some(64.0), "{model}");
    let quad_width = |points: &[Json]| -> f64 {
        points[2].as_f64().unwrap() - points[0].as_f64().unwrap()
    };
    let content = model.get("content").and_then(Json::as_array).expect("content quad");
    assert!((quad_width(content) - 100.0).abs() < 0.01, "{content:?}");
    let margin = model.get("margin").and_then(Json::as_array).expect("margin quad");
    assert!((quad_width(margin) - 120.0).abs() < 0.01, "{margin:?}");

    // DOM.getContentQuads: one *content*-box quad (the dispatch layer
    // extracts `model.content`, matching real Chrome's own semantics —
    // "content quads", not border quads); this driver does not split
    // inline content by line box, so every node gets exactly one.
    let quads = client.result(
        "DOM.getContentQuads",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let quads = quads.get("quads").and_then(Json::as_array).expect("quads array");
    assert_eq!(quads.len(), 1, "{quads:?}");
    assert!((quad_width(quads[0].as_array().unwrap()) - 100.0).abs() < 0.01, "{quads:?}");

    // Overlay.highlightNode succeeds on a real, laid-out node...
    let highlight = client.result(
        "Overlay.highlightNode",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(target_id as f64)),
            ("highlightConfig".into(), Json::Object(Vec::new())),
        ]),
    );
    assert_eq!(highlight, Json::Object(Vec::new()), "{highlight}");
    client.result("Overlay.hideHighlight", Json::Object(Vec::new()));

    // ...but reports a real error for a node with no box of its own
    // (`display: none`), exactly as a real highlight request would fail.
    let hidden = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#hidden".into())),
        ]),
    );
    let hidden_id = hidden.get("nodeId").and_then(Json::as_f64).expect("hidden nodeId") as u64;
    let bogus = client.raw(
        "Overlay.highlightNode",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(hidden_id as f64)),
            ("highlightConfig".into(), Json::Object(Vec::new())),
        ]),
    );
    assert!(
        bogus.get("error").is_some(),
        "highlighting a display:none node (no box) should fail: {bogus}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-20: `Debugger.enable`/`scriptParsed` (classic + module)/
/// `getScriptSource`/`setBreakpointByUrl`/`pause`/`resume`, plus
/// `Page.captureScreenshot` (already real since CDP-17; exercised here
/// too since CDP-20's own ticket names it alongside the Debugger work).
#[test]
fn end_to_end_debugger_domain_scripts_breakpoints_and_pause() {
    let host = spawn_test_host(
        "http://example.test/debug.html",
        "<title>Debug</title><body style=\"background:white\">\
         <script>var a = 1;\nvar b = 2;\nvar sum = a + b;</script>\
         <script type=\"module\">export const z = 1;</script>\
         </body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let mut events = Vec::new();

    // Debugger.enable backfills scriptParsed for both scripts the page
    // already ran/collected during its synchronous initial load, before
    // this client ever connected.
    let (enabled, more) = client.result_and_events("Debugger.enable", Json::Object(Vec::new()));
    events.extend(more);
    assert!(enabled.get("debuggerId").and_then(Json::as_str).is_some(), "{enabled}");
    // Nudge another poll cycle so the backfill (queued synchronously
    // inside the enable call, delivered on the server's own poll loop)
    // is flushed before asserting.
    let (_, more) = client.result_and_events(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("1".into()))]),
    );
    events.extend(more);

    let scripts_parsed: Vec<Json> = events
        .iter()
        .filter(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.scriptParsed"))
        .cloned()
        .collect();
    assert_eq!(scripts_parsed.len(), 2, "classic + module: {events:?}");

    let mut classic_id = None;
    let mut module_id = None;
    for parsed in &scripts_parsed {
        let params = parsed.get("params").expect("params");
        let script_id = params.get("scriptId").and_then(Json::as_str).expect("scriptId").to_owned();
        let source = client.result(
            "Debugger.getScriptSource",
            Json::Object(vec![("scriptId".into(), Json::String(script_id.clone()))]),
        );
        let text = source.get("scriptSource").and_then(Json::as_str).unwrap_or_default();
        if text.contains("var sum") {
            classic_id = Some(script_id);
            // 3 lines, 0-based endLine == 2.
            assert_eq!(params.get("endLine").and_then(Json::as_f64), Some(2.0), "{params}");
        } else if text.contains("export const z") {
            module_id = Some(script_id);
            assert_eq!(params.get("endLine").and_then(Json::as_f64), Some(0.0), "{params}");
        } else {
            panic!("unexpected script source: {text}");
        }
    }
    let classic_id = classic_id.expect("classic script reported and readable");
    module_id.expect("module script reported and readable even though it never ran");

    // setBreakpointByUrl resolves a real location for the script whose
    // source actually reaches that line, and an empty (not erroring)
    // `locations` for a line past every known script's end — both
    // scripts share this page's URL (both inline, no `src`), so this
    // also proves the match is genuinely per-script-extent, not just
    // "any script with this url".
    let resolved = client.result(
        "Debugger.setBreakpointByUrl",
        Json::Object(vec![
            ("url".into(), Json::String("http://example.test/debug.html".into())),
            ("lineNumber".into(), Json::Number(2.0)),
        ]),
    );
    let locations = resolved.get("locations").and_then(Json::as_array).expect("locations array");
    assert_eq!(locations.len(), 1, "{resolved}");
    assert_eq!(locations[0].get("scriptId").and_then(Json::as_str), Some(classic_id.as_str()), "{resolved}");
    // Removed immediately: it shares line 2 (0-based) with statements
    // the later breakpoint-hit check below runs, and this assertion is
    // already done with it.
    let resolved_id = resolved.get("breakpointId").and_then(Json::as_str).expect("breakpointId").to_owned();
    client.result("Debugger.removeBreakpoint", Json::Object(vec![("breakpointId".into(), Json::String(resolved_id))]));

    let unresolved = client.result(
        "Debugger.setBreakpointByUrl",
        Json::Object(vec![
            ("url".into(), Json::String("http://example.test/debug.html".into())),
            ("lineNumber".into(), Json::Number(50.0)),
        ]),
    );
    let unresolved_locations = unresolved.get("locations").and_then(Json::as_array).expect("locations array");
    assert!(unresolved_locations.is_empty(), "{unresolved}");

    // A real breakpoint hit: this driver attributes every statement to
    // whichever script the host most recently told the interpreter it
    // was running (see `Interpreter::set_current_script`'s doc comment),
    // and `Runtime.evaluate` never changes that — so a breakpoint set on
    // the page's own URL/line is still live for a later `evaluate` whose
    // *own* second statement happens to fall on that same line, because
    // the hook matches by (url, line) alone, not by which text is
    // actually running. That is an honest reflection of the real
    // limitation, not a workaround for this test.
    let hit_line = client.result(
        "Debugger.setBreakpointByUrl",
        Json::Object(vec![
            ("url".into(), Json::String("http://example.test/debug.html".into())),
            ("lineNumber".into(), Json::Number(1.0)),
        ]),
    );
    let hit_breakpoint_id = hit_line.get("breakpointId").and_then(Json::as_str).expect("breakpointId").to_owned();
    // Every event here is queued synchronously while `dispatch_message`
    // runs the triggering command, but the server's own poll loop
    // (`Connection::pump`) only flushes it *after* that command's reply
    // is already written (`Connection::run`'s `dispatch_message` then
    // `pump()`) — so it shows up as a push ahead of the *next* command's
    // reply, not the triggering command's own `more`. One extra no-op
    // round trip is the same "nudge" the CDP-18 network/console tests
    // already use for exactly this reason.
    let mut evaluate_and_collect = |client: &mut Client, expression: &str| -> Vec<Json> {
        let (_, mut more) = client.result_and_events(
            "Runtime.evaluate",
            Json::Object(vec![("expression".into(), Json::String(expression.into()))]),
        );
        let (_, nudge) = client.result_and_events(
            "Runtime.evaluate",
            Json::Object(vec![("expression".into(), Json::String("0;".into()))]),
        );
        more.extend(nudge);
        more
    };
    let more = evaluate_and_collect(&mut client, "100;\n200;\n300;");
    let paused: Vec<Json> = more
        .iter()
        .filter(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused"))
        .cloned()
        .collect();
    assert_eq!(paused.len(), 1, "{more:?}");
    let paused_params = paused[0].get("params").expect("params");
    assert_eq!(
        paused_params.get("hitBreakpoints").and_then(Json::as_array).map(<[Json]>::len),
        Some(1),
        "{paused_params}"
    );
    assert_eq!(
        paused_params.get("hitBreakpoints").and_then(|hits| hits.as_array()).and_then(|hits| hits.first()).and_then(Json::as_str),
        Some(hit_breakpoint_id.as_str()),
        "{paused_params}"
    );
    assert_eq!(paused_params.get("reason").and_then(Json::as_str), Some("Breakpoint"), "{paused_params}");

    client.result(
        "Debugger.removeBreakpoint",
        Json::Object(vec![("breakpointId".into(), Json::String(hit_breakpoint_id))]),
    );
    let more = evaluate_and_collect(&mut client, "1;\n2;\n3;");
    assert!(
        !more.iter().any(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused")),
        "removed breakpoint should not fire again: {more:?}"
    );

    // Debugger.pause: an unconditional one-shot "stop at the next
    // statement that actually runs" (see `debugger_pause`'s doc comment
    // for why this driver cannot halt anything already executing), and
    // Debugger.resume clears it again if unfired.
    client.result("Debugger.pause", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "42;");
    let forced: Vec<Json> = more
        .iter()
        .filter(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused"))
        .cloned()
        .collect();
    assert_eq!(forced.len(), 1, "{more:?}");
    assert_eq!(
        forced[0].get("params").and_then(|params| params.get("reason")).and_then(Json::as_str),
        Some("other"),
        "{forced:?}"
    );

    client.result("Debugger.pause", Json::Object(Vec::new()));
    client.result("Debugger.resume", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "7;");
    assert!(
        !more.iter().any(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused")),
        "resume should have cleared the pending pause: {more:?}"
    );

    // stepInto (CDP-23): real, in the same after-the-fact sense as
    // Debugger.pause — arms `StepMode::Any`, so the very next statement
    // that runs, at any depth, reports a pause.
    client.result("Debugger.stepInto", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "9;");
    let stepped: Vec<Json> = more
        .iter()
        .filter(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused"))
        .cloned()
        .collect();
    assert_eq!(stepped.len(), 1, "{more:?}");

    // evaluateOnCallFrame (CDP-23): real evaluation, delegated to the
    // page's global realm through the only call frame id this driver ever
    // mints ("cf0"); an unknown id is a genuine NotFound.
    let eval_on_frame = client.result(
        "Debugger.evaluateOnCallFrame",
        Json::Object(vec![
            ("callFrameId".into(), Json::String("cf0".into())),
            ("expression".into(), Json::String("1 + 2".into())),
        ]),
    );
    assert_eq!(
        eval_on_frame.get("result").and_then(|result| result.get("value")).and_then(Json::as_f64),
        Some(3.0),
        "{eval_on_frame}"
    );
    let unknown_frame = client.raw(
        "Debugger.evaluateOnCallFrame",
        Json::Object(vec![
            ("callFrameId".into(), Json::String("cf99".into())),
            ("expression".into(), Json::String("1".into())),
        ]),
    );
    assert!(unknown_frame.get("error").is_some(), "{unknown_frame}");

    // Page.captureScreenshot (CDP-17, exercised here as CDP-20's ticket
    // names it alongside the Debugger domain).
    let shot = client.result("Page.captureScreenshot", Json::Object(vec![("format".into(), Json::String("png".into()))]));
    let data = shot.get("data").and_then(Json::as_str).expect("data field");
    let bytes = amazing_devtools::base64::decode(data).expect("valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG signature: {bytes:?}");

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

/// Like [`serve_once`], but records the raw request head (method line +
/// headers) it received into a shared buffer, so a test can inspect what
/// actually reached the wire — CDP-21's `Network.setCacheDisabled`/
/// `setExtraHTTPHeaders` are proven this way, since this driver has no
/// cache to observe directly.
fn serve_capture(html: &'static str) -> (SocketAddr, Arc<Mutex<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let addr = listener.local_addr().expect("local_addr");
    let captured = Arc::new(Mutex::new(String::new()));
    let captured_thread = captured.clone();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            *captured_thread.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
            let body = html.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    (addr, captured)
}

/// CDP-21: `Network.setCacheDisabled` adds real `Cache-Control`/`Pragma`
/// no-cache request headers (this driver has no HTTP cache subsystem to
/// actually disable — see the module's "Known limitations" — so this is
/// the honest, wire-observable substitute), and
/// `Network.setExtraHTTPHeaders` adds caller-supplied headers to every
/// future request. Both are proven end to end: set before a real
/// `Page.navigate`, then inspect the raw request a real fixture HTTP
/// server received. A second `navigate()` (a reload) proves the state
/// survives across the `PageOwner` replacement that a navigate performs.
#[test]
fn end_to_end_network_set_cache_disabled_and_extra_headers_reach_the_wire() {
    let host = spawn_test_host("http://example.test/start.html", "<title>start</title>");
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    client.result("Network.setCacheDisabled", Json::Object(vec![("cacheDisabled".into(), Json::Bool(true))]));
    client.result(
        "Network.setExtraHTTPHeaders",
        Json::Object(vec![(
            "headers".into(),
            Json::Object(vec![("X-Devtools-Test".into(), Json::String("cdp21".into()))]),
        )]),
    );

    let (fixture_addr, captured) = serve_capture("<title>reloaded</title>");
    client.result(
        "Page.navigate",
        Json::Object(vec![("url".into(), Json::String(format!("http://{fixture_addr}/")))]),
    );
    // The fixture server is one-shot; give its accept thread a moment to
    // receive and record the request before reading the capture back.
    std::thread::sleep(Duration::from_millis(200));
    let head = captured.lock().unwrap().to_ascii_lowercase();
    assert!(head.contains("x-devtools-test: cdp21"), "extra header missing: {head}");
    assert!(head.contains("cache-control: no-cache"), "no-cache Cache-Control missing: {head}");
    assert!(head.contains("pragma: no-cache"), "no-cache Pragma missing: {head}");

    // A reload carries the same state forward (the chicken-and-egg case:
    // the very first document fetch of the *new* PageOwner must already
    // see it).
    let (second_addr, second_captured) = serve_capture("<title>reloaded again</title>");
    client.result(
        "Page.navigate",
        Json::Object(vec![("url".into(), Json::String(format!("http://{second_addr}/")))]),
    );
    std::thread::sleep(Duration::from_millis(200));
    let second_head = second_captured.lock().unwrap().to_ascii_lowercase();
    assert!(second_head.contains("x-devtools-test: cdp21"), "header did not survive reload: {second_head}");
    assert!(second_head.contains("cache-control: no-cache"), "no-cache did not survive reload: {second_head}");

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// Reads a PNG's `IHDR` width/height (bytes 16..20 and 20..24,
/// big-endian) directly from its encoded bytes — the simplest way to
/// prove `Emulation.setDeviceMetricsOverride` actually changed what
/// `Page.captureScreenshot` renders, without a full PNG decoder.
fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() >= 24, "too short to be a PNG: {} bytes", bytes.len());
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (width, height)
}

/// CDP-21: `Emulation.setDeviceMetricsOverride` changes the effective
/// viewport `Page.getLayoutMetrics`/`Page.captureScreenshot` use,
/// `clearDeviceMetricsOverride` reverts it, and `Input.insertText`
/// appends to the focused `<input>`'s `value` and dispatches a real
/// `input` event — all proven through a real WebSocket client against
/// the live page.
#[test]
fn end_to_end_device_metrics_override_and_insert_text() {
    let host = spawn_test_host(
        "http://example.test/form.html",
        "<title>Form</title><body style=\"background:white\">\
         <input id=\"i\" value=\"\">\
         <script>document.getElementById('i').addEventListener('input', () => { globalThis.gotInput = true; });</script>\
         </body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");

    let baseline = client.result("Page.getLayoutMetrics", Json::Object(Vec::new()));
    let baseline_width = baseline
        .get("contentSize")
        .and_then(|size| size.get("width"))
        .and_then(Json::as_f64)
        .expect("baseline width");
    assert_eq!(baseline_width, 800.0, "{baseline}");

    client.result(
        "Emulation.setDeviceMetricsOverride",
        Json::Object(vec![
            ("width".into(), Json::Number(320.0)),
            ("height".into(), Json::Number(480.0)),
            ("deviceScaleFactor".into(), Json::Number(1.0)),
            ("mobile".into(), Json::Bool(false)),
        ]),
    );

    let overridden = client.result("Page.getLayoutMetrics", Json::Object(Vec::new()));
    assert_eq!(
        overridden.get("contentSize").and_then(|size| size.get("width")).and_then(Json::as_f64),
        Some(320.0),
        "{overridden}"
    );

    let shot = client.result("Page.captureScreenshot", Json::Object(vec![("format".into(), Json::String("png".into()))]));
    let data = shot.get("data").and_then(Json::as_str).expect("data field");
    let bytes = amazing_devtools::base64::decode(data).expect("valid base64");
    assert_eq!(png_dimensions(&bytes), (320, 480), "screenshot did not honor the device metrics override");

    client.result("Emulation.clearDeviceMetricsOverride", Json::Object(Vec::new()));
    let cleared = client.result("Page.getLayoutMetrics", Json::Object(Vec::new()));
    assert_eq!(
        cleared.get("contentSize").and_then(|size| size.get("width")).and_then(Json::as_f64),
        Some(800.0),
        "clearDeviceMetricsOverride should restore the page's own base width: {cleared}"
    );

    // Input.insertText: focus the real <input>, insert, then read its
    // value and confirm the real `input` event listener observed it.
    client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("document.getElementById('i').focus();".into()))]),
    );
    client.result("Input.insertText", Json::Object(vec![("text".into(), Json::String("hello".into()))]));
    let value = client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("document.getElementById('i').value".into()))]),
    );
    assert_eq!(value.get("result").and_then(|result| result.get("value")).and_then(Json::as_str), Some("hello"));
    let got_input = client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("!!globalThis.gotInput".into()))]),
    );
    assert_eq!(
        got_input.get("result").and_then(|result| result.get("value")).and_then(Json::as_bool),
        Some(true),
        "insertText should dispatch a real bubbling input event: {got_input}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-22: `Debugger.stepOver` and a real `scopeChain` on paused frames.
/// At this test's own call depth (0, every statement here is top-level) a
/// depth-based `stepOver` (CDP-23) and the CDP-22 one-shot-flag
/// approximation it replaced are indistinguishable — both fire on the next
/// top-level statement — so this test still stands unmodified; see
/// `end_to_end_debugger_depth_based_stepping_and_conditional_breakpoints`
/// below for the depth-sensitive cases CDP-23 actually changes. The paused
/// event's `scopeChain` carries a real, live `"global"` scope object —
/// `Runtime.getProperties` on it reflects genuine `globalThis` state set
/// just before the pause, not a snapshot.
#[test]
fn end_to_end_debugger_step_over_and_scope_chain_reflect_live_globals() {
    let host = spawn_test_host(
        "http://example.test/step.html",
        "<title>Step</title><body><script>var a = 1;</script></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    client.result("Debugger.enable", Json::Object(Vec::new()));
    client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("globalThis.probe = 42;".into()))]),
    );

    // Same "nudge" pattern CDP-20's own Debugger test uses: every event here
    // is queued synchronously during its triggering command but only
    // flushed by the server's poll loop *after* that command's own reply.
    let evaluate_and_collect = |client: &mut Client, expression: &str| -> Vec<Json> {
        let (_, mut more) = client.result_and_events(
            "Runtime.evaluate",
            Json::Object(vec![("expression".into(), Json::String(expression.into()))]),
        );
        let (_, nudge) = client.result_and_events(
            "Runtime.evaluate",
            Json::Object(vec![("expression".into(), Json::String("0;".into()))]),
        );
        more.extend(nudge);
        more
    };

    client.result("Debugger.stepOver", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "1;");
    let paused: Vec<Json> = more
        .iter()
        .filter(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused"))
        .cloned()
        .collect();
    assert_eq!(paused.len(), 1, "stepOver should force a pause on the next statement: {more:?}");
    let params = paused[0].get("params").expect("params");
    let frames = params.get("callFrames").and_then(Json::as_array).expect("callFrames");
    assert_eq!(frames.len(), 1, "{frames:?}");
    let scope_chain = frames[0].get("scopeChain").and_then(Json::as_array).expect("scopeChain");
    assert_eq!(scope_chain.len(), 1, "{scope_chain:?}");
    assert_eq!(scope_chain[0].get("type").and_then(Json::as_str), Some("global"), "{scope_chain:?}");
    let object_id = scope_chain[0]
        .get("object")
        .and_then(|object| object.get("objectId"))
        .and_then(Json::as_str)
        .expect("the global scope's object should carry a real objectId")
        .to_owned();

    let properties = client.result(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(object_id)),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    let descriptors = properties.get("result").and_then(Json::as_array).expect("result array");
    let probe = descriptors
        .iter()
        .find(|descriptor| descriptor.get("name").and_then(Json::as_str) == Some("probe"))
        .unwrap_or_else(|| panic!("no probe property on the global scope object: {descriptors:?}"));
    assert_eq!(
        probe.get("value").and_then(|value| value.get("value")).and_then(Json::as_f64),
        Some(42.0),
        "the scope chain's global object should be live, not a snapshot: {probe}"
    );

    // Debugger.resume clears a still-pending step just like it clears a
    // pending pause (they arm the same flag).
    client.result("Debugger.stepOver", Json::Object(Vec::new()));
    client.result("Debugger.resume", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "2;");
    assert!(
        !more.iter().any(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused")),
        "resume should have cleared the pending step: {more:?}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-23: real depth-based `stepOut` (distinct from the trivial
/// "next-statement" behavior `stepInto`/`Debugger.pause` share), plus
/// `setBreakpointByUrl`'s `condition`.
///
/// `stepOver`/`stepOut`'s reference depth is `last_paused_depth` — the depth
/// recorded at the most recent pause (see `PageOwner::step_reference_depth`'s
/// doc comment) — and every `Runtime.evaluate` call is its own fresh
/// top-level script, so its *first* statement is always observed at depth 0
/// (there is no way for a client of this driver to arm a step and have the
/// very next statement it sees already be mid-call; entering a call is what
/// *increments* depth, and that only happens after the hook already fired
/// for the statement making the call). That makes `Over`/`Out` behave
/// identically to `Any` for a reference depth of 0 or more once execution is
/// back at depth 0 or shallower — the meaningful, honestly-testable
/// distinction this test proves instead is `StepMode::Out`'s depth-0
/// reference case: stepping out of the top-level frame is not a thing that
/// can ever happen (there is nothing shallower than depth 0), so it must
/// never fire, unlike `stepInto`/`pause`'s `Any`, which always fires on the
/// next statement regardless of depth.
#[test]
fn end_to_end_debugger_depth_based_step_out_and_conditional_breakpoints() {
    // `level1`'s body statement is deliberately on its own line (line 3,
    // 1-based; CDP `lineNumber: 2`), distinct from every later `Runtime.
    // evaluate` call site: a call statement's own line is relative to
    // *that* evaluate call's freshly parsed text (always line 1 there), so
    // sharing a line number with the body would make a breakpoint on that
    // line match both the call site and the body statement by (url, line)
    // alone (a real, separately-documented driver limitation — see the
    // module's "Known limitations" and CDP-20's own test) — this test wants
    // exactly one match per call, to isolate the depth/condition behavior
    // it is actually proving.
    let host = spawn_test_host(
        "http://example.test/step2.html",
        "<title>Step2</title><body><script>\nfunction level1() {\n  var inner1 = 1;\n}\n</script></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    client.result("Debugger.enable", Json::Object(Vec::new()));

    let evaluate_and_collect = |client: &mut Client, expression: &str| -> Vec<Json> {
        let (_, mut more) = client.result_and_events(
            "Runtime.evaluate",
            Json::Object(vec![("expression".into(), Json::String(expression.into()))]),
        );
        let (_, nudge) = client.result_and_events(
            "Runtime.evaluate",
            Json::Object(vec![("expression".into(), Json::String("0;".into()))]),
        );
        more.extend(nudge);
        more
    };
    let paused_events = |events: &[Json]| -> Vec<Json> {
        events.iter().filter(|event| event.get("method").and_then(Json::as_str) == Some("Debugger.paused")).cloned().collect()
    };

    // With no prior pause, the reference depth defaults to 0: `stepOut`
    // arms `Out(0)`, which requires `depth < 0` — never true for a `u32` —
    // so three ordinary top-level statements produce no pause at all. This
    // is the behavior that actually distinguishes `stepOut` from
    // `stepInto`/`pause`'s `Any`, which the next block proves fires
    // unconditionally in the exact same setting.
    client.result("Debugger.stepOut", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "1;\n2;\n3;");
    assert!(paused_events(&more).is_empty(), "stepOut from the top-level frame should never fire: {more:?}");

    client.result("Debugger.stepInto", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "4;\n5;");
    assert_eq!(paused_events(&more).len(), 1, "stepInto should fire on the very next statement regardless of depth: {more:?}");

    // Establish a real depth-1 reference: a breakpoint on `level1`'s own
    // (only) statement, hit while actually running inside that call.
    let breakpoint = client.result(
        "Debugger.setBreakpointByUrl",
        Json::Object(vec![
            ("url".into(), Json::String("http://example.test/step2.html".into())),
            ("lineNumber".into(), Json::Number(2.0)),
            ("condition".into(), Json::String("false".into())),
        ]),
    );
    let false_cond_id = breakpoint.get("breakpointId").and_then(Json::as_str).expect("breakpointId").to_owned();
    let more = evaluate_and_collect(&mut client, "level1();");
    assert!(paused_events(&more).is_empty(), "a false condition should suppress the breakpoint entirely: {more:?}");
    client.result("Debugger.removeBreakpoint", Json::Object(vec![("breakpointId".into(), Json::String(false_cond_id))]));

    let breakpoint = client.result(
        "Debugger.setBreakpointByUrl",
        Json::Object(vec![
            ("url".into(), Json::String("http://example.test/step2.html".into())),
            ("lineNumber".into(), Json::Number(2.0)),
            ("condition".into(), Json::String("1 === 1".into())),
        ]),
    );
    let true_cond_id = breakpoint.get("breakpointId").and_then(Json::as_str).expect("breakpointId").to_owned();
    let more = evaluate_and_collect(&mut client, "level1();");
    let hit = paused_events(&more);
    assert_eq!(hit.len(), 1, "a true condition should let the breakpoint fire: {more:?}");
    let hit_params = hit[0].get("params").expect("params");
    assert_eq!(
        hit_params.get("hitBreakpoints").and_then(Json::as_array).and_then(|hits| hits.first()).and_then(Json::as_str),
        Some(true_cond_id.as_str()),
        "{hit_params}"
    );
    assert_eq!(hit_params.get("reason").and_then(Json::as_str), Some("Breakpoint"), "{hit_params}");
    client.result("Debugger.removeBreakpoint", Json::Object(vec![("breakpointId".into(), Json::String(true_cond_id))]));

    // An erroring condition counts as false, same as a literal `false` —
    // the breakpoint is suppressed, not treated as an unconditional hit and
    // not surfaced as a driver error either.
    let breakpoint = client.result(
        "Debugger.setBreakpointByUrl",
        Json::Object(vec![
            ("url".into(), Json::String("http://example.test/step2.html".into())),
            ("lineNumber".into(), Json::Number(2.0)),
            ("condition".into(), Json::String("throw new Error('boom')".into())),
        ]),
    );
    let error_cond_id = breakpoint.get("breakpointId").and_then(Json::as_str).expect("breakpointId").to_owned();
    let more = evaluate_and_collect(&mut client, "level1();");
    assert!(paused_events(&more).is_empty(), "an erroring condition should count as false: {more:?}");
    client.result("Debugger.removeBreakpoint", Json::Object(vec![("breakpointId".into(), Json::String(error_cond_id))]));

    // That true-condition hit above ran with `level1`'s body at depth 1
    // (see `drain_debugger_paused`, which sets `last_paused_depth` for
    // every queued pause, breakpoint or forced), so a `stepOut` armed now
    // has reference depth 1: `Out(1)` requires `depth < 1`, which the very
    // next top-level statement (depth 0) satisfies — proving the reference
    // depth genuinely came from that pause, not a stale default of 0.
    client.result("Debugger.stepOut", Json::Object(Vec::new()));
    let more = evaluate_and_collect(&mut client, "6;");
    assert_eq!(paused_events(&more).len(), 1, "stepOut with a depth-1 reference should fire once back at depth 0: {more:?}");

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-22 completeness: `Runtime.evaluate`/`callFunctionOn`'s
/// `returnByValue: true` now serializes objects/arrays structurally instead
/// of minting an unwanted `objectId`, and `Runtime.getProperties` with
/// `ownProperties: false` now walks the prototype chain instead of silently
/// behaving exactly like `true`.
#[test]
fn end_to_end_runtime_return_by_value_and_inherited_properties() {
    let host = spawn_test_host("http://example.test/runtime.html", "<title>Runtime</title>");
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");

    // returnByValue on Runtime.evaluate: a nested array/object is serialized
    // structurally, with no objectId minted.
    let evaluated = client.result(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("[1, 'two', { three: 3 }]".into())),
            ("returnByValue".into(), Json::Bool(true)),
        ]),
    );
    let result = evaluated.get("result").expect("result");
    assert!(result.get("objectId").is_none(), "returnByValue should not mint an objectId: {result}");
    let array = result.get("value").and_then(Json::as_array).expect("array value");
    assert_eq!(array.len(), 3, "{array:?}");
    assert_eq!(array[0].as_f64(), Some(1.0), "{array:?}");
    assert_eq!(array[1].as_str(), Some("two"), "{array:?}");
    assert_eq!(array[2].get("three").and_then(Json::as_f64), Some(3.0), "{array:?}");

    // returnByValue on Runtime.callFunctionOn: same serialization path,
    // against a real live object this time (not a fresh literal).
    let object_result = client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("({ x: 1, y: 2 })".into()))]),
    );
    let object_id = object_result
        .get("result")
        .and_then(|result| result.get("objectId"))
        .and_then(Json::as_str)
        .expect("a plain object should have a real objectId when not returned by value")
        .to_owned();
    let called = client.result(
        "Runtime.callFunctionOn",
        Json::Object(vec![
            ("objectId".into(), Json::String(object_id)),
            (
                "functionDeclaration".into(),
                Json::String("function() { return { sum: this.x + this.y }; }".into()),
            ),
            ("returnByValue".into(), Json::Bool(true)),
        ]),
    );
    let called_result = called.get("result").expect("result");
    assert!(called_result.get("objectId").is_none(), "{called_result}");
    assert_eq!(
        called_result.get("value").and_then(|value| value.get("sum")).and_then(Json::as_f64),
        Some(3.0),
        "{called_result}"
    );

    // getProperties(ownProperties: false) walks the prototype chain.
    let with_proto = client.result(
        "Runtime.evaluate",
        Json::Object(vec![(
            "expression".into(),
            Json::String(
                "(() => { const o = Object.create({ inherited: 99 }); o.own = 1; return o; })()".into(),
            ),
        )]),
    );
    let with_proto_id = with_proto
        .get("result")
        .and_then(|result| result.get("objectId"))
        .and_then(Json::as_str)
        .expect("objectId")
        .to_owned();

    let own_only = client.result(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(with_proto_id.clone())),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    let own_descriptors = own_only.get("result").and_then(Json::as_array).expect("result array");
    assert!(
        !own_descriptors.iter().any(|d| d.get("name").and_then(Json::as_str) == Some("inherited")),
        "ownProperties: true should not see the prototype's own property: {own_descriptors:?}"
    );

    let with_inherited = client.result(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(with_proto_id)),
            ("ownProperties".into(), Json::Bool(false)),
        ]),
    );
    let all_descriptors = with_inherited.get("result").and_then(Json::as_array).expect("result array");
    let own_prop = all_descriptors
        .iter()
        .find(|d| d.get("name").and_then(Json::as_str) == Some("own"))
        .unwrap_or_else(|| panic!("own property missing: {all_descriptors:?}"));
    assert_eq!(own_prop.get("isOwn").and_then(Json::as_bool), Some(true), "{own_prop}");
    let inherited_prop = all_descriptors
        .iter()
        .find(|d| d.get("name").and_then(Json::as_str) == Some("inherited"))
        .unwrap_or_else(|| panic!("inherited property missing from an ownProperties:false walk: {all_descriptors:?}"));
    assert_eq!(inherited_prop.get("isOwn").and_then(Json::as_bool), Some(false), "{inherited_prop}");
    assert_eq!(
        inherited_prop.get("value").and_then(|value| value.get("value")).and_then(Json::as_f64),
        Some(99.0),
        "{inherited_prop}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-24: `Runtime.getProperties` always attaches a real `preview`
/// (`ObjectPreview`) for an object-typed property value; `Runtime.evaluate`
/// only does when `generatePreview: true`; the preview caps at 5 own
/// properties with `overflow: true` past that; and `objectGroup` tagging on
/// `evaluate`/`callFunctionOn` plus `Runtime.releaseObjectGroup` actually
/// invalidates the tagged `objectId`s (including the default `""` group).
#[test]
fn end_to_end_runtime_object_previews_and_release_object_group_work_through_the_real_websocket_client() {
    let host = spawn_test_host("http://example.test/doc.html", "<title>t</title>");
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");

    // evaluate() without generatePreview: no preview at all.
    let no_preview = client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("({ a: 1, b: 2 })".into()))]),
    );
    let no_preview_result = no_preview.get("result").expect("result");
    assert!(no_preview_result.get("preview").is_none(), "{no_preview_result}");

    // evaluate() with generatePreview: true builds a real ObjectPreview.
    let small = client.result(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("({ a: 1, b: 'two' })".into())),
            ("generatePreview".into(), Json::Bool(true)),
        ]),
    );
    let small_result = small.get("result").expect("result");
    let small_preview = small_result.get("preview").unwrap_or_else(|| panic!("no preview: {small_result}"));
    assert_eq!(small_preview.get("type").and_then(Json::as_str), Some("object"), "{small_preview}");
    assert_eq!(small_preview.get("overflow").and_then(Json::as_bool), Some(false), "{small_preview}");
    let small_props = small_preview.get("properties").and_then(Json::as_array).expect("properties array");
    assert_eq!(small_props.len(), 2, "{small_props:?}");
    let prop_a = small_props
        .iter()
        .find(|p| p.get("name").and_then(Json::as_str) == Some("a"))
        .unwrap_or_else(|| panic!("no 'a' property preview: {small_props:?}"));
    assert_eq!(prop_a.get("type").and_then(Json::as_str), Some("number"), "{prop_a}");
    assert_eq!(prop_a.get("value").and_then(Json::as_str), Some("1"), "{prop_a}");

    // A wide object (>5 own properties) overflows the preview cap.
    let wide = client.result(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("({ a: 1, b: 2, c: 3, d: 4, e: 5, f: 6 })".into())),
            ("generatePreview".into(), Json::Bool(true)),
        ]),
    );
    let wide_preview =
        wide.get("result").and_then(|r| r.get("preview")).unwrap_or_else(|| panic!("no preview: {wide}"));
    assert_eq!(wide_preview.get("overflow").and_then(Json::as_bool), Some(true), "{wide_preview}");
    assert_eq!(
        wide_preview.get("properties").and_then(Json::as_array).map(<[Json]>::len),
        Some(5),
        "{wide_preview}"
    );

    // Runtime.getProperties always attaches a preview to an object-typed
    // value, with no generatePreview flag involved at all.
    let holder = client.result(
        "Runtime.evaluate",
        Json::Object(vec![(
            "expression".into(),
            Json::String("({ nested: { x: 1, y: 2 } })".into()),
        )]),
    );
    let holder_id = holder
        .get("result")
        .and_then(|r| r.get("objectId"))
        .and_then(Json::as_str)
        .expect("holder objectId")
        .to_owned();
    let properties = client.result(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(holder_id)),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    let descriptors = properties.get("result").and_then(Json::as_array).expect("result array");
    let nested = descriptors
        .iter()
        .find(|d| d.get("name").and_then(Json::as_str) == Some("nested"))
        .unwrap_or_else(|| panic!("no 'nested' property: {descriptors:?}"));
    let nested_preview = nested
        .get("value")
        .and_then(|v| v.get("preview"))
        .unwrap_or_else(|| panic!("getProperties should attach a preview with no flag needed: {nested}"));
    assert_eq!(nested_preview.get("type").and_then(Json::as_str), Some("object"), "{nested_preview}");

    // objectGroup tagging + releaseObjectGroup: evaluate() under a named
    // group, then release it, then prove the objectId no longer resolves.
    let grouped = client.result(
        "Runtime.evaluate",
        Json::Object(vec![
            ("expression".into(), Json::String("({ grouped: true })".into())),
            ("objectGroup".into(), Json::String("cdp24-group".into())),
        ]),
    );
    let grouped_id = grouped
        .get("result")
        .and_then(|r| r.get("objectId"))
        .and_then(Json::as_str)
        .expect("grouped objectId")
        .to_owned();
    // Still resolvable before release.
    client.result(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(grouped_id.clone())),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    client.result(
        "Runtime.releaseObjectGroup",
        Json::Object(vec![("objectGroup".into(), Json::String("cdp24-group".into()))]),
    );
    let after_release = client.raw(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(grouped_id)),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    assert!(after_release.get("error").is_some(), "releaseObjectGroup should invalidate its ids: {after_release}");

    // The default ("" / omitted objectGroup) bucket also releases correctly.
    let ungrouped = client.result(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("({ ungrouped: true })".into()))]),
    );
    let ungrouped_id = ungrouped
        .get("result")
        .and_then(|r| r.get("objectId"))
        .and_then(Json::as_str)
        .expect("ungrouped objectId")
        .to_owned();
    client.result("Runtime.releaseObjectGroup", Json::Object(vec![("objectGroup".into(), Json::String("".into()))]));
    let ungrouped_after = client.raw(
        "Runtime.getProperties",
        Json::Object(vec![
            ("objectId".into(), Json::String(ungrouped_id)),
            ("ownProperties".into(), Json::Bool(true)),
        ]),
    );
    assert!(
        ungrouped_after.get("error").is_some(),
        "releaseObjectGroup(\"\") should invalidate the default bucket: {ungrouped_after}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-24's own re-proof of the DOM chain the ticket names explicitly:
/// `getDocument` -> `querySelector` -> `getOuterHTML` -> `setAttributeValue`
/// -> `removeNode`, end to end over a real WebSocket client against the
/// live DOM.
#[test]
fn end_to_end_dom_get_document_query_select_outer_html_set_attribute_and_remove_node_chain() {
    let host = spawn_test_host(
        "http://example.test/dom24.html",
        "<title>Dom24</title><body><div id=\"root\"><p id=\"target\" class=\"y\">hello</p></div></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");

    let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
    let root_id = document
        .get("root")
        .and_then(|root| root.get("nodeId"))
        .and_then(Json::as_f64)
        .expect("root nodeId") as u64;

    let found = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#target".into())),
        ]),
    );
    let target_id = found.get("nodeId").and_then(Json::as_f64).expect("target nodeId") as u64;
    assert!(target_id > 0, "{found}");

    let outer = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let outer_html = outer.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
    assert!(outer_html.contains("hello"), "{outer_html}");

    client.result(
        "DOM.setAttributeValue",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(target_id as f64)),
            ("name".into(), Json::String("data-cdp24".into())),
            ("value".into(), Json::String("yes".into())),
        ]),
    );
    let outer_after = client.result(
        "DOM.getOuterHTML",
        Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
    );
    let outer_after = outer_after.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
    assert!(outer_after.contains("data-cdp24=\"yes\""), "{outer_after}");

    client.result("DOM.removeNode", Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]));
    let gone = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_id as f64)),
            ("selector".into(), Json::String("#target".into())),
        ]),
    );
    assert_eq!(gone.get("nodeId").and_then(Json::as_f64), Some(0.0), "removeNode should drop it from the tree: {gone}");

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-24's own re-proof of the Network domain: `Network.enable` gates
/// `requestWillBeSent`/`responseReceived`/`loadingFinished` events from a
/// **live** `Page.navigate` call made after `enable` — not just from the
/// initial page-load fetch other tests exercise — proving the events are
/// wired to the fetch path itself and not to page construction.
#[test]
fn end_to_end_network_enable_then_live_navigate_fires_the_fetch_path_events() {
    let host = spawn_test_host("http://example.test/start.html", "<title>start</title>");
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let mut events = Vec::new();
    let (_, more) = client.result_and_events("Network.enable", Json::Object(Vec::new()));
    events.extend(more);

    let fixture_addr = serve_once("<title>navigated</title>");
    let (_, more) = client.result_and_events(
        "Page.navigate",
        Json::Object(vec![("url".into(), Json::String(format!("http://{fixture_addr}/")))]),
    );
    events.extend(more);
    // Nudge another poll cycle so anything still queued after navigate()
    // returned is flushed before this test starts asserting.
    let (_, more) = client.result_and_events(
        "Runtime.evaluate",
        Json::Object(vec![("expression".into(), Json::String("1 + 1".into()))]),
    );
    events.extend(more);

    let find_all = |method: &str| -> Vec<Json> {
        events.iter().filter(|event| event.get("method").and_then(Json::as_str) == Some(method)).cloned().collect()
    };
    let request_id_of = |event: &Json| -> String {
        event.get("params").and_then(|params| params.get("requestId")).and_then(Json::as_str).unwrap_or_default().to_owned()
    };

    let requests = find_all("Network.requestWillBeSent");
    assert_eq!(requests.len(), 1, "the live navigate should fire exactly one requestWillBeSent: {events:?}");
    let request_id = request_id_of(&requests[0]);
    assert!(!request_id.is_empty(), "{requests:?}");

    let responses = find_all("Network.responseReceived");
    assert!(
        responses.iter().any(|event| request_id_of(event) == request_id),
        "no responseReceived for the live navigate's request: {events:?}"
    );

    let finished = find_all("Network.loadingFinished");
    assert!(
        finished.iter().any(|event| request_id_of(event) == request_id),
        "no loadingFinished for the live navigate's request: {events:?}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-25: `Accessibility.enable` queues a real `loadComplete` with the
/// current tree's root, then `getFullAXTree`/`getPartialAXTree`/
/// `getChildAXNodes`/`queryAXTree` all read the live tree through
/// `AxTreeWatcher`, end to end over a real WebSocket client.
#[test]
fn end_to_end_accessibility_enable_full_partial_child_and_query_ax_tree() {
    let host = spawn_test_host(
        "http://example.test/a11y.html",
        "<title>A11y</title><body><nav><a href=\"#\">Home</a></nav><h1>Welcome</h1><button>Go</button><p>Body text</p></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");

    let mut events = Vec::new();
    let (_, more) = client.result_and_events("Accessibility.enable", Json::Object(Vec::new()));
    events.extend(more);
    // The server's poll loop only flushes an event queued during command N's
    // own dispatch after N's reply is written, so `loadComplete` (queued by
    // `enable` itself) is only observed on this next command — the same
    // nudge pattern every other event-based test in this file uses.
    let (full, more) = client.result_and_events("Accessibility.getFullAXTree", Json::Object(Vec::new()));
    events.extend(more);
    let load_complete = events
        .iter()
        .find(|event| event.get("method").and_then(Json::as_str) == Some("Accessibility.loadComplete"))
        .unwrap_or_else(|| panic!("no Accessibility.loadComplete after enable: {events:?}"));
    let root = load_complete.get("params").and_then(|params| params.get("root")).expect("loadComplete.root");
    assert!(
        root.get("nodeId").and_then(Json::as_str).is_some_and(|id| !id.is_empty()),
        "loadComplete root should carry a real AXNodeId: {root}"
    );
    let nodes = full.get("nodes").and_then(Json::as_array).expect("nodes array");
    assert!(nodes.len() > 1, "expected more than just the root: {nodes:?}");
    let role_of = |node: &Json| -> String {
        node.get("role").and_then(|r| r.get("value")).and_then(Json::as_str).unwrap_or_default().to_owned()
    };
    let name_of = |node: &Json| -> String {
        node.get("name").and_then(|r| r.get("value")).and_then(Json::as_str).unwrap_or_default().to_owned()
    };
    assert!(nodes.iter().any(|n| role_of(n) == "navigation"), "{nodes:?}");
    let heading = nodes.iter().find(|n| role_of(n) == "heading").unwrap_or_else(|| panic!("no heading: {nodes:?}"));
    assert_eq!(name_of(heading), "Welcome");
    let button = nodes.iter().find(|n| role_of(n) == "button").unwrap_or_else(|| panic!("no button: {nodes:?}"));
    assert_eq!(name_of(button), "Go");

    // getPartialAXTree with fetchRelatives: true against the <button>'s real
    // DOM nodeId (via DOM.getDocument/querySelector, the same nodeId
    // registry every other domain's node params resolve through).
    let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
    let root_dom_id =
        document.get("root").and_then(|root| root.get("nodeId")).and_then(Json::as_f64).expect("root nodeId") as u64;
    let button_query = client.result(
        "DOM.querySelector",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(root_dom_id as f64)),
            ("selector".into(), Json::String("button".into())),
        ]),
    );
    let button_dom_id = button_query.get("nodeId").and_then(Json::as_f64).expect("button nodeId") as u64;

    let partial = client.result(
        "Accessibility.getPartialAXTree",
        Json::Object(vec![
            ("nodeId".into(), Json::Number(button_dom_id as f64)),
            ("fetchRelatives".into(), Json::Bool(true)),
        ]),
    );
    let partial_nodes = partial.get("nodes").and_then(Json::as_array).expect("nodes array");
    assert!(
        partial_nodes.iter().any(|n| role_of(n) == "button" && name_of(n) == "Go"),
        "{partial_nodes:?}"
    );
    assert!(
        partial_nodes.len() > 1,
        "fetchRelatives: true should include at least one ancestor besides the node itself: {partial_nodes:?}"
    );

    // getChildAXNodes against the root's own AXNodeId: same child count as
    // getFullAXTree's root childIds, each with its own childIds filled in.
    let root_ax_id = nodes[0].get("nodeId").and_then(Json::as_str).expect("root ax id").to_owned();
    let root_child_ids = nodes[0].get("childIds").and_then(Json::as_array).map(<[Json]>::len).unwrap_or(0);
    let children = client.result(
        "Accessibility.getChildAXNodes",
        Json::Object(vec![("id".into(), Json::String(root_ax_id))]),
    );
    let child_nodes = children.get("nodes").and_then(Json::as_array).expect("nodes array");
    assert_eq!(child_nodes.len(), root_child_ids, "{child_nodes:?}");

    // queryAXTree: by role alone, and by accessible name alone.
    let by_role = client.result(
        "Accessibility.queryAXTree",
        Json::Object(vec![("role".into(), Json::String("BUTTON".into()))]),
    );
    let by_role_nodes = by_role.get("nodes").and_then(Json::as_array).expect("nodes array");
    assert_eq!(by_role_nodes.len(), 1, "role match should be case-insensitive: {by_role_nodes:?}");
    assert_eq!(name_of(&by_role_nodes[0]), "Go");

    // "Welcome" alone also matches the heading's own static-text child (same
    // accessible name), so combine it with a role filter to pin down the
    // heading exactly — proving the two filters are applied together (AND),
    // not as independent alternatives.
    let by_name_and_role = client.result(
        "Accessibility.queryAXTree",
        Json::Object(vec![
            ("accessibleName".into(), Json::String("Welcome".into())),
            ("role".into(), Json::String("heading".into())),
        ]),
    );
    let by_name_and_role_nodes = by_name_and_role.get("nodes").and_then(Json::as_array).expect("nodes array");
    assert_eq!(by_name_and_role_nodes.len(), 1, "{by_name_and_role_nodes:?}");
    assert_eq!(role_of(&by_name_and_role_nodes[0]), "heading");
    assert_eq!(name_of(&by_name_and_role_nodes[0]), "Welcome");

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

/// CDP-25: `Accessibility.nodesUpdated` reflects a real DOM mutation made
/// between two tree reads (this driver has no continuous mutation-observer
/// hook, so a checkpoint is only actually recorded when a client asks to
/// look at the tree — see `PageOwner::refresh_ax_tree`'s doc comment), and
/// never reports a `Removed` node (real CDP signals DOM removal through
/// `DOM.*` instead).
#[test]
fn end_to_end_accessibility_nodes_updated_reflects_a_mutation_between_two_reads() {
    let host = spawn_test_host(
        "http://example.test/a11y-live.html",
        "<title>Live</title><body><h1>old</h1><button>bye</button></body>",
    );
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));

    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let mut events = Vec::new();
    let (_, more) = client.result_and_events("Accessibility.enable", Json::Object(Vec::new()));
    events.extend(more);

    // Baseline checkpoint: first read after enable, nothing to diff against.
    let (_, more) = client.result_and_events("Accessibility.getFullAXTree", Json::Object(Vec::new()));
    events.extend(more);
    assert!(
        !events.iter().any(|event| event.get("method").and_then(Json::as_str) == Some("Accessibility.nodesUpdated")),
        "the first read after enable should not itself report an update: {events:?}"
    );

    // Mutate the heading's text and remove the button through the real JS
    // realm, then read again — this is the checkpoint the mutation is
    // observed on.
    client.result(
        "Runtime.evaluate",
        Json::Object(vec![(
            "expression".into(),
            Json::String(
                "document.querySelector('h1').textContent = 'new'; document.querySelector('button').remove();"
                    .into(),
            ),
        )]),
    );
    let (_, more) = client.result_and_events("Accessibility.getFullAXTree", Json::Object(Vec::new()));
    events.extend(more);
    // Nudge another poll cycle so the nodesUpdated queued by that read's own
    // checkpoint is flushed before this test starts asserting (same timing
    // quirk `loadComplete` has in the other new test).
    let (_, more) =
        client.result_and_events("Runtime.evaluate", Json::Object(vec![("expression".into(), Json::String("1 + 1".into()))]));
    events.extend(more);

    let updates = events
        .iter()
        .filter(|event| event.get("method").and_then(Json::as_str) == Some("Accessibility.nodesUpdated"))
        .flat_map(|event| {
            event
                .get("params")
                .and_then(|p| p.get("nodes"))
                .and_then(Json::as_array)
                .map(<[Json]>::to_vec)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    assert!(!updates.is_empty(), "expected at least one nodesUpdated node: {events:?}");
    let heading_update = updates
        .iter()
        .find(|node| node.get("role").and_then(|r| r.get("value")).and_then(Json::as_str) == Some("heading"))
        .unwrap_or_else(|| panic!("no updated heading node: {updates:?}"));
    assert_eq!(
        heading_update.get("name").and_then(|n| n.get("value")).and_then(Json::as_str),
        Some("new"),
        "{heading_update}"
    );
    assert!(
        !updates.iter().any(|node| node.get("role").and_then(|r| r.get("value")).and_then(Json::as_str) == Some("button")),
        "a removed node should not surface through nodesUpdated: {updates:?}"
    );

    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

#[test]
fn real_cdp_empty_accessors_project_undefined_without_losing_data_or_getters() {
    let host = spawn_test_host("http://example.test/descriptors.html", "<title>Descriptors</title>");
    let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("local_addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));
    let mut client = Client::connect(addr, "/devtools/page/page-1");
    let source = "globalThis.descriptorProbe = Object.defineProperties({}, {empty: {get: undefined, set: undefined, enumerable: true}, data: {value: 7, enumerable: true}, active: {get() {return 11}, enumerable: true}})";
    let evaluated = client.result("Runtime.evaluate", Json::Object(vec![
        ("expression".into(), Json::String(source.into())),
        ("generatePreview".into(), Json::Bool(true)),
    ]));
    let result = evaluated.get("result").expect("result");
    let id = result.get("objectId").and_then(Json::as_str).expect("object id").to_owned();
    let preview = result.get("preview").and_then(|p| p.get("properties"))
        .and_then(Json::as_array).expect("preview properties");
    let empty = preview.iter().find(|p| p.get("name").and_then(Json::as_str) == Some("empty")).expect("empty preview");
    assert_eq!(empty.get("type").and_then(Json::as_str), Some("undefined"), "{empty}");
    let properties = client.result("Runtime.getProperties", Json::Object(vec![
        ("objectId".into(), Json::String(id)), ("ownProperties".into(), Json::Bool(true)),
    ]));
    let properties = properties.get("result").and_then(Json::as_array).expect("properties");
    for (name, expected) in [("empty", None), ("data", Some(7.0)), ("active", Some(11.0))] {
        let property = properties.iter().find(|p| p.get("name").and_then(Json::as_str) == Some(name)).expect("property");
        let value = property.get("value").expect("remote value");
        if let Some(expected) = expected { assert_eq!(value.get("value").and_then(Json::as_f64), Some(expected), "{value}"); }
        else { assert_eq!(value.get("type").and_then(Json::as_str), Some("undefined"), "{value}"); }
    }
    let by_value = client.result("Runtime.evaluate", Json::Object(vec![
        ("expression".into(), Json::String("descriptorProbe".into())),
        ("returnByValue".into(), Json::Bool(true)),
    ]));
    let value = by_value.get("result").and_then(|r| r.get("value")).expect("value");
    assert!(matches!(value.get("empty"), Some(Json::Null)), "{value}");
    assert_eq!(value.get("data").and_then(Json::as_f64), Some(7.0));
    assert_eq!(value.get("active").and_then(Json::as_f64), Some(11.0));
    shutdown.store(true, Ordering::Relaxed);
    let _ = join.join();
}

#[test]
fn native_script_record_execution_and_source_wire_keep_exact_units() {
    for payload in [vec![0xd800], vec![0xdc00], vec![0xd800, 0xdc00], vec![0xfffd], vec![0xe000]] {
        let units: Vec<u16> = "var retainedSource='".encode_utf16().chain(payload.iter().copied()).chain("';".encode_utf16()).collect();
        let mut dom = amazing_dom::Dom::new();
        let script = dom.create_element_expanded_in(amazing_dom::NodeIdx(0), Some(amazing_html::xml::HTML_NAMESPACE), None, "script").unwrap();
        let text = dom.create_text(script, "");
        dom.replace_character_data(text, 0, 0, &units).unwrap();
        dom.pre_insert(script, amazing_dom::NodeIdx(0), None).unwrap();
        dom.set_document_scripting_enabled(amazing_dom::NodeIdx(0), true).unwrap();
        let mut page = PageOwner::from_parsed("exact-source", "https://example.test/source", dom, 800.0).unwrap();
        assert_eq!(page.scheduler.run_script("retainedSource").unwrap(), amazing_js::value::JsValue::Str(amazing_js::value::JsString::from_units(payload)));
        assert_eq!(page.scripts.len(), 1);
        let source = page.get_script_source(&page.scripts[0].script_id).unwrap();
        assert_eq!(source, units);
        let wire = Json::Object(vec![("scriptSource".into(), Json::string_utf16(source))]).to_string();
        let decoded = Json::parse(&wire).unwrap();
        assert_eq!(decoded.get("scriptSource").unwrap().as_utf16().unwrap().as_ref(), units);
    }
    for text in ["plain", "é", "😀", "function scalar() {}"] {
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(script_units_hash(&units), script_hash(text));
        assert_eq!(script_units_extent(&units), script_extent(text));
    }
}
