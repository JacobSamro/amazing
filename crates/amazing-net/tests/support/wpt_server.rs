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
                            if matches!(
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
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
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
                headers.push(header(
                    "x-control-request-headers",
                    preflight.cloned().unwrap_or_default(),
                ));
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
        _ => reply(404, "Not Found", headers, b"not found"),
    }
}
