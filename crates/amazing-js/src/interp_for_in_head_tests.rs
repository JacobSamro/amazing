use super::*;

fn matrix(body: &str) {
    for ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = ast;
        i.run_script(&format!("var tested=function(){{{body}}};")).unwrap();
        let JsValue::Object(f) = i.run_script("tested").unwrap() else { panic!("callable") };
        assert_eq!(matches!(f.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !ast);
        assert_eq!(i.run_script("tested()").unwrap(), JsValue::Bool(true), "AST={ast}: {body}");
    }
}

#[test]
fn initializer_runs_once_before_rhs_even_when_enumeration_is_empty() {
    for rhs in ["{}", "null", "undefined", "false", "0", "''"] {
        matrix(&format!("var log=[],stored;for(var x=(log.push('init'),17) in (stored=x,log.push('rhs'),{rhs})){{log.push('body')}}return x===17&&stored===17&&log.join(',')==='init,rhs'"));
    }
    matrix("var effects=0,stored,iterations=0;for(var x=(++effects,-1) in (stored=x,{a:0,b:1,c:2}))iterations++;return effects===1&&stored===-1&&iterations===3&&x==='c'");
}

#[test]
fn head_reference_is_captured_without_get_before_initializer() {
    matrix("var x=1,reads=0,writes=0,stored=0,obj={get x(){reads++;return 2},set x(v){writes++;stored=v}};with(obj){for(var x=17 in {}){}}return reads===0&&writes===1&&stored===17&&x===1");
    matrix("var x=1,obj={x:2};with(obj){for(var x=(delete obj.x,17) in {}){}}return obj.x===17&&x===1");
    matrix("var x=1,obj={};with(obj){for(var x=(obj.x=2,17) in {}){}}return obj.x===2&&x===17");
    matrix("var x=1,obj={x:2,[Symbol.unscopables]:{x:true}};with(obj){for(var x=17 in {}){}}return obj.x===2&&x===17");
}

#[test]
fn anonymous_initializers_receive_binding_names_and_class_inner_names() {
    for initializer in ["function(){}", "function*(){}", "async function(){}", "async function*(){}", "()=>0", "class{ static value(){return x} }"] {
        matrix(&format!("for(var x={initializer} in {{}}){{}}return x.name==='x'"));
    }
    matrix("for(var x=class{ static value(){return x} } in {}){}var c=x;x=9;return c.name==='x'&&c.value()===9");
    matrix("for(var x=class Inner{static value(){return Inner}} in {}){}var c=x;x=9;return c.name==='Inner'&&c.value()===c");
    matrix("for(var x=function(){return x} in {}){}var f=x;x=9;return f.name==='x'&&f()===9");
    matrix("for(var x=function Inner(){return Inner} in {}){}var f=x;x=9;return f.name==='Inner'&&f()===f");
    matrix("for(var x=(0,function(){}) in {}){}return x.name===''");
}

#[test]
fn abrupt_head_steps_preserve_identity_and_stop_later_steps() {
    matrix("var marker={},rhs=false,caught=false,x=1;try{for(var x=(()=>{throw marker})() in (rhs=true,{})){} }catch(e){caught=e===marker}return caught&&!rhs&&x===1");
    matrix("var marker={},x=1,caught=false;try{for(var x=17 in (()=>{throw marker})()){} }catch(e){caught=e===marker}return caught&&x===17");
    matrix("var marker={},rhs=false,caught=false,obj={set x(v){throw marker}};with(obj){try{for(var x=17 in (rhs=true,{})){} }catch(e){caught=e===marker}}return caught&&!rhs");
}

#[test]
fn lexical_head_tdz_and_fresh_iterations_remain_separate_from_var_initializer() {
    for kind in ["let", "const"] {
        matrix(&format!("var x={{a:1}},caught=false;try{{for({kind} x in x){{}}}}catch(e){{caught=e instanceof ReferenceError}}return caught"));
        matrix(&format!("var closures=[];for({kind} x in {{a:1,b:2}})closures.push(()=>x);return closures[0]()==='a'&&closures[1]()==='b'"));
    }
    matrix("var closures=[];for(var x=17 in {a:1,b:2})closures.push(()=>x);return closures[0]()==='b'&&closures[1]()==='b'");
    matrix("var a,b;for(var [a,b] in {xy:1}){}return a==='x'&&b==='y'");
}

#[test]
fn enumeration_mutation_labels_and_completion_controls_keep_existing_contract() {
    matrix("var proto={a:1,b:2},obj=Object.create(proto);Object.defineProperty(obj,'a',{value:3,enumerable:false});obj.c=4;var keys=[];for(var x=17 in obj)keys.push(x);return keys.join(',')==='c,b'");
    matrix("var obj={a:1,b:2,c:3},keys=[];for(var x=17 in obj){keys.push(x);delete obj.b}return keys.join(',')==='a,c'");
    matrix("var count=0,init=0;outer:for(var x=++init in {a:1,b:2}){for(var y=++init in {c:1,d:2}){count++;continue outer}}return count===2&&init===3");
    matrix("var init=0,fin=0;outer:for(var x=++init in {a:1,b:2}){try{break outer}finally{fin++}}return init===1&&fin===1&&x==='a'");
}

#[test]
fn for_in_initializer_grammar_rejects_every_disallowed_head() {
    let invalid = ["'use strict';for(var x=1 in {}){}", "for(let x=1 in {}){}", "for(const x=1 in {}){}", "for(var [x]=[] in {}){}", "for(var {x}={} in {}){}", "for(var x=1,y in {}){}", "for(var x=1 of []){}", "async function f(){for await(var x=1 of []){}}", "for(using x=1 in {}){}"];
    for source in invalid { assert!(crate::parse_script(source).is_err(), "{source}"); }
    for source in ["for(var x=1 in {}){}", "for(var x in {}){}", "for(let x in {}){}", "for(const x in {}){}", "for(var [x] in {}){}", "for(var x=1;false;){}", "for(let x=1;false;){}"] { assert!(crate::parse_script(source).is_ok(), "{source}"); }
}

#[test]
fn initializer_and_rhs_suspension_preserve_captured_binding_in_all_callable_kinds() {
    for ast in [false, true] { for kind in ["generator", "async", "async-generator"] {
        let pause = if kind == "async" { "await 0" } else { "yield 0" };
        let source = format!("var obj={{x:2}},out,done=false,error;var f={prefix}function{star}(){{var x=1;with(obj){{for(var x=({pause},delete obj.x,17) in ({pause},{{}})){{}}}}return obj.x===17&&x===1}};",prefix=if kind=="generator"{""}else{"async "},star=if kind=="async"{""}else{"*"});
        let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();
        let JsValue::Object(f)=i.run_script("f").unwrap() else {panic!("callable")};
        assert_eq!(matches!(f.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!ast);
        i.run_script(match kind {"generator"=>"var g=f(),r;do{r=g.next()}while(!r.done);out=r.value;done=true", "async-generator"=>"var g=f();async function drive(){var r;do{r=await g.next()}while(!r.done);return r.value}drive().then(v=>{out=v;done=true},e=>error=e)",_=>"f().then(v=>{out=v;done=true},e=>error=e)"}).unwrap();i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("done&&error===undefined&&out===true").unwrap(),JsValue::Bool(true),"AST={ast} {kind}");
    }}
}

#[test]
fn all_attached_for_in_labels_route_continue_break_and_finally() {
    matrix("var n=0,init=0;a:b:for(var x=++init in {p:1,q:2,r:3}){n++;if(n===1)continue a;if(n===2)continue b;break a}return n===3&&init===1&&x==='r'");
    matrix("var n=0,fin=0;a:b:for(var x=0 in {p:1,q:2}){try{n++;continue a}finally{fin++}}return n===2&&fin===2");
    matrix("var n=0,fin=0;outer:for(var x=0 in {p:1,q:2}){inner:for(var y=0 in {r:1,s:2}){try{n++;continue outer}finally{fin++}}}return n===2&&fin===2");
    matrix("var n=0;a:b:for(let x in {p:1,q:2}){n++;break b}return n===1");
}

#[test]
fn attached_labels_survive_generator_async_and_async_generator_body_suspension() {
    for ast in [false,true] {for kind in ["generator","async","async-generator"] {
        let pause=if kind=="async"{"await 0"}else{"yield 0"};
        let source=format!("var out,done=false,error;var f={prefix}function{star}(){{var n=0,fin=0,init=0;a:b:for(var x=++init in {{p:1,q:2}}){{try{{{pause};n++;continue a}}finally{{fin++}}}}return n===2&&fin===2&&init===1}};",prefix=if kind=="generator"{""}else{"async "},star=if kind=="async"{""}else{"*"});
        let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();
        let JsValue::Object(f)=i.run_script("f").unwrap() else {panic!("callable")};assert_eq!(matches!(f.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!ast);
        i.run_script(match kind {"generator"=>"var g=f(),r;do{r=g.next()}while(!r.done);out=r.value;done=true", "async-generator"=>"var g=f();async function drive(){var r;do{r=await g.next()}while(!r.done);return r.value}drive().then(v=>{out=v;done=true},e=>error=e)",_=>"f().then(v=>{out=v;done=true},e=>error=e)"}).unwrap();i.drain_microtasks().unwrap();assert_eq!(i.run_script("done&&error===undefined&&out===true").unwrap(),JsValue::Bool(true),"AST={ast} {kind}");
    }}
}
