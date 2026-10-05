use super::*;

fn default_check(source: &str, check: &str) {
    for ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = ast;
        i.run_script(source).unwrap();
        let JsValue::Object(function) = i.run_script("C").unwrap() else { panic!("class") };
        assert!(super::super::is_default_class_constructor(&function));
        assert!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Native(_)));
        assert_eq!(i.run_script(check).unwrap(), JsValue::Bool(true), "AST={ast} {source}");
    }
}

#[test]
fn default_constructor_whole_base_derived_live_parent_and_null_matrix() {
    for base in ["class C { field=7;#p=11;read(){return this.#p} }",
        "class A{};class C extends A { field=7;#p=11;read(){return this.#p} }"] {
        default_check(&format!("{base};var value=new C(1,2)"),
            "value.field===7&&value.read()===11&&value instanceof C&&C.length===0");
    }
    for parent in ["null", "{}", "()=>{}", "function*(){}"] {
        default_check(&format!("var count=0,error;class A{{}};class C extends A{{field=++count}};Object.setPrototypeOf(C,{parent});try{{new C}}catch(e){{error=e}}"),
            "error instanceof TypeError&&count===0");
    }
    default_check("class A{constructor(){this.parent='A'}};class B{constructor(){this.parent='B'}};class C extends A{};Object.setPrototypeOf(C,B);var value=new C",
        "value.parent==='B'&&value instanceof C");
}

#[test]
fn default_forwarding_uses_original_arguments_without_array_iterator() {
    default_check(r#"
        var calls=0,marker={},saved=Array.prototype[Symbol.iterator],value;
        function A(){this.args=arguments.length;this.first=arguments[0];this.last=arguments[3];this.nt=new.target}
        class C extends A{};
        Array.prototype[Symbol.iterator]=function(){calls++;throw marker};
        try{value=new C(marker,undefined,3,4)}finally{Array.prototype[Symbol.iterator]=saved}
    "#, "calls===0&&value.args===4&&value.first===marker&&value.last===4&&value.nt===C");
    default_check("var log=[];class A{constructor(x){log.push(x)}};class C extends A{};var iterable={[Symbol.iterator](){log.push('iterator');return [13][Symbol.iterator]()}};new C(...iterable)",
        "log.join(',')==='iterator,13'");
}

#[test]
fn default_builtin_call_errors_and_explicit_class_errors_have_distinct_realms() {
    for ast in [false, true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
        let exported=i.binding_eval_in_realm(child,
            "class A{};class DefaultBase{};class DefaultDerived extends A{};class ExplicitBase{constructor(){}};class ExplicitDerived extends A{constructor(){super()}};[DefaultBase,DefaultDerived,ExplicitBase,ExplicitDerived,TypeError]").unwrap();
        i.global_env.declare_var("exports",exported).unwrap();
        assert_eq!(i.run_script(r#"
            var ok=true;
            for(var n=0;n<4;n++){var c=exports[n];
                for(var mode=0;mode<7;mode++){try{
                    if(mode===0)c();else if(mode===1)c.call(null);else if(mode===2)c.bind(null)();
                    else if(mode===3)new Proxy(c,{} )();
                    else if(mode===4)Function.prototype.call.call(c,null);
                    else if(mode===5)Function.prototype.apply.call(c,null,[]);
                    else Reflect.apply(c,null,[]);
                    ok=false
                }catch(e){ok=ok&&(Object.getPrototypeOf(e)===exports[4].prototype)}}}
            ok
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}

#[test]
fn default_base_newtarget_prototype_read_and_foreign_fallback_are_exact() {
    default_check("var count=0,target=function(){},prototype={},value;var nt=new Proxy(target,{get(t,k,r){if(k==='prototype'){count++;return prototype}return Reflect.get(t,k,r)}});class C{field=17};value=Reflect.construct(C,[],nt)",
        "count===1&&Object.getPrototypeOf(value)===prototype&&value.field===17");
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
        let exported=i.binding_eval_in_realm(child,"function N(){};N.prototype=1;[N,Object.prototype]").unwrap();
        i.global_env.declare_var("exports",exported).unwrap();
        assert_eq!(i.run_script("class C{};var value=Reflect.construct(C,[],exports[0]);Object.getPrototypeOf(value)===exports[1]").unwrap(),JsValue::Bool(true));
    }
    default_check("var marker={},count=0,error;class C{field=++count};var nt=new Proxy(function(){},{get(t,k,r){if(k==='prototype')throw marker;return Reflect.get(t,k,r)}});try{Reflect.construct(C,[],nt)}catch(e){error=e}",
        "error===marker&&count===0");
}

#[test]
fn default_derived_preserves_existing_parent_result_and_newtarget_wrappers() {
    default_check("var result={},nt;class A{constructor(){nt=new.target;return result}};class C extends A{field=19;#p=23;read(){return this.#p}};var bound=C.bind(null,1);var value=new bound(2)",
        "value===result&&nt===C&&value.field===19&&C.prototype.read.call(value)===23&&Object.getPrototypeOf(result)===Object.prototype");
    default_check("var result={},seen;function A(){seen=new.target;return result};class C extends A{field=29};var proxy=new Proxy(C,{});var value=new proxy(1)",
        "seen===proxy&&value===result&&value.field===29&&Object.getPrototypeOf(value)===Object.prototype");
    default_check("var seen,args;var A=new Proxy(function(){},{construct(t,a,n){seen=n;args=a;return {}}});class C extends A{field=31};var value=new C(3,5)",
        "seen===C&&args.length===2&&args[0]===3&&args[1]===5&&value.field===31");
}

#[test]
fn default_instance_elements_are_strict_ordered_and_initialized_once() {
    default_check("var log=[];class A{a=(log.push('base'),1)};class C extends A{#p=(log.push('private'),2);b=(log.push('derived'),3);read(){return this.#p}};var value=new C",
        "log.join(',')==='base,private,derived'&&value.a===1&&value.b===3&&value.read()===2");
    default_check("var log=[],marker={},error;class A{constructor(){log.push('parent')}};class C extends A{first=(log.push('first'),1);second=(()=>{throw marker})();last=log.push('last')};try{new C}catch(e){error=e}",
        "error===marker&&log.join(',')==='parent,first'");
    default_check("var error;class C{field=(()=>{unbound_default_field_name=1})()};try{new C}catch(e){error=e}",
        "error instanceof ReferenceError&&typeof unbound_default_field_name==='undefined'");
}

#[test]
fn default_constructor_keeps_class_source_metadata_and_builtin_descriptors() {
    default_check("class C /*original*/ { field=37 };var keys=Reflect.ownKeys(C),d=Object.getOwnPropertyDescriptor(C,'length'),p=Object.getOwnPropertyDescriptor(C,'prototype')",
        "C.toString()==='class C /*original*/ { field=37 }'&&keys[0]==='length'&&keys[1]==='name'&&d.value===0&&!d.writable&&!d.enumerable&&d.configurable&&!p.writable&&!p.configurable&&C.prototype.constructor===C");
}

#[test]
fn selected_super_parent_survives_argument_await_yield_and_finally() {
    for kind in ["generator", "async", "async-generator"] {
        for ast in [false,true] {
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            // Derived constructors are synchronous, but an escaped lexical
            // arrow can retain their FunctionEnvironment after abrupt return.
            let source=match kind {
                "generator" => "function* drive(){yield 1;return escaped()}var g=drive();g.next();Object.setPrototypeOf(C,B);var value=g.next().value",
                "async" => "async function drive(){await 0;return escaped()}var promise=drive();Object.setPrototypeOf(C,B);var value;promise.then(v=>value=v)",
                _ => "async function* drive(){yield 1;return escaped()}var g=drive();var value;g.next().then(()=>{Object.setPrototypeOf(C,B);return g.next()}).then(r=>value=r.value)",
            };
            i.run_script(&format!("var escaped;class A{{constructor(){{this.which='A'}}}};class B{{constructor(){{this.which='B'}}}};class C extends A{{field=41;constructor(){{escaped=()=>super();throw 1}}}};try{{new C}}catch(e){{}};{source}")).unwrap();
            // Drain actual promise/generator jobs explicitly.
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("value.which==='B'&&value.field===41").unwrap(),JsValue::Bool(true),"AST={ast} {kind}");
        }
    }
}

#[test]
fn async_lexical_super_snapshots_parent_before_await_argument_and_preserves_throw() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var escaped,value,marker={},caught;
            class A{constructor(x){this.which='A';this.arg=x}};
            class B{constructor(){this.which='B'}};
            class C extends A{field=43;constructor(){escaped=async(throwing)=>super(await (throwing?Promise.reject(marker):Promise.resolve(7)));throw 1}};
            try{new C}catch(e){};
            escaped(false).then(v=>value=v);Object.setPrototypeOf(C,B);
        "#).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("value.which==='A'&&value.arg===7&&value.field===43").unwrap(),JsValue::Bool(true),"AST={ast}");
        i.run_script("escaped(true).then(()=>caught=false,e=>caught=e===marker)").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("caught===true").unwrap(),JsValue::Bool(true));
    }
}
