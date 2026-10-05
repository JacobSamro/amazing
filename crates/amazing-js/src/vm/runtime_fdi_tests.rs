use super::*;
use crate::parse_script;

#[test]
fn function_body_lexical_instantiation_reserves_tdz_and_captures_direct_functions() {
    let cases = [
        ("", "return delete local===false;let local=1;"),
        ("", "try{return local}catch(e){return e instanceof ReferenceError}let local=1;"),
        ("", "try{return typeof local}catch(e){return e instanceof ReferenceError}const local=1;"),
        ("", "try{return Local}catch(e){return e instanceof ReferenceError}class Local{}"),
        ("", "try{return local}catch(e){return e instanceof ReferenceError}using local=null;"),
        ("", "let before;try{x}catch(e){before=e instanceof ReferenceError}let {x,y:[z]}={x:2,y:[3]};return before&&x===2&&z===3;"),
        ("", "let before;try{x}catch(e){before=e instanceof ReferenceError}const [x,...rest]=[2,3];return before&&x===2&&rest[0]===3;"),
        ("", "function read(){return x}let before;try{read()}catch(e){before=e instanceof ReferenceError}let x=7;return before&&read()===7;"),
        ("", "function read(){return x}const x=7;return read()===7;"),
        ("", "function read(){return Local}class Local{}return read()===Local;"),
        ("", "let arguments=7;return arguments===7;"),
        ("a", "a=7;return arguments[0]===7;"),
        ("", "return typeof nested==='undefined';{let nested=1;}"),
        ("", "let caught=false;try{eval('var x=7')}catch(e){caught=e instanceof SyntaxError}let x=2;return caught&&x===2;"),
        ("get=eval('var x=1; ()=>x')", "let x=2;return get()===1&&x===2;"),
        ("get=eval('var x=1; ()=>x')", "const x=2;return get()===1&&x===2;"),
        ("get=eval('var x=1; ()=>x')", "class x{}return get()===1&&typeof x==='function';"),
        ("get=()=>arguments", "let arguments=7;return get()[1]===3&&arguments===7;"),
        ("", "'use strict';function read(){return x}let x=7;return read()===7;"),
        ("", "'use strict';try{return local}catch(e){return e instanceof ReferenceError}let local=1;"),
        ("", "'use strict';const [x]=[7];return x===7;"),
    ];
    let mut failures=Vec::new();
    for (params,body) in cases { for ast in [false,true] { for kind in ["ordinary","generator","async","async-generator"] {
        let source=format!("var out,error,done=false,local=91,x=91,Local=91;var home={{{async_prefix}{star}m({params}){{{body}}}}};",async_prefix=if kind.starts_with("async"){"async "}else{""},star=if kind.contains("generator"){"*"}else{""});
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();
            let JsValue::Object(home)=i.run_script("home.m").unwrap() else{panic!("method")};
            // Class expressions retain their explicit whole-class adapter.
            // This proves the actual callable driver, not a mode label.
            assert!(matches!((&home.borrow().func.as_ref().unwrap().body,ast),
                (FunctionBody::Bytecode(_),false)|(FunctionBody::Block(_),true)));
            let drive=match kind {
                "ordinary"=>"try{out=home.m(undefined,3);done=true}catch(e){error=e;done=true}",
                "generator"=>"try{var r=home.m(undefined,3).next();out=r.value;done=r.done}catch(e){error=e;done=true}",
                "async"=>"home.m(undefined,3).then(v=>{out=v;done=true},e=>{error=e;done=true})",
                _=>"home.m(undefined,3).next().then(r=>{out=r.value;done=r.done},e=>{error=e;done=true})",
            };
            i.run_script(drive).unwrap();i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("done&&error===undefined&&out===true").unwrap(),JsValue::Bool(true));
        }));if result.is_err(){failures.push(format!("AST={ast} {kind} ({params}) {body}"));}
    }}}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn genuine_block_concise_and_async_arrow_body_instantiation() {
    let cases = [
        ("", "()=>{return delete local===false;let local=1;}",false,false),
        ("", "()=>{try{return local}catch(e){return e instanceof ReferenceError}let local=1;}",false,false),
        ("", "()=>{let [x,...rest]=[2,3];return x===2&&rest[0]===3;}",false,false),
        ("", "()=>{const {x}= {x:2};return x===2;}",false,false),
        ("", "()=>{using item=null;return delete item===false;}",false,false),
        ("", "()=>{function read(){return x}let x=7;return read()===7;}",false,false),
        ("", "(get=eval('var x=1;()=>x'))=>{let x=2;return get()===1&&x===2;}",false,false),
        ("", "()=> (eval('var arrowFresh=7'),delete arrowFresh)",false,true),
        ("'use strict';", "()=> (eval('var arrowFresh=7'),typeof arrowFresh==='undefined')",false,true),
        ("'use strict';", "()=>{function read(){return x}const x=7;return read()===7;}",false,false),
        ("", "async()=>{await 0;return delete local===false;let local=1;}",true,false),
        ("", "async()=>{let [x]=[2];await 0;return x===2;}",true,false),
        ("", "async()=>{await using item=null;return delete item===false;}",true,false),
        ("", "async()=>{function read(){return x}await 0;let x=7;return read()===7;}",true,false),
        ("", "async()=> (await 0,eval('var arrowFresh=7'),delete arrowFresh)",true,true),
        ("'use strict';", "async()=> (await 0,eval('var arrowFresh=7'),typeof arrowFresh==='undefined')",true,true),
    ];
    let mut failures=Vec::new();
    for (prefix,arrow,asynchronous,concise) in cases {for ast in [false,true] {
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            i.run_script(&format!("{prefix}var local=91,x=91;var arrow={arrow},out,error,done=false;")).unwrap();
            let JsValue::Object(object)=i.run_script("arrow").unwrap() else{panic!("arrow")};
            assert!(matches!((&object.borrow().func.as_ref().unwrap().body,ast,concise),
                (FunctionBody::Bytecode(_),false,_)|(FunctionBody::Block(_),true,false)|(FunctionBody::Expr(_),true,true)));
            i.run_script(if asynchronous {"arrow().then(v=>{out=v;done=true},e=>{error=e;done=true})"}
                else {"try{out=arrow();done=true}catch(e){error=e;done=true}"}).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("done&&error===undefined&&out===true").unwrap(),JsValue::Bool(true));
        }));if result.is_err(){failures.push(format!("AST={ast} {prefix}{arrow}"));}
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn genuine_empty_function_body_scope_and_generic_program_entry_remain_distinct() {
    let parsed=parse_script("function f(){}").unwrap();
    let crate::ast::StmtKind::Function(function)=&parsed.body[0].kind else{panic!("function")};
    for strict in [false,true] {
        let code=compile_function_item(function,parsed.body[0].span,strict).unwrap();
        let mut i=Interpreter::new();
        let variable=EnvHandle::new(EnvKind::Function,Some(i.global_env.clone()));
        let frame=code.0.program.start_function_frame(&mut i,variable.clone(),1000,strict).unwrap();
        assert_eq!(frame.env.kind(),if strict{EnvKind::Function}else{EnvKind::Block});
        assert_eq!(frame.env.var_scope().kind(),EnvKind::Function);
        assert_eq!(frame.pc,0);
    }
    for source in ["let x=1;x", "{let x=1;x}"] {
        let program=compile(&parse_script(source).unwrap(), false).unwrap();let mut i=Interpreter::new();
        let env=EnvHandle::new(EnvKind::Block,None);
        let frame=program.start_frame(&mut i,env,1000).unwrap();
        assert_eq!(frame.pc,0);assert!(frame.env.local_cell("x").is_none());
    }
}


#[test]
fn function_declared_formals_copy_distinct_cells_and_choose_last_closure() {
    let cases = [
        ("x=1,get=()=>x", "function x(){return 7}return get()===1&&x()===7;"),
        ("x=1,get=()=>x", "var x;return get()===1&&x===1&&(x=2,get()===1);"),
        ("", "function read(){return 1}function read(){return x}let x=7;return read()===7;"),
        ("", "function arguments(){return 7}return arguments()===7;"),
        ("a,a", "a=7;return arguments[0]===undefined&&arguments[1]===7;"),
    ];
    let mut failures = Vec::new();
    for (params,body) in cases { for ast in [false,true] { for kind in ["ordinary","generator","async","async-generator"] {
        let source=format!("var out,error,done=false;var home={{{async_prefix}{star}m({params}){{{body}}}}};",async_prefix=if kind.starts_with("async"){"async "}else{""},star=if kind.contains("generator"){"*"}else{""});
        // Methods require unique formals. The duplicate case uses a genuine
        // declaration, preserving the live all-four sloppy-simple rule.
        let source=if params=="a,a" {format!("var out,error,done=false;{async_prefix}function{star} f(a,a){{{body}}}var home={{m:f}};",async_prefix=if kind.starts_with("async"){"async "}else{""},star=if kind.contains("generator"){"*"}else{""})}else{source};
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();
            let JsValue::Object(home)=i.run_script("home.m").unwrap() else{panic!("callable")};
            assert!(matches!((&home.borrow().func.as_ref().unwrap().body,ast),(FunctionBody::Bytecode(_),false)|(FunctionBody::Block(_),true)));
            let drive=match kind {"ordinary"=>"try{out=home.m(undefined,3);done=true}catch(e){error=e;done=true}","generator"=>"try{var r=home.m(undefined,3).next();out=r.value;done=r.done}catch(e){error=e;done=true}","async"=>"home.m(undefined,3).then(v=>{out=v;done=true},e=>{error=e;done=true})",_=>"home.m(undefined,3).next().then(r=>{out=r.value;done=r.done},e=>{error=e;done=true})"};
            let drive=if params=="a,a" {drive.to_owned()} else {drive.replace("home.m(undefined,3)","home.m(undefined,undefined)")};
            i.run_script(&drive).unwrap();
            i.drain_microtasks().unwrap();assert_eq!(i.run_script("done&&error===undefined&&out===true").unwrap(),JsValue::Bool(true));
        }));if result.is_err(){failures.push(format!("AST={ast} {kind} ({params}) {body}"));}
    }}}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}
