//! Case-insensitive, multi-value HTTP header map.
//!
//! Field names are stored lowercased on insertion (HTTP field names are
//! case-insensitive per RFC 9110 §5.1) and looked up case-insensitively.
//! Repeated headers (e.g. several `Set-Cookie` lines) are preserved in
//! arrival order via [`Headers::get_all`].
//!
//! Deferred: comma-merging duplicates into one combined value and obsolete
//! line folding (obs-fold).

/// Why a public header append was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    InvalidName,
    InvalidValue,
}

/// An ordered multimap of HTTP header fields with case-insensitive names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    entries: Vec<(String, String)>,
}

impl Headers {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Appends a header; `name` is lowercased for storage. Values are kept
    /// verbatim (callers trim whitespace before inserting).
    pub(crate) fn insert(&mut self, name: &str, value: String) {
        self.entries.push((name.to_ascii_lowercase(), value));
    }

    /// Appends a field while preserving duplicate values in insertion order.
    /// The name must be an RFC 9110 token; CR, LF, and NUL are forbidden in
    /// the value. Existing internal `insert` callers keep their own policy.
    pub fn append(&mut self, name: &str, value: String) -> Result<(), HeaderError> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return Err(HeaderError::InvalidName);
        }
        if value.bytes().any(|b| matches!(b, b'\r' | b'\n' | b'\0')) {
            return Err(HeaderError::InvalidValue);
        }
        self.insert(name, value);
        Ok(())
    }

    /// First value for `name`, if any (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&str> {
        let lowered = name.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|(n, _)| *n == lowered)
            .map(|(_, v)| v.as_str())
    }

    /// All values for `name` in arrival order; empty when absent.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        let lowered = name.to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|(n, _)| *n == lowered)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.entries.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_case_insensitive_in_both_directions() {
        let mut h = Headers::new();
        h.insert("Content-Type", "text/html".into());
        assert_eq!(h.get("content-type"), Some("text/html"));
        assert_eq!(h.get("CONTENT-TYPE"), Some("text/html"));
        assert_eq!(h.get("Content-Type"), Some("text/html"));
        assert_eq!(h.get("content-typo"), None);
    }

    #[test]
    fn repeated_headers_preserve_order() {
        let mut h = Headers::new();
        h.insert("Set-Cookie", "a=1".into());
        h.insert("X-Other", "x".into());
        h.insert("set-cookie", "b=2".into());
        assert_eq!(h.get_all("Set-Cookie"), vec!["a=1", "b=2"]);
        assert_eq!(h.get("Set-Cookie"), Some("a=1"));
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn contains_and_iteration() {
        let mut h = Headers::new();
        h.insert("Accept", "*/*".into());
        h.insert("Connection", "close".into());
        assert!(h.contains("accept"));
        assert!(!h.contains("cookie"));
        let pairs: Vec<(&str, &str)> = h.iter().collect();
        assert_eq!(pairs, vec![("accept", "*/*"), ("connection", "close")]);
    }

    #[test]
    fn empty_map_defaults_to_no_entries() {
        let h = Headers::new();
        assert!(h.is_empty());
        assert_eq!(h.get("anything"), None);
        assert!(h.get_all("anything").is_empty());
    }

    #[test]
    fn public_append_preserves_duplicates_in_order() {
        let mut h = Headers::new();
        h.append("Age", "1".into()).unwrap();
        h.append("X-Other", "x".into()).unwrap();
        h.append("age", "2".into()).unwrap();
        assert_eq!(h.get_all("AGE"), vec!["1", "2"]);
        assert_eq!(
            h.iter().collect::<Vec<_>>(),
            vec![("age", "1"), ("x-other", "x"), ("age", "2")]
        );
    }

    #[test]
    fn public_append_rejects_invalid_names_and_line_breaks() {
        let mut h = Headers::new();
        assert_eq!(h.append("", "x".into()), Err(HeaderError::InvalidName));
        assert_eq!(
            h.append("bad name", "x".into()),
            Err(HeaderError::InvalidName)
        );
        assert_eq!(h.append("é", "x".into()), Err(HeaderError::InvalidName));
        for value in ["a\rb", "a\nb", "a\0b"] {
            assert_eq!(
                h.append("Age", value.into()),
                Err(HeaderError::InvalidValue)
            );
        }
        assert!(h.is_empty());
    }
}
