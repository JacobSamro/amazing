//! Assertion ports from WPT c48d58747e1f211527fb695fd60548a997fae617.
use amazing_fetch::api::{ApiError, ApiHeaders, HeaderGuard};
fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// fetch/api/headers/headers-basic.any.js, headers-casing.any.js,
// headers-combine.any.js, headers-normalize.any.js, header-values-normalize.any.js.
#[test]
fn basic_case_normalize_and_combine() {
    let mut h = ApiHeaders::default();
    h.append(&u("X-Name"), &u("\t one \t")).unwrap();
    h.append(&u("x-name"), &u("two")).unwrap();
    assert_eq!(h.get(&u("X-NAME")).unwrap().as_deref(), Some("one, two"));
    assert!(h.has(&u("x-name")).unwrap());
    h.set(&u("X-Name"), &u("three")).unwrap();
    assert_eq!(h.get(&u("x-name")).unwrap().as_deref(), Some("three"));
    h.delete(&u("X-Name")).unwrap();
    assert!(!h.has(&u("x-name")).unwrap());
    assert_eq!(h.get(&u("missing")).unwrap(), None);
}

// fetch/api/headers/headers-record.any.js and headers-basic.any.js.
#[test]
fn typed_sequence_and_record_fill() {
    let rows = vec![vec![u("B"), u("2")], vec![u("a"), u("1")]];
    let sequence = ApiHeaders::from_sequence(HeaderGuard::None, &rows).unwrap();
    assert_eq!(
        sequence.sorted_entries(),
        vec![("a".into(), "1".into()), ("b".into(), "2".into())]
    );
    let record = ApiHeaders::from_pairs(HeaderGuard::None, &[(u("A"), u("x"))]).unwrap();
    assert_eq!(record.get_string("a").unwrap().as_deref(), Some("x"));
}

// fetch/api/headers/header-setcookie.any.js and headers-combine.any.js.
#[test]
fn sorted_iteration_keeps_set_cookie_separate() {
    let mut h = ApiHeaders::default();
    for (n, v) in [
        ("z", "1"),
        ("set-cookie", "a=1"),
        ("X", "a"),
        ("set-cookie", "b=2"),
        ("x", "b"),
    ] {
        h.append(&u(n), &u(v)).unwrap();
    }
    assert_eq!(h.get_set_cookie(), vec!["a=1", "b=2"]);
    assert_eq!(
        h.get(&u("set-cookie")).unwrap().as_deref(),
        Some("a=1, b=2")
    );
    assert_eq!(
        h.sorted_entries(),
        vec![
            ("set-cookie".into(), "a=1".into()),
            ("set-cookie".into(), "b=2".into()),
            ("x".into(), "a, b".into()),
            ("z".into(), "1".into())
        ]
    );
}

// fetch/api/headers/headers-errors.any.js and header-values.any.js.
#[test]
fn invalid_names_values_and_webidl_bytestring() {
    let mut h = ApiHeaders::default();
    assert_eq!(
        ApiHeaders::from_sequence(HeaderGuard::None, &[vec![u("one")]]),
        Err(ApiError::InvalidHeaderTuple)
    );
    assert_eq!(
        ApiHeaders::from_sequence(HeaderGuard::None, &[vec![u("one"), u("two"), u("three")]]),
        Err(ApiError::InvalidHeaderTuple)
    );
    assert_eq!(
        h.append(&u("bad name"), &u("x")),
        Err(ApiError::InvalidHeaderName)
    );
    assert_eq!(
        h.append(&u("name"), &u("a\nb")),
        Err(ApiError::InvalidHeaderValue)
    );
    assert_eq!(h.get(&u("bad:name")), Err(ApiError::InvalidHeaderName));
    assert_eq!(h.append(&u("name"), &u("Ā")), Err(ApiError::ByteString));
    h.append(&u("name"), &u("é")).unwrap();
    assert_eq!(h.get(&u("name")).unwrap().as_deref(), Some("é"));
}

// fetch/api/headers/headers-forbidden-override.any.js, headers-no-cors.any.js.
#[test]
fn guards_ignore_forbidden_names_and_reject_mutation_when_immutable() {
    let mut request = ApiHeaders::new(HeaderGuard::Request);
    request.append(&u("Cookie"), &u("a=b")).unwrap();
    request.append(&u("Sec-Fetch-Mode"), &u("cors")).unwrap();
    request
        .append(&u("X-HTTP-Method-Override"), &u("TRACE"))
        .unwrap();
    assert!(!request.has(&u("cookie")).unwrap());
    assert!(!request.has(&u("sec-fetch-mode")).unwrap());
    assert!(!request.has(&u("x-http-method-override")).unwrap());
    request
        .append(&u("X-HTTP-Method-Override"), &u("PATCH"))
        .unwrap();
    assert!(request.has(&u("x-http-method-override")).unwrap());
    let mut response = ApiHeaders::new(HeaderGuard::Response);
    response.append(&u("Set-Cookie"), &u("a=b")).unwrap();
    assert!(!response.has(&u("set-cookie")).unwrap());
    response.set_guard(HeaderGuard::Immutable);
    assert_eq!(
        response.append(&u("x"), &u("y")),
        Err(ApiError::ImmutableHeaders)
    );
}

// fetch/api/headers/headers-no-cors.any.js.
#[test]
fn no_cors_combined_safelist_and_privileged_range_removal() {
    let mut h = ApiHeaders::new(HeaderGuard::RequestNoCors);
    h.append(&u("accept"), &u(&"a".repeat(127))).unwrap();
    h.append(&u("accept"), &u("b")).unwrap();
    assert_eq!(
        h.get(&u("accept")).unwrap().as_deref(),
        Some("a".repeat(127).as_str())
    );
    h.set(&u("x-custom"), &u("x")).unwrap();
    assert!(!h.has(&u("x-custom")).unwrap());
    h.append(&u("range"), &u("bytes=0-1")).unwrap();
    assert!(!h.has(&u("range")).unwrap());
    h.delete(&u("accept")).unwrap();
    assert!(!h.has(&u("accept")).unwrap());
}
