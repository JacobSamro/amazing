use amazing_js::{Interpreter, ast::{ArrowBody, ClassMember, ExprKind, ObjElem, PropValue, StmtKind}, interp::ExecutionBackend,
    value::{FunctionBody, JsValue}};

fn matrix(signature: &str, body: &str, setup: &str, drive: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        // Every current explicit constructor and method uses its genuine
        // native factory as well as the original forced AST body.
        let modes: &[bool] = &[false, true];
        for &actor in modes {
            let mut i = Interpreter::new();
            i.run_script_with_backend("var ok=false, log=[], marker={}, key={ [Symbol.toPrimitive](){log.push('convert');throw marker} };function Base(){}", backend).unwrap();
            let source = format!("class C extends Base {{ {signature} {{ {body} }} }}");
            i.run_script_with_backend(&source, backend).unwrap();
            let constructor = signature == "constructor()";
            let target = if constructor { "C" } else if signature.starts_with("static ") { "C.m" } else { "C.prototype.m" };
            let JsValue::Object(function) = i.run_script(target).unwrap() else { panic!("method") };
            let native = matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_));
            if !native && !actor {
                failures.push(format!("{backend:?} native-eligibility unavailable: {signature} {body}"));
                continue;
            }
            if actor && native {
                let parsed = amazing_js::parse_script(&source).unwrap();
                let StmtKind::Class(class) = &parsed.body[0].kind else { panic!("class") };
                let ClassMember::Method(method) = &class.members[0] else { panic!("method") };
                function.borrow_mut().func.as_mut().unwrap().body = FunctionBody::Block(method.body.clone());
            }
            i.run_script_with_backend(setup, backend).unwrap();
            let result = i.run_script_with_backend(drive, backend).and_then(|_|i.drain_microtasks()).and_then(|_|i.run_script("ok"));
            if !matches!(&result, Ok(JsValue::Bool(true))) { failures.push(format!("{backend:?} actor={actor}: {signature} {body}: {result:?}")); }
        }
    }
    failures
}

#[test]
fn direct_super_delete_rejects_before_key_conversion_and_null_base_coercion() {
    let mut failures = Vec::new();
    for expression in ["super.x", "super[key]", "((super[key]))"] {
        let body = format!("try{{delete {expression}}}catch(e){{return e instanceof ReferenceError&&log.length===0}}return false");
        for setup in ["var receiver=new C();", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "var receiver=new C();Object.setPrototypeOf(C.prototype,new Proxy({}, {get(){throw marker},deleteProperty(){throw marker},getPrototypeOf(){throw marker}}));"] {
            failures.extend(matrix("m()", &body, setup, "ok=receiver.m()"));
            failures.extend(matrix("m()", &body, setup, "ok=C.prototype.m.call(undefined)&&C.prototype.m.call(0)"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn computed_expression_evaluates_once_and_abrupt_identity_precedes_rejection() {
    let mut failures = Vec::new();
    failures.extend(matrix("m()", "try{delete super[(log.push('expression'),key)]}catch(e){return e instanceof ReferenceError&&log.join(',')==='expression'}", "var receiver=new C();", "ok=receiver.m()"));
    failures.extend(matrix("m()", "try{delete super[(()=>{log.push('expression');throw marker})()]}catch(e){return e===marker&&log.join(',')==='expression'}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "ok=receiver.m()"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn derived_this_tdz_precedes_computed_expression() {
    let mut failures = Vec::new();
    failures.extend(matrix("constructor()", "var valid=false;try{delete super[(log.push('expression'),key)]}catch(e){valid=e instanceof ReferenceError&&log.length===0}super();this.ok=valid", "", "ok=new C().ok"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn lexical_arrow_and_direct_eval_keep_super_reference_without_conversion() {
    let mut failures = Vec::new();
    failures.extend(matrix("m()", "var arrow=()=>delete super[key];var first=false;try{arrow()}catch(e){first=e instanceof ReferenceError}try{eval('delete ((super[key]))')}catch(e){return first&&e instanceof ReferenceError&&log.length===0}", "var receiver=new C();", "ok=receiver.m()"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn ordinary_prefixes_comma_and_optional_delete_keep_get_value_semantics() {
    let mut failures = Vec::new();
    failures.extend(matrix("m()", "var one=delete super.box.x;var two=delete (super.box).y;var three=delete (0,super.box);var four=delete super.missing?.x;return one&&two&&three&&four&&log.join(',')==='get,get,get'", "var receiver=new C();Object.defineProperty(Base.prototype,'box',{get(){log.push('get');return {x:1,y:2}},configurable:true});", "ok=receiver.m()"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn ordinary_delete_retains_strict_proxy_traps_and_optional_short_circuit() {
    let mut failures = Vec::new();
    failures.extend(matrix("m()", "var object=new Proxy({x:1},{deleteProperty(t,k){log.push(k);return false}});var strict=false;try{delete object.x}catch(e){strict=e instanceof TypeError}var missing=null;var short=delete missing?.[key];return strict&&short&&log.join(',')==='x'", "var receiver=new C();", "ok=receiver.m()"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn async_computed_expression_awaits_before_rejecting_without_key_conversion() {
    let mut failures = Vec::new();
    failures.extend(matrix("async m()", "try{delete super[await Promise.resolve(key)]}catch(e){return e instanceof ReferenceError&&log.length===0}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "receiver.m().then(v=>ok=v)"));
    failures.extend(matrix("async m()", "try{delete super[await Promise.reject(marker)]}catch(e){return e===marker&&log.length===0}", "var receiver=new C();", "receiver.m().then(v=>ok=v)"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn sync_and_async_generator_keys_retain_this_and_raw_value_across_yield() {
    let mut failures = Vec::new();
    failures.extend(matrix("*m()", "try{delete super[yield 'key']}catch(e){return e instanceof ReferenceError&&log.length===0}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "var g=receiver.m();var first=g.next();var last=g.next(key);ok=first.value==='key'&&last.done&&last.value"));
    failures.extend(matrix("async *m()", "try{delete super[yield 'key']}catch(e){return e instanceof ReferenceError&&log.length===0}", "var receiver=new C();Object.setPrototypeOf(C.prototype,null);", "var g=receiver.m();g.next().then(first=>g.next(key).then(last=>ok=first.value==='key'&&last.done&&last.value))"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn native_and_ast_arrows_keep_captured_derived_this_tdz_after_constructor_unwind() {
    let mut failures = Vec::new();
    let source = "var captured;class C extends Base {constructor(){captured=()=>{try{delete super[(log.push('expression'),key)]}catch(e){return e instanceof ReferenceError&&log.length===0}};throw marker}}try{new C()}catch(e){if(e!==marker)throw e}";
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
            let mut i = Interpreter::new();
            i.run_script("var log=[], marker={},key={toString(){throw marker}};function Base(){}").unwrap();
            i.run_script_with_backend(source, backend).unwrap();
            let JsValue::Object(function) = i.run_script("captured").unwrap() else { panic!("arrow") };
            assert!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), "actual native arrow required");
            if actor {
                let parsed = amazing_js::parse_script(source).unwrap();
                let class = parsed.body.iter().find_map(|statement| match &statement.kind { StmtKind::Class(class) => Some(class), _ => None }).unwrap();
                let ClassMember::Method(method) = &class.members[0] else { panic!("constructor") };
                let StmtKind::Expression(expression) = &method.body.body[0].kind else { panic!("assignment") };
                let ExprKind::Assign { value, .. } = &expression.kind else { panic!("assignment") };
                let ExprKind::Arrow(arrow) = &value.kind else { panic!("arrow") };
                let ArrowBody::Block(block) = &arrow.body else { panic!("block") };
                function.borrow_mut().func.as_mut().unwrap().body = FunctionBody::Block(block.clone());
            }
            let result = i.run_script_with_backend("captured()", backend);
            if !matches!(&result, Ok(JsValue::Bool(true))) { failures.push(format!("{backend:?} actor={actor}: {result:?}")); }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn static_home_objects_reject_raw_super_delete_after_suspension() {
    let mut failures = Vec::new();
    failures.extend(matrix("static m()", "try{delete super[(log.push('expression'),key)]}catch(e){return e instanceof ReferenceError&&log.join(',')==='expression'}", "Object.setPrototypeOf(C,null);", "ok=C.m()"));
    failures.extend(matrix("static async m()", "try{delete super[await Promise.resolve(key)]}catch(e){return e instanceof ReferenceError&&log.length===0}", "Object.setPrototypeOf(C,new Proxy({}, {get(){throw marker},deleteProperty(){throw marker}}));", "C.m().then(v=>ok=v)"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn object_literal_home_objects_keep_raw_references_and_sloppy_delete_behavior() {
    let mut failures = Vec::new();
    let source = "var object={m(){try{delete ((super[(log.push('expression'),key)]))}catch(e){return e instanceof ReferenceError&&log.join(',')==='expression'}}}";
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
            let mut i = Interpreter::new();
            i.run_script("var log=[],marker={},key={toString(){throw marker}}").unwrap();
            i.run_script_with_backend(source, backend).unwrap();
            let JsValue::Object(function) = i.run_script("object.m").unwrap() else { panic!("method") };
            assert!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), "actual native object method required");
            if actor {
                let parsed = amazing_js::parse_script(source).unwrap();
                let StmtKind::Var(declaration) = &parsed.body[0].kind else { panic!("declaration") };
                let ExprKind::Object(elements) = &declaration.decls[0].init.as_ref().unwrap().kind else { panic!("object") };
                let ObjElem::Prop(property) = &elements[0] else { panic!("property") };
                let PropValue::Method(method) = &property.value else { panic!("method") };
                function.borrow_mut().func.as_mut().unwrap().body = FunctionBody::Block(method.body.clone());
            }
            i.run_script("Object.setPrototypeOf(object,new Proxy({}, {get(){throw marker},deleteProperty(){throw marker}}))").unwrap();
            let result = i.run_script_with_backend("object.m.call(undefined)", backend);
            if !matches!(&result, Ok(JsValue::Bool(true))) { failures.push(format!("{backend:?} actor={actor}: {result:?}")); }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
