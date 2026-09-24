//! Pinned native network expectations from WPT fetch/api/ and xhr/.
//! The selected source files and ported Python handlers are recorded in
//! tests/wpt/fetch_xhr/PROVENANCE.md; JS bindings remain an adoption item.
#[path = "../../amazing-net/tests/support/wpt_server.rs"]
mod wpt_server;
use amazing_fetch::{
    api::{ApiError, ApiRequest, ApiRequestInit, ApiRequestInput, ApiResponse, BodyInput, NavigationRequestInit},
    service::{ClientContext, NetworkService, ServiceEvent, ServiceId},
    xhr::{Xhr, XhrEventKind, XhrResponseType, XhrResponseValue, XhrState, XhrTextDecoder},
    HttpUrl,
};
use amazing_net::dispatch::DispatchLimits;
use std::{
    thread,
    time::{Duration, Instant},
};
use wpt_server::WptServer;
fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn base(server: &WptServer) -> HttpUrl {
    HttpUrl::parse(&server.url("/page")).unwrap()
}
fn request(base: &HttpUrl, url: &str, init: ApiRequestInit) -> ApiRequest {
    ApiRequest::new(
        ApiRequestInput::Url(&u(url)),
        init,
        base,
        base.origin().clone(),
    )
    .unwrap()
}
fn navigation(url: &str, method: &str, body: &[u8], origin: &HttpUrl) -> ApiRequest {
    ApiRequest::navigation(HttpUrl::parse(url).unwrap(), NavigationRequestInit {
        method: method.into(), body: body.to_vec(), referrer: String::new(),
        referrer_policy: String::new(), initiator_origin: origin.origin().clone(),
        reload_or_history: false, client_reserved_id: None,
    }).unwrap()
}
fn finish(service: &mut NetworkService, id: ServiceId) -> Result<ApiResponse, ApiError> {
    let now = Instant::now();
    for tick in 0..4000 {
        for event in service.step(8192, now + Duration::from_millis(tick)) {
            if let ServiceEvent::Finished { id: found, result } = event {
                if found == id {
                    return result;
                }
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("WPT service request did not finish");
}
fn finish_subresource(service: &mut NetworkService, id: ServiceId)
    -> Result<amazing_fetch::service::SubresourceResponse, ApiError> {
    let now = Instant::now();
    for tick in 0..4000 {
        for event in service.step(8192, now + Duration::from_millis(tick)) {
            match event {
                ServiceEvent::SubresourceFinished { id: found, response } if found == id => return Ok(response),
                ServiceEvent::SubresourceFailed { id: found, error } if found == id => return Err(error),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("WPT subresource {id:?} did not finish");
}
fn fetch(
    service: &mut NetworkService,
    base: &HttpUrl,
    url: &str,
    init: ApiRequestInit,
) -> Result<ApiResponse, ApiError> {
    let id = service.start(request(base, url, init), ClientContext::default())?;
    finish(service, id)
}
fn header(response: &ApiResponse, name: &str) -> Option<String> {
    response.headers.get_string(name).unwrap()
}
#[derive(Default)]
struct Matrix {
    pass: usize,
    fail: Vec<String>,
}
impl Matrix {
    fn check(&mut self, name: impl Into<String>, ok: bool) {
        let name = name.into();
        if ok {
            self.pass += 1;
        } else {
            self.fail.push(name);
        }
    }
    fn done(self) {
        println!(
            "WPT expectations pass={} fail={}",
            self.pass,
            self.fail.len()
        );
        for f in &self.fail {
            println!("DIVERGENCE {f}");
        }
        assert!(self.fail.is_empty(), "{} WPT divergences", self.fail.len());
    }
}
#[test]
fn fetch_request_headers_from_wpt() {
    // fetch/api/basic/request-headers.any.js: GET lacks Origin; non-GET has it.
    let server = WptServer::new();
    let b = base(&server);
    let origin = b.origin().serialize();
    let mut m = Matrix::default();
    for (method, body, expected_origin, expected_len) in [
        ("GET", None, None, None),
        ("HEAD", None, None, None),
        ("POST", None, Some(origin.as_str()), Some("0")),
        (
            "PUT",
            Some("Request's body"),
            Some(origin.as_str()),
            Some("14"),
        ),
        (
            "POST",
            Some("Request's body"),
            Some(origin.as_str()),
            Some("14"),
        ),
        ("Chicken", None, Some(origin.as_str()), None),
    ] {
        let mut init = ApiRequestInit::default();
        init.method = Some(u(method));
        if let Some(body) = body {
            init.body = Some(Some(BodyInput::String(u(body))));
        }
        let mut service = NetworkService::new(DispatchLimits::default());
        let url=server.url("/fetch/api/resources/inspect-headers.py?headers=origin|user-agent|content-length|content-type");
        let result = fetch(&mut service, &b, &url, init);
        let ok = result.as_ref().is_ok_and(|r| {
            header(r, "x-request-origin").as_deref() == expected_origin
                && expected_len
                    .is_none_or(|n| header(r, "x-request-content-length").as_deref() == Some(n))
                && header(r, "x-request-user-agent").is_some()
        });
        m.check(
            format!("request-headers {method} body={body:?}: {result:?}"),
            ok,
        );
    }
    m.done();
}
#[test]
fn fetch_redirect_mode_matrix_from_wpt() {
    // fetch/api/redirect/redirect-mode.any.js: two origins x 5 statuses x 3 redirect modes x 2 modes.
    let same = WptServer::new();
    let cross = WptServer::new();
    let b = base(&same);
    let mut m = Matrix::default();
    for remote in [false, true] {
        for status in [301, 302, 303, 307, 308] {
            for redirect in ["error", "manual", "follow"] {
                for mode in ["cors", "no-cors"] {
                    let source = if remote { &cross } else { &same };
                    let url=source.url(&format!("/fetch/api/resources/redirect.py?redirect_status={status}&location=cors-top.txt"));
                    let mut init = ApiRequestInit::default();
                    init.redirect = Some(redirect.into());
                    init.mode = Some(mode.into());
                    let mut service = NetworkService::new(DispatchLimits::default());
                    let result = fetch(&mut service, &b, &url, init);
                    let reject = redirect == "error"
                        || (remote && mode == "no-cors" && redirect != "follow");
                    let ok = if reject {
                        result.is_err()
                    } else if redirect == "manual" {
                        result.as_ref().is_ok_and(|r| {
                            r.status == 0
                                && r.response_type
                                    == amazing_fetch::api::ApiResponseType::OpaqueRedirect
                        })
                    } else if remote && mode == "no-cors" {
                        result.as_ref().is_ok_and(|r| {
                            r.status == 0
                                && r.response_type == amazing_fetch::api::ApiResponseType::Opaque
                        })
                    } else {
                        result
                            .as_ref()
                            .is_ok_and(|r| r.status == 200 && r.url.contains("cors-top.txt"))
                    };
                    m.check(
                        format!("redirect remote={remote} {status} {redirect} {mode}: {result:?}"),
                        ok,
                    );
                }
            }
        }
    }
    m.done();
}
#[test]
fn fetch_cors_preflight_and_credentials_from_wpt() {
    // cors-basic.any.js, cors-preflight.any.js, credentials/cookies.any.js.
    let same = WptServer::new();
    let cross = WptServer::new();
    let b = base(&same);
    let mut m = Matrix::default();
    for (mode, cors, expected) in [
        ("no-cors", true, "opaque"),
        ("cors", false, "error"),
        ("cors", true, "cors"),
    ] {
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut init = ApiRequestInit::default();
        init.mode = Some(mode.into());
        let url = cross.url(if cors {
            "/fetch/api/resources/cors-top.txt"
        } else {
            "/fetch/api/resources/top.txt"
        });
        let result = fetch(&mut service, &b, &url, init);
        let ok = match expected {
            "error" => result.is_err(),
            "opaque" => result.as_ref().is_ok_and(|r| r.status == 0),
            _ => result.as_ref().is_ok_and(|r| r.status == 200),
        };
        m.check(format!("cors-basic {mode} cors={cors}: {result:?}"), ok);
    }
    for (method, allow) in [("DELETE", true), ("DELETE", false), ("PUT", true)] {
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut init = ApiRequestInit::default();
        init.method = Some(u(method));
        let url = cross.url(&format!(
            "/fetch/api/resources/preflight.py?token={method}{allow}&allow_methods={}",
            if allow { method } else { "GET" }
        ));
        let result = fetch(&mut service, &b, &url, init);
        m.check(
            format!("preflight {method} allow={allow}: {result:?}"),
            if allow {
                result
                    .as_ref()
                    .is_ok_and(|r| header(r, "x-did-preflight").as_deref() == Some("1"))
            } else {
                result.is_err()
            },
        );
        let seen = cross.seen();
        m.check(
            format!("OPTIONS sent {method} allow={allow}"),
            seen.iter().any(|r| {
                r.method == "OPTIONS" && r.target.contains(&format!("token={method}{allow}"))
            }),
        );
    }
    // Same-origin cookie storage/sending, including omit modes.
    for (save, send, expect) in [
        ("include", "include", true),
        ("same-origin", "same-origin", true),
        ("omit", "include", false),
        ("include", "omit", false),
    ] {
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut first = ApiRequestInit::default();
        first.credentials = Some(save.into());
        let set = same.url(
            "/fetch/api/resources/top.txt?pipe=header(Set-Cookie%2Ca%3D1%3B%20Path%3D%2F%2CTrue)",
        );
        let _ = fetch(&mut service, &b, &set, first);
        let mut second = ApiRequestInit::default();
        second.credentials = Some(send.into());
        let inspect = same.url("/fetch/api/resources/inspect-headers.py?headers=cookie");
        let result = fetch(&mut service, &b, &inspect, second);
        m.check(
            format!("credentials save={save} send={send}: {result:?}"),
            result
                .as_ref()
                .is_ok_and(|r| header(r, "x-request-cookie").is_some() == expect),
        );
    }
    m.done();
}
#[test]
fn fetch_body_abort_and_header_guards_from_wpt() {
    // fetch/api/abort/general.any.js, body/ and headers/ network-visible subset.
    let server = WptServer::new();
    let b = base(&server);
    let mut m = Matrix::default();
    let mut init = ApiRequestInit::default();
    init.signal_aborted = Some(true);
    let mut service = NetworkService::new(DispatchLimits::default());
    m.check(
        "pre-aborted fetch",
        matches!(
            fetch(
                &mut service,
                &b,
                &server.url("/fetch/api/resources/top.txt"),
                init
            ),
            Err(ApiError::Aborted)
        ),
    );
    let slow = server.url("/fetch/api/resources/infinite-slow-response.py");
    let id = service
        .start(
            request(&b, &slow, ApiRequestInit::default()),
            ClientContext::default(),
        )
        .unwrap();
    m.check(
        "in-flight abort",
        service.abort(id) && matches!(finish(&mut service, id), Err(ApiError::Aborted)),
    );
    let mut post = ApiRequestInit::default();
    post.method = Some(u("POST"));
    post.body = Some(Some(BodyInput::String(u("payload"))));
    let result = fetch(
        &mut service,
        &b,
        &server.url("/fetch/api/resources/echo-content.py"),
        post,
    );
    m.check(
        format!("body mixin text: {result:?}"),
        result
            .and_then(|mut r| r.body.bytes())
            .is_ok_and(|body| body == b"payload"),
    );
    // Forbidden author headers are ignored by Request guard; server sees no Cookie override.
    let mut headers = amazing_fetch::api::ApiHeaders::new(amazing_fetch::api::HeaderGuard::Request);
    let _ = headers.append_string("cookie", "injected");
    let mut init = ApiRequestInit::default();
    init.headers = Some(headers);
    let result = fetch(
        &mut service,
        &b,
        &server.url("/fetch/api/resources/inspect-headers.py?headers=cookie"),
        init,
    );
    m.check(
        format!("forbidden Cookie author header: {result:?}"),
        result
            .as_ref()
            .is_ok_and(|r| header(r, "x-request-cookie").is_none()),
    );
    for (name, path) in [
        ("text", "/xhr/resources/content.py?content=hello"),
        ("arrayBuffer", "/xhr/resources/content.py?content=bytes"),
        ("json", "/fetch/api/resources/status.py?content=%7B%22a%22%3A1%7D&type=application%2Fjson"),
        ("formData", "/fetch/api/resources/status.py?content=a%3D1%26b%3D2&type=application%2Fx-www-form-urlencoded"),
    ] {
        let result = fetch(&mut service, &b, &server.url(path), ApiRequestInit::default());
        let ok = result.is_ok_and(|mut r| match name {
            "text" => r.body.text().is_ok_and(|s| s == "hello"),
            "arrayBuffer" => r.body.array_buffer().is_ok_and(|v| v == b"bytes"),
            "json" => r.body.json().is_ok(),
            _ => r.body.form_data().is_ok_and(|form| form.get("a").is_some()),
        });
        m.check(format!("response body mixin {name}"), ok);
    }
    let result = fetch(
        &mut service,
        &b,
        &server.url("/fetch/api/resources/top.txt?pipe=header(Set-Cookie%2Ca%3D1%2CTrue)"),
        ApiRequestInit::default(),
    );
    m.check(
        "response guard hides Set-Cookie",
        result
            .as_ref()
            .is_ok_and(|r| header(r, "set-cookie").is_none()),
    );
    m.done();
}
struct Decoder;
impl XhrTextDecoder for Decoder {
    fn decode(&self, bytes: &[u8], _: Option<&str>) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
fn xhr_run(service: &mut NetworkService, xhr: &mut Xhr) -> Vec<XhrEventKind> {
    let now = Instant::now();
    let mut pending = xhr.start_dispatch(&mut service.dispatcher, 0, now).unwrap();
    let mut kinds = Vec::new();
    for tick in 0..4000 {
        for event in service.step(8192, now + Duration::from_millis(tick)) {
            if let ServiceEvent::ExternalNetwork(net) = event {
                let batch = xhr
                    .on_dispatch_event(
                        &mut pending,
                        &net,
                        &mut service.dispatcher,
                        tick,
                        now + Duration::from_millis(tick),
                    )
                    .unwrap();
                kinds.extend(batch.into_iter().map(|e| e.kind));
            }
        }
        if xhr.state() == XhrState::HeadersReceived || xhr.state() == XhrState::Done {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    while matches!(xhr.state(), XhrState::HeadersReceived | XhrState::Loading) {
        kinds.extend(
            xhr.pull_response(3, 1000)
                .unwrap()
                .into_iter()
                .map(|e| e.kind),
        );
    }
    kinds
}
#[test]
fn xhr_states_events_response_type_and_redirect_from_wpt() {
    // xhr/event-*.any.js, xhr/responsetype.any.js, xhr/send-redirect.htm.
    let server = WptServer::new();
    let b = base(&server);
    let mut m = Matrix::default();
    for (path, kind) in [
        (
            "/xhr/resources/content.py?content=hello",
            XhrResponseType::Text,
        ),
        (
            "/xhr/resources/content.py?content=%7B%22a%22%3A1%7D",
            XhrResponseType::Json,
        ),
        (
            "/xhr/resources/content.py?content=bytes",
            XhrResponseType::ArrayBuffer,
        ),
        (
            "/xhr/resources/redirect.py?location=content.py&code=302",
            XhrResponseType::Text,
        ),
    ] {
        let mut xhr = Xhr::default();
        xhr.open(
            &u("GET"),
            &u(&server.url(path)),
            true,
            &b,
            b.origin().clone(),
        )
        .unwrap();
        xhr.set_response_type(kind).unwrap();
        let mut events = xhr
            .send(None, 0)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect::<Vec<_>>();
        let mut service = NetworkService::new(DispatchLimits::default());
        events.extend(xhr_run(&mut service, &mut xhr));
        m.check(
            format!("xhr states {path}"),
            xhr.state() == XhrState::Done && xhr.status() == 200,
        );
        m.check(
            format!("xhr events {path}: {events:?}"),
            events.contains(&XhrEventKind::Progress)
                && events.contains(&XhrEventKind::Load)
                && events.contains(&XhrEventKind::LoadEnd),
        );
        let ok = match kind {
            XhrResponseType::Json => {
                matches!(xhr.response(&Decoder), Ok(XhrResponseValue::Json(Some(_))))
            }
            XhrResponseType::ArrayBuffer => {
                matches!(xhr.response(&Decoder),Ok(XhrResponseValue::ArrayBuffer(v)) if v==b"bytes")
            }
            _ if path.contains("redirect.py") => {
                matches!(xhr.response(&Decoder), Ok(XhrResponseValue::Text(_)))
            }
            _ => matches!(xhr.response(&Decoder),Ok(XhrResponseValue::Text(s)) if !s.is_empty()),
        };
        m.check(format!("xhr responseType {path}"), ok);
    }
    m.done();
}

#[test]
fn xhr_status_chunked_and_timeout_from_wpt() {
    // xhr/status-basic.htm: 9 status rows x GET, HEAD, CHICKEN.
    // xhr/getresponseheader-chunked-trailer.htm and xhr/event-timeout.any.js.
    let server = WptServer::new();
    let b = base(&server);
    let mut m = Matrix::default();
    let cases = [
        (204, "UNICORNSWIN", ""),
        (401, "OH HELLO", "Not today."),
        (402, "FIVE BUCKS", "<x>402</x>"),
        (402, "FREE", "Nice!"),
        (402, "402 TEH AWESOME", ""),
        (502, "YO", ""),
        (502, "lowercase", "SWEET POTATO"),
        (503, "HOUSTON WE HAVE A", "503"),
        (699, "WAY OUTTA RANGE", "699"),
    ];
    for (code, text, body) in cases {
        for method in ["GET", "HEAD", "CHICKEN"] {
            let mut xhr = Xhr::default();
            let path = format!(
                "/xhr/resources/status.py?code={code}&text={}&content={}",
                text.replace(' ', "%20"),
                body.replace(' ', "%20")
            );
            xhr.open(
                &u(method),
                &u(&server.url(&path)),
                true,
                &b,
                b.origin().clone(),
            )
            .unwrap();
            xhr.send(None, 0).unwrap();
            let mut service = NetworkService::new(DispatchLimits::default());
            let _ = xhr_run(&mut service, &mut xhr);
            m.check(
                format!(
                    "xhr status {method} {code} {text}: state={:?} status={} text={}",
                    xhr.state(),
                    xhr.status(),
                    xhr.status_text()
                ),
                xhr.state() == XhrState::Done
                    && xhr.status() == code
                    && xhr.status_text() == text
                    && xhr
                        .get_response_header(&u("x-request-method"))
                        .unwrap()
                        .as_deref()
                        == Some(method)
                    && (method == "HEAD"
                        || xhr.response_text(&Decoder).unwrap_or_default() == body),
            );
        }
    }
    let mut xhr = Xhr::default();
    xhr.open(
        &u("GET"),
        &u(&server.url("/xhr/resources/chunked.py")),
        true,
        &b,
        b.origin().clone(),
    )
    .unwrap();
    xhr.send(None, 0).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let _ = xhr_run(&mut service, &mut xhr);
    m.check("chunked body and trailer header",xhr.state()==XhrState::Done &&
        xhr.get_response_header(&u("trailer")).unwrap().as_deref()==Some("X-Test-Me") &&
        xhr.get_response_header(&u("x-test-me")).unwrap().is_none() &&
        xhr.response_text(&Decoder).unwrap_or_default()=="First chunk\r\nSecond chunk\r\nYet another (third) chunk\r\nYet another (fourth) chunk\r\n");
    let mut xhr = Xhr::default();
    xhr.open(
        &u("GET"),
        &u(&server.url("/xhr/resources/delay.py?ms=20000")),
        true,
        &b,
        b.origin().clone(),
    )
    .unwrap();
    xhr.set_timeout(5).unwrap();
    xhr.send(None, 0).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let pending = xhr
        .start_dispatch(&mut service.dispatcher, 0, Instant::now())
        .unwrap();
    let events = xhr
        .check_dispatch_timeout(&pending, &mut service.dispatcher, 6)
        .unwrap()
        .unwrap();
    m.check(
        "XHR timeout event and DONE state",
        xhr.state() == XhrState::Done && events.iter().any(|e| e.kind == XhrEventKind::Timeout),
    );
    m.done();
}

#[test]
fn navigation_redirect_heads_allow_browser_controlled_multi_hop() {
    let first = WptServer::new();
    let second = WptServer::new();
    let origin = HttpUrl::parse(&first.url("/start")).unwrap();
    let second_redirect = second.url("/xhr/resources/redirect.py?code=307&location=status.py%3Fcode%3D200");
    for status in [301, 302] {
        let first_url = first.url(&format!(
            "/xhr/resources/redirect.py?code={status}&location={}", second_redirect.replace('&', "%26")
        ));
        let mut service = NetworkService::new(DispatchLimits::default());
        let id = service.start(navigation(&first_url, "POST", b"a=1", &origin), ClientContext::default()).unwrap();
        let response = finish(&mut service, id).unwrap();
        assert_eq!(response.status, status);
        let second_location = response.headers.get_string("location").unwrap().unwrap();
        assert!(second_location.starts_with(&second.url("/xhr/resources/redirect.py?code=307")));
        assert_eq!(response.url_list, vec![first_url.clone()]);
        assert_eq!(response.url, first_url);

        // HTML owns the method rewrite and starts each hop in the same navigation epoch.
        let id = service.start(navigation(&second_location, "GET", b"", &origin), ClientContext::default()).unwrap();
        let second_response = finish(&mut service, id).unwrap();
        assert_eq!(second_response.status, 307);
        let final_url = second_response.headers.get_string("location").unwrap().unwrap();
        let final_url = HttpUrl::parse(&second_location).unwrap().resolve(&final_url).unwrap().serialize();
        let id = service.start(navigation(&final_url, "GET", b"", &origin), ClientContext::default()).unwrap();
        assert_eq!(finish(&mut service, id).unwrap().status, 200);
    }
    assert!(first.seen().iter().all(|request| request.method == "POST"));
    assert!(second.seen().iter().all(|request| request.method == "GET"));
}

#[test]
fn streaming_navigation_303_emits_location_head_before_credit() {
    let server = WptServer::new();
    let url = server.url("/xhr/resources/redirect.py?code=303&location=status.py%3Fcode%3D200");
    let origin = HttpUrl::parse(&url).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service.start_stream_with_credit(
        navigation(&url, "POST", b"a=1", &origin), ClientContext::default(), 0,
    ).unwrap();
    let start = Instant::now();
    let mut head = None;
    for tick in 0..4000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            match event {
                ServiceEvent::ResponseHead { id: found, head: value } if found == id => head = Some(value),
                ServiceEvent::StreamFailed { id: found, error } if found == id => panic!("{error:?}"),
                _ => {}
            }
        }
        if head.is_some() { break; }
        thread::sleep(Duration::from_millis(1));
    }
    let head = head.expect("navigation redirect head");
    assert_eq!(head.status, 303);
    assert_eq!(head.headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.as_str()), Some("status.py?code=200"));
    assert_eq!(head.url_list, vec![url]);
    assert!(!head.redirect_tainted);
    assert_eq!(head.response_type, amazing_fetch::api::ApiResponseType::Basic);
    assert_eq!(server.seen()[0].method, "POST");
    let final_url = origin.resolve("status.py?code=200").unwrap().serialize();
    let next = service.start(navigation(&final_url, "GET", b"", &origin), ClientContext::default()).unwrap();
    assert_eq!(finish(&mut service, next).unwrap().status, 200);
    assert_eq!(server.seen()[1].method, "GET");
}

#[test]
fn streaming_cross_origin_navigation_head_marks_redirect_taint() {
    let first = WptServer::new();
    let second = WptServer::new();
    let target = second.url("/xhr/resources/status.py?code=200");
    let url = first.url(&format!("/xhr/resources/redirect.py?code=302&location={target}"));
    let origin = HttpUrl::parse(&url).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service.start_stream_with_credit(
        navigation(&url, "POST", b"a=1", &origin), ClientContext::default(), 0,
    ).unwrap();
    let start = Instant::now();
    for tick in 0..4000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            if let ServiceEvent::ResponseHead { id: found, head } = event {
                if found == id {
                    assert_eq!(head.status, 302);
                    assert_eq!(head.url_list, vec![url]);
                    assert!(head.redirect_tainted);
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("cross-origin navigation head did not arrive");
}

#[test]
fn trusted_subresource_keeps_validated_opaque_bytes_and_rejects_orb() {
    let server = WptServer::new();
    let initiator = HttpUrl::parse("http://127.0.0.1:1/page").unwrap();
    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    let url = server.url("/fetch/api/resources/status.py?type=image/png&content=abc");
    let mut image_request = request(&initiator, &url, init);
    image_request.set_response_body_limit(3).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service.start_subresource(image_request, ClientContext::default(), amazing_fetch::Destination::Image);
    let response = finish_subresource(&mut service, id).unwrap();
    assert_eq!(response.response_type, amazing_fetch::ResponseType::Opaque);
    assert_eq!(response.head.status, 200);
    assert_eq!(response.body, b"abc");
    assert!(response.head.headers.iter().any(|(name, value)|
        name.eq_ignore_ascii_case("content-type") && value == "image/png"));

    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    let blocked = server.url("/fetch/api/resources/status.py?type=application/json&content=%7B%22x%22%3A1%7D");
    let id = service.start_subresource(request(&initiator, &blocked, init),
        ClientContext::default(), amazing_fetch::Destination::Image);
    assert!(matches!(finish_subresource(&mut service, id),
        Err(ApiError::Network(amazing_fetch::FetchError::OpaqueResponse))));

    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    let mut limited = request(&initiator, &url, init);
    limited.set_response_body_limit(2).unwrap();
    let id = service.start_subresource(limited, ClientContext::default(), amazing_fetch::Destination::Image);
    assert!(matches!(finish_subresource(&mut service, id),
        Err(ApiError::Network(amazing_fetch::FetchError::ResponseTooLarge))));
}
