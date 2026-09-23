//! Opt-in authenticated localhost fixtures. Never affects connect()/Mozilla roots.
use super::*;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use std::net::TcpListener;
const CA: &[u8] = include_bytes!("../tests/fixtures/ca.der");
const CERT: &[u8] = include_bytes!("../tests/fixtures/server.der");
const KEY: &[u8] = include_bytes!("../tests/fixtures/server-key.der");
pub struct ServerStream(StreamOwned<ServerConnection, TcpStream>);
impl Read for ServerStream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}
impl Write for ServerStream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl ServerStream {
    pub fn protocol(&self) -> Option<&[u8]> {
        self.0.conn.alpn_protocol()
    }
}
pub fn serve<F>(
    protocols: Vec<Vec<u8>>,
    handler: F,
) -> (u16, std::thread::JoinHandle<Result<(), TlsError>>)
where
    F: FnOnce(&mut ServerStream) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT.to_vec())],
                PrivatePkcs8KeyDer::from(KEY.to_vec()).into(),
            )
            .unwrap();
    config.alpn_protocols = protocols;
    let thread = std::thread::spawn(move || {
        let (tcp, _) = listener.accept().map_err(transport_error)?;
        tcp.set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(transport_error)?;
        tcp.set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(transport_error)?;
        let conn = ServerConnection::new(Arc::new(config))
            .map_err(|e| TlsError::Transport(e.to_string()))?;
        let mut stream = ServerStream(StreamOwned::new(conn, tcp));
        while stream.0.conn.is_handshaking() {
            stream
                .0
                .conn
                .complete_io(&mut stream.0.sock)
                .map_err(transport_error)?;
        }
        handler(&mut stream);
        stream.0.conn.send_close_notify();
        let _ = stream.flush();
        Ok(())
    });
    (port, thread)
}
pub fn connect(port: u16) -> Result<TlsStream, TlsError> {
    connect_fixture(port, false)
}
pub fn connect_http1(port: u16) -> Result<TlsStream, TlsError> {
    connect_fixture(port, true)
}
fn connect_fixture(port: u16, http1: bool) -> Result<TlsStream, TlsError> {
    connect_config(
        "localhost",
        port,
        Duration::from_secs(10),
        fixture_config(http1),
    )
}
fn fixture_config(http1: bool) -> Arc<ClientConfig> {
    let mut config = (*(if http1 {
        legacy_config()
    } else {
        client_config()
    }))
    .clone();
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(CA.to_vec())).unwrap();
    // A new verified config, not a dangerous certificate-verification bypass.
    let replacement =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let protocols = std::mem::take(&mut config.alpn_protocols);
    config = replacement;
    config.alpn_protocols = protocols;
    Arc::new(config)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_handshake_retains_state_across_would_block() {
        let (port, thread) = serve(vec![b"http/1.1".to_vec()], |_| {});
        let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut pending =
            PendingTls::with_config("localhost", socket, fixture_config(true)).unwrap();
        assert!(pending.step(1).unwrap().is_none());
        let mut result = None;
        for _ in 0..100_000 {
            if let Some(stream) = pending.step(256).unwrap() {
                result = Some(stream);
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(
            result.expect("handshake progressed").protocol(),
            Protocol::Http11
        );
        thread.join().unwrap().unwrap();
    }
    #[test]
    fn alpn_h2_h1_and_absent_with_verified_fixture() {
        for (protocols, expected) in [
            (vec![b"h2".to_vec(), b"http/1.1".to_vec()], Protocol::H2),
            (vec![b"http/1.1".to_vec()], Protocol::Http11),
            (vec![], Protocol::Http11),
        ] {
            let (port, thread) = serve(protocols, |io| {
                let mut b = [0];
                io.read_exact(&mut b).unwrap();
                io.write_all(&b).unwrap();
            });
            let mut stream = connect(port).unwrap();
            assert_eq!(stream.protocol(), expected);
            stream.write_all(b"x").unwrap();
            stream.flush().unwrap();
            let mut b = [0];
            stream.read_exact(&mut b).unwrap();
            assert_eq!(b, b"x"[..]);
            thread.join().unwrap().unwrap();
        }
    }
    #[test]
    fn production_roots_do_not_trust_the_test_ca() {
        let (port, thread) = serve(vec![b"h2".to_vec()], |_| {
            panic!("untrusted handshake accepted")
        });
        assert!(super::super::connect("localhost", port, Duration::from_secs(3)).is_err());
        assert!(thread.join().unwrap().is_err());
    }
}
