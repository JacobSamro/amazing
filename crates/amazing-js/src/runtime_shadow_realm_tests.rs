use super::*;

fn matrix(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
        assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}: {source}");
    }
}

#[test]
fn constructor_brand_descriptors_subclass_and_host_global_isolation() {
    matrix(r#"(()=>{let p=ShadowRealm.prototype,d=Object.getOwnPropertyDescriptor(ShadowRealm,'prototype'),s=new ShadowRealm();
      let caught=false;try{ShadowRealm()}catch(e){caught=e instanceof TypeError}
      class Child extends ShadowRealm{}let c=new Child();
      return caught&&!d.writable&&!d.enumerable&&!d.configurable&&ShadowRealm.length===0&&
      p.evaluate.length===1&&p.importValue.length===2&&Object.prototype.toString.call(s)==='[object ShadowRealm]'&&
      Object.getPrototypeOf(c)===Child.prototype&&c.evaluate('2')===2&&
      s.evaluate('typeof location+","+typeof localStorage+","+typeof fetch')==='undefined,undefined,undefined';})()"#);
    matrix(r#"(()=>{let count=0;for(let v of [null,undefined,{},ShadowRealm.prototype,new Proxy(new ShadowRealm(),{})])
      try{ShadowRealm.prototype.evaluate.call(v,'1')}catch(e){if(e instanceof TypeError)count++}return count===5})()"#);
}

#[test]
fn evaluate_primitive_utf16_and_fresh_lexical_vs_persistent_var_environments() {
    matrix(r#"(()=>{let s=new ShadowRealm();return s.evaluate('')===undefined&&s.evaluate('null')===null&&
      s.evaluate('true')===true&&s.evaluate('123n')===123n&&s.evaluate('Symbol.for("shared")')===Symbol.for('shared')&&
      s.evaluate('"\\ud800"')==='\ud800'&&s.evaluate('var x=3;let y=4;x+y')===7&&
      s.evaluate('x')===3&&s.evaluate('typeof y')==='undefined'&&s.evaluate('let y=5;y')===5&&
      s.evaluate('"use strict";var z=7;z')===7&&s.evaluate('typeof z')==='undefined'&&typeof x==='undefined'})()"#);
}

#[test]
fn parse_and_runtime_errors_are_distinct_caller_intrinsics_without_error_gets() {
    matrix(r#"(()=>{let s=new ShadowRealm(),syntax=0,type=0;for(let text of ['return 1','super.x','new.target','let ='])
      try{s.evaluate(text)}catch(e){if(e instanceof SyntaxError)syntax++}
      for(let text of ['throw 1','throw {get name(){throw 2},get message(){throw 3}}','({})','[]','Promise.resolve(1)'])
      try{s.evaluate(text)}catch(e){if(e instanceof TypeError)type++}
      let coercions=0;try{s.evaluate({toString(){coercions++;return '1'}})}catch(e){if(e instanceof TypeError)type++}
      return syntax===4&&type===6&&coercions===0&&s.evaluate('1')===1})()"#);
}

#[test]
fn wrapped_functions_transfer_both_directions_and_rewrap_every_callable() {
    matrix(r#"(()=>{let s=new ShadowRealm(),f=s.evaluate('(x,cb)=>cb(x)+1'),identity=s.evaluate('x=>x');
      let cb=x=>x*2,a=identity(cb),b=identity(cb);return f(4,cb)===9&&a!==cb&&a!==b&&a(3)===6&&
      Object.getPrototypeOf(f)===Function.prototype&&!Object.hasOwn(f,'prototype')&&
      identity(Symbol.for('x'))===Symbol.for('x')&&s.evaluate('()=>()=>7')()()===7})()"#);
}

#[test]
fn wrapped_call_this_argument_result_rejection_and_abrupt_cleanup() {
    matrix(r#"(()=>{let s=new ShadowRealm(),f=s.evaluate('(function f(x){return x})'),receiver=s.evaluate('(function f(){"use strict";return this})');
      let caught=0;for(let value of [{},[],new Number(1)])try{f(value)}catch(e){if(e instanceof TypeError)caught++}
      try{receiver.call({})}catch(e){if(e instanceof TypeError)caught++}
      try{s.evaluate('()=>({})')()}catch(e){if(e instanceof TypeError)caught++}
      try{s.evaluate('cb=>cb()')(()=>{throw {get message(){throw 2}}})}catch(e){if(e instanceof TypeError)caught++}
      try{new f()}catch(e){if(e instanceof TypeError)caught++}
      return caught===7&&receiver.call(3)===3&&receiver.call(null)===null&&f(7)===7})()"#);
}

#[test]
fn copied_name_length_uses_exact_units_and_semantic_order() {
    matrix(r#"(()=>{let s=new ShadowRealm();s.evaluate('var log=[]');let f=s.evaluate(`new Proxy(function(){},{
      getOwnPropertyDescriptor(t,k){if(k==='length')log.push('own');return Reflect.getOwnPropertyDescriptor(t,k)},
      get(t,k,r){if(k==='length'){log.push('length');return 3.9}if(k==='name'){log.push('name');return '\\ud800'}return Reflect.get(t,k,r)}})`);
      return f.length===3&&f.name==='\ud800'&&s.evaluate('log.join(",")')==='own,length,name'})()"#);
    matrix(r#"(()=>{let s=new ShadowRealm(),caught=false;try{s.evaluate('Object.defineProperty(function(){},"name",{get(){throw {get message(){throw 2}}}})')}catch(e){caught=e instanceof TypeError}return caught})()"#);
}

fn source(specifier: &str, _: &str) -> Result<(String, String), String> {
    match specifier {
        "state" => Ok((specifier.into(), "export let value=undefined;let n=0;export function count(){return ++n}export const object={};export const symbol=Symbol.for('shared');".into())),
        "pending" => Ok((specifier.into(), "await Promise.resolve();export const value=9;".into())),
        "throw" => Ok((specifier.into(), "throw {get message(){throw 2}};export const value=1;".into())),
        _ => Err("not found".into()),
    }
}

fn module_fixture() -> Interpreter {
    type Linker = crate::module::ModuleLinker<fn(&str, &str) -> Result<(String, String), String>>;
    let registry = Rc::new(RefCell::new(HashMap::<RealmHandle, Rc<RefCell<Linker>>>::new()));
    let mut i = Interpreter::new();
    i.binding_set_phase_module_loader_hook(Rc::new(move |i, request, referrer| {
        let key = i.binding_current_realm();
        let linker = registry.borrow_mut().entry(key).or_insert_with(|| Rc::new(RefCell::new(Linker::new(source)))).clone();
        crate::module::ModuleLinker::import_shared(&linker, request, referrer, i)
            .map_err(|failure| failure.thrown.unwrap_or_else(|| Throw::type_error(failure.message)))
    }));
    i
}

#[test]
fn import_value_real_modules_are_cached_per_realm_and_undefined_is_an_export() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i=module_fixture();i.force_ast_callables=backend==ExecutionBackend::TreeWalk;
        i.run_script_with_backend(r#"var a=new ShadowRealm(),b=new ShadowRealm(),results=[];
          a.importValue('state','count').then(f=>results.push(f(),f()));
          a.importValue('state','count').then(f=>results.push(f()));
          b.importValue('state','count').then(f=>results.push(f()));
          a.importValue('state','value').then(v=>results.push(v===undefined));"#,backend).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("results.join(',')==='1,2,3,1,true'").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn import_value_sync_validation_and_async_error_copy_pending_module_completion() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i=module_fixture();i.force_ast_callables=backend==ExecutionBackend::TreeWalk;
        i.run_script_with_backend(r#"var s=new ShadowRealm(),sync=0,async=0,pending=false;
          try{s.importValue({toString(){throw 7}},'value')}catch(e){if(e===7)sync++}
          try{s.importValue('state',{})}catch(e){if(e instanceof TypeError)sync++}
          for(let pair of [['state','missing'],['state','object'],['throw','value'],['missing','value']])
            s.importValue(pair[0],pair[1]).catch(e=>{if(e instanceof TypeError)async++});
          s.importValue('pending','value').then(v=>pending=v===9);"#,backend).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("sync===2&&async===4&&pending").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn foreign_method_wrappers_and_errors_keep_callee_realm_intrinsics() {
    let mut i=Interpreter::new();
    let foreign=i.binding_create_realm(i.object_value()).unwrap();
    let constructor=i.binding_eval_in_realm(foreign,"ShadowRealm").unwrap();
    let function_proto=i.binding_eval_in_realm(foreign,"Function.prototype").unwrap();
    let type_proto=i.binding_eval_in_realm(foreign,"TypeError.prototype").unwrap();
    i.define_global("ForeignShadow",constructor.clone());
    i.define_global("foreignFunctionProto",function_proto);
    i.define_global("foreignTypeProto",type_proto);
    i.binding_eval_in_realm(foreign,"TypeError=()=>{throw 7};Promise=undefined;ShadowRealm=undefined").unwrap();
    assert_eq!(i.run_script(r#"(()=>{let s=new ForeignShadow(),f=s.evaluate('()=>7');let caught=false;
      try{s.evaluate('throw {get message(){throw 2}}')}catch(e){caught=Object.getPrototypeOf(e)===foreignTypeProto}
      return Object.getPrototypeOf(f)===foreignFunctionProto&&f()===7&&caught})()"#).unwrap(),JsValue::Bool(true));
}

#[test]
fn metadata_numeric_boundaries_and_target_prototype_never_supply_wrapper_identity() {
    matrix(r#"(()=>{let s=new ShadowRealm();
      for(let pair of [['Infinity',Infinity],['-Infinity',0],['NaN',0],['3.9',3],['-3.9',0],['"7"',0]]){
        let f=s.evaluate('Object.defineProperty(()=>1,"length",{value:'+pair[0]+'})');
        if(f.length!==pair[1]||f()!==1)return false;
      }
      let f=s.evaluate('let f=()=>1;delete f.length;Object.setPrototypeOf(f,{get length(){throw 3}});f');
      let p=s.evaluate('new Proxy(()=>2,{getPrototypeOf(){throw 7}})');
      return f.length===0&&f()===1&&p()===2&&Object.getPrototypeOf(p)===Function.prototype;})()"#);
}

#[test]
fn eval_declaration_instantiation_keeps_errors_and_captured_lexicals_in_the_inner_realm() {
    matrix(r#"(()=>{let s=new ShadowRealm();let captured=s.evaluate('let x=3;()=>++x');
      let next=s.evaluate('let x=10;()=>++x');let caught=false;
      try{s.evaluate('let x;var x')}catch(e){caught=e instanceof SyntaxError}
      s.evaluate('Object.defineProperty(globalThis,"blocked",{value:1,configurable:false});undefined');
      let restricted=false;try{s.evaluate('function blocked(){}')}catch(e){restricted=e instanceof TypeError}
      return captured()===4&&captured()===5&&next()===11&&caught&&restricted&&s.evaluate('typeof x')==='undefined';})()"#);
}
