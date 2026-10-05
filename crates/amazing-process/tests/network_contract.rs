use amazing_ipc::codec::{self, ByteBuf};
use amazing_process::network_v2::{origin_to_wire, NetworkEndpointV2, NetworkSession};
use amazing_process::protocol::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn read_request(stream: &mut impl Read) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        assert!(header.len() < 16 * 1024, "request header too large");
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        header.push(byte[0]);
    }
}

fn next_reply(session: &NetworkSession, started: &NetworkStarted, credit: u32) -> NetworkReply {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match session.read(started, credit).unwrap() {
            NetworkReply::Pending(()) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "network event timed out"
                );
                std::thread::yield_now();
            }
            event => return event,
        }
    }
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
fn versioned_contract_roundtrip_and_malformed_frames() {
    let start = request("/index", "http://example.test");
    let head = NetworkHead {
        id: 1,
        client_id: 7,
        epoch: 1,
        status: 200,
        url: "http://example.test/index".into(),
        reason: "OK".into(),
        headers: vec![HeaderField {
            name: "content-type".into(),
            value: "text/html; charset=utf-8".into(),
        }],
        response_type: "Basic".into(),
        redirected: false,
        url_list: vec!["http://example.test/index".into()],
        redirect_tainted: true,
    };
    let packets = vec![
        codec::encode(&start).unwrap(),
        codec::encode(&NetworkStarted {
            id: 1,
            client_id: 7,
            epoch: 1,
        })
        .unwrap(),
        codec::encode(&NetworkRead {
            id: 1,
            client_id: 7,
            epoch: 1,
            credit: NETWORK_CHUNK,
        })
        .unwrap(),
        codec::encode(&NetworkCancel {
            id: 1,
            client_id: 7,
            epoch: 1,
        })
        .unwrap(),
        codec::encode(&NetworkEpoch {
            client_id: 7,
            epoch: 1,
        })
        .unwrap(),
        codec::encode(&NetworkReply::Head(head)).unwrap(),
        codec::encode(&NetworkReply::Body(NetworkBody {
            id: 1,
            epoch: 1,
            bytes: ByteBuf(b"a".to_vec()),
        }))
        .unwrap(),
        codec::encode(&NetworkReply::Complete(NetworkComplete {
            id: 1,
            epoch: 1,
            error: None,
        }))
        .unwrap(),
    ];
    for (index, packet) in packets.iter().enumerate() {
        let decode = |bytes: &[u8]| -> Result<(), codec::Error> {
            match index {
                0 => {
                    let _ = codec::decode::<NetworkStart>(bytes)?;
                }
                1 => {
                    let _ = codec::decode::<NetworkStarted>(bytes)?;
                }
                2 => {
                    let _ = codec::decode::<NetworkRead>(bytes)?;
                }
                3 => {
                    let _ = codec::decode::<NetworkCancel>(bytes)?;
                }
                4 => {
                    let _ = codec::decode::<NetworkEpoch>(bytes)?;
                }
                _ => {
                    let _ = codec::decode::<NetworkReply>(bytes)?;
                }
            }
            Ok(())
        };
        assert!(decode(packet).is_ok());
        for end in 0..packet.len() {
            assert!(decode(&packet[..end]).is_err());
        }
        let mut seed = index as u64 + 0x19ad_2701;
        for _ in 0..1000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut random = vec![0u8; (seed as usize) % 257];
            for byte in &mut random {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            let _ = decode(&random);
        }
        assert!(decode(&vec![0; codec::MAX_FRAME + 1]).is_err());
    }
    assert_eq!(codec::decode::<NetworkStart>(&packets[0]).unwrap(), start);
}

#[test]
fn opaque_origin_identity_survives_the_wire() {
    use amazing_process::protocol::NetworkRequestsV2;
    let opaque = amazing_fetch::Origin::opaque_with_id((1_u128 << 127) | 37);
    assert_eq!(
        origin_to_wire(&opaque),
        WireOrigin::Opaque((1_u128 << 127) | 37)
    );
    let endpoint = NetworkEndpointV2::new();
    let mut start = request("/opaque", "http://example.test");
    start.context.origin = origin_to_wire(&opaque);
    start.context.cookie_site = WireCookieSite::Origin(origin_to_wire(&opaque));
    let started = endpoint.start(start).unwrap();
    endpoint
        .cancel(NetworkCancel {
            id: started.id,
            client_id: started.client_id,
            epoch: started.epoch,
        })
        .unwrap();
}

#[test]
fn network_session_preserves_non_success_head_and_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (consumed_tx, consumed_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        let body = b"<p>network contract</p>";
        write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Type: text/html; charset=utf-8\r\nX-Test: two\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(body).unwrap();
        consumed_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 7, Duration::from_secs(5)).unwrap();
    let response = session.fetch(request("/index", &origin)).unwrap();
    consumed_tx.send(()).unwrap();
    server.join().unwrap();
    assert_eq!(response.head.status, 404);
    assert_eq!(response.head.reason, "Not Found");
    assert_eq!(response.head.url, format!("{origin}/index"));
    assert!(response
        .head
        .headers
        .iter()
        .any(|h| h.name == "content-type" && h.value == "text/html; charset=utf-8"));
    assert_eq!(response.body, b"<p>network contract</p>");
}

#[test]
fn document_cookie_roundtrips_in_network_child_and_hides_httponly() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 71, Duration::from_secs(5)).unwrap();
    let host = format!("cookie-ipc-{}.test", std::process::id());
    let mut request = DocumentCookie {
        client_id: 0,
        epoch: 0,
        document_url: format!("https://{host}/path"),
        top_level_origin: WireOrigin::Tuple(format!("https://{host}")),
        now_unix: 1_780_000_000,
        assignment: Some("visible=yes; Secure; SameSite=None".into()),
    };
    assert_eq!(session.document_cookie(request.clone()).unwrap(), "");
    request.assignment = Some("hidden=no; HttpOnly".into());
    assert_eq!(session.document_cookie(request.clone()).unwrap(), "");
    request.assignment = None;
    assert_eq!(session.document_cookie(request.clone()).unwrap(), "visible=yes");
    request.assignment = Some("partitioned=one; Secure; SameSite=None; Partitioned".into());
    session.document_cookie(request.clone()).unwrap();
    request.assignment = None;
    assert_eq!(session.document_cookie(request.clone()).unwrap(), "visible=yes; partitioned=one");
    request.top_level_origin = WireOrigin::Tuple("https://other.test".into());
    assert_eq!(session.document_cookie(request).unwrap(), "visible=yes");
}

#[test]
fn v2_matches_direct_http_bytes_status_and_content_type() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=windows-1252\r\nContent-Length: 4\r\nConnection: close\r\n\r\nA\x80BC").unwrap();
        }
    });
    let url = format!("{origin}/parity");
    let direct = amazing_net::fetch(&url, &amazing_net::FetchOptions::default()).unwrap();
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 11, Duration::from_secs(5)).unwrap();
    let remote = session.fetch(request(&url, &origin)).unwrap();
    server.join().unwrap();
    assert_eq!(remote.head.status, direct.status);
    assert_eq!(remote.head.url, direct.url);
    assert_eq!(remote.body, direct.body);
    assert_eq!(
        remote
            .head
            .headers
            .iter()
            .find(|h| h.name == "content-type")
            .map(|h| h.value.as_str()),
        direct.headers.get("content-type").as_deref()
    );
}

#[test]
fn redirected_head_reports_final_url_and_redirect_flag() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone".as_slice(),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(response).unwrap();
        }
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 13, Duration::from_secs(5)).unwrap();
    let response = session.fetch(request("/start", &origin)).unwrap();
    server.join().unwrap();
    assert_eq!(response.head.status, 200);
    assert_eq!(response.head.url, format!("{origin}/final"));
    assert!(response.head.redirected);
    assert_eq!(response.body, b"done");
}

#[test]
fn browser_navigation_uses_explicit_initiator_context() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnav").unwrap();
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 12, Duration::from_secs(5)).unwrap();
    let mut nav = request(&format!("{origin}/nav"), &origin);
    nav.mode = "navigate".into();
    nav.credentials = "include".into();
    nav.redirect = "manual".into();
    nav.navigation_client_reserved_id = Some(42);
    let response = session.fetch(nav).unwrap();
    server.join().unwrap();
    assert_eq!(response.body, b"nav");
    assert_eq!(response.head.status, 200);
}

#[test]
fn child_navigation_checks_embedding_and_keeps_manual_redirect_epoch() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for response in [
            b"HTTP/1.1 200 OK\r\nX-Frame-Options: DENY\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nX-Frame-Options: SAMEORIGIN\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".as_slice(),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(response).unwrap();
        }
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 72, Duration::from_secs(5)).unwrap();
    let mut nav = request(&format!("{origin}/blocked"), &origin);
    nav.destination = "frame".into();
    nav.mode = "navigate".into();
    nav.credentials = "include".into();
    nav.redirect = "manual".into();
    nav.context.top_level_navigation = false;
    nav.context.top_level_origin = Some(WireOrigin::Tuple(origin.clone()));
    nav.context.ancestor_origins = vec![WireOrigin::Tuple(origin.clone())];
    assert!(session.fetch(nav.clone()).is_err());
    nav.url = format!("{origin}/start");
    let first = session.fetch(nav.clone()).unwrap();
    assert_eq!(first.head.status, 302);
    let started = NetworkStarted { id: first.head.id, client_id: first.head.client_id, epoch: first.head.epoch };
    nav.url = format!("{origin}/final");
    let second = session.restart_navigation(&started, &first.head, nav).unwrap();
    assert_eq!(second.epoch, started.epoch);
    let mut final_head = None;
    loop {
        match next_reply(&session, &second, NETWORK_CHUNK) {
            NetworkReply::Head(head) => final_head = Some(head),
            NetworkReply::Complete(done) => { assert!(done.error.is_none()); break; }
            NetworkReply::Body(_) => {},
            NetworkReply::Pending(()) => unreachable!(),
        }
    }
    assert_eq!(final_head.unwrap().status, 200);
    server.join().unwrap();
}

#[test]
fn browser_restarts_manual_navigation_redirect_in_same_epoch() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone".as_slice(),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(response).unwrap();
        }
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 23, Duration::from_secs(5)).unwrap();
    let mut nav = request(&format!("{origin}/start"), &origin);
    nav.mode = "navigate".into();
    nav.credentials = "include".into();
    nav.redirect = "manual".into();
    let first = session.start(nav.clone()).unwrap();
    let mut head = None;
    loop {
        match next_reply(&session, &first, NETWORK_CHUNK) {
            NetworkReply::Head(value) => head = Some(value),
            NetworkReply::Complete(done) => {
                assert!(done.error.is_none());
                break;
            }
            NetworkReply::Body(_) => (),
            NetworkReply::Pending(()) => unreachable!(),
        }
    }
    let head = head.expect("navigation redirect head");
    assert_eq!(head.status, 302);
    assert_eq!(head.url, format!("{origin}/start"));
    assert_eq!(head.url_list, vec![format!("{origin}/start")]);
    assert!(!head.redirect_tainted);
    assert!(head
        .headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("location") && h.value == "/final"));
    let mut invalid = head.clone();
    invalid.epoch += 1;
    assert!(session
        .restart_navigation(&first, &invalid, nav.clone())
        .is_err());
    nav.url = format!("{origin}/final");
    let second = session.restart_navigation(&first, &head, nav).unwrap();
    assert_eq!(first.epoch, second.epoch);
    let mut final_head = None;
    let mut body = Vec::new();
    loop {
        match next_reply(&session, &second, NETWORK_CHUNK) {
            NetworkReply::Head(value) => final_head = Some(value),
            NetworkReply::Body(chunk) => body.extend_from_slice(&chunk.bytes.0),
            NetworkReply::Complete(done) => {
                assert!(done.error.is_none());
                break;
            }
            NetworkReply::Pending(()) => unreachable!(),
        }
    }
    assert_eq!(final_head.unwrap().status, 200);
    assert_eq!(body, b"done");
    server.join().unwrap();
}

#[test]
fn browser_only_subresource_receives_validated_no_cors_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: 13\r\nConnection: close\r\n\r\nbody{color:r}").unwrap();
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 24, Duration::from_secs(5)).unwrap();
    let mut style = request(&format!("{origin}/site.css"), "http://other.test");
    style.mode = "no-cors".into();
    style.destination = "style".into();
    style.context.top_level_navigation = false;
    style.context.cookie_site = WireCookieSite::CrossSite(());
    let response = session.fetch_subresource(style).unwrap();
    assert_eq!(response.head.status, 200);
    assert_eq!(response.head.response_type, "Opaque");
    assert_eq!(response.body, b"body{color:r}");
    server.join().unwrap();
}

#[test]
fn browser_loads_multi_resource_page_through_network_child_with_direct_parity() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let fixtures: [(&str, &str, &[u8]); 6] = [
        ("/index.html", "text/html; charset=utf-8", b"<!doctype html><link rel=stylesheet href=/site.css><img src=/pixel.png><script src=/app.js></script><main>network child</main>"),
        ("/site.css", "text/css", b"@font-face{font-family:Fixture;src:url(/fixture.woff)}main{font-family:Fixture}"),
        ("/pixel.png", "image/png", b"\x89PNG\r\n\x1a\nfixture-pixels"),
        ("/fixture.woff", "font/woff", b"wOFFfixture-font-bytes"),
        ("/app.js", "text/javascript", b"fetch('/api.json').then(r=>r.json())"),
        ("/api.json", "application/json", b"{\"loaded\":true}"),
    ];
    let server = std::thread::spawn(move || {
        for _ in 0..fixtures.len() * 2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                assert!(header.len() < 16 * 1024);
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let line = std::str::from_utf8(&header)
                .unwrap()
                .lines()
                .next()
                .unwrap();
            let path = line.split_whitespace().nth(1).unwrap();
            let (_, mime, body) = fixtures.iter().find(|(url, _, _)| *url == path).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nX-Fixture: ipc6\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(body).unwrap();
        }
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 19, Duration::from_secs(5)).unwrap();
    for (path, destination) in [
        ("/index.html", "document"),
        ("/site.css", "style"),
        ("/pixel.png", "image"),
        ("/fixture.woff", "font"),
        ("/app.js", "script"),
        ("/api.json", "connect"),
    ] {
        let url = format!("{origin}{path}");
        let direct = amazing_net::fetch(&url, &amazing_net::FetchOptions::default()).unwrap();
        let mut start = request(&url, &origin);
        start.destination = destination.into();
        start.context.top_level_navigation = destination == "document";
        let remote = session.fetch(start).unwrap();
        assert_eq!(remote.head.status, direct.status, "{path}");
        assert_eq!(remote.head.url, direct.url, "{path}");
        assert_eq!(remote.body, direct.body, "{path}");
        for name in ["content-type", "x-fixture"] {
            assert_eq!(
                remote
                    .head
                    .headers
                    .iter()
                    .find(|h| h.name == name)
                    .map(|h| h.value.as_str()),
                direct.headers.get(name).as_deref(),
                "{path} {name}",
            );
        }
    }
    server.join().unwrap();
}

#[test]
fn response_bytes_arrive_under_credit_before_network_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (consumed_tx, consumed_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\nConnection: close\r\n\r\nhello").unwrap();
        release_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        stream.write_all(b"world").unwrap();
        consumed_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 9, Duration::from_secs(5)).unwrap();
    let started = session.start(request("/stream", &origin)).unwrap();
    let mut first = Vec::new();
    let mut saw_head = false;
    while first.len() < 5 {
        match next_reply(&session, &started, 3) {
            NetworkReply::Head(head) => {
                assert_eq!(head.status, 200);
                saw_head = true;
            }
            NetworkReply::Body(chunk) => {
                assert!(saw_head);
                assert!(chunk.bytes.0.len() <= 3);
                first.extend_from_slice(&chunk.bytes.0);
            }
            NetworkReply::Pending(()) => unreachable!(),
            NetworkReply::Complete(done) => panic!("stream completed before second half: {done:?}"),
        }
    }
    release_tx.send(()).unwrap();
    assert_eq!(first, b"hello");
    let mut second = Vec::new();
    loop {
        match next_reply(&session, &started, 3) {
            NetworkReply::Body(chunk) => {
                assert!(chunk.bytes.0.len() <= 3);
                second.extend_from_slice(&chunk.bytes.0);
            }
            NetworkReply::Complete(done) => {
                assert!(done.error.is_none());
                consumed_tx.send(()).unwrap();
                break;
            }
            NetworkReply::Pending(()) => unreachable!(),
            NetworkReply::Head(_) => panic!("duplicate response head"),
        }
    }
    server.join().unwrap();
    assert_eq!(second, b"world");
}

#[test]
fn malformed_start_and_ownership_are_rejected_before_fetch() {
    use amazing_process::protocol::NetworkRequestsV2;
    let endpoint = NetworkEndpointV2::new();
    let mut start = request("/", "http://example.test");
    start.version = 1;
    assert!(endpoint.start(start.clone()).is_err());
    start.version = NETWORK_VERSION;
    start.response_limit = NETWORK_MAX_RESPONSE_BODY + 1;
    assert!(endpoint.start(start.clone()).is_err());
    start.response_limit = 1;
    start.url = "file:///etc/passwd".into();
    assert!(endpoint.start(start).is_err());
    let mut oversized = request("/", "http://example.test");
    oversized.headers[0].value = "x".repeat(65 * 1024);
    assert!(endpoint.start(oversized).is_err());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let started = endpoint.start(request("/owned", &origin)).unwrap();
    assert!(endpoint
        .read(NetworkRead {
            id: started.id,
            client_id: 8,
            epoch: started.epoch,
            credit: NETWORK_CHUNK
        })
        .is_err());
    assert!(endpoint
        .cancel(NetworkCancel {
            id: started.id,
            client_id: 8,
            epoch: started.epoch
        })
        .is_err());
    assert_eq!(
        endpoint
            .cancel(NetworkCancel {
                id: started.id,
                client_id: 7,
                epoch: started.epoch
            })
            .unwrap()
            .error
            .as_deref(),
        Some("cancelled")
    );
    assert!(endpoint
        .read(NetworkRead {
            id: started.id,
            client_id: 7,
            epoch: started.epoch,
            credit: NETWORK_CHUNK
        })
        .is_err());

    let second = endpoint.start(request("/epoch", &origin)).unwrap();
    let cancelled = endpoint
        .cancel_epoch(NetworkEpoch {
            client_id: 7,
            epoch: 1,
        })
        .unwrap();
    assert_eq!(
        cancelled,
        vec![NetworkComplete {
            id: second.id,
            epoch: 1,
            error: Some("cancelled".into())
        }]
    );
    assert!(endpoint
        .read(NetworkRead {
            id: second.id,
            client_id: 7,
            epoch: 1,
            credit: 1
        })
        .is_err());
}

#[test]
fn network_child_credentialless_executes_real_no_cors_and_preserves_other_policy_controls() {
    for policy in ["unsafe-none", "require-corp", "credentialless"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_origin = format!("http://{}", listener.local_addr().unwrap());
        let client_origin = "http://127.0.0.1:9";
        let (tx, rx) = std::sync::mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
                assert!(head.len() < 64 * 1024);
            }
            tx.send(String::from_utf8(head).unwrap()).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nCross-Origin-Resource-Policy: cross-origin\r\nSet-Cookie: server=received;Path=/;SameSite=Lax\r\nConnection: close\r\n\r\nimage").unwrap();
        });
        let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
        let session = NetworkSession::launch(exe, 91, Duration::from_secs(5)).unwrap();
        let cookie = DocumentCookie {
            client_id: 0,
            epoch: 0,
            document_url: format!("{target_origin}/resource"),
            top_level_origin: WireOrigin::Tuple(target_origin.clone()),
            now_unix: 1_780_000_000,
            assignment: Some("seed=present;Path=/;SameSite=Lax".into()),
        };
        session.document_cookie(cookie.clone()).unwrap();
        let mut start = request(&format!("{target_origin}/resource"), client_origin);
        start.destination = "image".into();
        start.mode = "no-cors".into();
        start.credentials = "include".into();
        start.context.top_level_navigation = false;
        start.context.embedder_policy = policy.into();
        let response = session.fetch(start).unwrap();
        assert_eq!(response.head.response_type, "Opaque");
        let wire = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .to_ascii_lowercase();
        assert_eq!(
            wire.contains("cookie: seed=present"),
            policy != "credentialless"
        );
        let mut query = cookie;
        query.assignment = None;
        let cookie = session.document_cookie(query).unwrap();
        assert!(cookie.contains("seed=present"));
        assert_eq!(
            cookie.contains("server=received"),
            policy != "credentialless"
        );
        peer.join().unwrap();
    }
}

#[test]
fn network_embedder_policy_wire_rejects_unknown_and_malformed_authority_before_socket_admission() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mut endpoint = NetworkEndpointV2::default();
    for policy in ["", "Credentialless", "credentialless ", "credentialless,require-corp", "credentialless;report-to=\"x\"", "unknown", "credentialless\0"] {
        let mut start = request(&format!("{origin}/authority"), &origin);
        start.context.embedder_policy = policy.into();
        assert!(endpoint.start(start).is_err(), "wire policy {policy:?}");
        assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }
}
