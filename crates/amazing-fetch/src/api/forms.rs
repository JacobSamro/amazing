//! Native Blob, File and FormData value models and multipart serialization.
use super::ApiError;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobPart {
    Bytes(Vec<u8>),
    Text(String),
    Blob(Blob),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub bytes: Vec<u8>,
    pub content_type: String,
}
impl Blob {
    pub fn new(parts: impl IntoIterator<Item = BlobPart>, content_type: &str) -> Self {
        let mut bytes = Vec::new();
        for part in parts {
            match part {
                BlobPart::Bytes(b) => bytes.extend(b),
                BlobPart::Text(s) => bytes.extend(s.as_bytes()),
                BlobPart::Blob(b) => bytes.extend(b.bytes),
            }
        }
        Self {
            bytes,
            content_type: normalize_type(content_type),
        }
    }
    pub fn from_bytes(bytes: Vec<u8>, content_type: &str) -> Self {
        Self {
            bytes,
            content_type: normalize_type(content_type),
        }
    }
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
    pub fn slice(&self, start: Option<i64>, end: Option<i64>, content_type: &str) -> Self {
        let size = self.bytes.len() as i64;
        let index = |value: i64| {
            if value < 0 {
                (size + value).max(0) as usize
            } else {
                value.min(size) as usize
            }
        };
        let start = index(start.unwrap_or(0));
        let end = index(end.unwrap_or(size)).max(start);
        Self::from_bytes(self.bytes[start..end].to_vec(), content_type)
    }
}
fn normalize_type(input: &str) -> String {
    if input.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        input.to_ascii_lowercase()
    } else {
        String::new()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub blob: Blob,
    pub name: String,
    pub last_modified: i64,
}
impl File {
    pub fn new(
        parts: impl IntoIterator<Item = BlobPart>,
        name: &str,
        content_type: &str,
        last_modified: i64,
    ) -> Self {
        Self {
            blob: Blob::new(parts, content_type),
            name: name.into(),
            last_modified,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormValue {
    Text(String),
    File(File),
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormData {
    pub entries: Vec<(String, FormValue)>,
}
impl FormData {
    pub fn append_text(&mut self, name: &str, value: &str) {
        self.entries
            .push((name.into(), FormValue::Text(value.into())));
    }
    pub fn append_file(&mut self, name: &str, file: File, filename: Option<&str>) {
        let mut file = file;
        if let Some(filename) = filename {
            file.name = filename.into();
        }
        self.entries.push((name.into(), FormValue::File(file)));
    }
    pub fn get(&self, name: &str) -> Option<&FormValue> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    pub fn get_all(&self, name: &str) -> Vec<&FormValue> {
        self.entries
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    pub fn delete(&mut self, name: &str) {
        self.entries.retain(|(n, _)| n != name);
    }
    pub fn set(&mut self, name: &str, value: FormValue) {
        let mut seen = false;
        self.entries.retain_mut(|(n, v)| {
            if n != name {
                true
            } else if !seen {
                *v = value.clone();
                seen = true;
                true
            } else {
                false
            }
        });
        if !seen {
            self.entries.push((name.into(), value));
        }
    }
    pub fn encode_multipart(&self) -> (Vec<u8>, String) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let boundary = format!("----amazing-{stamp:032x}-{serial:016x}");
        let mut out = Vec::new();
        for (name, value) in &self.entries {
            out.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"",
                    escape(name)
                )
                .as_bytes(),
            );
            match value {
                FormValue::Text(text) => {
                    out.extend_from_slice(b"\r\n\r\n");
                    out.extend_from_slice(normalize_lines(text).as_bytes());
                }
                FormValue::File(file) => {
                    out.extend_from_slice(
                        format!(
                            "; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                            escape(&file.name),
                            if file.blob.content_type.is_empty() {
                                "application/octet-stream"
                            } else {
                                &file.blob.content_type
                            }
                        )
                        .as_bytes(),
                    );
                    out.extend_from_slice(&file.blob.bytes);
                }
            }
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (out, format!("multipart/form-data; boundary={boundary}"))
    }
    pub fn decode_multipart(bytes: &[u8], boundary: &str) -> Result<Self, ApiError> {
        if boundary.is_empty() || !boundary.is_ascii() || boundary.contains(['\r', '\n']) {
            return Err(ApiError::InvalidFormData);
        }
        let marker = format!("--{boundary}").into_bytes();
        if !bytes.starts_with(&marker) {
            return Err(ApiError::InvalidFormData);
        }
        let mut pos = 0;
        let mut result = Self::default();
        loop {
            if !bytes[pos..].starts_with(&marker) {
                return Err(ApiError::InvalidFormData);
            }
            pos += marker.len();
            if bytes[pos..].starts_with(b"--") {
                return Ok(result);
            }
            if !bytes[pos..].starts_with(b"\r\n") {
                return Err(ApiError::InvalidFormData);
            }
            pos += 2;
            let header_end = find(&bytes[pos..], b"\r\n\r\n").ok_or(ApiError::InvalidFormData)?;
            let headers = String::from_utf8_lossy(&bytes[pos..pos + header_end]);
            pos += header_end + 4;
            let next_marker = [b"\r\n--".as_slice(), boundary.as_bytes()].concat();
            let body_end = find(&bytes[pos..], &next_marker).ok_or(ApiError::InvalidFormData)?;
            let body = &bytes[pos..pos + body_end];
            pos += body_end + 2;
            let disposition = headers
                .lines()
                .find(|h| h.to_ascii_lowercase().starts_with("content-disposition:"))
                .ok_or(ApiError::InvalidFormData)?;
            let name = quoted_parameter(disposition, "name").ok_or(ApiError::InvalidFormData)?;
            if let Some(filename) = quoted_parameter(disposition, "filename") {
                let ty = headers
                    .lines()
                    .find_map(|h| {
                        h.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                            .map(|(_, v)| v.trim())
                    })
                    .unwrap_or("text/plain");
                result.append_file(
                    &name,
                    File {
                        blob: Blob::from_bytes(body.to_vec(), ty),
                        name: filename,
                        last_modified: 0,
                    },
                    None,
                );
            } else {
                result.append_text(&name, &String::from_utf8_lossy(body));
            }
        }
    }
}
fn normalize_lines(input: &str) -> String {
    input
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}
fn escape(input: &str) -> String {
    let mut out = String::new();
    for ch in normalize_lines(input).chars() {
        match ch {
            '\r' | '\n' | '"' => out.push_str(&format!("%{:02X}", ch as u8)),
            _ => out.push(ch),
        }
    }
    out
}
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
fn quoted_parameter(input: &str, name: &str) -> Option<String> {
    let mut quoted = false;
    let mut start = 0;
    let mut value = None;
    for (index, ch) in input
        .char_indices()
        .chain(std::iter::once((input.len(), ';')))
    {
        if ch == '"' {
            quoted = !quoted;
        }
        if ch == ';' && !quoted {
            if start != 0 {
                if let Some((key, candidate)) = input[start..index].trim().split_once('=') {
                    if key.trim().eq_ignore_ascii_case(name) {
                        value = Some(candidate.trim());
                        break;
                    }
                }
            }
            start = index + 1;
        }
    }
    let value = value?;
    let raw = value.strip_prefix('"')?.strip_suffix('"')?.as_bytes();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let (Some(a), Some(b)) = (
                (raw[i + 1] as char).to_digit(16),
                (raw[i + 2] as char).to_digit(16),
            ) {
                bytes.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
