use amazing_fetch::cookies::*;
use amazing_fetch::psl::{DomainBoundary, PublicSuffixList};
use amazing_fetch::{HttpUrl, RegistrableDomain};

#[test]
fn upstream_normalized_fixtures() {
    let mut count = 0;
    for line in include_str!("../data/psl/fixtures.tsv").lines() {
        let (host, expected) = line.split_once('\t').unwrap();
        assert_eq!(
            PublicSuffixList.registrable_domain(host).as_deref(),
            (expected != "-").then_some(expected),
            "{host}"
        );
        count += 1;
    }
    assert!(count > 70);
}
#[test]
fn exact_wildcard_exception_private_and_unknown_boundaries() {
    for (host, expected) in [
        ("a.b.co.uk", "b.co.uk"),
        ("a.b.ck", "a.b.ck"),
        ("x.www.ck", "www.ck"),
        ("x.city.kawasaki.jp", "city.kawasaki.jp"),
        ("a.tenant.github.io", "tenant.github.io"),
        ("a.tenant.blogspot.com", "tenant.blogspot.com"),
        ("a.b.unknown-tld", "b.unknown-tld"),
        ("a.b.co.uk.", "b.co.uk."),
    ] {
        assert_eq!(
            PublicSuffixList.classify(host),
            DomainBoundary::Registrable(expected)
        );
    }
    for host in [
        "com",
        "co.uk",
        "b.ck",
        "github.io",
        "localhost",
        "unknown-tld",
    ] {
        assert_eq!(
            PublicSuffixList.classify(host),
            DomainBoundary::PublicSuffix,
            "{host}"
        );
        assert_eq!(PublicSuffixList.is_public_suffix(host), Some(true));
    }
}
#[test]
fn ips_and_invalid_hosts_never_acquire_a_registrable_domain() {
    for host in ["127.0.0.1", "[::1]", "::1", "2001:db8::1"] {
        assert_eq!(PublicSuffixList.classify(host), DomainBoundary::Ip);
        assert_eq!(PublicSuffixList.is_public_suffix(host), Some(false));
        assert_eq!(PublicSuffixList.registrable_domain(host), None);
    }
    for host in [
        "",
        ".",
        ".com",
        "a..com",
        "a.com..",
        "EXAMPLE.COM",
        "-a.com",
        "a_.com",
        "é.com",
    ] {
        assert_eq!(
            PublicSuffixList.classify(host),
            DomainBoundary::Invalid,
            "{host}"
        );
        assert_eq!(PublicSuffixList.is_public_suffix(host), None);
    }
    assert_eq!(
        PublicSuffixList.is_public_suffix(&format!("{}.com", "a".repeat(64))),
        None
    );
}
fn access(u: &HttpUrl) -> CookieAccess<'_> {
    CookieAccess {
        url: u,
        site_for_cookies: CookieSite::Origin(u.origin().clone()),
        http_api: true,
        top_level_navigation: false,
        method: "GET",
        partition_key: None,
    }
}
#[test]
fn real_psl_rejects_cross_tenant_and_public_suffix_cookies() {
    for (host, suffix) in [
        ("a.example.co.uk", "co.uk"),
        ("a.github.io", "github.io"),
        ("a.b.ck", "b.ck"),
        ("foo.blogspot.com", "blogspot.com"),
    ] {
        let u = HttpUrl::parse(&format!("https://{host}/")).unwrap();
        let mut store = CookieStore::new(PublicSuffixList, CookieLimits::default()).unwrap();
        assert_eq!(
            store.set_cookie(format!("a=b; Domain={suffix}").as_bytes(), &access(&u), 0),
            Err(CookieError::Domain)
        );
    }
}
#[test]
fn real_psl_domain_cookie_reaches_siblings_and_exception_subdomains() {
    for (host, domain, sibling) in [
        ("a.example.co.uk", "example.co.uk", "b.example.co.uk"),
        ("a.www.ck", "www.ck", "b.www.ck"),
    ] {
        let u = HttpUrl::parse(&format!("https://{host}/")).unwrap();
        let v = HttpUrl::parse(&format!("https://{sibling}/")).unwrap();
        let mut store = CookieStore::new(PublicSuffixList, CookieLimits::default()).unwrap();
        store
            .set_cookie(
                format!("a=b; Domain={domain}; SameSite=Strict").as_bytes(),
                &access(&u),
                0,
            )
            .unwrap();
        let mut ctx = access(&v);
        ctx.site_for_cookies = CookieSite::Origin(u.origin().clone());
        assert_eq!(store.cookie_header(&ctx, 0).unwrap(), b"a=b");
    }
}
#[test]
fn public_suffix_exact_host_is_host_only_and_private_sites_are_separate() {
    let u = HttpUrl::parse("https://github.io/").unwrap();
    let mut store = CookieStore::new(PublicSuffixList, CookieLimits::default()).unwrap();
    store
        .set_cookie(b"a=b; Domain=github.io", &access(&u), 0)
        .unwrap();
    assert!(store.cookies(0)[0].host_only());
    let a = HttpUrl::parse("https://a.github.io/").unwrap();
    let b = HttpUrl::parse("https://b.github.io/").unwrap();
    assert_ne!(
        a.origin().site(&PublicSuffixList),
        b.origin().site(&PublicSuffixList)
    );
    assert!(store.cookie_header(&access(&a), 0).unwrap().is_empty());
}
#[test]
fn psl_does_not_authorize_unvalidated_idna() {
    assert!(!PublicSuffixList.validated_a_label("xn--invalid"));
    let u = HttpUrl::parse("https://xn--invalid.com/").unwrap();
    let mut store = CookieStore::new(PublicSuffixList, CookieLimits::default()).unwrap();
    assert_eq!(
        store.set_cookie(b"a=b", &access(&u), 0),
        Err(CookieError::InvalidHost)
    );
}

#[test]
fn every_pinned_rule_has_a_boundary_witness() {
    let shards = [
        include_str!("../data/psl/rules-00.txt"),
        include_str!("../data/psl/rules-01.txt"),
        include_str!("../data/psl/rules-02.txt"),
        include_str!("../data/psl/rules-03.txt"),
        include_str!("../data/psl/rules-04.txt"),
    ];
    let mut previous = "";
    for rule in shards.into_iter().flat_map(str::lines) {
        assert!(rule > previous, "rule ordering/deduplication: {rule}");
        previous = rule;
        if let Some(exception) = rule.strip_prefix('!') {
            assert_eq!(
                PublicSuffixList.registrable_domain(exception).as_deref(),
                Some(exception),
                "{rule}"
            );
        } else {
            let suffix = if let Some(base) = rule.strip_prefix("*.") {
                format!("net6-fixture.{base}")
            } else {
                rule.to_string()
            };
            assert_eq!(
                PublicSuffixList.is_public_suffix(&suffix),
                Some(true),
                "{rule}"
            );
            let child = format!("net6-child.{suffix}");
            assert_eq!(
                PublicSuffixList.registrable_domain(&child).as_deref(),
                Some(child.as_str()),
                "{rule}"
            );
        }
    }
}
