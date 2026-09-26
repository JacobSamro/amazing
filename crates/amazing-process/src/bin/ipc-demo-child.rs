use amazing_ipc::codec::ByteBuf;
use amazing_ipc::interface;
use amazing_process::protocol::{
    AcquireFrame, AcquireResult, BrowserEvents, BrowserEventsReceiver, FrameReady, Navigate,
    RendererProxy, Viewport,
};
use amazing_process::{bootstrap_child, Capabilities, Role};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;
interface! {pub interface Echo(17) => EchoProxy, EchoReceiver {fn echo(message:String) -> String = 1;}}
interface! {pub interface SandboxProbe(18) => SandboxProbeProxy, SandboxProbeReceiver {fn probe(message:String) -> String = 1;}}
struct EchoHandler;
struct SandboxProbeHandler;
impl SandboxProbe for SandboxProbeHandler {
    fn probe(&self, message: String) -> Result<String, amazing_ipc::Error> {
        let result = if let Some(path) = message.strip_prefix("open:") {
            std::fs::File::open(path).map(|_| ())
        } else if let Some(address) = message.strip_prefix("connect:") {
            let address = address
                .parse()
                .map_err(|_| amazing_ipc::Error::Remote("address".into()))?;
            std::net::TcpStream::connect_timeout(&address, Duration::from_millis(300)).map(|_| ())
        } else if let Some(pid) = message.strip_prefix("open-process:") {
            #[cfg(windows)]
            {
                let pid = pid
                    .parse()
                    .map_err(|_| amazing_ipc::Error::Remote("pid".into()))?;
                amazing_ipc_platform::probe_open_process(pid)
            }
            #[cfg(not(windows))]
            {
                let _ = pid;
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Windows only",
                ))
            }
        } else if message == "spawn" {
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--probe-noop")
                .status()
                .map(|_| ())
        } else {
            return Err(amazing_ipc::Error::Remote("unknown probe".into()));
        };
        Ok(match result {
            Ok(()) => "allowed".into(),
            Err(error) => format!("denied:{}", error.raw_os_error().unwrap_or(0)),
        })
    }
}
struct ProbeEvents;
impl BrowserEvents for ProbeEvents {
    fn frame_ready(&self, _: FrameReady) -> Result<(), amazing_ipc::Error> {
        Ok(())
    }
}
fn browser_crash_probe(phase: &str) {
    let exe = std::env::current_exe().unwrap();
    let renderer = amazing_process::ProcessHost::spawn(
        &exe,
        Role::Renderer,
        Capabilities(BTreeMap::new()),
        amazing_process::RestartPolicy::Never,
    )
    .unwrap();
    renderer
        .connection
        .bind(9, BrowserEventsReceiver(ProbeEvents));
    if phase != "spawned" {
        let proxy = RendererProxy::new(renderer.connection.clone(), Duration::from_secs(5));
        let ready = proxy
            .navigate_shared(Navigate {
                url: "http://example.test/crash".into(),
                html_bytes: ByteBuf(b"<p>crash</p>".to_vec()),
                viewport: Viewport {
                    width: 64,
                    height: 64,
                },
            })
            .unwrap();
        if phase == "acquired" {
            assert!(matches!(
                proxy
                    .acquire_frame(AcquireFrame {
                        generation: ready.generation
                    })
                    .unwrap(),
                AcquireResult::Transferred(())
            ));
            assert!(matches!(
                proxy
                    .acquire_frame(AcquireFrame {
                        generation: ready.generation
                    })
                    .unwrap(),
                AcquireResult::Busy(())
            ));
        } else if phase == "mapped" {
            let _view = renderer.receive_frame(&ready).unwrap();
        }
    }
    println!("{}", renderer.pid());
    std::io::stdout().flush().unwrap();
    std::process::exit(77);
}
impl Echo for EchoHandler {
    fn echo(&self, message: String) -> Result<String, amazing_ipc::Error> {
        Ok(format!("echo:{message}"))
    }
}
fn main() {
    if std::env::args().any(|arg| arg == "--probe-noop") {
        return;
    }
    if let Some(phase) =
        std::env::args().find_map(|s| s.strip_prefix("--browser-crash-probe=").map(str::to_owned))
    {
        browser_crash_probe(&phase);
    }
    let Some(role) = std::env::args().find_map(|s| s.strip_prefix("--type=").and_then(Role::parse))
    else {
        return;
    };
    if role == Role::Network {
        amazing_process::network::run_network_child().unwrap();
        return;
    }
    let caps = Capabilities(BTreeMap::from([
        ("echo".into(), "1".into()),
        ("renderer".into(), "1".into()),
        ("network".into(), "1".into()),
    ]));
    let Ok(conn) = bootstrap_child(role, caps, |conn, parent, channel| {
        if let Some(delay) = parent
            .0
            .get("__bind_delay_ms")
            .and_then(|value| value.parse::<u64>().ok())
        {
            std::thread::sleep(Duration::from_millis(delay.min(1000)));
        }
        if parent
            .0
            .get("__sandbox_probe")
            .is_some_and(|value| value == "1")
        {
            conn.bind(18, SandboxProbeReceiver(SandboxProbeHandler));
        }
        // IPC-11c follow-up: a real JS realm for the postMessage/MessagePort
        // cross-process proof (`amazing_process::demo_realm`), instead of the
        // production byte-frame renderer path this role would otherwise run.
        if parent
            .0
            .get("__js_realm")
            .is_some_and(|value| value == "1")
        {
            amazing_process::demo_realm::serve_demo_realm(conn);
            return Ok(());
        }
        match role {
            Role::Renderer => amazing_process::renderer::serve_renderer(conn, channel),
            Role::Gpu => amazing_process::gpu::serve_gpu(conn),
            Role::Network => amazing_process::network::serve_network(conn),
            Role::Utility => {
                conn.bind(17, EchoReceiver(EchoHandler));
                amazing_process::utility::serve_utility(conn, channel);
            }
            Role::Storage => {
                let root = parent
                    .0
                    .get("__storage_root")
                    .ok_or(amazing_process::Error::Role)?;
                let quota = parent
                    .0
                    .get("__storage_quota")
                    .and_then(|s| s.parse().ok())
                    .ok_or(amazing_process::Error::Role)?;
                amazing_process::storage::serve_storage(conn, std::path::Path::new(root), quota)?;
            }
            Role::Browser => return Err(amazing_process::Error::Role),
        }
        Ok(())
    }) else {
        return;
    };
    while !conn.is_closed() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
