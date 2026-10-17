//! Minimal RFC 1951 compressor (greedy LZ77 + fixed Huffman, one final
//! block) for `CompressionStream`. Std-only; output is valid for any inflater.
const LENGTH_BASE: [usize; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [usize; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const WINDOW: usize = 32768;
const MAX_CHAIN: usize = 48;

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    count: u32,
}
impl BitWriter {
    fn bits(&mut self, value: u32, count: u32) {
        self.acc |= (value as u64) << self.count;
        self.count += count;
        while self.count >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.count -= 8;
        }
    }
    /// Huffman codes are written most-significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        let mut reversed = 0;
        for i in 0..len {
            reversed |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.bits(reversed, len);
    }
    fn literal_length(&mut self, symbol: usize) {
        match symbol {
            0..=143 => self.code(0x30 + symbol as u32, 8),
            144..=255 => self.code(0x190 + (symbol as u32 - 144), 9),
            256..=279 => self.code(symbol as u32 - 256, 7),
            _ => self.code(0xc0 + (symbol as u32 - 280), 8),
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

pub(crate) fn deflate_raw(input: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(input.len() / 2 + 16), acc: 0, count: 0 };
    w.bits(1, 1); // final block
    w.bits(1, 2); // fixed Huffman
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; input.len()];
    let hash = |i: usize| -> usize {
        ((input[i] as usize) << 10 ^ (input[i + 1] as usize) << 5 ^ input[i + 2] as usize) & 0x7fff
    };
    let mut i = 0;
    while i < input.len() {
        let mut best_len = 0;
        let mut best_dist = 0;
        if i + 3 <= input.len() {
            let h = hash(i);
            let mut candidate = head[h];
            let mut chain = 0;
            let max = (input.len() - i).min(258);
            while candidate != usize::MAX && i - candidate <= WINDOW && chain < MAX_CHAIN {
                let mut len = 0;
                while len < max && input[candidate + len] == input[i + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = i - candidate;
                    if len == max {
                        break;
                    }
                }
                candidate = prev[candidate];
                chain += 1;
            }
        }
        let advance = if best_len >= 3 {
            let code = LENGTH_BASE.iter().rposition(|&b| b <= best_len).unwrap();
            w.literal_length(257 + code);
            w.bits((best_len - LENGTH_BASE[code]) as u32, LENGTH_EXTRA[code]);
            let dcode = DIST_BASE.iter().rposition(|&b| b <= best_dist).unwrap();
            w.code(dcode as u32, 5);
            w.bits((best_dist - DIST_BASE[dcode]) as u32, DIST_EXTRA[dcode]);
            best_len
        } else {
            w.literal_length(input[i] as usize);
            1
        };
        for j in i..i + advance {
            if j + 3 <= input.len() {
                let h = hash(j);
                prev[j] = head[h];
                head[h] = j;
            }
        }
        i += advance;
    }
    w.literal_length(256);
    w.finish()
}

/// A valid Brotli stream made of uncompressed meta-blocks (RFC 7932 section 9.2).
pub(crate) fn brotli_uncompressed(input: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(input.len() + 16), acc: 0, count: 0 };
    w.bits(0, 1); // WBITS = 16
    for chunk in input.chunks(65536) {
        w.bits(0, 1); // ISLAST
        w.bits(0, 2); // MNIBBLES = 4
        w.bits((chunk.len() - 1) as u32, 16);
        w.bits(1, 1); // ISUNCOMPRESSED
        if w.count > 0 {
            let pad = 8 - w.count;
            w.bits(0, pad);
        }
        w.out.extend_from_slice(chunk);
    }
    w.bits(1, 1); // ISLAST
    w.bits(1, 1); // ISLASTEMPTY
    w.finish()
}
