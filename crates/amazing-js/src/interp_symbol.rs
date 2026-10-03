// Portions ported from boa-dev/boa (core/engine/src/builtins/array/array_iterator.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Array iterator length, index advancement and exhaustion adapted to our values.
// Symbol constructor, registry, well-known identities, and prototype methods.

fn symbol_method(i: &Interpreter, name: &str, native: crate::value::NativeFn) -> JsValue {
    let value = Interpreter::native(name, native, &i.function_proto);
    if let JsValue::Object(object) = &value {
        object.borrow_mut().constructible = false;
        object.borrow_mut().delete_own("prototype");
        object.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
        object.borrow_mut().define("length", symbol_data(JsValue::num(0.0), false, true)).unwrap();
    }
    value
}

fn symbol_method_length(value: &JsValue, length: u32) {
    if let JsValue::Object(object) = value {
        object.borrow_mut().define("length", symbol_data(JsValue::num(length as f64), false, true)).unwrap();
    }
}

fn symbol_data(value: JsValue, writable: bool, configurable: bool) -> Property {
    Property { accessor: false, value, writable, enumerable: false, configurable, getter: None, setter: None }
}

impl Interpreter {
    fn install_symbols(&mut self) {
        let has_instance = symbol_method(self, "[Symbol.hasInstance]", native_function_has_instance);
        symbol_method_length(&has_instance, 1);
        self.function_proto.borrow_mut().define(
            WellKnownSymbol::HasInstance.symbol(),
            symbol_data(has_instance, false, false),
        ).unwrap();
        let constructor = Self::native_constructor("Symbol", native_symbol, &self.function_proto);
        if let JsValue::Object(object) = &constructor {
            object.borrow_mut().define("name", symbol_data(JsValue::str("Symbol"), false, true)).unwrap();
            object.borrow_mut().define("prototype", symbol_data(JsValue::Object(self.symbol_proto.clone()), false, false)).unwrap();
            object.borrow_mut().define("length", symbol_data(JsValue::num(0.0), false, true)).unwrap();
            for (name, native) in [
                ("for", native_symbol_for as crate::value::NativeFn),
                ("keyFor", native_symbol_key_for as crate::value::NativeFn),
            ] {
                let method = symbol_method(self, name, native);
                symbol_method_length(&method, 1);
                object.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
            }
            for kind in WellKnownSymbol::ALL {
                object.borrow_mut().define(kind.name(), symbol_data(JsValue::Symbol(kind.symbol()), false, false)).unwrap();
            }
        }
        {
            let mut prototype = self.symbol_proto.borrow_mut();
            prototype.define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
            for (name, native) in [
                ("toString", native_symbol_to_string as crate::value::NativeFn),
                ("valueOf", native_symbol_value_of as crate::value::NativeFn),
            ] {
                prototype.define(name, symbol_data(symbol_method(self, name, native), true, true)).unwrap();
            }
            let getter = symbol_method(self, "get description", native_symbol_description);
            prototype.define("description", Property {
                accessor: true,
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                getter: Some(getter), setter: None,
            }).unwrap();
            let to_primitive = symbol_method(self, "[Symbol.toPrimitive]", native_symbol_value_of);
            symbol_method_length(&to_primitive, 1);
            prototype.define(WellKnownSymbol::ToPrimitive.symbol(), symbol_data(to_primitive, false, true)).unwrap();
            prototype.define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str("Symbol"), false, true)).unwrap();
        }
        let array_values = symbol_method(self, "values", native_array_iterator_values);
        {
            let mut array = self.array_proto.borrow_mut();
            array.define("values", symbol_data(array_values.clone(), true, true)).unwrap();
            array.define(WellKnownSymbol::Iterator.symbol(), symbol_data(array_values, true, true)).unwrap();
            array.define("keys", symbol_data(symbol_method(self, "keys", native_array_iterator_keys), true, true)).unwrap();
            array.define("entries", symbol_data(symbol_method(self, "entries", native_array_iterator_entries), true, true)).unwrap();
        }
        self.string_proto.borrow_mut().define(
            WellKnownSymbol::Iterator.symbol(),
            symbol_data(symbol_method(self, "[Symbol.iterator]", native_string_iterator), true, true),
        ).unwrap();
        self.map_proto.borrow_mut().define(
            WellKnownSymbol::Iterator.symbol(),
            symbol_data(symbol_method(self, "[Symbol.iterator]", native_map_iterator), true, true),
        ).unwrap();
        self.set_proto.borrow_mut().define(
            WellKnownSymbol::Iterator.symbol(),
            symbol_data(symbol_method(self, "[Symbol.iterator]", native_set_iterator), true, true),
        ).unwrap();
        self.define_global("Symbol", constructor);
    }

    fn sequence_iterator(&self, target: JsValue, kind: SequenceIteratorKind) -> JsValue {
        let mut object = JsObject::ordinary();
        object.proto = self.std_sequence_iterator_proto(kind).or_else(|| Some(self.object_proto.clone()));
        object.exotic = Exotic::SequenceIterator(SequenceIteratorState { target, next_index: 0, kind });
        JsValue::object(object)
    }

    fn collection_iterator(&self, target: Obj, kind: CollectionIteratorKind) -> JsValue {
        let mut object = JsObject::ordinary();
        object.proto = Some(self.object_proto.clone());
        object.exotic = Exotic::CollectionIterator(CollectionIteratorState { target, next_index: 0, kind });
        object.define("next", symbol_data(symbol_method(self, "next", native_collection_iterator_next), true, true)).unwrap();
        object.define(WellKnownSymbol::Iterator.symbol(), symbol_data(symbol_method(self, "[Symbol.iterator]", native_iterator_self), true, true)).unwrap();
        let tag = if kind == CollectionIteratorKind::MapEntries { "Map Iterator" } else { "Set Iterator" };
        object.define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(tag), false, true)).unwrap();
        JsValue::object(object)
    }
}

fn sequence_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut object = JsObject::ordinary();
    object.proto = Some(i.object_proto.clone());
    object.set_own("value", value);
    object.set_own("done", JsValue::Bool(done));
    JsValue::object(object)
}

fn native_array_iterator_values(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let this = array_receiver(i, this)?;
    Ok(i.sequence_iterator(this, SequenceIteratorKind::ArrayValues))
}
fn native_array_iterator_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let this = array_receiver(i, this)?;
    Ok(i.sequence_iterator(this, SequenceIteratorKind::ArrayKeys))
}
fn native_array_iterator_entries(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let this = array_receiver(i, this)?;
    Ok(i.sequence_iterator(this, SequenceIteratorKind::ArrayEntries))
}
fn native_string_iterator(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    // Spec requires RequireObjectCoercible(this) before ToString; a bare
    // ToString would coerce null/undefined to "null"/"undefined" instead.
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("String iterator receiver is null or undefined"));
    }
    let value = i.to_js_string(&this)?;
    Ok(i.sequence_iterator(JsValue::Str(value), SequenceIteratorKind::String))
}
fn native_iterator_self(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    match &this {
        JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::SequenceIterator(_) | Exotic::CollectionIterator(_)) => Ok(this.clone()),
        _ => Err(Throw::type_error("incompatible iterator receiver")),
    }
}

fn native_map_iterator(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("Map iterator receiver is not a Map")); };
    if !matches!(object.borrow().exotic, Exotic::Map(_)) { return Err(Throw::type_error("Map iterator receiver is not a Map")); }
    Ok(i.collection_iterator(object, CollectionIteratorKind::MapEntries))
}
fn native_set_iterator(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("Set iterator receiver is not a Set")); };
    if !matches!(object.borrow().exotic, Exotic::Set(_)) { return Err(Throw::type_error("Set iterator receiver is not a Set")); }
    Ok(i.collection_iterator(object, CollectionIteratorKind::SetValues))
}
fn native_collection_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    let state = {
        let mut object = object.borrow_mut();
        let Exotic::CollectionIterator(state) = &mut object.exotic else { return Err(Throw::type_error("incompatible iterator receiver")); };
        let current = state.clone();
        state.next_index = state.next_index.saturating_add(1);
        current
    };
    let value = match state.kind {
        CollectionIteratorKind::MapEntries => {
            let entry = match &state.target.borrow().exotic {
                Exotic::Map(map) => map.entries.get(state.next_index).cloned(),
                _ => return Err(Throw::type_error("invalid Map iterator")),
            };
            match entry {
                Some((key, value)) => array_values(i, vec![key_value(&key), value])?,
                None => return Ok(sequence_result(i, JsValue::Undefined, true)),
            }
        }
        CollectionIteratorKind::SetValues => {
            let entry = match &state.target.borrow().exotic {
                Exotic::Set(set) => set.entries.get(state.next_index).cloned(),
                _ => return Err(Throw::type_error("invalid Set iterator")),
            };
            match entry {
                Some(key) => key_value(&key),
                None => return Ok(sequence_result(i, JsValue::Undefined, true)),
            }
        }
    };
    Ok(sequence_result(i, value, false))
}
fn native_sequence_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    if binary_iterator_state(&object).is_some() {
        return binary_typed_iterator_next(i, JsValue::Object(object), Vec::new());
    }
    let state = {
        let object = object.borrow();
        let Exotic::SequenceIterator(state) = &object.exotic else { return Err(Throw::type_error("incompatible iterator receiver")); };
        state.clone()
    };
    let index = state.next_index;
    match state.kind {
        SequenceIteratorKind::ArrayKeys | SequenceIteratorKind::ArrayValues | SequenceIteratorKind::ArrayEntries => {
            if state.target.is_undefined() { return Ok(sequence_result(i, JsValue::Undefined, true)); }
            let typed = matches!(&state.target, JsValue::Object(target) if matches!(target.borrow().exotic, Exotic::TypedArray));
            let length = if typed {
                let (view, _) = binary_typed(&state.target)?;
                if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("out-of-bounds TypedArray")); }
                binary_length(&view)
            } else {
                // Get/ToLength are observable and must finish before advancing
                // the cursor. Inherited live accessors work like own properties.
                array_search_length(i, &state.target)?
            };
            if index >= length {
                if let Exotic::SequenceIterator(current) = &mut object.borrow_mut().exotic {
                    current.target = JsValue::Undefined;
                }
                return Ok(sequence_result(i, JsValue::Undefined, true));
            }
            if let Exotic::SequenceIterator(current) = &mut object.borrow_mut().exotic {
                current.next_index = index + 1;
            }
            let value = match state.kind {
                SequenceIteratorKind::ArrayKeys => JsValue::num(index as f64),
                SequenceIteratorKind::ArrayValues => i.get_prop(&state.target, index.to_string())?,
                SequenceIteratorKind::ArrayEntries => {
                    let element = i.get_prop(&state.target, index.to_string())?;
                    array_values(i, vec![JsValue::num(index as f64), element])?
                }
                SequenceIteratorKind::String => return Err(Throw::type_error("invalid Array iterator kind")),
            };
            Ok(sequence_result(i, value, false))
        }
        SequenceIteratorKind::String => {
            let JsValue::Str(text) = state.target else { return Err(Throw::type_error("invalid String iterator")); };
            if index >= text.len() { return Ok(sequence_result(i, JsValue::Undefined, true)); }
            let first = text.units()[index];
            let paired = (0xd800..=0xdbff).contains(&first)
                && text.units().get(index + 1).is_some_and(|next| (0xdc00..=0xdfff).contains(next));
            if let Exotic::SequenceIterator(state) = &mut object.borrow_mut().exotic {
                state.next_index = index + if paired { 2 } else { 1 };
            }
            let units = if paired { text.units()[index..index + 2].to_vec() } else { vec![first] };
            Ok(sequence_result(i, JsValue::str_units(units), false))
        }
    }
}

fn symbol_receiver(value: &JsValue) -> Result<JsSymbol, Throw> {
    match value {
        JsValue::Symbol(symbol) => Ok(symbol.clone()),
        JsValue::Object(object) => match &object.borrow().exotic {
            Exotic::SymbolObject(symbol) => Ok(symbol.clone()),
            _ => Err(Throw::type_error("Symbol method called on incompatible receiver")),
        },
        _ => Err(Throw::type_error("Symbol method called on incompatible receiver")),
    }
}

fn native_symbol(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_some() {
        return Err(Throw::type_error("Symbol cannot be invoked with new"));
    }
    let description = match args.first() {
        None | Some(JsValue::Undefined) => None,
        Some(value) => Some(i.to_js_string(value)?),
    };
    Ok(JsValue::Symbol(JsSymbol::new(description)))
}

fn native_symbol_for(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let key = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let symbol = i.symbol_registry.entry(key.clone()).or_insert_with(|| JsSymbol::new(Some(key))).clone();
    Ok(JsValue::Symbol(symbol))
}

fn native_symbol_key_for(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let Some(JsValue::Symbol(symbol)) = args.first() else { return Err(Throw::type_error("Symbol.keyFor requires a Symbol")); };
    Ok(i.symbol_registry.iter().find_map(|(key, registered)| (registered == symbol).then(|| JsValue::Str(key.clone()))).unwrap_or(JsValue::Undefined))
}

fn native_symbol_to_string(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str_units(symbol_receiver(&this)?.descriptive_units()))
}

fn native_symbol_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Symbol(symbol_receiver(&this)?))
}

fn native_symbol_description(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(symbol_receiver(&this)?.description().cloned().map(JsValue::Str).unwrap_or(JsValue::Undefined))
}

fn native_function_has_instance(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.ordinary_has_instance(args.first().unwrap_or(&JsValue::Undefined), &this)
}

#[cfg(test)]
mod symbol_tests {
    use super::*;

    fn iterator_case(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut runtime = Interpreter::new();
            let result = runtime.run_script_with_backend(source, backend)
                .unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe()));
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn array_iterators_observe_inherited_live_lengths_and_box_primitive_receivers() {
        iterator_case("var reads=0; var o=Object.create({get length(){reads++;return '2.9'}}); o[0]='a';o[1]='b';o[Symbol.iterator]=Array.prototype.values; var result=Array.from(o); var boxed=Array.from(Array.prototype.values.call('ab')); result.join('')==='ab' && reads===3 && boxed.join('')==='ab' && Array.prototype.keys.call(7).next().done");
    }

    #[test]
    fn array_iterator_exhaustion_is_permanent_and_large_lengths_are_not_scan_capped() {
        iterator_case("var limit=Infinity,reads=0;var o={get length(){reads++;return limit},0:'a'};var it=Array.prototype.keys.call(o);var first=it.next();limit=0;var end=it.next();limit=3;first.value===0&&!first.done&&end.done&&it.next().done&&reads===2");
    }

    #[test]
    fn array_iterator_length_abrupt_completions_preserve_the_current_index() {
        iterator_case("var marker={},bad=true,reads=0;var length={valueOf(){if(bad){bad=false;throw marker}return 2}};var o={get length(){reads++;return length},0:'a',1:'b'};var it=Array.prototype.values.call(o),caught=false;try{it.next()}catch(e){caught=e===marker}var next=it.next();caught&&next.value==='a'&&!next.done&&reads===2");
        iterator_case("var marker={},bad=true;var o={get length(){if(bad){bad=false;throw marker}return 1},0:'a'};var it=Array.prototype.values.call(o),caught=false;try{it.next()}catch(e){caught=e===marker}caught&&it.next().value==='a'");
    }

    #[test]
    fn array_iterator_indices_advance_before_element_get_and_keys_skip_element_get() {
        iterator_case("var marker={},gets=0;var o={length:2,get 0(){gets++;throw marker},1:'b'};var it=Array.prototype.values.call(o),caught=false;try{it.next()}catch(e){caught=e===marker}var next=it.next();var key=Array.prototype.keys.call(o).next();caught&&next.value==='b'&&key.value===0&&gets===1");
        iterator_case("var o={length:2,0:'a',1:'b'};var it=Array.prototype.entries.call(o);var first=it.next().value,second=it.next().value;first!==second&&first[0]===0&&first[1]==='a'&&second[0]===1&&second[1]==='b'");
    }

    #[test]
    fn array_iterators_keep_typed_array_bounds_and_shared_next_semantics() {
        iterator_case("var a=new Uint8Array([4,5]);Object.defineProperty(a,'length',{get(){throw 7}});var it=Array.prototype.values.call(a);var first=it.next();var next=Object.getPrototypeOf([][Symbol.iterator]()).next.call(a.values());var b=new Uint8Array([9]);var detached=Array.prototype.values.call(b);b.buffer.transfer();var caught=false;try{detached.next()}catch(e){caught=e instanceof TypeError}first.value===4&&next.value===4&&caught");
    }

    #[test]
    fn symbols_have_identity_description_and_registry_keys() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script("let a=Symbol('same'), b=Symbol('same'); typeof a==='symbol' && a!==b && a.description==='same' && a.toString()==='Symbol(same)' && String(a)==='Symbol(same)' && Symbol.for('key')===Symbol.for('key') && Symbol.keyFor(Symbol.for('key'))==='key' && Symbol.keyFor(a)===undefined").unwrap(), JsValue::Bool(true));
        assert!(i.run_script("new Symbol()").is_err());
        assert!(i.run_script("'' + Symbol()").is_err());
    }

    #[test]
    fn symbol_keys_keep_identity_and_follow_string_keys() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script("let key=Symbol('key'), o={b:1,2:2,1:1,[key]:7,a:3}; let keys=Reflect.ownKeys(o); keys[0]==='1' && keys[1]==='2' && keys[2]==='b' && keys[3]==='a' && keys[4]===key && Object.keys(o).length===4 && Object.getOwnPropertySymbols(o)[0]===key && Object.assign({},o)[key]===7 && key in o && Object.getOwnPropertyDescriptor(o,key).value===7 && delete o[key] && !(key in o)").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn well_known_hooks_use_symbol_keys() {
        let mut i = Interpreter::new();
        for (name, source) in [
            ("tag", "let tagged={[Symbol.toStringTag]:'Widget'}; Object.prototype.toString.call(tagged)==='[object Widget]'"),
            ("symbol tag", "Object.prototype.toString.call(Symbol())==='[object Symbol]'"),
            ("primitive", "({[Symbol.toPrimitive]:function(hint){return hint}})+''==='default'"),
            ("instance", "1 instanceof {[Symbol.hasInstance]:function(){return true}}"),
        ] {
            let result = i.run_script(source).unwrap_or_else(|error| panic!("{name}: {}", error.describe()));
            assert_eq!(result, JsValue::Bool(true), "{name}");
        }
    }

    #[test]
    fn array_and_string_iterators_are_real_symbol_properties() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script("let a=[4,5], it=a[Symbol.iterator](); let one=it.next(), two=it.next(), end=it.next(); let s='A'[Symbol.iterator](); Array.prototype[Symbol.iterator]===Array.prototype.values && one.value===4 && two.value===5 && end.done && s.next().value==='A' && s.next().done").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn map_and_set_symbol_iterators_return_iterator_records() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script("let m=new Map(); m.set('a',3); let pair=m[Symbol.iterator]().next().value; let s=new Set(); s.add(8); pair[0]==='a' && pair[1]===3 && s[Symbol.iterator]().next().value===8").unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod instance_protocol_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn bound_targets_delegate_before_primitive_checks_with_original_receiver() {
        check(r#"
          var valid=true,has=Function.prototype[Symbol.hasInstance],seen=[],C=function(){};
          Object.defineProperty(C,Symbol.hasInstance,{get(){seen.push('get');return function(v){valid=valid && this===C;seen.push(v);return v===7?'yes':0;};},configurable:true});
          var B=C.bind({}),BB=B.bind({});
          Object.defineProperty(B,'prototype',{get(){throw 'bound prototype must not be read';}});
          valid=valid && has.call(B,7)===true && (7 instanceof B) && has.call(BB,7)===true && !(8 instanceof BB);
          valid=valid && seen.join(',')==='get,7,get,7,get,7,get,8';
          Object.defineProperty(C,Symbol.hasInstance,{value:function(v){return v===undefined;},configurable:true});
          valid=valid && has.call(B)===true;
          Object.defineProperty(C,Symbol.hasInstance,{value:null,configurable:true});
          valid=valid && has.call(B,7)===false;
          Object.defineProperty(C,Symbol.hasInstance,{value:undefined,configurable:true});
          valid=valid && has.call(B,Object.create(C.prototype))===true;valid;
        "#);
    }
    #[test]
    fn bound_and_ordinary_prototype_walks_observe_proxy_order_and_original_errors() {
        check(r#"
          var valid=true,error={},has=Function.prototype[Symbol.hasInstance],C=function(){},proto=C.prototype,log=[];
          var P=new Proxy(C,{get(t,k,r){log.push(k===Symbol.hasInstance?'has':'prototype');return Reflect.get(t,k,r);}}),B=P.bind(null);
          log=[];var mid=new Proxy({}, {getPrototypeOf(t){log.push('mid');return proto;}}),obj=new Proxy({}, {getPrototypeOf(t){log.push('obj');return mid;}});
          valid=valid && has.call(B,obj) && log.join(',')==='has,prototype,obj,mid';
          log=[];valid=valid && !has.call(B,1) && log.join(',')==='has';
          var R=Proxy.revocable(C,{}),bound=R.proxy.bind(null);R.revoke();
          for(var v of [1,{}]){try{has.call(bound,v);valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          Object.defineProperty(C,Symbol.hasInstance,{get(){throw error;},configurable:true});
          try{has.call(C.bind(null),1);valid=false;}catch(e){valid=valid && e===error;}
          Object.defineProperty(C,Symbol.hasInstance,{value:3,configurable:true});
          try{has.call(C.bind(null),1);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          var plain=function(){};plain.prototype=1;
          valid=valid && !has.call(plain,1);try{has.call(plain,{});valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          var throwing=new Proxy(function(){},{get(t,k,r){if(k==='prototype')throw error;return Reflect.get(t,k,r);}});
          try{has.call(throwing,{});valid=false;}catch(e){valid=valid && e===error;}
          valid;
        "#);
    }
    #[test]
    fn operator_and_borrowed_builtin_preserve_their_distinct_callable_contracts() {
        check(r#"
          var valid=true,has=Function.prototype[Symbol.hasInstance],d=Object.getOwnPropertyDescriptor(Function.prototype,Symbol.hasInstance);
          valid=valid && has.length===1 && has.name==='[Symbol.hasInstance]' && !Object.hasOwn(has,'prototype') && !d.writable && !d.enumerable && !d.configurable;
          for(var c of [undefined,null,1,'x',true,2n,Symbol(),{}, { [Symbol.hasInstance](){throw 'unused';} }])valid=valid && has.call(c,{})===false;
          var handler={[Symbol.hasInstance](v){valid=valid && this===handler;return 'truthy';}};
          valid=valid && (1 instanceof handler);
          for(var c of [undefined,null,1,'x',{}, {[Symbol.hasInstance]:3}]){try{1 instanceof c;valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          var C=function(){},B=C.bind(null),value=new C;
          valid=valid && has.call(B,value) && (value instanceof B) && !(Object.create(null) instanceof B);
          var arrow=()=>{};valid=valid && !has.call(arrow,1);try{has.call(arrow,{});valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          valid;
        "#);
    }
    #[test]
    fn foreign_builtin_errors_and_window_proxy_prototypes_keep_semantic_realms() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let prototype=i.binding_create_object(None).unwrap();
            let global=i.binding_create_object(Some(&prototype)).unwrap();let child=i.binding_create_realm(global).unwrap();
            let window=i.binding_create_window_proxy(child).unwrap();
            i.global_env.declare_var("windowProxy",window).unwrap();i.global_env.declare_var("windowPrototype",prototype).unwrap();
            for(name,source) in [("foreignHas","Function.prototype[Symbol.hasInstance]"),("foreignTypeError","TypeError.prototype"),
                ("foreignC","function C(){};C"),("foreignProto","C.prototype")] {
                let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var valid=true,C=function(){};C.prototype=1;
              try{foreignHas.call(C,{});valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignTypeError;}
              var value=Object.create(foreignProto);valid=valid && foreignHas.call(foreignC.bind(null),value) && (value instanceof foreignC.bind(null));
              var W=function(){};W.prototype=windowPrototype;
              valid=valid && (windowProxy instanceof W.bind(null)) && foreignHas.call(W.bind(null),windowProxy);
              var error={};Object.defineProperty(W,Symbol.hasInstance,{get(){throw error;}});
              try{foreignHas.call(W.bind(null),windowProxy);valid=false;}catch(e){valid=valid && e===error;}
              valid;
            "#,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }
}
