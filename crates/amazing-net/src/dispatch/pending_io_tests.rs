//! Native producer controls. Provider gates delay delivery around genuine std
//! operations; these tests do not claim interruption of a libc resolver/syscall.
use super::*;
use std::{
    net::TcpListener,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
};

#[derive(Clone)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(false), Condvar::new())))
    }
    fn wait(&self) {
        let (lock, changed) = &*self.0;
        let held = lock.lock().unwrap();
        let (held, timeout) = changed
            .wait_timeout_while(held, Duration::from_secs(5), |released| !*released)
            .unwrap();
        assert!(*held && !timeout.timed_out(), "fixture release absent");
    }
    fn release(&self) {
        let (lock, changed) = &*self.0;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}
static SERIAL: Mutex<()> = Mutex::new(());
// Always release native workers if an assertion unwinds.
impl Drop for Gate {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 2 {
            self.release();
        }
    }
}
struct DelayedResolver {
    gate: Gate,
    entered: mpsc::Sender<()>,
}
impl Resolver for DelayedResolver {
    fn resolve(&mut self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        StdResolver.resolve(host, port)
    }
    fn begin_resolve(&mut self, host: &str, port: u16, context: &PendingIoContext) -> ResolveStep {
        if host != "localhost" {
            return ResolveStep::Ready(self.resolve(host, port));
        }
        let gate = self.gate.clone();
        let entered = self.entered.clone();
        let host = host.to_owned();
        ResolveStep::Pending(
            context
                .resolve(move || {
                    entered.send(()).unwrap();
                    gate.wait();
                    StdResolver.resolve(&host, port)
                })
                .unwrap(),
        )
    }
}
struct DelayedConnector {
    gate: Gate,
    entered: mpsc::Sender<()>,
}
impl Connector for DelayedConnector {
    fn connect(&mut self, address: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        StdConnector.connect(address, timeout)
    }
    fn begin_connect(
        &mut self,
        address: SocketAddr,
        timeout: Duration,
        context: &PendingIoContext,
    ) -> ConnectStep {
        let gate = self.gate.clone();
        let entered = self.entered.clone();
        ConnectStep::Pending(
            context
                .connect(move || {
                    let socket = TcpStream::connect_timeout(&address, timeout)?;
                    entered.send(()).unwrap();
                    gate.wait();
                    Ok(socket)
                })
                .unwrap(),
        )
    }
}
fn read_head(io: &mut impl Read) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        io.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
        assert!(head.len() < 32768);
    }
    String::from_utf8(head).unwrap()
}
fn pump<R: Resolver, C: Connector>(
    dispatcher: &mut Dispatcher<R, C>,
    mut accept: impl FnMut(NetEvent) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < end, "dispatcher stalled");
        for event in dispatcher.step(128, Instant::now()) {
            if accept(event) {
                return;
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn four_pending_lookups_do_not_block_sibling_and_abort_supersede_emit_once() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let gate = Gate::new();
    let (entered, seen) = mpsc::channel();
    let mut limits = DispatchLimits::default();
    limits.max_in_flight = 16;
    let mut dispatcher = Dispatcher::with_io_pending(
        limits,
        DelayedResolver {
            gate: gate.clone(),
            entered,
        },
        StdConnector,
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut slow = Vec::new();
    for _ in 0..4 {
        slow.push(
            dispatcher
                .submit(
                    &format!("http://localhost:{port}/slow"),
                    FetchOptions::default(),
                    Instant::now(),
                )
                .unwrap(),
        );
    }
    dispatcher.step(128, Instant::now());
    for _ in 0..4 {
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    let peer = thread::spawn(move || {
        let (mut io, _) = listener.accept().unwrap();
        io.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert!(read_head(&mut io).starts_with("GET /replacement "));
        io.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    let fast = dispatcher
        .submit(
            &format!("http://127.0.0.1:{port}/replacement"),
            FetchOptions::default(),
            Instant::now(),
        )
        .unwrap();
    let mut bytes = Vec::new();
    pump(&mut dispatcher, |event| match event {
        NetEvent::Body { id, bytes: body } if id == fast => {
            bytes.extend(body);
            false
        }
        NetEvent::Done { id } if id == fast => true,
        NetEvent::Error { error, .. } => panic!("{error:?}"),
        _ => false,
    });
    assert_eq!(bytes, b"ok");
    peer.join().unwrap();
    let snapshot = dispatcher.pending_io_snapshot().unwrap();
    assert!(snapshot.running_resolution >= 4);
    for id in &slow {
        assert!(dispatcher.cancel(*id));
        assert!(!dispatcher.cancel(*id));
    }
    let events = dispatcher.step(128, Instant::now());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, NetEvent::Aborted { .. }))
            .count(),
        4
    );
    assert!(
        dispatcher
            .pending_io_snapshot()
            .unwrap()
            .cancelled_running_resolution
            >= 4
    );
    gate.release();
    for _ in 0..20 {
        assert!(dispatcher.step(128, Instant::now()).is_empty());
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn pending_real_connect_cancel_timeout_and_actor_drop_never_send_http_or_ws() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for websocket in [false, true] {
        for action in 0..3 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let gate = Gate::new();
            let (entered, seen) = mpsc::channel();
            let mut dispatcher = Dispatcher::with_io_pending(
                DispatchLimits::default(),
                StdResolver,
                DelayedConnector {
                    gate: gate.clone(),
                    entered,
                },
            )
            .unwrap();
            let now = Instant::now();
            let timeout = Duration::from_secs(2);
            let mut http = None;
            let mut ws = None;
            if websocket {
                let mut options = crate::websocket::Options::default();
                options.timeout = timeout;
                ws = Some(
                    dispatcher
                        .open_websocket(&format!("ws://{address}/"), options, None, now)
                        .unwrap(),
                );
                dispatcher.step_websockets(128, now);
                dispatcher.step_websockets(128, now);
            } else {
                let mut options = FetchOptions::default();
                options.timeout = timeout;
                http = Some(
                    dispatcher
                        .submit(&format!("http://{address}/"), options, now)
                        .unwrap(),
                );
                dispatcher.step(128, now);
                dispatcher.step(128, now);
            }
            seen.recv_timeout(Duration::from_secs(5)).unwrap();
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            match action {
                0 => {
                    if let Some(id) = ws {
                        assert!(dispatcher.forget_websocket(id));
                        assert!(!dispatcher.forget_websocket(id));
                        assert!(dispatcher.step_websockets(128, now).is_empty());
                    } else {
                        let id = http.unwrap();
                        assert!(dispatcher.cancel(id));
                        assert!(!dispatcher.cancel(id));
                        let events = dispatcher.step(128, now);
                        assert_eq!(events.len(), 1);
                        assert!(matches!(events[0],NetEvent::Aborted{id:got}if got==id));
                    }
                }
                1 => {
                    if websocket {
                        let events = dispatcher.step_websockets(128, now + timeout);
                        assert_eq!(events.len(), 1);
                        assert!(matches!(events[0], WebSocketEvent::Error { .. }));
                    } else {
                        let events = dispatcher.step(128, now + timeout);
                        assert_eq!(events.len(), 1);
                        assert!(matches!(
                            events[0],
                            NetEvent::Error {
                                error: DispatchError::Timeout,
                                ..
                            }
                        ));
                    }
                }
                _ => {}
            }
            if action < 2 {
                assert!(
                    dispatcher
                        .pending_io_snapshot()
                        .unwrap()
                        .cancelled_running_connect
                        >= 1
                );
            }
            drop(dispatcher);
            // A replacement actor sees the same physical allocation: logical
            // retirement cannot turn a still-running callback into a free slot.
            let replacement = Dispatcher::new_pending_io(DispatchLimits::default()).unwrap();
            assert!(
                replacement
                    .pending_io_snapshot()
                    .unwrap()
                    .cancelled_running_connect
                    >= 1
            );
            drop(replacement);
            gate.release();
            assert_eq!(
                peer.read(&mut [0; 1]).unwrap(),
                0,
                "cancelled native stream sent request bytes"
            );
        }
    }
}

#[test]
fn pending_std_http_https_ws_wss_use_actual_requests_and_verified_handshake() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for tls in [false, true] {
        for websocket in [false, true] {
            fn respond(io: &mut impl ReadWrite, websocket: bool) {
                let head = read_head(io);
                assert!(head.starts_with("GET /native "));
                if websocket {
                    let key = head
                        .lines()
                        .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                        .unwrap()
                        .trim();
                    let accept = crate::websocket::handshake::accept(key);
                    io.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).unwrap();
                    io.flush().unwrap();
                    let mut byte = [0];
                    let _ = io.read(&mut byte);
                } else {
                    io.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .unwrap();
                    io.flush().unwrap();
                }
            }
            let (port, plain, secure) = if tls {
                let (port, peer) =
                    amazing_net_tls::test_support::serve(vec![b"http/1.1".to_vec()], move |io| {
                        respond(io, websocket)
                    });
                (port, None, Some(peer))
            } else {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let port = listener.local_addr().unwrap().port();
                let peer = thread::spawn(move || {
                    let (mut io, _) = listener.accept().unwrap();
                    io.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    respond(&mut io, websocket)
                });
                (port, Some(peer), None)
            };
            let policy = ClientPolicy::from_der_trust_anchors([include_bytes!(
                "../../../amazing-net-tls/tests/fixtures/ca.der"
            )
            .to_vec()])
            .unwrap();
            let mut dispatcher =
                Dispatcher::with_tls_policy_pending_io(DispatchLimits::default(), policy).unwrap();
            let scheme = match (tls, websocket) {
                (false, false) => "http",
                (true, false) => "https",
                (false, true) => "ws",
                (true, true) => "wss",
            };
            let url = format!("{scheme}://localhost:{port}/native");
            if websocket {
                let id = dispatcher
                    .open_websocket(
                        &url,
                        crate::websocket::Options::default(),
                        None,
                        Instant::now(),
                    )
                    .unwrap();
                let end = Instant::now() + Duration::from_secs(5);
                let mut opened = false;
                while !opened {
                    assert!(Instant::now() < end);
                    for event in dispatcher.step_websockets(128, Instant::now()) {
                        match event {
                            WebSocketEvent::Open { id: got, .. } => {
                                assert_eq!(got, id);
                                opened = true
                            }
                            WebSocketEvent::Error { error, .. } => panic!("{error:?}"),
                            _ => {}
                        }
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                assert!(dispatcher.forget_websocket(id));
            } else {
                let id = dispatcher
                    .submit(&url, FetchOptions::default(), Instant::now())
                    .unwrap();
                let mut body = Vec::new();
                pump(&mut dispatcher, |event| match event {
                    NetEvent::Body { bytes, .. } => {
                        body.extend(bytes);
                        false
                    }
                    NetEvent::Done { id: got } => {
                        assert_eq!(got, id);
                        true
                    }
                    NetEvent::Error { error, .. } => panic!("{error:?}"),
                    _ => false,
                });
                assert_eq!(body, b"ok");
            }
            drop(dispatcher);
            if let Some(peer) = plain {
                peer.join().unwrap();
            }
            if let Some(peer) = secure {
                peer.join().unwrap().unwrap();
            }
        }
    }
}
trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

#[test]
fn pending_constructor_preserves_non_send_synchronous_provider_and_ordered_address_fallback() {
    use std::{cell::Cell, rc::Rc};
    struct OrderedResolver {
        calls: Rc<Cell<usize>>,
        addresses: Vec<SocketAddr>,
    }
    impl Resolver for OrderedResolver {
        fn resolve(&mut self, _: &str, _: u16) -> io::Result<Vec<SocketAddr>> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.addresses.clone())
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let live = listener.local_addr().unwrap();
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let refused = dead.local_addr().unwrap();
    drop(dead);
    let peer = thread::spawn(move || {
        let (mut io, _) = listener.accept().unwrap();
        io.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert!(read_head(&mut io).contains("Host: example.invalid:"));
        io.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let calls = Rc::new(Cell::new(0));
    let resolver = OrderedResolver {
        calls: calls.clone(),
        addresses: vec![refused, live],
    };
    let mut dispatcher =
        Dispatcher::with_io_pending(DispatchLimits::default(), resolver, StdConnector).unwrap();
    let id = dispatcher
        .submit(
            &format!("http://example.invalid:{}/fallback", live.port()),
            FetchOptions::default(),
            Instant::now(),
        )
        .unwrap();
    pump(&mut dispatcher, |event| match event {
        NetEvent::Done { id: got } => {
            assert_eq!(got, id);
            true
        }
        NetEvent::Error { error, .. } => panic!("{error:?}"),
        _ => false,
    });
    assert_eq!(calls.get(), 1);
    peer.join().unwrap();
}

#[test]
fn pending_io_default_roots_still_reject_untrusted_https_and_wss() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for websocket in [false, true] {
        let admitted = Arc::new(AtomicBool::new(false));
        let observed = admitted.clone();
        let (port, peer) =
            amazing_net_tls::test_support::serve(vec![b"http/1.1".to_vec()], move |_| {
                observed.store(true, Ordering::SeqCst);
            });
        let mut dispatcher = Dispatcher::new_pending_io(DispatchLimits::default()).unwrap();
        if websocket {
            let id = dispatcher
                .open_websocket(
                    &format!("wss://localhost:{port}/"),
                    crate::websocket::Options::default(),
                    None,
                    Instant::now(),
                )
                .unwrap();
            let limit = Instant::now() + Duration::from_secs(5);
            let mut rejected = false;
            while !rejected {
                assert!(Instant::now() < limit);
                for event in dispatcher.step_websockets(128, Instant::now()) {
                    match event {
                        WebSocketEvent::Error { id: got, .. } => {
                            assert_eq!(got, id);
                            rejected = true
                        }
                        WebSocketEvent::Open { .. } => panic!("untrusted WSS opened"),
                        _ => {}
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        } else {
            let id = dispatcher
                .submit(
                    &format!("https://localhost:{port}/"),
                    FetchOptions::default(),
                    Instant::now(),
                )
                .unwrap();
            pump(&mut dispatcher, |event| match event {
                NetEvent::Error {
                    id: got,
                    error: DispatchError::Network(NetError::Tls(_)),
                } => {
                    assert_eq!(got, id);
                    true
                }
                other => panic!("unexpected untrusted TLS event {other:?}"),
            });
        }
        drop(dispatcher);
        assert!(peer.join().unwrap().is_err());
        assert!(!admitted.load(Ordering::SeqCst));
    }
}
