//! Dynamic WPT `.py` resources answered by the std-only fixture server.
//! Test tooling only; engines never see this. Add a row to `DYNAMIC_RESOURCES`
//! to serve another pinned wptserve resource.
use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

type Handler = fn(&mut TcpStream, &str);

/// (path suffix, handler). The pinned file must exist under the WPT root.
const DYNAMIC_RESOURCES: &[(&str, Handler)] = &[("/resources/trickle.py", respond_trickle_py)];

/// Returns the handler and the repository-relative path of the pinned source
/// whose existence authorizes serving it.
pub(super) fn lookup(route: &str) -> Option<(Handler, &str)> {
    let route = route.strip_prefix('/')?;
    DYNAMIC_RESOURCES
        .iter()
        .find(|(suffix, _)| route.ends_with(suffix.trim_start_matches('/')) && route.len() >= suffix.len() - 1)
        .map(|(_, handler)| (*handler, route))
}

/// wptserve `resources/trickle.py` semantics: `count` (default 50) chunks of
/// "TEST_TRICKLE\n", `ms` (default 500) milliseconds between them,
/// text/plain, no Content-Length (the body ends with the connection).
fn respond_trickle_py(stream: &mut TcpStream, query: &str) {
    let (mut delay_ms, mut count) = (500u64, 50u64);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match (key, value.parse::<u64>()) {
            ("ms", Ok(value)) => delay_ms = value,
            ("count", Ok(value)) => count = value,
            _ => {}
        }
    }
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
    );
    let _ = stream.flush();
    for _ in 0..count {
        if stream.write_all(b"TEST_TRICKLE\n").is_err() || stream.flush().is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trickle_route_is_recognized_in_any_directory() {
        assert!(lookup("/xhr/resources/trickle.py").is_some());
        assert!(lookup("/fetch/api/resources/trickle.py").is_some());
        assert!(lookup("/xhr/resources/other.py").is_none());
    }
}
