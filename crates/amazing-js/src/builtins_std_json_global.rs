// JSON and global functions with observable coercion and UTF-16 strings.

impl Interpreter {
    fn install_std_json_global(&mut self) {
        if let Some(JsValue::Object(json)) = self.global_env.resolve("JSON") {
            std_core_define(self, &json, "parse", 2, std_json_parse);
            std_core_define(self, &json, "stringify", 3, std_json_stringify);
            json.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                symbol_data(JsValue::str("JSON"), false, true)).unwrap();
        }
        for (name, length, native) in [
            ("parseInt", 2, std_global_parse_int as crate::value::NativeFn),
            ("parseFloat", 1, std_global_parse_float as crate::value::NativeFn),
            ("isFinite", 1, std_global_is_finite as crate::value::NativeFn),
            ("isNaN", 1, std_global_is_nan as crate::value::NativeFn),
            ("encodeURI", 1, std_global_encode_uri as crate::value::NativeFn),
            ("encodeURIComponent", 1, std_global_encode_component as crate::value::NativeFn),
            ("decodeURI", 1, std_global_decode_uri as crate::value::NativeFn),
            ("decodeURIComponent", 1, std_global_decode_component as crate::value::NativeFn),
            ("escape", 1, std_global_escape as crate::value::NativeFn),
            ("unescape", 1, std_global_unescape as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.define_global(name, method);
        }
        let global = JsValue::Object(self.global_obj.clone());
        self.global_obj.borrow_mut().define("globalThis", symbol_data(global.clone(), true, true)).unwrap();
        let _ = self.global_env.declare_var("globalThis", global);
        for (name, value) in [("NaN", JsValue::num(f64::NAN)),
            ("Infinity", JsValue::num(f64::INFINITY)), ("undefined", JsValue::Undefined)] {
            self.global_obj.borrow_mut().define(name, symbol_data(value, false, false)).unwrap();
        }
        if let (Some(JsValue::Object(number)), Some(parse_int), Some(parse_float)) =
            (self.global_env.resolve("Number"), self.global_env.resolve("parseInt"), self.global_env.resolve("parseFloat")) {
            number.borrow_mut().define("parseInt", symbol_data(parse_int, true, true)).unwrap();
            number.borrow_mut().define("parseFloat", symbol_data(parse_float, true, true)).unwrap();
        }
        let uri_error = Interpreter::native("URIError", native_error, &self.function_proto);
        if let JsValue::Object(function) = &uri_error {
            if let Some(JsValue::Object(prototype)) = function.borrow().get_own("prototype") {
                prototype.borrow_mut().proto = self.global_env.resolve("Error")
                    .and_then(|error| if let JsValue::Object(object) = error { object.borrow().get_own("prototype") } else { None })
                    .and_then(|value| if let JsValue::Object(object) = value { Some(object) } else { None });
                prototype.borrow_mut().set_own("name", JsValue::str("URIError"));
                prototype.borrow_mut().set_own("message", JsValue::str(""));
                prototype.borrow_mut().set_own("constructor", uri_error.clone());
            }
        }
        self.define_global("URIError", uri_error);
    }
}

fn std_uri_error(i: &mut Interpreter, message: &str) -> Throw {
    if let Some(constructor) = i.global_env.resolve("URIError") {
        if let Ok(value) = i.construct(&constructor, vec![JsValue::str(message)]) { return Throw(value); }
    }
    Throw::error("URIError", message)
}

struct StdJsonParser<'a> {
    units: &'a [u16],
    position: usize,
    depth: usize,
    last_span: (usize, usize),
    sources: HashMap<(usize, crate::value::JsString), crate::value::JsString>,
}
impl<'a> StdJsonParser<'a> {
    fn whitespace(&mut self) {
        while self.units.get(self.position).is_some_and(|unit| matches!(unit, 0x20 | 0x09 | 0x0a | 0x0d)) {
            self.position += 1;
        }
    }
    fn take(&mut self, unit: u16) -> bool {
        if self.units.get(self.position) == Some(&unit) { self.position += 1; true } else { false }
    }
    fn string(&mut self) -> Result<crate::value::JsString, Throw> {
        if !self.take(b'"' as u16) { return Err(Throw::syntax_error("expected JSON string")); }
        let mut out = Vec::new();
        while let Some(&unit) = self.units.get(self.position) {
            self.position += 1;
            if unit == b'"' as u16 { return Ok(crate::value::JsString::from_units(out)); }
            if unit < 0x20 { return Err(Throw::syntax_error("control character in JSON string")); }
            if unit != b'\\' as u16 { out.push(unit); continue; }
            let escaped = *self.units.get(self.position).ok_or_else(|| Throw::syntax_error("unfinished JSON escape"))?;
            self.position += 1;
            match escaped {
                0x22 | 0x5c | 0x2f => out.push(escaped),
                0x62 => out.push(0x08), 0x66 => out.push(0x0c), 0x6e => out.push(0x0a),
                0x72 => out.push(0x0d), 0x74 => out.push(0x09),
                0x75 => {
                    let mut point = 0u16;
                    for _ in 0..4 {
                        let digit = *self.units.get(self.position).ok_or_else(|| Throw::syntax_error("short JSON Unicode escape"))?;
                        self.position += 1;
                        point = (point << 4) | (char::from_u32(digit as u32).and_then(|ch| ch.to_digit(16))
                            .ok_or_else(|| Throw::syntax_error("invalid JSON Unicode escape"))? as u16);
                    }
                    out.push(point);
                }
                _ => return Err(Throw::syntax_error("invalid JSON escape")),
            }
        }
        Err(Throw::syntax_error("unterminated JSON string"))
    }
    fn literal(&mut self, literal: &str, value: JsValue) -> Result<JsValue, Throw> {
        let units = literal.encode_utf16().collect::<Vec<_>>();
        if self.units[self.position..].starts_with(&units) { self.position += units.len(); Ok(value) }
        else { Err(Throw::syntax_error("invalid JSON literal")) }
    }
    fn number(&mut self) -> Result<JsValue, Throw> {
        let start = self.position;
        self.take(b'-' as u16);
        if self.take(b'0' as u16) {
            if self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) {
                return Err(Throw::syntax_error("leading zero in JSON number"));
            }
        } else {
            let mut digits = 0;
            while self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) {
                self.position += 1; digits += 1;
            }
            if digits == 0 { return Err(Throw::syntax_error("invalid JSON number")); }
        }
        if self.take(b'.' as u16) {
            let begin = self.position;
            while self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) { self.position += 1; }
            if begin == self.position { return Err(Throw::syntax_error("missing JSON fraction digits")); }
        }
        if self.take(b'e' as u16) || self.take(b'E' as u16) {
            if !self.take(b'+' as u16) { self.take(b'-' as u16); }
            let begin = self.position;
            while self.units.get(self.position).is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(unit)) { self.position += 1; }
            if begin == self.position { return Err(Throw::syntax_error("missing JSON exponent digits")); }
        }
        let text = String::from_utf16(&self.units[start..self.position]).unwrap_or_default();
        Ok(JsValue::num(text.parse::<f64>().map_err(|_| Throw::syntax_error("invalid JSON number"))?))
    }
    fn value(&mut self, i: &mut Interpreter) -> Result<JsValue, Throw> {
        self.whitespace();
        let start = self.position;
        self.depth += 1;
        if self.depth > 512 { return Err(Throw::syntax_error("JSON nesting limit")); }
        let result = match self.units.get(self.position).copied() {
            Some(0x22) => Ok(JsValue::str(self.string()?)),
            Some(0x74) => self.literal("true", JsValue::Bool(true)),
            Some(0x66) => self.literal("false", JsValue::Bool(false)),
            Some(0x6e) => self.literal("null", JsValue::Null),
            Some(0x5b) => {
                self.position += 1; self.whitespace();
                let mut values = Vec::new();
                let mut spans = Vec::new();
                if !self.take(b']' as u16) {
                    loop {
                        values.push(self.value(i)?);
                        spans.push(self.last_span);
                        self.whitespace();
                        if self.take(b']' as u16) { break; }
                        if !self.take(b',' as u16) { return Err(Throw::syntax_error("expected JSON array comma")); }
                    }
                }
                let array = array_values(i, values)?;
                if let JsValue::Object(object) = &array {
                    for (index, (begin, end)) in spans.into_iter().enumerate() {
                        self.sources.insert((std_weak_id(object), crate::value::JsString::from(index.to_string())),
                            crate::value::JsString::from_units(self.units[begin..end].to_vec()));
                    }
                }
                Ok(array)
            }
            Some(0x7b) => {
                self.position += 1; self.whitespace();
                let object = i.object_value();
                if !self.take(b'}' as u16) {
                    loop {
                        self.whitespace();
                        let key = self.string()?; self.whitespace();
                        if !self.take(b':' as u16) { return Err(Throw::syntax_error("expected JSON colon")); }
                        let value = self.value(i)?;
                        let (begin, end) = self.last_span;
                        if let JsValue::Object(handle) = &object {
                            handle.borrow_mut().set_own(key.clone(), value);
                            self.sources.insert((std_weak_id(handle), key),
                                crate::value::JsString::from_units(self.units[begin..end].to_vec()));
                        }
                        self.whitespace();
                        if self.take(b'}' as u16) { break; }
                        if !self.take(b',' as u16) { return Err(Throw::syntax_error("expected JSON object comma")); }
                    }
                }
                Ok(object)
            }
            Some(0x2d | 0x30..=0x39) => self.number(),
            _ => Err(Throw::syntax_error("invalid JSON token")),
        };
        self.depth -= 1;
        self.last_span = (start, self.position);
        result
    }
}

fn std_json_revive(i: &mut Interpreter, holder: JsValue, key: crate::value::JsString,
    reviver: &JsValue, sources: &HashMap<(usize, crate::value::JsString), crate::value::JsString>,
    depth: usize) -> Result<JsValue, Throw> {
    if depth > 512 { return Err(Throw::range_error("JSON reviver recursion limit")); }
    let value = i.get_prop(&holder, key.clone())?;
    if let JsValue::Object(object) = &value {
        let keys = if matches!(object.borrow().exotic, Exotic::Array) {
            (0..i.array_length(&value)).map(|index| crate::value::JsString::from(index.to_string())).collect::<Vec<_>>()
        } else { object.borrow().own_keys_enumerable() };
        for child in keys {
            let replaced = std_json_revive(i, value.clone(), child.clone(), reviver, sources, depth + 1)?;
            if replaced.is_undefined() { object.borrow_mut().delete_own(child); }
            else {
                // CreateDataProperty returns false when an existing descriptor
                // cannot be replaced; InternalizeJSONProperty ignores that result.
                let can_create = object.borrow().own_prop(child.clone())
                    .is_none_or(|property| property.configurable);
                if can_create {
                    let _ = object.borrow_mut().define(child, symbol_data(replaced, true, true));
                }
            }
        }
    }
    let context = i.object_value();
    if !value.is_object() {
        if let JsValue::Object(object) = &holder {
            if let Some(source) = sources.get(&(std_weak_id(object), key.clone())) {
                if let JsValue::Object(context_object) = &context {
                    context_object.borrow_mut().set_own("source", JsValue::str(source.clone()));
                }
            }
        }
    }
    i.call_value(reviver, holder, vec![JsValue::str(key), value, context])
}
fn std_json_parse(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_js_string(&std_reflect_arg(&args, 0))?;
    let mut parser = StdJsonParser { units: source.units(), position: 0, depth: 0,
        last_span: (0, 0), sources: HashMap::new() };
    let value = parser.value(i)?;
    parser.whitespace();
    if parser.position != source.len() { return Err(Throw::syntax_error("unexpected text after JSON value")); }
    let reviver = std_reflect_arg(&args, 1);
    if callable_function(&reviver).is_none() { return Ok(value); }
    let root = i.object_value();
    if let JsValue::Object(object) = &root {
        object.borrow_mut().set_own("", value);
        let (begin, end) = parser.last_span;
        parser.sources.insert((std_weak_id(object), crate::value::JsString::from("")),
            crate::value::JsString::from_units(source.units()[begin..end].to_vec()));
    }
    std_json_revive(i, root, crate::value::JsString::from(""), &reviver, &parser.sources, 0)
}

struct StdJsonWriter {
    replacer: Option<JsValue>,
    keys: Option<Vec<crate::value::JsString>>,
    gap: String,
    stack: Vec<usize>,
}
impl StdJsonWriter {
    fn property(&mut self, i: &mut Interpreter, holder: &JsValue,
        key: crate::value::JsString, depth: usize) -> Result<Option<String>, Throw> {
        if depth > 512 { return Err(Throw::range_error("JSON nesting limit")); }
        let mut value = i.get_prop(holder, key.clone())?;
        if value.is_object() || matches!(value, JsValue::BigInt(_)) {
            let method = i.get_prop(&value, "toJSON")?;
            if callable_function(&method).is_some() {
                value = i.call_value(&method, value, vec![JsValue::str(key.clone())])?;
            }
        }
        if let Some(replacer) = &self.replacer {
            value = i.call_value(replacer, holder.clone(), vec![JsValue::str(key), value])?;
        }
        let unboxed = if let JsValue::Object(object) = &value {
            let boxed = object.borrow();
            boxed.get_own("[[NumberData]]").or_else(|| boxed.get_own("[[StringData]]"))
                .or_else(|| boxed.get_own("[[BigIntData]]"))
        } else { None };
        if let Some(unboxed) = unboxed { value = unboxed; }
        match &value {
            JsValue::Undefined | JsValue::Symbol(_) => return Ok(None),
            JsValue::Null => return Ok(Some("null".to_string())),
            JsValue::Bool(v) => return Ok(Some(v.to_string())),
            JsValue::Num(v) => return Ok(Some(if v.is_finite() { crate::value::number_to_string(*v) }
                else { "null".to_string() })),
            JsValue::Str(v) => return Ok(Some(json_escape(v))),
            JsValue::BigInt(_) => return Err(Throw::type_error("BigInt is not JSON serializable")),
            JsValue::Object(object) if object.borrow().func.is_some() => return Ok(None),
            JsValue::Object(_) => {}
        }
        let JsValue::Object(object) = &value else { unreachable!() };
        let id = std_weak_id(object);
        if self.stack.contains(&id) { return Err(Throw::type_error("cyclic JSON value")); }
        self.stack.push(id);
        let indent = self.gap.repeat(depth + 1);
        let outer = self.gap.repeat(depth);
        let array = matches!(object.borrow().exotic, Exotic::Array);
        let mut parts = Vec::new();
        if array {
            for index in 0..i.array_length(&value) {
                parts.push(self.property(i, &value, crate::value::JsString::from(index.to_string()), depth + 1)?
                    .unwrap_or_else(|| "null".to_string()));
            }
        } else {
            let keys = if let Some(keys) = &self.keys { keys.clone() } else { i.enumerable_keys(&value)? };
            for child in keys {
                if let Some(serialized) = self.property(i, &value, child.clone(), depth + 1)? {
                    parts.push(format!("{}:{}{}", json_escape(&child), if self.gap.is_empty() { "" } else { " " }, serialized));
                }
            }
        }
        self.stack.pop();
        let (open, close) = if array { ('[', ']') } else { ('{', '}') };
        if parts.is_empty() { return Ok(Some(format!("{open}{close}"))); }
        if self.gap.is_empty() { Ok(Some(format!("{open}{}{close}", parts.join(",")))) }
        else { Ok(Some(format!("{open}\n{indent}{}\n{outer}{close}", parts.join(&format!(",\n{indent}"))))) }
    }
}
fn std_json_stringify(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_reflect_arg(&args, 0);
    let replacer_value = std_reflect_arg(&args, 1);
    let replacer = if callable_function(&replacer_value).is_some() { Some(replacer_value.clone()) } else { None };
    let keys = if let JsValue::Object(object) = &replacer_value {
        if matches!(object.borrow().exotic, Exotic::Array) {
            let mut list = Vec::new();
            for index in 0..i.array_length(&replacer_value) {
                let entry = i.get_prop(&replacer_value, &index.to_string())?;
                let key = match entry {
                    JsValue::Str(key) => Some(key),
                    JsValue::Num(_) => Some(i.to_js_string(&entry)?),
                    JsValue::Object(object) => {
                        let boxed = {
                            let object = object.borrow();
                            object.get_own("[[StringData]]").or_else(|| object.get_own("[[NumberData]]"))
                        };
                        if let Some(value) = boxed { Some(i.to_js_string(&value)?) } else { None }
                    }
                    _ => None,
                };
                if let Some(key) = key { if !list.contains(&key) { list.push(key); } }
            }
            Some(list)
        } else { None }
    } else { None };
    let mut space = std_reflect_arg(&args, 2);
    if let JsValue::Object(object) = &space {
        let unboxed = {
            let object = object.borrow();
            object.get_own("[[NumberData]]").or_else(|| object.get_own("[[StringData]]"))
        };
        if let Some(value) = unboxed { space = value; }
    }
    let gap = match space {
        JsValue::Num(number) => " ".repeat(if number.is_nan() { 0 } else { number.max(0.0).min(10.0) as usize }),
        JsValue::Str(text) => crate::value::JsString::from_units(text.units().iter().take(10).copied().collect::<Vec<_>>()).to_utf8_lossy(),
        _ => String::new(),
    };
    let root = i.object_value();
    if let JsValue::Object(object) = &root { object.borrow_mut().set_own("", value); }
    let mut writer = StdJsonWriter { replacer, keys, gap, stack: Vec::new() };
    Ok(writer.property(i, &root, crate::value::JsString::from(""), 0)?
        .map(JsValue::str).unwrap_or(JsValue::Undefined))
}

fn std_global_parse_int(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_string(&std_reflect_arg(&args, 0))?;
    let source = source.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}');
    let (negative, mut text) = if let Some(rest) = source.strip_prefix('-') { (true, rest) }
        else if let Some(rest) = source.strip_prefix('+') { (false, rest) } else { (false, source) };
    let radix_value = std_reflect_arg(&args, 1);
    let number = i.to_number(&radix_value)?;
    let mut radix = if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as u32 as i32 } else { 0 };
    if radix == 0 { radix = if text.starts_with("0x") || text.starts_with("0X") { 16 } else { 10 }; }
    if !(2..=36).contains(&radix) { return Ok(JsValue::num(f64::NAN)); }
    if radix == 16 && (text.starts_with("0x") || text.starts_with("0X")) { text = &text[2..]; }
    let mut count = 0;
    let mut value = 0.0;
    for ch in text.chars() {
        let Some(digit) = ch.to_digit(radix as u32) else { break; };
        value = value * radix as f64 + digit as f64;
        count += 1;
    }
    if count == 0 { return Ok(JsValue::num(f64::NAN)); }
    Ok(JsValue::num(if negative { -value } else { value }))
}
fn std_global_parse_float(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_string(&std_reflect_arg(&args, 0))?;
    let text = source.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}');
    let (sign, rest) = if let Some(rest) = text.strip_prefix('-') { (-1.0, rest) }
        else if let Some(rest) = text.strip_prefix('+') { (1.0, rest) } else { (1.0, text) };
    if rest.starts_with("Infinity") { return Ok(JsValue::num(sign * f64::INFINITY)); }
    let bytes = rest.as_bytes();
    let mut at = 0;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; }
    let before = at;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; }
    }
    if at == 0 || (before == 0 && at == 1) { return Ok(JsValue::num(f64::NAN)); }
    let exponent_start = at;
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) { at += 1; }
        let digits = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; }
        if digits == at { at = exponent_start; }
    }
    Ok(JsValue::num(sign * rest[..at].parse::<f64>().unwrap_or(f64::NAN)))
}
fn std_global_is_finite(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(i.to_number(&std_reflect_arg(&args, 0))?.is_finite()))
}
fn std_global_is_nan(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(i.to_number(&std_reflect_arg(&args, 0))?.is_nan()))
}

fn std_uri_encode(i: &mut Interpreter, args: Vec<JsValue>, component: bool) -> Result<JsValue, Throw> {
    let value = i.to_js_string(&std_reflect_arg(&args, 0))?;
    let text = String::from_utf16(value.units()).map_err(|_| std_uri_error(i, "unpaired surrogate"))?;
    let mut out = String::new();
    for byte in text.bytes() {
        let ch = byte as char;
        let safe = byte.is_ascii_alphanumeric() || "-_.!~*'()".contains(ch)
            || (!component && ";/?:@&=+$,#".contains(ch));
        if safe { out.push(ch); } else { out.push_str(&format!("%{byte:02X}")); }
    }
    Ok(JsValue::str(out))
}
fn std_uri_decode(i: &mut Interpreter, args: Vec<JsValue>, component: bool) -> Result<JsValue, Throw> {
    let source = i.to_string(&std_reflect_arg(&args, 0))?;
    let bytes = source.as_bytes();
    let mut out = String::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'%' {
            let ch = source[at..].chars().next().unwrap(); out.push(ch); at += ch.len_utf8(); continue;
        }
        let start = at;
        let decode = |index: usize| -> Option<u8> {
            if bytes.get(index) != Some(&b'%') { return None; }
            let hi = (*bytes.get(index + 1)? as char).to_digit(16)?;
            let lo = (*bytes.get(index + 2)? as char).to_digit(16)?;
            Some(((hi << 4) | lo) as u8)
        };
        let first = decode(at).ok_or_else(|| std_uri_error(i, "invalid percent escape"))?;
        let width = if first < 0x80 { 1 } else if (0xc2..=0xdf).contains(&first) { 2 }
            else if (0xe0..=0xef).contains(&first) { 3 }
            else if (0xf0..=0xf4).contains(&first) { 4 }
            else { return Err(std_uri_error(i, "invalid UTF-8 escape")); };
        let mut decoded = vec![first];
        at += 3;
        for _ in 1..width {
            let next = decode(at).ok_or_else(|| std_uri_error(i, "invalid UTF-8 escape"))?;
            decoded.push(next); at += 3;
        }
        let text = String::from_utf8(decoded).map_err(|_| std_uri_error(i, "invalid UTF-8 escape"))?;
        if !component && width == 1 && ";/?:@&=+$,#".contains(text.chars().next().unwrap()) {
            out.push_str(&source[start..at]);
        } else { out.push_str(&text); }
    }
    Ok(JsValue::str(out))
}
fn std_global_encode_uri(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_uri_encode(i, args, false) }
fn std_global_encode_component(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_uri_encode(i, args, true) }
fn std_global_decode_uri(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_uri_decode(i, args, false) }
fn std_global_decode_component(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_uri_decode(i, args, true) }

fn std_global_escape(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_js_string(&std_reflect_arg(&args, 0))?;
    let mut out = String::new();
    for unit in source.units() {
        if *unit < 128 && ((*unit as u8).is_ascii_alphanumeric() || "@*_+-./".contains(*unit as u8 as char)) {
            out.push(*unit as u8 as char);
        } else if *unit < 256 { out.push_str(&format!("%{unit:02X}")); }
        else { out.push_str(&format!("%u{unit:04X}")); }
    }
    Ok(JsValue::str(out))
}
fn std_global_unescape(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.to_js_string(&std_reflect_arg(&args, 0))?;
    let units = source.units();
    let mut out = Vec::new();
    let mut at = 0;
    while at < units.len() {
        if units[at] == b'%' as u16 {
            let (width, start) = if units.get(at + 1) == Some(&(b'u' as u16)) { (4, at + 2) } else { (2, at + 1) };
            if start + width <= units.len() {
                let mut point = 0u16;
                let mut valid = true;
                for unit in &units[start..start + width] {
                    if let Some(digit) = char::from_u32(*unit as u32).and_then(|ch| ch.to_digit(16)) {
                        point = (point << 4) | digit as u16;
                    } else { valid = false; break; }
                }
                if valid { out.push(point); at = start + width; continue; }
            }
        }
        out.push(units[at]); at += 1;
    }
    Ok(JsValue::str_units(out))
}
