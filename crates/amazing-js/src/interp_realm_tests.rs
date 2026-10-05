#[cfg(test)]
mod realm_tests {
    use super::*;

    #[test]
    fn realms_keep_intrinsics_separate_and_objects_shared() {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let global = i.binding_create_object(None).unwrap();
        let child = i.binding_create_realm(global).unwrap();
        let object = i.binding_eval_in_realm(child, "({ marker: 7 })").unwrap();
        i.global_env.declare_var("foreign", object.clone()).unwrap();
        assert_eq!(i.run_script("foreign.marker = 9; foreign").unwrap(), object);
        let child_array = i.binding_eval_in_realm(child, "Array").unwrap();
        assert_ne!(child_array, i.run_script("Array").unwrap());
        let factory = i.binding_eval_in_realm(child, "(() => [])").unwrap();
        let array = i.call_value(&factory, JsValue::Undefined, vec![]).unwrap();
        i.global_env.declare_var("foreignArray", array).unwrap();
        assert_eq!(i.run_script("Array.isArray(foreignArray) && !(foreignArray instanceof Array)").unwrap(), JsValue::Bool(true));
        assert_eq!(i.binding_current_realm(), parent);
        let parent_symbol = i.run_script("Symbol.for('shared')").unwrap();
        assert_eq!(parent_symbol, i.binding_eval_in_realm(child, "Symbol.for('shared')").unwrap());
    }

    #[test]
    fn window_proxy_keeps_identity_when_its_target_changes() {
        let mut i = Interpreter::new();
        let first_global = i.binding_create_object(None).unwrap();
        let first = i.binding_create_realm(first_global).unwrap();
        let second_global = i.binding_create_object(None).unwrap();
        let second = i.binding_create_realm(second_global).unwrap();
        i.binding_eval_in_realm(first, "var version = 1").unwrap();
        i.binding_eval_in_realm(second, "var version = 2").unwrap();
        let first_handle = HostHandle { arena: 1, slot: 1, generation: 0, brand: 77 };
        let second_handle = HostHandle { arena: 2, slot: 1, generation: 0, brand: 77 };
        i.binding_with_realm(first, |i| i.binding_brand_global(first_handle)).unwrap();
        i.binding_with_realm(second, |i| i.binding_brand_global(second_handle)).unwrap();
        let proxy = i.binding_create_window_proxy(first).unwrap();
        assert_eq!(i.binding_host_handle(&proxy, 77), Some(first_handle));
        let same = proxy.clone();
        assert_eq!(i.get_prop(&proxy, "version").unwrap(), JsValue::num(1.0));
        i.binding_retarget_window_proxy(&proxy, second).unwrap();
        assert_eq!(proxy, same);
        assert_eq!(i.binding_host_handle(&proxy, 77), Some(second_handle));
        assert_eq!(i.get_prop(&proxy, "version").unwrap(), JsValue::num(2.0));
    }
    #[test]
    fn realm_calls_restore_globals_and_error_prototypes() {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let function = i.binding_eval_in_realm(child, "(function () { return [this, Object, new Error('child')]; })").unwrap();
        let returned = i.call_value(&function, JsValue::Undefined, vec![]).unwrap();
        assert_eq!(i.get_prop(&returned, "0").unwrap(), i.binding_realm_global(child).unwrap());
        assert_ne!(i.get_prop(&returned, "1").unwrap(), i.run_script("Object").unwrap());
        let error = i.binding_eval_in_realm(child, "null.missing()").unwrap_err();
        let child_type_error = i.binding_eval_in_realm(child, "TypeError.prototype").unwrap();
        assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined, vec![error.0]).unwrap(), child_type_error);
        assert_eq!(i.binding_current_realm(), parent);
        let foreign_constructor = i.binding_eval_in_realm(child, "(function Foreign(){})").unwrap();
        i.set_prop(&foreign_constructor, "prototype", JsValue::Null).unwrap();
        i.global_env.declare_var("Foreign", foreign_constructor).unwrap();
        let constructed = i.run_script("Reflect.construct(function(){}, [], Foreign)").unwrap();
        let child_object_prototype = i.binding_eval_in_realm(child, "Object.prototype").unwrap();
        assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined, vec![constructed]).unwrap(), child_object_prototype);
        let foreign_global = i.binding_realm_global(child).unwrap();
        i.global_env.declare_var("foreignGlobal", foreign_global).unwrap();
        assert_eq!(i.run_script(r#"
            Object.getPrototypeOf(Reflect.construct(Array, [1, 2], Foreign)) === foreignGlobal.Array.prototype &&
            Object.getPrototypeOf(Reflect.construct(TypeError, ['child'], Foreign)) === foreignGlobal.TypeError.prototype &&
            Object.getPrototypeOf(Reflect.construct(String, ['abc'], Foreign)) === foreignGlobal.String.prototype &&
            Object.getPrototypeOf(Reflect.construct(Object, [42], Foreign)) === foreignGlobal.Object.prototype
        "#).unwrap(), JsValue::Bool(true));
        let foreign = Interpreter::new().binding_current_realm();
        assert!(i.binding_eval_in_realm(foreign, "1").is_err());
    }

    #[test]
    fn binding_function_realm_resolves_native_bound_and_proxy_targets() {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let native = i.binding_eval_in_realm(child, "Object").unwrap();
        i.global_env.declare_var("foreign", native.clone()).unwrap();
        let bound = i.run_script("foreign.bind(null)").unwrap();
        let proxy = i.run_script("new Proxy(foreign,{})").unwrap();
        let nested = i.run_script("new Proxy(foreign.bind(null),{}).bind(null)").unwrap();
        for function in [native, bound, proxy, nested] {
            assert_eq!(i.binding_function_realm(&function).unwrap(), child);
        }
        let revoked = i.run_script("var r=Proxy.revocable(foreign,{});r.revoke();r.proxy").unwrap();
        let error = i.binding_function_realm(&revoked).unwrap_err();
        assert!(error.describe().contains("revoked"));
        let local = i.run_script("Object").unwrap();
        assert_eq!(i.binding_function_realm(&local).unwrap(), parent);
        assert_eq!(i.binding_current_realm(), parent);
    }

    #[test]
    fn realms_share_one_external_job_queue() {
        let mut i = Interpreter::new_with_external_scheduler(32).unwrap();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        i.run_script("Promise.resolve().then(() => console.log('parent-1'))").unwrap();
        i.binding_eval_in_realm(child, "Promise.resolve().then(() => console.log('child')); queueMicrotask(() => console.log('child-microtask'))").unwrap();
        i.run_script("Promise.resolve().then(() => console.log('parent-2'))").unwrap();
        assert!(i.output.borrow().is_empty());
        for request in i.take_scheduling_requests() {
            if let SchedulingRequest::Microtask { job_token } = request { i.invoke_one_job(job_token).unwrap(); }
        }
        assert_eq!(*i.output.borrow(), ["parent-1", "child", "child-microtask", "parent-2"]);
    }

    #[test]
    fn window_proxy_reflection_receivers_and_global_bindings() {
        let mut i = Interpreter::new();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let proxy = i.binding_create_window_proxy(child).unwrap();
        i.global_env.declare_var("win", proxy.clone()).unwrap();
        i.binding_eval_in_realm(child, "var count = 1; Object.defineProperty(globalThis, 'receiver', {get: function(){return this;}, configurable:true});").unwrap();
        assert_eq!(i.run_script("win.count === 1 && win.receiver === win && Object.hasOwn(win, 'count') && Reflect.has(win, 'count')").unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script("win.count = 2; Reflect.defineProperty(win, 'extra', {value:3, enumerable:true, configurable:true}); win.extra").unwrap(), JsValue::num(3.0));
        assert_eq!(i.binding_eval_in_realm(child, "count").unwrap(), JsValue::num(2.0));
        assert_eq!(i.run_script("var derived = Object.create(win); derived.count = 7; derived.count === 7 && win.count === 2 && 'count' in derived").unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script("Object.keys(win).includes('extra') && Reflect.deleteProperty(win, 'extra') && !Object.hasOwn(win, 'extra')").unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script("var cyclic = {}; var rejected = false; try { Object.setPrototypeOf(cyclic, cyclic); } catch (e) { rejected = e instanceof TypeError; } rejected").unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script("Reflect.isExtensible(win) && !Reflect.preventExtensions(win) && Reflect.setPrototypeOf(win, Reflect.getPrototypeOf(win)) && !Reflect.setPrototypeOf(win, null)").unwrap(), JsValue::Bool(true));
        assert_eq!(i.binding_eval_in_realm(child, "globalThis === window && this === window && (function(){return this})() === window").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn window_proxy_access_hook_observes_calling_and_target_realms() {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        i.binding_set_realm_origin(parent, "https://parent.test").unwrap();
        i.binding_set_realm_origin(child, "https://child.test").unwrap();
        let proxy = i.binding_create_window_proxy(child).unwrap();
        assert!(i.get_prop(&proxy, "Object").is_err());
        assert_eq!(i.window_proxy_prototype(match &proxy { JsValue::Object(o) => o, _ => unreachable!() }).unwrap().unwrap(), JsValue::Null);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observations = seen.clone();
        i.binding_set_window_proxy_access_hook(Rc::new(move |_, access| {
            observations.borrow_mut().push(access); Ok(())
        }));
        assert!(i.get_prop(&proxy, "Object").unwrap().is_object());
        let seen = seen.borrow();
        assert_eq!(seen[0].current, parent);
        assert_eq!(seen[0].target, child);
        assert_eq!(seen[0].target_origin, "https://child.test");
    }

    #[test]
    fn host_dispatch_and_brands_follow_the_callable_realm() {
        let mut i = Interpreter::new();
        i.set_host_native_dispatcher(Rc::new(|_, _, _, _, _| Ok(JsValue::str("parent"))));
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let (function, wrapper) = i.binding_with_realm(child, |i| {
            i.set_host_native_dispatcher(Rc::new(|_, _, _, _, _| Ok(JsValue::str("child"))));
            let function = i.binding_create_native_function(HostFunctionId(7), "host", 0, HostCallPolicy::CallOnly);
            let wrapper = i.binding_wrap_host(HostHandle { arena: 7, slot: 2, generation: 1, brand: 9 }, &i.binding_object_prototype())?;
            Ok((function, wrapper))
        }).unwrap();
        assert_eq!(i.call_value(&function, JsValue::Undefined, vec![]).unwrap(), JsValue::str("child"));
        assert!(i.binding_host_handle(&wrapper, 9).is_some());
        assert!(Interpreter::new().binding_host_handle(&wrapper, 9).is_none());
        let parent_function = i.binding_create_native_function(HostFunctionId(7), "host", 0, HostCallPolicy::CallOnly);
        assert_eq!(i.call_value(&parent_function, JsValue::Undefined, vec![]).unwrap(), JsValue::str("parent"));
    }

    #[test]
    fn cross_realm_await_resumes_with_child_intrinsics_on_both_backends() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new_with_external_scheduler(32).unwrap();
            let parent = i.binding_current_realm();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            i.binding_with_realm(child, |i| i.run_script_with_backend(r#"
                var settle;
                var pending = new Promise(function(resolve) { settle = resolve; });
                var completed;
                async function run() {
                    await pending;
                    completed = [[], new Error('child'), globalThis];
                }
                run();
            "#, backend)).unwrap();
            let settle = i.binding_eval_in_realm(child, "settle").unwrap();
            i.call_value(&settle, JsValue::Undefined, vec![]).unwrap();
            let mut requests: VecDeque<_> = i.take_scheduling_requests().into();
            let mut count = 0;
            while let Some(request) = requests.pop_front() {
                count += 1;
                assert!(count < 16, "async continuation failed to finish");
                if let SchedulingRequest::Microtask { job_token } = request {
                    i.invoke_one_job(job_token).unwrap();
                }
                requests.extend(i.take_scheduling_requests());
            }
            assert_eq!(i.binding_current_realm(), parent);
            assert_eq!(i.binding_eval_in_realm(child,
                "Object.getPrototypeOf(completed[0]) === Array.prototype && Object.getPrototypeOf(completed[1]) === Error.prototype && completed[2] === globalThis"
            ).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn module_graph_uses_one_realm_and_restores_the_global_environment() {
        use crate::module::ModuleLinker;
        let mut i = Interpreter::new_with_external_scheduler(32).unwrap();
        let global = i.global_env.clone();
        let resolver = |_: &str, _: &str| Ok(("dependency".into(),
            "export const array = []; export function constructor() { return Object; } Promise.resolve().then(() => console.log('dependency'));".into()));
        let mut linker = ModuleLinker::new(resolver);
        linker.link("root", "import {array, constructor} from './dependency'; export const shared = array instanceof Array && constructor() === Object; Promise.resolve().then(() => console.log('root')); let moduleOnly = 1;").unwrap();
        let result = linker.evaluate("root", &mut i).unwrap();
        assert_eq!(result["shared"], JsValue::Bool(true));
        assert!(Rc::ptr_eq(&i.global_env.0, &global.0));
        assert_eq!(i.run_script("typeof moduleOnly").unwrap(), JsValue::str("undefined"));
        for request in i.take_scheduling_requests() {
            if let SchedulingRequest::Microtask { job_token } = request { i.invoke_one_job(job_token).unwrap(); }
        }
        assert_eq!(*i.output.borrow(), ["dependency", "root"]);
        let mut failing = ModuleLinker::new(|_: &str, _: &str| Err("not found".into()));
        failing.link("root", "throw new Error('failure')").unwrap();
        assert!(failing.evaluate("root", &mut i).is_err());
        assert!(Rc::ptr_eq(&i.global_env.0, &global.0));
    }

    #[test]
    fn eval_global_instantiation_is_atomic_and_strict_eval_is_isolated() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            try { eval('var mustNotExist; function NaN() {}'); } catch (e) {}
            try { (0,eval)('function alsoAbsent() {} function Infinity() {}'); } catch (e) {}
            (0,eval)('"use strict"; var strictLocal = 88;');
            var stable = 7;
            eval('var stable; var configurableEval = 1;');
            !Object.hasOwn(globalThis, 'mustNotExist') && !Object.hasOwn(globalThis, 'alsoAbsent') &&
            !Object.hasOwn(globalThis, 'strictLocal') && stable === 7 &&
            Object.getOwnPropertyDescriptor(globalThis, 'configurableEval').configurable
        "#).unwrap(), JsValue::Bool(true));
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let foreign_eval = i.binding_eval_in_realm(child, "eval").unwrap();
        i.global_env.declare_var("foreignEval", foreign_eval).unwrap();
        assert_eq!(i.run_script(r#"
            var outer = 'outside';
            (function() { var eval = foreignEval; eval('var outer = "inside"'); })();
            outer
        "#).unwrap(), JsValue::str("outside"));
        assert_eq!(i.binding_eval_in_realm(child, "outer").unwrap(), JsValue::str("inside"));
    }

    #[test]
    fn constructor_completion_errors_use_the_callers_realm() {
        let mut i = Interpreter::new();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        for (source, error_name, construct) in [
            ("(class { constructor() {} })", "TypeError", false),
            ("(class extends Object { constructor() { super(); } })", "TypeError", false),
            ("(class extends Object { constructor() { return null; } })", "TypeError", true),
            ("(class extends Object { constructor() {} })", "ReferenceError", true),
        ] {
            let class = i.binding_eval_in_realm(child, source).unwrap();
            let error = if construct { i.construct(&class, vec![]) } else { i.call_value(&class, JsValue::Undefined, vec![]) }.unwrap_err();
            let error = i.normalize_throw(error);
            // [[Call]] rejects after callee context installation. Derived
            // [[Construct]] completion errors occur after caller restoration.
            let prototype = if construct {
                i.run_script(&format!("{error_name}.prototype")).unwrap()
            } else {
                i.binding_eval_in_realm(child, &format!("{error_name}.prototype")).unwrap()
            };
            assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined, vec![error.0]).unwrap(), prototype, "{source}");
        }
        // Default constructor creation and [[Call]] use the builtin context;
        // these observations preserve the former implicit-class inputs with
        // their genuine callee-realm expectation, separately from explicit F.
        let prototype = i.binding_eval_in_realm(child, "TypeError.prototype").unwrap();
        for source in ["(class {})", "(class extends Object {})"] {
            let class = i.binding_eval_in_realm(child, source).unwrap();
            let error = i.call_value(&class, JsValue::Undefined, vec![]).unwrap_err();
            assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined, vec![error.0]).unwrap(), prototype, "{source}");
        }
    }

    #[test]
    fn dynamic_code_uses_realm_globals_during_module_evaluation() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var untouched = {toString() { throw new Error('coercion'); }};
            eval(untouched) === untouched && eval(42) === 42 && eval() === undefined
        "#).unwrap(), JsValue::Bool(true));
        i.run_script("var visible = 'global';").unwrap();
        let mut linker = crate::module::ModuleLinker::new(|_: &str, _: &str| Err("missing".into()));
        linker.link("root", r#"
            let visible = 'module';
            export const indirect = (0, eval)('visible');
            export const indirectStrict = (0, eval)('"use strict"; visible');
            export const dynamic = Function('return visible')();
            export const receiver = Function('return this')() === globalThis;
            export const direct = eval('visible');
        "#).unwrap();
        let result = linker.evaluate("root", &mut i).unwrap();
        for key in ["indirect", "indirectStrict", "dynamic"] {
            assert_eq!(result[key], JsValue::str("global"), "{key}");
        }
        assert_eq!(result["receiver"], JsValue::Bool(true));
        assert_eq!(result["direct"], JsValue::str("module"));
        let global = i.global_env.clone();
        let active_module = EnvHandle::new(EnvKind::Module, Some(global.clone()));
        active_module.declare_lexical("visible", JsValue::str("temporary module"), true).unwrap();
        i.global_env = active_module.clone();
        assert_eq!(i.binding_eval_in_realm(i.binding_current_realm(), "visible").unwrap(), JsValue::str("global"));
        assert!(i.binding_eval_in_realm(i.binding_current_realm(), "throw 1").is_err());
        assert!(Rc::ptr_eq(&i.global_env.0, &active_module.0));
        i.global_env = global;
    }



    #[test]
    fn strict_tail_class_calls_keep_callee_errors_and_derived_completion_keeps_caller() {
        for ast in [false, true] {
            let mut i = Interpreter::new();
            i.force_ast_callables = ast;
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            let child_type = i.binding_eval_in_realm(child, "TypeError.prototype").unwrap();
            let parent_type = i.run_script("TypeError.prototype").unwrap();
            let parent_reference = i.run_script("ReferenceError.prototype").unwrap();
            for class_source in [
                "(class {})", "(class extends Object {})",
                "(class { constructor() {} })",
                "(class extends Object { constructor() { super(); } })",
            ] {
                let class = i.binding_eval_in_realm(child, class_source).unwrap();
                i.global_env.declare_var("ForeignClass", class).unwrap();
                for tail in [
                    "ForeignClass()", "ForeignClass.call(null)",
                    "ForeignClass.apply(null, [])", "Reflect.apply(ForeignClass, null, [])",
                    "ForeignClass.bind(null)()", "new Proxy(ForeignClass, {})()",
                ] {
                    // The strict frame exits before the forwarded call. Class
                    // rejection still belongs to the admitted callee context,
                    // including native forwarding adapters in the trampoline.
                    let source = format!("(function(){{'use strict';return {tail};}})()");
                    let error = i.run_script(&source).unwrap_err();
                    let error = i.normalize_throw(error);
                    assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined,
                        vec![error.0]).unwrap(), child_type, "AST={ast} {class_source} {tail}");
                }
            }
            for (source, prototype) in [
                ("(class extends Object { constructor() { return null; } })", parent_type),
                ("(class extends Object { constructor() {} })", parent_reference),
            ] {
                let class = i.binding_eval_in_realm(child, source).unwrap();
                i.global_env.declare_var("ForeignClass", class).unwrap();
                let error = i.run_script("(function(){'use strict';return new ForeignClass;})()")
                    .unwrap_err();
                let error = i.normalize_throw(error);
                assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined,
                    vec![error.0]).unwrap(), prototype, "AST={ast} {source}");
            }
        }
    }

}
