use super::*;
use crate::Interpreter;
use crate::interp::ExecutionBackend;
use crate::value::{JsValue, FunctionBody};

fn script(source: &str, expected: bool) {
    assert_eq!(crate::parse_script(source).is_ok(), expected, "{source}");
    let units: Vec<_>=source.encode_utf16().collect();
    assert_eq!(crate::parse_script_utf16(&units).is_ok(), expected, "UTF16 {source}");
}
fn module(source: &str, expected: bool) {
    assert_eq!(crate::parse_module(source).is_ok(), expected, "module {source}");
    let units: Vec<_>=source.encode_utf16().collect();
    assert_eq!(crate::parse_module_utf16(&units).is_ok(), expected, "UTF16 module {source}");
}

#[test]
fn fields_preserve_the_normative_await_yield_parameters() {
    for field in ["value", "static value", "#privateValue", "static #privateValue"] {
        for (outer, is_async, is_generator) in [("function outer()",false,false),
            ("async function outer()",true,false),("function* outer()",false,true),
            ("async function* outer()",true,true)] {
            // Initializer[+In, ~Yield, ~Await]: `await` is an identifier, never an operator.
            script(&format!("{outer}{{return class{{{field}=await;}};}}"),true);
            script(&format!("{outer}{{return class{{{field}=await 9;}};}}"),false);
            for value in ["yield 9", "yield"] {
                script(&format!("{outer}{{return class{{{field}={value};}};}}"),is_generator);
            }
            script(&format!("{outer}{{return class{{{field}=new.target;}};}}"),true);
        }
    }
    script("for(var Value=class{field='key' in {};};false;){}",true);
    script("function* f(x=class{field=async()=>await 1;}){}",true);
    script("class Outer{static{class Inner{value=await 1;}}}",true);
    script("class Outer{static{class Inner{value=await;}}}",false);
}

#[test]
fn computed_keys_retain_the_enclosing_await_and_yield_context() {
    for suffix in ["=7", "(){return 7;}", "=class{}"] {
        script(&format!("async function f(){{return class{{[await Promise.resolve('key')]{suffix};}};}}"),true);
        script(&format!("function* f(){{return class{{[yield 'key']{suffix};}};}}"),true);
        script(&format!("async function* f(){{return class{{[await Promise.resolve(yield 'key')]{suffix};}};}}"),true);
    }
    script("async()=>class{[await]=1}",false);
    script("function* f(){return class{[yield 1]=2;after=yield 3;};}",true);
    script("async function f(){class C{value=await 1;}await Promise.resolve();}",false);
}

#[test]
fn module_and_static_block_identifier_reservations_survive_context_restoration() {
    for source in ["class C{value=await;}",
        "class C{static{await 8;}}", "class C{static{var await;}}"] {
        module(source,false);
    }
    for source in ["class C{value=async()=>await 8;}await Promise.resolve();",
        "class C{static{async function nested(){await 8;}}}",
        "class C{value=1;}await Promise.resolve();"] {module(source,true);}
    module("async()=>class{value=await 8}",false);
    script("class C{static{class D{value=await;}await;}}",false);
    script("function* outer(){class C{value=1;}yield 3;}",true);
    assert!(crate::parse_module("class C{value=await 1;}").is_err());
    assert!(crate::parse_module("class C{[await 1]=2;}").unwrap().has_top_level_await);
}

#[test]
fn method_modifiers_commit_to_method_grammar_in_all_object_and_binding_positions() {
    for prefix in ["async key", "async *key", "*key", "get key", "set key"] {
        for end in [":0", "", ",", "=0"] {
            let object=format!("{{{prefix}{end}}}");
            script(&format!("({object})"),false);
            script(&format!("({object}={{}})"),false);
            for declaration in ["var", "let", "const"] {
                script(&format!("{declaration} {object}={{}};"),false);
            }
        }
    }
    for source in ["({get key(a){}})","({set key(){}})","({set key(a,b){}})",
        "({async get key(){}})","({*get key(){}})"] {script(source,false);}
}

#[test]
fn ordinary_keyword_property_names_and_valid_method_prefixes_remain_valid() {
    for name in ["async", "get", "set", "static", "await", "yield", "class", "enum"] {
        script(&format!("({{{name}:1,{name}(){{return 1;}}}})"),true);
        script(&format!("class C{{{name}(){{return 1;}}}}"),true);
    }
    for source in ["({async(){},get(){},set(){},async method(){},*method(){},async *method(){}})",
        "({get\n key(){return 1;},set\n key(value){}})",
        "class C{get\n key(){return 1;}set\n key(value){}}",
        "class C{async\n method(){return 1;}}",
        "class C{get;set;async;static;}"] {script(source,true);}
    script("class C{get\n key=1;}",false);
    script("class C{set\n key=1;}",false);
    script("({async\n method(){}})",false);
}

#[test]
fn every_own_strict_function_name_uses_complete_strict_binding_validation() {
    for name in ["implements","interface","let","package","private","protected","public","static","yield","eval","arguments"] {
        for head in ["function", "function*", "async function", "async function*"] {
            for wrap in [false,true] {
                let source=format!("{head} {name}(){{'use strict';return 1;}}");
                script(&if wrap{format!("({source})")}else{source},false);
            }
        }
        if !matches!(name,"eval"|"arguments") {
            script(&format!("function {name}(){{return 1;}}"),true);
            script(&format!("(function {name}(){{return 1;}})"),true);
        }
    }
    for name in ["ordinary","async","await","get","set"] {
        script(&format!("function {name}(){{'use strict';return 1;}}"),true);
    }
}

#[test]
fn private_brands_are_relational_productions_not_unary_or_primary_values() {
    for expr in ["#slot in target", "#slot in (target)", "#slot in target && true",
        "#slot in target ? 1 : 2", "!(#slot in target)", "(#slot in target) === true",
        "#slot in (target || {})", "#slot in target in {}"] {
        script(&format!("class C{{#slot;check(target){{return {expr};}}}}"),true);
    }
    for expr in ["!#slot", "!#slot in target", "typeof #slot", "void #slot in target",
        "1+#slot in target", "1<#slot in target", "(#slot) in target", "#slot.key in target",
        "#slot[0] in target", "#slot() in target", "#slot++ in target", "++#slot in target",
        "#slot?1:2", "[#slot]", "#slot in ()=>target"] {
        script(&format!("class C{{#slot;check(target){{return {expr};}}}}"),false);
    }
    script("class C{check(target){return #later in target;}#later;}",true);
    script("class C{check(target){return #missing in target;}}",false);
    script("class C{#slot;check(target){for(#slot in target;;){}}}",false);
    script("class C{#slot;check(target){for(;(#slot in target);){break;}}}",true);
}

#[test]
fn initializer_super_arguments_and_nested_callable_boundaries_are_complete() {
    for source in ["class C{value=arguments;}","class C{value=()=>arguments;}",
        "class C{value=super();}",
        "class Outer extends Object{constructor(){class Inner{value=super();}}}"] {script(source,false);}
    for source in ["class C{value=function(){return arguments;};}",
        "class C{value=async()=>await 1;}","class C{value=function*(){yield 1;};}",
        "class C extends Object{value=super.toString;}",
        "class C{value=new.target;static other=new.target;}"] {script(source,true);}
}

#[test]
fn escaped_names_and_utf16_source_are_kept_separate_from_modifier_tokens() {
    script(r"({async:1,get:2,set:3})",true);
    script(r"({a\u0073ync key(){}})",false);
    script(r"left \u0069n right",false);
    script(r"left \u0069nstanceof right",false);
    script(r"class C{#slot;test(target){return #slot \u0069n target;}}",false);
    script(r"({\u0069n:1}).in",true);
    let source="class C{['field']=async()=>{return '\\uD800';};get\n ['key'](){return 1;}}";
    let program=crate::parse_script_utf16(&source.encode_utf16().collect::<Vec<_>>()).unwrap();
    let StmtKind::Class(class)=&program.body[0].kind else{panic!("class");};
    assert_eq!(class.source.units().unwrap(),source.encode_utf16().collect::<Vec<_>>());
    let clone=program.clone();assert_eq!(clone,program);
}

#[test]
fn real_ast_and_native_method_field_consumers_observe_the_new_grammar() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);
        let function=i.run_script_with_backend("({method(){return 1;}}).method",backend).unwrap();
        let JsValue::Object(object)=function else{panic!("method");};
        let body=object.borrow().func.as_ref().unwrap().body.clone();
        match backend {
            ExecutionBackend::TreeWalk=>assert!(matches!(body,FunctionBody::Block(_))),
            ExecutionBackend::Bytecode=>assert!(matches!(body,FunctionBody::Bytecode(_))),
        }
        for source in ["class C{value=new.target;static other=new.target;}new C().value===undefined&&C.other===undefined",
            "class C{get\n key(){return 9;}set\n key(v){this.saved=v;}}let c=new C();c.key=3;c.key===9&&c.saved===3",
            "class C{#slot;check(target){return #slot in target;}}let c=new C();c.check(c)&&!c.check({})",
            "class C{value=function(){return arguments.length;};}new C().value(1,2)===2"] {
            let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);
            assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?}: {source}");
        }
        let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);
        assert_eq!(i.run_script_with_backend("class C{value=eval('new.target');}new C().value===undefined",backend).unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn public_function_eval_and_module_goals_share_authentic_early_errors() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);
        for body in ["({async member:0})", "class C{#slot;method(){return !#slot;}}",
            "function implements(){'use strict';}", "async()=>class{[await]=1}"] {
            let quoted=format!("{body:?}");
            let source=format!("var caught=0;try{{Function({quoted});}}catch(e){{if(e instanceof SyntaxError)caught++;}}try{{eval({quoted});}}catch(e){{if(e instanceof SyntaxError)caught++;}}caught===2");
            assert_eq!(i.run_script_with_backend(&source,backend).unwrap(),JsValue::Bool(true),"{body}");
        }
    }
}
