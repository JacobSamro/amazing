use amazing_ipc::interface;
use amazing_process::{bootstrap_child, Capabilities, Ping, Role};
use std::collections::BTreeMap;
interface! {pub interface Echo(17) => EchoProxy, EchoReceiver {fn echo(message:String) -> String = 1;}}
struct EchoHandler;
impl Echo for EchoHandler {
    fn echo(&self, message: String) -> Result<String, amazing_ipc::Error> {
        Ok(format!("echo:{message}"))
    }
}
fn main() {
    let Some(role) = std::env::args().find_map(|s| s.strip_prefix("--type=").and_then(Role::parse))
    else {
        return;
    };
    let caps = Capabilities(BTreeMap::from([
        ("echo".into(), "1".into()),
        ("renderer".into(), "1".into()),
        ("network".into(), "1".into()),
    ]));
    let Ok((conn, _)) = bootstrap_child(role, caps) else {
        return;
    };
    match role {
        Role::Renderer => amazing_process::renderer::serve_renderer(&conn),
        Role::Network => amazing_process::network::serve_network(&conn),
        _ => {
            conn.bind(0, Ping);
            conn.bind(17, EchoReceiver(EchoHandler));
        }
    }
    while !conn.is_closed() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
