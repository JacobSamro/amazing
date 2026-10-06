#[cfg(test)]
mod resizable_buffer_protocol_tests {
    use super::*;
    fn run(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let source =
                format!("function check(ok,label){{if(!ok)throw label;}}\n{source}\ntrue;");
            assert_eq!(
                i.run_script_with_backend(&source, backend)
                    .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe())),
                JsValue::Bool(true)
            );
        }
    }
    const CTORS: &str = "var ctors=[Int8Array,Uint8Array,Uint8ClampedArray,Int16Array,Uint16Array,Int32Array,Uint32Array,Float16Array,Float32Array,Float64Array,BigInt64Array,BigUint64Array];";
    #[test]
    fn all_twelve_kinds_fixed_tracking_offset_shrink_and_regrow_iterators() {
        run(&format!(
            "{CTORS}{}",
            r#"
          for(var C of ctors) for(var method of ['keys','values','entries']) for(var tracking of [false,true]) {
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(6*size,{maxByteLength:10*size});
            var a=tracking?new C(b,2*size):new C(b,2*size,3),it=a[method]();
            check(!it.next().done,'first'); b.resize(size);
            var caught;try{it.next();}catch(e){caught=e;}
            check(caught instanceof TypeError,'all out-of-bounds views throw');
            b.resize(6*size);check(!it.next().done,'failed bounds does not advance cursor');
            var j=0;while(!it.next().done)j++;
            check(j===(tracking?2:1),'remaining live length');
            b.resize(10*size);check(it.next().done,'exhaustion stays done');
          }
        "#
        ));
    }
    #[test]
    fn all_kinds_tracking_zero_offset_shrink_to_zero_ends_but_positive_offset_throws() {
        run(&format!(
            "{CTORS}{}",
            r#"
          for(var C of ctors) for(var offset of [0,1]) {
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(4*size,{maxByteLength:8*size}),a=new C(b,offset*size),it=a.values();
            check(!it.next().done,'first');b.resize(0);var caught,done;
            try{done=it.next().done;}catch(e){caught=e;}
            check(offset?caught instanceof TypeError:done===true,'zero versus invalid offset');
            b.resize(8*size);check(offset?!it.next().done:it.next().done,'cursor or completed');
          }
        "#
        ));
    }
    #[test]
    fn shared_array_iterator_identity_borrowing_and_no_own_methods() {
        run(r#"
          var next=Object.getPrototypeOf([].values()).next;
          for(var factory of [()=>new Uint8Array(2).values(),()=>Array.prototype.values.call(new Uint8Array(2)),()=>[].values()]) {
            var it=factory();check(it.next===next&&!Object.hasOwn(it,'next')&&!Object.hasOwn(it,Symbol.iterator),'shared protocol');
            check(next.call(it).done===false || it.next().done===true,'genuine borrowed next');
          }
          var a=new Uint8Array([7,8]);var it=a.entries();check(next.call(it).value.join(',')==='0,7','entries');
          var caught;try{next.call({});}catch(e){caught=e;}check(caught instanceof TypeError,'brand');
          var proxied=new Proxy(a,{});caught=undefined;try{Array.prototype.values.call(proxied).next();}catch(e){caught=e;}check(caught instanceof TypeError,'proxy not internal typed brand');
        "#);
    }
    #[test]
    fn growable_shared_buffers_live_growth_fixed_views_and_done_are_distinct() {
        run(&format!(
            "{CTORS}{}",
            r#"
          for(var C of ctors) {
            var size=C.BYTES_PER_ELEMENT,b=new SharedArrayBuffer(2*size,{maxByteLength:5*size}),fixed=new C(b,0,2),tracking=new C(b),f=fixed.keys(),t=tracking.keys();
            check(f.next().value===0&&t.next().value===0,'start');b.grow(4*size);
            check(f.next().value===1&&f.next().done,'fixed extent');
            check(t.next().value===1&&t.next().value===2&&t.next().value===3&&t.next().done,'tracking extent');
            b.grow(5*size);check(t.next().done,'done cannot revive');
          }
        "#
        ));
    }
    #[test]
    fn resize_and_grow_observable_conversion_revalidation_and_zero_filling() {
        run(r#"
          var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b);a[0]=9;a[3]=7;
          b.resize(2);b.resize(6);check(a[0]===9&&a[3]===0&&a[5]===0,'truncation zero filling');
          var n=0;var v={valueOf(){n++;b.transfer();return 1;}};var caught;
          try{b.resize(v);}catch(e){caught=e;}check(n===1&&caught instanceof TypeError&&b.detached,'detach during conversion');
          n=0;try{b.resize({valueOf(){n++;return 0;}});}catch(e){}check(n===1,'detached still coerces');
          n=0;try{new ArrayBuffer(0).resize({valueOf(){n++;return 0;}});}catch(e){}check(n===0,'fixed brand first');
          var s=new SharedArrayBuffer(2,{maxByteLength:8});caught=undefined;
          try{s.grow({valueOf(){s.grow(6);return 4;}});}catch(e){caught=e;}check(caught instanceof RangeError&&s.byteLength===6,'growth observes new length');
        "#);
    }
    #[test]
    fn buffer_constructors_coerce_then_get_new_target_prototype_once_and_keep_subclass() {
        run(r#"
          for(var C of [ArrayBuffer,SharedArrayBuffer]) {
            var log=[],proto={},target=new Proxy(function(){},{get(t,k){if(k==='prototype'){log.push('proto');return proto;}return t[k];}});
            var b=Reflect.construct(C,[{valueOf(){log.push('length');return 2;}},{get maxByteLength(){log.push('max');return 5;}}],target);
            check(log.join(',')==='length,max,proto'&&Object.getPrototypeOf(b)===proto,'construction order');
            check(b.byteLength===undefined,'custom prototype');
            class Child extends C{};var child=new Child(2,{maxByteLength:5});check(child instanceof Child&&child.byteLength===2,'subclass');
          }
        "#);
    }
    #[test]
    fn data_view_tracking_fixed_offset_coercions_and_constructor_bounds_order() {
        run(r#"
          var b=new ArrayBuffer(8,{maxByteLength:16}),tracking=new DataView(b,2),fixed=new DataView(b,2,4);
          b.resize(4);check(tracking.byteLength===2,'tracking shrink');var caught;try{fixed.byteLength;}catch(e){caught=e;}check(caught instanceof TypeError,'fixed oob');
          b.resize(12);check(tracking.byteLength===10&&fixed.byteLength===4,'regrow');
          var log=[],target=new Proxy(function(){},{get(t,k){if(k==='prototype'){log.push('proto');b.resize(1);return DataView.prototype;}return t[k];}});
          caught=undefined;try{Reflect.construct(DataView,[b,{valueOf(){log.push('offset');return 2;}},{valueOf(){log.push('length');return 3;}}],target);}catch(e){caught=e;}
          check(log.join(',')==='offset,length,proto'&&caught instanceof RangeError,'post prototype bounds');
          log=[];try{new DataView(b,2,{valueOf(){log.push('length');return 0;}});}catch(e){}check(log.length===0,'invalid offset stops before length');
          b=new ArrayBuffer(4,{maxByteLength:8});log=[];caught=undefined;
          target=new Proxy(function(){},{get(t,k){if(k==='prototype'){log.push('proto');return DataView.prototype;}return t[k];}});
          try{Reflect.construct(DataView,[b,0,{valueOf(){b.transfer();return 1;}}],target);}catch(e){caught=e;}
          check(caught instanceof TypeError&&log.join(',')==='proto','length detach then prototype then detached check');
        "#);
    }
    #[test]
    fn data_view_access_witnesses_after_offset_and_value_conversion() {
        run(r#"
          for(var shared of [false,true]) {
            var b=shared?new SharedArrayBuffer(4,{maxByteLength:8}):new ArrayBuffer(4,{maxByteLength:8}),v=new DataView(b);
            v.setUint8({valueOf(){shared?b.grow(8):b.resize(8);return 7;}},23);check(v.getUint8(7)===23,'grow during offset');
            if(!shared){var caught;try{v.setUint16(0,{valueOf(){b.resize(0);return 1;}});}catch(e){caught=e;}check(caught instanceof RangeError,'zero tracking in bounds range error');b.resize(8);check(v.getUint8(0)===0,'no write');}
          }
        "#);
    }
    #[test]
    fn tracking_subarray_omitted_end_tracks_but_explicit_end_and_fixed_parent_do_not() {
        run(&format!(
            "{CTORS}{}",
            r#"
          for(var C of ctors) {
            var z=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(4*z,{maxByteLength:8*z}),a=new C(b),fixed=new C(b,0,3);
            var tracking=a.subarray(1),explicit=a.subarray(1,3),fromFixed=fixed.subarray(1);
            b.resize(8*z);check(tracking.length===7&&explicit.length===2&&fromFixed.length===2,'derived extents');
            b.resize(2*z);check(tracking.length===1&&explicit.length===0&&fromFixed.length===0,'derived shrink');
            b.resize(4*z);a.constructor={ [Symbol.species]:function(buffer,offset){buffer.resize(2*z);return new C(buffer,offset);} };
            check(a.subarray(1).length===1,'species may shorten tracking result');
          }
        "#
        ));
    }
    #[test]
    fn slice_species_resize_and_current_bytes_zero_tail() {
        run(r#"
          var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b);a.set([1,2,3,4]);
          b.constructor={get [Symbol.species](){b.resize(2);return ArrayBuffer;}};
          var sliced=b.slice(0,4);check(new Uint8Array(sliced).join(',')==='1,2,0,0','ArrayBuffer slice partial current bytes');
          b=new ArrayBuffer(4,{maxByteLength:8});a=new Uint8Array(b);a.set([1,2,3,4]);
          a.constructor={ [Symbol.species]:function(n){b.resize(2);a[0]=9;return new Uint8Array(n);} };
          check(a.slice(0,4).join(',')==='9,2,0,0','typed slice reads after constructor');
        "#);
    }
    #[test]
    fn set_offset_then_source_gets_live_writes_overlap_and_coercion_resize() {
        run(r#"
          var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b),log=[];
          var source={get length(){log.push('length');return 2;},get 0(){log.push('0');return 7;},get 1(){log.push('1');check(a[0]===7,'prior indexed write visible');return 8;}};
          a.set(source,{valueOf(){log.push('offset');return 0;}});check(log.join(',')==='offset,length,0,1'&&a.join(',')==='7,8,0,0','set order');
          a.set(a.subarray(0,2),1);check(a.join(',')==='7,7,8,0','overlap snapshot');
          a.set({length:2,0:{valueOf(){b.resize(0);return 4;}},1:5});check(a.length===0,'invalid indexed writes skip');
        "#);
    }
    #[test]
    fn searches_callbacks_and_sort_keep_snapshot_lengths_but_read_live_indices() {
        run(r#"
          var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b);a.set([1,2,3,4]);
          check(a.indexOf(9,{valueOf(){a[1]=9;return 0;}})===1,'search current bytes');
          check(a.includes(undefined,{valueOf(){b.resize(2);return 0;}}),'includes absent tail');
          b.resize(4);a.set([4,3,2,1]);var calls=0;
          a.sort(function(x,y){calls++;b.resize(0);return x-y;});check(calls>0&&a.length===0,'sort writes use integer indexed set');
          b.resize(4);a.set([1,2,3,4]);var seen=[];a.forEach(function(x,j){seen.push(x);if(j===0)b.resize(1);});
          check(seen.length===4&&seen[1]===undefined,'callback captured length current reads');
        "#);
    }
    #[test]
    fn genuine_for_of_all_kinds_mid_loop_resize_observes_needed_offset_bounds() {
        run(&format!(
            "{CTORS}{}",
            r#"
          for(var C of ctors) for(var offset of [0,2]) for(var fixed of [false,true]) {
            var size=C.BYTES_PER_ELEMENT,b=new ArrayBuffer(6*size,{maxByteLength:8*size});
            var a=fixed?new C(b,offset*size,3):new C(b,offset*size),seen=0,caught=undefined;
            try{for(var x of a){seen++;if(seen===2)b.resize(size);}}catch(e){caught=e;}
            check(seen===2,'two genuine yields before shrink');
            check(fixed||offset?caught instanceof TypeError:caught===undefined,'offset bounds versus completed tracking');
          }
        "#
        ));
    }
    #[test]
    fn array_and_string_next_have_distinct_internal_brands_and_ignore_typed_length_getters() {
        run(r#"
          var arrayNext=Object.getPrototypeOf([].values()).next,stringNext=Object.getPrototypeOf('a'[Symbol.iterator]()).next;
          for(var pair of [[arrayNext,'a'[Symbol.iterator]()],[stringNext,[].values()]]) {
            var caught;try{pair[0].call(pair[1]);}catch(e){caught=e;}check(caught instanceof TypeError,'distinct internal brand');
          }
          var a=new Uint8Array([7,8]);Object.defineProperty(a,'length',{get(){throw 'shadow length';}});
          check(Array.prototype.values.call(a).next().value===7,'typed witness skips author length');
        "#);
    }
    #[test]
    fn binary_constructor_options_and_typed_alignment_preserve_abrupt_order() {
        run(r#"
          for(var C of [ArrayBuffer,SharedArrayBuffer]) for(var options of [null,false,1,'x',undefined,true,Symbol('x'),1n]) {
            var buffer=new C(0,options);
            check(buffer.byteLength===0 && buffer.maxByteLength===0 && (C===ArrayBuffer?!buffer.resizable:!buffer.growable),'non-object options are ignored');
          }
          for(var P of [Boolean.prototype,Number.prototype,String.prototype,Symbol.prototype,BigInt.prototype]) {
            Object.defineProperty(P,'maxByteLength',{get(){throw 'must not box primitive options';},configurable:true});
          }
          for(var C of [ArrayBuffer,SharedArrayBuffer]) for(var options of [false,1,'x',Symbol('x'),1n]) {
            check(new C(2,options).byteLength===2,'primitive prototype maxByteLength is never read');
          }
          var b=new ArrayBuffer(8,{maxByteLength:16}),calls=0,caught;
          try{new Uint16Array(b,1,{valueOf(){calls++;return 1;}});}catch(e){caught=e;}
          check(caught instanceof RangeError&&calls===0,'alignment first');
          b.transfer();caught=undefined;
          try{new Uint8Array(b,0,{valueOf(){calls++;return 1;}});}catch(e){caught=e;}
          check(caught instanceof TypeError&&calls===1,'typed length before detached check');
          var sentinel={},count=0;try{new ArrayBuffer(0,{get maxByteLength(){count++;throw sentinel;}});}catch(e){check(e===sentinel,'abrupt identity');}
          check(count===1,'maximum getter once');
        "#);
    }
    #[test]
    fn exhausted_shared_iterator_releases_its_actual_target_object() {
        let mut i = Interpreter::new();
        let target = i.binding_create_uint8_array(vec![3]).unwrap();
        let JsValue::Object(object) = &target else {
            panic!("typed target");
        };
        let weak = Rc::downgrade(&object.0);
        let values = i.get_prop(&target, "values").unwrap();
        let iterator = i.call_value(&values, target.clone(), Vec::new()).unwrap();
        drop(target);
        assert!(weak.upgrade().is_some());
        let next = i.get_prop(&iterator, "next").unwrap();
        i.call_value(&next, iterator.clone(), Vec::new()).unwrap();
        i.call_value(&next, iterator.clone(), Vec::new()).unwrap();
        assert!(
            weak.upgrade().is_none(),
            "done clears the iterated object, not only a scalar flag"
        );
        assert!(matches!(iterator, JsValue::Object(_)));
    }

    #[test]
    fn typed_clones_set_overlap_and_dataview_array_like_preserve_real_bytes() {
        run(r#"
          var b=new ArrayBuffer(16,{maxByteLength:32}),d=new DataView(b);d.setUint32(0,0x56789abc,true);d.setUint32(4,0x7ff81234,true);
          var a=new Float64Array(b,0,1),copy=new Float64Array(a),bits=new Uint8Array(copy.buffer),original=new Uint8Array(b,0,8);
          check(bits.join(',')===original.join(','),'same kind clone preserves bytes');
          var target=new Float64Array(2);target.set(a,1);check(new Uint8Array(target.buffer,8,8).join(',')===original.join(','),'same kind set preserves bytes');
          check(new Uint8Array(new DataView(b)).length===0,'DataView is ordinary array-like');
          b.resize(0);var caught;try{new Float64Array(a);}catch(e){caught=e;}check(caught instanceof TypeError,'source witness bounds');
        "#);
    }
    #[test]
    fn iterator_entry_array_uses_create_data_properties_without_inherited_setters() {
        run(r#"
          var a=new Uint8Array(1);a[0]=7;var calls=0;Object.defineProperty(Array.prototype,'0',{set(v){calls++;},configurable:true});
          var entry;try{entry=a.entries().next().value;}finally{delete Array.prototype['0'];}
          check(calls===0&&entry[0]===0&&entry[1]===7&&entry.length===2,'entry own data properties');
          var desc=Object.getOwnPropertyDescriptor(entry,'length');check(desc.writable&&!desc.enumerable&&!desc.configurable,'array length descriptor');
        "#);
    }
    #[test]
    fn join_locale_and_copywithin_coercion_consumption_preserves_units_and_live_limits() {
        run(r#"
          var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b);a.set([1,2,3,4]);
          check(a.join({toString(){b.resize(2);return '\ud800';}})==='1\ud8002\ud800\ud800','UTF16 separator captured length');
          b.resize(4);a.set([1,2,3,4]);a.copyWithin(0,1,{valueOf(){b.resize(2);return 4;}});check(a.join(',')==='2,2','live clipped copy');
          check(a.at(-0.5)===2,'negative fractional at truncates before sign');
          var saved=Number.prototype.toLocaleString;Number.prototype.toLocaleString=function(){return '\udc00';};
          var text;try{text=a.toLocaleString();}finally{Number.prototype.toLocaleString=saved;}
          check(text==='\udc00,\udc00','locale return units');
          var c=new Uint8Array(new ArrayBuffer(1,{maxByteLength:4}));
          check(c.copyWithin(0,0,{valueOf(){c.buffer.transfer();return 0;}})===c,'zero count does not revalidate');
        "#);
    }
    #[test]
    fn typed_constructor_and_from_cache_iterator_once_and_do_not_close_step_errors() {
        run(r#"
          for(var factory of [x=>new Uint8Array(x),x=>Uint8Array.from(x)]) {
            var methodGets=0,nextGets=0,calls=0,source={get [Symbol.iterator](){methodGets++;return function(){return {get next(){nextGets++;return function(){return calls++?{done:true}:{value:7,done:false};};}};};}};
            check(factory(source)[0]===7&&methodGets===1&&nextGets===1&&calls===2,'cached method and next');
            var sentinel={},closes=0;source={ [Symbol.iterator](){return {next(){throw sentinel;},get return(){closes++;return function(){};}};}};
            var caught;try{factory(source);}catch(e){caught=e;}check(caught===sentinel&&closes===0,'IteratorToList step abrupt');
          }
        "#);
    }
    #[test]
    fn array_like_constructor_and_from_interleave_get_mapping_and_index_conversion() {
        run(r#"
          var log=[],source={length:2,get 0(){log.push('get0');return {valueOf(){log.push('convert0');return 7;}};},get 1(){log.push('get1');return 8;}};
          check(new Uint8Array(source).join(',')==='7,8'&&log.join(',')==='get0,convert0,get1','constructor conversion order');
          log=[];source={length:2,get 0(){log.push('get0');return 7;},get 1(){log.push('get1');return 8;}};
          check(Uint8Array.from(source,function(v,j){log.push('map'+j);return v;}).join(',')==='7,8'&&log.join(',')==='get0,map0,get1,map1','array like from order');
          var b=new ArrayBuffer(2,{maxByteLength:4}),a=new Uint8Array(b),calls=0;
          function C(){return a;}Uint8Array.from.call(C,[1,2],function(v){b.resize(0);return {valueOf(){calls++;return v;}};});
          check(calls===2&&a.length===0,'invalid target indices still convert mapper results');
        "#);
    }
    #[test]
    fn immutable_slice_shrink_checks_original_end_after_detachment_and_ignores_species() {
        run(r#"
          for(var count of [0,3,5]) {
            var b=new ArrayBuffer(10,{maxByteLength:10}),a=new Uint8Array(b);a.set([1,2,3,4,5,6,7,8,9,10]);
            Object.defineProperty(b,'constructor',{get(){throw 'immutable slice has no species';}});
            var start={valueOf(){b.resize(9);return 3;}},end={valueOf(){b.resize(8);return 3+count;}};
            var result=b.sliceToImmutable(start,end);
            check(result.immutable && result.byteLength===count,'immutable resolved copy size');
            if(count)check(new Uint8Array(result)[0]===4,'original resolved start');
            b.resize(10);var caught=undefined;
            end={valueOf(){b.resize(2);return 3+count;}};
            try{b.sliceToImmutable(start,end);}catch(e){caught=e;}
            check(caught instanceof RangeError,'current length must cover original resolved end even for empty copy');
            b.resize(10);caught=undefined;
            end={valueOf(){b.resize(2);b.transfer();return 3+count;}};
            try{b.sliceToImmutable(start,end);}catch(e){caught=e;}
            check(caught instanceof TypeError,'detachment precedes final immutable bounds');
          }
        "#);
    }
    #[test]
    fn typed_factory_constructor_admission_precedes_source_protocol() {
        run(r#"
          var calls=0,source={get [Symbol.iterator](){calls++;return function(){};}};
          for(var C of [{},()=>{},(()=>{}).bind(null),new Proxy(()=>{},{}),async function(){},function*(){},async function*(){}]) {var caught;try{Uint8Array.from.call(C,source);}catch(e){caught=e;}check(caught instanceof TypeError&&calls===0,'constructor admission before source');}
          var caught;try{Uint8Array.of.call(()=>{},1);}catch(e){caught=e;}check(caught instanceof TypeError,'of constructor admission');
        "#);
    }
}
