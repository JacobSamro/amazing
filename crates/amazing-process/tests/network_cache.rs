use amazing_fetch::cache::{CacheLimits, CacheLookup, CacheRequest, CacheResponse, CacheStore};
use amazing_fetch::HeaderList;
use amazing_process::network_cache::DiskCache;
use amazing_process::network_v2::{get_request, NetworkSession};
use amazing_process::protocol::{NetworkContext, WireCookieSite, WireOrigin};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

fn dir(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "amazing-http-cache-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).unwrap();
    path
}
fn headers(pairs: &[(&str, &str)]) -> HeaderList {
    let mut out = HeaderList::default();
    for &(name, value) in pairs {
        out.append(name, value).unwrap();
    }
    out
}
fn request<'a>(uri: &'a str, headers: &'a HeaderList) -> CacheRequest<'a> {
    CacheRequest {
        uri,
        method: "GET",
        headers,
        cookie: "",
    }
}
fn response(body: &[u8], pairs: &[(&str, &str)]) -> CacheResponse {
    CacheResponse {
        request_includes_credentials: true,
        status: 200,
        headers: headers(pairs),
        body: body.to_vec(),
    }
}
fn body(cache: &CacheStore, request: &CacheRequest<'_>, now: i64) -> Option<Vec<u8>> {
    match cache.lookup(request, now).unwrap() {
        CacheLookup::Fresh(entry) => Some(entry.response.body.clone()),
        _ => None,
    }
}

#[test]
fn cache_survives_restart_and_preserves_vary_and_validators() {
    let path = dir("restart");
    let limits = CacheLimits::default();
    let (mut cache, disk) = DiskCache::open(&path, limits).unwrap();
    let a = headers(&[("Accept-Language", "en")]);
    let b = headers(&[("Accept-Language", "fr")]);
    let uri = "https://example.test/resource";
    cache
        .store(
            &request(uri, &a),
            response(
                b"english",
                &[
                    ("Cache-Control", "max-age=60"),
                    ("Vary", "Accept-Language"),
                    ("ETag", "\"en\""),
                ],
            ),
            100,
            100,
        )
        .unwrap();
    cache
        .store(
            &request(uri, &b),
            response(
                b"francais",
                &[
                    ("Cache-Control", "max-age=60"),
                    ("Vary", "Accept-Language"),
                    ("Last-Modified", "Thu, 01 Jan 1970 00:00:00 GMT"),
                ],
            ),
            100,
            100,
        )
        .unwrap();
    disk.save(&cache).unwrap();
    let (mut cache, disk) = DiskCache::open(&path, limits).unwrap();
    assert_eq!(
        body(&cache, &request(uri, &a), 110).as_deref(),
        Some(&b"english"[..])
    );
    assert_eq!(
        body(&cache, &request(uri, &b), 110).as_deref(),
        Some(&b"francais"[..])
    );
    let empty = headers(&[]);
    assert_eq!(body(&cache, &request(uri, &empty), 110), None);
    let validators = match cache.lookup(&request(uri, &a), 161).unwrap() {
        CacheLookup::Validate { headers, .. } => headers,
        _ => panic!("expected ETag validation"),
    };
    assert_eq!(validators.get("if-none-match").as_deref(), Some("\"en\""));
    cache
        .update_304(
            &request(uri, &a),
            &headers(&[("ETag", "\"en\""), ("Cache-Control", "max-age=120")]),
            161,
            161,
        )
        .unwrap()
        .unwrap();
    disk.save(&cache).unwrap();
    let (cache, _) = DiskCache::open(&path, limits).unwrap();
    assert_eq!(
        body(&cache, &request(uri, &a), 200).as_deref(),
        Some(&b"english"[..])
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn bounded_snapshot_evicts_entries_and_rejects_corruption() {
    let path = dir("bound");
    let limits = CacheLimits {
        entries: 2,
        bytes: 256,
        entry_bytes: 64,
    };
    let (mut cache, disk) = DiskCache::open(&path, limits).unwrap();
    let empty = headers(&[]);
    for id in 0..3 {
        let uri = format!("https://e.test/{id}");
        cache
            .store(
                &request(&uri, &empty),
                response(&[b'x'; 40], &[("Cache-Control", "max-age=60")]),
                0,
                0,
            )
            .unwrap();
    }
    assert!(cache.len() <= 2 && cache.bytes() <= limits.bytes);
    disk.save(&cache).unwrap();
    let (loaded, _) = DiskCache::open(&path, limits).unwrap();
    assert_eq!(loaded.len(), cache.len());
    assert_eq!(loaded.bytes(), cache.bytes());
    fs::write(path.join("http-cache-v2.bin"), b"AMHC0002invalid").unwrap();
    let (empty_cache, _) = DiskCache::open(&path, limits).unwrap();
    assert_eq!(empty_cache.len(), 0);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn request_and_response_no_store_do_not_enter_disk() {
    let path = dir("no-store");
    let (mut cache, disk) = DiskCache::open(&path, CacheLimits::default()).unwrap();
    let empty = headers(&[]);
    let no_store = headers(&[("Cache-Control", "no-store")]);
    assert!(!cache
        .store(
            &request("https://e.test/a", &empty),
            response(b"secret", &[("Cache-Control", "no-store")]),
            0,
            0
        )
        .unwrap());
    assert!(!cache
        .store(
            &request("https://e.test/b", &no_store),
            response(b"secret", &[("Cache-Control", "max-age=60")]),
            0,
            0
        )
        .unwrap());
    disk.save(&cache).unwrap();
    let (loaded, _) = DiskCache::open(&path, CacheLimits::default()).unwrap();
    assert_eq!(loaded.len(), 0);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn network_child_serves_disk_cache_after_restart() {
    let path = dir("process-restart");
    std::env::set_var("AMAZING_HTTP_CACHE_DIR", &path);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let url = format!("{origin}/cached");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 16 * 1024);
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 7\r\nConnection: close\r\n\r\npersist").unwrap();
    });
    let context = NetworkContext {
        origin: WireOrigin::Tuple(origin.clone()),
        cookie_site: WireCookieSite::NoClient(()),
        top_level_origin: None,
        ancestor_origins: Vec::new(),
        sandbox_flags: 0,
        now_unix: 1_780_000_000,
        restricts_mixed_content: false,
        image_set_initiator: false,
        top_level_navigation: false,
        embedder_policy: "unsafe-none".into(),
        no_cors_media_state: "none".into(),
        csp_headers: Vec::new(),
    };
    let make_request = || get_request(&url, &origin, "other", 1024, context.clone());
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    {
        let session = NetworkSession::launch(exe, 101, Duration::from_secs(5)).unwrap();
        let reply = session.fetch(make_request()).unwrap();
        assert_eq!(reply.body, b"persist");
    }
    server.join().unwrap();
    assert!(path.join("http-cache-v2.bin").exists());
    {
        let session = NetworkSession::launch(exe, 102, Duration::from_secs(5)).unwrap();
        let reply = session.fetch(make_request()).unwrap();
        assert_eq!(reply.body, b"persist");
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn simultaneous_network_children_leave_a_valid_snapshot() {
    let path = dir("simultaneous");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for id in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let (mut cache, disk) = DiskCache::open(&path, CacheLimits::default()).unwrap();
            let uri = format!("https://example.test/{id}");
            let empty = headers(&[]);
            cache
                .store(
                    &request(&uri, &empty),
                    response(b"body", &[("Cache-Control", "max-age=60")]),
                    100,
                    100,
                )
                .unwrap();
            barrier.wait();
            disk.save(&cache)
        }));
    }
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(results.iter().any(Result::is_ok));
    let (loaded, _) = DiskCache::open(&path, CacheLimits::default()).unwrap();
    assert_eq!(loaded.len(), 1);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn cache_v2_preserves_actual_credentials_receipts_and_rejects_missing_v1_facts() {
    let path = dir("coep-receipts");
    let limits = CacheLimits::default();
    let (mut cache, disk) = DiskCache::open(&path, limits).unwrap();
    let empty = headers(&[]);
    for includes in [false, true] {
        let uri = format!("https://example.test/receipt/{includes}");
        let mut candidate = response(b"image", &[("Cache-Control", "max-age=60")]);
        candidate.request_includes_credentials = includes;
        cache
            .store(&request(&uri, &empty), candidate, 100, 100)
            .unwrap();
    }
    disk.save(&cache).unwrap();
    let (loaded, _) = DiskCache::open(&path, limits).unwrap();
    for includes in [false, true] {
        let uri = format!("https://example.test/receipt/{includes}");
        let CacheLookup::Fresh(entry) = loaded.lookup(&request(&uri, &empty), 110).unwrap() else {
            panic!("stored response missing");
        };
        assert_eq!(entry.response.request_includes_credentials, includes);
    }
    let snapshot = path.join("http-cache-v2.bin");
    let mut bytes = fs::read(&snapshot).unwrap();
    assert_eq!(&bytes[..8], b"AMHC0002");
    bytes[..8].copy_from_slice(b"AMHC0001");
    fs::write(&snapshot, &bytes).unwrap();
    let (rejected, _) = DiskCache::open(&path, limits).unwrap();
    assert_eq!(
        rejected.len(),
        0,
        "missing old-format fact must not become false authority"
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn invalid_persisted_credential_receipt_is_rejected_instead_of_becoming_false_authority() {
    let path = dir("invalid-credential-receipt");
    let empty = headers(&[]);
    let limits = CacheLimits::default();
    let (mut store, disk) = DiskCache::open(&path, limits).unwrap();
    store.store(&request("https://example.test/receipt", &empty), response(b"body", &[("Cache-Control", "max-age=60")]), 100, 100).unwrap();
    disk.save(&store).unwrap();
    let file = path.join("http-cache-v2.bin");
    let original = fs::read(&file).unwrap();
    let mut offset = 12; // magic and entry count
    for _ in 0..2 { // URI and legacy cache key
        let count = u32::from_le_bytes(original[offset..offset+4].try_into().unwrap()) as usize;
        offset += 4 + count;
    }
    assert_eq!(u32::from_le_bytes(original[offset..offset+4].try_into().unwrap()), 0, "fixture has no Vary fields");
    offset += 4;
    assert_eq!(original[offset], 1, "actual saved credentialed response");
    for invalid in [2, 255] {
        let mut malformed = original.clone();
        malformed[offset] = invalid;
        fs::write(&file, malformed).unwrap();
        let (restored, _) = DiskCache::open(&path, limits).unwrap();
        assert_eq!(restored.len(), 0, "invalid credential fact {invalid}");
    }
    fs::write(&file, &original[..offset]).unwrap();
    let (missing, _) = DiskCache::open(&path, limits).unwrap();
    assert_eq!(missing.len(), 0, "truncated missing credential fact");
    fs::remove_dir_all(path).unwrap();
}
