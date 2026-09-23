//! URL Standard application/x-www-form-urlencoded and URLSearchParams algorithms.
use super::{encoding::percent_decode, ParseError, ParsedUrl};
use std::{cell::RefCell, fmt, rc::Rc};

pub fn parse_form_urlencoded(input: &str) -> Vec<(String, String)> {
    input
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            (decode(name), decode(value))
        })
        .collect()
}
fn decode(input: &str) -> String {
    String::from_utf8_lossy(&percent_decode(&input.replace('+', " "))).into_owned()
}
pub fn serialize_form_urlencoded(items: &[(String, String)]) -> String {
    items
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name, false), encode(value, false)))
        .collect::<Vec<_>>()
        .join("&")
}
/// `is_name` applies the HTML form-urlencoded line-ending normalization to names.
pub fn encode(input: &str, is_name: bool) -> String {
    let normalized = if is_name {
        input
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n")
    } else {
        input.to_owned()
    };
    let mut out = String::new();
    for b in normalized.bytes() {
        if b.is_ascii_alphanumeric() || b"*-._".contains(&b) {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            use fmt::Write;
            write!(out, "%{b:02X}").unwrap();
        }
    }
    out
}

/// A URL wrapper whose search parameters retain and update the same record.
#[derive(Clone, Debug)]
pub struct UrlWithSearchParams(Rc<RefCell<ParsedUrl>>);
impl UrlWithSearchParams {
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        Ok(Self(Rc::new(RefCell::new(ParsedUrl::parse(input)?))))
    }
    pub fn serialize(&self) -> String {
        self.0.borrow().serialize()
    }
    pub fn url(&self) -> ParsedUrl {
        self.0.borrow().clone()
    }
    pub fn set_search(&self, input: &str) {
        self.0.borrow_mut().set_search(input);
    }
    pub fn set_href(&self, input: &str) -> Result<(), ParseError> {
        self.0.borrow_mut().set_href(input)
    }
    pub fn search_params(&self) -> UrlSearchParams {
        UrlSearchParams {
            items: Vec::new(),
            url: Some(self.0.clone()),
        }
    }
}
#[derive(Clone, Debug)]
pub struct UrlSearchParams {
    items: Vec<(String, String)>,
    url: Option<Rc<RefCell<ParsedUrl>>>,
}
impl UrlSearchParams {
    pub fn new(input: &str) -> Self {
        Self {
            items: parse_form_urlencoded(input.strip_prefix('?').unwrap_or(input)),
            url: None,
        }
    }
    pub fn from_pairs(items: Vec<(String, String)>) -> Self {
        Self { items, url: None }
    }
    fn items(&self) -> Vec<(String, String)> {
        if let Some(url) = &self.url {
            parse_form_urlencoded(url.borrow().query().unwrap_or(""))
        } else {
            self.items.clone()
        }
    }
    fn mutate(&mut self, action: impl FnOnce(&mut Vec<(String, String)>)) {
        let mut items = self.items();
        action(&mut items);
        if let Some(url) = &self.url {
            url.borrow_mut()
                .set_search(&serialize_form_urlencoded(&items));
        } else {
            self.items = items;
        }
    }
    pub fn append(&mut self, name: &str, value: &str) {
        self.mutate(|items| items.push((name.into(), value.into())));
    }
    pub fn delete(&mut self, name: &str, value: Option<&str>) {
        self.mutate(|items| items.retain(|(n, v)| n != name || value.is_some_and(|x| x != v)));
    }
    pub fn get(&self, name: &str) -> Option<String> {
        self.items()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
    pub fn get_all(&self, name: &str) -> Vec<String> {
        self.items()
            .into_iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }
    pub fn has(&self, name: &str, value: Option<&str>) -> bool {
        self.items()
            .iter()
            .any(|(n, v)| n == name && value.is_none_or(|x| x == v))
    }
    pub fn set(&mut self, name: &str, value: &str) {
        self.mutate(|items| {
            let mut seen = false;
            items.retain_mut(|(n, v)| {
                if n != name {
                    true
                } else if !seen {
                    *v = value.into();
                    seen = true;
                    true
                } else {
                    false
                }
            });
            if !seen {
                items.push((name.into(), value.into()));
            }
        });
    }
    pub fn sort(&mut self) {
        self.mutate(|items| items.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16())));
    }
    pub fn size(&self) -> usize {
        self.items().len()
    }
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.items()
    }
}
impl fmt::Display for UrlSearchParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&serialize_form_urlencoded(&self.items()))
    }
}
