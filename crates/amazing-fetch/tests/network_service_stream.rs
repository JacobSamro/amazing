use amazing_fetch::{
    api::{ApiError, ApiRequest, ApiRequestInit, ApiRequestInput},
    service::{ClientContext, NetworkService, ServiceEvent},
    HttpUrl,
};
use amazing_net::dispatch::DispatchLimits;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn request(url: &str, base: &HttpUrl) -> ApiRequest {
    let units: Vec<u16> = url.encode_utf16().collect();
    ApiRequest::new(
        ApiRequestInput::Url(&units),
        ApiRequestInit::default(),
        base,
        base.origin().clone(),
    )
    .unwrap()
}

#[test]
fn credit_starvation_and_buffered_parity() {
    let base = HttpUrl::parse("https://example.test/").unwrap();
    let body = "abc".repeat(24_000);
    let url = format!("data:text/plain,{body}");
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service
        .start_stream(request(&url, &base), ClientContext::default())
        .unwrap();
    let first = service.step(8192, Instant::now());
    assert!(first.iter().any(|event| matches!(event,
        ServiceEvent::ResponseHead { id: found, head } if *found == id && head.status == 200
            && head.final_url.starts_with("data:text/plain,"))));
    let mut chunks = Vec::new();
    for event in &first {
        if let ServiceEvent::BodyChunk { id: found, bytes } = event {
            assert_eq!(*found, id);
            chunks.extend_from_slice(bytes);
        }
    }
    assert_eq!(chunks.len(), 64 * 1024);
    assert!(!first
        .iter()
        .any(|event| matches!(event, ServiceEvent::StreamFinished { .. })));
    assert!(service.step(8192, Instant::now()).is_empty());
    assert!(service.grant_credit(id, body.len() - chunks.len()));
    let rest = service.step(8192, Instant::now());
    for event in &rest {
        if let ServiceEvent::BodyChunk { id: found, bytes } = event {
            assert_eq!(*found, id);
            chunks.extend_from_slice(bytes);
        }
    }
    assert_eq!(chunks, body.as_bytes());
    assert_eq!(
        rest.iter()
            .filter(|event| matches!(event,
        ServiceEvent::StreamFinished { id: found } if *found == id))
            .count(),
        1
    );
    let buffered = service
        .start(request(&url, &base), ClientContext::default())
        .unwrap();
    let response = service
        .step(8192, Instant::now())
        .into_iter()
        .find_map(|event| match event {
            ServiceEvent::Finished { id: found, result } if found == buffered => {
                Some(result.unwrap())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(response.body.clone_body().unwrap().bytes().unwrap(), chunks);
}

#[test]
fn head_before_slow_body_and_cancel_mid_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/slow",
        listener.local_addr().unwrap().port()
    );
    let (send_body, receive_body) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\n\r\n")
            .unwrap();
        receive_body.recv_timeout(Duration::from_secs(3)).unwrap();
        let _ = socket.write_all(b"first");
        thread::sleep(Duration::from_millis(100));
        let _ = socket.write_all(b"later");
    });
    let base = HttpUrl::parse(&url).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service
        .start_stream(request(&url, &base), ClientContext::default())
        .unwrap();
    let start = Instant::now();
    let mut saw_head = false;
    for tick in 0..2000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            if let ServiceEvent::ResponseHead { id: found, head } = &event {
                assert_eq!(*found, id);
                assert_eq!(head.status, 200);
                saw_head = true;
            }
            assert!(!matches!(event, ServiceEvent::BodyChunk { .. }));
        }
        if saw_head {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_head, "response head must precede delayed body");
    send_body.send(()).unwrap();
    let mut saw_chunk = false;
    for tick in 0..2000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            if let ServiceEvent::BodyChunk { id: found, bytes } = event {
                assert_eq!(found, id);
                assert_eq!(bytes, b"first");
                saw_chunk = true;
            }
        }
        if saw_chunk {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_chunk);
    assert!(service.cancel(id));
    let tail = service.step(8192, Instant::now());
    assert_eq!(
        tail.iter()
            .filter(|event| matches!(event,
        ServiceEvent::StreamFailed { id: found, error: ApiError::Aborted } if *found == id))
            .count(),
        1
    );
    assert!(!tail
        .iter()
        .any(|event| matches!(event, ServiceEvent::StreamFinished { .. })));
    assert!(!service.cancel(id));
    server.join().unwrap();
}

#[test]
fn network_chunk_order_matches_buffered_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/chunks",
        listener.local_addr().unwrap().port()
    );
    let pieces: [&[u8]; 3] = [b"first", b"-second", b"-third"];
    let expected = pieces.concat();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: 18\r\n\r\n").unwrap();
            for piece in pieces {
                socket.write_all(piece).unwrap();
                thread::sleep(Duration::from_millis(5));
            }
        }
    });
    let base = HttpUrl::parse(&url).unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service
        .start_stream(request(&url, &base), ClientContext::default())
        .unwrap();
    let start = Instant::now();
    let mut streamed = Vec::new();
    let mut done = false;
    for tick in 0..2000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            match event {
                ServiceEvent::BodyChunk { id: found, bytes } if found == id => {
                    streamed.extend(bytes)
                }
                ServiceEvent::StreamFinished { id: found } if found == id => done = true,
                ServiceEvent::StreamFailed { id: found, error } if found == id => {
                    panic!("{error:?}")
                }
                _ => {}
            }
        }
        if done {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(done);
    assert_eq!(streamed, expected);
    let id = service
        .start(request(&url, &base), ClientContext::default())
        .unwrap();
    let mut buffered = None;
    for tick in 0..2000 {
        for event in service.step(8192, start + Duration::from_millis(2000 + tick)) {
            if let ServiceEvent::Finished { id: found, result } = event {
                if found == id {
                    buffered = Some(result.unwrap());
                }
            }
        }
        if buffered.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    let mut buffered = buffered.expect("buffered response");
    assert_eq!(buffered.body.bytes().unwrap(), streamed);
    server.join().unwrap();
}

#[test]
fn opaque_head_waits_for_bounded_sniff_prefix() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/opaque",
        listener.local_addr().unwrap().port()
    );
    let (release, wait) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2048\r\n\r\n",
            )
            .unwrap();
        socket.write_all(&vec![b'x'; 1024]).unwrap();
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        socket.write_all(&vec![b'y'; 1024]).unwrap();
    });
    let base = HttpUrl::parse("http://client.example/").unwrap();
    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    let units: Vec<_> = url.encode_utf16().collect();
    let request = ApiRequest::new(
        ApiRequestInput::Url(&units),
        init,
        &base,
        base.origin().clone(),
    )
    .unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service
        .start_stream(request, ClientContext::default())
        .unwrap();
    let start = Instant::now();
    let mut head = None;
    for tick in 0..2000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            match event {
                ServiceEvent::ResponseHead {
                    id: found,
                    head: value,
                } if found == id => head = Some(value),
                ServiceEvent::BodyChunk { id: found, .. } if found == id => {
                    panic!("opaque body exposed")
                }
                _ => {}
            }
        }
        if head.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    let head = head.expect("head should be exposed after 1024 bytes, before EOF");
    assert_eq!(head.status, 0);
    assert!(head.headers.is_empty());
    release.send(()).unwrap();
    let mut done = false;
    for tick in 0..2000 {
        for event in service.step(8192, start + Duration::from_millis(2000 + tick)) {
            match event {
                ServiceEvent::StreamFinished { id: found } if found == id => done = true,
                ServiceEvent::BodyChunk { id: found, .. } if found == id => {
                    panic!("opaque body exposed")
                }
                _ => {}
            }
        }
        if done {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(done);
    server.join().unwrap();
}
