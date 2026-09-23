use amazing_fetch::{
    api::{ApiRequest, ApiRequestInit, ApiRequestInput, PendingFetchApi},
    xhr::{Xhr, XhrState},
    HttpUrl,
};
use amazing_net::dispatch::{DispatchLimits, Dispatcher};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

struct Server {
    listener: TcpListener,
    peer: Option<TcpStream>,
    input: Vec<u8>,
    replied: bool,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        Self {
            listener,
            peer: None,
            input: Vec::new(),
            replied: false,
        }
    }
    fn url(&self) -> HttpUrl {
        HttpUrl::parse(&format!(
            "http://127.0.0.1:{}/resource",
            self.listener.local_addr().unwrap().port()
        ))
        .unwrap()
    }
    fn tick(&mut self) {
        if self.peer.is_none() {
            if let Ok((socket, _)) = self.listener.accept() {
                socket.set_nonblocking(true).unwrap();
                self.peer = Some(socket);
            }
        }
        if let Some(peer) = &mut self.peer {
            let mut buf = [0; 4096];
            loop {
                match peer.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => self.input.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("{e}"),
                }
            }
            if !self.replied && self.input.windows(4).any(|w| w == b"\r\n\r\n") {
                peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .unwrap();
                self.replied = true;
            }
        }
    }
}
#[test]
fn fetch_api_dispatch_finishes_without_blocking_event_loop() {
    let mut server = Server::new();
    let url = server.url();
    let units: Vec<_> = url.serialize().encode_utf16().collect();
    let mut request = ApiRequest::new(
        ApiRequestInput::Url(&units),
        ApiRequestInit::default(),
        &url,
        url.origin().clone(),
    )
    .unwrap();
    let mut dispatcher = Dispatcher::new(DispatchLimits::default());
    let now = Instant::now();
    let mut pending = PendingFetchApi::start(&mut request, &mut dispatcher, now).unwrap();
    let mut response = None;
    for tick in 0..1000 {
        server.tick();
        for event in dispatcher.step(1024, now + Duration::from_millis(tick)) {
            if let Some(result) =
                pending.on_event(&event, &mut dispatcher, now + Duration::from_millis(tick))
            {
                response = Some(result.unwrap());
            }
        }
        if response.is_some() {
            break;
        }
    }
    let mut response = response.expect("completed");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.bytes().unwrap(), b"ok");
}
#[test]
fn xhr_can_consume_dispatcher_events() {
    let mut server = Server::new();
    let url = server.url();
    let now = Instant::now();
    let mut xhr = Xhr::default();
    xhr.open(
        &"GET".encode_utf16().collect::<Vec<_>>(),
        &url.serialize().encode_utf16().collect::<Vec<_>>(),
        true,
        &url,
        url.origin().clone(),
    )
    .unwrap();
    xhr.send(None, 0).unwrap();
    let mut dispatcher = Dispatcher::new(DispatchLimits::default());
    let mut pending = xhr.start_dispatch(&mut dispatcher, 0, now).unwrap();
    for tick in 0..1000 {
        server.tick();
        for event in dispatcher.step(1024, now + Duration::from_millis(tick)) {
            xhr.on_dispatch_event(
                &mut pending,
                &event,
                &mut dispatcher,
                tick,
                now + Duration::from_millis(tick),
            )
            .unwrap();
        }
        if xhr.state() == XhrState::HeadersReceived {
            break;
        }
    }
    assert_eq!(xhr.state(), XhrState::HeadersReceived);
    xhr.pull_response(10, 1000).unwrap();
    assert_eq!(xhr.state(), XhrState::Done);
    assert_eq!(xhr.status(), 200);
}

#[test]
fn xhr_timeout_cancels_without_network_event() {
    let server = Server::new();
    let url = server.url();
    let now = Instant::now();
    let mut xhr = Xhr::default();
    xhr.open(
        &"GET".encode_utf16().collect::<Vec<_>>(),
        &url.serialize().encode_utf16().collect::<Vec<_>>(),
        true,
        &url,
        url.origin().clone(),
    )
    .unwrap();
    xhr.set_timeout(1).unwrap();
    xhr.send(None, 0).unwrap();
    let mut dispatcher = Dispatcher::new(DispatchLimits::default());
    let pending = xhr.start_dispatch(&mut dispatcher, 0, now).unwrap();
    assert_eq!(dispatcher.in_flight(), 1);
    assert!(xhr
        .check_dispatch_timeout(&pending, &mut dispatcher, 2)
        .unwrap()
        .is_some());
    assert_eq!(dispatcher.in_flight(), 0);
}
