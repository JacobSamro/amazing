#[cfg(test)]
mod typedarray_prototype_transaction_tests {
    use super::*;
    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(
                i.run_script_with_backend(source, backend)
                    .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe())),
                JsValue::Bool(true)
            );
        }
    }
    const PRELUDE: &str = r#"
      function check(ok,label){if(!ok)throw label;}
      function rejects(f,ctor){try{f();return false;}catch(e){return e instanceof ctor;}}
      var ctors=[Int8Array,Uint8Array,Uint8ClampedArray,Int16Array,Uint16Array,Int32Array,Uint32Array,BigInt64Array,BigUint64Array,Float16Array,Float32Array,Float64Array];
      function big(C){return C===BigInt64Array||C===BigUint64Array;}
      function num(C,n){return big(C)?BigInt(n):n;}
    "#;

    #[test]
    fn whole_with_matrix_uses_initial_length_and_current_index_after_both_coercions() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(2*size,{maxByteLength:5*size}),a=new C(b);a[0]=num(C,11);a[1]=num(C,22);
            var events=[],index={valueOf(){events.push('index');return 4;}},value={valueOf(){events.push('value');b.resize(5*size);return num(C,9);}};
            var result=a.with(index,value);check(events.join(',')==='index,value'&&result.length===2&&result[0]===num(C,11)&&result[1]===num(C,22),'grown valid index outside original result');
            b=new ArrayBuffer(0,{maxByteLength:size});a=new C(b);
            result=a.with(0,{valueOf(){b.resize(size);return num(C,1);}});check(result.length===0&&a.length===1,'zero grows but output remains zero');
            b=new ArrayBuffer(3*size,{maxByteLength:5*size});a=new C(b);a[0]=num(C,3);a[1]=num(C,4);a[2]=num(C,5);
            result=a.with({valueOf(){b.resize(5*size);return -1;}},num(C,8));
            check(result.length===3&&result[2]===num(C,8)&&a[2]===num(C,5),'negative index uses initial length');
            events=[];check(rejects(()=>a.with(Infinity,{valueOf(){events.push('value');return num(C,1);}}),RangeError)&&events.join(',')==='value','value before infinite invalidity');
            var marker={};events=[];try{a.with({valueOf(){throw marker;}},{valueOf(){events.push('value');return num(C,1);}});throw 'missing index abrupt';}catch(e){check(e===marker&&events.length===0,'index abrupt before value');}
          }true;
        "#
        ));
    }

    #[test]
    fn whole_with_detach_shrink_fixed_tracking_and_conversion_abrupt_matrix() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var fixed of [false,true]){for(var change of ['detach','shrink']){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(3*size,{maxByteLength:5*size}),a=fixed?new C(b,0,3):new C(b),events=[];
            check(rejects(()=>a.with(-1,{valueOf(){events.push('value');if(change==='detach')b.transfer();else b.resize(size);return num(C,1);}}),RangeError)&&events.join(',')==='value','fresh index validity after callback');
          }}}
          for(var C of [Int8Array,Uint8Array,Float16Array,Float32Array,Float64Array]){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(2*size,{maxByteLength:3*size}),a=new C(b);
            var r=a.with(0,{valueOf(){b.resize(size);return 7;}});
            check(r.length===2&&r[0]===7,'tracking shrink retains original output');
            check(C===Float16Array||C===Float32Array||C===Float64Array?Number.isNaN(r[1]):r[1]===0,'missing numeric element conversion');
          }
          true;
        "#
        ));
    }

    #[test]
    fn change_by_copy_ignores_species_keeps_intrinsic_kind_and_immutable_source() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var a=new C([num(C,3),num(C,1),num(C,2)]),b=a.buffer;a=new C(b.transferToImmutable());
            Object.defineProperty(a,'constructor',{get(){throw 'unexpected constructor';}});
            for(var pair of [['with',()=>a.with(1,num(C,8))],['toReversed',()=>a.toReversed()],['toSorted',()=>a.toSorted()]]){
              var r=pair[1]();check(Object.getPrototypeOf(r)===C.prototype&&r.length===3&&!r.buffer.immutable,'intrinsic mutable copy');
              check(a[0]===num(C,3)&&a[1]===num(C,1)&&a[2]===num(C,2),'immutable source retained');
            }
            check(a.with(1,num(C,8))[1]===num(C,8)&&a.toReversed()[0]===num(C,2)&&a.toSorted()[0]===num(C,1),'copy values');
          }true;
        "#
        ));
    }

    #[test]
    fn map_species_write_admission_precedes_callbacks_including_empty_and_mixed_content() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var length of [0,2]){
            var a=new C(length),events=[],immutable=new C(new ArrayBuffer(length*C.BYTES_PER_ELEMENT).transferToImmutable());
            Object.defineProperty(a,'constructor',{get(){events.push('constructor');return {[Symbol.species]:function(n){events.push('construct:'+n);return immutable;}};}});
            check(rejects(()=>a.map(()=>{events.push('callback');return num(C,1);}),TypeError),'immutable destination rejected');
            check(events.join(',')==='constructor,construct:'+length,'before callback immutable admission');
            var source=new C(length),other=big(C)?new Uint8Array(length):new BigInt64Array(length),calls=0;
            source.constructor={[Symbol.species]:function(){return other;}};
            check(rejects(()=>source.map(()=>{calls++;return num(C,1);}),TypeError)&&calls===0,'mixed content before callback');
          }}true;
        "#
        ));
    }

    #[test]
    fn all_species_numeric_factories_reject_detached_oob_short_and_non_native_results() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            for(var failure of ['short','detached','oob','object','proxy']){
              var size=C.BYTES_PER_ELEMENT,buffer=new ArrayBuffer(2*size,{maxByteLength:3*size}),destination=new C(buffer,0,2),a=new C(2),calls=0;
              if(failure==='short')destination=new C(1);
              if(failure==='detached')buffer.transfer();
              if(failure==='oob')buffer.resize(0);
              if(failure==='object')destination={length:2};
              if(failure==='proxy')destination=new Proxy(destination,{});
              a.constructor={[Symbol.species]:function(){return destination;}};
              check(rejects(()=>a.map(()=>{calls++;return num(C,1);}),TypeError)&&calls===0,'species validates real result before callback');
            }
          }true;
        "#
        ));
    }

    #[test]
    fn map_species_and_callback_mutations_observe_original_source_and_real_destination() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var a=new C([num(C,1),num(C,2)]),b=a.buffer,events=[];
            a.constructor={get [Symbol.species](){b.transfer();return C;}};
            var r=a.map(function(value,index,source){events.push(index);check(value===undefined&&source===a,'source changed during species');return num(C,index+3);});
            check(events.join(',')==='0,1'&&r[0]===num(C,3)&&r[1]===num(C,4),'callbacks continue after source detach');
            var size=C.BYTES_PER_ELEMENT,buffer=new ArrayBuffer(2*size,{maxByteLength:3*size}),target=new C(buffer),source=new C(2),calls=0;
            source.constructor={[Symbol.species]:function(){return target;}};
            r=source.map(function(){calls++;return {valueOf(){buffer.resize(0);return num(C,6);}};});
            check(r===target&&calls===2&&target.length===0,'destination conversion detaches bounds without data-view throw');
          }true;
        "#
        ));
    }

    #[test]
    fn whole_write_species_family_and_read_subarray_keep_distinct_admission() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var length of [0,2]){
            var immutable=new C(new ArrayBuffer(length*C.BYTES_PER_ELEMENT).transferToImmutable()),a=new C(length),events=[];
            function species(){events.push('construct');return immutable;}
            a.constructor={[Symbol.species]:species};
            check(rejects(()=>a.slice(),TypeError),'slice immutable even zero');
            check(rejects(()=>a.filter(()=>{events.push('filter');return true;}),TypeError),'filter immutable even zero');
            check(a.subarray()===immutable,'subarray read species accepts immutable');
            function factory(){return immutable;}
            check(rejects(()=>C.of.call(factory),TypeError)&&rejects(()=>C.from.call(factory,[]),TypeError),'from and of write zero immutable');
          }}true;
        "#
        ));
    }

    #[test]
    fn all_kinds_same_buffer_slice_streams_forward_for_overlapping_species_offsets() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var shared of [false,true]){
            var size=C.BYTES_PER_ELEMENT,b=shared?new SharedArrayBuffer(6*size):new ArrayBuffer(6*size),a=new C(b);
            for(var index=0;index<6;index++)a[index]=num(C,(index+1)*10);
            a.constructor={[Symbol.species]:function(){return new C(b,2*size);}};
            var r=a.slice(1,4);check(r.length===4&&r[0]===num(C,20)&&r[1]===num(C,20)&&r[2]===num(C,20)&&r[3]===num(C,60),'forward alias copy');
            check(a[1]===num(C,20)&&a[2]===num(C,20)&&a[3]===num(C,20)&&a[4]===num(C,20),'actual shared store changed');
          }}true;
        "#
        ));
    }

    #[test]
    fn slice_preserves_same_kind_nan_bytes_and_cross_kind_streams_values() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var pair of [[Float32Array,4],[Float64Array,8]]){
            var C=pair[0],size=pair[1],b=new ArrayBuffer(2*size),raw=new Uint8Array(b);
            if(size===4){raw.set([1,0,192,127,2,0,192,127]);}else{raw.set([1,0,0,0,0,0,248,127,2,0,0,0,0,0,248,127]);}
            var a=new C(b),r=a.slice();check(new Uint8Array(r.buffer).join(',')===raw.join(','),'exact NaN payload copy');
          }
          var a=new Uint8Array([250,251,252]);a.constructor={[Symbol.species]:function(n){return new Uint16Array(n);}};
          var r=a.slice(1);check(r instanceof Uint16Array&&r[0]===251&&r[1]===252,'different kind same content');
          true;
        "#
        ));
    }

    #[test]
    fn slice_coercions_species_resize_and_zero_count_preserve_their_checkpoints() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(3*size,{maxByteLength:5*size}),a=new C(b);a[0]=num(C,1);a[1]=num(C,2);a[2]=num(C,3);var events=[];
            a.constructor={get [Symbol.species](){events.push('species');b.resize(size);return C;}};
            var r=a.slice({valueOf(){events.push('start');return 0;}},{valueOf(){events.push('end');return 3;}});
            check(events.join(',')==='start,end,species'&&r.length===3&&r[0]===num(C,1)&&r[1]===num(C,0)&&r[2]===num(C,0),'clip copy after species with zero target tail');
            a=new C(1);b=a.buffer;a.constructor={get [Symbol.species](){b.transfer();return C;}};
            check(a.slice(0,0).length===0,'zero count skips source revalidation');
            a=new C(1);b=a.buffer;a.constructor={get [Symbol.species](){b.transfer();return C;}};
            check(rejects(()=>a.slice(),TypeError),'positive count revalidates source');
          }
          for(var C of ctors){for(var D of ctors){if(big(C)!==big(D))continue;
            for(var stage of ['start','end','species']){for(var first=0;first<=4;first++){
              var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(4*size,{maxByteLength:4*size}),a=new C(b);
              a.constructor={[Symbol.species]:function(n){if(stage==='species')b.resize(0);return new D(n);}};
              var start=stage==='start'?{valueOf(){b.resize(0);return first;}}:first;
              var end=stage==='end'?{valueOf(){b.resize(0);return 4;}}:4;
              var r=a.slice(start,end);
              check(Object.getPrototypeOf(r)===D.prototype&&r.length===4-first&&a.length===0&&b.byteLength===0,'clipped empty copy retains genuine species allocation');
              for(var k=0;k<r.length;k++)check(r[k]===num(D,0),'empty copy leaves every allocated destination element zero');
            }}
          }}true;
        "#
        ));
    }

    #[test]
    fn canonical_to_string_is_generic_exact_identity_and_real_get_call_fallback() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          var method=TypedArray.prototype.toString,descriptor=Object.getOwnPropertyDescriptor(TypedArray.prototype,'toString');
          check(method===Array.prototype.toString&&descriptor.value===method&&descriptor.writable&&!descriptor.enumerable&&descriptor.configurable,'canonical intrinsic identity');
          var object={get join(){check(this===object,'join getter receiver');return function(){check(this===object&&arguments.length===0,'join call receiver');return 'joined';};}};
          check(method.call(object)==='joined','generic callable join');
          object={join:3,[Symbol.toStringTag]:'custom'};check(method.call(object)==='[object custom]','generic object fallback');
          var marker={},p=new Proxy({}, {get(t,k){if(k==='join')throw marker;return Reflect.get(t,k);}});
          try{method.call(p);throw 'missing get abrupt';}catch(e){check(e===marker,'actual join abrupt');}
          check(rejects(()=>method.call(null),TypeError)&&rejects(()=>method.call(undefined),TypeError),'nullish generic rejection');
          for(var C of ctors){var a=new C([num(C,1),num(C,2)]);check(a.toString()==='1,2','typed join');a.join=null;check(a.toString()==='[object '+C.name+']','typed ordinary fallback');}
          true;
        "#
        ));
    }

    #[test]
    fn comparator_validation_precedes_to_sorted_receiver_and_copies_remain_stable() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          var method=Uint8Array.prototype.toSorted;check(rejects(()=>method.call({},1),TypeError),'comparator before receiver');
          var a=new Uint8Array([3,1,2]),events=[],detaches=0,buffer=a.buffer;var result=a.toSorted(function(x,y){events.push('compare');if(detaches===0){buffer.transfer();detaches++;}return x-y;});
          check(result.join(',')==='1,2,3'&&a.length===0&&events.length>1&&detaches===1,'sorted original list after one callback detach');
          check(rejects(()=>buffer.transfer(),TypeError),'a second transfer of the detached source still throws');
          var marker={};a=new Uint8Array([2,1]);try{a.toSorted(()=>{throw marker;});throw 'missing compare abrupt';}catch(e){check(e===marker,'compare abrupt identity');}
          true;
        "#
        ));
    }

    #[test]
    fn copy_and_default_species_use_actual_callee_realm_intrinsics_after_global_poison() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let realm = i
                .binding_create_realm(i.binding_create_object(None).unwrap())
                .unwrap();
            for (name, source) in [
                ("foreignWith", "Uint8Array.prototype.with"),
                ("foreignMap", "Uint8Array.prototype.map"),
                ("foreignProto", "Uint8Array.prototype"),
                ("foreignArrayString", "Array.prototype.toString"),
                ("foreignTypedString", "TypedArray.prototype.toString"),
            ] {
                let value = i
                    .binding_with_realm(realm, |i| i.run_script_with_backend(source, backend))
                    .unwrap();
                i.define_global(name, value);
            }
            i.binding_eval_in_realm(realm,"Uint8Array=function(){throw 'poisoned constructor';};ArrayBuffer=function(){throw 'poisoned buffer';};").unwrap();
            let source = r#"var a=new Uint8Array([1,2]);a.constructor=undefined;Object.getPrototypeOf(foreignWith.call(a,0,9))===foreignProto&&Object.getPrototypeOf(foreignMap.call(a,x=>x+1))===foreignProto&&foreignArrayString===foreignTypedString"#;
            assert_eq!(
                i.run_script_with_backend(source, backend)
                    .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe())),
                JsValue::Bool(true)
            );
        }
    }
}
