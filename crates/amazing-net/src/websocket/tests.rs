use super::*;
use std::{cell::RefCell, io::Cursor, rc::Rc};
fn fixed(bytes: &mut [u8]) -> Result<(), Error> {
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = [0x37, 0xfa, 0x21, 0x3d][i % 4];
    }
    Ok(())
}
struct Io {
    input: Cursor<Vec<u8>>,
    out: Rc<RefCell<Vec<u8>>>,
}
impl Read for Io {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(b)
    }
}
impl Write for Io {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.out.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn client(bytes: Vec<u8>, limits: Limits) -> (Client<Io>, Rc<RefCell<Vec<u8>>>) {
    let out = Rc::new(RefCell::new(Vec::new()));
    (
        Client::test_client(
            Io {
                input: Cursor::new(bytes),
                out: out.clone(),
            },
            limits,
            fixed,
        ),
        out,
    )
}
fn server(op: u8, fin: bool, data: &[u8]) -> Vec<u8> {
    let mut out = vec![op | if fin { 128 } else { 0 }];
    if data.len() < 126 {
        out.push(data.len() as u8);
    } else if data.len() <= 65535 {
        out.push(126);
        out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(data.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(data);
    out
}
fn client_frame(io: &mut impl Read) -> (u8, bool, Vec<u8>, [u8; 4]) {
    let mut header = [0; 2];
    io.read_exact(&mut header).unwrap();
    assert_ne!(header[1] & 128, 0);
    let len = match header[1] & 127 {
        126 => {
            let mut n = [0; 2];
            io.read_exact(&mut n).unwrap();
            u16::from_be_bytes(n) as usize
        }
        127 => {
            let mut n = [0; 8];
            io.read_exact(&mut n).unwrap();
            u64::from_be_bytes(n) as usize
        }
        n => n as usize,
    };
    assert!(len <= 1024 * 1024);
    let mut mask = [0; 4];
    io.read_exact(&mut mask).unwrap();
    let mut data = vec![0; len];
    io.read_exact(&mut data).unwrap();
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    (header[0] & 15, header[0] & 128 != 0, data, mask)
}
fn response(accept: &str, extra: &str) -> Vec<u8> {
    format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{extra}\r\n").into_bytes()
}
#[test]
fn rfc_handshake_key_accept_and_request_vector() {
    assert_eq!(
        handshake::base64(b"the sample nonce"),
        "dGhlIHNhbXBsZSBub25jZQ=="
    );
    assert_eq!(
        handshake::accept("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
    assert_eq!(
        handshake::base64(&(1u8..=16).collect::<Vec<_>>()),
        "AQIDBAUGBwgJCgsMDQ4PEA=="
    );
    fn nonce(b: &mut [u8]) -> Result<(), Error> {
        b.copy_from_slice(b"the sample nonce");
        Ok(())
    }
    let out = Rc::new(RefCell::new(Vec::new()));
    let mut io = Io {
        input: Cursor::new(response(
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=",
            "Sec-WebSocket-Protocol: chat\r\n",
        )),
        out: out.clone(),
    };
    let options = Options {
        origin: Some("http://example.com".into()),
        protocols: vec!["chat".into(), "superchat".into()],
        ..Options::default()
    };
    assert_eq!(
        handshake::perform(&mut io, "server.example.com", "/chat", &options, nonce).unwrap(),
        Some("chat".into())
    );
    let wire = String::from_utf8(out.borrow().clone()).unwrap();
    assert!(wire.starts_with("GET /chat HTTP/1.1\r\nHost: server.example.com\r\n"));
    for line in [
        "Origin: http://example.com\r\n",
        "Sec-WebSocket-Version: 13\r\n",
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
        "Sec-WebSocket-Protocol: chat, superchat\r\n",
    ] {
        assert!(wire.contains(line));
    }
}
#[test]
fn handshake_rejects_bad_accept_status_extensions_and_subprotocols() {
    let key = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
    let offered = vec!["chat".into()];
    for extra in [
        "Sec-WebSocket-Protocol: nope\r\n",
        "Sec-WebSocket-Protocol: Chat\r\n",
        "Sec-WebSocket-Protocol: chat, chat\r\n",
        "Sec-WebSocket-Protocol: chat\r\nSec-WebSocket-Protocol: chat\r\n",
        "Sec-WebSocket-Extensions: permessage-deflate\r\n",
        "Sec-WebSocket-Accept: duplicate\r\n",
        "Transfer-Encoding: chunked\r\n",
        "Content-Length: 0\r\n",
        " folded: value\r\n",
        "Bad Name: value\r\n",
        "x: bad\nvalue\r\n",
    ] {
        assert!(
            handshake::validate_response(&response(key, extra), key, &offered).is_err(),
            "{extra}"
        );
    }
    for (old, new) in [
        ("101 Switching Protocols", "200 OK"),
        ("HTTP/1.1", "HTTP/1.0"),
        ("Upgrade: websocket", "Upgrade: h2c"),
        ("keep-alive, Upgrade", "keep-alive"),
        (key, "bad"),
    ] {
        let bytes = String::from_utf8(response(key, ""))
            .unwrap()
            .replace(old, new);
        assert!(handshake::validate_response(bytes.as_bytes(), key, &offered).is_err());
    }
    assert_eq!(
        handshake::validate_response(&response(key, ""), key, &offered).unwrap(),
        None
    );
}
#[test]
fn handshake_stops_at_terminator_without_eating_first_frame() {
    let mut bytes = response("test", "");
    let length = bytes.len();
    bytes.extend(server(1, true, b"hi"));
    let mut io = Cursor::new(bytes);
    assert_eq!(handshake::read_headers(&mut io).unwrap().len(), length);
    assert_eq!(io.position(), length as u64);
    assert_eq!(frame::Header::read(&mut io, 100).unwrap().len, 2);
}
#[test]
fn handshake_and_request_bounds_and_injection() {
    let mut io = Cursor::new(vec![b'a'; handshake::MAX_HANDSHAKE + 1]);
    assert!(matches!(
        handshake::read_headers(&mut io),
        Err(Error::Limit)
    ));
    assert_eq!(io.position(), handshake::MAX_HANDSHAKE as u64);
    for (host, target) in [
        ("x\r\nInjected: yes", "/"),
        ("x", "/bad space"),
        ("x", "relative"),
        ("x", "/#fragment"),
    ] {
        assert!(handshake::validate_request(host, target, &Options::default()).is_err());
    }
    for protocols in [
        vec!["chat".into(), "chat".into()],
        vec!["".into()],
        vec!["bad,token".into()],
        vec!["x".repeat(257)],
        vec!["x".into(); 33],
    ] {
        assert!(handshake::validate_request(
            "x",
            "/",
            &Options {
                protocols,
                ..Options::default()
            }
        )
        .is_err());
    }
    assert!(handshake::validate_request(
        "x",
        "/",
        &Options {
            origin: Some("x\r\ny".into()),
            ..Options::default()
        }
    )
    .is_err());
    assert!(matches!(
        handshake::validate_request("x", &format!("/{}", "x".repeat(16384)), &Options::default()),
        Err(Error::Limit)
    ));
}
#[test]
fn rfc_masked_hello_vector_and_empty_binary() {
    let (mut c, out) = client(vec![], Limits::default());
    c.send_text("Hello").unwrap();
    assert_eq!(
        *out.borrow(),
        vec![0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]
    );
    out.borrow_mut().clear();
    c.send_binary(&[]).unwrap();
    assert_eq!(client_frame(&mut Cursor::new(out.borrow().clone())).2, b"");
}
#[test]
fn outbound_fragmentation_uses_masked_continuations_and_size_bound() {
    let (mut c, out) = client(
        vec![],
        Limits {
            frame_bytes: 3,
            message_bytes: 8,
            ..Limits::default()
        },
    );
    c.send_text("€hi").unwrap();
    let mut cursor = Cursor::new(out.borrow().clone());
    let a = client_frame(&mut cursor);
    let b = client_frame(&mut cursor);
    assert_eq!((a.0, a.1, a.2), (1, false, "€".as_bytes().to_vec()));
    assert_eq!((b.0, b.1, b.2), (0, true, b"hi".to_vec()));
    let before = out.borrow().len();
    assert!(matches!(c.send_binary(&[0; 9]), Err(Error::Limit)));
    assert_eq!(out.borrow().len(), before);
    assert_eq!(c.state(), State::Open);
}
#[test]
fn fragmentation_and_interleaved_ping_pong_preserve_message() {
    let bytes = [
        server(1, false, b"Hel"),
        server(9, true, b"ping"),
        server(10, true, b"unsolicited"),
        server(0, true, b"lo"),
    ]
    .concat();
    let (mut c, out) = client(bytes, Limits::default());
    for _ in 0..3 {
        assert_eq!(c.poll().unwrap(), None);
    }
    assert_eq!(c.pop_control(), Some(Control::Ping(b"ping".to_vec())));
    assert_eq!(
        c.pop_control(),
        Some(Control::Pong(b"unsolicited".to_vec()))
    );
    assert_eq!(c.poll().unwrap(), Some(Event::Text("Hello".into())));
    let reply = client_frame(&mut Cursor::new(out.borrow().clone()));
    assert_eq!((reply.0, reply.2), (10, b"ping".to_vec()));
}
#[test]
fn utf8_scalar_can_span_every_fragment_boundary() {
    let text = "a€😀z";
    for split in 0..=text.len() {
        let (mut c, _) = client(
            [
                server(1, false, &text.as_bytes()[..split]),
                server(0, true, &text.as_bytes()[split..]),
            ]
            .concat(),
            Limits::default(),
        );
        assert_eq!(c.poll().unwrap(), None);
        assert_eq!(c.poll().unwrap(), Some(Event::Text(text.into())));
    }
}
#[test]
fn invalid_utf8_fails_connection_and_sends_1007() {
    for bad in [
        vec![0xc0, 0xaf],
        vec![0xed, 0xa0, 0x80],
        vec![0xf4, 0x90, 0x80, 0x80],
        vec![0x80],
        vec![0xf5, 0x80, 0x80, 0x80],
        vec![0xe2, 0x82],
    ] {
        let (mut c, out) = client(server(1, true, &bad), Limits::default());
        assert!(matches!(c.poll(), Err(Error::Utf8)));
        assert_eq!(c.state(), State::Failed);
        let close = client_frame(&mut Cursor::new(out.borrow().clone()));
        assert_eq!((close.0, close.2), (8, 1007u16.to_be_bytes().to_vec()));
        assert!(matches!(c.poll(), Err(Error::Closed)));
        assert!(matches!(c.send_text("later"), Err(Error::Closed)));
    }
    let (mut c, _) = client(server(1, false, &[0xff]), Limits::default());
    assert!(matches!(c.poll(), Err(Error::Utf8)));
}
#[test]
fn binary_has_no_utf8_requirement_and_fragment_limit_is_cumulative() {
    let (mut c, _) = client(server(2, true, &[0xff, 0xc0]), Limits::default());
    assert_eq!(c.poll().unwrap(), Some(Event::Binary(vec![0xff, 0xc0])));
    let (mut c, out) = client(
        [server(2, false, b"abc"), server(0, true, b"def")].concat(),
        Limits {
            message_bytes: 5,
            ..Limits::default()
        },
    );
    c.poll().unwrap();
    assert!(matches!(c.poll(), Err(Error::Limit)));
    assert_eq!(
        client_frame(&mut Cursor::new(out.borrow().clone())).2,
        1009u16.to_be_bytes()
    );
}
#[test]
fn invalid_fragment_sequences_and_control_shapes_fail() {
    let cases = vec![
        server(0, true, b"orphan"),
        [server(1, false, b"a"), server(2, true, b"b")].concat(),
        vec![0x09, 0],
        vec![0x89, 126],
        vec![0xc1, 0],
        vec![0x83, 0],
        vec![0x8b, 0],
        vec![0x81, 0x80],
    ];
    for bytes in cases {
        let (mut c, _) = client(bytes, Limits::default());
        let result = match c.poll() {
            Ok(None) => c.poll(),
            other => other,
        };
        assert!(matches!(result, Err(Error::Protocol)));
        assert_eq!(c.state(), State::Failed);
    }
}
#[test]
fn nonminimal_lengths_high_bit_and_oversize_fail_before_payload_read() {
    for (bytes, limit) in [
        (vec![0x82, 126, 0, 125], 1000),
        (vec![0x82, 127, 0, 0, 0, 0, 0, 0, 255, 255], 100000),
        (vec![0x82, 127, 128, 0, 0, 0, 0, 0, 0, 0], usize::MAX),
        (vec![0x82, 126, 4, 0], 10),
    ] {
        let len = bytes.len();
        let mut io = Cursor::new(bytes);
        assert!(frame::Header::read(&mut io, limit).is_err());
        assert_eq!(io.position(), len as u64);
    }
}
#[test]
fn lengths_125_126_65535_and_65536_roundtrip() {
    for len in [125, 126, 65535, 65536] {
        let bytes = vec![42; len];
        let (mut c, out) = client(server(2, true, &bytes), Limits::default());
        assert_eq!(c.poll().unwrap(), Some(Event::Binary(bytes.clone())));
        c.send_binary(&bytes).unwrap();
        assert_eq!(
            client_frame(&mut Cursor::new(out.borrow().clone())).2,
            bytes
        );
    }
}
#[test]
fn queued_controls_and_empty_fragment_cpu_budgets_fail_closed() {
    let (mut c, _) = client(
        [server(9, true, b"1"), server(10, true, b"2")].concat(),
        Limits {
            queued_controls: 1,
            ..Limits::default()
        },
    );
    c.poll().unwrap();
    assert!(matches!(c.poll(), Err(Error::Limit)));
    let (mut c, _) = client(
        [server(2, false, b""), server(0, false, b"")].concat(),
        Limits {
            frames: 1,
            ..Limits::default()
        },
    );
    c.poll().unwrap();
    assert!(matches!(c.poll(), Err(Error::Limit)));
}
#[test]
fn peer_initiated_close_echo_and_no_data_after_close() {
    let bytes = [1000u16.to_be_bytes().as_slice(), b"bye"].concat();
    let (mut c, out) = client(server(8, true, &bytes), Limits::default());
    assert_eq!(
        c.poll().unwrap(),
        Some(Event::Close(Some(Close {
            code: 1000,
            reason: "bye".into()
        })))
    );
    assert_eq!(c.state(), State::Closed);
    assert_eq!(
        client_frame(&mut Cursor::new(out.borrow().clone())).2,
        bytes
    );
    assert!(c.send_text("no").is_err());
    c.finish_close().unwrap();
}
#[test]
fn local_and_simultaneous_close_send_exactly_one_close() {
    let (mut c, out) = client(
        [
            server(1, true, b"in flight"),
            server(9, true, b"p"),
            server(8, true, &1001u16.to_be_bytes()),
        ]
        .concat(),
        Limits::default(),
    );
    c.close(Some(1000), "done").unwrap();
    assert_eq!(c.state(), State::Closing);
    assert!(c.send_binary(b"no").is_err());
    assert!(c.close(None, "").is_err());
    assert_eq!(c.poll().unwrap(), None);
    assert_eq!(c.poll().unwrap(), None);
    assert!(matches!(
        c.poll().unwrap(),
        Some(Event::Close(Some(Close { code: 1001, .. })))
    ));
    let mut cursor = Cursor::new(out.borrow().clone());
    assert_eq!(client_frame(&mut cursor).0, 8);
    assert_eq!(client_frame(&mut cursor).0, 10);
    assert_eq!(cursor.position(), out.borrow().len() as u64);
    c.finish_close().unwrap();
}
#[test]
fn empty_close_and_reserved_status_codes() {
    let (mut c, out) = client(server(8, true, b""), Limits::default());
    assert_eq!(c.poll().unwrap(), Some(Event::Close(None)));
    assert!(client_frame(&mut Cursor::new(out.borrow().clone()))
        .2
        .is_empty());
    for code in [0, 999, 1004, 1005, 1006, 1010, 1015, 1016, 2999, 5000] {
        let (mut c, _) = client(server(8, true, &code.to_be_bytes()), Limits::default());
        assert!(matches!(c.poll(), Err(Error::Protocol)), "{code}");
    }
    for code in [
        1000, 1001, 1002, 1003, 1007, 1008, 1009, 1011, 1012, 1013, 1014, 3000, 4999,
    ] {
        let (mut c, _) = client(server(8, true, &code.to_be_bytes()), Limits::default());
        assert!(matches!(c.poll().unwrap(), Some(Event::Close(_))), "{code}");
    }
}
#[test]
fn close_reason_must_be_utf8_and_status_must_be_complete() {
    for bytes in [vec![0], vec![3, 232, 0xff]] {
        let (mut c, _) = client(server(8, true, &bytes), Limits::default());
        assert!(c.poll().is_err());
        assert_eq!(c.state(), State::Failed);
    }
    let (mut c, _) = client(vec![], Limits::default());
    for code in [1004, 1005, 1006, 1015, 5000] {
        assert!(c.close(Some(code), "").is_err());
    }
    assert!(c.close(None, "reason").is_err());
    assert!(c.close(Some(1000), &"x".repeat(124)).is_err());
    assert_eq!(c.state(), State::Open);
}
#[test]
fn truncated_frame_and_abrupt_eof_are_not_a_clean_close() {
    for bytes in [vec![], vec![0x81], vec![0x82, 126, 1], vec![0x81, 3, b'a']] {
        let (mut c, _) = client(bytes, Limits::default());
        assert!(c.poll().is_err());
        assert_eq!(c.state(), State::Failed);
        assert!(c.finish_close().is_err());
    }
}
#[test]
fn entropy_failure_prevents_writes_and_poisons_existing_connection() {
    fn fail(_: &mut [u8]) -> Result<(), Error> {
        Err(Error::Entropy)
    }
    let out = Rc::new(RefCell::new(Vec::new()));
    let io = Io {
        input: Cursor::new(vec![]),
        out: out.clone(),
    };
    let mut c = Client::test_client(io, Limits::default(), fail);
    assert!(matches!(c.send_text("x"), Err(Error::Entropy)));
    assert!(out.borrow().is_empty());
    assert_eq!(c.state(), State::Failed);
    let mut io = Io {
        input: Cursor::new(vec![]),
        out: out.clone(),
    };
    assert!(matches!(
        handshake::perform(&mut io, "x", "/", &Options::default(), fail),
        Err(Error::Entropy)
    ));
    assert!(out.borrow().is_empty());
}
#[test]
fn production_entropy_is_requested_for_each_frame() {
    // A secure source can theoretically repeat; assert invocation separately
    // with deterministic unique masks, not a probabilistic uniqueness claim.
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    fn unique(b: &mut [u8]) -> Result<(), Error> {
        b.copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        Ok(())
    }
    let out = Rc::new(RefCell::new(Vec::new()));
    let io = Io {
        input: Cursor::new(vec![]),
        out: out.clone(),
    };
    let mut c = Client::test_client(
        io,
        Limits {
            frame_bytes: 2,
            ..Limits::default()
        },
        unique,
    );
    c.send_binary(b"12345").unwrap();
    let mut cursor = Cursor::new(out.borrow().clone());
    let masks = (0..3)
        .map(|_| client_frame(&mut cursor).3)
        .collect::<Vec<_>>();
    assert_ne!(masks[0], masks[1]);
    assert_ne!(masks[1], masks[2]);
}
fn echo(io: &mut (impl Read + Write)) {
    let request = handshake::read_headers(io).unwrap();
    let request = String::from_utf8(request).unwrap();
    assert!(request.starts_with("GET /echo?x=1 HTTP/1.1\r\n"));
    assert!(request.contains("Sec-WebSocket-Version: 13\r\n"));
    assert!(request.contains("Origin: http://example.test\r\n"));
    let key = request
        .split("\r\n")
        .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
        .unwrap();
    assert_eq!(key.len(), 24);
    io.write_all(&response(
        &handshake::accept(key),
        "Sec-WebSocket-Protocol: echo\r\n",
    ))
    .unwrap();
    io.flush().unwrap();
    for expected in [
        (1, b"hello".as_slice()),
        (2, b"\0\xff".as_slice()),
        (9, b"ping".as_slice()),
    ] {
        let (op, fin, bytes, _) = client_frame(io);
        assert!(fin);
        assert_eq!((op, bytes.as_slice()), expected);
        io.write_all(&server(if op == 9 { 10 } else { op }, true, &bytes))
            .unwrap();
        io.flush().unwrap();
    }
    let (op, _, bytes, _) = client_frame(io);
    assert_eq!(op, 8);
    io.write_all(&server(8, true, &bytes)).unwrap();
    io.flush().unwrap();
}
fn conversation<S: Read + Write>(mut c: Client<S>) {
    assert_eq!(c.protocol(), Some("echo"));
    c.send_text("hello").unwrap();
    assert_eq!(c.poll().unwrap(), Some(Event::Text("hello".into())));
    c.send_binary(b"\0\xff").unwrap();
    assert_eq!(c.poll().unwrap(), Some(Event::Binary(b"\0\xff".to_vec())));
    c.ping(b"ping").unwrap();
    assert_eq!(c.poll().unwrap(), None);
    assert_eq!(c.pop_control(), Some(Control::Pong(b"ping".to_vec())));
    c.close(Some(1000), "done").unwrap();
    assert!(matches!(
        c.poll().unwrap(),
        Some(Event::Close(Some(Close { code: 1000, .. })))
    ));
    c.finish_close().unwrap();
}
#[test]
fn loopback_ws_echo_uses_live_connect_handshake_masks_and_close() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let (mut io, _) = listener.accept().unwrap();
        io.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        echo(&mut io);
    });
    let options = Options {
        origin: Some("http://example.test".into()),
        protocols: vec!["echo".into()],
        ..Options::default()
    };
    conversation(connect(&format!("ws://127.0.0.1:{port}/echo?x=1"), &options).unwrap());
    thread.join().unwrap();
}
#[test]
fn loopback_wss_echo_negotiates_http1_even_when_peer_prefers_h2() {
    use amazing_net_tls::test_support;
    let (port, thread) = test_support::serve(vec![b"h2".to_vec(), b"http/1.1".to_vec()], |io| {
        assert_eq!(io.protocol(), Some(b"http/1.1".as_slice()));
        echo(io);
    });
    let io = test_support::connect_http1(port).unwrap();
    assert_eq!(io.protocol(), amazing_net_tls::Protocol::Http11);
    let options = Options {
        origin: Some("http://example.test".into()),
        protocols: vec!["echo".into()],
        ..Options::default()
    };
    conversation(
        Client::handshake(io, &format!("localhost:{port}"), "/echo?x=1", &options).unwrap(),
    );
    thread.join().unwrap().unwrap();
}
#[test]
fn invalid_urls_and_limits_fail_before_connecting() {
    for url in [
        "https://example.com/",
        "ws://user:pass@example.com/",
        "ws://example.com/#x",
        "file:///x",
    ] {
        assert!(matches!(
            connect(url, &Options::default()),
            Err(Error::InvalidInput)
        ));
    }
    for limits in [
        Limits {
            frame_bytes: 0,
            ..Limits::default()
        },
        Limits {
            frame_bytes: usize::MAX,
            ..Limits::default()
        },
        Limits {
            message_bytes: usize::MAX,
            ..Limits::default()
        },
        Limits {
            queued_controls: 33,
            ..Limits::default()
        },
        Limits {
            frames: 0,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            connect(
                "ws://127.0.0.1:1/",
                &Options {
                    limits,
                    ..Options::default()
                }
            ),
            Err(Error::Limit)
        ));
    }
}
