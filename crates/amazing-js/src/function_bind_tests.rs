#[cfg(test)]
mod function_bind_protocol_tests {
    use super::*;

    fn matrix(source: &str) {
        for ast in [false, true] {
            let mut i = Interpreter::new();
            i.force_ast_callables = ast;
            let probe = i.run_script("(function(a,b){return a+b})").unwrap();
            let JsValue::Object(probe) = probe else { panic!("callable probe") };
            let body = probe.borrow().func.as_ref().unwrap().body.clone();
            assert!(if ast { matches!(body, FunctionBody::Block(_)) }
                else { matches!(body, FunctionBody::Bytecode(_)) }, "actual callable carrier ast={ast}");
            assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true), "ast={ast}: {source}");
        }
    }

    #[test]
    fn bound_creation_observes_actual_prototype_before_own_length_and_name() {
        matrix(r#"
            var log=[], f=function(a,b,c){return this.x+a+b+c}, receiver={x:4};
            var proto=Object.create(Function.prototype);
            var p=new Proxy(f,{
                getPrototypeOf(t){log.push('proto');return proto},
                getOwnPropertyDescriptor(t,k){log.push('own:'+k);return Reflect.getOwnPropertyDescriptor(t,k)},
                get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r)},
                has(){throw 'unexpected has'}
            });
            var b=Function.prototype.bind.call(p,receiver,1);
            log.join(',')==='proto,own:length,get:length,get:name' &&
            Object.getPrototypeOf(b)===proto && b(2,3)===10 && b.length===2 && b.name==='bound f'
        "#);
    }

    #[test]
    fn bound_metadata_number_cases_do_not_coerce_non_numbers_or_inherited_length() {
        for (length, expected) in [("NaN","0"),("Infinity","Infinity"),
            ("-Infinity","0"),("-0","0"),("3.9","1"),("-3.9","0"),
            ("'9'","0"),("({valueOf(){throw 'coercion'}})","0")] {
            matrix(&format!(r#"
                var f=function(){{}}; Object.defineProperty(f,'length',{{value:{length},configurable:true}});
                var b=f.bind(null,1,2),d=Object.getOwnPropertyDescriptor(b,'length');
                b.length==={expected} && !d.writable && !d.enumerable && d.configurable
            "#));
        }
        matrix(r#"
            var f=function(){};delete f.length;
            var proto=Object.create(Function.prototype);
            Object.defineProperty(proto,'length',{get(){throw 'inherited length'}});
            Object.setPrototypeOf(f,proto);f.bind(null).length===0
        "#);
    }

    #[test]
    fn bound_names_keep_exact_utf16_without_string_coercion() {
        for name in [r#"'\ud800\u0000\udc00😀'"#, "Symbol('name')", "({toString(){throw 'coercion'}})"] {
            let expected = if name.starts_with('\'') { name } else { "''" };
            matrix(&format!(r#"
                var f=function(){{}};Object.defineProperty(f,'name',{{value:{name},configurable:true}});
                var b=f.bind(null),c=b.bind(null),d=Object.getOwnPropertyDescriptor(b,'name');
                b.name==='bound '+{expected} && c.name==='bound bound '+{expected} &&
                !d.writable && !d.enumerable && d.configurable && !Object.prototype.hasOwnProperty.call(b,'prototype')
            "#));
        }
    }

    #[test]
    fn every_bind_protocol_abrupt_keeps_original_throw_and_stops_following_steps() {
        for stop in ["proto", "own:length", "get:length", "get:name"] {
            matrix(&format!(r#"
                var marker={{}},log=[],f=function(a){{}};
                function step(k){{log.push(k);if(k==='{stop}')throw marker}}
                var p=new Proxy(f,{{getPrototypeOf(t){{step('proto');return Function.prototype}},
                    getOwnPropertyDescriptor(t,k){{step('own:'+k);return Reflect.getOwnPropertyDescriptor(t,k)}},
                    get(t,k,r){{step('get:'+k);return Reflect.get(t,k,r)}}}});
                var caught=false;try{{Function.prototype.bind.call(p,null)}}catch(e){{caught=e===marker}}
                caught && log[log.length-1]==='{stop}'
            "#));
        }
        matrix(r#"
            var r=Proxy.revocable(function(){},{});r.revoke();
            var ok=false;try{Function.prototype.bind.call(r.proxy,null)}catch(e){ok=e instanceof TypeError}ok
        "#);
    }

    #[test]
    fn private_bound_argument_list_bypasses_array_setters_and_iterators() {
        matrix(r#"
            var f=function(a,b,c){return a===marker&&b===2&&c===3},marker={},hits=0;
            Object.defineProperty(Array.prototype,'0',{set(){hits++;throw 'indexed setter'},configurable:true});
            var saved=Array.prototype[Symbol.iterator];
            Array.prototype[Symbol.iterator]=function(){throw 'iterator'};
            var b=f.bind(null,marker,2),ok=b(3);
            delete Array.prototype[0];Array.prototype[Symbol.iterator]=saved;ok&&hits===0
        "#);
    }

    #[test]
    fn nullable_and_live_proxy_prototype_is_captured_once_before_metadata() {
        matrix(r#"
            var f=function(){return 7},first=Object.create(Function.prototype),second={};
            var p=new Proxy(f,{getPrototypeOf(){return first},get(t,k,r){
                if(k==='length')Object.setPrototypeOf(t,second);return Reflect.get(t,k,r)}});
            var b=Function.prototype.bind.call(p,null);
            Object.getPrototypeOf(b)===first && b()===7
        "#);
        matrix(r#"
            var f=function(){return 8};Object.setPrototypeOf(f,null);
            var b=Function.prototype.bind.call(f,null);Object.getPrototypeOf(b)===null&&b()===8
        "#);
    }

    #[test]
    fn repeated_genuine_bind_through_mixed_forwarders_preserves_call_and_construct() {
        matrix(r#"
            function f(a){this.a=a;this.target=new.target;return this===receiver?a:undefined}var receiver={};
            var wrapped=f;for(var n=0;n<96;n++)wrapped=n%2?new Proxy(wrapped,{}):wrapped.bind(receiver);
            var called=wrapped(9),created=new wrapped(11);
            called===9&&created.a===11&&created.target===wrapped&&Object.getPrototypeOf(created)===Object.prototype
        "#);
    }

    #[test]
    fn bound_construct_substitutes_only_its_own_new_target_and_keeps_nonconstructors() {
        matrix(r#"
            function F(a,b){this.args=a+','+b;this.target=new.target}
            var b=F.bind({ignored:true},1),a=new b(2);function Other(){}
            var c=Reflect.construct(b,[3],Other),arrow=(()=>1).bind(null),caught=false;
            try{new arrow}catch(e){caught=e instanceof TypeError}
            a.args==='1,2'&&a.target===F&&Object.getPrototypeOf(a)===F.prototype&&
            c.args==='1,3'&&c.target===Other&&Object.getPrototypeOf(c)===Other.prototype&&caught
        "#);
    }

    #[test]
    fn all_callable_kinds_keep_capabilities_and_original_abrupt_values() {
        matrix(r#"
            var marker={},f=function(){throw marker},b=f.bind(null),same=false;
            try{b()}catch(e){same=e===marker}
            var g=(function*(a){yield a}).bind(null,7),a=(async function(a){return a}).bind(null,8);
            var ag=(async function*(a){yield a}).bind(null,9),ok=same&&g().next().value===7;
            for(var q of [g,a,ag]){try{new q;ok=false}catch(e){ok=ok&&e instanceof TypeError}}
            var bad=[null,undefined,0,'x',{},Symbol('x')];
            for(var value of bad){try{Function.prototype.bind.call(value,null);ok=false}
                catch(e){ok=ok&&e instanceof TypeError}}
            ok&&a.length===0&&ag.length===0
        "#);
    }

    #[test]
    fn borrowed_bind_preserves_actual_foreign_target_prototype_and_invocation() {
        for ast in [false,true] {
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            let realm=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            let target=i.binding_eval_in_realm(realm,"(function(a){return a})").unwrap();
            let child_proto=i.binding_eval_in_realm(realm,"Function.prototype").unwrap();
            let bind=i.run_script("Function.prototype.bind").unwrap();
            let bound=i.call_value(&bind,target,vec![JsValue::Null,JsValue::num(9.0)]).unwrap();
            let JsValue::Object(object)=&bound else{panic!("bound object")};
            assert_eq!(i.default_get_prototype(object).unwrap(),child_proto);
            assert_eq!(i.call_value(&bound,JsValue::Undefined,vec![]).unwrap(),JsValue::num(9.0));
        }
    }
}
