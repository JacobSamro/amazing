use amazing_js::interp::ExecutionBackend;
use amazing_js::value::JsValue;
use amazing_js::Interpreter;

fn check_both(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let result = i
            .run_script_with_backend(source, backend)
            .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe()));
        assert_eq!(result, JsValue::Bool(true), "{backend:?}");
    }
}

#[test]
fn calls_and_arguments_run_before_reference_error_without_rhs_or_conversion() {
    for op in [
        "=", "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", ">>>=", "&=", "|=", "^=",
    ] {
        for target in ["target(argument())", "(target(argument()))"] {
            check_both(&format!(
                r#"
                var events=[],caught=false;
                function argument(){{events.push('argument');return 1}}
                function target(value){{events.push('call');return {{[Symbol.toPrimitive](){{events.push('convert');return 1}}}}}}
                function rhs(){{events.push('rhs');return 2}}
                try{{{target} {op} rhs()}}catch(e){{caught=e instanceof ReferenceError}}
                caught&&events.join(',')==='argument,call';
            "#
            ));
        }
    }
}

#[test]
fn prefix_and_postfix_calls_reject_before_numeric_conversion() {
    for target in ["target(argument())", "(target(argument()))"] {
        for expression in [
            format!("++{target}"),
            format!("--{target}"),
            format!("{target}++"),
            format!("{target}--"),
        ] {
            check_both(&format!(
                r#"
                var events=[],caught=false;
                function argument(){{events.push('argument');return 1}}
                function target(value){{events.push('call');return {{valueOf(){{events.push('convert');return 1}}}}}}
                try{{{expression}}}catch(e){{caught=e instanceof ReferenceError}}
                caught&&events.join(',')==='argument,call';
            "#
            ));
        }
    }
}

#[test]
fn loop_targets_run_per_iteration_and_abrupt_reference_failure_closes_iterator() {
    check_both(
        r#"
        var calls=0,bodies=0,closed=0,referenceErrors=0;
        function target(){calls++;return 1}
        for(target() in {}){bodies++}
        for(target() of []){bodies++}
        try{for(target() in {a:1,b:2}){bodies++}}catch(e){if(e instanceof ReferenceError)referenceErrors++}
        var source={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){closed++;return {}}}}};
        try{for(target() of source){bodies++}}catch(e){if(e instanceof ReferenceError)referenceErrors++}
        calls===2&&bodies===0&&closed===1&&referenceErrors===2;
    "#,
    );
}

#[test]
fn call_abrupt_completion_keeps_identity_and_blocks_rhs() {
    check_both(
        r#"
        var marker={},rhsCalls=0,argumentsCalled=0,caught;
        function argument(){argumentsCalled++;return 1}
        function target(value){throw marker}
        function rhs(){rhsCalls++;return 2}
        try{target(argument())+=rhs()}catch(e){caught=e}
        caught===marker&&rhsCalls===0&&argumentsCalled===1;
    "#,
    );
}

#[test]
fn strict_logical_and_optional_call_targets_remain_syntax_errors() {
    for source in [
        "'use strict';target()=1;",
        "function strict(){'use strict';target()=1}",
        "target()&&=1;",
        "target()||=1;",
        "target()??=1;",
        "target?.()=1;",
        "target?.()()=1;",
        "target?.member()=1;",
        "(target?.())=1;",
        "target?.()++;",
    ] {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let error = i
                .run_script_with_backend(source, backend)
                .expect_err(source);
            assert!(
                error.describe().contains("SyntaxError"),
                "{backend:?} {source}: {}",
                error.describe()
            );
        }
    }
}

#[test]
fn grouping_closes_optional_callee_chain_and_optional_arguments_are_allowed() {
    check_both(
        r#"
        var calls=0,argumentCalls=0,rhsCalls=0,caught=0;
        function inner(){calls++;return 1}
        var holder={method(){calls++;return inner}};
        function argument(){argumentCalls++;return 7}
        function target(value){calls++;return value}
        function rhs(){rhsCalls++;return 2}
        try{(holder?.method())()=rhs()}catch(e){if(e instanceof ReferenceError)caught++}
        try{target(argument?.())=rhs()}catch(e){if(e instanceof ReferenceError)caught++}
        calls===3&&argumentCalls===1&&rhsCalls===0&&caught===2;
    "#,
    );
}

#[test]
fn suspended_arguments_finish_before_call_target_rejection_and_generator_close() {
    check_both(
        r#"
        var events=[],marker={},caught;
        function target(value){events.push('call:'+value);return 1}
        function rhs(){events.push('rhs');return 2}
        function* generator(){try{target(yield 'argument')=rhs()}finally{events.push('finally')}}
        var g=generator(),first=g.next();
        try{g.next(7)}catch(e){caught=e instanceof ReferenceError}
        var h=generator();h.next();var ended=h.return(marker);
        first.value==='argument'&&!first.done&&caught&&ended.value===marker&&ended.done&&events.join(',')==='call:7,finally,finally';
    "#,
    );
}

#[test]
fn awaited_arguments_complete_before_reference_error_without_rhs_execution() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.run_script_with_backend(
            r#"
            var events=[],completed=false;
            function target(value){events.push('call:'+value);return 1}
            function rhs(){events.push('rhs');return 2}
            async function work(){
                try{target(await Promise.resolve(9))=rhs()}
                catch(e){completed=e instanceof ReferenceError}
                finally{events.push('finally')}
            }
            work();
        "#,
            backend,
        )
        .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe()));
        i.drain_microtasks().unwrap();
        assert_eq!(
            i.run_script("completed&&events.join(',')==='call:9,finally'")
                .unwrap(),
            JsValue::Bool(true),
            "{backend:?}"
        );
    }
}
