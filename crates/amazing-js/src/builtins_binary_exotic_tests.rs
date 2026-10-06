#[cfg(test)]
mod binary_exotic_protocol_tests {
    use super::*;

    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut interpreter = Interpreter::new();
            assert_eq!(
                interpreter
                    .run_script_with_backend(source, backend)
                    .unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe())),
                JsValue::Bool(true)
            );
        }
    }

    const PRELUDE: &str = r#"
      function check(ok,label){if(!ok)throw label;}
      var ctors=[Int8Array,Uint8Array,Uint8ClampedArray,Int16Array,Uint16Array,Int32Array,Uint32Array,BigInt64Array,BigUint64Array,Float16Array,Float32Array,Float64Array];
      function num(C,n){return C===BigInt64Array||C===BigUint64Array?BigInt(n):n;}
    "#;

    #[test]
    fn all_kinds_inherited_set_preserves_original_receiver_and_canonical_boundary() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var a=new C(2),n=num(C,7),child=Object.create(a),calls=0;
            var upper=Object.create(Object.getPrototypeOf(a));
            Object.defineProperty(upper,'0',{set(){calls++;},get(){return 91;}});
            Object.defineProperty(upper,'3',{set(){calls++;},get(){return 92;}});
            Object.setPrototypeOf(a,upper);
            check(Reflect.set(child,'0',n),'inherited valid set');
            check(Object.hasOwn(child,'0')&&child[0]===n&&a[0]===num(C,0),'receiver shadow');
            var poison={valueOf(){throw 'unexpected conversion';}};
            check(Reflect.set(child,'3',poison)&&!Object.hasOwn(child,'3')&&calls===0,'invalid inherited no conversion or setter');
            check(child[3]===undefined&&!Reflect.has(child,'3'),'invalid inherited get and has stop');
            check(Reflect.set(a,'-0',poison,{})&&!Reflect.has(child,'-0'),'negative zero distinct receiver');
            check(!Reflect.set(a,'0',n,1)&&Reflect.set(a,'3',poison,1),'primitive receiver branch');
            check(!Reflect.set(a,'0',n,Object.preventExtensions({})),'nonextensible receiver');
            var frozen={};Object.defineProperty(frozen,'0',{value:1});
            check(!Reflect.set(a,'0',n,frozen),'nonwritable receiver');
            var accessor={};Object.defineProperty(accessor,'0',{set(){throw 'receiver setter';}});
            check(!Reflect.set(a,'0',n,accessor),'receiver accessor not invoked');
          }true;
        "#
        ));
    }

    #[test]
    fn all_kinds_same_receiver_conversion_precedes_validity_and_abrupt_completion() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var a=new C(1),calls=0,n=num(C,4),value={valueOf(){calls++;return n;}};
            check(Reflect.set(a,'NaN',value)&&calls===1,'invalid converts');
            check(Reflect.set(a,'0',value)&&calls===2&&a[0]===n,'valid converts once');
            var marker={};try{Reflect.set(a,'Infinity',{valueOf(){throw marker;}});throw 'missing abrupt';}catch(e){check(e===marker,'abrupt identity');}
            var b=a.buffer;b.transfer();calls=0;
            check(Reflect.set(a,'0',value)&&calls===1,'detached converts then no-op');
            check(!Reflect.defineProperty(a,'0',{value:value})&&calls===1,'define detached skips conversion');
          }true;
        "#
        ));
    }

    #[test]
    fn all_kinds_define_and_set_revalidate_after_detach_or_resize_callbacks() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var op of ['set','define']){for(var change of ['detach','shrink','regrow']){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(2*size,{maxByteLength:4*size}),a=new C(b,0,2),calls=0,n=num(C,5);
            var value={valueOf(){calls++;if(change==='detach')b.transfer();else{b.resize(0);if(change==='regrow')b.resize(2*size);}return n;}};
            check(op==='set'?Reflect.set(a,'0',value):Reflect.defineProperty(a,'0',{value:value}),'conversion mutation succeeds');
            check(calls===1,'one conversion');
            check(change==='regrow'?a[0]===n:a[0]===undefined,'fresh witness after callback');
          }}}true;
        "#
        ));
    }

    #[test]
    fn all_kinds_mutable_partial_descriptors_and_typed_receiver_define_are_distinct() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var a=new C(1),n=num(C,3),calls=0,poison={valueOf(){calls++;throw 'coercion';}};
            check(Reflect.defineProperty(a,'0',{})&&Reflect.defineProperty(a,'0',{value:n}),'partial descriptor');
            for(var desc of [{writable:false,value:poison},{enumerable:false,value:poison},{configurable:false,value:poison},{get:undefined},{set:undefined}])check(!Reflect.defineProperty(a,'0',desc),'incompatible descriptor');
            check(!Reflect.defineProperty(a,'1',{value:poison})&&calls===0,'invalid before conversion');
            var d=Object.getOwnPropertyDescriptor(a,'0');check(d.value===n&&d.writable&&d.enumerable&&d.configurable,'virtual flags');
            check(Reflect.set({},'0',n,a)&&a[0]===n,'ordinary target typed receiver');
            check(!Reflect.set({},'1',poison,a)&&calls===0,'ordinary target invalid typed receiver rejects');
            check(!Reflect.deleteProperty(a,'0')&&Reflect.deleteProperty(a,'1'),'delete validity');
          }true;
        "#
        ));
    }

    #[test]
    fn all_kinds_immutable_elements_reject_set_before_conversion_and_use_same_value_define() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var b=new ArrayBuffer(C.BYTES_PER_ELEMENT),original=new C(b);original[0]=num(C,3);
            var a=new C(b.transferToImmutable()),calls=0,value={valueOf(){calls++;return num(C,3);}};
            for(var key of ['0','1','-0','NaN'])check(!Reflect.set(a,key,value)&&!Reflect.set(a,key,value,{}),'immutable set false');
            check(calls===0,'immutable no coercion');
            var d=Object.getOwnPropertyDescriptor(a,'0'),r=Reflect.getOwnPropertyDescriptor(a,'0');
            check(!d.writable&&!d.configurable&&d.enumerable&&!r.writable&&!r.configurable,'immutable descriptor');
            check(Reflect.defineProperty(a,'0',{})&&Reflect.defineProperty(a,'0',{value:num(C,3),writable:false,configurable:false,enumerable:true}),'compatible immutable define');
            check(!Reflect.defineProperty(a,'0',{value:value})&&!Reflect.defineProperty(a,'0',{value:num(C,4)})&&calls===0,'same value no coercion');
            check(!Reflect.defineProperty(a,'0',{writable:true})&&!Reflect.defineProperty(a,'0',{configurable:true}),'immutable flags');
            a[0]=value;check(calls===0&&a[0]===num(C,3),'sloppy immutable');
            try{(function(){'use strict';a[0]=value;})();throw 'missing strict failure';}catch(e){check(e instanceof TypeError,'strict immutable TypeError');}
            check(!Reflect.deleteProperty(a,'0')&&Reflect.deleteProperty(a,'1'),'immutable delete');
          }
          var b=new ArrayBuffer(16),a=new Float64Array(b);a[0]=NaN;a[1]=-0;a=new Float64Array(b.transferToImmutable());
          check(Reflect.defineProperty(a,'0',{value:NaN})&&Reflect.defineProperty(a,'1',{value:-0})&&!Reflect.defineProperty(a,'1',{value:0}),'immutable SameValue NaN and signed zero');true;
        "#
        ));
    }

    #[test]
    fn receiver_proxy_and_descriptor_proxy_observe_real_trap_order() {
        both(
            r#"
          function check(ok,label){if(!ok)throw label;}
          var a=new Uint8Array(1),events=[],backing={},receiver=new Proxy(backing,{
            getOwnPropertyDescriptor(t,k){events.push('own:'+k);return Reflect.getOwnPropertyDescriptor(t,k);},
            defineProperty(t,k,d){events.push('define:'+k+':'+d.value+':'+d.writable);return Reflect.defineProperty(t,k,d);}
          });
          check(Reflect.set(a,'0',7,receiver)&&events.join(',')==='own:0,define:0:7:true'&&backing[0]===7&&a[0]===0,'receiver traps');
          events=[];check(Reflect.set(a,'1',7,receiver)&&events.length===0,'invalid no receiver traps');
          var descriptor=new Proxy({value:8},{has(t,k){events.push('has:'+k);return Reflect.has(t,k);},get(t,k){events.push('get:'+k);return Reflect.get(t,k);}});
          check(Reflect.defineProperty(a,'0',descriptor)&&a[0]===8,'descriptor proxy');
          check(events.join(',')==='has:enumerable,has:configurable,has:value,get:value,has:writable,has:get,has:set','descriptor has/get order');
          var marker={};var p=new Proxy(a,{set(){throw marker;}});try{Reflect.set(p,'0',1);throw 'missing trap';}catch(e){check(e===marker,'target proxy abrupt');}
          true;
        "#,
        );
    }

    #[test]
    fn exact_names_symbols_canonical_spellings_and_with_use_native_prototype_dispatch() {
        both(
            r#"
          function check(ok,label){if(!ok)throw label;}
          var a=new Uint8Array(2),s=Symbol('s'),names=['01','1.0','+1',' 1','-0.0','1e0','\ud800','\ufffd'];
          for(var name of names)a[name]=name;a[s]=9;
          for(var name of names)check(a[name]===name&&Reflect.has(a,name),'ordinary exact name');
          check(a['\ud800']!==a['\ufffd'],'distinct UTF16 names');
          var keys=Reflect.ownKeys(a);check(keys[0]==='0'&&keys[1]==='1'&&keys.slice(2,-1).join('|')===names.join('|')&&keys[keys.length-1]===s,'key order');
          var child=Object.create(a),upper=Object.create(Object.getPrototypeOf(a));upper.NaN=99;Object.setPrototypeOf(a,upper);
          check(!('NaN' in child)&&child.NaN===undefined,'canonical inherited stop');
          function scope(){var NaN=17;with(child){NaN=18;}return NaN;}
          check(scope()===18&&!Object.hasOwn(child,'NaN'),'With Has binding');
          var b=a.buffer;b.transfer();check(Reflect.ownKeys(a).length===names.length+1,'detached own keys retain expandos');true;
        "#,
        );
    }

    #[test]
    fn all_kinds_fixed_tracking_shared_and_regrown_views_use_current_bounds() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var shared of [false,true]){
            var size=C.BYTES_PER_ELEMENT,b=shared?new SharedArrayBuffer(2*size,{maxByteLength:4*size}):new ArrayBuffer(2*size,{maxByteLength:4*size});
            var fixed=new C(b,0,2),tracking=new C(b),n=num(C,6);
            if(shared)b.grow(4*size);else b.resize(4*size);
            check(Reflect.ownKeys(fixed).join(',')==='0,1'&&Reflect.ownKeys(tracking).join(',')==='0,1,2,3','current fixed/tracking keys');
            check(Reflect.defineProperty(tracking,'3',{value:n})&&tracking[3]===n&&!Reflect.defineProperty(fixed,'3',{value:n}),'grown validity');
            if(!shared){
              b.resize(size);check(Reflect.ownKeys(fixed).length===0&&Reflect.ownKeys(tracking).join(',')==='0','OOB fixed and live tracking');
              check(!Reflect.has(fixed,'0')&&fixed[0]===undefined&&Reflect.deleteProperty(fixed,'0'),'OOB protocol');
              b.resize(4*size);check(Reflect.has(fixed,'0')&&Reflect.ownKeys(fixed).length===2,'regrown fixed');
            }
          }}true;
        "#
        ));
    }

    #[test]
    fn descriptor_field_callbacks_precede_index_witness_and_receiver_callbacks_do_not_retarget() {
        both(
            r#"
          function check(ok,label){if(!ok)throw label;}
          var b=new ArrayBuffer(2,{maxByteLength:4}),a=new Uint8Array(b),calls=0;
          var desc={get value(){b.resize(0);return {valueOf(){calls++;return 7;}};}};
          check(!Reflect.defineProperty(a,'0',desc)&&calls===0,'descriptor getter invalidates before conversion');
          b.resize(2);var receiver=new Proxy({}, {
            getOwnPropertyDescriptor(){b.resize(0);return undefined;},
            defineProperty(t,k,d){return Reflect.defineProperty(t,k,d);}
          });
          check(Reflect.set(a,'0',9,receiver)&&receiver[0]===9&&a[0]===undefined,'receiver callback preserves descriptor decision');
          var marker={};b.resize(2);desc=new Proxy({}, {has(){throw marker;}});
          try{Reflect.defineProperty(a,'0',desc);throw 'missing descriptor abrupt';}catch(e){check(e===marker,'descriptor abrupt identity');}
          true;
        "#,
        );
    }

    #[test]
    fn immutable_virtual_descriptors_enforce_actual_proxy_invariants() {
        both(
            r#"
          function check(ok,label){if(!ok)throw label;}
          function rejects(f){try{f();return false;}catch(e){return e instanceof TypeError;}}
          var b=new ArrayBuffer(1);new Uint8Array(b)[0]=7;var a=new Uint8Array(b.transferToImmutable());
          check(rejects(()=>Reflect.get(new Proxy(a,{get(){return 8;}}),'0')),'get invariant');
          check(rejects(()=>Reflect.set(new Proxy(a,{set(){return true;}}),'0',8)),'set invariant');
          check(rejects(()=>Reflect.has(new Proxy(a,{has(){return false;}}),'0')),'has invariant');
          check(rejects(()=>Reflect.deleteProperty(new Proxy(a,{deleteProperty(){return true;}}),'0')),'delete invariant');
          check(rejects(()=>Reflect.ownKeys(new Proxy(a,{ownKeys(){return [];}}))),'ownKeys invariant');
          check(Reflect.set(new Proxy(a,{set(){return true;}}),'0',7),'SameValue lying set permitted');
          var p=new Proxy(a,{});check(p[0]===7&&Reflect.has(p,'0')&&!Reflect.set(p,'0',7)&&!Reflect.deleteProperty(p,'0'),'transparent immutable');
          true;
        "#,
        );
    }

    #[test]
    fn all_kinds_transparent_proxy_keeps_distinct_actual_receiver_semantics() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var a=new C(1),p=new Proxy(a,{}),calls=0,n=num(C,5),value={valueOf(){calls++;return n;}};
            check(Reflect.set(p,'0',value)&&a[0]===n&&calls===1,'transparent valid receiver define');
            calls=0;check(Reflect.set(p,'1',value)&&calls===0,'transparent invalid distinct no conversion');
            check(Reflect.set(a,'1',value)&&calls===1,'direct invalid same receiver converts');
            check(p[0]===n&&Reflect.has(p,'0')&&!Reflect.deleteProperty(p,'0'),'transparent virtual methods');
          }true;
        "#
        ));
    }

    #[test]
    fn delegated_symbol_has_observes_actual_inherited_proxy_traps_and_abrupt_identity() {
        both(
            r#"
          function check(ok,label){if(!ok)throw label;}
          var s=Symbol('lookup'),marker={},calls=0,a=new Uint8Array(1);
          var p=new Proxy({}, {has(t,k){check(k===s,'exact symbol');calls++;return true;}});
          Object.setPrototypeOf(a,p);var child=Object.create(a);
          check(Reflect.has(a,s)&&Reflect.has(child,s)&&(s in a)&&(s in child)&&calls===4,'actual symbol Has delegation');
          a[s]=7;check(Reflect.has(a,s)&&calls===4,'own symbol before inherited trap');
          delete a[s];Object.setPrototypeOf(a,new Proxy({}, {has(){throw marker;}}));
          try{Reflect.has(a,s);throw 'missing Reflect abrupt';}catch(e){check(e===marker,'Reflect abrupt identity');}
          try{s in a;throw 'missing in abrupt';}catch(e){check(e===marker,'in abrupt identity');}
          true;
        "#,
        );
    }

    #[test]
    fn own_keys_does_not_truncate_large_actual_element_count() {
        let mut interpreter = Interpreter::new();
        let value = interpreter.run_script("new Uint8Array(1000001)").unwrap();
        let JsValue::Object(object) = value else {
            panic!("typed array object");
        };
        let keys = binary_own_keys(&object).unwrap();
        assert_eq!(keys.len(), 1000001);
        assert_eq!(keys.last(), Some(&PropertyKey::from("1000000")));
    }
}
