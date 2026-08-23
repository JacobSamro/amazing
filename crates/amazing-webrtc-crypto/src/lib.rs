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
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::runtime::Builder;
use webrtc_dtls::cipher_suite::CipherSuiteId;
use webrtc_dtls::config::{ClientAuthType, Config, ExtendedMasterSecretType};
use webrtc_dtls::conn::DTLSConn;
use webrtc_dtls::crypto::Certificate;
use webrtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc_dtls::signature_hash_algorithm::SignatureScheme;
use webrtc_srtp::context::Context;
use webrtc_srtp::option::{srtcp_replay_protection, srtp_replay_protection};
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::KeyingMaterialExporter;
use webrtc_util::conn::conn_pipe::pipe;
use zeroize::Zeroizing;

const DTLS_SRTP_EXPORTER_LABEL: &str = "EXTRACTOR-dtls_srtp";
const DTLS_CIPHER: CipherSuiteId = CipherSuiteId::Tls_Ecdhe_Ecdsa_With_Aes_128_Gcm_Sha256;
const DTLS_SRTP_PROFILE: SrtpProtectionProfile = SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm;
const SRTP_PROFILE: ProtectionProfile = ProtectionProfile::AeadAes128Gcm;
const MASTER_KEY_LEN: usize = 16;
const MASTER_SALT_LEN: usize = 12;
const EXPORTED_KEYING_MATERIAL_LEN: usize = 2 * (MASTER_KEY_LEN + MASTER_SALT_LEN);
const REPLAY_WINDOW: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureLoopbackReport {
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

/// Runs a complete in-memory DTLS-SRTP association through the vetted adapter.
///
/// The two endpoints use ephemeral P-256 certificates and pin the peer's
/// SHA-256 certificate fingerprint after the mutual DTLS handshake, matching
/// WebRTC's SDP identity model. RTP and RTCP are then protected with distinct
/// directional keys exported under RFC 5764's label.
pub fn run_secure_loopback() -> Result<SecureLoopbackReport, SecureMediaError> {
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| SecureMediaError::new(format!("cannot create DTLS runtime: {error}")))?;
    runtime.block_on(run_secure_loopback_async())
}

async fn run_secure_loopback_async() -> Result<SecureLoopbackReport, SecureMediaError> {
    let client_certificate = Certificate::generate_self_signed(vec!["ferric-client".into()])
        .map_err(|error| SecureMediaError::new(format!("client certificate: {error}")))?;
    let server_certificate = Certificate::generate_self_signed(vec!["ferric-server".into()])
        .map_err(|error| SecureMediaError::new(format!("server certificate: {error}")))?;
    let expected_client_fingerprint = certificate_fingerprint(&client_certificate)?;
    let expected_server_fingerprint = certificate_fingerprint(&server_certificate)?;

    let common = |certificate| Config {
        certificates: vec![certificate],
        cipher_suites: vec![DTLS_CIPHER],
        signature_schemes: vec![SignatureScheme::EcdsaWithP256AndSha256],
        srtp_protection_profiles: vec![DTLS_SRTP_PROFILE],
        extended_master_secret: ExtendedMasterSecretType::Require,
        ..Default::default()
    };
    let mut client_config = common(client_certificate);
    // WebRTC authenticates the self-signed certificate by the SDP fingerprint
    // below, rather than by the Web PKI hostname/CA path.
    client_config.insecure_skip_verify = true;
    let mut server_config = common(server_certificate);
    server_config.client_auth = ClientAuthType::RequireAnyClientCert;

    let (client_transport, server_transport) = pipe();
    let client_task = tokio::spawn(async move {
        DTLSConn::new(Arc::new(client_transport), client_config, true, None).await
    });
    let server = DTLSConn::new(Arc::new(server_transport), server_config, false, None)
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

    let client_key = 0..MASTER_KEY_LEN;
    let server_key = MASTER_KEY_LEN..2 * MASTER_KEY_LEN;
    let client_salt = 2 * MASTER_KEY_LEN..2 * MASTER_KEY_LEN + MASTER_SALT_LEN;
    let server_salt = 2 * MASTER_KEY_LEN + MASTER_SALT_LEN..EXPORTED_KEYING_MATERIAL_LEN;
    let mut client_out = crypto_context(
        &keying_material[client_key.clone()],
        &keying_material[client_salt.clone()],
        false,
    )?;
    let mut server_in = crypto_context(
        &keying_material[client_key.clone()],
        &keying_material[client_salt.clone()],
        true,
    )?;
    let mut server_in_tamper = crypto_context(
        &keying_material[client_key],
        &keying_material[client_salt],
        true,
    )?;
    let mut server_out = crypto_context(
        &keying_material[server_key.clone()],
        &keying_material[server_salt.clone()],
        false,
    )?;
    let mut client_in = crypto_context(
        &keying_material[server_key.clone()],
        &keying_material[server_salt.clone()],
        true,
    )?;
    let mut client_in_tamper = crypto_context(
        &keying_material[server_key],
        &keying_material[server_salt],
        true,
    )?;
    drop(server_keying_material);
    drop(keying_material);

    let rtp_packet = amazing_webrtc::RtpPacket {
        marker: true,
        payload_type: 111,
        sequence_number: 7,
        timestamp: 960,
        ssrc: 0xf322_1001,
        payload: b"ferric-loopback".to_vec(),
    };
    let rtp_plaintext = amazing_webrtc::encode_rtp(&rtp_packet)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let protected_rtp = client_out
        .encrypt_rtp(&rtp_plaintext)
        .map_err(|error| SecureMediaError::new(format!("protect RTP: {error}")))?;
    let mut tampered_rtp = protected_rtp.to_vec();
    let last = tampered_rtp
        .last_mut()
        .ok_or_else(|| SecureMediaError::new("protected RTP was empty"))?;
    *last ^= 1;
    let srtp_tamper_rejected = server_in_tamper.decrypt_rtp(&tampered_rtp).is_err();
    if !srtp_tamper_rejected {
        return Err(SecureMediaError::new("tampered SRTP packet authenticated"));
    }
    let decrypted_rtp = server_in
        .decrypt_rtp(&protected_rtp)
        .map_err(|error| SecureMediaError::new(format!("unprotect RTP: {error}")))?;
    if decrypted_rtp.as_ref() != rtp_plaintext {
        return Err(SecureMediaError::new("SRTP round trip changed RTP bytes"));
    }
    let srtp_replay_rejected = server_in.decrypt_rtp(&protected_rtp).is_err();
    if !srtp_replay_rejected {
        return Err(SecureMediaError::new("replayed SRTP packet was accepted"));
    }

    let server_rtp_packet = amazing_webrtc::RtpPacket {
        marker: true,
        payload_type: 111,
        sequence_number: 11,
        timestamp: 1_920,
        ssrc: 0xa440_2002,
        payload: b"ferric-response".to_vec(),
    };
    let server_rtp_plaintext = amazing_webrtc::encode_rtp(&server_rtp_packet)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    let server_protected_rtp = server_out
        .encrypt_rtp(&server_rtp_plaintext)
        .map_err(|error| SecureMediaError::new(format!("protect server RTP: {error}")))?;
    let server_decrypted_rtp = client_in
        .decrypt_rtp(&server_protected_rtp)
        .map_err(|error| SecureMediaError::new(format!("unprotect server RTP: {error}")))?;
    if server_decrypted_rtp.as_ref() != server_rtp_plaintext {
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
    let rtcp_plaintext = amazing_webrtc::encode_rtcp_sender_report(sender_report);
    let protected_rtcp = server_out
        .encrypt_rtcp(&rtcp_plaintext)
        .map_err(|error| SecureMediaError::new(format!("protect RTCP: {error}")))?;
    let mut tampered_rtcp = protected_rtcp.to_vec();
    let last = tampered_rtcp
        .last_mut()
        .ok_or_else(|| SecureMediaError::new("protected RTCP was empty"))?;
    *last ^= 1;
    let srtcp_tamper_rejected = client_in_tamper.decrypt_rtcp(&tampered_rtcp).is_err();
    if !srtcp_tamper_rejected {
        return Err(SecureMediaError::new("tampered SRTCP packet authenticated"));
    }
    let decrypted_rtcp = client_in
        .decrypt_rtcp(&protected_rtcp)
        .map_err(|error| SecureMediaError::new(format!("unprotect RTCP: {error}")))?;
    let decoded_report = amazing_webrtc::parse_rtcp_sender_report(&decrypted_rtcp)
        .map_err(|error| SecureMediaError::new(error.to_string()))?;
    if decoded_report != sender_report {
        return Err(SecureMediaError::new(
            "SRTCP round trip changed sender report",
        ));
    }
    let srtcp_replay_rejected = client_in.decrypt_rtcp(&protected_rtcp).is_err();
    if !srtcp_replay_rejected {
        return Err(SecureMediaError::new("replayed SRTCP packet was accepted"));
    }

    let _ = tokio::join!(client.close(), server.close());
    Ok(SecureLoopbackReport {
        dtls_version: "DTLS1.2",
        cipher_suite: "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
        srtp_profile: "AEAD_AES_128_GCM",
        exported_keying_material_bytes: EXPORTED_KEYING_MATERIAL_LEN,
        rtp_payload_bytes: rtp_packet.payload.len(),
        protected_rtp_bytes: protected_rtp.len(),
        rtcp_packet_count: decoded_report.packet_count,
        protected_rtcp_bytes: protected_rtcp.len(),
        fingerprints_verified: true,
        srtp_tamper_rejected,
        srtp_replay_rejected,
        srtcp_tamper_rejected,
        srtcp_replay_rejected,
    })
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
