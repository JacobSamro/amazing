//! Fast deterministic-per-process hashing for engine-internal tables.
//!
//! Property tables, environment records and similar maps are probed on every
//! JavaScript property access and identifier lookup. The std default
//! (SipHash-1-3) cost 12-14% of samples in the r2-jsperf baseline profiles.
//! This is a multiply-rotate hasher in the FxHash family (written from
//! scratch; the 64-bit multiplier is the well-known golden-ratio constant).
//! The initial state is a per-process random seed so table layouts are not
//! predictable across runs; it is not a keyed PRF, so it trades the std
//! default's collision-flooding resistance for speed on short keys.
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

static SEED: AtomicU64 = AtomicU64::new(0);

fn seed() -> u64 {
    let current = SEED.load(Ordering::Relaxed);
    if current != 0 {
        return current;
    }
    let fresh = std::collections::hash_map::RandomState::new().build_hasher().finish() | 1;
    match SEED.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => fresh,
        Err(existing) => existing,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FastState(u64);

impl Default for FastState {
    fn default() -> Self {
        Self(seed())
    }
}

impl BuildHasher for FastState {
    type Hasher = FastHasher;
    #[inline]
    fn build_hasher(&self) -> FastHasher {
        FastHasher { hash: self.0 }
    }
}

pub struct FastHasher {
    hash: u64,
}

const MULTIPLIER: u64 = 0x517c_c1b7_2722_0a95;

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(MULTIPLIER);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().expect("8-byte chunk")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = 0u64;
            for (shift, byte) in rest.iter().enumerate() {
                word |= (*byte as u64) << (shift * 8);
            }
            // Fold the tail length in so "a" and "a\0" differ.
            self.add(word ^ ((rest.len() as u64) << 56));
        }
    }
    #[inline]
    fn write_u8(&mut self, value: u8) { self.add(value as u64); }
    #[inline]
    fn write_u16(&mut self, value: u16) { self.add(value as u64); }
    #[inline]
    fn write_u32(&mut self, value: u32) { self.add(value as u64); }
    #[inline]
    fn write_u64(&mut self, value: u64) { self.add(value); }
    #[inline]
    fn write_usize(&mut self, value: usize) { self.add(value as u64); }
    #[inline]
    fn finish(&self) -> u64 {
        // The multiply leaves the best-mixed bits on top; hashbrown indexes
        // with the low bits and tags with the top 7, so rotate to feed both.
        self.hash.rotate_left(26)
    }
}

pub type FastMap<K, V> = HashMap<K, V, FastState>;
pub type FastSet<T> = HashSet<T, FastState>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_as_a_map_and_distinguishes_tails() {
        let mut map: FastMap<String, usize> = FastMap::default();
        for index in 0..2000 {
            map.insert(format!("key{index}"), index);
        }
        map.insert("a".into(), 1);
        map.insert("a\0".into(), 2);
        assert_eq!(map.len(), 2002);
        assert_eq!(map["key1999"], 1999);
        assert_eq!(map["a"], 1);
        assert_eq!(map["a\0"], 2);
        let mut units: FastMap<Vec<u16>, u32> = FastMap::default();
        for index in 0..500u32 {
            units.insert(vec![index as u16, 7, 9], index);
        }
        assert_eq!(units[&vec![499u16, 7, 9]], 499);
    }
}
