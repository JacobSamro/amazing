mod array_join_utf16_tests {
    use super::*;

    fn matrix(source: &str) {
        for ast in [true, false] {
            let mut i = Interpreter::new();
            i.force_ast_callables = ast;
            i.run_script(&format!("var tested=function(){{{source}}};")).unwrap();
            let JsValue::Object(function) = i.run_script("tested").unwrap() else { panic!("function") };
            assert_eq!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !ast);
            assert_eq!(i.run_script("tested()").unwrap_or_else(|e| panic!("AST={ast}: {}", e.describe())), JsValue::Bool(true), "AST={ast}");
            assert!(ARRAY_JOIN_STACK.with(|stack| stack.borrow().is_empty()), "join identities restored");
        }
    }

    #[test]
    fn native_join_result_matches_independent_original_unit_vectors_without_source_conversion() {
        let mut i = Interpreter::new();
        let prototype = JsValue::Object(i.array_proto.clone());
        let join = i.get_prop(&prototype, "join").unwrap();
        let strings = [vec![], vec![0xd800], vec![0xd801], vec![0xdc00], vec![0xdfff],
            vec![0xd800, 0xdc00], vec![0], vec![0xfffd], vec![0xd800, 0x78, 0xdc00]];
        for first in &strings { for second in &strings { for separator in &strings {
            let array = native_array(&mut i, JsValue::Undefined,
                vec![JsValue::str_units(first.clone()), JsValue::str_units(second.clone())]).unwrap();
            let result = i.call_value(&join, array, vec![JsValue::str_units(separator.clone())]).unwrap();
            let JsValue::Str(result) = result else { panic!("native join String") };
            let mut expected = first.clone();
            expected.extend_from_slice(separator);
            expected.extend_from_slice(second);
            assert_eq!(result.units(), expected, "original native unit vectors");
        } } }
    }

    #[test]
    fn join_retains_all_utf16_units_in_elements_separators_and_adjacent_boundaries() {
        matrix(r#"
            var strings=['','\ud800','\ud801','\udfff','\udc00','\ud800\udc00','\ud800x\udc00','\u0000','\ufffd','é'];
            for(var a of strings)for(var b of strings)for(var separator of strings){
                if([a,b].join(separator)!==a+separator+b)return false;
                if(Array.prototype.join.call({0:a,1:b,length:2},separator)!==a+separator+b)return false;
            }
            return ['\ud800','\udc00'].join('')==='\ud800\udc00' && ['\ud800'].toString()==='\ud800' &&
                Array.prototype.join.call('\ud800x\udc00','|')==='\ud800|x|\udc00';
        "#);
    }

    #[test]
    fn join_coercions_are_once_in_length_separator_index_order_with_string_hint() {
        matrix(r#"
            var log=[],target={length:{valueOf(){log.push('length number');return 2}},
                0:{[Symbol.toPrimitive](hint){log.push('element '+hint);return '\ud800'}},1:'\udfff'};
            var p=new Proxy(target,{get(t,k,r){log.push('get '+k);return Reflect.get(t,k,r)},has(){throw 'unexpected HasProperty'}});
            var separator={[Symbol.toPrimitive](hint){log.push('separator '+hint);target.length=0;return '\udc00'}};
            var result=Array.prototype.join.call(p,separator);
            return result==='\ud800\udc00\udfff' && log.join('|')==='get length|length number|separator string|get 0|element string|get 1';
        "#);
    }

    #[test]
    fn join_reads_holes_inherited_properties_and_mutations_without_has_queries() {
        matrix(r#"
            var proto={1:'\ud800'},target=Object.create(proto),log=[];
            target.length=4;target[0]=null;target[3]=undefined;
            var p=new Proxy(target,{get(t,k,r){log.push(k);if(k==='1'){t[2]='\udc00';t.length=0}return Reflect.get(t,k,r)},has(){throw 'unexpected HasProperty'}});
            return Array.prototype.join.call(p,'\udfff')==='\udfff\ud800\udfff\udc00\udfff' && log.join(',')==='length,0,1,2,3';
        "#);
    }

    #[test]
    fn join_length_and_separator_abrupts_preserve_identity_and_do_not_read_indices() {
        matrix(r#"
            var marker={},gets=0,separator={toString(){throw marker}},caught=0;
            for(var length of [0,-1,NaN,-Infinity]){
                var p=new Proxy({length},{get(t,k,r){if(k!=='length')gets++;return Reflect.get(t,k,r)}});
                try{Array.prototype.join.call(p,separator)}catch(e){if(e===marker)caught++}
                try{Array.prototype.join.call(p,Symbol())}catch(e){if(e instanceof TypeError)caught++}
                if(Array.prototype.join.call(p)!=='')return false;
            }
            for(var receiver of [null,undefined]){
                try{Array.prototype.join.call(receiver,separator)}catch(e){if(e instanceof TypeError)caught++}
            }
            for(var length of [Symbol(),1n]){
                try{Array.prototype.join.call({length},separator)}catch(e){if(e instanceof TypeError)caught++}
            }
            var p={get length(){throw marker}};
            try{Array.prototype.join.call(p,separator)}catch(e){if(e===marker)caught++}
            return caught===13 && gets===0;
        "#);
    }

    #[test]
    fn join_snapshot_length_uses_tolength_and_avoids_eager_length_allocations() {
        matrix(r#"
            for(var length of [0,-0,-2,NaN,-Infinity])if(Array.prototype.join.call({0:'unused',length})!=='')return false;
            if(Array.prototype.join.call({0:'\ud800',1:'\udfff',2:'unused',length:2.9},'')!=='\ud800\udfff')return false;
            for(var length of [Infinity,9007199254740992,9007199254740991]){
                var marker={},log=[],p=new Proxy({length},{get(t,k,r){log.push(k);if(k==='0')throw marker;return Reflect.get(t,k,r)}});
                try{Array.prototype.join.call(p);return false}catch(e){if(e!==marker)return false}
                if(log.join(',')!=='length,0')return false;
            }
            return true;
        "#);
    }

    #[test]
    fn join_element_get_and_conversion_errors_restore_cycle_guard_for_later_calls() {
        matrix(r#"
            var marker={},a=['\ud800','\udfff'],caught=0;
            Object.defineProperty(a,'1',{get(){throw marker},configurable:true});
            try{a.join('')}catch(e){if(e===marker)caught++}
            Object.defineProperty(a,'1',{value:'\udfff',writable:true,configurable:true});
            if(a.join('')!=='\ud800\udfff')return false;
            a[1]={[Symbol.toPrimitive](hint){if(hint!=='string')throw 'hint';throw marker}};
            try{a.join('')}catch(e){if(e===marker)caught++}
            a[1]=Symbol();try{a.join('')}catch(e){if(e instanceof TypeError)caught++}
            a[1]='\udfff';
            return caught===3 && a.join('')==='\ud800\udfff';
        "#);
    }

    #[test]
    fn join_identity_cycles_and_authored_conversion_keep_existing_empty_contribution_policy() {
        matrix(r#"
            var a=['\ud800'];a.push(a,'\udfff');
            if(a.join('|')!=='\ud800||\udfff')return false;
            var b=['\udc00'],c=['\ud800',b];b.push(c);
            if(c.join('|')!=='\ud800|\udc00,')return false;
            var calls=0,d=[];d.push(d);d.toString=function(){calls++;return '\ud800'};
            if(Array.prototype.join.call(d)!=='\ud800'||calls!==1)return false;
            var marker={},bad={toString(){throw marker}};
            try{a.join(bad);return false}catch(e){if(e!==marker)return false}
            return a.join('|')==='\ud800||\udfff' && b.join(',')==='\udc00,\ud800,';
        "#);
    }

    #[test]
    fn array_tostring_observes_join_lookup_callable_proxy_and_generic_fallback() {
        matrix(r#"
            var calls=0,o={get join(){calls++;return new Proxy(function(){'use strict';return this===o?'\ud800':'wrong'}, {})}};
            if(Array.prototype.toString.call(o)!=='\ud800'||calls!==1)return false;
            var marker={},bad={get join(){throw marker}};
            try{Array.prototype.toString.call(bad);return false}catch(e){if(e!==marker)return false}
            var tagged={join:3,[Symbol.toStringTag]:'\udfff'};
            return Array.prototype.toString.call(tagged)==='[object \udfff]' && [1,2n,true,null,undefined].join()==='1,2,true,,';
        "#);
    }
}
