use super::*;

fn run_method(ast: bool, kind: &str, body: &str, setup: &str, drive: &str) {
    let mut i=Interpreter::new();i.force_ast_callables=ast;
    let signature=match kind { "async"=>"async m()", "async-generator"=>"async *m()", _=>"*m()" };
    i.run_script(&format!(r#"
        var log=[],marker={{}},ok=false,key={{[Symbol.toPrimitive](){{log.push('convert');throw marker}}}};
        function B(){{}}class C extends B{{v=17;{signature}{{{body}}}}}var receiver=new C();
    "#)).unwrap();
    let JsValue::Object(method)=i.run_script("C.prototype.m").unwrap() else {panic!("method")};
    assert_eq!(matches!(method.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!ast);
    i.run_script(setup).unwrap();i.run_script(drive).unwrap();i.drain_microtasks().unwrap();
    assert_eq!(i.run_script("ok").unwrap(),JsValue::Bool(true),"AST={ast} {kind}: {:?}",i.run_script("JSON.stringify(log)").unwrap());
}

#[test]
fn super_raw_key_evaluation_and_abrupt_resume_precede_null_base_get_value() {
    let mut failures=Vec::new();
    for ast in [false,true] {for kind in ["generator","async","async-generator"] {
        for abrupt in [false,true] {
            let pause=if kind=="async" {"await Promise.resolve(key)"} else {"yield 'pause'"};
            let raw=if abrupt {format!("(log.push('key'),({pause}),(()=>{{throw marker}})())")}
                else {format!("(log.push('key'),({pause}))")};
            let expected=if abrupt {"e===marker"} else {"e instanceof TypeError"};
            let body=format!("try{{return super[{raw}](0)}}catch(e){{return {expected}&&log.join(',')==='key'}}");
            let drive=match kind {
                "generator"=>"var g=receiver.m(),r=g.next();if(!r.done)r=g.next(key);ok=r.done&&r.value",
                "async"=>"receiver.m().then(v=>ok=v)",
                _=>"var g=receiver.m();g.next().then(r=>r.done?r:g.next(key)).then(r=>ok=r.done&&r.value)",
            };
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||run_method(ast,kind,&body,"Object.setPrototypeOf(C.prototype,null)",drive)));
            if result.is_err(){failures.push(format!("AST={ast} {kind} abrupt={abrupt}"));}
        }
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn super_base_is_read_after_parked_key_and_nested_receiver_remains_rooted() {
    let mut failures=Vec::new();
    for ast in [false,true] {for kind in ["generator","async","async-generator"] {
        for nested in [false,true] {
            let pause=if kind=="async" {"await pending"} else {"yield 'pause'"};
            let tail=if nested {".m(2)===25&&log.join(',')==='get:17,call:23'"}
                else {"(2)===19&&log.join(',')==='get:17,call:17'"};
            let body=format!("return super[{pause}]{tail}");
            let property=if nested {"box"} else {"m"};
            let setup=format!(r#"
                var resolve,pending=new Promise(r=>resolve=r),replacement={{}};
                Object.defineProperty(B.prototype,'{property}',{{get(){{log.push('old');return {{m(){{return false}}}}}}}});
                Object.defineProperty(replacement,'{property}',{{get(){{log.push('get:'+this.v);
                    {returned}
                }}}});
            "#,returned=if nested {"return {v:23,m(x){log.push('call:'+this.v);return this.v+x}}"}
                else {"return function(x){log.push('call:'+this.v);return this.v+x}"});
            let drive=match kind {
                "generator"=>format!("var g=receiver.m(),r=g.next();Object.setPrototypeOf(C.prototype,replacement);if(!r.done)r=g.next('{property}');ok=r.done&&r.value"),
                "async"=>format!("receiver.m().then(v=>ok=v);Object.setPrototypeOf(C.prototype,replacement);resolve('{property}')"),
                _=>format!("var g=receiver.m();g.next().then(r=>{{Object.setPrototypeOf(C.prototype,replacement);return r.done?r:g.next('{property}')}}).then(r=>ok=r.done&&r.value)"),
            };
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||run_method(ast,kind,&body,&setup,&drive)));
            if result.is_err(){failures.push(format!("AST={ast} {kind} nested={nested}"));}
        }
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn captured_derived_this_tdz_precedes_super_home_and_computed_expression() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var log=[],marker={},captured,ok=false;function B(){}
            class C extends B{constructor(){captured=async()=>{
                try{return super[(log.push('key'),await 0)](1)}
                catch(e){return e instanceof ReferenceError&&log.length===0}
            };throw marker}}
            try{new C()}catch(e){if(e!==marker)throw e}
            Object.setPrototypeOf(C.prototype,null);
        "#).unwrap();
        let JsValue::Object(arrow)=i.run_script("captured").unwrap() else {panic!("arrow")};
        assert_eq!(matches!(arrow.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!ast);
        i.run_script("captured().then(v=>ok=v)").unwrap();i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok").unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}

#[test]
fn native_suspended_super_read_consumers_dispatch_through_rooted_reference_chain() {
    for kind in ["generator", "async", "async-generator"] {
        let signature = match kind { "async" => "async m()", "async-generator" => "async *m()", _ => "*m()" };
        let pause = if kind == "async" { "await 'key'" } else { "yield 'key'" };
        for expression in [
            format!("super[{pause}](1)"),
            format!("(super[{pause}])(1)"),
            format!("super[{pause}].method(1)"),
            format!("(super[{pause}])"),
        ] {
            let mut i = Interpreter::new();
            i.run_script(&format!("function B(){{}} class C extends B {{{signature}{{return {expression}}}}}")).unwrap();
            let JsValue::Object(method) = i.run_script("C.prototype.m").unwrap() else { panic!("method") };
            let method = method.borrow();
            let FunctionBody::Bytecode(code) = &method.func.as_ref().unwrap().body else {
                panic!("actual native body required: {kind} {expression}")
            };
            let ops = &code.0.program.code;
            let this = ops.iter().position(|op| matches!(op, Instruction::LoadThis(_))).expect("capture actual this");
            let pause = ops.iter().position(|op| matches!(op, Instruction::Yield(..) | Instruction::Await(..))).expect("raw key suspension");
            let home = ops.iter().position(|op| matches!(op, Instruction::LoadSuperHome(_))).expect("current Super base");
            let key = ops.iter().position(|op| matches!(op, Instruction::ToKey(_))).expect("key conversion");
            let read = ops.iter().position(|op| matches!(op, Instruction::GetMemberReceiver(..))).expect("actual receiver read");
            assert!(this < pause && pause < home && home < key && key < read, "{kind} {expression}");
            assert!(!ops.iter().any(|op| matches!(op, Instruction::TreeEval(..))), "Super read must use native reference chain: {kind} {expression}");
        }
    }
}
