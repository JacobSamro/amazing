// Portions ported from lucid-softworks/lumen (crates/lumen/src/modules.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Module evaluation capabilities use internal reactions, never observable then/species.
impl Interpreter {
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
