//! A small, dependency-free JSON value, parser and serializer.
//!
//! The DevTools protocol is JSON in both directions, so the crate carries its
//! own model rather than pull a dependency into an engine-adjacent crate. It
//! follows RFC 8259: UTF-8 text, one top-level value, no comments, no trailing
//! commas. Object member order is preserved so a response round-trips the way
//! the sender wrote it.
//!
//! Parsing is bounded by an input byte limit and a nesting limit; both exist
//! because the input arrives from a socket.

use std::fmt;

/// Maximum nesting depth accepted by [`Json::parse`] and friends.
pub const MAX_DEPTH: usize = 128;
/// Default input limit for [`Json::parse_bounded`] callers that do not pass one.
pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// A parsed JSON value.
///
/// Numbers are stored as `f64`. Protocol message ids are small integers and
/// serialize back without a fractional part, so clients see `1`, not `1.0`.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// Exact UTF-16 string value, including unpaired surrogate units.
    /// Construct through string_utf16 to retain the legacy scalar variant when possible.
    StringUtf16(Vec<u16>),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Why a JSON document was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonError {
    /// The document was empty or malformed. `offset` is a byte position.
    Syntax { offset: usize, message: &'static str },
    /// Nesting exceeded [`MAX_DEPTH`].
    Depth,
    /// The document exceeded the caller's byte limit.
    Limit,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { offset, message } => write!(f, "json syntax error at {offset}: {message}"),
            Self::Depth => write!(f, "json nesting depth exceeded"),
            Self::Limit => write!(f, "json input limit exceeded"),
        }
    }
}

impl std::error::Error for JsonError {}

impl Json {
    /// Parses `text`, bounded by [`DEFAULT_MAX_BYTES`] and [`MAX_DEPTH`].
    pub fn parse(text: &str) -> Result<Json, JsonError> {
        Self::parse_bounded(text, DEFAULT_MAX_BYTES)
    }

    /// Parses `text`, rejecting inputs larger than `max_bytes`.
    pub fn parse_bounded(text: &str, max_bytes: usize) -> Result<Json, JsonError> {
        if text.len() > max_bytes {
            return Err(JsonError::Limit);
        }
        Parser {
            bytes: text.as_bytes(),
            pos: 0,
        }
        .parse_document()
    }

    /// Convenience constructor for a JSON string.
    pub fn string(value: impl Into<String>) -> Json {
        Json::String(value.into())
    }

    /// Construct a protocol string without losing native UTF-16 source units.
    pub fn string_utf16(value: impl Into<Vec<u16>>) -> Json {
        let units = value.into();
        match String::from_utf16(&units) {
            Ok(value) => Json::String(value),
            Err(_) => Json::StringUtf16(units),
        }
    }

    /// Exact code units of a string value. Scalar strings encode without loss.
    pub fn as_utf16(&self) -> Option<std::borrow::Cow<'_, [u16]>> {
        match self {
            Json::String(value) => Some(std::borrow::Cow::Owned(value.encode_utf16().collect())),
            Json::StringUtf16(value) => Some(std::borrow::Cow::Borrowed(value)),
            _ => None,
        }
    }

    /// Convenience constructor for a JSON number.
    pub fn number(value: impl Into<f64>) -> Json {
        Json::Number(value.into())
    }

    /// Convenience constructor for a JSON boolean.
    pub fn bool(value: bool) -> Json {
        Json::Bool(value)
    }

    /// Returns the string contents, or `None` for any other variant.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the number as `f64`, or `None` for any other variant.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the number as `u64` when it is a non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= u64::MAX as f64 => {
                Some(*n as u64)
            }
            _ => None,
        }
    }

    /// Returns the number as `i64` when it is an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(*n as i64),
            _ => None,
        }
    }

    /// Returns the boolean, or `None` for any other variant.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the array elements, or `None` for any other variant.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Returns the object members, or `None` for any other variant.
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(members) => Some(members),
            _ => None,
        }
    }

    /// Looks up a member of an object (first match wins).
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Mutable lookup for object members.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Object(members) => members
                .iter_mut()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Inserts or replaces an object member, converting `Null` to an object.
    pub fn insert(&mut self, key: impl Into<String>, value: Json) {
        let key = key.into();
        match self {
            Json::Object(members) => {
                if let Some(slot) = members.iter_mut().find(|(name, _)| *name == key) {
                    slot.1 = value;
                } else {
                    members.push((key, value));
                }
            }
            other => {
                *other = Json::Object(vec![(key, value)]);
            }
        }
    }

    /// Serializes to compact JSON text.
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(n) => write_number(*n, out),
            Json::String(s) => write_string(s, out),
            Json::StringUtf16(s) => write_string_utf16(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string())
    }
}

/// Serializes a finite number as an integer when it has no fractional part,
/// otherwise in Rust's shortest round-trip form. Non-finite values become
/// `null`, which is the only JSON-representable fallback.
fn write_number(n: f64, out: &mut String) {
    use std::fmt::Write;
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n}");
    }
}

/// Writes a JSON string with the mandatory escapes. Bytes at or above U+0080
/// are emitted as UTF-8, which RFC 8259 allows.
fn write_string(s: &str, out: &mut String) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Escape code units directly: JSON's \u grammar admits lone surrogates
/// (RFC 8259 §8.2). No Rust scalar-string conversion is involved.
fn write_string_utf16(units: &[u16], out: &mut String) {
    use std::fmt::Write;
    out.push('"');
    for unit in units { let _ = write!(out, "\\u{unit:04x}"); }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn parse_document(mut self) -> Result<Json, JsonError> {
        self.skip_ws();
        if self.pos == self.bytes.len() {
            return Err(self.syntax("empty document"));
        }
        let value = self.parse_value(0)?;
        self.skip_ws();
        if self.pos != self.bytes.len() {
            return Err(self.syntax("trailing characters"));
        }
        Ok(value)
    }

    fn syntax(&self, message: &'static str) -> JsonError {
        JsonError::Syntax {
            offset: self.pos,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::Depth);
        }
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => self.parse_string_units().map(Json::string_utf16),
            Some(b't') => self.parse_literal(b"true", Json::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Json::Bool(false)),
            Some(b'n') => self.parse_literal(b"null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(self.syntax("unexpected byte")),
            None => Err(self.syntax("unexpected end of input")),
        }
    }

    fn parse_literal(&mut self, word: &[u8], value: Json) -> Result<Json, JsonError> {
        if self.bytes[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.syntax("invalid literal"))
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.pos += 1; // '{'
        let mut members = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.syntax("object key must be a string"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.syntax("missing ':'"));
            }
            self.pos += 1;
            self.skip_ws();
            let value = self.parse_value(depth + 1)?;
            members.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.syntax("expected ',' or '}'")),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            let value = self.parse_value(depth + 1)?;
            items.push(value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.syntax("expected ',' or ']'")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        let units = self.parse_string_units()?;
        // Protocol member names remain the existing scalar identifier interface.
        String::from_utf16(&units).map_err(|_| self.syntax("unpaired UTF-16 surrogate in object key"))
    }

    fn parse_string_units(&mut self) -> Result<Vec<u16>, JsonError> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let b = self.peek().ok_or_else(|| self.syntax("unterminated string"))?;
            match b {
                b'"' => { self.pos += 1; return Ok(out); }
                b'\\' => { self.pos += 1; self.parse_escape(&mut out)?; }
                0x00..=0x1f => return Err(self.syntax("raw control character in string")),
                0x20..=0x7f => { out.push(u16::from(b)); self.pos += 1; }
                _ => {
                    let (c, len) = self.next_char()?;
                    let mut units = [0; 2];
                    out.extend_from_slice(c.encode_utf16(&mut units));
                    self.pos += len;
                }
            }
        }
    }

    fn next_char(&self) -> Result<(char, usize), JsonError> {
        let rest = std::str::from_utf8(&self.bytes[self.pos..])
            .map_err(|_| self.syntax("invalid UTF-8"))?;
        let c = rest.chars().next().ok_or_else(|| self.syntax("unexpected end"))?;
        Ok((c, c.len_utf8()))
    }

    fn parse_escape(&mut self, out: &mut Vec<u16>) -> Result<(), JsonError> {
        let b = self.peek().ok_or_else(|| self.syntax("unterminated escape"))?;
        self.pos += 1;
        out.push(match b {
            b'"' => 0x22, b'\\' => 0x5c, b'/' => 0x2f,
            b'b' => 8, b'f' => 12, b'n' => 10, b'r' => 13, b't' => 9,
            b'u' => self.parse_hex4()? as u16,
            _ => return Err(self.syntax("invalid escape")),
        });
        Ok(())
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let b = self.peek().ok_or_else(|| self.syntax("truncated \\u escape"))?;
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return Err(self.syntax("invalid hex digit")),
            };
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let int_start = self.pos;
        if self.peek() == Some(b'0') {
            self.pos += 1;
        } else {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if self.pos == int_start {
            return Err(self.syntax("number needs an integer part"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let frac_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(self.syntax("number needs fraction digits"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(self.syntax("number needs exponent digits"));
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.syntax("invalid number"))?;
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|_| self.syntax("invalid number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars() {
        assert_eq!(Json::parse("null").unwrap(), Json::Null);
        assert_eq!(Json::parse("true").unwrap(), Json::Bool(true));
        assert_eq!(Json::parse("false").unwrap(), Json::Bool(false));
        assert_eq!(Json::parse("12").unwrap(), Json::Number(12.0));
        assert_eq!(Json::parse("-1.5e2").unwrap(), Json::Number(-150.0));
        assert_eq!(Json::parse("\"hi\"").unwrap(), Json::String("hi".into()));
    }

    #[test]
    fn round_trips_integers_without_fraction() {
        let value = Json::parse(r#"{"id":1,"n":2.5}"#).unwrap();
        assert_eq!(value.to_string(), r#"{"id":1,"n":2.5}"#);
        assert_eq!(value.get("id").unwrap().as_u64(), Some(1));
    }

    #[test]
    fn parses_escapes_and_surrogates() {
        let value = Json::parse(r#""a\n\"b\u00e9\u0041\ud83d\ude00""#).unwrap();
        assert_eq!(value.as_str().unwrap(), "a\n\"béA😀");
    }

    #[test]
    fn preserves_object_order_and_serializes_stably() {
        let value = Json::parse(r#"{"b":1,"a":[true,null,"x"]}"#).unwrap();
        assert_eq!(value.to_string(), r#"{"b":1,"a":[true,null,"x"]}"#);
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "",
            "   ",
            "{",
            "{\"a\":}",
            "[1,]",
            "{\"a\":1,}",
            "trailing",
            "\"unterminated",
            "01",
            "1.",
            "1e",
            "\"\\x\"",
        ] {
            assert!(Json::parse(bad).is_err(), "expected rejection: {bad:?}");
        }
    }

    #[test]
    fn rejects_excessive_depth_and_size() {
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert_eq!(Json::parse(&deep).unwrap_err(), JsonError::Depth);
        assert_eq!(Json::parse_bounded("[1,2,3]", 3).unwrap_err(), JsonError::Limit);
    }

    #[test]
    fn inserts_and_replaces_members() {
        let mut value = Json::Object(vec![("a".into(), Json::Number(1.0))]);
        value.insert("a", Json::Number(2.0));
        value.insert("b", Json::Bool(true));
        assert_eq!(value.to_string(), r#"{"a":2,"b":true}"#);
    }
    #[test]
    fn exact_utf16_string_values_round_trip_json_escapes() {
        for units in [vec![0xd800], vec![0xdc00], vec![0xd800, 0xdc00],
            vec![0xd800, 65], vec![0xdc00, 0xd800], vec![0xfffd],
            vec![0, 8, 9, 10, 13, 34, 92, 0xd800, 0xd83d, 0xde00]] {
            let value = Json::string_utf16(units.clone());
            let wire = value.to_string();
            let parsed = Json::parse(&wire).unwrap();
            assert_eq!(parsed.as_utf16().unwrap().as_ref(), units);
            assert_eq!(parsed, value);
            assert!(!wire.contains('\u{fffd}') || units.contains(&0xfffd));
        }
        // These original rejection fixtures are legal JSON string values.
        assert_eq!(Json::parse(r#""\ud800""#).unwrap().as_utf16().unwrap().as_ref(), &[0xd800]);
        assert_eq!(Json::parse(r#""\ud800\u0041""#).unwrap().as_utf16().unwrap().as_ref(), &[0xd800, 65]);
        for text in ["plain", "é", "😀", "\n\t\"\\"] {
            let value = Json::string_utf16(text.encode_utf16().collect::<Vec<_>>());
            assert_eq!(value, Json::String(text.into()));
            assert_eq!(value.as_str(), Some(text));
        }
    }

}
