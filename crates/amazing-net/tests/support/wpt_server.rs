//! Std-only loopback ports of selected WPT fetch/api/resources and xhr/resources handlers.
//! See amazing-fetch/tests/wpt/fetch_xhr/PROVENANCE.md for pinned sources and scope.
#![allow(dead_code)]
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct SeenRequest {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub raw_headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl SeenRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
    fn query(&self, name: &str) -> Option<String> {
        self.target.split_once('?')?.1.split('&').find_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (percent_decode(key) == name).then(|| percent_decode(value))
        })
    }
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }
}

pub struct WptServer {
    port: u16,
    running: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
    thread: Option<JoinHandle<()>>,
}
impl WptServer {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let running = Arc::new(AtomicBool::new(true));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(BTreeMap::new()));
        let running_thread = running.clone();
        let seen_thread = seen.clone();
        let thread = thread::spawn(move || {
            while running_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Accepted sockets inherit nonblocking mode on Windows.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        if let Some(request) = read_request(&mut stream) {
                            seen_thread.lock().unwrap().push(request.clone());
                            if is_websocket_upgrade(&request) {
                                // A WebSocket connection stays open for the
                                // test's duration; run it on its own thread
                                // so the accept loop keeps serving other
                                // connections.
                                thread::spawn(move || handle_websocket(stream, request));
                            } else if matches!(
                                request.path(),
                                "/xhr/resources/chunked.py"
                                    | "/xhr/resources/trickle.py"
                                    | "/fetch/api/resources/infinite-slow-response.py"
                            ) {
                                write_chunks(&mut stream, &request);
                            } else {
                                let reply = handle(&request, &mut state.lock().unwrap());
                                let _ = stream.write_all(&reply);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("WPT accept: {error}"),
                }
            }
        });
        Self {
            port,
            running,
            seen,
            thread: Some(thread),
        }
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
    pub fn seen(&self) -> Vec<SeenRequest> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for WptServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<SeenRequest> {
    let mut input = Vec::new();
    let mut bytes = [0; 4096];
    let header_end = loop {
        let n = stream.read(&mut bytes).ok()?;
        if n == 0 {
            return None;
        }
        input.extend_from_slice(&bytes[..n]);
        if input.len() > 128 * 1024 {
            return None;
        }
        if let Some(i) = input.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&input[..header_end]);
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let mut headers = BTreeMap::new();
    let mut raw_headers = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let raw_value = value.strip_prefix(' ').unwrap_or(value).to_owned();
        headers.insert(
            name.to_ascii_lowercase(),
            raw_value.trim_matches([' ', '\t']).to_owned(),
        );
        raw_headers.push((name.to_owned(), raw_value));
    }
    let body_len = headers
        .get("content-length")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    if body_len > 1024 * 1024 {
        return None;
    }
    while input.len() - header_end < body_len {
        let n = stream.read(&mut bytes).ok()?;
        if n == 0 {
            return None;
        }
        input.extend_from_slice(&bytes[..n]);
    }
    Some(SeenRequest {
        method,
        target,
        headers,
        raw_headers,
        body: input[header_end..header_end + body_len].to_vec(),
    })
}
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
fn reply(status: u16, text: &str, mut headers: Vec<(String, String)>, body: &[u8]) -> Vec<u8> {
    headers.push(("Content-Length".into(), body.len().to_string()));
    headers.push(("Connection".into(), "close".into()));
    let mut out = format!("HTTP/1.1 {status} {text}\r\n").into_bytes();
    for (name, value) in headers {
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}
fn header(name: &str, value: impl Into<String>) -> (String, String) {
    (name.into(), value.into())
}
fn write_chunks(stream: &mut TcpStream, request: &SeenRequest) {
    let slow = request.path().ends_with("trickle.py")
        || request.path().ends_with("infinite-slow-response.py");
    let head: &[u8] = if request.path().ends_with("chunked.py") {
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nTrailer: X-Test-Me\r\nConnection: close\r\n\r\n"
    } else {
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    };
    let _ = stream.write_all(head);
    let chunks: &[&[u8]] = if request.path().ends_with("chunked.py") {
        &[
            b"First chunk\r\n",
            b"Second chunk\r\n",
            b"Yet another (third) chunk\r\n",
            b"Yet another (fourth) chunk\r\n",
        ]
    } else {
        &[b"a", b"b", b"c", b"d"]
    };
    for chunk in chunks {
        if stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .is_err()
        {
            return;
        }
        if stream.write_all(chunk).is_err() || stream.write_all(b"\r\n").is_err() {
            return;
        }
        if slow {
            thread::sleep(Duration::from_millis(20));
        }
    }
    let ending: &[u8] = if request.path().ends_with("chunked.py") {
        b"0\r\nX-Test-Me: Trailer header value\r\n\r\n"
    } else {
        b"0\r\n\r\n"
    };
    let _ = stream.write_all(ending);
}
/// wptserve's `resources/cache.py` records, per request, which of a fixed
/// set of conditional/no-cache request headers were observed, as a stashed
/// list of dicts later retrieved verbatim (`&querystate`) by the test as
/// JSON. The shared `state` stash only holds strings, so the list is kept
/// encoded with control-character delimiters that never appear in header
/// values (`\x1e` between entries, `\x1f` between an entry's pairs, `\x01`
/// between a pair's key and value) and only turned into JSON on read.
fn cache_state_decode(raw: &str) -> Vec<Vec<(String, String)>> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split('\u{1e}')
        .map(|entry| {
            if entry.is_empty() {
                return Vec::new();
            }
            entry
                .split('\u{1f}')
                .filter_map(|pair| {
                    pair.split_once('\u{1}')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                })
                .collect()
        })
        .collect()
}
fn cache_state_encode(entries: &[Vec<(String, String)>]) -> String {
    entries
        .iter()
        .map(|entry| {
            entry
                .iter()
                .map(|(k, v)| format!("{k}\u{1}{v}"))
                .collect::<Vec<_>>()
                .join("\u{1f}")
        })
        .collect::<Vec<_>>()
        .join("\u{1e}")
}
fn cache_state_to_json(entries: &[Vec<(String, String)>]) -> String {
    let mut out = String::from("[");
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        for (j, (k, v)) in entry.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)));
        }
        out.push('}');
    }
    out.push(']');
    out
}
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}
fn cors(request: &SeenRequest, headers: &mut Vec<(String, String)>) {
    headers.push(header(
        "Access-Control-Allow-Origin",
        request.header("origin").unwrap_or("*"),
    ));
    headers.push(header("Access-Control-Allow-Credentials", "true"));
}
fn handle(request: &SeenRequest, state: &mut BTreeMap<String, String>) -> Vec<u8> {
    let path = request.path();
    let fetch = path.starts_with("/fetch/api/resources/");
    let xhr = path.starts_with("/xhr/resources/");
    let name = path.rsplit('/').next().unwrap_or("");
    let mut headers = vec![header("Content-Type", "text/plain")];
    // WPT tests such as xhr/responsetype.any.js open "/" and wait for
    // readyState LOADING, which needs a non-empty body; the real WPT server
    // returns a directory listing there.
    if path == "/" {
        return reply(200, "OK", headers, b"amazing wpt shim root\n");
    }
    if fetch && name == "data.json" {
        return reply(200, "OK", vec![header("Content-Type", "application/json")],
            b"{\"key\": \"value\"}\n");
    }
    if fetch && name == "stash-put.py" {
        headers.push(header("Access-Control-Allow-Origin", "*"));
        if request.method == "OPTIONS" {
            headers.push(header("Access-Control-Allow-Methods", "*"));
            headers.push(header("Access-Control-Allow-Headers", "*"));
        } else if let (Some(key), Some(value)) = (request.query("key"), request.query("value")) {
            state.insert(key, value);
        }
        return reply(200, "OK", headers, b"done");
    }
    if fetch && name == "cache.py" {
        let token = request.query("token").unwrap_or_default();
        let stash_key = format!("cache:{token}");
        if request.query("querystate").is_some() {
            // Matches wptserve's `request.server.stash.take(token)`: a
            // destructive read of the recorded per-request header state.
            let recorded = cache_state_decode(state.remove(&stash_key).unwrap_or_default().as_str());
            return reply(
                200,
                "OK",
                vec![header("Content-Type", "text/plain")],
                cache_state_to_json(&recorded).as_bytes(),
            );
        }
        let content = request.query("content").unwrap_or_default();
        let tag = request.query("tag");
        let date = request.query("date");
        let expires = request.query("expires");
        let vary = request.query("vary");
        let cc = request.query("cache_control");
        let redirect = request.query("redirect");
        let ignore = request.query("ignore").is_some();
        let inm = request.header("if-none-match").map(str::to_owned);
        let ims = request.header("if-modified-since").map(str::to_owned);
        let pragma = request.header("pragma").map(str::to_owned);
        let cache_control_header = request.header("cache-control").map(str::to_owned);

        let mut recorded = cache_state_decode(state.remove(&stash_key).unwrap_or_default().as_str());
        let mut entry: Vec<(String, String)> = Vec::new();
        if !ignore {
            if let Some(value) = &inm {
                entry.push(("If-None-Match".into(), value.clone()));
            }
            if let Some(value) = &ims {
                entry.push(("If-Modified-Since".into(), value.clone()));
            }
            if let Some(value) = &pragma {
                entry.push(("Pragma".into(), value.clone()));
            }
            if let Some(value) = &cache_control_header {
                entry.push(("Cache-Control".into(), value.clone()));
            }
        }
        recorded.push(entry);
        state.insert(stash_key, cache_state_encode(&recorded));

        let quoted_tag = tag.as_ref().map(|t| format!("\"{t}\""));
        let mut headers = vec![header("Access-Control-Allow-Origin", "*")];
        if let Some(quoted_tag) = &quoted_tag {
            headers.push(header("ETag", quoted_tag.clone()));
        } else if let Some(date) = &date {
            headers.push(header("Last-Modified", date.clone()));
        }
        if let Some(expires) = &expires {
            headers.push(header("Expires", expires.clone()));
        }
        if let Some(vary) = &vary {
            headers.push(header("Vary", vary.clone()));
        }
        if let Some(cc) = &cc {
            headers.push(header("Cache-Control", cc.clone()));
        }
        if let Some(location) = &redirect {
            headers.push(header("Location", location.clone()));
            return reply(302, "Redirect", headers, b"");
        }
        let matched =
            (inm.is_some() && inm == quoted_tag) || (ims.is_some() && ims == date);
        if matched {
            return reply(304, "Not Modified", headers, b"");
        }
        headers.push(header("Content-Type", "text/plain"));
        return reply(200, "OK", headers, content.as_bytes());
    }
    if fetch && name == "stash-take.py" {
        let body = match request.query("key").and_then(|key| state.remove(&key)) {
            Some(value) => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
            None => "null".into(),
        };
        return reply(200, "OK", vec![header("Content-Type", "application/json"),
            header("Access-Control-Allow-Origin", "*")], body.as_bytes());
    }
    if xhr && name == "inspect-headers.py" {
        let filter_value = request.query("filter_value").unwrap_or_default();
        let filter_name = request.query("filter_name").unwrap_or_default();
        let mut body = String::new();
        for (field, value) in &request.raw_headers {
            if !filter_value.is_empty() {
                if *value == filter_value {
                    body.push_str(field);
                    body.push(',');
                }
            } else if field.eq_ignore_ascii_case(&filter_name) {
                body.push_str(&format!("{field}: {value}\n"));
            }
        }
        return reply(200, "OK", headers, body.as_bytes());
    }
    if xhr && name == "echo-headers.py" {
        let body = request
            .raw_headers
            .iter()
            .map(|(n, v)| format!("{n}: {v}\n"))
            .collect::<String>();
        return reply(200, "OK", headers, body.as_bytes());
    }
    if xhr && name == "headers.py" {
        return reply(
            200,
            "OK",
            vec![
                header("Content-Type", "text/plain"),
                header("X-Custom-Header", "test"),
                header("Set-Cookie", "test"),
                header("Set-Cookie2", "test"),
                header("X-Custom-Header-Empty", ""),
                header("X-Custom-Header-Comma", "1"),
                header("X-Custom-Header-Comma", "2"),
                header("X-Custom-Header-Bytes", "…"),
            ],
            b"TEST",
        );
    }
    if xhr {
        let raw: Option<&[u8]> = match name {
            "headers.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/headers.asis")),
            "headers-basic.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/headers-basic.asis")),
            "headers-double-empty.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/headers-double-empty.asis")),
            "headers-some-are-empty.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/headers-some-are-empty.asis")),
            "headers-www-authenticate.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/headers-www-authenticate.asis")),
            "header-content-length.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/header-content-length.asis")),
            "header-content-length-twice.asis" => Some(include_bytes!("../../../amazing-fetch/tests/wpt/header_xhr/source/xhr/resources/header-content-length-twice.asis")),
            _ => None,
        };
        if let Some(raw) = raw {
            let mut wire = Vec::new();
            for &byte in raw {
                if byte == b'\n' {
                    wire.extend_from_slice(b"\r\n");
                } else {
                    wire.push(byte);
                }
            }
            wire.extend_from_slice(b"\r\n");
            return wire;
        }
    }
    match (fetch, xhr, name) {
        (true, _, "inspect-headers.py") => {
            let checked = request.query("headers").unwrap_or_default();
            for name in checked.split('|').filter(|n| !n.is_empty()) {
                if let Some(value) = request.header(name) {
                    headers.push(header(&format!("x-request-{name}"), value));
                }
            }
            if request.query("cors").is_some() {
                cors(request, &mut headers);
                headers.push(header("Access-Control-Allow-Methods", "GET, POST, HEAD"));
                headers.push(header(
                    "Access-Control-Expose-Headers",
                    checked
                        .split('|')
                        .map(|n| format!("x-request-{n}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
                headers.push(header(
                    "Access-Control-Allow-Headers",
                    request.query("allow_headers").unwrap_or_else(|| {
                        request
                            .headers
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    }),
                ));
            }
            reply(200, "OK", headers, b"")
        }
        (true, _, "preflight.py") => {
            let token = request.query("token").unwrap_or_default();
            headers.push(header(
                "Access-Control-Allow-Origin",
                request.query("origin").unwrap_or_else(|| "*".into()),
            ));
            if request.query("credentials").is_some() {
                headers.push(header("Access-Control-Allow-Credentials", "true"));
            }
            if request.method == "OPTIONS" {
                if request.header("access-control-request-method").is_none()
                    || request.header("accept") != Some("*/*")
                {
                    return reply(400, "Bad Request", headers, b"");
                }
                if let Some(value) = request.query("allow_headers") {
                    headers.push(header("Access-Control-Allow-Headers", value));
                }
                if let Some(value) = request.query("allow_methods") {
                    headers.push(header("Access-Control-Allow-Methods", value));
                }
                if let Some(value) = request.query("max_age") {
                    headers.push(header("Access-Control-Max-Age", value));
                }
                state.insert(
                    token,
                    request
                        .header("access-control-request-headers")
                        .unwrap_or("")
                        .into(),
                );
                reply(
                    request
                        .query("preflight_status")
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(200),
                    "OK",
                    headers,
                    b"",
                )
            } else {
                let preflight = state.get(&token);
                headers.push(header(
                    "Access-Control-Expose-Headers",
                    "x-did-preflight, x-control-request-headers, x-origin",
                ));
                headers.push(header(
                    "x-did-preflight",
                    if preflight.is_some() { "1" } else { "0" },
                ));
                if let Some(value) = preflight.filter(|value| !value.is_empty()) {
                    headers.push(header("x-control-request-headers", value));
                }
                headers.push(header("x-origin", request.header("origin").unwrap_or("")));
                reply(200, "OK", headers, b"")
            }
        }
        (true, _, "redirect.py") | (_, true, "redirect.py") => {
            if request.method == "OPTIONS" {
                cors(request, &mut headers);
                return reply(200, "OK", headers, b"");
            }
            if request.query("followed").is_some() {
                return reply(200, "OK", headers, b"MAGIC HAPPENED");
            }
            let status = request
                .query("redirect_status")
                .or_else(|| request.query("code"))
                .and_then(|n| n.parse().ok())
                .unwrap_or(302);
            if fetch {
                cors(request, &mut headers);
            }
            let mut location = request
                .query("location")
                .unwrap_or_else(|| "redirect.py?followed".into());
            if xhr && location.starts_with("redirect.py") {
                location.push_str(&format!("&code={status}"));
            }
            if fetch && request.query("simple").is_none() && !location.is_empty() {
                location.push(if location.contains('?') { '&' } else { '?' });
                location.push_str("count=1");
            }
            headers.push(header("Location", location));
            reply(status, "Found", headers, if fetch { b"" } else { b"TEST" })
        }
        (_, _, "status.py") if fetch || xhr => {
            let code = request
                .query("code")
                .and_then(|n| n.parse().ok())
                .unwrap_or(200);
            let text = request.query("text").unwrap_or_else(|| "OMG".into());
            headers.clear();
            headers.push(header(
                "Content-Type",
                request.query("type").unwrap_or_default(),
            ));
            headers.push(header("X-Request-Method", &request.method));
            reply(
                code,
                &text,
                headers,
                request.query("content").unwrap_or_default().as_bytes(),
            )
        }
        (true, _, "echo-content.py") => {
            headers.push(header("X-Request-Method", &request.method));
            headers.push(header(
                "X-Request-Content-Length",
                request.header("content-length").unwrap_or("NO"),
            ));
            headers.push(header(
                "X-Request-Content-Type",
                request.header("content-type").unwrap_or("NO"),
            ));
            reply(200, "OK", headers, &request.body)
        }
        (_, true, "content.py") => {
            headers.push(header("X-Request-Method", &request.method));
            headers.push(header(
                "X-Request-Query",
                request
                    .target
                    .split_once('?')
                    .map(|(_, q)| q)
                    .unwrap_or("NO"),
            ));
            headers.push(header(
                "X-Request-Content-Length",
                request.header("content-length").unwrap_or("NO"),
            ));
            headers.push(header(
                "X-Request-Content-Type",
                request.header("content-type").unwrap_or("NO"),
            ));
            let body = request
                .query("content")
                .map(String::into_bytes)
                .unwrap_or_else(|| request.body.clone());
            reply(200, "OK", headers, &body)
        }
        (_, true, "delay.py") => {
            thread::sleep(Duration::from_millis(
                request
                    .query("ms")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(50)
                    .min(200),
            ));
            headers.push(header("Access-Control-Allow-Origin", "*"));
            reply(200, "OK", headers, b"TEST_DELAY")
        }
        (_, true, "echo-headers.py") => reply(
            200,
            "OK",
            headers,
            format!("{:?}", request.headers).as_bytes(),
        ),
        (_, _, "top.txt") | (_, _, "cors-top.txt") => {
            if name == "cors-top.txt" {
                headers.push(header("Access-Control-Allow-Origin", "*"));
            }
            if let Some(pipe) = request.query("pipe") {
                for instruction in pipe.split('|') {
                    if let Some(inner) = instruction
                        .strip_prefix("header(")
                        .and_then(|s| s.strip_suffix(')'))
                    {
                        if let Some((name, value)) = inner.split_once(',') {
                            headers.push(header(name, value.trim_end_matches(",True")));
                        }
                    }
                }
            }
            reply(200, "OK", headers, b"top")
        }
        (_, true, "well-formed.xml") => reply(
            200,
            "OK",
            vec![header("Content-Type", "application/xml")],
            b"<root/>",
        ),
        _ => {
            // Selected tests also fetch pinned static resources (for example
            // fetch/api/cors/resources/not-cors-safelisted.json) that have no
            // handler. Serve those from the pinned WPT checkout when the runner
            // points at one, so a missing handler does not fail the file.
            if let Some(body) = static_file(path) {
                let content_type = match name.rsplit_once('.').map(|(_, ext)| ext) {
                    Some("json") => "application/json",
                    Some("js") => "text/javascript",
                    Some("css") => "text/css",
                    Some("xml") => "application/xml",
                    Some("headers" | "txt" | "asis") => "text/plain",
                    _ => "text/plain",
                };
                match headers
                    .iter_mut()
                    .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
                {
                    Some(existing) => existing.1 = content_type.to_string(),
                    None => headers.push(header("Content-Type", content_type)),
                }
                headers.push(header("Access-Control-Allow-Origin", "*"));
                return reply(200, "OK", headers, &body);
            }
            reply(404, "Not Found", headers, b"not found")
        }
    }
}
/// Read a pinned static WPT resource from `AMAZING_WPT_SHIM_ROOT`, if set.
fn static_file(path: &str) -> Option<Vec<u8>> {
    let root = std::path::PathBuf::from(std::env::var_os("AMAZING_WPT_SHIM_ROOT")?);
    let relative = path.trim_start_matches('/');
    if relative.is_empty()
        || relative.contains("..")
        || relative.contains('\\')
        || relative.contains('%')
    {
        return None;
    }
    let canonical = root.join(relative).canonicalize().ok()?;
    if !canonical.starts_with(&root)
        || canonical.extension().and_then(|ext| ext.to_str()) == Some("py")
    {
        return None;
    }
    let metadata = canonical.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 {
        return None;
    }
    std::fs::read(canonical).ok()
}

// --- WebSocket (RFC 6455) upgrade support -------------------------------
//
// Ports the subset of the pinned WPT `websockets/handlers/*_wsh.py`
// (pywebsocket3) handlers that `websockets/` tests exercise: `echo`,
// `echo_raw`, the close-behavior handlers (`echo_close_data`, `echo_exit`,
// `remote-close`, `passive-close-abort`, `delayed-passive-close`), and the
// handshake/header handlers (`handshake_no_extensions`,
// `handshake_no_protocol`, `handshake_protocol`, `handshake_sleep_2`,
// `protocol`, `protocol_array`, `origin`, `referrer`, `echo-cookie`,
// `set-cookie*`, `wrong_accept_key`, `simple_handshake`, `invalid`,
// `basic_auth`, `empty-message`, `echo-query*`). Any other path fails the
// opening handshake with a plain 404, matching pywebsocket3's behavior for
// an unmapped handler name.

fn is_websocket_upgrade(request: &SeenRequest) -> bool {
    request.method.eq_ignore_ascii_case("GET")
        && request
            .header("upgrade")
            .map(|value| value.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false)
        && request
            .header("connection")
            .map(|value| {
                value
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false)
        && request.header("sec-websocket-key").is_some()
}

fn handle_websocket(mut stream: TcpStream, request: SeenRequest) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let handler = request.path().trim_start_matches('/').to_string();
    let key = request.header("sec-websocket-key").unwrap_or("").to_string();
    let origin = request.header("origin").unwrap_or("").to_string();
    let query = request.target.split_once('?').map(|(_, q)| q).unwrap_or("");

    // Handlers that speak a raw, non-standard opening handshake themselves.
    match handler.as_str() {
        "wrong_accept_key" => {
            let msg = format!(
                "HTTP/1.1 101 Switching Protocols:\r\nConnection: Upgrade\r\nUpgrade: WebSocket\r\nSec-WebSocket-Origin: {origin}\r\nSec-WebSocket-Accept: thisisawrongacceptkey\r\n\r\n"
            );
            if stream.write_all(msg.as_bytes()).is_ok() {
                let _ = write_ws_frame(&mut stream, 0x1, b"test");
            }
            return;
        }
        "simple_handshake" => {
            let accept = compute_accept(&key);
            let head = format!(
                "HTTP/1.1 101 Switching Protocols:\r\nConnection: Upgrade\r\nUpgrade: WebSocket\r\nSet-Cookie: ws_test=test\r\nSec-WebSocket-Origin: {origin}\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            let mut out = head.into_bytes();
            let close_body = build_close_body(Some(1001), "PASS");
            let mut close_frame = vec![0x88u8, close_body.len() as u8];
            close_frame.extend_from_slice(&close_body);
            out.extend_from_slice(&close_frame);
            if stream.write_all(&out).is_ok() {
                // Read the client's masked close-frame reply (frame header +
                // 4-byte mask + same-length body) and drop the connection
                // without a further handshake, mirroring the Python
                // handler's manual `AbortedByUserException` path.
                let mut drain = vec![0u8; close_frame.len() + 4];
                let _ = stream.read_exact(&mut drain);
            }
            return;
        }
        "invalid" => {
            let _ = stream.write_all(b"FOO BAR BAZ\r\n\r\n");
            return;
        }
        "basic_auth" => {
            if request.header("authorization") != Some("Basic Zm9vOmJhcg==") {
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Basic realm=\"camelot\"\r\n\r\n",
                );
                return;
            }
        }
        _ => {}
    }

    let known = matches!(
        handler.as_str(),
        "echo"
            | "echo_raw"
            | "echo_close_data"
            | "echo_exit"
            | "remote-close"
            | "passive-close-abort"
            | "delayed-passive-close"
            | "handshake_no_extensions"
            | "handshake_no_protocol"
            | "handshake_protocol"
            | "handshake_sleep_2"
            | "protocol"
            | "protocol_array"
            | "origin"
            | "referrer"
            | "echo-cookie"
            | "echo-query"
            | "echo-query_v13"
            | "empty-message"
            | "set-cookie"
            | "set-cookie_http"
            | "set-cookie-secure"
            | "set-cookies-samesite"
            | "basic_auth"
            | "receive-many-with-backpressure"
    );
    if !known {
        // No handler for this path: pywebsocket3 fails the opening
        // handshake with a plain 404, same as a missing `*_wsh.py` file.
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }

    if handler == "handshake_sleep_2" {
        thread::sleep(Duration::from_secs(2));
    }

    let protocol = ws_protocol(&handler, &request);
    let mut extra_headers: Vec<(String, String)> = Vec::new();
    match handler.as_str() {
        "set-cookie" => extra_headers.push(header("Set-Cookie", format!("ws_test_{query}=test; Path=/"))),
        "set-cookie_http" => extra_headers.push(header(
            "Set-Cookie",
            format!("ws_test_{query}=test; Path=/; HttpOnly\r\nSec-WebSocket-Origin: {origin}"),
        )),
        "set-cookie-secure" => extra_headers.push(header(
            "Set-Cookie",
            format!("ws_test_{query}=test; Secure; Path=/"),
        )),
        "set-cookies-samesite" => {
            let max_age = if query.contains("clear") { "; Max-Age=0" } else { "" };
            let value = request.query("value").unwrap_or_else(|| "1".into());
            for (name, extra) in [
                ("samesite-unspecified", ""),
                ("samesite-lax", "; SameSite=Lax"),
                ("samesite-strict", "; SameSite=Strict"),
                ("samesite-none", "; SameSite=None; Secure"),
            ] {
                extra_headers.push(header(
                    "Set-Cookie",
                    format!("{name}={value}; Path=/{extra}{max_age}"),
                ));
            }
        }
        _ => {}
    }

    let accept = compute_accept(&key);
    let mut head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n"
    );
    if let Some(protocol) = &protocol {
        head.push_str(&format!("Sec-WebSocket-Protocol: {protocol}\r\n"));
    }
    for (name, value) in &extra_headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }

    match handler.as_str() {
        "echo" => ws_loop_echo(&mut stream),
        "echo_raw" => ws_loop_echo_raw(&mut stream),
        "echo_close_data" | "echo_exit" => ws_loop_close_data(&mut stream),
        "delayed-passive-close" => ws_loop_idle(&mut stream, true, false),
        "passive-close-abort" => ws_loop_idle(&mut stream, false, true),
        "remote-close" => ws_remote_close(&mut stream, &request),
        "receive-many-with-backpressure" => ws_loop_receive_many_with_backpressure(&mut stream),
        "protocol" | "protocol_array" => {
            let value = protocol.unwrap_or_default();
            let _ = write_ws_frame(&mut stream, 0x1, value.as_bytes());
            ws_close_active(&mut stream);
        }
        "origin" => {
            let _ = write_ws_frame(&mut stream, 0x1, origin.as_bytes());
            ws_close_active(&mut stream);
        }
        "referrer" => {
            let referrer = request.header("referer").unwrap_or("MISSING AS PER FETCH");
            let _ = write_ws_frame(&mut stream, 0x1, referrer.as_bytes());
            ws_close_active(&mut stream);
        }
        "echo-cookie" => {
            let cookie = request.header("cookie").unwrap_or("(none)");
            let _ = write_ws_frame(&mut stream, 0x1, cookie.as_bytes());
            ws_close_active(&mut stream);
        }
        "echo-query" | "echo-query_v13" => {
            let _ = write_ws_frame(&mut stream, 0x1, query.as_bytes());
            ws_close_active(&mut stream);
        }
        "empty-message" => match read_ws_message(&mut stream) {
            Some((0x8, payload)) => ws_passive_close_default(&mut stream, &payload),
            Some((_, payload)) => {
                let reply_text = if payload.is_empty() { "pass" } else { "fail" };
                let _ = write_ws_frame(&mut stream, 0x1, reply_text.as_bytes());
                ws_close_active(&mut stream);
            }
            None => {}
        },
        // handshake_no_extensions, handshake_no_protocol, handshake_protocol,
        // handshake_sleep_2, set-cookie*, and basic_auth (post-auth): accept
        // and idle, replying to pings and to the client's own close.
        _ => ws_loop_idle(&mut stream, false, false),
    }
}

fn ws_protocol(handler: &str, request: &SeenRequest) -> Option<String> {
    let raw = request.header("sec-websocket-protocol").map(str::to_owned);
    match handler {
        "handshake_no_protocol" => None,
        "handshake_protocol" => Some("foobar".to_owned()),
        "echo" => raw
            .filter(|value| value.split(',').map(str::trim).any(|part| part == "echo"))
            .map(|_| "echo".to_owned()),
        "protocol" => raw,
        "protocol_array" => raw.map(|value| value.split(',').next().unwrap_or("").to_owned()),
        _ => None,
    }
}

fn ws_loop_echo(stream: &mut TcpStream) {
    const GOODBYE: &[u8] = b"Goodbye";
    loop {
        match read_ws_message(stream) {
            None => return,
            Some((0x8, payload)) => {
                ws_passive_close_default(stream, &payload);
                return;
            }
            Some((0x9, payload)) => {
                let _ = write_ws_frame(stream, 0xA, &payload);
            }
            Some((0xA, _)) => {}
            Some((opcode, payload)) => {
                let is_goodbye = opcode == 0x1 && payload == GOODBYE;
                if write_ws_frame(stream, opcode, &payload).is_err() {
                    return;
                }
                if is_goodbye {
                    ws_close_active(stream);
                    return;
                }
            }
        }
    }
}

fn ws_loop_echo_raw(stream: &mut TcpStream) {
    loop {
        match read_ws_message(stream) {
            None => return,
            Some((0x8, payload)) => {
                ws_passive_close_default(stream, &payload);
                return;
            }
            Some((0x9, payload)) => {
                let _ = write_ws_frame(stream, 0xA, &payload);
            }
            Some((_, payload)) => {
                if payload == b"exit" {
                    ws_close_active(stream);
                    return;
                }
                if stream.write_all(&payload).is_err() {
                    return;
                }
            }
        }
    }
}

/// Ports `websockets/resources/receive-many-with-backpressure_wsh.py`: reads
/// up to 50 data messages, replying to each with the byte length of that
/// message as text after a short server-side delay (the pinned
/// `send-many-64K-messages-with-backpressure.any.js` comment: "with 0.1
/// second server-side delay per message, sending 50 messages will take
/// around 5 seconds" -- the delay is what applies backpressure to the
/// client's `send()` calls, which is the behavior under test).
fn ws_loop_receive_many_with_backpressure(stream: &mut TcpStream) {
    const MAX_MESSAGES: usize = 50;
    let mut received = 0;
    loop {
        match read_ws_message(stream) {
            None => return,
            Some((0x8, payload)) => {
                ws_passive_close_default(stream, &payload);
                return;
            }
            Some((0x9, payload)) => {
                let _ = write_ws_frame(stream, 0xA, &payload);
            }
            Some((0xA, _)) => {}
            Some((_, payload)) => {
                thread::sleep(Duration::from_millis(100));
                let reply = payload.len().to_string();
                if write_ws_frame(stream, 0x1, reply.as_bytes()).is_err() {
                    return;
                }
                received += 1;
                if received >= MAX_MESSAGES {
                    // The client closes once it has every reply; wait for
                    // that close rather than racing it with our own.
                    loop {
                        match read_ws_message(stream) {
                            None => return,
                            Some((0x8, payload)) => {
                                ws_passive_close_default(stream, &payload);
                                return;
                            }
                            Some((0x9, payload)) => {
                                let _ = write_ws_frame(stream, 0xA, &payload);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

fn ws_loop_close_data(stream: &mut TcpStream) {
    loop {
        match read_ws_message(stream) {
            None => return,
            Some((0x8, payload)) => {
                ws_passive_close_default(stream, &payload);
                return;
            }
            Some((0x9, payload)) => {
                let _ = write_ws_frame(stream, 0xA, &payload);
            }
            Some((0x1, payload)) if payload == b"Goodbye" => {
                ws_close_active(stream);
                return;
            }
            _ => {}
        }
    }
}

fn ws_loop_idle(stream: &mut TcpStream, delay_before_close: bool, abort_on_close: bool) {
    loop {
        match read_ws_message(stream) {
            None => return,
            Some((0x8, payload)) => {
                if abort_on_close {
                    // Drop the connection without a closing handshake.
                    return;
                }
                if delay_before_close {
                    thread::sleep(Duration::from_secs(1));
                }
                ws_passive_close_default(stream, &payload);
                return;
            }
            Some((0x9, payload)) => {
                let _ = write_ws_frame(stream, 0xA, &payload);
            }
            _ => {}
        }
    }
}

fn ws_remote_close(stream: &mut TcpStream, request: &SeenRequest) {
    let query = request.target.split_once('?').map(|(_, q)| q).unwrap_or("");
    if query
        .split('&')
        .any(|part| part == "abrupt=1" || part == "abrupt")
    {
        let _ = write_ws_frame(stream, 0x9, b"ping");
        return; // Abrupt: no close frame, just drop the connection.
    }
    match request.query("code") {
        Some(code) => {
            let code: u16 = code.parse().unwrap_or(1000);
            let reason = request.query("reason").unwrap_or_default();
            let body = build_close_body(Some(code), &reason);
            let _ = write_ws_frame(stream, 0x8, &body);
        }
        None => {
            let _ = write_ws_frame(stream, 0x8, &[]);
        }
    }
    drain_briefly(stream);
}

fn ws_close_active(stream: &mut TcpStream) {
    let body = build_close_body(Some(1000), "");
    let _ = write_ws_frame(stream, 0x8, &body);
    drain_briefly(stream);
}

fn ws_passive_close_default(stream: &mut TcpStream, client_payload: &[u8]) {
    let _ = write_ws_frame(stream, 0x8, client_payload);
}

fn drain_briefly(stream: &mut TcpStream) {
    let mut discard = [0u8; 256];
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.read(&mut discard);
}

fn build_close_body(code: Option<u16>, reason: &str) -> Vec<u8> {
    match code {
        Some(code) => {
            let mut body = code.to_be_bytes().to_vec();
            body.extend_from_slice(reason.as_bytes());
            body
        }
        None => Vec::new(),
    }
}

/// Read one complete WebSocket message (merging any continuation frames),
/// returning `(opcode, payload)`. Control frames (`0x8` close, `0x9` ping,
/// `0xA` pong) are never fragmented and are returned as their own message.
fn read_ws_message(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut message_opcode = None;
    let mut buffer = Vec::new();
    loop {
        let (opcode, fin, payload) = read_ws_frame(stream)?;
        if opcode >= 0x8 {
            return Some((opcode, payload));
        }
        if opcode != 0x0 {
            message_opcode = Some(opcode);
        }
        buffer.extend_from_slice(&payload);
        if fin {
            break;
        }
    }
    Some((message_opcode.unwrap_or(0x1), buffer))
}

fn read_ws_frame(stream: &mut TcpStream) -> Option<(u8, bool, Vec<u8>)> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).ok()?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let mut len = (head[1] & 0x7F) as u64;
    if len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext).ok()?;
        len = u16::from_be_bytes(ext) as u64;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext).ok()?;
        len = u64::from_be_bytes(ext);
    }
    if len > 16 * 1024 * 1024 {
        return None;
    }
    let mask = if masked {
        let mut mask = [0u8; 4];
        stream.read_exact(&mut mask).ok()?;
        Some(mask)
    } else {
        None
    };
    let mut payload = vec![0u8; len as usize];
    if len > 0 {
        stream.read_exact(&mut payload).ok()?;
    }
    if let Some(mask) = mask {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }
    Some((opcode, fin, payload))
}

fn write_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        out.push(len as u8);
    } else if len <= 0xFFFF {
        out.push(126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    stream.write_all(&out)
}

fn compute_accept(key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let digest = sha1(format!("{key}{GUID}").as_bytes());
    base64_encode(&digest)
}

fn sha1(input: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (input.len() as u64) * 8;
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
