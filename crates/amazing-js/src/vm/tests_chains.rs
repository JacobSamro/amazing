use super::*;

#[test]
fn suspended_optional_chains_preserve_groups_receivers_and_skipped_evaluation() {
    // Each case contains syntactic suspension even when the nullish branch
    // skips it. Verify factory selection as well as observable semantics.
    let cases = [
        ("o?.m((PAUSE,3))", "out===7", "m"),
        ("o.m?.((PAUSE,3))", "out===7", "m"),
        ("(o?.m)((PAUSE,3))", "out===7", "m"),
        ("(o?.m)?.((PAUSE,3))", "out===7", "m"),
        ("(0,o?.strict)((PAUSE,3))", "out===true", ""),
        ("nil?.[(PAUSE,'m')]((PAUSE,3))", "out===undefined", ""),
        ("nil?.m((PAUSE,3)).x", "out===undefined", ""),
        ("nil?.m(...(PAUSE,spread))", "out===undefined", ""),
        ("o?.m(...(PAUSE,[3]))", "out===7", "m"),
        ("o?.[(PAUSE,'m')]((PAUSE,3))", "out===7", "m"),
        ("o?.child?.[(PAUSE,'x')]", "out===9", ""),
        ("o?.missing?.[(PAUSE,'x')]", "out===undefined", ""),
        ("o?.m((PAUSE,3))?.toString()", "out==='7'", "m"),
        ("delete nil?.[(PAUSE,'x')]", "out===true", ""),
        ("delete o?.[(PAUSE,'victim')]", "out===true&&!('victim' in o)", ""),
        ("delete (o?.[(PAUSE,'victim')])", "out===true&&!('victim' in o)", ""),
        ("delete o?.m((PAUSE,3))", "out===true", "m"),
        ("eval?.((PAUSE,'typeof local'))", "out==='undefined'", ""),
    ];
    let mut failures = Vec::new();
    for force_ast in [false, true] {
        for kind in ["async", "generator", "async-generator"] {
            for (expression, expected, log) in cases {
                let pause = if kind == "async" { "await 0" } else { "yield 0" };
                let expression = expression.replace("PAUSE", pause);
                let source = format!(r#"
                    var log=[],nil=null,out,done=false,failed=false;
                    var spread={{get [Symbol.iterator](){{throw 'skipped spread'}}}};
                    var o={{n:4,child:{{x:9}},victim:1,m(v){{log.push('m');return this.n+v}},
                        strict:function(){{'use strict';return this===undefined}}}};
                    var f={prefix}function{star}(){{var local=1;return {expression}}};
                "#,
                    prefix=if kind == "generator" { "" } else { "async " },
                    star=if kind == "async" { "" } else { "*" });
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut i = Interpreter::new();
                    i.force_ast_callables = force_ast;
                    i.run_script(&source).unwrap();
                    let Some(JsValue::Object(function)) = i.global_env.resolve("f") else { unreachable!() };
                    assert_eq!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !force_ast);
                    let driver = match kind {
                        "generator" => "var g=f(),r;do{r=g.next(1)}while(!r.done);out=r.value;done=true",
                        "async-generator" => "var g=f();async function drive(){var r;do{r=await g.next(1)}while(!r.done);return r.value}drive().then(v=>{out=v;done=true},e=>{out=e;failed=true;done=true})",
                        _ => "f().then(v=>{out=v;done=true},e=>{out=e;failed=true;done=true})",
                    };
                    i.run_script(driver).unwrap();
                    i.drain_microtasks().unwrap();
                    assert_eq!(i.run_script(&format!("done&&!failed&&({expected})&&log.join(',')==='{log}'")).unwrap(), JsValue::Bool(true),
                        "{:?}", i.run_script("JSON.stringify([out,done,failed,log])").unwrap());
                }));
                if outcome.is_err() { failures.push(format!("AST={force_ast} {kind}: {expression}")); }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn suspended_optional_chain_group_errors_and_delete_traps_keep_order() {
    for force_ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = force_ast;
        i.run_script(r#"
            var log=[],nil=null,marker={},ok=false;
            var p=new Proxy({}, {get(){log.push('get');throw marker},
                deleteProperty(target,key){log.push('delete:'+key);return true}});
            var key={toString(){log.push('key');return 'x'}};
            async function f(){
                try{(nil?.m)((await 0,log.push('arg')))}catch(e){log.push(e instanceof TypeError?'type':'wrong')}
                try{(nil?.x)[(await 0,log.push('computed'),'x')]}catch(e){log.push(e instanceof TypeError?'type':'wrong')}
                return delete p?.[(await 0,key)];
            }
            f().then(v=>{ok=v},e=>{log.push('wrong rejection')});
        "#).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok&&log.join(',')==='arg,type,computed,type,key,delete:x'").unwrap(), JsValue::Bool(true), "AST={force_ast}");
    }
}

#[test]
fn suspended_optional_chain_super_private_and_reference_roots_survive_resume() {
    for force_ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = force_ast;
        i.run_script(r#"
            var log=[],ok=false;
            var proto={get m(){log.push(this.tag);return function(v){return this.tag+v}}};
            var method={__proto__:proto,tag:'child',async f(){return super[(await 0,'m')]?.((await 0,'!'))}};
            class C{#m(v){return this.tag+v}constructor(){this.tag='private'}
                async f(o){return o?.#m((await 0,'!'))}}
            var c=new C();
            var original={tag:'original',m(v){return this.tag+v}},o=original;
            function* captured(){return o?.m((yield 'pause','!'))}
            var g=captured(),first=g.next();o={tag:'replacement'};
            var second=g.next();
            Promise.all([method.f(),c.f(c),c.f(null)]).then(values=>{
                ok=values[0]==='child!'&&values[1]==='private!'&&values[2]===undefined&&
                    first.value==='pause'&&second.done&&second.value==='original!'&&log.join(',')==='child';
            });
        "#).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok").unwrap(), JsValue::Bool(true), "AST={force_ast}");
    }
}

#[test]
fn suspended_optional_delete_preserves_strict_failures_and_skips_getters() {
    for force_ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = force_ast;
        i.run_script(r#"
            var count=0,ok=false,p={};
            Object.defineProperty(p,'x',{get(){count++;throw 'getter'},configurable:false});
            async function sloppy(){return delete p?.[(await 0,'x')]}
            async function strict(){'use strict';try{return delete p?.[(await 0,'x')]}catch(e){return e instanceof TypeError}}
            Promise.all([sloppy(),strict()]).then(values=>{ok=values[0]===false&&values[1]===true&&count===0});
        "#).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok").unwrap(), JsValue::Bool(true), "AST={force_ast}");
    }
}
