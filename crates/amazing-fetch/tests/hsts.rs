use amazing_fetch::{hsts::HstsStore, HttpUrl};

fn url(input: &str) -> HttpUrl {
    HttpUrl::parse(input).unwrap()
}

#[test]
fn dynamic_hsts_expires_and_uses_domain_boundaries() {
    let mut store = HstsStore::new();
    assert!(!store.observe(&url("http://example.test/"), "max-age=100", 10));
    assert!(store.observe(
        &url("https://example.test/"),
        "max-age=100; includeSubDomains",
        10
    ));
    assert!(store.is_strict("child.example.test", 109));
    assert!(!store.is_strict("evil-example.test", 11));
    assert_eq!(
        store
            .upgrade(&url("http://child.example.test:80/a"), 11)
            .unwrap()
            .serialize(),
        "https://child.example.test/a"
    );
    assert!(!store.is_strict("example.test", 110));
    assert!(store.observe(&url("https://example.test/"), "max-age=0", 20));
    assert!(!store.is_strict("example.test", 21));
}

#[test]
fn pinned_preload_applies_to_exact_and_subdomain_entries() {
    let store = HstsStore::new();
    assert!(store.is_strict("app", 0));
    assert!(store.is_strict("example.app", 0));
    assert!(!store.is_strict("not-app", 0));
    assert!(store.upgrade(&url("http://example.app/path"), 0).is_some());
}

#[test]
fn invalid_policy_does_not_replace_valid_policy() {
    let mut store = HstsStore::new();
    let u = url("https://example.test/");
    assert!(store.observe(&u, "max-age=30", 0));
    for bad in ["includeSubDomains", "max-age=-1", "max-age=1; max-age=0"] {
        assert!(!store.observe(&u, bad, 1));
    }
    assert!(store.is_strict("example.test", 2));
    assert!(!store.is_strict("sub.example.test", 2));
}
