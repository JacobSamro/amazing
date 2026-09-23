use crate::FetchError;
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeaderField {
    name: String,
    wire_name: String,
    value: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderList(Vec<HeaderField>);
pub(crate) fn token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
impl HeaderList {
    pub fn append(&mut self, name: &str, value: &str) -> Result<(), FetchError> {
        self.append_wire(name, name, value.trim_matches([' ', '\t']))
    }
    /// `ApiHeaders` has already normalized this value. Keep its trailing SP
    /// when XHR combines an empty final field, and retain the first spelling
    /// of the name for HTTP/1.1 serialization.
    pub(crate) fn append_wire(
        &mut self,
        name: &str,
        wire_name: &str,
        value: &str,
    ) -> Result<(), FetchError> {
        if !token(name)
            || !token(wire_name)
            || value.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n')
        {
            return Err(FetchError::InvalidHeader);
        }
        self.0.push(HeaderField {
            name: name.to_ascii_lowercase(),
            wire_name: wire_name.to_owned(),
            value: value.to_owned(),
        });
        Ok(())
    }
    pub fn get(&self, name: &str) -> Option<String> {
        let values: Vec<_> = self
            .0
            .iter()
            .filter(|field| field.name.eq_ignore_ascii_case(name))
            .map(|field| field.value.as_str())
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values.join(", "))
        }
    }
    pub fn remove(&mut self, name: &str) {
        self.0
            .retain(|field| !field.name.eq_ignore_ascii_case(name));
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|field| (field.name.as_str(), field.value.as_str()))
    }
    pub(crate) fn wire_iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|field| (field.wire_name.as_str(), field.value.as_str()))
    }
    pub(crate) fn retain(&mut self, mut f: impl FnMut(&str) -> bool) {
        self.0.retain(|field| f(&field.name));
    }
}
pub(crate) fn forbidden_response(name: &str) -> bool {
    matches!(name, "set-cookie" | "set-cookie2")
}
pub(crate) fn body_header(name: &str) -> bool {
    matches!(
        name,
        "content-encoding" | "content-language" | "content-location" | "content-type"
    )
}
