#[cfg(test)]
mod json_data_tests {
    use super::*;
    use crate::json_data::JsonDocument;

    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn data_parser_whole_grammar_acceptance_and_rejection() {
        for text in ["null", "true", "false", "-0", "0", "1.25", "-2e+3", "1E-9", "1e400",
            "[]", "{}", "[null,true,false,0,\"x\"]", "{\"a\":1,\"a\":2}",
            " \t\r\n {\"__proto__\":{\"x\":1}} \t", "\"\\b\\f\\n\\r\\t\\/\\\\\\\"\\u0000\\uD800\"",
            "\"\u{2028}\u{2029}\"", "\"😀\""] {
            assert!(JsonDocument::parse_utf16(&text.encode_utf16().collect::<Vec<_>>()).is_ok(), "{text}");
        }
        for text in ["", " ", "undefined", "NaN", "Infinity", "+1", "01", "-01", "-", ".1", "1.", "1e", "1e+", "1e-",
            "true false", "null;", "(1)", "[1,]", "[,1]", "{a:1}", "{'a':1}", "{\"a\":1,}", "{\"a\" 1}",
            "\"\u{0}\"", "\"\n\"", "\"\\v\"", "\"\\x00\"", "\"\\u00GG\"", "\"\\u123\"", "\"x",
            "/*x*/1", "1//x", "\u{a0}1", "1\u{2028}"] {
            assert!(JsonDocument::parse_utf16(&text.encode_utf16().collect::<Vec<_>>()).is_err(), "{text}");
        }
    }

    #[test]
    fn exact_raw_and_escaped_utf16_data_and_source_ranges() {
        for units in [vec![0x22, 0xd800, 0x22], vec![0x22, 0xdc00, 0x22], vec![0x22, 0xd83d, 0xde00, 0x22]] {
            let document = JsonDocument::parse_utf16(&units).unwrap();
            assert_eq!(document.source_units(), units);
            let mut i = Interpreter::new();
            let value = i.binding_materialize_json(&document).unwrap();
            let JsValue::Str(value) = value else { panic!("string JSON") };
            assert_eq!(value.units(), &units[1..units.len()-1]);
        }
        both(r#"var s;var v=JSON.parse('"\\ud800"',function(k,v,c){s=c.source;return v;});
            v.length===1&&v.charCodeAt(0)===0xd800&&s==='"\\ud800"';"#);
    }

    #[test]
    fn validated_document_materializes_independently_in_each_actual_realm() {
        let document = JsonDocument::parse_utf16(&"{\"a\":[1]}".encode_utf16().collect::<Vec<_>>()).unwrap();
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let global = i.binding_create_object(None).unwrap();
        let child = i.binding_create_realm(global).unwrap();
        for realm in [parent, child] {
            i.binding_with_realm(realm, |i| {
                let value = i.binding_materialize_json(&document)?;
                let JsValue::Object(object) = &value else { panic!("JSON object") };
                assert!(object.borrow().proto.as_ref().is_some_and(|prototype| i.binding_same_value(&JsValue::Object(prototype.clone()), &JsValue::Object(i.object_proto.clone()))));
                let array = i.get_prop(&value, "a")?;
                let JsValue::Object(array) = array else { panic!("JSON array") };
                assert!(array.borrow().proto.as_ref().is_some_and(|prototype| i.binding_same_value(&JsValue::Object(prototype.clone()), &JsValue::Object(i.array_proto.clone()))));
                Ok(())
            }).unwrap();
        }
        let foreign_parse = i.binding_eval_in_realm(child, "JSON.parse").unwrap();
        let value = i.binding_call(&foreign_parse, JsValue::Undefined, vec![JsValue::str("[]")]).unwrap();
        let prototype = i.binding_eval_in_realm(child, "Array.prototype").unwrap();
        let JsValue::Object(value) = value else { panic!("array") };
        assert!(i.binding_same_value(&JsValue::Object(value.borrow().proto.clone().unwrap()), &prototype));
        assert_eq!(i.binding_current_realm(), parent);
    }
}
