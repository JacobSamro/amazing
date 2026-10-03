use amazing_js::{interp::ExecutionBackend, value::JsValue, Interpreter};

fn check_both(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let result = i
            .run_script_with_backend(source, backend)
            .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe()));
        assert_eq!(result, JsValue::Bool(true), "{backend:?}: {source}");
    }
}

#[test]
fn forwards_original_results_without_value_get_and_caches_next() {
    check_both(
        r#"var values=0,gets=0,doneGets=0,calls=[];var result={get done(){doneGets++;return false},get value(){values++;return 9}};var iterator={get next(){gets++;return function(v){calls.push(v);return result}},[Symbol.iterator](){return this}};function* g(){yield* iterator};var it=g(),a=it.next(77),b=it.next(8);a===result&&b===result&&values===0&&doneGets===2&&gets===1&&calls.length===2&&calls[0]===undefined&&calls[1]===8&&a.value===9&&values===1;"#,
    );
}

#[test]
fn arrays_use_overridden_iterator_and_native_iterator_return() {
    check_both(
        r#"var calls=0,a=[1,2];a[Symbol.iterator]=function(){calls++;return {next(){return {value:7,done:true}}}};function* g(){return yield* a};var r=g().next();calls===1&&r.done&&r.value===7;"#,
    );
    check_both(
        r#"var proto=Object.getPrototypeOf([][Symbol.iterator]()),calls=0;proto.return=function(v){calls++;return {done:true,value:v+1}};function* g(){yield* [1,2]};var it=g(),first=it.next(),last=it.return(8);first.value===1&&calls===1&&last.done&&last.value===9;"#,
    );
    check_both("var a=[1];function* g(){yield* a};var it=g();var first=it.next();a.push(2);first.value===1&&it.next().value===2&&it.next().done;");
}

#[test]
fn primitive_strings_delegate_by_unicode_code_point() {
    check_both("function* g(){return yield* 'a😀b'};var it=g();it.next().value==='a'&&it.next().value==='😀'&&it.next().value==='b'&&it.next().done;");
}

#[test]
fn malformed_iterator_validation_precedes_next_and_next_is_cached_without_prevalidation() {
    check_both(
        r#"var count=0,ok=false;Object.defineProperty(Number.prototype,'next',{get(){count++;return function(){return {done:true}}}});function* g(){yield* {[Symbol.iterator](){return 2}}};try{g().next()}catch(e){ok=e instanceof TypeError};ok&&count===0;"#,
    );
    for method in ["null", "undefined", "7", "{}"] {
        check_both(&format!("var ok=false;function* g(){{yield* {{[Symbol.iterator]:{method}}}}};try{{g().next()}}catch(e){{ok=e instanceof TypeError}};ok;"));
    }
    check_both("var gets=0,ok=false;function* g(){yield* {[Symbol.iterator](){return {get next(){gets++;return 1}}}}};try{g().next()}catch(e){ok=e instanceof TypeError};ok&&gets===1;");
}

#[test]
fn missing_throw_closes_with_no_argument_and_close_errors_take_precedence() {
    for throw in ["null", "undefined"] {
        check_both(&format!("var count=0,args=-1,caught=false;var x={{next(){{return {{done:false}}}},throw:{throw},return(){{count++;args=arguments.length;return {{done:true}}}},[Symbol.iterator](){{return this}}}};function* g(){{yield* x}};var it=g();it.next();try{{it.throw(9)}}catch(e){{caught=e instanceof TypeError}};caught&&count===1&&args===0;"));
    }
    check_both("var marker={},same=false,x={next(){return {done:false}},return(){throw marker},[Symbol.iterator](){return this}};function* g(){yield* x};var it=g();it.next();try{it.throw(7)}catch(e){same=e===marker};same;");
    check_both("var close=0,ok=false,x={next(){return {done:false}},throw:7,return(){close++;return {}},[Symbol.iterator](){return this}};function* g(){yield* x};var it=g();it.next();try{it.throw(7)}catch(e){ok=e instanceof TypeError};ok&&close===0;");
}

#[test]
fn absent_return_completes_and_present_noncallable_return_throws() {
    for method in ["null", "undefined"] {
        check_both(&format!("var caught=false,finalized=0,x={{next(){{return {{done:false}}}},return:{method},[Symbol.iterator](){{return this}}}};function* g(){{try{{yield* x}}catch(e){{caught=true}}finally{{finalized++}}}};var it=g();it.next();var r=it.return(8);r.done&&r.value===8&&!caught&&finalized===1;"));
    }
    check_both("var ok=false,x={next(){return {done:false}},return:7,[Symbol.iterator](){return this}};function* g(){yield* x};var it=g();it.next();try{it.return(8)}catch(e){ok=e instanceof TypeError};ok;");
}

#[test]
fn throw_and_return_live_results_preserve_identity_and_defer_value_get() {
    for method in ["throw", "return"] {
        check_both(&format!("var values=0,received;var live={{done:false,get value(){{values++;return 3}}}};var x={{next(){{return {{done:false}}}},{method}(v){{received=v;return live}},[Symbol.iterator](){{return this}}}};function* g(){{yield* x}};var it=g();it.next();var r=it.{method}(8);r===live&&received===8&&values===0&&r.value===3&&values===1;"));
    }
    check_both("var x={next(){return {done:false}},throw(v){return {done:true,value:v+1}},[Symbol.iterator](){return this}};function* g(){var v=yield* x;return v+1};var it=g();it.next();var r=it.throw(8);r.done&&r.value===10;");
}

#[test]
fn errors_drop_old_record_before_a_caught_new_delegation() {
    check_both("var first=0,second=0;var x={next(){first++;throw 1},[Symbol.iterator](){return this}},y={next(){second++;return {done:true,value:9}},[Symbol.iterator](){return this}};function* g(){try{yield* x}catch(e){}return yield* y};var r=g().next();r.done&&r.value===9&&first===1&&second===1;");
}

#[test]
fn result_getter_errors_and_nonobjects_keep_abrupt_identity() {
    for result in ["null", "undefined", "7"] {
        check_both(&format!("var ok=false;var x={{next(){{return {result}}},[Symbol.iterator](){{return this}}}};function* g(){{yield* x}};try{{g().next()}}catch(e){{ok=e instanceof TypeError}};ok;"));
    }
    for property in ["done", "value"] {
        check_both(&format!("var marker={{}},same=false;var r={{done:true,get {property}(){{throw marker}}}};var x={{next(){{return r}},[Symbol.iterator](){{return this}}}};function* g(){{yield* x}};try{{g().next()}}catch(e){{same=e===marker}};same;"));
    }
}
