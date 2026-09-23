//! WHATWG Encoding Standard codecs from pinned index data.
//!
//! Data provenance and reproducibility: `data/*.json` and `tools/generate.py`.
#![forbid(unsafe_code)]

mod generated {
    pub(super) mod labels;
    pub(super) mod tables;
    pub(super) mod reverse;
}
mod decoder;
mod encoder;

pub use decoder::{DecodeError, Decoder};
pub use encoder::{EncodeError, Encoder};
pub use generated::labels::Encoding;

/// Resolves an Encoding Standard label after ASCII whitespace trimming and
/// ASCII case folding. Non-ASCII labels are never accepted.
pub fn encoding_for_label(raw: &str) -> Option<Encoding> {
    let label = raw.trim_matches(|c| matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' '));
    if !label.is_ascii() { return None; }
    let lower = label.to_ascii_lowercase();
    generated::labels::LABELS.binary_search_by(|(name, _)| name.cmp(&lower.as_str()))
        .ok().map(|index| generated::labels::LABELS[index].1)
}

/// The three Encoding Standard BOMs. Returns the encoding and marker length.
pub fn sniff_bom(bytes: &[u8]) -> Option<(Encoding, usize)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { Some((Encoding::Utf8, 3)) }
    else if bytes.starts_with(&[0xFE, 0xFF]) { Some((Encoding::Utf16Be, 2)) }
    else if bytes.starts_with(&[0xFF, 0xFE]) { Some((Encoding::Utf16Le, 2)) }
    else { None }
}

/// Decodes a complete byte sequence using replacement or fatal mode.
pub fn decode(bytes: &[u8], encoding: Encoding, fatal: bool) -> Result<String, DecodeError> {
    let mut decoder = Decoder::new(encoding, fatal);
    let mut output = String::new();
    decoder.push(bytes, &mut output)?;
    decoder.finish(&mut output)?;
    Ok(output)
}

/// Encodes Unicode scalars using the Standard's numeric-character-reference
/// fallback, or returns an error in fatal mode.
pub fn encode(text: &str, encoding: Encoding, fatal: bool) -> Result<Vec<u8>, EncodeError> {
    let mut encoder = Encoder::new(encoding, fatal);
    let mut output = Vec::new();
    encoder.push(text, &mut output)?;
    encoder.finish(&mut output)?;
    Ok(output)
}

impl Encoding {
    pub(crate) fn single_byte_index(self) -> Option<&'static str> {
        match self {
            Self::Utf8 | Self::Utf16Be | Self::Utf16Le | Self::Gbk | Self::Gb18030
            | Self::Big5 | Self::EucJp | Self::Iso2022Jp | Self::ShiftJis
            | Self::EucKr | Self::Replacement | Self::XUserDefined => None,
            Self::Iso88598I => Some("iso-8859-8"),
            other => Some(other.name()),
        }
    }
}
