use amazing_js::{Interpreter, ast::{ArrowBody, ClassMember, ExprKind, StmtKind}, interp::ExecutionBackend, value::{FunctionBody, JsValue}};

fn matrix(signature: &str, body: &str, setup: &str, drive: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
            let mut i = Interpreter::new();
            i.run_script_with_backend("var ok=false,log=[],marker={};function tag(s){return s}", backend).unwrap();
            let source = format!("{signature} {{ {body} }}");
            i.run_script_with_backend(&source, backend).unwrap();
            let JsValue::Object(function) = i.run_script("make").unwrap() else { panic!("function") };
            let native = matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_));
            if !native && !actor {
                failures.push(format!("{backend:?} native-eligibility unavailable: {source}"));
                continue;
            }
            if actor && native {
                let parsed = amazing_js::parse_script(&source).unwrap();
                let StmtKind::Function(function_ast) = &parsed.body[0].kind else { panic!("function") };
                function.borrow_mut().func.as_mut().unwrap().body = FunctionBody::Block(function_ast.body.clone());
            }
            i.run_script_with_backend(setup, backend).unwrap();
            let result = i.run_script_with_backend(drive, backend).and_then(|_| i.drain_microtasks()).and_then(|_| i.run_script("ok"));
            if !matches!(&result, Ok(JsValue::Bool(true))) { failures.push(format!("{backend:?} actor={actor}: {source}: {result:?}")); }
        }
    }
    failures
}

#[test]
fn cloned_programs_preserve_site_identity_but_fresh_parses_are_distinct() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.run_script("function tag(s){return s}").unwrap();
        let program = amazing_js::parse_script("tag`same${1}site`").unwrap();
        let cloned = program.clone();
        let fresh = amazing_js::parse_script("tag`same${1}site`").unwrap();
        assert_eq!(program, cloned);
        assert_eq!(program, fresh, "semantic AST equality ignores parse identity");
        let first = i.run_program_with_backend(&program, backend).unwrap();
        assert_eq!(first, i.run_program_with_backend(&cloned, backend).unwrap());
        assert_ne!(first, i.run_program_with_backend(&fresh, backend).unwrap());
        assert_eq!(first, i.run_program_with_backend(&program.clone(), backend).unwrap());
    }
}

#[test]
fn repeated_functions_share_sites_and_independent_sites_do_not_alias() {
    let mut failures = Vec::new();
    failures.extend(matrix("function make()", "return tag`same${1}site`", "", "var first=make();ok=first===make()&&first===make()"));
    failures.extend(matrix("function make()", "return [tag`same`,tag`same`]", "", "var first=make(),second=make();ok=first[0]!==first[1]&&first[0]===second[0]&&first[1]===second[1]"));
    failures.extend(matrix("function make()", "return [eval('tag`same`'),eval('tag`same`')]", "", "var pair=make();ok=pair[0]!==pair[1]"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn worker_generators_and_async_functions_retain_sites_across_suspension() {
    let mut failures = Vec::new();
    failures.extend(matrix("function* make()", "for(var n=0;n<2;n++)yield tag`worker`", "", "var g=make(),a=g.next().value,b=g.next().value,c=make().next().value;ok=a===b&&a===c"));
    failures.extend(matrix("async function make()", "return tag`worker${await Promise.resolve(1)}`", "", "make().then(a=>make().then(b=>ok=a===b))"));
    failures.extend(matrix("async function* make()", "for(var n=0;n<2;n++)yield tag`worker${await Promise.resolve(n)}`", "", "var g=make();g.next().then(a=>g.next().then(b=>make().next().then(c=>ok=a.value===b.value&&a.value===c.value)))"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn substituted_yield_await_values_are_raw_and_tag_reference_precedes_arguments() {
    let mut failures = Vec::new();
    failures.extend(matrix("function* make()", "return object.tag`a${yield 'value'}b`", "var object={get tag(){log.push('get');return function(s,v){log.push('call');return this===object&&v===marker&&s.raw.join('|')==='a|b'}}};", "var g=make(),first=g.next();var last=g.next(marker);ok=first.value==='value'&&last.done&&last.value&&log.join(',')==='get,call'"));
    failures.extend(matrix("async function make()", "return (object.tag)`a${await Promise.resolve(marker)}b`", "var object={get tag(){log.push('get');return function(s,v){log.push('call');return this===object&&v===marker}}};", "make().then(v=>ok=v&&log.join(',')==='get,call')"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn raw_and_cooked_utf16_invalid_escapes_and_descriptors_stay_frozen() {
    let mut failures = Vec::new();
    failures.extend(matrix("function make()", r"return tag`\uD800${1}\u{110000}`", "", "var s=make(),raw=Object.getOwnPropertyDescriptor(s,'raw'),part=Object.getOwnPropertyDescriptor(s,'0');ok=s[0].charCodeAt(0)===0xd800&&s[1]===undefined&&s.raw[0]==='\\\\uD800'&&s.raw[1]==='\\\\u{110000}'&&Object.isFrozen(s)&&Object.isFrozen(s.raw)&&!raw.writable&&!raw.enumerable&&!raw.configurable&&!part.writable&&part.enumerable&&!part.configurable"));
    failures.extend(matrix("function make()", r"return tag`\xZ`", "", "var s=make();ok=s.length===1&&s[0]===undefined&&s.raw[0]==='\\\\xZ'&&Object.isFrozen(s)&&Object.isFrozen(s.raw)"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn tag_and_substitution_errors_preserve_identity_and_noncallable_timing() {
    let mut failures = Vec::new();
    failures.extend(matrix("async function make()", "return object.tag`a${await Promise.resolve((log.push('argument'),1))}`", "var object={get tag(){log.push('get');throw marker}};", "make().then(()=>{},e=>ok=e===marker&&log.join(',')==='get')"));
    failures.extend(matrix("async function make()", "return object.tag`a${await Promise.reject(marker)}`", "var object={get tag(){log.push('get');return function(){log.push('call')}}};", "make().then(()=>{},e=>ok=e===marker&&log.join(',')==='get')"));
    failures.extend(matrix("function* make()", "return object.tag`a${yield 'argument'}`", "var object={get tag(){log.push('get');return null}};", "var g=make(),first=g.next();try{g.next(marker)}catch(e){ok=e instanceof TypeError&&first.value==='argument'&&log.join(',')==='get'}"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn same_parsed_site_uses_distinct_actual_realm_registries_and_intrinsics() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        for realm in [parent, child] { i.binding_eval_in_realm(realm,"var result;function tag(s){return s}").unwrap(); }
        let program = amazing_js::parse_script("result=tag`realm`").unwrap();
        let first = i.binding_with_realm(parent, |i|i.run_program_with_backend(&program,backend)).unwrap();
        let second = i.binding_with_realm(child, |i|i.run_program_with_backend(&program.clone(),backend)).unwrap();
        assert_ne!(first,second);
        for realm in [parent,child] {
            assert_eq!(i.binding_eval_in_realm(realm,"Object.getPrototypeOf(result)===Array.prototype&&Object.getPrototypeOf(result.raw)===Array.prototype").unwrap(),JsValue::Bool(true));
        }
        assert_eq!(first,i.binding_with_realm(parent, |i|i.run_program_with_backend(&program.clone(),backend)).unwrap());
        assert_eq!(second,i.binding_with_realm(child, |i|i.run_program_with_backend(&program,backend)).unwrap());
    }
}

#[test]
fn foreign_async_body_templates_keep_original_realm_across_borrowed_calls() {
    let mut failures = Vec::new();
    let source = "var seen=[];function tag(s){return s}async function make(){return tag`foreign${await Promise.resolve(1)}`}function run(){return make().then(v=>seen.push(v))}";
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false,true] {
            let mut i = Interpreter::new();
            let parent = i.binding_current_realm();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            i.binding_with_realm(child, |i|i.run_script_with_backend(source,backend)).unwrap();
            let JsValue::Object(make) = i.binding_eval_in_realm(child,"make").unwrap() else { panic!("make") };
            let native = matches!(make.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_));
            if !native && !actor { failures.push(format!("{backend:?} native foreign eligibility unavailable"));continue; }
            if actor && native {
                let program = amazing_js::parse_script(source).unwrap();
                let function_ast = program.body.iter().find_map(|s|match &s.kind {StmtKind::Function(f) if f.name.as_ref().unwrap().name=="make"=>Some(f),_=>None}).unwrap();
                make.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(function_ast.body.clone());
            }
            let run=i.binding_eval_in_realm(child,"run").unwrap();
            for _ in 0..2 { i.binding_call_in_realm(parent,&run,JsValue::Undefined,vec![]).unwrap();i.drain_microtasks().unwrap(); }
            let result=i.binding_eval_in_realm(child,"seen.length===2&&seen[0]===seen[1]&&Object.getPrototypeOf(seen[0])===Array.prototype&&Object.getPrototypeOf(seen[0].raw)===Array.prototype");
            if !matches!(&result,Ok(JsValue::Bool(true))) {failures.push(format!("{backend:?} actor={actor}: {result:?}"));}
        }
    }
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}


fn super_matrix(signature: &str, body: &str, setup: &str, drive: &str) -> Vec<String> {
    let mut failures=Vec::new();
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        for actor in [false,true] {
            let mut i=Interpreter::new();
            i.run_script("var ok=false,log=[],marker={},key={ [Symbol.toPrimitive](){log.push('key');return 'tag'}};function Base(){}").unwrap();
            let source=format!("class C extends Base {{{signature} {{{body}}}}}");
            i.run_script_with_backend(&source,backend).unwrap();
            let JsValue::Object(function)=i.run_script("C.prototype.m").unwrap() else {panic!("method")};
            let native=matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_));
            if !native&&!actor {failures.push(format!("{backend:?} native super-tag eligibility unavailable: {source}"));continue;}
            if actor&&native {
                let program=amazing_js::parse_script(&source).unwrap();
                let StmtKind::Class(class)=&program.body[0].kind else {panic!("class")};
                let ClassMember::Method(method)=&class.members[0] else {panic!("method")};
                function.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(method.body.clone());
            }
            i.run_script_with_backend(setup,backend).unwrap();
            let result=i.run_script_with_backend(drive,backend).and_then(|_|i.drain_microtasks()).and_then(|_|i.run_script("ok"));
            if !matches!(&result,Ok(JsValue::Bool(true))) {failures.push(format!("{backend:?} actor={actor}: {source}: {result:?}"));}
        }
    }
    failures
}

#[test]
fn super_tags_capture_raw_expression_before_null_base_and_before_arguments() {
    let mut failures=Vec::new();
    failures.extend(super_matrix("m()", "try{return super[(log.push('expression'),key)]`head${log.push('argument')}`}catch(e){return e instanceof TypeError&&log.join(',')==='expression'}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "ok=receiver.m()"));
    failures.extend(super_matrix("m()", "try{return super[(()=>{log.push('expression');throw marker})()]`head${log.push('argument')}`}catch(e){return e===marker&&log.join(',')==='expression'}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "ok=receiver.m()"));
    failures.extend(super_matrix("async m()", "try{return super[await Promise.resolve((log.push('expression'),key))]`head${await Promise.resolve(log.push('argument'))}`}catch(e){return e instanceof TypeError&&log.join(',')==='expression'}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "receiver.m().then(v=>ok=v)"));
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn parked_super_tag_key_observes_current_prototype_and_retains_receiver() {
    let mut failures=Vec::new();
    failures.extend(super_matrix("*m()", "return super[yield 'key']`head${(log.push('argument'),marker)}`", "var receiver=new C();", "var g=receiver.m(),first=g.next();Object.setPrototypeOf(C.prototype,{get tag(){log.push('get');return function(s,v){log.push('call');return this===receiver&&v===marker}}});var last=g.next(key);ok=first.value==='key'&&last.done&&last.value&&log.join(',')==='key,get,argument,call'"));
    failures.extend(super_matrix("async m()", "return super[await Promise.resolve(key)]`head${await Promise.resolve(marker)}`", "var receiver=new C();Object.setPrototypeOf(C.prototype,{get tag(){log.push('get');return function(s,v){log.push('call');return this===receiver&&v===marker}}});", "receiver.m().then(v=>ok=v&&log.join(',')==='key,get,call')"));
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn native_and_actor_super_tag_arrows_keep_captured_derived_this_tdz() {
    let mut failures=Vec::new();
    let source="var captured;class C extends Base{constructor(){captured=()=>{try{return super[(log.push('expression'),key)]`head${log.push('argument')}`}catch(e){return e instanceof ReferenceError&&log.length===0}};throw marker}}try{new C()}catch(e){if(e!==marker)throw e}";
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        for actor in [false,true] {
            let mut i=Interpreter::new();
            i.run_script("var log=[],marker={},key={toString(){throw marker}};function Base(){}").unwrap();
            i.run_script_with_backend(source,backend).unwrap();
            let JsValue::Object(function)=i.run_script("captured").unwrap() else {panic!("arrow")};
            assert!(matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),"native arrow required");
            if actor {
                let program=amazing_js::parse_script(source).unwrap();
                let class=program.body.iter().find_map(|s|match &s.kind {StmtKind::Class(c)=>Some(c),_=>None}).unwrap();
                let ClassMember::Method(method)=&class.members[0] else {panic!("method")};
                let StmtKind::Expression(expression)=&method.body.body[0].kind else {panic!("assignment")};
                let ExprKind::Assign{value,..}=&expression.kind else {panic!("assignment")};
                let ExprKind::Arrow(arrow)=&value.kind else {panic!("arrow")};
                let ArrowBody::Block(block)=&arrow.body else {panic!("block")};
                function.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(block.clone());
            }
            let result=i.run_script_with_backend("captured()",backend);
            if !matches!(&result,Ok(JsValue::Bool(true))) {failures.push(format!("{backend:?} actor={actor}: {result:?}"));}
        }
    }
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn parameter_and_class_initializer_sites_survive_repeated_instantiation() {
    let failures=matrix("function make(value=tag`parameter`)","return value","","var a=make();ok=a===make()&&a===make(undefined)&&make(marker)===marker");
    assert!(failures.is_empty(),"{}",failures.join("\n"));
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut i=Interpreter::new();
        let source="function tag(s){return s}class C{field=tag`field`;static field=tag`field`};var a=new C(),b=new C();a.field===b.field&&a.field!==C.field";
        assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true));
    }
}
