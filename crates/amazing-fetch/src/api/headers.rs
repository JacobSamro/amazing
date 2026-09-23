use super::{byte_string, ApiError};
use crate::{safelisted_request_header, HeaderList};

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
        }
    }
    pub fn guard(&self) -> HeaderGuard {
        self.guard
    }
    pub fn raw_entries(&self) -> &[(String, String)] {
        &self.entries
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
        Self {
            guard,
            entries: list
                .iter()
                .map(|(n, v)| (n.to_owned(), v.to_owned()))
                .collect(),
        }
    }
    pub fn to_header_list(&self) -> Result<HeaderList, ApiError> {
        let mut list = HeaderList::default();
        for (name, value) in &self.entries {
            list.append(name, value)
                .map_err(|_| ApiError::InvalidHeaderValue)?;
        }
        Ok(list)
    }
    pub fn append(&mut self, name: &[u16], value: &[u16]) -> Result<(), ApiError> {
        self.append_string(&byte_string(name)?, &byte_string(value)?)
    }
    pub fn append_string(&mut self, name: &str, value: &str) -> Result<(), ApiError> {
        let name = valid_name(name)?;
        let value = normalize_value(value)?;
        if !self.allowed(&name, &value)? {
            return Ok(());
        }
        if self.guard == HeaderGuard::RequestNoCors {
            let combined = match self.get_string(&name)? {
                Some(old) => format!("{old}, {value}"),
                None => value.clone(),
            };
            if !safelisted_request_header(&name, &combined) {
                return Ok(());
            }
        }
        self.entries.push((name, value));
        self.remove_privileged_no_cors();
        Ok(())
    }
    pub fn set(&mut self, name: &[u16], value: &[u16]) -> Result<(), ApiError> {
        self.set_string(&byte_string(name)?, &byte_string(value)?)
    }
    pub fn set_string(&mut self, name: &str, value: &str) -> Result<(), ApiError> {
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
        self.entries.insert(
            first.unwrap_or(self.entries.len()).min(self.entries.len()),
            (name, value),
        );
        self.remove_privileged_no_cors();
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
    let value = value.trim_matches([' ', '\t']);
    if value.chars().any(|c| {
        c == '\0'
            || c == '\r'
            || c == '\n'
            || c == '\u{7f}'
            || (c < ' ' && c != '\t')
            || c as u32 > 255
    }) {
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
