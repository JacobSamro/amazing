//! Bounded RFC 1950/1951/1952 decoding. No dictionaries, raw-deflate HTTP
//! fallback. Brotli crosses the isolated approved adapter. Each coding layer
//! has the caller's output ceiling.
mod deflate;
mod inflate;
pub(crate) mod stream;
use crate::NetError;
use inflate::{inflate, Output};

fn invalid(message: &str) -> NetError {
    NetError::MalformedResponse(format!("content coding: {message}"))
}

pub(crate) fn decode(input: &[u8], coding: &str, limit: usize) -> Result<Vec<u8>, NetError> {
    match coding {
        "gzip" | "x-gzip" => gzip(input, limit),
        "deflate" => zlib(input, limit),
        "br" => amazing_net_brotli::decode(input, limit).map_err(|error| match error {
            amazing_net_brotli::DecodeError::OutputLimit => NetError::ResponseTooLarge,
            amazing_net_brotli::DecodeError::Truncated => NetError::IncompleteBody,
            _ => invalid(&error.to_string()),
        }),
        _ => Err(invalid("unsupported encoding")),
    }
}

fn gzip(input: &[u8], limit: usize) -> Result<Vec<u8>, NetError> {
    gzip_with(input, limit, false).map(|(bytes, _)| bytes)
}
fn zlib(input: &[u8], limit: usize) -> Result<Vec<u8>, NetError> {
    zlib_with(input, limit, false).map(|(bytes, _)| bytes)
}

fn take<'a>(input: &'a [u8], pos: &mut usize, count: usize) -> Result<&'a [u8], NetError> {
    let end = pos
        .checked_add(count)
        .ok_or_else(|| invalid("offset overflow"))?;
    let data = input.get(*pos..end).ok_or(NetError::IncompleteBody)?;
    *pos = end;
    Ok(data)
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("four bytes"))
}

fn gzip_with(input: &[u8], limit: usize, allow_trailing: bool) -> Result<(Vec<u8>, usize), NetError> {
    let mut pos = 0;
    let mut out = Output::new(limit);
    // RFC 1952 members concatenate, but dictionaries never cross members.
    loop {
        let start = pos;
        let head = take(input, &mut pos, 10)?;
        if head[..3] != [0x1f, 0x8b, 8] || head[3] & 0xe0 != 0 {
            return Err(invalid("invalid gzip header"));
        }
        let flags = head[3];
        if flags & 4 != 0 {
            let size = take(input, &mut pos, 2)?;
            let size = u16::from_le_bytes([size[0], size[1]]) as usize;
            take(input, &mut pos, size)?;
        }
        for flag in [8, 16] {
            if flags & flag != 0 {
                let len = input[pos..]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or(NetError::IncompleteBody)?;
                take(input, &mut pos, len + 1)?;
            }
        }
        if flags & 2 != 0 {
            let actual = crc32(&input[start..pos]) as u16;
            let bytes = take(input, &mut pos, 2)?;
            if u16::from_le_bytes([bytes[0], bytes[1]]) != actual {
                return Err(invalid("gzip header CRC mismatch"));
            }
        }
        let member_start = out.bytes.len();
        pos += inflate(&input[pos..], &mut out, 32768)?;
        let trailer = take(input, &mut pos, 8)?;
        let member = &out.bytes[member_start..];
        if crc32(member) != le32(&trailer[..4]) {
            return Err(invalid("gzip CRC32 mismatch"));
        }
        if member.len() as u32 != le32(&trailer[4..]) {
            return Err(invalid("gzip ISIZE mismatch"));
        }
        if pos == input.len() || (allow_trailing && !input[pos..].starts_with(&[0x1f, 0x8b])) {
            return Ok((out.bytes, pos));
        }
        // Trailing garbage is not a gzip member and fails on its header.
    }
}

fn zlib_with(input: &[u8], limit: usize, allow_trailing: bool) -> Result<(Vec<u8>, usize), NetError> {
    let mut pos = 0;
    let head = take(input, &mut pos, 2)?;
    if head[0] & 15 != 8 || head[0] >> 4 > 7 || u16::from_be_bytes([head[0], head[1]]) % 31 != 0 {
        return Err(invalid("invalid zlib header"));
    }
    if head[1] & 32 != 0 {
        return Err(invalid("zlib preset dictionary unsupported"));
    }
    let window = 1usize << ((head[0] >> 4) + 8);
    let mut out = Output::new(limit);
    pos += inflate(&input[pos..], &mut out, window)?;
    let expected = u32::from_be_bytes(take(input, &mut pos, 4)?.try_into().unwrap());
    if adler32(&out.bytes) != expected {
        return Err(invalid("zlib Adler32 mismatch"));
    }
    if pos != input.len() && !allow_trailing {
        return Err(invalid("trailing zlib data"));
    }
    Ok((out.bytes, pos))
}

fn crc32(bytes: &[u8]) -> u32 {
    !crc32_update(!0, bytes)
}
pub(super) fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut bit = 0;
            while bit < 8 {
                c = (c >> 1) ^ if c & 1 != 0 { 0xedb88320 } else { 0 };
                bit += 1;
            }
            table[i] = c;
            i += 1;
        }
        table
    };
    for &byte in bytes {
        crc = (crc >> 8) ^ TABLE[((crc as u8) ^ byte) as usize];
    }
    crc
}
fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
pub(crate) mod tests;

/// `CompressionStream` formats: "gzip", "deflate" (zlib) and "deflate-raw".
pub fn compress_format(input: &[u8], format: &str) -> Option<Vec<u8>> {
    let raw = deflate::deflate_raw(input);
    match format {
        "deflate-raw" => Some(raw),
        "deflate" => {
            let mut out = vec![0x78, 0x01];
            out.extend_from_slice(&raw);
            out.extend_from_slice(&adler32(input).to_be_bytes());
            Some(out)
        }
        "gzip" => {
            let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
            out.extend_from_slice(&raw);
            out.extend_from_slice(&crc32(input).to_le_bytes());
            out.extend_from_slice(&(input.len() as u32).to_le_bytes());
            Some(out)
        }
        "brotli" => Some(deflate::brotli_uncompressed(input)),
        _ => None,
    }
}

/// `DecompressionStream` formats. Returns the decoded bytes and how many input
/// bytes the compressed stream occupied; anything after that is trailing data.
pub fn decompress_prefix(input: &[u8], format: &str, limit: usize) -> Result<(Vec<u8>, usize), NetError> {
    match format {
        "gzip" => gzip_with(input, limit, true),
        "deflate" => zlib_with(input, limit, true),
        "deflate-raw" => {
            let mut out = Output::new(limit);
            let used = inflate(input, &mut out, 32768)?;
            Ok((out.bytes, used))
        }
        "brotli" => amazing_net_brotli::decode(input, limit)
            .map(|bytes| (bytes, input.len()))
            .map_err(|error| match error {
                amazing_net_brotli::DecodeError::OutputLimit => NetError::ResponseTooLarge,
                amazing_net_brotli::DecodeError::Truncated => NetError::IncompleteBody,
                _ => invalid(&error.to_string()),
            }),
        _ => Err(invalid("unsupported format")),
    }
}

/// Whole-input variant: any trailing data is an error.
pub fn decompress_format(input: &[u8], format: &str, limit: usize) -> Result<Vec<u8>, NetError> {
    let (bytes, used) = decompress_prefix(input, format, limit)?;
    if used != input.len() {
        return Err(invalid("trailing data"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod compression_stream_tests {
    use super::*;

    #[test]
    fn compress_then_decompress_round_trips_every_format() {
        let mut data = Vec::new();
        for i in 0..20000u32 {
            data.extend_from_slice(format!("line {} of the corpus {}\n", i % 97, i % 13).as_bytes());
        }
        for input in [&b""[..], &b"a"[..], &b"hello hello hello hello"[..], &data[..]] {
            for format in ["gzip", "deflate", "deflate-raw", "brotli"] {
                let packed = compress_format(input, format).unwrap();
                assert_eq!(decompress_format(&packed, format, 1 << 26).unwrap(), input, "{format}");
                if packed.len() > 4 {
                    assert!(decompress_format(&packed[..packed.len() - 1], format, 1 << 26).is_err());
                }
            }
        }
        assert!(compress_format(&data, "gzip").unwrap().len() < data.len() / 3);
    }
}
