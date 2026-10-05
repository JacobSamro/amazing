use amazing_fetch::{
    a_priori_authenticated, bad_port, check_corp, check_nosniff, check_opaque_response,
    check_response_security_with_prefix, determine_nosniff, filter_response, psl::PublicSuffixList,
    redirect, Destination, EmbedderPolicy, FetchError, HeaderList, HttpUrl, InternalResponse,
    NoCorsMediaState, Request, RequestMode, SecurityContext,
};

fn url(s: &str) -> HttpUrl {
    HttpUrl::parse(s).unwrap()
}
fn headers(values: &[(&str, &str)]) -> HeaderList {
    let mut out = HeaderList::default();
    for (name, value) in values {
        out.append(name, value).unwrap();
    }
    out
}
fn response(status: u16, values: &[(&str, &str)], body: &[u8]) -> InternalResponse {
    InternalResponse {
        request_includes_credentials: true,
        status,
        status_text: "OK".into(),
        headers: headers(values),
        body: Some(body.into()),
    }
}
fn cross(target: &str) -> Request {
    let mut r = Request::new(url(target), url("https://a.example.com/").origin().clone());
    r.mode = RequestMode::NoCors;
    r
}

#[test]
fn exact_bad_port_table_and_explicit_zero() {
    for port in [0, 1, 21, 25, 53, 554, 6667, 10080] {
        assert!(
            bad_port(&url(&format!("http://a.test:{port}/"))),
            "port {port}"
        );
        assert!(
            bad_port(&url(&format!("https://a.test:{port}/"))),
            "port {port}"
        );
    }
    for port in [80, 81, 443, 65535] {
        assert!(
            !bad_port(&url(&format!("http://a.test:{port}/"))),
            "port {port}"
        );
    }
    let mut r = cross("https://b.test:65535/");
    r.prepare().unwrap();
    r = cross("https://b.test:10080/");
    assert_eq!(r.prepare().unwrap_err(), FetchError::BadPort);
}
#[test]
fn mixed_content_upgrades_images_but_blocks_scripts_and_imageset() {
    assert!(a_priori_authenticated(&url("http://127.0.0.1/")));
    assert!(a_priori_authenticated(&url("http://localhost/")));
    assert!(!a_priori_authenticated(&url("http://a.test/")));
    let mut r = cross("http://b.test:8080/p.png");
    r.security = SecurityContext {
        client_restricts_mixed_content: true,
        destination: Destination::Image,
        ..Default::default()
    };
    r.prepare().unwrap();
    assert_eq!(r.url().serialize(), "https://b.test:8080/p.png");
    let mut script = cross("http://b.test/x.js");
    script.security.client_restricts_mixed_content = true;
    script.security.destination = Destination::Script;
    assert_eq!(script.prepare().unwrap_err(), FetchError::MixedContent);
    let mut imageset = cross("http://b.test/x.png");
    imageset.security.client_restricts_mixed_content = true;
    imageset.security.destination = Destination::Image;
    imageset.security.image_set_initiator = true;
    assert_eq!(imageset.prepare().unwrap_err(), FetchError::MixedContent);
    let mut loopback = cross("http://127.0.0.1/x.js");
    loopback.security.client_restricts_mixed_content = true;
    loopback.security.destination = Destination::Script;
    loopback.prepare().unwrap();
    assert_eq!(loopback.url().serialize(), "http://127.0.0.1/x.js");
}
#[test]
fn mixed_content_top_level_navigation_exception_and_cors_image() {
    let mut nav = cross("http://b.test/");
    nav.mode = RequestMode::Navigate;
    nav.security.client_restricts_mixed_content = true;
    nav.security.destination = Destination::Document;
    nav.security.top_level_navigation = true;
    nav.prepare().unwrap();
    let mut cors_image = cross("http://b.test/i.png");
    cors_image.mode = RequestMode::Cors;
    cors_image.security.client_restricts_mixed_content = true;
    cors_image.security.destination = Destination::Image;
    cors_image.prepare().unwrap();
    assert_eq!(cors_image.url().serialize(), "https://b.test/i.png");
}
#[test]
fn corp_same_origin_same_site_scheme_downgrade_and_coep() {
    let domains = PublicSuffixList;
    let same_site = cross("https://b.example.com/r");
    check_corp(
        &same_site,
        &headers(&[("Cross-Origin-Resource-Policy", "same-site")]),
        &domains,
    )
    .unwrap();
    assert_eq!(
        check_corp(
            &same_site,
            &headers(&[("Cross-Origin-Resource-Policy", "same-origin")]),
            &domains
        ),
        Err(FetchError::CrossOriginResourcePolicy)
    );
    check_corp(
        &same_site,
        &headers(&[("Cross-Origin-Resource-Policy", "cross-origin")]),
        &domains,
    )
    .unwrap();
    let downgrade = cross("http://b.example.com/r");
    check_corp(
        &downgrade,
        &headers(&[("Cross-Origin-Resource-Policy", "same-site")]),
        &domains,
    )
    .unwrap();
    let mut insecure_initiator = Request::new(
        url("https://b.example.com/r"),
        url("http://a.example.com/").origin().clone(),
    );
    insecure_initiator.mode = RequestMode::NoCors;
    assert_eq!(
        check_corp(
            &insecure_initiator,
            &headers(&[("Cross-Origin-Resource-Policy", "same-site")]),
            &domains
        ),
        Err(FetchError::CrossOriginResourcePolicy)
    );
    let mut coep = same_site.clone();
    coep.security.embedder_policy = EmbedderPolicy::RequireCorp;
    assert_eq!(
        check_corp(&coep, &headers(&[]), &domains),
        Err(FetchError::CrossOriginResourcePolicy)
    );
    check_corp(
        &coep,
        &headers(&[("Cross-Origin-Resource-Policy", "same-site")]),
        &domains,
    )
    .unwrap();
    // An invalid combined header has no policy; require-corp then falls back to same-origin.
    assert_eq!(
        check_corp(
            &coep,
            &headers(&[("Cross-Origin-Resource-Policy", "same-site, same-origin")]),
            &domains
        ),
        Err(FetchError::CrossOriginResourcePolicy)
    );
}
#[test]
fn corp_on_same_origin_redirect_chain_is_checked_before_following() {
    let start = url("https://a.example.com/start");
    let mut r = Request::new(start.clone(), start.origin().clone());
    r.mode = RequestMode::NoCors;
    r.security.embedder_policy = EmbedderPolicy::RequireCorp;
    redirect(
        &mut r,
        &response(302, &[("Cross-Origin-Resource-Policy", "same-origin")], b""),
        Some(url("https://b.example.com/next")),
    )
    .unwrap();
    assert_eq!(r.url().serialize(), "https://b.example.com/next");
    assert_eq!(
        redirect(
            &mut r,
            &response(302, &[("Cross-Origin-Resource-Policy", "same-origin")], b""),
            Some(url("https://c.example.com/final"))
        ),
        Err(FetchError::CrossOriginResourcePolicy)
    );
    assert_eq!(r.url().serialize(), "https://b.example.com/next");
}
#[test]
fn nosniff_first_value_and_missing_unknown_mime() {
    assert!(determine_nosniff(&headers(&[(
        "X-Content-Type-Options",
        "NoSniff, bad"
    )])));
    assert!(!determine_nosniff(&headers(&[(
        "X-Content-Type-Options",
        "bad, nosniff"
    )])));
    for values in [
        vec![("X-Content-Type-Options", "nosniff")],
        vec![
            ("X-Content-Type-Options", "nosniff"),
            ("Content-Type", "application/octet-stream"),
        ],
    ] {
        assert_eq!(
            check_nosniff(Destination::Script, &headers(&values)),
            Err(FetchError::Nosniff)
        );
        assert_eq!(
            check_nosniff(Destination::Style, &headers(&values)),
            Err(FetchError::Nosniff)
        );
    }
    check_nosniff(
        Destination::Script,
        &headers(&[
            ("X-Content-Type-Options", "nosniff"),
            ("Content-Type", "text/javascript; charset=utf-8"),
        ]),
    )
    .unwrap();
    check_nosniff(
        Destination::Style,
        &headers(&[
            ("X-Content-Type-Options", "nosniff"),
            ("Content-Type", "text/css"),
        ]),
    )
    .unwrap();
}
#[test]
fn orb_blocks_json_prefix_and_allows_image_signature_with_bounded_input() {
    let json = headers(&[("Content-Type", "application/json")]);
    assert_eq!(
        check_opaque_response(200, &json, br#"{"secret":42}"#, NoCorsMediaState::NotMedia),
        Err(FetchError::OpaqueResponse)
    );
    let png = b"\x89PNG\r\n\x1a\nrest";
    check_opaque_response(
        200,
        &headers(&[("Content-Type", "image/png")]),
        png,
        NoCorsMediaState::NotMedia,
    )
    .unwrap();
    check_opaque_response(200, &headers(&[]), png, NoCorsMediaState::NotMedia).unwrap();
    check_opaque_response(
        200,
        &headers(&[("Content-Type", "text/plain")]),
        b"let x=1;",
        NoCorsMediaState::NotMedia,
    )
    .unwrap();
    check_opaque_response(
        200,
        &headers(&[]),
        b"{ statement(); }",
        NoCorsMediaState::NotMedia,
    )
    .unwrap();
    check_opaque_response(200, &headers(&[]), b"[1, 2]", NoCorsMediaState::NotMedia).unwrap();
    for prefix in [
        br#"{"secret":42}"#.as_slice(),
        b")]}'\n{}".as_slice(),
        b"{}&&alert(1)",
        b"for(;;);[1]",
        b"<html><body>x",
    ] {
        assert_eq!(
            check_opaque_response(200, &headers(&[]), prefix, NoCorsMediaState::NotMedia),
            Err(FetchError::OpaqueResponse)
        );
    }
    let mp4 = b"\0\0\0\x18ftypisom";
    check_opaque_response(
        200,
        &headers(&[("Content-Type", "video/mp4")]),
        mp4,
        NoCorsMediaState::Initial,
    )
    .unwrap();
    assert_eq!(
        check_opaque_response(
            200,
            &headers(&[("Content-Type", "video/mp4")]),
            mp4,
            NoCorsMediaState::NotMedia
        ),
        Err(FetchError::OpaqueResponse)
    );
}
#[test]
fn response_checks_precede_opaque_filter_and_use_only_prefix() {
    let mut r = cross("https://b.test/data");
    r.security.destination = Destination::Image;
    let json = response(
        200,
        &[("Content-Type", "application/json")],
        br#"{"key":1}"#,
    );
    assert_eq!(filter_response(&r, &json), Err(FetchError::OpaqueResponse));
    let png = response(
        200,
        &[("Content-Type", "image/png")],
        b"\x89PNG\r\n\x1a\nlong body",
    );
    let filtered = filter_response(&r, &png).unwrap();
    assert_eq!(filtered.status, 0);
    assert_eq!(filtered.body, None);
    check_response_security_with_prefix(&r, &png, b"\x89PNG\r\n\x1a\n", &PublicSuffixList).unwrap();
}
