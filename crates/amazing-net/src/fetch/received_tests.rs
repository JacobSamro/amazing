//! Successful synchronous-router provenance, independent of Response URL text.
use super::*;
use super::tests::{file_url_of, temp_file};
use crate::http::tests::serve;

#[test]
fn local_receipt_retains_bare_path_and_file_url_without_rewriting() {
    let path = temp_file("received-local", "html", b"<p>local</p>");
    for input in [path.to_str().unwrap().to_owned(), file_url_of(&path)] {
        let received = fetch_received(&input, &FetchOptions::default()).unwrap();
        assert!(received.is_local_file());
        assert_eq!(received.response().status, 200);
        assert_eq!(received.response().url, input);
        assert_eq!(received.response().body, b"<p>local</p>");
        assert_eq!(received.clone().into_response(), fetch(&input, &FetchOptions::default()).unwrap());
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn receipt_router_uses_original_bare_path_bytes_and_file_url_decoding() {
    let path = temp_file("received space", "txt", b"space");
    for input in [format!("  {}  ", path.to_str().unwrap()), file_url_of(&path).replace(' ', "%20")] {
        let received = fetch_received(&input, &FetchOptions::default()).unwrap();
        assert!(received.is_local_file());
        assert_eq!(received.response().body, b"space");
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn network_response_with_reporting_headers_never_mints_local_authority() {
    let (port, rx) = serve(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nReporting-Endpoints: csp=\"/report\"\r\n\r\nok".to_vec()]);
    let received = fetch_received(&format!("http://127.0.0.1:{port}/local.html"), &FetchOptions::default()).unwrap();
    assert!(!received.is_local_file());
    assert_eq!(received.response().body, b"ok");
    assert_eq!(received.response().headers.get("reporting-endpoints"), Some("csp=\"/report\""));
    assert!(String::from_utf8(rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap().starts_with("GET /local.html HTTP/1.1\r\n"));
    let clone = received.clone();
    assert!(!clone.is_local_file());
    assert_eq!(clone.into_response(), received.into_response());
}

#[test]
fn all_network_redirect_statuses_retain_actual_network_provenance() {
    for status in [301, 302, 303, 307, 308] {
        let (port, rx) = serve(vec![format!("HTTP/1.1 {status} Redirect\r\nLocation: /final#kept\r\nContent-Length: 0\r\n\r\n").into_bytes(), b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()]);
        let received = fetch_received(&format!("http://127.0.0.1:{port}/start"), &FetchOptions::default()).unwrap();
        assert!(!received.is_local_file());
        assert_eq!(received.response().url, format!("http://127.0.0.1:{port}/final#kept"));
        assert_eq!(received.into_response().body, b"ok");
        for target in ["/start", "/final"] {
            assert!(String::from_utf8(rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap().starts_with(&format!("GET {target} HTTP/1.1\r\n")));
        }
    }
}

#[test]
fn network_redirect_to_real_existing_local_file_is_rejected() {
    let path = temp_file("received-secret", "txt", b"secret");
    let (port, rx) = serve(vec![format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n", file_url_of(&path)).into_bytes()]);
    assert!(matches!(fetch_received(&format!("http://127.0.0.1:{port}/start"), &FetchOptions::default()), Err(NetError::UnsupportedScheme(s)) if s == "file"));
    assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), b"secret");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn local_errors_and_size_boundaries_never_return_success_receipts() {
    let path = temp_file("received-limit", "txt", b"1234");
    let options = FetchOptions { max_response_bytes: 4, ..FetchOptions::default() };
    for input in [path.to_str().unwrap().to_owned(), file_url_of(&path)] {
        assert!(fetch_received(&input, &options).unwrap().is_local_file());
        let smaller = FetchOptions { max_response_bytes: 3, ..options.clone() };
        assert!(matches!(fetch_received(&input, &smaller), Err(NetError::ResponseTooLarge)));
    }
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(fetch_received(path.to_str().unwrap(), &options), Err(NetError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound));
    assert!(fetch_received("", &options).is_err());
    assert!(matches!(fetch_received("ftp://example.test/", &options), Err(NetError::UnsupportedScheme(s)) if s == "ftp"));
}

#[test]
fn network_framing_and_response_limit_failures_do_not_become_local_receipts() {
    for (response, limit) in [(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nno".to_vec(), 4), (b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n1234".to_vec(), 3)] {
        let (port, rx) = serve(vec![response]);
        let options = FetchOptions { max_response_bytes: limit, ..FetchOptions::default() };
        assert!(fetch_received(&format!("http://127.0.0.1:{port}/"), &options).is_err());
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    }
}

fn drain(response: &mut crate::StreamingResponse) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().unwrap() { body.extend(chunk); }
    body
}

#[test]
fn streaming_local_source_copies_without_body_and_survives_drain_or_drop() {
    let path = temp_file("received-stream", "txt", b"abcdefgh");
    for input in [path.to_str().unwrap().to_owned(), file_url_of(&path)] {
        let mut response = crate::fetch_stream(&input, &FetchOptions::default(), crate::StreamOptions { chunk_bytes: 2, ..Default::default() }).unwrap();
        let source = response.source();
        assert!(source.is_local_file());
        assert_eq!(source, fetch_received(&input, &FetchOptions::default()).unwrap().source());
        assert_eq!(drain(&mut response), b"abcdefgh");
        assert_eq!(source, response.source());
        drop(response);
        assert!(source.is_local_file());
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn streaming_http_headers_and_every_redirect_status_remain_network_source() {
    for status in [200, 301, 302, 303, 307, 308] {
        let mut script = Vec::new();
        if status != 200 { script.push(format!("HTTP/1.1 {status} Redirect\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n").into_bytes()); }
        script.push(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nReporting-Endpoints: csp=\"/report\"\r\n\r\nbody".to_vec());
        let (port, rx) = serve(script);
        let mut response = crate::fetch_stream(&format!("http://127.0.0.1:{port}/start"), &FetchOptions::default(), crate::StreamOptions::default()).unwrap();
        let source = response.source();
        assert!(!source.is_local_file());
        assert_eq!(response.headers.get("reporting-endpoints"), Some("csp=\"/report\""));
        assert_eq!(drain(&mut response), b"body");
        assert_eq!(source, response.source());
        for target in if status == 200 { vec!["/start"] } else { vec!["/start", "/final"] } {
            assert!(String::from_utf8(rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap().starts_with(&format!("GET {target} HTTP/1.1\r\n")));
        }
    }
}

#[test]
fn streaming_network_to_file_and_local_admission_errors_mint_no_local_receipt() {
    let path = temp_file("received-stream-reject", "txt", b"1234");
    let (port, rx) = serve(vec![format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n", file_url_of(&path)).into_bytes()]);
    assert!(matches!(crate::fetch_stream(&format!("http://127.0.0.1:{port}/start"), &FetchOptions::default(), crate::StreamOptions::default()), Err(NetError::UnsupportedScheme(s)) if s == "file"));
    assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    let smaller = FetchOptions { max_response_bytes: 3, ..Default::default() };
    assert!(matches!(crate::fetch_stream(path.to_str().unwrap(), &smaller, crate::StreamOptions::default()), Err(NetError::ResponseTooLarge)));
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(crate::fetch_stream(path.to_str().unwrap(), &FetchOptions::default(), crate::StreamOptions::default()), Err(NetError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound));
}

#[test]
fn streaming_network_source_never_becomes_local_after_late_body_failure() {
    let (port, rx) = serve(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nno".to_vec()]);
    let mut response = crate::fetch_stream(&format!("http://127.0.0.1:{port}/"), &FetchOptions::default(), crate::StreamOptions::default()).unwrap();
    let source = response.source();
    assert!(!source.is_local_file());
    let mut failed = false;
    loop { match response.next_chunk() { Ok(Some(_)) => {}, Ok(None) => break, Err(_) => { failed = true; break; } } }
    assert!(failed);
    assert_eq!(source, response.source());
    assert_eq!(response.next_chunk().unwrap(), None);
    assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
}
