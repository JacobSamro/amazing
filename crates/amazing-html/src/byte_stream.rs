//! Bounded byte input for the existing incremental HTML tree builder.

use amazing_dom::Dom;

use crate::encoding::{self, Encoding};
use crate::IncrementalParser;

const SNIFF_LIMIT: usize = 1024;
const TEXT_BATCH: usize = 4096;

/// Parses successive byte chunks using the same four encodings and sniffing
/// precedence as [`crate::parse_bytes`]. Call [`Self::finish`] exactly once at
/// the end of the response. The transport label, if any, must be known at
/// construction time.
///
/// No decoded text is emitted before the encoding choice is final. With no
/// early EOF, this happens after 1024 input bytes. After each `push_chunk`,
/// retained undecoded bytes are at most 1024 before sniffing, or three UTF-8
/// bytes / one UTF-16 byte plus one high surrogate afterwards. Decoded input
/// is handed to `IncrementalParser` in batches of at most 4099 UTF-8 bytes
/// (the 4096-byte flush threshold plus one four-byte scalar).
/// The parser may retain one incomplete token and the growing DOM.
pub struct IncrementalByteParser {
    parser: IncrementalParser,
    transport_label: Option<String>,
    prefix: Vec<u8>,
    encoding: Option<Encoding>,
    utf8: Vec<u8>,
    odd_utf16_byte: Option<u8>,
    high_surrogate: Option<u16>,
    text: String,
}

impl IncrementalByteParser {
    pub fn new(transport_label: Option<&str>) -> Self {
        Self {
            parser: IncrementalParser::new(),
            transport_label: transport_label.map(str::to_owned),
            prefix: Vec::new(),
            encoding: None,
            utf8: Vec::new(),
            odd_utf16_byte: None,
            high_surrogate: None,
            text: String::new(),
        }
    }

    pub fn push_chunk(&mut self, mut chunk: &[u8]) {
        if self.encoding.is_none() {
            let take = chunk.len().min(SNIFF_LIMIT - self.prefix.len());
            self.prefix.extend_from_slice(&chunk[..take]);
            chunk = &chunk[take..];
            if self.prefix.len() == SNIFF_LIMIT {
                self.choose_encoding();
            }
        }
        if self.encoding.is_some() {
            self.feed_bytes(chunk);
            self.flush_text();
        }
    }

    /// Retained byte input, excluding the caller-owned chunk and decoded
    /// parser state. Useful for checking the consumed-input bound.
    pub fn buffered_input_bytes(&self) -> usize {
        self.prefix.len() + self.utf8.len() + usize::from(self.odd_utf16_byte.is_some())
            + 2 * usize::from(self.high_surrogate.is_some())
    }

    pub fn buffered_input_scalars(&self) -> usize {
        self.parser.buffered_input_scalars()
    }

    pub fn finish(mut self) -> Dom {
        if self.encoding.is_none() {
            self.choose_encoding();
        }
        match self.encoding.expect("encoding chosen") {
            Encoding::Utf8 if !self.utf8.is_empty() => self.emit('\u{fffd}'),
            Encoding::Utf16Le | Encoding::Utf16Be => {
                if self.high_surrogate.is_some() { self.emit('\u{fffd}'); }
                if self.odd_utf16_byte.is_some() { self.emit('\u{fffd}'); }
            }
            _ => {}
        }
        self.flush_text();
        self.parser.finish()
    }

    fn choose_encoding(&mut self) {
        let (encoding, _, skip) = encoding::sniff(&self.prefix, self.transport_label.as_deref());
        self.encoding = Some(encoding);
        self.transport_label = None;
        let prefix = std::mem::take(&mut self.prefix);
        self.feed_bytes(&prefix[skip..]);
        self.flush_text();
    }

    fn feed_bytes(&mut self, bytes: &[u8]) {
        match self.encoding.expect("encoding chosen") {
            Encoding::Utf8 => {
                for &byte in bytes {
                    self.utf8.push(byte);
                    loop {
                        match std::str::from_utf8(&self.utf8) {
                            Ok(valid) => {
                                let valid = valid.to_owned();
                                self.utf8.clear();
                                self.emit_str(&valid);
                                break;
                            }
                            Err(error) => {
                                let valid_len = error.valid_up_to();
                                if valid_len > 0 {
                                    let valid = String::from_utf8(self.utf8.drain(..valid_len).collect())
                                        .expect("validated UTF-8 prefix");
                                    self.emit_str(&valid);
                                }
                                if let Some(invalid_len) = error.error_len() {
                                    self.utf8.drain(..invalid_len);
                                    self.emit('\u{fffd}');
                                    if self.utf8.is_empty() { break; }
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let little = self.encoding == Some(Encoding::Utf16Le);
                for &byte in bytes {
                    if let Some(first) = self.odd_utf16_byte.take() {
                        let unit = if little { u16::from_le_bytes([first, byte]) }
                                   else { u16::from_be_bytes([first, byte]) };
                        self.emit_utf16(unit);
                    } else {
                        self.odd_utf16_byte = Some(byte);
                    }
                }
            }
            Encoding::Windows1252 => {
                for &byte in bytes { self.emit(encoding::windows1252(byte)); }
            }
        }
    }

    fn emit_utf16(&mut self, unit: u16) {
        if let Some(high) = self.high_surrogate.take() {
            if (0xdc00..=0xdfff).contains(&unit) {
                let scalar = 0x10000 + ((high as u32 - 0xd800) << 10) + (unit as u32 - 0xdc00);
                self.emit(char::from_u32(scalar).expect("valid surrogate pair"));
                return;
            }
            self.emit('\u{fffd}');
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high_surrogate = Some(unit);
        } else if (0xdc00..=0xdfff).contains(&unit) {
            self.emit('\u{fffd}');
        } else {
            self.emit(char::from_u32(unit as u32).expect("valid BMP scalar"));
        }
    }

    fn emit_str(&mut self, value: &str) {
        for c in value.chars() { self.emit(c); }
    }

    fn emit(&mut self, value: char) {
        self.text.push(value);
        if self.text.len() >= TEXT_BATCH { self.flush_text(); }
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.parser.push_chunk(&self.text);
            self.text.clear();
        }
    }
}
