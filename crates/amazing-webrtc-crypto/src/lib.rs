//! Vetted DTLS-SRTP infrastructure adapter for Phase 9.
//!
//! Amazing's protocol crate remains safe and `std`-only. Security-critical
//! certificate generation, DTLS 1.2, key export, and SRTP/SRTCP packet
//! protection are delegated here to the maintained WebRTC.rs, RustCrypto,
//! rustls, and ring implementations. This is the documented TLS exception;
//! the adapter contains no local cryptographic primitive or unsafe code.

#![forbid(unsafe_code)]

use std::fmt;
use std::fmt::Write as _;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use tokio::net::UdpSocket;
use tokio::runtime::Builder;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use webrtc_dtls::cipher_suite::CipherSuiteId;
use webrtc_dtls::config::{ClientAuthType, Config, ExtendedMasterSecretType};
use webrtc_dtls::conn::DTLSConn;
use webrtc_dtls::crypto::Certificate;
use webrtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc_dtls::signature_hash_algorithm::SignatureScheme;
use webrtc_srtp::context::Context;
use webrtc_srtp::option::{srtcp_replay_protection, srtp_replay_protection};
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::{Conn, KeyingMaterialExporter};
use zeroize::Zeroizing;

const DTLS_SRTP_EXPORTER_LABEL: &str = "EXTRACTOR-dtls_srtp";
const DTLS_CIPHER: CipherSuiteId = CipherSuiteId::Tls_Ecdhe_Ecdsa_With_Aes_128_Gcm_Sha256;
const DTLS_SRTP_PROFILE: SrtpProtectionProfile = SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm;
const SRTP_PROFILE: ProtectionProfile = ProtectionProfile::AeadAes128Gcm;
const MASTER_KEY_LEN: usize = 16;
const MASTER_SALT_LEN: usize = 12;
const EXPORTED_KEYING_MATERIAL_LEN: usize = 2 * (MASTER_KEY_LEN + MASTER_SALT_LEN);
const REPLAY_WINDOW: usize = 64;
const UDP_TIMEOUT: Duration = Duration::from_secs(2);
const CONSENT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(7);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureLoopbackReport {
    pub ice_state: &'static str,
    pub ice_credentials_bound: bool,
    pub udp_client_address: SocketAddr,
    pub udp_server_address: SocketAddr,
    pub sdp_identity_verified: bool,
    pub client_setup: &'static str,
    pub server_setup: &'static str,
    pub dtls_version: &'static str,
    pub cipher_suite: &'static str,
    pub srtp_profile: &'static str,
    pub exported_keying_material_bytes: usize,
    pub rtp_payload_bytes: usize,
    pub protected_rtp_bytes: usize,
    pub rtcp_packet_count: u32,
    pub protected_rtcp_bytes: usize,
    pub fingerprints_verified: bool,
    pub srtp_tamper_rejected: bool,
    pub srtp_replay_rejected: bool,
    pub srtcp_tamper_rejected: bool,
    pub srtcp_replay_rejected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureMediaError(String);

impl SecureMediaError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SecureMediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SecureMediaError {}

/// An opaque ephemeral WebRTC certificate identity.
///
/// Provider certificate/key types remain private; callers can signal only the
/// SHA-256 fingerprint before consuming the identity in a handshake adapter.
pub struct SecureIdentity {
    certificate: Certificate,
    fingerprint: String,
}

impl SecureIdentity {
    pub fn generate(subject: &str) -> Result<Self, SecureMediaError> {
        let certificate = Certificate::generate_self_signed(vec![subject.into()])
            .map_err(|error| SecureMediaError::new(format!("{subject} certificate: {error}")))?;
        let fingerprint = certificate_fingerprint(&certificate)?;
        Ok(Self {
            certificate,
            fingerprint,
        })
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

/// Directional secure-media state with provider details hidden at the seam.
struct SecureMediaSession {
    outgoing: Context,
    incoming: Context,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    Controlling,
    Controlled,
}

/// Inputs for establishing one endpoint on an already-connected UDP socket.
pub struct SecureEndpointConfig {
    pub socket: std::net::UdpSocket,
    pub identity: SecureIdentity,
    pub local_description: amazing_webrtc::SdpDescription,
    pub remote_description: amazing_webrtc::SdpDescription,
    pub local_is_offerer: bool,
    pub ice_role: IceRole,
}

/// A caller-constructible single-endpoint DTLS-SRTP association.
pub struct SecureUdpEndpoint {
    runtime: tokio::runtime::Runtime,
    socket: Arc<UdpSocket>,
    dtls: Option<DTLSConn>,
    demux_task: JoinHandle<()>,
    stun_rx: mpsc::Receiver<Vec<u8>>,
    media_rx: mpsc::Receiver<Vec<u8>>,
    session: SecureMediaSession,
    local_description: amazing_webrtc::SdpDescription,
    remote_description: amazing_webrtc::SdpDescription,
    ice_role: IceRole,
    consent: amazing_webrtc::ConsentFreshness,
    consent_clock: Instant,
}

impl SecureUdpEndpoint {
    pub fn establish(config: SecureEndpointConfig) -> Result<Self, SecureMediaError> {
        let SecureEndpointConfig {
            socket: std_socket,
            identity,
            local_description,
            remote_description,
            local_is_offerer,
            ice_role,
        } = config;
        let endpoint_local_description = local_description.clone();
        let endpoint_remote_description = remote_description.clone();
        std_socket.set_nonblocking(true).map_err(|error| {
            SecureMediaError::new(format!("configure UDP nonblocking: {error}"))
        })?;
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| {
                SecureMediaError::new(format!("cannot create endpoint runtime: {error}"))
            })?;
        let socket = {
            let _runtime_context = runtime.enter();
            UdpSocket::from_std(std_socket)
                .map_err(|error| SecureMediaError::new(format!("adopt UDP socket: {error}")))?
        };
        let socket = Arc::new(socket);
        let parts = runtime.block_on(establish_endpoint_async(
            Arc::clone(&socket),
            identity,
            local_description,
            remote_description,
            local_is_offerer,
            ice_role,
        ))?;
        Ok(Self {
            runtime,
            socket,
            dtls: Some(parts.dtls),
            demux_task: parts.demux_task,
            stun_rx: parts.stun_rx,
            media_rx: parts.media_rx,
            session: parts.session,
            local_description: endpoint_local_description,
            remote_description: endpoint_remote_description,
            ice_role,
            consent: parts.consent,
            consent_clock: parts.consent_clock,
        })
    }

    pub fn send_rtp(&mut self, plaintext: &[u8]) -> Result<usize, SecureMediaError> {
        self.require_consent_to_send()?;
        let packet = self.session.protect_rtp(plaintext)?;
        let length = packet.len();
        self.runtime
            .block_on(send_datagram(&self.socket, &packet))?;
        Ok(length)
    }

    pub fn receive_rtp(&mut self) -> Result<Vec<u8>, SecureMediaError> {
        let packet = self
            .runtime
            .block_on(receive_media_datagram(&mut self.media_rx))?;
        self.session.unprotect_rtp(&packet)
    }

    pub fn send_rtcp(&mut self, plaintext: &[u8]) -> Result<usize, SecureMediaError> {
        self.require_consent_to_send()?;
        let packet = self.session.protect_rtcp(plaintext)?;
        let length = packet.len();
        self.runtime
            .block_on(send_datagram(&self.socket, &packet))?;
        Ok(length)
    }

    pub fn receive_rtcp(&mut self) -> Result<Vec<u8>, SecureMediaError> {
        let packet = self
            .runtime
            .block_on(receive_media_datagram(&mut self.media_rx))?;
        self.session.unprotect_rtcp(&packet)
    }

    pub fn wait_for_consent_due(&mut self) -> Result<(), SecureMediaError> {
        loop {
            let now_ms = self.consent_now_ms();
            match self.consent.poll(now_ms) {
                amazing_webrtc::ConsentPoll::SendNow => return Ok(()),
                amazing_webrtc::ConsentPoll::Expired => {
                    return Err(SecureMediaError::new("ICE consent has expired"));
                }
                amazing_webrtc::ConsentPoll::WaitUntil(due_ms) => {
                    std::thread::sleep(Duration::from_millis(due_ms.saturating_sub(now_ms)));
                }
            }
        }
    }

    pub fn refresh_consent(&mut self) -> Result<amazing_webrtc::ConsentState, SecureMediaError> {
        if self.ice_role != IceRole::Controlling {
            return Err(SecureMediaError::new(
                "only the controlling endpoint sends this consent check",
            ));
        }
        self.wait_for_consent_due()?;
        let transaction = random_transaction_id()?;
        let next_interval = random_consent_interval()?;
        let now_ms = self.consent_now_ms();
        self.consent
            .record_sent(now_ms, transaction, next_interval)
            .map_err(|error| SecureMediaError::new(error.to_string()))?;
        let local = self
            .local_description
            .candidates
            .first()
            .ok_or_else(|| SecureMediaError::new("local consent candidate is missing"))?;
        let username = format!(
            "{}:{}",
            self.remote_description.ice_ufrag, self.local_description.ice_ufrag
        );
        let key = self.remote_description.ice_pwd.as_bytes();
        let request = amazing_webrtc::encode_ice_binding_request(
            transaction,
            &username,
            local.priority,
            0x1020_3040_5060_7080,
            false,
            key,
        )
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
        self.runtime
            .block_on(send_datagram(&self.socket, &request))?;
        let response = self
            .runtime
            .block_on(receive_stun_datagram(&mut self.stun_rx, UDP_TIMEOUT))?;
        let parsed = amazing_webrtc::parse_stun(&response)
            .map_err(|error| SecureMediaError::new(error.to_string()))?;
        if parsed.class != amazing_webrtc::StunClass::SuccessResponse
            || parsed.xor_mapped_address != Some(local.address)
            || !amazing_webrtc::verify_message_integrity(&response, key)
                .map_err(|error| SecureMediaError::new(error.to_string()))?
            || !amazing_webrtc::verify_fingerprint(&response)
                .map_err(|error| SecureMediaError::new(error.to_string()))?
        {
            return Err(SecureMediaError::new(
                "consent response failed authentication or transaction matching",
            ));
        }
        let response_time_ms = self.consent_now_ms();
        if !self
            .consent
            .record_authenticated_response(response_time_ms, parsed.transaction_id)
            .map_err(|error| SecureMediaError::new(error.to_string()))?
        {
            return Err(SecureMediaError::new(
                "consent response did not match an outstanding transaction",
            ));
        }
        Ok(self.consent.state(response_time_ms))
    }

    pub fn answer_consent(&mut self) -> Result<(), SecureMediaError> {
        if self.ice_role != IceRole::Controlled {
            return Err(SecureMediaError::new(
                "only the controlled endpoint answers this consent check",
            ));
        }
        let request = self.runtime.block_on(receive_stun_datagram(
            &mut self.stun_rx,
            CONSENT_RESPONSE_TIMEOUT,
        ))?;
        let parsed = amazing_webrtc::parse_stun(&request)
            .map_err(|error| SecureMediaError::new(error.to_string()))?;
        let remote = self
            .remote_description
            .candidates
            .first()
            .ok_or_else(|| SecureMediaError::new("remote consent candidate is missing"))?;
        let username = format!(
            "{}:{}",
            self.local_description.ice_ufrag, self.remote_description.ice_ufrag
        );
        let key = self.local_description.ice_pwd.as_bytes();
        if parsed.class != amazing_webrtc::StunClass::Request
            || parsed.username.as_deref() != Some(username.as_str())
            || parsed.priority != Some(remote.priority)
            || parsed.ice_controlling.is_none()
            || parsed.ice_controlled.is_some()
            || parsed.use_candidate
            || !amazing_webrtc::verify_message_integrity(&request, key)
                .map_err(|error| SecureMediaError::new(error.to_string()))?
            || !amazing_webrtc::verify_fingerprint(&request)
                .map_err(|error| SecureMediaError::new(error.to_string()))?
        {
            return Err(SecureMediaError::new(
                "consent request failed SDP authentication",
            ));
        }
        let response =
            amazing_webrtc::encode_ice_binding_success(parsed.transaction_id, remote.address, key)
                .map_err(|error| SecureMediaError::new(error.to_string()))?;
        self.runtime
            .block_on(send_datagram(&self.socket, &response))
    }

    pub fn consent_state(&self) -> amazing_webrtc::ConsentState {
        self.consent.state(self.consent_now_ms())
    }

    fn consent_now_ms(&self) -> u64 {
        u64::try_from(self.consent_clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn require_consent_to_send(&self) -> Result<(), SecureMediaError> {
        if self.consent_state() == amazing_webrtc::ConsentState::Expired {
            return Err(SecureMediaError::new(
                "ICE consent expired; application traffic is forbidden until ICE restart",
            ));
        }
        Ok(())
    }

    pub fn close(mut self) -> Result<(), SecureMediaError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<(), SecureMediaError> {
        if let Some(dtls) = self.dtls.take() {
            self.runtime
                .block_on(async { timeout(UDP_TIMEOUT, dtls.close()).await })
                .map_err(|_| SecureMediaError::new("DTLS endpoint close timed out"))?
                .map_err(|error| SecureMediaError::new(format!("close DTLS endpoint: {error}")))?;
        }
        self.demux_task.abort();
        Ok(())
    }
}

impl Drop for SecureUdpEndpoint {
    fn drop(&mut self) {
        if self.dtls.is_some() {
            let _ = self.shutdown();
        }
    }
}

struct EstablishedEndpoint {
    dtls: DTLSConn,
    demux_task: JoinHandle<()>,
    stun_rx: mpsc::Receiver<Vec<u8>>,
    media_rx: mpsc::Receiver<Vec<u8>>,
    session: SecureMediaSession,
    consent: amazing_webrtc::ConsentFreshness,
    consent_clock: Instant,
}

impl SecureMediaSession {
    fn protect_rtp(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SecureMediaError> {
        self.outgoing
            .encrypt_rtp(plaintext)
            .map(|packet| packet.to_vec())
            .map_err(|error| SecureMediaError::new(format!("protect RTP: {error}")))
    }

    fn unprotect_rtp(&mut self, protected: &[u8]) -> Result<Vec<u8>, SecureMediaError> {
        self.incoming
            .decrypt_rtp(protected)
            .map(|packet| packet.to_vec())
            .map_err(|error| SecureMediaError::new(format!("unprotect RTP: {error}")))
    }

    fn protect_rtcp(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SecureMediaError> {
        self.outgoing
            .encrypt_rtcp(plaintext)
            .map(|packet| packet.to_vec())
            .map_err(|error| SecureMediaError::new(format!("protect RTCP: {error}")))
    }

    fn unprotect_rtcp(&mut self, protected: &[u8]) -> Result<Vec<u8>, SecureMediaError> {
        self.incoming
            .decrypt_rtcp(protected)
            .map(|packet| packet.to_vec())
            .map_err(|error| SecureMediaError::new(format!("unprotect RTCP: {error}")))
    }
}

/// DTLS-facing half of one RFC 7983-style UDP demultiplexer.
///
/// A single socket reader routes DTLS records here and SRTP/SRTCP datagrams to
/// a separate media receiver, so the DTLS association remains alive while
/// media uses the same nominated five-tuple.
struct DtlsDemuxConn {
    socket: Arc<UdpSocket>,
    local_address: SocketAddr,
    remote_address: SocketAddr,
    dtls_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
}

struct DemuxParts {
    dtls: Arc<dyn Conn + Send + Sync>,
    stun_rx: mpsc::Receiver<Vec<u8>>,
    media_rx: mpsc::Receiver<Vec<u8>>,
    task: JoinHandle<()>,
}

#[async_trait]
impl Conn for DtlsDemuxConn {
    async fn connect(&self, address: SocketAddr) -> webrtc_util::Result<()> {
        if address != self.remote_address {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "DTLS demux is already connected to a different peer",
            )
            .into());
        }
        Ok(())
    }

    async fn recv(&self, buffer: &mut [u8]) -> webrtc_util::Result<usize> {
        let packet = self.dtls_rx.lock().await.recv().await.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "DTLS demux closed")
        })?;
        if packet.len() > buffer.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "DTLS datagram exceeds provider receive buffer",
            )
            .into());
        }
        buffer[..packet.len()].copy_from_slice(&packet);
        Ok(packet.len())
    }

    async fn recv_from(&self, buffer: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        Ok((self.recv(buffer).await?, self.remote_address))
    }

    async fn send(&self, packet: &[u8]) -> webrtc_util::Result<usize> {
        Ok(self.socket.send(packet).await?)
    }

    async fn send_to(&self, packet: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        if target != self.remote_address {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "DTLS send target does not match the nominated peer",
            )
            .into());
        }
        self.send(packet).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local_address)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.remote_address)
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        // The outer adapter owns the socket and demux-task lifetime. DTLSConn
        // independently stops its reader before this no-op transport close.
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

/// Runs a complete ICE-nominated UDP DTLS-SRTP association through the adapter.
///
/// The two endpoints signal ephemeral P-256 fingerprints and setup roles
/// through the std-only SDP model, nominate the same UDP pair with authenticated
/// ICE, perform mutual DTLS on that pair, then send protected RTP and RTCP over
/// the same sockets using RFC 5764's directional exporter layout.
pub fn run_secure_loopback() -> Result<SecureLoopbackReport, SecureMediaError> {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| SecureMediaError::new(format!("cannot create DTLS runtime: {error}")))?;
    runtime.block_on(run_secure_loopback_async())
}

async fn run_secure_loopback_async() -> Result<SecureLoopbackReport, SecureMediaError> {
    let client_identity = SecureIdentity::generate("ferric-client")?;
    let server_identity = SecureIdentity::generate("ferric-server")?;
    let client_transport = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|error| SecureMediaError::new(format!("bind client UDP: {error}")))?,
    );
    let server_transport = Arc::new(
        UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|error| SecureMediaError::new(format!("bind server UDP: {error}")))?,
    );
    let client_address = client_transport
        .local_addr()
        .map_err(|error| SecureMediaError::new(format!("client UDP address: {error}")))?;
    let server_address = server_transport
        .local_addr()
        .map_err(|error| SecureMediaError::new(format!("server UDP address: {error}")))?;
    client_transport
        .connect(server_address)
        .await
        .map_err(|error| SecureMediaError::new(format!("connect client UDP: {error}")))?;
    server_transport
        .connect(client_address)
        .await
        .map_err(|error| SecureMediaError::new(format!("connect server UDP: {error}")))?;
    let client_v4 = socket_v4(client_address)?;
    let server_v4 = socket_v4(server_address)?;
    let client_signal = build_local_description(
        client_v4,
        client_identity.fingerprint(),
        "actpass",
        "clienthost",
        "client",
        "FerricClientPassword123456",
        100,
    )?;
    let server_signal = build_local_description(
        server_v4,
        server_identity.fingerprint(),
        "passive",
        "serverhost",
        "server",
        "FerricServerPassword123456",
        200,
    )?;
    let ice_state = nominate_udp_pair(
        client_transport.as_ref(),
        server_transport.as_ref(),
        &client_signal,
        &server_signal,
    )
    .await?;
    let (client_is_dtls_client, server_is_dtls_client) =
        dtls_roles_from_setup(&client_signal.setup, &server_signal.setup)?;
    if !client_is_dtls_client || server_is_dtls_client {
        return Err(SecureMediaError::new(
            "secure loopback expected active/passive DTLS roles",
        ));
    }
    let expected_client_fingerprint = client_signal.fingerprint.clone();
    let expected_server_fingerprint = server_signal.fingerprint.clone();

    let common = |certificate| Config {
        certificates: vec![certificate],
        cipher_suites: vec![DTLS_CIPHER],
        signature_schemes: vec![SignatureScheme::EcdsaWithP256AndSha256],
        srtp_protection_profiles: vec![DTLS_SRTP_PROFILE],
        extended_master_secret: ExtendedMasterSecretType::Require,
        ..Default::default()
    };
    let mut client_config = common(client_identity.certificate);
    // WebRTC authenticates the self-signed certificate by the SDP fingerprint
    // below, rather than by the Web PKI hostname/CA path.
    client_config.insecure_skip_verify = true;
    let mut server_config = common(server_identity.certificate);
    server_config.client_auth = ClientAuthType::RequireAnyClientCert;

    let DemuxParts {
        dtls: client_dtls_transport,
        stun_rx: _client_stun_rx,
        media_rx: mut client_media_rx,
        task: client_demux_task,
    } = start_demux(
        Arc::clone(&client_transport),
        client_address,
        server_address,
    );
    let DemuxParts {
        dtls: server_dtls_transport,
        stun_rx: _server_stun_rx,
        media_rx: mut server_media_rx,
        task: server_demux_task,
    } = start_demux(
        Arc::clone(&server_transport),
        server_address,
        client_address,
    );
    let client_task = tokio::spawn(async move {
        DTLSConn::new(
            client_dtls_transport,
            client_config,
            client_is_dtls_client,
            None,
        )
        .await
    });
    let server = DTLSConn::new(
        server_dtls_transport,
        server_config,
        server_is_dtls_client,
        None,
    )
    .await
    .map_err(|error| SecureMediaError::new(format!("DTLS server handshake: {error}")))?;
    let client = client_task
        .await
        .map_err(|error| SecureMediaError::new(format!("DTLS client task: {error}")))?
        .map_err(|error| SecureMediaError::new(format!("DTLS client handshake: {error}")))?;

    if client.selected_srtpprotection_profile() != DTLS_SRTP_PROFILE
        || server.selected_srtpprotection_profile() != DTLS_SRTP_PROFILE
    {
        return Err(SecureMediaError::new(
            "DTLS peers did not negotiate AEAD_AES_128_GCM through use_srtp",
        ));
    }
    let client_state = client.connection_state().await;
    let server_state = server.connection_state().await;
    verify_peer_fingerprint(
        &client_state.peer_certificates,
        &expected_server_fingerprint,
    )?;
    verify_peer_fingerprint(
        &server_state.peer_certificates,
        &expected_client_fingerprint,
    )?;

    let keying_material = Zeroizing::new(
        client_state
            .export_keying_material(DTLS_SRTP_EXPORTER_LABEL, &[], EXPORTED_KEYING_MATERIAL_LEN)
            .await
            .map_err(|error| SecureMediaError::new(format!("client SRTP exporter: {error}")))?,
    );
    let server_keying_material = Zeroizing::new(
        server_state
            .export_keying_material(DTLS_SRTP_EXPORTER_LABEL, &[], EXPORTED_KEYING_MATERIAL_LEN)
            .await
            .map_err(|error| SecureMediaError::new(format!("server SRTP exporter: {error}")))?,
    );
    if keying_material != server_keying_material {
        return Err(SecureMediaError::new(
            "DTLS peers exported different SRTP keying material",
        ));
    }

    let mut client_session = secure_session(&keying_material, true)?;
    let mut server_session = secure_session(&keying_material, false)?;
    let mut client_tamper_session = secure_session(&keying_material, true)?;
    let mut server_tamper_session = secure_session(&keying_material, false)?;
    drop(server_keying_material);
    drop(keying_material);

    let rtp_packet = amazing_webrtc::RtpPacket {
        marker: true,
        payload_type: 0,
        sequence_number: 7,
        timestamp: 960,
        ssrc: 0xf322_1001,
        payload: b"ferric-loopback".to_vec(),
    };
    let rtp_plaintext = amazing_webrtc::encode_rtp(&rtp_packet)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let protected_rtp = client_session.protect_rtp(&rtp_plaintext)?;
    let mut tampered_rtp = protected_rtp.to_vec();
    let last = tampered_rtp
        .last_mut()
        .ok_or_else(|| SecureMediaError::new("protected RTP was empty"))?;
    *last ^= 1;
    send_datagram(&client_transport, &tampered_rtp).await?;
    let wire = receive_media_datagram(&mut server_media_rx).await?;
    let srtp_tamper_rejected = server_tamper_session.unprotect_rtp(&wire).is_err();
    if !srtp_tamper_rejected {
        return Err(SecureMediaError::new("tampered SRTP packet authenticated"));
    }
    send_datagram(&client_transport, &protected_rtp).await?;
    let wire = receive_media_datagram(&mut server_media_rx).await?;
    let decrypted_rtp = server_session.unprotect_rtp(&wire)?;
    if decrypted_rtp != rtp_plaintext {
        return Err(SecureMediaError::new("SRTP round trip changed RTP bytes"));
    }
    send_datagram(&client_transport, &protected_rtp).await?;
    let wire = receive_media_datagram(&mut server_media_rx).await?;
    let srtp_replay_rejected = server_session.unprotect_rtp(&wire).is_err();
    if !srtp_replay_rejected {
        return Err(SecureMediaError::new("replayed SRTP packet was accepted"));
    }

    let server_rtp_packet = amazing_webrtc::RtpPacket {
        marker: true,
        payload_type: 0,
        sequence_number: 11,
        timestamp: 1_920,
        ssrc: 0xa440_2002,
        payload: b"ferric-response".to_vec(),
    };
    let server_rtp_plaintext = amazing_webrtc::encode_rtp(&server_rtp_packet)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let server_protected_rtp = server_session.protect_rtp(&server_rtp_plaintext)?;
    send_datagram(&server_transport, &server_protected_rtp).await?;
    let wire = receive_media_datagram(&mut client_media_rx).await?;
    let server_decrypted_rtp = client_session.unprotect_rtp(&wire)?;
    if server_decrypted_rtp != server_rtp_plaintext {
        return Err(SecureMediaError::new(
            "server SRTP round trip changed RTP bytes",
        ));
    }

    let sender_report = amazing_webrtc::RtcpSenderReport {
        ssrc: server_rtp_packet.ssrc,
        ntp_timestamp: 0xe123_4567_89ab_cdef,
        rtp_timestamp: server_rtp_packet.timestamp,
        packet_count: 1,
        octet_count: server_rtp_packet.payload.len() as u32,
    };
    let compound_sender = amazing_webrtc::RtcpCompoundSenderReport {
        sender: sender_report,
        cname: "server@ferric".into(),
    };
    let rtcp_plaintext = amazing_webrtc::encode_rtcp_compound_sender_report(&compound_sender)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let protected_rtcp = server_session.protect_rtcp(&rtcp_plaintext)?;
    let mut tampered_rtcp = protected_rtcp.to_vec();
    let last = tampered_rtcp
        .last_mut()
        .ok_or_else(|| SecureMediaError::new("protected RTCP was empty"))?;
    *last ^= 1;
    send_datagram(&server_transport, &tampered_rtcp).await?;
    let wire = receive_media_datagram(&mut client_media_rx).await?;
    let srtcp_tamper_rejected = client_tamper_session.unprotect_rtcp(&wire).is_err();
    if !srtcp_tamper_rejected {
        return Err(SecureMediaError::new("tampered SRTCP packet authenticated"));
    }
    send_datagram(&server_transport, &protected_rtcp).await?;
    let wire = receive_media_datagram(&mut client_media_rx).await?;
    let decrypted_rtcp = client_session.unprotect_rtcp(&wire)?;
    let decoded_compound = amazing_webrtc::parse_rtcp_compound_sender_report(&decrypted_rtcp)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    if decoded_compound != compound_sender {
        return Err(SecureMediaError::new(
            "SRTCP round trip changed compound sender report",
        ));
    }
    send_datagram(&server_transport, &protected_rtcp).await?;
    let wire = receive_media_datagram(&mut client_media_rx).await?;
    let srtcp_replay_rejected = client_session.unprotect_rtcp(&wire).is_err();
    if !srtcp_replay_rejected {
        return Err(SecureMediaError::new("replayed SRTCP packet was accepted"));
    }

    let compound = amazing_webrtc::RtcpCompoundReceiverReport {
        receiver_ssrc: 0x5252_4356,
        reception: amazing_webrtc::ReceptionReport {
            source_ssrc: server_rtp_packet.ssrc,
            fraction_lost: 0,
            cumulative_lost: 0,
            extended_highest_sequence: server_rtp_packet.sequence_number as u32,
            interarrival_jitter: 0,
            last_sender_report: (sender_report.ntp_timestamp >> 16) as u32,
            delay_since_last_sender_report: 0,
        },
        cname: "client@ferric".into(),
    };
    let compound_wire = amazing_webrtc::encode_rtcp_compound_receiver_report(&compound)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let compound_protected = client_session.protect_rtcp(&compound_wire)?;
    send_datagram(&client_transport, &compound_protected).await?;
    let compound_received = receive_media_datagram(&mut server_media_rx).await?;
    let compound_plaintext = server_session.unprotect_rtcp(&compound_received)?;
    let compound_parsed = amazing_webrtc::parse_rtcp_compound_receiver_report(&compound_plaintext)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    if compound_parsed != compound {
        return Err(SecureMediaError::new(
            "compound SRTCP round trip changed receiver report",
        ));
    }

    let _ = tokio::join!(client.close(), server.close());
    drop(client);
    drop(server);
    client_demux_task.abort();
    server_demux_task.abort();

    Ok(SecureLoopbackReport {
        ice_state: match ice_state {
            amazing_webrtc::ChecklistState::Completed => "Completed",
            amazing_webrtc::ChecklistState::Running => "Running",
            amazing_webrtc::ChecklistState::Failed => "Failed",
        },
        ice_credentials_bound: true,
        udp_client_address: client_address,
        udp_server_address: server_address,
        sdp_identity_verified: true,
        client_setup: "active",
        server_setup: "passive",
        dtls_version: "DTLS1.2",
        cipher_suite: "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
        srtp_profile: "AEAD_AES_128_GCM",
        exported_keying_material_bytes: EXPORTED_KEYING_MATERIAL_LEN,
        rtp_payload_bytes: rtp_packet.payload.len(),
        protected_rtp_bytes: protected_rtp.len(),
        rtcp_packet_count: decoded_compound.sender.packet_count,
        protected_rtcp_bytes: protected_rtcp.len(),
        fingerprints_verified: true,
        srtp_tamper_rejected,
        srtp_replay_rejected,
        srtcp_tamper_rejected,
        srtcp_replay_rejected,
    })
}

async fn establish_endpoint_async(
    socket: Arc<UdpSocket>,
    identity: SecureIdentity,
    local_description: amazing_webrtc::SdpDescription,
    remote_description: amazing_webrtc::SdpDescription,
    local_is_offerer: bool,
    ice_role: IceRole,
) -> Result<EstablishedEndpoint, SecureMediaError> {
    if local_description.fingerprint != identity.fingerprint {
        return Err(SecureMediaError::new(
            "local SDP fingerprint does not match the supplied identity",
        ));
    }
    validate_description_socket(&socket, &local_description, &remote_description)?;
    match ice_role {
        IceRole::Controlling => {
            run_ice_controlling(&socket, &local_description, &remote_description).await?;
        }
        IceRole::Controlled => {
            run_ice_controlled(&socket, &local_description, &remote_description).await?;
        }
    }
    let consent_clock = Instant::now();
    let consent = amazing_webrtc::ConsentFreshness::new(0, random_consent_interval()?)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let local_address = socket
        .local_addr()
        .map_err(|error| SecureMediaError::new(format!("endpoint local address: {error}")))?;
    let remote_address = socket
        .peer_addr()
        .map_err(|error| SecureMediaError::new(format!("endpoint peer address: {error}")))?;
    let DemuxParts {
        dtls: dtls_transport,
        stun_rx,
        media_rx,
        task: demux_task,
    } = start_demux(Arc::clone(&socket), local_address, remote_address);
    let is_dtls_client = local_dtls_is_client(
        &local_description.setup,
        &remote_description.setup,
        local_is_offerer,
    )?;
    let mut provider_config = Config {
        certificates: vec![identity.certificate],
        cipher_suites: vec![DTLS_CIPHER],
        signature_schemes: vec![SignatureScheme::EcdsaWithP256AndSha256],
        srtp_protection_profiles: vec![DTLS_SRTP_PROFILE],
        extended_master_secret: ExtendedMasterSecretType::Require,
        ..Default::default()
    };
    if is_dtls_client {
        // WebRTC pins the self-signed peer through the parsed SDP fingerprint.
        provider_config.insecure_skip_verify = true;
    } else {
        provider_config.client_auth = ClientAuthType::RequireAnyClientCert;
    }
    let dtls = timeout(
        Duration::from_secs(10),
        DTLSConn::new(dtls_transport, provider_config, is_dtls_client, None),
    )
    .await
    .map_err(|_| SecureMediaError::new("DTLS endpoint handshake timed out"))?
    .map_err(|error| SecureMediaError::new(format!("establish DTLS endpoint: {error}")))?;
    if dtls.selected_srtpprotection_profile() != DTLS_SRTP_PROFILE {
        return Err(SecureMediaError::new(
            "DTLS endpoint did not negotiate AEAD_AES_128_GCM",
        ));
    }
    let state = dtls.connection_state().await;
    verify_peer_fingerprint(&state.peer_certificates, &remote_description.fingerprint)?;
    let keying_material = Zeroizing::new(
        state
            .export_keying_material(DTLS_SRTP_EXPORTER_LABEL, &[], EXPORTED_KEYING_MATERIAL_LEN)
            .await
            .map_err(|error| SecureMediaError::new(format!("endpoint SRTP exporter: {error}")))?,
    );
    let session = secure_session(&keying_material, is_dtls_client)?;
    Ok(EstablishedEndpoint {
        dtls,
        demux_task,
        stun_rx,
        media_rx,
        session,
        consent,
        consent_clock,
    })
}

fn local_dtls_is_client(
    local_setup: &str,
    remote_setup: &str,
    local_is_offerer: bool,
) -> Result<bool, SecureMediaError> {
    if local_is_offerer {
        match (local_setup, remote_setup) {
            ("actpass", "passive") => Ok(true),
            ("actpass", "active") => Ok(false),
            _ => Err(SecureMediaError::new(
                "local offer and remote answer have incompatible setup roles",
            )),
        }
    } else {
        match (remote_setup, local_setup) {
            ("actpass", "active") => Ok(true),
            ("actpass", "passive") => Ok(false),
            _ => Err(SecureMediaError::new(
                "remote offer and local answer have incompatible setup roles",
            )),
        }
    }
}

fn validate_description_socket(
    socket: &UdpSocket,
    local: &amazing_webrtc::SdpDescription,
    remote: &amazing_webrtc::SdpDescription,
) -> Result<(), SecureMediaError> {
    let local_address = socket_v4(
        socket
            .local_addr()
            .map_err(|error| SecureMediaError::new(format!("endpoint local address: {error}")))?,
    )?;
    let remote_address = socket_v4(
        socket
            .peer_addr()
            .map_err(|error| SecureMediaError::new(format!("endpoint peer address: {error}")))?,
    )?;
    let local_candidate = local
        .candidates
        .first()
        .ok_or_else(|| SecureMediaError::new("local SDP has no ICE candidate"))?;
    let remote_candidate = remote
        .candidates
        .first()
        .ok_or_else(|| SecureMediaError::new("remote SDP has no ICE candidate"))?;
    if local_candidate.address != local_address
        || remote_candidate.address != remote_address
        || local.connection_address != *local_address.ip()
        || remote.connection_address != *remote_address.ip()
    {
        return Err(SecureMediaError::new(
            "SDP candidates do not match the connected endpoint socket",
        ));
    }
    Ok(())
}

pub fn build_local_description(
    address: SocketAddrV4,
    fingerprint: &str,
    setup: &str,
    foundation: &str,
    ice_ufrag: &str,
    ice_pwd: &str,
    origin_id: u64,
) -> Result<amazing_webrtc::SdpDescription, SecureMediaError> {
    let description = amazing_webrtc::SdpDescription {
        origin: format!("- {origin_id} 1 IN IP4 {}", address.ip()),
        session_name: "Ferric secure UDP loopback".into(),
        timing: "0 0".into(),
        connection_address: *address.ip(),
        media: "audio".into(),
        port: address.port(),
        protocol: "UDP/TLS/RTP/SAVPF".into(),
        payload_types: vec![96, 0],
        ice_ufrag: ice_ufrag.into(),
        ice_pwd: ice_pwd.into(),
        fingerprint_algorithm: "sha-256".into(),
        fingerprint: fingerprint.into(),
        setup: setup.into(),
        candidates: vec![amazing_webrtc::IceCandidate {
            foundation: foundation.into(),
            component: 1,
            priority: 2_130_706_431,
            address,
        }],
        rtp_maps: vec![
            amazing_webrtc::RtpMap {
                payload_type: 96,
                encoding: "L16".into(),
                clock_rate: 48_000,
                channels: 1,
            },
            amazing_webrtc::RtpMap {
                payload_type: 0,
                encoding: "PCMU".into(),
                clock_rate: 8_000,
                channels: 1,
            },
        ],
    };
    let wire = amazing_webrtc::encode_sdp(&description)
        .map_err(|error| SecureMediaError::new(format!("encode secure SDP: {error}")))?;
    amazing_webrtc::parse_sdp(&wire)
        .map_err(|error| SecureMediaError::new(format!("parse secure SDP: {error}")))
}

fn dtls_roles_from_setup(
    offer_setup: &str,
    answer_setup: &str,
) -> Result<(bool, bool), SecureMediaError> {
    match (offer_setup, answer_setup) {
        ("actpass", "passive") => Ok((true, false)),
        ("actpass", "active") => Ok((false, true)),
        _ => Err(SecureMediaError::new(
            "SDP setup roles do not form a supported offer/answer pair",
        )),
    }
}

fn secure_session(
    keying_material: &[u8],
    is_client: bool,
) -> Result<SecureMediaSession, SecureMediaError> {
    if keying_material.len() != EXPORTED_KEYING_MATERIAL_LEN {
        return Err(SecureMediaError::new(
            "DTLS-SRTP exporter returned the wrong keying-material length",
        ));
    }
    let client_key = &keying_material[..MASTER_KEY_LEN];
    let server_key = &keying_material[MASTER_KEY_LEN..2 * MASTER_KEY_LEN];
    let client_salt = &keying_material[2 * MASTER_KEY_LEN..2 * MASTER_KEY_LEN + MASTER_SALT_LEN];
    let server_salt =
        &keying_material[2 * MASTER_KEY_LEN + MASTER_SALT_LEN..EXPORTED_KEYING_MATERIAL_LEN];
    let (outgoing_key, outgoing_salt, incoming_key, incoming_salt) = if is_client {
        (client_key, client_salt, server_key, server_salt)
    } else {
        (server_key, server_salt, client_key, client_salt)
    };
    Ok(SecureMediaSession {
        outgoing: crypto_context(outgoing_key, outgoing_salt, false)?,
        incoming: crypto_context(incoming_key, incoming_salt, true)?,
    })
}

async fn nominate_udp_pair(
    client: &UdpSocket,
    server: &UdpSocket,
    client_signal: &amazing_webrtc::SdpDescription,
    server_signal: &amazing_webrtc::SdpDescription,
) -> Result<amazing_webrtc::ChecklistState, SecureMediaError> {
    let (controlling, controlled) = tokio::join!(
        run_ice_controlling(client, client_signal, server_signal),
        run_ice_controlled(server, server_signal, client_signal),
    );
    let controlling = controlling?;
    let controlled = controlled?;
    if controlling != amazing_webrtc::ChecklistState::Completed
        || controlled != amazing_webrtc::ChecklistState::Completed
    {
        return Err(SecureMediaError::new(
            "both secure UDP ICE checklists did not complete",
        ));
    }
    Ok(controlling)
}

async fn run_ice_controlling(
    socket: &UdpSocket,
    local_signal: &amazing_webrtc::SdpDescription,
    remote_signal: &amazing_webrtc::SdpDescription,
) -> Result<amazing_webrtc::ChecklistState, SecureMediaError> {
    validate_description_socket(socket, local_signal, remote_signal)?;
    let local = local_signal
        .candidates
        .first()
        .ok_or_else(|| SecureMediaError::new("controlling SDP has no local candidate"))?;
    let remote = remote_signal
        .candidates
        .first()
        .ok_or_else(|| SecureMediaError::new("controlling SDP has no remote candidate"))?;
    let mut checklist = amazing_webrtc::IceChecklist::new(
        std::slice::from_ref(local),
        std::slice::from_ref(remote),
        true,
    );
    let pair = checklist
        .next_check()
        .ok_or_else(|| SecureMediaError::new("controlling ICE checklist had no pair"))?;
    let transaction = amazing_webrtc::TransactionId(*b"secure-ice!!");
    let username = format!("{}:{}", remote_signal.ice_ufrag, local_signal.ice_ufrag);
    let key = remote_signal.ice_pwd.as_bytes();
    let request = amazing_webrtc::encode_ice_binding_request(
        transaction,
        &username,
        local.priority,
        0x1020_3040_5060_7080,
        true,
        key,
    )
    .map_err(|error| SecureMediaError::new(error.to_string()))?;
    send_datagram(socket, &request).await?;
    let response_wire = receive_datagram(socket).await?;
    let parsed_response = amazing_webrtc::parse_stun(&response_wire)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    if parsed_response.class != amazing_webrtc::StunClass::SuccessResponse
        || parsed_response.transaction_id != transaction
        || parsed_response.xor_mapped_address != Some(local.address)
        || !amazing_webrtc::verify_message_integrity(&response_wire, key)
            .map_err(|error| SecureMediaError::new(error.to_string()))?
        || !amazing_webrtc::verify_fingerprint(&response_wire)
            .map_err(|error| SecureMediaError::new(error.to_string()))?
    {
        return Err(SecureMediaError::new(
            "secure UDP ICE response failed authentication",
        ));
    }
    checklist
        .mark_succeeded(pair, true)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let state = checklist.state();
    if state != amazing_webrtc::ChecklistState::Completed {
        return Err(SecureMediaError::new(
            "secure UDP pair was not nominated to Completed",
        ));
    }
    Ok(state)
}

async fn run_ice_controlled(
    socket: &UdpSocket,
    local_signal: &amazing_webrtc::SdpDescription,
    remote_signal: &amazing_webrtc::SdpDescription,
) -> Result<amazing_webrtc::ChecklistState, SecureMediaError> {
    validate_description_socket(socket, local_signal, remote_signal)?;
    let local = local_signal
        .candidates
        .first()
        .ok_or_else(|| SecureMediaError::new("controlled SDP has no local candidate"))?;
    let remote = remote_signal
        .candidates
        .first()
        .ok_or_else(|| SecureMediaError::new("controlled SDP has no remote candidate"))?;
    let mut checklist = amazing_webrtc::IceChecklist::new(
        std::slice::from_ref(local),
        std::slice::from_ref(remote),
        false,
    );
    let pair = checklist
        .next_check()
        .ok_or_else(|| SecureMediaError::new("controlled ICE checklist had no pair"))?;
    let request_wire = receive_datagram(socket).await?;
    let parsed_request = amazing_webrtc::parse_stun(&request_wire)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let username = format!("{}:{}", local_signal.ice_ufrag, remote_signal.ice_ufrag);
    let key = local_signal.ice_pwd.as_bytes();
    if parsed_request.class != amazing_webrtc::StunClass::Request
        || parsed_request.username.as_deref() != Some(username.as_str())
        || parsed_request.priority != Some(remote.priority)
        || parsed_request.ice_controlling.is_none()
        || parsed_request.ice_controlled.is_some()
        || !parsed_request.use_candidate
        || !amazing_webrtc::verify_message_integrity(&request_wire, key)
            .map_err(|error| SecureMediaError::new(error.to_string()))?
        || !amazing_webrtc::verify_fingerprint(&request_wire)
            .map_err(|error| SecureMediaError::new(error.to_string()))?
    {
        return Err(SecureMediaError::new(
            "controlled ICE request failed SDP authentication",
        ));
    }
    let response = amazing_webrtc::encode_ice_binding_success(
        parsed_request.transaction_id,
        remote.address,
        key,
    )
    .map_err(|error| SecureMediaError::new(error.to_string()))?;
    send_datagram(socket, &response).await?;
    checklist
        .mark_succeeded(pair, parsed_request.use_candidate)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let state = checklist.state();
    if state != amazing_webrtc::ChecklistState::Completed {
        return Err(SecureMediaError::new(
            "controlled ICE checklist did not complete",
        ));
    }
    Ok(state)
}

fn socket_v4(address: SocketAddr) -> Result<SocketAddrV4, SecureMediaError> {
    match address {
        SocketAddr::V4(address) => Ok(address),
        SocketAddr::V6(_) => Err(SecureMediaError::new(
            "secure loopback currently requires IPv4",
        )),
    }
}

pub fn secure_random_ice_tiebreaker() -> Result<u64, SecureMediaError> {
    loop {
        let mut bytes = [0u8; 8];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| SecureMediaError::new("secure random ICE tie-breaker failed"))?;
        let value = u64::from_be_bytes(bytes);
        if value != 0 && value != 10 {
            return Ok(value);
        }
    }
}

fn random_transaction_id() -> Result<amazing_webrtc::TransactionId, SecureMediaError> {
    let mut bytes = [0u8; 12];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SecureMediaError::new("secure random consent transaction failed"))?;
    Ok(amazing_webrtc::TransactionId(bytes))
}

fn random_consent_interval() -> Result<u64, SecureMediaError> {
    let mut bytes = [0u8; 2];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SecureMediaError::new("secure random consent interval failed"))?;
    Ok(
        amazing_webrtc::ConsentFreshness::MIN_INTERVAL_MS
            + u16::from_be_bytes(bytes) as u64 % 2_001,
    )
}

fn start_demux(
    socket: Arc<UdpSocket>,
    local_address: SocketAddr,
    remote_address: SocketAddr,
) -> DemuxParts {
    let (dtls_tx, dtls_rx) = mpsc::channel(32);
    let (stun_tx, stun_rx) = mpsc::channel(32);
    let (media_tx, media_rx) = mpsc::channel(32);
    let reader_socket = Arc::clone(&socket);
    let task = tokio::spawn(async move {
        let mut buffer = vec![0u8; 2_048];
        loop {
            if dtls_tx.is_closed() && stun_tx.is_closed() && media_tx.is_closed() {
                break;
            }
            let Ok(length) = reader_socket.recv(&mut buffer).await else {
                break;
            };
            if length == 0 {
                continue;
            }
            let packet = buffer[..length].to_vec();
            match packet[0] {
                0..=3 => {
                    let _ = stun_tx.try_send(packet);
                }
                20..=63 => {
                    // UDP congestion is represented as packet loss. Never
                    // block the sole socket reader behind either protocol
                    // class, or a full media queue could starve DTLS control.
                    let _ = dtls_tx.try_send(packet);
                }
                128..=191 => {
                    let _ = media_tx.try_send(packet);
                }
                _ => {
                    // TURN, ZRTP, and unknown datagrams are outside this
                    // milestone.
                }
            }
        }
    });
    let connection: Arc<dyn Conn + Send + Sync> = Arc::new(DtlsDemuxConn {
        socket,
        local_address,
        remote_address,
        dtls_rx: Mutex::new(dtls_rx),
    });
    DemuxParts {
        dtls: connection,
        stun_rx,
        media_rx,
        task,
    }
}

async fn send_datagram(socket: &UdpSocket, packet: &[u8]) -> Result<(), SecureMediaError> {
    let sent = timeout(UDP_TIMEOUT, socket.send(packet))
        .await
        .map_err(|_| SecureMediaError::new("UDP send timed out"))?
        .map_err(|error| SecureMediaError::new(format!("UDP send: {error}")))?;
    if sent != packet.len() {
        return Err(SecureMediaError::new("UDP send truncated a datagram"));
    }
    Ok(())
}

async fn receive_datagram(socket: &UdpSocket) -> Result<Vec<u8>, SecureMediaError> {
    let mut buffer = vec![0u8; 2_048];
    let length = timeout(UDP_TIMEOUT, socket.recv(&mut buffer))
        .await
        .map_err(|_| SecureMediaError::new("UDP receive timed out"))?
        .map_err(|error| SecureMediaError::new(format!("UDP receive: {error}")))?;
    buffer.truncate(length);
    Ok(buffer)
}

async fn receive_stun_datagram(
    receiver: &mut mpsc::Receiver<Vec<u8>>,
    wait: Duration,
) -> Result<Vec<u8>, SecureMediaError> {
    timeout(wait, receiver.recv())
        .await
        .map_err(|_| SecureMediaError::new("STUN consent receive timed out"))?
        .ok_or_else(|| SecureMediaError::new("STUN consent demux closed"))
}

async fn receive_media_datagram(
    receiver: &mut mpsc::Receiver<Vec<u8>>,
) -> Result<Vec<u8>, SecureMediaError> {
    timeout(UDP_TIMEOUT, receiver.recv())
        .await
        .map_err(|_| SecureMediaError::new("SRTP/SRTCP receive timed out"))?
        .ok_or_else(|| SecureMediaError::new("SRTP/SRTCP demux closed"))
}

fn crypto_context(
    master_key: &[u8],
    master_salt: &[u8],
    replay_protection: bool,
) -> Result<Context, SecureMediaError> {
    Context::new(
        master_key,
        master_salt,
        SRTP_PROFILE,
        replay_protection.then(|| srtp_replay_protection(REPLAY_WINDOW)),
        replay_protection.then(|| srtcp_replay_protection(REPLAY_WINDOW)),
    )
    .map_err(|error| SecureMediaError::new(format!("create SRTP context: {error}")))
}

fn certificate_fingerprint(certificate: &Certificate) -> Result<String, SecureMediaError> {
    let der = certificate
        .certificate
        .first()
        .ok_or_else(|| SecureMediaError::new("certificate chain was empty"))?;
    Ok(sha256_fingerprint(der.as_ref()))
}

fn verify_peer_fingerprint(
    peer_certificates: &[Vec<u8>],
    expected: &str,
) -> Result<(), SecureMediaError> {
    let peer = peer_certificates
        .first()
        .ok_or_else(|| SecureMediaError::new("DTLS peer did not present a certificate"))?;
    if sha256_fingerprint(peer) != expected {
        return Err(SecureMediaError::new(
            "DTLS peer certificate did not match the signaled SHA-256 fingerprint",
        ));
    }
    Ok(())
}

fn sha256_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut output = String::with_capacity(digest.len() * 3 - 1);
    for (index, byte) in digest.iter().enumerate() {
        if index != 0 {
            output.push(':');
        }
        write!(output, "{byte:02X}").expect("writing to String cannot fail");
    }
    output
}
