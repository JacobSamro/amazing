use super::*;

const SETUP: &str = r#"
var out,done=false,error,log=[],nil=null,undef=undefined,sentinel={},outer=23;
var key={[Symbol.toPrimitive](){log.push('key');return 'x'}},badkey={[Symbol.toPrimitive](){log.push('key');throw sentinel}};
var target={x:7,get g(){log.push('get');return 9},m(){log.push('call');return 8}},nested={target};
Object.defineProperty(target,'fixed',{value:1,configurable:false});
var proxy=new Proxy(target,{deleteProperty(t,k){log.push('delete:'+k);return Reflect.deleteProperty(t,k)}});
var falseProxy=new Proxy({x:1},{deleteProperty(){log.push('delete');return false}});
var badProxy=new Proxy(target,{deleteProperty(){log.push('delete');return true}});
var raw=()=>{log.push('raw');return key},badraw=()=>{log.push('raw');throw sentinel};
var arg=()=>{log.push('arg');return 0};
"#;

fn assert_native(i: &mut Interpreter, ast: bool) {
    let JsValue::Object(object)=i.run_script("home.m").unwrap() else {panic!("callable")};
    let object=object.borrow();
    match &object.func.as_ref().unwrap().body {
        FunctionBody::Bytecode(code) if !ast=>assert!(!code.0.program.code.iter().any(|op|matches!(op,Instruction::TreeEval(..))),"Delete delegated"),
        FunctionBody::Block(_) if ast=>{},
        other=>panic!("AST={ast}: {other:?}"),
    }
}

#[test]
fn whole_delete_reference_and_value_matrix_is_native_in_all_four_kinds() {
    // Prefix, operand, result/state, observable log, abrupt kind. Every row is
    // evaluated in all four callable kinds and both actual execution drivers.
    let cases=[
        ("","1","out===true","",""),
        ("","null","out===true","",""),
        ("","undefined","out===false","",""),
        ("","true","out===true","",""),
        ("","'text'","out===true","",""),
        ("","123n","out===true","",""),
        ("","missingDeleteName","out===true","",""),
        ("","((missingDeleteName))","out===true","",""),
        ("var local=4;","local","out===false","",""),
        ("let local=4;","local","out===false","",""),
        ("const local=4;","local","out===false","",""),
        ("","outer","out===false","",""),
        ("","target.x","out===true&&!('x' in target)","",""),
        ("","(target.x)","out===true&&!('x' in target)","",""),
        ("","((target.x))","out===true&&!('x' in target)","",""),
        ("","target.g","out===true&&!('g' in target)","",""),
        ("","target.fixed","out===false&&target.fixed===1","",""),
        ("","target.absent","out===true","",""),
        ("","target[key]","out===true&&!('x' in target)","key",""),
        ("","target[raw()]","out===true&&!('x' in target)","raw,key",""),
        ("","target[badkey]","true","key","sentinel"),
        ("","target[badraw()]","true","raw","sentinel"),
        ("","nil[raw()]","true","raw","type"),
        ("","undef[raw()]","true","raw","type"),
        ("","nil.x","true","","type"),
        ("","nil?.[raw()]","out===true","",""),
        ("","undef?.[badraw()]","out===true","",""),
        ("","nil?.m(arg())","out===true","",""),
        ("","target?.x","out===true&&!('x' in target)","",""),
        ("","target?.[key]","out===true&&!('x' in target)","key",""),
        ("","(target?.x)","out===true&&!('x' in target)","",""),
        ("","nested.target.x","out===true&&!('x' in target)","",""),
        ("","proxy.x","out===true&&!('x' in target)","delete:x",""),
        ("","proxy[key]","out===true&&!('x' in target)","key,delete:x",""),
        ("","falseProxy.x","out===false","delete",""),
        ("","badProxy.fixed","true","delete","type"),
        ("","(0,target.g)","out===true&&('g' in target)","get",""),
        ("","(true?target.g:0)","out===true&&('g' in target)","get",""),
        ("","(false?0:target.g)","out===true&&('g' in target)","get",""),
        ("","(target.g||0)","out===true&&('g' in target)","get",""),
        ("","(target.g&&1)","out===true&&('g' in target)","get",""),
        ("","(target.g??0)","out===true&&('g' in target)","get",""),
        ("","target.m()","out===true","call",""),
        ("","(target.x=3)","out===true&&target.x===3","",""),
        ("","(target.x++)","out===true&&target.x===8","",""),
        ("","'abc'.length","out===false","",""),
        ("","'abc'[0]","out===false","",""),
        ("","'abc'[9]","out===true","",""),
        ("","'\\uD800\\uDC00'[0]","out===false","",""),
        ("","'\\uD800'[1]","out===true","",""),
        ("","true.x","out===true","",""),
        ("","123n.x","out===true","",""),
        ("","Symbol('x').x","out===true","",""),
        ("","(7).x","out===true","",""),
        ("","nested?.target?.[key]","out===true&&!('x' in target)","key",""),
        ("","target?.missing?.[badraw()]","out===true","",""),
        ("","(nil?.x).y","true","","type"),
        ("eval('var freshDelete=1');","freshDelete","out===true","",""),
        ("eval('function freshDelete(){}');","freshDelete","out===true","",""),
        ("var local=1;eval('var local=2');","local","out===false","",""),
    ];
    let mut failures=Vec::new();
    for (prefix,operand,check,expected,abrupt) in cases {for ast in [false,true] {for kind in ["ordinary","generator","async","async-generator"] {
        let source=format!("{SETUP}var home={{{async_prefix}{star}m(){{{prefix}return delete {operand};}}}};",async_prefix=if kind.starts_with("async"){"async "}else{""},star=if kind.contains("generator"){"*"}else{""});
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let mut i=Interpreter::new();i.force_ast_callables=ast;i.run_script(&source).unwrap();assert_native(&mut i,ast);
            let drive=match kind {
                "ordinary"=>"try{out=home.m();done=true}catch(e){error=e;done=true}",
                "generator"=>"try{var r=home.m().next();out=r.value;done=r.done}catch(e){error=e;done=true}",
                "async"=>"home.m().then(v=>{out=v;done=true},e=>{error=e;done=true})",
                _=>"home.m().next().then(r=>{out=r.value;done=r.done},e=>{error=e;done=true})",
            };
            i.run_script(drive).unwrap();i.drain_microtasks().unwrap();
            let error=match abrupt {"type"=>"error instanceof TypeError","sentinel"=>"error===sentinel",_=>"error===undefined"};
            assert_eq!(i.run_script(&format!("done&&({error})&&({check})&&log.join(',')==='{expected}'")).unwrap(),JsValue::Bool(true));
        }));if result.is_err(){failures.push(format!("AST={ast} {kind} {prefix}delete {operand}"));}
    }}}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn local_eval_delete_policy_preserves_existing_cells_and_resolution_after_removal() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
          var outer=41;
          function f(parameter){
            var existing=1;eval('var fresh=7;var outer=9;function created(){return 3};var existing=2;function parameter(){return 5}');
            var capture=()=>fresh;
            if(delete existing||delete parameter||!delete fresh||!delete created||!delete outer)return false;
            if(outer!==41||typeof fresh!=='undefined'||typeof created!=='undefined')return false;
            var threw=false;try{capture()}catch(e){threw=e instanceof ReferenceError}
            eval('var fresh=13');return threw&&fresh===13&&delete fresh&&existing===2&&parameter()===5;
          } f(3)
        "#).unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script(r#"function mapped(a){eval('var a=8;var b=2');return arguments[0]===8&&!delete a&&delete b}mapped(1)"#).unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn with_identifier_delete_uses_semantic_proxy_gateway_without_getting_value() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
          var log=[],object={x:1},proxy=new Proxy(object,{has(t,k){log.push('has:'+String(k));return Reflect.has(t,k)},
            get(t,k,r){if(k===Symbol.unscopables)return undefined;throw 'unexpected GetValue'},
            deleteProperty(t,k){log.push('delete:'+k);return Reflect.deleteProperty(t,k)}});
          function f(){with(proxy){return delete x}};
          var result=f();result&&!('x' in object)&&log.join(',')==='has:x,delete:x'
        "#).unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script(r#"
          var p=new Proxy(Object.defineProperty({},'x',{value:1,configurable:false}),{deleteProperty(){return true}});
          function f(){with(p){return delete x}};var correct=false;try{f()}catch(e){correct=e instanceof TypeError}correct
        "#).unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn delete_tdz_binding_without_getvalue_and_strict_property_errors() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script("var home={m(){return delete local;let local=1;}};home.m()===false").unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        assert_eq!(i.run_script(r#"
          var home={m(){'use strict';return delete Object.defineProperty({},'x',{value:1}).x}};
          var correct=false;try{home.m()}catch(e){correct=e instanceof TypeError}correct
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        for source in ["function f(){'use strict';return delete local}","function f(){'use strict';return delete (((local)))}"] {
            assert!(i.run_script(source).is_err(),"strict identifier Delete remains a parser error");
        }
    }
}

#[test]
fn suspended_delete_operand_retains_base_and_key_through_finally_and_abrupt_resume() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
          var log=[],first={x:1},second={x:2},object=first;
          var home={*m(){try{return delete object[yield 'key']}finally{log.push('finally')}}};
          var iterator=home.m();var a=iterator.next();object=second;var b=iterator.next('x');
          a.value==='key'&&!a.done&&b.value===true&&b.done&&!('x' in first)&&second.x===2&&log.join(',')==='finally'
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        assert_eq!(i.run_script(r#"
          var log=[],object={x:1},home={*m(){try{return delete object[yield 'key']}finally{log.push('finally')}}};
          var iterator=home.m();iterator.next();var r=iterator.return(7);
          r.done&&r.value===7&&object.x===1&&log.join(',')==='finally'
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        assert_eq!(i.run_script(r#"
          var log=[],object={x:1},sentinel={},home={*m(){try{return delete object[yield 'key']}finally{log.push('finally')}}};
          var iterator=home.m();iterator.next();var same=false;try{iterator.throw(sentinel)}catch(e){same=e===sentinel}
          same&&object.x===1&&log.join(',')==='finally'
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        i.run_script(r#"
          var complete=false,object={x:1},saved=object,release,gate=new Promise(r=>release=r);
          var home={async m(){return delete object[await gate]}};
          home.m().then(v=>complete=v);object={x:2};release('x');
        "#).unwrap();assert_native(&mut i,ast);i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("complete&&!('x' in saved)&&object.x===2").unwrap(),JsValue::Bool(true));
        i.run_script(r#"
          var complete=false,object={x:1},home={async *m(){return delete object[yield 'key']}};
          var iterator=home.m();iterator.next().then(r=>{if(r.value==='key')iterator.next('x').then(r=>complete=r.done&&r.value)});
        "#).unwrap();assert_native(&mut i,ast);i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("complete&&!('x' in object)").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn super_delete_preserves_this_raw_expression_and_rejects_before_key_coercion() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
          var log=[],key={[Symbol.toPrimitive](){log.push('key');return 'x'}};
          var raw=()=>{log.push('raw');return key};
          var home={m(){return delete super[raw()]}};Object.setPrototypeOf(home,null);
          var correct=false;try{home.m()}catch(e){correct=e instanceof ReferenceError}
          correct&&log.join(',')==='raw'
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        assert_eq!(i.run_script(r#"
          var log=[],sentinel={},raw=()=>{log.push('raw');throw sentinel};
          var home={m(){return delete super[raw()]}};
          var same=false;try{home.m()}catch(e){same=e===sentinel}same&&log.join(',')==='raw'
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
    }
}

#[test]
fn local_delete_api_preserves_live_cell_identity_and_non_deletable_binding_kinds() {
    let env=EnvHandle::new(crate::env::EnvKind::Var,None);
    env.declare_var("existing",JsValue::num(1.0)).unwrap();
    let existing=env.local_cell("existing").unwrap();
    env.declare_sloppy_eval_var("existing",Some(JsValue::num(2.0))).unwrap();
    assert_eq!(existing.read().unwrap(),JsValue::num(2.0));assert!(!env.delete_local_binding("existing"));
    env.declare_sloppy_eval_var("fresh",Some(JsValue::num(7.0))).unwrap();
    let escaped=env.local_cell("fresh").unwrap();let clone=env.clone();
    assert!(clone.delete_local_binding("fresh"));assert!(env.local_cell("fresh").is_none());
    assert_eq!(escaped.read().unwrap(),JsValue::num(7.0));
    env.declare_sloppy_eval_var("fresh",Some(JsValue::num(9.0))).unwrap();
    assert_eq!(escaped.read().unwrap(),JsValue::num(7.0));assert_eq!(env.resolve("fresh").unwrap(),JsValue::num(9.0));
    env.declare_lexical("constant",JsValue::num(1.0),false).unwrap();
    assert!(!env.delete_local_binding("constant"));
    assert!(env.declare_sloppy_eval_var("constant",None).is_err());
    assert!(!env.delete_local_binding("absent"));
    env.declare_import("imported",escaped).unwrap();
    assert!(!env.delete_local_binding("imported"));
    assert!(env.declare_sloppy_eval_var("imported",None).is_err());
    assert!(env.set_local_binding("imported",JsValue::num(1.0),false).is_err());
    assert!(env.set_local_binding("constant",JsValue::num(2.0),false).is_err());
    env.declare_uninitialized("tdz",true).unwrap();
    assert!(env.set_local_binding("tdz",JsValue::num(2.0),false).is_err());
    assert!(env.set_local_binding("strictMissing",JsValue::num(2.0),true).is_err());
    env.set_local_binding("sloppyMissing",JsValue::num(2.0),false).unwrap();
    assert!(env.delete_local_binding("sloppyMissing"));
}

#[test]
fn foreign_delete_errors_use_callee_realm_and_restore_caller() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        let caller=i.binding_current_realm();let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
        let f=i.binding_eval_in_realm(child,r#"({m(){'use strict';return delete Object.defineProperty({},'x',{value:1}).x}}).m"#).unwrap();
        let error_proto=i.binding_eval_in_realm(child,"TypeError.prototype").unwrap();
        i.global_env.declare_var("foreignDelete",f).unwrap();i.global_env.declare_var("foreignTypeErrorPrototype",error_proto).unwrap();
        assert_eq!(i.run_script("var correct=false;try{foreignDelete()}catch(e){correct=Object.getPrototypeOf(e)===foreignTypeErrorPrototype}correct").unwrap(),JsValue::Bool(true));
        assert_eq!(i.binding_current_realm(),caller);
    }
}

#[test]
fn delete_quanta_and_budget_unwind_preserve_environment_strictness_and_roots() {
    let mut i=Interpreter::new();i.run_script("var object={x:1},key={toString(){return 'x'}};").unwrap();
    let ast=crate::parse_script("delete object[key]").unwrap();let code=compile(&ast,false).unwrap();drop(ast);
    assert!(!code.code.iter().any(|op|matches!(op,Instruction::TreeEval(..))));
    let depth=i.frames.len();let realm=i.binding_current_realm();let strict=i.strict_eval_context;
    let env=i.global_env.clone();let mut frame=code.start_frame(&mut i,env,1000).unwrap();
    let value=loop {match code.resume_frame(&mut i,&mut frame,1).unwrap() {
        FrameOutcome::Suspended=>{},FrameOutcome::Completed(v)=>break v,_=>panic!("Delete quantum outcome"),
    }};
    assert_eq!(value,JsValue::Bool(true));assert_eq!(i.run_script("!('x' in object)").unwrap(),JsValue::Bool(true));
    let error=code.run(&mut i,0).unwrap_err();let error=i.normalize_throw(error).0;i.global_obj.borrow_mut().set_own("error",error);
    assert_eq!(i.run_script("error instanceof RangeError").unwrap(),JsValue::Bool(true));
    assert_eq!(i.frames.len(),depth);assert_eq!(i.binding_current_realm(),realm);assert_eq!(i.strict_eval_context,strict);
}

#[test]
fn putvalue_after_deletion_recreates_captured_local_and_never_writes_outer() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
          var x=99;
          function plain(){eval('var x=1');x=(delete x,7);return x===7&&delete x&&x===99}
          function compound(){eval('var x=1');x+=(delete x,7);return x===8&&delete x&&x===99}
          function logical(){eval('var x=1');x&&=(delete x,7);return x===7&&delete x&&x===99}
          function update(){eval('var x={valueOf(){delete x;return 3}}');var old=x++;return old===3&&x===4&&delete x&&x===99}
          plain()&&compound()&&logical()&&update()&&x===99
        "#).unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script(r#"
          var x=99;
          function f(){eval('var x=1');var drop=()=>delete x;
            var strictPut=function(){'use strict';try{x=(drop(),7)}catch(e){return e instanceof ReferenceError}return false};
            return strictPut()&&x===99;
          } f()&&x===99
        "#).unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script(r#"
          function f(){eval('var uniqueLocalDelete=1');uniqueLocalDelete=(delete uniqueLocalDelete,7);
            return uniqueLocalDelete===7&&delete uniqueLocalDelete&&typeof uniqueLocalDelete==='undefined';}
          f()&&!Object.prototype.hasOwnProperty.call(globalThis,'uniqueLocalDelete')
        "#).unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script(r#"
          var x=99,home={*m(){eval('var x=1');x=(delete x,yield 'park');return x===7&&delete x&&x===99}};
          var iterator=home.m();iterator.next();var r=iterator.next(7);r.done&&r.value&&x===99
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
    }
}

#[test]
fn global_object_mirrors_and_deleted_rhs_fallback_keep_original_semantics() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        assert_eq!(i.run_script(r#"
          (0,eval)('var evalGlobalDelete=1');
          var home={m(){evalGlobalDelete=(delete evalGlobalDelete,7);return evalGlobalDelete}};
          home.m()===7&&globalThis.evalGlobalDelete===7
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        assert_eq!(i.run_script(r#"
          (0,eval)('var evalGlobalDelete=1');
          var home={m(){evalGlobalDelete+=(delete evalGlobalDelete,7);return evalGlobalDelete}};
          home.m()===8&&globalThis.evalGlobalDelete===8
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
        assert_eq!(i.run_script(r#"
          var preservedGlobalMirror=1;
          var home={m(){preservedGlobalMirror=(delete preservedGlobalMirror,3);return preservedGlobalMirror}};
          home.m()===3&&globalThis.preservedGlobalMirror===3&&
            delete preservedGlobalMirror===false&&delete undefined===false&&undefined===void 0
        "#).unwrap(),JsValue::Bool(true));assert_native(&mut i,ast);
    }
}

#[test]
fn native_identifier_delete_uses_delete_name_without_reading_binding_value() {
    let mut i=Interpreter::new();
    let JsValue::Object(object)=i.run_script("(function(){return delete local;let local=1})").unwrap() else {panic!("function")};
    let object=object.borrow();let FunctionBody::Bytecode(code)=&object.func.as_ref().unwrap().body else {panic!("native callable")};
    assert!(code.0.program.code.iter().any(|op|matches!(op,Instruction::DeleteName(_,name) if name=="local")));
    assert!(!code.0.program.code.iter().any(|op|matches!(op,Instruction::LoadName(_,name,_) if name=="local")));
    assert!(!code.0.program.code.iter().any(|op|matches!(op,Instruction::TreeEval(..))));
}

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
fn genuine_empty_function_body_scope_and_generic_program_entry_remain_distinct() {
    let parsed=crate::parse_script("function f(){}").unwrap();
    let crate::ast::StmtKind::Function(function)=&parsed.body[0].kind else{panic!("function")};
    for strict in [false,true] {
        let code=compile_function_item(function.as_ref(),parsed.body[0].span,strict).unwrap();
        let mut i=Interpreter::new();
        let variable=EnvHandle::new(EnvKind::Function,Some(i.global_env.clone()));
        let frame=code.0.program.start_function_frame(&mut i,variable.clone(),1000,strict).unwrap();
        assert_eq!(frame.env.kind(),if strict{EnvKind::Function}else{EnvKind::Block});
        assert_eq!(frame.env.var_scope().kind(),EnvKind::Function);
        assert_eq!(frame.pc,0);
    }
    for source in ["let x=1;x", "{let x=1;x}"] {
        let program=compile(&crate::parse_script(source).unwrap(),false).unwrap();let mut i=Interpreter::new();
        let env=EnvHandle::new(EnvKind::Block,None);
        let frame=program.start_frame(&mut i,env,1000).unwrap();
        assert_eq!(frame.pc,0);assert!(frame.env.local_cell("x").is_none());
    }
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
fn delete_call_values_preserve_eval_reference_and_pre_argument_receiver() {
    let body = r#"
        var local=2;
        var direct=delete (eval)('local=3;var temporaryDelete=4;temporaryDelete');
        var indirect=delete eval?.('var globalDelete=5;globalDelete');
        var received=false,r={m(v){received=this===r&&v===7;return 0}},holder=r;
        var called=delete (holder.m)((holder={m(){throw 'changed'}},7));
        var superValue=delete (super[(log.push('expr'),key)])(9);
        return direct&&indirect&&called&&received&&superValue&&superReceived&&
               log.join(',')==='expr,convert,get,call'&&local===3&&
               (delete temporaryDelete)&&typeof temporaryDelete==='undefined'&&globalDelete===5;
    "#;
    let mut failures=Vec::new();
    for ast in [false,true] { for kind in ["ordinary","generator","async","async-generator"] {
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut i=Interpreter::new(); i.force_ast_callables=ast;
            let source=format!("var out,done=false,error,log=[],superReceived=false;var key={{[Symbol.toPrimitive](){{log.push('convert');return 'm'}}}};var base={{get m(){{if(this!==home)throw 'getter receiver';log.push('get');return function(v){{if(this!==home||v!==9)throw 'call receiver';log.push('call');superReceived=true;return 0}}}}}};var home={{{prefix}{star}m(){{{body}}}}};Object.setPrototypeOf(home,base);",
                prefix=if kind.starts_with("async"){"async "}else{""},
                star=if kind.contains("generator"){"*"}else{""});
            i.run_script(&source).unwrap(); assert_native(&mut i,ast);
            let drive=match kind {
                "ordinary"=>"try{out=home.m();done=true}catch(e){error=e;done=true}",
                "generator"=>"try{var r=home.m().next();out=r.value;done=r.done}catch(e){error=e;done=true}",
                "async"=>"home.m().then(v=>{out=v;done=true},e=>{error=e;done=true})",
                _=>"home.m().next().then(r=>{out=r.value;done=r.done},e=>{error=e;done=true})",
            };
            i.run_script(drive).unwrap();i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("done&&error===undefined&&out===true").unwrap(),JsValue::Bool(true));
        }));
        if result.is_err(){failures.push(format!("AST={ast} {kind}"));}
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}
