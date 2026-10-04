use amazing_js::{Interpreter, ast::StmtKind, interp::ExecutionBackend, value::{FunctionBody, JsValue}};

fn matrix(signature: &str, body: &str, setup: &str, drive: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
            let mut i = Interpreter::new();
            i.run_script_with_backend("var ok=false,log=[],marker={},source,excluded,included;",backend).unwrap();
            let script=format!("{signature}{{{body}}}");
            i.run_script_with_backend(&script,backend).unwrap();
            let JsValue::Object(function)=i.run_script("make").unwrap() else {panic!("function")};
            let native=matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_));
            if !native && !actor {failures.push(format!("{backend:?} native eligibility unavailable: {script}"));continue;}
            if actor && native {
                let parsed=amazing_js::parse_script(&script).unwrap();
                let StmtKind::Function(item)=&parsed.body[0].kind else {panic!("function AST")};
                function.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(item.body.clone());
            }
            i.run_script_with_backend(setup,backend).unwrap();
            let result=i.run_script_with_backend(drive,backend).and_then(|_|i.drain_microtasks()).and_then(|_|i.run_script("ok"));
            if !matches!(result,Ok(JsValue::Bool(true))) {failures.push(format!("{backend:?} actor={actor} {script}: {result:?}"));}
        }
    }
    failures
}
fn check(failures:Vec<String>){assert!(failures.is_empty(),"{}",failures.join("\n"));}

#[test]
fn excluded_keys_are_filtered_before_descriptor_traps_with_symbol_identity() {
    let mut failures=Vec::new();
    for body in ["var {[excluded]:value,...rest}=source;return rest", "var rest;({[excluded]:value,...rest}=source);return rest"] {
        failures.extend(matrix("function make(source)",body,
            "var value;excluded=Symbol('same');included=Symbol('same');var target={[excluded]:1,[included]:2,a:3};source=new Proxy(target,{ownKeys(){log.push('keys');return [excluded,included,'a']},getOwnPropertyDescriptor(t,k){if(k===excluded)throw marker;log.push('desc:'+String(k===included?'included':k));return Reflect.getOwnPropertyDescriptor(t,k)},get(t,k){log.push('get:'+String(k===excluded?'excluded':k===included?'included':k));return Reflect.get(t,k)}});",
            "var r=make(source);ok=r[included]===2&&r.a===3&&!Object.prototype.hasOwnProperty.call(r,excluded)&&log.join(',')==='get:excluded,keys,desc:included,get:included,desc:a,get:a'"));
    }
    failures.extend(matrix("function make(source)","var {a,...rest}=source;return rest",
        "source=new Proxy({a:1,b:2},{ownKeys(){return ['a','b']},getOwnPropertyDescriptor(t,k){if(k==='a')throw marker;return Reflect.getOwnPropertyDescriptor(t,k)}})","var r=make(source);ok=r.b===2&&!Object.prototype.hasOwnProperty.call(r,'a')"));
    check(failures);
}

#[test]
fn descriptor_and_get_are_interleaved_over_a_single_own_keys_snapshot() {
    let mut failures=Vec::new();
    for body in ["return {...source}","var {...rest}=source;return rest","var rest;({...rest}=source);return rest"] {
        failures.extend(matrix("function make(source)",body,
            "var target={a:1,b:2,c:3};source=new Proxy(target,{ownKeys(){log.push('keys');return ['a','b','c']},getOwnPropertyDescriptor(t,k){log.push('desc:'+k);return Reflect.getOwnPropertyDescriptor(t,k)},get(t,k){log.push('get:'+k);if(k==='a'){delete t.b;Object.defineProperty(t,'c',{enumerable:false});t.d=4}return Reflect.get(t,k)}})",
            "var r=make(source);ok=Reflect.ownKeys(r).join(',')==='a'&&r.a===1&&log.join(',')==='keys,desc:a,get:a,desc:b,desc:c'"));
    }
    check(failures);
}

#[test]
fn copied_properties_are_data_and_bypass_inherited_setters_and_proto_setter() {
    let mut failures=Vec::new();
    for body in ["return {...source}","var {...rest}=source;return rest"] {
        failures.extend(matrix("function make(source)",body,
            "source=Object.create(null);Object.defineProperty(source,'__proto__',{value:marker,enumerable:true});Object.defineProperty(source,'a',{get(){log.push('get');return 7},enumerable:true});Object.defineProperty(Object.prototype,'a',{set(v){log.push('inherited-set')},configurable:true});",
            "var r=make(source),a=Object.getOwnPropertyDescriptor(r,'a'),p=Object.getOwnPropertyDescriptor(r,'__proto__');ok=Object.getPrototypeOf(r)===Object.prototype&&p.value===marker&&a.value===7&&a.writable&&a.enumerable&&a.configurable&&!('get' in a)&&log.join(',')==='get'"));
    }
    failures.extend(matrix("function make(source)","return {get a(){log.push('old-get');return 0},set a(v){log.push('old-set')},...source}","source={a:7}","var r=make(source),d=Object.getOwnPropertyDescriptor(r,'a');ok=r.a===7&&d.value===7&&d.writable&&d.enumerable&&d.configurable&&!('get' in d)&&log.length===0"));
    check(failures);
}

#[test]
fn own_keys_descriptor_get_abrupts_keep_identity_and_stop_later_keys() {
    let mut failures=Vec::new();
    for stage in ["keys","desc:a","get:a"] {
        let setup=format!("source=new Proxy({{a:1,b:2}},{{ownKeys(){{log.push('keys');if('{stage}'==='keys')throw marker;return ['a','b']}},getOwnPropertyDescriptor(t,k){{log.push('desc:'+k);if('{stage}'==='desc:'+k)throw marker;return Reflect.getOwnPropertyDescriptor(t,k)}},get(t,k){{log.push('get:'+k);if('{stage}'==='get:'+k)throw marker;return Reflect.get(t,k)}}}})");
        let expected=match stage {"keys"=>"keys","desc:a"=>"keys,desc:a",_=>"keys,desc:a,get:a"};
        for body in ["return {...source}","var {...rest}=source;return rest"] {
            let drive=format!("try{{make(source)}}catch(e){{ok=e===marker&&log.join(',')==='{expected}'}}");
            failures.extend(matrix("function make(source)",body,&setup,&drive));
        }
    }
    check(failures);
}

#[test]
fn proxy_key_invariants_and_revocation_remain_semantic() {
    let mut failures=Vec::new();
    for setup in ["source=new Proxy({a:1},{ownKeys(){return ['a','a']}})","var pair=Proxy.revocable({a:1},{});source=pair.proxy;pair.revoke()","var t={};Object.defineProperty(t,'a',{value:1});source=new Proxy(t,{ownKeys(){return []}})"] {
        failures.extend(matrix("function make(source)","return {...source}",setup,"try{make(source)}catch(e){ok=e instanceof TypeError}"));
    }
    check(failures);
}

#[test]
fn primitive_boxing_keeps_utf16_units_and_nullish_caller_rules() {
    let mut failures=Vec::new();
    for body in ["return {...source}","var {...rest}=source;return rest"] {
        failures.extend(matrix("function make(source)",body,"source='\\uD800A\\uDC00';Object=function(){throw marker};",
            "var r=make(source);ok=r[0].charCodeAt(0)===0xd800&&r[1]==='A'&&r[2].charCodeAt(0)===0xdc00&&r.length===undefined&&r[3]===undefined"));
        for primitive in ["true","1","1n","Symbol('x')"] {
            failures.extend(matrix("function make(source)",body,&format!("source={primitive}"),"ok=Reflect.ownKeys(make(source)).length===0"));
        }
    }
    failures.extend(matrix("function make(source)","return {...source}","","ok=Reflect.ownKeys(make(null)).length===0&&Reflect.ownKeys(make(undefined)).length===0"));
    failures.extend(matrix("function make(source)","var {...rest}=source;return rest","","var count=0;for(var v of [null,undefined]){try{make(v)}catch(e){if(e instanceof TypeError)count++}}ok=count===2"));
    check(failures);
}

#[test]
fn declaration_parameter_assignment_and_nested_rest_share_the_same_protocol() {
    let mut failures=Vec::new();
    for (signature,body) in [("function make(source)","let {a,...r}=source;return r"),("function make(source)","const {a,...r}=source;return r"),("function make({a,...r})","return r"),("function make(source)","var r;({nested:{a,...r}}={nested:source});return r"),("function make(source)","var r,holder={};({a,...holder.r}=source);return holder.r")] {
        failures.extend(matrix(signature,body,"source=new Proxy({a:1,b:2},{getOwnPropertyDescriptor(t,k){if(k==='a')throw marker;return Reflect.getOwnPropertyDescriptor(t,k)}})","var r=make(source);ok=r.b===2&&!Object.prototype.hasOwnProperty.call(r,'a')"));
    }
    check(failures);
}

#[test]
fn suspended_source_and_rest_assignment_preserve_the_captured_destination() {
    let mut failures=Vec::new();
    failures.extend(matrix("function* make(source)","return {...(yield 'source')}","source={a:7}","var g=make(),first=g.next(),last=g.next(source);ok=first.value==='source'&&last.done&&last.value.a===7"));
    failures.extend(matrix("async function make(source)","return {...await Promise.resolve(source)}","source={a:7}","make(source).then(r=>ok=r.a===7)"));
    failures.extend(matrix("function* make(source)","var original={};var holder=original;({...holder.r}=yield 'source');return original.r", "source={a:7}","var g=make(),first=g.next(),last=g.next(source);ok=first.value==='source'&&last.done&&last.value.a===7"));
    failures.extend(matrix("async function* make(source)","var {a,...r}=await Promise.resolve(source);yield r", "source=new Proxy({a:1,b:2},{getOwnPropertyDescriptor(t,k){if(k==='a')throw marker;return Reflect.getOwnPropertyDescriptor(t,k)}})","var g=make(source);g.next().then(r=>ok=!r.done&&r.value.b===2&&!Object.prototype.hasOwnProperty.call(r.value,'a'))"));
    check(failures);
}

#[test]
fn mapped_arguments_and_binary_sources_use_live_semantic_descriptors() {
    let mut failures=Vec::new();
    for body in ["return {...source}","var {...r}=source;return r"] {
        failures.extend(matrix("function make(source)",body,
            "source=(function(a){var args=arguments;Object.defineProperty(args,'z',{get(){a=7;return 3},enumerable:true});return new Proxy(args,{ownKeys(){return ['z','0']},getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)},get(t,k){return Reflect.get(t,k)}})})(1)",
            "var r=make(source);ok=r.z===3&&r[0]===7&&Object.getOwnPropertyDescriptor(r,'0').value===7"));
        failures.extend(matrix("function make(source)",body,"source=new Uint8Array([1,2,255])","var r=make(source);ok=r[0]===1&&r[1]===2&&r[2]===255&&Reflect.ownKeys(r).join(',')==='0,1,2'"));
    }
    check(failures);
}

#[test]
fn namespace_exports_are_live_sources_but_copied_results_are_ordinary_snapshots() {
    use amazing_js::module::ModuleLinker;
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        for actor in [false,true] {
            for body in ["return {...source}","var {...r}=source;return r"] {
                let mut i=Interpreter::new();let script=format!("function make(source){{{body}}}");
                i.run_script_with_backend(&script,backend).unwrap();
                let JsValue::Object(function)=i.run_script("make").unwrap() else {panic!("function")};
                assert!(matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),"actual native factory");
                if actor {
                    let parsed=amazing_js::parse_script(&script).unwrap();
                    let StmtKind::Function(item)=&parsed.body[0].kind else {panic!("function AST")};
                    function.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(item.body.clone());
                }
                let mut modules=ModuleLinker::new(|_:&str,_:&str|->Result<(String,String),String>{Err("no dependencies".into())});
                modules.link("fixture","export let a=1;export const b=2;export function update(){a=7}").unwrap();
                modules.evaluate("fixture",&mut i).unwrap();
                let namespace=modules.namespace_object("fixture").unwrap();let global=i.binding_global_object();
                i.binding_set_property(&global,"source",namespace).unwrap();
                let value=i.run_script_with_backend("var first=make(source);source.update();var second=make(source);first.a===1&&second.a===7&&first.b===2&&first.update===source.update&&Object.getPrototypeOf(first)===Object.prototype&&Object.getOwnPropertyDescriptor(first,'a').writable&&Reflect.ownKeys(first).join(',')==='a,b,update'",backend).unwrap();
                assert_eq!(value,JsValue::Bool(true),"{backend:?} actor={actor} {body}");
            }
        }
    }
}

#[test]
fn primitive_copying_inside_construction_retains_pure_to_object_behavior() {
    let mut failures=Vec::new();
    failures.extend(matrix("function make(source)","class C{constructor(){this.copy={...source}}}return new C().copy", "source='AB'", "var r=make(source);ok=r[0]==='A'&&r[1]==='B'&&Object.getPrototypeOf(r)===Object.prototype"));
    failures.extend(matrix("function make(source)","return {...source}","source='AB'", "var copied;var newTarget=new Proxy(function(){},{get(t,k){if(k==='prototype'){copied=make(source);return Object.prototype}return Reflect.get(t,k)}});var constructed=Reflect.construct(Object,[],newTarget);ok=copied[0]==='A'&&copied[1]==='B'&&Object.getPrototypeOf(constructed)===Object.prototype&&Object.getPrototypeOf(copied)===Object.prototype"));
    failures.extend(matrix("function make(source)","var {...r}=source;return r", "source=new Proxy(new String('AB'),{})", "var r=make(source);ok=r[0]==='A'&&r[1]==='B'&&Reflect.ownKeys(r).join(',')==='0,1'"));
    check(failures);
}

#[test]
fn foreign_primitive_wrappers_and_proxy_sources_keep_caller_destination_realm() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        for actor in [false,true] {
            let mut i=Interpreter::new();let script="function make(source){return {...source}}";
            i.run_script_with_backend(script,backend).unwrap();
            let JsValue::Object(function)=i.run_script("make").unwrap() else {panic!("function")};
            assert!(matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)));
            if actor {
                let parsed=amazing_js::parse_script(script).unwrap();let StmtKind::Function(item)=&parsed.body[0].kind else{panic!("function AST")};
                function.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(item.body.clone());
            }
            let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for source in ["new String('\\uD800A')","new Proxy(new String('\\uD800A'),{})"] {
                let value=i.binding_eval_in_realm(child,source).unwrap();let global=i.binding_global_object();
                i.binding_set_property(&global,"source",value).unwrap();
                let result=i.run_script_with_backend("var r=make(source);r[0].charCodeAt(0)===0xd800&&r[1]==='A'&&Object.getPrototypeOf(r)===Object.prototype&&Object.getPrototypeOf(source)!==Object.prototype&&Reflect.ownKeys(r).join(',')==='0,1'",backend).unwrap();
                assert_eq!(result,JsValue::Bool(true),"{backend:?} actor={actor} {source}");
            }
        }
    }
}
