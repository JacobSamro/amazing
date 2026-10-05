use amazing_fetch::{
    CspDestination, CspDisposition, CspParseError, CspPolicyList, HeaderList, HttpUrl, Request,
    RequestMode,
};

fn url(s: &str) -> HttpUrl {
    HttpUrl::parse(s).unwrap()
}
fn request(target: &str, destination: CspDestination, csp: &str, report: &str) -> Request {
    let owner = url("https://site.example/");
    let mut headers = HeaderList::default();
    if !csp.is_empty() {
        headers.append("Content-Security-Policy", csp).unwrap();
    }
    if !report.is_empty() {
        headers
            .append("Content-Security-Policy-Report-Only", report)
            .unwrap();
    }
    let mut r = Request::new(url(target), owner.origin().clone());
    r.csp = CspPolicyList::from_headers(&headers).unwrap();
    r.csp_destination = destination;
    r
}
#[test]
fn policy_list_dispositions_and_bounds() {
    let r = request(
        "https://a.example/x",
        CspDestination::Image,
        "default-src 'none', img-src https://a.example; img-src 'none'",
        "img-src 'none'",
    );
    assert_eq!(r.csp.policies.len(), 3);
    let check = r.csp.pre_request(&r);
    assert!(check.blocked);
    assert_eq!(check.violations.len(), 2);
    assert_eq!(check.violations[0].disposition, CspDisposition::Enforce);
    assert_eq!(check.violations[1].disposition, CspDisposition::ReportOnly);
    let mut headers = HeaderList::default();
    headers
        .append("Content-Security-Policy", &"x".repeat(65_537))
        .unwrap();
    assert_eq!(
        CspPolicyList::from_headers(&headers),
        Err(CspParseError::TooLong)
    );
    let mut headers = HeaderList::default();
    headers
        .append("Content-Security-Policy", &"img-src a;".repeat(257))
        .unwrap();
    assert_eq!(
        CspPolicyList::from_headers(&headers),
        Err(CspParseError::TooManyDirectives)
    );
}
#[test]
fn fallback_chains_and_report_only() {
    let cases = [
        (
            CspDestination::ScriptElement,
            "script-src 'none'",
            "script-src",
        ),
        (
            CspDestination::StyleElement,
            "style-src 'none'",
            "style-src",
        ),
        (CspDestination::Frame, "child-src 'none'", "child-src"),
        (CspDestination::Worker, "script-src 'none'", "script-src"),
        (
            CspDestination::Manifest,
            "default-src 'none'",
            "default-src",
        ),
    ];
    for (dest, policy, expected) in cases {
        let r = request("https://else.example/x", dest, policy, "");
        let result = r.csp.pre_request(&r);
        assert!(result.blocked, "{policy}");
        assert_eq!(result.violations[0].directive, expected);
    }
    let mut r = request(
        "https://else.example/x",
        CspDestination::Image,
        "",
        "img-src 'none'",
    );
    assert!(!r.csp.pre_request(&r).blocked);
    assert!(r.prepare().is_ok());
    assert_eq!(r.csp_violations.len(), 1);
}
#[test]
fn wildcard_host_port_path_and_scheme_upgrade() {
    let policy = "img-src http://*.example.com:*/assets/";
    for target in [
        "http://a.example.com:456/assets/x",
        "https://deep.a.example.com:987/assets/x",
    ] {
        let r = request(target, CspDestination::Image, policy, "");
        assert!(!r.csp.pre_request(&r).blocked, "{target}");
    }
    for target in [
        "https://example.com:987/assets/x",
        "https://badexample.com:987/assets/x",
        "https://a.example.com:987/other/x",
    ] {
        let r = request(target, CspDestination::Image, policy, "");
        assert!(r.csp.pre_request(&r).blocked, "{target}");
    }
    let r = request(
        "https://site.example/x",
        CspDestination::Image,
        "img-src 'self'",
        "",
    );
    assert!(!r.csp.pre_request(&r).blocked);
    let ws = amazing_net::ParsedUrl::parse("wss://socket.example/live").unwrap();
    assert!(amazing_fetch::csp_source_matches(
        "ws:",
        &ws,
        &r.csp_self_origin,
        false
    ));
    assert!(!amazing_fetch::csp_source_matches(
        "https://other.example",
        &ws,
        &r.csp_self_origin,
        false
    ));
}
#[test]
fn redirected_path_is_ignored_but_host_is_checked() {
    let mut r = request(
        "https://a.example/allowed/file",
        CspDestination::Image,
        "img-src https://a.example/allowed/",
        "",
    );
    r.mode = RequestMode::NoCors;
    assert!(!r.csp.pre_request(&r).blocked);
    let response = amazing_fetch::InternalResponse {
        request_includes_credentials: true,
        status: 302,
        status_text: String::new(),
        headers: HeaderList::default(),
        body: None,
    };
    amazing_fetch::redirect(&mut r, &response, Some(url("https://a.example/other/file"))).unwrap();
    assert!(!r.csp.pre_request(&r).blocked);
    assert!(!r.csp.post_request(&r, r.url()).blocked);
    assert!(
        r.csp
            .post_request(&r, &url("https://other.example/file"))
            .blocked
    );
}
#[test]
fn script_nonce_integrity_and_strict_dynamic() {
    let mut r = request(
        "https://bad.example/x.js",
        CspDestination::ScriptElement,
        "script-src 'nonce-abc'",
        "",
    );
    r.csp_nonce = Some("abc".into());
    assert!(!r.csp.pre_request(&r).blocked);
    r.csp_nonce = None;
    assert!(r.csp.pre_request(&r).blocked);
    let mut r = request(
        "https://bad.example/x.js",
        CspDestination::ScriptElement,
        "script-src 'sha256-xyz'",
        "",
    );
    r.csp_integrity.push("'sha256-xyz'".into());
    assert!(!r.csp.pre_request(&r).blocked);
    let mut r = request(
        "https://bad.example/x.js",
        CspDestination::ScriptElement,
        "script-src 'strict-dynamic' https://bad.example",
        "",
    );
    assert!(!r.csp.pre_request(&r).blocked);
    r.csp_parser_inserted = true;
    assert!(r.csp.pre_request(&r).blocked);
}
#[test]
fn fetch_pre_and_post_checks_return_typed_violations() {
    let mut r = request(
        "https://a.example/x",
        CspDestination::Image,
        "img-src https://a.example",
        "img-src 'none'",
    );
    r.mode = RequestMode::NoCors;
    r.prepare().unwrap();
    assert_eq!(r.csp_violations.len(), 1);
    assert_eq!(r.csp_violations[0].blocked_url, "https://a.example/x");
    let post = r.csp.post_request(&r, &url("https://b.example/x"));
    assert!(post.blocked);
    assert_eq!(post.violations[0].directive, "img-src");
    assert_eq!(post.violations[0].blocked_url, "https://b.example/x");
    let response = amazing_fetch::InternalResponse {
        request_includes_credentials: true,
        status: 200,
        status_text: String::new(),
        headers: HeaderList::default(),
        body: None,
    };
    assert!(matches!(
        amazing_fetch::check_response_security(
            &r,
            &response,
            &amazing_fetch::psl::PublicSuffixList
        ),
        Ok(())
    ));
    let mut blocked = request(
        "https://a.example/x",
        CspDestination::Image,
        "img-src 'none'",
        "",
    );
    assert!(matches!(
        blocked.prepare(),
        Err(amazing_fetch::FetchError::Csp(_))
    ));
}
