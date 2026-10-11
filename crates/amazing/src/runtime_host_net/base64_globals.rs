//! `atob()`/`btoa()` window operations (HTML Standard, "Base64 utility methods",
//! with the Infra "forgiving-base64" encode/decode algorithms).
use super::*;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Infra "forgiving-base64 encode" of a byte sequence.
pub(super) fn forgiving_base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Infra "forgiving-base64 decode"; `None` is the spec's failure.
pub(super) fn forgiving_base64_decode(input: &[u16]) -> Option<Vec<u8>> {
    let mut data: Vec<u16> = input
        .iter()
        .copied()
        .filter(|unit| !matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
        .collect();
    if data.len() % 4 == 0 {
        for _ in 0..2 {
            if data.last() == Some(&(b'=' as u16)) {
                data.pop();
            }
        }
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut values = Vec::with_capacity(data.len());
    for unit in &data {
        let byte = u8::try_from(*unit).ok()?;
        values.push(ALPHABET.iter().position(|candidate| *candidate == byte)? as u32);
    }
    let mut out = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, v)| acc | (v << (18 - 6 * i as u32)));
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_base64_globals(&mut self) -> Result<(), Throw> {
        let global = self.js.binding_global_object();
        for (name, entry) in [("atob", NetNativeEntry::Atob), ("btoa", NetNativeEntry::Btoa)] {
            let function = self.native(name, 1, NativeEntry::Net(entry), HostCallPolicy::CallOnly);
            self.define_data_property(&global, name, function, true, true, true)?;
        }
        Ok(())
    }

    pub(super) fn base64_invoke(&mut self, encode: bool, args: &[JsValue]) -> Result<JsValue, Throw> {
        let name = if encode { "btoa" } else { "atob" };
        let Some(argument) = args.first() else {
            return Err(Throw::type_error(format!(
                "Failed to execute '{name}' on 'Window': 1 argument required, but only 0 present."
            )));
        };
        let units = self.js.binding_to_dom_string(argument)?;
        if encode {
            let mut bytes = Vec::with_capacity(units.len());
            for unit in &units {
                match u8::try_from(*unit) {
                    Ok(byte) => bytes.push(byte),
                    Err(_) => {
                        return Err(self.ws_dom_exception(
                            "Failed to execute 'btoa' on 'Window': The string to be encoded contains characters outside of the Latin1 range.",
                            "InvalidCharacterError",
                        ))
                    }
                }
            }
            return Ok(JsValue::str(forgiving_base64_encode(&bytes)));
        }
        match forgiving_base64_decode(&units) {
            Some(bytes) => {
                let text: Vec<u16> = bytes.iter().map(|byte| u16::from(*byte)).collect();
                Ok(self.js.binding_string(&text))
            }
            None => Err(self.ws_dom_exception(
                "Failed to execute 'atob' on 'Window': The string to be decoded is not correctly encoded.",
                "InvalidCharacterError",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn encodes_with_padding() {
        assert_eq!(forgiving_base64_encode(b""), "");
        assert_eq!(forgiving_base64_encode(b"f"), "Zg==");
        assert_eq!(forgiving_base64_encode(b"fo"), "Zm8=");
        assert_eq!(forgiving_base64_encode(b"foo"), "Zm9v");
        assert_eq!(forgiving_base64_encode(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn decode_is_forgiving_about_whitespace_and_padding_only() {
        assert_eq!(forgiving_base64_decode(&units("Zm9v")).unwrap(), b"foo");
        assert_eq!(forgiving_base64_decode(&units(" Z m\t9\nv ")).unwrap(), b"foo");
        assert_eq!(forgiving_base64_decode(&units("Zg")).unwrap(), b"f");
        assert_eq!(forgiving_base64_decode(&units("Zg==")).unwrap(), b"f");
        assert_eq!(forgiving_base64_decode(&units("")).unwrap(), b"");
        assert!(forgiving_base64_decode(&units("Z")).is_none());
        assert!(forgiving_base64_decode(&units("Zg=")).is_none());
        assert!(forgiving_base64_decode(&units("Zg===")).is_none());
        assert!(forgiving_base64_decode(&units("Zm9v!")).is_none());
        assert!(forgiving_base64_decode(&units("Zm\u{100}v")).is_none());
        assert!(forgiving_base64_decode(&units("=Zg=")).is_none());
    }

    fn run(script: &str) -> amazing_js::value::JsValue {
        use crate::{dom_host::BrowserDomHost, js_scheduler::DriverScheduler};
        use std::{cell::RefCell, rc::Rc};
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/");
        scheduler
            .run_script(script)
            .unwrap_or_else(|error| panic!("script failed: {}", error.describe()))
    }

    #[test]
    fn window_atob_and_btoa_round_trip_and_throw_invalid_character_error() {
        let result = run(r#"
            var out = [btoa('foo'), atob('Zm9v'), atob(' Zg '), btoa('\u00ff'), atob('/w==').charCodeAt(0)];
            var errors = [];
            for (var f of [function() { btoa('\u0100'); }, function() { atob('Z'); }]) {
              try { f(); errors.push('no throw'); } catch (e) { errors.push(e.name); }
            }
            try { atob(); errors.push('no throw'); } catch (e) { errors.push(e.constructor === TypeError); }
            out.join(',') + '|' + errors.join(',')
        "#);
        assert_eq!(
            result,
            amazing_js::value::JsValue::str("Zm9v,foo,f,/w==,255|InvalidCharacterError,InvalidCharacterError,true")
        );
    }

    #[test]
    fn response_json_static_sets_content_type_and_rejects_unserializable_values() {
        let result = run(r#"
            var r = Response.json({a: 1}, {status: 201});
            var parts = [r.status, r.headers.get('content-type')];
            var kept = Response.json(1, {headers: {'Content-Type': 'application/x-foo'}});
            parts.push(kept.headers.get('content-type'));
            try { Response.json(undefined); parts.push('no throw'); } catch (e) { parts.push(e.constructor === TypeError); }
            try { Response.json(1, {status: 204}); parts.push('no throw'); } catch (e) { parts.push(e.constructor === TypeError); }
            parts.join(',')
        "#);
        assert_eq!(
            result,
            amazing_js::value::JsValue::str("201,application/json,application/x-foo,true,true")
        );
    }
}
