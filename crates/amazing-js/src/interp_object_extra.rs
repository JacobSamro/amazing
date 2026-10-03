// Portions ported from boa-dev/boa (core/engine/src/builtins/object/mod.rs,
// core/engine/src/object/operations.rs) @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// The remaining Object, Function and Boolean built-ins.

fn object_argument(args: &[JsValue]) -> JsValue {
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

#[cfg(test)]
mod object_semantic_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn static_object_boundaries_box_or_preserve_primitives_as_required() {
        check(r#"
            var valid=true, values=[undefined,null,true,false,0,-0,NaN,'x',2n,Symbol('x')];
            for(var v of values){valid=valid && !Object.isExtensible(v) && Object.is(Object.preventExtensions(v),v);}
            valid=valid && Object.preventExtensions.call({})===undefined && Object.isExtensible()===false;
            var primitives=[true,3,'x',2n,Symbol('x')], prototypes=[Boolean.prototype,Number.prototype,String.prototype,BigInt.prototype,Symbol.prototype];
            for(var k=0;k<primitives.length;k++)valid=valid && Object.getPrototypeOf(primitives[k])===prototypes[k];
            for(var v of [undefined,null])for(var fn of [Object.getPrototypeOf,Object.keys,Object.values,Object.entries,Object.getOwnPropertyNames,Object.getOwnPropertySymbols,Object.getOwnPropertyDescriptors]){
                try{fn(v);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            }
            valid=valid && Object.keys('\ud83d\ude00').join(',')==='0,1' && Object.keys(3).length===0 && Object.keys(Symbol()).length===0;
            var boxed=Object('ab');valid=valid && Object.keys(boxed).join(',')==='0,1' && Object.getOwnPropertyNames(boxed).join(',')==='0,1,length';
            var ordinary={valueOf(){throw 7;},toString(){throw 8;}};
            valid=valid && Object.isExtensible(ordinary) && Object.preventExtensions(ordinary)===ordinary && !Object.isExtensible(ordinary);
            valid;
        "#);
    }

    #[test]
    fn static_object_proxy_trap_order_and_abrupt_identity_are_semantic() {
        check(r#"
            var log=[], valid=true, target={}, sentinel={};
            var p=new Proxy(target,{getPrototypeOf(t){log.push('proto');return null;},isExtensible(t){log.push('extensible');return Reflect.isExtensible(t);},
                preventExtensions(t){log.push('prevent');return Reflect.preventExtensions(t);}});
            valid=valid && Object.getPrototypeOf(p)===null && Object.isExtensible(p) && Object.preventExtensions(p)===p && !Object.isExtensible(p);
            valid=valid && log.join(',')==='proto,extensible,prevent,extensible';
            for(var name of ['getPrototypeOf','isExtensible','preventExtensions']){
                var handler={};Object.defineProperty(handler,name,{get(){throw sentinel;}});p=new Proxy({},handler);
                try{Object[name](p);valid=false;}catch(e){valid=valid && e===sentinel;}
            }
            p=new Proxy({},{preventExtensions(){return false;}});try{Object.preventExtensions(p);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            p=new Proxy({},{preventExtensions(){return true;}});try{Object.preventExtensions(p);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            var s=Symbol(),reads=0;log=[];
            p=new Proxy({a:1,b:2,[s]:3},{ownKeys(t){log.push('keys');return ['b',s,'a'];},getOwnPropertyDescriptor(t,k){log.push(k);return Reflect.getOwnPropertyDescriptor(t,k);},get(){reads++;throw sentinel;}});
            valid=valid && Object.keys(p).join(',')==='b,a' && log.join(',')==='keys,b,a' && reads===0;
            var r=Proxy.revocable({},{});r.revoke();
            for(var fn of [Object.getPrototypeOf,Object.isExtensible,Object.preventExtensions,Object.keys]){try{fn(r.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
            valid;
        "#);
    }

    #[test]
    fn static_object_boxing_uses_callee_realm_and_preserves_window_policy() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for (name, expression) in [("foreignGetProto", "Object.getPrototypeOf"), ("foreignKeys", "Object.keys"),
                ("foreignStringProto", "String.prototype"), ("foreignNumberProto", "Number.prototype"), ("foreignTypeErrorProto", "TypeError.prototype")] {
                let value = i.binding_eval_in_realm(child, expression).unwrap();
                i.global_env.declare_var(name, value).unwrap();
            }
            let window = i.binding_create_window_proxy(child).unwrap();
            i.global_env.declare_var("windowProxy", window).unwrap();
            let result = i.run_script_with_backend(r#"
                var valid=foreignGetProto('x')===foreignStringProto && foreignGetProto(3)===foreignNumberProto;
                try{foreignKeys(null);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignTypeErrorProto;}
                try{Object.preventExtensions(windowProxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
                valid;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn prototype_accessors_and_membership_use_semantic_proxy_methods() {
        check(r#"
            var getter=function(){return 1;}, setter=function(v){}, calls='';
            var ancestor={};Object.defineProperty(ancestor,'x',{get:getter,set:setter,configurable:true});
            var p=new Proxy({}, {
                getOwnPropertyDescriptor(t,k){calls+='own;';return undefined;},
                getPrototypeOf(t){calls+='proto;';return ancestor;}
            });
            var ok=Object.prototype.__lookupGetter__.call(p,'x')===getter && calls==='own;proto;';
            calls='';ok=ok && Object.prototype.__lookupSetter__.call(p,'x')===setter && calls==='own;proto;';
            calls='';ok=ok && ancestor.isPrototypeOf(p) && calls==='proto;';
            ok=ok && Object.prototype.isPrototypeOf.call(null,3)===false &&
                Object.prototype.isPrototypeOf.call(undefined,'x')===false;
            var threw=false;try{Object.prototype.__lookupGetter__.call(new Proxy({}, {getOwnPropertyDescriptor(){throw 'own';}}),'x');}catch(e){threw=e==='own';}
            ok && threw;
        "#);
    }
    #[test]
    fn own_property_predicates_coerce_keys_before_receivers() {
        check(r#"
            var own=Object.prototype.hasOwnProperty, enumerable=Object.prototype.propertyIsEnumerable;
            var ok=own.call('ab','0') && enumerable.call('ab','0') && own.call('ab','length') &&
                !enumerable.call('ab','length') && !own.call(7,'x') && !enumerable.call(true,'x');
            var key={toString(){throw 'key';}};
            for(var method of [own,enumerable]) {
                var threw=false;try{method.call(null,key);}catch(e){threw=e==='key';}ok=ok&&threw;
            }
            var calls=0,p=new Proxy({x:1},{getOwnPropertyDescriptor(t,k){calls++;return Reflect.getOwnPropertyDescriptor(t,k);}});
            ok && own.call(p,'x') && enumerable.call(p,'x') && calls===2;
        "#);
    }
    #[test]
    fn builtin_tags_observe_is_array_before_tag_get() {
        check(r#"
            var tag=Object.prototype.toString;
            var revoked=Proxy.revocable([],{get(t,k){revoked.revoke();return undefined;}}), ok=false;
            var p=revoked.proxy;
            ok=tag.call(p)==='[object Array]';
            var threw=false;try{tag.call(revoked.proxy);}catch(e){threw=e instanceof TypeError;}
            ok=ok&&threw;
            for(var value of [new Map(),new Set(),Promise.resolve(),Object(1n),Object(Symbol())]) {
                Object.defineProperty(value,Symbol.toStringTag,{value:42,configurable:true});
                ok=ok&&tag.call(value)==='[object Object]';
            }
            ok && tag.call(new Proxy([],{}))==='[object Array]' && tag.call(Object(3))==='[object Number]' &&
                tag.call(Object(true))==='[object Boolean]' && tag.call(Object('x'))==='[object String]';
        "#);
    }
    #[test]
    fn seal_and_freeze_project_minimal_descriptors_in_order() {
        check(r#"
            var log='', target={x:1}, p=new Proxy(target,{
                preventExtensions(t){log+='prevent;';return Reflect.preventExtensions(t);},
                ownKeys(t){log+='keys;';return Reflect.ownKeys(t);},
                getOwnPropertyDescriptor(t,k){log+='own;';return Reflect.getOwnPropertyDescriptor(t,k);},
                defineProperty(t,k,d){log+='define:'+Object.keys(d).join(',')+';';return Reflect.defineProperty(t,k,d);}
            });
            Object.seal(p);
            var ok=log==='prevent;keys;define:configurable;' && target.x===1;
            log='';Object.freeze(p);
            ok=ok && log==='prevent;keys;own;define:writable,configurable;' && Object.isFrozen(target);
            var refused=new Proxy({x:1},{defineProperty(){return false;}}), threw=false;
            try{Object.seal(refused);}catch(e){threw=e instanceof TypeError;}
            var accessor={};Object.defineProperty(accessor,'x',{get(){return 1;},configurable:true});
            log='';Object.freeze(new Proxy(accessor,{defineProperty(t,k,d){log=Object.keys(d).join(',');return Reflect.defineProperty(t,k,d);}}));
            ok && threw && log==='configurable' && accessor.x===1;
        "#);
    }
}

#[cfg(test)]
mod boxed_primitive_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn wrapper_slots_are_private_and_cannot_be_forged() {
        check(r#"
            function rejects(method, value) { try { method.call(value); } catch(e) { return e instanceof TypeError; } return false; }
            var fake = {'[[StringData]]':'abc','[[NumberData]]':7,'[[BooleanData]]':true,'[[BigIntData]]':1n};
            var n=Object(7), b=Object(true), s=Object('abc'), big=Object(1n);
            var ok = Reflect.ownKeys(n).length===0 && Reflect.ownKeys(b).length===0 && Reflect.ownKeys(big).length===0 &&
                Reflect.ownKeys(s).join(',')==='0,1,2,length' &&
                Object.getOwnPropertyNames(new Proxy(new Proxy(s,{}),{})).join(',')==='0,1,2,length';
            n['[[NumberData]]']=8; b['[[BooleanData]]']=false; s['[[StringData]]']='changed'; big['[[BigIntData]]']=2n;
            var d=Object.getOwnPropertyDescriptor(s,'0'), len=Object.getOwnPropertyDescriptor(s,'length');
            ok && n.valueOf()===7 && b.valueOf()===true && s.valueOf()==='abc' && big.valueOf()===1n &&
                d.value==='a' && !d.writable && d.enumerable && !d.configurable && len.value===3 && !len.writable &&
                !len.enumerable && !len.configurable &&
                rejects(Number.prototype.valueOf,fake) && rejects(Boolean.prototype.valueOf,fake) &&
                rejects(String.prototype.valueOf,fake) && rejects(BigInt.prototype.valueOf,fake) &&
                rejects(BigInt.prototype.valueOf,BigInt.prototype) &&
                Object.prototype.toString.call(fake)==='[object Object]' &&
                Number.prototype.valueOf()===0 && Boolean.prototype.valueOf()===false && String.prototype.valueOf()==='';
        "#);
    }

    #[test]
    fn coercion_observes_methods_hints_and_json_wrapper_steps() {
        check(r#"
            var n=Object(1), s=Object('old'), b=Object(true), log='';
            n.valueOf=function(){log+='number;';return 9;};
            s.toString=function(){log+='string;';return 'new';};
            b.valueOf=function(){throw 1;};
            var ok=+n===9 && String(s)==='new' && String.prototype.charAt.call(s,0)==='n';
            log=''; ok=ok && JSON.stringify([n,s,b])==='[9,"new",true]' && log==='number;string;';
            var key=Object('x'); key.toString=function(){return 'y';};
            var gap=Object(2); gap.valueOf=function(){return 3;};
            ok=ok && JSON.stringify({x:1,y:2},[key],gap)==='{\n   "y": 2\n}';
            var hint=''; n[Symbol.toPrimitive]=function(h){hint=h;return 4;};
            ok=ok && +n===4 && hint==='number';
            ok=ok && String(n)==='4' && hint==='string';
            delete n[Symbol.toPrimitive]; n.valueOf=function(){log+='v;';return {};}; n.toString=function(){log+='s;';return '6';};
            log=''; ok=ok && +n===6 && log==='v;s;';
            n.valueOf=function(){throw 'abrupt';};
            var abrupt=false;try{JSON.stringify(n);}catch(e){abrupt=e==='abrupt';}
            ok && abrupt;
        "#);
    }

    #[test]
    fn cloned_wrappers_retain_private_data_and_string_descriptors() {
        check(r#"
            var values=[Object(true),Object(7),Object('abc'),Object(1n)];
            var copied=structuredClone(values), ok=true;
            for(var j=0;j<values.length;j++) {
                ok=ok && copied[j]!==values[j] && copied[j].valueOf()===values[j].valueOf() &&
                    Object.getPrototypeOf(copied[j])===Object.getPrototypeOf(values[j]);
            }
            var d=Object.getOwnPropertyDescriptor(copied[2],'0');
            var fake={'[[StringData]]':'user'};var clone=structuredClone(fake);
            ok && Reflect.ownKeys(copied[0]).length===0 && Reflect.ownKeys(copied[1]).length===0 &&
                Reflect.ownKeys(copied[3]).length===0 && Reflect.ownKeys(copied[2]).join(',')==='0,1,2,length' &&
                d.value==='a' && !d.writable && !d.configurable && d.enumerable && copied[2].length===3 &&
                Object.getPrototypeOf(clone)===Object.prototype && clone['[[StringData]]']==='user' &&
                Object.prototype.toString.call(clone)==='[object Object]';
        "#);
    }
}

fn object_required(args: &[JsValue]) -> Result<Obj, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(object.clone()),
        _ => Err(Throw::type_error("Object operation requires an object")),
    }
}

fn native_object_entries(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = object_argument(&args);
    if object.is_null() || object.is_undefined() {
        return Err(Throw::type_error("Object.entries requires an object"));
    }
    let mut entries = Vec::new();
    for key in i.own_property_keys(&object)? {
        let PropertyKey::String(key) = key else { continue; };
        if !i.object_own_enumerable(&object, &PropertyKey::String(key.clone()))? { continue; }
        let value = i.get_prop(&object, &key)?;
        entries.push(array_values(i, vec![JsValue::Str(key), value])?);
    }
    array_values(i, entries)
}

// These Object operations require an iterable, unlike Iterator.from which
// also accepts an iterator directly. Capture `next` once for the whole walk.
fn object_iterator(i: &mut Interpreter, input: JsValue) -> Result<StdIteratorRecord, Throw> {
    let method = i.get_prop(&input, WellKnownSymbol::Iterator.symbol())?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("value is not iterable"));
    }
    let iterator = i.call_value(&method, input, Vec::new())?;
    i.std_iterator_direct(iterator)
}

fn native_object_from_entries(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let iterator = object_iterator(i, object_argument(&args))?;
    let object = i.object_value();
    let JsValue::Object(result) = &object else { unreachable!() };
    // Iterator protocol failures propagate without closing. Failures while
    // processing an entry close the iterator, preserving the original throw.
    while let Some(entry) = std_iterator_step(i, &iterator)? {
        let added = (|| {
            if !entry.is_object() {
                return Err(Throw::type_error("entry is not an object"));
            }
            let key_value = i.get_prop(&entry, "0")?;
            let value = i.get_prop(&entry, "1")?;
            let key = i.to_property_key(&key_value)?;
            result.borrow_mut().define(key, Property::data(value)).unwrap();
            Ok(())
        })();
        if let Err(error) = added {
            let _ = std_iterator_close(i, &iterator);
            return Err(error);
        }
    }
    Ok(object)
}

fn native_object_is(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(std_same_value(
        args.first().unwrap_or(&JsValue::Undefined),
        args.get(1).unwrap_or(&JsValue::Undefined),
    )))
}

fn native_object_descriptors(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = object_argument(&args);
    if target.is_null() || target.is_undefined() {
        return Err(Throw::type_error(
            "Object.getOwnPropertyDescriptors requires an object",
        ));
    }
    let result = i.object_value();
    for key in i.own_property_keys(&target)? {
        let descriptor = native_object_get_own_property_descriptor(
            i,
            JsValue::Undefined,
            vec![
                target.clone(),
                match &key {
                    PropertyKey::String(name) => JsValue::Str(name.clone()),
                    PropertyKey::Symbol(symbol) => JsValue::Symbol(symbol.clone()),
                },
            ],
        )?;
        if !descriptor.is_undefined() {
            let JsValue::Object(result) = &result else { unreachable!() };
            result.borrow_mut().define(key, Property::data(descriptor)).unwrap();
        }
    }
    Ok(result)
}

fn native_object_define_properties(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = object_required(&args)?;
    let descriptors = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if descriptors.is_null() || descriptors.is_undefined() {
        return Err(Throw::type_error("property descriptors are nullish"));
    }
    // Validate all descriptors before defining any of them.
    let mut pending = Vec::new();
    for key in i.own_property_keys(&descriptors)? {
        if !i.object_own_enumerable(&descriptors, &key)? { continue; }
        let descriptor = i.get_prop(&descriptors, &key)?;
        let fields = i.std_descriptor_fields(&descriptor)?;
        // Keep the converted record, not the user object whose getters or
        // prototype can change while subsequent descriptors are collected.
        let mut snapshot = JsObject::ordinary();
        for (name, value) in [
            ("value", fields.value), ("writable", fields.writable.map(JsValue::Bool)),
            ("get", fields.get), ("set", fields.set),
            ("enumerable", fields.enumerable.map(JsValue::Bool)),
            ("configurable", fields.configurable.map(JsValue::Bool)),
        ] {
            if let Some(value) = value { snapshot.set_own(name, value); }
        }
        pending.push((key, JsValue::object(snapshot)));
    }
    for (key, descriptor) in pending {
        if !i.define_own_property(&target, &key, &descriptor)? {
            return Err(Throw::type_error("cannot define property"));
        }
    }
    Ok(JsValue::Object(target))
}

fn object_integrity(
    i: &mut Interpreter,
    args: Vec<JsValue>,
    freeze: bool,
) -> Result<JsValue, Throw> {
    let target = object_argument(&args);
    let JsValue::Object(object) = &target else {
        return Ok(target);
    };
    if i.is_window_proxy(object) || i.legacy_for(object).is_some() {
        return Err(Throw::type_error("cannot seal a legacy platform object"));
    }
    if freeze && matches!(object.borrow().exotic, Exotic::TypedArray) {
        let view = binary_view(&target)?;
        if binary_length(&view) != 0 || view.buffer.borrow().resizable {
            return Err(Throw::type_error("cannot freeze a TypedArray with indexed elements"));
        }
    }
    if !i.default_prevent_extensions(object)? {
        return Err(Throw::type_error("cannot prevent extensions on this object"));
    }
    for key in i.own_property_keys(&target)? {
        let writable = if freeze {
            let Some(property) = i.default_get_own_property(object, &key)? else { continue; };
            (property.getter.is_none() && property.setter.is_none()).then_some(false)
        } else { None };
        let descriptor = i.descriptor_fields_to_value(&StdDescriptorFields {
            configurable: Some(false), writable, ..Default::default()
        });
        if !i.define_own_property(object, &key, &descriptor)? {
            return Err(Throw::type_error("cannot set integrity level"));
        }
    }
    Ok(target)
}
fn native_object_freeze(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity(i, args, true)
}
fn native_object_seal(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity(i, args, false)
}

fn object_integrity_test(
    i: &mut Interpreter,
    args: Vec<JsValue>,
    freeze: bool,
) -> Result<JsValue, Throw> {
    let target = object_argument(&args);
    let JsValue::Object(object) = &target else {
        return Ok(JsValue::Bool(true));
    };
    if i.is_window_proxy(object) || i.legacy_for(object).is_some() || i.default_is_extensible(object)? {
        return Ok(JsValue::Bool(false));
    }
    for key in i.own_property_keys(&target)? {
        if let Some(property) = i.default_get_own_property(object, &key)? {
            if property.configurable
                || (freeze
                    && property.getter.is_none()
                    && property.setter.is_none()
                    && property.writable)
            {
                return Ok(JsValue::Bool(false));
            }
        }
    }
    Ok(JsValue::Bool(true))
}
fn native_object_is_frozen(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity_test(i, args, true)
}
fn native_object_is_sealed(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity_test(i, args, false)
}

fn native_object_group_by(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let items = object_argument(&args);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("groupBy callback is not callable"));
    }
    let iterator = object_iterator(i, items)?;
    let result = JsValue::object(JsObject::ordinary());
    let mut index = 0u64;
    loop {
        if index >= 9_007_199_254_740_991 {
            let _ = std_iterator_close(i, &iterator);
            return Err(Throw::type_error("too many groupBy items"));
        }
        let Some(value) = std_iterator_step(i, &iterator)? else { break; };
        let added = (|| {
            let group = i.call_value(
                &callback,
                JsValue::Undefined,
                vec![value.clone(), JsValue::num(index as f64)],
            )?;
            let key = i.to_property_key(&group)?;
            let current = i.get_prop(&result, &key)?;
            if current.is_undefined() {
                let group = array_values(i, vec![value])?;
                i.set_prop(&result, key, group)?;
            } else {
                let length = array_like_length(i, &current)?;
                i.set_prop(&current, length.to_string(), value)?;
            }
            Ok(())
        })();
        if let Err(error) = added {
            let _ = std_iterator_close(i, &iterator);
            return Err(error);
        }
        index += 1;
    }
    Ok(result)
}

fn native_object_is_prototype_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let Some(JsValue::Object(object)) = args.first() else {
        return Ok(JsValue::Bool(false));
    };
    let JsValue::Object(prototype) = std_legacy_to_object(i, this)? else { unreachable!() };
    let mut cursor = object.clone();
    loop {
        let JsValue::Object(current) = i.default_get_prototype(&cursor)? else { return Ok(JsValue::Bool(false)); };
        if Rc::ptr_eq(&current.0, &prototype.0) {
            return Ok(JsValue::Bool(true));
        }
        cursor = current;
    }
}

fn native_object_to_locale_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("toLocaleString receiver is nullish"));
    }
    let method = i.get_prop(&this, "toString")?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("toString is not callable"));
    }
    i.call_value(&method, this, Vec::new())
}

fn native_function_prototype(
    _: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Undefined)
}

fn native_function_to_string(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error(
            "Function.toString receiver is not callable",
        ));
    };
    let function = object
        .borrow()
        .func
        .clone()
        .ok_or_else(|| Throw::type_error("Function.toString receiver is not callable"))?;
    // A private method's spec name is "#name" (SetFunctionName's Description
    // for a Private Name key genuinely includes the `#`, and that's what
    // .name correctly reports) -- but harness/nativeFunctionMatcher.js's
    // NativeFunction grammar has no `#` production, so embedding it here
    // makes every private-method toString() fail both the exact-source-text
    // branch (we never have real source text to offer) and this fallback.
    // Render it as the anonymous form instead, exactly like a nameless
    // function already does.
    let name = if function.name.starts_with('#') { "" } else { function.name.as_str() };
    // NativeFunctionAccessor and PropertyName are separate grammar slots
    // (`function NativeFunctionAccessor? PropertyName? (...) {...}`), and
    // harness/nativeFunctionMatcher.js's PropertyName handling only accepts
    // a bare IdentifierName or a `[...]`-bracketed computed form (which is
    // how every Symbol-keyed method name here is already spelled from
    // construction, e.g. "[Symbol.iterator]") -- never an arbitrary
    // non-identifier string bare. RegExp's Annex B legacy accessors have
    // names like "get $&"/"get $'" whose property part (`$&`, `$'`, `` $` ``,
    // `$+`) isn't an IdentifierName, so omit that part entirely rather than
    // emit text the matcher can't parse
    // (built-ins/Function/prototype/toString/built-in-function-object.js
    // walks every built-in function reachable from the global object and
    // requires this to hold generically, not just for RegExp).
    let (accessor, prop_name) = if let Some(rest) = name.strip_prefix("get ") {
        ("get ", rest)
    } else if let Some(rest) = name.strip_prefix("set ") {
        ("set ", rest)
    } else {
        ("", name)
    };
    let prop_name = if prop_name.is_empty()
        || prop_name.starts_with('[')
        || is_valid_identifier_name(prop_name)
    {
        prop_name
    } else {
        ""
    };
    Ok(JsValue::str(format!(
        "function {accessor}{prop_name}() {{ [native code] }}"
    )))
}
/// A conservative ASCII-biased `IdentifierName` check: every legitimate
/// built-in function/accessor name in this engine is a plain ASCII
/// identifier (Symbol-keyed and private-name forms are already handled
/// separately, above), so this only needs to correctly reject the small set
/// of non-identifier property names that actually occur (`$&`, `$'`, `` $` ``,
/// `$+`), not fully replicate Unicode ID_Start/ID_Continue.
fn is_valid_identifier_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c == '$' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c == '$' || c.is_alphanumeric())
}

fn boolean_value(this: JsValue) -> Result<bool, Throw> {
    match this {
        JsValue::Bool(value) => Ok(value),
        JsValue::Object(object) => match object.borrow().boxed_primitive.clone() {
            Some(JsValue::Bool(value)) => Ok(value),
            _ => Err(Throw::type_error(
                "Boolean method called on incompatible receiver",
            )),
        },
        _ => Err(Throw::type_error(
            "Boolean method called on incompatible receiver",
        )),
    }
}
fn native_boolean_to_string(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::str(if boolean_value(this)? {
        "true"
    } else {
        "false"
    }))
}
fn native_boolean_value_of(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(boolean_value(this)?))
}

impl Interpreter {
    fn install_object_extra(&mut self, constructor: &JsValue) {
        let JsValue::Object(object) = constructor else {
            return;
        };
        for (name, length) in [
            ("assign", 2),
            ("create", 2),
            ("defineProperty", 3),
            ("getOwnPropertyDescriptor", 2),
            ("getOwnPropertyNames", 1),
            ("getOwnPropertySymbols", 1),
            ("getPrototypeOf", 1),
            ("hasOwn", 2),
            ("isExtensible", 1),
            ("keys", 1),
            ("preventExtensions", 1),
            ("setPrototypeOf", 2),
            ("values", 1),
        ] {
            let existing = object.borrow().own_prop(name).cloned();
            if let Some(mut property) = existing {
                property.enumerable = false;
                if let JsValue::Object(method) = &property.value {
                    method.borrow_mut().constructible = false;
                    method.borrow_mut().delete_own("prototype");
                }
                symbol_method_length(&property.value, length);
                object.borrow_mut().define(name, property).unwrap();
            }
        }
        for (name, native, length) in [
            (
                "defineProperties",
                native_object_define_properties as crate::value::NativeFn,
                2,
            ),
            ("entries", native_object_entries, 1),
            ("freeze", native_object_freeze, 1),
            ("fromEntries", native_object_from_entries, 1),
            ("getOwnPropertyDescriptors", native_object_descriptors, 1),
            ("groupBy", native_object_group_by, 2),
            ("is", native_object_is, 2),
            ("isFrozen", native_object_is_frozen, 1),
            ("isSealed", native_object_is_sealed, 1),
            ("seal", native_object_seal, 1),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            object
                .borrow_mut()
                .define(name, symbol_data(method, true, true))
                .unwrap();
        }
        for (name, native, length) in [
            (
                "isPrototypeOf",
                native_object_is_prototype_of as crate::value::NativeFn,
                1,
            ),
            ("toLocaleString", native_object_to_locale_string, 0),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.object_proto
                .borrow_mut()
                .define(name, symbol_data(method, true, true))
                .unwrap();
        }
        let method = symbol_method(self, "toString", native_function_to_string);
        self.function_proto
            .borrow_mut()
            .define("toString", symbol_data(method, true, true))
            .unwrap();
    }

    fn install_boolean_extra(&mut self, constructor: &JsValue) {
        let JsValue::Object(object) = constructor else {
            return;
        };
        object
            .borrow_mut()
            .define(
                "prototype",
                symbol_data(JsValue::Object(self.boolean_proto.clone()), false, false),
            )
            .unwrap();
        self.boolean_proto
            .borrow_mut()
            .define("constructor", symbol_data(constructor.clone(), true, true))
            .unwrap();
        self.boolean_proto.borrow_mut().boxed_primitive = Some(JsValue::Bool(false));
        for (name, native) in [
            (
                "toString",
                native_boolean_to_string as crate::value::NativeFn,
            ),
            ("valueOf", native_boolean_value_of),
        ] {
            let method = symbol_method(self, name, native);
            self.boolean_proto
                .borrow_mut()
                .define(name, symbol_data(method, true, true))
                .unwrap();
        }
    }
}

// EnumerableOwnProperties and ObjectDefineProperties snapshot keys but inspect
// each descriptor immediately before Get, after previous getters have run.
impl Interpreter {
    fn object_own_enumerable(&mut self, value: &JsValue, key: &PropertyKey) -> Result<bool, Throw> {
        match value {
            JsValue::Object(object) => {
                Ok(self.default_get_own_property(object, key)?.is_some_and(|property| property.enumerable))
            }
            JsValue::Str(string) => Ok(array_index_key(key).is_some_and(|index| (index as usize) < string.len())),
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod object_collection_tests {
    use super::*;

    #[test]
    fn descriptor_collection_snapshots_converted_fields_once() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var reads = 0, descriptor = { get value() { reads++; return 7; } };
            var props = { a: descriptor, get b() {
                Object.defineProperty(descriptor, 'value', {value: 9});
                return {value: 2};
            }};
            var target = Object.defineProperties({}, props);
            reads === 1 && target.a === 7 && target.b === 2;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn property_collection_rechecks_enumerability_after_getters() {
        for operation in ["Object.values(source)", "Object.entries(source)", "Object.assign({}, source)"] {
            let mut i = Interpreter::new();
            let script = format!(r#"
                var reads = 0, source = {{ get a() {{
                    Object.defineProperty(source, 'b', {{enumerable: false}});
                    Object.defineProperty(source, 'c', {{enumerable: true}});
                    return 1;
                }}, get b() {{ reads++; return 2; }} }};
                Object.defineProperty(source, 'c', {{value: 3, configurable: true}});
                var result = {operation};
                reads === 0 && (Array.isArray(result) ? result.length === 2 : result.c === 3 && !Object.hasOwn(result, 'b'));
            "#);
            assert_eq!(i.run_script(&script).unwrap(), JsValue::Bool(true), "{operation}");
        }
    }

    #[test]
    fn define_properties_rechecks_keys_and_validates_before_writing() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var target = {}, source = { get a() { delete source.b; return {value: 1}; }, b: null };
            Object.defineProperties(target, source);
            var sentinel = {}, failed = false, untouched = {};
            try { Object.defineProperties(untouched, {a: {value: 1}, get b() { throw sentinel; }}); }
            catch (e) { failed = e === sentinel; }
            target.a === 1 && !Object.hasOwn(target, 'b') && failed && !Object.hasOwn(untouched, 'a');
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn primitive_descriptors_preserve_code_units_and_key_conversion() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var descriptors = Object.getOwnPropertyDescriptors('\uD800x');
            var calls = 0, key = { toString() { calls++; return 'absent'; } };
            var absent = Object.getOwnPropertyDescriptor(Symbol(), key);
            descriptors[0].value === '\uD800' && descriptors[1].value === 'x' &&
                descriptors[0].enumerable && !descriptors[0].writable &&
                !descriptors[0].configurable && descriptors.length.value === 2 &&
                !descriptors.length.enumerable && absent === undefined && calls === 1 &&
                Object.getOwnPropertyDescriptor(42, 'x') === undefined;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn descriptors_create_own_proto_property_and_create_requires_prototype() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var source = Object.create(null);
            source.__proto__ = 42;
            var descriptors = Object.getOwnPropertyDescriptors(source), failed = false;
            try { Object.create(); } catch (e) { failed = e instanceof TypeError; }
            Object.hasOwn(descriptors, '__proto__') && descriptors.__proto__.value === 42 &&
                Object.getPrototypeOf(descriptors) === Object.prototype && failed;
        "#).unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod object_iterator_tests {
    use super::*;

    #[test]
    fn from_entries_interleaves_iteration_and_entry_conversion() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var effects = [], count = 0;
            var entries = { [Symbol.iterator]() { return this; }, next() {
                effects.push('next');
                if (count++) return {done:true};
                return {done:false, value: {
                    get 0() { effects.push('key'); return {toString() {effects.push('coerce'); return 'x';}}; },
                    get 1() { effects.push('value'); return 7; }
                }};
            }};
            var result = Object.fromEntries(entries);
            result.x === 7 && effects.join(',') === 'next,key,value,coerce,next';
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn object_iterators_close_on_consumer_errors_and_keep_original_throw() {
        for operation in ["Object.fromEntries(iterable)", "Object.groupBy(iterable, function() {throw sentinel;})"] {
            let mut i = Interpreter::new();
            let script = format!(r#"
                var sentinel = {{}}, closed = 0, caught;
                var iterable = {{ [Symbol.iterator]() {{ return {{
                    next() {{ return {{done:false, value: {{get 0() {{throw sentinel;}}}}}}; }},
                    return() {{ closed++; throw 'close error'; }}
                }}; }} }};
                try {{ {operation}; }} catch (error) {{ caught = error; }}
                closed === 1 && caught === sentinel;
            "#);
            assert_eq!(i.run_script(&script).unwrap(), JsValue::Bool(true), "{operation}");
        }
    }

    #[test]
    fn object_iterators_do_not_close_after_step_failure() {
        for operation in ["Object.fromEntries(iterable)", "Object.groupBy(iterable, function() {return 0;})"] {
            let mut i = Interpreter::new();
            let script = format!(r#"
                var sentinel = {{}}, closed = 0, caught;
                var iterable = {{ [Symbol.iterator]() {{ return {{
                    next() {{ return {{ get done() {{ throw sentinel; }} }}; }},
                    return() {{ closed++; return {{}}; }}
                }}; }} }};
                try {{ {operation}; }} catch (error) {{ caught = error; }}
                closed === 0 && caught === sentinel;
            "#);
            assert_eq!(i.run_script(&script).unwrap(), JsValue::Bool(true), "{operation}");
        }
    }

    #[test]
    fn group_by_callbacks_observe_and_extend_live_array_iteration() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var input = [1,2], seen = [];
            var groups = Object.groupBy(input, function(value, index) {
                seen.push(index);
                if (index === 0) input.push(3);
                return value % 2;
            });
            groups[1].join(',') === '1,3' && groups[0][0] === 2 &&
                seen.join(',') === '0,1,2' && Object.getPrototypeOf(groups) === null;
        "#).unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod object_string_wrapper_tests {
    use super::*;

    #[test]
    fn string_wrappers_expose_immutable_code_units_to_object_operations() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var boxed = Object('a\uD800b');
            var descriptor = Object.getOwnPropertyDescriptor(boxed, '1');
            var length = Object.getOwnPropertyDescriptor(boxed, 'length');
            var copied = Object.assign({}, boxed);
            var before = boxed[1];
            boxed[1] = 'changed';
            var deleted = delete boxed[1];
            Object.keys(boxed).join(',') === '0,1,2' &&
                Object.values(boxed)[1] === '\uD800' &&
                Object.entries(boxed)[1][1] === '\uD800' &&
                copied[1] === '\uD800' && descriptor.value === '\uD800' &&
                descriptor.enumerable && !descriptor.writable && !descriptor.configurable &&
                length.value === 3 && !length.enumerable && !length.writable &&
                !length.configurable && boxed[1] === before && !deleted;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn from_entries_accepts_boxed_string_entries_and_constructed_strings() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var entries = Object.fromEntries([Object('ab'), new String('cd')]);
            var receiver = {};
            var called = String.call(receiver, 'text');
            var boxed = new String('xy');
            var same = Object.defineProperty(boxed, '0', { value: 'x' });
            var rejected = false;
            try { Object.defineProperty(boxed, '0', { value: 'z' }); }
            catch (error) { rejected = error instanceof TypeError; }
            called === 'text' && !Object.hasOwn(receiver, 'length') &&
                entries.a === 'b' && entries.c === 'd' && same === boxed && rejected &&
                Object.keys(boxed).join(',') === '0,1';
        "#).unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod object_assignment_tests {
    use super::*;

    #[test]
    fn unsuccessful_property_assignment_only_throws_for_strict_references() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for prefix in ["", "'use strict';"] {
                let mut i = Interpreter::new();
                let source = format!(r#"{prefix}
                    var object = {{}};
                    Object.defineProperty(object, 'fixed', {{value: 3}});
                    var threw = false;
                    try {{ object.fixed = 4; }} catch (error) {{ threw = error instanceof TypeError; }}
                    threw === {} && object.fixed === 3;
                "#, !prefix.is_empty());
                assert_eq!(i.run_script_with_backend(&source, backend).unwrap(), JsValue::Bool(true),
                    "{backend:?}: {prefix}");
            }
        }
    }
}

// Complete the attributes of the runtime-owned intrinsic constructors after
// installation. Other lanes retain ownership of their constructors/methods.
impl Interpreter {
    fn install_runtime_intrinsic_attributes(&mut self) {
        for name in ["Object", "Function", "Array", "Boolean", "Promise", "Error",
            "EvalError", "RangeError", "ReferenceError", "SyntaxError", "TypeError", "URIError",
            "AggregateError", "SuppressedError"] {
            let Some(JsValue::Object(constructor)) = self.global_env.resolve(name) else { continue; };
            symbol_method_length(&JsValue::Object(constructor.clone()), match name {
                "AggregateError" => 2, "SuppressedError" => 3, _ => 1 });
            let prototype = constructor.borrow().get_own("prototype");
            if let Some(JsValue::Object(prototype)) = prototype {
                constructor.borrow_mut().define("prototype",
                    symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
                for key in ["constructor", "name", "message"] {
                    let property = prototype.borrow().own_prop(key).cloned();
                    if let Some(mut property) = property {
                        property.enumerable = false;
                        prototype.borrow_mut().define(key, property).unwrap();
                    }
                }
            }
        }
        if let Some(JsValue::Object(error)) = self.global_env.resolve("Error") {
            if let Some(JsValue::Object(prototype)) = error.borrow().get_own("prototype") {
                runtime_method_attributes(&prototype, "toString", 0);
            }
        }
        if let Some(JsValue::Object(promise)) = self.global_env.resolve("Promise") {
            for name in ["resolve", "reject", "all", "allSettled", "any", "race"] {
                runtime_method_attributes(&promise, name, 1);
            }
        }
        for (name, length) in [("then", 2), ("catch", 1), ("finally", 1)] {
            runtime_method_attributes(&self.promise_proto, name, length);
        }
    }
}

fn runtime_method_attributes(object: &Obj, name: &str, length: u32) {
    let property = object.borrow().own_prop(name).cloned();
    if let Some(mut property) = property {
        property.enumerable = false;
        if let JsValue::Object(method) = &property.value {
            method.borrow_mut().constructible = false;
            method.borrow_mut().delete_own("prototype");
        }
        symbol_method_length(&property.value, length);
        object.borrow_mut().define(name, property).unwrap();
    }
}
