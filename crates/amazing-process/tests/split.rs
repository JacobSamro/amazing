use amazing_ipc::codec::{self, ByteBuf};
use amazing_ipc::Error;
use amazing_process::network::{fetch_resource, NetworkBroker};
use amazing_process::protocol::*;
use amazing_process::renderer::render_document;
use amazing_process::{Capabilities, ProcessHost, RendererMonitor, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn exe() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"))
}
fn spawn(role: Role) -> ProcessHost {
    ProcessHost::spawn(
        exe(),
        role,
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap()
}
struct Events(mpsc::Sender<u64>);
impl BrowserEvents for Events {
    fn frame_ready(&self, event: FrameReady) -> Result<(), Error> {
        self.0.send(event.sequence).map_err(|_| Error::Disconnected)
    }
}
fn connect(renderer: &ProcessHost, network: &ProcessHost) -> (RendererProxy, mpsc::Receiver<u64>) {
    let (tx, rx) = mpsc::channel();
    renderer.connection.bind(
        8,
        NetworkReceiver(NetworkBroker {
            remote: NetworkProxy::new(network.connection.clone(), Duration::from_secs(5)),
        }),
    );
    renderer
        .connection
        .bind(9, BrowserEventsReceiver(Events(tx)));
    (
        RendererProxy::new(renderer.connection.clone(), Duration::from_secs(5)),
        rx,
    )
}
fn data_url(mime: &str, bytes: &[u8]) -> String {
    let mut out = format!("data:{mime},");
    for &b in bytes {
        if b.is_ascii_alphanumeric() {
            out.push(b as char)
        } else {
            out.push_str(&format!("%{b:02X}"))
        }
    }
    out
}
#[test]
fn renderer_network_frame_parity_and_input_events() {
    let mut network = spawn(Role::Network);
    let mut renderer = spawn(Role::Renderer);
    network.ping(Duration::from_secs(2)).unwrap();
    renderer.ping(Duration::from_secs(2)).unwrap();
    let (proxy, notifications) = connect(&renderer, &network);
    let css = data_url(
        "text/css",
        b"body{background:#123456}img{width:12px;height:8px}",
    );
    let svg = data_url("image/svg+xml", br#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="8"><rect width="12" height="8" fill="red"/></svg>"#);
    let html = format!("<!doctype html><html><head><link rel=stylesheet href=\"{css}\"></head><body><img src=\"{svg}\"><p>split</p></body></html>");
    let request = Navigate {
        url: "http://example.test/page".into(),
        html_bytes: ByteBuf(html.into_bytes()),
        viewport: Viewport {
            width: 96,
            height: 64,
        },
    };
    let mut expected = render_document(&request, |url, base| {
        fetch_resource(
            &NetworkProxy::new(network.connection.clone(), Duration::from_secs(5)),
            url,
            base,
        )
    })
    .unwrap();
    let actual = proxy.navigate(request).unwrap();
    expected.sequence = actual.sequence;
    actual.validate().unwrap();
    assert!(
        actual
            .rgba
            .0
            .chunks_exact(4)
            .any(|p| p[0] > 180 && p[1] < 80 && p[2] < 80),
        "image pixels missing"
    );
    assert!(
        actual
            .rgba
            .0
            .chunks_exact(4)
            .any(|p| p[0] == 0x12 && p[1] == 0x34 && p[2] == 0x56),
        "stylesheet pixels missing"
    );
    assert_eq!(actual, expected);
    assert_eq!(
        notifications.recv_timeout(Duration::from_secs(2)).unwrap(),
        actual.sequence
    );
    for event in [
        InputEvent::Click(Point { x: 4, y: 5 }),
        InputEvent::Scroll(ScrollDelta { x: 0, y: 2 }),
        InputEvent::Key("a".into()),
    ] {
        let ack = proxy.input(event).unwrap();
        assert_eq!(
            notifications.recv_timeout(Duration::from_secs(2)).unwrap(),
            ack.sequence
        );
    }
    renderer.kill().unwrap();
    network.kill().unwrap();
}
#[test]
fn crash_during_navigation_reports_one_sad_tab() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/slow.css",
        listener.local_addr().unwrap().port()
    );
    let mut network = spawn(Role::Network);
    let mut renderer = spawn(Role::Renderer);
    let (proxy, _notifications) = connect(&renderer, &network);
    let request = Navigate {
        url: url.clone(),
        html_bytes: ByteBuf(
            format!("<link rel=stylesheet href=\"{url}\"><p>wait</p>").into_bytes(),
        ),
        viewport: Viewport {
            width: 64,
            height: 64,
        },
    };
    let pending = std::thread::spawn(move || proxy.navigate(request));
    let deadline = Instant::now() + Duration::from_secs(2);
    let socket = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) => panic!("navigation never reached network process: {e}"),
        }
    };
    renderer.kill().unwrap();
    assert!(matches!(
        pending.join().unwrap(),
        Err(Error::Disconnected) | Err(Error::Io(_))
    ));
    let mut monitor = RendererMonitor::new();
    let sad = monitor.poll(&mut renderer).unwrap().expect("sad tab");
    assert_eq!(sad.pid, renderer.pid());
    assert!(monitor.poll(&mut renderer).unwrap().is_none());
    drop(socket);
    network.kill().unwrap();
}
#[test]
fn all_interface_decoders_are_bounded_and_panic_free() {
    let mut seed = 0x8c0f_1b2d_37a5_ef90u64;
    for len in 0..1024 {
        let mut bytes = vec![0; len % 271];
        for b in &mut bytes {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *b = seed as u8;
        }
        macro_rules! decode_all {
            ($bytes:expr) => {{
                let _ = codec::decode::<Navigate>($bytes);
                let _ = codec::decode::<Frame>($bytes);
                let _ = codec::decode::<InputEvent>($bytes);
                let _ = codec::decode::<InputAck>($bytes);
                let _ = codec::decode::<FrameReady>($bytes);
                let _ = codec::decode::<SadTab>($bytes);
                let _ = codec::decode::<StartRequest>($bytes);
                let _ = codec::decode::<Started>($bytes);
                let _ = codec::decode::<RequestId>($bytes);
                let _ = codec::decode::<NetworkEvent>($bytes);
            }};
        }
        decode_all!(&bytes);
    }
    fn check<T: codec::Wire>(sample: T, oversized: &[u8]) {
        let valid = codec::encode(&sample).unwrap();
        for len in 0..valid.len() {
            assert!(
                codec::decode::<T>(&valid[..len]).is_err(),
                "truncated at {len}"
            );
        }
        assert!(matches!(
            codec::decode::<T>(oversized),
            Err(codec::Error::Limit)
        ));
    }
    let oversized = vec![0; codec::MAX_FRAME + 1];
    check(
        Navigate {
            url: "http://example.test".into(),
            html_bytes: ByteBuf(vec![1, 2, 3]),
            viewport: Viewport {
                width: 1,
                height: 1,
            },
        },
        &oversized,
    );
    check(
        Frame {
            sequence: 1,
            width: 1,
            height: 1,
            rgba: ByteBuf(vec![1, 2, 3, 4]),
        },
        &oversized,
    );
    check(InputEvent::Key("a".into()), &oversized);
    check(InputAck { sequence: 1 }, &oversized);
    check(FrameReady { sequence: 1 }, &oversized);
    check(
        SadTab {
            pid: 1,
            reason: "crash".into(),
        },
        &oversized,
    );
    check(
        StartRequest {
            url: "data:text/plain,a".into(),
            base_url: "http://example.test".into(),
        },
        &oversized,
    );
    check(Started { id: 1 }, &oversized);
    check(RequestId { id: 1 }, &oversized);
    check(
        NetworkEvent::Body(BodyChunk {
            bytes: ByteBuf(vec![1, 2, 3]),
        }),
        &oversized,
    );
}
