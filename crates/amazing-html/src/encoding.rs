//! Complete-buffer HTML byte decoding using WHATWG Encoding codecs.
//!
//! Labels use the complete pinned Encoding table. The default is windows-1252.
//! The prescan examines the first 1024 bytes for ordinary meta charset and
//! content-type declarations; it does not implement streaming reparses.

pub use amazing_encoding::Encoding;

/// Encoding Standard name exposed by Document.characterSet and legacy aliases.
/// TextDecoder.encoding deliberately uses the lowercase Encoding::name instead.
pub fn canonical_name(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Ibm866 => "IBM866",
        Encoding::Iso88592 => "ISO-8859-2",
        Encoding::Iso88593 => "ISO-8859-3",
        Encoding::Iso88594 => "ISO-8859-4",
        Encoding::Iso88595 => "ISO-8859-5",
        Encoding::Iso88596 => "ISO-8859-6",
        Encoding::Iso88597 => "ISO-8859-7",
        Encoding::Iso88598 => "ISO-8859-8",
        Encoding::Iso88598I => "ISO-8859-8-I",
        Encoding::Iso885910 => "ISO-8859-10",
        Encoding::Iso885913 => "ISO-8859-13",
        Encoding::Iso885914 => "ISO-8859-14",
        Encoding::Iso885915 => "ISO-8859-15",
        Encoding::Iso885916 => "ISO-8859-16",
        Encoding::Koi8R => "KOI8-R",
        Encoding::Koi8U => "KOI8-U",
        Encoding::Gbk => "GBK",
        Encoding::Big5 => "Big5",
        Encoding::EucJp => "EUC-JP",
        Encoding::Iso2022Jp => "ISO-2022-JP",
        Encoding::ShiftJis => "Shift_JIS",
        Encoding::EucKr => "EUC-KR",
        Encoding::Utf16Be => "UTF-16BE",
        Encoding::Utf16Le => "UTF-16LE",
        Encoding::Macintosh | Encoding::Windows874 | Encoding::Windows1250
        | Encoding::Windows1251 | Encoding::Windows1252 | Encoding::Windows1253
        | Encoding::Windows1254 | Encoding::Windows1255 | Encoding::Windows1256
        | Encoding::Windows1257 | Encoding::Windows1258 | Encoding::XMacCyrillic
        | Encoding::Gb18030 | Encoding::Replacement | Encoding::XUserDefined => encoding.name(),
    }
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
    decode_with_fallback(bytes, transport_label, Encoding::Windows1252)
}

/// Caller-selected tentative fallback, used only after BOM, transport and meta.
/// Browsing-context origin and UTF-16 inheritance checks belong to the caller.
pub fn decode_with_fallback(bytes: &[u8], transport_label: Option<&str>, fallback: Encoding) -> DecodedHtml {
    let (encoding, source, skip) = sniff_with_fallback(bytes, transport_label, fallback);
    let bytes = &bytes[skip..];
    let text = amazing_encoding::decode(bytes, encoding, false).expect("replacement mode cannot fail");
    DecodedHtml { text, encoding, source }
}

// Both complete and incremental input use this exact choice function. The
// caller supplies at most the first 1024 bytes when streaming.
pub(crate) fn sniff(bytes: &[u8], transport_label: Option<&str>) -> (Encoding, Source, usize) {
    sniff_with_fallback(bytes, transport_label, Encoding::Windows1252)
}

fn sniff_with_fallback(bytes: &[u8], transport_label: Option<&str>, fallback: Encoding) -> (Encoding, Source, usize) {
    let bom = amazing_encoding::sniff_bom(bytes);
    let (encoding, source, skip) = if let Some((encoding, skip)) = bom {
        (encoding, Source::Bom, skip)
    } else if let Some(encoding) = transport_label.and_then(label) {
        (encoding, Source::Transport, 0)
    } else if let Some(encoding) = xml_utf16_preamble(bytes) {
        (encoding, Source::Utf16XmlPreamble, 0)
    } else if let Some(encoding) = prescan_meta(bytes) {
        (encoding, Source::Meta, 0)
    } else {
        (fallback, Source::Default, 0)
    };
    (encoding, source, skip)
}

fn label(raw: &str) -> Option<Encoding> {
    amazing_encoding::encoding_for_label(raw)
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
                    Encoding::XUserDefined => Encoding::Windows1252,
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
            if let Some(quote) = quote {
                let end = value[start..].iter().position(|byte| *byte == quote)?;
                return Some(&value[start..start + end]);
            }
            while j < value.len()
                && !ascii_space(value[j])
                && value[j] != b';'
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, parse_bytes};
    use amazing_dom::NodeIdx;

    #[test]
    fn caller_fallback_preserves_bom_transport_meta_and_default_precedence() {
        for fallback in [Encoding::Utf8, Encoding::Windows1253, Encoding::Windows1252] {
            for (body, transport, expected, source) in [
                (&b"<p>\xa2"[..], None, fallback, Source::Default),
                (&b"<meta charset=bogus><p>\xa2"[..], Some("bogus"), fallback, Source::Default),
                (&b"<meta charset=windows-1252><p>\xa2"[..], None, Encoding::Windows1252, Source::Meta),
                (&b"<meta charset=UTF-8><p>\xa2"[..], Some("windows-1253"), Encoding::Windows1253, Source::Transport),
                (&b"\xef\xbb\xbf<p>\xa2"[..], Some("windows-1253"), Encoding::Utf8, Source::Bom),
            ] {
                let decoded = decode_with_fallback(body, transport, fallback);
                assert_eq!((decoded.encoding, decoded.source), (expected, source));
            }
        }
        assert!(decode_with_fallback(b"<p>\xa2", None, Encoding::Windows1253).text.ends_with('Ά'));
        assert!(decode_with_fallback(b"<p>\xa2", None, Encoding::Utf8).text.ends_with('�'));
        assert!(decode(b"<p>\xa2", None).text.ends_with('¢'));
    }

    #[test]
    fn content_charset_requires_the_matching_quote_before_using_a_label() {
        for content in ["charset=\"windows-1251", "charset='windows-1251", "charset=\"windows-1251'"] {
            assert_eq!(extract_content_charset(content.as_bytes()), None);
        }
        assert_eq!(extract_content_charset(b"text/html; charset=' windows-1250 '; other=x"), Some(&b" windows-1250 "[..]));
        assert_eq!(extract_content_charset(b"text/html; charset=windows-1250; other=x"), Some(&b"windows-1250"[..]));
        let bytes = b"<!doctype html><meta http-equiv=Content-Type content='charset=\"windows-1251'><meta charset=windows-1250><p>\x80";
        assert_eq!(decode(bytes, None).encoding, Encoding::Windows1250);
        for split in 0..=bytes.len() {
            let mut parser = crate::IncrementalByteParser::new(None);
            parser.push_chunk(&bytes[..split]);
            parser.push_chunk(&bytes[split..]);
            assert_eq!(parser.finish().dump(NodeIdx(0)), parse_bytes(bytes, None).dump(NodeIdx(0)));
        }
    }

    #[test]
    fn meta_user_defined_normalizes_without_changing_transport_or_codec_labels() {
        for prefix in ["<meta charset=x-user-defined><p>",
            "<meta http-equiv=content-type content='text/html; charset=x-user-defined'><p>"] {
            let mut bytes = prefix.as_bytes().to_vec();
            bytes.push(0x80);
            let decoded = decode(&bytes, None);
            assert_eq!(decoded.encoding, Encoding::Windows1252);
            assert!(decoded.text.ends_with('€'));
            let expected = parse_bytes(&bytes, None).dump(amazing_dom::NodeIdx(0));
            for split in 0..=bytes.len() {
                let mut parser = crate::IncrementalByteParser::new(None);
                parser.push_chunk(&bytes[..split]);
                parser.push_chunk(&bytes[split..]);
                assert_eq!(parser.finish().dump(amazing_dom::NodeIdx(0)), expected);
            }
            let transported = decode(&bytes, Some("x-user-defined"));
            assert_eq!(transported.encoding, Encoding::XUserDefined);
            assert!(transported.text.ends_with('\u{f780}'));
        }
    }

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
            "<p>�"
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
