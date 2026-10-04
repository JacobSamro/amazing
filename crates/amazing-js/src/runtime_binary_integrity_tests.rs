#[cfg(test)]
mod binary_integrity_protocol_tests {
    use super::*;

    #[test]
    fn whole_typed_array_integrity_matrix_covers_fixed_tracking_oob_detached_and_shared() {
        let source = r#"
          function check(ok,label){if(!ok)throw label;}
          function make(C,tag){
            var size=C.BYTES_PER_ELEMENT,b,v;
            if(tag==='fixed-zero')b=new ArrayBuffer(0);
            else if(tag==='sab-fixed')b=new SharedArrayBuffer(4*size);
            else if(tag.startsWith('gsb'))b=new SharedArrayBuffer(4*size,{maxByteLength:8*size});
            else if(tag.startsWith('rab'))b=new ArrayBuffer(4*size,{maxByteLength:8*size});
            else b=new ArrayBuffer(4*size);
            if(tag==='fixed-zero'||tag==='gsb-zero')v=new C(b,0,0);
            else if(tag==='fixed-auto'||tag.endsWith('track'))v=new C(b);
            else v=new C(b,0,2);
            if(tag.startsWith('rab-oob'))b.resize(0);
            if(tag.endsWith('detached'))b.transfer();
            Object.defineProperty(v,'control',{value:1,writable:true,enumerable:true,configurable:true});
            return v;
          }
          var ctors=[Int8Array,Uint8Array,Uint8ClampedArray,Int16Array,Uint16Array,Int32Array,Uint32Array,Float16Array,Float32Array,Float64Array,BigInt64Array,BigUint64Array];
          var tags=['fixed','fixed-auto','fixed-zero','sab-fixed','gsb-fixed','gsb-zero','gsb-track','rab-fixed','rab-track','rab-oob-fixed','rab-oob-track','fixed-detached','rab-detached'];
          for(var C of ctors){for(var tag of tags){for(var op of ['reflect','prevent','seal','freeze']){
            var v=make(C,tag),label=C.name+'/'+tag+'/'+op,oldProto=Object.getPrototypeOf(v),length=v.length;
            var fixed=tag.startsWith('fixed')||tag==='sab-fixed'||tag==='gsb-fixed'||tag==='gsb-zero';
            var expectedThrow=op!=='reflect' && (!fixed || ((op==='seal'||op==='freeze') && length>0));
            var threw=false,result;
            try{result=op==='reflect'?Reflect.preventExtensions(v):op==='prevent'?Object.preventExtensions(v):op==='seal'?Object.seal(v):Object.freeze(v);}
            catch(e){threw=true;check(Object.getPrototypeOf(e)===TypeError.prototype,label+'/native TypeError');}
            check(threw===expectedThrow,label+'/outcome');
            if(!threw)check(result===(op==='reflect'?fixed:v),label+'/identity');
            check(Object.isExtensible(v)===!fixed,label+'/extensibility');
            check(Object.getPrototypeOf(v)===oldProto,label+'/prototype retained');
            var control=Object.getOwnPropertyDescriptor(v,'control');
            check(control.writable===!(fixed&&!threw&&op==='freeze'),label+'/control writable');
            check(control.configurable===!(fixed&&!threw&&(op==='seal'||op==='freeze')),label+'/control configurable');
            check(Reflect.defineProperty(v,'fresh',{value:2,configurable:true})===!fixed,label+'/ordinary extension');
            if(fixed&&!threw&&op==='seal')check(Object.isSealed(v),label+'/sealed');
            if(fixed&&!threw&&op==='freeze')check(Object.isFrozen(v),label+'/frozen');
          }}}
          // DataView and backing stores remain ordinary even when their bytes grow.
          for(var factory of [()=>new DataView(new ArrayBuffer(4,{maxByteLength:8})),()=>new DataView(new SharedArrayBuffer(4,{maxByteLength:8})),()=>new ArrayBuffer(4,{maxByteLength:8}),()=>new SharedArrayBuffer(4,{maxByteLength:8})]) {
            var v=factory();check(Object.freeze(v)===v && Object.isFrozen(v),'ordinary view/buffer integrity');
          }
          true;
        "#;
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|error|panic!("{backend:?}: {}",error.describe())),JsValue::Bool(true));
        }
    }

    #[test]
    fn proxy_integrity_preserves_trap_order_abrupt_identity_and_no_partial_variable_mutation() {
        let source = r#"
          function check(ok,label){if(!ok)throw label;}
          for(var method of [Reflect.preventExtensions,Object.preventExtensions,Object.seal,Object.freeze]) {
            var v=new Uint8Array(new ArrayBuffer(4,{maxByteLength:8}));
            var transparent=new Proxy(v,{}),threw=false,result;
            try{result=method(transparent);}catch(e){threw=true;check(Object.getPrototypeOf(e)===TypeError.prototype,'transparent TypeError');}
            check(method===Reflect.preventExtensions?(!threw&&result===false):threw,'transparent outcome');
            check(Object.isExtensible(v),'transparent keeps target extensible');
            var events=[],handler={};
            Object.defineProperty(handler,'preventExtensions',{get(){events.push('get trap');return function(t){events.push('call trap');return Reflect.preventExtensions(t);};}});
            handler.ownKeys=function(){throw 'ownKeys after failed PreventExtensions';};
            var p=new Proxy(v,handler);threw=false;
            try{result=method(p);}catch(e){threw=true;check(Object.getPrototypeOf(e)===TypeError.prototype,'forwarded TypeError');}
            check(events.join(',')==='get trap,call trap'&&Object.isExtensible(v),'trap count and atomic failure');
            check(method===Reflect.preventExtensions?(!threw&&result===false):threw,'forwarded outcome');
            var marker={};p=new Proxy(v,{preventExtensions(){throw marker;}});
            try{method(p);throw 'missing abrupt';}catch(e){check(e===marker,'abrupt identity');}
            p=new Proxy(v,{preventExtensions(){return true;}});
            try{method(p);throw 'missing invariant TypeError';}catch(e){check(Object.getPrototypeOf(e)===TypeError.prototype,'success invariant');}
            check(Object.isExtensible(v),'lying trap cannot mutate target');
            var pair=Proxy.revocable(v,{});pair.revoke();
            try{method(pair.proxy);throw 'missing revoked TypeError';}catch(e){check(Object.getPrototypeOf(e)===TypeError.prototype,'revoked');}
          }
          var b=new ArrayBuffer(4,{maxByteLength:8}),v=new Uint8Array(b,0,2);
          var p=new Proxy(v,{preventExtensions(t){b.resize(0);return Reflect.preventExtensions(t);}});
          check(Reflect.preventExtensions(p)===false && Object.isExtensible(v),'OOB does not become fixed');
          b.resize(8);check(Reflect.preventExtensions(v)===false&&Object.isExtensible(v),'regrown RAB stays variable');
          var sb=new SharedArrayBuffer(4,{maxByteLength:8}),fixed=new Uint8Array(sb,0,0),tracking=new Uint8Array(sb);
          sb.grow(8);check(Reflect.preventExtensions(fixed)===true&&!Object.isExtensible(fixed)&&Reflect.preventExtensions(tracking)===false&&Object.isExtensible(tracking),'GSAB fixed/tracking after grow');
          var empty=new Uint8Array(new SharedArrayBuffer(4,{maxByteLength:8}),0,0);
          var p=new Proxy(empty,{preventExtensions(t){return Reflect.preventExtensions(t);}});
          check(Object.freeze(p)===p && Object.isFrozen(empty),'fixed empty Proxy integrity');
          true;
        "#;
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|error|panic!("{backend:?}: {}",error.describe())),JsValue::Bool(true));
        }
    }

    #[test]
    fn genuine_foreign_integrity_functions_use_creation_error_realm_and_restore_caller() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let root=i.binding_current_realm();
            let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for (name,source) in [("foreignPrevent","Object.preventExtensions"),("foreignSeal","Object.seal"),("foreignFreeze","Object.freeze"),("foreignReflect","Reflect.preventExtensions"),("foreignType","TypeError.prototype"),("foreignVariable","new Uint8Array(new ArrayBuffer(4,{maxByteLength:8}))"),("foreignFixedEmpty","new Uint8Array(new SharedArrayBuffer(4,{maxByteLength:8}),0,0)")] {
                let value=i.binding_with_realm(child,|i|i.run_script_with_backend(source,backend)).unwrap();
                i.global_env.declare_var(name,value).unwrap();
            }
            for name in ["foreignPrevent","foreignSeal","foreignFreeze","foreignReflect"] {
                let value=i.run_script_with_backend(name,backend).unwrap();assert_eq!(i.binding_function_realm(&value).unwrap(),child);
            }
            i.binding_eval_in_realm(child,"Object.defineProperty(globalThis,'TypeError',{get(){throw 'poisoned TypeError';},configurable:true});").unwrap();
            assert_eq!(i.run_script_with_backend(r#"
              function check(ok,label){if(!ok)throw label;}
              for(var method of [foreignPrevent,foreignSeal,foreignFreeze]) {
                for(var value of [foreignVariable,new Uint8Array(new ArrayBuffer(4,{maxByteLength:8}))]) {
                  for(var invoke of [()=>method(value),()=>method.call(null,value),()=>Reflect.apply(method,null,[value]),()=>method.bind(null,value)(),()=>new Proxy(method,{})(value)]) {
                    try{invoke();throw 'missing variable TypeError';}catch(e){check(Object.getPrototypeOf(e)===foreignType,'callee error realm');}
                    check(Object.isExtensible(value),'foreign variable remains extensible');
                  }
                }
              }
              check(foreignReflect(foreignVariable)===false,'foreign Reflect false');
              var liar=new Proxy(foreignVariable,{preventExtensions(){return true;}});
              try{foreignReflect(liar);throw 'missing proxy error';}catch(e){check(Object.getPrototypeOf(e)===foreignType,'foreign Proxy invariant error');}
              var fixed=new Uint8Array(2);
              try{foreignFreeze(fixed);throw 'missing indexed freeze error';}catch(e){check(Object.getPrototypeOf(e)===foreignType&&!Object.isExtensible(fixed),'fixed nonempty freeze state/error');}
              check(foreignFreeze(foreignFixedEmpty)===foreignFixedEmpty && Object.isFrozen(foreignFixedEmpty),'foreign fixed GSAB empty freeze');
              true;
            "#,backend).unwrap_or_else(|error|panic!("{backend:?}: {}",error.describe())),JsValue::Bool(true));
            assert_eq!(i.binding_current_realm(),root);assert_eq!(i.binding_incumbent_realm(),root);
        }
    }
}
