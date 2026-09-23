//! Unicode 17.0 UTS #46 nontransitional processing for URL domains.
//! Tables are generated from pinned Unicode files; no runtime Unicode crate.
use super::host::IdnaProvider;
use super::parsed::ParseError;
mod data {
    include!("idna_data.rs");
}

pub struct Idna;

impl IdnaProvider for Idna {
    fn domain_to_ascii(&self, domain: &str) -> Result<String, ParseError> {
        domain_to_ascii(domain)
    }
}

fn range_value(ranges: &[(u32, u32, u8)], cp: u32, default: u8) -> u8 {
    let index = ranges.partition_point(|r| r.0 <= cp);
    if index != 0 && cp <= ranges[index - 1].1 {
        ranges[index - 1].2
    } else {
        default
    }
}

fn mapping(cp: u32) -> (u8, &'static [u32]) {
    let index = data::MAPPING.partition_point(|r| r.0 <= cp) - 1;
    let row = data::MAPPING[index];
    debug_assert!(cp <= row.1);
    (
        row.2,
        &data::MAP_VALUES[row.3 as usize..row.3 as usize + row.4 as usize],
    )
}

fn ccc(cp: u32) -> u8 {
    range_value(data::CCC, cp, 0)
}

fn is_mark(cp: u32) -> bool {
    let index = data::MARKS.partition_point(|r| r.0 <= cp);
    index != 0 && cp <= data::MARKS[index - 1].1
}

fn decompose(cp: u32, out: &mut Vec<u32>) {
    // Hangul canonical decomposition is algorithmic (UAX #15).
    if (0xac00..=0xd7a3).contains(&cp) {
        let s = cp - 0xac00;
        out.push(0x1100 + s / 588);
        out.push(0x1161 + (s % 588) / 28);
        if s % 28 != 0 {
            out.push(0x11a7 + s % 28);
        }
        return;
    }
    if let Ok(i) = data::DECOMP.binary_search_by_key(&cp, |r| r.0) {
        let (_, start, len) = data::DECOMP[i];
        for part in &data::DECOMP_VALUES[start as usize..start as usize + len as usize] {
            decompose(*part, out);
        }
    } else {
        out.push(cp);
    }
}

fn compose(a: u32, b: u32) -> Option<u32> {
    if (0x1100..=0x1112).contains(&a) && (0x1161..=0x1175).contains(&b) {
        return Some(0xac00 + (a - 0x1100) * 588 + (b - 0x1161) * 28);
    }
    if (0xac00..=0xd7a3).contains(&a) && (a - 0xac00) % 28 == 0 && (0x11a8..=0x11c2).contains(&b) {
        return Some(a + b - 0x11a7);
    }
    data::COMPOSE
        .binary_search_by_key(&(a, b), |r| (r.0, r.1))
        .ok()
        .map(|i| data::COMPOSE[i].2)
}

fn nfc(input: &[u32]) -> Vec<u32> {
    let mut decomposed = Vec::with_capacity(input.len());
    for cp in input {
        decompose(*cp, &mut decomposed);
    }
    // Canonical order: stable insertion among nonstarters.
    for i in 1..decomposed.len() {
        let mut j = i;
        while j > 0 && ccc(decomposed[j]) != 0 && ccc(decomposed[j - 1]) > ccc(decomposed[j]) {
            decomposed.swap(j - 1, j);
            j -= 1;
        }
    }
    let mut out = Vec::with_capacity(decomposed.len());
    let mut starter = None;
    let mut last_ccc = 0;
    for cp in decomposed {
        let class = ccc(cp);
        if let Some(i) = starter {
            if last_ccc < class || last_ccc == 0 {
                if let Some(composite) = compose(out[i], cp) {
                    out[i] = composite;
                    continue;
                }
            }
        }
        if class == 0 {
            starter = Some(out.len());
        }
        out.push(cp);
        last_ccc = class;
    }
    out
}

fn adapt(mut delta: u64, points: u64, first: bool) -> Option<u64> {
    delta = if first { delta / 700 } else { delta / 2 };
    delta = delta.checked_add(delta / points)?;
    let mut k = 0u64;
    while delta > 455 {
        delta /= 35;
        k = k.checked_add(36)?;
    }
    k.checked_add(36 * delta / (delta + 38))
}

fn digit(d: u64) -> char {
    if d < 26 {
        (b'a' + d as u8) as char
    } else {
        (b'0' + (d - 26) as u8) as char
    }
}

fn decode_digit(c: u8) -> Option<u64> {
    match c {
        b'a'..=b'z' => Some((c - b'a') as u64),
        b'A'..=b'Z' => Some((c - b'A') as u64),
        b'0'..=b'9' => Some((c - b'0' + 26) as u64),
        _ => None,
    }
}

/// RFC 3492 Punycode with checked arithmetic.
pub fn punycode_encode(input: &str) -> Result<String, ParseError> {
    let points: Vec<u64> = input.chars().map(|c| c as u64).collect();
    let mut out = String::new();
    for cp in &points {
        if *cp < 0x80 {
            out.push(*cp as u8 as char);
        }
    }
    let basic = out.len();
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u64, 0u64, 72u64);
    let mut handled = basic;
    while handled < points.len() {
        let m = *points
            .iter()
            .filter(|cp| **cp >= n)
            .min()
            .ok_or(ParseError::Host)?;
        delta = delta
            .checked_add(
                (m - n)
                    .checked_mul((handled + 1) as u64)
                    .ok_or(ParseError::Host)?,
            )
            .ok_or(ParseError::Host)?;
        n = m;
        for cp in &points {
            if *cp < n {
                delta = delta.checked_add(1).ok_or(ParseError::Host)?;
            }
            if *cp == n {
                let mut q = delta;
                let mut k = 36u64;
                loop {
                    let t = if k <= bias {
                        1
                    } else if k >= bias + 26 {
                        26
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (36 - t)));
                    q = (q - t) / (36 - t);
                    k = k.checked_add(36).ok_or(ParseError::Host)?;
                }
                out.push(digit(q));
                bias =
                    adapt(delta, (handled + 1) as u64, handled == basic).ok_or(ParseError::Host)?;
                delta = 0;
                handled += 1;
            }
        }
        delta = delta.checked_add(1).ok_or(ParseError::Host)?;
        n = n.checked_add(1).ok_or(ParseError::Host)?;
    }
    Ok(out)
}

/// Decode a Punycode payload without its `xn--` prefix.
pub fn punycode_decode(input: &str) -> Result<String, ParseError> {
    if !input.is_ascii() {
        return Err(ParseError::Host);
    }
    let bytes = input.as_bytes();
    let mut points = Vec::<u32>::new();
    let mut position = 0;
    if let Some(dash) = bytes.iter().rposition(|b| *b == b'-') {
        if bytes[..dash].iter().any(|b| !b.is_ascii()) {
            return Err(ParseError::Host);
        }
        points.extend(bytes[..dash].iter().map(|b| *b as u32));
        position = dash + 1;
    }
    let (mut n, mut i, mut bias) = (128u64, 0u64, 72u64);
    while position < bytes.len() {
        let old = i;
        let mut weight = 1u64;
        let mut k = 36u64;
        loop {
            let d = decode_digit(*bytes.get(position).ok_or(ParseError::Host)?)
                .ok_or(ParseError::Host)?;
            position += 1;
            i = i
                .checked_add(d.checked_mul(weight).ok_or(ParseError::Host)?)
                .ok_or(ParseError::Host)?;
            let t = if k <= bias {
                1
            } else if k >= bias + 26 {
                26
            } else {
                k - bias
            };
            if d < t {
                break;
            }
            weight = weight.checked_mul(36 - t).ok_or(ParseError::Host)?;
            k = k.checked_add(36).ok_or(ParseError::Host)?;
        }
        let len = points.len() as u64 + 1;
        bias = adapt(i - old, len, old == 0).ok_or(ParseError::Host)?;
        n = n.checked_add(i / len).ok_or(ParseError::Host)?;
        let cp = char::from_u32(u32::try_from(n).map_err(|_| ParseError::Host)?)
            .ok_or(ParseError::Host)?;
        let index = (i % len) as usize;
        points.insert(index, cp as u32);
        i = index as u64 + 1;
    }
    points
        .into_iter()
        .map(|cp| char::from_u32(cp).ok_or(ParseError::Host))
        .collect()
}

fn mapped(input: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for cp in input.chars().map(|c| c as u32) {
        let (status, replacement) = mapping(cp);
        match status {
            1 => out.extend_from_slice(replacement),
            2 => {}
            _ => out.push(cp), // valid, deviation, or disallowed
        }
    }
    nfc(&out)
}

fn bidi_class(cp: u32) -> u8 {
    range_value(data::BIDI, cp, 1)
}
fn joining_type(cp: u32) -> u8 {
    range_value(data::JOINING, cp, 0)
}

fn valid_joiners(label: &[u32]) -> bool {
    for (i, cp) in label.iter().enumerate() {
        if *cp != 0x200c && *cp != 0x200d {
            continue;
        }
        if i > 0 && ccc(label[i - 1]) == 9 {
            continue;
        }
        if *cp == 0x200d {
            return false;
        }
        let before = label[..i]
            .iter()
            .rev()
            .find(|cp| joining_type(**cp) != 5)
            .map(|cp| joining_type(*cp));
        let after = label[i + 1..]
            .iter()
            .find(|cp| joining_type(**cp) != 5)
            .map(|cp| joining_type(*cp));
        if !matches!(before, Some(2 | 3)) || !matches!(after, Some(1 | 3)) {
            return false;
        }
    }
    true
}

fn valid_bidi(label: &[u32]) -> bool {
    let Some(first) = label.first().map(|cp| bidi_class(*cp)) else {
        return true;
    };
    let rtl = first == 2 || first == 3;
    if !rtl && first != 1 {
        return false;
    }
    let mut en = false;
    let mut an = false;
    for cp in label {
        let class = bidi_class(*cp);
        if rtl {
            if !matches!(class, 2..=11) {
                return false;
            }
        } else if !matches!(class, 1 | 5..=11) {
            return false;
        }
        en |= class == 5;
        an |= class == 4;
    }
    if rtl && en && an {
        return false;
    }
    let last = label
        .iter()
        .rev()
        .map(|cp| bidi_class(*cp))
        .find(|class| *class != 11);
    if rtl {
        matches!(last, Some(2 | 3 | 4 | 5))
    } else {
        matches!(last, Some(1 | 5))
    }
}

fn validate_label(label: &[u32], from_punycode: bool, bidi_domain: bool) -> Result<(), ParseError> {
    if label.is_empty() {
        return Ok(());
    }
    if from_punycode && nfc(label) != label {
        return Err(ParseError::Host);
    }
    if is_mark(label[0]) {
        return Err(ParseError::Host);
    }
    if label.len() >= 4 && label[..4] == [120, 110, 45, 45] {
        return Err(ParseError::Host);
    }
    if label
        .iter()
        .any(|cp| *cp == 46 || !matches!(mapping(*cp).0, 0 | 3))
    {
        return Err(ParseError::Host);
    }
    if !valid_joiners(label) {
        return Err(ParseError::Host);
    }
    if bidi_domain && !valid_bidi(label) {
        return Err(ParseError::Host);
    }
    Ok(())
}

fn process(input: &str) -> Result<String, ParseError> {
    let normalized = mapped(input);
    let mut labels: Vec<Vec<u32>> = normalized
        .split(|cp| *cp == 46)
        .map(|s| s.to_vec())
        .collect();
    let mut punycode = Vec::with_capacity(labels.len());
    for label in &mut labels {
        let from_punycode = label.starts_with(&[120, 110, 45, 45]);
        if from_punycode {
            if label.iter().any(|cp| *cp > 127) {
                return Err(ParseError::Host);
            }
            let payload: String = label[4..].iter().map(|cp| *cp as u8 as char).collect();
            *label = punycode_decode(&payload)?
                .chars()
                .map(|c| c as u32)
                .collect();
            if label.is_empty() || label.iter().all(|cp| *cp < 128) {
                return Err(ParseError::Host);
            }
        }
        punycode.push(from_punycode);
    }
    let bidi_domain = labels
        .iter()
        .flatten()
        .any(|cp| matches!(bidi_class(*cp), 2 | 3 | 4));
    for (label, from_punycode) in labels.iter().zip(punycode) {
        validate_label(label, from_punycode, bidi_domain)?;
    }
    let mut result = Vec::with_capacity(labels.len());
    for label in labels {
        let s: String = label
            .into_iter()
            .map(|cp| char::from_u32(cp).unwrap())
            .collect();
        if s.is_ascii() {
            result.push(s);
        } else {
            result.push(format!("xn--{}", punycode_encode(&s)?));
        }
    }
    Ok(result.join("."))
}

/// URL Standard domain-to-ASCII with `beStrict=false` and Unicode 17.0 data.
/// ASCII input retains the URL Standard's compatibility behavior.
pub fn domain_to_ascii(domain: &str) -> Result<String, ParseError> {
    if domain.is_ascii() {
        return Ok(domain.to_ascii_lowercase());
    }
    process(domain)
}

/// URL Standard domain-to-Unicode. Invalid domains round-trip unchanged.
pub fn domain_to_unicode(domain: &str) -> String {
    let normalized = mapped(domain);
    let labels: Vec<_> = normalized
        .split(|cp| *cp == 46)
        .map(|s| s.to_vec())
        .collect();
    let mut converted = Vec::new();
    let mut flags = Vec::new();
    for mut label in labels {
        let from_punycode = label.starts_with(&[120, 110, 45, 45]);
        if from_punycode {
            if label.iter().any(|cp| *cp > 127) {
                return domain.into();
            }
            let payload: String = label[4..].iter().map(|cp| *cp as u8 as char).collect();
            let Ok(decoded) = punycode_decode(&payload) else {
                return domain.into();
            };
            label = decoded.chars().map(|c| c as u32).collect();
            if label.is_empty() || label.iter().all(|cp| *cp < 128) {
                return domain.into();
            }
        }
        flags.push(from_punycode);
        converted.push(label);
    }
    let bidi_domain = converted
        .iter()
        .flatten()
        .any(|cp| matches!(bidi_class(*cp), 2 | 3 | 4));
    if converted
        .iter()
        .zip(flags)
        .any(|(label, punycode)| validate_label(label, punycode, bidi_domain).is_err())
    {
        return domain.into();
    }
    converted
        .into_iter()
        .map(|label| {
            label
                .into_iter()
                .map(|cp| char::from_u32(cp).unwrap())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(".")
}
