//! Complete-buffer HTML byte decoding, with a deliberately bounded encoding set.
//!
//! Supported decoders: UTF-8, UTF-16LE, UTF-16BE, and windows-1252. The
//! accepted labels are enumerated by `label`; other labels are ignored during
//! sniffing. The default is windows-1252, matching the HTML fallback for an
//! unspecified locale. The prescan examines only the first 1024 bytes and
//! recognizes ordinary meta charset and content-type declarations. It does
//! not implement the complete Encoding label table or streaming reparses.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1252,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Bom,
    Transport,
    Meta,
    Utf16XmlPreamble,
    Default,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DecodedHtml {
    pub text: String,
    pub encoding: Encoding,
    pub source: Source,
}

/// Decode a complete HTML input using BOM > transport > prescan > fallback.
/// Malformed byte sequences become U+FFFD. A leading BOM is consumed.
pub fn decode(bytes: &[u8], transport_label: Option<&str>) -> DecodedHtml {
    let (encoding, source, skip) = sniff(bytes, transport_label);
    let bytes = &bytes[skip..];
    let text = match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        Encoding::Utf16Le => decode_utf16(bytes, true),
        Encoding::Utf16Be => decode_utf16(bytes, false),
        Encoding::Windows1252 => bytes.iter().map(|&byte| windows1252(byte)).collect(),
    };
    DecodedHtml { text, encoding, source }
}

// Both complete and incremental input use this exact choice function. The
// caller supplies at most the first 1024 bytes when streaming.
pub(crate) fn sniff(bytes: &[u8], transport_label: Option<&str>) -> (Encoding, Source, usize) {
    let bom = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        Some((Encoding::Utf8, 3))
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some((Encoding::Utf16Be, 2))
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        Some((Encoding::Utf16Le, 2))
    } else {
        None
    };
    let (encoding, source, skip) = if let Some((encoding, skip)) = bom {
        (encoding, Source::Bom, skip)
    } else if let Some(encoding) = transport_label.and_then(label) {
        (encoding, Source::Transport, 0)
    } else if let Some(encoding) = xml_utf16_preamble(bytes) {
        (encoding, Source::Utf16XmlPreamble, 0)
    } else if let Some(encoding) = prescan_meta(bytes) {
        (encoding, Source::Meta, 0)
    } else {
        (Encoding::Windows1252, Source::Default, 0)
    };
    (encoding, source, skip)
}

fn label(raw: &str) -> Option<Encoding> {
    let label = raw.trim_matches(|ch: char| matches!(ch, '\t' | '\n' | '\x0c' | '\r' | ' '));
    if [
        "utf-8",
        "utf8",
        "unicode-1-1-utf-8",
        "unicode11utf8",
        "unicode20utf8",
        "x-unicode20utf8",
    ]
    .iter()
    .any(|name| label.eq_ignore_ascii_case(name))
    {
        Some(Encoding::Utf8)
    } else if ["utf-16", "utf-16le"]
        .iter()
        .any(|name| label.eq_ignore_ascii_case(name))
    {
        Some(Encoding::Utf16Le)
    } else if label.eq_ignore_ascii_case("utf-16be") {
        Some(Encoding::Utf16Be)
    } else if [
        "windows-1252",
        "cp1252",
        "x-cp1252",
        "iso-8859-1",
        "latin1",
        "l1",
        "us-ascii",
        "ascii",
        "iso8859-1",
        "iso88591",
        "iso_8859-1",
        "csisolatin1",
        "ibm819",
        "cp819",
        "iso-ir-100",
        "ansi_x3.4-1968",
    ]
    .iter()
    .any(|name| label.eq_ignore_ascii_case(name))
    {
        Some(Encoding::Windows1252)
    } else {
        None
    }
}

fn xml_utf16_preamble(bytes: &[u8]) -> Option<Encoding> {
    if bytes.starts_with(b"<\0?\0x\0") {
        Some(Encoding::Utf16Le)
    } else if bytes.starts_with(b"\0<\0?\0x") {
        Some(Encoding::Utf16Be)
    } else {
        None
    }
}

fn prescan_meta(bytes: &[u8]) -> Option<Encoding> {
    let bytes = &bytes[..bytes.len().min(1024)];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            if let Some(end) = bytes[i + 4..].windows(3).position(|s| s == b"-->") {
                i += 4 + end + 3;
                continue;
            }
            break;
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if bytes
            .get(i + 1..i + 5)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"meta"))
            && bytes
                .get(i + 5)
                .is_some_and(|b| ascii_space(*b) || *b == b'/')
        {
            i += 5;
            let mut charset = None;
            let mut content = None;
            let mut pragma = false;
            while i < bytes.len() && bytes[i] != b'>' {
                while i < bytes.len() && (ascii_space(bytes[i]) || bytes[i] == b'/') {
                    i += 1;
                }
                if i >= bytes.len() || bytes[i] == b'>' {
                    break;
                }
                let start = i;
                while i < bytes.len()
                    && !ascii_space(bytes[i])
                    && !matches!(bytes[i], b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                if i == start {
                    i += 1;
                    continue;
                }
                let name = &bytes[start..i];
                while i < bytes.len() && ascii_space(bytes[i]) {
                    i += 1;
                }
                let mut value = &b""[..];
                if i < bytes.len() && bytes[i] == b'=' {
                    i += 1;
                    while i < bytes.len() && ascii_space(bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() && matches!(bytes[i], b'\'' | b'"') {
                        let quote = bytes[i];
                        i += 1;
                        let start = i;
                        while i < bytes.len() && bytes[i] != quote {
                            i += 1;
                        }
                        value = &bytes[start..i];
                        if i < bytes.len() {
                            i += 1;
                        }
                    } else {
                        let start = i;
                        while i < bytes.len() && !ascii_space(bytes[i]) && bytes[i] != b'>' {
                            i += 1;
                        }
                        value = &bytes[start..i];
                    }
                }
                if name.eq_ignore_ascii_case(b"charset") && charset.is_none() {
                    charset = Some(value);
                } else if name.eq_ignore_ascii_case(b"content") && content.is_none() {
                    content = Some(value);
                } else if name.eq_ignore_ascii_case(b"http-equiv")
                    && value.eq_ignore_ascii_case(b"content-type")
                {
                    pragma = true;
                }
            }
            let declared = charset.or_else(|| {
                if pragma {
                    content.and_then(extract_content_charset)
                } else {
                    None
                }
            });
            if let Some(encoding) = declared
                .and_then(|value| std::str::from_utf8(value).ok())
                .and_then(label)
            {
                return Some(match encoding {
                    Encoding::Utf16Le | Encoding::Utf16Be => Encoding::Utf8,
                    other => other,
                });
            }
        } else {
            i += 1;
        }
    }
    None
}

fn extract_content_charset(value: &[u8]) -> Option<&[u8]> {
    if value.len() < 7 {
        return None;
    }
    for i in 0..=value.len() - 7 {
        if value[i..]
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"charset"))
        {
            let mut j = i + 7;
            while j < value.len() && ascii_space(value[j]) {
                j += 1;
            }
            if value.get(j) != Some(&b'=') {
                continue;
            }
            j += 1;
            while j < value.len() && ascii_space(value[j]) {
                j += 1;
            }
            let quote = value.get(j).copied().filter(|b| matches!(b, b'\'' | b'"'));
            if quote.is_some() {
                j += 1;
            }
            let start = j;
            while j < value.len()
                && !ascii_space(value[j])
                && value[j] != b';'
                && Some(value[j]) != quote
            {
                j += 1;
            }
            return Some(&value[start..j]);
        }
    }
    None
}

fn ascii_space(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn decode_utf16(bytes: &[u8], little: bool) -> String {
    let units = bytes.chunks_exact(2);
    let odd = !units.remainder().is_empty();
    let mut result: String = char::decode_utf16(units.map(|pair| {
        if little {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    }))
    .map(|value| value.unwrap_or('\u{fffd}'))
    .collect();
    if odd {
        result.push('\u{fffd}');
    }
    result
}

pub(crate) fn windows1252(byte: u8) -> char {
    const C1: [u16; 32] = [
        0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x008d, 0x017d, 0x008f, 0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x009d, 0x017e, 0x0178,
    ];
    char::from_u32(if (0x80..=0x9f).contains(&byte) {
        C1[(byte - 0x80) as usize] as u32
    } else {
        byte as u32
    })
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, parse_bytes};
    use amazing_dom::NodeIdx;

    #[test]
    fn bom_beats_transport_and_meta_and_is_removed() {
        let bytes = b"\xef\xbb\xbf<meta charset=windows-1252><p>\xc3\xa9";
        let decoded = decode(bytes, Some("utf-16le"));
        assert_eq!(
            (decoded.encoding, decoded.source),
            (Encoding::Utf8, Source::Bom)
        );
        assert!(decoded.text.ends_with("<p>é"));
        assert!(!decoded.text.starts_with('\u{feff}'));
    }

    #[test]
    fn utf16_boms_and_invalid_units() {
        assert_eq!(
            decode(
                &[0xff, 0xfe, b'<', 0, b'p', 0, b'>', 0, 0x3d, 0xd8, b'x'],
                None
            )
            .text,
            "<p>��"
        );
        assert_eq!(
            decode(&[0xfe, 0xff, 0, b'<', 0, b'p', 0, b'>'], Some("utf-8")).text,
            "<p>"
        );
    }

    #[test]
    fn transport_beats_meta_and_labels_are_trimmed_case_insensitively() {
        let bytes = b"<meta charset=utf-8><p>\x80";
        let decoded = decode(bytes, Some("  LaTiN1\t"));
        assert_eq!(
            (decoded.encoding, decoded.source),
            (Encoding::Windows1252, Source::Transport)
        );
        assert!(decoded.text.ends_with("<p>€"));
        assert_eq!(decode(bytes, Some("nonsense")).source, Source::Meta);
    }

    #[test]
    fn meta_charset_and_pragma_and_false_matches() {
        let direct = decode(b"<!-- <meta charset=latin1> --><metadata charset=latin1><meta CHARSET='utf8'><p>\xc3\xa9", None);
        assert_eq!(
            (direct.encoding, direct.source),
            (Encoding::Utf8, Source::Meta)
        );
        assert!(direct.text.ends_with("<p>é"));
        let pragma = decode(
            b"<meta content='text/html; charset=UTF-8' http-equiv='CONTENT-TYPE'><p>\xc3\xa9",
            None,
        );
        assert_eq!(
            (pragma.encoding, pragma.source),
            (Encoding::Utf8, Source::Meta)
        );
        let no_pragma = decode(b"<meta content='text/html; charset=UTF-8'><p>\x80", None);
        assert_eq!(
            (no_pragma.encoding, no_pragma.source),
            (Encoding::Windows1252, Source::Default)
        );
        assert!(no_pragma.text.ends_with("<p>€"));
    }

    #[test]
    fn meta_utf16_label_maps_to_utf8_and_preamble_detects_utf16() {
        assert_eq!(
            decode(b"<meta charset=utf-16><p>\xc3\xa9", None).encoding,
            Encoding::Utf8
        );
        assert_eq!(decode(b"<\0?\0x\0m\0l\0", None).encoding, Encoding::Utf16Le);
    }

    #[test]
    fn prescan_limit_and_malformed_utf8() {
        let mut bytes = vec![b' '; 1024];
        bytes.extend_from_slice(b"<meta charset=utf-8><p>\xc3\xa9");
        assert_eq!(decode(&bytes, None).source, Source::Default);
        assert_eq!(decode(b"<p>\xc3(", Some("utf-8")).text, "<p>�(");
    }

    #[test]
    fn parsed_bytes_and_existing_string_entry_have_same_tree() {
        let bytes = b"<p>\xc3\xa9\r\nx";
        let from_bytes = parse_bytes(bytes, Some("utf-8"));
        let from_string = parse("<p>é\r\nx");
        assert_eq!(from_bytes.dump(NodeIdx(0)), from_string.dump(NodeIdx(0)));
    }
}
