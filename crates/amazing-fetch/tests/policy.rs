use amazing_fetch::*;
fn url(s: &str) -> HttpUrl {
    HttpUrl::parse(s).unwrap()
}
fn request(cross: bool) -> Request {
    Request::new(
        url(if cross {
            "https://b.test/path"
        } else {
            "https://a.test/path"
        }),
        url("https://a.test/").origin().clone(),
    )
}
fn response(status: u16, fields: &[(&str, &str)]) -> InternalResponse {
    let mut headers = HeaderList::default();
    for (n, v) in fields {
        headers.append(n, v).unwrap();
    }
    InternalResponse {
        status,
        status_text: "OK".into(),
        headers,
        body: Some(b"secret".to_vec()),
    }
}
#[test]
fn origins_normalize_host_default_port_and_ipv6() {
    assert_eq!(
        url("HTTPS://EXAMPLE.COM:443/x").origin(),
        url("https://example.com/").origin()
    );
    assert_eq!(
        url("https://[0:0:0:0:0:0:0:1]/").origin(),
        url("https://[::1]:443/").origin()
    );
    assert_ne!(
        url("http://example.com/").origin(),
        url("https://example.com/").origin()
    );
    assert_ne!(
        url("https://example.com:444/").origin(),
        url("https://example.com/").origin()
    );
}
#[test]
fn opaque_origins_have_identity() {
    let a = Origin::opaque();
    let b = Origin::opaque();
    assert_ne!(a, b);
    assert_eq!(a, a.clone());
    assert_eq!(a.serialize(), "null");
    assert_eq!(b.serialize(), "null");
}
#[test]
fn schemeful_sites_use_provider_not_last_two_labels() {
    struct Psl;
    impl RegistrableDomain for Psl {
        fn registrable_domain(&self, host: &str) -> Option<String> {
            host.ends_with(".example.co.uk")
                .then(|| "example.co.uk".into())
        }
    }
    assert_eq!(
        url("https://a.example.co.uk:99").origin().site(&Psl),
        url("https://b.example.co.uk").origin().site(&Psl)
    );
    assert_ne!(
        url("http://a.example.co.uk").origin().site(&Psl),
        url("https://a.example.co.uk").origin().site(&Psl)
    );
    assert_ne!(
        url("https://127.0.0.1").origin().site(&Psl),
        url("https://127.0.0.2").origin().site(&Psl)
    );
    assert_ne!(Origin::opaque().site(&Psl), Origin::opaque().site(&Psl));
}
#[test]
fn url_parser_recovery_replaces_the_strict_ascii_shortcut() {
    // NET-3 intentional changes: these were rejected by NET-1's shortcut.
    for (input, expected) in [
        ("https://u:p@a.test/", "https://u:p@a.test/"),
        ("https://127.1/", "https://127.0.0.1/"),
        ("https://0x7f000001/", "https://127.0.0.1/"),
        ("https://a\\b/", "https://a/b/"),
        ("https://a.test/\r\nx", "https://a.test/x"),
        ("https://a%2etest/", "https://a.test/"),
    ] {
        assert_eq!(HttpUrl::parse(input).unwrap().serialize(), expected);
    }
    // These rejections are unchanged: Fetch supports HTTP(S), no IDNA yet,
    // and the standard still rejects out-of-range ports.
    for input in ["file:///x", "https://é.test/", "https://a.test:65536/"] {
        assert!(HttpUrl::parse(input).is_err(), "{input}");
    }
}

#[test]
fn methods_and_bodies_are_validated() {
    let mut r = request(false);
    for m in ["CONNECT", "trace", "TRACK", "bad method", ""] {
        assert_eq!(r.set_method_body(m, None), Err(FetchError::InvalidMethod));
    }
    assert_eq!(
        r.set_method_body("get", Some(Body::Bytes(vec![]))),
        Err(FetchError::InvalidBody)
    );
    r.set_method_body("post", Some(Body::Bytes(vec![1])))
        .unwrap();
    assert_eq!(r.method(), "POST");
    r.set_method_body("patch", None).unwrap();
    assert_eq!(r.method(), "patch");
}
#[test]
fn headers_reject_injection_combine_and_preserve_cookie_lines() {
    let mut h = HeaderList::default();
    assert!(h.append("bad name", "x").is_err());
    assert!(h.append("x", "a\rb").is_err());
    assert!(h.append("x", "a\0b").is_err());
    h.append("X-A", "  one\t").unwrap();
    h.append("x-a", "two").unwrap();
    assert_eq!(h.get("X-A").as_deref(), Some("one, two"));
    h.append("set-cookie", "a=1").unwrap();
    h.append("set-cookie", "b=2").unwrap();
    assert_eq!(h.iter().filter(|(n, _)| *n == "set-cookie").count(), 2);
}
#[test]
fn redirect_method_body_matrix() {
    for status in [301, 302, 303, 307, 308] {
        for method in ["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH"] {
            let mut r = request(false);
            let body = (!matches!(method, "GET" | "HEAD")).then(|| Body::Bytes(vec![1, 2]));
            r.set_method_body(method, body.clone()).unwrap();
            for n in [
                "content-type",
                "content-encoding",
                "content-language",
                "content-location",
            ] {
                r.headers.append(n, "x").unwrap();
            }
            let rewrite = (matches!(status, 301 | 302) && method == "POST")
                || (status == 303 && !matches!(method, "GET" | "HEAD"));
            assert_eq!(
                redirect(
                    &mut r,
                    &response(status, &[]),
                    Some(url("https://a.test/new"))
                )
                .unwrap(),
                RedirectAction::Follow
            );
            assert_eq!(r.method(), if rewrite { "GET" } else { method });
            assert_eq!(r.body(), if rewrite { None } else { body.as_ref() });
            for n in [
                "content-type",
                "content-encoding",
                "content-language",
                "content-location",
            ] {
                assert_eq!(r.headers.get(n).is_none(), rewrite);
            }
        }
    }
}
#[test]
fn redirect_removes_authorization_only_across_origins() {
    let mut r = request(false);
    r.headers.append("Authorization", "secret").unwrap();
    redirect(
        &mut r,
        &response(302, &[]),
        Some(url("https://a.test/next")),
    )
    .unwrap();
    assert!(r.headers.get("authorization").is_some());
    redirect(
        &mut r,
        &response(302, &[]),
        Some(url("https://b.test/next")),
    )
    .unwrap();
    assert!(r.headers.get("authorization").is_none());
}
#[test]
fn redirect_twenty_hops_then_error() {
    let mut r = request(false);
    for _ in 0..20 {
        redirect(
            &mut r,
            &response(302, &[]),
            Some(url("https://a.test/next")),
        )
        .unwrap();
    }
    assert_eq!(r.url_list().len(), 21);
    assert_eq!(
        redirect(
            &mut r,
            &response(302, &[]),
            Some(url("https://a.test/next"))
        ),
        Err(FetchError::RedirectLimit)
    );
}
#[test]
fn redirect_modes_and_missing_location() {
    let mut r = request(false);
    assert_eq!(
        redirect(&mut r, &response(302, &[]), None).unwrap(),
        RedirectAction::NotRedirect
    );
    r.redirect = RedirectMode::Error;
    assert_eq!(
        redirect(&mut r, &response(302, &[]), None),
        Err(FetchError::Redirect)
    );
    r.redirect = RedirectMode::Manual;
    let RedirectAction::Manual(filtered) =
        redirect(&mut r, &response(302, &[]), Some(url("https://b.test"))).unwrap()
    else {
        panic!()
    };
    let mut expected = Response::hidden(ResponseType::OpaqueRedirect);
    expected.url_list = r.url_list().to_vec();
    assert_eq!(filtered, expected);
    assert_eq!(r.url_list().len(), 1);
    assert_eq!(
        redirect(&mut r, &response(304, &[]), None).unwrap(),
        RedirectAction::NotRedirect
    );
}
#[test]
fn unreplayable_body_rejected_except_303() {
    for status in [301, 302, 307, 308] {
        let mut r = request(false);
        r.set_method_body("POST", Some(Body::Unreplayable)).unwrap();
        assert_eq!(
            redirect(
                &mut r,
                &response(status, &[]),
                Some(url("https://a.test/new"))
            ),
            Err(FetchError::BodyNotReplayable)
        );
    }
    let mut r = request(false);
    r.set_method_body("POST", Some(Body::Unreplayable)).unwrap();
    redirect(&mut r, &response(303, &[]), Some(url("https://a.test/new"))).unwrap();
    assert!(r.body().is_none());
}
#[test]
fn redirect_fragment_inheritance_and_empty_override() {
    let mut r = Request::new(
        url("https://a.test/p#old"),
        url("https://a.test").origin().clone(),
    );
    redirect(&mut r, &response(302, &[]), Some(url("https://a.test/new"))).unwrap();
    assert_eq!(r.url().serialize(), "https://a.test/new#old");
    redirect(
        &mut r,
        &response(302, &[]),
        Some(url("https://a.test/new#")),
    )
    .unwrap();
    assert_eq!(r.url().serialize(), "https://a.test/new#");
}
#[test]
fn simple_and_non_simple_preflight_matrix() {
    for (method, name, value, expected) in [
        ("GET", "accept", "*/*", false),
        ("POST", "content-type", "text/plain;charset=UTF-8", false),
        ("POST", "content-type", "application/json", true),
        ("PUT", "accept", "*/*", true),
        ("GET", "x-custom", "1", true),
        ("GET", "range", "bytes=0-99", false),
    ] {
        let mut r = request(true);
        r.set_method_body(method, None).unwrap();
        r.headers.append(name, value).unwrap();
        assert_eq!(
            r.prepare().unwrap().preflight,
            expected,
            "{method}/{name}/{value}"
        );
    }
    let mut r = request(false);
    r.set_method_body("PUT", None).unwrap();
    assert!(!r.prepare().unwrap().preflight);
}
#[test]
fn safelist_values_and_length_boundaries() {
    for (name, value) in [
        ("accept", "a(b)"),
        ("accept-language", "en_US"),
        ("content-type", "text/plain, application/json"),
        ("content-type", "text/plain; charset=\"UTF-8\""),
        ("range", "bytes=-500"),
        ("range", "bytes=10-9"),
        ("range", "bytes=0-1,3-4"),
    ] {
        assert!(!safelisted_request_header(name, value));
    }
    assert!(safelisted_request_header("accept", &"a".repeat(128)));
    assert!(!safelisted_request_header("accept", &"a".repeat(129)));
    assert!(safelisted_request_header("range", "bytes=1-"));
}
#[test]
fn duplicate_content_type_cannot_bypass_preflight() {
    let mut r = request(true);
    r.headers
        .append("Content-Type", "application/json")
        .unwrap();
    r.headers.append("Content-Type", "text/plain").unwrap();
    assert!(r.prepare().unwrap().preflight);
}
#[test]
fn aggregate_safelist_limit_and_sorted_unique_names() {
    let mut h = HeaderList::default();
    for _ in 0..9 {
        h.append("accept", &"a".repeat(120)).unwrap();
    }
    h.append("Z-Extra", "x").unwrap();
    h.append("a-extra", "x").unwrap();
    h.append("A-extra", "x").unwrap();
    assert_eq!(
        unsafe_request_header_names(&h),
        vec!["a-extra", "accept", "z-extra"]
    );
}
#[test]
fn preflight_headers_are_sorted_and_credentials_omitted() {
    let mut r = request(true);
    r.credentials = CredentialsMode::Include;
    r.set_method_body("PUT", None).unwrap();
    r.headers.append("Z-Test", "1").unwrap();
    r.headers.append("Authorization", "secret").unwrap();
    let p = preflight(&r).unwrap().unwrap();
    assert_eq!(p.method, "OPTIONS");
    assert!(!p.credentials);
    assert_eq!(
        p.headers.get("access-control-request-headers").as_deref(),
        Some("authorization,z-test")
    );
    assert_eq!(p.headers.get("origin").as_deref(), Some("https://a.test"));
}
#[test]
fn forced_preflight_for_simple_request() {
    let mut r = request(true);
    r.use_cors_preflight = true;
    assert!(preflight(&r).unwrap().is_some());
}
#[test]
fn cors_credentials_and_allow_origin_matrix() {
    for credentials in [
        CredentialsMode::Omit,
        CredentialsMode::SameOrigin,
        CredentialsMode::Include,
    ] {
        for origin in [
            "*",
            "https://a.test",
            "https://a.test/",
            "https://wrong.test",
        ] {
            for acac in [None, Some("true"), Some("True")] {
                let mut r = request(true);
                r.credentials = credentials;
                let mut h = HeaderList::default();
                h.append("access-control-allow-origin", origin).unwrap();
                if let Some(v) = acac {
                    h.append("access-control-allow-credentials", v).unwrap();
                }
                let expected = if credentials == CredentialsMode::Include {
                    origin == "https://a.test" && acac == Some("true")
                } else {
                    origin == "*" || origin == "https://a.test"
                };
                assert_eq!(cors_check(&r, &h).is_ok(), expected);
            }
        }
    }
}
#[test]
fn duplicate_allow_origin_fails() {
    let r = request(true);
    let s = response(
        200,
        &[
            ("access-control-allow-origin", "https://a.test"),
            ("access-control-allow-origin", "https://a.test"),
        ],
    );
    assert!(cors_check(&r, &s.headers).is_err());
}
#[test]
fn preflight_wildcard_never_authorizes_authorization() {
    let mut r = request(true);
    r.set_method_body("PUT", None).unwrap();
    r.headers.append("Authorization", "x").unwrap();
    let s = response(
        204,
        &[
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "*"),
            ("access-control-allow-headers", "*"),
        ],
    );
    assert!(check_preflight(&r, &s).is_err());
    let mut s = s;
    s.headers
        .append("access-control-allow-headers", "Authorization")
        .unwrap();
    assert!(check_preflight(&r, &s).is_ok());
}
#[test]
fn credentialed_preflight_requires_explicit_methods_and_headers() {
    let mut r = request(true);
    r.credentials = CredentialsMode::Include;
    r.set_method_body("PUT", None).unwrap();
    r.headers.append("x-test", "x").unwrap();
    let mut s = response(
        200,
        &[
            ("access-control-allow-origin", "https://a.test"),
            ("access-control-allow-credentials", "true"),
            ("access-control-allow-methods", "*"),
            ("access-control-allow-headers", "*"),
        ],
    );
    assert!(check_preflight(&r, &s).is_err());
    s.headers
        .append("access-control-allow-methods", "PUT")
        .unwrap();
    s.headers
        .append("access-control-allow-headers", "X-Test")
        .unwrap();
    assert!(check_preflight(&r, &s).is_ok());
    s.status = 302;
    assert!(check_preflight(&r, &s).is_err());
}
#[test]
fn preflight_method_names_case_sensitive() {
    let mut r = request(true);
    r.set_method_body("PUT", None).unwrap();
    let s = response(
        200,
        &[
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "put"),
        ],
    );
    assert!(check_preflight(&r, &s).is_err());
}
#[test]
fn mode_and_taint_decisions() {
    let mut r = request(true);
    r.mode = RequestMode::SameOrigin;
    assert_eq!(r.prepare(), Err(FetchError::SameOrigin));
    r.mode = RequestMode::NoCors;
    assert_eq!(r.prepare().unwrap().taint, ResponseType::Opaque);
    let mut r = request(true);
    r.mode = RequestMode::Navigate;
    assert_eq!(r.prepare().unwrap().taint, ResponseType::Basic);
    let mut r = request(true);
    assert_eq!(r.prepare().unwrap().taint, ResponseType::Cors);
    let mut r = request(false);
    assert_eq!(r.prepare().unwrap().taint, ResponseType::Basic);
}
#[test]
fn no_cors_rejects_unsafe_requests_and_manual_redirect() {
    let mut r = request(true);
    r.mode = RequestMode::NoCors;
    r.set_method_body("PUT", None).unwrap();
    assert_eq!(r.prepare(), Err(FetchError::NoCors));
    r.set_method_body("GET", None).unwrap();
    r.redirect = RedirectMode::Manual;
    assert_eq!(r.prepare(), Err(FetchError::NoCors));
    r.redirect = RedirectMode::Follow;
    r.headers.append("x-test", "x").unwrap();
    assert_eq!(r.prepare(), Err(FetchError::NoCors));
}
#[test]
fn credential_modes_apply_per_hop() {
    let mut r = request(false);
    assert!(r.prepare().unwrap().credentials);
    r.credentials = CredentialsMode::Omit;
    assert!(!r.prepare().unwrap().credentials);
    let mut r = request(true);
    assert!(!r.prepare().unwrap().credentials);
    r.credentials = CredentialsMode::Include;
    assert!(r.prepare().unwrap().credentials);
}
#[test]
fn cors_taint_stays_on_return_to_initiator() {
    let mut r = request(true);
    r.prepare().unwrap();
    redirect(
        &mut r,
        &response(302, &[("access-control-allow-origin", "*")]),
        Some(url("https://a.test/final")),
    )
    .unwrap();
    assert_eq!(r.prepare().unwrap().taint, ResponseType::Cors);
    assert!(!r.credentials_allowed());
    assert_eq!(cors_origin(&r), "null");
}
#[test]
fn basic_filter_hides_cookies_but_keeps_custom_header() {
    let r = request(false);
    let s = response(
        200,
        &[
            ("set-cookie", "a=1"),
            ("set-cookie2", "b=2"),
            ("x-private", "yes"),
        ],
    );
    let f = filter_response(&r, &s).unwrap();
    assert!(f.headers.get("set-cookie").is_none());
    assert!(f.headers.get("set-cookie2").is_none());
    assert!(f.headers.get("x-private").is_some());
    assert_eq!(f.body, s.body);
}
#[test]
fn cors_filter_exposes_only_safe_and_listed_headers() {
    let r = request(true);
    let s = response(
        200,
        &[
            ("access-control-allow-origin", "*"),
            ("content-type", "text/plain"),
            ("x-secret", "no"),
            ("x-public", "yes"),
            ("access-control-expose-headers", "X-Public, Set-Cookie"),
            ("set-cookie", "secret"),
        ],
    );
    let f = filter_response(&r, &s).unwrap();
    assert_eq!(f.response_type, ResponseType::Cors);
    assert!(f.headers.get("content-type").is_some());
    assert!(f.headers.get("x-public").is_some());
    assert!(f.headers.get("x-secret").is_none());
    assert!(f.headers.get("set-cookie").is_none());
}
#[test]
fn exposed_wildcard_is_literal_with_credentials() {
    let mut r = request(true);
    let s = response(
        200,
        &[
            ("access-control-allow-origin", "https://a.test"),
            ("access-control-allow-credentials", "true"),
            ("access-control-expose-headers", "*"),
            ("x-secret", "x"),
            ("set-cookie", "x"),
        ],
    );
    assert!(filter_response(&r, &s)
        .unwrap()
        .headers
        .get("x-secret")
        .is_some());
    r.credentials = CredentialsMode::Include;
    assert!(filter_response(&r, &s)
        .unwrap()
        .headers
        .get("x-secret")
        .is_none());
}
#[test]
fn opaque_filters_hide_everything() {
    let mut r = request(true);
    r.mode = RequestMode::NoCors;
    let mut s = response(200, &[("x-secret", "value"), ("content-type", "image/png")]);
    s.body = Some(b"\x89PNG\r\n\x1a\nsecret".to_vec());
    assert_eq!(
        filter_response(&r, &s).unwrap(),
        Response::hidden(ResponseType::Opaque)
    );
}
#[test]
fn cors_failure_never_returns_body() {
    assert_eq!(
        filter_response(&request(true), &response(200, &[])),
        Err(FetchError::Cors)
    );
}
#[test]
fn referrer_policy_matrix() {
    let source = url("https://a.test/path?q=1#secret");
    let same = url("https://a.test/other");
    let cross = url("https://b.test/");
    let down = url("http://b.test/");
    use ReferrerPolicy::*;
    let full = Some("https://a.test/path?q=1".to_owned());
    let origin = Some("https://a.test/".to_owned());
    for (p, expect_same, expect_cross, expect_down) in [
        (NoReferrer, None, None, None),
        (NoReferrerWhenDowngrade, full.clone(), full.clone(), None),
        (SameOrigin, full.clone(), None, None),
        (Origin, origin.clone(), origin.clone(), origin.clone()),
        (StrictOrigin, origin.clone(), origin.clone(), None),
        (
            OriginWhenCrossOrigin,
            full.clone(),
            origin.clone(),
            origin.clone(),
        ),
        (
            StrictOriginWhenCrossOrigin,
            full.clone(),
            origin.clone(),
            None,
        ),
        (UnsafeUrl, full.clone(), full.clone(), full.clone()),
    ] {
        assert_eq!(determine_referrer(Some(&source), &same, p), expect_same);
        assert_eq!(determine_referrer(Some(&source), &cross, p), expect_cross);
        assert_eq!(determine_referrer(Some(&source), &down, p), expect_down);
    }
}
#[test]
fn long_referrer_reduces_to_origin_and_absent_stays_absent() {
    let source = url(&format!("https://a.test/{}", "x".repeat(4096)));
    let target = url("https://a.test/");
    assert_eq!(
        determine_referrer(Some(&source), &target, ReferrerPolicy::UnsafeUrl).as_deref(),
        Some("https://a.test/")
    );
    assert!(determine_referrer(None, &target, ReferrerPolicy::UnsafeUrl).is_none());
}
#[test]
fn redirect_referrer_policy_last_recognized_token_wins() {
    let mut r = request(false);
    r.referrer = Some(url("https://a.test/private#x"));
    redirect(
        &mut r,
        &response(
            302,
            &[("referrer-policy", "unsafe-url, no-referrer, nonsense")],
        ),
        Some(url("https://b.test/")),
    )
    .unwrap();
    assert_eq!(r.referrer_policy, ReferrerPolicy::NoReferrer);
    assert!(r.prepare().unwrap().referrer.is_none());
}
#[test]
fn redirect_checks_cors_before_mutating_target() {
    let mut r = request(true);
    let before = r.url().clone();
    assert_eq!(
        redirect(&mut r, &response(302, &[]), Some(url("https://a.test/"))),
        Err(FetchError::Cors)
    );
    assert_eq!(r.url(), &before);
}
#[test]
fn navigation_manual_returns_controller_handoff() {
    let mut r = request(true);
    r.mode = RequestMode::Navigate;
    r.redirect = RedirectMode::Manual;
    assert_eq!(
        redirect(&mut r, &response(302, &[]), Some(url("https://c.test/"))).unwrap(),
        RedirectAction::NavigationPending
    );
    assert_eq!(r.url_list().len(), 1);
}
#[test]
fn redirect_taint_is_updated_without_a_separate_prepare_call() {
    let mut r = request(true);
    redirect(
        &mut r,
        &response(302, &[("access-control-allow-origin", "*")]),
        Some(url("https://a.test/final")),
    )
    .unwrap();
    assert_eq!(
        filter_response(&r, &response(200, &[])),
        Err(FetchError::Cors)
    );
}
#[test]
fn redirect_referrer_cannot_regain_stripped_path() {
    let mut r = request(true);
    r.referrer = Some(url("https://a.test/private"));
    redirect(
        &mut r,
        &response(
            302,
            &[
                ("access-control-allow-origin", "*"),
                ("referrer-policy", "unsafe-url"),
            ],
        ),
        Some(url("https://c.test/")),
    )
    .unwrap();
    assert_eq!(
        r.prepare().unwrap().referrer.as_deref(),
        Some("https://a.test/")
    );
}

#[test]
fn parsed_url_adoption_unifies_origins_and_cors_serialization() {
    let a = HttpUrl::from_parsed(
        amazing_net::ParsedUrl::parse("https://u:p@0x7f000001:443/é").unwrap(),
    )
    .unwrap();
    assert_eq!(a.origin(), url("https://127.0.0.1/").origin());
    assert_eq!(a.origin().serialize(), "https://127.0.0.1");
    assert_eq!(a.serialize(), "https://u:p@127.0.0.1/%C3%A9");
    let mut r = Request::new(url("https://b.test/"), a.origin().clone());
    r.prepare().unwrap();
    assert!(cors_check(
        &r,
        &response(200, &[("access-control-allow-origin", "https://127.0.0.1")]).headers
    )
    .is_ok());
    assert!(HttpUrl::from_parsed(amazing_net::ParsedUrl::parse("file:///x").unwrap()).is_err());
}
#[test]
fn raw_location_resolution_preserves_query_and_fragment_presence() {
    let mut r = Request::new(
        url("https://a.test/a/b#old"),
        url("https://a.test/").origin().clone(),
    );
    assert_eq!(
        redirect_location(&mut r, &response(302, &[]), Some("../%2e/é?")),
        Ok(RedirectAction::Follow)
    );
    assert_eq!(r.url().serialize(), "https://a.test/%C3%A9?#old");
    redirect_location(&mut r, &response(302, &[]), Some("#")).unwrap();
    assert_eq!(r.url().serialize(), "https://a.test/%C3%A9?#");
}
#[test]
fn location_absence_is_distinct_from_invalid_and_unsupported_targets() {
    let mut r = request(false);
    assert_eq!(
        redirect_location(&mut r, &response(302, &[]), None),
        Ok(RedirectAction::NotRedirect)
    );
    for input in ["https://x:65536/", "https://é.test/", "file:///x"] {
        assert_eq!(
            redirect_location(&mut r, &response(302, &[]), Some(input)),
            Err(FetchError::InvalidUrl)
        );
        assert_eq!(r.url_list().len(), 1);
    }
    r.redirect = RedirectMode::Manual;
    assert!(matches!(
        redirect_location(&mut r, &response(302, &[]), Some("https://x:65536/")),
        Ok(RedirectAction::Manual(_))
    ));
}
#[test]
fn cors_redirects_reject_credentials_even_when_chain_returns_to_origin() {
    let mut r = request(false);
    assert_eq!(
        redirect_location(&mut r, &response(302, &[]), Some("https://u:p@b.test/")),
        Err(FetchError::Cors)
    );
    assert_eq!(r.url_list().len(), 1);
    // Same-origin credentials remain represented, rather than rejected by URL syntax.
    redirect_location(&mut r, &response(302, &[]), Some("https://u:p@a.test/next")).unwrap();
    assert!(r.url().includes_credentials());
    redirect_location(&mut r, &response(302, &[]), Some("https://b.test/")).unwrap();
    assert_eq!(
        redirect_location(
            &mut r,
            &response(302, &[("access-control-allow-origin", "https://a.test")]),
            Some("https://u:p@a.test/")
        ),
        Err(FetchError::Cors)
    );
}
#[test]
fn referrers_strip_credentials_and_fragments_but_keep_encoded_path() {
    let source = url("https://u:p@a.test/é?q#private");
    assert_eq!(
        determine_referrer(
            Some(&source),
            &url("https://a.test/"),
            ReferrerPolicy::UnsafeUrl
        ),
        Some("https://a.test/%C3%A9?q".into())
    );
    assert!(source.without_fragment().contains("u:p@"));
}
