#![cfg(windows)]
use amazing_ipc::interface;
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::time::Duration;
interface! {pub interface SandboxProbe(18) => SandboxProbeProxy, SandboxProbeReceiver {fn probe(message:String) -> String = 1;}}

#[test]
fn renderer_appcontainer_denies_file_network_child_and_browser_process() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let file = std::env::temp_dir().join(format!("amazing-sandbox-private-{}", std::process::id()));
    std::fs::write(&file, b"private").unwrap();
    std::fs::File::open(&file).unwrap();
    let sanity = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let _ = listener.accept().unwrap();
    drop(sanity);
    amazing_ipc_platform::probe_open_process(std::process::id()).unwrap();
    assert!(std::process::Command::new(env!("CARGO_BIN_EXE_ipc-demo-child"))
        .arg("--probe-noop").status().unwrap().success());
    let mut host = ProcessHost::spawn_sandboxed_renderer(
        std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child")),
        Capabilities(BTreeMap::from([("__sandbox_probe".into(), "1".into())])),
        RestartPolicy::Never,
    )
    .unwrap();
    assert_eq!(host.role, Role::Renderer);
    host.ping(Duration::from_secs(2)).unwrap();
    let proxy = SandboxProbeProxy::new(host.connection.clone(), Duration::from_secs(5));
    for probe in [
        format!("open:{}", file.display()),
        format!("connect:{}", listener.local_addr().unwrap()),
        "spawn".into(),
        format!("open-process:{}", std::process::id()),
    ] {
        let result = proxy
            .probe(probe.clone())
            .unwrap_or_else(|error| panic!("{probe}: {error:?}"));
        assert!(
            result.starts_with("denied:"),
            "{probe} unexpectedly returned {result}"
        );
    }
    host.kill().unwrap();
    std::fs::remove_file(file).unwrap();
}
