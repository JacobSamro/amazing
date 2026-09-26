//! Minimal RFC 4648 base64, used by the WebSocket handshake and screenshots.
//!
//! Only the standard alphabet with `=` padding is supported. Encoding is used
//! for `Sec-WebSocket-Accept` and for `Page.captureScreenshot` data; decoding is
//! used by tests and by callers that accept a data URL.

/// Why a base64 string could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base64Error {
    /// The input length is not a multiple of four or has invalid padding.
    Length,
    /// A character is outside the standard alphabet.
    Alphabet,
}

impl std::fmt::Display for Base64Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Length => f.write_str("invalid base64 length or padding"),
            Self::Alphabet => f.write_str("invalid base64 character"),
        }
    }
}

impl std::error::Error for Base64Error {}

const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `bytes` with standard padding.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let x = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(DIGITS[(x >> 18) as usize] as char);
        out.push(DIGITS[((x >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            DIGITS[((x >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            DIGITS[(x & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Decodes a padded or unpadded standard-alphabet base64 string.
pub fn decode(text: &str) -> Result<Vec<u8>, Base64Error> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut pad = 0usize;
    for &b in bytes {
        if b == b'=' {
            pad += 1;
            continue;
        }
        if pad > 0 {
            return Err(Base64Error::Length);
        }
        let value = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(Base64Error::Alphabet),
        };
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // Padding, when present, must make the length a multiple of four; unpadded
    // input may never be 1 modulo 4.
    let total = bytes.len();
    if pad > 0 {
        if pad > 2 || total % 4 != 0 {
            return Err(Base64Error::Length);
        }
    } else if total % 4 == 1 {
        return Err(Base64Error::Length);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_known_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn round_trips_every_length_and_byte() {
        for len in 0..64usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(decode("Zg=").unwrap_err(), Base64Error::Length);
        assert_eq!(decode("Z===").unwrap_err(), Base64Error::Length);
        assert_eq!(decode("Zg!!").unwrap_err(), Base64Error::Alphabet);
        assert_eq!(decode("AAAAA").unwrap_err(), Base64Error::Length);
    }
}
