//! Native expectations ported from pinned WPT fetch/api/headers and xhr/ headers.
//! See tests/wpt/header_xhr/PROVENANCE.md for exact sources and limits.
#[path = "../../amazing-net/tests/support/wpt_server.rs"]
mod wpt_server;
use amazing_fetch::{
    api::{ApiHeaders, ApiRequest, ApiRequestInit, ApiRequestInput, HeaderGuard},
    service::{ClientContext, NetworkService, ServiceEvent},
    xhr::{Xhr, XhrState},
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
#[derive(Default)]
struct Matrix {
    pass: usize,
    fail: Vec<String>,
}
impl Matrix {
    fn check(&mut self, label: impl Into<String>, ok: bool) {
        let label = label.into();
        if ok {
            self.pass += 1
        } else {
            self.fail.push(label)
        }
    }
    fn done(self) {
        println!(
            "WPT header expectations pass={} fail={}",
            self.pass,
            self.fail.len()
        );
        for f in &self.fail {
            println!("DIVERGENCE {f}");
        }
        assert!(
            self.fail.is_empty(),
            "{} WPT header divergences",
            self.fail.len()
        );
    }
}
fn xhr(server: &WptServer, path: &str) -> Xhr {
    let b = HttpUrl::parse(&server.url("/page")).unwrap();
    let mut xhr = Xhr::default();
    xhr.open(
        &u("GET"),
        &u(&server.url(path)),
        true,
        &b,
        b.origin().clone(),
    )
    .unwrap();
    xhr.send(None, 0).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let now = Instant::now();
    let mut pending = xhr.start_dispatch(&mut service.dispatcher, 0, now).unwrap();
    for tick in 0..3000 {
        for event in service.step(8192, now + Duration::from_millis(tick)) {
            if let ServiceEvent::ExternalNetwork(net) = event {
                xhr.on_dispatch_event(
                    &mut pending,
                    &net,
                    &mut service.dispatcher,
                    tick,
                    now + Duration::from_millis(tick),
                )
                .unwrap();
            }
        }
        if matches!(xhr.state(), XhrState::HeadersReceived | XhrState::Done) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    while matches!(xhr.state(), XhrState::HeadersReceived | XhrState::Loading) {
        xhr.pull_response(8192, 1000).unwrap();
    }
    xhr
}
#[test]
fn headers_combine_and_case() {
    let mut m = Matrix::default();
    let mut h = ApiHeaders::default();
    for (n, v) in [
        ("single", "singleValue"),
        ("double", "doubleValue1"),
        ("double", "doubleValue2"),
        ("triple", "tripleValue1"),
        ("triple", "tripleValue2"),
        ("triple", "tripleValue3"),
    ] {
        h.append_string(n, v).unwrap();
    }
    for (n, v) in [
        ("single", "singleValue"),
        ("double", "doubleValue1, doubleValue2"),
        ("triple", "tripleValue1, tripleValue2, tripleValue3"),
    ] {
        m.check(
            format!("combine {n}"),
            h.get_string(n).unwrap().as_deref() == Some(v),
        );
        h.append_string(n, "newSingleValue").unwrap();
        m.check(
            format!("append {n}"),
            h.get_string(n).unwrap().as_deref() == Some(format!("{v}, newSingleValue").as_str()),
        );
        h.set_string(n, "newSingleValue").unwrap();
        m.check(
            format!("set {n}"),
            h.get_string(n).unwrap().as_deref() == Some("newSingleValue"),
        );
        h.delete_string(n).unwrap();
        m.check(format!("delete {n}"), !h.has_string(n).unwrap());
    }
    for (n, v) in [
        ("UPPERCASE", "value1"),
        ("lowercase", "value2"),
        ("mixedCase", "value3"),
        ("Content-TYPE", "value4"),
    ] {
        h.append_string(n, v).unwrap();
        for variant in [n.to_owned(), n.to_ascii_lowercase(), n.to_ascii_uppercase()] {
            m.check(
                format!("case {variant}"),
                h.get_string(&variant).unwrap().as_deref() == Some(v),
            );
        }
        h.delete_string(&n.to_ascii_uppercase()).unwrap();
        m.check(format!("case delete {n}"), !h.has_string(n).unwrap());
    }
    for (pairs, expected) in [
        (
            vec![("1", "a"), ("1", "b")],
            vec![("1".to_owned(), "a, b".to_owned())],
        ),
        (
            vec![("2", "a"), ("1", "b"), ("2", "b")],
            vec![
                ("1".to_owned(), "b".to_owned()),
                ("2".to_owned(), "a, b".to_owned()),
            ],
        ),
    ] {
        let mut h = ApiHeaders::default();
        for (n, v) in pairs {
            h.append_string(n, v).unwrap();
        }
        m.check("sorted combined entries", h.sorted_entries() == expected);
    }
    m.done();
}
#[test]
fn headers_normalize() {
    let mut m = Matrix::default();
    for (actual, expected) in [
        (" space ", "space"),
        ("\ttab\t", "tab"),
        (" spaceAndTab\t", "spaceAndTab"),
        ("\r\n newLine", "newLine"),
        ("newLine\r\n ", "newLine"),
        ("\r\n\tnewLine", "newLine"),
        ("\t\x0c\tnewLine\n", "\x0c\tnewLine"),
        ("newLine\u{a0}", "newLine\u{a0}"),
    ] {
        let mut h = ApiHeaders::default();
        let result = h.append_string("x", actual);
        m.check(
            format!("normalize append {actual:?}: {result:?}"),
            result.is_ok() && h.get_string("x").unwrap().as_deref() == Some(expected),
        );
        let mut h = ApiHeaders::default();
        let result = h.set_string("x", actual);
        m.check(
            format!("normalize set {actual:?}: {result:?}"),
            result.is_ok() && h.get_string("x").unwrap().as_deref() == Some(expected),
        );
    }
    m.done();
}
#[test]
fn headers_no_cors_guard() {
    // headers-no-cors.any.js: appending or setting past the safelist bound is ignored.
    let mut m = Matrix::default();
    for name in ["accept", "accept-language", "content-language"] {
        for values in [
            vec!["s".repeat(127), String::new(), "s".repeat(127)],
            vec![String::new(), "s".repeat(127)],
        ] {
            let mut h = ApiHeaders::new(HeaderGuard::RequestNoCors);
            let first = &values[0];
            for value in &values {
                let result = h.append_string(name, value);
                m.check(
                    format!("no-cors append {name} {}: {result:?}", value.len()),
                    result.is_ok() && h.get_string(name).unwrap().as_deref() == Some(first),
                );
            }
            let result = h.set_string(name, &values.join(", "));
            m.check(
                format!("no-cors set {name}: {result:?}"),
                result.is_ok() && h.get_string(name).unwrap().as_deref() == Some(first),
            );
            h.delete_string(name).unwrap();
            m.check(
                format!("no-cors delete {name}"),
                !h.has_string(name).unwrap(),
            );
        }
    }
    for name in [
        "dpr",
        "rtt",
        "downlink",
        "ect",
        "save-data",
        "viewport-width",
        "width",
        "unknown",
    ] {
        let mut h = ApiHeaders::new(HeaderGuard::RequestNoCors);
        h.append_string(name, "2").unwrap();
        m.check(
            format!("no-cors forbidden append {name}"),
            !h.has_string(name).unwrap(),
        );
        h.set_string(name, "2").unwrap();
        m.check(
            format!("no-cors forbidden set {name}"),
            !h.has_string(name).unwrap(),
        );
    }
    m.done();
}
#[test]
fn headers_control_byte_normalization() {
    // header-values-normalize.any.js: WPT skips VT/FF, tests the other 31 bytes.
    let mut m = Matrix::default();
    for byte in (0_u8..=0x20).filter(|b| !matches!(b, 0x0b | 0x0c)) {
        let c = char::from(byte);
        for value in [c.to_string(), format!("x{c}"), format!("{c}x")] {
            let mut h = ApiHeaders::default();
            let result = h.append_string("val1", &value);
            let expected = if matches!(byte, b'\t' | b'\r' | b'\n' | b' ') {
                value.trim_matches(['\t', '\r', '\n', ' ']).to_owned()
            } else {
                value.clone()
            };
            m.check(
                format!("control {byte:#04x} {value:?}: {result:?}"),
                if byte == 0 {
                    result.is_err()
                } else {
                    result.is_ok()
                        && h.get_string("val1").unwrap().as_deref() == Some(expected.as_str())
                },
            );
        }
    }
    m.done();
}
#[test]
fn fetch_request_header_control_bytes_on_wire() {
    // Fetch branch of header-values-normalize.any.js through the server echo.
    let server = WptServer::new();
    let base = HttpUrl::parse(&server.url("/page")).unwrap();
    let mut m = Matrix::default();
    for byte in (0_u8..=0x20).filter(|b| !matches!(b, 0x0b | 0x0c)) {
        let c = char::from(byte);
        let values = [c.to_string(), format!("x{c}"), format!("{c}x")];
        let mut headers = ApiHeaders::default();
        let accepted = (1..=3).all(|index| {
            headers
                .append_string(&format!("val{index}"), &values[index - 1])
                .is_ok()
        });
        if byte == 0 {
            m.check("NUL rejected before wire", !accepted);
            continue;
        }
        m.check(format!("control {byte:#04x} accepted"), accepted);
        if !accepted {
            for index in 1..=3 {
                m.check(
                    format!("wire {byte:#04x} x-request-val{index}: header rejected"),
                    false,
                );
            }
            continue;
        }
        let mut init = ApiRequestInit::default();
        init.headers = Some(headers);
        let url = server.url("/fetch/api/resources/inspect-headers.py?headers=val1|val2|val3");
        let request = ApiRequest::new(
            ApiRequestInput::Url(&u(&url)),
            init,
            &base,
            base.origin().clone(),
        )
        .unwrap();
        let mut service = NetworkService::new(DispatchLimits::default());
        let id = service.start(request, ClientContext::default()).unwrap();
        let now = Instant::now();
        let mut response = None;
        for tick in 0..3000 {
            for event in service.step(8192, now + Duration::from_millis(tick)) {
                if let ServiceEvent::Finished { id: found, result } = event {
                    if found == id {
                        response = Some(result);
                        break;
                    }
                }
            }
            if response.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        for (index, value) in values.iter().enumerate() {
            let expected = if matches!(byte, b'\t' | b'\n' | b'\r' | b' ') {
                value.trim_matches(['\t', '\n', '\r', ' '])
            } else {
                value
            };
            let header = format!("x-request-val{}", index + 1);
            let actual = response
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|r| r.headers.get_string(&header).ok().flatten());
            m.check(
                format!("wire {byte:#04x} {header}: {response:?}"),
                actual.as_deref() == Some(expected),
            );
        }
    }
    m.done();
}
#[test]
fn xhr_header_visibility_and_case() {
    let server = WptServer::new();
    let mut m = Matrix::default();
    let x = xhr(&server, "/xhr/resources/headers.py");
    m.check("XHR status", x.status() == 200);
    for (n, v) in [
        ("x-custom-header", Some("test")),
        ("X-Custom-Header", Some("test")),
        ("X-CUSTOM-HEADER", Some("test")),
        ("X-custom-HEADER", Some("test")),
        ("X-CUSTOM-header-COMMA", Some("1, 2")),
        ("X-CUSTOM-no-such-header-in-response", None),
        ("CONTENT-TYPE", Some("text/plain")),
        ("x-custom-header-empty", Some("")),
        ("set-cookie", None),
        ("set-cookie2", None),
    ] {
        let actual = x.get_response_header(&u(n));
        m.check(
            format!("getResponseHeader {n}: {actual:?}"),
            actual.ok().flatten().as_deref() == v,
        );
    }
    let all = x.get_all_response_headers().to_ascii_lowercase();
    for name in ["content-type", "x-custom-header", "\r\n"] {
        m.check(format!("all includes {name}"), all.contains(name));
    }
    for name in ["set-cookie", "set-cookie2"] {
        m.check(format!("all excludes {name}"), !all.contains(name));
    }
    m.done();
}
#[test]
fn xhr_raw_header_combining() {
    let server = WptServer::new();
    let mut m = Matrix::default();
    for (name, header, expected) in [
        ("header-content-length", "content-length", "0"),
        ("header-content-length-twice", "content-length", "0, 0"),
        ("headers-double-empty", "double-trouble", ", "),
        ("headers-basic", "foo-test", "1, 2, 3"),
        ("headers-some-are-empty", "heya", ", \x0b\x0c, 1, , , 2"),
        ("headers-www-authenticate", "www-authenticate", "1, 2, 3, 4"),
    ] {
        let x = xhr(&server, &format!("/xhr/resources/{name}.asis"));
        let actual = x.get_response_header(&u(header));
        m.check(
            format!(
                "{name} get {header}: state={:?} value={actual:?}",
                x.state()
            ),
            actual.ok().flatten().as_deref() == Some(expected),
        );
        let expected_all = format!("{header}: {expected}\r\n");
        m.check(
            format!("{name} all: {:?}", x.get_all_response_headers()),
            x.get_all_response_headers() == expected_all,
        );
    }
    let x = xhr(&server, "/xhr/resources/headers.asis");
    m.check(
        format!("headers.asis all: {:?}", x.get_all_response_headers()),
        x.get_all_response_headers()
            == "also-here: Mr. PB\r\newok: lego\r\nfoo-test: 1, 2\r\n__custom: token\r\n",
    );
    m.done();
}
