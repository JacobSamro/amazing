use super::{byte_string, ApiError};
use crate::{safelisted_request_header, HeaderList};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderGuard {
    Immutable,
    Request,
    RequestNoCors,
    Response,
    None,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiHeaders {
    guard: HeaderGuard,
    entries: Vec<(String, String)>,
    wire_names: BTreeMap<String, String>,
}
impl Default for ApiHeaders {
    fn default() -> Self {
        Self::new(HeaderGuard::None)
    }
}
impl ApiHeaders {
    pub fn new(guard: HeaderGuard) -> Self {
        Self {
            guard,
            entries: Vec::new(),
            wire_names: BTreeMap::new(),
        }
    }
    pub fn guard(&self) -> HeaderGuard {
        self.guard
    }
    pub fn raw_entries(&self) -> &[(String, String)] {
        &self.entries
    }
    pub(crate) fn wire_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.wire_names.get(name).map_or(name, String::as_str)
    }
    pub fn set_guard(&mut self, guard: HeaderGuard) {
        self.guard = guard;
    }
    pub fn from_pairs(
        guard: HeaderGuard,
        pairs: &[(Vec<u16>, Vec<u16>)],
    ) -> Result<Self, ApiError> {
        let mut result = Self::new(guard);
        for (name, value) in pairs {
            result.append(name, value)?;
        }
        Ok(result)
    }
    /// The bindings pass a JS sequence as rows so Fetch can reject rows whose
    /// size is not exactly two (Headers.fill, §5.1).
    pub fn from_sequence(guard: HeaderGuard, rows: &[Vec<Vec<u16>>]) -> Result<Self, ApiError> {
        let mut result = Self::new(guard);
        for row in rows {
            if row.len() != 2 {
                return Err(ApiError::InvalidHeaderTuple);
            }
            result.append(&row[0], &row[1])?;
        }
        Ok(result)
    }
    pub fn from_header_list(guard: HeaderGuard, list: &HeaderList) -> Self {
        let mut result = Self::new(guard);
        for ((name, value), (wire_name, _)) in list.iter().zip(list.wire_iter()) {
            result.entries.push((name.to_owned(), isomorphic_header_value(value)));
            result
                .wire_names
                .entry(name.to_owned())
                .or_insert_with(|| wire_name.to_owned());
        }
        result
    }
    pub fn to_header_list(&self) -> Result<HeaderList, ApiError> {
        let mut list = HeaderList::default();
        for (name, value) in &self.entries {
            list.append_wire(name, self.wire_name(name), value)
                .map_err(|_| ApiError::InvalidHeaderValue)?;
        }
        Ok(list)
    }
    pub fn append(&mut self, name: &[u16], value: &[u16]) -> Result<(), ApiError> {
        self.append_string(&byte_string(name)?, &byte_string(value)?)
    }
    pub fn append_string(&mut self, name: &str, value: &str) -> Result<(), ApiError> {
        self.append_existing(name, &normalize_value(value)?)
    }
    /// Copy a value that has already passed API normalization. XHR may have
    /// deliberately appended `", "` to an empty value; do not trim it again.
    /// A response header received from the transport (see
    /// [`isomorphic_header_value`]); the checks of `append_existing` still apply.
    pub(crate) fn append_network(&mut self, name: &str, value: &str) -> Result<(), ApiError> {
        self.append_existing(name, &isomorphic_header_value(value))
    }
    pub(crate) fn append_existing(&mut self, name: &str, value: &str) -> Result<(), ApiError> {
        let wire_name = name;
        let name = valid_name(name)?;
        if value
            .chars()
            .any(|c| matches!(c, '\0' | '\r' | '\n') || c as u32 > 255)
        {
            return Err(ApiError::InvalidHeaderValue);
        }
        if !self.allowed(&name, &value)? {
            return Ok(());
        }
        if self.guard == HeaderGuard::RequestNoCors {
            let combined = match self.get_string(&name)? {
                Some(old) => format!("{old}, {value}"),
                None => value.to_owned(),
            };
            if !safelisted_request_header(&name, &combined) {
                return Ok(());
            }
        }
        self.entries.push((name, value.to_owned()));
        let name = &self.entries.last().unwrap().0;
        self.wire_names
            .entry(name.clone())
            .or_insert_with(|| wire_name.to_owned());
        self.remove_privileged_no_cors();
        Ok(())
    }
    pub fn set(&mut self, name: &[u16], value: &[u16]) -> Result<(), ApiError> {
        self.set_string(&byte_string(name)?, &byte_string(value)?)
    }
    pub fn set_string(&mut self, name: &str, value: &str) -> Result<(), ApiError> {
        let wire_name = name;
        let name = valid_name(name)?;
        let value = normalize_value(value)?;
        if !self.allowed(&name, &value)? {
            return Ok(());
        }
        if self.guard == HeaderGuard::RequestNoCors && !safelisted_request_header(&name, &value) {
            return Ok(());
        }
        let first = self.entries.iter().position(|(n, _)| *n == name);
        self.entries.retain(|(n, _)| *n != name);
        let wire_key = name.clone();
        self.entries.insert(
            first.unwrap_or(self.entries.len()).min(self.entries.len()),
            (name, value),
        );
        self.wire_names
            .entry(wire_key)
            .or_insert_with(|| wire_name.to_owned());
        self.remove_privileged_no_cors();
        Ok(())
    }
    /// XHR combines an already normalized author value with an existing one.
    /// The separator's trailing SP is part of the combined value even when
    /// the newly appended value is empty.
    pub(crate) fn replace_combined_string(
        &mut self,
        name: &str,
        value: String,
    ) -> Result<(), ApiError> {
        let name = valid_name(name)?;
        if value.bytes().any(|b| matches!(b, 0 | b'\r' | b'\n')) {
            return Err(ApiError::InvalidHeaderValue);
        }
        let (_, old) = self
            .entries
            .iter_mut()
            .find(|(n, _)| *n == name)
            .ok_or(ApiError::InvalidHeaderName)?;
        *old = value;
        Ok(())
    }
    pub fn delete(&mut self, name: &[u16]) -> Result<(), ApiError> {
        self.delete_string(&byte_string(name)?)
    }
    pub fn delete_string(&mut self, name: &str) -> Result<(), ApiError> {
        let name = valid_name(name)?;
        if !self.allowed(&name, "")? {
            return Ok(());
        }
        if self.guard == HeaderGuard::RequestNoCors && !no_cors_name(&name) && name != "range" {
            return Ok(());
        }
        self.entries.retain(|(n, _)| *n != name);
        self.wire_names.remove(&name);
        self.remove_privileged_no_cors();
        Ok(())
    }
    pub fn get(&self, name: &[u16]) -> Result<Option<String>, ApiError> {
        self.get_string(&byte_string(name)?)
    }
    pub fn get_string(&self, name: &str) -> Result<Option<String>, ApiError> {
        let name = valid_name(name)?;
        let values: Vec<_> = self
            .entries
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .collect();
        Ok(if values.is_empty() {
            None
        } else {
            Some(values.join(", "))
        })
    }
    pub fn has(&self, name: &[u16]) -> Result<bool, ApiError> {
        self.has_string(&byte_string(name)?)
    }
    pub fn has_string(&self, name: &str) -> Result<bool, ApiError> {
        let name = valid_name(name)?;
        Ok(self.entries.iter().any(|(n, _)| *n == name))
    }
    pub fn get_set_cookie(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .map(|(_, v)| v.clone())
            .collect()
    }
    /// Fetch §2.2.2 sort-and-combine, retaining each Set-Cookie entry.
    pub fn sorted_entries(&self) -> Vec<(String, String)> {
        let mut names: Vec<_> = self.entries.iter().map(|(n, _)| n.clone()).collect();
        names.sort();
        names.dedup();
        let mut out = Vec::new();
        for name in names {
            if name == "set-cookie" {
                out.extend(self.entries.iter().filter(|(n, _)| *n == name).cloned());
            } else if let Ok(Some(value)) = self.get_string(&name) {
                out.push((name, value));
            }
        }
        out
    }
    fn allowed(&self, name: &str, value: &str) -> Result<bool, ApiError> {
        if self.guard == HeaderGuard::Immutable {
            return Err(ApiError::ImmutableHeaders);
        }
        Ok(match self.guard {
            HeaderGuard::Request => !forbidden_request(name, value),
            HeaderGuard::Response => !matches!(name, "set-cookie" | "set-cookie2"),
            _ => true,
        })
    }
    fn remove_privileged_no_cors(&mut self) {
        if self.guard == HeaderGuard::RequestNoCors {
            self.entries.retain(|(n, _)| n != "range");
            self.wire_names.remove("range");
        }
    }
}
fn valid_name(name: &str) -> Result<String, ApiError> {
    if !crate::headers::token(name) {
        return Err(ApiError::InvalidHeaderName);
    }
    Ok(name.to_ascii_lowercase())
}
fn normalize_value(value: &str) -> Result<String, ApiError> {
    // Fetch's normalize-a-header-value removes HTTP whitespace at the edges.
    // Other control bytes remain valid header values; CR/LF inside the value
    // must still be rejected before serialization to prevent line injection.
    let value = value.trim_matches([' ', '\t', '\r', '\n']);
    if value
        .chars()
        .any(|c| matches!(c, '\0' | '\r' | '\n') || c as u32 > 255)
    {
        return Err(ApiError::InvalidHeaderValue);
    }
    Ok(value.to_owned())
}
fn no_cors_name(name: &str) -> bool {
    matches!(
        name,
        "accept" | "accept-language" | "content-language" | "content-type" | "range"
    )
}
fn forbidden_request(name: &str, value: &str) -> bool {
    if name.starts_with("proxy-") || name.starts_with("sec-") {
        return true;
    }
    if matches!(
        name,
        "accept-charset"
            | "accept-encoding"
            | "access-control-request-headers"
            | "access-control-request-method"
            | "connection"
            | "content-length"
            | "cookie"
            | "cookie2"
            | "date"
            | "dnt"
            | "expect"
            | "host"
            | "keep-alive"
            | "origin"
            | "referer"
            | "set-cookie"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "via"
    ) {
        return true;
    }
    matches!(
        name,
        "x-http-method" | "x-http-method-override" | "x-method-override"
    ) && value.split(',').any(|v| {
        matches!(
            v.trim().to_ascii_uppercase().as_str(),
            "CONNECT" | "TRACE" | "TRACK"
        )
    })
}

/// The transport hands header values over as UTF-8 text decoded from the wire
/// bytes, and the request writer encodes ByteString code points the same way, so
/// values whose code points all fit a byte round-trip unchanged. A code point
/// above U+00FF can only come from a multi-byte sequence, which script must see
/// as its bytes ("isomorphic decode"); rejecting such a header fails the whole
/// response. TODO(net): make the transport byte-exact (latin-1) in both
/// directions, then expand every non-ASCII value.
pub(crate) fn isomorphic_header_value(value: &str) -> String {
    if value.chars().all(|c| (c as u32) <= 0xFF) {
        value.to_owned()
    } else {
        value.bytes().map(char::from).collect()
    }
}

#[cfg(test)]
mod isomorphic_value_tests {
    use super::*;

    #[test]
    fn only_values_that_cannot_be_byte_strings_are_expanded_to_bytes() {
        assert_eq!(isomorphic_header_value("plain"), "plain");
        assert_eq!(isomorphic_header_value("\u{ff}"), "\u{ff}");
        assert_eq!(isomorphic_header_value("\u{2026}"), "\u{e2}\u{80}\u{a6}");
        let mut headers = ApiHeaders::new(HeaderGuard::None);
        headers.append_network("X-Bytes", "\u{2026}").unwrap();
        assert_eq!(headers.get_string("x-bytes").unwrap().as_deref(), Some("\u{e2}\u{80}\u{a6}"));
    }
}
