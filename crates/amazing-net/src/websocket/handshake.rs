use super::{Error, Options};
use std::io::{Read, Write};
pub(crate) const MAX_HANDSHAKE: usize = 16384;
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
fn value(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| matches!(b, 0x21..=0x7e))
}
pub(crate) fn validate_request(host: &str, target: &str, options: &Options) -> Result<(), Error> {
    if !value(host) || !target.starts_with('/') || !value(target) || target.contains('#') {
        return Err(Error::InvalidInput);
    }
    let mut size = 256usize
        .checked_add(host.len())
        .and_then(|n| n.checked_add(target.len()))
        .ok_or(Error::Limit)?;
    if let Some(origin) = &options.origin {
        if !value(origin) {
            return Err(Error::InvalidInput);
        }
        size = size.checked_add(origin.len() + 10).ok_or(Error::Limit)?;
    }
    // Count bound precedes duplicate scans and request serialization.
    if options.protocols.len() > 32 {
        return Err(Error::Limit);
    }
    for (i, p) in options.protocols.iter().enumerate() {
        if p.len() > 256 {
            return Err(Error::Limit);
        }
        if !token(p) || options.protocols[..i].contains(p) {
            return Err(Error::InvalidInput);
        }
        size = size.checked_add(p.len() + 2).ok_or(Error::Limit)?;
    }
    if size > MAX_HANDSHAKE {
        return Err(Error::Limit);
    }
    Ok(())
}
pub(super) fn perform(
    io: &mut (impl Read + Write),
    host: &str,
    target: &str,
    options: &Options,
    random: fn(&mut [u8]) -> Result<(), Error>,
) -> Result<Option<String>, Error> {
    validate_request(host, target, options)?;
    let mut nonce = [0; 16];
    random(&mut nonce)?;
    let key = base64(&nonce);
    let mut request=format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n");
    if let Some(origin) = &options.origin {
        request.push_str("Origin: ");
        request.push_str(origin);
        request.push_str("\r\n");
    }
    if !options.protocols.is_empty() {
        request.push_str("Sec-WebSocket-Protocol: ");
        request.push_str(&options.protocols.join(", "));
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    io.write_all(request.as_bytes())?;
    io.flush()?;
    let bytes = read_headers(io)?;
    validate_response(&bytes, &accept(&key), &options.protocols)
}
pub(super) fn read_headers(io: &mut impl Read) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    // Single-byte reads deliberately avoid swallowing the first WebSocket frame
    // coalesced with the HTTP response. TLS already buffers decrypted records.
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() == MAX_HANDSHAKE {
            return Err(Error::Limit);
        }
        let mut b = [0];
        io.read_exact(&mut b)?;
        bytes.try_reserve_exact(1).map_err(|_| Error::Limit)?;
        bytes.push(b[0]);
    }
    Ok(bytes)
}
pub(crate) fn validate_response(
    bytes: &[u8],
    expected: &str,
    offered: &[String],
) -> Result<Option<String>, Error> {
    if bytes.len() > MAX_HANDSHAKE {
        return Err(Error::Limit);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Handshake)?;
    if !text.ends_with("\r\n\r\n") {
        return Err(Error::Handshake);
    }
    let mut lines = text[..text.len() - 4].split("\r\n");
    let status = lines.next().ok_or(Error::Handshake)?;
    let mut parts = status.splitn(3, ' ');
    if parts.next() != Some("HTTP/1.1")
        || parts.next() != Some("101")
        || status.bytes().any(|b| b < 32 || b == 127)
    {
        return Err(Error::Handshake);
    }
    let mut upgrade = false;
    let mut connection = false;
    let mut accepted = false;
    let mut selected = None;
    for (i, line) in lines.enumerate() {
        if i >= 128 {
            return Err(Error::Limit);
        }
        let (name, value) = line.split_once(':').ok_or(Error::Handshake)?;
        if !token(name) || value.bytes().any(|b| b < 32 && b != 9 || b == 127) {
            return Err(Error::Handshake);
        }
        let value = value.trim_matches([' ', '\t']);
        if name.eq_ignore_ascii_case("upgrade") {
            if upgrade || !value.eq_ignore_ascii_case("websocket") {
                return Err(Error::Handshake);
            }
            upgrade = true;
        } else if name.eq_ignore_ascii_case("connection") {
            for part in value.split(',') {
                let part = part.trim_matches([' ', '\t']);
                if !token(part) {
                    return Err(Error::Handshake);
                }
                connection |= part.eq_ignore_ascii_case("upgrade");
            }
        } else if name.eq_ignore_ascii_case("sec-websocket-accept") {
            if accepted || value != expected {
                return Err(Error::Handshake);
            }
            accepted = true;
        } else if name.eq_ignore_ascii_case("sec-websocket-protocol") {
            if selected.is_some() || !token(value) || !offered.iter().any(|p| p == value) {
                return Err(Error::Handshake);
            }
            selected = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("sec-websocket-extensions")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            return Err(Error::Handshake);
        }
    }
    if !upgrade || !connection || !accepted {
        return Err(Error::Handshake);
    }
    Ok(selected)
}
pub(crate) fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for b in bytes.chunks(3) {
        let x = (u32::from(b[0]) << 16)
            | (u32::from(*b.get(1).unwrap_or(&0)) << 8)
            | u32::from(*b.get(2).unwrap_or(&0));
        out.push(DIGITS[(x >> 18) as usize] as char);
        out.push(DIGITS[((x >> 12) & 63) as usize] as char);
        out.push(if b.len() > 1 {
            DIGITS[((x >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if b.len() > 2 {
            DIGITS[(x & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
/// SHA-1 only for RFC6455's fixed 24-byte key plus GUID challenge. This is not
/// an authentication primitive, signature, password hash or general crypto API.
pub(crate) fn accept(key: &str) -> String {
    assert_eq!(key.len(), 24);
    let mut input = [0u8; 128];
    input[..24].copy_from_slice(key.as_bytes());
    input[24..60].copy_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    input[60] = 128;
    input[120..].copy_from_slice(&480u64.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for chunk in input.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, b) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(b.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
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
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut digest = [0; 20];
    for (part, word) in digest.chunks_exact_mut(4).zip(h) {
        part.copy_from_slice(&word.to_be_bytes());
    }
    base64(&digest)
}
