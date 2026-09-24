use amazing_fetch::api::{
    ApiError, ApiRequest, ApiRequestInit, ApiRequestInput, NavigationRequestInit, NavigableContext,
};
use amazing_fetch::{check_frame_embedding, CredentialsMode, Destination, HeaderList, HttpUrl, RedirectMode, RequestMode};

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

#[test]
fn trusted_child_navigation_has_frame_destination_and_manual_redirects() {
    let parent = HttpUrl::parse("https://parent.test/").unwrap();
    let top = HttpUrl::parse("https://top.test/").unwrap();
    let frame = HttpUrl::parse("https://child.test/frame").unwrap();
    let context = NavigableContext {
        top_level_origin: top.origin().clone(),
        parent_origin: parent.origin().clone(),
        ancestor_origins: vec![top.origin().clone(), parent.origin().clone()],
        sandbox_flags: 1,
        frame_csp: Default::default(),
        referrer: parent.serialize(),
        referrer_policy: "strict-origin-when-cross-origin".into(),
        now_unix: 100,
    };
    let request = ApiRequest::child_navigation(frame, &context).unwrap();
    assert_eq!(request.mode, RequestMode::Navigate);
    assert_eq!(request.destination, Destination::Other);
    assert_eq!(request.redirect, RedirectMode::Manual);
    assert_eq!(request.credentials, CredentialsMode::Include);
    assert_eq!(request.method(), "GET");
    assert_eq!(request.referrer, parent.serialize());
}

#[test]
fn frame_ancestors_and_xfo_guard_embedded_responses() {
    let response = HttpUrl::parse("https://child.test/").unwrap();
    let same = response.origin().clone();
    let cross = HttpUrl::parse("https://other.test/").unwrap().origin().clone();
    let mut headers = HeaderList::default();
    headers.append("X-Frame-Options", "SAMEORIGIN").unwrap();
    assert!(check_frame_embedding(&response, &headers, &[same.clone()]).is_ok());
    assert!(check_frame_embedding(&response, &headers, &[cross.clone()]).is_err());
    headers.append("Content-Security-Policy", "frame-ancestors https://other.test").unwrap();
    assert!(check_frame_embedding(&response, &headers, &[cross.clone()]).is_ok());
    assert!(check_frame_embedding(&response, &headers, &[same]).is_err());
    headers.append("Content-Security-Policy", "frame-ancestors 'none'").unwrap();
    assert!(check_frame_embedding(&response, &headers, &[cross]).is_err());
}
