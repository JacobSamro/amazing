use super::*;

fn matrix(body: &str) {
    let mut failures = Vec::new();
    for actor in [false, true] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut i = Interpreter::new();
            i.test_force_ast_callables(actor);
            i.run_script(&format!("var tested=function(){{{body}}};"))
                .unwrap();
            let JsValue::Object(function) = i.run_script("tested").unwrap() else {
                panic!("callable")
            };
            assert_eq!(
                matches!(
                    function.borrow().func.as_ref().unwrap().body,
                    FunctionBody::Bytecode(_)
                ),
                !actor,
                "actual factory"
            );
            assert_eq!(
                i.run_script("tested()")
                    .unwrap_or_else(|e| panic!("{}", e.describe())),
                JsValue::Bool(true)
            );
        }));
        if result.is_err() {
            failures.push(format!("AST={actor}: {body}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn strict_mutual_and_concise_tail_recursion_releases_activation_resources() {
    matrix(
        r#"
        function f(n){'use strict';if(!n)return 7;return g(n-1)}
        function g(n){'use strict';if(!n)return 7;return f(n-1)}
        var h=(n)=>n?h(n-1):7;
        function strictArrow(){'use strict';var a=n=>n?a(n-1):7;return a(2500)}
        return f(2500)===7&&g(2501)===7&&strictArrow()===7;
    "#,
    );
}

#[test]
fn complete_return_expression_tail_positions_and_nonfinal_operands() {
    for expression in [
        "f(n-1)",
        "(f(n-1))",
        "(((f(n-1))))",
        "(0,f(n-1))",
        "true?f(n-1):0",
        "false?0:f(n-1)",
        "true&&f(n-1)",
        "false||f(n-1)",
        "null??f(n-1)",
        "undefined??f(n-1)",
    ] {
        matrix(&format!(
            "function f(n){{'use strict';if(!n)return 7;return {expression}}}return f(2500)===7;"
        ));
    }
    matrix(
        r#"
        var calls=0;function value(){calls++;return 7}
        function f(){'use strict';return (value(),8)}
        function g(){'use strict';return 0&&value()}
        function h(){'use strict';return 1||value()}
        function k(){'use strict';return 0??value()}
        return f()===8&&g()===0&&h()===1&&k()===0&&calls===1;
    "#,
    );
}

#[test]
fn whole_statement_tail_positions_preserve_loops_labels_switch_and_catch() {
    for statement in [
        "{return f(n-1)}",
        "while(true){return f(n-1)}",
        "do{return f(n-1)}while(false)",
        "for(;;){return f(n-1)}",
        "for(var x=0;x<1;x++){return f(n-1)}",
        "for(let x=0;x<1;x++){return f(n-1)}",
        "for(const x=0;x<1;){return f(n-1)}",
        "for(var x in {x:1}){return f(n-1)}",
        "if(n)return f(n-1);else return 0",
        "if(!n)return 0;else return f(n-1)",
        "a:b:{return f(n-1)}",
        "switch(n){case 1:return f(n-1);default:return f(n-1)}",
        "try{throw 0}catch(e){return f(n-1)}",
        "try{}finally{return f(n-1)}",
        "try{}catch(e){}finally{return f(n-1)}",
    ] {
        matrix(&format!(
            "function f(n){{'use strict';if(!n)return 7;{statement}}}return f(2500)===7;"
        ));
    }
}

#[test]
fn tail_callee_arguments_spreads_and_raw_template_substitutions_evaluate_once() {
    matrix(
        r#"
        var log=[],o={get f(){log.push('get');return function(n){'use strict';return this===o&&n===3}}};
        var key={toString(){log.push('key');return 'f'}};
        function raw(){log.push('raw');return key}function arg(){log.push('arg');return 3}
        function call(){'use strict';return (o[raw()])(...[arg()])}
        var a=call();
        var rawValue={toString(){throw 1}};
        function tag(strings,value){'use strict';return this===o&&value===rawValue&&Object.isFrozen(strings)&&Object.isFrozen(strings.raw)}
        o.tag=tag;function tagged(){'use strict';return o.tag`x${rawValue}`}
        return a&&tagged()&&log.join(',')==='raw,key,get,arg';
    "#,
    );
}

#[test]
fn terminal_tagged_template_calls_trampoline_with_original_site_identity() {
    matrix(
        r#"
        var saved,count=0;function f(strings,n){'use strict';if(saved===undefined)saved=strings;
            else if(saved!==strings)return -1;if(!n)return 7;count++;return f`x${n-1}`}
        function start(){'use strict';return f`x${2500}`}
        // The start and recursive productions are distinct sites; only the
        // recursive site must retain identity on every subsequent invocation.
        function g(strings,n){'use strict';if(!n)return 7;return g`x${n-1}`}
        return start()===-1&&g`x${2500}`===7;
    "#,
    );
}

#[test]
fn terminal_optional_calls_private_and_super_calls_keep_the_reference_receiver() {
    matrix(
        r#"
        function f(n){'use strict';if(!n)return 7;return f?.(n-1)}
        var object={f(n){'use strict';if(!n)return 7;return this?.f(n-1)}};
        var missed=0;function nil(){'use strict';return null?.f(missed++)}
        class Base{f(n){return n?this.f(n-1):7}}
        class Derived extends Base{f(n){return n?super.f(n-1):7}}
        class Private{#f(n){return n?this.#f(n-1):7}run(n){return this.#f(n)}}
        return f(2500)===7&&object.f(2500)===7&&nil()===undefined&&missed===0&&
            new Derived().f(2500)===7&&new Private().run(2500)===7;
    "#,
    );
}

#[test]
fn real_direct_eval_and_other_functions_named_eval_keep_distinct_call_semantics() {
    matrix(
        r#"
        function actual(n){'use strict';return eval('n+1')}
        function other(n){'use strict';if(!n)return 7;var evalAlias=other;return evalAlias(n-1)}
        function enclosing(){var eval=other;return function(n){'use strict';return eval(n)}}
        var named=enclosing();
        return actual(2)===3&&other(2500)===7&&named(2500)===7;
    "#,
    );
}

#[test]
fn native_call_apply_reflect_and_bound_proxy_forwarders_do_not_accumulate_frames() {
    for call in [
        "f.call(undefined,n-1)",
        "f.apply(undefined,[n-1])",
        "Reflect.apply(f,undefined,[n-1])",
        "bound(n-1)",
        "proxy(n-1)",
        "trapped(n-1)",
    ] {
        matrix(&format!(
            r#"
            function f(n){{'use strict';if(!n)return 7;return {call}}}
            var bound=f.bind(undefined),proxy=new Proxy(f,{{}}),trapped=new Proxy(f,{{apply(t,r,a){{'use strict';return Reflect.apply(t,r,a)}}}});
            return f(2500)===7;
        "#
        ));
    }
    matrix(
        r#"
        function f(n){'use strict';if(!n)return 7;return wrapped(n-1)}
        var wrapped=f;for(var k=0;k<64;k++)wrapped=k%2?wrapped.bind(undefined):new Proxy(wrapped,{});
        return f(2500)===7;
    "#,
    );
}

#[test]
fn protected_try_for_of_and_using_keep_postcall_cleanup_and_error_precedence() {
    matrix(
        r#"
        var log=[],marker={};function target(){log.push('call');throw marker}
        function caught(){'use strict';try{return target()}catch(e){log.push('catch');return e===marker}}
        function finalized(){'use strict';try{return target()}finally{log.push('finally')}}
        var a=caught(),b=false;try{finalized()}catch(e){b=e===marker}
        var resource={[Symbol.dispose](){log.push('dispose')}};
        function usingBody(){'use strict';using x=resource;try{return target()}catch(e){return e===marker}}
        var c=usingBody();
        var iterable={[Symbol.iterator](){return {next(){return {value:1,done:false}},return(){log.push('close');return {}}}}};
        function of(){'use strict';for(var x of iterable)return 7}
        return a&&b&&c&&of()===7&&log.join(',')==='call,catch,call,finally,call,dispose,close';
    "#,
    );
}

#[test]
fn calls_before_using_still_tail_but_static_preceding_null_using_is_not_tail() {
    matrix(
        r#"
        function before(n){'use strict';if(!n)return 7;return before(n-1);using x=null}
        function after(n){'use strict';using x=null;if(!n)return 7;return after(n-1)}
        var bounded=false;try{after(2500)}catch(e){bounded=e instanceof RangeError}
        return before(2500)===7&&bounded;
    "#,
    );
}

#[test]
fn non_tail_sloppy_and_async_generator_bodies_retain_normal_activation_rules() {
    matrix(
        r#"
        function sloppy(n){if(!n)return 7;return sloppy(n-1)}
        function valueUse(n){'use strict';if(!n)return 0;return 1+valueUse(n-1)}
        function protectedCall(n){'use strict';if(!n)return 0;try{return protectedCall(n-1)}finally{}}
        var caught=0;for(var f of [sloppy,valueUse,protectedCall]){try{f(2500)}catch(e){if(e instanceof RangeError)caught++}}
        return caught===3;
    "#,
    );
}

#[test]
fn strict_constructor_tail_returns_preserve_base_and_derived_result_obligations() {
    matrix(
        r#"
        function f(n){'use strict';return n?f(n-1):{value:7}}
        function Base(){'use strict';return f(2500)}
        class Derived extends Object{constructor(){return f(2500)}}
        function primitive(n){'use strict';return n?primitive(n-1):3}
        class Bad extends Object{constructor(){return primitive(2500)}}
        function absent(n){'use strict';return n?absent(n-1):undefined}
        class Missing extends Object{constructor(){return absent(2500)}}
        class Initialized extends Object{constructor(){super();this.value=9;return absent(2500)}}
        var rejected=false,uninitialized=false;
        try{new Bad()}catch(e){rejected=e instanceof TypeError}
        try{new Missing()}catch(e){uninitialized=e instanceof ReferenceError}
        return new Base().value===7&&new Derived().value===7&&new Initialized().value===9&&rejected&&uninitialized;
    "#,
    );
}

#[test]
fn failing_call_checks_follow_arguments_and_preserve_original_thrown_values() {
    matrix(
        r#"
        var log=[],marker={},nil=null;
        function argument(){log.push('arg');return 1}
        function bad(){'use strict';return nil(argument())}
        function abrupt(){'use strict';return nil((()=>{throw marker})())}
        var a=false,b=false;try{bad()}catch(e){a=e instanceof TypeError}try{abrupt()}catch(e){b=e===marker}
        var revoked=Proxy.revocable(function(){},{});revoked.revoke();
        function proxy(){'use strict';return revoked.proxy(argument())}
        var c=false;try{proxy()}catch(e){c=e instanceof TypeError}
        return a&&b&&c&&log.join(',')==='arg,arg';
    "#,
    );
}

#[test]
fn mixed_factory_tail_calls_preserve_actual_ast_and_bytecode_targets() {
    for actor_first in [false, true] {
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor_first);
        let f = i
            .run_script("var f=function(n){'use strict';return n?g(n-1):7};f")
            .unwrap();
        i.test_force_ast_callables(!actor_first);
        let g = i
            .run_script("var g=function(n){'use strict';return n?f(n-1):7};g")
            .unwrap();
        for (value, actor) in [(&f, actor_first), (&g, !actor_first)] {
            let JsValue::Object(function) = value else {
                panic!("callable")
            };
            assert_eq!(
                matches!(
                    function.borrow().func.as_ref().unwrap().body,
                    FunctionBody::Bytecode(_)
                ),
                !actor
            );
        }
        assert_eq!(
            i.call_value(&f, JsValue::Undefined, vec![JsValue::num(2500.0)])
                .unwrap(),
            JsValue::num(7.0)
        );
    }
}

#[test]
fn foreign_tail_targets_execute_in_their_own_realm_and_restore_the_host_realm() {
    for actor in [false, true] {
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        let parent = i.binding_current_realm();
        let object = i.binding_create_object(None).unwrap();
        let child = i.binding_create_realm(object).unwrap();
        let foreign = i
            .binding_eval_in_realm(
                child,
                "var f=function(n){'use strict';return n?back(n-1):Object.prototype};f",
            )
            .unwrap();
        let local = i
            .run_script("var f=function(n){'use strict';return n?foreign(n-1):Object.prototype};f")
            .unwrap();
        let global = i.binding_realm_global(child).unwrap();
        i.binding_set_property(&global, "back", local.clone())
            .unwrap();
        let global = i.binding_global_object();
        i.binding_set_property(&global, "foreign", foreign.clone())
            .unwrap();
        let expected = i.binding_eval_in_realm(child, "Object.prototype").unwrap();
        assert_eq!(
            i.call_value(&local, JsValue::Undefined, vec![JsValue::num(2501.0)])
                .unwrap(),
            expected
        );
        assert_eq!(i.binding_current_realm(), parent);
        assert_eq!(i.frames.len(), 0);
        assert_eq!(i.depth, 0);
        let error=i.binding_eval_in_realm(child,"var failure=new Error('child');var fail=function(n){'use strict';if(!n)throw failure;return fail(n-1)};failure").unwrap();
        let fail = i.binding_eval_in_realm(child, "fail").unwrap();
        assert_eq!(
            i.call_value(&fail, JsValue::Undefined, vec![JsValue::num(2500.0)])
                .unwrap_err()
                .0,
            error
        );
        assert_eq!(i.binding_current_realm(), parent);
    }
}

#[test]
fn actual_activation_probe_observes_flat_depth_after_the_caller_is_discarded() {
    fn probe(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
        Ok(JsValue::num(i.depth as f64))
    }
    for actor in [false, true] {
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        let callable = Interpreter::native("probe", probe, &i.function_proto);
        let global = i.binding_global_object();
        i.binding_set_property(&global, "probe", callable).unwrap();
        i.run_script("function f(n){'use strict';return n?f(n-1):probe()}")
            .unwrap();
        assert_eq!(i.run_script("f(2500)").unwrap(), JsValue::num(1.0));
        assert_eq!(i.depth, 0);
        assert!(i.frames.is_empty());
    }
}

#[test]
fn suspended_callable_kinds_keep_await_yield_and_finalizer_boundaries() {
    for actor in [false, true] {
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        i.run_script(r#"
            var log=[],out=[];
            function target(v){'use strict';log.push('target');return v}
            function* g(){'use strict';try{return target((yield 1,7))}finally{log.push('g-finally')}}
            async function a(){'use strict';try{return target(await 7)}finally{log.push('a-finally')}}
            async function* ag(){'use strict';try{return target((yield 1,7))}finally{log.push('ag-finally')}}
            var x=g();x.next();out.push(x.next().value);
            a().then(v=>out.push(v));var y=ag();y.next().then(()=>y.next()).then(r=>out.push(r.value));
        "#).unwrap();
        i.drain_jobs().unwrap();
        assert_eq!(i.run_script("out.length===3&&out.every(v=>v===7)&&log.filter(v=>v==='target').length===3&&log.indexOf('g-finally')>log.indexOf('target')").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn switch_nested_resources_preserve_call_disposal_and_fallthrough_order() {
    matrix(
        r#"
        var log=[],failure={};
        function target(){'use strict';log.push('call');return 7}
        function same(){'use strict';switch(1){case 1:{using x={[Symbol.dispose](){log.push('dispose')}};return target()}}}
        function crossing(){'use strict';switch(1){case 1:{using x={[Symbol.dispose](){log.push('dispose')}};}case 2:return target()}}
        function abrupt(){'use strict';switch(1){case 1:{using x={[Symbol.dispose](){log.push('dispose')}};}case 2:return (()=>{log.push('throw');throw failure})()}}
        var a=same(),b=crossing(),caught=false;try{abrupt()}catch(e){caught=e===failure}
        return a===7&&b===7&&caught&&log.join(',')==='call,dispose,dispose,call,dispose,throw';
    "#,
    );
}

#[test]
fn tail_target_retains_its_genuine_module_job_source_context() {
    for actor in [false, true] {
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        let observed = Rc::new(RefCell::new(Vec::new()));
        let captured = observed.clone();
        i.binding_set_typed_module_loader_hook(Rc::new(move |_, specifier, referrer| {
            captured
                .borrow_mut()
                .push((specifier.to_string(), referrer.to_string()));
            Ok(JsValue::Undefined)
        }));
        i.run_script_with_source_context(
            "function target(){'use strict';return import('./child.js')}",
            "https://example.test/target/body.js",
        )
        .unwrap();
        i.run_script_with_source_context(
            "function f(n){'use strict';return n?f(n-1):target()};f(2500)",
            "https://example.test/caller/start.js",
        )
        .unwrap();
        i.drain_jobs().unwrap();
        assert_eq!(
            &*observed.borrow(),
            &[(
                "./child.js".to_owned(),
                "https://example.test/target/body.js".to_owned()
            )]
        );
        assert!(i.frames.is_empty());
        assert_eq!(i.depth, 0);
    }
}
