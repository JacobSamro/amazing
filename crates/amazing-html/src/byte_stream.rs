//! Bounded byte input for the existing incremental HTML tree builder.

use amazing_dom::Dom;

use crate::encoding::{self, Encoding};
use crate::IncrementalParser;

const SNIFF_LIMIT: usize = 1024;

/// Parses successive byte chunks using the same encodings and sniffing
/// precedence as [`crate::parse_bytes`]. Call [`Self::finish`] exactly once at
/// the end of the response. The transport label, if any, must be known at
/// construction time.
///
/// No decoded text is emitted before the encoding choice is final. With no
/// early EOF, this happens after 1024 input bytes. After each `push_chunk`,
/// retained undecoded bytes are at most 1024 before sniffing, or four codec
/// bytes afterwards. Decoded input is handed to `IncrementalParser` after
/// each input chunk.
/// The parser may retain one incomplete token and the growing DOM.
pub struct IncrementalByteParser {
    parser: IncrementalParser,
    transport_label: Option<String>,
    prefix: Vec<u8>,
    encoding: Option<Encoding>,
    decoder: Option<amazing_encoding::Decoder>,
    text: String,
}

impl IncrementalByteParser {
    pub fn new(transport_label: Option<&str>) -> Self {
        Self {
            parser: IncrementalParser::new(),
            transport_label: transport_label.map(str::to_owned),
            prefix: Vec::new(),
            encoding: None,
            decoder: None,
            text: String::new(),
        }
    }

    /// Sets scripting before feeding the first byte chunk.
    pub fn set_scripting_mode(&mut self, mode: crate::ScriptingMode) {
        self.parser.set_scripting_mode(mode);
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
        self.prefix.len() + self.decoder.as_ref().map_or(0, |d| d.buffered_input_bytes())
    }

    pub fn buffered_input_scalars(&self) -> usize {
        self.parser.buffered_input_scalars()
    }

    pub fn finish(mut self) -> Dom {
        if self.encoding.is_none() {
            self.choose_encoding();
        }
        self.decoder.as_mut().expect("decoder chosen").finish(&mut self.text)
            .expect("replacement mode cannot fail");
        self.flush_text();
        self.parser.finish()
    }

    fn choose_encoding(&mut self) {
        let (encoding, _, skip) = encoding::sniff(&self.prefix, self.transport_label.as_deref());
        self.encoding = Some(encoding);
        self.decoder = Some(amazing_encoding::Decoder::new(encoding, false));
        self.transport_label = None;
        let prefix = std::mem::take(&mut self.prefix);
        self.feed_bytes(&prefix[skip..]);
        self.flush_text();
    }

    fn feed_bytes(&mut self, bytes: &[u8]) {
        self.decoder.as_mut().expect("decoder chosen").push(bytes, &mut self.text)
            .expect("replacement mode cannot fail");
        self.flush_text();
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.parser.push_chunk(&self.text);
            self.text.clear();
        }
    }
}
