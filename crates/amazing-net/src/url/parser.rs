//! Basic URL parser with an optional document query encoder. Each enum arm is
//! the corresponding named WHATWG state; validation errors recover unless the
//! algorithm explicitly returns failure. Inputs are scalar-value strings.
use super::encoding::{percent_encode, PercentEncodeSet as Set};
use super::host::{parse_host, Host, IdnaProvider};
use super::parsed::{default_port, ParseError, ParsedUrl, Path};
#[derive(Debug, Clone, Copy)]
enum State {
    SchemeStart,
    Scheme,
    NoScheme,
    SpecialRelativeOrAuthority,
    PathOrAuthority,
    Relative,
    RelativeSlash,
    SpecialAuthoritySlashes,
    SpecialAuthorityIgnoreSlashes,
    Authority,
    Host,
    Port,
    File,
    FileSlash,
    FileHost,
    PathStart,
    Path,
    OpaquePath,
    Query,
    Fragment,
}
fn drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && matches!(b[1], b':' | b'|')
}
fn starts_drive(c: &[char]) -> bool {
    c.len() >= 2
        && c[0].is_ascii_alphabetic()
        && matches!(c[1], ':' | '|')
        && (c.len() == 2 || matches!(c[2], '/' | '\\' | '?' | '#'))
}
fn shorten(url: &mut ParsedUrl) {
    if let Path::Segments(p) = &mut url.path {
        if !(url.scheme == "file" && p.len() == 1 && drive(&p[0])) {
            p.pop();
        }
    }
}
fn push(url: &mut ParsedUrl, s: String) {
    if let Path::Segments(p) = &mut url.path {
        p.push(s);
    } else {
        unreachable!("path state on opaque path")
    }
}
fn copy_authority(url: &mut ParsedUrl, base: &ParsedUrl) {
    url.username = base.username.clone();
    url.password = base.password.clone();
    url.host = base.host.clone();
    url.port = base.port;
}
fn delimiter(c: Option<char>, special: bool) -> bool {
    matches!(c, None | Some('/' | '?' | '#')) || (special && c == Some('\\'))
}
pub(super) fn parse(
    input: &str,
    base: Option<&ParsedUrl>,
    idna: &dyn IdnaProvider,
    query_encoder: Option<&dyn Fn(&str) -> Vec<u8>>,
) -> Result<ParsedUrl, ParseError> {
    let chars: Vec<char> = input
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let mut url = ParsedUrl {
        scheme: String::new(),
        username: String::new(),
        password: String::new(),
        host: None,
        port: None,
        path: Path::Segments(vec![]),
        query: None,
        fragment: None,
    };
    let mut state = State::SchemeStart;
    let mut buffer = String::new();
    let mut i = 0usize;
    let mut at_seen = false;
    let mut password_seen = false;
    let mut brackets = false;
    loop {
        let c = chars.get(i).copied();
        let next = chars.get(i + 1).copied();
        let special = url.is_special();
        // Continue reconsumes the current code point, matching the spec's
        // pointer decrement followed by its common increment.
        match state {
            State::SchemeStart => {
                if c.is_some_and(|c| c.is_ascii_alphabetic()) {
                    buffer.push(c.unwrap().to_ascii_lowercase());
                    state = State::Scheme;
                } else {
                    state = State::NoScheme;
                    continue;
                }
            }
            State::Scheme => {
                if c.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
                    buffer.push(c.unwrap().to_ascii_lowercase());
                } else if c == Some(':') {
                    url.scheme = std::mem::take(&mut buffer);
                    state = if url.scheme == "file" {
                        State::File
                    } else if url.is_special() && base.is_some_and(|b| b.scheme == url.scheme) {
                        State::SpecialRelativeOrAuthority
                    } else if url.is_special() {
                        State::SpecialAuthoritySlashes
                    } else if next == Some('/') {
                        i += 1;
                        State::PathOrAuthority
                    } else {
                        url.path = Path::Opaque(String::new());
                        State::OpaquePath
                    };
                } else {
                    buffer.clear();
                    state = State::NoScheme;
                    i = 0;
                    continue;
                }
            }
            State::NoScheme => {
                let b = base.ok_or(ParseError::MissingSchemeOrBase)?;
                if b.has_opaque_path() {
                    if c != Some('#') {
                        return Err(ParseError::MissingSchemeOrBase);
                    }
                    url.scheme = b.scheme.clone();
                    url.path = b.path.clone();
                    url.query = b.query.clone();
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else {
                    state = if b.scheme == "file" {
                        State::File
                    } else {
                        State::Relative
                    };
                    continue;
                }
            }
            State::SpecialRelativeOrAuthority => {
                if c == Some('/') && next == Some('/') {
                    state = State::SpecialAuthorityIgnoreSlashes;
                    i += 1;
                } else {
                    state = State::Relative;
                    continue;
                }
            }
            State::PathOrAuthority => {
                if c == Some('/') {
                    state = State::Authority;
                } else {
                    state = State::Path;
                    continue;
                }
            }
            State::Relative => {
                let b = base.ok_or(ParseError::MissingSchemeOrBase)?;
                url.scheme = b.scheme.clone();
                if c == Some('/') || (url.is_special() && c == Some('\\')) {
                    state = State::RelativeSlash;
                } else {
                    copy_authority(&mut url, b);
                    url.path = b.path.clone();
                    url.query = b.query.clone();
                    match c {
                        Some('?') => {
                            url.query = Some(String::new());
                            state = State::Query;
                        }
                        Some('#') => {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        }
                        Some(_) => {
                            url.query = None;
                            shorten(&mut url);
                            state = State::Path;
                            continue;
                        }
                        None => return Ok(url),
                    }
                }
            }
            State::RelativeSlash => {
                if special && matches!(c, Some('/' | '\\')) {
                    state = State::SpecialAuthorityIgnoreSlashes;
                } else if c == Some('/') {
                    state = State::Authority;
                } else {
                    copy_authority(&mut url, base.ok_or(ParseError::MissingSchemeOrBase)?);
                    state = State::Path;
                    continue;
                }
            }
            State::SpecialAuthoritySlashes => {
                state = State::SpecialAuthorityIgnoreSlashes;
                if c == Some('/') && next == Some('/') {
                    i += 1;
                } else {
                    continue;
                }
            }
            State::SpecialAuthorityIgnoreSlashes => {
                if !matches!(c, Some('/' | '\\')) {
                    state = State::Authority;
                    continue;
                }
            }
            State::Authority => {
                if c == Some('@') {
                    if at_seen {
                        buffer.insert_str(0, "%40");
                    }
                    at_seen = true;
                    for p in buffer.chars() {
                        if p == ':' && !password_seen {
                            password_seen = true;
                            continue;
                        }
                        let encoded = percent_encode(&p.to_string(), Set::Userinfo);
                        if password_seen {
                            url.password.push_str(&encoded);
                        } else {
                            url.username.push_str(&encoded);
                        }
                    }
                    buffer.clear();
                } else if delimiter(c, special) {
                    if at_seen && buffer.is_empty() {
                        return Err(ParseError::Host);
                    }
                    i -= buffer.chars().count();
                    buffer.clear();
                    state = State::Host;
                    continue;
                } else {
                    buffer.push(c.unwrap());
                }
            }
            State::Host => {
                if c == Some(':') && !brackets {
                    if buffer.is_empty() {
                        return Err(ParseError::Host);
                    }
                    url.host = Some(parse_host(&buffer, special, idna)?);
                    buffer.clear();
                    state = State::Port;
                } else if delimiter(c, special) {
                    if special && buffer.is_empty() {
                        return Err(ParseError::Host);
                    }
                    url.host = Some(parse_host(&buffer, special, idna)?);
                    buffer.clear();
                    state = State::PathStart;
                    continue;
                } else {
                    if c == Some('[') {
                        brackets = true;
                    }
                    if c == Some(']') {
                        brackets = false;
                    }
                    buffer.push(c.unwrap());
                }
            }
            State::Port => {
                if c.is_some_and(|c| c.is_ascii_digit()) {
                    buffer.push(c.unwrap());
                } else if delimiter(c, special) {
                    if !buffer.is_empty() {
                        // Leading zeroes do not cause artificial integer overflow.
                        let number = buffer
                            .bytes()
                            .try_fold(0u16, |n, b| {
                                n.checked_mul(10)?.checked_add((b - b'0') as u16)
                            })
                            .ok_or(ParseError::Port)?;
                        if Some(number) != default_port(&url.scheme) {
                            url.port = Some(number);
                        }
                    }
                    buffer.clear();
                    state = State::PathStart;
                    continue;
                } else {
                    return Err(ParseError::Port);
                }
            }
            State::File => {
                url.scheme = "file".into();
                url.host = Some(Host::Empty);
                if matches!(c, Some('/' | '\\')) {
                    state = State::FileSlash;
                } else if let Some(b) = base.filter(|b| b.scheme == "file") {
                    url.host = b.host.clone();
                    url.path = b.path.clone();
                    url.query = b.query.clone();
                    match c {
                        Some('?') => {
                            url.query = Some(String::new());
                            state = State::Query;
                        }
                        Some('#') => {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        }
                        Some(_) => {
                            url.query = None;
                            if !starts_drive(&chars[i..]) {
                                shorten(&mut url);
                            } else {
                                url.path = Path::Segments(vec![]);
                            }
                            state = State::Path;
                            continue;
                        }
                        None => return Ok(url),
                    }
                } else {
                    state = State::Path;
                    continue;
                }
            }
            State::FileSlash => {
                if matches!(c, Some('/' | '\\')) {
                    state = State::FileHost;
                } else {
                    if let Some(b) = base.filter(|b| b.scheme == "file") {
                        url.host = b.host.clone();
                        if !starts_drive(&chars[i..]) {
                            if let Path::Segments(p) = &b.path {
                                if let Some(first) = p.first().filter(|s| drive(s)) {
                                    push(&mut url, first.clone());
                                }
                            }
                        }
                    }
                    state = State::Path;
                    continue;
                }
            }
            State::FileHost => {
                if delimiter(c, true) {
                    if drive(&buffer) {
                        state = State::Path;
                    } else if buffer.is_empty() {
                        url.host = Some(Host::Empty);
                        state = State::PathStart;
                    } else {
                        let host = parse_host(&buffer, true, idna)?;
                        url.host = Some(if matches!(&host,Host::Domain(s) if s=="localhost") {
                            Host::Empty
                        } else {
                            host
                        });
                        buffer.clear();
                        state = State::PathStart;
                    }
                    continue;
                } else {
                    buffer.push(c.unwrap());
                }
            }
            State::PathStart => {
                if special {
                    state = State::Path;
                    if !matches!(c, Some('/' | '\\')) {
                        continue;
                    }
                } else if c == Some('?') {
                    url.query = Some(String::new());
                    state = State::Query;
                } else if c == Some('#') {
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if c.is_some() {
                    state = State::Path;
                    if c != Some('/') {
                        continue;
                    }
                } else {
                    return Ok(url);
                }
            }
            State::Path => {
                if delimiter(c, special) {
                    let lower = buffer.to_ascii_lowercase();
                    let slash = c == Some('/') || (special && c == Some('\\'));
                    if matches!(lower.as_str(), ".." | ".%2e" | "%2e." | "%2e%2e") {
                        shorten(&mut url);
                        if !slash {
                            push(&mut url, String::new());
                        }
                    } else if matches!(lower.as_str(), "." | "%2e") {
                        if !slash {
                            push(&mut url, String::new());
                        }
                    } else {
                        if url.scheme == "file"
                            && matches!(&url.path,Path::Segments(p) if p.is_empty())
                            && drive(&buffer)
                        {
                            buffer.replace_range(1..2, ":");
                        }
                        push(&mut url, std::mem::take(&mut buffer));
                    }
                    buffer.clear();
                    if c == Some('?') {
                        url.query = Some(String::new());
                        state = State::Query;
                    }
                    if c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    }
                } else {
                    buffer.push_str(&percent_encode(&c.unwrap().to_string(), Set::Path));
                }
            }
            State::OpaquePath => match c {
                Some('?') => {
                    url.query = Some(String::new());
                    state = State::Query;
                }
                Some('#') => {
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                }
                Some(p) => {
                    if let Path::Opaque(path) = &mut url.path {
                        if p == ' ' && matches!(next, Some('?' | '#')) {
                            path.push_str("%20");
                        } else {
                            path.push_str(&percent_encode(&p.to_string(), Set::C0Control));
                        }
                    }
                }
                None => (),
            },
            State::Query => {
                // Encode the complete buffer, rather than one scalar at a time:
                // ISO-2022-JP maintains escape state until the buffer is flushed.
                if c.is_none() || c == Some('#') {
                    let bytes = if special && !matches!(url.scheme.as_str(), "ws" | "wss") {
                        query_encoder.map(|encode| encode(&buffer))
                    } else {
                        None
                    };
                    let encoded = super::encoding::percent_encode_bytes(
                        bytes.as_deref().unwrap_or(buffer.as_bytes()),
                        if special { Set::SpecialQuery } else { Set::Query },
                    );
                    url.query.as_mut().unwrap().push_str(&encoded);
                    buffer.clear();
                }
                if c == Some('#') {
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if let Some(p) = c {
                    buffer.push(p);
                }
            }
            State::Fragment => {
                if let Some(p) = c {
                    url.fragment
                        .as_mut()
                        .unwrap()
                        .push_str(&percent_encode(&p.to_string(), Set::Fragment));
                }
            }
        }
        if c.is_none() {
            return Ok(url);
        }
        i += 1;
    }
}
