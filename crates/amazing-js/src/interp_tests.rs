#[cfg(test)]
mod tests {
    use super::*;

    fn num(v: JsValue) -> f64 {
        match v {
            JsValue::Num(n) => n,
            _ => panic!("expected number: {v:?}"),
        }
    }

    #[test]
    fn evaluates_arithmetic_and_coercion() {
        let mut i = Interpreter::new();
        assert_eq!(num(i.run_script("1 + 2 * 3").unwrap()), 7.0);
        let v = i.run_script("'a' + 2").unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a2");
    }

    #[test]
    fn closures_capture_and_mutate_lexical_bindings() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("function make(){ let n=0; return ()=>++n; } let f=make(); f()+f();")
            .unwrap();
        assert_eq!(num(v), 3.0);
    }

    #[test]
    fn arrays_objects_and_builtin_methods_work() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let a=[1,2]; a.push(3); a.join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "1-2-3");
        let v = i.run_script("Object.keys({a:1,b:2}).length").unwrap();
        assert_eq!(num(v), 2.0);
    }

    #[test]
    fn object_keys_result_inherits_array_methods() {
        let mut interpreter = Interpreter::new();
        let value = interpreter
            .run_script("Object.keys({a:1,b:2}).join(',');")
            .expect("Object.keys result has Array.prototype");
        assert_eq!(interpreter.to_string(&value).unwrap(), "a,b");
    }

    #[test]
    fn common_array_string_and_object_utilities_work() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("[1,2,3].map(x=>x*2).filter(x=>x>2).reduce((a,b)=>a+b,0);")
            .unwrap();
        assert_eq!(num(value), 10.0);
        let value = i
            .run_script("'  hello  '.trim().includes('ell') && 'abc'.charAt(1)==='b' && 'abc'.indexOf('c')===2;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i.run_script("Object.values({a:1,b:2}).join(',');").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "1,2");
        let value = i
            .run_script("[1,2,3,4].slice(1,3).concat([5]).includes(5) && [1,2,3].indexOf(2)===1 && 'abcdef'.replace('cd','X')==='abXef' && 'abc'.startsWith('a') && 'abc'.endsWith('c');")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let mut k = Interpreter::new();
        let value = k
            .run_script("Array.from('ab').concat(Array.of(3)).join(',') === 'a,b,3' && Number.isNaN(NaN) && Number.isFinite(3);")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = k
            .run_script("Math.floor(1.9)===1 && Math.ceil(1.1)===2 && Math.round(1.5)===2 && Math.sqrt(9)===3 && Math.pow(2,3)===8 && Math.trunc(-1.9)===-1 && Math.sign(-2)===-1;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let mut j = Interpreter::new();
        let value = j
            .run_script("JSON.stringify({a:1,b:[2]}) === '{\"a\":1,\"b\":[2]}' && JSON.parse('{\"x\":4}').x === 4 && parseInt('0x10') === 16 && parseFloat('1.5px') === 1.5;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn function_call_apply_and_bind_preserve_receiver_and_arguments() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("function f(a,b){return this.x+a+b;} let o={x:1}; f.call(o,2,3)+f.apply(o,[2,3])+f.bind(o,2)(3);")
            .unwrap();
        assert_eq!(num(value), 18.0);
    }

    #[test]
    fn object_prototype_helpers_and_array_is_array_work() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("let p={x:4}; let o=Object.create(p); o.y=2; Object.getPrototypeOf(o)===p && Object.hasOwn(o,'y') && !Object.hasOwn(o,'x') && Array.isArray([]) && !Array.isArray({});")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn class_inheritance_dispatches_super_methods_with_the_real_receiver() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class A { value(){ return 4; } } class B extends A { value(){ return super.value()+1; } } new B().value();")
            .unwrap();
        assert_eq!(num(value), 5.0);
    }

    #[test]
    fn derived_constructor_can_call_super() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class A { constructor(value){ this.value=value; } } class B extends A { constructor(value){ super(value+1); } } new B(4).value;")
            .unwrap();
        assert_eq!(num(value), 5.0);
    }

    #[test]
    fn class_getters_and_setters_use_property_descriptors() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class C { constructor(){this._x=1;} get x(){return this._x;} set x(v){this._x=v;} } let c=new C(); c.x=7; c.x;")
            .unwrap();
        assert_eq!(num(value), 7.0);
    }

    #[test]
    fn map_uses_same_value_zero_and_preserves_insertion_order() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let m=new Map([['a',1],['b',2]]); m.set('a',9).set('c',3); m.get('a') + m.size;",
            )
            .unwrap();
        assert_eq!(num(v), 12.0);
        let v = i
            .run_script("new Map([['a',1],['b',2]]).keys().join(',');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a,b");
        let v = i
            .run_script("let m2=new Map(); m2.set(NaN,1); m2.has(NaN);")
            .unwrap();
        assert!(matches!(v, JsValue::Bool(true)));
    }

    #[test]
    fn set_deduplicates_same_value_zero_and_supports_mutation() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let s=new Set([1,1,2,NaN,NaN]); s.add(3); s.delete(2); s.size;")
            .unwrap();
        assert_eq!(num(v), 3.0);
        let v = i
            .run_script("new Set(['x','y']).values().join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "x-y");
    }

    #[test]
    fn settled_promises_chain_and_catch_synchronously() {
        let mut i = Interpreter::new();
        i.run_script(
            "Promise.resolve(2).then(x=>x+1).then(x=>console.log(x)); Promise.reject('bad').catch(e=>console.log(e));",
        )
        .unwrap();
        assert_eq!(&*i.output.borrow(), &["bad".to_string(), "3".to_string()]);
    }

    #[test]
    fn pending_promise_reactions_run_when_resolved() {
        let mut i = Interpreter::new();
        i.run_script(
            "let resolve; let p=new Promise(r=>{resolve=r;}); p.then(x=>console.log(x)); resolve(5);",
        )
        .unwrap();
        assert_eq!(&*i.output.borrow(), &["5".to_string()]);
    }

    #[test]
    fn async_function_returns_a_promise() {
        let mut i = Interpreter::new();
        i.run_script("async function f(){ return 4; } f().then(x=>console.log(x+1));")
            .unwrap();
        assert_eq!(&*i.output.borrow(), &["5".to_string()]);
    }

    #[test]
    fn promise_reactions_run_after_current_script_turn() {
        let mut i = Interpreter::new();
        i.run_script(
            "console.log('before'); Promise.resolve(1).then(x=>console.log(x)); console.log('after');",
        )
        .unwrap();
        assert_eq!(
            &*i.output.borrow(),
            &["before".to_string(), "after".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn eval_wrappers_and_number_constants_follow_primitive_rules() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("eval('1 + 1') + new Boolean(true) + Number.MIN_VALUE > 2;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i
            .run_script("Number.POSITIVE_INFINITY + 1 === Infinity;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn date_default_value_uses_string_conversion_and_in_uses_left_key() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("let d = new Date(0); d + d === d.toString() + d.toString();")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i.run_script("'x' in {x: 1};").unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn compound_assignment_evaluates_member_reference_once() {
        let mut i = Interpreter::new();
        let value = i
            .run_script(
                "let calls=0; function base(){calls++; return {x:0};} base().x ||= 2; calls;",
            )
            .unwrap();
        assert_eq!(num(value), 1.0);
    }

    #[test]
    fn descriptors_and_completion_values_are_observable() {
        let mut i = Interpreter::new();
        let value = i
            .run_script(
                "let o={}; Object.defineProperty(o,'x',{get:function(){return 0;},set:undefined,enumerable:true,configurable:true}); o.x &&= 1; o.x;",
            )
            .unwrap();
        assert_eq!(num(value), 0.0);
        let value = i
            .run_script("eval('1; for (var x=true; x; x=false) { 3; }');")
            .unwrap();
        assert_eq!(num(value), 3.0);
    }

    #[test]
    fn promise_combinators_preserve_order_and_settlement_kind() {
        let mut i = Interpreter::new();
        i.run_script(
            "Promise.all([Promise.resolve(1), 2]).then(v=>console.log(v.join(','))); Promise.allSettled([Promise.resolve(3), Promise.reject(4)]).then(v=>console.log(v[0].status, v[1].status)); Promise.any([Promise.reject(1), Promise.resolve(5)]).then(v=>console.log(v));",
        )
        .unwrap();
        assert_eq!(
            &*i.output.borrow(),
            &[
                "1,2".to_string(),
                "fulfilled rejected".to_string(),
                "5".to_string()
            ]
        );
    }

    #[test]
    fn array_object_and_for_of_patterns_bind_values_defaults_and_rest() {
        let mut i = Interpreter::new();
        let value = match i.run_script(
            "let [a,b=3,...tail]=[1,undefined,4,5]; let {x,y:z=4,...rest}={x:1,y:undefined,q:2}; a+b+tail.length+x+z+Object.keys(rest).length;",
        ) {
            Ok(value) => value,
            Err(Throw(JsValue::Object(object))) => panic!("pattern error: {:?}", object.borrow().get_own("message")),
            Err(error) => panic!("pattern error: {:?}", error.0),
        };
        assert_eq!(num(value), 12.0);
        let mut j = Interpreter::new();
        let value = j
            .run_script("let total=0; for (const [a,b] of [[2,3]]) { total=a+b; } total;")
            .unwrap();
        assert_eq!(num(value), 5.0);
        let mut k = Interpreter::new();
        let value = k
            .run_script("let a=0,b=0; [a,b]=[3,4]; ({a,b}={a:5,b:6}); a+b;")
            .unwrap();
        assert_eq!(num(value), 11.0);
        let mut m = Interpreter::new();
        let value = m
            .run_script("let key='x'; let {[key]: value}={x:9}; value;")
            .unwrap();
        assert_eq!(num(value), 9.0);
        let mut n = Interpreter::new();
        let value = n
            .run_script("let a=0,b=0; [a=3,b]=[undefined,4]; ({a,b}={a:5,b:6}); a+b;")
            .unwrap();
        assert_eq!(num(value), 11.0);
        let mut p = Interpreter::new();
        let value = p
            .run_script(
                "let out=[]; for (const [a,b] of [[1,2],[3,4]]) { out.push(a+b); } out.join(',');",
            )
            .unwrap();
        assert_eq!(p.to_string(&value).unwrap(), "3,7");
        let mut q = Interpreter::new();
        let value = q
            .run_script("let out=[]; let it={n:0,next:function(){this.n++;return {value:this.n,done:this.n>2};}}; for (const x of it) out.push(x); out.join(',');")
            .unwrap();
        assert_eq!(q.to_string(&value).unwrap(), "1,2");
        let mut r = Interpreter::new();
        let value = r
            .run_script("let closed=0; let it={next:function(){throw 'stop';},return:function(){closed++;}}; try {[...it];} catch(e) {} closed;")
            .unwrap();
        assert_eq!(num(value), 1.0);
        let mut s = Interpreter::new();
        let value = s
            .run_script("let closed=0; let n=0; let it={next:function(){n++;return {value:n,done:n>3};},return:function(){closed++;}}; try { for (const x of it) { if (x===2) throw 'stop'; } } catch(e) {} closed;")
            .unwrap();
        assert_eq!(num(value), 1.0);
        let mut t = Interpreter::new();
        let value = t
            .run_script("let it={n:0,next:function(){this.n++;return {value:this.n,done:this.n>2};}}; let iterable={ [Symbol.iterator]:function(){return it;} }; [...iterable].join(',');")
            .unwrap();
        assert_eq!(t.to_string(&value).unwrap(), "1,2");
    }

    #[test]
    fn loops_and_try_catch_are_executable() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let s=0; for(let n=0;n<5;n++){s+=n;} try { throw 4; } catch(e) { s+=e; } s;",
            )
            .unwrap();
        assert_eq!(num(v), 14.0);
    }

    #[test]
    fn console_output_is_captured_without_io() {
        let mut i = Interpreter::new();
        i.run_script("console.log('hello', 3);").unwrap();
        assert_eq!(&*i.output.borrow(), &["hello 3".to_string()]);
    }

    #[test]
    fn timers_run_only_when_the_host_pumps_them() {
        let mut i = Interpreter::new();
        i.run_script("setTimeout(()=>console.log('later'),0);").unwrap();
        assert!(i.output.borrow().is_empty());
        assert!(i.pump_timers().unwrap());
        assert_eq!(&*i.output.borrow(), &["later".to_string()]);
        assert!(!i.pump_timers().unwrap());
        let mut interval = Interpreter::new();
        interval.run_script("setInterval(()=>console.log('tick'),0);").unwrap();
        assert!(interval.pump_timers().unwrap());
        assert!(interval.pump_timers().unwrap());
        interval.run_script("clearInterval(1);").unwrap();
        assert!(!interval.pump_timers().unwrap());
    }

    #[test]
    fn location_requests_navigation_and_updates_href() {
        let mut i = Interpreter::new();
        i.set_location_url("page.html");
        let value = i
            .run_script("location.assign('next.html'); location.href;")
            .unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "next.html");
        assert_eq!(i.take_navigation().as_deref(), Some("next.html"));
        i.run_script("window.location='other.html';").unwrap();
        assert_eq!(i.take_navigation().as_deref(), Some("other.html"));
        i.run_script("history.back();").unwrap();
        assert_eq!(i.take_history_delta(), Some(-1));
        i.run_script("location.reload();").unwrap();
        assert_eq!(i.take_navigation().as_deref(), Some("other.html"));
        i.set_history_length(3);
        let length = i.run_script("history.length;").unwrap();
        assert_eq!(length, JsValue::num(3.0));
        let state_value = i.run_script("history.pushState({id:7}, '', 'spa.html'); history.state.id;").unwrap();
        assert_eq!(state_value, JsValue::num(7.0));
        let update = i.take_history_update().unwrap();
        assert_eq!((&update.0, update.1), (&"spa.html".to_string(), false));
        i.run_script("history.replaceState({id:8}, '', 'spa2.html');").unwrap();
        let update = i.take_history_update().unwrap();
        assert_eq!((&update.0, update.1), (&"spa2.html".to_string(), true));
    }
}
