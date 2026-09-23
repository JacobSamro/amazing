use super::ApiError;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}
pub fn parse_json(bytes: &[u8]) -> Result<JsonValue, ApiError> {
    if bytes.len() > super::MAX_API_BODY_BYTES {
        return Err(ApiError::BodyTooLarge);
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let decoded = String::from_utf8_lossy(bytes);
    let s = decoded.as_ref();
    let mut parser = JsonParser { s, at: 0 };
    let value = parser.value(0)?;
    parser.ws();
    if parser.at != s.len() {
        return Err(ApiError::InvalidJson);
    }
    Ok(value)
}
struct JsonParser<'a> {
    s: &'a str,
    at: usize,
}
impl JsonParser<'_> {
    fn ws(&mut self) {
        while self
            .s
            .as_bytes()
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.at += 1;
        }
    }
    fn byte(&mut self, b: u8) -> bool {
        if self.s.as_bytes().get(self.at) == Some(&b) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<JsonValue, ApiError> {
        if depth > 128 {
            return Err(ApiError::InvalidJson);
        }
        self.ws();
        match self.s.as_bytes().get(self.at).copied() {
            Some(b'n') if self.literal("null") => Ok(JsonValue::Null),
            Some(b't') if self.literal("true") => Ok(JsonValue::Boolean(true)),
            Some(b'f') if self.literal("false") => Ok(JsonValue::Boolean(false)),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b'[') => self.array(depth + 1),
            Some(b'{') => self.object(depth + 1),
            Some(b'-' | b'0'..=b'9') => self.number().map(JsonValue::Number),
            _ => Err(ApiError::InvalidJson),
        }
    }
    fn literal(&mut self, word: &str) -> bool {
        if self.s[self.at..].starts_with(word) {
            self.at += word.len();
            true
        } else {
            false
        }
    }
    fn number(&mut self) -> Result<f64, ApiError> {
        let start = self.at;
        self.byte(b'-');
        if !self.byte(b'0') {
            if !self
                .s
                .as_bytes()
                .get(self.at)
                .is_some_and(|b| (b'1'..=b'9').contains(b))
            {
                return Err(ApiError::InvalidJson);
            }
            while self
                .s
                .as_bytes()
                .get(self.at)
                .is_some_and(u8::is_ascii_digit)
            {
                self.at += 1;
            }
        }
        if self.byte(b'.') {
            let digit = self.at;
            while self
                .s
                .as_bytes()
                .get(self.at)
                .is_some_and(u8::is_ascii_digit)
            {
                self.at += 1;
            }
            if self.at == digit {
                return Err(ApiError::InvalidJson);
            }
        }
        if self.byte(b'e') || self.byte(b'E') {
            let _ = self.byte(b'+') || self.byte(b'-');
            let digit = self.at;
            while self
                .s
                .as_bytes()
                .get(self.at)
                .is_some_and(u8::is_ascii_digit)
            {
                self.at += 1;
            }
            if self.at == digit {
                return Err(ApiError::InvalidJson);
            }
        }
        self.s[start..self.at]
            .parse::<f64>()
            .map_err(|_| ApiError::InvalidJson)
    }
    fn string(&mut self) -> Result<String, ApiError> {
        if !self.byte(b'"') {
            return Err(ApiError::InvalidJson);
        }
        let mut out = String::new();
        loop {
            let Some(ch) = self.s[self.at..].chars().next() else {
                return Err(ApiError::InvalidJson);
            };
            self.at += ch.len_utf8();
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let Some(escaped) = self.s[self.at..].chars().next() else {
                        return Err(ApiError::InvalidJson);
                    };
                    self.at += 1;
                    match escaped {
                        '"' | '\\' | '/' => out.push(escaped),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let first = self.hex4()?;
                            let scalar = if (0xd800..=0xdbff).contains(&first) {
                                if !self.s[self.at..].starts_with("\\u") {
                                    return Err(ApiError::InvalidJson);
                                }
                                self.at += 2;
                                let second = self.hex4()?;
                                if !(0xdc00..=0xdfff).contains(&second) {
                                    return Err(ApiError::InvalidJson);
                                }
                                0x10000 + ((first as u32 - 0xd800) << 10) + second as u32 - 0xdc00
                            } else {
                                if (0xdc00..=0xdfff).contains(&first) {
                                    return Err(ApiError::InvalidJson);
                                }
                                first as u32
                            };
                            out.push(char::from_u32(scalar).ok_or(ApiError::InvalidJson)?);
                        }
                        _ => return Err(ApiError::InvalidJson),
                    }
                }
                c if c < ' ' => return Err(ApiError::InvalidJson),
                _ => out.push(ch),
            }
        }
    }
    fn hex4(&mut self) -> Result<u16, ApiError> {
        let bytes = self
            .s
            .as_bytes()
            .get(self.at..self.at + 4)
            .ok_or(ApiError::InvalidJson)?;
        let mut result = 0u16;
        for b in bytes {
            result = (result << 4) | (*b as char).to_digit(16).ok_or(ApiError::InvalidJson)? as u16;
        }
        self.at += 4;
        Ok(result)
    }
    fn array(&mut self, depth: usize) -> Result<JsonValue, ApiError> {
        self.at += 1;
        self.ws();
        let mut items = Vec::new();
        if self.byte(b']') {
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.ws();
            if self.byte(b']') {
                return Ok(JsonValue::Array(items));
            }
            if !self.byte(b',') {
                return Err(ApiError::InvalidJson);
            }
        }
    }
    fn object(&mut self, depth: usize) -> Result<JsonValue, ApiError> {
        self.at += 1;
        self.ws();
        let mut items = Vec::new();
        if self.byte(b'}') {
            return Ok(JsonValue::Object(items));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            if !self.byte(b':') {
                return Err(ApiError::InvalidJson);
            }
            let value = self.value(depth)?;
            if let Some((_, previous)) = items.iter_mut().find(|(name, _)| *name == key) {
                *previous = value;
            } else {
                items.push((key, value));
            }
            self.ws();
            if self.byte(b'}') {
                return Ok(JsonValue::Object(items));
            }
            if !self.byte(b',') {
                return Err(ApiError::InvalidJson);
            }
        }
    }
}
