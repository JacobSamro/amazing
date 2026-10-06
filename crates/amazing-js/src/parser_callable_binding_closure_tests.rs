// Additive controls after the full original corpus exposed remaining consumers.
use super::*;
use crate::interp::{Interpreter,ExecutionBackend};
use crate::value::JsValue;

#[test]
fn complete_strict_binding_rules_apply_to_every_callable_parameter_consumer() {
    let names=["implements","interface","let","package","private","protected","public","static","yield","eval","arguments"];
    for name in names {
        for head in ["method", "*method", "async method", "async *method", "set method"] {
            let source=format!("({{{head}({name}){{'use strict';}}}})");
            assert!(crate::parse_script(&source).is_err(),"{source}");
            let units: Vec<_>=source.encode_utf16().collect();
            assert!(crate::parse_script_utf16(&units).is_err(),"UTF16 {source}");
            let inherited=format!("'use strict';({{{head}({name}){{}}}})");
            assert!(crate::parse_script(&inherited).is_err(),"{inherited}");
        }
        for head in [format!("({name})=>"),format!("async ({name})=>")] {
            let source=format!("{head}{{'use strict';}}");
            assert!(crate::parse_script(&source).is_err(),"{source}");
        }
        for binding in [format!("{{{name}}}"),format!("[{name}]"),format!("...{name}")] {
            let source=format!("'use strict';({{method({binding}){{}}}})");
            assert!(crate::parse_script(&source).is_err(),"{source}");
        }
        if !matches!(name,"eval"|"arguments") {
            assert!(crate::parse_script(&format!("({{method({name}){{return 1;}}}})")).is_ok());
        }
    }
    for source in ["({set value(value){'use strict';this.saved=value;}})",
        "({method(value){'use strict';return value;}})","(value)=>{'use strict';return value;}",
        "async(value)=>{'use strict';return value;}","class C{method(value){return value;}}"] {
        assert!(crate::parse_script(source).is_ok(),"{source}");
    }
}

#[test]
fn accessor_name_lookahead_preserves_actual_field_generator_asi_and_valid_lf_accessors() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        for field in ["get","set"] {
            for modifier in ["", "static "] {
                let source=format!("class C{{{modifier}{field}\n*values(){{yield 7;}}}}let receiver={};Object.prototype.hasOwnProperty.call(receiver,'{field}')&&receiver.{field}===undefined&&Object.prototype.hasOwnProperty.call(C.prototype,'values')&&(new C()).values().next().value===7",if modifier.is_empty(){"new C()"}else{"C"});
                let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);
                assert_eq!(i.run_script_with_backend(&source,backend).unwrap(),JsValue::Bool(true),"{source}");
            }
        }
        let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);
        assert_eq!(i.run_script_with_backend("class C{get\n value(){return 3;}set\n value(v){this.saved=v;}}let c=new C();c.value=7;c.value===3&&c.saved===7",backend).unwrap(),JsValue::Bool(true));
    }
}
