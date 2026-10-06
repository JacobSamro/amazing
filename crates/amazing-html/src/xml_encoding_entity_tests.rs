//! Complete document-entity authority matrix, separate from MIME recovery.
use super::*;

fn bytes(text: &str, encoding: Encoding, bom: bool) -> Vec<u8> {
    let mut result = if bom {
        match encoding {
            Encoding::Utf8 => vec![0xef, 0xbb, 0xbf],
            Encoding::Utf16Le => vec![0xff, 0xfe],
            Encoding::Utf16Be => vec![0xfe, 0xff],
            _ => panic!("no XML signature for this test encoding"),
        }
    } else { Vec::new() };
    result.extend(amazing_encoding::encode(text, encoding, true).unwrap());
    result
}

#[test]
fn document_bom_and_complete_declared_encoding_matrix() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for label in ["UTF-8", "uTf-16", "UTF-16LE", "UTF-16BE", "ISO-8859-1", "windows-1252", "unknown"] {
            for quote in ['\'', '"'] {
                let text = format!("<?xml version='1.0' encoding={quote}{label}{quote}?><root>é🦀</root>");
                let input = bytes(&text, encoding, true);
                let expected = match label {
                    "UTF-8" => encoding == Encoding::Utf8,
                    "uTf-16" => encoding != Encoding::Utf8,
                    "UTF-16LE" => encoding == Encoding::Utf16Le,
                    "UTF-16BE" => encoding == Encoding::Utf16Be,
                    _ => false,
                };
                match decode_entity(&input) {
                    Ok(decoded) => {
                        assert!(expected, "{encoding:?} versus {label}");
                        assert_eq!(decoded.encoding, encoding);
                        assert_eq!(decoded.text, text);
                        assert!(crate::xml::parse_xml_decoded(&decoded.text).is_ok());
                    }
                    Err(_) => assert!(!expected, "{encoding:?} versus {label}"),
                }
            }
        }
        for text in ["<root>é🦀</root>", "<?xml version='1.0'?><root/>"] {
            assert_eq!(decode_entity(&bytes(text, encoding, true)).unwrap().text, text);
        }
    }
    let utf8_alias = bytes("<?xml version='1.0' encoding='unicode-1-1-utf-8'?><root/>", Encoding::Utf8, true);
    assert_eq!(decode_entity(&utf8_alias).unwrap().encoding, Encoding::Utf8);
    // An ASCII-only body is compatible with several codecs, but a genuine
    // signature/declaration contradiction remains an error without MIME authority.
    for (encoding, label) in [(Encoding::Utf8, "ISO-8859-1"), (Encoding::Utf16Be, "UTF-8")] {
        let text = format!("<?xml version='1.0' encoding='{label}'?><root/>");
        assert!(decode_entity(&bytes(&text, encoding, true)).is_err());
    }
}

#[test]
fn unmarked_entity_signatures_and_required_declaration_family() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let label = if encoding == Encoding::Utf16Le { "UTF-16LE" } else { "UTF-16BE" };
        let opposite = if encoding == Encoding::Utf16Le { "UTF-16BE" } else { "UTF-16LE" };
        let text = format!("<?xml version='1.0' encoding='{label}'?><root>水</root>");
        assert_eq!(decode_entity(&bytes(&text, encoding, false)).unwrap().text, text);
        for declaration in [opposite, "UTF-8", "UTF-16"] {
            let text = format!("<?xml version='1.0' encoding='{declaration}'?><root/>");
            assert!(decode_entity(&bytes(&text, encoding, false)).is_err());
        }
        assert!(decode_entity(&bytes("<?xml version='1.0'?><root/>", encoding, false)).is_err());
    }
    for label in ["UTF-16LE", "UTF-16BE", "UTF-16"] {
        let text = format!("<?xml version='1.0' encoding='{label}'?><root/>");
        assert!(decode_entity(text.as_bytes()).is_err());
    }
    assert_eq!(decode_entity("<root>水</root>".as_bytes()).unwrap().encoding, Encoding::Utf8);
    assert!(decode_entity(b"<root>\xe9</root>").is_err());
    let legacy = b"<?xml version='1.0' encoding='windows-1253'?><root>\xa2</root>";
    assert!(decode_entity(legacy).unwrap().text.contains('\u{0386}'));
}

#[test]
fn actual_mime_authority_remains_distinct_from_file_entity_consistency() {
    // RFC 7303 gives a MIME BOM priority even when charset is absent.
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let text = "<?xml version='1.0' encoding='unknown'?><root>é</root>";
        let input = bytes(text, encoding, true);
        assert!(decode_entity(&input).is_err());
        for transport in [None, Some("utf-8"), Some("windows-1252"), Some("bogus")] {
            let decoded = decode(&input, transport).unwrap();
            assert_eq!(decoded.encoding, encoding);
            assert_eq!(decoded.text, text);
        }
    }
    let text = b"<?xml version='1.0' encoding='UTF-8'?><root>\xe9</root>";
    assert!(decode_entity(text).is_err());
    assert!(decode(text, Some("windows-1252")).unwrap().text.contains('é'));
    assert!(decode(b"<?xml version='1.0' encoding='unknown'?><root/>", Some("utf-8")).is_ok());
}

#[test]
fn declaration_grammar_is_checked_after_one_signature_only() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for text in [
            "<?xml encoding='UTF-8' version='1.0'?><root/>",
            "<?xml version='1.0' encoding='utf 8'?><root/>",
            "<?xml version='1.0' encoding='UTF-8' encoding='UTF-8'?><root/>",
            "<?xml version='1'?><root/>",
            "<?xml version='1.0' encoding='UTF-8'><root/>",
        ] {
            assert!(decode_entity(&bytes(text, encoding, true)).is_err(), "{text}");
        }
        let text = "\u{feff}<root/>";
        let decoded = decode_entity(&bytes(text, encoding, true)).unwrap();
        assert_eq!(decoded.text, text, "only the actual signature is stripped");
        assert!(crate::xml::parse_xml_decoded(&decoded.text).is_err());
    }
}

#[test]
fn malformed_and_unsupported_entity_bytes_never_return_partial_text() {
    for input in [
        b"\xef\xbb\xbf<root>\xff</root>".as_slice(),
        b"\xff\xfe<\0r\0o\0o\0t\0>\0\0\xd8",
        b"\xfe\xff\0<\0r>",
        b"\xff\xfe\0\0<\0\0\0",
        b"\0\0\xfe\xff\0\0\0<",
        b"\0\0\xff\xfe",
        b"\xfe\xff\0\0",
        b"\x4c\x6f\xa7\x94",
    ] {
        assert!(decode_entity(input).is_err(), "{input:?}");
    }
}

#[test]
fn every_supported_codec_retains_its_actual_declared_entity_authority() {
    let encodings = [
        Encoding::Utf8, Encoding::Ibm866, Encoding::Iso88592, Encoding::Iso88593,
        Encoding::Iso88594, Encoding::Iso88595, Encoding::Iso88596, Encoding::Iso88597,
        Encoding::Iso88598, Encoding::Iso88598I, Encoding::Iso885910, Encoding::Iso885913,
        Encoding::Iso885914, Encoding::Iso885915, Encoding::Iso885916, Encoding::Koi8R,
        Encoding::Koi8U, Encoding::Macintosh, Encoding::Windows874, Encoding::Windows1250,
        Encoding::Windows1251, Encoding::Windows1252, Encoding::Windows1253,
        Encoding::Windows1254, Encoding::Windows1255, Encoding::Windows1256,
        Encoding::Windows1257, Encoding::Windows1258, Encoding::XMacCyrillic,
        Encoding::Gbk, Encoding::Gb18030, Encoding::Big5, Encoding::EucJp,
        Encoding::Iso2022Jp, Encoding::ShiftJis, Encoding::EucKr,
        Encoding::Utf16Be, Encoding::Utf16Le, Encoding::XUserDefined,
    ];
    for encoding in encodings {
        let text = format!("<?xml version='1.0' encoding='{}'?><root>A</root>", encoding.name());
        let input = bytes(&text, encoding, false);
        let decoded = decode_entity(&input).unwrap();
        assert_eq!(decoded.encoding, encoding);
        assert_eq!(decoded.text, text);
        assert!(crate::xml::parse_xml_decoded(&decoded.text).is_ok());
        if encoding != Encoding::Utf8 {
            assert!(decode_entity(&bytes(&text, Encoding::Utf8, true)).is_err());
        }
    }
    assert!(decode_entity(b"<?xml version='1.0' encoding='replacement'?><root/>").is_err());
}
