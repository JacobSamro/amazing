use amazing_fetch::cache::*;
use amazing_fetch::HeaderList;

fn fields(values: &[(&str, &str)]) -> HeaderList {
    let mut headers = HeaderList::default();
    for (name, value) in values {
        headers.append(name, value).unwrap();
    }
    headers
}
fn response(values: &[(&str, &str)], body: &[u8]) -> CacheResponse {
    CacheResponse {
        request_includes_credentials: true,
        status: 200,
        headers: fields(values),
        body: body.to_vec(),
    }
}
fn request<'a>(uri: &'a str, headers: &'a HeaderList, cookie: &'a str) -> CacheRequest<'a> {
    CacheRequest {
        uri,
        method: "GET",
        headers,
        cookie,
    }
}
fn fresh(cache: &CacheStore, req: &CacheRequest<'_>, now: i64) -> CacheResponse {
    match cache.lookup(req, now).unwrap() {
        CacheLookup::Fresh(entry) => entry.response_at(now),
        _ => panic!("expected fresh response"),
    }
}
fn validate(cache: &CacheStore, req: &CacheRequest<'_>, now: i64) -> HeaderList {
    match cache.lookup(req, now).unwrap() {
        CacheLookup::Validate { headers, .. } => headers,
        _ => panic!("expected validation"),
    }
}
fn private() -> CacheStore {
    CacheStore::new(CacheKind::Private, CacheLimits::default()).unwrap()
}

#[test]
fn max_age_and_age_account_for_response_delay_and_residence() {
    let mut cache = private();
    let empty = fields(&[]);
    let req = request("https://example.test/a", &empty, "");
    cache
        .store(
            &req,
            response(
                &[
                    ("Cache-Control", "max-age=60"),
                    ("Age", "10"),
                    ("Date", "Thu, 01 Jan 1970 00:01:35 GMT"),
                ],
                b"one",
            ),
            100,
            105,
        )
        .unwrap();
    assert_eq!(
        fresh(&cache, &req, 105).headers.get("age").as_deref(),
        Some("15")
    );
    assert_eq!(
        fresh(&cache, &req, 149).headers.get("age").as_deref(),
        Some("59")
    );
    assert!(matches!(
        cache.lookup(&req, 150).unwrap(),
        CacheLookup::Validate { .. }
    ));
    let limited = fields(&[("Cache-Control", "max-age=20")]);
    assert!(matches!(
        cache.lookup(&request(req.uri, &limited, ""), 126).unwrap(),
        CacheLookup::Validate { .. }
    ));
}
#[test]
fn shared_s_maxage_private_and_authorization_rules() {
    let mut shared = CacheStore::new(CacheKind::Shared, CacheLimits::default()).unwrap();
    let mut private = private();
    let empty = fields(&[]);
    let req = request("https://example.test/a", &empty, "");
    let r = response(&[("Cache-Control", "max-age=100, s-maxage=5")], b"one");
    shared.store(&req, r.clone(), 0, 0).unwrap();
    private.store(&req, r, 0, 0).unwrap();
    assert!(matches!(
        shared.lookup(&req, 6).unwrap(),
        CacheLookup::Validate { .. }
    ));
    assert_eq!(fresh(&private, &req, 6).body, b"one");
    assert!(!shared
        .store(
            &req,
            response(&[("Cache-Control", "private, max-age=50")], b"two"),
            0,
            0
        )
        .unwrap());
    let auth = fields(&[("Authorization", "Bearer secret")]);
    let auth_req = request("https://example.test/secret", &auth, "");
    assert!(!shared
        .store(
            &auth_req,
            response(&[("Cache-Control", "max-age=50")], b"x"),
            0,
            0
        )
        .unwrap());
    assert!(shared
        .store(
            &auth_req,
            response(&[("Cache-Control", "public, max-age=50")], b"x"),
            0,
            0
        )
        .unwrap());
}
#[test]
fn expires_then_last_modified_heuristic_and_invalid_dates() {
    let mut cache = private();
    let empty = fields(&[]);
    let expires_req = request("https://example.test/expires", &empty, "");
    cache
        .store(
            &expires_req,
            response(
                &[
                    ("Date", "Thu, 01 Jan 1970 00:01:40 GMT"),
                    ("Expires", "Thu, 01 Jan 1970 00:02:40 GMT"),
                ],
                b"e",
            ),
            100,
            100,
        )
        .unwrap();
    assert_eq!(fresh(&cache, &expires_req, 159).body, b"e");
    validate(&cache, &expires_req, 160);
    let heuristic = request("https://example.test/heuristic?x=1", &empty, "");
    cache
        .store(
            &heuristic,
            response(
                &[
                    ("Date", "Thu, 01 Jan 1970 00:16:40 GMT"),
                    ("Last-Modified", "Thu, 01 Jan 1970 00:00:00 GMT"),
                ],
                b"h",
            ),
            1000,
            1000,
        )
        .unwrap();
    assert_eq!(fresh(&cache, &heuristic, 1099).body, b"h");
    validate(&cache, &heuristic, 1100);
    let bad = request("https://example.test/bad", &empty, "");
    cache
        .store(
            &bad,
            response(
                &[
                    ("Expires", "Thu, 01 Jan 1970 00:10:00 PST"),
                    ("Last-Modified", "Thu, 01 Jan 1970 00:00:00 GMT"),
                ],
                b"b",
            ),
            0,
            0,
        )
        .unwrap();
    validate(&cache, &bad, 0);
}
#[test]
fn no_store_no_cache_must_revalidate_and_request_controls() {
    let mut cache = private();
    let empty = fields(&[]);
    let req = request("https://example.test/a", &empty, "");
    assert!(!cache
        .store(
            &req,
            response(&[("Cache-Control", "no-store, max-age=60")], b"x"),
            0,
            0
        )
        .unwrap());
    cache
        .store(
            &req,
            response(
                &[
                    ("Cache-Control", "no-cache, max-age=60"),
                    ("ETag", "\"v1\""),
                ],
                b"x",
            ),
            0,
            0,
        )
        .unwrap();
    assert_eq!(
        validate(&cache, &req, 1).get("if-none-match").as_deref(),
        Some("\"v1\"")
    );
    let req_no_store = fields(&[("Cache-Control", "no-store")]);
    assert!(matches!(
        cache
            .lookup(&request(req.uri, &req_no_store, ""), 1)
            .unwrap(),
        CacheLookup::Miss
    ));
    assert!(!cache
        .store(
            &request("https://example.test/other", &req_no_store, ""),
            response(&[("Cache-Control", "max-age=60")], b"x"),
            0,
            0
        )
        .unwrap());
    let req_no_cache = fields(&[("Cache-Control", "no-cache")]);
    cache
        .store(
            &request("https://example.test/fresh", &empty, ""),
            response(&[("Cache-Control", "must-revalidate, max-age=10")], b"f"),
            0,
            0,
        )
        .unwrap();
    validate(
        &cache,
        &request("https://example.test/fresh", &req_no_cache, ""),
        1,
    );
    validate(
        &cache,
        &request("https://example.test/fresh", &empty, ""),
        10,
    );
}
#[test]
fn etag_304_replaces_metadata_preserves_body_and_rejects_mismatch() {
    let mut cache = private();
    let empty = fields(&[]);
    let req = request("https://example.test/a", &empty, "");
    cache
        .store(
            &req,
            response(
                &[
                    ("Cache-Control", "max-age=0"),
                    ("ETag", "\"v1\""),
                    ("Last-Modified", "Thu, 01 Jan 1970 00:00:00 GMT"),
                    ("Content-Type", "text/plain"),
                ],
                b"body",
            ),
            0,
            0,
        )
        .unwrap();
    let conditional = validate(&cache, &req, 0);
    assert_eq!(conditional.get("if-none-match").as_deref(), Some("\"v1\""));
    assert!(conditional.get("if-modified-since").is_some());
    assert!(cache
        .update_304(&req, &fields(&[("ETag", "\"wrong\"")]), 1, 1)
        .unwrap()
        .is_none());
    assert!(cache
        .update_304(
            &req,
            &fields(&[("Date", "Thu, 01 Jan 1970 00:00:01 GMT")]),
            1,
            1
        )
        .unwrap()
        .is_none());
    let updated = cache
        .update_304(
            &req,
            &fields(&[
                ("ETag", "\"v1\""),
                ("Cache-Control", "max-age=30"),
                ("Content-Type", "text/html"),
            ]),
            2,
            3,
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.body, b"body");
    assert_eq!(
        updated.headers.get("content-type").as_deref(),
        Some("text/html")
    );
    assert_eq!(fresh(&cache, &req, 20).body, b"body");
}
#[test]
fn last_modified_validation_and_304_no_store() {
    let mut cache = private();
    let empty = fields(&[]);
    let req = request("https://example.test/a", &empty, "");
    let modified = "Thu, 01 Jan 1970 00:00:00 GMT";
    cache
        .store(
            &req,
            response(
                &[("Cache-Control", "max-age=0"), ("Last-Modified", modified)],
                b"x",
            ),
            100,
            100,
        )
        .unwrap();
    assert_eq!(
        validate(&cache, &req, 100)
            .get("if-modified-since")
            .as_deref(),
        Some(modified)
    );
    assert!(cache
        .update_304(
            &req,
            &fields(&[("Last-Modified", modified), ("Cache-Control", "max-age=50")]),
            101,
            101
        )
        .unwrap()
        .is_some());
    assert_eq!(fresh(&cache, &req, 102).body, b"x");
    assert_eq!(
        cache
            .update_304(
                &req,
                &fields(&[("Last-Modified", modified), ("Cache-Control", "no-store")]),
                103,
                103
            )
            .unwrap()
            .unwrap()
            .body,
        b"x"
    );
    assert_eq!(cache.len(), 0);
}
#[test]
fn vary_matches_headers_and_cookie_partition_preserves_shell_key() {
    let mut cache = private();
    let en = fields(&[("Accept-Language", "en"), ("Cookie", "a=1")]);
    let fr = fields(&[("Accept-Language", "fr"), ("Cookie", "a=1")]);
    let req_en = request("https://example.test/a", &en, "a=1");
    assert_eq!(req_en.legacy_key().unwrap(), "https://example.test/a\na=1");
    cache
        .store(
            &req_en,
            response(
                &[
                    ("Cache-Control", "max-age=60"),
                    ("Vary", "Accept-Language, Cookie"),
                ],
                b"en",
            ),
            0,
            0,
        )
        .unwrap();
    assert_eq!(fresh(&cache, &req_en, 1).body, b"en");
    assert!(matches!(
        cache.lookup(&request(req_en.uri, &fr, "a=1"), 1).unwrap(),
        CacheLookup::Miss
    ));
    assert!(matches!(
        cache.lookup(&request(req_en.uri, &en, "a=2"), 1).unwrap(),
        CacheLookup::Miss
    ));
    assert!(!cache
        .store(
            &req_en,
            response(&[("Cache-Control", "max-age=60"), ("Vary", "*")], b"never"),
            0,
            0
        )
        .unwrap());
}
#[test]
fn unsafe_success_invalidates_every_variant_and_errors_do_not() {
    let mut cache = private();
    let empty = fields(&[]);
    for cookie in ["a=1", "a=2"] {
        cache
            .store(
                &request("https://example.test/a", &empty, cookie),
                response(&[("Cache-Control", "max-age=60")], b"x"),
                0,
                0,
            )
            .unwrap();
    }
    cache.invalidate_unsafe("POST", "https://example.test/a", 500);
    assert_eq!(cache.len(), 2);
    cache.invalidate_unsafe("POST", "https://example.test/a", 204);
    assert_eq!(cache.len(), 0);
}
#[test]
fn entry_count_bytes_and_input_bounds_precede_insertion() {
    let limits = CacheLimits {
        entries: 2,
        bytes: 1024,
        entry_bytes: 512,
    };
    let mut cache = CacheStore::new(CacheKind::Private, limits).unwrap();
    let empty = fields(&[]);
    let oversized = request("https://example.test/huge", &empty, "");
    assert_eq!(
        cache.store(
            &oversized,
            response(&[("Cache-Control", "max-age=60")], &[0; 513]),
            0,
            0
        ),
        Err(CacheError::TooLarge)
    );
    assert_eq!((cache.len(), cache.bytes()), (0, 0));
    for name in ["a", "b", "c"] {
        let uri = format!("https://example.test/{name}");
        cache
            .store(
                &request(&uri, &empty, ""),
                response(&[("Cache-Control", "max-age=60")], &[0; 100]),
                0,
                0,
            )
            .unwrap();
    }
    assert_eq!(cache.len(), 2);
    assert!(cache.bytes() <= 1024);
    assert!(matches!(
        cache
            .lookup(&request("https://example.test/a", &empty, ""), 1)
            .unwrap(),
        CacheLookup::Miss
    ));
    let huge_uri = "x".repeat(8193);
    assert_eq!(
        request(&huge_uri, &empty, "").legacy_key(),
        Err(CacheError::TooLarge)
    );
}

#[test]
fn invalid_or_duplicate_freshness_never_falls_back_to_a_fresh_hit() {
    let mut cache = private();
    let empty = fields(&[]);
    for (path, control) in [
        ("invalid", "max-age=invalid"),
        ("duplicate", "max-age=60, max-age=70"),
        ("conflict", "max-age=60, no-cache"),
    ] {
        let uri = format!("https://example.test/{path}");
        let req = request(&uri, &empty, "");
        cache
            .store(
                &req,
                response(
                    &[
                        ("Cache-Control", control),
                        ("Last-Modified", "Thu, 01 Jan 1970 00:00:00 GMT"),
                    ],
                    b"x",
                ),
                100,
                100,
            )
            .unwrap();
        validate(&cache, &req, 101);
    }
}

#[test]
fn validation_updates_vary_and_rejects_mismatched_content_length() {
    let mut cache = private();
    let en = fields(&[("Accept-Language", "en")]);
    let fr = fields(&[("Accept-Language", "fr")]);
    let req = request("https://example.test/a", &en, "");
    cache
        .store(
            &req,
            response(
                &[("Cache-Control", "max-age=0"), ("ETag", "\"v1\"")],
                b"body",
            ),
            0,
            0,
        )
        .unwrap();
    assert!(cache
        .update_304(
            &req,
            &fields(&[("ETag", "\"v1\""), ("Content-Length", "5")]),
            1,
            1
        )
        .unwrap()
        .is_none());
    cache
        .update_304(
            &req,
            &fields(&[
                ("ETag", "\"v1\""),
                ("Cache-Control", "max-age=60"),
                ("Vary", "Accept-Language"),
            ]),
            2,
            2,
        )
        .unwrap()
        .unwrap();
    assert_eq!(fresh(&cache, &req, 3).body, b"body");
    assert!(matches!(
        cache.lookup(&request(req.uri, &fr, ""), 3).unwrap(),
        CacheLookup::Miss
    ));
}

#[test]
fn storage_seam_restores_owned_entries_and_rechecks_limits() {
    let limits = CacheLimits {
        entries: 2,
        bytes: 1024,
        entry_bytes: 512,
    };
    let mut cache = CacheStore::new(CacheKind::Private, limits).unwrap();
    let empty = fields(&[]);
    let req = request("https://example.test/a", &empty, "a=1");
    cache
        .store(
            &req,
            response(&[("Cache-Control", "max-age=60")], b"ok"),
            0,
            0,
        )
        .unwrap();
    let storage = cache.into_storage();
    assert_eq!(
        storage.get(0).unwrap().legacy_key,
        "https://example.test/a\na=1"
    );
    let restored = CacheStore::with_storage(storage, CacheKind::Private, limits)
        .unwrap_or_else(|_| panic!("valid restored cache"));
    assert_eq!(fresh(&restored, &req, 1).body, b"ok");
    let mut storage = restored.into_storage();
    storage.get_mut(0).unwrap().response.body.resize(513, 0);
    assert!(matches!(
        CacheStore::with_storage(storage, CacheKind::Private, limits),
        Err(CacheError::TooLarge)
    ));
}

#[test]
fn revalidated_304_without_validators_updates_the_only_candidate_and_request_directives_apply() {
    let mut cache = private();
    let empty = fields(&[]);
    let req = request("https://example.test/b", &empty, "");
    cache
        .store(&req, response(&[("Cache-Control", "no-cache"), ("ETag", "\"v1\"")], b"body"), 0, 0)
        .unwrap();
    let not_modified = fields(&[("Content-Length", "0"), ("Test-Header", "B")]);
    // The strict entry point still refuses a validator-less 304.
    assert!(cache.update_304(&req, &not_modified, 1, 1).unwrap().is_none());
    // A different credentials mode may not adopt it.
    assert!(cache.update_304_revalidated(&req, &not_modified, 1, 1, false).unwrap().is_none());
    let merged = cache.update_304_revalidated(&req, &not_modified, 1, 1, true).unwrap().unwrap();
    assert_eq!(merged.body, b"body");
    assert_eq!(merged.headers.get("test-header").as_deref(), Some("B"));
    // Request Cache-Control: min-fresh and only-if-cached.
    let req2 = request("https://example.test/c", &empty, "");
    cache.store(&req2, response(&[("Cache-Control", "max-age=100")], b"c"), 0, 0).unwrap();
    let min_fresh = fields(&[("Cache-Control", "min-fresh=200")]);
    assert!(matches!(cache.lookup(&request("https://example.test/c", &min_fresh, ""), 1).unwrap(),
        CacheLookup::Validate { .. }));
    let only = fields(&[("Cache-Control", "only-if-cached")]);
    assert!(matches!(cache.lookup(&request("https://example.test/none", &only, ""), 1).unwrap(),
        CacheLookup::Miss));
    assert!(amazing_fetch::cache::request_only_if_cached(&only));
}
