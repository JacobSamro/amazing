#[cfg(test)]
mod atomic_integer_transaction_tests {
    use super::*;

    const PRELUDE: &str = r#"
      function check(ok,label){if(!ok)throw label;}
      function rejects(f,C){try{f();return false;}catch(e){return e instanceof C;}}
      var ctors=[Int8Array,Uint8Array,Int16Array,Uint16Array,Int32Array,Uint32Array,BigInt64Array,BigUint64Array];
      var ops=['load','store','exchange','compareExchange','add','sub','and','or','xor'];
      function big(C){return C===BigInt64Array||C===BigUint64Array;}
      function num(C,n){return big(C)?BigInt(n):n;}
      function invoke(op,a,index,value,replacement){return Atomics[op](a,index,value,replacement);}
      function expected(op,old,value){switch(op){case 'load':return old;case 'store':case 'exchange':case 'compareExchange':return value;case 'add':return old+value;case 'sub':return old-value;case 'and':return old&value;case 'or':return old|value;case 'xor':return old^value;}}
    "#;

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

    #[test]
    fn all_nine_operations_all_integer_widths_real_offset_shared_and_ordinary_aliases() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var B of [ArrayBuffer,SharedArrayBuffer]){for(var op of ops){
            var size=C.BYTES_PER_ELEMENT,b=new B(8*size),a=new C(b,2*size,4),alias=new C(b,2*size,4);
            a[0]=num(C,9);a[1]=num(C,7);a[2]=num(C,11);
            var result=op==='compareExchange'?invoke(op,a,1,num(C,7),num(C,3)):invoke(op,a,1,num(C,3));
            check(result===(op==='store'?num(C,3):num(C,7)),'old or unwrapped store return');
            check(alias[1]===expected(op,num(C,7),num(C,3))&&a[0]===num(C,9)&&a[2]===num(C,11),'actual aliased word and neighbor preservation');
          }}}true;
        "#
        ));
    }

    #[test]
    fn compare_exchange_fixed_width_expected_number_bigint_wrap_and_unconditional_replacement() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var B of [ArrayBuffer,SharedArrayBuffer]){
            var a=new C(new B(2*C.BYTES_PER_ELEMENT));
            var values=big(C)?[0n,-5n,123456789n,(2n**100n)+5n,-(2n**100n)-5n]:[NaN,undefined,-0,-0.8,-5,12345,123456789,4294967301,Infinity,-Infinity];
            for(var value of values){a[0]=value;var old=a[0],seen=0;
              var replacement={valueOf(){seen++;return num(C,3);}};
              check(Object.is(Atomics.compareExchange(a,0,value,replacement),old)&&a[0]===num(C,3)&&seen===1,'compare truncated element bits and coerce replacement exactly once');
            }
            a[0]=num(C,7);var seen=[];var result=Atomics.compareExchange(a,0,{valueOf(){seen.push('expected');return num(C,1);}},{valueOf(){seen.push('replacement');return num(C,3);}});
            check(result===num(C,7)&&a[0]===num(C,7)&&seen.join(',')==='expected,replacement','mismatch still converts replacement');
          }}true;
        "#
        ));
    }

    #[test]
    fn store_returns_original_integer_or_bigint_while_memory_keeps_only_element_bits() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){var a=new C(1),control=new C(1);
            var values=big(C)?[0n,-5n,(2n**100n)+5n,-(2n**100n)-5n]:[NaN,undefined,-0,-0.8,0.8,Math.PI,-Math.PI,Infinity,-Infinity,123456789,4294967301];
            for(var value of values){var wanted=big(C)?value:(value===undefined||Number.isNaN(value)?0:Math.trunc(value));if(wanted===0)wanted=0;
              var calls=0,result=Atomics.store(a,0,{valueOf(){calls++;return value;}});control[0]=value;
              check(Object.is(result,wanted)&&Object.is(a[0],control[0])&&calls===1,'unwrapped normalized store return and one conversion');
            }
          }true;
        "#
        ));
    }

    #[test]
    fn all_rmw_read_current_memory_after_reentrant_index_and_value_conversions() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var B of [ArrayBuffer,SharedArrayBuffer]){for(var op of ops){
            var a=new C(new B(2*C.BYTES_PER_ELEMENT));a[0]=num(C,7);var seen=[];
            var index={valueOf(){seen.push('index');Atomics.store(a,0,num(C,9));return 0;}};
            var value={valueOf(){seen.push('value');Atomics.exchange(a,0,num(C,10));return num(C,3);}};
            if(op==='load'){
              check(invoke(op,a,index,value)===num(C,9)&&seen.join(',')==='index','load reads after index but never converts surplus value');
            }else if(op==='compareExchange'){
              var replacement={valueOf(){seen.push('replacement');Atomics.store(a,0,num(C,20));return num(C,3);}};
              var result=invoke(op,a,index,value,replacement);
              check(result===num(C,20)&&a[0]===num(C,20)&&seen.join(',')==='index,value,replacement','comparison reads after both reentrant conversions');
            }else{
              var result=invoke(op,a,index,value);
              check(result===(op==='store'?num(C,3):num(C,10))&&a[0]===expected(op,num(C,10),num(C,3))&&seen.join(',')==='index,value','rmw uses fresh word after nested actual transaction');
            }
          }}}true;
        "#
        ));
    }

    #[test]
    fn coercion_abrupts_and_all_operands_precede_detached_or_shrunken_revalidation() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var op of ops){
            var b=new ArrayBuffer(2*C.BYTES_PER_ELEMENT),a=new C(b),seen=[],marker={};
            var index={valueOf(){seen.push('index');b.transfer();return 0;}};
            var value={valueOf(){seen.push('value');return num(C,1);}};
            var replacement={valueOf(){seen.push('replacement');return num(C,2);}};
            check(rejects(()=>invoke(op,a,index,value,replacement),TypeError),'detached fresh validation');
            check(seen.join(',')===(op==='load'?'index':op==='compareExchange'?'index,value,replacement':'index,value'),'all required conversions finish before revalidation');
            if(op!=='load'){
              b=new ArrayBuffer(2*C.BYTES_PER_ELEMENT);a=new C(b);seen=[];
              index={valueOf(){b.transfer();return 0;}};value={valueOf(){seen.push('value');throw marker;}};
              try{invoke(op,a,index,value,replacement);throw 'missing abrupt';}catch(e){check(e===marker&&seen.join(',')==='value','operand abrupt beats later detached validation');}
            }
            if(op==='compareExchange'){
              b=new ArrayBuffer(2*C.BYTES_PER_ELEMENT);a=new C(b);seen=[];
              try{invoke(op,a,0,{valueOf(){b.transfer();seen.push('expected');return num(C,1);}},{valueOf(){seen.push('replacement');throw marker;}});throw 'missing replacement abrupt';}catch(e){check(e===marker&&seen.join(',')==='expected,replacement','replacement abrupt after expected detach');}
            }
          }}true;
        "#
        ));
    }

    #[test]
    fn original_length_witness_and_fresh_fixed_tracking_partial_word_bounds_all_methods() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var op of ops){for(var fixed of [false,true]){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(4*size,{maxByteLength:8*size}),a=fixed?new C(b,size,3):new C(b,size),seen=[];
            var index={valueOf(){seen.push('index');b.resize(size);return 1;}};
            var value={valueOf(){seen.push('value');return num(C,1);}},replacement={valueOf(){seen.push('replacement');return num(C,2);}};
            check(rejects(()=>invoke(op,a,index,value,replacement),fixed?TypeError:RangeError),'fixed view OOB versus valid empty tracking view');
            check(seen.join(',')===(op==='load'?'index':op==='compareExchange'?'index,value,replacement':'index,value'),'fresh bounds after all conversions');
          }
          for(var B of [ArrayBuffer,SharedArrayBuffer]){
            var size=C.BYTES_PER_ELEMENT,b=new B(size,{maxByteLength:3*size}),a=new C(b),seen=[];
            var index={valueOf(){if(B===ArrayBuffer)b.resize(3*size);else b.grow(3*size);seen.push('index');return 2;}};
            check(rejects(()=>invoke(op,a,index,{valueOf(){seen.push('value');return num(C,1);}},num(C,2)),RangeError)&&seen.join(',')==='index','growth cannot extend original witness admission');
          }
          if(C.BYTES_PER_ELEMENT>1){
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(3*size,{maxByteLength:4*size}),a=new C(b);
            var index={valueOf(){b.resize(size+1);return 1;}};
            check(rejects(()=>invoke(op,a,index,num(C,1),num(C,2)),RangeError),'partial current word never reads or resurrects truncated bytes');
          }
          }}true;
        "#
        ));
    }

    #[test]
    fn all_methods_native_brand_immutable_policy_and_real_slots_ignore_public_getters() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var op of ops){for(var bad of [{},[],new DataView(new ArrayBuffer(8)),new Float16Array(1),new Float32Array(1),new Float64Array(1),new Uint8ClampedArray(1),new Proxy(new Uint8Array(1),{})]){
            var calls=0,poison={valueOf(){calls++;throw 'coercion';}};
            check(rejects(()=>invoke(op,bad,poison,poison,poison),TypeError)&&calls===0,'actual brand/type before author conversions');
          }}
          for(var C of ctors){for(var op of ops){
            var b=new ArrayBuffer(C.BYTES_PER_ELEMENT),a=new C(b);a[0]=num(C,7);a=new C(b.transferToImmutable());var calls=0;
            var index={valueOf(){calls++;return 0;}};
            if(op==='load')check(invoke(op,a,index)===num(C,7)&&calls===1,'immutable load allowed');
            else check(rejects(()=>invoke(op,a,index,num(C,1),num(C,2)),TypeError)&&calls===0,'immutable write forbidden before index');
            a=new C(1);a[0]=num(C,7);var reads=0;
            for(var name of ['buffer','byteOffset','length','constructor'])Object.defineProperty(a,name,{get(){reads++;throw 'public getter';}});
            invoke(op,a,0,num(C,1),num(C,2));check(reads===0,'private native slots rather than public spoofable properties');
          }}true;
        "#
        ));
    }

    #[test]
    fn true_shared_aliases_signed_unsigned_and_growable_byte_windows_stay_coherent() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var pair of [[Int8Array,Uint8Array],[Int16Array,Uint16Array],[Int32Array,Uint32Array],[BigInt64Array,BigUint64Array]]){
            var S=pair[0],U=pair[1],size=S.BYTES_PER_ELEMENT,b=new SharedArrayBuffer(4*size,{maxByteLength:8*size}),signed=new S(b,size),unsigned=new U(b,size),bytes=new Uint8Array(b);
            signed[0]=num(S,-1);var old=unsigned[0];
            check(Atomics.compareExchange(unsigned,0,num(U,-1),num(U,0))===old&&signed[0]===num(S,0),'shared aliases compare width bits across signedness');
            signed[0]=num(S,7);var seen=[];
            var result=Atomics.add(signed,0,{valueOf(){seen.push('value');b.grow(8*size);Atomics.store(unsigned,0,num(U,10));return num(S,3);}});
            check(result===num(S,10)&&unsigned[0]===num(U,13)&&seen.join(',')==='value'&&bytes[size]===13,'one real growable block survives nested access');
          }true;
        "#
        ));
    }

    #[test]
    fn full_argument_conversion_domain_hint_order_symbol_and_surplus_argument_matrix() {
        both(&format!(
            "{PRELUDE}{}",
            r#"
          for(var C of ctors){for(var op of ops){
            var a=new C(2);a[0]=num(C,7);var seen=[];
            var index={[Symbol.toPrimitive](hint){seen.push('index:'+hint);return -0.8;}},value={[Symbol.toPrimitive](hint){seen.push('value:'+hint);return num(C,1);}},replacement={[Symbol.toPrimitive](hint){seen.push('replacement:'+hint);return num(C,2);}};
            invoke(op,a,index,value,replacement);
            check(seen.join(',')===(op==='load'?'index:number':op==='compareExchange'?'index:number,value:number,replacement:number':'index:number,value:number'),'number hint exactly once in required order');
            var calls=0,poison={valueOf(){calls++;throw 'value';}};
            check(rejects(()=>invoke(op,a,Symbol(),poison,poison),TypeError)&&calls===0,'symbol index before values');
            if(op!=='load'){
              check(rejects(()=>invoke(op,a,0,Symbol(),num(C,1)),TypeError),'symbol value rejected');
              check(rejects(()=>invoke(op,a,0,big(C)?1:1n,num(C,1)),TypeError),'wrong Number BigInt content conversion');
            }
            if(op==='compareExchange')check(rejects(()=>invoke(op,a,0,num(C,1),Symbol()),TypeError),'replacement symbol even on mismatch');
            for(var index of [Infinity,-Infinity,-1,9007199254740992])check(rejects(()=>invoke(op,a,index,poison,poison),RangeError)&&calls===0,'invalid ToIndex before value');
          }}true;
        "#
        ));
    }
}
