// Portions adapted from Amazing's Boa port (boa-dev/boa,
// core/engine/src/builtins/json/mod.rs) @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Copyright (c) 2019 Jason Williams. Grammar records are data-only; no realm allocation.

use std::{fmt, ops::Range};

/// A validated JSON source and its lossless data-only parse record.
/// No object/function/environment handle is allocated during parsing.
#[derive(Debug, Clone)]
pub struct JsonDocument {
    pub(crate) source: Vec<u16>,
    pub(crate) root: JsonNode,
}
#[derive(Debug, Clone)]
pub(crate) struct JsonNode {
    pub(crate) span: Range<usize>,
    pub(crate) value: JsonValue,
}
#[derive(Debug, Clone)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(Vec<u16>),
    Array(Vec<JsonNode>),
    Object(Vec<(Vec<u16>, JsonNode)>),
}
/// Invalid JSON grammar, without an exception allocated in any private realm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError(String);
impl JsonError {
    fn new(message: impl Into<String>) -> Self { Self(message.into()) }
}
impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}
impl std::error::Error for JsonError {}
impl JsonDocument {
    pub fn parse_utf16(source: &[u16]) -> Result<Self, JsonError> {
        let mut parser = JsonParser { units: source, position: 0, depth: 0 };
        let root = parser.value()?;
        parser.whitespace();
        if parser.position != source.len() { return Err(JsonError::new("unexpected text after JSON value")); }
        Ok(Self { source: source.to_vec(), root })
    }
    pub fn is_primitive(&self) -> bool {
        !matches!(self.root.value, JsonValue::Object(_) | JsonValue::Array(_))
    }
    pub fn source_units(&self) -> &[u16] { &self.source }
}
struct JsonParser<'a> { units: &'a [u16], position: usize, depth: usize }
impl JsonParser<'_> {
    fn whitespace(&mut self) {
        while self.units.get(self.position).is_some_and(|unit| matches!(unit, 0x20 | 0x09 | 0x0a | 0x0d)) {
            self.position += 1;
        }
    }
    fn take(&mut self, unit: u16) -> bool {
        if self.units.get(self.position) == Some(&unit) { self.position += 1; true } else { false }
    }
    fn string(&mut self) -> Result<Vec<u16>, JsonError> {
        if !self.take(b'"' as u16) { return Err(JsonError::new("expected JSON string")); }
        let mut out = Vec::new();
        while let Some(&unit) = self.units.get(self.position) {
            self.position += 1;
            if unit == b'"' as u16 { return Ok(out); }
            if unit < 0x20 { return Err(JsonError::new("control character in JSON string")); }
            if unit != b'\\' as u16 { out.push(unit); continue; }
            let escaped = *self.units.get(self.position).ok_or_else(|| JsonError::new("unfinished JSON escape"))?;
            self.position += 1;
            match escaped {
                0x22 | 0x5c | 0x2f => out.push(escaped),
                0x62 => out.push(0x08), 0x66 => out.push(0x0c), 0x6e => out.push(0x0a),
                0x72 => out.push(0x0d), 0x74 => out.push(0x09),
                0x75 => {
                    let mut point = 0u16;
                    for _ in 0..4 {
                        let digit = *self.units.get(self.position).ok_or_else(|| JsonError::new("short JSON Unicode escape"))?;
                        self.position += 1;
                        point = (point << 4) | (char::from_u32(digit as u32).and_then(|ch| ch.to_digit(16))
                            .ok_or_else(|| JsonError::new("invalid JSON Unicode escape"))? as u16);
                    }
                    out.push(point);
                }
                _ => return Err(JsonError::new("invalid JSON escape")),
            }
        }
        Err(JsonError::new("unterminated JSON string"))
    }
    fn literal(&mut self, literal: &str, value: JsonValue) -> Result<JsonValue, JsonError> {
        let units = literal.encode_utf16().collect::<Vec<_>>();
        if self.units[self.position..].starts_with(&units) { self.position += units.len(); Ok(value) }
        else { Err(JsonError::new("invalid JSON literal")) }
    }
    fn number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.position;
        self.take(b'-' as u16);
        if self.take(b'0' as u16) {
            if self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) {
                return Err(JsonError::new("leading zero in JSON number"));
            }
        } else {
            let mut digits = 0;
            while self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) {
                self.position += 1; digits += 1;
            }
            if digits == 0 { return Err(JsonError::new("invalid JSON number")); }
        }
        if self.take(b'.' as u16) {
            let begin = self.position;
            while self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) { self.position += 1; }
            if begin == self.position { return Err(JsonError::new("missing JSON fraction digits")); }
        }
        if self.take(b'e' as u16) || self.take(b'E' as u16) {
            if !self.take(b'+' as u16) { self.take(b'-' as u16); }
            let begin = self.position;
            while self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) { self.position += 1; }
            if begin == self.position { return Err(JsonError::new("missing JSON exponent digits")); }
        }
        let text = String::from_utf16(&self.units[start..self.position]).unwrap_or_default();
        Ok(JsonValue::Number(text.parse::<f64>().map_err(|_| JsonError::new("invalid JSON number"))?))
    }
    fn value(&mut self) -> Result<JsonNode, JsonError> {
        self.whitespace();
        let start = self.position;
        self.depth += 1;
        if self.depth > 512 { return Err(JsonError::new("JSON nesting limit")); }
        let value = match self.units.get(self.position).copied() {
            Some(0x22) => JsonValue::String(self.string()?),
            Some(0x74) => self.literal("true", JsonValue::Bool(true))?,
            Some(0x66) => self.literal("false", JsonValue::Bool(false))?,
            Some(0x6e) => self.literal("null", JsonValue::Null)?,
            Some(0x5b) => {
                self.position += 1;
                self.whitespace();
                let mut values = Vec::new();
                if !self.take(b']' as u16) {
                    loop {
                        values.push(self.value()?);
                        self.whitespace();
                        if self.take(b']' as u16) { break; }
                        if !self.take(b',' as u16) { return Err(JsonError::new("expected JSON array comma")); }
                    }
                }
                JsonValue::Array(values)
            }
            Some(0x7b) => {
                self.position += 1;
                self.whitespace();
                let mut members = Vec::new();
                if !self.take(b'}' as u16) {
                    loop {
                        self.whitespace();
                        let key = self.string()?;
                        self.whitespace();
                        if !self.take(b':' as u16) { return Err(JsonError::new("expected JSON colon")); }
                        members.push((key, self.value()?));
                        self.whitespace();
                        if self.take(b'}' as u16) { break; }
                        if !self.take(b',' as u16) { return Err(JsonError::new("expected JSON object comma")); }
                    }
                }
                JsonValue::Object(members)
            }
            Some(0x2d | 0x30..=0x39) => self.number()?,
            _ => return Err(JsonError::new("invalid JSON token")),
        };
        self.depth -= 1;
        Ok(JsonNode { span: start..self.position, value })
    }
}
