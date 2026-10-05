use amazing_process::network_v2::NetworkSession;
use amazing_process::protocol::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

fn server(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut requests = Vec::new();
        for response in responses {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing native network request");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            requests.push(read_request(&mut stream));
            stream.write_all(&response).unwrap();
        }
        requests
    });
    (origin, thread)
}
fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 64 * 1024);
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}
fn response(status: &str, fields: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status}\r\n{fields}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
fn session(id: u64) -> NetworkSession {
    // Each native test owns its child cache; persistent pool state cannot
    // turn an expected authentic request into a hit from an earlier run.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::set_var(
        "AMAZING_HTTP_CACHE_DIR",
        std::env::temp_dir().join(format!(
            "amazing-navigation-metadata-{}-{id}-{stamp}",
            std::process::id()
        )),
    );
    NetworkSession::launch(
        std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child")),
        id,
        Duration::from_secs(5),
    )
    .unwrap()
}
fn navigation(url: &str, origin: &str) -> NetworkStart {
    let mut request = request(url, origin);
    request.mode = "navigate".into();
    request.credentials = "include".into();
    request.redirect = "manual".into();
    request.cache = "default".into();
    request.referrer = format!("{origin}/creator?q=1#private");
    request
}
fn policy_values(head: &NavigationResponseHead) -> Vec<(&str, &str)> {
    head.headers
        .iter()
        .filter(|h| h.name.starts_with("content-security-policy"))
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect()
}
fn request(url: &str, origin: &str) -> NetworkStart {
    NetworkStart {
        version: NETWORK_VERSION,
        client_id: 7,
        epoch: 1,
        url: url.into(),
        base_url: format!("{origin}/"),
        method: "GET".into(),
        headers: vec![HeaderField {
            name: "Accept".into(),
            value: "text/html".into(),
        }],
        body_present: false,
        body: ByteBuf(Vec::new()),
        destination: "document".into(),
        mode: "cors".into(),
        credentials: "same-origin".into(),
        redirect: "follow".into(),
        cache: "default".into(),
        referrer: "about:client".into(),
        referrer_policy: "strict-origin-when-cross-origin".into(),
        response_limit: 1024 * 1024,
        navigation_reload_or_history: false,
        navigation_client_reserved_id: None,
        context: NetworkContext {
            origin: WireOrigin::Tuple(origin.into()),
            cookie_site: WireCookieSite::NoClient(()),
            top_level_origin: None,
            ancestor_origins: Vec::new(),
            sandbox_flags: 0,
            now_unix: 1_780_000_000,
            restricts_mixed_content: false,
            image_set_initiator: false,
            top_level_navigation: true,
            embedder_policy: "unsafe-none".into(),
            no_cors_media_state: "none".into(),
            csp_headers: Vec::new(),
        },
    }
}

#[test]
fn native_navigation_snapshot_preserves_duplicate_fields_and_actual_referrer() {
    let fields = "Content-Type: text/html\r\nContent-Security-Policy: script-src 'none'\r\nContent-Security-Policy: default-src 'self'\r\nContent-Security-Policy-Report-Only: script-src 'none'\r\nSet-Cookie: private=1; HttpOnly\r\n";
    let (origin, thread) = server(
        (0..4)
            .map(|i| {
                response(
                    if i == 0 {
                        "418 Native Status"
                    } else {
                        "200 OK"
                    },
                    fields,
                    b"\x00<body>actual bytes</body>\xff",
                )
            })
            .collect(),
    );
    let session = session(81);
    for (i, policy) in ["strict-origin-when-cross-origin", "origin", "no-referrer"]
        .into_iter()
        .enumerate()
    {
        let mut request = navigation(&format!("{origin}/native{i}"), &origin);
        request.referrer_policy = policy.into();
        // A browser request's policy is not the received document's policy.
        request.context.csp_headers = vec![HeaderField {
            name: "content-security-policy".into(),
            value: "script-src 'unsafe-inline'".into(),
        }];
        let result = session.fetch(request).unwrap();
        let head = result
            .head
            .navigation
            .as_ref()
            .expect("accepted native navigation head");
        assert_eq!(head.status, if i == 0 { 418 } else { 200 });
        assert_eq!(head.reason, if i == 0 { "Native Status" } else { "OK" });
        assert_eq!(head.url, format!("{origin}/native{i}"));
        assert_eq!(
            head.referrer,
            match policy {
                "origin" => format!("{origin}/"),
                "no-referrer" => String::new(),
                _ => format!("{origin}/creator?q=1"),
            }
        );
        assert_eq!(
            policy_values(head),
            vec![
                ("content-security-policy", "script-src 'none'"),
                ("content-security-policy", "default-src 'self'"),
                ("content-security-policy-report-only", "script-src 'none'")
            ]
        );
        assert!(head
            .headers
            .iter()
            .any(|h| h.name == "set-cookie" && h.value == "private=1; HttpOnly"));
        assert!(!result.head.headers.iter().any(|h| h.name == "set-cookie"));
        assert_eq!((result.head.client_id, result.head.epoch), (81, 1));
        assert_ne!(result.head.id, 0);
        assert_eq!(result.body, b"\x00<body>actual bytes</body>\xff");
    }
    let ordinary = session.fetch(request("/ordinary", &origin)).unwrap();
    assert!(ordinary.head.navigation.is_none());
    assert!(!ordinary.head.headers.iter().any(|h| h.name == "set-cookie"));
    let requests = thread.join().unwrap();
    assert!(requests[0]
        .to_ascii_lowercase()
        .contains(&format!("referer: {origin}/creator?q=1\r\n")));
    assert!(requests[1]
        .to_ascii_lowercase()
        .contains(&format!("referer: {origin}/\r\n")));
    assert!(!requests[2].to_ascii_lowercase().contains("referer:"));
    assert!(!requests[3].to_ascii_lowercase().contains("referer:"));
}

#[test]
fn cached_and_revalidated_navigation_keep_accepted_head_and_current_request() {
    let first = response("200 Cached", "Content-Type: text/html\r\nCache-Control: max-age=3600\r\nETag: \"native\"\r\nContent-Security-Policy: script-src 'none'\r\nContent-Security-Policy: default-src 'self'\r\n", b"cached native body");
    // RFC 9110 §§8.6, 15.4.5: return the selected representation's ETag;
    // a 304 has no body, and Content-Length: 0 would misdescribe the cached
    // 17-byte representation. Omit Content-Length instead of using response().
    let merged = b"HTTP/1.1 304 Not Modified\r\nETag: \"native\"\r\nCache-Control: max-age=3600\r\nContent-Security-Policy: script-src 'self'\r\nContent-Security-Policy-Report-Only: default-src 'none'\r\nConnection: close\r\n\r\n".to_vec();
    let (origin, thread) = server(vec![first, merged]);
    let session = session(82);
    let mut request = navigation(&format!("{origin}/cache"), &origin);
    request.cache = "default".into();
    let first = session.fetch(request.clone()).unwrap();
    request.referrer = format!("{origin}/cache-creator");
    let cached = session.fetch(request.clone()).unwrap();
    assert_eq!(cached.body, first.body);
    let cached_metadata = cached.head.navigation.as_ref().unwrap();
    assert_eq!(cached_metadata.status, 200);
    assert_eq!(cached_metadata.referrer, format!("{origin}/cache-creator"));
    assert_eq!(
        policy_values(cached_metadata),
        policy_values(first.head.navigation.as_ref().unwrap())
    );
    assert_ne!(cached.head.id, first.head.id);
    request.headers.push(HeaderField {
        name: "Cache-Control".into(),
        value: "no-cache".into(),
    });
    request.referrer_policy = "origin".into();
    let revalidated = session.fetch(request).unwrap();
    let head = revalidated.head.navigation.as_ref().unwrap();
    assert_eq!(head.status, 200, "accepted merged head is not the wire 304");
    assert_eq!(head.url, format!("{origin}/cache"));
    assert_eq!(head.referrer, format!("{origin}/"));
    assert_eq!(
        policy_values(head),
        vec![
            ("content-security-policy", "script-src 'self'"),
            ("content-security-policy-report-only", "default-src 'none'")
        ]
    );
    assert_eq!(revalidated.body, first.body);
    let requests = thread.join().unwrap();
    assert_eq!(requests.len(), 2, "fresh cache hit causes no extra request");
    assert!(requests[1]
        .to_ascii_lowercase()
        .contains("if-none-match: \"native\""));
    assert!(requests[1]
        .to_ascii_lowercase()
        .contains(&format!("referer: {origin}/\r\n")));
}

#[test]
fn restarted_navigation_uses_final_response_request_identity_and_policy() {
    let (origin, thread) = server(vec![
        response("302 Found", "Location: /final\r\nContent-Security-Policy: script-src 'none'\r\n", b"redirect body"),
        response("201 Final", "Content-Type: text/html\r\nContent-Security-Policy: script-src 'self'\r\nContent-Security-Policy: img-src 'none'\r\n", b"final body"),
    ]);
    let session = session(83);
    let mut request = navigation(&format!("{origin}/start"), &origin);
    let started = session.start(request.clone()).unwrap();
    let first = collect(&session, &started);
    assert_eq!(
        first.0.navigation.as_ref().unwrap().url,
        format!("{origin}/start")
    );
    assert_eq!(first.1, b"redirect body");
    let mut forged = first.0.clone();
    forged.id += 1;
    assert!(session
        .restart_navigation(&started, &forged, request.clone())
        .is_err());
    request.url = format!("{origin}/final");
    request.referrer_policy = "origin".into();
    let final_started = session
        .restart_navigation(&started, &first.0, request)
        .unwrap();
    assert_eq!(final_started.epoch, started.epoch);
    assert_ne!(final_started.id, started.id);
    let final_response = collect(&session, &final_started);
    let head = final_response.0.navigation.as_ref().unwrap();
    assert_eq!(head.status, 201);
    assert_eq!(head.reason, "Final");
    assert_eq!(head.url, format!("{origin}/final"));
    assert_eq!(head.referrer, format!("{origin}/"));
    assert_eq!(
        policy_values(head),
        vec![
            ("content-security-policy", "script-src 'self'"),
            ("content-security-policy", "img-src 'none'")
        ]
    );
    assert_eq!(final_response.1, b"final body");
    let mut stale = final_started.clone();
    stale.client_id += 1;
    assert!(session.read(&stale, NETWORK_CHUNK).is_err());
    assert_eq!(thread.join().unwrap().len(), 2);
}
fn collect(session: &NetworkSession, started: &NetworkStarted) -> (NetworkHead, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut head = None;
    let mut bytes = Vec::new();
    loop {
        assert!(Instant::now() < deadline);
        match session.read(started, NETWORK_CHUNK).unwrap() {
            NetworkReply::Pending(()) => std::thread::yield_now(),
            NetworkReply::Head(value) => {
                assert!(head.is_none());
                head = Some(value);
            }
            NetworkReply::Body(value) => {
                assert!(head.is_some());
                assert_eq!((value.id, value.epoch), (started.id, started.epoch));
                bytes.extend(value.bytes.0);
            }
            NetworkReply::Complete(value) => {
                assert!(value.error.is_none(), "{value:?}");
                assert_eq!((value.id, value.epoch), (started.id, started.epoch));
                return (head.unwrap(), bytes);
            }
        }
    }
}

#[test]
fn rejected_child_navigation_never_returns_a_policy_snapshot() {
    let (origin, thread) = server(vec![response(
        "200 OK",
        "Content-Type: text/html\r\nContent-Security-Policy: frame-ancestors 'none'\r\n",
        b"denied body",
    )]);
    let session = session(84);
    let mut request = navigation(&format!("{origin}/denied"), &origin);
    request.destination = "frame".into();
    request.context.top_level_navigation = false;
    request.context.top_level_origin = Some(WireOrigin::Tuple(origin.clone()));
    request.context.ancestor_origins = vec![WireOrigin::Tuple(origin)];
    assert!(
        session.fetch(request).is_err(),
        "denied response cannot admit document policy"
    );
    assert_eq!(thread.join().unwrap().len(), 1);
}
