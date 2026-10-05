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

#[test]
fn coep_registered_body_creation_is_observer_visible_without_endpoint_and_rejects_missing_authority()
 {
    let url = HttpUrl::parse("https://receiving.test/current").unwrap();
    let client = receiving_client(&url, "https://receiving.test/creation#fragment");
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service.register_reporting_initial_client(&client).unwrap();
    let response_urls = [
        ParsedUrl::parse("https://user:secret@resource.test/initial#fragment").unwrap(),
        ParsedUrl::parse("https://redirect.test/private-final").unwrap(),
    ];
    let now = Instant::now();
    let receipt = service
        .generate_coep_report(
            handle,
            CoepReportBody::Corp {
                response_url_list: &response_urls,
                disposition: CoepReportDisposition::Reporting,
                destination: "image",
            },
            &[0xdfff],
            now,
            now,
        )
        .unwrap();
    assert_eq!(
        receipt.report_type,
        "coep".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(
        receipt.report_url,
        "https://receiving.test/creation"
            .encode_utf16()
            .collect::<Vec<_>>()
    );
    assert_eq!(receipt.visibility, NativeReportVisibility::ObserverVisible);
    assert_eq!(receipt.delivery, Err(ReportError::NoEndpoint));
    let body: BTreeMap<_, _> = receipt
        .body
        .iter()
        .map(|(key, value)| {
            (
                String::from_utf16(key).unwrap(),
                String::from_utf16(value).unwrap(),
            )
        })
        .collect();
    assert_eq!(body["blockedURL"], "https://resource.test/initial");
    assert_eq!(body["type"], "corp");
    assert_eq!(body["disposition"], "reporting");
    assert_eq!(body["destination"], "image");
    assert_eq!(service.dispatcher.in_flight(), 0);
    assert_eq!(
        service.generate_coep_report(
            handle,
            CoepReportBody::Navigation {
                response_url_list: &[],
                disposition: CoepReportDisposition::Enforce
            },
            &[],
            now,
            now
        ),
        Err(ReportError::InvalidReport)
    );
    assert_eq!(
        service.generate_coep_report(
            handle,
            CoepReportBody::Corp {
                response_url_list: &response_urls,
                disposition: CoepReportDisposition::Enforce,
                destination: "guessed-other"
            },
            &[],
            now,
            now
        ),
        Err(ReportError::InvalidReport)
    );
    let mut other = NetworkService::new(DispatchLimits::default());
    assert_eq!(
        other.generate_coep_report(
            handle,
            CoepReportBody::Navigation {
                response_url_list: &response_urls,
                disposition: CoepReportDisposition::Enforce
            },
            &[],
            now,
            now
        ),
        Err(ReportError::StaleDocument)
    );
    assert!(service.forget_reporting_document(handle));
    assert_eq!(
        service.generate_coep_report(
            handle,
            CoepReportBody::Navigation {
                response_url_list: &response_urls,
                disposition: CoepReportDisposition::Enforce
            },
            &[],
            now,
            now
        ),
        Err(ReportError::StaleDocument)
    );
}

#[test]
fn coep_native_post_preserves_type_original_creation_and_response_chain_across_all_registered_bodies()
 {
    // Native report transport and body controls; not installed child/worker proof.
    for kind in ["navigation", "worker initialization", "corp"] {
        for disposition in [
            CoepReportDisposition::Enforce,
            CoepReportDisposition::Reporting,
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = HttpUrl::parse(&format!(
                "http://{}/current",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let client = receiving_client(
                &url,
                &format!("http://{}/creation#old", listener.local_addr().unwrap()),
            );
            let (tx, rx) = mpsc::channel();
            let peer = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                tx.send(read_request(&mut stream)).unwrap();
                stream
                    .write_all(
                        b"HTTP/1.1 204 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            });
            let mut service = NetworkService::new(DispatchLimits::default());
            let handle = service
                .register_reporting_client_response(&client, &url, &headers(r#"coep="/reports""#))
                .unwrap();
            let response_urls = [
                ParsedUrl::parse("https://user:secret@resource.test/initial#fragment").unwrap(),
                ParsedUrl::parse("https://redirect.test/private-final").unwrap(),
            ];
            let body = match kind {
                "navigation" => CoepReportBody::Navigation {
                    response_url_list: &response_urls,
                    disposition,
                },
                "worker initialization" => CoepReportBody::WorkerInitialization {
                    response_url_list: &response_urls,
                    disposition,
                },
                _ => CoepReportBody::Corp {
                    response_url_list: &response_urls,
                    disposition,
                    destination: "script",
                },
            };
            let now = Instant::now();
            let receipt = service
                .generate_coep_report(
                    handle,
                    body,
                    &"coep".encode_utf16().collect::<Vec<_>>(),
                    now,
                    now + Duration::from_secs(2),
                )
                .unwrap();
            let id = receipt.delivery.unwrap();
            assert!(service.forget_reporting_document(handle));
            let completed = run(&mut service, 1);
            assert_eq!(completed[0].id, id);
            assert_eq!(completed[0].document, handle);
            assert_eq!(completed[0].route, ReportRoute::ReportingApi);
            assert_eq!(
                completed[0].outcome,
                ReportOutcome::Response {
                    status: 204,
                    accepted: true
                }
            );
            let wire = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(wire.starts_with("POST /reports HTTP/1.1"));
            assert!(
                wire.to_ascii_lowercase()
                    .contains("content-type: application/reports+json")
            );
            assert!(wire.contains("\"type\":\"coep\""));
            assert!(wire.contains(&format!("\"type\":{}", quote(kind))));
            assert!(wire.contains("\"blockedURL\":\"https://resource.test/initial\""));
            assert!(wire.contains("\"age\":2000"));
            assert!(wire.contains(&quote(&strip_url(&client.creation_url.serialize()))));
            assert!(!wire.contains("private-final"));
            assert!(!wire.contains("secret"));
            assert!(!wire.contains("fragment"));
            assert_eq!(wire.contains("\"destination\":\"script\""), kind == "corp");
            assert!(wire.contains(&format!(
                "\"disposition\":{}",
                quote(if disposition == CoepReportDisposition::Enforce {
                    "enforce"
                } else {
                    "reporting"
                })
            )));
            assert!(service.reporting.document(handle).is_err());
            assert!(!service.has_pending_reporting_io());
            peer.join().unwrap();
        }
    }
}

#[test]
fn coep_native_terminal_status_and_410_endpoint_retirement_preserve_observer_creation() {
    for status in [200, 410, 500] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!(
            "http://{}/document",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let wire = read_request(&mut stream);
            assert!(wire.contains("\"type\":\"coep\""));
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status} Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let handle = service
            .register_reporting_client_response(
                &receiving_client(&url, &url.serialize()),
                &url,
                &headers(r#"coep="/reports""#),
            )
            .unwrap();
        let urls = [url.as_parsed().clone()];
        let now = Instant::now();
        let receipt = service
            .generate_coep_report(
                handle,
                CoepReportBody::Navigation {
                    response_url_list: &urls,
                    disposition: CoepReportDisposition::Enforce,
                },
                &"coep".encode_utf16().collect::<Vec<_>>(),
                now,
                now,
            )
            .unwrap();
        assert!(receipt.delivery.is_ok());
        let completed = run(&mut service, 1);
        assert_eq!(
            completed[0].outcome,
            ReportOutcome::Response {
                status,
                accepted: status == 200
            }
        );
        peer.join().unwrap();
        assert_eq!(
            service
                .reporting
                .document(handle)
                .unwrap()
                .endpoints
                .contains_key("coep"),
            status != 410
        );
        if status == 410 {
            let receipt = service
                .generate_coep_report(
                    handle,
                    CoepReportBody::Navigation {
                        response_url_list: &urls,
                        disposition: CoepReportDisposition::Enforce,
                    },
                    &"coep".encode_utf16().collect::<Vec<_>>(),
                    now,
                    now,
                )
                .unwrap();
            assert_eq!(receipt.visibility, NativeReportVisibility::ObserverVisible);
            assert_eq!(receipt.delivery, Err(ReportError::NoEndpoint));
        }
    }
}

#[test]
fn coep_corp_report_retains_all_current_fetch_destination_types_without_other_defaults() {
    let url = HttpUrl::parse("https://receiving.test/document").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service
        .register_reporting_initial_client(&receiving_client(&url, &url.serialize()))
        .unwrap();
    let urls = [url.as_parsed().clone()];
    let now = Instant::now();
    for destination in [
        "",
        "audio",
        "audioworklet",
        "document",
        "embed",
        "font",
        "frame",
        "iframe",
        "image",
        "json",
        "manifest",
        "object",
        "paintworklet",
        "report",
        "script",
        "serviceworker",
        "sharedworker",
        "style",
        "text",
        "track",
        "video",
        "webidentity",
        "worker",
        "xslt",
    ] {
        let receipt = service
            .generate_coep_report(
                handle,
                CoepReportBody::Corp {
                    response_url_list: &urls,
                    disposition: CoepReportDisposition::Reporting,
                    destination,
                },
                &[],
                now,
                now,
            )
            .unwrap();
        assert_eq!(receipt.delivery, Err(ReportError::NoEndpoint));
        assert!(receipt.body.contains(&(
            "destination".encode_utf16().collect(),
            destination.encode_utf16().collect()
        )));
    }
    assert!(!service.has_pending_reporting_io());
}

#[test]
fn prepared_report_preserves_original_global_after_native_callback_retirement_and_error() {
    // Native event/report boundary control, not installed SPVE proof.
    for modern in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = HttpUrl::parse(&format!(
            "http://{}/original",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let mut service = NetworkService::new(DispatchLimits::default());
        let handle = service
            .register_reporting_response(url.clone(), &headers(r#"csp="/report""#), context(&url))
            .unwrap();
        let mut v = violation(&url.serialize());
        if !modern {
            v.report_to = None;
            v.report_uris = vec!["/legacy".into()];
        }
        let queued = Instant::now();
        let plan = service.prepare_csp_report(handle, &v, queued).unwrap();
        assert!(!service.has_pending_reporting_io());
        assert_eq!(service.dispatcher.in_flight(), 0);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let author_callback = |service: &mut NetworkService| -> Result<(), &'static str> {
            assert!(service.forget_reporting_document(handle));
            Err("native callback exception")
        };
        assert!(author_callback(&mut service).is_err());
        v.original_policy_units = "changed after event".encode_utf16().collect();
        v.report_uris = vec!["/wrong-after-event".into()];
        v.report_to = Some("wrong-after-event".into());
        listener.set_nonblocking(false).unwrap();
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 204 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let ids = service
            .queue_prepared_csp_report(plan, queued + Duration::from_secs(2))
            .unwrap();
        let completed = run(&mut service, 1);
        assert_eq!(completed[0].id, ids[0]);
        assert_eq!(completed[0].document, handle);
        assert_eq!(
            completed[0].route,
            if modern {
                ReportRoute::ReportingApi
            } else {
                ReportRoute::LegacyCsp
            }
        );
        let wire = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(wire.starts_with(if modern {
            "POST /report HTTP/1.1"
        } else {
            "POST /legacy HTTP/1.1"
        }));
        assert!(!wire.contains("changed after event"));
        assert!(wire.contains("default-src 'none'; report-to csp"));
        if modern {
            assert!(wire.contains("\"age\":2000"));
        }
        assert!(service.reporting.document(handle).is_err());
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
        assert!(!service.has_pending_reporting_io());
        peer.join().unwrap();
    }
}

#[test]
fn prepared_report_no_endpoint_does_not_restore_global_or_leak_to_fresh_about() {
    let url = HttpUrl::parse("https://original.test/page").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let original = service
        .register_reporting_response(url.clone(), &HeaderList::default(), context(&url))
        .unwrap();
    let v = violation(&url.serialize());
    let plan = service
        .prepare_csp_report(original, &v, Instant::now())
        .unwrap();
    assert!(service.forget_reporting_document(original));
    let client = receiving_client(&url, "about:blank");
    let child = service.register_reporting_initial_client(&client).unwrap();
    assert_eq!(
        service.queue_prepared_csp_report(plan, Instant::now()),
        Err(ReportError::NoEndpoint)
    );
    assert!(service.reporting.document(original).is_err());
    assert!(
        service
            .reporting
            .document(child)
            .unwrap()
            .endpoints
            .is_empty()
    );
    assert!(matches!(
        service.prepare_csp_report(original, &v, Instant::now()),
        Err(ReportError::StaleDocument)
    ));
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
    assert_eq!(service.dispatcher.in_flight(), 0);
}

#[test]
fn prepared_report_is_service_bound_and_drop_releases_bounded_snapshot_credit() {
    let url = HttpUrl::parse("https://original.test/page").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let handle = service
        .register_reporting_response(url.clone(), &HeaderList::default(), context(&url))
        .unwrap();
    let mut v = violation(&url.serialize());
    let plan = service
        .prepare_csp_report(handle, &v, Instant::now())
        .unwrap();
    let mut other = NetworkService::new(DispatchLimits::default());
    assert_eq!(
        other.queue_prepared_csp_report(plan, Instant::now()),
        Err(ReportError::StaleDocument)
    );
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
    assert_eq!(other.reporting.prepared_bytes.load(Ordering::Acquire), 0);
    v.sample = vec![0xdfff; 3000];
    let mut plans = Vec::new();
    loop {
        match service.prepare_csp_report(handle, &v, Instant::now()) {
            Ok(plan) => plans.push(plan),
            Err(ReportError::Limit) => break,
            Err(error) => panic!("unexpected admission error: {error:?}"),
        }
        assert!(
            plans.len() < 1024,
            "prepared snapshot credit must bound admission"
        );
    }
    assert!(!plans.is_empty());
    assert!(service.reporting.prepared_bytes.load(Ordering::Acquire) <= MAX_REPORT_BYTES);
    assert_eq!(service.dispatcher.in_flight(), 0);
    drop(plans);
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
    let plan = service
        .prepare_csp_report(handle, &v, Instant::now())
        .unwrap();
    drop(plan);
    v.original_policy_units = vec![0xd800; MAX_REPORT_BYTES];
    assert!(matches!(
        service.prepare_csp_report(handle, &v, Instant::now()),
        Err(ReportError::Limit)
    ));
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
}

#[test]
fn coep_saved_request_settings_real_main_hops_preserve_retirement_and_normative_report_order() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,Destination,api::{ApiRequest,ApiRequestInit,ApiRequestInput}};
    // Six whole native request cases; actual installed Window retirement is separate.
    for (enforced,resource_corp,endpoint,expected_reports,blocked) in [
        (EmbedderPolicy::UnsafeNone,"",true,1,false),
        (EmbedderPolicy::RequireCorp,"",true,2,true),
        (EmbedderPolicy::Credentialless,"",true,1,false),
        (EmbedderPolicy::RequireCorp,"same-origin",true,0,true),
        (EmbedderPolicy::RequireCorp,"cross-origin",true,0,false),
        (EmbedderPolicy::UnsafeNone,"",false,1,false),
    ] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
        let (tx,rx)=mpsc::channel();
        let network_reports=if endpoint {expected_reports}else{0};
        let peer=thread::spawn(move|| {
            for index in 0..1+network_reports {
                let (mut stream,_)=listener.accept().unwrap();let request=read_request(&mut stream);
                tx.send(request).unwrap();
                if index==0 {
                    let corp=if resource_corp.is_empty(){String::new()}else{format!("Cross-Origin-Resource-Policy: {resource_corp}\r\n")};
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\n{corp}Content-Length: 4\r\nConnection: close\r\n\r\ntrue").as_bytes()).unwrap();
                } else {stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();}
            }
        });
        let document=HttpUrl::parse(&format!("http://localhost:{port}/document")).unwrap();
        let mut client=receiving_client(&document,&format!("http://localhost:{port}/creation#secret"));
        client.context.security.embedder_policy=enforced;
        let container=EmbedderPolicyContainer {value:enforced,reporting_endpoint:endpoint.then(||"coep".into()),
            report_only_value:EmbedderPolicy::RequireCorp,report_only_reporting_endpoint:endpoint.then(||"coep".into())};
        let mut service=NetworkService::new(DispatchLimits::default());
        // Deliberately retain a default endpoint: absent COEP endpoint must not use it.
        let handle=service.register_reporting_client_response(&client,&document,&headers(r#"coep="/report", default="/wrong-default""#)).unwrap();
        let settings=service.prepare_coep_request_reporting(handle,&client,&container).unwrap();
        let mut init=ApiRequestInit::default();init.mode=Some("no-cors".into());init.credentials=Some("include".into());
        let request=ApiRequest::new(ApiRequestInput::Url(&format!("http://127.0.0.1:{port}/resource#private").encode_utf16().collect::<Vec<_>>()),init,&document,client.origin.clone()).unwrap();
        let id=service.start_subresource_with_coep_reporting(request,client.context.clone(),Destination::Script,settings).unwrap();
        assert!(service.forget_reporting_document(handle));
        let deadline=Instant::now()+Duration::from_secs(15);let mut reports=Vec::new();let mut finished=None;let mut deliveries=Vec::new();
        while Instant::now()<deadline && (finished.is_none()||service.has_pending_reporting_io()) {
            for event in service.step(31,Instant::now()) {
                match event {
                    ServiceEvent::CoepReport{id:found,document:original,report}=>{assert_eq!(found,id);assert_eq!(original,handle);reports.push(report.unwrap());},
                    ServiceEvent::SubresourceFinished{id:found,response}=>{assert_eq!(found,id);assert_eq!(response.body,b"true");finished=Some(false);},
                    ServiceEvent::SubresourceFailed{id:found,failure}=>{assert_eq!(found,id);assert_eq!(failure.error,crate::api::ApiError::Network(FetchError::CrossOriginResourcePolicy));finished=Some(true);},
                    ServiceEvent::ReportFinished(receipt)=>deliveries.push(receipt),_=>{}
                }
            }
            thread::yield_now();
        }
        assert_eq!(finished,Some(blocked));assert_eq!(reports.len(),expected_reports);assert_eq!(deliveries.len(),network_reports);
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        assert!(service.reporting.document(handle).is_err());
        for (index,report) in reports.iter().enumerate() {
            let body:BTreeMap<_,_>=report.body.iter().map(|(k,v)|(String::from_utf16(k).unwrap(),String::from_utf16(v).unwrap())).collect();
            assert_eq!(body["disposition"],if index==0 {"reporting"}else{"enforce"});
            assert_eq!(body["blockedURL"],format!("http://127.0.0.1:{port}/resource"));assert_eq!(body["destination"],"script");
            assert_eq!(String::from_utf16(&report.report_url).unwrap(),format!("http://localhost:{port}/creation"));
            if endpoint {assert!(report.delivery.is_ok());}else{assert_eq!(report.delivery,Err(ReportError::NoEndpoint));}
        }
        let get=rx.recv_timeout(Duration::from_secs(10)).unwrap();assert!(get.starts_with("GET /resource HTTP/1.1"));
        for _ in 0..network_reports {let post=rx.recv_timeout(Duration::from_secs(10)).unwrap();assert!(post.starts_with("POST /report HTTP/1.1"));assert!(post.contains("application/reports+json"));assert!(post.contains("\"type\":\"coep\""));assert!(!post.contains("#secret"));}
        peer.join().unwrap();
    }
}
#[test]
fn coep_request_settings_reject_cross_service_and_client_mismatch_and_release_credit() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,api::{ApiRequest,ApiRequestInit,ApiRequestInput}};
    let url=HttpUrl::parse("https://original.test/document").unwrap();let client=receiving_client(&url,"https://original.test/creation");
    let mut service=NetworkService::new(DispatchLimits::default());let handle=service.register_reporting_initial_client(&client).unwrap();
    let container=EmbedderPolicyContainer::default();
    let mut wrong=container.clone();wrong.value=EmbedderPolicy::Credentialless;
    assert!(matches!(service.prepare_coep_request_reporting(handle,&client,&wrong),Err(ReportError::InvalidReport)));
    let settings=service.prepare_coep_request_reporting(handle,&client,&container).unwrap();assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)>0);
    let mut other=NetworkService::new(DispatchLimits::default());
    let request=ApiRequest::new(ApiRequestInput::Url(&"https://resource.test/script".encode_utf16().collect::<Vec<_>>()),ApiRequestInit::default(),&url,client.origin.clone()).unwrap();
    assert!(matches!(other.start_subresource_with_coep_reporting(request,client.context.clone(),crate::Destination::Script,settings),Err(ReportError::StaleDocument)));
    assert_eq!(other.dispatcher.in_flight(),0);assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let settings=service.prepare_coep_request_reporting(handle,&client,&container).unwrap();drop(settings);assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    service.forget_reporting_document(handle);assert!(matches!(service.prepare_coep_request_reporting(handle,&client,&container),Err(ReportError::StaleDocument)));
}

#[test]
fn coep_buffered_fresh_cache_uses_real_selected_response_and_original_report_only_settings() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,api::{ApiRequest,ApiRequestInit,ApiRequestInput}};
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
    let (tx,rx)=mpsc::channel();let peer=thread::spawn(move||{
        let (mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=600\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntrue").unwrap();
    });
    let document=HttpUrl::parse(&format!("http://localhost:{port}/document")).unwrap();let client=receiving_client(&document,&document.serialize());
    let container=EmbedderPolicyContainer{report_only_value:EmbedderPolicy::RequireCorp,..Default::default()};
    let mut service=NetworkService::new(DispatchLimits::default());let handle=service.register_reporting_initial_client(&client).unwrap();
    for phase in 0..2 {
        let settings=service.prepare_coep_request_reporting(handle,&client,&container).unwrap();
        let mut init=ApiRequestInit::default();init.mode=Some("no-cors".into());init.credentials=Some("include".into());
        let input=format!("http://127.0.0.1:{port}/cached").encode_utf16().collect::<Vec<_>>();
        let request=ApiRequest::new(ApiRequestInput::Url(&input),init,&document,client.origin.clone()).unwrap();
        let id=service.start_at_with_coep_reporting(request,client.context.clone(),Instant::now(),settings).unwrap();
        if phase==1 {assert_eq!(service.dispatcher.in_flight(),0,"fresh cache must not submit a request");}
        let mut finished=false;let mut reports=0;let deadline=Instant::now()+Duration::from_secs(15);
        while !finished && Instant::now()<deadline {
            for event in service.step(4096,Instant::now()) {
                match event {
                    ServiceEvent::CoepReport{id:found,document:original,report}=>{assert_eq!(found,id);assert_eq!(original,handle);let receipt=report.unwrap();assert_eq!(receipt.delivery,Err(ReportError::NoEndpoint));reports+=1;},
                    ServiceEvent::Finished{id:found,result}=>{assert_eq!(found,id);let response=result.unwrap();assert_eq!(response.response_type,crate::api::ApiResponseType::Opaque);assert_eq!(response.status,0);assert!(response.url.is_empty());finished=true;},_=>{}
                }
            }
            thread::yield_now();
        }
        assert!(finished);assert_eq!(reports,1);assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    }
    assert!(rx.recv_timeout(Duration::from_secs(10)).unwrap().starts_with("GET /cached HTTP/1.1"));peer.join().unwrap();
}

#[test]
fn coep_original_owner_prepared_inheritance_reports_preserve_retirement_enforcement_and_real_post() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,EmbedderPolicyOwner};
    // Native owner-check/report boundary; not an installed worker constructor.
    for owner in [EmbedderPolicyOwner::NestedNavigation,EmbedderPolicyOwner::DedicatedWorker] {
        for parent_value in [EmbedderPolicy::UnsafeNone,EmbedderPolicy::RequireCorp,EmbedderPolicy::Credentialless] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
            let count=if parent_value==EmbedderPolicy::UnsafeNone {1}else{2};let (tx,rx)=mpsc::channel();
            let peer=thread::spawn(move||{
                for _ in 0..count {let (mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();}
            });
            let url=HttpUrl::parse(&format!("http://127.0.0.1:{port}/parent")).unwrap();let mut client=receiving_client(&url,&format!("http://127.0.0.1:{port}/original#private"));client.context.security.embedder_policy=parent_value;
            let parent=EmbedderPolicyContainer {value:parent_value,reporting_endpoint:Some("parent".into()),report_only_value:EmbedderPolicy::RequireCorp,report_only_reporting_endpoint:Some("parent".into())};
            let mut service=NetworkService::new(DispatchLimits::default());let handle=service.register_reporting_client_response(&client,&url,&headers(r#"parent="/report""#)).unwrap();
            let plan=service.prepare_coep_inheritance_reporting(handle,&client,&parent).unwrap();assert!(service.forget_reporting_document(handle));
            let child=EmbedderPolicyContainer::from_response(&HeaderList::default(),true);
            let response_urls=[ParsedUrl::parse("https://user:secret@child.test/first#private").unwrap(),ParsedUrl::parse("https://child.test/redirected").unwrap()];
            let queued_at=Instant::now();let reports=service.queue_coep_inheritance_reports(plan,&child,owner,&response_urls,queued_at,queued_at+Duration::from_secs(2)).unwrap();
            assert_eq!(reports.document,handle);assert_eq!(reports.check.allowed,parent_value==EmbedderPolicy::UnsafeNone);assert!(reports.check.report_only_violation);assert_eq!(reports.check.enforced_violation,parent_value!=EmbedderPolicy::UnsafeNone);
            assert_eq!(reports.reports.len(),count);
            for (index,report) in reports.reports.into_iter().enumerate() {let report=report.unwrap();assert!(report.delivery.is_ok());let fields:BTreeMap<_,_>=report.body.into_iter().map(|(k,v)|(String::from_utf16(&k).unwrap(),String::from_utf16(&v).unwrap())).collect();assert_eq!(fields["blockedURL"],"https://child.test/first");assert_eq!(fields["disposition"],if index==0 {"reporting"}else{"enforce"});assert_eq!(fields["type"],if owner==EmbedderPolicyOwner::NestedNavigation {"navigation"}else{"worker initialization"});}
            assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);let finished=run(&mut service,count);assert!(finished.iter().all(|report|report.document==handle));
            for _ in 0..count {let wire=rx.recv_timeout(Duration::from_secs(10)).unwrap();assert!(wire.starts_with("POST /report HTTP/1.1"));assert!(wire.contains("\"age\":2000"));assert!(!wire.contains("secret"));assert!(!wire.contains("redirected"));}
            assert!(service.reporting.document(handle).is_err());peer.join().unwrap();
        }
    }
}
#[test]
fn coep_owner_settings_noninheriting_types_no_endpoint_and_cross_service_remain_distinct_from_fresh_global_authority() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,EmbedderPolicyOwner};
    let url=HttpUrl::parse("https://parent.test/current").unwrap();let mut client=receiving_client(&url,"https://parent.test/creation");client.context.security.embedder_policy=EmbedderPolicy::RequireCorp;
    let parent=EmbedderPolicyContainer{value:EmbedderPolicy::RequireCorp,report_only_value:EmbedderPolicy::Credentialless,..Default::default()};let child=EmbedderPolicyContainer::default();
    let mut service=NetworkService::new(DispatchLimits::default());let handle=service.register_reporting_initial_client(&client).unwrap();
    for owner in [EmbedderPolicyOwner::TopLevelNavigation,EmbedderPolicyOwner::SharedWorker,EmbedderPolicyOwner::ServiceWorker,EmbedderPolicyOwner::Worklet] {
        let plan=service.prepare_coep_inheritance_reporting(handle,&client,&parent).unwrap();let now=Instant::now();let reports=service.queue_coep_inheritance_reports(plan,&child,owner,&[],now,now).unwrap();assert!(reports.check.allowed);assert!(!reports.check.report_only_violation);assert!(!reports.check.enforced_violation);assert!(reports.reports.is_empty());
    }
    let plan=service.prepare_coep_inheritance_reporting(handle,&client,&parent).unwrap();let mut other=NetworkService::new(DispatchLimits::default());let now=Instant::now();assert!(matches!(other.queue_coep_inheritance_reports(plan,&child,EmbedderPolicyOwner::NestedNavigation,&[],now,now),Err(ReportError::StaleDocument)));assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let plan=service.prepare_coep_inheritance_reporting(handle,&client,&parent).unwrap();service.forget_reporting_document(handle);let urls=[ParsedUrl::parse("https://child.test/document").unwrap()];let reports=service.queue_coep_inheritance_reports(plan,&child,EmbedderPolicyOwner::NestedNavigation,&urls,now,now).unwrap();assert!(!reports.check.allowed);assert_eq!(reports.reports.len(),2);for report in reports.reports {assert_eq!(report.unwrap().delivery,Err(ReportError::NoEndpoint));}
    assert_eq!(service.dispatcher.in_flight(),0);assert!(service.reporting.document(handle).is_err());assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
}
#[test]
fn coep_graph_dependency_after_retirement_preserves_original_settings_and_real_posts() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,Destination,api::{ApiRequest,ApiRequestInit,ApiRequestInput}};
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
    let (tx,rx)=mpsc::channel();
    let peer=thread::spawn(move|| {
        for _ in 0..4 {
            let (mut stream,_)=listener.accept().unwrap();let text=read_request(&mut stream);
            let get=text.starts_with("GET ");tx.send(text).unwrap();
            stream.write_all(if get {b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntrue".as_slice()}
                else {b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()}).unwrap();
        }
    });
    let url=HttpUrl::parse(&format!("http://localhost:{port}/document")).unwrap();
    let client=receiving_client(&url,&format!("http://localhost:{port}/creation#private"));
    let container=EmbedderPolicyContainer {value:EmbedderPolicy::UnsafeNone,reporting_endpoint:None,
        report_only_value:EmbedderPolicy::RequireCorp,report_only_reporting_endpoint:Some("coep".into())};
    let mut service=NetworkService::new(DispatchLimits::default());
    let handle=service.register_reporting_client_response(&client,&url,&headers(r#"coep="/report""#)).unwrap();
    let mut graph=service.prepare_coep_graph_reporting(handle,&client,&container).unwrap();let graph_id=graph.id();
    let initial_credit=service.reporting.prepared_bytes.load(Ordering::Acquire);
    assert!(initial_credit>0);assert!(service.forget_reporting_document(handle));
    for name in ["root","dependency"] {
        // The second dependency is discovered only after actual first response
        // completion; this is native producer coverage, not installed JS parsing.
        let plan=service.issue_coep_graph_request_settings(&mut graph,graph_id).unwrap();
        let mut init=ApiRequestInit::default();init.mode=Some("no-cors".into());
        let request=ApiRequest::new(ApiRequestInput::Url(&format!("http://127.0.0.1:{port}/{name}").encode_utf16().collect::<Vec<_>>()),init,&url,client.origin.clone()).unwrap();
        let id=service.start_subresource_with_coep_reporting(request,client.context.clone(),Destination::Script,plan).unwrap();
        let deadline=Instant::now()+Duration::from_secs(15);let mut finished=false;let mut reports=0;let mut delivered=0;
        while Instant::now()<deadline && (!finished||service.has_pending_reporting_io()) {
            for event in service.step(31,Instant::now()) {match event {
                ServiceEvent::SubresourceFinished{id:found,response}=>{assert_eq!(found,id);assert_eq!(response.body,b"true");finished=true;},
                ServiceEvent::CoepReport{id:found,document,report}=>{assert_eq!(found,id);assert_eq!(document,handle);let report=report.unwrap();
                    assert_eq!(String::from_utf16(&report.report_url).unwrap(),format!("http://localhost:{port}/creation"));assert!(report.delivery.is_ok());reports+=1;},
                ServiceEvent::ReportFinished(receipt)=>{assert_eq!(receipt.document,handle);delivered+=1;},
                ServiceEvent::SubresourceFailed{failure,..}=>panic!("genuine graph request failed: {:?}",failure.error),_=>{}
            }}
            thread::yield_now();
        }
        assert!(finished);assert_eq!(reports,1);assert_eq!(delivered,1);
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),initial_credit);
        assert!(rx.recv_timeout(Duration::from_secs(10)).unwrap().starts_with(&format!("GET /{name} HTTP/1.1")));
        let post=rx.recv_timeout(Duration::from_secs(10)).unwrap();assert!(post.starts_with("POST /report HTTP/1.1"));assert!(post.contains("application/reports+json"));assert!(!post.contains("#private"));
    }
    service.release_coep_graph_reporting(graph,graph_id).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    assert!(service.reporting.document(handle).is_err());peer.join().unwrap();
}

#[test]
fn coep_graph_identity_quota_release_and_fresh_global_endpoint_isolation() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer};
    let url=HttpUrl::parse("https://original.test/document").unwrap();let client=receiving_client(&url,"https://original.test/creation");
    let mut service=NetworkService::new(DispatchLimits::default());
    let handle=service.register_reporting_client_response(&client,&url,&headers(r#"coep="/report""#)).unwrap();
    let container=EmbedderPolicyContainer {value:EmbedderPolicy::UnsafeNone,reporting_endpoint:None,
        report_only_value:EmbedderPolicy::RequireCorp,report_only_reporting_endpoint:Some("coep".into())};
    let mut graph=service.prepare_coep_graph_reporting(handle,&client,&container).unwrap();let id=graph.id();
    let other_graph=service.prepare_coep_graph_reporting(handle,&client,&container).unwrap();let wrong=other_graph.id();
    let before=service.reporting.prepared_bytes.load(Ordering::Acquire);
    assert!(matches!(service.issue_coep_graph_request_settings(&mut graph,wrong),Err(ReportError::StaleDocument)));
    let mut other=NetworkService::new(DispatchLimits::default());
    assert!(matches!(other.issue_coep_graph_request_settings(&mut graph,id),Err(ReportError::StaleDocument)));
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),before);
    service.release_coep_graph_reporting(other_graph,wrong).unwrap();
    assert!(service.forget_reporting_document(handle));
    assert!(matches!(service.prepare_coep_graph_reporting(handle,&client,&container),Err(ReportError::StaleDocument)));
    // A fresh authentic global retains its own initial empty endpoint list.
    let fresh=service.register_reporting_initial_client(&client).unwrap();let now=Instant::now();
    let report=service.generate_coep_report(fresh,CoepReportBody::Corp {response_url_list:&[url.as_parsed().clone()],
        disposition:CoepReportDisposition::Reporting,destination:"script"},&"coep".encode_utf16().collect::<Vec<_>>(),now,now).unwrap();
    assert_eq!(report.delivery,Err(ReportError::NoEndpoint));assert_eq!(service.dispatcher.in_flight(),0);assert_eq!(other.dispatcher.in_flight(),0);
    let mut loans=Vec::new();
    loop {match service.issue_coep_graph_request_settings(&mut graph,id) {
        Ok(plan)=>loans.push(plan),Err(ReportError::Limit)=>break,Err(error)=>panic!("unexpected loan error: {error:?}"),
    }}
    assert!(!loans.is_empty());assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)<=MAX_REPORT_BYTES);
    service.release_coep_graph_reporting(graph,id).unwrap();assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)>0);
    drop(loans);assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
}

#[test]
fn coep_streamed_head_credit_and_retired_original_reporting_settings() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,api::{ApiRequest,ApiRequestInit,ApiRequestInput}};
    for (same_origin,enforced,reports_expected,blocked) in [
        (true,EmbedderPolicy::RequireCorp,0,false),
        (false,EmbedderPolicy::UnsafeNone,1,false),
        (false,EmbedderPolicy::RequireCorp,2,true),
    ] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
        let (tx,rx)=mpsc::channel();let peer=thread::spawn(move|| {
            let (mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntrue").unwrap();
        });
        let document=HttpUrl::parse(&format!("http://localhost:{port}/document")).unwrap();
        let mut client=receiving_client(&document,&document.serialize());client.context.security.embedder_policy=enforced;
        let container=EmbedderPolicyContainer {value:enforced,report_only_value:EmbedderPolicy::RequireCorp,..Default::default()};
        let mut service=NetworkService::new(DispatchLimits::default());let handle=service.register_reporting_initial_client(&client).unwrap();
        let settings=service.prepare_coep_request_reporting(handle,&client,&container).unwrap();
        let mut init=ApiRequestInit::default();init.mode=Some("no-cors".into());init.credentials=Some("include".into());
        let target=format!("http://{}:{port}/script",if same_origin {"localhost"}else{"127.0.0.1"}).encode_utf16().collect::<Vec<_>>();
        let request=ApiRequest::new(ApiRequestInput::Url(&target),init,&document,client.origin.clone()).unwrap();
        let id=service.start_stream_with_coep_reporting(request,client.context.clone(),0,Instant::now(),settings).unwrap();
        assert!(service.forget_reporting_document(handle));let deadline=Instant::now()+Duration::from_secs(15);
        let mut finished=false;let mut heads=0;let mut reports=0;let mut bytes=Vec::new();
        while !finished && Instant::now()<deadline {
            for event in service.step(31,Instant::now()) {match event {
                ServiceEvent::CoepReport{id:found,document:original,report}=>{assert_eq!(found,id);assert_eq!(original,handle);assert_eq!(report.unwrap().delivery,Err(ReportError::NoEndpoint));reports+=1;},
                ServiceEvent::ResponseHead{id:found,head}=>{assert_eq!(found,id);heads+=1;assert_eq!(head.status,if same_origin{200}else{0});if same_origin{assert!(service.grant_credit(id,4));}},
                ServiceEvent::BodyChunk{id:found,bytes:chunk}=>{assert_eq!(found,id);assert!(same_origin);bytes.extend(chunk);},
                ServiceEvent::StreamFinished{id:found}=>{assert_eq!(found,id);assert!(!blocked);finished=true;},
                ServiceEvent::StreamFailed{id:found,error}=>{assert_eq!(found,id);assert!(blocked);assert_eq!(error,crate::api::ApiError::Network(FetchError::CrossOriginResourcePolicy));finished=true;},_=>{}
            }}thread::yield_now();
        }
        assert!(finished);assert_eq!(heads,usize::from(!blocked));assert_eq!(reports,reports_expected);
        assert_eq!(bytes,if same_origin {b"true".to_vec()}else{Vec::new()});
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        assert!(!service.has_pending_reporting_io());assert!(service.reporting.document(handle).is_err());
        assert!(rx.recv_timeout(Duration::from_secs(10)).unwrap().starts_with("GET /script HTTP/1.1"));peer.join().unwrap();
    }
}

#[test]
fn coep_revalidation_reports_merged_response_once_and_retains_actual_conditional_requests() {
    use crate::{EmbedderPolicy,EmbedderPolicyContainer,api::{ApiRequest,ApiRequestInit,ApiRequestInput}};
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
    let (tx,rx)=mpsc::channel();let peer=thread::spawn(move|| {
        for phase in 0..3 {
            let (mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();
            let response=match phase {
                0=>"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=0\r\nETag: \"v1\"\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntrue",
                1=>"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=0\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",
                _=>"HTTP/1.1 304 Not Modified\r\nCross-Origin-Resource-Policy: cross-origin\r\nCache-Control: max-age=600\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",
            };stream.write_all(response.as_bytes()).unwrap();
        }
    });
    let document=HttpUrl::parse(&format!("http://localhost:{port}/document")).unwrap();let mut client=receiving_client(&document,&document.serialize());
    let container=EmbedderPolicyContainer {report_only_value:EmbedderPolicy::RequireCorp,..Default::default()};
    let mut service=NetworkService::new(DispatchLimits::default());let handle=service.register_reporting_initial_client(&client).unwrap();
    for phase in 0..3 {
        client.context.now_unix=1800000000+phase;
        let settings=service.prepare_coep_request_reporting(handle,&client,&container).unwrap();
        let mut init=ApiRequestInit::default();init.mode=Some("no-cors".into());init.credentials=Some("include".into());
        let target=format!("http://127.0.0.1:{port}/cached").encode_utf16().collect::<Vec<_>>();
        let request=ApiRequest::new(ApiRequestInput::Url(&target),init,&document,client.origin.clone()).unwrap();
        let id=service.start_at_with_coep_reporting(request,client.context.clone(),Instant::now(),settings).unwrap();
        let deadline=Instant::now()+Duration::from_secs(15);let mut finished=false;let mut reports=0;
        while !finished && Instant::now()<deadline {
            for event in service.step(4096,Instant::now()) {match event {
                ServiceEvent::CoepReport{id:found,report,..}=>{assert_eq!(found,id);assert_eq!(report.unwrap().delivery,Err(ReportError::NoEndpoint));reports+=1;},
                ServiceEvent::Finished{id:found,result}=>{assert_eq!(found,id);let response=result.unwrap();assert_eq!(response.response_type,crate::api::ApiResponseType::Opaque);assert_eq!(response.status,0);finished=true;},_=>{}
            }}thread::yield_now();
        }
        assert!(finished);assert_eq!(reports,usize::from(phase<2));
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        let wire=rx.recv_timeout(Duration::from_secs(10)).unwrap();assert!(wire.starts_with("GET /cached HTTP/1.1"));
        if phase>0 {assert!(wire.to_ascii_lowercase().contains("if-none-match: \"v1\""));}
    }
    peer.join().unwrap();
}
