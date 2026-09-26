//! Assertion ports from WPT fetch/api/request at c48d58747e1f211527fb695fd60548a997fae617.
use amazing_fetch::{
    api::*, FetchError, FetchPlan, HttpUrl, InternalResponse, NetworkOrCache, Request, RequestMode,
    Site,
};
fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn base() -> HttpUrl {
    HttpUrl::parse("https://site.example/base/").unwrap()
}
fn req(url: &str, init: ApiRequestInit) -> Result<ApiRequest, ApiError> {
    let base = base();
    ApiRequest::new(
        ApiRequestInput::Url(&u(url)),
        init,
        &base,
        base.origin().clone(),
    )
}

#[test]
fn about_blank_request_exposes_no_cors_header_guard() {
    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    let mut request = req("about:blank", init).unwrap();
    assert_eq!(request.url().serialize(), "about:blank");
    request.headers.append(&u("dpr"), &u("2")).unwrap();
    assert_eq!(request.headers.get(&u("dpr")).unwrap(), None);
}

// request-error.js and request-error.any.js: each distinct TypeError condition.
#[test]
fn constructor_error_matrix() {
    let mut init = ApiRequestInit::default();
    init.window_non_null = Some(true);
    assert!(matches!(req("x", init), Err(ApiError::InvalidWindow)));
    assert!(matches!(
        req("http://:bad", ApiRequestInit::default()),
        Err(ApiError::InvalidUrl)
    ));
    assert!(matches!(
        req("http://u:p@site.example", ApiRequestInit::default()),
        Err(ApiError::UrlCredentials)
    ));
    let mut init = ApiRequestInit::default();
    init.mode = Some("navigate".into());
    assert!(matches!(req("x", init), Err(ApiError::NavigateMode)));
    let mut init = ApiRequestInit::default();
    init.referrer = Some(u("http://:bad"));
    assert!(matches!(req("x", init), Err(ApiError::InvalidReferrer)));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("IN VALID"));
    assert!(matches!(req("x", init), Err(ApiError::InvalidMethod)));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("TRACE"));
    assert!(matches!(req("x", init), Err(ApiError::ForbiddenMethod)));
    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    init.method = Some(u("PUT"));
    assert!(matches!(req("x", init), Err(ApiError::NoCorsMethod)));
    let mut init = ApiRequestInit::default();
    init.cache = Some("only-if-cached".into());
    assert!(matches!(req("x", init), Err(ApiError::OnlyIfCachedMode)));
    let mut init = ApiRequestInit::default();
    init.cache = Some("only-if-cached".into());
    init.mode = Some("same-origin".into());
    assert!(req("x", init).is_ok());
    for (field, value) in [
        ("mode", "BAD"),
        ("credentials", "BAD"),
        ("cache", "BAD"),
        ("redirect", "BAD"),
        ("referrer-policy", "BAD"),
    ] {
        let mut init = ApiRequestInit::default();
        match field {
            "mode" => init.mode = Some(value.into()),
            "credentials" => init.credentials = Some(value.into()),
            "cache" => init.cache = Some(value.into()),
            "redirect" => init.redirect = Some(value.into()),
            _ => init.referrer_policy = Some(value.into()),
        }
        assert!(req("x", init).is_err(), "{field}");
    }
}

// request-error.any.js and request-structure.any.js.
#[test]
fn constructor_inherits_and_overrides_headers_body_and_defaults() {
    let mut init = ApiRequestInit::default();
    init.method = Some(u("post"));
    init.body = Some(Some(BodyInput::String(u("plate"))));
    let mut original = req("x", init).unwrap();
    assert_eq!(original.method(), "POST");
    assert_eq!(original.mode, RequestMode::Cors);
    assert_eq!(
        original
            .headers
            .get_string("content-type")
            .unwrap()
            .as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    let base = base();
    let unconsumed = original.clone_request().unwrap();
    let mut override_init = ApiRequestInit::default();
    override_init.headers = Some(ApiHeaders::default());
    let replaced = ApiRequest::new(
        ApiRequestInput::Request(&unconsumed),
        override_init,
        &base,
        base.origin().clone(),
    )
    .unwrap();
    assert!(!replaced.headers.has_string("content-type").unwrap());
    let mut clone = ApiRequest::new(
        ApiRequestInput::Request(&original),
        ApiRequestInit::default(),
        &base,
        base.origin().clone(),
    )
    .unwrap();
    assert_eq!(
        clone.headers.get_string("content-type").unwrap().as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    assert_eq!(clone.body.text().unwrap(), "plate");
    assert_eq!(original.body.text().unwrap(), "plate");
    assert!(matches!(original.clone_request(), Err(ApiError::BodyUsed)));
}

// request-error.any.js: GET/HEAD body and stream duplex errors.
#[test]
fn body_and_stream_restrictions() {
    let mut init = ApiRequestInit::default();
    init.body = Some(Some(BodyInput::String(u("x"))));
    assert!(matches!(req("x", init), Err(ApiError::GetHeadBody)));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("HEAD"));
    init.body = Some(Some(BodyInput::Bytes(vec![])));
    assert!(matches!(req("x", init), Err(ApiError::GetHeadBody)));
    struct EmptyPull;
    impl PullBody for EmptyPull {
        fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError> {
            Ok(None)
        }
    }
    let stream = || BodyInput::Stream(Box::new(EmptyPull));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("POST"));
    init.body = Some(Some(stream()));
    assert!(matches!(req("x", init), Err(ApiError::StreamNeedsDuplex)));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("POST"));
    init.body = Some(Some(stream()));
    init.duplex = Some("half".into());
    init.keepalive = Some(true);
    assert!(matches!(req("x", init), Err(ApiError::KeepaliveStream)));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("POST"));
    init.body = Some(Some(stream()));
    init.duplex = Some("half".into());
    init.mode = Some("no-cors".into());
    assert!(matches!(req("x", init), Err(ApiError::StreamMode)));
    let mut init = ApiRequestInit::default();
    init.method = Some(u("POST"));
    init.body = Some(Some(stream()));
    init.duplex = Some("half".into());
    assert!(req("x", init).is_ok());
}

struct Stub {
    calls: usize,
}
impl NetworkOrCache for Stub {
    fn exchange(
        &mut self,
        _request: &Request,
        _plan: &FetchPlan,
        _site: &Site,
    ) -> Result<InternalResponse, FetchError> {
        self.calls += 1;
        Ok(InternalResponse {
            status: 200,
            status_text: "OK".into(),
            headers: Default::default(),
            body: Some(b"ok".to_vec()),
        })
    }
}
// request-bad-port.any.js: a representative port, plus abort and normal Fetch glue.
#[test]
fn fetch_checks_bad_port_and_abort_before_network() {
    let site = Site::Tuple {
        scheme: "https".into(),
        domain: "site.example".into(),
    };
    let mut network = Stub { calls: 0 };
    let mut blocked = req("https://site.example:22/x", ApiRequestInit::default()).unwrap();
    assert!(matches!(
        fetch_api(&mut blocked, &mut network, &site, &false),
        Err(ApiError::Network(FetchError::BadPort))
    ));
    assert_eq!(network.calls, 0);
    let mut aborted = req("x", ApiRequestInit::default()).unwrap();
    assert!(matches!(
        fetch_api(&mut aborted, &mut network, &site, &true),
        Err(ApiError::Aborted)
    ));
    assert_eq!(network.calls, 0);
    let mut normal = req("x", ApiRequestInit::default()).unwrap();
    let response = fetch_api(&mut normal, &mut network, &site, &false).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(network.calls, 1);
}

#[test]
fn fetch_does_not_replay_stream_body_on_redirect() {
    struct OneChunk(bool);
    impl PullBody for OneChunk {
        fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError> {
            Ok(if std::mem::replace(&mut self.0, false) {
                Some(b"upload".to_vec())
            } else {
                None
            })
        }
    }
    struct RedirectStub(usize);
    impl NetworkOrCache for RedirectStub {
        fn exchange(
            &mut self,
            _request: &Request,
            _plan: &FetchPlan,
            _site: &Site,
        ) -> Result<InternalResponse, FetchError> {
            self.0 += 1;
            let mut headers = amazing_fetch::HeaderList::default();
            headers.append("location", "/next").unwrap();
            Ok(InternalResponse {
                status: 307,
                status_text: String::new(),
                headers,
                body: None,
            })
        }
    }
    let mut init = ApiRequestInit::default();
    init.method = Some(u("POST"));
    init.duplex = Some("half".into());
    init.body = Some(Some(BodyInput::Stream(Box::new(OneChunk(true)))));
    let mut request = req("upload", init).unwrap();
    let mut network = RedirectStub(0);
    let site = Site::Tuple {
        scheme: "https".into(),
        domain: "site.example".into(),
    };
    assert!(matches!(
        fetch_api(&mut request, &mut network, &site, &false),
        Err(ApiError::Network(FetchError::BodyNotReplayable))
    ));
    assert_eq!(network.0, 1);
    assert!(request.body_used());
}
