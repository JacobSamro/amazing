//! Real authenticated loopback handshakes; no permissive verifier is installed.
use super::test_support::serve_certificates;
use super::*;
use rustls::pki_types::CertificateDer;

const CA: &[u8] = include_bytes!("../tests/fixtures/validation/ca.der");
const KEY: &[u8] = include_bytes!("../tests/fixtures/validation/key.der");
const VALID: &[u8] = include_bytes!("../tests/fixtures/validation/valid.der");
const INTERMEDIATE: &[u8] = include_bytes!("../tests/fixtures/validation/intermediate.der");
const CHAINED: &[u8] = include_bytes!("../tests/fixtures/validation/chained.der");

fn config() -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(CA.to_vec())).unwrap();
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(config)
}

fn handshake(host: &str, certificates: &[&[u8]], expect: Result<Protocol, CertificateError>) {
    let (port, thread) = serve_certificates(
        vec![b"http/1.1".to_vec()],
        certificates
            .iter()
            .map(|c| CertificateDer::from(c.to_vec()))
            .collect(),
        KEY,
        true,
        |io| {
            let mut byte = [0];
            io.read_exact(&mut byte).unwrap();
            io.write_all(&byte).unwrap();
        },
    );
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut pending = PendingTls::with_config(host, socket, config()).unwrap();
    let result = loop {
        match pending.step(256) {
            Ok(None) => continue,
            done => break done,
        }
    };
    match expect {
        Ok(protocol) => {
            let mut stream = result.unwrap().expect("authenticated stream");
            assert_eq!(stream.protocol(), protocol);
            stream.write_all(b"x").unwrap();
            stream.flush().unwrap();
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            assert_eq!(byte, *b"x");
            thread.join().unwrap().unwrap();
        }
        Err(reason) => {
            assert!(
                matches!(result, Err(TlsError::Certificate(actual)) if actual == reason),
                "expected {reason:?}"
            );
            // A rejected handshake drops its socket and cannot progress again,
            // including a zero-budget retry by a non-blocking dispatcher.
            assert!(pending.inner.is_none());
            assert!(matches!(pending.step(0), Err(TlsError::Transport(_))));
            assert!(matches!(pending.step(65536), Err(TlsError::Transport(_))));
            assert!(thread.join().unwrap().is_err());
        }
    }
}

#[test]
fn dns_and_ip_subject_alternative_names() {
    for host in ["localhost", "LOCALHOST", "localhost.", "127.0.0.1", "::1"] {
        handshake(host, &[VALID], Ok(Protocol::Http11));
    }
    for host in ["wrong.example.test", "127.0.0.2", "::2"] {
        handshake(host, &[VALID], Err(CertificateError::HostnameMismatch));
    }
}

#[test]
fn wildcard_is_one_dns_label() {
    let wildcard = include_bytes!("../tests/fixtures/validation/wildcard.der");
    handshake("www.example.test", &[wildcard], Ok(Protocol::Http11));
    for host in ["example.test", "a.b.example.test", "www.other.test"] {
        handshake(host, &[wildcard], Err(CertificateError::HostnameMismatch));
    }
}

#[test]
fn common_name_and_dns_ip_do_not_replace_san_identity() {
    handshake(
        "localhost",
        &[include_bytes!(
            "../tests/fixtures/validation/common-name-only.der"
        )],
        Err(CertificateError::HostnameMismatch),
    );
    handshake(
        "127.0.0.1",
        &[include_bytes!(
            "../tests/fixtures/validation/dns-ip-only.der"
        )],
        Err(CertificateError::HostnameMismatch),
    );
}

#[test]
fn validity_and_server_authentication_constraints() {
    for (certificate, reason) in [
        (
            &include_bytes!("../tests/fixtures/validation/expired.der")[..],
            CertificateError::Expired,
        ),
        (
            &include_bytes!("../tests/fixtures/validation/future.der")[..],
            CertificateError::NotValidYet,
        ),
        (
            &include_bytes!("../tests/fixtures/validation/wrong-purpose.der")[..],
            CertificateError::InvalidPurpose,
        ),
        (
            &include_bytes!("../tests/fixtures/validation/critical.der")[..],
            // The pinned provider encapsulates this WebPKI rejection as Other.
            CertificateError::Other,
        ),
        (
            &include_bytes!("../tests/fixtures/validation/bad-signature.der")[..],
            CertificateError::BadSignature,
        ),
    ] {
        handshake("localhost", &[certificate], Err(reason));
    }
}

#[test]
fn complete_intermediate_chain_required() {
    handshake("localhost", &[CHAINED, INTERMEDIATE], Ok(Protocol::Http11));
    handshake(
        "localhost",
        &[CHAINED],
        Err(CertificateError::UnknownIssuer),
    );
}

#[test]
fn no_common_alpn_is_terminal() {
    let (port, thread) = serve_certificates(
        vec![b"unoffered".to_vec()],
        vec![CertificateDer::from(VALID.to_vec())],
        KEY,
        true,
        |_| panic!("unoffered ALPN accepted"),
    );
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut pending = PendingTls::with_config("localhost", socket, config()).unwrap();
    let error = loop {
        match pending.step(4096) {
            Ok(None) => continue,
            Err(error) => break error,
            Ok(Some(_)) => panic!("unexpected authenticated stream"),
        }
    };
    // A remote no_application_protocol alert has the same stable reason
    // as a local ALPN rejection. Neither is a certificate error.
    assert!(matches!(error, TlsError::NoApplicationProtocol));
    assert!(pending.inner.is_none());
    assert!(pending.step(4096).is_err());
    assert!(thread.join().unwrap().is_err());
}
