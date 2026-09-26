//! SHA-1, needed only for the RFC 6455 handshake challenge.
//!
//! This is deliberately a small, general SHA-1 so the `Sec-WebSocket-Accept`
//! value can be checked against published vectors. It is **not** a security
//! primitive: the handshake digest is a fixed non-secret challenge, not a
//! signature, password hash or authentication token.

/// The RFC 6455 handshake GUID.
const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Computes the SHA-1 digest of `bytes`.
pub fn sha1(bytes: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0,
    ];
    let mut chunks = bytes.chunks_exact(64);
    for chunk in &mut chunks {
        compress(&mut h, chunk);
    }
    let remainder = chunks.remainder();
    let mut tail = [0u8; 128];
    tail[..remainder.len()].copy_from_slice(remainder);
    tail[remainder.len()] = 0x80;
    let total = remainder.len() + 1;
    let padded = if total <= 56 { 64 } else { 128 };
    tail[padded - 8..padded].copy_from_slice(&((bytes.len() as u64) * 8).to_be_bytes());
    for chunk in tail[..padded].chunks_exact(64) {
        compress(&mut h, chunk);
    }
    let mut digest = [0u8; 20];
    for (out, word) in digest.chunks_exact_mut(4).zip(h) {
        out.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Computes the `Sec-WebSocket-Accept` value for a client key: the base64 of
/// SHA-1(key + GUID).
pub fn websocket_accept(key: &str) -> String {
    let mut input = Vec::with_capacity(key.len() + GUID.len());
    input.extend_from_slice(key.as_bytes());
    input.extend_from_slice(GUID);
    crate::base64::encode(&sha1(&input))
}

fn compress(h: &mut [u32; 5], chunk: &[u8]) {
    let mut w = [0u32; 80];
    for (i, word) in chunk.chunks_exact(4).enumerate() {
        w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *h;
    for (i, &word) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | (!b & d), 0x5a827999),
            20..=39 => (b ^ c ^ d, 0x6ed9eba1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
            _ => (b ^ c ^ d, 0xca62c1d6),
        };
        let t = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(word);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for (slot, value) in h.iter_mut().zip([a, b, c, d, e]) {
        *slot = slot.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn matches_published_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            hex(&sha1(&[b'a'; 1_000_000])),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    #[test]
    fn rfc6455_accept_example() {
        // RFC 6455 section 1.3 worked example.
        assert_eq!(
            websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }
}
