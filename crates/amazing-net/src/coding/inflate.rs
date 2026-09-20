use super::invalid;
use crate::NetError;

/// Allocation never follows an untrusted length before checking the budget.
/// Geometric growth is clamped to the limit; no reservation from gzip ISIZE.
pub(super) struct Output {
    pub(super) bytes: Vec<u8>,
    limit: usize,
}
impl Output {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
    fn room(&mut self, count: usize) -> Result<(), NetError> {
        let end = self
            .bytes
            .len()
            .checked_add(count)
            .ok_or(NetError::ResponseTooLarge)?;
        if end > self.limit {
            return Err(NetError::ResponseTooLarge);
        }
        if end > self.bytes.capacity() {
            let capacity = end
                .max(self.bytes.capacity().saturating_mul(2))
                .max(256)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| NetError::ResponseTooLarge)?;
        }
        Ok(())
    }
    fn literal(&mut self, byte: u8) -> Result<(), NetError> {
        self.room(1)?;
        self.bytes.push(byte);
        Ok(())
    }
    fn copy(
        &mut self,
        length: usize,
        distance: usize,
        start: usize,
        window: usize,
    ) -> Result<(), NetError> {
        if distance == 0 || distance > window || distance > self.bytes.len() - start {
            return Err(invalid("distance exceeds DEFLATE history"));
        }
        self.room(length)?;
        // Overlap is intentional (e.g. distance=1 generates a repeated run).
        for _ in 0..length {
            self.bytes.push(self.bytes[self.bytes.len() - distance]);
        }
        Ok(())
    }
}

struct Bits<'a> {
    input: &'a [u8],
    bit: usize,
}
impl Bits<'_> {
    fn read(&mut self, count: usize) -> Result<usize, NetError> {
        let mut value = 0;
        for shift in 0..count {
            let byte = *self
                .input
                .get(self.bit / 8)
                .ok_or(NetError::IncompleteBody)?;
            value |= (((byte >> (self.bit % 8)) & 1) as usize) << shift;
            self.bit += 1;
        }
        Ok(value)
    }
    fn align(&mut self) {
        self.bit = self.bit.div_ceil(8) * 8;
    }
}

/// Canonical codes grouped by length, then symbol. Small fixed tables avoid
/// allocations controlled by a stream and decoding scans at most 15 bits.
struct Huffman {
    counts: [usize; 16],
    symbols: [usize; 288],
}
impl Huffman {
    fn new(lengths: &[u8], code_lengths: bool) -> Result<Self, NetError> {
        let mut tree = Self {
            counts: [0; 16],
            symbols: [0; 288],
        };
        for &length in lengths {
            if length > 15 {
                return Err(invalid("Huffman length exceeds 15"));
            }
            tree.counts[length as usize] += 1;
        }
        tree.counts[0] = 0;
        let mut left = 1i32;
        for bits in 1..=15 {
            left = (left << 1) - tree.counts[bits] as i32;
            if left < 0 {
                return Err(invalid("oversubscribed Huffman tree"));
            }
        }
        let total: usize = tree.counts.iter().sum();
        // Empty distance alphabet is valid only when unused. An incomplete
        // literal/distance alphabet may contain one one-bit code; never CL.
        if left != 0 && (code_lengths || (total != 0 && !(total == 1 && tree.counts[1] == 1))) {
            return Err(invalid("incomplete Huffman tree"));
        }
        let mut offsets = [0; 16];
        for bits in 1..15 {
            offsets[bits + 1] = offsets[bits] + tree.counts[bits];
        }
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                tree.symbols[offsets[length as usize]] = symbol;
                offsets[length as usize] += 1;
            }
        }
        Ok(tree)
    }
    fn symbol(&self, bits: &mut Bits<'_>) -> Result<usize, NetError> {
        let (mut code, mut first, mut index) = (0, 0, 0);
        for length in 1..=15 {
            // Huffman codes are transmitted MSB first, inside LSB-first bytes.
            code = (code << 1) | bits.read(1)?;
            let count = self.counts[length];
            if code >= first && code - first < count {
                return Ok(self.symbols[index + code - first]);
            }
            index += count;
            first = (first + count) << 1;
        }
        Err(invalid("invalid Huffman code"))
    }
}

fn trees(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), NetError> {
    let literals = bits.read(5)? + 257;
    let distances = bits.read(5)? + 1;
    let codes = bits.read(4)? + 4;
    if literals > 286 {
        return Err(invalid("reserved HLIT"));
    }
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let mut cl = [0u8; 19];
    for &index in &ORDER[..codes] {
        cl[index] = bits.read(3)? as u8;
    }
    let cl_tree = Huffman::new(&cl, true)?;
    let mut lengths = [0u8; 318];
    let total = literals + distances;
    let mut pos = 0;
    while pos < total {
        let symbol = cl_tree.symbol(bits)?;
        let (value, count) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                if pos == 0 {
                    return Err(invalid("repeat without preceding code length"));
                }
                (lengths[pos - 1], bits.read(2)? + 3)
            }
            17 => (0, bits.read(3)? + 3),
            18 => (0, bits.read(7)? + 11),
            _ => return Err(invalid("invalid code length symbol")),
        };
        if pos + count > total {
            return Err(invalid("code length repeat overrun"));
        }
        lengths[pos..pos + count].fill(value);
        pos += count;
    }
    if lengths[256] == 0 {
        return Err(invalid("missing end-of-block code"));
    }
    Ok((
        Huffman::new(&lengths[..literals], false)?,
        Huffman::new(&lengths[literals..total], false)?,
    ))
}

const LENGTH: [usize; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_BITS: [usize; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE: [usize; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_BITS: [usize; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

pub(super) fn inflate(input: &[u8], out: &mut Output, window: usize) -> Result<usize, NetError> {
    let mut bits = Bits { input, bit: 0 };
    let start = out.bytes.len();
    loop {
        let final_block = bits.read(1)? != 0;
        match bits.read(2)? {
            0 => {
                bits.align();
                let len = bits.read(16)?;
                if bits.read(16)? != (len ^ 0xffff) {
                    return Err(invalid("stored LEN/NLEN mismatch"));
                }
                out.room(len)?;
                let pos = bits.bit / 8;
                let bytes = input.get(pos..pos + len).ok_or(NetError::IncompleteBody)?;
                out.bytes.extend_from_slice(bytes);
                bits.bit += len * 8;
            }
            kind @ (1 | 2) => {
                let (literal, distance) = if kind == 1 {
                    let mut lengths = [8; 288];
                    lengths[144..256].fill(9);
                    lengths[256..280].fill(7);
                    (
                        Huffman::new(&lengths, false)?,
                        Huffman::new(&[5; 32], false)?,
                    )
                } else {
                    trees(&mut bits)?
                };
                loop {
                    match literal.symbol(&mut bits)? {
                        byte @ 0..=255 => out.literal(byte as u8)?,
                        256 => break,
                        symbol @ 257..=285 => {
                            let index = symbol - 257;
                            let length = LENGTH[index] + bits.read(LENGTH_BITS[index])?;
                            let index = distance.symbol(&mut bits)?;
                            if index >= 30 {
                                return Err(invalid("reserved distance code"));
                            }
                            let dist = DISTANCE[index] + bits.read(DISTANCE_BITS[index])?;
                            out.copy(length, dist, start, window)?;
                        }
                        _ => return Err(invalid("reserved literal/length code")),
                    }
                }
            }
            _ => return Err(invalid("reserved block type")),
        }
        if final_block {
            return Ok(bits.bit.div_ceil(8));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_limit_is_checked_before_reserving_or_copying() {
        let mut out = Output::new(8);
        assert!(matches!(out.room(9), Err(NetError::ResponseTooLarge)));
        assert_eq!(out.bytes.capacity(), 0);
        out.literal(b'x').unwrap();
        let capacity = out.bytes.capacity();
        assert!(matches!(
            out.copy(8, 1, 0, 32768),
            Err(NetError::ResponseTooLarge)
        ));
        assert_eq!(out.bytes, b"x");
        assert_eq!(out.bytes.capacity(), capacity);
        assert!(capacity <= 8);
    }
    #[test]
    fn invalid_trees_and_empty_distance_alphabet() {
        assert!(Huffman::new(&[1, 1, 1], false).is_err());
        assert!(Huffman::new(&[2, 2], false).is_err());
        assert!(Huffman::new(&[1], true).is_err());
        assert!(Huffman::new(&[0], false).is_ok());
        assert!(Huffman::new(&[1], false).is_ok());
    }
    #[test]
    fn history_is_limited_to_the_current_member_and_declared_window() {
        let mut out = Output::new(1024);
        out.bytes.resize(300, 1);
        assert!(out.copy(3, 257, 0, 256).is_err());
        assert!(out.copy(3, 1, 300, 32768).is_err());
        assert!(out.copy(3, 301, 0, 32768).is_err());
    }
}
