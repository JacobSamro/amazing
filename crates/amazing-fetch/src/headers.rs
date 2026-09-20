use crate::FetchError;
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderList(Vec<(String, String)>);
pub(crate) fn token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
impl HeaderList {
    pub fn append(&mut self, name: &str, value: &str) -> Result<(), FetchError> {
        if !token(name) || value.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n') {
            return Err(FetchError::InvalidHeader);
        }
        self.0.push((
            name.to_ascii_lowercase(),
            value.trim_matches([' ', '\t']).to_owned(),
        ));
        Ok(())
    }
    pub fn get(&self, name: &str) -> Option<String> {
        let values: Vec<_> = self
            .0
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values.join(", "))
        }
    }
    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }
    pub(crate) fn retain(&mut self, mut f: impl FnMut(&str) -> bool) {
        self.0.retain(|(n, _)| f(n));
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
