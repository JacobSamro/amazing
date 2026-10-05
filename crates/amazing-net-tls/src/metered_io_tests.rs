//! Native authenticated transport progress; no installed-browser claims.
use super::*;
use std::{io::ErrorKind, time::Instant};

#[test]
fn ciphertext_drain_reports_record_bytes_for_all_budgets() {
    for budget in [1, 7, 31, 128, 1024] {
        let payload = vec![0x5a; 257];
        let expected = payload.clone();
        let (port, thread) = test_support::serve(vec![b"http/1.1".to_vec()], move |io| {
            let mut bytes = vec![0; expected.len()];
            io.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
            io.write_all(b"ok").unwrap();
        });
        let mut stream = test_support::connect_nonblocking(port).unwrap();
        stream.inner.sock.set_nodelay(true).unwrap();
        stream.inner.conn.writer().write_all(&payload).unwrap();
        assert!(!stream.output_drained());
        let zero = stream.flush_metered(0);
        assert_eq!(zero.socket_bytes, 0);
        assert!(!zero.result.unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut cipher_bytes = 0;
        while !stream.output_drained() {
            assert!(Instant::now() < deadline);
            let step = stream.flush_metered(budget);
            assert!(step.socket_bytes <= budget);
            cipher_bytes += step.socket_bytes;
            assert_eq!(step.result.unwrap(), step.output_drained);
            std::thread::yield_now();
        }
        assert!(
            cipher_bytes > payload.len(),
            "TLS record overhead was metered"
        );
        let idle = stream.flush_metered(budget);
        assert_eq!(idle.socket_bytes, 0);
        assert!(idle.result.unwrap());
        let mut reply = [0; 2];
        let mut offset = 0;
        while offset < 2 {
            assert!(Instant::now() < deadline);
            let step = stream.read_metered(&mut reply[offset..], budget);
            assert!(step.socket_bytes <= budget);
            match step.result {
                Ok(0) => panic!("early EOF"),
                Ok(n) => offset += n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::yield_now(),
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(&reply, b"ok");
        thread.join().unwrap().unwrap();
    }
}

#[test]
fn would_block_retains_cipher_progress_and_plaintext_retry_boundary() {
    let (port, thread) = test_support::serve(vec![b"http/1.1".to_vec()], |io| {
        let mut bytes = [0; 65];
        io.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes[..64], &[b'a'; 64]);
        assert_eq!(bytes[64], b'z');
        io.write_all(b"ok").unwrap();
    });
    let mut stream = test_support::connect_nonblocking(port).unwrap();
    stream.inner.conn.set_buffer_limit(Some(64));
    stream.inner.conn.writer().write_all(&[b'a'; 64]).unwrap();
    let blocked = stream.write_metered(b"z", 1);
    assert_eq!(blocked.result.unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(
        blocked.socket_bytes, 1,
        "progress before WouldBlock survives"
    );
    assert!(!blocked.output_drained);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        let step = stream.write_metered(b"z", 1);
        assert!(step.socket_bytes <= 1);
        match step.result {
            Ok(1) => break,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            other => panic!("{other:?}"),
        }
    }
    while !stream.output_drained() {
        assert!(Instant::now() < deadline);
        assert!(stream.flush_metered(1).result.is_ok());
    }
    let mut bytes = [0; 2];
    stream.inner.sock.set_nonblocking(false).unwrap();
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"ok");
    thread.join().unwrap().unwrap();
}

#[test]
fn failed_authentication_reports_progress_then_consumes_pending_socket() {
    let (port, thread) = test_support::serve(vec![b"http/1.1".to_vec()], |_| {
        panic!("production roots accepted fixture CA");
    });
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut pending = PendingTls::new_http1("localhost", socket).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = 0;
    loop {
        assert!(Instant::now() < deadline);
        let step = pending.step_metered(7);
        assert!(step.socket_bytes <= 7);
        bytes += step.socket_bytes;
        match step.result {
            Ok(None) => std::thread::yield_now(),
            Ok(Some(_)) => panic!("untrusted certificate authenticated"),
            Err(_) => break,
        }
    }
    assert!(bytes > 0);
    let consumed = pending.step_metered(7);
    assert!(consumed.result.is_err());
    assert_eq!(consumed.socket_bytes, 0);
    assert!(thread.join().unwrap().is_err());
}

#[test]
fn explicit_instance_policy_keeps_full_authentication_and_default_trust_isolated() {
    let anchor = include_bytes!("../tests/fixtures/ca.der").to_vec();
    let policy = ClientPolicy::from_der_trust_anchors([anchor]).unwrap();
    let empty = ClientPolicy::from_der_trust_anchors([]).unwrap();
    assert!(ClientPolicy::from_der_trust_anchors([b"invalid DER".to_vec()]).is_err());
    for (http1, host, policy, accept) in [
        (false, "localhost", policy.clone(), true),
        (true, "localhost", policy.clone(), true),
        (false, "wrong.example", policy, false),
        (false, "localhost", ClientPolicy::default(), false),
        (false, "localhost", empty, false),
    ] {
        let (release, hold) = std::sync::mpsc::channel();
        let (port, peer) =
            test_support::serve(vec![b"h2".to_vec(), b"http/1.1".to_vec()], move |_| {
                hold.recv_timeout(Duration::from_secs(5)).unwrap();
            });
        let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut pending = if http1 {
            PendingTls::new_http1_with_policy(host, socket, &policy)
        } else {
            PendingTls::new_with_policy(host, socket, &policy)
        }
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            assert!(Instant::now() < deadline);
            let step = pending.step_metered(31);
            assert!(step.socket_bytes <= 31);
            match step.result {
                Ok(None) => std::thread::yield_now(),
                result => break result,
            }
        };
        if accept {
            let stream = result.unwrap().unwrap();
            assert_eq!(
                stream.protocol(),
                if http1 {
                    Protocol::Http11
                } else {
                    Protocol::H2
                }
            );
            release.send(()).unwrap();
            peer.join().unwrap().unwrap();
        } else {
            assert!(result.is_err());
            assert!(peer.join().unwrap().is_err());
        }
    }
}
