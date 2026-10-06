// Portions ported from tafia/quick-xml (src/encoding.rs) @
// 2eaa844b4b40117fb7ed4d1f512ede918125aeab, MIT.
// Copyright (c) 2016 Johann Tuffe. Full license: xml_encoding.LICENSE-MIT.
//! XML entity byte decoding: RFC 7303 authority and XML 1.0 Appendix F.
//! Encoding Standard codecs are reused in fatal mode; HTML meta sniffing and
//! replacement recovery never participate in an XML response.
use amazing_encoding::Encoding;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlDecodeError(pub String);
impl fmt::Display for XmlDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for XmlDecodeError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedXml { pub text: String, pub encoding: Encoding }
#[derive(Clone, Copy, PartialEq, Eq)]
enum ByteAuthority { Mime, DocumentEntity }

/// Decodes a complete XML entity. BOM is authoritative, then HTTP charset,
/// then the entity's declaration/autodetection; otherwise UTF-8 is required.
/// Unsupported codecs and malformed sequences fail without a partial string.
pub fn decode(bytes: &[u8], transport_label: Option<&str>) -> Result<DecodedXml, XmlDecodeError> {
    decode_with_authority(bytes, transport_label, ByteAuthority::Mime)
}

/// Decodes a document entity without a MIME/transport authority. XML 1.0
/// §4.3.3 requires its declaration to agree with the detected byte signature.
/// This is distinct from RFC 7303's MIME recovery rules used by `decode`.
pub fn decode_entity(bytes: &[u8]) -> Result<DecodedXml, XmlDecodeError> {
    decode_with_authority(bytes, None, ByteAuthority::DocumentEntity)
}

/// Decoded replacement text of a genuine external parsed entity. Its optional
/// TextDecl is validated and removed, never imported as a processing instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedExternalXml {
    pub text: String,
    pub encoding: Encoding,
    pub version: Option<String>,
}
pub fn decode_external_entity(bytes: &[u8], authority: &crate::xml_resources::XmlExternalEncoding)
    -> Result<DecodedExternalXml, XmlDecodeError> {
    use crate::xml_resources::XmlExternalEncoding;
    let parse = |text: &str| crate::xml_text_declaration::parse(text)
        .map_err(|error| XmlDecodeError(error.to_string()));
    if [[0, 0, 0xfe, 0xff], [0xff, 0xfe, 0, 0], [0, 0, 0xff, 0xfe], [0xfe, 0xff, 0, 0]]
        .iter().any(|marker| bytes.starts_with(marker)) {
        return Err(XmlDecodeError("unsupported UCS-4 XML encoding".into()));
    }
    if [[0, 0, 0, b'<'], [b'<', 0, 0, 0], [0, 0, b'<', 0], [0, b'<', 0, 0], [0x4c, 0x6f, 0xa7, 0x94]]
        .iter().any(|prefix| bytes.starts_with(prefix)) {
        return Err(XmlDecodeError("unsupported XML byte encoding".into()));
    }
    let found = detect_encoding(bytes);
    let bom = found.filter(|found| found.bom_len() != 0);
    let transport = match authority { XmlExternalEncoding::Entity => None, XmlExternalEncoding::Mime { charset } => charset.as_deref() };
    let selected = if let Some(found) = bom { found.encoding() }
    else if let Some(label) = transport {
        if label.trim().eq_ignore_ascii_case("utf-16") { return Err(XmlDecodeError("UTF-16 XML requires a byte order mark".into())); }
        codec(label)?
    } else {
        let initial = found.map_or(Encoding::Utf8, |found| found.encoding());
        if matches!(initial, Encoding::Utf16Le | Encoding::Utf16Be) {
            let end = bytes.chunks_exact(2).position(|pair| match initial {
                Encoding::Utf16Le => pair == [b'>', 0], _ => pair == [0, b'>'],
            }).ok_or_else(|| XmlDecodeError("unterminated external text declaration".into()))?;
            let prefix = decode_as(&bytes[..(end + 1) * 2], initial)?;
            let decl = parse(&prefix.text)?.ok_or_else(|| XmlDecodeError("unmarked UTF-16 entity requires explicit endian text declaration".into()))?;
            if decl.encoding.eq_ignore_ascii_case("utf-16") || codec(&decl.encoding)? != initial {
                return Err(XmlDecodeError("external text declaration conflicts with byte signature".into()));
            }
            initial
        } else if bytes.starts_with(b"<?xml") && bytes.get(5).is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
            let end = bytes.windows(2).position(|pair| pair == b"?>")
                .ok_or_else(|| XmlDecodeError("unterminated external text declaration".into()))?;
            let prefix = std::str::from_utf8(&bytes[..end + 2])
                .map_err(|_| XmlDecodeError("non-ASCII external text declaration".into()))?;
            let decl = parse(prefix)?.ok_or_else(|| XmlDecodeError("missing external text declaration".into()))?;
            if decl.encoding.eq_ignore_ascii_case("utf-16") { return Err(XmlDecodeError("UTF-16 XML requires a byte order mark".into())); }
            let selected = codec(&decl.encoding)?;
            if matches!(selected, Encoding::Utf16Le | Encoding::Utf16Be) {
                return Err(XmlDecodeError("external text declaration conflicts with byte signature".into()));
            }
            selected
        } else { initial }
    };
    let decoded = decode_as(&bytes[bom.map_or(0, |found| found.bom_len())..], selected)?;
    let declaration = parse(&decoded.text)?;
    if matches!(authority, XmlExternalEncoding::Entity) && bom.is_some() {
        if let Some(decl) = &declaration {
            let matches = if decl.encoding.eq_ignore_ascii_case("utf-16") {
                matches!(selected, Encoding::Utf16Le | Encoding::Utf16Be)
            } else { codec(&decl.encoding)? == selected };
            if !matches { return Err(XmlDecodeError("external text declaration conflicts with byte order mark".into())); }
        }
    }
    let version = declaration.as_ref().and_then(|decl| decl.version.clone());
    let start = declaration.map_or(0, |decl| decl.body_start);
    Ok(DecodedExternalXml { text: decoded.text[start..].into(), encoding: selected, version })
}

fn decode_with_authority(bytes: &[u8], transport_label: Option<&str>, authority: ByteAuthority)
    -> Result<DecodedXml, XmlDecodeError> {
    if [[0, 0, 0xfe, 0xff], [0xff, 0xfe, 0, 0], [0, 0, 0xff, 0xfe], [0xfe, 0xff, 0, 0]]
        .iter().any(|marker| bytes.starts_with(marker)) {
        return Err(XmlDecodeError("unsupported UCS-4 XML encoding".into()));
    }
    let detected = detect_encoding(bytes);
    if let Some(found) = detected.filter(|found| found.bom_len() != 0) {
        let decoded = decode_as(&bytes[found.bom_len()..], found.encoding())?;
        if authority == ByteAuthority::DocumentEntity {
            let declaration = crate::xml::encoding_declaration(&decoded.text)
                .map_err(|error| XmlDecodeError(error.to_string()))?;
            if let Some(label) = declaration {
                let matches = if label.eq_ignore_ascii_case("utf-16") {
                    matches!(decoded.encoding, Encoding::Utf16Le | Encoding::Utf16Be)
                } else { codec(&label)? == decoded.encoding };
                if !matches {
                    return Err(XmlDecodeError("XML declaration conflicts with byte order mark".into()));
                }
            }
        }
        return Ok(decoded);
    }
    if let Some(label) = transport_label {
        if label.trim().eq_ignore_ascii_case("utf-16") {
            return Err(XmlDecodeError("UTF-16 XML requires a byte order mark".into()));
        }
        return decode_as(bytes, codec(label)?);
    }
    if [[0, 0, 0, b'<'], [b'<', 0, 0, 0], [0, 0, b'<', 0], [0, b'<', 0, 0], [0x4c, 0x6f, 0xa7, 0x94]]
        .iter().any(|prefix| bytes.starts_with(prefix)) {
        return Err(XmlDecodeError("unsupported XML byte encoding".into()));
    }
    let initial = detected.map_or(Encoding::Utf8, |found| found.encoding());
    let declaration = if matches!(initial, Encoding::Utf16Le | Encoding::Utf16Be) {
        // Only the declaration must be tentatively decoded: later bytes may
        // belong to the explicitly declared codec, never replacement text.
        let end = bytes.chunks_exact(2).position(|pair| match initial {
            Encoding::Utf16Le => pair == [b'>', 0], _ => pair == [0, b'>'],
        }).ok_or_else(|| XmlDecodeError("unterminated XML encoding declaration".into()))?;
        let prefix = decode_as(&bytes[..(end + 1) * 2], initial)?;
        crate::xml::encoding_declaration(&prefix.text)
    } else if bytes.starts_with(b"<?xml") && bytes.get(5).is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
        let end = bytes.windows(2).position(|pair| pair == b"?>")
            .ok_or_else(|| XmlDecodeError("unterminated XML encoding declaration".into()))?;
        let prefix = std::str::from_utf8(&bytes[..end + 2])
            .map_err(|_| XmlDecodeError("non-ASCII XML encoding declaration".into()))?;
        crate::xml::encoding_declaration(prefix)
    } else { Ok(None) }.map_err(|error| XmlDecodeError(error.to_string()))?;
    let selected = match declaration.as_deref() {
        Some(label) => {
            if label.eq_ignore_ascii_case("utf-16") {
                return Err(XmlDecodeError("UTF-16 XML requires a byte order mark".into()));
            }
            codec(label)?
        }
        None if matches!(initial, Encoding::Utf16Le | Encoding::Utf16Be) => {
            return Err(XmlDecodeError("XML without a BOM requires an explicit endian encoding declaration".into()));
        }
        None => initial,
    };
    if matches!(initial, Encoding::Utf16Le | Encoding::Utf16Be) && selected != initial {
        return Err(XmlDecodeError("XML declaration conflicts with detected byte order".into()));
    }
    if authority == ByteAuthority::DocumentEntity && initial == Encoding::Utf8
        && matches!(selected, Encoding::Utf16Le | Encoding::Utf16Be) {
        return Err(XmlDecodeError("XML declaration conflicts with byte signature".into()));
    }
    decode_as(bytes, selected)
}

#[cfg(test)]
#[path = "xml_encoding_entity_tests.rs"]
mod entity_tests;
fn codec(label: &str) -> Result<Encoding, XmlDecodeError> {
    amazing_encoding::encoding_for_label(label)
        .filter(|encoding| *encoding != Encoding::Replacement)
        .ok_or_else(|| XmlDecodeError(format!("unsupported XML encoding: {label}")))
}
fn decode_as(bytes: &[u8], encoding: Encoding) -> Result<DecodedXml, XmlDecodeError> {
    amazing_encoding::decode(bytes, encoding, true)
        .map(|text| DecodedXml { text, encoding })
        .map_err(|error| XmlDecodeError(format!("malformed XML bytes at {}", error.byte_offset)))
}

// Complete supported byte-detection area adapted from quick-xml's detector.
#[derive(Clone, Copy)]
enum DetectedEncoding { AsciiCompatible, Utf8Bom, Utf16LeLike, Utf16LeBom, Utf16BeLike, Utf16BeBom }
fn detect_encoding(bytes: &[u8]) -> Option<DetectedEncoding> {
    match bytes {
        _ if bytes.starts_with(&[0xfe, 0xff]) => Some(DetectedEncoding::Utf16BeBom),
        _ if bytes.starts_with(&[0xff, 0xfe]) => Some(DetectedEncoding::Utf16LeBom),
        _ if bytes.starts_with(&[0xef, 0xbb, 0xbf]) => Some(DetectedEncoding::Utf8Bom),
        _ if bytes.starts_with(&[0, b'<', 0, b'?']) => Some(DetectedEncoding::Utf16BeLike),
        _ if bytes.starts_with(&[b'<', 0, b'?', 0]) => Some(DetectedEncoding::Utf16LeLike),
        _ if bytes.starts_with(b"<?xm") => Some(DetectedEncoding::AsciiCompatible),
        _ => None,
    }
}
impl DetectedEncoding {
    fn encoding(self) -> Encoding {
        match self {
            Self::AsciiCompatible | Self::Utf8Bom => Encoding::Utf8,
            Self::Utf16LeLike | Self::Utf16LeBom => Encoding::Utf16Le,
            Self::Utf16BeLike | Self::Utf16BeBom => Encoding::Utf16Be,
        }
    }
    fn bom_len(self) -> usize {
        match self { Self::Utf8Bom => 3, Self::Utf16LeBom | Self::Utf16BeBom => 2, _ => 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_byte_authorities_and_fatal_boundaries() {
        let declared = b"<?xml version='1.0' encoding='windows-1253'?><r>\xa2</r>";
        assert_eq!(decode(declared, None).unwrap().text.chars().rev().nth(4), Some('\u{0386}'));
        assert!(decode(declared, Some("utf-8")).is_err());
        assert_eq!(decode("<r>é</r>".as_bytes(), None).unwrap().encoding, Encoding::Utf8);
        assert!(decode(b"<r>\xe9</r>", None).is_err());
        assert!(decode(b"<r><meta charset='windows-1252'/>\xe9</r>", None).is_err());
        let bom = [b"\xef\xbb\xbf".as_slice(), "<r>é</r>".as_bytes()].concat();
        assert_eq!(decode(&bom, Some("bogus")).unwrap().encoding, Encoding::Utf8);
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
            let label = if encoding == Encoding::Utf16Le { "UTF-16LE" } else { "UTF-16BE" };
            let text = format!("<?xml version='1.0' encoding='{label}'?><r>é</r>");
            let bytes = amazing_encoding::encode(&text, encoding, true).unwrap();
            assert_eq!(decode(&bytes, None).unwrap().encoding, encoding);
            let marker = if encoding == Encoding::Utf16Le { [0xff, 0xfe] } else { [0xfe, 0xff] };
            assert_eq!(decode(&[marker.as_slice(), bytes.as_slice()].concat(), Some("utf-8")).unwrap().encoding, encoding);
        }
        for bytes in [b"\xff\xfe\0\0<\0\0\0".as_slice(), b"\0\0\xfe\xff\0\0\0<", b"\x4c\x6f\xa7\x94", b"\xff", b"\xc0\xaf"] {
            assert!(decode(bytes, None).is_err(), "{bytes:?}");
        }
        for label in ["unknown", "replacement"] { assert!(decode(b"<r/>", Some(label)).is_err()); }
        assert_eq!(decode(b"<r>\x80</r>", Some("x-user-defined")).unwrap().text, "<r>\u{f780}</r>");
        assert!(decode(b"<?xml version='1.0' encoding='unknown'?><r/>", None).is_err());
        assert!(decode(b"<?xml version='1.0' encoding='UTF-16'?><r/>", None).is_err());
        assert_eq!(decode(b"<?xml version='1.0' encoding='unknown'?><r/>", Some("utf-8")).unwrap().encoding, Encoding::Utf8);
        assert!(decode(b"<?xml version='1.0' encoding='utf 8'?><r/>", None).is_err());
        assert!(decode(&[0xff, 0xfe, b'<', 0, b'r'], None).is_err());
    }
}
