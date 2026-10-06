// Embedding access to existing allocation provenance. This is distinct from
// ECMAScript GetFunctionRealm, which follows bound/Proxy invocation targets.
impl Interpreter {
    /// Return the JavaScript object's own associated creation realm.
    ///
    /// Reads engine allocation metadata without invoking authored properties,
    /// traversing prototypes, or following bound/Proxy targets. In particular,
    /// revoking a Proxy does not discard the Proxy object's creation realm.
    /// Objects without registered allocation provenance are rejected rather
    /// than assigned the current realm. Embedding constructors should be
    /// created through the existing realm-aware binding allocation APIs.
    pub fn binding_object_creation_realm(&self, value: &JsValue) -> Result<RealmHandle, Throw> {
        let JsValue::Object(object) = value else {
            return Err(Throw::type_error("creation realm requires an object"));
        };
        let realm = object.borrow().realm
            .ok_or_else(|| Throw::type_error("object has no associated creation realm"))?;
        let handle = RealmHandle { agent: self.agent_id, realm };
        self.check_realm(handle)?;
        Ok(handle)
    }
}

#[cfg(test)]
mod object_creation_realm_tests {
    use super::*;

    fn runtime(backend: ExecutionBackend) -> (Interpreter, RealmHandle, RealmHandle) {
        let mut i = Interpreter::new();
        i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
        let parent = i.binding_current_realm();
        let global = i.binding_create_object(None).unwrap();
        let child = i.binding_create_realm(global).unwrap();
        (i, parent, child)
    }

    fn evaluate(i: &mut Interpreter, realm: RealmHandle, backend: ExecutionBackend, source: &str) -> JsValue {
        i.binding_with_realm(realm, |i| i.run_script_with_backend(source, backend)).unwrap()
    }

    #[test]
    fn object_realm_uses_actual_ast_and_vm_callable_allocation() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let (mut i, parent, child) = runtime(backend);
            for source in ["(function ordinary(){return 1;})", "(()=>2)",
                "(class Explicit{constructor(){this.x=1;}})"] {
                let value = evaluate(&mut i, child, backend, source);
                assert_eq!(i.binding_object_creation_realm(&value).unwrap(), child);
                let JsValue::Object(object) = &value else { panic!("callable"); };
                let object = object.borrow();
                let function = object.func.as_ref().unwrap();
                match backend {
                    ExecutionBackend::TreeWalk => assert!(matches!(function.body, FunctionBody::Block(_) | FunctionBody::Expr(_))),
                    ExecutionBackend::Bytecode => assert!(matches!(function.body, FunctionBody::Bytecode(_))),
                }
            }
            assert_eq!(i.binding_current_realm(), parent);
        }
    }

    #[test]
    fn object_realm_covers_default_classes_methods_async_generators_and_native_entries() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let (mut i, parent, child) = runtime(backend);
            for source in ["(class Default{})", "(function* generator(){yield 1;})",
                "(async function asyncFunction(){return 1;})", "(async function* asyncGenerator(){yield 1;})",
                "({method(){return 1;}}).method", "Object", "Function", "Proxy"] {
                let value = evaluate(&mut i, child, backend, source);
                assert_eq!(i.binding_object_creation_realm(&value).unwrap(), child, "{source}");
            }
            let native = i.binding_with_realm(child, |i| Ok(i.binding_create_native_function(
                HostFunctionId(931), "NativeConstructor", 0, HostCallPolicy::ConstructOnlyAllocates))).unwrap();
            assert_eq!(i.binding_object_creation_realm(&native).unwrap(), child);
            assert_eq!(i.binding_current_realm(), parent);
        }
    }

    #[test]
    fn object_realm_distinguishes_bound_creation_from_target_invocation_realm() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let (mut i, parent, child) = runtime(backend);
            let foreign = evaluate(&mut i, child, backend, "(function Foreign(){})");
            i.global_env.declare_var("foreign", foreign.clone()).unwrap();
            let bound = i.run_script_with_backend("Function.prototype.bind.call(foreign,null)", backend).unwrap();
            assert_eq!(i.binding_object_creation_realm(&bound).unwrap(), parent);
            assert_eq!(i.binding_function_realm(&bound).unwrap(), child);
            let borrowed = i.binding_eval_in_realm(child, "Function.prototype.bind").unwrap();
            i.global_env.declare_var("foreignBind", borrowed).unwrap();
            let local = i.run_script_with_backend("(function Local(){})", backend).unwrap();
            i.global_env.declare_var("local", local).unwrap();
            let bound = i.run_script_with_backend("foreignBind.call(local,null)", backend).unwrap();
            assert_eq!(i.binding_object_creation_realm(&bound).unwrap(), child);
            assert_eq!(i.binding_function_realm(&bound).unwrap(), parent);
        }
    }

    #[test]
    fn object_realm_proxy_revocation_and_traps_do_not_change_creation_identity() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let (mut i, parent, child) = runtime(backend);
            let foreign = evaluate(&mut i, child, backend, "(class Foreign{})");
            i.global_env.declare_var("foreign", foreign).unwrap();
            let proxy = i.run_script_with_backend(r#"
                var trapCount=0;
                var record=Proxy.revocable(foreign,{get(){trapCount++;throw 'get';},
                    getPrototypeOf(){trapCount++;throw 'proto';}});
                record.proxy;
            "#, backend).unwrap();
            assert_eq!(i.binding_object_creation_realm(&proxy).unwrap(), parent);
            assert_eq!(i.binding_function_realm(&proxy).unwrap(), child);
            i.run_script("record.revoke()").unwrap();
            assert_eq!(i.binding_object_creation_realm(&proxy).unwrap(), parent);
            assert!(i.binding_function_realm(&proxy).is_err());
            assert_eq!(i.run_script("trapCount").unwrap(), JsValue::num(0.0));
            let local = i.run_script_with_backend("(class Local{})", backend).unwrap();
            let global = i.binding_realm_global(child).unwrap();
            i.binding_set_property(&global, "local", local).unwrap();
            let child_proxy = evaluate(&mut i, child, backend,
                "var childRecord=Proxy.revocable(local,{});childRecord.revoke();childRecord.proxy");
            assert_eq!(i.binding_object_creation_realm(&child_proxy).unwrap(), child);
        }
    }

    #[test]
    fn object_realm_ignores_authored_metadata_and_later_prototype_mutation() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let (mut i, parent, child) = runtime(backend);
            let value = evaluate(&mut i, child, backend, "(class Captured{})");
            i.global_env.declare_var("captured", value.clone()).unwrap();
            i.run_script(r#"
                Object.defineProperty(captured,'realm',{get(){throw 'realm';}});
                Object.defineProperty(captured,'constructor',{get(){throw 'constructor';}});
                Object.setPrototypeOf(captured,null);
            "#).unwrap();
            assert_eq!(i.binding_object_creation_realm(&value).unwrap(), child);
            assert_eq!(i.binding_current_realm(), parent);
        }
    }

    #[test]
    fn object_realm_rejects_missing_or_foreign_provenance_without_current_fallback() {
        let mut i = Interpreter::new();
        for value in [JsValue::Undefined, JsValue::Null, JsValue::num(1.0), JsValue::str("x")] {
            assert!(i.binding_object_creation_realm(&value).is_err());
        }
        let unstamped = JsValue::Object(Obj(Rc::new(RefCell::new(JsObject::ordinary()))));
        assert!(i.binding_object_creation_realm(&unstamped).is_err());
        let mut other = Interpreter::new();
        let foreign = other.run_script("(function ForeignAgent(){})").unwrap();
        assert!(i.binding_object_creation_realm(&foreign).is_err());
        let stamped = i.binding_object_prototype();
        assert_eq!(i.binding_object_creation_realm(&stamped).unwrap(), i.binding_current_realm());
    }
}
