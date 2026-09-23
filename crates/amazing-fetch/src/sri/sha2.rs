//! Small std-only SHA-256/SHA-384/SHA-512 implementation for SRI body checks.
//! Compression follows FIPS 180-4; the caller has already bounded response size.
use super::constants::{H256, H384, H512, K256, K512};

pub(super) fn digest(kind: usize, input: &[u8]) -> Vec<u8> {
    match kind {
        0 => sha256(input).to_vec(),
        1 => sha512_family(input, true)[..48].to_vec(),
        _ => sha512_family(input, false).to_vec(),
    }
}
fn sha256(input: &[u8]) -> [u8; 32] {
    let mut state = H256;
    let mut chunks = input.chunks_exact(64);
    for chunk in &mut chunks {
        compress256(&mut state, chunk);
    }
    let rest = chunks.remainder();
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let final_len = if rest.len() < 56 { 64 } else { 128 };
    tail[final_len - 8..final_len].copy_from_slice(&((input.len() as u64) * 8).to_be_bytes());
    for chunk in tail[..final_len].chunks_exact(64) {
        compress256(&mut state, chunk);
    }
    let mut out = [0u8; 32];
    for (word, bytes) in state.iter().zip(out.chunks_exact_mut(4)) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    out
}
fn compress256(state: &mut [u32; 8], chunk: &[u8]) {
    let mut w = [0u32; 64];
    for (i, word) in chunk.chunks_exact(4).enumerate() {
        w[i] = u32::from_be_bytes(word.try_into().unwrap());
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let mut v = *state;
    for i in 0..64 {
        let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
        let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
        let t1 = v[7]
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K256[i])
            .wrapping_add(w[i]);
        let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
        let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let t2 = s0.wrapping_add(maj);
        v.copy_within(0..7, 1);
        v[4] = v[4].wrapping_add(t1);
        v[0] = t1.wrapping_add(t2);
    }
    for (a, b) in state.iter_mut().zip(v) {
        *a = a.wrapping_add(b);
    }
}
fn sha512_family(input: &[u8], sha384: bool) -> [u8; 64] {
    let mut state = if sha384 { H384 } else { H512 };
    let mut chunks = input.chunks_exact(128);
    for chunk in &mut chunks {
        compress512(&mut state, chunk);
    }
    let rest = chunks.remainder();
    let mut tail = [0u8; 256];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let final_len = if rest.len() < 112 { 128 } else { 256 };
    tail[final_len - 16..final_len].copy_from_slice(&((input.len() as u128) * 8).to_be_bytes());
    for chunk in tail[..final_len].chunks_exact(128) {
        compress512(&mut state, chunk);
    }
    let mut out = [0u8; 64];
    for (word, bytes) in state.iter().zip(out.chunks_exact_mut(8)) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    out
}
fn compress512(state: &mut [u64; 8], chunk: &[u8]) {
    let mut w = [0u64; 80];
    for (i, word) in chunk.chunks_exact(8).enumerate() {
        w[i] = u64::from_be_bytes(word.try_into().unwrap());
    }
    for i in 16..80 {
        let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
        let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let mut v = *state;
    for i in 0..80 {
        let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
        let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
        let t1 = v[7]
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K512[i])
            .wrapping_add(w[i]);
        let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
        let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let t2 = s0.wrapping_add(maj);
        v.copy_within(0..7, 1);
        v[4] = v[4].wrapping_add(t1);
        v[0] = t1.wrapping_add(t2);
    }
    for (a, b) in state.iter_mut().zip(v) {
        *a = a.wrapping_add(b);
    }
}
