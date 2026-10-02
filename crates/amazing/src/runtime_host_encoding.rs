//! `TextDecoder`/`TextEncoder` (WHATWG Encoding Standard,
//! <https://encoding.spec.whatwg.org/#interface-textdecoder>). Owned by the
//! html lane -- `amazing-encoding` is this lane's own crate -- and entirely
//! self-contained the way `runtime_host_messaging.rs` is: its own
//! brand-numbered host-object arena (`ENCODING_BRAND_BASE`), no new
//! `amazing-bindings` `Interface`/`Operation` table entries. See
//! `docs/lanes/html_TODO.md` (HTML-37) for the WPT measurement that
//! motivated this (`encoding/*.any.js` was ~entirely failing: neither class
//! existed as a JS global at all, even though the codec itself -- decoding,
//! encoding, every label, every family -- was already complete since
//! HTML-15).
use super::*;
use amazing_encoding::{encoding_for_label, DecodeError, Decoder, Encoding};

pub(crate) const ENCODING_BRAND_BASE: u64 = 400;
const KIND_TEXT_DECODER: u64 = 0;
const KIND_TEXT_ENCODER: u64 = 1;

#[derive(Clone, Copy, Debug)]
pub(crate) enum EncodingOp {
    TextDecoderConstructor,
    TextDecoderDecode,
    TextDecoderEncodingGetter,
    TextDecoderFatalGetter,
    TextDecoderIgnoreBomGetter,
    TextEncoderConstructor,
    TextEncoderEncode,
    TextEncoderEncodeInto,
    TextEncoderEncodingGetter,
}

/// Per-instance state for one `TextDecoder`. `decoder`/`bom_seen` are the
/// spec's own "decoder"/"BOM seen" internal slots -- `None` means "not
/// mid-stream" (the previous `decode()` call, if any, had `stream: false`,
/// or this is the first call), matching the spec's "do not flush" flag.
struct TextDecoderState {
    encoding: Encoding,
    fatal: bool,
    ignore_bom: bool,
    decoder: Option<Decoder>,
    bom_seen: bool,
}

impl TextDecoderState {
    /// <https://encoding.spec.whatwg.org/#dom-textdecoder-decode>. `input`
    /// is a byte snapshot already taken by the caller (a detach mid-decode
    /// cannot retroactively invalidate it, matching every other host binding
    /// in this driver that reads a BufferSource once up front).
    fn decode(&mut self, input: &[u8], stream: bool) -> Result<String, DecodeError> {
        let mut decoder = self
            .decoder
            .take()
            .unwrap_or_else(|| { self.bom_seen = false; Decoder::new(self.encoding, self.fatal) });
        let mut output = String::new();
        let result = decoder.push(input, &mut output)
            .and_then(|()| if stream { Ok(()) } else { decoder.finish(&mut output) });
        // "Set this's do not flush to stream" happens regardless of whether
        // the steps above signal an error -- so the decoder is kept (continue
        // next call) exactly when this call was itself streaming, discarded
        // otherwise, independent of `result`.
        if stream { self.decoder = Some(decoder); }
        result?;
        if !self.bom_seen && !output.is_empty() {
            self.bom_seen = true;
            if output.starts_with('\u{FEFF}')
                && matches!(self.encoding, Encoding::Utf8 | Encoding::Utf16Be | Encoding::Utf16Le)
                && !self.ignore_bom
            {
                output.remove(0);
            }
        }
        Ok(output)
    }
}

enum EncodingObject {
    Decoder(TextDecoderState),
    Encoder,
}

#[derive(Default)]
pub(crate) struct EncodingRealm {
    objects: Vec<EncodingObject>,
    classes: HashMap<u64, (JsValue, JsValue)>,
}

impl RuntimeBindingHost<'_> {
    fn encoding_ensure(&mut self) {
        let mut state = self.state.borrow_mut();
        if state.encoding.is_none() {
            state.encoding = Some(EncodingRealm::default());
        }
    }

    fn encoding_set_class(&self, kind: u64, prototype: JsValue, constructor: JsValue) {
        if let Some(realm) = self.state.borrow_mut().encoding.as_mut() {
            realm.classes.insert(kind, (prototype, constructor));
        }
    }

    fn encoding_class(&self, kind: u64) -> Option<JsValue> {
        self.state.borrow().encoding.as_ref()
            .and_then(|realm| realm.classes.get(&kind))
            .map(|pair| pair.0.clone())
    }

    fn encoding_allocate(&mut self, kind: u64, object: EncodingObject) -> Result<JsValue, Throw> {
        self.encoding_ensure();
        let prototype = self.encoding_class(kind).unwrap_or_else(|| self.js.binding_object_prototype());
        let slot = {
            let mut state = self.state.borrow_mut();
            let realm = state.encoding.as_mut().unwrap();
            let slot = realm.objects.len();
            realm.objects.push(object);
            slot
        };
        let arena = self.state.borrow().arena;
        self.js.binding_wrap_host(
            HostHandle { arena, slot: slot as u64, generation: 1, brand: ENCODING_BRAND_BASE + kind },
            &prototype,
        )
    }

    fn encoding_receiver(&self, value: &JsValue, kind: u64) -> Result<usize, Throw> {
        let handle = self.js.binding_host_handle(value, ENCODING_BRAND_BASE + kind)
            .ok_or_else(|| Throw::type_error("Illegal invocation"))?;
        if handle.arena != self.state.borrow().arena || handle.generation != 1 {
            return Err(Throw::type_error("Illegal invocation"));
        }
        let slot = handle.slot as usize;
        if self.state.borrow().encoding.as_ref().is_none_or(|realm| slot >= realm.objects.len()) {
            return Err(Throw::type_error("Illegal invocation"));
        }
        Ok(slot)
    }

    /// A WebIDL dictionary member's boolean field, `ToBoolean`-coerced
    /// (matching this driver's existing dictionary-reading convention, e.g.
    /// `MessagingOp::WindowPostMessage`'s options handling) rather than the
    /// stricter ECMA-262 `ToBoolean` abstract op distinction that doesn't
    /// exist here anyway. `undefined`/`null` (options omitted entirely) and
    /// an explicit `undefined` member both fall back to `default`.
    fn encoding_dict_bool(&mut self, dict: &JsValue, key: &str, default: bool) -> Result<bool, Throw> {
        if matches!(dict, JsValue::Undefined | JsValue::Null) { return Ok(default); }
        let value = self.js.binding_get_property(dict, key)?;
        Ok(if matches!(value, JsValue::Undefined) { default } else { self.js.binding_to_boolean(&value) })
    }

    fn encoding_arg_string(&mut self, args: &[JsValue], index: usize, default: &str) -> Result<String, Throw> {
        match args.get(index) {
            Some(value) if !matches!(value, JsValue::Undefined) => {
                let units = self.js.binding_to_dom_string(value)?;
                Ok(String::from_utf16_lossy(&units))
            }
            _ => Ok(default.to_string()),
        }
    }

    pub(super) fn invoke_encoding(&mut self, op: EncodingOp, this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        match op {
            EncodingOp::TextDecoderConstructor => {
                let label = self.encoding_arg_string(args, 0, "utf-8")?;
                let Some(encoding) = encoding_for_label(&label) else {
                    return Err(Throw::range_error(format!(
                        "Failed to construct 'TextDecoder': The encoding label provided ('{label}') is invalid."
                    )));
                };
                if encoding == Encoding::Replacement {
                    return Err(Throw::range_error("The replacement encoding is not supported by TextDecoder."));
                }
                let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
                let fatal = self.encoding_dict_bool(&options, "fatal", false)?;
                let ignore_bom = self.encoding_dict_bool(&options, "ignoreBOM", false)?;
                self.encoding_allocate(KIND_TEXT_DECODER, EncodingObject::Decoder(TextDecoderState {
                    encoding, fatal, ignore_bom, decoder: None, bom_seen: false,
                }))
            }
            EncodingOp::TextDecoderDecode => {
                let slot = self.encoding_receiver(&this, KIND_TEXT_DECODER)?;
                // WebIDL converts arguments left to right, and the `options`
                // dictionary conversion (which can run an `options.stream`
                // getter) completes before the method body's own "get a copy
                // of the bytes held by input" step runs. A getter that
                // detaches `input`'s buffer as a side effect must therefore
                // see that detach reflected in the byte copy: options are
                // read first, and the buffer bytes are snapshotted after.
                let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
                let stream = self.encoding_dict_bool(&options, "stream", false)?;
                let input = match args.first() {
                    Some(value) if !matches!(value, JsValue::Undefined) => {
                        // WebIDL's "get a copy of the bytes held by" a
                        // BufferSource treats an already-detached buffer as
                        // an empty byte sequence rather than an error; this
                        // is what lets a `stream` getter detach `input`'s
                        // buffer as a side effect without decode() throwing.
                        // `binding_buffer_source_bytes` otherwise errors on
                        // detached buffers for callers that want that; only
                        // that specific error is downgraded here.
                        match self.js.binding_buffer_source_bytes(value) {
                            Ok(Some(bytes)) => bytes,
                            Ok(None) => return Err(Throw::type_error(
                                "Failed to execute 'decode' on 'TextDecoder': The provided value is not of type '(ArrayBuffer or ArrayBufferView)'."
                            )),
                            Err(_) => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                };
                let output = {
                    let mut state = self.state.borrow_mut();
                    let realm = state.encoding.as_mut().unwrap();
                    let EncodingObject::Decoder(decoder) = &mut realm.objects[slot] else {
                        return Err(Throw::type_error("Illegal invocation"));
                    };
                    decoder.decode(&input, stream)
                        .map_err(|_| Throw::type_error("The encoded data was not valid."))?
                };
                Ok(JsValue::str(output))
            }
            EncodingOp::TextDecoderEncodingGetter | EncodingOp::TextDecoderFatalGetter
            | EncodingOp::TextDecoderIgnoreBomGetter => {
                let slot = self.encoding_receiver(&this, KIND_TEXT_DECODER)?;
                let state = self.state.borrow();
                let EncodingObject::Decoder(decoder) = &state.encoding.as_ref().unwrap().objects[slot] else {
                    return Err(Throw::type_error("Illegal invocation"));
                };
                Ok(match op {
                    EncodingOp::TextDecoderEncodingGetter => JsValue::str(decoder.encoding.name()),
                    EncodingOp::TextDecoderFatalGetter => JsValue::Bool(decoder.fatal),
                    _ => JsValue::Bool(decoder.ignore_bom),
                })
            }
            EncodingOp::TextEncoderConstructor => {
                self.encoding_allocate(KIND_TEXT_ENCODER, EncodingObject::Encoder)
            }
            EncodingOp::TextEncoderEncode => {
                self.encoding_receiver(&this, KIND_TEXT_ENCODER)?;
                let input = self.encoding_arg_string(args, 0, "")?;
                self.js.binding_create_uint8_array(input.into_bytes())
            }
            EncodingOp::TextEncoderEncodingGetter => {
                self.encoding_receiver(&this, KIND_TEXT_ENCODER)?;
                Ok(JsValue::str("utf-8"))
            }
            EncodingOp::TextEncoderEncodeInto => {
                self.encoding_receiver(&this, KIND_TEXT_ENCODER)?;
                if args.len() < 2 { return Err(Throw::type_error("encodeInto requires source and destination")); }
                // WebIDL converts the USVString before validating the view;
                // conversion may detach the destination through user code.
                let units = self.js.binding_to_dom_string(&args[0])?;
                let source = String::from_utf16_lossy(&units);
                let destination = &args[1];
                let capacity = self.js.binding_uint8_array_info(destination)?
                    .ok_or_else(|| Throw::type_error("encodeInto destination must be a Uint8Array"))?;
                let mut bytes = Vec::with_capacity(capacity.min(source.len()));
                let mut read = 0;
                for scalar in source.chars() {
                    if scalar.len_utf8() > capacity - bytes.len() { break; }
                    let mut encoded = [0; 4];
                    bytes.extend_from_slice(scalar.encode_utf8(&mut encoded).as_bytes());
                    read += scalar.len_utf16();
                }
                self.js.binding_write_uint8_array(destination, 0, &bytes)?;
                let result = self.js.binding_create_object(None)?;
                self.define_data_property(&result, "read", JsValue::num(read as f64), true, true, true)?;
                self.define_data_property(&result, "written", JsValue::num(bytes.len() as f64), true, true, true)?;
                Ok(result)
            }
        }
    }

    /// Installs both classes onto *this* realm's own global object, mirroring
    /// `install_messaging`'s own top-level-plus-child-realm install sequence
    /// (`runtime_host_html.rs::setup_iframe_window` calls this too, so
    /// `frame.contentWindow.TextDecoder !== window.TextDecoder`, matching
    /// every other constructor there).
    ///
    /// Not covered by this ticket: `TextDecoderStream`/`TextEncoderStream`
    /// (the Streams-API transform-stream wrappers) -- this engine has no
    /// `ReadableStream`/`WritableStream`/`TransformStream` at all yet, a
    /// separate, much bigger cross-cutting gap, not specific to encoding.
    pub(super) fn install_text_codec(&mut self) -> Result<(), Throw> {
        self.encoding_ensure();
        let global = self.js.binding_global_object();
        let object_proto = self.js.binding_object_prototype();

        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        let constructor = self.native("TextDecoder", 0,
            NativeEntry::Encoding(EncodingOp::TextDecoderConstructor), HostCallPolicy::ConstructOnly);
        self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
        let decode = self.native("decode", 0, NativeEntry::Encoding(EncodingOp::TextDecoderDecode), HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "decode", decode, true, false, true)?;
        for (name, getter_op) in [
            ("encoding", EncodingOp::TextDecoderEncodingGetter),
            ("fatal", EncodingOp::TextDecoderFatalGetter),
            ("ignoreBOM", EncodingOp::TextDecoderIgnoreBomGetter),
        ] {
            let getter = self.native(name, 0, NativeEntry::Encoding(getter_op), HostCallPolicy::CallOnly);
            self.define_accessor_property(&prototype, name, Some(getter), None, true, true)?;
        }
        self.define_data_property(&global, "TextDecoder", constructor.clone(), true, false, true)?;
        self.encoding_set_class(KIND_TEXT_DECODER, prototype, constructor);

        let prototype = self.js.binding_create_object(Some(&object_proto))?;
        let constructor = self.native("TextEncoder", 0,
            NativeEntry::Encoding(EncodingOp::TextEncoderConstructor), HostCallPolicy::ConstructOnly);
        self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
        self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
        let encode = self.native("encode", 0, NativeEntry::Encoding(EncodingOp::TextEncoderEncode), HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "encode", encode, true, false, true)?;
        let encode_into = self.native("encodeInto", 2, NativeEntry::Encoding(EncodingOp::TextEncoderEncodeInto), HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "encodeInto", encode_into, true, false, true)?;
        let getter = self.native("encoding", 0, NativeEntry::Encoding(EncodingOp::TextEncoderEncodingGetter), HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "encoding", Some(getter), None, true, true)?;
        self.define_data_property(&global, "TextEncoder", constructor.clone(), true, false, true)?;
        self.encoding_set_class(KIND_TEXT_ENCODER, prototype, constructor);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_encoder_encode_into_preserves_scalar_boundaries_and_view_edges() {
        assert_eq!(run(r#"
            var encoder = new TextEncoder(), buffer = new ArrayBuffer(8);
            var all = new Uint8Array(buffer); all.fill(9);
            var view = new Uint8Array(buffer, 2, 4);
            var a = encoder.encodeInto('😀A', view);
            var b = encoder.encodeInto('😀', new Uint8Array(3));
            var c = encoder.encodeInto('\uD800A', new Uint8Array(4));
            a.read === 2 && a.written === 4 && b.read === 0 && b.written === 0 &&
            c.read === 2 && c.written === 4 && all[0] === 9 && all[1] === 9 &&
            all[2] === 240 && all[5] === 128 && all[6] === 9 && all[7] === 9;
        "#), JsValue::Bool(true));
    }

    #[test]
    fn text_encoder_encode_into_rejects_invalid_views_and_missing_arguments() {
        assert_eq!(run(r#"
            var encoder = new TextEncoder(), rejected = 0;
            for (var value of [new Int8Array(2), new Uint8ClampedArray(2), new Uint16Array(2), new DataView(new ArrayBuffer(2)), new ArrayBuffer(2), {}, null]) {
                try { encoder.encodeInto('A', value); } catch (error) { if (error instanceof TypeError) rejected++; }
            }
            try { encoder.encodeInto('A'); } catch (error) { if (error instanceof TypeError) rejected++; }
            rejected === 8;
        "#), JsValue::Bool(true));
    }

    #[test]
    fn text_decoder_fatal_stream_recovers_consumed_errors_and_keeps_mode() {
        assert_eq!(run(r#"
            var d = new TextDecoder('utf-8', {fatal:true}), failures = 0;
            try { d.decode(new Uint8Array([192]), {stream:true}); } catch(e) { failures++; }
            try { d.decode(new Uint8Array([255]), {stream:true}); } catch(e) { failures++; }
            var cleared = d.decode() === '';
            var j = new TextDecoder('iso-2022-jp', {fatal:true});
            try { j.decode(new Uint8Array([27,40,74,255]), {stream:true}); } catch(e) { failures++; }
            var roman = j.decode(new Uint8Array([126])) === '\u203e';
            try { new TextDecoder('replacement'); } catch(e) { if(e instanceof RangeError) failures++; }
            cleared && roman && failures === 4;
        "#), JsValue::Bool(true));
    }

    fn run(script: &str) -> JsValue {
        let dom = amazing_html::parse("<!DOCTYPE html><html><head></head><body></body></html>");
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::install(&mut js, host).unwrap();
        js.run_script(script).unwrap()
    }

    #[test]
    fn text_decoder_decodes_utf8_and_defaults_label_to_utf8() {
        let result = run(r#"
            const bytes = new Uint8Array([0xE2, 0x9C, 0x93]);
            const decoder = new TextDecoder();
            decoder.encoding === 'utf-8' && decoder.decode(bytes) === '✓';
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_decoder_strips_bom_unless_ignore_bom() {
        let result = run(r#"
            const bytes = new Uint8Array([0xEF, 0xBB, 0xBF, 0x61]);
            const stripped = new TextDecoder('utf-8').decode(bytes);
            const kept = new TextDecoder('utf-8', { ignoreBOM: true }).decode(bytes);
            stripped === 'a' && kept === '﻿a';
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_decoder_fatal_throws_type_error_and_non_fatal_emits_replacement() {
        let result = run(r#"
            const bad = new Uint8Array([0xFF]);
            let threw = false;
            try { new TextDecoder('utf-8', { fatal: true }).decode(bad); }
            catch (e) { threw = e instanceof TypeError; }
            const lenient = new TextDecoder('utf-8').decode(bad);
            threw && lenient === '�';
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_decoder_streaming_reassembles_a_multi_byte_sequence_split_across_calls() {
        let result = run(r#"
            const decoder = new TextDecoder();
            const first = decoder.decode(new Uint8Array([0xE2]), { stream: true });
            const second = decoder.decode(new Uint8Array([0x9C, 0x93]), { stream: true });
            const rest = decoder.decode();
            first === '' && second === '✓' && rest === '';
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_decoder_unknown_label_throws_range_error() {
        let result = run(r#"
            let threw = false;
            try { new TextDecoder('not-a-real-encoding'); }
            catch (e) { threw = e instanceof RangeError; }
            threw;
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_decoder_requires_new() {
        let result = run(r#"
            let threw = false;
            try { TextDecoder(); } catch (e) { threw = e instanceof TypeError; }
            threw;
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_encoder_encodes_utf8_and_fixes_encoding_to_utf8() {
        let result = run(r#"
            const encoder = new TextEncoder();
            const bytes = encoder.encode('✓');
            encoder.encoding === 'utf-8' && bytes.length === 3
                && bytes[0] === 0xE2 && bytes[1] === 0x9C && bytes[2] === 0x93;
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn text_encoder_replaces_lone_surrogates_with_u_fffd() {
        let result = run(r#"
            const bytes = new TextEncoder().encode('\uD800');
            bytes.length === 3 && bytes[0] === 0xEF && bytes[1] === 0xBF && bytes[2] === 0xBD;
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn round_trips_a_non_utf8_legacy_encoding_by_label() {
        let result = run(r#"
            const decoder = new TextDecoder('shift_jis');
            decoder.decode(new Uint8Array([0x82, 0xA0])) === 'あ';
        "#);
        assert_eq!(result, JsValue::Bool(true));
    }
}
