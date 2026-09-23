use super::*;
use std::{
    cell::RefCell,
    io::{Cursor, Read, Write},
    rc::Rc,
};
struct Io {
    input: Cursor<Vec<u8>>,
    output: Rc<RefCell<Vec<u8>>>,
}
impl Read for Io {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(b)
    }
}
impl Write for Io {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.output.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn f(kind: u8, flags: u8, id: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame::write(&mut bytes, kind, flags, id, data).unwrap();
    bytes
}
fn client(frames: Vec<Vec<u8>>, limits: Limits) -> (Client<Io>, Rc<RefCell<Vec<u8>>>) {
    custom_client(Vec::new(), frames, limits)
}
fn custom_client(
    settings: Vec<u8>,
    frames: Vec<Vec<u8>>,
    limits: Limits,
) -> (Client<Io>, Rc<RefCell<Vec<u8>>>) {
    let mut input = f(4, 0, 0, &settings);
    input.extend(frames.into_iter().flatten());
    let output = Rc::new(RefCell::new(Vec::new()));
    (
        Client::new(
            Io {
                input: Cursor::new(input),
                output: output.clone(),
            },
            limits,
        )
        .unwrap(),
        output,
    )
}
fn request() -> Vec<Field> {
    [
        (":method", "GET"),
        (":scheme", "https"),
        (":authority", "example.test"),
        (":path", "/"),
    ]
    .iter()
    .map(|(n, v)| (n.as_bytes().to_vec(), v.as_bytes().to_vec()))
    .collect()
}
fn setting(id: u16, value: u32) -> Vec<u8> {
    [id.to_be_bytes().as_slice(), &value.to_be_bytes()].concat()
}
fn head(id: u32, flags: u8, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut fields = vec![(b":status".to_vec(), b"200".to_vec())];
    fields.extend(
        extra
            .iter()
            .map(|(n, v)| (n.as_bytes().to_vec(), v.as_bytes().to_vec())),
    );
    f(
        1,
        flags,
        id,
        &hpack::Encoder::new().encode(&fields, 65536).unwrap(),
    )
}
fn sent(trace: &Rc<RefCell<Vec<u8>>>) -> Vec<frame::Frame> {
    let bytes = trace.borrow();
    let mut cursor = Cursor::new(&bytes[PREFACE.len()..]);
    let mut frames = Vec::new();
    while cursor.position() < (bytes.len() - PREFACE.len()) as u64 {
        frames.push(frame::Frame::read(&mut cursor).unwrap());
    }
    frames
}
#[test]
fn multiplex_interleaved_streams_finish_out_of_order() {
    let (mut c, _) = client(
        vec![
            head(1, 4, &[]),
            head(3, 4, &[]),
            f(0, 1, 3, b"three"),
            f(0, 0, 1, b"one"),
            f(0, 1, 1, b"!"),
        ],
        Limits::default(),
    );
    assert_eq!(c.start(&request(), b"").unwrap(), 1);
    assert_eq!(c.start(&request(), b"").unwrap(), 3);
    for _ in 0..3 {
        c.poll().unwrap();
    }
    assert!(c.take_reply(1).unwrap().is_none());
    assert_eq!(c.take_reply(3).unwrap().unwrap().body, b"three");
    for _ in 0..2 {
        c.poll().unwrap();
    }
    assert_eq!(c.take_reply(1).unwrap().unwrap().body, b"one!");
    assert_eq!(c.start(&request(), b"").unwrap(), 5);
}
#[test]
fn retained_frame_io_interleaves_two_streams_and_resets_one() {
    let mut c = Client::new_pending(FrameIo::default(), Limits::default()).unwrap();
    let preface = c.io_mut().unwrap().take_output();
    assert!(preface.starts_with(PREFACE));
    c.io_mut().unwrap().feed(&f(4, 0, 0, &[]));
    c.poll().unwrap();
    let one = c.start_streaming(&request(), b"").unwrap();
    let two = c.start_streaming(&request(), b"").unwrap();
    assert_eq!((one, two), (1, 3));
    for frame in [
        head(1, 4, &[]),
        head(3, 4, &[]),
        f(0, 0, 1, b"one"),
        f(0, 0, 3, b"two"),
    ] {
        c.io_mut().unwrap().feed(&frame);
        c.poll().unwrap();
    }
    assert!(matches!(
        c.next_event_ready(one).unwrap(),
        Some(StreamEvent::Head { status: 200, .. })
    ));
    assert!(matches!(
        c.next_event_ready(two).unwrap(),
        Some(StreamEvent::Head { status: 200, .. })
    ));
    assert!(
        matches!(c.next_event_ready(one).unwrap(), Some(StreamEvent::Data(bytes)) if bytes == b"one")
    );
    assert!(
        matches!(c.next_event_ready(two).unwrap(), Some(StreamEvent::Data(bytes)) if bytes == b"two")
    );
    c.cancel(one).unwrap();
    let output = c.io_mut().unwrap().take_output();
    let mut cursor = Cursor::new(output);
    let mut reset = false;
    while let Ok(frame) = frame::Frame::read(&mut cursor) {
        reset |= frame.kind == 3 && frame.stream == one && frame.payload == 8u32.to_be_bytes();
    }
    assert!(reset);
    assert!(matches!(
        c.next_event_ready(one),
        Err(Error::StreamReset { stream: 1, code: 8 })
    ));
    c.io_mut().unwrap().feed(&f(0, 1, two, b"!"));
    c.poll().unwrap();
    assert!(
        matches!(c.next_event_ready(two).unwrap(), Some(StreamEvent::Data(bytes)) if bytes == b"!")
    );
    assert!(matches!(
        c.next_event_ready(two).unwrap(),
        Some(StreamEvent::End { .. })
    ));
}
#[test]
fn streaming_events_release_data_before_end_and_keep_cumulative_length() {
    let (mut c, _) = client(
        vec![
            head(1, 4, &[("content-length", "5")]),
            f(0, 0, 1, b"hel"),
            f(0, 1, 1, b"lo"),
        ],
        Limits {
            buffered_body_bytes: 3,
            ..Limits::default()
        },
    );
    let id = c.start_streaming(&request(), b"").unwrap();
    assert!(matches!(
        c.next_event(id).unwrap(),
        StreamEvent::Head { status: 200, .. }
    ));
    assert!(matches!(c.next_event(id).unwrap(), StreamEvent::Data(bytes) if bytes == b"hel"));
    assert!(matches!(c.next_event(id).unwrap(), StreamEvent::Data(bytes) if bytes == b"lo"));
    assert!(matches!(c.next_event(id).unwrap(), StreamEvent::End { .. }));
}
#[test]
fn reset_is_stream_local_and_idle_reset_is_connection_error() {
    let (mut c, _) = client(
        vec![
            f(3, 0, 1, &8u32.to_be_bytes()),
            head(3, 5, &[]),
            f(3, 0, 7, &0u32.to_be_bytes()),
        ],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(
        c.take_reply(1),
        Err(Error::StreamReset { stream: 1, code: 8 })
    ));
    c.poll().unwrap();
    assert_eq!(c.take_reply(3).unwrap().unwrap().status, 200);
    assert!(matches!(c.poll(), Err(Error::Protocol)));
    assert!(matches!(c.start(&request(), b""), Err(Error::Closed)));
}
#[test]
fn goaway_drains_accepted_streams_and_never_replays_unprocessed_requests() {
    let goaway = [1u32.to_be_bytes(), 0u32.to_be_bytes()].concat();
    let (mut c, _) = client(
        vec![f(7, 0, 0, &goaway), head(1, 5, &[])],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(c.take_reply(3), Err(Error::Unprocessed(3))));
    assert!(matches!(
        c.start(&request(), b""),
        Err(Error::GoAway {
            last_stream: 1,
            code: 0
        })
    ));
    c.poll().unwrap();
    assert_eq!(c.take_reply(1).unwrap().unwrap().status, 200);
}
#[test]
fn settings_concurrency_table_updates_acknowledgement_and_invalid_values() {
    let (mut c, trace) = custom_client(
        setting(3, 0),
        vec![
            f(4, 0, 0, &[setting(3, 1), setting(1, 0)].concat()),
            f(4, 1, 0, &[]),
        ],
        Limits::default(),
    );
    assert!(matches!(c.start(&request(), b""), Err(Error::Limit)));
    c.poll().unwrap();
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(c.start(&request(), b""), Err(Error::Limit)));
    let frames = sent(&trace);
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.kind == 4 && f.flags == 1)
            .count(),
        2
    );
    assert_eq!(
        frames.iter().find(|f| f.kind == 1).unwrap().payload[0],
        0x20
    );
    for settings in [
        setting(2, 0),
        setting(4, 0x80000000),
        setting(5, 16383),
        setting(5, 16777216),
    ] {
        let (mut c, _) = client(vec![f(4, 0, 0, &settings)], Limits::default());
        assert!(c.poll().is_err());
    }
}
#[test]
fn stream_and_connection_flow_control_gate_upload_independently() {
    let body = vec![7; 70000];
    let (mut c, trace) = client(
        vec![
            f(8, 0, 1, &4465u32.to_be_bytes()),
            f(8, 0, 0, &4465u32.to_be_bytes()),
        ],
        Limits::default(),
    );
    c.start(&request(), &body).unwrap();
    let initial: usize = sent(&trace)
        .iter()
        .filter(|f| f.kind == 0)
        .map(|f| f.payload.len())
        .sum();
    assert_eq!(initial, 65535);
    c.poll().unwrap();
    assert_eq!(
        sent(&trace)
            .iter()
            .filter(|f| f.kind == 0)
            .map(|f| f.payload.len())
            .sum::<usize>(),
        65535
    );
    c.poll().unwrap();
    let frames = sent(&trace);
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.kind == 0)
            .map(|f| f.payload.len())
            .sum::<usize>(),
        70000
    );
    assert_eq!(
        frames.iter().filter(|f| f.kind == 0).last().unwrap().flags,
        1
    );
}
#[test]
fn settings_zero_window_and_negative_shrink_resume_only_after_credit() {
    let (mut c, trace) = custom_client(
        setting(4, 0),
        vec![f(8, 0, 1, &2u32.to_be_bytes()), f(4, 0, 0, &setting(4, 3))],
        Limits::default(),
    );
    c.start(&request(), b"hello").unwrap();
    assert!(!sent(&trace).iter().any(|f| f.kind == 0));
    c.poll().unwrap();
    c.poll().unwrap();
    assert_eq!(
        sent(&trace)
            .iter()
            .filter(|f| f.kind == 0)
            .flat_map(|f| f.payload.clone())
            .collect::<Vec<_>>(),
        b"hello"
    );
    let (mut c, trace) = client(
        vec![
            f(4, 0, 0, &setting(4, 10)),
            f(8, 0, 0, &4465u32.to_be_bytes()),
            f(8, 0, 1, &69990u32.to_be_bytes()),
        ],
        Limits::default(),
    );
    c.start(&request(), &vec![0; 70000]).unwrap();
    c.poll().unwrap();
    c.poll().unwrap();
    assert_eq!(
        sent(&trace)
            .iter()
            .filter(|f| f.kind == 0)
            .map(|f| f.payload.len())
            .sum::<usize>(),
        65535
    );
    c.poll().unwrap();
    assert_eq!(
        sent(&trace)
            .iter()
            .filter(|f| f.kind == 0)
            .map(|f| f.payload.len())
            .sum::<usize>(),
        70000
    );
}
#[test]
fn window_update_zero_and_overflow_have_correct_error_scope() {
    let (mut c, _) = client(
        vec![f(8, 0, 1, &0u32.to_be_bytes()), head(3, 5, &[])],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(
        c.take_reply(1),
        Err(Error::StreamReset { code: 1, .. })
    ));
    c.poll().unwrap();
    assert!(c.take_reply(3).unwrap().is_some());
    for value in [0, 0x7fffffff] {
        let (mut c, _) = client(
            vec![f(8, 0, 0, &u32::to_be_bytes(value))],
            Limits::default(),
        );
        assert!(c.poll().is_err());
    }
    let (mut c, _) = client(
        vec![f(8, 0, 1, &0x7fffffffu32.to_be_bytes())],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(
        c.take_reply(1),
        Err(Error::StreamReset { code: 3, .. })
    ));
}
#[test]
fn padded_data_counts_full_flow_credit_but_only_data_in_content_length() {
    let (mut c, trace) = client(
        vec![
            head(1, 4, &[("content-length", "3")]),
            f(0, 9, 1, &[2, b'a', b'b', b'c', 0, 0]),
        ],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    c.poll().unwrap();
    assert_eq!(c.take_reply(1).unwrap().unwrap().body, b"abc");
    assert!(sent(&trace)
        .iter()
        .any(|f| f.kind == 8 && f.stream == 0 && frame::number(&f.payload) == 6));
}
#[test]
fn continuations_are_connection_ordered_and_closed_streams_preserve_hpack() {
    let block = hpack::Encoder::new()
        .encode(
            &[
                (b":status".to_vec(), b"200".to_vec()),
                (b"x-a".to_vec(), b"value".to_vec()),
            ],
            4096,
        )
        .unwrap();
    let (mut c, _) = client(
        vec![
            f(1, 1, 1, &block[..2]),
            f(9, 4, 1, &block[2..]),
            f(1, 5, 3, &[0x88, 0xbe]),
        ],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.cancel(1).unwrap();
    assert!(c.take_reply(1).is_err());
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    c.poll().unwrap();
    c.poll().unwrap();
    assert_eq!(
        c.take_reply(3).unwrap().unwrap().headers,
        vec![(b"x-a".to_vec(), b"value".to_vec())]
    );
    let (mut c, _) = client(
        vec![f(1, 0, 1, &[0x88]), f(6, 0, 0, &[0; 8])],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(c.poll(), Err(Error::Protocol)));
}
#[test]
fn frame_lengths_reject_before_payload_read_and_fixed_shapes_are_checked() {
    let mut bytes = vec![0, 0x40, 1, 0, 0, 0, 0, 0, 1];
    bytes.extend_from_slice(&[0; 10]);
    let mut io = Cursor::new(bytes);
    assert!(matches!(frame::Frame::read(&mut io), Err(Error::FrameSize)));
    assert_eq!(io.position(), 9);
    for (kind, flags, id, len) in [
        (4, 1, 0, 6),
        (4, 0, 1, 0),
        (6, 0, 0, 7),
        (3, 0, 1, 3),
        (8, 0, 0, 5),
        (7, 0, 0, 7),
        (0, 0, 0, 0),
    ] {
        let mut data = vec![0, 0, len, kind, flags, 0, 0, 0, id];
        data.extend(vec![0; len as usize]);
        assert!(frame::Frame::read(&mut Cursor::new(data)).is_err());
    }
}
#[test]
fn header_list_block_body_stream_and_frame_budgets_fail_closed() {
    let (mut c, _) = client(
        vec![f(1, 5, 1, &[0x88, 0x88])],
        Limits {
            header_list_bytes: 64,
            ..Limits::default()
        },
    );
    c.start(&request(), b"").unwrap_err(); // outgoing list is independently bounded
    let (mut c, _) = client(vec![f(1, 5, 1, &vec![0x88; 1600])], Limits::default());
    c.start(&request(), b"").unwrap();
    assert!(matches!(c.poll(), Err(Error::Limit)));
    let mut fragments = vec![f(1, 0, 1, &vec![0; MAX_FRAME])];
    for _ in 0..4 {
        fragments.push(f(9, 0, 1, &vec![0; MAX_FRAME]));
    }
    let (mut c, _) = client(fragments, Limits::default());
    c.start(&request(), b"").unwrap();
    for _ in 0..4 {
        c.poll().unwrap();
    }
    assert!(matches!(c.poll(), Err(Error::Limit)));
    let (mut c, _) = client(
        vec![head(1, 4, &[]), f(0, 1, 1, b"ab")],
        Limits {
            body_bytes: 1,
            ..Limits::default()
        },
    );
    c.start(&request(), b"").unwrap();
    c.poll().unwrap();
    assert!(matches!(c.poll(), Err(Error::Limit)));
    let (mut c, _) = client(
        vec![],
        Limits {
            concurrent_streams: 1,
            ..Limits::default()
        },
    );
    c.start(&request(), b"").unwrap();
    assert!(matches!(c.start(&request(), b""), Err(Error::Limit)));
    let (mut c, _) = client(
        vec![f(0xf0, 0, 0, &[])],
        Limits {
            frames: 1,
            ..Limits::default()
        },
    );
    assert!(matches!(c.poll(), Err(Error::Limit)));
}
#[test]
fn response_semantics_informational_trailers_and_content_length() {
    let info = hpack::Encoder::new()
        .encode(&[(b":status".to_vec(), b"103".to_vec())], 1024)
        .unwrap();
    let trailer = hpack::Encoder::new()
        .encode(&[(b"x-trailer".to_vec(), b"yes".to_vec())], 1024)
        .unwrap();
    let (mut c, _) = client(
        vec![
            f(1, 4, 1, &info),
            head(1, 4, &[("content-length", "3")]),
            f(0, 0, 1, b"abc"),
            f(1, 5, 1, &trailer),
        ],
        Limits::default(),
    );
    c.start(&request(), b"").unwrap();
    for _ in 0..4 {
        c.poll().unwrap();
    }
    let reply = c.take_reply(1).unwrap().unwrap();
    assert_eq!(reply.body, b"abc");
    assert_eq!(reply.trailers[0].1, b"yes");
    for bad in [
        head(1, 5, &[("content-length", "1")]),
        head(1, 5, &[("Connection", "close")]),
        head(1, 5, &[("connection", "close")]),
        f(1, 5, 1, &[0x88, 0x88]),
    ] {
        let (mut c, _) = client(vec![bad], Limits::default());
        c.start(&request(), b"").unwrap();
        c.poll().unwrap();
        assert!(matches!(
            c.take_reply(1),
            Err(Error::StreamReset { code: 1, .. })
        ));
    }
}
#[test]
fn server_preface_and_push_are_rejected_and_ping_is_acknowledged() {
    let io = Io {
        input: Cursor::new(f(6, 0, 0, &[0; 8])),
        output: Rc::new(RefCell::new(Vec::new())),
    };
    assert!(matches!(
        Client::new(io, Limits::default()),
        Err(Error::Protocol)
    ));
    let (mut c, trace) = client(
        vec![f(6, 0, 0, b"abcdefgh"), f(5, 4, 1, &[0; 4])],
        Limits::default(),
    );
    c.poll().unwrap();
    assert!(sent(&trace)
        .iter()
        .any(|f| f.kind == 6 && f.flags == 1 && f.payload == b"abcdefgh"));
    assert!(matches!(c.poll(), Err(Error::Protocol)));
}
#[test]
fn authenticated_h2_fetch_and_http1_fallback_use_production_dispatch() {
    use amazing_net_tls::test_support;
    let (port, thread) = test_support::serve(vec![b"h2".to_vec()], |io| {
        assert_eq!(io.protocol(), Some(b"h2".as_slice()));
        let mut preface = [0; 24];
        io.read_exact(&mut preface).unwrap();
        assert_eq!(&preface, PREFACE);
        assert_eq!(frame::Frame::read(io).unwrap().kind, 4);
        frame::write(io, 4, 0, 0, &[]).unwrap();
        io.flush().unwrap();
        assert_eq!(frame::Frame::read(io).unwrap().kind, 4);
        let request = frame::Frame::read(io).unwrap();
        assert_eq!(request.kind, 1);
        let fields = hpack::Decoder::new(4096)
            .decode(&request.payload, 65536)
            .unwrap();
        assert!(fields.contains(&(b":path".to_vec(), b"/h2?x=1".to_vec())));
        assert!(fields.contains(&(b"accept-encoding".to_vec(), b"gzip, deflate, br".to_vec())));
        let body = include_bytes!("../../../amazing-net-brotli/tests/vectors/hello-5.br");
        let length = body.len().to_string();
        io.write_all(&head(
            request.stream,
            4,
            &[("content-encoding", "br"), ("content-length", &length)],
        ))
        .unwrap();
        frame::write(io, 0, 1, request.stream, body).unwrap();
        io.flush().unwrap();
        // Keep the peer open through the client's final connection-credit
        // return. Dropping with unread DATA credit can cause a TCP reset on
        // Windows while the client flushes that frame, racing fixture teardown.
        let credit = frame::Frame::read(io).unwrap();
        assert_eq!((credit.kind, credit.stream), (8, 0));
        assert_eq!(frame::number(&credit.payload) as usize, body.len());
    });
    let url = crate::ParsedUrl::parse(&format!("https://localhost:{port}/h2?x=1")).unwrap();
    let stream = test_support::connect(port).unwrap();
    let response = crate::http::tls_response(
        stream,
        &url,
        &crate::FetchOptions::default(),
        b"THIS MUST NOT BE SENT",
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"hello brotli\n");
    thread.join().unwrap().unwrap();
    for protocols in [vec![b"http/1.1".to_vec()], vec![]] {
        let (port, thread) = test_support::serve(protocols, |io| {
            let mut request = [0; 18];
            io.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"GET / HTTP/1.1\r\n\r\n");
            io.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
        });
        let url = crate::ParsedUrl::parse(&format!("https://localhost:{port}/")).unwrap();
        let stream = test_support::connect(port).unwrap();
        let response = crate::http::tls_response(
            stream,
            &url,
            &crate::FetchOptions::default(),
            b"GET / HTTP/1.1\r\n\r\n",
        )
        .unwrap();
        assert_eq!(response.body, b"ok");
        thread.join().unwrap().unwrap();
    }
}

#[test]
fn authenticated_h2_pull_stream_yields_data_before_second_frame() {
    use amazing_net_tls::test_support;
    let (port, thread) = test_support::serve(vec![b"h2".to_vec()], |io| {
        let mut preface = [0; 24];
        io.read_exact(&mut preface).unwrap();
        assert_eq!(&preface, PREFACE);
        assert_eq!(frame::Frame::read(io).unwrap().kind, 4);
        frame::write(io, 4, 0, 0, &[]).unwrap();
        io.flush().unwrap();
        assert_eq!(frame::Frame::read(io).unwrap().kind, 4);
        let request = frame::Frame::read(io).unwrap();
        assert_eq!(request.kind, 1);
        io.write_all(&head(request.stream, 4, &[("content-length", "5")]))
            .unwrap();
        frame::write(io, 0, 0, request.stream, b"hel").unwrap();
        io.flush().unwrap();
        let credit = frame::Frame::read(io).unwrap();
        assert_eq!(
            (credit.kind, credit.stream, frame::number(&credit.payload)),
            (8, 0, 3)
        );
        let credit = frame::Frame::read(io).unwrap();
        assert_eq!(
            (credit.kind, credit.stream, frame::number(&credit.payload)),
            (8, request.stream, 3)
        );
        frame::write(io, 0, 1, request.stream, b"lo").unwrap();
        io.flush().unwrap();
        let credit = frame::Frame::read(io).unwrap();
        assert_eq!(
            (credit.kind, credit.stream, frame::number(&credit.payload)),
            (8, 0, 2)
        );
    });
    let url = crate::ParsedUrl::parse(&format!("https://localhost:{port}/")).unwrap();
    let io = test_support::connect(port).unwrap();
    let mut response = crate::stream::open_h2(
        io,
        &url,
        &crate::FetchOptions::default(),
        crate::StreamOptions {
            chunk_bytes: 3,
            ..crate::StreamOptions::default()
        },
    )
    .unwrap();
    assert_eq!(response.next_chunk().unwrap(), Some(b"hel".to_vec()));
    assert_eq!(response.next_chunk().unwrap(), Some(b"lo".to_vec()));
    assert_eq!(response.next_chunk().unwrap(), None);
    thread.join().unwrap().unwrap();
}

#[test]
fn completed_response_survives_late_closed_frames_and_error_goaway() {
    let mut goaway = 1u32.to_be_bytes().to_vec();
    goaway.extend_from_slice(&2u32.to_be_bytes());
    let (mut c, trace) = client(
        vec![
            head(1, 5, &[]),
            f(3, 0, 1, &8u32.to_be_bytes()),
            f(0, 0, 1, b"discard"),
            f(7, 0, 0, &goaway),
        ],
        Limits::default(),
    );
    c.start(&request(), &[]).unwrap();
    for _ in 0..4 {
        c.poll().unwrap();
    }
    assert_eq!(c.take_reply(1).unwrap().unwrap().status, 200);
    assert!(sent(&trace)
        .iter()
        .any(|f| f.kind == 8 && f.stream == 0 && frame::number(&f.payload) == 7));
    assert!(matches!(
        c.start(&request(), &[]),
        Err(Error::GoAway { code: 2, .. })
    ));
}

#[test]
fn early_response_survives_no_error_reset_or_fatal_eof() {
    for reset in [false, true] {
        let mut frames = vec![head(1, 5, &[])];
        if reset {
            frames.push(f(3, 0, 1, &0u32.to_be_bytes()));
        }
        let (mut c, _) = custom_client(setting(4, 0), frames, Limits::default());
        c.start(&request(), b"pending upload").unwrap();
        c.poll().unwrap();
        if reset {
            c.poll().unwrap();
        } else {
            assert!(c.poll().is_err());
        }
        assert_eq!(c.take_reply(1).unwrap().unwrap().status, 200);
    }
}

#[test]
fn response_larger_than_initial_window_returns_both_credits() {
    let mut frames = vec![head(1, 4, &[("content-length", "81920")])];
    for i in 0..5 {
        frames.push(f(0, u8::from(i == 4), 1, &vec![42; MAX_FRAME]));
    }
    let (mut c, trace) = client(frames, Limits::default());
    c.start(&request(), &[]).unwrap();
    for _ in 0..6 {
        c.poll().unwrap();
    }
    assert_eq!(c.take_reply(1).unwrap().unwrap().body, vec![42; 81920]);
    let frames = sent(&trace);
    for (id, total) in [(0, 81920), (1, 65536)] {
        assert_eq!(
            frames
                .iter()
                .filter(|f| f.kind == 8 && f.stream == id)
                .map(|f| frame::number(&f.payload))
                .sum::<u32>(),
            total
        );
    }
}

#[test]
fn fatal_errors_and_cancel_write_failure_drop_transport() {
    use std::cell::Cell;
    struct Tracked {
        input: Cursor<Vec<u8>>,
        dropped: Rc<Cell<bool>>,
        fail: Rc<Cell<bool>>,
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }
    impl Read for Tracked {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(b)
        }
    }
    impl Write for Tracked {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.fail.get() {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(b.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for cancellation in [false, true] {
        let dropped = Rc::new(Cell::new(false));
        let fail = Rc::new(Cell::new(false));
        let io = Tracked {
            input: Cursor::new(f(4, 0, 0, &[])),
            dropped: dropped.clone(),
            fail: fail.clone(),
        };
        let mut c = Client::new(io, Limits::default()).unwrap();
        c.start(&request(), &[]).unwrap();
        if cancellation {
            fail.set(true);
            assert!(c.cancel(1).is_err());
        } else {
            assert!(c.poll().is_err());
        }
        assert!(dropped.get());
        assert!(matches!(c.poll(), Err(Error::Closed)));
        assert!(matches!(c.start(&request(), &[]), Err(Error::Closed)));
    }
}

#[test]
fn goaway_last_stream_cannot_increase_and_truncated_frames_fail() {
    let (mut c, _) = client(
        vec![
            f(7, 0, 0, &[0, 0, 0, 1, 0, 0, 0, 0]),
            f(7, 0, 0, &[0, 0, 0, 3, 0, 0, 0, 0]),
        ],
        Limits::default(),
    );
    c.poll().unwrap();
    assert!(matches!(c.poll(), Err(Error::Protocol)));
    let bytes = f(6, 0, 0, b"12345678");
    for n in 0..bytes.len() {
        assert!(frame::Frame::read(&mut Cursor::new(&bytes[..n])).is_err());
    }
}

#[test]
fn head_response_does_not_decode_absent_encoded_body() {
    let mut input = f(4, 0, 0, &[]);
    input.extend(head(
        1,
        5,
        &[("content-length", "123"), ("content-encoding", "br")],
    ));
    let io = Io {
        input: Cursor::new(input),
        output: Rc::new(RefCell::new(Vec::new())),
    };
    let mut options = crate::FetchOptions::default();
    options.method = "HEAD".into();
    let reply = fetch(
        io,
        &crate::ParsedUrl::parse("https://example.test/").unwrap(),
        &options,
    )
    .unwrap();
    assert_eq!(reply.status, 200);
    assert!(reply.body.is_empty());
}

#[test]
fn acknowledged_lower_table_limit_requires_hpack_size_update() {
    for update in [false, true] {
        let bytes = if update { vec![0x20, 0x88] } else { vec![0x88] };
        let (mut c, _) = client(
            vec![f(4, 1, 0, &[]), f(1, 5, 1, &bytes)],
            Limits {
                dynamic_table_bytes: 0,
                ..Limits::default()
            },
        );
        c.start(&request(), &[]).unwrap();
        c.poll().unwrap();
        if update {
            c.poll().unwrap();
            assert_eq!(c.take_reply(1).unwrap().unwrap().status, 200);
        } else {
            assert!(matches!(c.poll(), Err(Error::Compression)));
            assert!(matches!(c.poll(), Err(Error::Closed)));
        }
    }
    let mut decoder = hpack::Decoder::new(0);
    decoder.decode(&[0x20], 1024).unwrap();
    decoder.acknowledge_limit();
    assert!(decoder.decode(&[0x88], 1024).is_ok());
}

#[test]
fn te_trailers_is_allowed_only_in_requests() {
    let mut fields = request();
    fields.push((b"te".to_vec(), b"trailers".to_vec()));
    assert!(semantics::request(&fields).is_ok());
    fields.last_mut().unwrap().1 = b"gzip".to_vec();
    assert!(semantics::request(&fields).is_err());
    let (mut c, _) = client(vec![head(1, 5, &[("te", "trailers")])], Limits::default());
    c.start(&request(), &[]).unwrap();
    c.poll().unwrap();
    assert!(matches!(
        c.take_reply(1),
        Err(Error::StreamReset { stream: 1, code: 1 })
    ));
}
