//! Subresource Integrity Level 2 metadata parsing and strongest-digest checks.
//! https://www.w3.org/TR/sri/#response-verification-algorithms
mod constants;
mod sha2;

pub(crate) struct Metadata {
    strongest: Option<usize>,
    digests: Vec<Vec<u8>>,
}
impl Metadata {
    pub(crate) fn parse(input: &str) -> Self {
        let mut result = Self {
            strongest: None,
            digests: Vec::new(),
        };
        for token in input.split_ascii_whitespace() {
            let expression = token.split('?').next().unwrap_or(""); // no options defined
            let Some((algorithm, encoded)) = expression.split_once('-') else {
                continue;
            };
            let rank = match algorithm {
                "sha256" => 0,
                "sha384" => 1,
                "sha512" => 2,
                _ => continue,
            };
            let Some(decoded) = decode_base64(encoded) else {
                continue;
            };
            match result.strongest {
                Some(old) if rank < old => {}
                Some(old) if rank == old => result.digests.push(decoded),
                _ => {
                    result.strongest = Some(rank);
                    result.digests.clear();
                    result.digests.push(decoded);
                }
            }
        }
        result
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.strongest.is_none()
    }
    pub(crate) fn matches(&self, bytes: &[u8]) -> bool {
        let Some(rank) = self.strongest else {
            return true;
        };
        let actual = sha2::digest(rank, bytes);
        self.digests
            .iter()
            .any(|expected| constant_time_equal(expected, &actual))
    }
}
fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    difference == 0
}
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() {
        return None;
    }
    let trimmed = input.trim_end_matches('=');
    let padding = input.len() - trimmed.len();
    if padding > 2
        || input.len() % 4 == 1
        || (padding > 0 && input.len() % 4 != 0)
        || trimmed.len() % 4 == 1
    {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4 + 2);
    let mut value = 0u32;
    let mut bits = 0;
    for byte in trimmed.bytes() {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        value = (value << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((value >> bits) as u8);
            value &= (1 << bits) - 1;
        }
    }
    if value != 0 {
        return None;
    }
    Some(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fips_vectors() {
        let cases = [
            (0,"", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            (0,"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            (1,"abc", "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"),
            (2,"abc", "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"),
        ];
        for (kind, input, expected) in cases {
            let actual = sha2::digest(kind, input.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn strongest_and_invalid_tokens() {
        assert!(Metadata::parse("sha256-...").is_empty());
        assert!(Metadata::parse("sha256-").is_empty());
        assert!(Metadata::parse("unknown-AAAA").is_empty());
        assert!(!Metadata::parse("sha256-ZGVhZGJlZWY=").is_empty());
        assert!(
            Metadata::parse("sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=").matches(b"abc")
        );
        assert!(!Metadata::parse(
            "sha512-ZGVhZA== sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        )
        .matches(b"abc"));
    }
}
