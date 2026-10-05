#![forbid(unsafe_code)]

//! Isolated HTTPS transport for `amazing-net`.
//!
//! This crate is the Phase 5 vetted-TLS exception. It owns every rustls type
//! and exposes a provider-neutral negotiated stream plus a legacy bounded
//! HTTP/1.1 exchange. Certificate verification is always enabled; default
//! instances use Mozilla's compiled roots and explicit instance policies may
//! supply their own trust anchors. The network core owns the HTTP/2 protocol; ALPN negotiates
//! h2 preferentially and retains HTTP/1.1 fallback.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

static CLIENT_CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();

#[derive(Debug)]
pub enum TlsError {
    InvalidServerName,
    Certificate(CertificateError),
    NoApplicationProtocol,
    Transport(String),
    ResponseTooLarge,
}

/// Stable browser-facing rejection reasons, independent of the TLS provider.
/// These report the verifier's decision; they never allow overriding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CertificateError {
    Expired,
    NotValidYet,
    HostnameMismatch,
    UnknownIssuer,
    Revoked,
    InvalidPurpose,
    BadSignature,
    UnsupportedSignatureAlgorithm,
    BadEncoding,
    UnhandledCriticalExtension,
    UnknownRevocationStatus,
    InvalidOcspResponse,
    Missing,
    Other,
}

impl std::fmt::Display for CertificateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Expired => "expired",
            Self::NotValidYet => "not valid yet",
            Self::HostnameMismatch => "hostname mismatch",
            Self::UnknownIssuer => "unknown issuer",
            Self::Revoked => "revoked",
            Self::InvalidPurpose => "invalid server authentication purpose",
            Self::BadSignature => "invalid signature",
            Self::UnsupportedSignatureAlgorithm => "unsupported signature algorithm",
            Self::BadEncoding => "invalid encoding",
            Self::UnhandledCriticalExtension => "unhandled critical extension",
            Self::UnknownRevocationStatus => "unknown revocation status",
            Self::InvalidOcspResponse => "invalid OCSP response",
            Self::Missing => "missing peer certificate",
            Self::Other => "verification failed",
        })
    }
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidServerName => formatter.write_str("invalid TLS server name"),
            Self::Certificate(reason) => write!(formatter, "TLS certificate error: {reason}"),
            Self::NoApplicationProtocol => {
                formatter.write_str("TLS has no common application protocol")
            }
            Self::Transport(error) => write!(formatter, "TLS transport error: {error}"),
            Self::ResponseTooLarge => formatter.write_str("TLS response exceeded the size cap"),
        }
    }
}

impl std::error::Error for CertificateError {}

impl std::error::Error for TlsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Certificate(reason) => Some(reason),
            _ => None,
        }
    }
}

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

/// Immutable per-instance trust policy. Every handshake still uses the vetted
/// provider's normal chain, name, time, signature and ALPN verification.
/// Explicit anchors replace the default roots for this policy only.
#[derive(Clone)]
pub struct ClientPolicy {
    config: Arc<ClientConfig>,
}
impl Default for ClientPolicy {
    fn default() -> Self {
        Self {
            config: client_config(),
        }
    }
}
impl ClientPolicy {
    pub fn from_der_trust_anchors(
        anchors: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<Self, TlsError> {
        let mut roots = RootCertStore::empty();
        for anchor in anchors {
            roots
                .add(CertificateDer::from(anchor))
                .map_err(|e| TlsError::Transport(e.to_string()))?;
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring supports safe protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(Self {
            config: Arc::new(config),
        })
    }
    fn config(&self, http1: bool) -> Arc<ClientConfig> {
        if !http1 {
            return self.config.clone();
        }
        let mut config = (*self.config).clone();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(config)
    }
}

/// Result and actual encrypted socket progress for one bounded operation.
/// Socket bytes include reads and writes, including progress before an error.
/// output_drained does not mean the peer has acknowledged or consumed data.
pub struct IoStep<T, E = std::io::Error> {
    pub result: Result<T, E>,
    pub socket_bytes: usize,
    pub output_drained: bool,
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
    /// Whether every accepted plaintext byte has reached the socket transport.
    /// This is transport drainage, not acknowledgement by the remote peer.
    pub fn output_drained(&self) -> bool {
        !self.inner.conn.wants_write()
    }

    /// Compatibility wrapper; cooperative consumers should use the metered API.
    pub fn write_step(&mut self, bytes: &[u8], io_budget: usize) -> std::io::Result<usize> {
        self.write_metered(bytes, io_budget).result
    }
    pub fn read_step(&mut self, bytes: &mut [u8], io_budget: usize) -> std::io::Result<usize> {
        self.read_metered(bytes, io_budget).result
    }

    /// Accepts plaintext and advances encrypted I/O. Socket progress is retained
    /// even when the result is WouldBlock or another error.
    pub fn write_metered(&mut self, bytes: &[u8], budget: usize) -> IoStep<usize> {
        let mut left = budget;
        let result = (|| {
            if budget == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let n = self
                .inner
                .conn
                .writer()
                .write(&bytes[..bytes.len().min(budget)])?;
            self.drive_io(&mut left)?;
            if n == 0 && !bytes.is_empty() {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            Ok(n)
        })();
        IoStep {
            result,
            socket_bytes: budget - left,
            output_drained: self.output_drained(),
        }
    }

    pub fn read_metered(&mut self, bytes: &mut [u8], budget: usize) -> IoStep<usize> {
        let mut left = budget;
        let result = (|| {
            if bytes.is_empty() {
                return Ok(0);
            }
            if budget == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let limit = bytes.len().min(budget);
            match self.inner.conn.reader().read(&mut bytes[..limit]) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            self.drive_io(&mut left)?;
            self.inner.conn.reader().read(&mut bytes[..limit])
        })();
        IoStep {
            result,
            socket_bytes: budget - left,
            output_drained: self.output_drained(),
        }
    }

    fn drive_io(&mut self, left: &mut usize) -> std::io::Result<()> {
        let mut io = BudgetIo {
            socket: &mut self.inner.sock,
            left: *left,
        };
        let result = self.inner.conn.complete_io(&mut io);
        *left = io.left;
        match result {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Writes queued ciphertext only, within the socket budget. Never reads or
    /// accepts additional plaintext. Ok(false) requires a later drain step.
    pub fn flush_metered(&mut self, budget: usize) -> IoStep<bool> {
        let mut io = BudgetIo {
            socket: &mut self.inner.sock,
            left: budget,
        };
        let result = (|| {
            while self.inner.conn.wants_write() {
                match self.inner.conn.write_tls(&mut io) {
                    Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                    Err(e) => return Err(e),
                }
            }
            Ok(true)
        })();
        let socket_bytes = budget - io.left;
        IoStep {
            result,
            socket_bytes,
            output_drained: self.output_drained(),
        }
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

/// Retained client handshake for a caller-owned, already connected socket.
/// The socket may be non-blocking. `WouldBlock` preserves all TLS state.
pub struct PendingTls {
    inner: Option<StreamOwned<ClientConnection, TcpStream>>,
}

impl PendingTls {
    pub fn new_http1(host: &str, socket: TcpStream) -> Result<Self, TlsError> {
        Self::with_config(host, socket, legacy_config())
    }

    pub fn new(host: &str, socket: TcpStream) -> Result<Self, TlsError> {
        Self::with_config(host, socket, client_config())
    }

    pub fn new_with_policy(
        host: &str,
        socket: TcpStream,
        policy: &ClientPolicy,
    ) -> Result<Self, TlsError> {
        Self::with_config(host, socket, policy.config(false))
    }
    pub fn new_http1_with_policy(
        host: &str,
        socket: TcpStream,
        policy: &ClientPolicy,
    ) -> Result<Self, TlsError> {
        Self::with_config(host, socket, policy.config(true))
    }

    fn with_config(
        host: &str,
        socket: TcpStream,
        config: Arc<ClientConfig>,
    ) -> Result<Self, TlsError> {
        let server_name =
            ServerName::try_from(host.to_owned()).map_err(|_| TlsError::InvalidServerName)?;
        let connection = ClientConnection::new(config, server_name)
            .map_err(|error| TlsError::Transport(error.to_string()))?;
        Ok(Self {
            inner: Some(StreamOwned::new(connection, socket)),
        })
    }

    /// Performs at most `max_io_bytes` of socket I/O. `Ok(None)` means the
    /// handshake needs another step; cancellation is dropping this value.
    /// A fatal error closes the socket and consumes the handshake. Retrying
    /// cannot publish a stream from a connection whose authentication failed.
    pub fn step(&mut self, max_io_bytes: usize) -> Result<Option<TlsStream>, TlsError> {
        self.step_metered(max_io_bytes).result
    }
    pub fn step_metered(&mut self, budget: usize) -> IoStep<Option<TlsStream>, TlsError> {
        let mut left = budget;
        let result = self.step_inner(budget, &mut left);
        let output_drained = self
            .inner
            .as_ref()
            .is_none_or(|inner| !inner.conn.wants_write());
        IoStep {
            result,
            socket_bytes: budget - left,
            output_drained,
        }
    }
    fn step_inner(
        &mut self,
        max_io_bytes: usize,
        left: &mut usize,
    ) -> Result<Option<TlsStream>, TlsError> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| TlsError::Transport("TLS handshake already completed".into()))?;
        if max_io_bytes == 0 {
            return Ok(None);
        }
        let mut io = BudgetIo {
            socket: &mut inner.sock,
            left: max_io_bytes,
        };
        let result = inner.conn.complete_io(&mut io);
        *left = io.left;
        match result {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                self.inner = None;
                return Err(transport_error(error));
            }
        }
        if inner.conn.is_handshaking() || inner.conn.wants_write() {
            return Ok(None);
        }
        let protocol = match inner.conn.alpn_protocol() {
            Some(b"h2") => Protocol::H2,
            Some(b"http/1.1") | None => Protocol::Http11,
            _ => {
                self.inner = None;
                return Err(TlsError::NoApplicationProtocol);
            }
        };
        Ok(Some(TlsStream {
            inner: self.inner.take().expect("handshake retained"),
            protocol,
        }))
    }
}

struct BudgetIo<'a> {
    socket: &'a mut TcpStream,
    left: usize,
}
impl Read for BudgetIo<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let limit = buf.len().min(self.left);
        let n = self.socket.read(&mut buf[..limit])?;
        self.left -= n;
        Ok(n)
    }
}
impl Write for BudgetIo<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let n = self.socket.write(&buf[..buf.len().min(self.left)])?;
        self.left -= n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
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
    if let Some(tls) = error
        .get_ref()
        .and_then(|e| e.downcast_ref::<rustls::Error>())
    {
        match tls {
            rustls::Error::InvalidCertificate(reason) => {
                return TlsError::Certificate(certificate_error(reason));
            }
            rustls::Error::NoCertificatesPresented => {
                return TlsError::Certificate(CertificateError::Missing);
            }
            rustls::Error::NoApplicationProtocol
            | rustls::Error::AlertReceived(rustls::AlertDescription::NoApplicationProtocol) => {
                return TlsError::NoApplicationProtocol;
            }
            _ => {}
        }
    }
    TlsError::Transport(error.to_string())
}

#[allow(deprecated)] // The pinned provider can still produce the legacy variants.
fn certificate_error(error: &rustls::CertificateError) -> CertificateError {
    use rustls::CertificateError as Provider;
    match error {
        Provider::Expired | Provider::ExpiredContext { .. } => CertificateError::Expired,
        Provider::NotValidYet | Provider::NotValidYetContext { .. } => {
            CertificateError::NotValidYet
        }
        Provider::NotValidForName | Provider::NotValidForNameContext { .. } => {
            CertificateError::HostnameMismatch
        }
        Provider::UnknownIssuer => CertificateError::UnknownIssuer,
        Provider::Revoked => CertificateError::Revoked,
        Provider::InvalidPurpose | Provider::InvalidPurposeContext { .. } => {
            CertificateError::InvalidPurpose
        }
        Provider::BadSignature => CertificateError::BadSignature,
        Provider::UnsupportedSignatureAlgorithm
        | Provider::UnsupportedSignatureAlgorithmContext { .. }
        | Provider::UnsupportedSignatureAlgorithmForPublicKeyContext { .. } => {
            CertificateError::UnsupportedSignatureAlgorithm
        }
        Provider::BadEncoding => CertificateError::BadEncoding,
        Provider::UnhandledCriticalExtension => CertificateError::UnhandledCriticalExtension,
        Provider::UnknownRevocationStatus
        | Provider::ExpiredRevocationList
        | Provider::ExpiredRevocationListContext { .. } => {
            CertificateError::UnknownRevocationStatus
        }
        Provider::InvalidOcspResponse => CertificateError::InvalidOcspResponse,
        _ => CertificateError::Other,
    }
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
    ServerName::try_from(host.to_owned()).map_err(|_| TlsError::InvalidServerName)?;
    let tcp = TcpStream::connect((host, port)).map_err(transport_error)?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(transport_error)?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(transport_error)?;

    let mut pending = PendingTls::with_config(host, tcp, config)?;
    loop {
        if let Some(stream) = pending.step(64 * 1024)? {
            return Ok(stream);
        }
    }
}
#[cfg(feature = "test-support")]
pub mod test_support;

#[cfg(all(test, feature = "test-support"))]
mod validation_tests;
#[cfg(all(test, feature = "test-support"))]
mod metered_io_tests;
#[cfg(all(test, feature = "test-support"))]
mod write_pressure_tests;
