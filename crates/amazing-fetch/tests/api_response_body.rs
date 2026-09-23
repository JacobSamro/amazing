//! Assertion ports from WPT fetch/api/response at c48d58747e1f211527fb695fd60548a997fae617.
use amazing_fetch::api::*;
use amazing_fetch::HttpUrl;
fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// response/response-error.any.js: invalid status, statusText, null-body status.
#[test]
fn response_error_matrix() {
    for status in [0, 100, 199, 600, 1000] {
        assert!(matches!(
            ApiResponse::new(
                None,
                ApiResponseInit {
                    status: Some(status),
                    ..Default::default()
                }
            ),
            Err(ApiError::InvalidStatus)
        ));
    }
    for text in ["\n", "Ā"] {
        assert!(ApiResponse::new(
            None,
            ApiResponseInit {
                status_text: Some(u(text)),
                ..Default::default()
            }
        )
        .is_err());
    }
    for status in [204, 205, 304] {
        assert!(matches!(
            ApiResponse::new(
                Some(BodyInput::String(u("body"))),
                ApiResponseInit {
                    status: Some(status),
                    ..Default::default()
                }
            ),
            Err(ApiError::NullBodyStatus)
        ));
        assert!(ApiResponse::new(
            None,
            ApiResponseInit {
                status: Some(status),
                ..Default::default()
            }
        )
        .is_ok());
    }
}

// response/response-init-001.any.js and response-init-002.any.js.
#[test]
fn response_init_defaults_and_content_type() {
    let r = ApiResponse::new(None, ApiResponseInit::default()).unwrap();
    assert_eq!(r.response_type, ApiResponseType::Default);
    assert_eq!(r.url, "");
    assert_eq!(r.status, 200);
    assert_eq!(r.status_text, "");
    assert!(r.ok());
    assert!(!r.body_used());
    let mut r = ApiResponse::new(
        Some(BodyInput::String(u("Hello"))),
        ApiResponseInit {
            status: Some(201),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.headers.get_string("content-type").unwrap().as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    assert_eq!(r.body.text().unwrap(), "Hello");
    assert!(r.body_used());
    assert!(matches!(r.clone_response(), Err(ApiError::BodyUsed)));
}

// response/response-init-contenttype.any.js and response/response-init-002.any.js.
#[test]
fn body_types_and_explicit_content_type() {
    let mut r = ApiResponse::new(
        Some(BodyInput::UrlSearchParams(vec![(u("name"), u("a b"))])),
        ApiResponseInit::default(),
    )
    .unwrap();
    assert_eq!(
        r.headers.get_string("content-type").unwrap().as_deref(),
        Some("application/x-www-form-urlencoded;charset=UTF-8")
    );
    assert_eq!(r.body.text().unwrap(), "name=a+b");
    let mut headers = ApiHeaders::default();
    headers
        .append_string("content-type", "custom/type")
        .unwrap();
    let r = ApiResponse::new(
        Some(BodyInput::String(u("x"))),
        ApiResponseInit {
            headers: Some(headers),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.headers.get_string("content-type").unwrap().as_deref(),
        Some("custom/type")
    );
    assert!(matches!(
        ApiResponse::new(Some(BodyInput::FormData), ApiResponseInit::default()),
        Err(ApiError::UnsupportedFormData)
    ));
}

// response/response-static-error.any.js, response-static-redirect.any.js,
// response-static-json.any.js and response-clone.any.js.
#[test]
fn response_statics_and_clone() {
    let mut error = ApiResponse::error();
    assert_eq!(error.status, 0);
    assert_eq!(error.response_type, ApiResponseType::Error);
    assert_eq!(
        error.headers.append_string("x", "y"),
        Err(ApiError::ImmutableHeaders)
    );
    let base = HttpUrl::parse("https://site.example/base/").unwrap();
    let mut redirect = ApiResponse::redirect(&u("next"), 302, &base).unwrap();
    assert_eq!(
        redirect.headers.get_string("location").unwrap().as_deref(),
        Some("https://site.example/base/next")
    );
    assert_eq!(
        redirect.headers.append_string("x", "y"),
        Err(ApiError::ImmutableHeaders)
    );
    assert!(matches!(
        ApiResponse::redirect(&u("next"), 200, &base),
        Err(ApiError::InvalidRedirectStatus)
    ));
    let mut json =
        ApiResponse::json(b"{\"ok\":true}".to_vec(), ApiResponseInit::default()).unwrap();
    assert_eq!(
        json.headers.get_string("content-type").unwrap().as_deref(),
        Some("application/json")
    );
    let mut clone = json.clone_response().unwrap();
    assert_eq!(clone.body.text().unwrap(), "{\"ok\":true}");
    assert_eq!(json.body.text().unwrap(), "{\"ok\":true}");
}

struct Chunks(Vec<Vec<u8>>);
impl PullBody for Chunks {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError> {
        if self.0.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.0.remove(0)))
        }
    }
}
// Body mixin assertions used by response-consume-empty.any.js and response-clone.any.js.
#[test]
fn body_stream_consumption_json_and_empty() {
    let mut empty = ApiBody::empty();
    assert_eq!(empty.text().unwrap(), "");
    assert_eq!(empty.text().unwrap(), "");
    assert!(!empty.body_used());
    let mut body = ApiBody::from_input(BodyInput::Stream(Box::new(Chunks(vec![
        b"{\"x\":".to_vec(),
        b"1}".to_vec(),
    ]))))
    .unwrap();
    let mut clone = body.clone_body().unwrap();
    assert_eq!(
        body.json().unwrap(),
        JsonValue::Object(vec![("x".into(), JsonValue::Number(1.0))])
    );
    assert!(body.body_used());
    assert_eq!(clone.bytes().unwrap(), b"{\"x\":1}");
    assert_eq!(body.array_buffer(), Err(ApiError::BodyUsed));
    let mut invalid = ApiBody::from_input(BodyInput::Bytes(b"not-json".to_vec())).unwrap();
    assert_eq!(invalid.json(), Err(ApiError::InvalidJson));
    let mut bom = ApiBody::from_input(BodyInput::Bytes(b"\xef\xbb\xbfhello".to_vec())).unwrap();
    assert_eq!(bom.text().unwrap(), "hello");
    let mut locked = ApiBody::from_input(BodyInput::Bytes(b"x".to_vec())).unwrap();
    locked.lock();
    assert!(matches!(locked.clone_body(), Err(ApiError::BodyUsed)));
    locked.unlock();
    assert_eq!(locked.bytes().unwrap(), b"x");
    let mut oversized = ApiBody::from_input(BodyInput::Stream(Box::new(Chunks(vec![vec![
        0;
        MAX_API_BODY_BYTES
            + 1
    ]]))))
    .unwrap();
    assert_eq!(oversized.bytes(), Err(ApiError::BodyTooLarge));
    assert!(oversized.body_used());
}
