use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn fetched_http_body_is_parsed_into_dom() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind HTTP fixture");
    listener
        .set_nonblocking(true)
        .expect("configure HTTP fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "browser request timed out");
                    std::thread::sleep(Duration::from_millis(10));
                },
                Err(error) => panic!("accept browser request: {error}"),
            }
        };
        let mut request = [0u8; 1024];
        let length = stream.read(&mut request).expect("read request");
        assert!(String::from_utf8_lossy(&request[..length]).starts_with("GET /page HTTP/1.1\r\n"));
        let body = b"<!doctype html><p id='network'>fetched<div>parsed";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .expect("write response head");
        stream.write_all(body).expect("write response body");
    });

    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args(["fetch-parse", &format!("http://{address}/page")])
        .output()
        .expect("run fetch-parse artifact");
    server.join().expect("HTTP fixture thread");
    assert!(
        result.status.success(),
        "fetch-parse failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).expect("UTF-8 DOM dump");
    assert!(output.contains("<p id=\"network\">"), "{output}");
    assert!(output.contains("\"fetched\""), "{output}");
    assert!(output.contains("<div>"), "{output}");
    assert!(output.contains("\"parsed\""), "{output}");
}
