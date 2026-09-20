use super::encoding::{percent_decode, percent_encode, PercentEncodeSet};
use super::parsed::ParseError;

/// Trusted future Unicode adapter. Supply URL Standard domain-parser ToASCII
/// (UTS #46, nontransitional, Bidi/Joiners checked); no adapter/data is bundled.
/// The URL layer independently validates returned ASCII and forbidden points.
pub trait IdnaProvider {
    fn domain_to_ascii(&self, domain: &str) -> Result<String, ParseError>;
}
/// Default policy explicitly reports Unicode domain hosts as unsupported.
pub struct AsciiOnly;
impl IdnaProvider for AsciiOnly {
    fn domain_to_ascii(&self, _: &str) -> Result<String, ParseError> {
        Err(ParseError::IdnaRequired)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    Domain(String),
    Ipv4(u32),
    Ipv6([u16; 8]),
    Opaque(String),
    Empty,
}
impl Host {
    pub fn serialize(&self) -> String {
        match self {
            Self::Domain(s) | Self::Opaque(s) => s.clone(),
            Self::Empty => String::new(),
            Self::Ipv4(n) => std::net::Ipv4Addr::from(*n).to_string(),
            // std's serializer uses dotted decimal for mapped IPv4; WHATWG
            // always uses hexadecimal pieces and first-longest compression.
            Self::Ipv6(p) => {
                let (mut best, mut best_len) = (0, 1);
                let mut i = 0;
                while i < 8 {
                    if p[i] == 0 {
                        let start = i;
                        while i < 8 && p[i] == 0 {
                            i += 1;
                        }
                        if i - start > best_len {
                            best = start;
                            best_len = i - start;
                        }
                    } else {
                        i += 1;
                    }
                }
                let mut out = String::from("[");
                let mut i = 0;
                while i < 8 {
                    if best_len > 1 && i == best {
                        out.push_str("::");
                        i += best_len;
                    } else {
                        if i > 0 && !(best_len > 1 && i == best + best_len) {
                            out.push(':');
                        }
                        out.push_str(&format!("{:x}", p[i]));
                        i += 1;
                    }
                }
                out.push(']');
                out
            }
        }
    }
}
fn forbidden_host(c: char) -> bool {
    matches!(
        c,
        '\0' | '\t'
            | '\n'
            | '\r'
            | ' '
            | '#'
            | '/'
            | ':'
            | '<'
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '|'
    )
}
fn forbidden_domain(c: char) -> bool {
    forbidden_host(c) || c <= '\u{1f}' || c == '\u{7f}' || c == '%'
}
pub(super) fn parse_host(
    input: &str,
    special: bool,
    idna: &dyn IdnaProvider,
) -> Result<Host, ParseError> {
    if input.starts_with('[') {
        let inside = input
            .strip_prefix('[')
            .unwrap()
            .strip_suffix(']')
            .ok_or(ParseError::Ipv6)?;
        let ip: std::net::Ipv6Addr = inside.parse().map_err(|_| ParseError::Ipv6)?;
        return Ok(Host::Ipv6(ip.segments()));
    }
    if !special {
        if input.chars().any(forbidden_host) {
            return Err(ParseError::Host);
        }
        return Ok(if input.is_empty() {
            Host::Empty
        } else {
            Host::Opaque(percent_encode(input, PercentEncodeSet::C0Control))
        });
    }
    if input.is_empty() {
        return Err(ParseError::Host);
    }
    // UTF-8 decode without BOM; replacement is fed to the IDNA seam, not
    // silently accepted as an ASCII hostname.
    let bytes = percent_decode(input);
    let decoded = String::from_utf8_lossy(&bytes);
    let ascii = if decoded.is_ascii() {
        decoded.to_ascii_lowercase()
    } else {
        idna.domain_to_ascii(&decoded)?
    };
    if ascii.is_empty() || !ascii.is_ascii() || ascii.chars().any(forbidden_domain) {
        return Err(ParseError::Host);
    }
    let ascii = ascii.to_ascii_lowercase();
    if ends_in_number(&ascii) {
        return ipv4(&ascii).map(Host::Ipv4);
    }
    Ok(Host::Domain(ascii))
}
fn number(input: &str) -> Option<u64> {
    if input.is_empty() {
        return None;
    }
    let (rest, radix) = if input.len() >= 2 && input[..2].eq_ignore_ascii_case("0x") {
        (&input[2..], 16)
    } else if input.len() >= 2 && input.starts_with('0') {
        (&input[1..], 8)
    } else {
        (input, 10)
    };
    let mut value = 0u64;
    for b in rest.bytes() {
        let digit = (b as char).to_digit(radix)?;
        value = value
            .saturating_mul(radix as u64)
            .saturating_add(digit as u64);
    }
    Some(value)
}
fn ends_in_number(input: &str) -> bool {
    let without_dot = input.strip_suffix('.').unwrap_or(input);
    let last = without_dot.rsplit('.').next().unwrap_or("");
    (!last.is_empty() && last.bytes().all(|b| b.is_ascii_digit())) || number(last).is_some()
}
fn ipv4(input: &str) -> Result<u32, ParseError> {
    let input = input.strip_suffix('.').unwrap_or(input);
    let parts: Vec<_> = input.split('.').collect();
    if parts.len() > 4 {
        return Err(ParseError::Ipv4);
    }
    let nums: Vec<_> = parts
        .iter()
        .map(|s| number(s).ok_or(ParseError::Ipv4))
        .collect::<Result<_, _>>()?;
    if nums[..nums.len() - 1].iter().any(|n| *n > 255)
        || nums[nums.len() - 1] >= (1u64 << (8 * (5 - nums.len())))
    {
        return Err(ParseError::Ipv4);
    }
    let mut value = nums[nums.len() - 1];
    for (i, n) in nums[..nums.len() - 1].iter().enumerate() {
        value += n << (8 * (3 - i));
    }
    Ok(value as u32)
}
