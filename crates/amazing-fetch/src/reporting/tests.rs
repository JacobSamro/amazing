use super::*;
use amazing_net::dispatch::DispatchLimits;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn violation(url: &str) -> CspInlineViolation {
    CspInlineViolation {
        effective_directive: "script-src-elem".into(),
        violated_directive: "script-src-elem".into(),
        source_directive: "default-src".into(),
        original_policy: "default-src 'none'; report-to csp".into(),
        original_policy_units: "default-src 'none'; report-to csp".encode_utf16().collect(),
        policy_index: 0,
        disposition: CspDisposition::Enforce,
        blocked_uri: "inline".into(),
        element: Some(7),
        document_uri: url.into(),
        report_url: strip_url(url),
        referrer: String::new(),
        status_code: 200,
        source_file: url.into(),
        line_number: 4,
        column_number: 9,
        sample: vec![0x61, 0xd800],
        report_to: Some("csp".into()),
        report_uris: vec!["/legacy".into()],
    }
}
fn context(url: &HttpUrl) -> ClientContext {
    ClientContext {
        cookie_site: crate::cookies::CookieSite::Origin(url.origin().clone()),
        now_unix: 1800000000,
        ..Default::default()
    }
}
fn headers(value: &str) -> HeaderList {
    let mut headers = HeaderList::default();
    headers.append("Reporting-Endpoints", value).unwrap();
    headers
}
fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert_ne!(count, 0, "request ended early");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = head
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}
fn run(service: &mut NetworkService, count: usize) -> Vec<ReportFinished> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut finished = Vec::new();
    while service.has_pending_reporting_io() && Instant::now() < deadline {
        for event in service.step(31, Instant::now()) {
            if let ServiceEvent::ReportFinished(event) = event {
                finished.push(event);
            }
        }
        thread::yield_now();
    }
    assert!(
        !service.has_pending_reporting_io(),
        "report transport deadline"
    );
    assert_eq!(finished.len(), count);
    assert!(
        service
            .step(4096, Instant::now())
            .into_iter()
            .all(|event| !matches!(event, ServiceEvent::ReportFinished(_)))
    );
    finished
}
#[test]
fn complete_dictionary_registration_and_document_custody() {
    let url = HttpUrl::parse("https://example.test/dir/page").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let mut raw = headers(r#"csp="/first";ignored=(bad)"#); // invalid parameter invalidates all
    let handle = service
        .register_reporting_response(url.clone(), &raw, context(&url))
        .unwrap();
    assert!(
        service
            .reporting
            .document(handle)
            .unwrap()
            .endpoints
            .is_empty()
    );
    raw = headers(r#"a=42, b=("x";foo=?1 token);p=@12, csp="/first";p=%"ok""#);
    raw.append(
        "reporting-endpoints",
        r#"csp="../last", unknown=:YWJj:, x=?1"#,
    )
    .unwrap();
    let handle = service
        .register_reporting_response(url.clone(), &raw, context(&url))
        .unwrap();
    assert_eq!(
        service.reporting.document(handle).unwrap().endpoints["csp"].serialize(),
        "https://example.test/last"
    );
    let replaced = service
        .register_reporting_response(
            url.clone(),
            &headers(r#"csp="/ok", csp=token"#),
            context(&url),
        )
        .unwrap();
    assert!(
        service
            .reporting
            .document(replaced)
            .unwrap()
            .endpoints
            .is_empty()
    );
    let child = service
        .inherit_reporting_document(
            handle,
            &ParsedUrl::parse("about:srcdoc").unwrap(),
            context(&url),
        )
        .unwrap();
    assert!(service.forget_reporting_document(handle));
    assert!(service.reporting.document(handle).is_err());
    assert_eq!(
        service.reporting.document(child).unwrap().endpoints.len(),
        0
    );
    let other = NetworkService::new(DispatchLimits::default());
    assert!(other.reporting.document(child).is_err());
    assert!(
        service
            .inherit_reporting_document(
                child,
                &ParsedUrl::parse("https://example.test").unwrap(),
                context(&url)
            )
            .is_err()
    );
    for (base, value) in [
        (
            "http://example.test/",
            r#"csp="https://example.test/report""#,
        ),
        ("https://example.test/", r#"csp="http://evil.test/report""#),
        (
            "https://example.test/",
            r#"csp="https://u:p@example.test/report""#,
        ),
        ("https://example.test/", r#"csp="/valid", =broken"#),
    ] {
        let url = HttpUrl::parse(base).unwrap();
        assert!(endpoints(&url, &headers(value)).is_empty());
    }
}
#[test]
fn serializers_keep_exact_dom_sample_and_remove_url_secrets() {
    let mut v = violation("https://user:password@example.test/path#fragment");
    let modern = String::from_utf8(serialize(&v, true, 25)).unwrap();
    let legacy = String::from_utf8(serialize(&v, false, 0)).unwrap();
    for data in [&modern, &legacy] {
        assert!(data.contains(r#"a\ud800"#));
        assert!(!data.contains("password"));
        assert!(!data.contains("fragment"));
    }
    assert!(modern.contains(r#""age":25"#));
    assert!(modern.contains(r#""user_agent":"Ferric/0.1""#));
    assert!(legacy.contains(r#""csp-report":{"document-uri":"https://example.test/path""#));
    v.source_file.clear();
    v.document_uri = "about:srcdoc".into();
    let modern = String::from_utf8(serialize(&v, true, 0)).unwrap();
    assert!(modern.contains(r#""documentURL":"about""#));
    assert!(modern.contains(r#""lineNumber":null"#));
    assert!(
        !String::from_utf8(serialize(&v, false, 0))
            .unwrap()
            .contains("line-number")
    );
}
#[test]
fn real_same_origin_report_retirement_and_gone_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = HttpUrl::parse(&format!("http://{}/page", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::channel();
    let peer = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(b"HTTP/1.1 410 Gone\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
        }
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service
        .register_reporting_response(url.clone(), &headers(r#"csp="/report""#), context(&url))
        .unwrap();
    let v = violation(&url.serialize());
    let now = Instant::now();
    let ids = service.queue_csp_report(handle, &v, now, now).unwrap();
    let finished = run(&mut service, 1);
    assert_eq!(finished[0].id, ids[0]);
    assert_eq!(
        finished[0].outcome,
        ReportOutcome::Response {
            status: 410,
            accepted: false
        }
    );
    assert_eq!(
        service.queue_csp_report(handle, &v, now, Instant::now()),
        Err(ReportError::NoEndpoint)
    );
    let handle = service
        .register_reporting_response(url.clone(), &headers(r#"csp="/report""#), context(&url))
        .unwrap();
    service
        .queue_csp_report(handle, &v, now, Instant::now())
        .unwrap();
    assert!(service.forget_reporting_document(handle));
    assert_eq!(
        service.queue_csp_report(handle, &v, now, Instant::now()),
        Err(ReportError::StaleDocument)
    );
    run(&mut service, 1);
    for _ in 0..2 {
        let request = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(request.starts_with("POST /report HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/reports+json")
        );
        assert!(request.contains(r#""type":"csp-violation""#));
    }
    peer.join().unwrap();
    assert_eq!(service.reporting.retained_bytes, 0);
}
#[test]
fn real_cross_origin_preflight_then_post_and_failure_stops_post() {
    for allowed in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = format!("http://{}/report", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            let allow = if allowed {
                "Access-Control-Allow-Origin: http://localhost:12345\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Max-Age: 0\r\n"
            } else {
                "Access-Control-Allow-Origin: http://wrong.test\r\n"
            };
            stream.write_all(format!("HTTP/1.1 204 No Content\r\n{allow}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            drop(stream);
            if allowed {
                let (mut stream, _) = listener.accept().unwrap();
                tx.send(read_request(&mut stream)).unwrap();
                stream.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: http://localhost:12345\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let url = HttpUrl::parse("http://localhost:12345/page").unwrap();
        let mut service = NetworkService::new(DispatchLimits::default());
        let handle = service
            .register_reporting_response(
                url.clone(),
                &headers(&format!("csp={}", quote(&target))),
                context(&url),
            )
            .unwrap();
        let now = Instant::now();
        service
            .queue_csp_report(handle, &violation(&url.serialize()), now, now)
            .unwrap();
        let finished = run(&mut service, 1);
        let options = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(options.starts_with("OPTIONS /report HTTP/1.1"));
        assert!(
            options
                .to_ascii_lowercase()
                .contains("access-control-request-headers: content-type")
        );
        assert!(!options.to_ascii_lowercase().contains("cookie:"));
        if allowed {
            assert_eq!(
                finished[0].outcome,
                ReportOutcome::Response {
                    status: 204,
                    accepted: true
                }
            );
            let post = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(post.starts_with("POST /report HTTP/1.1"));
            assert!(!post.to_ascii_lowercase().contains("cookie:"));
        } else {
            assert_eq!(
                finished[0].outcome,
                ReportOutcome::Failed(ReportError::Policy(FetchError::Cors))
            );
            assert!(rx.try_recv().is_err());
        }
        peer.join().unwrap();
    }
}
#[test]
fn real_legacy_has_unsafe_native_content_type_and_never_follows_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = HttpUrl::parse(&format!("http://{}/page", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::channel();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        tx.send(read_request(&mut stream)).unwrap();
        stream.write_all(b"HTTP/1.1 307 Redirect\r\nLocation: /must-not-follow\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service
        .register_reporting_response(url.clone(), &HeaderList::default(), context(&url))
        .unwrap();
    let mut v = violation(&url.serialize());
    let now = Instant::now();
    assert_eq!(
        service.queue_csp_report(handle, &v, now, now),
        Err(ReportError::NoEndpoint)
    ); // report-to suppresses legacy
    v.report_to = Some(String::new());
    assert_eq!(
        service.queue_csp_report(handle, &v, now, now),
        Err(ReportError::NoEndpoint)
    );
    v.report_to = None;
    service.queue_csp_report(handle, &v, now, now).unwrap();
    let finished = run(&mut service, 1);
    assert_eq!(
        finished[0].outcome,
        ReportOutcome::Failed(ReportError::Policy(FetchError::Redirect))
    );
    let request = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(request.starts_with("POST /legacy HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/csp-report")
    );
    assert!(request.contains(r#""csp-report""#));
    assert!(
        request
            .to_ascii_lowercase()
            .contains(&format!("referer: {}", url.serialize()))
    );
    peer.join().unwrap();
}
#[test]
fn bounded_atomic_admission_and_transport_terminal_are_explicit() {
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = HttpUrl::parse(&format!("http://{}/page", closed.local_addr().unwrap())).unwrap();
    drop(closed);
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service
        .register_reporting_response(url.clone(), &headers(r#"csp="/report""#), context(&url))
        .unwrap();
    let mut v = violation(&url.serialize());
    v.sample = vec![0xd800; MAX_REPORT_BYTES];
    let now = Instant::now();
    assert_eq!(
        service.queue_csp_report(handle, &v, now, now),
        Err(ReportError::Limit)
    );
    assert_eq!(service.dispatcher.in_flight(), 0);
    let ids = service
        .queue_csp_report(handle, &violation(&url.serialize()), now, now)
        .unwrap();
    assert!(service.has_pending_reporting_io());
    let finished = run(&mut service, 1);
    assert_eq!(finished[0].id, ids[0]);
    assert_eq!(
        finished[0].outcome,
        ReportOutcome::Failed(ReportError::Transport)
    );
    assert_eq!(service.reporting.retained_bytes, 0);
}

#[test]
fn real_same_origin_cookies_and_modern_redirect_body_are_preserved() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = HttpUrl::parse(&format!("http://{}/page", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::channel();
    let peer = thread::spawn(move || {
        for response in [b"HTTP/1.1 307 Redirect\r\nLocation: /final\r\nSet-Cookie: changed=two; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()] {
            let(mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();stream.write_all(response).unwrap();
        }
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let ctx = context(&url);
    let access = crate::cookies::CookieAccess {
        url: &url,
        site_for_cookies: ctx.cookie_site.clone(),
        http_api: true,
        top_level_navigation: false,
        method: "POST",
        partition_key: None,
    };
    service
        .cookies
        .set_cookie(b"session=one; Path=/; HttpOnly", &access, ctx.now_unix)
        .unwrap();
    let handle = service
        .register_reporting_response(url.clone(), &headers(r#"csp="/first""#), ctx)
        .unwrap();
    let now = Instant::now();
    service
        .queue_csp_report(handle, &violation(&url.serialize()), now, now)
        .unwrap();
    let finished = run(&mut service, 1);
    assert_eq!(
        finished[0].outcome,
        ReportOutcome::Response {
            status: 202,
            accepted: true
        }
    );
    let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(first.starts_with("POST /first HTTP/1.1"));
    assert!(second.starts_with("POST /final HTTP/1.1"));
    assert!(first.to_ascii_lowercase().contains("cookie: session=one"));
    assert!(second.contains("changed=two"));
    assert_eq!(
        first.split_once("\r\n\r\n").unwrap().1,
        second.split_once("\r\n\r\n").unwrap().1
    );
    peer.join().unwrap();
}

#[test]
fn real_cross_origin_legacy_sends_post_without_cookies_or_cors_preflight() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target =
        HttpUrl::parse(&format!("http://{}/legacy", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::channel();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        tx.send(read_request(&mut stream)).unwrap();
        stream
            .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let url = HttpUrl::parse("http://localhost:12345/page").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let access = crate::cookies::CookieAccess {
        url: &target,
        site_for_cookies: crate::cookies::CookieSite::NoClient,
        http_api: true,
        top_level_navigation: false,
        method: "POST",
        partition_key: None,
    };
    service
        .cookies
        .set_cookie(b"target=secret; Path=/", &access, 1800000000)
        .unwrap();
    let handle = service
        .register_reporting_response(url.clone(), &HeaderList::default(), context(&url))
        .unwrap();
    let mut v = violation(&url.serialize());
    v.report_to = None;
    v.report_uris = vec![target.serialize()];
    let now = Instant::now();
    service.queue_csp_report(handle, &v, now, now).unwrap();
    let finished = run(&mut service, 1);
    assert_eq!(
        finished[0].outcome,
        ReportOutcome::Response {
            status: 500,
            accepted: true
        }
    );
    let request = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(request.starts_with("POST /legacy HTTP/1.1"));
    assert!(!request.to_ascii_lowercase().contains("cookie:"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("origin: http://localhost:12345")
    );
    peer.join().unwrap();
}

#[test]
fn admission_limit_rolls_back_every_legacy_wire_request() {
    let url = HttpUrl::parse("http://127.0.0.1:23456/page").unwrap();
    let mut limits = DispatchLimits::default();
    limits.max_in_flight = 1;
    let mut service = NetworkService::new(limits);
    let handle = service
        .register_reporting_response(url.clone(), &HeaderList::default(), context(&url))
        .unwrap();
    let mut v = violation(&url.serialize());
    v.report_to = None;
    v.report_uris = vec!["/one".into(), "/two".into()];
    let now = Instant::now();
    assert_eq!(
        service.queue_csp_report(handle, &v, now, now),
        Err(ReportError::Transport)
    );
    assert!(!service.has_pending_reporting_io());
    assert_eq!(service.dispatcher.in_flight(), 0);
    assert_eq!(service.reporting.retained_bytes, 0);
    assert!(
        service
            .step(4096, now)
            .into_iter()
            .all(|event| !matches!(event, ServiceEvent::ReportFinished(_)))
    );
}

fn receiving_client(url: &HttpUrl, creation: &str) -> crate::ReceivingDocumentClient<(u64, u64)> {
    crate::ReceivingDocumentClient {
        identity: (17, 4),
        creation_url: ParsedUrl::parse(creation).unwrap(),
        document_url: url.as_parsed().clone(),
        base_url: url.as_parsed().clone(),
        origin: url.origin().clone(),
        csp_self_origin: url.origin().clone(),
        context: context(url),
        referrer_policy: crate::ReferrerPolicy::NoReferrer,
    }
}
#[test]
fn test_report_creation_is_exact_independent_of_endpoint_and_client_url() {
    let accepted = HttpUrl::parse("https://response.test/redirected/page").unwrap();
    let creation = "https://user:secret@creation.test/original?q=1#private";
    let client = receiving_client(&accepted, creation);
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service
        .register_reporting_client_response(&client, &accepted, &HeaderList::default())
        .unwrap();
    let message = vec![0, 0x22, 0x5c, 0x61, 0xd800, 0xdc00, 0xffff];
    for group in [
        vec![],
        "default".encode_utf16().collect(),
        vec![0xd800],
        vec![0xfffd],
    ] {
        let snapshot = service
            .generate_test_report(handle, &message, &group, Instant::now())
            .unwrap();
        assert_eq!(snapshot.report_url, "https://creation.test/original?q=1");
        assert_eq!(snapshot.message, message);
        assert_eq!(snapshot.delivery, Err(ReportError::NoEndpoint));
        assert!(!service.has_pending_reporting_io());
    }
    assert_eq!(
        service.reporting.document(handle).unwrap().origin,
        client.origin
    );
    assert_eq!(
        service.reporting.document(handle).unwrap().referrer_policy,
        crate::ReferrerPolicy::NoReferrer
    );
    let other = NetworkService::new(DispatchLimits::default());
    assert!(other.reporting.document(handle).is_err());
    assert!(service.forget_reporting_document(handle));
    assert_eq!(
        service.generate_test_report(handle, &message, &[], Instant::now()),
        Err(ReportError::StaleDocument)
    );
}
#[test]
fn test_report_about_child_retains_actual_client_custody_after_parent_retirement() {
    let url = HttpUrl::parse("https://example.test/page").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let parent = service
        .register_reporting_response(url.clone(), &headers(r#"default="/report""#), context(&url))
        .unwrap();
    let mut client = receiving_client(&url, "about:srcdoc#private");
    client.origin = Origin::opaque();
    client.context.now_unix = 42;
    let child = service.inherit_reporting_client(parent, &client).unwrap();
    assert!(service.forget_reporting_document(parent));
    let saved = service.reporting.document(child).unwrap();
    assert_eq!(saved.origin, client.origin);
    assert_eq!(saved.context.now_unix, 42);
    assert!(saved.endpoints.is_empty());
    assert!(saved.failures.is_empty());
    let snapshot = service
        .generate_test_report(child, &[0xdfff], &[], Instant::now())
        .unwrap();
    assert_eq!(snapshot.report_url, "about");
    assert_eq!(snapshot.message, vec![0xdfff]);
    assert_eq!(snapshot.delivery, Err(ReportError::NoEndpoint));
    client.creation_url = ParsedUrl::parse("https://wrong.test/").unwrap();
    assert_eq!(
        service.inherit_reporting_client(child, &client),
        Err(ReportError::InvalidEndpoint)
    );
}
#[test]
fn real_test_report_post_exact_utf16_terminal_status_and_endpoint_retirement() {
    for status in [204, 410, 500] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!(
            "http://{}/changed",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let creation = format!(
            "http://user:secret@{}/original#private",
            listener.local_addr().unwrap()
        );
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status} Result\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let client = receiving_client(&url, &creation);
        let handle = service
            .register_reporting_client_response(
                &client,
                &url,
                &headers(r#"default="/report", other="/unused""#),
            )
            .unwrap();
        let mut message = vec![0x22, 0x5c, 0, 0xd800, 0xdc00, 0xdfff];
        let snapshot = service
            .generate_test_report(
                handle,
                &message,
                &"default".encode_utf16().collect::<Vec<_>>(),
                Instant::now(),
            )
            .unwrap();
        let id = snapshot.delivery.unwrap();
        message.clear();
        assert_eq!(snapshot.message.len(), 6);
        let completed = run(&mut service, 1);
        assert_eq!(completed[0].id, id);
        assert_eq!(
            completed[0].outcome,
            ReportOutcome::Response {
                status,
                accepted: (200..300).contains(&status)
            }
        );
        let request = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(request.starts_with("POST /report HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/reports+json")
        );
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let expected = format!(
            "[{{\"age\":0,\"type\":\"test\",\"url\":{},\"user_agent\":{},\"body\":{{\"body_message\":{}}}}}]",
            quote(&snapshot.report_url),
            quote(amazing_net::http::USER_AGENT),
            quote_units(&snapshot.message)
        );
        assert_eq!(body, expected);
        assert!(!body.contains("csp-violation"));
        assert!(!body.contains("originalPolicy"));
        assert_eq!(service.reporting.retained_bytes, 0);
        assert_eq!(
            service
                .reporting
                .document(handle)
                .unwrap()
                .endpoints
                .contains_key("default"),
            status != 410
        );
        assert!(
            service
                .reporting
                .document(handle)
                .unwrap()
                .endpoints
                .contains_key("other")
        );
        peer.join().unwrap();
    }
}
#[test]
fn real_test_report_cross_origin_preflight_allow_and_deny_keep_creation() {
    for allowed in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = format!("http://{}/test-report", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            let allow = if allowed {
                "Access-Control-Allow-Origin: http://localhost:12345\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Max-Age: 0\r\n"
            } else {
                "Access-Control-Allow-Origin: http://wrong.test\r\n"
            };
            stream.write_all(format!("HTTP/1.1 204 No Content\r\n{allow}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            drop(stream);
            if allowed {
                let (mut stream, _) = listener.accept().unwrap();
                tx.send(read_request(&mut stream)).unwrap();
                stream.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: http://localhost:12345\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let url = HttpUrl::parse("http://localhost:12345/page").unwrap();
        let mut service = NetworkService::new(DispatchLimits::default());
        let client = receiving_client(&url, "http://localhost:12345/creation#private");
        let handle = service
            .register_reporting_client_response(
                &client,
                &url,
                &headers(&format!("named={}", quote(&target))),
            )
            .unwrap();
        let snapshot = service
            .generate_test_report(
                handle,
                &[0xd800],
                &"named".encode_utf16().collect::<Vec<_>>(),
                Instant::now(),
            )
            .unwrap();
        assert!(snapshot.delivery.is_ok());
        assert_eq!(snapshot.message, vec![0xd800]);
        assert!(service.forget_reporting_document(handle));
        let completed = run(&mut service, 1);
        let options = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(options.starts_with("OPTIONS /test-report HTTP/1.1"));
        assert!(!options.to_ascii_lowercase().contains("cookie:"));
        if allowed {
            let post = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(post.starts_with("POST /test-report HTTP/1.1"));
            assert!(post.contains(r#""type":"test""#));
            assert!(!post.to_ascii_lowercase().contains("cookie:"));
            assert_eq!(
                completed[0].outcome,
                ReportOutcome::Response {
                    status: 204,
                    accepted: true
                }
            );
        } else {
            assert!(rx.try_recv().is_err());
            assert_eq!(
                completed[0].outcome,
                ReportOutcome::Failed(ReportError::Policy(FetchError::Cors))
            );
        }
        peer.join().unwrap();
    }
}
#[test]
fn test_report_snapshot_bound_and_transport_credit_are_distinct() {
    let url = HttpUrl::parse("http://localhost:12345/page").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let client = receiving_client(&url, &url.serialize());
    let empty = service
        .register_reporting_client_response(&client, &url, &HeaderList::default())
        .unwrap();
    let message = vec![0xd800; MAX_REPORT_BYTES / 2];
    assert_eq!(
        service
            .generate_test_report(empty, &message, &[], Instant::now())
            .unwrap()
            .message,
        message
    );
    assert_eq!(
        service.generate_test_report(
            empty,
            &vec![0; MAX_REPORT_BYTES / 2 + 1],
            &[],
            Instant::now()
        ),
        Err(ReportError::Limit)
    );
    let configured = service
        .register_reporting_client_response(&client, &url, &headers(r#"default="/report""#))
        .unwrap();
    let snapshot = service
        .generate_test_report(
            configured,
            &message,
            &"default".encode_utf16().collect::<Vec<_>>(),
            Instant::now(),
        )
        .unwrap();
    assert_eq!(snapshot.message, message);
    assert_eq!(snapshot.delivery, Err(ReportError::Limit));
    assert!(!service.has_pending_reporting_io());
    assert_eq!(service.reporting.retained_bytes, 0);
    for group in [
        vec![],
        vec![0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0],
        vec![0xd800],
    ] {
        assert_eq!(
            service
                .generate_test_report(configured, &[], &group, Instant::now())
                .unwrap()
                .delivery,
            Err(ReportError::NoEndpoint)
        );
    }
}

#[test]
fn initial_reporting_globals_preserve_real_client_without_response_or_endpoint() {
    let base = HttpUrl::parse("https://base.test/environment").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    for creation in [
        "about:blank",
        "about:srcdoc",
        "data:text/html,literal",
        "https://literal.test/initial#fragment",
    ] {
        let mut client = receiving_client(&base, creation);
        client.origin = Origin::opaque();
        client.context.now_unix = 73;
        let handle = service.register_reporting_initial_client(&client).unwrap();
        let saved = service.reporting.document(handle).unwrap();
        assert_eq!(saved.creation_url, client.creation_url);
        assert_eq!(saved.origin, client.origin);
        assert_eq!(saved.context.now_unix, 73);
        assert_eq!(saved.referrer_policy, client.referrer_policy);
        assert!(saved.endpoints.is_empty());
        assert!(saved.failures.is_empty());
        let report = service
            .generate_test_report(
                handle,
                &[0xd800],
                &"default".encode_utf16().collect::<Vec<_>>(),
                Instant::now(),
            )
            .unwrap();
        assert_eq!(report.report_url, strip_url(creation));
        assert_eq!(report.message, [0xd800]);
        assert_eq!(report.delivery, Err(ReportError::NoEndpoint));
        assert!(!service.has_pending_reporting_io());
        assert!(service.forget_reporting_document(handle));
    }
}

#[test]
fn fresh_about_commit_after_creator_retirement_never_leaks_endpoint_configuration() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base =
        HttpUrl::parse(&format!("http://{}/parent", listener.local_addr().unwrap())).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let parent = service
        .register_reporting_response(
            base.clone(),
            &headers(r#"default="/must-not-receive""#),
            context(&base),
        )
        .unwrap();
    assert!(service.forget_reporting_document(parent));
    for creation in ["about:blank", "about:srcdoc#child"] {
        let mut client = receiving_client(&base, creation);
        client.origin = Origin::opaque();
        client.context.now_unix = 87;
        let child = service.register_reporting_initial_client(&client).unwrap();
        assert_ne!(parent, child);
        let saved = service.reporting.document(child).unwrap();
        assert_eq!(saved.origin, client.origin);
        assert_eq!(saved.context.now_unix, 87);
        assert!(saved.endpoints.is_empty());
        let report = service
            .generate_test_report(
                child,
                &[0xdfff],
                &"default".encode_utf16().collect::<Vec<_>>(),
                Instant::now(),
            )
            .unwrap();
        assert_eq!(report.report_url, "about");
        assert_eq!(report.message, [0xdfff]);
        assert_eq!(report.delivery, Err(ReportError::NoEndpoint));
        assert_eq!(service.dispatcher.in_flight(), 0);
        assert!(!service.has_pending_reporting_io());
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(matches!(
        service.generate_test_report(parent, &[], &[], Instant::now()),
        Err(ReportError::StaleDocument)
    ));
}

#[test]
fn actual_same_global_reporting_handle_reuses_own_endpoints_without_fresh_child_authority() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base =
        HttpUrl::parse(&format!("http://{}/parent", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::channel();
    let peer = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 204 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let client = receiving_client(&base, &base.serialize());
    let global = service
        .register_reporting_client_response(&client, &base, &headers(r#"default="/own-global""#))
        .unwrap();
    for units in [vec![0xd800], vec![0x61, 0xdfff]] {
        let receipt = service
            .generate_test_report(
                global,
                &units,
                &"default".encode_utf16().collect::<Vec<_>>(),
                Instant::now(),
            )
            .unwrap();
        let id = receipt.delivery.unwrap();
        let completed = run(&mut service, 1);
        assert_eq!(completed[0].document, global);
        assert_eq!(completed[0].id, id);
        assert_eq!(
            completed[0].outcome,
            ReportOutcome::Response {
                status: 204,
                accepted: true
            }
        );
        let wire = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(wire.starts_with("POST /own-global HTTP/1.1"));
        assert!(wire.contains(&format!("\"body_message\":{}", quote_units(&units))));
    }
    peer.join().unwrap();
    let about = receiving_client(&base, "about:srcdoc");
    let child = service.inherit_reporting_client(global, &about).unwrap();
    assert!(
        service
            .reporting
            .document(child)
            .unwrap()
            .endpoints
            .is_empty()
    );
    assert!(
        service
            .reporting
            .document(global)
            .unwrap()
            .endpoints
            .contains_key("default")
    );
    let other = NetworkService::new(DispatchLimits::default());
    assert!(other.reporting.document(child).is_err());
    assert!(service.forget_reporting_document(global));
    let receipt = service
        .generate_test_report(
            child,
            &[0xdfff],
            &"default".encode_utf16().collect::<Vec<_>>(),
            Instant::now(),
        )
        .unwrap();
    assert_eq!(receipt.delivery, Err(ReportError::NoEndpoint));
    assert!(!service.has_pending_reporting_io());
}
