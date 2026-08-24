#![forbid(unsafe_code)]

//! Isolated HTTPS transport for `amazing-net`.
//!
//! This crate is the Phase 5 vetted-TLS exception. It owns every rustls type
//! and exposes one bounded, blocking HTTP/1.1 byte exchange to the std-only
//! network core. Certificate verification is always enabled with Mozilla's
//! compiled root set; ALPN advertises only HTTP/1.1 because Amazing does not
//! yet implement HTTP/2 framing.

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
    let server_name =
        ServerName::try_from(host.to_owned()).map_err(|_| TlsError::InvalidServerName)?;
    let tcp = TcpStream::connect((host, port)).map_err(transport_error)?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(transport_error)?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(transport_error)?;

    let connection = ClientConnection::new(client_config(), server_name)
        .map_err(|error| TlsError::Transport(error.to_string()))?;
    let mut stream = StreamOwned::new(connection, tcp);
    stream.write_all(request).map_err(transport_error)?;
    stream.flush().map_err(transport_error)?;

    let mut response = Vec::with_capacity(16 * 1024);
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
                if error.kind() == std::io::ErrorKind::UnexpectedEof && !response.is_empty() =>
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
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
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
    fn client_config_advertises_only_http_1_1() {
        assert_eq!(client_config().alpn_protocols, vec![b"http/1.1".to_vec()]);
    }
}
