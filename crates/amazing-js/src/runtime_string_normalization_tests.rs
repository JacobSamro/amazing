// Native interface tests authored for Amazing; Unicode values are normalization examples.
#[cfg(test)]
mod string_normalization_protocol_tests {
    use super::*;

    #[test]
    fn all_forms_cover_recursive_ordering_compatibility_hangul_and_utf16() {
        let examples = [
            ("e\u{301}", "é", "e\u{301}", "é", "e\u{301}"),
            ("\u{212b}", "Å", "A\u{30a}", "Å", "A\u{30a}"),
            ("\u{fb03}", "\u{fb03}", "\u{fb03}", "ffi", "ffi"),
            ("\u{ff21}", "\u{ff21}", "\u{ff21}", "A", "A"),
            ("\u{1100}\u{1161}\u{11a8}", "각", "\u{1100}\u{1161}\u{11a8}", "각", "\u{1100}\u{1161}\u{11a8}"),
            ("각", "각", "\u{1100}\u{1161}\u{11a8}", "각", "\u{1100}\u{1161}\u{11a8}"),
            ("\u{315}\u{300}", "\u{300}\u{315}", "\u{300}\u{315}", "\u{300}\u{315}", "\u{300}\u{315}"),
            ("A\u{305}\u{30a}", "A\u{305}\u{30a}", "A\u{305}\u{30a}", "A\u{305}\u{30a}", "A\u{305}\u{30a}"),
            ("\u{344}", "\u{308}\u{301}", "\u{308}\u{301}", "\u{308}\u{301}", "\u{308}\u{301}"),
            ("\u{2126}", "Ω", "Ω", "Ω", "Ω"),
            ("ḋ\u{323}", "ḍ\u{307}", "d\u{323}\u{307}", "ḍ\u{307}", "d\u{323}\u{307}"),
            ("\u{1d15e}", "\u{1d157}\u{1d165}", "\u{1d157}\u{1d165}", "\u{1d157}\u{1d165}", "\u{1d157}\u{1d165}"),
            // Tai Yo combining class230 is new in Unicode17, and must order
            // after class220 while preserving the actual supplementary pair.
            ("\u{1e6e6}\u{323}", "\u{323}\u{1e6e6}", "\u{323}\u{1e6e6}", "\u{323}\u{1e6e6}", "\u{323}\u{1e6e6}"),
        ];
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let method = i.run_script_with_backend("String.prototype.normalize", backend).unwrap();
            for &(source, nfc, nfd, nfkc, nfkd) in &examples {
                for (form, expected) in [("NFC", nfc), ("NFD", nfd), ("NFKC", nfkc), ("NFKD", nfkd)] {
                    let result = i.binding_call(&method, JsValue::str(source), vec![JsValue::str(form)]).unwrap();
                    assert_eq!(result, JsValue::str(expected), "{backend:?}: {form}, {source:?}");
                    assert_eq!(i.binding_call(&method, result.clone(), vec![JsValue::str(form)]).unwrap(), result);
                }
            }
            for (source, expected) in [
                (vec![0xd800, 0x00e9, 0xdc00], vec![0xd800, 0x0065, 0x0301, 0xdc00]),
                (vec![0xdc00, 0x0301, 0xd800], vec![0xdc00, 0x0301, 0xd800]),
                (vec![0xd800, 0xd800, 0xdc00], vec![0xd800, 0xd800, 0xdc00]),
            ] {
                assert_eq!(i.binding_call(&method, JsValue::str_units(source), vec![JsValue::str("NFD")]).unwrap(), JsValue::str_units(expected), "{backend:?}");
            }
            // Long descending-class segments exercise bounded stable ordering,
            // including leading marks, equal classes and an inert surrogate barrier.
            let mut source = Vec::new();
            let mut expected = Vec::new();
            for barrier in [None, Some(0x0041), Some(0xd800)] {
                if let Some(unit) = barrier { source.push(unit); expected.push(unit); }
                for _ in 0..2048 { source.extend([0x0315, 0x0300, 0x0301]); }
                for _ in 0..2048 { expected.extend([0x0300, 0x0301]); }
                expected.extend(std::iter::repeat_n(0x0315, 2048));
            }
            for form in ["NFD", "NFKD"] {
                assert_eq!(i.binding_call(&method, JsValue::str_units(source.clone()), vec![JsValue::str(form)]).unwrap(), JsValue::str_units(expected.clone()), "{backend:?}: stable long {form}");
            }

        }
    }

    #[test]
    fn generic_coercions_and_intrinsic_errors_preserve_order_and_identity() {
        let source = r#"
          function check(ok,label){if(!ok)throw label;return true;}var good=true,log=[],marker={};
          var receiver={[Symbol.toPrimitive](hint){log.push('source:'+hint);return 'e\u0301';}};
          var form={[Symbol.toPrimitive](hint){log.push('form:'+hint);return 'NFC';}};
          good=check(String.prototype.normalize.call(receiver,form)==='é' && log.join(',')==='source:string,form:string','coercion order');
          log=[];var failSource={toString(){log.push('source');throw marker;}};
          try{String.prototype.normalize.call(failSource,form);good=false;}catch(e){good=check(e===marker && log.join(',')==='source','source abrupt identity');}
          // Genuine user-created Errors with deliberately null prototypes
          // are propagated, rather than mapped like this builtin's fresh errors.
          for(var sourceError of [new Error('source'),new RangeError('source')]){
            Object.setPrototypeOf(sourceError,null);
            var throwSource={toString(){throw sourceError;}};
            try{String.prototype.normalize.call(throwSource,form);good=false;}catch(e){good=check(e===sourceError && Object.getPrototypeOf(e)===null,'custom null-prototype source Error');}
          }
          for(var formError of [new Error('form'),new RangeError('form')]){
            Object.setPrototypeOf(formError,null);
            var throwForm={toString(){throw formError;}};
            try{String.prototype.normalize.call(receiver,throwForm);good=false;}catch(e){good=check(e===formError && Object.getPrototypeOf(e)===null,'custom null-prototype form Error');}
          }
          log=[];var failForm={toString(){log.push('form');throw marker;}};
          try{String.prototype.normalize.call(receiver,failForm);good=false;}catch(e){good=check(e===marker && log.join(',')==='source:string,form','form abrupt identity');}
          for(var value of [null,undefined,Symbol('x')]){log=[];try{String.prototype.normalize.call(value,failForm);good=false;}catch(e){good=check(Object.getPrototypeOf(e)===TypeError.prototype && log.length===0,'invalid receiver before form');}}
          good=check('e\u0301'.normalize()==='é' && 'e\u0301'.normalize(undefined)==='é' && 'e\u0301'.normalize(new String('NFC'))==='é','default and boxed form');
          var Range=RangeError,rangeProto=RangeError.prototype,reads=0;
          Object.defineProperty(globalThis,'RangeError',{get(){reads++;throw marker;},configurable:true});
          for(var invalid of ['nfc','',123,{},'NFC\u0000']){try{'x'.normalize(invalid);good=false;}catch(e){good=check(Object.getPrototypeOf(e)===rangeProto,'intrinsic range error');}}
          good=check(reads===0,'mutable range constructor not observed');
          Object.defineProperty(globalThis,'RangeError',{value:Range,writable:true,configurable:true});
          good;
        "#;
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(source, backend).unwrap_or_else(|e|panic!("{backend:?}: {}",e.describe())), JsValue::Bool(true));
        }
    }

    #[test]
    fn borrowed_foreign_normalizers_throw_in_their_creation_realm_and_restore_caller() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let caller=i.binding_current_realm();
            let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for (name, source) in [("foreignNormalize","String.prototype.normalize"),("foreignRange","RangeError.prototype"),("foreignType","TypeError.prototype")] {
                let value=i.binding_with_realm(child,|i|i.run_script_with_backend(source,backend)).unwrap();
                i.global_env.declare_var(name,value).unwrap();
            }
            i.binding_eval_in_realm(child,"Object.defineProperty(globalThis,'RangeError',{get(){throw 'poison';},configurable:true});").unwrap();
            assert_eq!(i.run_script_with_backend(r#"
                var ok=true;for(var invoke of [()=>foreignNormalize.call('x','bad'),()=>Reflect.apply(foreignNormalize,'x',['bad']),()=>foreignNormalize.bind('x','bad')()]){
                  try{invoke();ok=false;}catch(e){ok=ok && Object.getPrototypeOf(e)===foreignRange;}
                }
                try{Reflect.apply(foreignNormalize,null,[]);ok=false;}catch(e){ok=ok && Object.getPrototypeOf(e)===foreignType;}
                ok && foreignNormalize.call('e\u0301','NFC')==='é';
            "#,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
            assert_eq!(i.binding_current_realm(),caller);assert_eq!(i.binding_incumbent_realm(),caller);
        }
    }
}
