//! A cursor over a byte slice with the WebAssembly primitive decoders.
//!
//! Everything the binary format needs to read: unsigned and signed LEB128,
//! fixed-width little-endian floats, length-prefixed UTF-8 names and raw
//! byte vectors. Every read is bounds checked; a short read is a
//! [`ErrorKind::Malformed`](crate::ErrorKind::Malformed) error.

use crate::error::{Result, WasmError};

/// A bounds-checked cursor over a borrowed byte slice.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Wrap a byte slice.
    pub fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    /// Current offset from the start of the slice.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// True once the slice is exhausted.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// The whole underlying slice.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Consume one byte, or fail with `what`.
    pub fn u8_raw(&mut self, what: &str) -> Result<u8> {
        if self.pos >= self.bytes.len() {
            return Err(WasmError::malformed(format!("unexpected end of {} input", what)));
        }
        let b = self.bytes[self.pos];
        self.pos += 1;
        Ok(b)
    }

    /// Peek at the next byte without consuming it.
    pub fn peek_u8(&self) -> Result<u8> {
        if self.pos >= self.bytes.len() {
            return Err(WasmError::malformed("unexpected end of input"));
        }
        Ok(self.bytes[self.pos])
    }

    /// Consume exactly `n` bytes.
    pub fn bytes_n(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(WasmError::malformed("unexpected end of section or function"));
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// Unsigned LEB128 constrained to 32 bits.
    pub fn var_u32(&mut self) -> Result<u32> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        for i in 0..5 {
            let byte = self.u8_raw("integer")?;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if i == 4 && byte > 0x0f {
                    return Err(WasmError::malformed("integer too large"));
                }
                return Ok(result as u32);
            }
            shift += 7;
        }
        Err(WasmError::malformed("integer representation too long"))
    }

    /// Unsigned LEB128 constrained to 64 bits.
    pub fn var_u64(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        for i in 0..10 {
            let byte = self.u8_raw("integer")?;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if i == 9 && byte > 0x01 {
                    return Err(WasmError::malformed("integer too large"));
                }
                return Ok(result);
            }
            shift += 7;
        }
        Err(WasmError::malformed("integer representation too long"))
    }

    /// Signed LEB128 constrained to `bits` bits (used for `i32`/`i64`
    /// constants and the 33-bit block type).
    pub fn var_signed(&mut self, bits: u32) -> Result<i64> {
        let max_bytes = bits.div_ceil(7);
        let mut result: i128 = 0;
        let mut shift: u32 = 0;
        for _ in 0..max_bytes {
            let byte = self.u8_raw("integer")?;
            result |= i128::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 128 && byte & 0x40 != 0 {
                    result |= -1i128 << shift;
                }
                let min = -(1i128 << (bits - 1));
                let max = (1i128 << (bits - 1)) - 1;
                if result < min || result > max {
                    return Err(WasmError::malformed("integer too large"));
                }
                return Ok(result as i64);
            }
        }
        Err(WasmError::malformed("integer representation too long"))
    }

    /// Signed 32-bit LEB128.
    pub fn var_i32(&mut self) -> Result<i32> {
        Ok(self.var_signed(32)? as i32)
    }

    /// Signed 64-bit LEB128.
    pub fn var_i64(&mut self) -> Result<i64> {
        self.var_signed(64)
    }

    /// Signed 33-bit LEB128 (block types).
    pub fn var_s33(&mut self) -> Result<i64> {
        self.var_signed(33)
    }

    /// Little-endian `f32` bit pattern.
    pub fn f32_bits(&mut self) -> Result<u32> {
        let b = self.bytes_n(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Little-endian `f64` bit pattern.
    pub fn f64_bits(&mut self) -> Result<u64> {
        let b = self.bytes_n(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// A length-prefixed UTF-8 name.
    pub fn name(&mut self) -> Result<String> {
        let len = self.var_u32()? as usize;
        let raw = self.bytes_n(len)?;
        match core::str::from_utf8(raw) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => Err(WasmError::malformed("malformed UTF-8 encoding")),
        }
    }

    /// A length-prefixed raw byte string.
    pub fn byte_vec(&mut self) -> Result<Vec<u8>> {
        let len = self.var_u32()? as usize;
        Ok(self.bytes_n(len)?.to_vec())
    }

    /// A length-prefixed vector decoded by `f`, with a plausibility guard so a
    /// bogus count cannot make us allocate gigabytes before hitting EOF.
    pub fn vec<T>(&mut self, what: &str, mut f: impl FnMut(&mut Reader<'a>) -> Result<T>) -> Result<Vec<T>> {
        let len = self.var_u32()? as usize;
        if len > self.remaining().saturating_add(1) && len > 1_000_000 {
            return Err(WasmError::malformed(format!("{} count too large", what)));
        }
        let mut out = Vec::with_capacity(len.min(1024));
        for _ in 0..len {
            out.push(f(self)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_leb_reads_multi_byte_values() {
        let mut r = Reader::new(&[0xE5, 0x8E, 0x26]);
        assert_eq!(r.var_u32().unwrap(), 624_485);
        assert!(r.is_empty());
    }

    #[test]
    fn overlong_unsigned_leb_is_malformed() {
        let mut r = Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]);
        assert_eq!(r.var_u32().unwrap_err().kind, crate::ErrorKind::Malformed);
    }

    #[test]
    fn unsigned_leb_rejects_bits_past_32() {
        let mut r = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
        assert!(r.var_u32().is_err());
    }

    #[test]
    fn signed_leb_sign_extends() {
        let mut r = Reader::new(&[0x7F]);
        assert_eq!(r.var_i32().unwrap(), -1);
        let mut r = Reader::new(&[0xC0, 0xBB, 0x78]);
        assert_eq!(r.var_i32().unwrap(), -123_456);
    }

    #[test]
    fn malformed_utf8_name_fails() {
        let mut r = Reader::new(&[0x02, 0xFF, 0xFE]);
        assert!(r.name().is_err());
    }
}
