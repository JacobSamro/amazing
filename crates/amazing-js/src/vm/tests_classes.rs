use super::*;

fn compiled(i: &Interpreter, name: &str, expected: bool) {
    let Some(JsValue::Object(value)) = i.global_env.resolve(name) else { panic!("missing {name}") };
    assert_eq!(matches!(value.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), expected, "{name}");
}

#[test]
fn suspended_classes_keep_fields_private_brands_and_static_order() {
    let cases = [
        ("class C{[(PAUSE,'x')]=7;#p=9;read(){return this.#p};static{log.push('static')}}", "v.x===7&&v.read()===9&&log.join(',')==='static'"),
        ("class C extends (PAUSE,Base){[(PAUSE,'x')]=7;static{log.push('static')}}", "v.x===7&&v.base===3&&log.join(',')==='static'"),
        ("class C{static [(PAUSE,'x')]=3;[(PAUSE,'y')]=C.x;static{C.x++;log.push('static')}}", "v.y===4&&out.x===4&&log.join(',')==='static'"),
        ("class C{[(PAUSE,key)]=1;[(PAUSE,key)]=2}", "v.x===2&&log.join(',')==='key,key'"),
        ("class C{[(PAUSE,'x')]=1;#m(){return 9};read(){return this.#m()};static #s=4;static read(){return this.#s}}", "v.read()===9&&out.read()===4"),
        ("class C{[(PAUSE,'x')]=1;static name='override';static{log.push(C.name)}}", "out.name==='override'&&log.join(',')==='override'"),
        ("class C extends (PAUSE,holder.base){[(PAUSE,'x')]=1}", "v.base===3&&log.join(',')==='get'"),
        ("class C extends holder?.[(PAUSE,'base')]{[(PAUSE,'x')]=1}", "v.base===3&&log.join(',')==='get'"),
        ("class C{[(PAUSE,symbol)](){};static [(PAUSE,'z')](){return C}}", "v[symbol].name==='[method]'&&out.z()===out"),
        ("class C{[(PAUSE,'x')]=(()=>C);static accessor a=3;accessor b=4}", "v.x()===out&&out.a===3&&v.b===4"),
        ("class C{[(PAUSE,(log.push('outer'),class D{static{log.push('inner')}},'x'))]=1}", "v.x===1&&log.join(',')==='outer,inner'"),
    ];
    let mut failures = Vec::new();
    for ast in [false, true] {
        for kind in ["async", "generator", "async-generator"] {
            for (expression, expected) in cases {
                let expression = expression.replace("PAUSE", if kind == "async" { "await 0" } else { "yield 0" });
                let source = format!(r#"
                    var log=[],out,done=false,error;
                    var symbol=Symbol('method'),key={{toString(){{log.push('key');return 'x'}}}};
                    class Base{{constructor(){{this.base=3}}}}
                    var holder={{get base(){{log.push('get');return Base}}}};
                    var f={prefix}function{star}(){{return {expression}}};
                "#, prefix=if kind=="generator" { "" } else { "async " }, star=if kind=="async" { "" } else { "*" });
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut i = Interpreter::new(); i.force_ast_callables = ast;
                    i.run_script(&source).unwrap(); compiled(&i,"f",!ast);
                    let driver = match kind {
                        "generator" => "var g=f(),r;do{r=g.next(1)}while(!r.done);out=r.value;done=true",
                        "async-generator" => "var g=f();async function drive(){var r;do{r=await g.next(1)}while(!r.done);return r.value}drive().then(v=>{out=v;done=true},e=>{error=e})",
                        _ => "f().then(v=>{out=v;done=true},e=>{error=e})",
                    };
                    i.run_script(driver).unwrap(); i.drain_microtasks().unwrap();
                    assert_eq!(i.run_script(&format!("var v=done?new out:{{}};done&&error===undefined&&({expected})")).unwrap(),JsValue::Bool(true),
                        "{:?}",i.run_script("JSON.stringify([done,String(error),log])").unwrap());
                }));
                if result.is_err() { failures.push(format!("AST={ast} {kind}: {expression}")); }
            }
        }
    }
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn suspended_class_inner_tdz_and_private_heritage_names_are_distinct() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var captures=[],first,second,tdz=false,cross=false;
            function* f(){return class C{[(captures.push(()=>C),yield 'key','x')]=1;#p=9;read(){return this.#p}}}
            var a=f();a.next();try{captures[0]()}catch(e){tdz=e instanceof ReferenceError}
            first=a.next().value;var b=f();b.next();second=b.next().value;
            try{first.prototype.read.call(new second)}catch(e){cross=e instanceof TypeError}
            class Base{}
            class Outer{#Base=Base;*make(){return class Inner extends this.#Base{[(yield 'key','x')]=1;#Base=7}}}
            var c=new Outer().make();c.next();var Child=c.next().value;
        "#).unwrap();compiled(&i,"f",!ast);
        assert_eq!(i.run_script("tdz&&cross&&captures[0]()===first&&captures[1]()===second&&new Child instanceof Base").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn class_abrupt_suspension_restores_with_scope_and_sloppy_finalizers() {
    for ast in [false,true] {
        for abrupt in ["return", "throw"] {
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            i.run_script(r#"
                var log=[],object={x:'a'},marker={},caught=false;
                function* f(){with(object){try{using r={[Symbol.dispose](){log.push('dispose')}};
                    return class C{[(yield 'key',x)]=1};
                }catch(e){caught=e===marker;outsideCatch=1}finally{outsideFinally=2;log.push(x)}}}
                var g=f();g.next();object.x='b';
            "#).unwrap();compiled(&i,"f",!ast);
            let source=if abrupt=="return" { "var r=g.return(marker);var correct=r.done&&r.value===marker&&!caught" }
                else { "var r=g.throw(marker);var correct=r.done&&caught" };
            i.run_script(source).unwrap();
            assert_eq!(i.run_script("correct&&outsideFinally===2&&log.join(',')==='dispose,b'").unwrap(),JsValue::Bool(true));
            if abrupt=="throw" { assert_eq!(i.run_script("outsideCatch===1").unwrap(),JsValue::Bool(true)); }
            assert!(!i.strict_eval_context);
        }
    }
}

#[test]
fn suspended_class_strictness_and_heritage_errors_keep_original_throw() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var log=[],marker={},done=false;
            async function f(){
                try{class C extends (await 0,C){[(log.push('unreachable'),'x')]=1}}catch(e){log.push(e instanceof ReferenceError?'tdz':'wrong')}
                try{class C{[(await 0,notDeclared=1,'x')]=1}}catch(e){log.push(e instanceof ReferenceError?'strict':'wrong')}
                try{class C extends (await 0,{get prototype(){throw marker}}){}}catch(e){log.push(e instanceof TypeError?'constructor':'wrong')}
                try{class C extends (await 0,Base){static [(await 0,'prototype')]=1}}catch(e){log.push(e instanceof TypeError?'prototype':'wrong')}
                afterClass=3;return true;
            }
            class Base{};f().then(v=>done=v);
        "#).unwrap();compiled(&i,"f",!ast);i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("done&&afterClass===3&&typeof notDeclared==='undefined'&&log.join(',')==='tdz,strict,constructor,prototype'").unwrap(),JsValue::Bool(true));
        assert!(!i.strict_eval_context);
    }
}

#[test]
fn rooted_class_phases_survive_every_instruction_quantum() {
    let ast=crate::parser::parse_script("function* f(){return class C{[(yield 0,'x')]=1;#p=2;read(){return this.#p};static{}}}").unwrap();
    let crate::ast::StmtKind::Function(function)=&ast.body[0].kind else{unreachable!()};
    let code=compiler::compile_function_item(function,ast.body[0].span,false).unwrap();
    let mut i=Interpreter::new();let env=i.global_env.clone();
    let mut frame=code.0.program.start_frame(&mut i,env,10000).unwrap();let mut stops=0;let mut yields=0;
    let value=loop{match code.0.program.resume_frame(&mut i,&mut frame,1).unwrap(){
        FrameOutcome::Suspended=>stops+=1,
        FrameOutcome::Yielded(_)=>{yields+=1;let dst=frame.yield_dst.take().unwrap();frame.registers[dst]=JsValue::Undefined;},
        FrameOutcome::Completed(value)=>break value,
        _=>panic!("unexpected outcome"),
    }assert!(!i.strict_eval_context);};
    assert!(stops>10);assert_eq!(yields,1);assert!(frame.classes.iter().all(Option::is_none));
    assert!(frame.scopes.is_empty());assert!(frame.strict_stack.is_empty());assert!(!frame.strict_context);
    i.global_env.declare_var("C",value).unwrap();
    assert_eq!(i.run_script("var c=new C;c.x===1&&c.read()===2").unwrap(),JsValue::Bool(true));
}

#[test]
fn suspended_class_decorators_keep_receiver_keys_and_extra_initializers() {
    for ast in [false,true] {
        for kind in ["async","generator","async-generator"] {
            let pause=if kind=="async"{"await 0"}else{"yield 0"};
            let source=format!(r#"
                var receivers=0,keys=0,extras=[],out,done=false,error;
                var object={{get dec(){{return function(value,context){{
                    if(this===object)receivers++;
                    context.addInitializer(function(){{extras.push(context.kind)}});
                    if(context.kind==='field')return v=>v+1;
                }}}}}};
                var key={{toString(){{keys++;return 'field'}}}};
                var f={prefix}function{star}(){{return @(({pause},object).dec) class C{{
                    @(({pause},object).dec) [({pause},key)]=3;
                    static value=9;static{{this.self=C}}
                }}};
            "#,prefix=if kind=="generator"{""}else{"async "},star=if kind=="async"{""}else{"*"});
            let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();compiled(&i,"f",!ast);
            let drive=match kind{
                "generator"=>"var g=f(),r;do{r=g.next(1)}while(!r.done);out=r.value;done=true",
                "async-generator"=>"var g=f();async function drive(){var r;do{r=await g.next(1)}while(!r.done);return r.value}drive().then(v=>{out=v;done=true},e=>error=e)",
                _=>"f().then(v=>{out=v;done=true},e=>error=e)",
            };
            i.run_script(drive).unwrap();i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("var v=new out;done&&error===undefined&&receivers===2&&keys===1&&v.field===4&&out.value===9&&out.self===out&&extras.join(',')==='class,field'").unwrap(),JsValue::Bool(true),"AST={ast} {kind}");
        }
    }
}

#[test]
fn suspended_class_await_rejections_restore_disposal_and_original_identity() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var marker={},log=[],done=false,finish;
            var pending=new Promise((r,j)=>finish=j);
            async function f(){try{await using resource={[Symbol.asyncDispose]:async function(){await 0;log.push('dispose')}};
                return class C{[(await pending,'x')]=1};
            }catch(e){sloppyAfter=1;log.push(e===marker?'caught':'wrong');return true}finally{log.push('finally')}}
            f().then(v=>done=v);log.push('before');finish(marker);
        "#).unwrap();compiled(&i,"f",!ast);i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("done&&sloppyAfter===1&&log.join(',')==='before,dispose,caught,finally'").unwrap(),JsValue::Bool(true));
        assert!(!i.strict_eval_context);
    }
}

#[test]
fn suspended_class_async_body_realm_and_names_survive_borrowed_calls() {
    for ast in [false,true] {
        for generator in [false,true] {
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            let child=i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();
            let source=if generator{"(async function*(){return class C{[(yield 0,'x')]=[];#p=3;read(){return this.#p};static{this.arr=[]}}})"}
                else{"(async function(){return class C{[(await 0,'x')]=[];#p=3;read(){return this.#p};static{this.arr=[]}}})"};
            let f=i.binding_eval_in_realm(child,source).unwrap();
            i.global_env.declare_var("f",f).unwrap();compiled(&i,"f",!ast);
            let array=i.binding_eval_in_realm(child,"Array.prototype").unwrap();
            let object=i.binding_eval_in_realm(child,"Object.prototype").unwrap();
            let function=i.binding_eval_in_realm(child,"Function.prototype").unwrap();
            i.global_env.declare_var("childArray",array).unwrap();
            i.global_env.declare_var("childObject",object).unwrap();
            i.global_env.declare_var("childFunction",function).unwrap();
            let drive=if generator{"var out,done=false;var next=Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())).next;var g=f();next.call(g).then(()=>next.call(g)).then(r=>{out=r.value;done=r.done})"}
                else{"var out,done=false;f.call(null).then(v=>{out=v;done=true})"};
            i.run_script(drive).unwrap();i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("var v=new out;done&&out.name==='C'&&Object.getPrototypeOf(out)===childFunction&&Object.getPrototypeOf(out.prototype)===childObject&&Object.getPrototypeOf(v.x)===childArray&&Object.getPrototypeOf(out.arr)===childArray&&v.read()===3").unwrap(),JsValue::Bool(true),"AST={ast} generator={generator}");
        }
    }
}
