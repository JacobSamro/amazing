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

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Venus AppArmor denies unprivileged user namespace UID/GID mapping"]
fn sandboxed_renderer_preserves_shared_frame_parity() {
    let mut network = spawn(Role::Network);
    let mut renderer = ProcessHost::spawn_sandboxed_renderer(
        exe(),
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap();
    let (proxy, _) = connect(&renderer, &network);
    let request = Navigate {
        url: "http://example.test/sandbox".into(),
        html_bytes: ByteBuf(b"<style>body{background:#123456}</style><p>sandbox</p>".to_vec()),
        viewport: Viewport {
            width: 64,
            height: 64,
        },
    };
    let expected = render_document(&request, |_, _| {
        Err(Error::Remote("unexpected resource".into()))
    })
    .unwrap();
    let ready = proxy.navigate_shared(request).unwrap();
    assert!(matches!(
        proxy
            .acquire_frame(AcquireFrame {
                generation: ready.generation
            })
            .unwrap(),
        AcquireResult::Transferred(())
    ));
    let actual = renderer.receive_frame(&ready).unwrap();
    assert_eq!(actual.bytes(), expected.rgba.0);
    assert!(proxy
        .frame_received(AcquireFrame {
            generation: ready.generation
        })
        .unwrap());
    assert!(proxy
        .release_frame(ReleaseFrame {
            generation: ready.generation
        })
        .unwrap());
    drop(actual);
    renderer.kill().unwrap();
    network.kill().unwrap();
}
struct Events(mpsc::Sender<FrameReady>);
impl BrowserEvents for Events {
    fn frame_ready(&self, event: FrameReady) -> Result<(), Error> {
        self.0.send(event).map_err(|_| Error::Disconnected)
    }
}
fn connect(
    renderer: &ProcessHost,
    network: &ProcessHost,
) -> (RendererProxy, mpsc::Receiver<FrameReady>) {
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
    let actual = proxy.navigate(request.clone()).unwrap();
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
        notifications
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .sequence,
        actual.sequence
    );
    let shared = proxy.navigate_shared(request).unwrap();
    let view = renderer.receive_frame(&shared).unwrap();
    assert_eq!(view.bytes(), actual.rgba.0);
    let event = notifications.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(event.shared, Some(shared.clone()));
    assert!(proxy
        .release_frame(ReleaseFrame {
            generation: shared.generation
        })
        .unwrap());
    drop(view);
    for event in [
        InputEvent::Click(Point { x: 4, y: 5 }),
        InputEvent::Scroll(ScrollDelta { x: 0, y: 2 }),
        InputEvent::Key("a".into()),
    ] {
        let ack = proxy.input(event).unwrap();
        assert_eq!(
            notifications
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .sequence,
            ack.sequence
        );
    }
    renderer.kill().unwrap();
    network.kill().unwrap();
}
#[test]
fn shared_frames_match_byte_fallback_at_multiple_viewports() {
    let mut network = spawn(Role::Network);
    let mut renderer = spawn(Role::Renderer);
    let (_short_proxy, notifications) = connect(&renderer, &network);
    let proxy = RendererProxy::new(renderer.connection.clone(), Duration::from_secs(5));
    let mut measurements = Vec::new();
    for (width, height) in [(96, 64), (112, 72), (128, 96)] {
        let request = Navigate {
            url: "http://example.test/shared".into(),
            html_bytes: ByteBuf(b"<style>body{background:#123456}</style><p>shared</p>".to_vec()),
            viewport: Viewport { width, height },
        };
        let byte_start = Instant::now();
        let copied = proxy.navigate(request.clone()).unwrap();
        let byte_time = byte_start.elapsed();
        assert_eq!(
            notifications
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .sequence,
            copied.sequence
        );
        let shared_start = Instant::now();
        let ready = proxy.navigate_shared(request).unwrap();
        let view = renderer.receive_frame(&ready).unwrap();
        let shared_time = shared_start.elapsed();
        let mut forged = ready.clone();
        forged.handle_id = 0;
        assert!(forged.metadata().is_err());
        forged = ready.clone();
        forged.byte_len += 4;
        assert!(forged.metadata().is_err());
        assert!(renderer.receive_frame(&ready).is_err()); // stale/duplicate metadata fails before mapping
        let event = notifications.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.sequence, ready.generation);
        assert_eq!(event.shared, Some(ready.clone()));
        assert_eq!(ready.byte_len as usize, copied.rgba.0.len());
        assert_eq!(view.bytes(), copied.rgba.0);
        assert!(proxy
            .release_frame(ReleaseFrame {
                generation: ready.generation
            })
            .unwrap());
        assert!(!proxy
            .release_frame(ReleaseFrame {
                generation: ready.generation
            })
            .unwrap());
        assert_eq!(view.bytes(), copied.rgba.0); // Browser view owns its mapping until Drop.
        measurements.push((
            width,
            height,
            byte_time.as_micros(),
            shared_time.as_micros(),
        ));
    }
    eprintln!("IPC-3 frame times (viewport, byte us, shared us): {measurements:?}; frame transfer copies: byte >=2, shared 1");
    renderer.kill().unwrap();
    network.kill().unwrap();
}

#[test]
fn shared_frame_survives_renderer_crash_and_releases_handles() {
    #[cfg(target_os = "linux")]
    let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
    let mut network = spawn(Role::Network);
    let request = Navigate {
        url: "http://example.test/crash".into(),
        html_bytes: ByteBuf(b"<p>crash phase</p>".to_vec()),
        viewport: Viewport {
            width: 128,
            height: 128,
        },
    };
    for phase in ["spawned", "metadata", "acquired", "mapped"] {
        let mut renderer = spawn(Role::Renderer);
        let (proxy, notifications) = connect(&renderer, &network);
        let mut view = None;
        if phase != "spawned" {
            let ready = proxy.navigate_shared(request.clone()).unwrap();
            assert_eq!(
                notifications
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .sequence,
                ready.generation
            );
            if phase == "acquired" {
                assert!(matches!(
                    proxy
                        .acquire_frame(AcquireFrame {
                            generation: ready.generation
                        })
                        .unwrap(),
                    AcquireResult::Transferred(())
                ));
            } else if phase == "mapped" {
                view = Some(renderer.receive_frame(&ready).unwrap());
            }
        }
        renderer.kill().unwrap();
        if let Some(view) = &view {
            assert_eq!(view.bytes().len(), 128 * 128 * 4);
        }
        drop(view);
        drop(proxy);
        drop(notifications);
        drop(renderer);
    }
    network.kill().unwrap();
    drop(network);
    #[cfg(target_os = "linux")]
    {
        for _ in 0..50 {
            if std::fs::read_dir("/proc/self/fd").unwrap().count() <= before + 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let after = std::fs::read_dir("/proc/self/fd").unwrap().count();
        assert!(
            after <= before + 2,
            "file descriptors: before={before}, after={after}"
        );
    }
}
#[test]
fn demand_driven_acquire_skips_superseded_frames() {
    let mut network = spawn(Role::Network);
    let mut renderer = spawn(Role::Renderer);
    let (proxy, notifications) = connect(&renderer, &network);
    let request = Navigate {
        url: "http://example.test/latest".into(),
        html_bytes: ByteBuf(b"<p>latest</p>".to_vec()),
        viewport: Viewport {
            width: 64,
            height: 64,
        },
    };
    let mut ready = Vec::new();
    for _ in 0..8 {
        let frame = proxy.navigate_shared(request.clone()).unwrap();
        assert_eq!(
            notifications
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .sequence,
            frame.generation
        );
        ready.push(frame);
    }
    assert!(matches!(
        proxy
            .acquire_frame(AcquireFrame {
                generation: ready[0].generation
            })
            .unwrap(),
        AcquireResult::Superseded(_)
    ));
    assert!(renderer.receive_frame(&ready[0]).is_err());
    let view = renderer.receive_frame(ready.last().unwrap()).unwrap();
    assert_eq!(view.bytes().len(), 64 * 64 * 4);
    assert!(proxy
        .release_frame(ReleaseFrame {
            generation: ready.last().unwrap().generation
        })
        .unwrap());
    drop(view);
    renderer.kill().unwrap();
    network.kill().unwrap();
}

#[test]
fn forged_transferred_handle_closes_the_channel() {
    let mut network = spawn(Role::Network);
    let mut renderer = spawn(Role::Renderer);
    let (proxy, notifications) = connect(&renderer, &network);
    let ready = proxy
        .navigate_shared(Navigate {
            url: "http://example.test/forged".into(),
            html_bytes: ByteBuf(b"<p>forged</p>".to_vec()),
            viewport: Viewport {
                width: 64,
                height: 64,
            },
        })
        .unwrap();
    assert_eq!(
        notifications
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .sequence,
        ready.generation
    );
    let mut forged = ready;
    forged.handle_id = u64::MAX;
    assert!(renderer.receive_frame(&forged).is_err());
    assert!(renderer.connection.is_closed());
    let _ = renderer.kill();
    network.kill().unwrap();
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn browser_crash_reaps_renderer_at_each_transfer_phase() {
    use std::io::{BufRead, BufReader};
    for phase in ["spawned", "published", "acquired", "mapped"] {
        let mut browser = std::process::Command::new(exe())
            .arg(format!("--browser-crash-probe={phase}"))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut pid_line = String::new();
        BufReader::new(browser.stdout.take().unwrap())
            .read_line(&mut pid_line)
            .unwrap();
        let pid: u32 = pid_line.trim().parse().unwrap();
        assert_eq!(browser.wait().unwrap().code(), Some(77));
        let deadline = Instant::now() + Duration::from_secs(5);
        while renderer_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !renderer_alive(pid),
            "renderer {pid} survived browser crash at {phase}"
        );
    }
}
#[cfg(target_os = "linux")]
fn renderer_alive(pid: u32) -> bool {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    status
        .split(") ")
        .nth(1)
        .is_some_and(|tail| !tail.starts_with('Z'))
}
#[cfg(windows)]
fn renderer_alive(pid: u32) -> bool {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).contains(&format!(",\"{pid}\","))
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
                let _ = codec::decode::<SharedFrameReady>($bytes);
                let _ = codec::decode::<ReleaseFrame>($bytes);
                let _ = codec::decode::<AcquireFrame>($bytes);
                let _ = codec::decode::<AcquireResult>($bytes);
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
    check(
        FrameReady {
            sequence: 1,
            shared: None,
        },
        &oversized,
    );
    check(
        SharedFrameReady {
            generation: 1,
            width: 1,
            height: 1,
            stride: 4,
            format: 1,
            byte_len: 4,
            handle_id: 1,
        },
        &oversized,
    );
    check(ReleaseFrame { generation: 1 }, &oversized);
    check(AcquireFrame { generation: 1 }, &oversized);
    check(AcquireResult::Superseded(2), &oversized);
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
