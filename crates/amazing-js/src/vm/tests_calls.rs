use super::*;

fn callable(i: &Interpreter, ast: bool) {
    let Some(JsValue::Object(f))=i.global_env.resolve("f") else {panic!("missing f")};
    assert_eq!(matches!(f.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!ast);
}

#[test]
fn contextual_calls_preserve_references_and_eval_across_argument_suspension() {
    let cases=[
        ("return (o.m)(PAUSE)","2","19"),
        ("return ((o.m))(PAUSE)","2","19"),
        ("return (o[PAUSE])(2)","'m'","19"),
        ("return (0,o.m)(PAUSE)","2","-1"),
        ("return (true?o.m:o.m)(PAUSE)","2","-1"),
        ("return (PAUSE)()","o.m","-1"),
        ("return eval(PAUSE)","'x'","17"),
        ("return (eval)(PAUSE)","'x'","17"),
        ("return ((eval))(PAUSE)","'x'","17"),
        ("return eval('x',PAUSE)","2","17"),
        ("return eval(...(PAUSE))","['x']","17"),
        ("return eval?.(PAUSE)","'typeof x'","'undefined'"),
        ("return (0,eval)(PAUSE)","'typeof x'","'undefined'"),
        ("with(o){return (m)(PAUSE)}","2","19"),
        ("with({eval:globalThis.eval}){return (eval)(PAUSE)}","'x'","17"),
        ("let eval=v=>v+1;return (eval)(PAUSE)","2","3"),
    ];
    let mut failures=Vec::new();
    for ast in [false,true] { for kind in ["generator","async","async-generator"] {
        for (body,input,expected) in cases {
            let body=body.replace("PAUSE",if kind=="async" {"await input"} else {"yield input"});
            let source=format!("var o={{v:17,m(v){{return this===o?this.v+(v||0):-1}}}},input={input},out,done=false,error;var f={prefix}function{star}(){{let x=17;{body}}};",
                prefix=if kind=="generator" {""} else {"async "},star=if kind=="async" {""} else {"*"});
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();callable(&i,ast);
                let driver=match kind {
                    "generator"=>"var g=f(),r=g.next();if(!r.done)r=g.next(input);out=r.value;done=r.done",
                    "async-generator"=>"var g=f();async function drive(){var r=await g.next();if(!r.done)r=await g.next(input);return r.value}drive().then(v=>{out=v;done=true},e=>{error=e})",
                    _=>"f().then(v=>{out=v;done=true},e=>{error=e})",
                };
                i.run_script(driver).unwrap();i.drain_microtasks().unwrap();
                assert_eq!(i.run_script(&format!("done&&error===undefined&&out===({expected})")).unwrap(),JsValue::Bool(true),"{:?}",i.run_script("JSON.stringify([done,String(error),out])").unwrap());
            }));
            if result.is_err(){failures.push(format!("AST={ast} {kind} {body}"));}
        }
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn call_callee_is_captured_before_arguments_and_abrupt_resume_restores_context() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var log=[],o={v:17,get m(){log.push('get');return function(v){return this.v+v}}},out;
            var f=function*(){return (o.m)(yield 'arg')};
            var g=f();g.next();Object.defineProperty(o,'m',{value:()=>90});out=g.next(2).value;
            var marker={},caught=false,fin=false;
            function* h(){let x=17;try{return (eval)(yield 'source')}catch(e){caught=e===marker}finally{fin=true}}
            var it=h();it.next();it.throw(marker);
        "#).unwrap();callable(&i,ast);
        assert_eq!(i.run_script("out===19&&log.join(',')==='get'&&caught&&fin&&eval('typeof x')==='undefined'").unwrap(),JsValue::Bool(true));
        assert!(!i.direct_eval_call);assert!(i.direct_eval_env.is_none());
    }
}

#[test]
fn suspended_super_and_private_call_receivers_are_preserved() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
            var log=[];class B{get m(){log.push(this.v);return function(x){return this.v+x}}}
            class C extends B{v=17;#m(x){return this.v+x}*a(){return (super.m)(yield 2)}*b(){return (this.#m)(yield 2)}}
            var c=new C,a=c.a(),b=c.b();a.next();b.next();a.next(2).value===19&&b.next(2).value===19&&log.join(',')==='17';
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}
