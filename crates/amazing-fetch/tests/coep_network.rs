//! Real native HTTP requests; these are not installed Window/worker proofs.
use amazing_fetch::{
    EmbedderPolicy, HttpUrl,
    api::{ApiError, ApiRequest, ApiRequestInit, ApiRequestInput},
    cookies::{CookieAccess, CookieSite},
    service::{ClientContext, NetworkService, ServiceEvent, ServiceId},
};
use amazing_fetch::cache::CacheStorage;
use amazing_net::dispatch::DispatchLimits;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn read_head(stream: &mut TcpStream) -> String {
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
    String::from_utf8(head).unwrap()
}
fn finish(
    service: &mut NetworkService,
    id: ServiceId,
) -> Result<amazing_fetch::api::ApiResponse, ApiError> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        for event in service.step(97, Instant::now()) {
            if let ServiceEvent::Finished { id: found, result } = event {
                assert_eq!(found, id);
                return result;
            }
        }
        thread::yield_now();
    }
    panic!("actual HTTP request did not finish");
}
fn api(target: &HttpUrl, origin: &HttpUrl, mode: &str, credentials: &str) -> ApiRequest {
    let mut init = ApiRequestInit::default();
    init.mode = Some(mode.into());
    init.credentials = Some(credentials.into());
    ApiRequest::new(
        ApiRequestInput::Url(&target.serialize().encode_utf16().collect::<Vec<_>>()),
        init,
        origin,
        origin.origin().clone(),
    )
    .unwrap()
}
#[test]
fn actual_http_coep_cookie_set_cookie_and_stored_transport_receipt_matrix() {
    for (policy, mode, credentials, same_origin, permitted) in [
        (
            EmbedderPolicy::UnsafeNone,
            "no-cors",
            "include",
            false,
            true,
        ),
        (
            EmbedderPolicy::RequireCorp,
            "no-cors",
            "include",
            false,
            true,
        ),
        (
            EmbedderPolicy::Credentialless,
            "no-cors",
            "include",
            false,
            false,
        ),
        (
            EmbedderPolicy::Credentialless,
            "no-cors",
            "include",
            true,
            true,
        ),
        (
            EmbedderPolicy::Credentialless,
            "no-cors",
            "omit",
            true,
            false,
        ),
        (
            EmbedderPolicy::Credentialless,
            "no-cors",
            "same-origin",
            false,
            false,
        ),
        (
            EmbedderPolicy::Credentialless,
            "cors",
            "include",
            false,
            true,
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = HttpUrl::parse(&format!(
            "http://{}/resource",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let origin = if same_origin {
            target.clone()
        } else {
            HttpUrl::parse("http://127.0.0.1:9/client").unwrap()
        };
        let (tx, rx) = mpsc::channel();
        let allowed_origin = origin.origin().serialize();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_head(&mut stream)).unwrap();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nCache-Control: max-age=60\r\nSet-Cookie: received=yes; Path=/; SameSite=Lax\r\nCross-Origin-Resource-Policy: cross-origin\r\nAccess-Control-Allow-Origin: {allowed_origin}\r\nAccess-Control-Allow-Credentials: true\r\nConnection: close\r\n\r\nimage").as_bytes()).unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut context = ClientContext {
            now_unix: 1_800_000_000,
            cookie_site: CookieSite::Origin(origin.origin().clone()),
            ..Default::default()
        };
        context.security.embedder_policy = policy;
        let access = CookieAccess {
            url: &target,
            site_for_cookies: context.cookie_site.clone(),
            http_api: true,
            top_level_navigation: false,
            method: "GET",
            partition_key: None,
        };
        service
            .cookies
            .set_cookie(
                b"seed=present; Path=/; SameSite=Lax",
                &access,
                context.now_unix,
            )
            .unwrap();
        let request = api(&target, &origin, mode, credentials);
        let observable_credentials = request.credentials;
        let id = service.start(request, context.clone()).unwrap();
        let received = finish(&mut service, id).unwrap();
        assert_eq!(received.credential_receipt().unwrap().includes_credentials(), permitted);
        assert_eq!(received.clone_response().unwrap().credential_receipt(), received.credential_receipt());
        let wire = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .to_ascii_lowercase();
        assert_eq!(
            wire.contains("cookie: seed=present"),
            permitted,
            "{policy:?}/{mode}/{credentials}/{same_origin}"
        );
        let stored_cookies = String::from_utf8(
            service
                .cookies
                .cookie_header(&access, context.now_unix)
                .unwrap(),
        )
        .unwrap();
        assert!(stored_cookies.contains("seed=present"));
        assert_eq!(stored_cookies.contains("received=yes"), permitted);
        assert_eq!(
            service
                .cache
                .storage()
                .get(0)
                .unwrap()
                .response
                .request_includes_credentials,
            permitted
        );
        assert_eq!(
            api(&target, &origin, mode, credentials).credentials,
            observable_credentials
        );
        peer.join().unwrap();
    }
}

#[test]
fn actual_credentialless_redirect_bounce_omits_credentials_at_cross_and_return_hops() {
    let first = TcpListener::bind("127.0.0.1:0").unwrap();
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = HttpUrl::parse(&format!("http://{}/start", first.local_addr().unwrap())).unwrap();
    let cross = format!("http://{}/cross", other.local_addr().unwrap());
    let final_url = origin.resolve("/final").unwrap().serialize();
    let (tx, rx) = mpsc::channel();
    let first_tx = tx.clone();
    let first_peer = thread::spawn(move || {
        let (mut stream, _) = first.accept().unwrap();
        first_tx.send(("initial", read_head(&mut stream))).unwrap();
        stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {cross}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        drop(stream);
        let (mut stream, _) = first.accept().unwrap();
        first_tx.send(("return", read_head(&mut stream))).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nSet-Cookie: bounce=forbidden;Path=/;SameSite=Lax\r\nConnection: close\r\n\r\nimage").unwrap();
    });
    let cross_peer = thread::spawn(move || {
        let (mut stream, _) = other.accept().unwrap();
        tx.send(("cross", read_head(&mut stream))).unwrap();
        stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {final_url}\r\nContent-Length: 0\r\nSet-Cookie: cross=forbidden;Path=/;SameSite=Lax\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let mut context = ClientContext {
        now_unix: 1_800_000_000,
        cookie_site: CookieSite::Origin(origin.origin().clone()),
        ..Default::default()
    };
    context.security.embedder_policy = EmbedderPolicy::Credentialless;
    let access = CookieAccess {
        url: &origin,
        site_for_cookies: context.cookie_site.clone(),
        http_api: true,
        top_level_navigation: false,
        method: "GET",
        partition_key: None,
    };
    service
        .cookies
        .set_cookie(
            b"seed=present;Path=/;SameSite=Lax",
            &access,
            context.now_unix,
        )
        .unwrap();
    let id = service
        .start(api(&origin, &origin, "no-cors", "include"), context.clone())
        .unwrap();
    assert!(finish(&mut service, id).is_ok());
    for _ in 0..3 {
        let (hop, wire) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            wire.to_ascii_lowercase().contains("cookie:"),
            hop == "initial",
            "{hop}"
        );
    }
    let cookies = String::from_utf8(
        service
            .cookies
            .cookie_header(&access, context.now_unix)
            .unwrap(),
    )
    .unwrap();
    assert!(cookies.contains("seed=present"));
    assert!(!cookies.contains("forbidden"));
    first_peer.join().unwrap();
    cross_peer.join().unwrap();
}

#[test]
fn actual_no_cors_missing_corp_applies_enforced_policy_with_real_response_receipt() {
    for policy in [
        EmbedderPolicy::UnsafeNone,
        EmbedderPolicy::RequireCorp,
        EmbedderPolicy::Credentialless,
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target =
            HttpUrl::parse(&format!("http://{}/image", listener.local_addr().unwrap())).unwrap();
        let origin = HttpUrl::parse("http://127.0.0.1:9/client").unwrap();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_head(&mut stream);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage").unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut context = ClientContext::default();
        context.security.embedder_policy = policy;
        let id = service
            .start(api(&target, &origin, "no-cors", "include"), context)
            .unwrap();
        let result = finish(&mut service, id);
        if policy == EmbedderPolicy::RequireCorp {
            assert!(matches!(
                result,
                Err(ApiError::Network(
                    amazing_fetch::FetchError::CrossOriginResourcePolicy
                ))
            ));
        } else {
            assert!(result.is_ok());
        }
        peer.join().unwrap();
    }
}

#[test]
fn actual_fresh_cache_applies_current_hop_credentials_without_losing_stored_receipt() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target =
        HttpUrl::parse(&format!("http://{}/image", listener.local_addr().unwrap())).unwrap();
    let origin = HttpUrl::parse("http://127.0.0.1:9/client").unwrap();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_head(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\nimage").unwrap();
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let context = ClientContext {
        now_unix: 1_800_000_000,
        ..Default::default()
    };
    let first = service
        .start(api(&target, &origin, "no-cors", "include"), context.clone())
        .unwrap();
    assert!(finish(&mut service, first).is_ok());
    peer.join().unwrap();
    assert!(
        service
            .cache
            .storage()
            .get(0)
            .unwrap()
            .response
            .request_includes_credentials
    );
    let mut credentialless = context;
    credentialless.security.embedder_policy = EmbedderPolicy::Credentialless;
    let cached = service
        .start(api(&target, &origin, "no-cors", "include"), credentialless)
        .unwrap();
    let cached_response = finish(&mut service, cached).unwrap();
    assert!(!cached_response.credential_receipt().unwrap().includes_credentials());
    assert!(
        service
            .cache
            .storage()
            .get(0)
            .unwrap()
            .response
            .request_includes_credentials
    );
    assert_eq!(service.dispatcher.in_flight(), 0);
}

#[test]
fn actual_credentialless_304_reuses_body_with_current_hop_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target =
        HttpUrl::parse(&format!("http://{}/image", listener.local_addr().unwrap())).unwrap();
    let origin = HttpUrl::parse("http://127.0.0.1:9/client").unwrap();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_head(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nCache-Control: max-age=0\r\nETag: \"image-v1\"\r\nConnection: close\r\n\r\nimage").unwrap();
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        let head = read_head(&mut stream).to_ascii_lowercase();
        assert!(head.contains("if-none-match: \"image-v1\""));
        assert!(!head.contains("cookie:"));
        stream.write_all(b"HTTP/1.1 304 Not Modified\r\nETag: \"image-v1\"\r\nCache-Control: max-age=60\r\nSet-Cookie: validation=forbidden;Path=/;SameSite=Lax\r\nConnection: close\r\n\r\n").unwrap();
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let mut context = ClientContext {
        now_unix: 1_800_000_000,
        ..Default::default()
    };
    let first = service
        .start(api(&target, &origin, "no-cors", "include"), context.clone())
        .unwrap();
    assert!(finish(&mut service, first).is_ok());
    assert!(
        service
            .cache
            .storage()
            .get(0)
            .unwrap()
            .response
            .request_includes_credentials
    );
    context.security.embedder_policy = EmbedderPolicy::Credentialless;
    let validated = service
        .start(api(&target, &origin, "no-cors", "include"), context.clone())
        .unwrap();
    let validated_response = finish(&mut service, validated).unwrap();
    assert!(!validated_response.credential_receipt().unwrap().includes_credentials());
    assert_eq!(
        service.cache.storage().get(0).unwrap().response.body,
        b"image"
    );
    let access = CookieAccess {
        url: &target,
        site_for_cookies: CookieSite::NoClient,
        http_api: true,
        top_level_navigation: false,
        method: "GET",
        partition_key: None,
    };
    assert!(
        service
            .cookies
            .cookie_header(&access, context.now_unix)
            .unwrap()
            .is_empty()
    );
    peer.join().unwrap();
}

#[test]
fn genuine_native_child_navigation_corp_uses_constructor_custody_and_actual_parent_client_policy() {
    for policy in [
        EmbedderPolicy::UnsafeNone,
        EmbedderPolicy::RequireCorp,
        EmbedderPolicy::Credentialless,
    ] {
        for explicit_corp in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let target =
                HttpUrl::parse(&format!("http://{}/child", listener.local_addr().unwrap()))
                    .unwrap();
            let parent_url = HttpUrl::parse("http://127.0.0.1:9/parent").unwrap();
            let peer = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                assert!(read_head(&mut stream).starts_with("GET /child HTTP/1.1"));
                let corp = if explicit_corp {
                    "Cross-Origin-Resource-Policy: cross-origin\r\n"
                } else {
                    ""
                };
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n{corp}Connection: close\r\n\r\nchild").as_bytes()).unwrap();
            });
            let parent = amazing_fetch::api::NavigableContext {
                top_level_origin: parent_url.origin().clone(),
                parent_origin: parent_url.origin().clone(),
                ancestor_origins: vec![parent_url.origin().clone()],
                sandbox_flags: 0,
                frame_csp: Default::default(),
                referrer: parent_url.serialize(),
                referrer_policy: "strict-origin-when-cross-origin".into(),
                now_unix: 1_800_000_000,
            };
            let request = ApiRequest::child_navigation(target, &parent).unwrap();
            let mut context = ClientContext {
                cookie_site: CookieSite::Origin(parent_url.origin().clone()),
                frame_ancestors: parent.ancestor_origins.clone(),
                now_unix: parent.now_unix,
                ..Default::default()
            };
            context.security.embedder_policy = policy;
            context.security.top_level_navigation = false;
            context.security.destination = amazing_fetch::Destination::Other;
            context.csp_destination = amazing_fetch::CspDestination::Frame;
            let mut service = NetworkService::new(DispatchLimits::default());
            let id = service.start(request, context).unwrap();
            let response = finish(&mut service, id);
            if explicit_corp || policy == EmbedderPolicy::UnsafeNone {
                assert!(response.is_ok());
            } else {
                assert_eq!(
                    response.err(),
                    Some(ApiError::Network(
                        amazing_fetch::FetchError::CrossOriginResourcePolicy
                    ))
                );
            }
            peer.join().unwrap();
        }
    }
}

#[test]
fn genuine_native_no_cors_data_scheme_keeps_basic_responses_under_every_enforced_coep_policy() {
    let base = HttpUrl::parse("https://receiving.test/").unwrap();
    for policy in [EmbedderPolicy::UnsafeNone, EmbedderPolicy::RequireCorp, EmbedderPolicy::Credentialless] {
        for input in ["data:application/json,%7B%7D", "data:text/html,hello", "data:text/plain,hello"] {
            let mut service = NetworkService::new(DispatchLimits::default());
            let mut init = ApiRequestInit::default();
            init.mode = Some("no-cors".into());
            let request = ApiRequest::new(ApiRequestInput::Url(&input.encode_utf16().collect::<Vec<_>>()),
                init, &base, base.origin().clone()).unwrap();
            let context = ClientContext { security:amazing_fetch::SecurityContext {
                embedder_policy:policy, ..Default::default()
            }, ..Default::default() };
            let id = service.start(request, context).unwrap();
            let mut response = finish(&mut service, id).unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.response_type, amazing_fetch::api::ApiResponseType::Basic);
            assert_eq!(response.url, input);
            assert_eq!(response.body.bytes().unwrap(), if input.contains("json") { b"{}".as_slice() } else { b"hello".as_slice() });
            assert!(!service.has_pending_output(), "local scheme never retains socket output");
        }
    }
}

#[test]
fn actual_304_without_matching_validator_cannot_replace_retained_body_or_credentials() {
    // RFC 9111 4.3.4: neither a mismatched strong validator nor a missing
    // validator selects this ETag-bearing representation for an update.
    for validator in ["ETag: \"different\"\r\n", ""] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = HttpUrl::parse(&format!("http://{}/image", listener.local_addr().unwrap())).unwrap();
        let origin = HttpUrl::parse("http://127.0.0.1:9/client").unwrap();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(read_head(&mut stream).starts_with("GET /image HTTP/1.1"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nCache-Control: max-age=0\r\nETag: \"image-v1\"\r\nConnection: close\r\n\r\nimage").unwrap();
            drop(stream);
            let (mut stream, _) = listener.accept().unwrap();
            let head = read_head(&mut stream).to_ascii_lowercase();
            assert!(head.contains("if-none-match: \"image-v1\""));
            assert!(!head.contains("cookie:"));
            stream.write_all(format!("HTTP/1.1 304 Not Modified\r\n{validator}Cache-Control: max-age=600\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut context = ClientContext { now_unix: 1_800_000_000, ..Default::default() };
        let first = service.start(api(&target, &origin, "no-cors", "include"), context.clone()).unwrap();
        assert!(finish(&mut service, first).is_ok());
        context.security.embedder_policy = EmbedderPolicy::Credentialless;
        let validated = service.start(api(&target, &origin, "no-cors", "include"), context).unwrap();
        assert_eq!(finish(&mut service, validated).unwrap_err(), ApiError::Network(amazing_fetch::FetchError::Transport));
        let retained = service.cache.storage().get(0).unwrap();
        assert_eq!(retained.response.body, b"image");
        assert!(retained.response.request_includes_credentials);
        assert_eq!(retained.response.headers.get("etag").as_deref(), Some("\"image-v1\""));
        assert_eq!(retained.response.headers.get("cache-control").as_deref(), Some("max-age=0"));
        peer.join().unwrap();
    }
}
