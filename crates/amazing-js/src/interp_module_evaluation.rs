// Portions ported from lucid-softworks/lumen (crates/lumen/src/modules.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Module evaluation capabilities use internal reactions, never observable then/species.
impl Interpreter {
    /// Materialize a host-generated error in this realm before caching it.
    /// Ordinary JavaScript thrown values retain their original identity.
    pub fn binding_normalize_throw(&self, error: Throw) -> Throw {
        self.normalize_throw(error)
    }
    /// The legacy synchronous entry point owns a checkpoint only in standalone mode.
    pub(crate) fn module_legacy_checkpoint(&mut self) -> Result<(), Throw> {
        if self.external_scheduling.is_none() { self.drain_microtasks() } else { Ok(()) }
    }

    /// Read an evaluation capability without any observable property access.
    pub fn binding_module_evaluation_state(&self, promise: &JsValue) -> Result<PromiseFate, Throw> {
        let object = promise_object(promise)?;
        let state = match &object.borrow().exotic {
            Exotic::Promise(state) => state.state.clone(),
            _ => unreachable!(),
        };
        Ok(state)
    }
    /// Completion is delivered as an internal reaction in the registering realm.
    /// This never reads `then`/`constructor`/species and does not pump the scheduler.
    pub fn binding_observe_module_evaluation(
        &mut self,
        promise: &JsValue,
        callback: Rc<dyn Fn(&mut Interpreter, Result<(), Throw>) -> Result<(), Throw>>,
    ) -> Result<(), Throw> {
        let realm = self.binding_current_realm();
        let success = callback.clone();
        self.module_observe(
            promise,
            Rc::new(move |i, _| i.binding_with_realm(realm, |i| success(i, Ok(())))),
            Rc::new(move |i, value| {
                i.binding_with_realm(realm, |i| callback(i, Err(Throw(value))))
            }),
        )
    }
    pub(crate) fn module_promise(&self) -> JsValue {
        new_promise(self, PromiseFate::Pending)
    }
    pub(crate) fn module_settle(
        &mut self,
        promise: &JsValue,
        error: Option<Throw>,
    ) -> Result<(), Throw> {
        let object = promise_object(promise)?;
        let fate = match error {
            Some(error) => PromiseFate::Rejected(self.normalize_throw(error).0),
            None => PromiseFate::Fulfilled(JsValue::Undefined),
        };
        settle_promise(&object, fate, self)
    }
    pub(crate) fn module_resolve(
        &mut self,
        promise: &JsValue,
        value: JsValue,
    ) -> Result<(), Throw> {
        vm::async_complete(self, &promise_object(promise)?, value)
    }
    pub(crate) fn module_callback(
        &self,
        callback: Rc<dyn Fn(&mut Interpreter, JsValue) -> Result<(), Throw>>,
    ) -> JsValue {
        let mut home = JsObject::ordinary();
        home.exotic =
            Exotic::ModuleCallback(Rc::new(crate::value::ModuleCallbackState { callback }));
        let value = Self::native("", native_module_callback, &self.function_proto);
        if let JsValue::Object(function) = &value {
            function.borrow_mut().func.as_mut().unwrap().home =
                Some(Obj(Rc::new(RefCell::new(home))));
        }
        value
    }
    pub(crate) fn module_observe(
        &mut self,
        promise: &JsValue,
        fulfilled: Rc<dyn Fn(&mut Interpreter, JsValue) -> Result<(), Throw>>,
        rejected: Rc<dyn Fn(&mut Interpreter, JsValue) -> Result<(), Throw>>,
    ) -> Result<(), Throw> {
        let fulfilled = self.module_callback(fulfilled);
        let rejected = self.module_callback(rejected);
        perform_promise_then(
            self,
            &promise_object(promise)?,
            Some(fulfilled),
            Some(rejected),
            None,
        )
    }
    pub(crate) fn module_start_body(
        &mut self,
        program: &ast::Program,
        env: EnvHandle,
        has_await: bool,
    ) -> Result<Option<JsValue>, Throw> {
        self.with_execution_context(None, |i| i.module_start_body_context(program, env, has_await))
    }

    fn module_start_body_context(&mut self, program: &ast::Program,
        env: EnvHandle, has_await: bool) -> Result<Option<JsValue>, Throw> {
        let previous = std::mem::replace(&mut self.global_env, env.clone());
        let result = if has_await {
            let body = FunctionBody::Block(ast::Block {
                body: program.body.clone(),
                span: program.span,
            });
            self.start_thread_async_function(body, env, true)
                .map(Some)
                .ok_or_else(|| {
                    Throw::type_error("async module execution is unavailable in this host")
                })
        } else {
            // Module instantiation already hoisted this body. Script entry would
            // pump jobs between siblings, violating graph evaluation's boundary.
            let strict = std::mem::replace(&mut self.strict_eval_context, true);
            let completion = self.completion.clone();
            let entry = self.entry_realm;
            self.entry_realm.get_or_insert(self.binding_current_realm());
            let result = self
                .exec_stmts_with_disposal(&program.body, &env)
                .map(|_| None)
                .map_err(|error| self.normalize_throw(error));
            self.strict_eval_context = strict;
            self.completion = completion;
            self.entry_realm = entry;
            result
        };
        self.global_env = previous;
        result
    }
}
fn native_module_callback(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let home = native_data(i)?;
    let state = match &home.borrow().exotic {
        Exotic::ModuleCallback(state) => state.clone(),
        _ => return Err(Throw::type_error("missing module continuation")),
    };
    (state.callback)(i, args.first().cloned().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

#[cfg(test)]
mod module_host_completion_tests {
    use super::*;
    #[test]
    fn public_completion_observer_is_internal_deferred_and_keeps_throw_identity() {
        let mut i = Interpreter::new_with_external_scheduler(64).unwrap();
        let promise = i.module_promise();
        i.global_env.declare_var("p", promise.clone()).unwrap();
        i.run_script("var touched=0;Object.defineProperty(p,'then',{get(){touched++;throw 98}});Object.defineProperty(p,'constructor',{get(){touched++;throw 97}});var marker={};").unwrap();
        let marker = i.global_env.resolve("marker").unwrap();
        let recorded = Rc::new(RefCell::new(Vec::new()));
        let seen = recorded.clone();
        i.binding_observe_module_evaluation(
            &promise,
            Rc::new(move |_, completion| {
                seen.borrow_mut().push(completion);
                Ok(())
            }),
        )
        .unwrap();
        assert!(matches!(
            i.binding_module_evaluation_state(&promise).unwrap(),
            PromiseFate::Pending
        ));
        i.module_settle(&promise, Some(Throw(marker.clone())))
            .unwrap();
        assert!(recorded.borrow().is_empty());
        for request in i.take_scheduling_requests() {
            let SchedulingRequest::Microtask { job_token } = request else {
                panic!("unexpected timer")
            };
            i.invoke_one_job(job_token).unwrap();
        }
        assert_eq!(recorded.borrow()[0].as_ref().unwrap_err().0, marker);
        assert_eq!(i.global_env.resolve("touched"), Some(JsValue::num(0.0)));
    }
}

#[cfg(test)]
mod module_resource_completion_tests {
    use super::*;
    use crate::module::ModuleLinker;
    #[test]
    fn module_await_using_completes_after_async_and_sync_disposers() {
        for symbol in ["asyncDispose", "dispose"] {
            let mut i = Interpreter::new();
            i.run_script("var disposed=false;function complete(){disposed=true;}")
                .unwrap();
            let source = format!(
                "await using resource={{async [Symbol.{symbol}](){{complete();}}}};if(disposed)throw 91;"
            );
            let linker = Rc::new(RefCell::new(ModuleLinker::new(|_: &str, _: &str| {
                Err("unexpected dependency".into())
            })));
            linker.borrow_mut().link("root", &source).unwrap();
            let promise = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
            assert_eq!(i.depth, 0, "{symbol}: leaked call depth");
            assert!(i.frames.is_empty(), "{symbol}: leaked frame");
            i.drain_microtasks().unwrap();
            assert!(
                matches!(
                    i.binding_module_evaluation_state(&promise).unwrap(),
                    PromiseFate::Fulfilled(_)
                ),
                "{symbol}: {:?}, depth {}, frames {}",
                i.binding_module_evaluation_state(&promise),
                i.depth,
                i.frames.len()
            );
            assert_eq!(i.global_env.resolve("disposed"), Some(JsValue::Bool(true)));
            assert!(i.using_stack.is_empty());
        }
    }
}

#[cfg(test)]
mod module_settled_observer_tests {
    use super::*;
    #[test]
    fn already_settled_completion_is_queued_in_registration_realm_and_validates_actual_brand() {
        for reject in [false, true] {
            let mut i = Interpreter::new_with_external_scheduler(64).unwrap();
            let parent = i.binding_current_realm();
            let global = i.binding_create_object(None).unwrap();
            let child = i.binding_create_realm(global).unwrap();
            let promise = i.module_promise();
            i.module_settle(&promise, reject.then(|| Throw(JsValue::num(99.0))))
                .unwrap();
            let order = Rc::new(RefCell::new(Vec::new()));
            let record = order.clone();
            i.binding_with_realm(child, |i| {
                i.binding_observe_module_evaluation(
                    &promise,
                    Rc::new(move |i, result| {
                        assert_eq!(i.binding_current_realm(), child);
                        assert_eq!(result.is_err(), reject);
                        record.borrow_mut().push("observer");
                        Ok(())
                    }),
                )
            })
            .unwrap();
            order.borrow_mut().push("after");
            assert_eq!(&*order.borrow(), &["after"]);
            for request in i.take_scheduling_requests() {
                let SchedulingRequest::Microtask { job_token } = request else {
                    panic!("unexpected timer")
                };
                i.invoke_one_job(job_token).unwrap();
            }
            assert_eq!(i.binding_current_realm(), parent);
            assert_eq!(&*order.borrow(), &["after", "observer"]);
            let ordinary = i.binding_create_object(None).unwrap();
            assert!(i.binding_module_evaluation_state(&ordinary).is_err());
            assert!(
                i.binding_observe_module_evaluation(&ordinary, Rc::new(|_, _| Ok(())))
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod module_pending_disposal_tests {
    use super::*;
    use crate::module::ModuleLinker;
    fn module(i: &mut Interpreter, source: &str) -> JsValue {
        let linker = Rc::new(RefCell::new(ModuleLinker::new(|_: &str, _: &str| {
            Err("unexpected dependency".into())
        })));
        linker.borrow_mut().link("root", source).unwrap();
        ModuleLinker::evaluate_async_shared(&linker, "root", i).unwrap()
    }
    #[test]
    fn pending_disposer_suspends_module_and_rejection_preserves_identity() {
        for reject in [false, true] {
            let mut i = Interpreter::new();
            i.run_script("var release,fail,marker={},called=0;var pending=new Promise((a,b)=>{release=a;fail=b});").unwrap();
            let promise = module(
                &mut i,
                "await using resource={[Symbol.asyncDispose](){called++;return pending;}};",
            );
            assert_eq!(i.global_env.resolve("called"), Some(JsValue::num(1.0)));
            assert!(matches!(
                i.binding_module_evaluation_state(&promise).unwrap(),
                PromiseFate::Pending
            ));
            i.run_script(if reject { "fail(marker)" } else { "release()" })
                .unwrap();
            i.drain_microtasks().unwrap();
            let fate = i.binding_module_evaluation_state(&promise).unwrap();
            if reject {
                assert!(
                    matches!(fate,PromiseFate::Rejected(value) if value==i.global_env.resolve("marker").unwrap())
                );
            } else {
                assert!(matches!(fate, PromiseFate::Fulfilled(_)));
            }
            assert_eq!(i.depth, 0);
            assert!(i.frames.is_empty());
            assert!(i.using_stack.is_empty());
        }
    }
    #[test]
    fn primitive_async_disposer_result_still_has_an_await_microtask_boundary() {
        let mut i = Interpreter::new();
        i.run_script("var disposed=false").unwrap();
        let promise = module(
            &mut i,
            "await using resource={[Symbol.asyncDispose](){disposed=true;return 7;}};",
        );
        assert_eq!(i.global_env.resolve("disposed"), Some(JsValue::Bool(true)));
        assert!(matches!(
            i.binding_module_evaluation_state(&promise).unwrap(),
            PromiseFate::Pending
        ));
        i.drain_microtasks().unwrap();
        assert!(matches!(
            i.binding_module_evaluation_state(&promise).unwrap(),
            PromiseFate::Fulfilled(_)
        ));
    }
}
