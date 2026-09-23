//! Driver reference: one service handles a document, its policy, cookies,
//! subresources, a redirect, cache validation, and a WebSocket in one session.
use amazing_fetch::{
    api::{ApiRequest, ApiRequestInit, ApiRequestInput},
    service::{ClientContext, NetworkService, ServiceEvent, ServiceId, WebSocketClient},
    CspDestination, CspPolicyList, HeaderList, HttpUrl,
};
use amazing_net::{dispatch::DispatchLimits, dispatch::WebSocketEvent};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn request(base: &HttpUrl, path: &str) -> ApiRequest {
    let input: Vec<u16> = path.encode_utf16().collect();
    ApiRequest::new(
        ApiRequestInput::Url(&input),
        ApiRequestInit::default(),
        base,
        base.origin().clone(),
    )
    .unwrap()
}

fn finish(service: &mut NetworkService, id: ServiceId) -> amazing_fetch::api::ApiResponse {
    let start = Instant::now();
    for tick in 0..4000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            if let ServiceEvent::Finished { id: found, result } = event {
                assert_eq!(found, id);
                return result.unwrap();
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("NetworkService request {id:?} did not finish");
}

fn fetch(
    service: &mut NetworkService,
    base: &HttpUrl,
    path: &str,
    context: ClientContext,
) -> amazing_fetch::api::ApiResponse {
    let id = service.start(request(base, path), context).unwrap();
    finish(service, id)
}

// RFC 6455 challenge response for this loopback fixture only.
fn websocket_accept(key: &str) -> String {
    assert_eq!(key.len(), 24);
    let mut input = [0u8; 128];
    input[..24].copy_from_slice(key.as_bytes());
    input[24..60].copy_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    input[60] = 128;
    input[120..].copy_from_slice(&480u64.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for chunk in input.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, bytes) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(bytes.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (part, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *part = part.wrapping_add(value);
        }
    }
    let mut bytes = [0; 20];
    for (out, word) in bytes.chunks_exact_mut(4).zip(h) {
        out.copy_from_slice(&word.to_be_bytes());
    }
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(DIGITS[(value >> 18) as usize] as char);
        result.push(DIGITS[((value >> 12) & 63) as usize] as char);
        result.push(if chunk.len() > 1 {
            DIGITS[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            DIGITS[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

#[test]
fn page_sequence_uses_one_network_service() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = HttpUrl::parse(&format!("http://127.0.0.1:{port}/page")).unwrap();
    let server = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = Vec::new();
        for _ in 0..7 {
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "only {} wire requests",
                            seen.len()
                        );
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            let response = match path.as_str() {
                "/page" => "HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc; Path=/\r\nContent-Security-Policy: default-src 'self'; img-src 'none'\r\nContent-Length: 4\r\nConnection: close\r\n\r\npage".to_owned(),
                "/script.js" => "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\njs".to_owned(),
                "/redirect" => "HTTP/1.1 302 Found\r\nLocation: /landing\r\nSet-Cookie: hop=ok; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                "/landing" => "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nland".to_owned(),
                "/cached" if request.to_ascii_lowercase().contains("if-none-match: \"v1\"") =>
                    "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n".to_owned(),
                "/cached" => "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nCache-Control: max-age=0\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody".to_owned(),
                "/socket" => {
                    let key = request.lines().find_map(|line| {
                        line.to_ascii_lowercase().starts_with("sec-websocket-key: ")
                            .then(|| line[19..].to_owned())
                    }).unwrap();
                    format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", websocket_accept(key.trim()))
                }
                _ => panic!("unexpected request: {request}"),
            };
            socket.write_all(response.as_bytes()).unwrap();
            seen.push(request);
            if path == "/socket" {
                thread::sleep(Duration::from_millis(100));
            }
        }
        seen
    });

    let mut service = NetworkService::new(DispatchLimits::default());
    let page = fetch(
        &mut service,
        &base,
        "/page",
        ClientContext {
            now_unix: 100,
            ..Default::default()
        },
    );
    assert_eq!(page.status, 200);
    let csp = page
        .headers
        .get(&"content-security-policy".encode_utf16().collect::<Vec<_>>())
        .unwrap()
        .unwrap();
    let mut policy_headers = HeaderList::default();
    policy_headers
        .append("content-security-policy", &csp)
        .unwrap();
    let policy = CspPolicyList::from_headers(&policy_headers).unwrap();
    let context = |destination, wall| ClientContext {
        now_unix: wall,
        csp: policy.clone(),
        csp_destination: destination,
        ..Default::default()
    };
    assert_eq!(
        fetch(
            &mut service,
            &base,
            "/script.js",
            context(CspDestination::Script, 100)
        )
        .status,
        200
    );
    let blocked = service.start(
        request(&base, "/blocked.png"),
        context(CspDestination::Image, 100),
    );
    assert!(blocked.is_err(), "CSP must block the image before dispatch");
    assert_eq!(
        fetch(
            &mut service,
            &base,
            "/redirect",
            context(CspDestination::None, 100)
        )
        .status,
        200
    );
    let mut first = fetch(
        &mut service,
        &base,
        "/cached",
        context(CspDestination::None, 100),
    );
    assert_eq!(first.body.bytes().unwrap(), b"body");
    let mut second = fetch(
        &mut service,
        &base,
        "/cached",
        context(CspDestination::None, 101),
    );
    assert_eq!(second.body.bytes().unwrap(), b"body");
    let ws = service
        .open_websocket(
            &format!("ws://127.0.0.1:{port}/socket"),
            vec![],
            WebSocketClient {
                origin: base.origin().clone(),
                context: context(CspDestination::Connect, 101),
            },
        )
        .unwrap();
    let start = Instant::now();
    let mut opened = false;
    for tick in 0..4000 {
        if service.step(8192, start + Duration::from_millis(tick)).into_iter().any(|event| matches!(event, ServiceEvent::WebSocket { id, event: WebSocketEvent::Open { .. } } if id == ws)) {
            opened = true;
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(opened, "WebSocket did not open");
    let seen = server.join().unwrap();
    let paths: Vec<_> = seen
        .iter()
        .map(|request| request.split_whitespace().nth(1).unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "/page",
            "/script.js",
            "/redirect",
            "/landing",
            "/cached",
            "/cached",
            "/socket"
        ]
    );
    for request in seen.iter().skip(1) {
        assert!(
            request.to_ascii_lowercase().contains("cookie: sid=abc"),
            "{request}"
        );
    }
    for request in seen.iter().skip(3) {
        assert!(request.to_ascii_lowercase().contains("hop=ok"), "{request}");
    }
    assert!(seen[5]
        .to_ascii_lowercase()
        .contains("if-none-match: \"v1\""));
}
