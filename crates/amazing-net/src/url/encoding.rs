//! URL Standard percent-encode sets. Existing escapes are not decoded/re-encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentEncodeSet {
    C0Control,
    Fragment,
    Query,
    SpecialQuery,
    Path,
    Userinfo,
    Component,
}
impl PercentEncodeSet {
    fn contains(self, b: u8) -> bool {
        use PercentEncodeSet::*;
        if b < 0x20 || b > 0x7e {
            return true;
        }
        match self {
            C0Control => false,
            Fragment => b" \"<>`".contains(&b),
            Query => b" \"#<>".contains(&b),
            SpecialQuery => Query.contains(b) || b == b'\'',
            Path => Query.contains(b) || b"?^`{}".contains(&b),
            Userinfo => Path.contains(b) || b"/:;=@[\\]|".contains(&b),
            Component => Userinfo.contains(b) || b"$%&+,".contains(&b),
        }
    }
}
pub fn percent_encode(input: &str, set: PercentEncodeSet) -> String {
    let mut result = String::new();
    for b in input.bytes() {
        if set.contains(b) {
            use std::fmt::Write;
            write!(result, "%{b:02X}").unwrap();
        } else {
            result.push(b as char);
        }
    }
    result
}
pub(super) fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}
