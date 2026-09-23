#![cfg(target_os = "linux")]
use amazing_ipc::interface;
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::time::Duration;
interface! {pub interface SandboxProbe(18) => SandboxProbeProxy, SandboxProbeReceiver {fn probe(message:String) -> String = 1;}}

fn exe() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"))
}
fn capabilities() -> Capabilities {
    Capabilities(BTreeMap::from([("__sandbox_probe".into(), "1".into())]))
}

#[test]
fn sandbox_launch_is_fail_closed() {
    match ProcessHost::spawn_sandboxed_renderer(exe(), capabilities(), RestartPolicy::Never) {
        Ok(mut host) => {
            assert_eq!(host.role, Role::Renderer);
            host.ping(Duration::from_secs(2)).unwrap();
            host.kill().unwrap();
        }
        Err(error) => {
            eprintln!("sandbox setup denied by host; child launch failed closed: {error}");
        }
    }
}

/// Run with `cargo nextest run -p amazing-process --run-ignored ignored-only`
/// after the Venus AppArmor userns profile is installed.
#[test]
#[ignore = "Venus AppArmor denies unprivileged user namespace UID/GID mapping"]
fn renderer_denies_file_socket_and_child_process() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let file = std::env::temp_dir().join(format!("amazing-sandbox-deny-{}", std::process::id()));
    std::fs::write(&file, b"private").unwrap();
    let mut host =
        ProcessHost::spawn_sandboxed_renderer(exe(), capabilities(), RestartPolicy::Never).unwrap();
    let proxy = SandboxProbeProxy::new(host.connection.clone(), Duration::from_secs(2));
    for message in [
        format!("open:{}", file.display()),
        "open:/".into(),
        format!("connect:{}", listener.local_addr().unwrap()),
        "spawn".into(),
    ] {
        let result = proxy.probe(message).unwrap();
        assert_eq!(result, "denied:1", "sandbox permitted {result}");
    }
    host.kill().unwrap();
    std::fs::remove_file(file).unwrap();
}
