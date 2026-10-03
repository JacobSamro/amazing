use amazing_js::interp::ExecutionBackend;
use amazing_js::value::JsValue;
use amazing_js::Interpreter;

fn check_both(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut interpreter = Interpreter::new();
        let result = interpreter
            .run_script_with_backend(source, backend)
            .unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe()));
        assert_eq!(result, JsValue::Bool(true), "{backend:?}");
    }
}

#[test]
fn class_method_names_preserve_symbol_descriptions_and_utf16_units() {
    check_both(
        r#"
        var symbol=Symbol('method'),empty=Symbol(),lone=Symbol('\uD800');
        class C {
            [symbol](a,b=0,c){} [empty](){} [lone](){}
            get ['\uD801'](){} set ['\uD802'](value){}
            static *[symbol](a,b){} static get [empty](){}
        }
        var get=Object.getOwnPropertyDescriptor(C.prototype,'\uD801').get;
        var set=Object.getOwnPropertyDescriptor(C.prototype,'\uD802').set;
        var emptyGet=Object.getOwnPropertyDescriptor(C,empty).get;
        var d=Object.getOwnPropertyDescriptor(C.prototype[symbol],'name');
        C.prototype[symbol].name==='[method]' && C.prototype[symbol].length===1 &&
        C.prototype[empty].name==='' && C.prototype[lone].name.length===3 &&
        C.prototype[lone].name.charCodeAt(1)===0xD800 &&
        get.name.length===5 && get.name.charCodeAt(4)===0xD801 && get.length===0 &&
        set.name.length===5 && set.name.charCodeAt(4)===0xD802 && set.length===1 &&
        C[symbol].name==='[method]' && C[symbol].length===2 && emptyGet.name==='get ' &&
        !d.writable && !d.enumerable && d.configurable;
    "#,
    );
}

#[test]
fn callable_property_order_survives_static_name_and_length_overrides() {
    check_both(
        r#"
        class C {static method(){} static name(){} static length(){}}
        class D {constructor(a,b=0,c){}}
        var f=function(a,b=0,c){},arrow=(a,b)=>a;
        var keys=Object.getOwnPropertyNames(C),ordinary=Object.getOwnPropertyNames(f);
        keys.join(',')==='length,name,prototype,method' &&
        ordinary.indexOf('length')<ordinary.indexOf('name') &&
        Object.getOwnPropertyNames(arrow).slice(0,2).join(',')==='length,name' &&
        C.name.name==='name' && C.length.name==='length' && D.length===1 && f.length===1;
    "#,
    );
}

#[test]
fn property_and_field_named_evaluation_is_lossless_and_source_restricted() {
    check_both(
        r#"
        var conversions=0,key={[Symbol.toPrimitive](){conversions++;return '\uD800';}};
        var sym=Symbol('\uD801'),o={[key]:function(){},[sym]:()=>0,
            ['\uD802']:async function(){},negative:(0,function(){})};
        class C {['\uD803']=function(){};[sym]=()=>1;negative=(0,function(){});
            static ['\uD804']=()=>0;}
        var c=new C();
        conversions===1 && o['\uD800'].name.length===1 && o['\uD800'].name.charCodeAt(0)===0xD800 &&
        o[sym].name.length===3 && o[sym].name.charCodeAt(1)===0xD801 &&
        o['\uD802'].name.charCodeAt(0)===0xD802 && o.negative.name==='' &&
        c['\uD803'].name.charCodeAt(0)===0xD803 && c[sym].name.charCodeAt(1)===0xD801 &&
        c.negative.name==='' && C['\uD804'].name.charCodeAt(0)===0xD804;
    "#,
    );
}

#[test]
fn suspending_class_keys_use_the_same_method_metadata() {
    check_both(
        r#"
        var sym=Symbol('\uD800');
        function* factory(){return class {[yield 1](a,b=0){};static get [yield 2](){};};}
        var iterator=factory();var first=iterator.next(),second=iterator.next(sym),last=iterator.next('\uD801');
        var C=last.value,get=Object.getOwnPropertyDescriptor(C,'\uD801').get;
        first.value===1 && second.value===2 && last.done && C.prototype[sym].name.length===3 &&
        C.prototype[sym].name.charCodeAt(1)===0xD800 && C.prototype[sym].length===1 &&
        get.name.length===5 && get.name.charCodeAt(4)===0xD801;
    "#,
    );
}

#[test]
fn assignment_naming_preserves_reference_conversion_order_and_negative_forms() {
    check_both(
        r#"
        var events=[],holder={},key={[Symbol.toPrimitive](){events.push('key');return 'method';}};
        holder[key]=(events.push('value'),function(){});
        var first=events.join(',')==='value,key' && holder.method.name==='';
        var plain,grouped,logical,negative,existing=(0,function(){}),reassigned;
        plain=function(){};grouped=(function(){});logical??=()=>0;
        negative??=(0,()=>0);reassigned=existing;
        first && plain.name==='plain' && grouped.name==='grouped' && logical.name==='logical' &&
        negative.name==='' && reassigned.name==='' && existing.name==='';
    "#,
    );
}
