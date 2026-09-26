//! NET-36: exercises the WPT `websockets/` handler behaviors ported into
//! `tests/support/wpt_server.rs` (RFC 6455 upgrade, echo/close/handshake
//! handlers) directly against the shim, with a minimal hand-rolled RFC 6455
//! client. This is independent of the installed JS wpt-harness, which is
//! currently blocked on an unrelated `{{ports[ws][0]}}` template-substitution
//! gap in `crates/amazing/src/cmd/wpt_harness.rs` (see
//! `.agents/net/questions.md`) — these tests prove the shim's own protocol
//! handling is correct so that gate is not held on that fix landing.
#![allow(dead_code)]
#[path = "support/wpt_server.rs"]
mod wpt_server;

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};
use wpt_server::WptServer;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

fn sha1(input: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (input.len() as u64) * 8;
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
fn expected_accept() -> String {
    base64(&sha1(format!("{KEY}{GUID}").as_bytes()))
}

struct Client {
    stream: TcpStream,
    /// Bytes already read past the header terminator during handshake
    /// (only non-empty for handlers that coalesce a frame into the same
    /// packet as the handshake response, e.g. `simple_handshake`).
    leftover: Vec<u8>,
}
struct Handshake {
    status: u16,
    headers: HashMap<String, String>,
}
impl Client {
    fn connect(server: &WptServer, path: &str, extra: &[(&str, &str)]) -> (Self, Handshake) {
        let addr = server.url("").trim_start_matches("http://").to_string();
        let mut stream = TcpStream::connect(&addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {KEY}\r\n"
        );
        for (name, value) in extra {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "connection closed before handshake completed");
            buffer.extend_from_slice(&chunk[..n]);
            if let Some(i) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let leftover = buffer[header_end..].to_vec();
        let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        let mut lines = head.split("\r\n");
        let status_line = lines.next().unwrap_or("");
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(
                    name.trim().to_ascii_lowercase(),
                    value.trim().to_string(),
                );
            }
        }
        (Self { stream, leftover }, Handshake { status, headers })
    }
    fn send(&mut self, opcode: u8, payload: &[u8]) {
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut out = vec![0x80 | opcode];
        let len = payload.len();
        if len < 126 {
            out.push(0x80 | len as u8);
        } else if len <= 0xFFFF {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
        out.extend_from_slice(&mask);
        for (i, byte) in payload.iter().enumerate() {
            out.push(byte ^ mask[i % 4]);
        }
        self.stream.write_all(&out).unwrap();
    }
    fn send_text(&mut self, text: &str) {
        self.send(0x1, text.as_bytes());
    }
    fn send_close(&mut self, code: Option<u16>, reason: &str) {
        let body = match code {
            Some(code) => {
                let mut body = code.to_be_bytes().to_vec();
                body.extend_from_slice(reason.as_bytes());
                body
            }
            None => Vec::new(),
        };
        self.send(0x8, &body);
    }
    fn fill(&mut self, want: usize) {
        while self.leftover.len() < want {
            let mut chunk = [0u8; 4096];
            let n = self.stream.read(&mut chunk).unwrap();
            assert!(n > 0, "connection closed early");
            self.leftover.extend_from_slice(&chunk[..n]);
        }
    }
    /// Reads one server (unmasked) frame: `(opcode, payload)`.
    fn recv(&mut self) -> Option<(u8, Vec<u8>)> {
        self.fill(2);
        let head: [u8; 2] = [self.leftover[0], self.leftover[1]];
        let opcode = head[0] & 0x0F;
        let mut len = (head[1] & 0x7F) as u64;
        let mut consumed = 2;
        if len == 126 {
            self.fill(4);
            len = u16::from_be_bytes([self.leftover[2], self.leftover[3]]) as u64;
            consumed = 4;
        } else if len == 127 {
            self.fill(10);
            len = u64::from_be_bytes(self.leftover[2..10].try_into().unwrap());
            consumed = 10;
        }
        self.fill(consumed + len as usize);
        let payload = self.leftover[consumed..consumed + len as usize].to_vec();
        self.leftover.drain(..consumed + len as usize);
        Some((opcode, payload))
    }
    /// True if the connection closes (EOF) within a short window without
    /// sending any more bytes.
    fn expect_no_more_bytes(&mut self, within: Duration) -> bool {
        if !self.leftover.is_empty() {
            return false;
        }
        self.stream.set_read_timeout(Some(within)).unwrap();
        let mut buf = [0u8; 16];
        match self.stream.read(&mut buf) {
            Ok(0) => true,                                       // EOF: connection closed.
            Ok(_) => false,                                      // More bytes arrived.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => true,
            Err(e) => panic!("unexpected read error: {e}"),
        }
    }
}

#[test]
fn echo_round_trip_and_goodbye_close() {
    let server = WptServer::new();
    let (mut client, hs) = Client::connect(&server, "/echo", &[]);
    assert_eq!(hs.status, 101);
    assert_eq!(hs.headers.get("sec-websocket-accept").unwrap(), &expected_accept());
    client.send_text("hello");
    assert_eq!(client.recv(), Some((0x1, b"hello".to_vec())));
    client.send_text("Goodbye");
    assert_eq!(client.recv(), Some((0x1, b"Goodbye".to_vec())));
    let (opcode, payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &1000u16.to_be_bytes());
}

#[test]
fn echo_protocol_negotiation() {
    let server = WptServer::new();
    let (_client, hs) = Client::connect(&server, "/echo", &[("Sec-WebSocket-Protocol", "chat, echo")]);
    assert_eq!(hs.headers.get("sec-websocket-protocol").unwrap(), "echo");
    let (_client2, hs2) = Client::connect(&server, "/echo", &[("Sec-WebSocket-Protocol", "chat")]);
    assert!(hs2.headers.get("sec-websocket-protocol").is_none());
}

#[test]
fn echo_client_initiated_close_is_echoed_default() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/echo", &[]);
    client.send_close(Some(1002), "bye");
    let (opcode, payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &1002u16.to_be_bytes());
    assert_eq!(&payload[2..], b"bye");
}

#[test]
fn echo_raw_passes_bytes_through_unframed_and_exits() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/echo_raw", &[]);
    client.send_text("abc");
    // The handler writes the raw payload directly, with no WS frame header.
    let mut raw = [0u8; 3];
    client.stream.read_exact(&mut raw).unwrap();
    assert_eq!(&raw, b"abc");
    client.send_text("exit");
    let (opcode, payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &1000u16.to_be_bytes());
}

#[test]
fn echo_close_data_ignores_messages_until_goodbye() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/echo_close_data", &[]);
    client.send_text("ignored");
    assert!(
        client.expect_no_more_bytes(Duration::from_millis(300)),
        "echo_close_data must not echo non-Goodbye messages"
    );
    client.send_text("Goodbye");
    let (opcode, payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &1000u16.to_be_bytes());
}

#[test]
fn remote_close_with_explicit_code_and_reason() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/remote-close?code=4001&reason=bye", &[]);
    let (opcode, payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &4001u16.to_be_bytes());
    assert_eq!(&payload[2..], b"bye");
}

#[test]
fn remote_close_abrupt_sends_ping_then_drops() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/remote-close?abrupt=1", &[]);
    let (opcode, _payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x9, "abrupt remote-close pings before dropping");
    let mut buf = [0u8; 8];
    client
        .stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let result = client.stream.read(&mut buf);
    assert!(matches!(result, Ok(0)) || result.is_err(), "connection should drop without a close frame");
}

#[test]
fn passive_close_abort_drops_without_replying() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/passive-close-abort", &[]);
    client.send_close(Some(1000), "");
    assert!(
        client.expect_no_more_bytes(Duration::from_millis(500)),
        "passive-close-abort must not send a close frame back"
    );
}

#[test]
fn delayed_passive_close_waits_about_a_second() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/delayed-passive-close", &[]);
    client.send_close(Some(1000), "");
    let started = Instant::now();
    let (opcode, payload) = client.recv().unwrap();
    let elapsed = started.elapsed();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &1000u16.to_be_bytes());
    assert!(elapsed >= Duration::from_millis(900), "elapsed={elapsed:?}");
}

#[test]
fn handshake_no_protocol_ignores_requested_protocol() {
    let server = WptServer::new();
    let (_client, hs) =
        Client::connect(&server, "/handshake_no_protocol", &[("Sec-WebSocket-Protocol", "chat")]);
    assert_eq!(hs.status, 101);
    assert!(hs.headers.get("sec-websocket-protocol").is_none());
}

#[test]
fn handshake_protocol_forces_foobar() {
    let server = WptServer::new();
    let (_client, hs) = Client::connect(&server, "/handshake_protocol", &[]);
    assert_eq!(hs.headers.get("sec-websocket-protocol").unwrap(), "foobar");
}

#[test]
fn protocol_echoes_raw_header_value_verbatim() {
    let server = WptServer::new();
    let (mut client, hs) =
        Client::connect(&server, "/protocol", &[("Sec-WebSocket-Protocol", "chat, superchat")]);
    assert_eq!(hs.headers.get("sec-websocket-protocol").unwrap(), "chat, superchat");
    assert_eq!(client.recv(), Some((0x1, b"chat, superchat".to_vec())));
}

#[test]
fn protocol_array_echoes_first_comma_token() {
    let server = WptServer::new();
    let (mut client, hs) =
        Client::connect(&server, "/protocol_array", &[("Sec-WebSocket-Protocol", "chat,superchat")]);
    assert_eq!(hs.headers.get("sec-websocket-protocol").unwrap(), "chat");
    assert_eq!(client.recv(), Some((0x1, b"chat".to_vec())));
}

#[test]
fn origin_echoes_handshake_origin_header() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/origin", &[("Origin", "http://example.test")]);
    assert_eq!(client.recv(), Some((0x1, b"http://example.test".to_vec())));
}

#[test]
fn referrer_reports_missing_as_per_fetch() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/referrer", &[]);
    assert_eq!(client.recv(), Some((0x1, b"MISSING AS PER FETCH".to_vec())));
}

#[test]
fn echo_cookie_echoes_or_reports_none() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/echo-cookie", &[("Cookie", "foo=bar")]);
    assert_eq!(client.recv(), Some((0x1, b"foo=bar".to_vec())));
    let (mut client2, _hs2) = Client::connect(&server, "/echo-cookie", &[]);
    assert_eq!(client2.recv(), Some((0x1, b"(none)".to_vec())));
}

#[test]
fn set_cookie_handlers_add_expected_headers() {
    let server = WptServer::new();
    let (_c, hs) = Client::connect(&server, "/set-cookie?1", &[]);
    assert_eq!(hs.headers.get("set-cookie").unwrap(), "ws_test_1=test; Path=/");
    let (_c2, hs2) = Client::connect(&server, "/set-cookie-secure?2", &[]);
    assert_eq!(hs2.headers.get("set-cookie").unwrap(), "ws_test_2=test; Secure; Path=/");
}

#[test]
fn wrong_accept_key_sends_deliberately_bad_accept() {
    let server = WptServer::new();
    let (mut client, hs) = Client::connect(&server, "/wrong_accept_key", &[]);
    assert_eq!(hs.status, 101);
    assert_eq!(hs.headers.get("sec-websocket-accept").unwrap(), "thisisawrongacceptkey");
    assert_ne!(hs.headers.get("sec-websocket-accept").unwrap(), &expected_accept());
    assert_eq!(client.recv(), Some((0x1, b"test".to_vec())));
}

#[test]
fn simple_handshake_coalesces_close_frame_with_response() {
    let server = WptServer::new();
    let (mut client, hs) = Client::connect(&server, "/simple_handshake", &[]);
    assert_eq!(hs.status, 101);
    assert_eq!(hs.headers.get("set-cookie").unwrap(), "ws_test=test");
    let (opcode, payload) = client.recv().unwrap();
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &1001u16.to_be_bytes());
    assert_eq!(&payload[2..], b"PASS");
}

#[test]
fn invalid_handler_sends_garbage_instead_of_a_handshake() {
    let server = WptServer::new();
    let addr = server.url("").trim_start_matches("http://").to_string();
    let mut stream = TcpStream::connect(&addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let request = format!(
        "GET /invalid HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {KEY}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut buf = [0u8; 32];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"FOO BAR BAZ\r\n\r\n");
}

#[test]
fn basic_auth_requires_correct_credentials() {
    let server = WptServer::new();
    let (_client, hs) = Client::connect(&server, "/basic_auth", &[]);
    assert_eq!(hs.status, 401);
    let (client_ok, hs_ok) =
        Client::connect(&server, "/basic_auth", &[("Authorization", "Basic Zm9vOmJhcg==")]);
    assert_eq!(hs_ok.status, 101);
    drop(client_ok);
}

#[test]
fn empty_message_pass_and_fail() {
    let server = WptServer::new();
    let (mut client, _hs) = Client::connect(&server, "/empty-message", &[]);
    client.send_text("");
    assert_eq!(client.recv(), Some((0x1, b"pass".to_vec())));
    let (mut client2, _hs2) = Client::connect(&server, "/empty-message", &[]);
    client2.send_text("not empty");
    assert_eq!(client2.recv(), Some((0x1, b"fail".to_vec())));
}

#[test]
fn unknown_handler_fails_opening_handshake_with_404() {
    let server = WptServer::new();
    let (_client, hs) = Client::connect(&server, "/does-not-exist", &[]);
    assert_eq!(hs.status, 404);
}
