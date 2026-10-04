use super::*;

fn assert_template_factory(value: JsValue, ast: bool) {
    let JsValue::Object(function)=value else {panic!("function")};
    let function=function.borrow();
    match &function.func.as_ref().unwrap().body {
        // Keep the tests-only before source compilable before the opcode is
        // added, while proving the after path uses it rather than TreeEval.
        FunctionBody::Bytecode(code) if !ast => assert!(code.0.program.code.iter().any(|op|format!("{op:?}").starts_with("TemplateObject(")),"actual template opcode required"),
        FunctionBody::Block(_) if ast => {}
        other=>panic!("unexpected template factory AST={ast}: {other:?}"),
    }
}

#[test]
fn tagged_template_registers_preserve_reference_boundaries_and_raw_substitution_values() {
    let cases=[
        ("return o.tag`a${VALUE}b`","19"),
        ("return (o.tag)`a${VALUE}b`","19"),
        ("return ((o.tag))`a${VALUE}b`","19"),
        ("return o[KEY]`a${VALUE}b`","19"),
        ("return o['tag']`a${VALUE}b`","19"),
        ("return (0,o.tag)`a${VALUE}b`","-1"),
        ("return (true?o.tag:o.tag)`a${VALUE}b`","-1"),
        ("return (TAG)`a${VALUE}b`","-1"),
        ("return (o?.tag)`a${VALUE}b`","19"),
        ("with(o){return (tag)`a${VALUE}b`}","19"),
        ("return o.makeTag()`a${VALUE}b`","-1"),
        ("return eval`a${VALUE}b`","array"),
    ];
    let mut failures=Vec::new();
    for ast in [false,true] {for kind in ["generator","async","async-generator"] {
        for (body,expected) in cases {
            let mut inputs=Vec::new();
            if body.contains("KEY"){inputs.push("key");}
            if body.contains("TAG"){inputs.push("o.tag");}
            inputs.push("marker");
            let operation=if kind=="async" {"await"} else {"yield"};
            let body=body.replace("KEY",&format!("({operation} key)"))
                .replace("TAG",&format!("({operation} o.tag)"))
                .replace("VALUE",&format!("({operation} marker)"));
            let source=format!(r#"
                var marker={{toString(){{throw 'coercion'}},valueOf(){{throw 'coercion'}}}},key='tag',out,done=false,error;
                var o={{tag(s,x){{if(x!==marker||s[0]!=='a'||s[1]!=='b'||s.raw[0]!=='a')throw 'arguments';return this===o?19:-1}},makeTag(){{return this.tag}}}};
                var inputs=[{inputs}],f={prefix}function{star}(){{{body}}};
            "#,inputs=inputs.join(","),prefix=if kind=="generator" {""} else {"async "},star=if kind=="async" {""} else {"*"});
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();
                assert_template_factory(i.run_script("f").unwrap(),ast);
                let driver=match kind {
                    "generator"=>"var g=f(),r=g.next(),j=0;while(!r.done&&j<inputs.length)r=g.next(inputs[j++]);out=r.value;done=r.done",
                    "async"=>"f().then(v=>{out=v;done=true},e=>error=e)",
                    _=>"var g=f(),j=0;g.next().then(function step(r){if(!r.done)return g.next(inputs[j++]).then(step);out=r.value;done=true}).catch(e=>error=e)",
                };
                i.run_script(driver).unwrap();i.drain_microtasks().unwrap();
                let expected=if expected=="array" {"Array.isArray(out)&&out[0]==='a'&&out[1]==='b'&&out.raw[0]==='a'".to_owned()}
                    else {format!("out===({expected})")};
                assert_eq!(i.run_script(&format!("done&&error===undefined&&({expected})")).unwrap(),JsValue::Bool(true),"{:?}",i.run_script("JSON.stringify([done,String(error),out])").unwrap());
            }));
            if result.is_err(){failures.push(format!("AST={ast} {kind} {body}"));}
        }
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn template_callee_is_captured_once_before_substitutions_and_injected_throw_restores_finally() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var log=[],marker={},ok=false,interrupted=false,site,o={v:17,get tag(){log.push('get');return function(s,x){site=s;return this===o&&x===marker}}};
            var f=function*(){try{return ((o.tag))`a${yield 'value'}b`}finally{log.push('finally')}};
            var g=f();g.next();Object.defineProperty(o,'tag',{value:()=>false});ok=g.next(marker).value;
            var h=function*(){try{return o.tag`a${yield 'value'}b`}catch(e){interrupted=e===marker}finally{log.push('interrupted')}};
            var it=h();it.next();it.throw(marker);
            var returned=false,k=function*(){try{return o.tag`a${yield 'value'}b`}finally{returned=true;log.push('returned')}};
            var ret=k();ret.next();var final=ret.return(99);
        "#).unwrap();
        assert_template_factory(i.run_script("f").unwrap(),ast);
        assert_template_factory(i.run_script("k").unwrap(),ast);
        assert_eq!(i.run_script("ok&&interrupted&&returned&&final.done&&final.value===99&&Object.isFrozen(site)&&Object.isFrozen(site.raw)&&log.join(',')==='get,finally,interrupted,returned'").unwrap(),JsValue::Bool(true),"AST={ast}");
        assert!(!i.direct_eval_call);assert!(i.direct_eval_env.is_none());
    }
}

#[test]
fn template_callee_receiver_and_site_survive_single_instruction_quantums() {
    let mut i=Interpreter::new();
    i.run_script(r#"
        var log=[],out,o={get tag(){log.push('get');return function(s,a,b){log.push('call');return this===o&&a===1&&b===2&&Object.isFrozen(s)&&Object.isFrozen(s.raw)}}};
        function one(){log.push('one');return 1}function two(){log.push('two');return 2}
    "#).unwrap();
    let code=compile(&crate::parse_script("out=(o.tag)`a${one()}b${two()}c`;out").unwrap(),false).unwrap();
    let env=i.global_env.clone();let mut frame=code.start_frame(&mut i,env,1000).unwrap();
    let mut changed=false;let mut checkpoints=0;
    loop {
        match code.resume_frame(&mut i,&mut frame,1).unwrap() {
            FrameOutcome::Suspended=>{
                checkpoints+=1;
                if !changed&&i.run_script("log.join(',')==='get'").unwrap()==JsValue::Bool(true) {
                    i.run_script("Object.defineProperty(o,'tag',{value:()=>false})").unwrap();changed=true;
                }
            }
            FrameOutcome::Completed(value)=>{assert_eq!(value,JsValue::Bool(true));break;}
            _=>panic!("unexpected template quantum outcome"),
        }
    }
    assert!(changed&&checkpoints>4);
    assert_eq!(i.run_script("log.join(',')==='get,one,two,call'").unwrap(),JsValue::Bool(true));
}

#[test]
fn private_and_super_tags_keep_actual_this_and_native_site_opcode_across_yield() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var log=[];class B{get tag(){log.push('get:'+this.v);return function(s,x){log.push('call:'+this.v);return this.v+x}}}
            class C extends B{v=17;#tag(s,x){return this.v+x}*a(){return (super.tag)`a${yield 2}b`}*b(){return (this.#tag)`a${yield 2}b`}}
            var c=new C,a=c.a(),b=c.b();a.next();b.next();var outA=a.next(2).value,outB=b.next(2).value;
        "#).unwrap();
        assert_template_factory(i.run_script("C.prototype.a").unwrap(),ast);
        assert_template_factory(i.run_script("C.prototype.b").unwrap(),ast);
        assert_eq!(i.run_script("outA===19&&outB===19&&log.join(',')==='get:17,call:17'").unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}
