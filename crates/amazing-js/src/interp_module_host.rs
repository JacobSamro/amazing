/// The embedding's loader hook for dynamic `import(specifier)` / `import.source(specifier)`
/// (13.3.10 EvaluateImportCall). `amazing-js` owns no module registry, resolver policy or fetch
/// capability of its own -- `module.rs`'s `ModuleLinker`/`ModuleResolver` are a standalone,
/// synchronous graph a *host* builds and owns (see its own doc comment), and a real embedding
/// (HTML script execution) needs `import()` calls made from arbitrary running script to resolve
/// through that SAME graph, not start an independent one per call site, so a specifier already
/// reachable from a `<script type=module>` tree resolves to the identical module record/namespace
/// rather than a duplicate evaluation. This hook is therefore how the driver plugs itself in:
/// `referrer` is the importing module's own identifier (the value `module.rs` stores under
/// `"\0module.id"` in that module's environment; empty for a classic script or top-level eval),
/// and a successful call returns the resolved module's namespace object.
///
/// The hook runs in a queued job after the importing expression returns. Its
/// own operation is synchronous; fetching or parsing must finish before it
/// returns. A failure rejects the import promise rather than the caller's frame.
pub type ModuleLoaderHook = dyn Fn(&mut Interpreter, &str, &str) -> Result<JsValue, String>;

/// A loader that preserves parser, resolution and evaluation error objects.
/// Hosts with a shared ModuleLinker should use evaluate_shared so no registry
/// borrow survives JavaScript execution or a nested import job.
pub type TypedModuleLoaderHook = dyn Fn(&mut Interpreter, &str, &str) -> Result<JsValue, Throw>;

struct ModuleImportJob {
    hook: Rc<TypedModuleLoaderHook>,
    specifier: String,
    referrer: String,
    promise: Obj,
    realm: RealmHandle,
}

#[cfg(test)]
mod module_import_job_tests {
    use super::*;

    #[test]
    fn classic_source_context_survives_later_scripts_and_delayed_closures() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let origins = Rc::new(RefCell::new(Vec::new()));
            let recorded = origins.clone();
            i.binding_set_typed_module_loader_hook(Rc::new(move |_, specifier, referrer| {
                recorded.borrow_mut().push((specifier.to_string(), referrer.to_string()));
                Ok(JsValue::Undefined)
            }));
            i.run_script_with_source_context_and_backend(
                "let shared=41; var later=()=>import('./first.js'); function nested(){return ()=>import('./nested.js');} var delayed=nested();",
                "https://example.test/a/source.js", backend,
            ).unwrap();
            i.run_script_with_source_context_and_backend(
                "var other=()=>import('./second.js'); shared++;",
                "https://example.test/b/source.js", backend,
            ).unwrap();
            i.run_script("later();other();delayed();import('./unlocated.js');").unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(&*origins.borrow(), &[
                ("./first.js".into(), "https://example.test/a/source.js".into()),
                ("./second.js".into(), "https://example.test/b/source.js".into()),
                ("./nested.js".into(), "https://example.test/a/source.js".into()),
                ("./unlocated.js".into(), String::new()),
            ], "{backend:?}");
            assert_eq!(i.run_script("shared").unwrap(), JsValue::Num(42.0));
            assert!(i.run_script_with_source_context("throw 3", "failed.js").is_err());
            assert!(i.global_env.source_context().is_none());
        }
    }

    fn take_job(i: &mut Interpreter) -> SchedulingToken {
        let requests = i.take_scheduling_requests();
        assert_eq!(requests.len(), 1);
        match requests[0] { SchedulingRequest::Microtask { job_token } => job_token, _ => panic!("expected a microtask") }
    }

    #[test]
    fn loader_is_deferred_and_argument_attribute_order_is_preserved() {
        let mut i = Interpreter::new_with_external_scheduler(64).unwrap();
        let calls = Rc::new(Cell::new(0));
        let loader_calls = calls.clone();
        i.binding_set_typed_module_loader_hook(Rc::new(move |i, specifier, referrer| {
            loader_calls.set(loader_calls.get()+1);
            assert_eq!(specifier, "dep");
            assert_eq!(referrer, "");
            i.run_script("order.push('load');({x:42})")
        }));
        i.run_script(r#"
            var order=[];
            function options(){order.push('options');return {get with(){order.push('attrs');return {};}};}
            var pending=import({toString(){order.push('coerce');return 'dep';}},options());
            pending.then(ns=>order.push('then:'+ns.x));order.push('after');
        "#).unwrap();
        assert_eq!(calls.get(), 0);
        assert_eq!(i.run_script("order.join(',')").unwrap(), JsValue::str("options,coerce,attrs,after"));
        let load = take_job(&mut i);
        i.invoke_one_job(load).unwrap();
        assert_eq!(calls.get(), 1);
        let reaction = take_job(&mut i);
        i.invoke_one_job(reaction).unwrap();
        assert_eq!(i.run_script("order.join(',')").unwrap(), JsValue::str("options,coerce,attrs,after,load,then:42"));
    }

    #[test]
    fn loader_errors_and_namespace_then_exports_resolve_through_promise_jobs() {
        let mut i = Interpreter::new();
        i.binding_set_typed_module_loader_hook(Rc::new(|i, specifier, _| {
            if specifier == "bad" { return Err(Throw::syntax_error("bad module source")); }
            i.run_script("({then(resolve){resolve(17);}})")
        }));
        i.run_script("var resolved,failed,invalid;import('then').then(v=>resolved=v);import('bad').catch(e=>failed=e instanceof SyntaxError);import('x',{with:{type:42}}).catch(e=>invalid=e instanceof TypeError);").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("resolved===17 && failed && invalid").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn loader_and_reactions_run_in_the_importing_realm() {
        let mut i = Interpreter::new_with_external_scheduler(64).unwrap();
        let parent = i.binding_current_realm();
        let global = i.binding_create_object(None).unwrap();
        let child = i.binding_create_realm(global).unwrap();
        i.binding_set_typed_module_loader_hook(Rc::new(move |i, _, _| {
            assert_eq!(i.binding_current_realm(), child);
            i.run_script("({marker:'child'})")
        }));
        i.binding_eval_in_realm(child, "var seen;import('x').then(ns=>seen=ns.marker);").unwrap();
        let load = take_job(&mut i);
        i.invoke_one_job(load).unwrap();
        assert_eq!(i.binding_current_realm(), parent);
        let reaction = take_job(&mut i);
        i.invoke_one_job(reaction).unwrap();
        assert_eq!(i.binding_current_realm(), parent);
        assert_eq!(i.binding_eval_in_realm(child, "seen").unwrap(), JsValue::str("child"));
    }
}

impl Interpreter {
    /// Execute a classic script whose closures retain its originating URL for import().
    /// This changes handle metadata, preserving the realm's live global binding frame.
    pub fn run_script_with_source_context(&mut self, source: &str, referrer: &str) -> Result<JsValue, Throw> {
        let previous = self.global_env.clone();
        self.global_env = previous.with_source_context(referrer);
        let result = self.run_script(source);
        self.global_env = previous;
        result
    }

    pub fn run_script_with_source_context_and_backend(
        &mut self, source: &str, referrer: &str, backend: ExecutionBackend,
    ) -> Result<JsValue, Throw> {
        let previous = self.global_env.clone();
        self.global_env = previous.with_source_context(referrer);
        let result = self.run_script_with_backend(source, backend);
        self.global_env = previous;
        result
    }

    /// Install the driver's loader. `crates/amazing`'s HTML/script-execution driver is the
    /// expected caller; see `questions.md`'s JS-18 item-2 entry for the wiring it still needs on
    /// that side (an `amazing-html`/`dom-bindings`-owned module registry keyed the same way this
    /// hook's `referrer` is).
    pub fn binding_set_module_loader_hook(&mut self, hook: Rc<ModuleLoaderHook>) {
        self.module_loader = Some(Rc::new(move |i, specifier, referrer| {
            hook(i, specifier, referrer).map_err(Throw::type_error)
        }));
    }

    pub fn binding_set_typed_module_loader_hook(&mut self, hook: Rc<TypedModuleLoaderHook>) {
        self.module_loader = Some(hook);
    }

    /// Argument evaluation may throw synchronously; subsequent conversion,
    /// attribute validation and host loading reject the returned promise.
    fn dynamic_import(&mut self, args: &[ast::Expr], env: &EnvHandle) -> Result<JsValue, Throw> {
        let specifier = args.first().ok_or_else(|| Throw::type_error("import() requires a specifier"))?;
        let value = self.eval_expr(specifier, env)?;
        let options = match args.get(1) {
            Some(options) => self.eval_expr(options, env)?,
            None => JsValue::Undefined,
        };
        let referrer = env.resolve("\0module.id");
        let referrer = match &referrer {
            Some(JsValue::Str(s)) => s.to_string(),
            _ => env.source_context().unwrap_or_default().to_string(),
        };
        let specifier_string = match self.to_js_string(&value) {
            Ok(s) => s.to_string(),
            Err(thrown) => return Ok(new_promise(self, PromiseFate::Rejected(self.normalize_throw(thrown).0))),
        };
        if let Err(error) = self.validate_import_options(&options) {
            return Ok(new_promise(self, PromiseFate::Rejected(self.normalize_throw(error).0)));
        }
        let Some(hook) = self.module_loader.clone() else {
            return Ok(new_promise(self, PromiseFate::Rejected(
                self.normalize_throw(Throw::type_error("dynamic import is not supported by this host")).0,
            )));
        };
        let promise = new_promise(self, PromiseFate::Pending);
        let JsValue::Object(target) = &promise else { unreachable!() };
        self.enqueue_job(Job::ModuleImport(ModuleImportJob {
            hook, specifier: specifier_string, referrer, promise: target.clone(),
            realm: self.binding_current_realm(),
        }))?;
        Ok(promise)
    }

    fn validate_import_options(&mut self, options: &JsValue) -> Result<(), Throw> {
        if options.is_undefined() { return Ok(()); }
        if !options.is_object() { return Err(Throw::type_error("import options must be an object")); }
        let attributes = self.get_prop(options, "with")?;
        if attributes.is_undefined() { return Ok(()); }
        let JsValue::Object(object) = &attributes else {
            return Err(Throw::type_error("import attributes must be an object"));
        };
        let mut nonempty = false;
        for key in self.own_property_keys(&attributes)? {
            if key.as_string().is_none() { continue; }
            if !self.default_get_own_property(object, &key)?.is_some_and(|p| p.enumerable) { continue; }
            let value = self.get_prop(&attributes, key)?;
            if !matches!(value, JsValue::Str(_)) { return Err(Throw::type_error("import attribute values must be strings")); }
            nonempty = true;
        }
        if nonempty { return Err(Throw::type_error("host does not support import attributes")); }
        Ok(())
    }

    fn run_module_import_job(&mut self, job: ModuleImportJob) -> Result<(), Throw> {
        self.binding_with_realm(job.realm, |i| {
            match (job.hook)(i, &job.specifier, &job.referrer) {
                Ok(namespace) => vm::async_complete(i, &job.promise, namespace),
                Err(error) => settle_promise(&job.promise, PromiseFate::Rejected(i.normalize_throw(error).0), i),
            }
        })
    }
}
