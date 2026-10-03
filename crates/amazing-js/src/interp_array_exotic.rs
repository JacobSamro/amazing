// Portions ported from boa-dev/boa (core/engine/src/builtins/array/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Array exotic DefineOwnProperty, ArraySetLength, ArrayCreate and
// ArraySpeciesCreate adapted to Amazing's ordinary descriptor completion.

impl Interpreter {
    fn array_define_fields(&mut self, array: &Obj, key: &PropertyKey,
        fields: StdDescriptorFields) -> Result<bool, Throw> {
        if key.as_string().is_some_and(|name| name.units() == [108, 101, 110, 103, 116, 104]) {
            return self.array_set_length_fields(array, fields);
        }
        let Some(index) = array_index_key(key) else {
            return self.ordinary_define_fields(array, key, fields);
        };
        let length = array.borrow().own_prop("length").cloned().expect("Array length exists");
        let old_length = match length.value { JsValue::Num(value) => value as u32, _ => unreachable!() };
        if index >= old_length && !length.writable { return Ok(false); }
        if !self.ordinary_define_fields(array, key, fields)? { return Ok(false); }
        if index >= old_length {
            let success = self.ordinary_define_fields(array, &"length".into(), StdDescriptorFields {
                value: Some(JsValue::num(index as f64 + 1.0)), writable: Some(length.writable),
                enumerable: Some(length.enumerable), configurable: Some(length.configurable),
                ..StdDescriptorFields::default()
            })?;
            debug_assert!(success);
        }
        Ok(true)
    }

    fn array_set_length_fields(&mut self, array: &Obj,
        mut fields: StdDescriptorFields) -> Result<bool, Throw> {
        let key = PropertyKey::from("length");
        let Some(value) = fields.value.clone() else {
            return self.ordinary_define_fields(array, &key, fields);
        };
        // These are two separate observable conversions, including when the
        // first valueOf changes the array or the second conversion throws.
        let new_length = self.to_uint32(&value)?;
        let number_length = self.to_number(&value)?;
        if new_length as f64 != number_length {
            return Err(Throw::range_error("invalid array length"));
        }
        fields.value = Some(JsValue::num(new_length as f64));
        let old = array.borrow().own_prop(&key).cloned().expect("Array length exists");
        let old_length = match old.value { JsValue::Num(value) => value as u32, _ => unreachable!() };
        if new_length >= old_length {
            return self.ordinary_define_fields(array, &key, fields);
        }
        if !old.writable { return Ok(false); }
        let new_writable = fields.writable.unwrap_or(true);
        if !new_writable { fields.writable = Some(true); }
        if !self.ordinary_define_fields(array, &key, fields.clone())? { return Ok(false); }

        let mut indices: Vec<_> = array.borrow().own_property_keys().into_iter()
            .filter_map(|key| array_index_key(&key).filter(|index| *index >= new_length).map(|index| (index, key)))
            .collect();
        indices.sort_unstable_by(|left, right| right.0.cmp(&left.0));
        for (index, index_key) in indices {
            if !array.borrow_mut().delete_own(&index_key) {
                fields.value = Some(JsValue::num(index as f64 + 1.0));
                if !new_writable { fields.writable = Some(false); }
                let success = self.ordinary_define_fields(array, &key, fields)?;
                debug_assert!(success);
                return Ok(false);
            }
        }
        if !new_writable {
            let success = self.ordinary_define_fields(array, &key, StdDescriptorFields {
                writable: Some(false), ..StdDescriptorFields::default()
            })?;
            debug_assert!(success);
        }
        Ok(true)
    }
}

fn array_create(i: &Interpreter, length: usize) -> Result<JsValue, Throw> {
    if length > u32::MAX as usize { return Err(Throw::range_error("invalid array length")); }
    Ok(array_result(i, length))
}

fn array_species_create(i: &mut Interpreter, source: &JsValue, length: usize) -> Result<JsValue, Throw> {
    if !is_array_value(i, source)? { return array_create(i, length); }
    let mut constructor = i.get_prop(source, "constructor")?;
    if is_constructor(&constructor) {
        let realm = i.get_function_realm(&constructor)?;
        if realm.realm != i.host_realm && i.realms.get(&realm.realm)
            .and_then(|state| state.intrinsic_constructors.get("Array"))
            .is_some_and(|(array, _)| matches!(&constructor, JsValue::Object(object) if Rc::ptr_eq(&array.0, &object.0))) {
            constructor = JsValue::Undefined;
        }
    }
    if constructor.is_object() {
        constructor = i.get_prop(&constructor, WellKnownSymbol::Species.symbol())?;
        if constructor.is_null() { constructor = JsValue::Undefined; }
    }
    if constructor.is_undefined() { return array_create(i, length); }
    if !is_constructor(&constructor) { return Err(Throw::type_error("array species is not a constructor")); }
    i.construct(&constructor, vec![JsValue::num(length as f64)])
}

#[cfg(test)]
mod array_exotic_tests {
    use super::*;

    #[test]
    fn length_converts_twice_and_reads_array_state_after_conversion() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var valid=true, calls=0, a=[1,2,3];
                var length={valueOf(){calls++;return 1;}};
                a.length=length;
                valid=valid && calls===2 && a.length===1 && !('1' in a);
                calls=0;a=[1,2,3];
                Object.defineProperty(a,'length',{value:length});
                valid=valid && calls===2 && a.length===1;
                calls=0;a=[1,2,3];
                valid=valid && Reflect.defineProperty(a,'length',{value:length}) && calls===2;
                Object.defineProperty(a,'length',{value:-0});
                valid=valid && Object.is(a.length,0) && !Object.is(a.length,-0);
                calls=0;a=[1,2,3];
                try{Reflect.defineProperty(a,'length',{value:{valueOf(){return calls++===0?1:2;}}});valid=false;}
                catch(e){valid=valid && e instanceof RangeError && calls===2 && a.length===3;}
                var sentinel={};calls=0;
                try{a.length={valueOf(){if(++calls===2)throw sentinel;return 1;}};valid=false;}
                catch(e){valid=valid && e===sentinel && calls===2 && a.length===3;}
                calls=0;a=[1,2,3];
                var reentrant={valueOf(){if(++calls===1){a[4]=5;Object.defineProperty(a,'length',{writable:false});}return 0;}};
                valid=valid && !Reflect.defineProperty(a,'length',{value:reentrant}) && calls===2 && a.length===5 && a[4]===5;
                calls=0;a=[1,2,3];
                Object.defineProperty(a,'length',{value:{valueOf(){if(++calls===1)a[4]=5;return 1;}}});
                valid=valid && calls===2 && a.length===1 && !('4' in a);
                valid;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn length_descriptor_validation_and_partial_deletion_preserve_integrity() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var valid=true,a=[0,1,2,3];
                valid=valid && !Reflect.defineProperty(a,'length',{value:0,enumerable:true}) && a.length===4 && a[3]===3;
                valid=valid && !Reflect.defineProperty(a,'length',{value:0,configurable:true}) && a.length===4 && a[3]===3;
                Object.defineProperty(a,'2',{configurable:false});
                valid=valid && !Reflect.defineProperty(a,'length',{value:0,writable:false}) && a.length===3 && !('3' in a) && a[2]===2 && a[1]===1;
                valid=valid && Object.getOwnPropertyDescriptor(a,'length').writable===false;
                valid=valid && !Reflect.defineProperty(a,'3',{value:9}) && !('3' in a) && a.length===3;
                valid=valid && Reflect.defineProperty(a,'1',{value:8}) && a[1]===8;
                a=[0,1,2,3];Object.defineProperty(a,'2',{configurable:false});
                valid=valid && !Reflect.set(a,'length',0) && a.length===3 && !('3' in a) && Object.getOwnPropertyDescriptor(a,'length').writable;
                a=[];
                Object.defineProperty(a,'4294967294',{value:7,configurable:true});
                valid=valid && a.length===4294967295;
                Object.defineProperty(a,'4294967295',{value:8,configurable:true});
                valid=valid && a.length===4294967295;
                a.length=0;
                valid=valid && !('4294967294' in a) && a['4294967295']===8;
                a=[];a['01']=1;a['-0']=2;a['1.0']=3;
                valid=valid && a.length===0;
                Object.defineProperty(a,'5',{value:9,configurable:true});
                valid=valid && a.length===6 && a[5]===9;
                Object.preventExtensions(a);
                valid=valid && !Reflect.defineProperty(a,'8',{value:4}) && a.length===6;
                valid;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn array_brand_and_species_use_semantic_proxy_operations() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var valid=true,reads=0;
                var wrapped=new Proxy(new Proxy([],{get(){reads++;throw 1;}}),{});
                valid=valid && Array.isArray(wrapped) && reads===0 && !Array.isArray(new Proxy({},{}));
                var revoked=Proxy.revocable([],{});revoked.revoke();
                try{Array.isArray(revoked.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
                var trace=[],source=[3,4];
                function Species(n){trace.push('construct:'+n);this.count=n;}
                var constructor={get [Symbol.species](){trace.push('species');return Species;}};
                var proxy=new Proxy(source,{get(t,k,r){if(k==='constructor'){trace.push('constructor');return constructor;}return Reflect.get(t,k,r);}});
                var mapped=Array.prototype.map.call(proxy,x=>x+1);
                valid=valid && mapped instanceof Species && mapped.count===2 && mapped[0]===4 && mapped[1]===5 && trace.join(',')==='constructor,species,construct:2';
                var methods=[a=>a.map(x=>x),a=>a.filter(()=>true),a=>a.slice(),a=>a.splice(0,1),a=>a.concat([]),a=>a.flat(),a=>a.flatMap(x=>[x])];
                for(var method of methods){var a=[1,2];a.constructor={[Symbol.species]:Species};valid=valid && method(a) instanceof Species;}
                for(var value of [null,1,true,'x',Symbol()]){
                    source=[1];source.constructor=value;
                    try{source.map(x=>x);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
                }
                for(var value of [null,undefined]){source=[1];source.constructor={[Symbol.species]:value};valid=valid && Array.isArray(source.map(x=>x));}
                source=[1];source.constructor={};valid=valid && Array.isArray(source.map(x=>x));
                source=[1];source.constructor={[Symbol.species]:()=>({})};
                try{source.map(x=>x);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
                reads=0;var ordinary={0:7,length:1,get constructor(){reads++;throw 9;}};
                valid=valid && Array.prototype.map.call(ordinary,x=>x)[0]===7 && reads===0;
                valid;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn internal_array_writes_share_length_and_index_definition_rules() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let array = i.run_script_with_backend("var calls=0;var array=[1,2,3];array", backend).unwrap();
            let length = i.run_script_with_backend("({valueOf(){calls++;return 1;}})", backend).unwrap();
            i.set_prop(&array, "length", length).unwrap();
            assert_eq!(i.run_script("calls===2 && array.length===1 && !('1' in array)").unwrap(), JsValue::Bool(true));
            i.set_prop(&array, "01", JsValue::num(4.0)).unwrap();
            i.set_prop(&array, "4294967295", JsValue::num(5.0)).unwrap();
            assert_eq!(i.get_prop(&array, "length").unwrap(), JsValue::num(1.0));
            i.run_script("Object.defineProperty(array,'length',{writable:false})").unwrap();
            assert!(i.set_prop(&array, "3", JsValue::num(9.0)).is_err());
            assert_eq!(i.run_script("array.length===1 && !('3' in array)").unwrap(), JsValue::Bool(true));
        }
    }

    #[test]
    fn species_constructor_realms_and_array_create_range_are_preserved() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let parent = i.binding_current_realm();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            let foreign_array = i.binding_with_realm(child, |i| i.run_script_with_backend("[1,2]", backend)).unwrap();
            let foreign_constructor = i.binding_eval_in_realm(child, "Array").unwrap();
            i.global_env.declare_var("foreignArray", foreign_array).unwrap();
            i.global_env.declare_var("ForeignArray", foreign_constructor).unwrap();
            let result = i.run_script_with_backend(r#"
                var sentinel={},reads=0,valid=true;
                Object.defineProperty(ForeignArray,Symbol.species,{get(){reads++;throw sentinel;},configurable:true});
                var mapped=Array.prototype.map.call(foreignArray,x=>x);
                valid=valid && Object.getPrototypeOf(mapped)===Array.prototype && reads===0;
                var source=[1];source.constructor=new Proxy(ForeignArray,{});
                try{source.map(x=>x);valid=false;}catch(e){valid=valid && e===sentinel && reads===1;}
                source.constructor=ForeignArray.bind(null);
                valid=valid && Object.getPrototypeOf(source.map(x=>x))===Array.prototype;
                var r=Proxy.revocable(ForeignArray,{});r.revoke();source.constructor=r.proxy;
                try{source.map(x=>x);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
                valid;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
            assert_eq!(i.binding_current_realm(), parent);
            let maximum = array_create(&i, u32::MAX as usize).unwrap();
            assert_eq!(i.get_prop(&maximum, "length").unwrap(), JsValue::num(u32::MAX as f64));
            let oversized = u32::MAX as usize + 1;
            assert!(array_create(&i, oversized).unwrap_err().describe().starts_with("RangeError"));
            let source = i.run_script("var received=0;var source=[];source.constructor={[Symbol.species]:function(n){received=n;}};source").unwrap();
            array_species_create(&mut i, &source, oversized).unwrap();
            assert_eq!(i.run_script("received").unwrap(), JsValue::num(oversized as f64));
        }
    }
}
