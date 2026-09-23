//! Run the selected std-only WPT resource shim for installed-runner diagnostics.
#[path = "../tests/support/wpt_server.rs"]
mod wpt_server;

fn main() {
    let server = wpt_server::WptServer::new();
    println!("{}", server.url(""));
    if std::env::args().any(|arg| arg == "--serve") {
        loop { std::thread::sleep(std::time::Duration::from_secs(60)); }
    } else {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
}
