use amazing_fetch::api::{
    ApiError, ApiRequest, ApiRequestInit, ApiRequestInput, NavigationRequestInit,
};
use amazing_fetch::{CredentialsMode, Destination, HttpUrl, RedirectMode, RequestMode};

#[test]
fn trusted_navigation_has_document_policy_and_preserves_browser_metadata() {
    let target = HttpUrl::parse("https://example.test/submit").unwrap();
    let origin = target.origin().clone();
    let mut request = ApiRequest::navigation(
        target,
        NavigationRequestInit {
            method: "POST".into(),
            body: b"a=1".to_vec(),
            referrer: "https://example.test/start".into(),
            referrer_policy: "strict-origin-when-cross-origin".into(),
            initiator_origin: origin,
            reload_or_history: true,
            client_reserved_id: Some(42),
        },
    )
    .unwrap();
    assert_eq!(request.mode, RequestMode::Navigate);
    assert_eq!(request.destination, Destination::Document);
    assert_eq!(request.credentials, CredentialsMode::Include);
    assert_eq!(request.redirect, RedirectMode::Manual);
    assert_eq!(request.method(), "POST");
    assert_eq!(request.referrer, "https://example.test/start");
    assert!(request.reload_or_history);
    assert_eq!(request.client_reserved_id, Some(42));
    assert_eq!(request.body.bytes().unwrap(), b"a=1");
}

#[test]
fn web_request_constructor_still_rejects_navigate_mode() {
    let base = HttpUrl::parse("https://example.test/").unwrap();
    let units: Vec<u16> = "/next".encode_utf16().collect();
    let mut init = ApiRequestInit::default();
    init.mode = Some("navigate".into());
    let result = ApiRequest::new(
        ApiRequestInput::Url(&units),
        init,
        &base,
        base.origin().clone(),
    );
    assert!(matches!(result, Err(ApiError::NavigateMode)));
}
