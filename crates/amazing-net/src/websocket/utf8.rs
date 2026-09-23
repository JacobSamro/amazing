use super::Error;
/// Incremental scalar validation; a fragment may end inside a scalar. Rejects
/// overlong encodings, surrogate scalars and values above U+10FFFF immediately.
#[derive(Default)]
pub(super) struct Validator {
    remaining: u8,
    low: u8,
    high: u8,
}
impl Validator {
    pub fn feed(&mut self, bytes: &[u8], final_part: bool) -> Result<(), Error> {
        for &b in bytes {
            if self.remaining != 0 {
                if b < self.low || b > self.high {
                    return Err(Error::Utf8);
                }
                self.remaining -= 1;
                self.low = 0x80;
                self.high = 0xbf;
            } else {
                let (n, low, high) = match b {
                    0..=0x7f => (0, 0x80, 0xbf),
                    0xc2..=0xdf => (1, 0x80, 0xbf),
                    0xe0 => (2, 0xa0, 0xbf),
                    0xe1..=0xec | 0xee..=0xef => (2, 0x80, 0xbf),
                    0xed => (2, 0x80, 0x9f),
                    0xf0 => (3, 0x90, 0xbf),
                    0xf1..=0xf3 => (3, 0x80, 0xbf),
                    0xf4 => (3, 0x80, 0x8f),
                    _ => return Err(Error::Utf8),
                };
                self.remaining = n;
                self.low = low;
                self.high = high;
            }
        }
        if final_part && self.remaining != 0 {
            return Err(Error::Utf8);
        }
        Ok(())
    }
}
