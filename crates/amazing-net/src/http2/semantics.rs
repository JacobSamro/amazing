use super::{Error, Field};
pub(super) fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$%&'*+-.^_`|~".contains(b))
}
pub(super) fn valid_value(value: &[u8]) -> bool {
    !value.iter().any(|&b| matches!(b, 0 | 10 | 13))
        && !value.first().is_some_and(|b| matches!(b, b' ' | b'\t'))
        && !value.last().is_some_and(|b| matches!(b, b' ' | b'\t'))
}
fn regular(name: &[u8], value: &[u8]) -> Result<(), Error> {
    if !valid_name(name)
        || !valid_value(value)
        || matches!(
            name,
            b"connection" | b"proxy-connection" | b"keep-alive" | b"transfer-encoding" | b"upgrade"
        )
        || name == b"te" && value != b"trailers"
    {
        return Err(Error::Protocol);
    }
    Ok(())
}
pub(super) fn request(fields: &[Field]) -> Result<bool, Error> {
    let mut seen = 0;
    let mut ordinary = false;
    let mut head = false;
    for (name, value) in fields {
        if name.starts_with(b":") {
            if ordinary || !valid_value(value) || value.is_empty() {
                return Err(Error::Protocol);
            }
            let flag = match name.as_slice() {
                b":method" => {
                    head = value == b"HEAD";
                    if value == b"CONNECT"
                        || !value
                            .iter()
                            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(b))
                    {
                        return Err(Error::Protocol);
                    }
                    1
                }
                b":scheme" => {
                    if value != b"https" && value != b"http" {
                        return Err(Error::Protocol);
                    }
                    2
                }
                b":authority" => 4,
                b":path" => {
                    if value[0] != b'/' && value != b"*" {
                        return Err(Error::Protocol);
                    }
                    8
                }
                _ => return Err(Error::Protocol),
            };
            if seen & flag != 0 {
                return Err(Error::Protocol);
            }
            seen |= flag;
        } else {
            ordinary = true;
            regular(name, value)?;
        }
    }
    if seen != 15 {
        return Err(Error::Protocol);
    }
    Ok(head)
}
pub(super) fn response(fields: &[Field], trailers: bool) -> Result<Option<u16>, Error> {
    let mut status = None;
    let mut ordinary = false;
    for (name, value) in fields {
        if name.starts_with(b":") {
            if trailers
                || ordinary
                || name != b":status"
                || status.is_some()
                || value.len() != 3
                || !value.iter().all(u8::is_ascii_digit)
            {
                return Err(Error::Protocol);
            }
            let code = ((value[0] - b'0') as u16) * 100
                + ((value[1] - b'0') as u16) * 10
                + (value[2] - b'0') as u16;
            if !(100..600).contains(&code) || code == 101 {
                return Err(Error::Protocol);
            }
            status = Some(code);
        } else {
            ordinary = true;
            regular(name, value)?;
            if name == b"te" {
                return Err(Error::Protocol);
            }
            if trailers
                && matches!(
                    name.as_slice(),
                    b"content-length" | b"host" | b"content-encoding"
                )
            {
                return Err(Error::Protocol);
            }
        }
    }
    if !trailers && status.is_none() {
        return Err(Error::Protocol);
    }
    Ok(status)
}
pub(super) fn length(fields: &[Field]) -> Result<Option<usize>, Error> {
    let mut length = None;
    for (name, value) in fields {
        if name != b"content-length" {
            continue;
        }
        for part in value.split(|&b| b == b',') {
            let s = std::str::from_utf8(part)
                .map_err(|_| Error::Protocol)?
                .trim_matches([' ', '\t']);
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::Protocol);
            }
            let n = s.parse::<usize>().map_err(|_| Error::Protocol)?;
            if length.is_some_and(|old| old != n) {
                return Err(Error::Protocol);
            }
            length = Some(n);
        }
    }
    Ok(length)
}
