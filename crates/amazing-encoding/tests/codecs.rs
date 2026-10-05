use amazing_encoding::{decode, encode, encoding_for_label, sniff_bom, Decoder, Encoding};

#[test]
fn utf16_eof_clears_incomplete_surrogate_and_byte_together() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        for (units, results) in [
            (vec![0xD800, 0x0041], vec!["", "�", "�", "�", "�A"]),
            (vec![0x0041, 0x0000], vec!["", "�", "A", "A�", "A\0"]),
            (vec![0xDC00], vec!["", "�", "�"]),
            (vec![0xD83D, 0xDE00], vec!["", "�", "�", "�", "😀"]),
        ] {
            let bytes: Vec<u8> = units.iter().flat_map(|unit: &u16|
                if encoding == Encoding::Utf16Le { unit.to_le_bytes() } else { unit.to_be_bytes() }).collect();
            for length in 0..=bytes.len() {
                let input = &bytes[..length];
                let expected = results[length];
                for split in 0..=input.len() {
                    let mut decoder = Decoder::new(encoding, false);
                    let mut output = String::new();
                    decoder.push(&input[..split], &mut output).unwrap();
                    decoder.push(&input[split..], &mut output).unwrap();
                    decoder.finish(&mut output).unwrap();
                    assert_eq!(output, expected, "{encoding:?} input{input:?} split{split}");
                    decoder.finish(&mut output).unwrap();
                    assert_eq!(output, expected);
                }
            }
        }
    }
}

#[test]
fn fatal_errors_consume_bad_bytes_but_preserve_iso2022_mode() {
    let mut utf8 = Decoder::new(Encoding::Utf8, true);
    let mut output = String::new();
    assert_eq!(utf8.push(&[0xC0], &mut output).unwrap_err().byte_offset, 0);
    assert_eq!(utf8.push(&[0xFF], &mut output).unwrap_err().byte_offset, 1);
    utf8.finish(&mut output).unwrap();
    assert_eq!(output, "");
    for (selector, byte, expected) in [(b'J', 0x7E, "‾"), (b'I', 0x42, "ﾂ")] {
        let mut decoder = Decoder::new(Encoding::Iso2022Jp, true);
        assert!(decoder.push(&[0x1B,b'(',selector,0xFF], &mut output).is_err());
        output.clear();
        decoder.push(&[byte], &mut output).unwrap();
        assert_eq!(output, expected);
    }
}

#[test]
fn standard_vectors_for_every_codec_family() {
    let vectors: &[(Encoding, &[u8], &str)] = &[
        (Encoding::Utf8, b"\xE2\x82\xAC", "€"),
        (Encoding::Utf16Le, b"\x3D\xD8\x00\xDE", "😀"),
        (Encoding::Utf16Be, b"\xD8\x3D\xDE\x00", "😀"),
        (Encoding::Windows1252, b"\x80", "€"),
        (Encoding::Gbk, b"\xD6\xD0", "中"),
        (Encoding::Gb18030, b"\xD6\xD0", "中"),
        (Encoding::Big5, b"\xA4\x40", "一"),
        (Encoding::EucJp, b"\xA4\xA2", "あ"),
        (Encoding::Iso2022Jp, b"\x1B$B\x24\x22\x1B(B", "あ"),
        (Encoding::ShiftJis, b"\x82\xA0", "あ"),
        (Encoding::EucKr, b"\xB0\xA1", "가"),
        (Encoding::XUserDefined, b"\x80", "\u{F780}"),
    ];
    for &(encoding, bytes, expected) in vectors {
        assert_eq!(decode(bytes, encoding, false).unwrap(), expected, "{encoding:?}");
        assert_eq!(encode(expected, encoding, false).unwrap(), bytes, "{encoding:?}");
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::new(encoding, false);
            let mut output = String::new();
            decoder.push(&bytes[..split], &mut output).unwrap();
            assert!(decoder.buffered_input_bytes() <= 4);
            decoder.push(&bytes[split..], &mut output).unwrap();
            decoder.finish(&mut output).unwrap();
            assert_eq!(output, expected, "{encoding:?} split {split}");
        }
        let mut decoder = Decoder::new(encoding, false);
        let mut output = String::new();
        for byte in bytes {
            decoder.push(std::slice::from_ref(byte), &mut output).unwrap();
            assert!(decoder.buffered_input_bytes() <= 4);
        }
        decoder.finish(&mut output).unwrap();
        assert_eq!(output, expected, "{encoding:?} bytewise");
    }
}

#[test]
fn replacement_fatal_labels_and_bom() {
    assert_eq!(decode(b"x", Encoding::Replacement, false).unwrap(), "�");
    assert!(decode(b"x", Encoding::Replacement, true).is_err());
    assert_eq!(decode(b"\xC3(", Encoding::Utf8, false).unwrap(), "�(");
    assert!(decode(b"\xC3(", Encoding::Utf8, true).is_err());
    assert_eq!(decode(b"\xD8", Encoding::Utf16Be, false).unwrap(), "�");
    assert_eq!(encoding_for_label("  LaTiN1\t"), Some(Encoding::Windows1252));
    assert_eq!(encoding_for_label("x-sjis"), Some(Encoding::ShiftJis));
    assert_eq!(encoding_for_label("☃"), None);
    assert_eq!(sniff_bom(b"\xEF\xBB\xBFx"), Some((Encoding::Utf8, 3)));
    assert_eq!(sniff_bom(b"\xFF\xFEx"), Some((Encoding::Utf16Le, 2)));
    assert_eq!(sniff_bom(b"\xFE\xFFx"), Some((Encoding::Utf16Be, 2)));
}

#[test]
fn numeric_reference_fallback_and_gb18030_range() {
    assert_eq!(encode("☃", Encoding::Windows1252, false).unwrap(), b"&#9731;");
    assert!(encode("☃", Encoding::Windows1252, true).is_err());
    let bytes = encode("😀", Encoding::Gb18030, true).unwrap();
    assert_eq!(decode(&bytes, Encoding::Gb18030, true).unwrap(), "😀");
}

#[test]
fn malformed_sequences_keep_ascii_and_chunk_equivalence() {
    let cases: &[(Encoding, &[u8], &str)] = &[
        (Encoding::Gbk, b"\x81!", "�!"),
        (Encoding::Big5, b"\x81!", "�!"),
        (Encoding::EucJp, b"\x8F\xA1!", "�!"),
        (Encoding::ShiftJis, b"\x81!", "�!"),
        (Encoding::EucKr, b"\x81!", "�!"),
        (Encoding::Iso2022Jp, b"\x1B$!", "�$!"),
        (Encoding::Utf8, b"\xE2(\xA1", "�(�"),
    ];
    for &(encoding, bytes, expected) in cases {
        assert_eq!(decode(bytes, encoding, false).unwrap(), expected, "{encoding:?}");
        assert!(decode(bytes, encoding, true).is_err(), "{encoding:?}");
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::new(encoding, false);
            let mut output = String::new();
            decoder.push(&bytes[..split], &mut output).unwrap();
            decoder.push(&bytes[split..], &mut output).unwrap();
            decoder.finish(&mut output).unwrap();
            assert_eq!(output, expected, "{encoding:?} split {split}");
        }
        let mut decoder = Decoder::new(encoding, false);
        let mut output = String::new();
        for byte in bytes { decoder.push(std::slice::from_ref(byte), &mut output).unwrap(); }
        decoder.finish(&mut output).unwrap();
        assert_eq!(output, expected, "{encoding:?} bytewise");
    }
}

#[test]
fn single_byte_encoder_vectors_cover_each_index() {
    // Byte 0x80 vectors from the pinned WPT single-byte data selection.
    let vectors = [
        ("IBM866", 0x0410), ("ISO-8859-2", 0x0080),
        ("ISO-8859-3", 0x0080), ("ISO-8859-4", 0x0080),
        ("ISO-8859-5", 0x0080), ("ISO-8859-6", 0x0080),
        ("ISO-8859-7", 0x0080), ("ISO-8859-8", 0x0080),
        ("ISO-8859-8-I", 0x0080), ("ISO-8859-10", 0x0080),
        ("ISO-8859-13", 0x0080), ("ISO-8859-14", 0x0080),
        ("ISO-8859-15", 0x0080), ("ISO-8859-16", 0x0080),
        ("KOI8-R", 0x2500), ("KOI8-U", 0x2500),
        ("macintosh", 0x00C4), ("windows-874", 0x20AC),
        ("windows-1250", 0x20AC), ("windows-1251", 0x0402),
        ("windows-1252", 0x20AC), ("windows-1253", 0x20AC),
        ("windows-1254", 0x20AC), ("windows-1255", 0x20AC),
        ("windows-1256", 0x20AC), ("windows-1257", 0x20AC),
        ("windows-1258", 0x20AC), ("x-mac-cyrillic", 0x0410),
    ];
    for (label, cp) in vectors {
        let encoding = encoding_for_label(label).unwrap();
        let scalar = char::from_u32(cp).unwrap().to_string();
        assert_eq!(encode(&scalar, encoding, true).unwrap(), [0x80], "{label}");
    }
}

#[test]
fn standard_exception_vectors() {
    assert_eq!(encode("€", Encoding::Gbk, true).unwrap(), [0x80]);
    assert_eq!(encode("\u{E78D}", Encoding::Gb18030, true).unwrap(), [0xA6, 0xD9]);
    assert!(encode("\u{E5E5}", Encoding::Gb18030, true).is_err());
    assert_eq!(decode(&[0x81, 0x35, 0xF4, 0x37], Encoding::Gb18030, false).unwrap(), "\u{E7C7}");
    assert_eq!(encode("\u{E7C7}", Encoding::Gb18030, true).unwrap(), [0x81, 0x35, 0xF4, 0x37]);
    assert_eq!(decode(&[0x88, 0x62], Encoding::Big5, true).unwrap(), "Ê\u{0304}");
    assert_eq!(encode("¥", Encoding::EucJp, true).unwrap(), [0x5C]);
    assert_eq!(encode("¥", Encoding::ShiftJis, true).unwrap(), [0x5C]);
    assert_eq!(encode("¥", Encoding::Iso2022Jp, true).unwrap(), b"\x1B(J\x5C\x1B(B");
    assert_eq!(encode("あ\x1B", Encoding::Iso2022Jp, false).unwrap(),
        b"\x1B$B\x24\x22\x1B(B&#65533;");
}

#[test]
fn iso2022_incomplete_escape_eof_restores_output_state() {
    // Exercise each output state, both incomplete escape states, every chunk
    // boundary, and both error modes. EOF must clear the consecutive-escape
    // flag without changing the selected character set.
    for selector in [b"(B", b"(J", b"(I", b"$B"] {
        for incomplete in [b"\x1B".as_slice(), b"\x1B(".as_slice(), b"\x1B$".as_slice()] {
            let input = [b"\x1B".as_slice(), selector.as_slice(), incomplete].concat();
            for fatal in [false, true] {
                for split in 0..=input.len() {
                    let mut decoder = Decoder::new(Encoding::Iso2022Jp, fatal);
                    let mut output = String::new();
                    decoder.push(&input[..split], &mut output).unwrap();
                    decoder.push(&input[split..], &mut output).unwrap();
                    assert!(decoder.finish(&mut output).is_err() == fatal);
                    // A fatal EOF can restore a byte which itself needs an EOF
                    // error in JIS0208. Consume it before resuming the decoder.
                    if fatal && decoder.buffered_input_bytes() != 0 {
                        let _ = decoder.finish(&mut output);
                    }
                    assert_eq!(decoder.buffered_input_bytes(), 0);
                    decoder.finish(&mut output).unwrap();
                    output.clear();
                    // No character occurred after the first valid escape, so
                    // a stale output flag would reject this valid escape.
                    decoder.push(b"\x1B(J\\", &mut output).unwrap();
                    decoder.finish(&mut output).unwrap();
                    assert_eq!(output, "¥", "selector{selector:?} incomplete{incomplete:?} fatal{fatal} split{split}");
                }
            }
        }
    }
}
