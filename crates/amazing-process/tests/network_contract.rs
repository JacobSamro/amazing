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
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        let body = b"<p>network contract</p>";
        write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Type: text/html; charset=utf-8\r\nX-Test: two\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(body).unwrap();
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 7, Duration::from_secs(5)).unwrap();
    let response = session.fetch(request("/index", &origin)).unwrap();
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
fn response_bytes_arrive_under_credit_before_network_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\nConnection: close\r\n\r\nhello").unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        stream.write_all(b"world").unwrap();
    });
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let session = NetworkSession::launch(exe, 9, Duration::from_secs(5)).unwrap();
    let started = session.start(request("/stream", &origin)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    let mut first = Vec::new();
    let mut saw_head = false;
    while std::time::Instant::now() < deadline && first.len() < 5 {
        match session.read(&started, 3).unwrap() {
            NetworkReply::Head(head) => {
                assert_eq!(head.status, 200);
                saw_head = true;
            }
            NetworkReply::Body(chunk) => {
                assert!(saw_head);
                assert!(chunk.bytes.0.len() <= 3);
                first.extend_from_slice(&chunk.bytes.0);
            }
            NetworkReply::Pending(()) => std::thread::sleep(Duration::from_millis(1)),
            NetworkReply::Complete(done) => panic!("stream completed before second half: {done:?}"),
        }
    }
    release_tx.send(()).unwrap();
    assert_eq!(first, b"hello");
    let mut second = Vec::new();
    let finish_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < finish_deadline,
            "stream completion timed out"
        );
        match session.read(&started, 3).unwrap() {
            NetworkReply::Body(chunk) => {
                assert!(chunk.bytes.0.len() <= 3);
                second.extend_from_slice(&chunk.bytes.0);
            }
            NetworkReply::Complete(done) => {
                assert!(done.error.is_none());
                break;
            }
            NetworkReply::Pending(()) => std::thread::sleep(Duration::from_millis(1)),
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
