use super::*;

#[test]
fn suspended_with_preserves_object_scopes_across_all_callable_drivers() {
    let cases = [
        "var o={x:1};with(o){PAUSE;x+=2;x++;}return o.x===4&&typeof x==='undefined'",
        "var o={x:1},capture;with(o){capture=()=>x;PAUSE;o.x=8;}return capture()===8",
        "var o={x:1};with(o){PAUSE;var x=(o.x=7,4);}return o.x===4",
        "var o={x:1};with(o){PAUSE;[x]=[6];({x}={x:9});}return o.x===9",
        "var o={x:1},sum=0;with(o){for(x of [2,3]){PAUSE;sum+=x;}}return o.x===3&&sum===5",
        "var o={x:1},closed=0;with(o){try{PAUSE;throw 4}catch(e){x=e}finally{closed++}}return o.x===4&&closed===1",
        "var o={x:1},closed=0;try{with(o){PAUSE;return x===1}}finally{closed++}",
        "var o={x:1};with(o){PAUSE;delete x;}return !Object.hasOwn(o,'x')",
        "var o={x:3,m(){return this.x}};with(o){PAUSE;return m()===3}",
        "var o={x:3},x=8;with(o){PAUSE;o[Symbol.unscopables]={x:true};return x===8}",
        "var p={x:3},o=Object.create(p);with(o){PAUSE;p.x=6;return x===6}",
        "var o={x:1};outer:for(var j=0;j<3;j++){with(o){PAUSE;if(j===0)continue;break outer}}return o.x===1&&j===1",
        "var o={x:1},failed=false;try{with(null){PAUSE}}catch(e){failed=e instanceof TypeError}return failed",
        "with('abc'){PAUSE;return length===3}",
        "var o={x:3},outer=1;with(o){let x=8;PAUSE;return x===8&&o.x===3&&outer===1}",
        "var target={x:1},reads=0,o=new Proxy(target,{get(t,k,r){if(k==='x')reads++;return Reflect.get(t,k,r)}});with(o){var first=x;PAUSE;target.x=2;return first===1&&x===2&&reads===2}",
        "var o={x:1},reads=0,x=7;Object.defineProperty(o,Symbol.unscopables,{get(){reads++;return {x:reads>1}}});with(o){var first=x;PAUSE;return first===1&&x===7&&reads>=2}",
        "var log=[],o={x:7};with(o){{using resource={[Symbol.dispose](){log.push(x)}};PAUSE;}}return log.join(',')==='7'",
    ];
    let mut failures = Vec::new();
    for force_ast in [false, true] {
        for kind in ["async", "generator", "async-generator", "async-method", "async-arrow"] {
            for body in cases {
                let pause = if kind == "generator" || kind == "async-generator" { "yield 0" } else { "await 0" };
                let body = body.replace("PAUSE", pause);
                let expression = match kind {
                    "async" => format!("async function outer(){{{body}}}"),
                    "generator" => format!("function* outer(){{{body}}}"),
                    "async-generator" => format!("async function* outer(){{{body}}}"),
                    "async-method" => format!("({{async outer(){{{body}}}}}).outer"),
                    _ => format!("async ()=>{{{body}}}"),
                };
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut i = Interpreter::new();
                    i.force_ast_callables = force_ast;
                    i.run_script(&format!("var outer={expression};var result,failed=false;")).unwrap();
                    let Some(JsValue::Object(function)) = i.global_env.resolve("outer") else { unreachable!() };
                    assert_eq!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !force_ast);
                    let driver = match kind {
                        "generator" => "var g=outer(),step;do{step=g.next()}while(!step.done);result=step.value",
                        "async-generator" => "var g=outer();async function drive(){var step;do{step=await g.next()}while(!step.done);result=step.value}drive().catch(()=>failed=true)",
                        _ => "outer().then(v=>result=v,()=>failed=true)",
                    };
                    i.run_script(driver).unwrap();
                    i.drain_microtasks().unwrap();
                    assert_eq!(i.run_script("result===true&&!failed").unwrap(), JsValue::Bool(true));
                    assert!(i.frames.is_empty());
                    assert!(!i.strict);
                }));
                if outcome.is_err() { failures.push(format!("AST={force_ast} {kind}: {body}")); }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn suspended_with_restores_scope_on_external_return_and_throw() {
    for force_ast in [false, true] {
        for asynchronous in [false, true] {
            let mut i = Interpreter::new();
            i.force_ast_callables = force_ast;
            let prefix = if asynchronous { "async " } else { "" };
            i.run_script(&format!("var o={{x:7}},log=[],capture;var f={prefix}function*(){{with(o){{capture=()=>x;try{{yield x;yield x}}finally{{log.push(x)}}}}}};var g=f();")).unwrap();
            if asynchronous {
                i.run_script("g.next().then(v=>log.push(v.value));").unwrap();
                i.run_script("g.return(9).then(v=>log.push(v.value));").unwrap();
            } else {
                i.run_script("log.push(g.next().value);log.push(g.return(9).value)").unwrap();
            }
            assert_eq!(i.run_script("log.join(',')==='7,7,9'&&capture()===7&&typeof x==='undefined'").unwrap(), JsValue::Bool(true));
            assert!(i.frames.is_empty());
            i.run_script("log=[];g=f()").unwrap();
            if asynchronous {
                i.run_script("g.next().then(v=>log.push(v.value));").unwrap();
                i.run_script("g.throw(9).catch(v=>log.push(v));").unwrap();
            } else {
                i.run_script("log.push(g.next().value);try{g.throw(9)}catch(e){log.push(e)}").unwrap();
            }
            assert_eq!(i.run_script("log.join(',')==='7,7,9'&&typeof x==='undefined'").unwrap(), JsValue::Bool(true));
        }
    }
}

#[test]
fn with_head_suspension_enters_scope_after_resuming_object_evaluation() {
    for force_ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = force_ast;
        i.run_script("var capture;function* f(){with(yield 1){capture=()=>x;yield x;return x}}var g=f();var a=g.next();var b=g.next({x:7});var c=g.next();").unwrap();
        assert_eq!(i.run_script("a.value===1&&b.value===7&&!b.done&&c.value===7&&c.done&&capture()===7&&typeof x==='undefined'").unwrap(), JsValue::Bool(true));
        i.run_script("var out;async function h(){with(await Promise.resolve({x:8})){await 0;return x}}h().then(v=>out=v)").unwrap();
        assert_eq!(i.run_script("out===8&&typeof x==='undefined'").unwrap(), JsValue::Bool(true));
        assert!(i.frames.is_empty());
    }
}

#[test]
fn with_async_iteration_and_disposal_keep_original_object_environment() {
    for force_ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = force_ast;
        i.run_script(r#"
            var o={x:0},log=[],out,failed=false;
            async function f(){
                with(o){
                    {await using resource={[Symbol.asyncDispose]:async function(){await 0;log.push(x)}};
                     for await(var x of [Promise.resolve(2),Promise.resolve(4)]){await 0;log.push(x)}
                    }
                    return x===4;
                }
            }
            f().then(v=>out=v,()=>failed=true);
        "#).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("out===true&&!failed&&o.x===4&&log.join(',')==='2,4,4'&&typeof x==='undefined'").unwrap(), JsValue::Bool(true));
        assert!(i.frames.is_empty());
    }
}
