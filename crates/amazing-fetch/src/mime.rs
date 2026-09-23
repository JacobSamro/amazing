//! MIME Sniffing §4 records and Fetch's Content-Type extraction.
use crate::HeaderList;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MimeType {
    pub type_: String,
    pub subtype: String,
    /// In insertion order. The first occurrence of a parameter wins.
    pub parameters: Vec<(String, String)>,
}

fn http_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}
fn token(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}
fn quoted(c: char) -> bool {
    c == '\t' || (' '..='~').contains(&c) || ('\u{80}'..='\u{ff}').contains(&c)
}

impl MimeType {
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim_matches(http_ws);
        let slash = input.find('/')?;
        let type_ = &input[..slash];
        if type_.is_empty() || !type_.chars().all(token) {
            return None;
        }
        let rest = &input[slash + 1..];
        let end = rest.find(';').unwrap_or(rest.len());
        let subtype = rest[..end].trim_end_matches(http_ws);
        if subtype.is_empty() || !subtype.chars().all(token) {
            return None;
        }
        let mut mime = Self {
            type_: type_.to_ascii_lowercase(),
            subtype: subtype.to_ascii_lowercase(),
            parameters: Vec::new(),
        };
        let mut pos = slash + 1 + end;
        while pos < input.len() {
            pos += 1; // semicolon
            while pos < input.len() && http_ws(input[pos..].chars().next().unwrap()) {
                pos += input[pos..].chars().next().unwrap().len_utf8();
            }
            let start = pos;
            while pos < input.len() && !matches!(input.as_bytes()[pos], b';' | b'=') {
                pos += input[pos..].chars().next().unwrap().len_utf8();
            }
            let name = input[start..pos].to_ascii_lowercase();
            if pos < input.len() && input.as_bytes()[pos] == b';' {
                continue;
            }
            if pos == input.len() {
                break;
            }
            pos += 1; // equals
            if pos == input.len() {
                break;
            }
            let value = if input.as_bytes()[pos] == b'"' {
                pos += 1;
                let mut value = String::new();
                while pos < input.len() {
                    let c = input[pos..].chars().next().unwrap();
                    pos += c.len_utf8();
                    if c == '"' {
                        break;
                    }
                    if c == '\\' {
                        if pos < input.len() {
                            let next = input[pos..].chars().next().unwrap();
                            value.push(next);
                            pos += next.len_utf8();
                        } else {
                            value.push('\\');
                            break;
                        }
                    } else {
                        value.push(c);
                    }
                }
                while pos < input.len() && input.as_bytes()[pos] != b';' {
                    pos += input[pos..].chars().next().unwrap().len_utf8();
                }
                value
            } else {
                let start = pos;
                while pos < input.len() && input.as_bytes()[pos] != b';' {
                    pos += input[pos..].chars().next().unwrap().len_utf8();
                }
                let value = input[start..pos].trim_end_matches(http_ws);
                if value.is_empty() {
                    continue;
                }
                value.to_owned()
            };
            if !name.is_empty()
                && name.chars().all(token)
                && value.chars().all(quoted)
                && !mime
                    .parameters
                    .iter()
                    .any(|(existing, _)| existing == &name)
            {
                mime.parameters.push((name, value));
            }
        }
        Some(mime)
    }

    pub fn essence(&self) -> String {
        format!("{}/{}", self.type_, self.subtype)
    }
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    pub fn set_parameter(&mut self, name: &str, value: &str) -> bool {
        if !name.chars().all(token) || name.is_empty() || !value.chars().all(quoted) {
            return false;
        }
        let name = name.to_ascii_lowercase();
        if let Some((_, old)) = self.parameters.iter_mut().find(|(key, _)| key == &name) {
            *old = value.to_owned();
        } else {
            self.parameters.push((name, value.to_owned()));
        }
        true
    }
    pub fn serialize(&self) -> String {
        let mut out = self.essence();
        for (name, value) in &self.parameters {
            out.push(';');
            out.push_str(name);
            out.push('=');
            if value.is_empty() || !value.chars().all(token) {
                out.push('"');
                for c in value.chars() {
                    if c == '"' || c == '\\' {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out.push('"');
            } else {
                out.push_str(value);
            }
        }
        out
    }
}

/// Fetch §3.5: the last valid Content-Type wins, with charset inheritance
/// across records of the same essence.
pub fn extract_mime_type(headers: &HeaderList) -> Option<MimeType> {
    let combined = headers.get("content-type")?;
    let mut last = None;
    let mut essence = None;
    let mut charset = None;
    for item in split_header_values(&combined) {
        let Some(mut mime) = MimeType::parse(item) else {
            continue;
        };
        if mime.essence() == "*/*" {
            continue;
        }
        let current = mime.essence();
        if essence.as_ref() != Some(&current) {
            charset = mime.parameter("charset").map(str::to_owned);
            essence = Some(current);
        } else if mime.parameter("charset").is_none() {
            if let Some(value) = &charset {
                mime.set_parameter("charset", value);
            }
        }
        last = Some(mime);
    }
    last
}

fn split_header_values(input: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, c) in input.char_indices() {
        if quoted && escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => {
                values.push(input[start..index].trim_matches([' ', '\t']));
                start = index + 1;
            }
            _ => {}
        }
    }
    values.push(input[start..].trim_matches([' ', '\t']));
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_parameters_and_quoted_values() {
        let record =
            MimeType::parse(" TEXT/HTML ; Charset=\"utf-8;late\";charset=gbk;x=\"a\\\"b\" ")
                .unwrap();
        assert_eq!(record.essence(), "text/html");
        assert_eq!(record.parameter("charset"), Some("utf-8;late"));
        assert_eq!(
            record.serialize(),
            "text/html;charset=\"utf-8;late\";x=\"a\\\"b\""
        );
    }

    #[test]
    fn fetch_extraction_keeps_charset_for_same_essence() {
        let mut headers = HeaderList::default();
        headers
            .append("Content-Type", "text/html;charset=gbk;a=b, text/html;x=y")
            .unwrap();
        assert_eq!(
            extract_mime_type(&headers).unwrap().serialize(),
            "text/html;x=y;charset=gbk"
        );
    }
}
