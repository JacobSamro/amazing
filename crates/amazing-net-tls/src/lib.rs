#![forbid(unsafe_code)]

//! Isolated HTTPS transport for `amazing-net`.
//!
//! This crate is the Phase 5 vetted-TLS exception. It owns every rustls type
//! and exposes a provider-neutral negotiated stream plus a legacy bounded
//! HTTP/1.1 exchange. Certificate verification is always enabled with Mozilla's
//! compiled root set. The network core owns the HTTP/2 protocol; ALPN negotiates
//! h2 preferentially and retains HTTP/1.1 fallback.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

static CLIENT_CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();

#[derive(Debug)]
pub enum TlsError {
    InvalidServerName,
    Transport(String),
    ResponseTooLarge,
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidServerName => formatter.write_str("invalid TLS server name"),
            Self::Transport(error) => write!(formatter, "TLS transport error: {error}"),
            Self::ResponseTooLarge => formatter.write_str("TLS response exceeded the size cap"),
        }
    }
}

impl std::error::Error for TlsError {}

#[derive(Debug)]
pub struct TlsResponse {
    pub bytes: Vec<u8>,
    pub peer_closed_cleanly: bool,
}

pub fn exchange(
    host: &str,
    port: u16,
    request: &[u8],
    timeout: Duration,
    max_response_bytes: usize,
) -> Result<TlsResponse, TlsError> {
    let stream = connect_config(host, port, timeout, legacy_config())?;
    stream.exchange_http1(request, max_response_bytes)
}

/// Provider-neutral ALPN result. Absence of ALPN selects HTTP/1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Http11,
    H2,
}

/// Opaque authenticated transport. Read/Write operate only on caller buffers;
/// framing, stream state, and all HTTP allocation policy stay in amazing-net.
pub struct TlsStream {
    inner: StreamOwned<ClientConnection, TcpStream>,
    protocol: Protocol,
}
impl TlsStream {
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }
    pub fn exchange_http1(
        mut self,
        request: &[u8],
        max_response_bytes: usize,
    ) -> Result<TlsResponse, TlsError> {
        if self.protocol != Protocol::Http11 {
            return Err(TlsError::Transport(
                "HTTP/1 exchange on h2 connection".into(),
            ));
        }
        let stream = &mut self.inner;
        stream.write_all(request).map_err(transport_error)?;
        stream.flush().map_err(transport_error)?;

        let mut response = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = match stream.read(&mut buffer) {
                Ok(0) => {
                    return Ok(TlsResponse {
                        bytes: response,
                        peer_closed_cleanly: true,
                    });
                }
                Ok(count) => count,
                Err(error)
                    if error.kind() == std::io::ErrorKind::UnexpectedEof
                        && !response.is_empty() =>
                {
                    return Ok(TlsResponse {
                        bytes: response,
                        peer_closed_cleanly: false,
                    });
                }
                Err(error) => return Err(transport_error(error)),
            };
            if response.len().saturating_add(count) > max_response_bytes {
                return Err(TlsError::ResponseTooLarge);
            }
            response.extend_from_slice(&buffer[..count]);
        }
    }
}

fn client_config() -> Arc<ClientConfig> {
    CLIENT_CONFIG
        .get_or_init(|| {
            let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring supports rustls safe protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            Arc::new(config)
        })
        .clone()
}

fn transport_error(error: std::io::Error) -> TlsError {
    TlsError::Transport(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_server_names_fail_before_network_io() {
        let error =
            exchange("not a host name", 443, b"", Duration::from_millis(1), 1024).unwrap_err();
        assert!(matches!(error, TlsError::InvalidServerName));
    }

    #[test]
    fn legacy_config_advertises_only_http_1_1() {
        assert_eq!(legacy_config().alpn_protocols, vec![b"http/1.1".to_vec()]);
    }
}

impl Read for TlsStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(bytes)
    }
}
impl Write for TlsStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
fn legacy_config() -> Arc<ClientConfig> {
    let mut config = (*client_config()).clone();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}
/// HTTP/1.1-only authenticated stream for RFC6455 upgrades.
pub fn connect_http1(host: &str, port: u16, timeout: Duration) -> Result<TlsStream, TlsError> {
    connect_config(host, port, timeout, legacy_config())
}
/// Fill caller-owned bytes from the existing vetted provider's OS entropy.
/// No provider type or random-state handle crosses this boundary.
pub fn fill_random(bytes: &mut [u8]) -> Result<(), TlsError> {
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(bytes)
        .map_err(|_| TlsError::Transport("secure entropy unavailable".into()))
}
pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<TlsStream, TlsError> {
    connect_config(host, port, timeout, client_config())
}
fn connect_config(
    host: &str,
    port: u16,
    timeout: Duration,
    config: Arc<ClientConfig>,
) -> Result<TlsStream, TlsError> {
    let server_name =
        ServerName::try_from(host.to_owned()).map_err(|_| TlsError::InvalidServerName)?;
    let tcp = TcpStream::connect((host, port)).map_err(transport_error)?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(transport_error)?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(transport_error)?;

    let connection = ClientConnection::new(config, server_name)
        .map_err(|error| TlsError::Transport(error.to_string()))?;
    let mut stream = StreamOwned::new(connection, tcp);

    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(transport_error)?;
    }
    let protocol = match stream.conn.alpn_protocol() {
        Some(b"h2") => Protocol::H2,
        Some(b"http/1.1") | None => Protocol::Http11,
        _ => return Err(TlsError::Transport("unsupported ALPN".into())),
    };
    Ok(TlsStream {
        inner: stream,
        protocol,
    })
}
#[cfg(feature = "test-support")]
pub mod test_support;
