//! Native assertion ports from WPT xhr/ at 2f7c70099b6ecc6a1f6a821217afd79b6ff81f28.
//! JS property descriptors, event dispatch and server endpoints are outside this port.
use amazing_fetch::{
    api::{BodyInput, FormData, JsonValue},
    xhr::*,
    FetchError, FetchPlan, HeaderList, HttpUrl, InternalResponse, NetworkOrCache, Request, Site,
};
use std::cell::RefCell;
fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn base() -> HttpUrl {
    HttpUrl::parse("https://site.example/base/").unwrap()
}
fn site() -> Site {
    Site::Tuple {
        scheme: "https".into(),
        domain: "site.example".into(),
    }
}
fn open(
    xhr: &mut Xhr,
    method: &str,
    url: &str,
    asynchronous: bool,
) -> Result<Vec<XhrEvent>, XhrError> {
    let base = base();
    xhr.open(
        &u(method),
        &u(url),
        asynchronous,
        &base,
        base.origin().clone(),
    )
}
fn kinds(events: &[XhrEvent]) -> Vec<(XhrEventTarget, XhrEventKind, XhrState)> {
    events
        .iter()
        .map(|e| (e.target, e.kind, e.ready_state))
        .collect()
}
struct Decoder(RefCell<Vec<Option<String>>>);
impl Decoder {
    fn new() -> Self {
        Self(RefCell::new(Vec::new()))
    }
}
impl XhrTextDecoder for Decoder {
    fn decode(&self, bytes: &[u8], charset: Option<&str>) -> String {
        self.0.borrow_mut().push(charset.map(str::to_owned));
        if charset.is_some_and(|s| s.eq_ignore_ascii_case("windows-1252")) {
            bytes.iter().map(|b| char::from(*b)).collect()
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}
struct Stub {
    calls: usize,
    response: Option<InternalResponse>,
    error: Option<FetchError>,
    seen_method: Option<String>,
    seen_header: Option<String>,
    seen_content_type: Option<String>,
}
impl Stub {
    fn ok(body: &[u8]) -> Self {
        let mut headers = HeaderList::default();
        headers
            .append("Content-Type", "text/plain;charset=utf-8")
            .unwrap();
        headers
            .append("Content-Length", &body.len().to_string())
            .unwrap();
        Self {
            calls: 0,
            response: Some(InternalResponse {
                status: 200,
                status_text: "OK".into(),
                headers,
                body: Some(body.to_vec()),
            }),
            error: None,
            seen_method: None,
            seen_header: None,
            seen_content_type: None,
        }
    }
}
impl NetworkOrCache for Stub {
    fn exchange(
        &mut self,
        req: &Request,
        _plan: &FetchPlan,
        _site: &Site,
    ) -> Result<InternalResponse, FetchError> {
        self.calls += 1;
        self.seen_method = Some(req.method().into());
        self.seen_header = req.headers.get("x-demo");
        self.seen_content_type = req.headers.get("content-type");
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(self.response.clone().unwrap())
    }
}

// xmlhttprequest-unsent.htm, xmlhttprequest-basic.htm, abort-during-unsent.any.js,
// abort-during-open.any.js, open-method-bogus.htm, open-method-insecure.htm.
#[test]
fn open_and_initial_state_matrix() {
    let mut x = Xhr::default();
    assert_eq!(x.state(), XhrState::Unsent);
    assert_eq!(x.status(), 0);
    assert_eq!(x.status_text(), "");
    assert_eq!(x.response_url(), "");
    assert_eq!(x.abort(), Vec::<XhrEvent>::new());
    assert_eq!(x.state(), XhrState::Unsent);
    assert_eq!(open(&mut x, "GET", "a", true).unwrap().len(), 1);
    assert_eq!(x.state(), XhrState::Opened);
    assert_eq!(x.abort(), Vec::<XhrEvent>::new());
    assert_eq!(x.state(), XhrState::Unsent);
    assert!(open(&mut x, "BAD METHOD", "a", true).is_err());
    assert_eq!(
        open(&mut x, "TRACE", "a", true),
        Err(XhrError::SecurityMethod)
    );
    assert_eq!(
        open(&mut x, "GET", "http://:bad", true),
        Err(XhrError::SyntaxUrl)
    );
    assert_eq!(open(&mut x, "GET", "b", true).unwrap().len(), 1);
}
// open-method-case-insensitive.htm and open-method-case-sensitive.htm.
#[test]
fn open_method_normalization() {
    let mut x = Xhr::default();
    let mut stub = Stub::ok(b"");
    open(&mut x, "pOsT", "a", true).unwrap();
    x.send(None, 0).unwrap();
    x.advance(&mut stub, &site(), 1).unwrap();
    assert_eq!(stub.seen_method.as_deref(), Some("POST"));
    open(&mut x, "x-MiXeD", "b", true).unwrap();
    x.send(None, 2).unwrap();
    x.advance(&mut stub, &site(), 3).unwrap();
    assert_eq!(stub.seen_method.as_deref(), Some("x-MiXeD"));
}
// setrequestheader-before-open.htm, setrequestheader-after-send.htm,
// setrequestheader-bogus-name.htm, setrequestheader-bogus-value.htm,
// setrequestheader-header-forbidden.htm, setrequestheader-case-insensitive.htm,
// setrequestheader-combining.window.js and the empty/whitespace-value files.
#[test]
fn request_header_validation_combining_and_guard() {
    let mut x = Xhr::default();
    assert_eq!(
        x.set_request_header(&u("x"), &u("v")),
        Err(XhrError::InvalidStateHeader)
    );
    open(&mut x, "POST", "a", true).unwrap();
    assert_eq!(
        x.set_request_header(&u("bad name"), &u("v")),
        Err(XhrError::SyntaxHeaderName)
    );
    assert_eq!(
        x.set_request_header(&u("x"), &u("a\rb")),
        Err(XhrError::SyntaxHeaderValue)
    );
    assert_eq!(
        x.set_request_header(&u("x"), &u("ā")),
        Err(XhrError::ByteString)
    );
    x.set_request_header(&u("X-Demo"), &u("  one  ")).unwrap();
    x.set_request_header(&u("x-demo"), &u("two")).unwrap();
    assert_eq!(
        x.request_headers().get_string("x-demo").unwrap().as_deref(),
        Some("one, two")
    );
    assert_eq!(x.request_headers().raw_entries().len(), 1);
    x.set_request_header(&u("Cookie"), &u("a=b")).unwrap();
    assert!(!x.request_headers().has_string("cookie").unwrap());
    x.set_request_header(&u("X-Empty"), &u(" \t ")).unwrap();
    assert_eq!(
        x.request_headers()
            .get_string("x-empty")
            .unwrap()
            .as_deref(),
        Some("")
    );
    x.send(Some(BodyInput::Bytes(b"x".to_vec())), 0).unwrap();
    assert_eq!(
        x.set_request_header(&u("x"), &u("v")),
        Err(XhrError::InvalidStateHeader)
    );
    let mut stub = Stub::ok(b"");
    x.advance(&mut stub, &site(), 1).unwrap();
    assert_eq!(stub.seen_header.as_deref(), Some("one, two"));
}
// XMLHttpRequest-withCredentials.any.js, sync-no-timeout.any.js,
// responsetype.any.js and overridemimetype-done-state.any.js.
#[test]
fn attribute_state_and_sync_rules() {
    let mut x = Xhr::default();
    assert!(!x.with_credentials());
    x.set_with_credentials(true).unwrap();
    assert!(x.with_credentials());
    x.set_timeout(10).unwrap();
    assert_eq!(
        open(&mut x, "GET", "a", false),
        Err(XhrError::InvalidAccessSyncTimeout)
    );
    x.set_timeout(0).unwrap();
    x.set_response_type(XhrResponseType::Json).unwrap();
    assert_eq!(
        open(&mut x, "GET", "a", false),
        Err(XhrError::InvalidAccessSyncResponseType)
    );
    x.set_response_type(XhrResponseType::Default).unwrap();
    open(&mut x, "GET", "a", false).unwrap();
    assert_eq!(x.set_timeout(1), Err(XhrError::InvalidAccessSyncTimeout));
    assert_eq!(
        x.set_response_type(XhrResponseType::Text),
        Err(XhrError::InvalidAccessSyncResponseType)
    );
    open(&mut x, "GET", "a", true).unwrap();
    x.set_response_type(XhrResponseType::Json).unwrap();
    x.set_with_credentials(false).unwrap();
    x.send(None, 0).unwrap();
    assert_eq!(
        x.set_with_credentials(true),
        Err(XhrError::InvalidStateCredentials)
    );
    let mut stub = Stub::ok(b"{}");
    x.advance(&mut stub, &site(), 1).unwrap();
    x.pull_response(10, 2).unwrap();
    assert_eq!(x.state(), XhrState::Done);
    assert_eq!(
        x.set_response_type(XhrResponseType::Text),
        Err(XhrError::InvalidStateResponseType)
    );
    assert_eq!(
        x.override_mime_type("text/plain"),
        Err(XhrError::InvalidStateOverrideMimeType)
    );
}
#[test]
fn disallowed_document_sync_and_inactive_window() {
    let base = base();
    let mut x = Xhr::new(XhrContext {
        sync_disallowed: true,
        ..XhrContext::default()
    });
    assert_eq!(
        x.open(&u("GET"), &u("a"), false, &base, base.origin().clone()),
        Err(XhrError::InvalidAccessSyncContext)
    );
    let mut y = Xhr::new(XhrContext {
        document_fully_active: false,
        ..XhrContext::default()
    });
    assert_eq!(
        y.open(&u("GET"), &u("a"), true, &base, base.origin().clone()),
        Err(XhrError::InvalidStateOpen)
    );
}
// event-loadstart.any.js, event-progress.any.js, event-load.any.js,
// event-loadend.any.js, send-response-event-order.htm.
#[test]
fn normal_pull_event_sequence_and_upload() {
    let mut x = Xhr::default();
    open(&mut x, "POST", "a", true).unwrap();
    x.set_upload_listeners(true);
    let sent = x.send(Some(BodyInput::String(u("upload"))), 100).unwrap();
    assert_eq!(
        kinds(&sent),
        vec![
            (
                XhrEventTarget::Request,
                XhrEventKind::LoadStart,
                XhrState::Opened
            ),
            (
                XhrEventTarget::Upload,
                XhrEventKind::LoadStart,
                XhrState::Opened
            )
        ]
    );
    let mut stub = Stub::ok(b"abcd");
    let headers = x.advance(&mut stub, &site(), 101).unwrap();
    assert_eq!(x.state(), XhrState::HeadersReceived);
    assert_eq!(
        headers.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            XhrEventKind::Progress,
            XhrEventKind::Load,
            XhrEventKind::LoadEnd,
            XhrEventKind::ReadyStateChange
        ]
    );
    assert_eq!(headers[3].ready_state, XhrState::HeadersReceived);
    let first = x.pull_response(2, 102).unwrap();
    assert_eq!(
        kinds(&first),
        vec![
            (
                XhrEventTarget::Request,
                XhrEventKind::ReadyStateChange,
                XhrState::Loading
            ),
            (
                XhrEventTarget::Request,
                XhrEventKind::Progress,
                XhrState::Loading
            )
        ]
    );
    assert_eq!(first[1].loaded, 2);
    assert_eq!(first[1].total, 4);
    assert!(first[1].length_computable);
    assert_eq!(x.response_text(&Decoder::new()).unwrap(), "ab");
    let end = x.pull_response(2, 103).unwrap();
    assert_eq!(
        end.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            XhrEventKind::ReadyStateChange,
            XhrEventKind::Progress,
            XhrEventKind::Progress,
            XhrEventKind::ReadyStateChange,
            XhrEventKind::Load,
            XhrEventKind::LoadEnd
        ]
    );
    assert_eq!(x.state(), XhrState::Done);
    assert_eq!(x.status(), 200);
    assert_eq!(x.status_text(), "OK");
    assert_eq!(x.response_text(&Decoder::new()).unwrap(), "abcd");
}
// abort-during-loading.window.js, abort-event-abort.any.js,
// abort-event-loadend.any.js, abort-upload-event-abort.any.js.
#[test]
fn abort_during_loading_and_reopen_generation() {
    let mut x = Xhr::default();
    open(&mut x, "GET", "a", true).unwrap();
    x.send(None, 0).unwrap();
    let mut stub = Stub::ok(b"abcd");
    x.advance(&mut stub, &site(), 1).unwrap();
    x.pull_response(1, 2).unwrap();
    let events = x.abort();
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            XhrEventKind::ReadyStateChange,
            XhrEventKind::Abort,
            XhrEventKind::LoadEnd
        ]
    );
    assert_eq!(x.state(), XhrState::Unsent);
    assert_eq!(x.status(), 0);
    assert_eq!(x.response_url(), "");
    assert_eq!(x.last_error(), Some(&XhrError::Abort));
    open(&mut x, "GET", "b", true).unwrap();
    assert!(!x.is_current_event(&events[0]));
    assert_eq!(x.state(), XhrState::Opened);
}
// event-timeout.any.js and send-timeout-events.htm: explicit monotonic clock.
#[test]
fn timeout_event_sequence() {
    let mut x = Xhr::default();
    open(&mut x, "POST", "a", true).unwrap();
    x.set_timeout(5).unwrap();
    x.set_upload_listeners(true);
    x.send(Some(BodyInput::Bytes(vec![1])), 10).unwrap();
    let mut stub = Stub::ok(b"ok");
    let events = x.advance(&mut stub, &site(), 15).unwrap();
    assert_eq!(stub.calls, 0);
    assert_eq!(
        events
            .iter()
            .map(|e| (e.target, e.kind))
            .collect::<Vec<_>>(),
        vec![
            (XhrEventTarget::Request, XhrEventKind::ReadyStateChange),
            (XhrEventTarget::Upload, XhrEventKind::Timeout),
            (XhrEventTarget::Upload, XhrEventKind::LoadEnd),
            (XhrEventTarget::Request, XhrEventKind::Timeout),
            (XhrEventTarget::Request, XhrEventKind::LoadEnd)
        ]
    );
    assert_eq!(x.last_error(), Some(&XhrError::Timeout));
}
// event-error.sub.any.js and response-body-errors.any.js: network error path.
#[test]
fn network_error_event_sequence() {
    let mut x = Xhr::default();
    open(&mut x, "GET", "a", true).unwrap();
    x.send(None, 0).unwrap();
    let mut stub = Stub::ok(b"");
    stub.error = Some(FetchError::BadPort);
    let events = x.advance(&mut stub, &site(), 1).unwrap();
    assert_eq!(stub.calls, 1);
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            XhrEventKind::ReadyStateChange,
            XhrEventKind::Error,
            XhrEventKind::LoadEnd
        ]
    );
    assert_eq!(x.status(), 0);
    assert_eq!(x.state(), XhrState::Done);
    assert_eq!(
        x.last_error(),
        Some(&XhrError::Network(FetchError::BadPort))
    );
}
// send-sync-response-event-order.htm and sync-no-progress.any.js.
#[test]
fn synchronous_completion_has_no_progress_or_loadstart() {
    let mut x = Xhr::default();
    open(&mut x, "GET", "a", false).unwrap();
    let mut stub = Stub::ok(b"done");
    let events = x.send_blocking(None, &mut stub, &site(), 0).unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            XhrEventKind::ReadyStateChange,
            XhrEventKind::Load,
            XhrEventKind::LoadEnd
        ]
    );
    assert_eq!(x.state(), XhrState::Done);
    assert_eq!(x.response_text(&Decoder::new()).unwrap(), "done");
}

#[test]
fn timeout_after_headers_during_body_pull() {
    let mut x = Xhr::default();
    open(&mut x, "GET", "a", true).unwrap();
    x.send(None, 10).unwrap();
    let mut stub = Stub::ok(b"body");
    x.advance(&mut stub, &site(), 11).unwrap();
    x.set_timeout(5).unwrap();
    let events = x.pull_response(2, 15).unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            XhrEventKind::ReadyStateChange,
            XhrEventKind::Timeout,
            XhrEventKind::LoadEnd
        ]
    );
    assert_eq!(x.state(), XhrState::Done);
    assert_eq!(x.status(), 0);
}

#[test]
fn exception_variants_have_binding_names() {
    assert_eq!(
        XhrError::InvalidStateSend.dom_exception_name(),
        "InvalidStateError"
    );
    assert_eq!(XhrError::SyntaxMethod.dom_exception_name(), "SyntaxError");
    assert_eq!(
        XhrError::SecurityMethod.dom_exception_name(),
        "SecurityError"
    );
    assert_eq!(
        XhrError::InvalidAccessSyncTimeout.dom_exception_name(),
        "InvalidAccessError"
    );
    assert_eq!(XhrError::ByteString.dom_exception_name(), "TypeError");
    assert_eq!(
        XhrError::Network(FetchError::Cors).dom_exception_name(),
        "NetworkError"
    );
    assert_eq!(XhrError::Timeout.dom_exception_name(), "TimeoutError");
    assert_eq!(XhrError::Abort.dom_exception_name(), "AbortError");
    assert_eq!(
        XhrError::UnsupportedBlob.dom_exception_name(),
        "NotSupportedError"
    );
}
// getresponseheader-unsent-opened-state.htm, getresponseheader.any.js,
// getallresponseheaders.htm, responseText-status.html.
#[test]
fn response_headers_and_text_state() {
    let mut x = Xhr::default();
    assert_eq!(x.get_response_header(&u("x-demo")).unwrap(), None);
    assert_eq!(x.get_all_response_headers(), "");
    open(&mut x, "GET", "a", true).unwrap();
    assert_eq!(x.get_response_header(&u("x-demo")).unwrap(), None);
    x.send(None, 0).unwrap();
    let mut stub = Stub::ok(b"abc");
    let response = stub.response.as_mut().unwrap();
    response.headers.append("X-Z", "z").unwrap();
    response.headers.append("X-Demo", "one").unwrap();
    response.headers.append("x-demo", "two").unwrap();
    response.headers.append("Set-Cookie", "secret=1").unwrap();
    x.advance(&mut stub, &site(), 1).unwrap();
    assert_eq!(
        x.get_response_header(&u("X-DEMO")).unwrap().as_deref(),
        Some("one, two")
    );
    assert_eq!(x.get_response_header(&u("set-cookie")).unwrap(), None);
    assert!(x
        .get_all_response_headers()
        .contains("x-demo: one, two\r\n"));
    assert!(!x.get_all_response_headers().contains("set-cookie"));
    assert!(
        x.get_all_response_headers().find("content-length").unwrap()
            < x.get_all_response_headers().find("x-z").unwrap()
    );
    assert_eq!(x.response_text(&Decoder::new()).unwrap(), "");
    assert!(x.response_url().starts_with("https://site.example/base/a"));
}
// json.any.js, response-data-arraybuffer.htm, response-json-bom.html,
// responsetype.any.js and overridemimetype-open-state-force-utf-8.htm.
#[test]
fn response_types_and_decoder_bridge() {
    let mut x = Xhr::default();
    open(&mut x, "GET", "a", true).unwrap();
    x.set_response_type(XhrResponseType::Json).unwrap();
    x.send(None, 0).unwrap();
    assert_eq!(x.response(&Decoder::new()).unwrap(), XhrResponseValue::Null);
    let mut stub = Stub::ok(b"\xef\xbb\xbf{\"ok\":true}");
    x.advance(&mut stub, &site(), 1).unwrap();
    x.pull_response(100, 2).unwrap();
    assert_eq!(
        x.response(&Decoder::new()).unwrap(),
        XhrResponseValue::Json(Some(JsonValue::Object(vec![(
            "ok".into(),
            JsonValue::Boolean(true)
        )])))
    );
    assert_eq!(
        x.response_text(&Decoder::new()),
        Err(XhrError::InvalidStateResponseText)
    );
    open(&mut x, "GET", "b", true).unwrap();
    x.set_response_type(XhrResponseType::ArrayBuffer).unwrap();
    x.send(None, 3).unwrap();
    let mut stub = Stub::ok(b"bytes");
    x.advance(&mut stub, &site(), 4).unwrap();
    x.pull_response(100, 5).unwrap();
    assert_eq!(
        x.response(&Decoder::new()).unwrap(),
        XhrResponseValue::ArrayBuffer(b"bytes".to_vec())
    );
    open(&mut x, "GET", "c", true).unwrap();
    x.set_response_type(XhrResponseType::Text).unwrap();
    x.override_mime_type("text/plain;charset=windows-1252")
        .unwrap();
    x.send(None, 6).unwrap();
    let mut stub = Stub::ok(&[0xe9]);
    x.advance(&mut stub, &site(), 7).unwrap();
    x.pull_response(100, 8).unwrap();
    let decoder = Decoder::new();
    assert_eq!(x.response_text(&decoder).unwrap(), "é");
    assert_eq!(
        decoder.0.borrow().as_slice(),
        &[Some("windows-1252".to_string())]
    );
    assert_eq!(
        x.get_response_header(&u("content-type"))
            .unwrap()
            .as_deref(),
        Some("text/plain;charset=utf-8")
    );
}
// send-send.any.js and send-entity-body-get-head.htm.
#[test]
fn repeat_send_and_get_body_ignored() {
    let mut x = Xhr::default();
    open(&mut x, "GET", "a", true).unwrap();
    x.send(Some(BodyInput::FormData(FormData::default())), 0)
        .unwrap();
    assert_eq!(x.send(None, 0), Err(XhrError::InvalidStateSend));
    let mut stub = Stub::ok(b"");
    x.advance(&mut stub, &site(), 1).unwrap();
    assert_eq!(stub.seen_method.as_deref(), Some("GET"));
}

#[test]
fn upload_listener_forces_cross_origin_preflight() {
    struct CorsStub(Vec<String>);
    impl NetworkOrCache for CorsStub {
        fn exchange(
            &mut self,
            request: &Request,
            _plan: &FetchPlan,
            _site: &Site,
        ) -> Result<InternalResponse, FetchError> {
            self.0.push(request.method().into());
            let mut headers = HeaderList::default();
            headers
                .append("access-control-allow-origin", "https://site.example")
                .unwrap();
            Ok(InternalResponse {
                status: 200,
                status_text: "OK".into(),
                headers,
                body: Some(Vec::new()),
            })
        }
    }
    let mut x = Xhr::default();
    open(&mut x, "GET", "https://other.example/x", true).unwrap();
    x.set_upload_listeners(true);
    x.send(None, 0).unwrap();
    let mut network = CorsStub(Vec::new());
    x.advance(&mut network, &site(), 1).unwrap();
    assert_eq!(network.0, vec!["OPTIONS", "GET"]);
    assert_eq!(x.state(), XhrState::Done);
}

// send-content-type-string.htm and open-method-insecure.htm: XHR request
// construction keeps URL userinfo while a UTF-8 string body corrects charset.
#[test]
fn string_body_corrects_author_charset_and_url_userinfo_is_accepted() {
    let mut x = Xhr::default();
    open(
        &mut x,
        "POST",
        "https://user:pass@site.example/upload",
        true,
    )
    .unwrap();
    x.set_request_header(&u("Content-Type"), &u("text/plain;charset=iso-8859-1"))
        .unwrap();
    x.send(Some(BodyInput::String(u("é"))), 0).unwrap();
    let mut stub = Stub::ok(b"");
    x.advance(&mut stub, &site(), 1).unwrap();
    // The transport observes the corrected request header through Fetch.
    assert_eq!(x.state(), XhrState::Done);
    assert_eq!(stub.calls, 1);
    assert_eq!(
        stub.seen_content_type.as_deref(),
        Some("text/plain;charset=UTF-8")
    );
}
