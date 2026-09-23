//! URL Standard state overrides for the mutable URL facade. Failed setter
//! parses leave the affected component unchanged, except host's port state:
//! the host can be accepted before an invalid port is encountered.
use super::encoding::{percent_encode, PercentEncodeSet as Set};
use super::host::{parse_host, Host};
use super::idna::Idna;
use super::parsed::{default_port, is_special, ParseError, ParsedUrl, Path};

fn strip_newlines(input: &str) -> String {
    input
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect()
}
fn can_have_credentials(url: &ParsedUrl) -> bool {
    url.scheme != "file" && url.host.as_ref().is_some_and(|h| *h != Host::Empty)
}
fn parse_port(value: &str, scheme: &str) -> Option<Option<u16>> {
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let number = digits.bytes().try_fold(0u16, |n, b| {
        n.checked_mul(10)?.checked_add((b - b'0') as u16)
    })?;
    Some((Some(number) != default_port(scheme)).then_some(number))
}
fn dot_kind(segment: &str) -> u8 {
    match segment.to_ascii_lowercase().as_str() {
        "." | "%2e" => 1,
        ".." | ".%2e" | "%2e." | "%2e%2e" => 2,
        _ => 0,
    }
}
fn drive(segment: &str) -> bool {
    let b = segment.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && matches!(b[1], b':' | b'|')
}
impl ParsedUrl {
    /// Set `href`; unlike component setters, invalid input is an error.
    pub fn set_href(&mut self, input: &str) -> Result<(), ParseError> {
        *self = Self::parse(input)?;
        Ok(())
    }
    pub fn set_protocol(&mut self, input: &str) {
        let cleaned = strip_newlines(input);
        let scheme = cleaned.split(':').next().unwrap_or("");
        let mut chars = scheme.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return;
        }
        let scheme = scheme.to_ascii_lowercase();
        if is_special(&scheme) != self.is_special() {
            return;
        }
        if (self.includes_credentials() || self.port.is_some()) && scheme == "file" {
            return;
        }
        if self.scheme == "file" && matches!(self.host, Some(Host::Empty)) {
            return;
        }
        if is_special(&scheme) && self.host.is_none() {
            return;
        }
        self.scheme = scheme;
        if self.port == default_port(&self.scheme) {
            self.port = None;
        }
    }
    pub fn set_username(&mut self, input: &str) {
        if can_have_credentials(self) {
            self.username = percent_encode(input, Set::Userinfo);
        }
    }
    pub fn set_password(&mut self, input: &str) {
        if can_have_credentials(self) {
            self.password = percent_encode(input, Set::Userinfo);
        }
    }
    pub fn set_host(&mut self, input: &str) {
        self.set_host_state(input, false);
    }
    pub fn set_hostname(&mut self, input: &str) {
        self.set_host_state(input, true);
    }
    fn set_host_state(&mut self, input: &str, hostname_only: bool) {
        if self.has_opaque_path() {
            return;
        }
        let cleaned = strip_newlines(input);
        let special = self.is_special();
        let end = cleaned
            .char_indices()
            .find_map(|(i, c)| {
                (matches!(c, '/' | '?' | '#') || (special && c == '\\')).then_some(i)
            })
            .unwrap_or(cleaned.len());
        let authority = &cleaned[..end];
        let mut bracketed = false;
        let colon = authority.char_indices().find_map(|(i, c)| {
            if c == '[' {
                bracketed = true;
            }
            if c == ']' {
                bracketed = false;
            }
            (c == ':' && !bracketed).then_some(i)
        });
        if hostname_only && colon.is_some() {
            return;
        }
        if self.scheme == "file" && colon.is_some() {
            return;
        }
        let host_input = colon.map_or(authority, |i| &authority[..i]);
        if host_input.is_empty()
            && (special && self.scheme != "file"
                || self.includes_credentials()
                || self.port.is_some()
                || colon.is_some())
        {
            return;
        }
        let parsed = if self.scheme == "file" && host_input.is_empty() {
            Host::Empty
        } else {
            match parse_host(host_input, special, &Idna) {
                Ok(Host::Domain(s)) if self.scheme == "file" && s == "localhost" => Host::Empty,
                Ok(host) => host,
                Err(_) => return,
            }
        };
        self.host = Some(parsed);
        if let Some(index) = colon {
            if let Some(port) = parse_port(&authority[index + 1..], &self.scheme) {
                self.port = port;
            }
        }
    }
    pub fn set_port(&mut self, input: &str) {
        if !can_have_credentials(self) {
            return;
        }
        if input.is_empty() {
            self.port = None;
            return;
        }
        if let Some(port) = parse_port(&strip_newlines(input), &self.scheme) {
            self.port = port;
        }
    }
    pub fn set_pathname(&mut self, input: &str) {
        if self.has_opaque_path() {
            return;
        }
        let special = self.is_special();
        let cleaned = strip_newlines(input);
        let mut chars = cleaned.chars().peekable();
        if chars
            .peek()
            .is_some_and(|c| *c == '/' || (special && *c == '\\'))
        {
            chars.next();
        }
        let mut parts = Vec::new();
        let mut buffer = String::new();
        for next in chars.map(Some).chain(std::iter::once(None)) {
            let separator = next.is_none() || next == Some('/') || (special && next == Some('\\'));
            if separator {
                let final_segment = next.is_none();
                match dot_kind(&buffer) {
                    1 => {
                        if final_segment {
                            parts.push(String::new());
                        }
                    }
                    2 => {
                        if !(self.scheme == "file" && parts.len() == 1 && drive(&parts[0])) {
                            parts.pop();
                        }
                        if final_segment {
                            parts.push(String::new());
                        }
                    }
                    _ => {
                        if self.scheme == "file" && parts.is_empty() && drive(&buffer) {
                            buffer.replace_range(1..2, ":");
                        }
                        parts.push(std::mem::take(&mut buffer));
                    }
                }
                buffer.clear();
            } else {
                buffer.push_str(&percent_encode(&next.unwrap().to_string(), Set::Path));
            }
        }
        if cleaned.is_empty() && !special && self.host.is_some() {
            parts.clear();
        }
        self.path = Path::Segments(parts);
    }
    pub fn set_search(&mut self, input: &str) {
        if input.is_empty() {
            self.query = None;
            return;
        }
        let input = input.strip_prefix('?').unwrap_or(input);
        let set = if self.is_special() {
            Set::SpecialQuery
        } else {
            Set::Query
        };
        self.query = Some(percent_encode(&strip_newlines(input), set));
    }
    pub fn set_hash(&mut self, input: &str) {
        if input.is_empty() {
            self.fragment = None;
            return;
        }
        let input = input.strip_prefix('#').unwrap_or(input);
        self.fragment = Some(percent_encode(&strip_newlines(input), Set::Fragment));
    }
}
