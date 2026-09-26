// Uint8Array to/from base64 and hex (ECMA-262 stage-4 proposal, folded into
// %Uint8Array% and %Uint8Array.prototype% only -- not shared with other
// TypedArray kinds). Spec: https://tc39.es/proposal-arraybuffer-base64/spec/
//
// FromBase64/FromHex operate on raw UTF-16 code units (matching JsString's
// representation exactly, so string indices line up 1:1 with the spec's
// string-index arithmetic) and never invoke user code.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Base64Alphabet { Base64, Base64Url }
#[derive(Clone, Copy, PartialEq, Eq)]
enum LastChunkHandling { Loose, Strict, StopBeforePartial }

struct FromBinaryTextResult { read: usize, bytes: Vec<u8>, error: Option<Throw> }

const BASE64_STD_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_URL_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64_std_value(unit: u16) -> Option<u8> {
    match unit {
        0x41..=0x5A => Some((unit - 0x41) as u8),
        0x61..=0x7A => Some((unit - 0x61 + 26) as u8),
        0x30..=0x39 => Some((unit - 0x30 + 52) as u8),
        0x2B => Some(62),
        0x2F => Some(63),
        _ => None,
    }
}

fn skip_ascii_whitespace(units: &[u16], mut index: usize) -> usize {
    let length = units.len();
    while index < length {
        match units[index] {
            0x0009 | 0x000A | 0x000C | 0x000D | 0x0020 => index += 1,
            _ => return index,
        }
    }
    index
}

// DecodeBase64Chunk: `values` holds 2, 3 or 4 already-decoded 6-bit values
// (0..=63); `throw_on_extra_bits` is required (Some) exactly when values.len()
// is 2 or 3, mirroring the spec's asserted-present optional argument.
fn decode_base64_chunk(values: &[u8], throw_on_extra_bits: Option<bool>) -> Result<Vec<u8>, Throw> {
    let mut padded = [0u8; 4];
    padded[..values.len()].copy_from_slice(values);
    let n = ((padded[0] as u32) << 18) | ((padded[1] as u32) << 12)
        | ((padded[2] as u32) << 6) | (padded[3] as u32);
    let out = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
    match values.len() {
        2 => {
            if throw_on_extra_bits == Some(true) && out[1] != 0 {
                return Err(Throw::syntax_error("base64: non-zero padding bits in final chunk"));
            }
            Ok(vec![out[0]])
        }
        3 => {
            if throw_on_extra_bits == Some(true) && out[2] != 0 {
                return Err(Throw::syntax_error("base64: non-zero padding bits in final chunk"));
            }
            Ok(vec![out[0], out[1]])
        }
        4 => Ok(out.to_vec()),
        _ => unreachable!("DecodeBase64Chunk chunk length is always 2, 3 or 4"),
    }
}

// FromBase64 ( string, alphabet, lastChunkHandling [ , maxLength ] )
fn from_base64(units: &[u16], alphabet: Base64Alphabet, last_chunk_handling: LastChunkHandling, max_length: Option<usize>) -> FromBinaryTextResult {
    if max_length == Some(0) {
        return FromBinaryTextResult { read: 0, bytes: Vec::new(), error: None };
    }
    let max_length = max_length.unwrap_or(usize::MAX);
    let length = units.len();
    let mut read = 0usize;
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk: Vec<u8> = Vec::new();
    let mut index = 0usize;
    loop {
        index = skip_ascii_whitespace(units, index);
        if index == length {
            if !chunk.is_empty() {
                match last_chunk_handling {
                    LastChunkHandling::StopBeforePartial => {
                        return FromBinaryTextResult { read, bytes, error: None };
                    }
                    LastChunkHandling::Loose => {
                        if chunk.len() == 1 {
                            return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64: incomplete final chunk")) };
                        }
                        let decoded = decode_base64_chunk(&chunk, Some(false))
                            .expect("throwOnExtraBits=false never throws");
                        bytes.extend(decoded);
                    }
                    LastChunkHandling::Strict => {
                        return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64: incomplete final chunk in strict mode")) };
                    }
                }
            }
            return FromBinaryTextResult { read: length, bytes, error: None };
        }
        let unit = units[index];
        index += 1;
        if unit == b'=' as u16 {
            if chunk.len() < 2 {
                return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64: unexpected padding")) };
            }
            index = skip_ascii_whitespace(units, index);
            if chunk.len() == 2 {
                if index == length {
                    if last_chunk_handling == LastChunkHandling::StopBeforePartial {
                        return FromBinaryTextResult { read, bytes, error: None };
                    }
                    return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64: unexpected end of input after padding")) };
                }
                if units[index] == b'=' as u16 {
                    index = skip_ascii_whitespace(units, index + 1);
                }
            }
            // A 2-value chunk may be followed by one more optional padding
            // character (consumed above); a 3-value chunk takes exactly one.
            // Either way nothing but ASCII whitespace may remain after it, in
            // every lastChunkHandling mode -- this is a hard structural
            // violation, not an ambiguous streaming boundary.
            if index != length {
                return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64: unexpected data after padding")) };
            }
            let throw_on_extra_bits = Some(last_chunk_handling == LastChunkHandling::Strict);
            return match decode_base64_chunk(&chunk, throw_on_extra_bits) {
                Ok(decoded) => { bytes.extend(decoded); FromBinaryTextResult { read: length, bytes, error: None } }
                Err(error) => FromBinaryTextResult { read, bytes, error: Some(error) },
            };
        }
        let mut ch = unit;
        if alphabet == Base64Alphabet::Base64Url {
            if ch == b'+' as u16 || ch == b'/' as u16 {
                return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64url: '+' and '/' are not valid characters")) };
            } else if ch == b'-' as u16 { ch = b'+' as u16; }
            else if ch == b'_' as u16 { ch = b'/' as u16; }
        }
        let Some(value) = base64_std_value(ch) else {
            return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("base64: invalid character")) };
        };
        let remaining = max_length - bytes.len();
        if (remaining == 1 && chunk.len() == 2) || (remaining == 2 && chunk.len() == 3) {
            return FromBinaryTextResult { read, bytes, error: None };
        }
        chunk.push(value);
        // `read` only advances when a full 4-value group actually decodes: a
        // pending partial group that later gets abandoned (via the remaining-
        // capacity check above, on a later iteration) must not count its
        // already-buffered characters as consumed.
        if chunk.len() == 4 {
            let decoded = decode_base64_chunk(&chunk, None).expect("a full 4-value chunk never throws");
            bytes.extend(decoded);
            chunk.clear();
            read = index;
            if bytes.len() == max_length {
                return FromBinaryTextResult { read, bytes, error: None };
            }
        }
    }
}

fn hex_digit(unit: u16) -> Option<u8> {
    match unit {
        0x30..=0x39 => Some((unit - 0x30) as u8),
        0x41..=0x46 => Some((unit - 0x41 + 10) as u8),
        0x61..=0x66 => Some((unit - 0x61 + 10) as u8),
        _ => None,
    }
}

// FromHex ( string [ , maxLength ] )
fn from_hex(units: &[u16], max_length: Option<usize>) -> FromBinaryTextResult {
    let max_length = max_length.unwrap_or(usize::MAX);
    let length = units.len();
    let mut bytes = Vec::new();
    let mut read = 0usize;
    if length % 2 != 0 {
        return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("hex: odd-length string")) };
    }
    while read < length && bytes.len() < max_length {
        let (Some(hi), Some(lo)) = (hex_digit(units[read]), hex_digit(units[read + 1])) else {
            return FromBinaryTextResult { read, bytes, error: Some(Throw::syntax_error("hex: invalid character")) };
        };
        bytes.push((hi << 4) | lo);
        read += 2;
    }
    FromBinaryTextResult { read, bytes, error: None }
}

fn to_base64_string(bytes: &[u8], alphabet: Base64Alphabet, omit_padding: bool) -> String {
    let table = match alphabet { Base64Alphabet::Base64 => BASE64_STD_ALPHABET, Base64Alphabet::Base64Url => BASE64_URL_ALPHABET };
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(table[(n >> 18 & 0x3f) as usize] as char);
        out.push(table[(n >> 12 & 0x3f) as usize] as char);
        match chunk.len() {
            1 => { if !omit_padding { out.push('='); out.push('='); } }
            2 => { out.push(table[(n >> 6 & 0x3f) as usize] as char); if !omit_padding { out.push('='); } }
            3 => { out.push(table[(n >> 6 & 0x3f) as usize] as char); out.push(table[(n & 0x3f) as usize] as char); }
            _ => unreachable!("chunks(3) never yields more than 3 bytes"),
        }
    }
    out
}
fn to_hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes { out.push_str(&format!("{byte:02x}")); }
    out
}

fn base64_get_option(i: &mut Interpreter, options: &JsValue, key: &str) -> Result<JsValue, Throw> {
    match options {
        JsValue::Undefined => Ok(JsValue::Undefined),
        JsValue::Object(_) => i.get_prop(options, key),
        _ => Err(Throw::type_error("options must be an object")),
    }
}
fn base64_alphabet_option(i: &mut Interpreter, options: &JsValue) -> Result<Base64Alphabet, Throw> {
    match base64_get_option(i, options, "alphabet")? {
        JsValue::Undefined => Ok(Base64Alphabet::Base64),
        JsValue::Str(s) if s == "base64" => Ok(Base64Alphabet::Base64),
        JsValue::Str(s) if s == "base64url" => Ok(Base64Alphabet::Base64Url),
        _ => Err(Throw::type_error("alphabet must be \"base64\" or \"base64url\"")),
    }
}
fn base64_last_chunk_handling_option(i: &mut Interpreter, options: &JsValue) -> Result<LastChunkHandling, Throw> {
    match base64_get_option(i, options, "lastChunkHandling")? {
        JsValue::Undefined => Ok(LastChunkHandling::Loose),
        JsValue::Str(s) if s == "loose" => Ok(LastChunkHandling::Loose),
        JsValue::Str(s) if s == "strict" => Ok(LastChunkHandling::Strict),
        JsValue::Str(s) if s == "stop-before-partial" => Ok(LastChunkHandling::StopBeforePartial),
        _ => Err(Throw::type_error("lastChunkHandling must be \"loose\", \"strict\" or \"stop-before-partial\"")),
    }
}

fn base64_require_string(value: Option<&JsValue>) -> Result<crate::value::JsString, Throw> {
    match value {
        Some(JsValue::Str(s)) => Ok(s.clone()),
        _ => Err(Throw::type_error("argument must be a string")),
    }
}
fn base64_require_uint8_array(this: &JsValue) -> Result<BinaryView, Throw> {
    let (view, kind) = binary_typed(this)?;
    if kind != BinaryKind::Uint8 { return Err(Throw::type_error("receiver is not a Uint8Array")); }
    Ok(view)
}
fn get_uint8_array_bytes(view: &BinaryView) -> Result<Vec<u8>, Throw> {
    if binary_view_out_of_bounds(view) { return Err(Throw::type_error("out-of-bounds Uint8Array")); }
    let byte_length = binary_view_bytes(view);
    let buffer = view.buffer.borrow();
    Ok(buffer.bytes[view.offset..view.offset + byte_length].to_vec())
}
fn set_uint8_array_bytes(view: &BinaryView, bytes: &[u8]) {
    let mut buffer = view.buffer.borrow_mut();
    let start = view.offset;
    buffer.bytes[start..start + bytes.len()].copy_from_slice(bytes);
}
fn base64_result_object(i: &Interpreter, read: usize, written: usize) -> JsValue {
    let value = i.object_value();
    if let JsValue::Object(object) = &value {
        let mut object = object.borrow_mut();
        let data = |n: usize| Property { value: JsValue::num(n as f64), writable: true, enumerable: true, configurable: true, getter: None, setter: None };
        let _ = object.define("read", data(read));
        let _ = object.define("written", data(written));
    }
    value
}

fn binary_uint8_static(i: &mut Interpreter, _this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let method = binary_current_name(i);
    let string = base64_require_string(args.first())?;
    let bytes = match method.as_str() {
        "fromBase64" => {
            let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let alphabet = base64_alphabet_option(i, &options)?;
            let last_chunk_handling = base64_last_chunk_handling_option(i, &options)?;
            let result = from_base64(string.units(), alphabet, last_chunk_handling, None);
            if let Some(error) = result.error { return Err(error); }
            result.bytes
        }
        "fromHex" => {
            let result = from_hex(string.units(), None);
            if let Some(error) = result.error { return Err(error); }
            result.bytes
        }
        _ => unreachable!("binary_uint8_static only installs fromBase64 and fromHex"),
    };
    i.binding_create_uint8_array(bytes)
}

fn binary_uint8_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let method = binary_current_name(i);
    match method.as_str() {
        "toBase64" => {
            let view = base64_require_uint8_array(&this)?;
            let options = args.first().cloned().unwrap_or(JsValue::Undefined);
            let alphabet = base64_alphabet_option(i, &options)?;
            let omit_padding = base64_get_option(i, &options, "omitPadding")?.to_boolean();
            let bytes = get_uint8_array_bytes(&view)?;
            Ok(JsValue::str(to_base64_string(&bytes, alphabet, omit_padding)))
        }
        "toHex" => {
            let view = base64_require_uint8_array(&this)?;
            let bytes = get_uint8_array_bytes(&view)?;
            Ok(JsValue::str(to_hex_string(&bytes)))
        }
        "setFromBase64" => {
            let view = base64_require_uint8_array(&this)?;
            let string = base64_require_string(args.first())?;
            // Buffer usability is verified before reading any option getter
            // (an immutable-backed target must reject before ever touching
            // `options.alphabet`/`options.lastChunkHandling`).
            if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("out-of-bounds Uint8Array")); }
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let alphabet = base64_alphabet_option(i, &options)?;
            let last_chunk_handling = base64_last_chunk_handling_option(i, &options)?;
            // An "alphabet"/"lastChunkHandling" getter can run arbitrary code
            // (including detaching the backing buffer), so bounds are
            // re-verified after reading options and before touching bytes.
            if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("out-of-bounds Uint8Array")); }
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let byte_length = binary_view_bytes(&view);
            let result = from_base64(string.units(), alphabet, last_chunk_handling, Some(byte_length));
            let written = result.bytes.len();
            set_uint8_array_bytes(&view, &result.bytes);
            if let Some(error) = result.error { return Err(error); }
            Ok(base64_result_object(i, result.read, written))
        }
        "setFromHex" => {
            let view = base64_require_uint8_array(&this)?;
            let string = base64_require_string(args.first())?;
            if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("out-of-bounds Uint8Array")); }
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let byte_length = binary_view_bytes(&view);
            let result = from_hex(string.units(), Some(byte_length));
            let written = result.bytes.len();
            set_uint8_array_bytes(&view, &result.bytes);
            if let Some(error) = result.error { return Err(error); }
            Ok(base64_result_object(i, result.read, written))
        }
        _ => unreachable!("binary_uint8_method only installs the four base64/hex prototype methods"),
    }
}

impl Interpreter {
    fn install_uint8_base64_hex(&mut self, uint8_ctor: &Obj, uint8_proto: &Obj) {
        binary_method(self, uint8_ctor, "fromBase64", binary_uint8_static, 1);
        binary_method(self, uint8_ctor, "fromHex", binary_uint8_static, 1);
        binary_method(self, uint8_proto, "setFromBase64", binary_uint8_method, 1);
        binary_method(self, uint8_proto, "setFromHex", binary_uint8_method, 1);
        binary_method(self, uint8_proto, "toBase64", binary_uint8_method, 0);
        binary_method(self, uint8_proto, "toHex", binary_uint8_method, 0);
    }
}
