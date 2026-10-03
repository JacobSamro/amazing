use super::*;

fn realm_factory(ast: bool, source: &str) -> Interpreter {
    let mut i=Interpreter::new();i.force_ast_callables=ast;
    let child=i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();
    let f=i.binding_eval_in_realm(child,source).unwrap();
    let JsValue::Object(function)=&f else{panic!("factory")};
    assert_eq!(matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!ast);
    i.global_env.declare_var("f",f).unwrap();
    for (name,source) in [("childArray","Array.prototype"),("childObject","Object.prototype"),
        ("childFunction","Function.prototype"),("childReferenceError","ReferenceError.prototype")] {
        let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
    }
    i.run_script("var methods=Object.getPrototypeOf(Object.getPrototypeOf((function*(){})()));var next=methods.next,ret=methods.return,thr=methods.throw;var marker={};").unwrap();
    i
}

#[test]
fn borrowed_sync_next_preserves_body_yield_and_completion_realms() {
    for ast in [false,true] {
        let mut i=realm_factory(ast,"(function*(){yield [];yield class C{[(yield 'key','x')]=[];#p=3;read(){return this.#p}};return []})");
        assert_eq!(i.run_script(r#"
            var g=f(),first=next.call(g),key=next.call(g),middle=next.call(g),last=next.call(g),after=next.call(g);
            var C=middle.value,v=new C;
            Object.getPrototypeOf(first)===childObject&&Object.getPrototypeOf(first.value)===childArray&&
            Object.getPrototypeOf(key)===childObject&&key.value==='key'&&
            Object.getPrototypeOf(middle)===childObject&&Object.getPrototypeOf(C)===childFunction&&
            Object.getPrototypeOf(C.prototype)===childObject&&Object.getPrototypeOf(v.x)===childArray&&v.read()===3&&
            last.done&&Object.getPrototypeOf(last.value)===childArray&&Object.getPrototypeOf(last)===Object.prototype&&
            after.done&&after.value===undefined&&Object.getPrototypeOf(after)===Object.prototype;
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}

#[test]
fn borrowed_sync_abrupt_methods_keep_caller_results_and_body_finalizers() {
    for ast in [false,true] {
        let mut i=realm_factory(ast,"(function*(){with({x:[]}){try{yield x;class C{[(yield 'key','x')]=1}}finally{yield []}}return []})");
        assert_eq!(i.run_script(r#"
            var g=f(),one=next.call(g),key=next.call(g),fin=ret.call(g,marker),done=next.call(g);
            var start=f(),initial=ret.call(start,marker),completed=ret.call(start,marker),identity=false;
            try{thr.call(start,marker)}catch(e){identity=e===marker}
            var wrong=false;try{next.call({})}catch(e){wrong=Object.getPrototypeOf(e)===TypeError.prototype}
            Object.getPrototypeOf(one)===childObject&&Object.getPrototypeOf(one.value)===childArray&&
            Object.getPrototypeOf(key)===childObject&&Object.getPrototypeOf(fin)===childObject&&
            Object.getPrototypeOf(fin.value)===childArray&&done.done&&done.value===marker&&
            Object.getPrototypeOf(done)===Object.prototype&&Object.getPrototypeOf(initial)===Object.prototype&&
            initial.value===marker&&completed.value===marker&&Object.getPrototypeOf(completed)===Object.prototype&&identity&&wrong;
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
        assert!(!i.strict_eval_context);
    }
}

#[test]
fn borrowed_sync_errors_and_delegated_results_keep_original_identity() {
    for ast in [false,true] {
        let mut i=realm_factory(ast,"(function*(){try{class C{[(yield 0,missingInClass=1,'x')]=1}}catch(e){yield e}return []})");
        assert_eq!(i.run_script(r#"
            var g=f();next.call(g);var error=next.call(g),done=next.call(g);
            Object.getPrototypeOf(error)===childObject&&Object.getPrototypeOf(error.value)===childReferenceError&&
            done.done&&Object.getPrototypeOf(done)===Object.prototype&&Object.getPrototypeOf(done.value)===childArray&&
            typeof missingInClass==='undefined';
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
        let mut i=realm_factory(ast,"(function*(source){try{yield* source}catch(e){yield e}return []})");
        assert_eq!(i.run_script(r#"
            var raw={value:1,done:false};var source={[Symbol.iterator](){return this},next(){return raw},throw(){throw marker}};
            var g=f(source),first=next.call(g),caught=thr.call(g,marker),done=next.call(g);
            first===raw&&caught.value===marker&&Object.getPrototypeOf(caught)===childObject&&
            done.done&&Object.getPrototypeOf(done)===Object.prototype&&Object.getPrototypeOf(done.value)===childArray;
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}
