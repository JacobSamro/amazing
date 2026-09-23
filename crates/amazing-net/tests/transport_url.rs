use amazing_net::{fetch, FetchOptions, NetError, ParsedUrl};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

// Exercise the public loading path and actual bytes on the wire, not just getters.
fn server(responses: Vec<String>) -> (u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        responses
            .into_iter()
            .map(|response| {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                    assert!(bytes.len() < 16384);
                }
                socket.write_all(response.as_bytes()).unwrap();
                String::from_utf8(bytes).unwrap()
            })
            .collect()
    });
    (port, worker)
}
fn ok() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".into()
}
fn moved(location: &str) -> String {
    format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n")
}
#[test]
fn loading_canonicalizes_host_path_and_wire_target() {
    let (port, worker) = server(vec![ok()]);
    let response = fetch(
        &format!("h\tttp://u:p@0x7f000001:{port}/a/%2e%2e/é ?#secret"),
        &FetchOptions::default(),
    )
    .unwrap();
    let wire = &worker.join().unwrap()[0];
    assert!(wire.starts_with("GET /%C3%A9%20? HTTP/1.1\r\n"), "{wire}");
    assert!(wire.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
    assert!(!wire.contains("u:p"));
    assert!(!wire.contains("secret"));
    assert_eq!(
        response.url,
        format!("http://u:p@127.0.0.1:{port}/%C3%A9%20?#secret")
    );
}
#[test]
fn loading_redirects_resolve_dot_segments_and_inherit_fragments() {
    let (port, worker) = server(vec![moved("../%2e/next?"), moved("#"), ok()]);
    let response = fetch(
        &format!("http://127.1:{port}/a/start#keep"),
        &FetchOptions::default(),
    )
    .unwrap();
    let wires = worker.join().unwrap();
    assert!(wires[1].starts_with("GET /next? HTTP/1.1\r\n"));
    assert!(wires[2].starts_with("GET /next? HTTP/1.1\r\n"));
    assert_eq!(response.url, format!("http://127.0.0.1:{port}/next?#"));
}
#[test]
fn loading_redirect_inherits_missing_fragment() {
    let (port, worker) = server(vec![moved("/end"), ok()]);
    let response = fetch(
        &format!("http://127.1:{port}/start#keep"),
        &FetchOptions::default(),
    )
    .unwrap();
    worker.join().unwrap();
    assert_eq!(response.url, format!("http://127.0.0.1:{port}/end#keep"));
}
#[test]
fn network_redirects_cannot_load_local_files_or_unsupported_schemes() {
    for location in ["file:///C:/Windows/win.ini", "data:text/plain,secret"] {
        let (port, worker) = server(vec![moved(location)]);
        assert!(matches!(
            fetch(&format!("http://127.1:{port}/"), &FetchOptions::default()),
            Err(NetError::UnsupportedScheme(_))
        ));
        worker.join().unwrap();
    }
}
#[test]
fn malformed_urls_fail_before_connecting_and_idna_hosts_parse() {
    for input in ["http://x:65536/", "http://[invalid]/"] {
        assert!(matches!(
            fetch(input, &FetchOptions::default()),
            Err(NetError::Url(_))
        ));
    }
    for input in ["http://é.test/", "http://%C3%A9.test/"] {
        assert_eq!(ParsedUrl::parse(input).unwrap().hostname(), "xn--9ca.test");
    }
    assert!(matches!(
        fetch("data:text/plain,ok", &FetchOptions::default()),
        Err(NetError::UnsupportedScheme(_))
    ));
}
#[test]
fn file_conversion_rejects_remote_hosts_nul_and_encoded_separators() {
    for input in [
        "file://server/share",
        "file:///x%00y",
        "file:///x%2fy",
        "file:///x%FF",
    ] {
        assert!(
            ParsedUrl::parse(input).unwrap().to_file_path().is_err(),
            "{input}"
        );
    }
    #[cfg(windows)]
    for input in [
        "file:///C:/x%5cy",
        "file:////server/share",
        "file:///%5c%5cserver/share",
    ] {
        assert!(
            ParsedUrl::parse(input).unwrap().to_file_path().is_err(),
            "{input}"
        );
    }
}
#[test]
fn local_file_loading_decodes_unicode_spaces_and_normalizes_localhost() {
    let path = std::env::temp_dir().join(format!("net3-{}-é file.txt", std::process::id()));
    std::fs::write(&path, b"local").unwrap();
    let native = path.to_string_lossy().replace('\\', "/");
    let native = if native.starts_with('/') {
        native
    } else {
        format!("/{native}")
    };
    let input = format!(
        "file://localhost{}?ignored#fragment",
        native.replace(' ', "%20")
    );
    let result = fetch(&input, &FetchOptions::default());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(result.unwrap().body, b"local");
}
#[test]
fn http_target_rejects_non_http_urls_and_preserves_empty_query() {
    assert_eq!(
        ParsedUrl::parse("https://u:p@[::1]:443?#f")
            .unwrap()
            .http_request_target()
            .unwrap(),
        "/?"
    );
    assert!(ParsedUrl::parse("file:///x")
        .unwrap()
        .http_request_target()
        .is_err());
}
