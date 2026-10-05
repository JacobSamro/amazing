use super::*;

fn check(body: &str, setup: &str, expected: &str) {
    for ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = ast;
        let source = format!("var log=[],token={{}},value;class B{{set x(v){{log.push('set');this.value=v}}}};class C extends B{{constructor(){{{body}}}}};{setup}");
        i.run_script(&source).unwrap();
        let JsValue::Object(c) = i.run_script("C").unwrap() else { panic!("constructor") };
        let c = c.borrow();
        match &c.func.as_ref().unwrap().body {
            FunctionBody::Bytecode(code) if !ast => {
                assert!(code.0.program.code.iter().any(|op| matches!(op,
                    Instruction::ResolveSuperMemberWrite(..) | Instruction::ResolveRawSuperMemberWrite(..))));
            }
            FunctionBody::Block(_) if ast => {},
            other => panic!("genuine constructor body AST={ast}: {other:?}"),
        }
        drop(c);
        assert_eq!(i.run_script(expected).unwrap(), JsValue::Bool(true), "AST={ast}: {body}");
    }
}

#[test]
fn super_patterns_static_and_computed_array_object_nested_rest_default_receiver() {
    for pattern in [
        "[super.x]=[7]", "[super['x']]=[7]", "({a:super.x}={a:7})",
        "({a:super['x']}={a:7})", "[[super.x]]=[[7]]", "({a:[super.x]}={a:[7]})",
        "[{a:super.x}]=[{a:7}]", "({a:{b:super.x}}={a:{b:7}})",
        "[super.x=7]=[undefined]", "({a:super.x=7}={})",
    ] {
        check(&format!("super();{pattern};"), "value=new C", "value.value===7&&log.join(',')==='set'&&!Object.hasOwn(B.prototype,'value')");
    }
    for pattern in ["[...super.x]=[7,8]", "({...super.x}={a:7,b:8})", "[...{0:super.x}]=[7]"] {
        check(&format!("super();{pattern};"), "value=new C", "log.join(',')==='set'&&!Object.hasOwn(B.prototype,'value')&&(value.value===7||value.value[0]===7||value.value.a===7)");
    }
}

#[test]
fn super_patterns_raw_key_captured_before_value_and_coerced_at_put() {
    for pattern in [
        "[super[(log.push('key'),key)]]=[undefined]",
        "[super[(log.push('key'),key)]=(log.push('default'),7)]=[undefined]",
        "({a:super[(log.push('key'),key)]}={get a(){log.push('get');return 7}})",
    ] {
        let expected = if pattern.contains("default") { "key,default,coerce,set" } else if pattern.starts_with('(') { "key,get,coerce,set" } else { "key,coerce,set" };
        check(&format!("super();var key={{toString(){{log.push('coerce');return 'x'}}}};{pattern};"), "value=new C", &format!("log.join(',')==='{expected}'"));
    }
}

#[test]
fn super_patterns_capture_actual_this_before_computed_expression() {
    check("[super[(log.push('key'),'x')]]=[1];", "try{new C}catch(e){value=e instanceof ReferenceError}", "value===true&&log.length===0");
}

#[test]
fn super_patterns_null_home_preserves_raw_reference_until_put_value() {
    check("super();Object.setPrototypeOf(C.prototype,null);({a:super[(log.push('key'),{toString(){log.push('coerce');return 'x'}})]}={get a(){log.push('get');return 1}});", "try{new C}catch(e){value=e instanceof TypeError}", "value===true&&log.join(',')==='key,get'");
}

#[test]
fn super_patterns_key_expression_mutation_selects_live_home_prototype() {
    check("super();var base={set x(v){log.push('other');this.value=v}};[super[(Object.setPrototypeOf(C.prototype,base),'x')]]=[7];", "value=new C", "value.value===7&&log.join(',')==='other'");
}

#[test]
fn super_patterns_key_conversion_and_rhs_abrupt_preserve_original_identity() {
    check("super();[super[{toString(){log.push('coerce');throw token}}]]=[7];", "try{new C}catch(e){value=e===token}", "value===true&&log.join(',')==='coerce'");
    check("super();({a:super[(log.push('key'),{toString(){log.push('coerce');return 'x'}})]}={get a(){throw token}});", "try{new C}catch(e){value=e===token}", "value===true&&log.join(',')==='key'");
}

#[test]
fn super_patterns_iterator_target_capture_precedes_step_and_close() {
    check("super();var it={next(){log.push('next');return {value:7,done:false}},return(){log.push('close');return {}}};[super[(log.push('key'),'x')]]={[Symbol.iterator](){log.push('iterator');return it}};", "value=new C", "value.value===7&&log.join(',')==='iterator,key,next,set,close'");
}

#[test]
fn super_patterns_suspended_keys_preserve_method_receiver_across_all_callable_kinds() {
    let cases = [
        ("m(){[super.x]=[7]}", "var c=new C;c.m();done=true"),
        ("*m(){[super[(yield 1,'x')]]=[7]}", "var c=new C,g=c.m();g.next();g.next();done=true"),
        ("async m(){[super[(await 1,'x')]]=[7]}", "var c=new C;c.m().then(()=>done=true)"),
        ("async *m(){[super[(yield 1,'x')]]=[7]}", "var c=new C,g=c.m();g.next().then(()=>g.next()).then(()=>done=true)"),
    ];
    for ast in [false,true] {
        for (method,driver) in cases {
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            i.run_script(&format!("var done=false;class B{{set x(v){{this.value=v}}}};class C extends B{{{method}}};var f=C.prototype.m")).unwrap();
            let JsValue::Object(f)=i.run_script("f").unwrap() else {panic!("method")};
            match &f.borrow().func.as_ref().unwrap().body {
                FunctionBody::Bytecode(code) if !ast => assert!(code.0.program.code.iter().any(|op| matches!(op,Instruction::ResolveSuperMemberWrite(..)|Instruction::ResolveRawSuperMemberWrite(..)))),
                FunctionBody::Block(_) if ast => {},
                other=>panic!("actual method AST={ast}: {other:?}"),
            }
            i.run_script(driver).unwrap();i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("done&&c.value===7&&!Object.hasOwn(B.prototype,'value')").unwrap(),JsValue::Bool(true),"AST={ast}: {method}");
        }
    }
}
