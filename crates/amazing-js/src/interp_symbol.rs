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
    Property { value, writable, enumerable: false, configurable, getter: None, setter: None }
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
    if !this.is_object() { return Err(Throw::type_error("Array iterator receiver is not an object")); }
    Ok(i.sequence_iterator(this, SequenceIteratorKind::ArrayValues))
}
fn native_array_iterator_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Array iterator receiver is not an object")); }
    Ok(i.sequence_iterator(this, SequenceIteratorKind::ArrayKeys))
}
fn native_array_iterator_entries(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Array iterator receiver is not an object")); }
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
    let state = {
        let mut object = object.borrow_mut();
        let Exotic::SequenceIterator(state) = &mut object.exotic else { return Err(Throw::type_error("incompatible iterator receiver")); };
        let current = state.clone();
        state.next_index = state.next_index.saturating_add(1);
        current
    };
    let index = state.next_index;
    match state.kind {
        SequenceIteratorKind::ArrayKeys | SequenceIteratorKind::ArrayValues | SequenceIteratorKind::ArrayEntries => {
            if index >= i.array_length(&state.target) { return Ok(sequence_result(i, JsValue::Undefined, true)); }
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
            if paired {
                if let Exotic::SequenceIterator(state) = &mut object.borrow_mut().exotic {
                    state.next_index = state.next_index.saturating_add(1);
                }
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
