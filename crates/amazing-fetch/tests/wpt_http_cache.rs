//! Native expectations selected from pinned WPT fetch/http-cache/ sources.
//! The source revision, hashes, and selection limits are in wpt/http_cache/PROVENANCE.md.
use amazing_fetch::api::{ApiHeaders, ApiRequest, ApiRequestInit, ApiRequestInput, HeaderGuard};
use amazing_fetch::cache::{
    CacheKind, CacheLimits, CacheLookup, CacheRequest, CacheResponse, CacheStore,
};
use amazing_fetch::service::{ClientContext, NetworkService, ServiceEvent};
use amazing_fetch::HeaderList;
use amazing_fetch::HttpUrl;
use amazing_net::dispatch::DispatchLimits;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn headers(fields: &[(&str, &str)]) -> HeaderList {
    let mut headers = HeaderList::default();
    for &(name, value) in fields {
        headers.append(name, value).unwrap();
    }
    headers
}
fn request<'a>(uri: &'a str, fields: &'a HeaderList) -> CacheRequest<'a> {
    CacheRequest {
        uri,
        method: "GET",
        headers: fields,
        cookie: "",
    }
}
fn response(fields: &[(&str, &str)], body: &[u8]) -> CacheResponse {
    CacheResponse {
        request_includes_credentials: true,
        status: 200,
        headers: headers(fields),
        body: body.to_vec(),
    }
}
fn cache() -> CacheStore {
    CacheStore::new(CacheKind::Private, CacheLimits::default()).unwrap()
}
fn fresh(cache: &CacheStore, request: &CacheRequest<'_>, now: i64) -> bool {
    matches!(cache.lookup(request, now).unwrap(), CacheLookup::Fresh(_))
}
fn validator(
    cache: &CacheStore,
    request: &CacheRequest<'_>,
    now: i64,
    name: &str,
) -> Option<String> {
    match cache.lookup(request, now).unwrap() {
        CacheLookup::Validate { headers, .. } => headers.get(name),
        _ => None,
    }
}

#[test]
fn wpt_freshness_and_cache_control_selection() {
    let empty = headers(&[]);
    // freshness.any.js: future/past Expires, max-age precedence, Age, no-store, no-cache.
    let cases = [
        (
            "future Expires",
            vec![("Expires", "Thu, 01 Jan 1970 00:03:20 GMT")],
            true,
        ),
        (
            "past Expires",
            vec![("Expires", "Thu, 01 Jan 1970 00:01:00 GMT")],
            false,
        ),
        (
            "max-age overrides past Expires",
            vec![
                ("Cache-Control", "max-age=60"),
                ("Expires", "Thu, 01 Jan 1970 00:01:00 GMT"),
            ],
            true,
        ),
        (
            "max-age=0 overrides future Expires",
            vec![
                ("Cache-Control", "max-age=0"),
                ("Expires", "Thu, 01 Jan 1970 00:03:20 GMT"),
            ],
            false,
        ),
        (
            "Age exceeds lifetime",
            vec![("Cache-Control", "max-age=60"), ("Age", "120")],
            false,
        ),
        (
            "no-cache requires validation",
            vec![
                ("Cache-Control", "no-cache, max-age=60"),
                ("ETag", "\"v1\""),
            ],
            false,
        ),
    ];
    for (name, fields, expected) in cases {
        let mut cache = cache();
        let uri = format!("https://example.test/{name}");
        cache
            .store(&request(&uri, &empty), response(&fields, b"body"), 100, 100)
            .unwrap();
        assert_eq!(
            fresh(&cache, &request(&uri, &empty), 101),
            expected,
            "{name}"
        );
    }
    let mut cache = cache();
    assert!(!cache
        .store(
            &request("https://example.test/no-store", &empty),
            response(&[("Cache-Control", "no-store, max-age=60")], b"body"),
            100,
            100
        )
        .unwrap());
    assert_eq!(cache.len(), 0, "WPT no-store response");

    // cc-request.any.js: max-age and no-cache/no-store request directives.
    let uri = "https://example.test/cc-request";
    cache
        .store(
            &request(uri, &empty),
            response(
                &[("Cache-Control", "max-age=3600"), ("ETag", "\"cc\"")],
                b"body",
            ),
            100,
            100,
        )
        .unwrap();
    assert!(!fresh(
        &cache,
        &request(uri, &headers(&[("Cache-Control", "max-age=0")])),
        102
    ));
    assert_eq!(
        validator(
            &cache,
            &request(uri, &headers(&[("Cache-Control", "no-cache")])),
            101,
            "if-none-match"
        )
        .as_deref(),
        Some("\"cc\"")
    );
    assert!(matches!(
        cache
            .lookup(
                &request(uri, &headers(&[("Cache-Control", "no-store")])),
                101
            )
            .unwrap(),
        CacheLookup::Miss
    ));
}

#[test]
fn wpt_vary_selection() {
    // vary.any.js: matching field, changed/omitted field, unlisted field, and Vary: *.
    let uri = "https://example.test/vary";
    let en = headers(&[("Foo", "1"), ("Bar", "en")]);
    let fr = headers(&[("Foo", "1"), ("Bar", "fr")]);
    let empty = headers(&[]);
    let mut cache = cache();
    cache
        .store(
            &request(uri, &en),
            response(
                &[("Cache-Control", "max-age=60"), ("Vary", "Foo, Bar")],
                b"en",
            ),
            100,
            100,
        )
        .unwrap();
    assert!(fresh(&cache, &request(uri, &en), 101), "matching Vary");
    assert!(
        !fresh(&cache, &request(uri, &fr), 101),
        "changed Vary field"
    );
    assert!(
        !fresh(&cache, &request(uri, &empty), 101),
        "omitted Vary field"
    );
    let unlisted = headers(&[("Foo", "1"), ("Bar", "en"), ("Other", "x")]);
    assert!(
        fresh(&cache, &request(uri, &unlisted), 101),
        "unlisted field ignored"
    );
    assert!(!cache
        .store(
            &request("https://example.test/star", &empty),
            response(&[("Cache-Control", "max-age=60"), ("Vary", "*")], b"x"),
            100,
            100
        )
        .unwrap());
}

#[test]
fn wpt_etag_and_last_modified_304_selection() {
    // 304-update.any.js: conditional request and replacement of returned/stored metadata.
    for (name, validator_name, value, conditional) in [
        ("etag", "ETag", "\"v1\"", "if-none-match"),
        (
            "last-modified",
            "Last-Modified",
            "Thu, 01 Jan 1970 00:00:00 GMT",
            "if-modified-since",
        ),
    ] {
        let uri = format!("https://example.test/{name}");
        let empty = headers(&[]);
        let mut cache = cache();
        cache
            .store(
                &request(&uri, &empty),
                response(
                    &[
                        ("Cache-Control", "max-age=0"),
                        (validator_name, value),
                        ("Test-Header", "A"),
                    ],
                    b"original",
                ),
                100,
                100,
            )
            .unwrap();
        assert_eq!(
            validator(&cache, &request(&uri, &empty), 101, conditional).as_deref(),
            Some(value),
            "{name}"
        );
        let merged = cache
            .update_304(
                &request(&uri, &empty),
                &headers(&[
                    (validator_name, value),
                    ("Cache-Control", "max-age=60"),
                    ("Test-Header", "B"),
                ]),
                101,
                101,
            )
            .unwrap()
            .unwrap();
        assert_eq!(merged.body, b"original", "{name} body");
        assert_eq!(
            merged.headers.get("test-header").as_deref(),
            Some("B"),
            "{name} response metadata"
        );
        let cached = match cache.lookup(&request(&uri, &empty), 102).unwrap() {
            CacheLookup::Fresh(entry) => entry,
            _ => panic!("{name} metadata not stored"),
        };
        assert_eq!(
            cached.response.headers.get("test-header").as_deref(),
            Some("B")
        );
    }
}

#[test]
fn wpt_status_with_explicit_freshness() {
    // status.any.js: 13 response codes with fresh and stale representations.
    let empty = headers(&[]);
    for status in [
        200, 203, 204, 299, 400, 404, 410, 499, 500, 502, 503, 504, 599,
    ] {
        let uri = format!("https://example.test/status/{status}");
        let mut cache = cache();
        let mut reply = response(&[("Cache-Control", "max-age=60")], b"body");
        reply.status = status;
        assert!(
            cache
                .store(&request(&uri, &empty), reply, 100, 100)
                .unwrap(),
            "store {status}"
        );
        assert!(fresh(&cache, &request(&uri, &empty), 101), "fresh {status}");
        assert!(
            !fresh(&cache, &request(&uri, &empty), 161),
            "stale {status}"
        );
    }
}

#[test]
fn wpt_fetch_cache_mode_request_headers() {
    // cache-mode.any.js: the five specified Cache-Control/Pragma request cases.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/resource",
        listener.local_addr().unwrap().port()
    );
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut received = Vec::new();
            while !received.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                received.push(byte[0]);
                assert!(received.len() < 16 * 1024);
            }
            requests.push(String::from_utf8(received).unwrap().to_ascii_lowercase());
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nok",
                )
                .unwrap();
        }
        requests
    });
    let base = HttpUrl::parse(&url).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let cases = [
        ("no-cache", None, None),
        ("no-cache", Some("foo"), None),
        ("no-store", None, None),
        ("no-store", Some("foo"), None),
        ("no-store", None, Some("foo")),
    ];
    for (index, (mode, cc, pragma)) in cases.into_iter().enumerate() {
        let mut init = ApiRequestInit::default();
        init.cache = Some(mode.into());
        let mut fields = ApiHeaders::new(HeaderGuard::None);
        if let Some(cc) = cc {
            fields.append_string("Cache-Control", cc).unwrap();
        }
        if let Some(pragma) = pragma {
            fields.append_string("Pragma", pragma).unwrap();
        }
        init.headers = Some(fields);
        let target = format!("{url}?case={index}");
        let request = ApiRequest::new(
            ApiRequestInput::Url(&target.encode_utf16().collect::<Vec<_>>()),
            init,
            &base,
            base.origin().clone(),
        )
        .unwrap();
        let id = service.start(request, ClientContext::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut finished = false;
            for event in service.step(8192, Instant::now()) {
                if let ServiceEvent::Finished { id: found, result } = event {
                    if found == id {
                        assert!(result.is_ok(), "WPT cache-mode request {index}: {result:?}");
                        finished = true;
                    }
                }
            }
            if finished {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "WPT cache-mode request {index} timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let requests = server.join().unwrap();
    assert!(requests[0].contains("cache-control: max-age=0\r\n"));
    assert!(requests[1].contains("cache-control: foo\r\n"));
    assert!(requests[2].contains("cache-control: no-cache\r\n"));
    assert!(requests[2].contains("pragma: no-cache\r\n"));
    assert!(requests[3].contains("cache-control: foo\r\n"));
    assert!(requests[4].contains("pragma: foo\r\n"));
}

#[test]
fn wpt_successful_post_invalidates_cached_get() {
    // invalidate.any.js: a successful unsafe response invalidates the target URI.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/cached",
        listener.local_addr().unwrap().port()
    );
    let server = std::thread::spawn(move || {
        let mut methods = Vec::new();
        for index in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut received = Vec::new();
            while !received.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                received.push(byte[0]);
                assert!(received.len() < 16 * 1024);
            }
            methods.push(
                String::from_utf8(received)
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap()
                    .to_owned(),
            );
            let body = match index {
                0 => b"old".as_slice(),
                1 => b"ok".as_slice(),
                _ => b"new".as_slice(),
            };
            write!(stream, "HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(body).unwrap();
        }
        methods
    });
    let base = HttpUrl::parse(&url).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    for (method, expected) in [
        ("GET", b"old".as_slice()),
        ("POST", b"ok".as_slice()),
        ("GET", b"new".as_slice()),
    ] {
        let mut init = ApiRequestInit::default();
        init.method = Some(method.encode_utf16().collect());
        let input: Vec<_> = url.encode_utf16().collect();
        let request = ApiRequest::new(
            ApiRequestInput::Url(&input),
            init,
            &base,
            base.origin().clone(),
        )
        .unwrap();
        let id = service.start(request, ClientContext::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut reply = None;
            for event in service.step(8192, Instant::now()) {
                if let ServiceEvent::Finished { id: found, result } = event {
                    if found == id {
                        reply = Some(result);
                    }
                }
            }
            if let Some(reply) = reply {
                let mut reply = reply.unwrap();
                assert_eq!(reply.body.bytes().unwrap(), expected, "{method}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "WPT invalidate {method} timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert_eq!(server.join().unwrap(), ["GET", "POST", "GET"]);
}
