use amazing_ipc::interface;
use amazing_process::{
    Capabilities, ProcessHost, RendererAssignments, RestartPolicy, Role, SiteInstance,
};
use std::collections::BTreeMap;
use std::time::Duration;
interface! {pub interface Echo(7) => EchoProxy, EchoReceiver {fn echo(message:String) -> String = 1;}}
#[test]
fn child_roundtrip_crash_and_restart() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let mut host = ProcessHost::spawn(
        exe,
        Role::Renderer,
        Capabilities(BTreeMap::new()),
        RestartPolicy::OnCrash { max_restarts: 1 },
    )
    .unwrap();
    assert_eq!(
        host.capabilities.0.get("echo").map(String::as_str),
        Some("1")
    );
    host.ping(Duration::from_secs(2)).unwrap();
    let proxy = EchoProxy::new(host.connection.clone(), Duration::from_secs(2));
    assert_eq!(proxy.echo("hello".into()).unwrap(), "echo:hello");
    host.kill().unwrap();
    assert!(host.crashed().unwrap());
    assert!(host
        .restart_if_crashed(exe, Capabilities(BTreeMap::new()))
        .unwrap());
    assert_eq!(
        EchoProxy::new(host.connection.clone(), Duration::from_secs(2))
            .echo("again".into())
            .unwrap(),
        "echo:again"
    );
    host.kill().unwrap();
}
#[test]
fn site_assignment() {
    let mut a = RendererAssignments::new(2);
    let x = SiteInstance::parse("https://www.example.com/a").unwrap();
    let y = SiteInstance::parse("https://shop.example.com/").unwrap();
    let z = SiteInstance::parse("http://www.example.com/").unwrap();
    assert_eq!(a.assign(x), a.assign(y));
    assert_ne!(
        a.assign(SiteInstance::parse("https://other.test/").unwrap()),
        a.assign(z.clone())
    );
    assert_eq!(a.assign(z.clone()), a.assign(z));
}
