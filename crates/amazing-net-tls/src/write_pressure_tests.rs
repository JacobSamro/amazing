//! Authenticated loopback transport transcripts under provider queue pressure.
//! These exercise the native adapter, not a browser page or a WPT harness.
use super::*;
use std::time::Instant;

fn fixture(payload: Vec<u8>) -> (TlsStream, std::thread::JoinHandle<Result<(), TlsError>>) {
    let expected = payload.clone();
    let (port, thread) = test_support::serve(vec![b"http/1.1".to_vec()], move |io| {
        let mut received = vec![0; expected.len()];
        io.read_exact(&mut received).unwrap();
        assert_eq!(
            received, expected,
            "peer received every application byte once"
        );
        io.write_all(b"ok").unwrap();
    });
    let stream = test_support::connect_nonblocking(port).unwrap();
    stream.inner.sock.set_nodelay(true).unwrap();
    (stream, thread)
}

fn await_reply(stream: &mut TlsStream, budget: usize) {
    let until = Instant::now() + Duration::from_secs(5);
    let mut received = [0; 2];
    let mut offset = 0;
    while offset < received.len() {
        assert!(Instant::now() < until, "TLS reply stalled");
        match stream.read_step(&mut received[offset..], budget) {
            Ok(0) => panic!("peer closed before its complete reply"),
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::yield_now(),
            Err(e) => panic!("TLS reply: {e}"),
        }
    }
    assert_eq!(received, *b"ok");
}

fn send_exact(stream: &mut TlsStream, bytes: &[u8], budget: usize) {
    let until = Instant::now() + Duration::from_secs(5);
    let mut offset = 0;
    while offset < bytes.len() {
        assert!(Instant::now() < until, "TLS write stalled");
        match stream.write_step(&bytes[offset..], budget) {
            Ok(0) => panic!("temporary TLS queue pressure reported as a closed writer"),
            Ok(n) => {
                assert!(n <= budget);
                offset += n;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::yield_now(),
            Err(e) => panic!("TLS write: {e}"),
        }
    }
}

#[test]
fn full_ciphertext_queue_is_would_block_not_closed_writer() {
    let prefix = vec![b'p'; 64];
    let mut expected = prefix.clone();
    expected.push(b'x');
    let (mut stream, thread) = fixture(expected);
    stream.inner.conn.set_buffer_limit(Some(64));
    // Queue an authenticated record without socket I/O. The next application
    // byte cannot fit, even though the socket itself is ready for writes.
    stream.inner.conn.writer().write_all(&prefix).unwrap();
    assert!(stream.inner.conn.wants_write());
    let error = stream
        .write_step(b"x", 1)
        .expect_err("queue is temporarily full");
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    send_exact(&mut stream, b"x", 1);
    // Read steps must also finish the last queued ciphertext bytes.
    await_reply(&mut stream, 7);
    thread.join().unwrap().unwrap();
}

#[test]
fn bounded_write_pressure_matrix_delivers_exact_transcripts() {
    for limit in [32, 64, 128, 1024, 65536] {
        for budget in [1, 7, 31, 128, 1024] {
            let payload: Vec<u8> = (0..257).map(|n| (n % 251) as u8).collect();
            let (mut stream, thread) = fixture(payload.clone());
            stream.inner.conn.set_buffer_limit(Some(limit));
            send_exact(&mut stream, &payload, budget);
            await_reply(&mut stream, budget);
            thread.join().unwrap().unwrap();
        }
    }
}

#[test]
fn zero_budget_preserves_the_queue_and_accepts_no_application_bytes() {
    let prefix = vec![b'q'; 64];
    let mut expected = prefix.clone();
    expected.push(b'y');
    let (mut stream, thread) = fixture(expected);
    stream.inner.conn.set_buffer_limit(Some(64));
    stream.inner.conn.writer().write_all(&prefix).unwrap();
    let error = stream.write_step(b"y", 0).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert!(stream.inner.conn.wants_write());
    send_exact(&mut stream, b"y", 1);
    await_reply(&mut stream, 31);
    thread.join().unwrap().unwrap();
}

#[test]
fn empty_writes_drain_queued_records_without_false_application_progress() {
    let prefix = vec![b'e'; 64];
    let (mut stream, thread) = fixture(prefix.clone());
    stream.inner.conn.set_buffer_limit(Some(64));
    stream.inner.conn.writer().write_all(&prefix).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while stream.inner.conn.wants_write() {
        assert!(Instant::now() < until, "queued record stalled");
        assert_eq!(stream.write_step(&[], 1).unwrap(), 0);
    }
    await_reply(&mut stream, 1);
    thread.join().unwrap().unwrap();
}

#[test]
fn read_steps_finish_a_successfully_accepted_partial_ciphertext_write() {
    let (mut stream, thread) = fixture(vec![b'z']);
    assert_eq!(stream.write_step(b"z", 1).unwrap(), 1);
    assert!(stream.inner.conn.wants_write());
    await_reply(&mut stream, 1);
    thread.join().unwrap().unwrap();
}
