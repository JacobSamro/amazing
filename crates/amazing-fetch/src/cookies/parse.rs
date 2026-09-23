use super::date::parse_date;
use super::{CookieError, SameSite, MAX_LIFETIME};

pub(super) struct Parsed {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    pub domain: Option<Vec<u8>>,
    pub path: Option<Vec<u8>>,
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}
fn trim(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|b| matches!(b, b' ' | b'\t')) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(|b| matches!(b, b' ' | b'\t')) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}
fn pair(bytes: &[u8]) -> (&[u8], &[u8]) {
    bytes
        .iter()
        .position(|b| *b == b'=')
        .map_or((&[][..], bytes), |p| (&bytes[..p], &bytes[p + 1..]))
}
fn max_age(value: &[u8], now: i64) -> Option<i64> {
    let (negative, digits) = if value.first() == Some(&b'-') {
        (true, &value[1..])
    } else {
        (false, value)
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let seconds = digits.iter().fold(0i64, |n, b| {
        (n.saturating_mul(10) + (b - b'0') as i64).min(MAX_LIFETIME)
    });
    Some(if negative || seconds == 0 {
        i64::MIN
    } else {
        now.saturating_add(seconds)
    })
}
pub(super) fn parse(input: &[u8], now: i64, default_path: &[u8]) -> Result<Parsed, CookieError> {
    // Implementation policy bounds parsing work, including unknown attributes.
    if input.len() > 65536 {
        return Err(CookieError::TooLarge);
    }
    if input.iter().any(|b| matches!(b,0..=8|10..=31|127)) {
        return Err(CookieError::ControlCharacter);
    }
    let mut parts = input.split(|b| *b == b';');
    let (name, value) = pair(parts.next().unwrap_or_default());
    let (name, value) = (trim(name), trim(value));
    if name.is_empty() && value.is_empty() {
        return Err(CookieError::Empty);
    }
    if name.len() + value.len() > 4096 {
        return Err(CookieError::TooLarge);
    }
    let mut cookie = Parsed {
        name: name.to_vec(),
        value: value.to_vec(),
        domain: None,
        path: None,
        expires: None,
        secure: false,
        http_only: false,
        same_site: SameSite::Default,
    };
    let mut age = None;
    for part in parts {
        let (name, value) = if part.contains(&b'=') {
            pair(part)
        } else {
            (part, &[][..])
        };
        let (name, value) = (trim(name), trim(value));
        if value.len() > 1024 {
            continue;
        }
        if name.eq_ignore_ascii_case(b"expires") {
            if let Some(time) = parse_date(value) {
                cookie.expires = Some(time.min(now.saturating_add(MAX_LIFETIME)));
            }
        } else if name.eq_ignore_ascii_case(b"max-age") {
            if let Some(time) = max_age(value, now) {
                age = Some(time);
            }
        } else if name.eq_ignore_ascii_case(b"domain") {
            cookie.domain = Some(
                // A bare Domain= is ignored, but Domain=. remains invalid.
                // Stripping its only dot would turn it into a host-only cookie.
                if value == b"." {
                    value
                } else {
                    value.strip_prefix(b".").unwrap_or(value)
                }
                .to_ascii_lowercase(),
            );
        } else if name.eq_ignore_ascii_case(b"path") {
            cookie.path = Some(
                if value.starts_with(b"/") {
                    value
                } else {
                    default_path
                }
                .to_vec(),
            );
        } else if name.eq_ignore_ascii_case(b"secure") {
            cookie.secure = true;
        } else if name.eq_ignore_ascii_case(b"httponly") {
            cookie.http_only = true;
        } else if name.eq_ignore_ascii_case(b"samesite") {
            cookie.same_site = if value.eq_ignore_ascii_case(b"strict") {
                SameSite::Strict
            } else if value.eq_ignore_ascii_case(b"lax") {
                SameSite::Lax
            } else if value.eq_ignore_ascii_case(b"none") {
                SameSite::None
            } else {
                SameSite::Default
            };
        }
    }
    if age.is_some() {
        cookie.expires = age;
    }
    Ok(cookie)
}
