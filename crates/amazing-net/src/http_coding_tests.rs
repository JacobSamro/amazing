use super::*;
use crate::coding::tests::fixture;
use std::time::Duration;

fn raw(fields: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("HTTP/1.1 200 OK\r\n{fields}\r\n").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
fn parse(bytes: &[u8], limit: usize, unclean: bool) -> Result<Response, NetError> {
    response_from_raw_with_limit(
        &ParsedUrl::parse("http://a.test/").unwrap(),
        bytes,
        unclean,
        limit,
    )
}
fn chunks(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    for chunk in bytes.chunks(7) {
        result.extend_from_slice(format!("{:x};ext=ok\r\n", chunk.len()).as_bytes());
        result.extend_from_slice(chunk);
        result.extend_from_slice(b"\r\n");
    }
    result.extend_from_slice(b"0\r\nX-Trailer: ignored\r\n\r\n");
    result
}
#[test]
fn real_http_path_decodes_gzip_zlib_and_chunked_gzip() {
    for (coding, name, chunked) in [
        ("gzip", "hello-fixed-gzip", false),
        ("deflate", "hello-default-zlib", false),
        ("gzip", "hello-fixed-gzip", true),
    ] {
        let (plain, encoded) = fixture(name);
        let (framing, wire) = if chunked {
            ("Transfer-Encoding: chunked\r\n".into(), chunks(&encoded))
        } else {
            (format!("Content-Length: {}\r\n", encoded.len()), encoded)
        };
        let script = raw(&format!("Content-Encoding: {coding}\r\n{framing}"), &wire);
        let (port, requests) = tests::serve(vec![script]);
        let response = crate::fetch(
            &format!("http://127.0.0.1:{port}/compressed"),
            &FetchOptions {
                max_response_bytes: plain.len(),
                request_headers: vec![("Accept-Encoding".into(), "br".into())],
                ..FetchOptions::default()
            },
        )
        .unwrap();
        assert_eq!(response.body, plain);
        assert_eq!(response.headers.get("content-encoding"), Some(coding));
        let request =
            String::from_utf8(requests.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(request.matches("Accept-Encoding:").count(), 1);
        assert!(request.contains("Accept-Encoding: gzip, deflate\r\n"));
        assert!(!request.contains("br\r\n"));
    }
}
#[test]
fn transfer_codings_and_content_layers_decode_in_reverse_header_order() {
    let (plain, gzip) = fixture("hello-fixed-gzip");
    let wire = chunks(&gzip);
    assert_eq!(
        parse(
            &raw("Transfer-Encoding: GZip, chunked\r\n", &wire),
            1024,
            true
        )
        .unwrap()
        .body,
        plain
    );
    assert_eq!(
        parse(&raw("Transfer-Encoding: gzip\r\n", &gzip), 1024, false)
            .unwrap()
            .body,
        plain
    );
    assert!(parse(&raw("Transfer-Encoding: gzip\r\n", &gzip), 1024, true).is_err());
    let stacked = include_bytes!("../tests/coding/stacked.bin");
    for fields in [
        "Content-Encoding: gzip, deflate\r\n",
        "Content-Encoding: gzip\r\nContent-Encoding: deflate\r\n",
        "Content-Encoding: gzip\r\nTransfer-Encoding: deflate\r\n",
    ] {
        assert_eq!(
            parse(&raw(fields, stacked), 1024, false).unwrap().body,
            plain
        );
    }
}
#[test]
fn declared_content_length_is_encoded_length_not_decoded_length() {
    let (plain, gzip) = fixture("overlap-default-gzip");
    let bytes = raw(
        &format!(
            "Content-Encoding: gzip\r\nContent-Length: {}\r\n",
            gzip.len()
        ),
        &gzip,
    );
    assert_eq!(parse(&bytes, plain.len(), true).unwrap().body, plain);
    assert!(matches!(
        parse(&bytes, plain.len() - 1, true),
        Err(NetError::ResponseTooLarge)
    ));
    for length in [gzip.len() - 1, gzip.len() + 1, plain.len()] {
        assert!(parse(
            &raw(
                &format!("Content-Encoding: gzip\r\nContent-Length: {length}\r\n"),
                &gzip
            ),
            plain.len(),
            false
        )
        .is_err());
    }
}
#[test]
fn ambiguous_lengths_and_invalid_coding_chains_fail_closed() {
    for fields in [
        "Content-Length: 1\r\nContent-Length: 2\r\n", "Content-Length: +1\r\n", "Content-Length: \r\n",
        "Transfer-Encoding: chunked\r\nContent-Length: 1\r\n", "Transfer-Encoding: chunked, gzip\r\n",
        "Transfer-Encoding: chunked, chunked\r\n", "Content-Encoding: br\r\n", "Content-Encoding: gzip,\r\n",
        "Content-Encoding: chunked\r\n", "Transfer-Encoding: identity\r\n", "Transfer-Encoding: \r\n",
        "Content-Encoding: identity,identity,identity,identity,identity,identity,identity,identity,identity\r\n",
    ] { assert!(parse(&raw(fields, b"a"),1024,false).is_err(), "{fields}"); }
    assert_eq!(
        parse(
            &raw("Content-Length: 1, 1\r\nContent-Length: 1\r\n", b"a"),
            1,
            true
        )
        .unwrap()
        .body,
        b"a"
    );
}
#[test]
fn incomplete_chunk_trailers_and_trailing_garbage_cannot_validate_tls_eof() {
    for body in [
        b"0\r\n".as_slice(),
        b"0\r\nX-T: yes\r\n",
        b"0\r\n\r\nextra",
        b"0\r\ninvalid\r\n\r\n",
    ] {
        assert!(parse(&raw("Transfer-Encoding: chunked\r\n", body), 1024, true).is_err());
    }
    assert!(parse(
        &raw("Transfer-Encoding: chunked\r\n", b"0\r\n\r\n"),
        0,
        true
    )
    .unwrap()
    .body
    .is_empty());
}
#[test]
fn bodyless_responses_do_not_decode_representation_metadata() {
    for status in [204, 304] {
        let bytes = format!(
            "HTTP/1.1 {status} Empty\r\nContent-Encoding: gzip\r\nContent-Length: 1234\r\n\r\n"
        );
        assert!(parse(bytes.as_bytes(), 0, true).unwrap().body.is_empty());
        assert!(parse(format!("{bytes}unexpected").as_bytes(), 1024, false).is_err());
    }
}
#[test]
fn corrupted_gzip_and_unknown_coding_never_reach_the_consumer() {
    let (_, mut gzip) = fixture("hello-fixed-gzip");
    let pos = gzip.len() - 8;
    gzip[pos] ^= 1;
    let (port, _) = tests::serve(vec![raw(
        &format!(
            "Content-Encoding: gzip\r\nContent-Length: {}\r\n",
            gzip.len()
        ),
        &gzip,
    )]);
    assert!(crate::fetch(
        &format!("http://127.0.0.1:{port}/"),
        &FetchOptions::default()
    )
    .is_err());
}
#[test]
fn real_http_expansion_is_bounded_by_caller_budget() {
    let (_, gzip) = fixture("overlap-default-gzip");
    let (port, _) = tests::serve(vec![raw(
        &format!(
            "Content-Encoding: gzip\r\nContent-Length: {}\r\n",
            gzip.len()
        ),
        &gzip,
    )]);
    assert!(matches!(
        crate::fetch(
            &format!("http://127.0.0.1:{port}/"),
            &FetchOptions {
                max_response_bytes: 100,
                ..FetchOptions::default()
            }
        ),
        Err(NetError::ResponseTooLarge)
    ));
}
