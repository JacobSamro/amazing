//! Native expectations from pinned WPT xhr/setrequestheader-* tests.
//! JS argument coercion and Document bodies require the embedding bindings.
#[path = "../../amazing-net/tests/support/wpt_server.rs"]
mod wpt_server;
use amazing_fetch::{
    api::BodyInput,
    service::{NetworkService, ServiceEvent},
    xhr::{Xhr, XhrError, XhrState, XhrTextDecoder},
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
struct Decoder;
impl XhrTextDecoder for Decoder {
    fn decode(&self, bytes: &[u8], _: Option<&str>) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
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
            self.pass += 1
        } else {
            self.fail.push(name)
        }
    }
    fn done(self) {
        println!(
            "WPT XHR request-header expectations pass={} fail={}",
            self.pass,
            self.fail.len()
        );
        for f in &self.fail {
            println!("DIVERGENCE {f}")
        }
        assert!(self.fail.is_empty(), "{} divergences", self.fail.len());
    }
}
fn open(server: &WptServer, method: &str, path: &str) -> Xhr {
    let base = HttpUrl::parse(&server.url("/page")).unwrap();
    let mut x = Xhr::default();
    x.open(
        &u(method),
        &u(&server.url(path)),
        true,
        &base,
        base.origin().clone(),
    )
    .unwrap();
    x
}
fn complete(x: &mut Xhr) {
    let mut service = NetworkService::new(DispatchLimits::default());
    let now = Instant::now();
    let mut pending = x.start_dispatch(&mut service.dispatcher, 0, now).unwrap();
    for tick in 0..3000 {
        for event in service.step(8192, now + Duration::from_millis(tick)) {
            if let ServiceEvent::ExternalNetwork(net) = event {
                x.on_dispatch_event(
                    &mut pending,
                    &net,
                    &mut service.dispatcher,
                    tick,
                    now + Duration::from_millis(tick),
                )
                .unwrap();
            }
        }
        if matches!(x.state(), XhrState::HeadersReceived | XhrState::Done) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    while matches!(x.state(), XhrState::HeadersReceived | XhrState::Loading) {
        x.pull_response(8192, 1000).unwrap();
    }
}
fn send(x: &mut Xhr, body: Option<BodyInput>) -> String {
    x.send(body, 0).unwrap();
    complete(x);
    x.response_text(&Decoder).unwrap_or_default()
}
fn set(x: &mut Xhr, n: &str, v: &str) -> Result<(), XhrError> {
    x.set_request_header(&u(n), &u(v))
}
#[test]
fn state_and_invalid_arguments() {
    let server = WptServer::new();
    let mut m = Matrix::default();
    let mut x = Xhr::default();
    m.check(
        "before open InvalidStateError",
        matches!(
            set(&mut x, "x-test", "test"),
            Err(XhrError::InvalidStateHeader)
        ),
    );
    let mut x = open(&server, "GET", "/xhr/resources/delay.py?ms=0");
    x.send(None, 0).unwrap();
    m.check(
        "after send InvalidStateError",
        matches!(
            set(&mut x, "x-test", "test"),
            Err(XhrError::InvalidStateHeader)
        ),
    );
    let invalid = [
        "(", ")", "<", ">", "@", ",", ";", ":", "\\", "\"", "/", "[", "]", "?", "=", "{", "}", " ",
        "\x7f", "", "t\rt", "t\nt", "t: t", "t:t", "t<t", "t t", " tt", ":tt", "\ttt", "\x0btt",
        "t\0t", "t\"t", "t,t", "t;t", "()[]{}", "a?B", "a=B",
    ];
    for name in invalid {
        let mut x = open(&server, "GET", "/page");
        let result = set(&mut x, name, "x-value");
        m.check(
            format!("invalid name {name:?}: {result:?}"),
            matches!(result, Err(XhrError::SyntaxHeaderName)),
        );
    }
    for byte in 0_u8..32 {
        let name = char::from(byte).to_string();
        let mut x = open(&server, "GET", "/page");
        let result = set(&mut x, &name, "x-value");
        m.check(
            format!("invalid control name {byte}: {result:?}"),
            matches!(result, Err(XhrError::SyntaxHeaderName)),
        );
    }
    for name in ["ﾃｽﾄ", "X-ﾃｽﾄ"] {
        let mut x = open(&server, "GET", "/page");
        let result = set(&mut x, name, "x-value");
        m.check(
            format!("non-ByteString name {name}: {result:?}"),
            matches!(result, Err(XhrError::ByteString)),
        );
    }
    for value in ["t\0t", "t\rt", "t\nt"] {
        let mut x = open(&server, "GET", "/page");
        let result = set(&mut x, "x-test", value);
        m.check(
            format!("invalid value {value:?}: {result:?}"),
            matches!(result, Err(XhrError::SyntaxHeaderValue)),
        );
    }
    let mut x = open(&server, "GET", "/page");
    m.check(
        "non-ByteString value",
        matches!(set(&mut x, "x-test", "ﾃｽﾄ"), Err(XhrError::ByteString)),
    );
    m.done();
}
#[test]
fn allowed_forbidden_and_method_override() {
    let server = WptServer::new();
    let mut m = Matrix::default();
    for name in [
        "Authorization",
        "Pragma",
        "User-Agent",
        "Content-Transfer-Encoding",
        "Content-Type",
        "Overwrite",
        "If",
        "Status-URI",
        "X-Pink-Unicorn",
        "!#$%&'*+-.^_`|~0123456789abcdefghijklmnopqrstuvwxyz",
    ] {
        let mut x = open(
            &server,
            "POST",
            "/xhr/resources/inspect-headers.py?filter_value=t1,%20t2",
        );
        set(&mut x, name, "t1").unwrap();
        set(&mut x, name, "t2").unwrap();
        let text = send(&mut x, None);
        m.check(
            format!("allowed {name}: {text:?}"),
            text == format!("{name},"),
        );
    }
    let forbidden = [
        "Accept-Charset",
        "Accept-Encoding",
        "Connection",
        "Content-Length",
        "Cookie",
        "Cookie2",
        "Date",
        "DNT",
        "Expect",
        "Host",
        "Keep-Alive",
        "Referer",
        "TE",
        "Trailer",
        "Transfer-Encoding",
        "Upgrade",
        "Via",
        "Proxy-",
        "Proxy-LIES",
        "Proxy-Authorization",
        "Sec-",
        "Sec-X",
    ];
    let mut x = open(
        &server,
        "POST",
        "/xhr/resources/inspect-headers.py?filter_value=TEST",
    );
    for name in forbidden {
        m.check(
            format!("forbidden call {name}"),
            set(&mut x, name, "TEST").is_ok(),
        );
    }
    let text = send(&mut x, None);
    m.check(format!("forbidden wire: {text:?}"), text.is_empty());
    for value in [
        "TRACE",
        "TRACK",
        "CONNECT",
        "trace",
        "track",
        "connect",
        "trace,",
        "GET,track ",
        " connect",
    ] {
        for name in [
            "x-http-method-override",
            "x-http-method",
            "x-method-override",
            "X-HTTP-METHOD-OVERRIDE",
            "X-HTTP-METHOD",
            "X-METHOD-OVERRIDE",
        ] {
            let mut x = open(
                &server,
                "POST",
                &format!(
                    "/xhr/resources/inspect-headers.py?filter_value={}",
                    value.replace(' ', "%20")
                ),
            );
            set(&mut x, name, value).unwrap();
            let text = send(&mut x, None);
            m.check(
                format!("forbidden override {name} {value:?}: {text:?}"),
                text.is_empty(),
            );
        }
    }
    for value in ["GETTRACE", "GET", "\",TRACE\","] {
        for name in [
            "x-http-method-override",
            "x-http-method",
            "x-method-override",
            "X-HTTP-METHOD-OVERRIDE",
            "X-HTTP-METHOD",
            "X-METHOD-OVERRIDE",
        ] {
            let mut x = open(
                &server,
                "POST",
                &format!("/xhr/resources/inspect-headers.py?filter_name={name}"),
            );
            set(&mut x, name, value).unwrap();
            let text = send(&mut x, None);
            m.check(
                format!("permitted override {name} {value:?}: {text:?}"),
                text == format!("{name}: {value}\n"),
            );
        }
    }
    m.done();
}
#[test]
fn value_combining_casing_and_reset() {
    let server = WptServer::new();
    let mut m = Matrix::default();
    for value in ["", " ", " t", "t ", " t "] {
        let mut x = open(
            &server,
            "POST",
            "/xhr/resources/inspect-headers.py?filter_name=X-Empty",
        );
        set(&mut x, "X-Empty", value).unwrap();
        let text = send(&mut x, None);
        m.check(
            format!("value normalization {value:?}: {text:?}"),
            text == format!("X-Empty: {}\n", value.trim()),
        );
    }
    let mut x = open(
        &server,
        "POST",
        "/xhr/resources/inspect-headers.py?filter_name=test-me",
    );
    for value in ["", "", " ", "\t", "x\tx", ""] {
        set(&mut x, "test-me", value).unwrap();
    }
    let text = send(&mut x, None);
    m.check(
        format!("combining empties: {text:?}"),
        text == "test-me: , , , , x\tx, \n",
    );
    let mut x = open(
        &server,
        "POST",
        "/xhr/resources/inspect-headers.py?filter_value=t1,%20t2,%20t3",
    );
    for (name, value) in [("x-test", "t1"), ("X-TEST", "t2"), ("X-teST", "t3")] {
        set(&mut x, name, value).unwrap();
    }
    let text = send(&mut x, None);
    m.check(
        format!("case insensitive combine: {text:?}"),
        text == "x-test,",
    );
    let mut x = open(&server, "GET", "/xhr/resources/echo-headers.py");
    set(&mut x, "THIS-IS-A-TEST", "1").unwrap();
    set(&mut x, "THIS-is-A-test", "2").unwrap();
    set(&mut x, "content-TYPE", "x/x").unwrap();
    let text = send(&mut x, None);
    m.check(
        format!("first case retained: {text:?}"),
        text.matches("THIS-IS-A-TEST: 1, 2").count() == 1,
    );
    m.check(
        format!("content-type case retained: {text:?}"),
        text.matches("content-TYPE").count() == 1,
    );
    for (name, value) in [("X-appended-to-this", "True"), ("Accept", "bar/foo")] {
        let path = format!("/xhr/resources/inspect-headers.py?filter_name={name}");
        let mut x = open(&server, "GET", &path);
        set(&mut x, name, "False").unwrap();
        let base = HttpUrl::parse(&server.url("/page")).unwrap();
        x.open(
            &u("GET"),
            &u(&server.url(&path)),
            true,
            &base,
            base.origin().clone(),
        )
        .unwrap();
        set(&mut x, name, value).unwrap();
        let text = send(&mut x, None);
        m.check(
            format!("open clears headers {name}: {text:?}"),
            text == format!("{name}: {value}\n"),
        );
    }
    m.done();
}
#[test]
fn string_content_type() {
    let server = WptServer::new();
    let mut m = Matrix::default();
    for (header, body, expected) in [
        (Some(""), "", Some("")),
        (Some(" "), "", Some("")),
        (None, "test", Some("text/plain;charset=UTF-8")),
        (
            Some("text/javascript;charset=ASCII"),
            "test()",
            Some("text/javascript;charset=UTF-8"),
        ),
        (Some("application/xml"), "test", Some("application/xml")),
    ] {
        let mut x = open(
            &server,
            "POST",
            "/xhr/resources/inspect-headers.py?filter_name=Content-Type",
        );
        if let Some(header) = header {
            set(&mut x, "Content-Type", header).unwrap();
        }
        let text = send(&mut x, Some(BodyInput::String(u(body))));
        let expected = expected
            .map(|v| format!("Content-Type: {v}\n"))
            .unwrap_or_default();
        m.check(
            format!("string Content-Type {header:?} body={body:?}: {text:?}"),
            text == expected,
        );
    }
    m.done();
}
