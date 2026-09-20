use amazing_fetch::cookies::*;
use amazing_fetch::{HttpUrl, Origin, RegistrableDomain};
use std::cell::Cell;
use std::rc::Rc;
fn url(s: &str) -> HttpUrl {
    HttpUrl::parse(s).unwrap()
}
fn context(u: &HttpUrl) -> CookieAccess<'_> {
    CookieAccess {
        url: u,
        site_for_cookies: CookieSite::Origin(u.origin().clone()),
        http_api: true,
        top_level_navigation: false,
        method: "GET",
    }
}
fn only<'a, P: RegistrableDomain>(s: &'a mut CookieStore<P>, now: i64) -> &'a Cookie {
    assert_eq!(s.cookies(now).len(), 1);
    &s.cookies(now)[0]
}
/// Fixed test fixture, not a bundled or production PSL.
#[derive(Clone, Default)]
struct Domains;
impl RegistrableDomain for Domains {
    fn registrable_domain(&self, host: &str) -> Option<String> {
        for domain in ["example.test", "other.test", "tenant.host.test"] {
            if domain_matches(host, domain) {
                return Some(domain.into());
            }
        }
        None
    }
    fn is_public_suffix(&self, host: &str) -> Option<bool> {
        Some(matches!(host, "test" | "host.test" | "co.uk"))
    }
}
fn store() -> CookieStore<Domains> {
    CookieStore::new(Domains, CookieLimits::default()).unwrap()
}
#[test]
fn parsing_trims_only_space_and_tab_preserves_quotes_equals_and_octets() {
    let u = url("https://example.test/a/b");
    let c = context(&u);
    let mut s = store();
    s.set_cookie(
        b" \tkey \t= \t\"a=b%20\xff\"\t ; pAtH= /a \t; Secure=ignored; HttpOnly",
        &c,
        0,
    )
    .unwrap();
    let v = only(&mut s, 0);
    assert_eq!(v.name(), b"key");
    assert_eq!(v.value(), b"\"a=b%20\xff\"");
    assert_eq!(v.path(), b"/a");
    assert!(v.secure() && v.http_only());
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"key=\"a=b%20\xff\"");
}
#[test]
fn absent_equals_means_unnamed_cookie_and_empty_name_value_is_rejected() {
    let u = url("https://example.test/");
    let c = context(&u);
    let mut s = store();
    for input in [b"".as_slice(), b"=", b" \t=\t "] {
        assert_eq!(s.set_cookie(input, &c, 0), Err(CookieError::Empty));
    }
    s.set_cookie(b"unnamed", &c, 0).unwrap();
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"unnamed");
    s.set_cookie(b"empty=", &c, 0).unwrap();
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"unnamed; empty=");
}
#[test]
fn control_characters_anywhere_reject_entire_field() {
    let u = url("https://example.test/");
    let c = context(&u);
    let mut s = store();
    for byte in (0..=31).chain([127]).filter(|b| *b != 9) {
        assert_eq!(
            s.set_cookie(&[b'a', b'=', b'b', b';', b'x', byte], &c, 0),
            Err(CookieError::ControlCharacter)
        );
    }
    assert!(s.cookies(0).is_empty());
}
#[test]
fn byte_size_limits_ignore_only_the_oversize_attribute() {
    let u = url("https://example.test/a/b");
    let c = context(&u);
    let mut s = store();
    let mut v = b"x=".to_vec();
    v.extend(vec![b'a'; 4095]);
    s.set_cookie(&v, &c, 0).unwrap();
    v.push(b'a');
    assert_eq!(s.set_cookie(&v, &c, 0), Err(CookieError::TooLarge));
    let v = format!(
        "x=y; Path=/ok; Path=/{}; Domain={}",
        "a".repeat(1024),
        "a".repeat(1025)
    );
    s.set_cookie(v.as_bytes(), &c, 0).unwrap();
    assert!(s
        .cookies(0)
        .iter()
        .any(|v| v.path() == b"/ok" && v.host_only()));
    assert_eq!(
        s.set_cookie(&vec![b'a'; 65537], &c, 0),
        Err(CookieError::TooLarge)
    );
}
#[test]
fn duplicate_attributes_use_last_valid_value() {
    let u = url("https://example.test/a/b");
    let c = context(&u);
    let mut s = store();
    s.set_cookie(b"a=b; Max-Age=10; Max-Age=bad; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Path=/first; Path=bad; SameSite=Strict; SameSite=unknown",&c,100).unwrap();
    let v = only(&mut s, 100);
    assert_eq!(v.expires(), Some(110));
    assert_eq!(v.path(), b"/a");
    assert_eq!(v.same_site(), SameSite::Default);
}
#[test]
fn max_age_overflow_clamps_without_wrapping_and_nonpositive_deletes() {
    let u = url("https://example.test/");
    let c = context(&u);
    let mut s = store();
    let input = format!("a=b; Max-Age={}", "9".repeat(1000));
    s.set_cookie(input.as_bytes(), &c, 100).unwrap();
    assert_eq!(only(&mut s, 100).expires(), Some(100 + MAX_LIFETIME));
    for age in ["0", "-0", "-1", "-99999999999999999999999999"] {
        s.set_cookie(b"a=b", &c, 100).unwrap();
        s.set_cookie(format!("a=deleted; Max-Age={age}").as_bytes(), &c, 100)
            .unwrap();
        assert!(s.cookies(100).is_empty());
    }
    for age in ["", "+1", "-", "1x", "1 2"] {
        s.set_cookie(format!("a=b; Max-Age={age}").as_bytes(), &c, 0)
            .unwrap();
        assert!(!only(&mut s, 0).persistent());
    }
}
#[test]
fn legacy_date_forms_and_delimiters_are_interpreted_in_utc() {
    let u = url("https://example.test/");
    let c = context(&u);
    for date in [
        "Thu, 01 Jan 1970 00:00:10 GMT",
        "Thursday, 01-Jan-70 00:00:10 GMT",
        "Thu Jan  1 00:00:10 1970",
        "garbage@01/jANuary/1970@0:0:10GMT",
    ] {
        let mut s = store();
        s.set_cookie(format!("a=b; Expires={date}").as_bytes(), &c, 0)
            .unwrap();
        assert_eq!(only(&mut s, 0).expires(), Some(10), "{date}");
        assert!(s.cookies(10).is_empty());
    }
}
#[test]
fn dates_reject_impossible_values_and_preserve_previous_valid_attribute() {
    let u = url("https://example.test/");
    let c = context(&u);
    for date in [
        "31 Feb 2024 00:00:00",
        "29 Feb 2023 00:00:00",
        "01 Jan 1600 00:00:00",
        "01 Jan 1970 24:00:00",
        "01 Jan 1970 00:60:00",
        "01 Jan 1970 00:00:60",
        "001 Jan 1970 00:00:00",
        "1 Jan 19700 00:00:00",
    ] {
        let mut s = store();
        s.set_cookie(
            format!("a=b; Expires=1 Jan 1970 00:00:10; Expires={date}").as_bytes(),
            &c,
            0,
        )
        .unwrap();
        assert_eq!(only(&mut s, 0).expires(), Some(10), "{date}");
    }
}
#[test]
fn two_digit_year_pivot_leap_days_and_400_day_expiry_cap() {
    let u = url("https://example.test/");
    let c = context(&u);
    for (date, now, expected) in [
        ("1 Jan 69 00:00:00", 3124223990, 3124224000),
        ("1 Jan 70 00:00:10", 0, 10),
        ("29 Feb 2000 00:00:00", 951782390, 951782400),
        ("1 Jan 9999 00:00:00", 0, MAX_LIFETIME),
    ] {
        let mut s = store();
        s.set_cookie(format!("a=b; Expires={date}").as_bytes(), &c, now)
            .unwrap();
        assert_eq!(only(&mut s, now).expires(), Some(expected), "{date}");
    }
}
#[test]
fn pre_epoch_cookie_dates_and_saturating_future_clock() {
    let u = url("https://example.test/");
    let c = context(&u);
    let mut s = store();
    s.set_cookie(b"a=b; Expires=1 Jan 1601 00:00:00", &c, -11644473601)
        .unwrap();
    assert_eq!(only(&mut s, -11644473601).expires(), Some(-11644473600));
    s.set_cookie(b"a=b; Max-Age=99", &c, i64::MAX - 1).unwrap();
    assert_eq!(only(&mut s, i64::MAX - 1).expires(), Some(i64::MAX));
}
#[test]
fn default_paths_and_matching_are_byte_exact() {
    for (path, expected) in [
        ("", "/"),
        ("x", "/"),
        ("/", "/"),
        ("/a", "/"),
        ("/a/", "/a"),
        ("/a/b", "/a"),
    ] {
        assert_eq!(default_path(path.as_bytes()), expected.as_bytes());
    }
    assert!(path_matches(b"/a/b", b"/a"));
    assert!(path_matches(b"/a/b", b"/a/"));
    assert!(!path_matches(b"/abc", b"/a"));
    assert!(!path_matches(b"/a%2fb", b"/a"));
    assert!(!path_matches(b"/A", b"/a"));
}
#[test]
fn conservative_default_accepts_host_only_and_rejects_unproven_domains() {
    let u = url("https://a.example.test/");
    let c = context(&u);
    let mut s = CookieStore::default();
    s.set_cookie(b"a=b", &c, 0).unwrap();
    for input in [
        b"a=b; Domain=example.test".as_slice(),
        b"a=b; Domain=a.example.test",
    ] {
        assert_eq!(
            s.set_cookie(input, &c, 0),
            Err(CookieError::UnknownPublicSuffix)
        );
    }
    assert_eq!(
        s.cookie_header(&context(&url("https://b.a.example.test/")), 0)
            .unwrap(),
        b""
    );
}
#[test]
fn provider_enables_boundary_checked_domain_cookies() {
    let u = url("https://a.example.test/");
    let mut s = store();
    s.set_cookie(b"a=b; Domain=.EXAMPLE.TEST", &context(&u), 0)
        .unwrap();
    assert!(!only(&mut s, 0).host_only());
    assert_eq!(
        s.cookie_header(&context(&url("https://b.example.test/")), 0)
            .unwrap(),
        b"a=b"
    );
    assert!(s
        .cookie_header(&context(&url("https://notexample.test/")), 0)
        .unwrap()
        .is_empty());
    for d in [
        "other.test",
        "..example.test",
        "example.test.",
        "%65xample.test",
        "é.test",
    ] {
        assert!(s
            .set_cookie(format!("a=b; Domain={d}").as_bytes(), &context(&u), 0)
            .is_err());
    }
}
#[test]
fn public_suffix_rejects_children_but_exact_host_converts_to_host_only() {
    let mut s = store();
    let u = url("https://example.test/");
    assert_eq!(
        s.set_cookie(b"a=b; Domain=test", &context(&u), 0),
        Err(CookieError::Domain)
    );
    let root = url("https://test/");
    s.set_cookie(b"a=b; Domain=test", &context(&root), 0)
        .unwrap();
    assert!(only(&mut s, 0).host_only());
}
#[test]
fn public_suffix_is_rechecked_when_provider_data_changes() {
    struct Changing(Rc<Cell<bool>>);
    impl RegistrableDomain for Changing {
        fn registrable_domain(&self, _: &str) -> Option<String> {
            None
        }
        fn is_public_suffix(&self, _: &str) -> Option<bool> {
            Some(self.0.get())
        }
    }
    let state = Rc::new(Cell::new(false));
    let mut s = CookieStore::new(Changing(state.clone()), CookieLimits::default()).unwrap();
    let u = url("https://example.test/");
    let c = context(&u);
    s.set_cookie(b"a=b; Domain=example.test", &c, 0).unwrap();
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"a=b");
    state.set(true);
    assert!(s.cookie_header(&c, 0).unwrap().is_empty());
}
#[test]
fn ip_literals_never_suffix_match_and_ports_do_not_partition_cookies() {
    assert!(domain_matches("127.0.0.1", "127.0.0.1"));
    assert!(!domain_matches("127.0.0.1", "0.0.1"));
    assert!(!domain_matches("[::1]", "1]"));
    let mut s = CookieStore::default();
    let u = url("https://127.1:444/");
    s.set_cookie(b"a=b", &context(&u), 0).unwrap();
    assert_eq!(
        s.cookie_header(&context(&url("https://127.0.0.1:443/")), 0)
            .unwrap(),
        b"a=b"
    );
}
#[test]
fn canonical_cookie_host_rejects_unproven_alabels_and_invalid_labels() {
    let mut s = CookieStore::default();
    for input in [
        "https://a_b.test/",
        "https://a..test/",
        "https://xn--bad.test/",
        "https://-a.test/",
    ] {
        let u = url(input);
        assert_eq!(
            s.set_cookie(b"a=b", &context(&u), 0),
            Err(CookieError::InvalidHost)
        );
    }
}
#[test]
fn secure_and_http_only_enforced_on_both_set_and_retrieval() {
    let https = url("https://example.test/");
    let http = url("http://example.test/");
    let mut s = store();
    assert_eq!(
        s.set_cookie(b"a=b; Secure", &context(&http), 0),
        Err(CookieError::Secure)
    );
    let mut script = context(&https);
    script.http_api = false;
    assert_eq!(
        s.set_cookie(b"a=b; HttpOnly", &script, 0),
        Err(CookieError::HttpOnly)
    );
    s.set_cookie(b"a=b; Secure; HttpOnly", &context(&https), 0)
        .unwrap();
    assert!(s.cookie_header(&script, 0).unwrap().is_empty());
    assert!(s.cookie_header(&context(&http), 0).unwrap().is_empty());
    assert_eq!(
        s.set_cookie(b"a=gone; Max-Age=0", &script, 0),
        Err(CookieError::HttpOnly)
    );
    assert_eq!(s.cookie_header(&context(&https), 0).unwrap(), b"a=b");
}
#[test]
fn insecure_overlay_check_is_asymmetric_in_path_and_covers_deletion() {
    let secure = url("https://example.test/login");
    let insecure = url("http://example.test/login");
    let mut s = store();
    s.set_cookie(b"a=secret; Secure; Path=/login", &context(&secure), 0)
        .unwrap();
    for path in ["/login", "/login/en"] {
        assert_eq!(
            s.set_cookie(
                format!("a=gone; Max-Age=0; Path={path}").as_bytes(),
                &context(&insecure),
                0
            ),
            Err(CookieError::Secure)
        );
    }
    s.set_cookie(b"a=other; Path=/", &context(&insecure), 0)
        .unwrap();
    assert_eq!(
        s.cookie_header(&context(&secure), 0).unwrap(),
        b"a=secret; a=other"
    );
}
#[test]
fn prefixes_are_case_insensitive_and_unnamed_values_cannot_spoof_them() {
    let u = url("https://example.test/a/b");
    let c = context(&u);
    let mut s = store();
    for bad in [
        "__Secure-a=b",
        "__sEcUrE-a=b",
        "__Host-a=b; Secure",
        "__Host-a=b; Secure; Path=/a",
        "__Host-a=b; Secure; Path=/; Domain=example.test",
        "__Host-a=b; Path=/",
        "=__Secure-a=b; Secure",
        "__hOsT-a=b",
    ] {
        assert!(s.set_cookie(bad.as_bytes(), &c, 0).is_err(), "{bad}");
    }
    s.set_cookie(b"__Host-a=b; Secure; Path=/", &c, 0).unwrap();
    s.set_cookie(b"__sEcUrE-a=b; Secure", &c, 0).unwrap();
    assert_eq!(s.cookies(0).len(), 2);
}
#[test]
fn samesite_none_requires_secure_and_allows_cross_site_subresources() {
    let u = url("https://example.test/");
    let mut c = context(&u);
    c.site_for_cookies = CookieSite::CrossSite;
    let mut s = store();
    assert_eq!(
        s.set_cookie(b"a=b; SameSite=None", &c, 0),
        Err(CookieError::Secure)
    );
    s.set_cookie(b"a=b; SameSite=None; Secure", &c, 0).unwrap();
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"a=b");
}
#[test]
fn cross_site_creation_requires_top_level_http_navigation_unless_none() {
    let u = url("https://example.test/");
    let mut c = context(&u);
    c.site_for_cookies = CookieSite::CrossSite;
    let mut s = store();
    for mode in ["Strict", "Lax", "unknown"] {
        assert_eq!(
            s.set_cookie(format!("a=b; SameSite={mode}").as_bytes(), &c, 0),
            Err(CookieError::SameSite)
        );
    }
    c.top_level_navigation = true;
    s.set_cookie(b"a=b; SameSite=Strict", &c, 0).unwrap();
    assert!(s.cookie_header(&c, 0).unwrap().is_empty());
    c.http_api = false;
    assert_eq!(
        s.set_cookie(b"a=b; SameSite=Lax", &c, 0),
        Err(CookieError::SameSite)
    );
}
#[test]
fn strict_lax_default_and_unsafe_grace_have_distinct_retrieval_rules() {
    let u = url("https://example.test/");
    let mut s = store();
    let mut c = context(&u);
    for cookie in [
        "strict=1; SameSite=Strict",
        "lax=1; SameSite=Lax",
        "default=1",
    ] {
        s.set_cookie(cookie.as_bytes(), &c, 100).unwrap();
    }
    c.site_for_cookies = CookieSite::CrossSite;
    assert!(s.cookie_header(&c, 100).unwrap().is_empty());
    c.top_level_navigation = true;
    assert_eq!(s.cookie_header(&c, 100).unwrap(), b"lax=1; default=1");
    c.method = "POST";
    assert_eq!(s.cookie_header(&c, 220).unwrap(), b"default=1");
    assert!(s.cookie_header(&c, 221).unwrap().is_empty());
    c.method = "get";
    assert!(s.cookie_header(&c, 221).unwrap().is_empty());
    c.method = "GET";
    c.http_api = false;
    assert!(s.cookie_header(&c, 100).unwrap().is_empty());
}
#[test]
fn provider_controls_schemeful_site_and_default_does_not_guess() {
    let u = url("https://a.example.test/");
    let sibling = url("https://b.example.test/");
    let mut c = context(&sibling);
    c.site_for_cookies = CookieSite::Origin(u.origin().clone());
    let mut s = store();
    s.set_cookie(
        b"a=b; Domain=example.test; SameSite=Strict",
        &context(&u),
        0,
    )
    .unwrap();
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"a=b");
    c.site_for_cookies = CookieSite::Origin(url("http://b.example.test/").origin().clone());
    assert!(s.cookie_header(&c, 0).unwrap().is_empty());
    let mut s = CookieStore::default();
    s.set_cookie(b"a=b; SameSite=Strict", &context(&sibling), 0)
        .unwrap();
    c.site_for_cookies = CookieSite::Origin(u.origin().clone());
    assert!(s.cookie_header(&c, 0).unwrap().is_empty());
    c.site_for_cookies = CookieSite::NoClient;
    assert_eq!(s.cookie_header(&c, 0).unwrap(), b"a=b");
    c.site_for_cookies = CookieSite::Origin(Origin::opaque());
    assert!(s.cookie_header(&c, 0).unwrap().is_empty());
}
#[test]
fn replacement_preserves_creation_time_and_host_only_is_part_of_identity() {
    let u = url("https://example.test/");
    let mut s = store();
    let mut c = context(&u);
    s.set_cookie(b"a=1", &c, 0).unwrap();
    s.set_cookie(b"a=2", &c, 100).unwrap();
    assert_eq!(only(&mut s, 100).creation_time(), 0);
    c.site_for_cookies = CookieSite::CrossSite;
    c.top_level_navigation = true;
    c.method = "POST";
    assert!(s.cookie_header(&c, 121).unwrap().is_empty());
    s.set_cookie(b"a=domain; Domain=example.test", &context(&u), 100)
        .unwrap();
    assert_eq!(s.cookies(100).len(), 2);
}
#[test]
fn header_sorts_long_paths_first_then_original_creation_time() {
    let u = url("https://example.test/a/b");
    let c = context(&u);
    let mut s = store();
    for (time, line) in [
        (0, "old=1; Path=/"),
        (1, "new=2; Path=/"),
        (2, "long=3; Path=/a"),
        (3, "old=replaced; Path=/"),
    ] {
        s.set_cookie(line.as_bytes(), &c, time).unwrap();
    }
    assert_eq!(
        s.cookie_header(&c, 4).unwrap(),
        b"long=3; old=replaced; new=2"
    );
    assert!(s.cookies(4).iter().all(|v| v.last_access_time() == 4));
}
#[test]
fn expiration_precedes_eviction_and_session_end_keeps_persistent_only() {
    let u = url("https://example.test/");
    let c = context(&u);
    let mut s = store();
    s.set_cookie(b"session=1", &c, 0).unwrap();
    s.set_cookie(b"expires=2; Max-Age=10", &c, 0).unwrap();
    s.end_session(1);
    assert_eq!(s.cookie_header(&c, 1).unwrap(), b"expires=2");
    assert!(s.cookie_header(&c, 10).unwrap().is_empty());
}
#[test]
fn domain_quota_evicts_nonsecure_first_then_least_recently_accessed() {
    let mut s = CookieStore::new(
        Domains,
        CookieLimits {
            total: 3,
            per_domain: 2,
            ..CookieLimits::default()
        },
    )
    .unwrap();
    let u = url("https://example.test/");
    let c = context(&u);
    s.set_cookie(b"secure=1; Secure", &c, 0).unwrap();
    s.set_cookie(b"plain=2", &c, 1).unwrap();
    s.set_cookie(b"new=3; Secure", &c, 2).unwrap();
    assert_eq!(s.cookie_header(&c, 2).unwrap(), b"secure=1; new=3");
    s.set_cookie(b"last=4; Secure", &c, 3).unwrap();
    assert_eq!(s.cookie_header(&c, 3).unwrap(), b"new=3; last=4");
}
#[test]
fn global_quota_uses_lru_access_not_creation() {
    let mut s = CookieStore::new(
        Domains,
        CookieLimits {
            total: 2,
            per_domain: 2,
            ..CookieLimits::default()
        },
    )
    .unwrap();
    let a = url("https://example.test/");
    let b = url("https://other.test/");
    let d = url("https://third.test/");
    s.set_cookie(b"a=1", &context(&a), 0).unwrap();
    s.set_cookie(b"b=2", &context(&b), 1).unwrap();
    s.cookie_header(&context(&a), 2).unwrap();
    s.set_cookie(b"c=3", &context(&d), 3).unwrap();
    assert_eq!(s.cookies(3).len(), 2);
    assert!(s.cookie_header(&context(&b), 3).unwrap().is_empty());
}
#[test]
fn header_limit_fails_atomically_and_grace_can_be_disabled() {
    let u = url("https://example.test/");
    let mut c = context(&u);
    let mut s = CookieStore::new(
        Domains,
        CookieLimits {
            header_bytes: 2,
            lax_allowing_unsafe: false,
            ..CookieLimits::default()
        },
    )
    .unwrap();
    s.set_cookie(b"a=b", &c, 0).unwrap();
    assert_eq!(s.cookie_header(&c, 1), Err(CookieError::HeaderTooLarge));
    assert_eq!(only(&mut s, 1).last_access_time(), 0);
    c.site_for_cookies = CookieSite::CrossSite;
    c.top_level_navigation = true;
    c.method = "POST";
    assert!(s.cookie_header(&c, 1).unwrap().is_empty());
}

#[test]
fn provider_alabel_validation_is_independent_of_suffix_data() {
    struct Validated;
    impl RegistrableDomain for Validated {
        fn registrable_domain(&self, _: &str) -> Option<String> {
            None
        }
        fn validated_a_label(&self, label: &str) -> bool {
            label == "xn--9ca"
        }
    }
    let u = url("https://xn--9ca.test/");
    let mut s = CookieStore::new(Validated, CookieLimits::default()).unwrap();
    s.set_cookie(b"a=b", &context(&u), 0).unwrap();
    assert_eq!(s.cookie_header(&context(&u), 0).unwrap(), b"a=b");
    assert_eq!(
        s.set_cookie(b"b=c; Domain=xn--9ca.test", &context(&u), 0),
        Err(CookieError::UnknownPublicSuffix)
    );
}
#[test]
fn private_suffix_boundary_separates_tenants_and_keeps_same_tenant_subdomains() {
    let a = url("https://tenant.host.test/");
    let b = url("https://other.host.test/");
    let sub = url("https://sub.tenant.host.test/");
    let mut s = store();
    assert_eq!(
        s.set_cookie(b"a=b; Domain=host.test", &context(&a), 0),
        Err(CookieError::Domain)
    );
    s.set_cookie(
        b"a=b; Domain=tenant.host.test; SameSite=Strict",
        &context(&a),
        0,
    )
    .unwrap();
    let mut cross = context(&a);
    cross.site_for_cookies = CookieSite::Origin(b.origin().clone());
    assert!(s.cookie_header(&cross, 0).unwrap().is_empty());
    let mut same = context(&sub);
    same.site_for_cookies = CookieSite::Origin(a.origin().clone());
    assert_eq!(s.cookie_header(&same, 0).unwrap(), b"a=b");
}
#[test]
fn ipv6_host_only_roundtrips_and_domain_matching_rejects_empty_inputs() {
    assert!(!domain_matches("x.", ""));
    assert!(!domain_matches("", ""));
    let mut s = CookieStore::default();
    let u = url("https://[::1]/");
    s.set_cookie(b"a=b", &context(&u), 0).unwrap();
    assert_eq!(s.cookie_header(&context(&u), 0).unwrap(), b"a=b");
}
#[test]
fn byte_inputs_and_hostile_dates_are_bounded_and_do_not_panic() {
    let u = url("https://example.test/a/b");
    let mut s = store();
    let c = context(&u);
    for byte in 0u8..=255 {
        let mut field = b"name=".to_vec();
        field.push(byte);
        field.extend_from_slice(b"; Expires=");
        field.extend([byte; 32]);
        let _ = s.set_cookie(&field, &c, 0);
        let _ = s.cookie_header(&c, 0);
    }
    assert!(s.cookies(0).len() <= 180);
}

#[test]
fn default_path_memory_and_configured_store_limits_are_bounded() {
    let u = url(&format!("https://example.test/{}/x", "a".repeat(1024)));
    let mut s = store();
    assert_eq!(
        s.set_cookie(b"a=b", &context(&u), 0),
        Err(CookieError::TooLarge)
    );
    assert!(CookieStore::new(
        Domains,
        CookieLimits {
            total: 3001,
            ..CookieLimits::default()
        }
    )
    .is_err());
    assert!(CookieStore::new(
        Domains,
        CookieLimits {
            per_domain: 0,
            ..CookieLimits::default()
        }
    )
    .is_err());
}
