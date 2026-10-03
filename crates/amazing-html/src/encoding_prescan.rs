// Portions ported from html5lib/html5lib-python (html5lib/_inputstream.py,
// EncodingParser.getAttribute) @ fd4f032bc090d44fb11a84b352dad7cbee0a4745, MIT.
// Copyright (c) 2006-2013 James Graham and other contributors.
// Full license: encoding_prescan.LICENSE-MIT. Adapted to bounded std-only byte
// buffers; surrounding meta/duplicate/XML rules follow the current HTML Standard.

use crate::encoding::{extract_content_charset, Encoding, Source};

/// Complete HTML prescan over its bounded prefix. EOF anywhere in a tag aborts
/// the scan rather than accepting a partly completed declaration.
pub(super) fn prescan(input: &[u8]) -> Option<(Encoding, Source)> {
    let bytes = &input[..input.len().min(1024)];
    let mut scan = Scanner { bytes, position: 0 };
    let result = scan.meta_encoding().ok().flatten();
    result.map(|encoding| (encoding, Source::Meta))
        .or_else(|| xml_encoding(bytes).map(|encoding| (encoding, Source::XmlDeclaration)))
}

struct Scanner<'a> { bytes: &'a [u8], position: usize }
type Attribute = (Vec<u8>, Vec<u8>);

impl Scanner<'_> {
    fn byte(&self) -> Result<u8, ()> { self.bytes.get(self.position).copied().ok_or(()) }
    fn rest(&self) -> &[u8] { &self.bytes[self.position..] }
    fn skip_space(&mut self) -> Result<(), ()> {
        while ascii_space(self.byte()?) { self.position += 1; }
        Ok(())
    }
    fn meta_encoding(&mut self) -> Result<Option<Encoding>, ()> {
        while self.position < self.bytes.len() {
            if self.rest().starts_with(b"<!--") {
                // The two dashes at the start can also belong to the closing row.
                let end = self.bytes[self.position + 2..].windows(3)
                    .position(|window| window == b"-->").ok_or(())?;
                self.position += 2 + end + 2;
            } else if self.rest().get(..5).is_some_and(|word| word.eq_ignore_ascii_case(b"<meta"))
                && self.rest().get(5).is_some_and(|byte| ascii_space(*byte) || *byte == b'/') {
                self.position += 5;
                let mut seen = Vec::<Vec<u8>>::new();
                let mut got_pragma = false;
                let mut need_pragma = None;
                let mut charset: Option<Option<Encoding>> = None;
                while let Some((name, value)) = self.attribute()? {
                    if seen.contains(&name) { continue; }
                    seen.push(name.clone());
                    match name.as_slice() {
                        b"http-equiv" => { if value == b"content-type" { got_pragma = true; } }
                        b"content" if charset.is_none() => {
                            if let Some(encoding) = extract_content_charset(&value).and_then(label) {
                                charset = Some(Some(encoding)); need_pragma = Some(true);
                            }
                        }
                        b"charset" => { charset = Some(label(&value)); need_pragma = Some(false); }
                        _ => {}
                    }
                }
                if need_pragma.is_some_and(|needed| !needed || got_pragma) {
                    if let Some(Some(encoding)) = charset {
                        return Ok(Some(match encoding {
                            Encoding::Utf16Le | Encoding::Utf16Be => Encoding::Utf8,
                            Encoding::XUserDefined => Encoding::Windows1252,
                            other => other,
                        }));
                    }
                }
            } else if self.byte()? == b'<' && {
                let index = if self.rest().get(1) == Some(&b'/') { 2 } else { 1 };
                self.rest().get(index).is_some_and(u8::is_ascii_alphabetic)
            } {
                // Skip a real tag's attributes, including quoted '<meta ...>'.
                while !ascii_space(self.byte()?) && self.byte()? != b'>' { self.position += 1; }
                while self.attribute()?.is_some() {}
            } else if [b"<!".as_slice(), b"</", b"<?"].iter().any(|prefix| self.rest().starts_with(prefix)) {
                while self.byte()? != b'>' { self.position += 1; }
            }
            self.position += 1;
        }
        Ok(None)
    }

    /// Adapted complete get-attribute state machine; '/' in an unquoted value
    /// is data, and an initial '=' is part of the attribute name.
    fn attribute(&mut self) -> Result<Option<Attribute>, ()> {
        while ascii_space(self.byte()?) || self.byte()? == b'/' { self.position += 1; }
        if self.byte()? == b'>' { return Ok(None); }
        let mut name = Vec::new();
        let mut value = Vec::new();
        loop {
            let byte = self.byte()?;
            if byte == b'=' && !name.is_empty() { self.position += 1; break; }
            if ascii_space(byte) {
                self.skip_space()?;
                if self.byte()? != b'=' { return Ok(Some((name, value))); }
                self.position += 1; break;
            }
            if matches!(byte, b'/' | b'>') { return Ok(Some((name, value))); }
            name.push(byte.to_ascii_lowercase()); self.position += 1;
        }
        self.skip_space()?;
        let byte = self.byte()?;
        if matches!(byte, b'\'' | b'"') {
            let quote = byte; self.position += 1;
            loop {
                let byte = self.byte()?;
                if byte == quote {
                    self.position += 1;
                    self.byte()?; // The outer prescan EOF rule applies immediately.
                    return Ok(Some((name, value)));
                }
                value.push(byte.to_ascii_lowercase()); self.position += 1;
            }
        }
        while !ascii_space(self.byte()?) && self.byte()? != b'>' {
            value.push(self.byte()?.to_ascii_lowercase()); self.position += 1;
        }
        Ok(Some((name, value)))
    }
}

fn ascii_space(byte: u8) -> bool { matches!(byte, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ') }
fn label(bytes: &[u8]) -> Option<Encoding> {
    std::str::from_utf8(bytes).ok().and_then(amazing_encoding::encoding_for_label)
}

/// Historical HTML-compatible XML encoding lookup. This deliberately does not
/// parse XML grammar or require an XML response; it only selects the HTML codec.
fn xml_encoding(bytes: &[u8]) -> Option<Encoding> {
    if !bytes.starts_with(b"<?xml") { return None; }
    let end = bytes.iter().position(|byte| *byte == b'>')?;
    let mut position = bytes[..end].windows(8).position(|window| window == b"encoding")? + 8;
    while position < end && bytes[position] <= 0x20 { position += 1; }
    if bytes.get(position) != Some(&b'=') { return None; }
    position += 1;
    while position < end && bytes[position] <= 0x20 { position += 1; }
    let quote = *bytes.get(position)?;
    if !matches!(quote, b'\'' | b'"') { return None; }
    position += 1;
    let finish = position + bytes[position..].iter().position(|byte| *byte == quote)?;
    let potential = &bytes[position..finish];
    if potential.iter().any(|byte| *byte <= 0x20) { return None; }
    label(potential).map(|encoding| match encoding {
        Encoding::Utf16Be | Encoding::Utf16Le => Encoding::Utf8,
        other => other,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{decode, decode_with_fallback};

    #[test]
    fn complete_prescan_handles_tag_attributes_duplicates_comments_and_eof() {
        for (source, expected) in [
            ("<MeTa / CHARSET=UTF-8>", Encoding::Utf8),
            ("<meta charset = 'UtF-8' data-x='>'>", Encoding::Utf8),
            ("<!--><meta charset=utf-8>", Encoding::Utf8),
            ("<!---><meta charset=utf-8>", Encoding::Utf8),
            ("<!-- <meta charset=windows-1252> --><meta charset=utf-8>", Encoding::Utf8),
            ("<meta http-equiv=content-type content='text/html; charset=utf-8'>", Encoding::Utf8),
            ("<meta content='text/html; charset=utf-8' http-equiv=content-type>", Encoding::Utf8),
            ("<meta HTTP-EQUIV=CONTENT-TYPE http-equiv=refresh content='charset=utf-8'>", Encoding::Utf8),
            ("<meta charset=UTF-8 charset=windows-1252>", Encoding::Utf8),
            ("<meta content='charset=windows-1252' charset=utf-8>", Encoding::Utf8),
            ("<meta charset=utf-8 content='charset=windows-1252'>", Encoding::Utf8),
            ("<meta content='charset=utf-16be' http-equiv=content-type>", Encoding::Utf8),
            ("<meta charset=bogus><meta charset=utf-8>", Encoding::Utf8),
            ("<meta =charset=x charset=UTF-8>", Encoding::Utf8),
            ("<meta data-x=\"<'>& charset=utf-8\" charset=utf-8>", Encoding::Utf8),
            ("<meta charset=x-user-defined>", Encoding::Windows1252),
            ("<div title='<meta charset=utf-8>'>", Encoding::Windows1252),
            ("</div title='<meta charset=utf-8>'>", Encoding::Windows1252),
            ("<div title='<meta charset=utf-8>", Encoding::Windows1252),
            ("<!-- <meta charset=utf-8>", Encoding::Windows1252),
            ("<!not-a-tag <meta charset=utf-8>", Encoding::Windows1252),
            ("<?processing <meta charset=utf-8>", Encoding::Windows1252),
            ("<meta http-equiv=refresh HTTP-EQUIV=content-type content='charset=utf-8'>", Encoding::Windows1252),
            ("<meta content=bad CONTENT='charset=utf-8' http-equiv=content-type>", Encoding::Windows1252),
            ("<meta charset='' CHARSET=utf-8>", Encoding::Windows1252),
            ("<meta charset=bogus charset=utf-8>", Encoding::Windows1252),
            ("<meta charset=bogus content='charset=utf-8' http-equiv=content-type>", Encoding::Windows1252),
            ("<meta content='charset=utf-8'>", Encoding::Windows1252),
            ("<meta charset=utf-8/>", Encoding::Windows1252),
            ("<meta charset=utf-8", Encoding::Windows1252),
            ("<meta charset='utf-8'", Encoding::Windows1252),
            ("<meta charset=utf-8 x='unfinished", Encoding::Windows1252),
        ] {
            let mut bytes = source.as_bytes().to_vec();
            // Do not accidentally complete an EOF-sensitive tag in the fixture.
            let decoded = decode(&bytes, None);
            assert_eq!(decoded.encoding, expected, "{source}");
            assert_eq!(decode(&bytes, Some("windows-1253")).source, Source::Transport);
            let tree = crate::parse_bytes(&bytes, None).dump(amazing_dom::NodeIdx(0));
            for split in 0..=bytes.len() {
                let mut parser = crate::IncrementalByteParser::new(None);
                parser.push_chunk(&bytes[..split]); parser.push_chunk(&bytes[split..]);
                assert_eq!(parser.finish().dump(amazing_dom::NodeIdx(0)), tree, "{source} split {split}");
            }
            bytes.splice(..0, [0xef,0xbb,0xbf]);
            assert_eq!(decode(&bytes, Some("windows-1253")).source, Source::Bom);
        }
    }

    #[test]
    fn html_compatible_xml_declaration_fallback_is_distinct_from_strict_xml_parsing() {
        for source in ["<?xml encoding='windows-1251'?>", "<?xmlbad encoding = \"windows-1251\">",
            "<?xml encoding\x0b=\x01'windows-1251'>"] {
            let mut bytes = source.as_bytes().to_vec(); bytes.extend_from_slice(b"<p>\xa2");
            let decoded = decode(&bytes, None);
            assert_eq!((decoded.encoding, decoded.source), (Encoding::Windows1251, Source::XmlDeclaration));
            assert!(decoded.text.ends_with('ў'));
            assert_eq!(decode(&bytes, Some("windows-1253")).source, Source::Transport);
        }
        for source in ["<?XML encoding='windows-1251'>", " <?xml encoding='windows-1251'>",
            "<?xml ENCODING='windows-1251'>", "<?xml encoding=windows-1251>",
            "<?xml encoding=' windows-1251'>", "<?xml encoding='windows-1251 '>"
            , "<?xml encoding='bogus'>", "<?xml encoding='windows-1251'", "<?xml encoding='windows-1251>"] {
            assert_eq!(decode_with_fallback(source.as_bytes(), None, Encoding::Windows1253).encoding,
                Encoding::Windows1253, "{source}");
        }
        assert_eq!(decode(b"<?xml encoding='utf-16be'>", None).encoding, Encoding::Utf8);
        assert_eq!(decode(b"<?xml encoding='x-user-defined'>", None).encoding, Encoding::XUserDefined);
        assert_eq!(decode(b"<?xml encoding='windows-1251'><meta charset=utf-8>", None).source, Source::Meta);
    }

    #[test]
    fn completed_and_incomplete_declarations_obey_the_same_1024_byte_end_condition() {
        let token = b"<meta charset='utf-8'>";
        for (extra, expected) in [(0, Encoding::Utf8), (1, Encoding::Windows1252), (8, Encoding::Windows1252)] {
            let mut bytes = vec![b' '; 1024 - token.len() + extra]; bytes.extend_from_slice(token);
            assert_eq!(decode(&bytes, None).encoding, expected);
        }
        let mut bytes = b"<?xml encoding='windows-1251'?>".to_vec();
        bytes.resize(1024 - 15, b' '); bytes.extend_from_slice(b"<meta charset=utf-8>");
        assert_eq!((decode(&bytes, None).encoding, decode(&bytes, None).source),
            (Encoding::Windows1251, Source::XmlDeclaration));
    }
}
