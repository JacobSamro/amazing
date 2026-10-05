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

/// Phase-aware hosts may return a pending Promise. The import capability adopts
/// it through the ordinary Promise resolution procedure in the importing realm.
pub type PhaseModuleLoaderHook = dyn Fn(&mut Interpreter, &crate::module::ModuleRequest, &str) -> Result<JsValue, Throw>;

struct ModuleImportJob {
    hook: Option<Rc<TypedModuleLoaderHook>>,
    phase_hook: Option<Rc<PhaseModuleLoaderHook>>,
    request: crate::module::ModuleRequest,
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
        self.run_script_utf16_with_source_context(&source.encode_utf16().collect::<Vec<_>>(), referrer)
    }

    pub fn run_script_utf16_with_source_context(&mut self, source: &[u16], referrer: &str) -> Result<JsValue, Throw> {
        let previous = self.global_env.clone();
        self.global_env = previous.with_source_context(referrer);
        let result = self.run_script_utf16(source);
        self.global_env = previous;
        result
    }

    pub fn run_script_with_source_context_and_backend(
        &mut self, source: &str, referrer: &str, backend: ExecutionBackend,
    ) -> Result<JsValue, Throw> {
        self.run_script_utf16_with_source_context_and_backend(&source.encode_utf16().collect::<Vec<_>>(), referrer, backend)
    }

    pub fn run_script_utf16_with_source_context_and_backend(
        &mut self, source: &[u16], referrer: &str, backend: ExecutionBackend,
    ) -> Result<JsValue, Throw> {
        let previous = self.global_env.clone();
        self.global_env = previous.with_source_context(referrer);
        let result = self.run_script_utf16_with_backend(source, backend);
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

    pub fn binding_set_phase_module_loader_hook(&mut self, hook: Rc<PhaseModuleLoaderHook>) {
        self.phase_module_loader = Some(hook);
    }

    /// Argument evaluation may throw synchronously; subsequent conversion,
    /// attribute validation and host loading reject the returned promise.
    fn dynamic_import(&mut self, args: &[ast::Expr], env: &EnvHandle) -> Result<JsValue, Throw> {
        self.dynamic_import_phase(args,env,crate::module::ImportPhase::Evaluation)
    }

    fn dynamic_import_phase(&mut self,args:&[ast::Expr],env:&EnvHandle,phase:crate::module::ImportPhase) -> Result<JsValue,Throw> {
        let specifier = args.first().ok_or_else(|| Throw::type_error("import() requires a specifier"))?;
        let value = self.eval_expr(specifier, env)?;
        let options = match args.get(1) {
            Some(options) => self.eval_expr(options, env)?,
            None => JsValue::Undefined,
        };
        self.dynamic_import_values(value, options, env, phase)
    }

    // AST and VM argument evaluation share the complete post-evaluation
    // ImportCall algorithm. Conversion and host errors reject its promise;
    // evaluating either argument remains the caller's synchronous boundary.
    fn dynamic_import_values(&mut self,value:JsValue,options:JsValue,env:&EnvHandle,phase:crate::module::ImportPhase) -> Result<JsValue,Throw> {
        let referrer = env.resolve("\0module.id");
        let referrer = match &referrer {
            Some(JsValue::Str(s)) => s.to_string(),
            _ => env.source_context().unwrap_or_default().to_string(),
        };
        let specifier_string = match self.to_js_string(&value) {
            Ok(s) => s,
            Err(thrown) => return Ok(new_promise(self, PromiseFate::Rejected(self.normalize_throw(thrown).0))),
        };
        let attributes=match self.validate_import_options(&options) {
            Ok(attributes)=>attributes,
            Err(error)=>return Ok(new_promise(self,PromiseFate::Rejected(self.normalize_throw(error).0))),
        };
        let request=crate::module::ModuleRequest::new(specifier_string,phase,attributes);
        let hook=self.module_loader.clone();let phase_hook=self.phase_module_loader.clone();
        if phase_hook.is_none() && (hook.is_none() || phase != crate::module::ImportPhase::Evaluation || !request.attributes.is_empty()) {
            return Ok(new_promise(self, PromiseFate::Rejected(
                self.normalize_throw(Throw::type_error("host does not support this module request")).0,
            )));
        }
        let promise = new_promise(self, PromiseFate::Pending);
        let JsValue::Object(target) = &promise else { unreachable!() };
        self.enqueue_job(Job::ModuleImport(ModuleImportJob {
            hook, phase_hook, request, referrer, promise: target.clone(),
            realm: self.binding_current_realm(),
        }))?;
        Ok(promise)
    }

    fn validate_import_options(&mut self, options: &JsValue) -> Result<Vec<(crate::value::JsString,crate::value::JsString)>, Throw> {
        if options.is_undefined() { return Ok(Vec::new()); }
        if !options.is_object() { return Err(Throw::type_error("import options must be an object")); }
        let attributes = self.get_prop(options, "with")?;
        if attributes.is_undefined() { return Ok(Vec::new()); }
        let JsValue::Object(object) = &attributes else {
            return Err(Throw::type_error("import attributes must be an object"));
        };
        let mut result = Vec::new();
        for key in self.own_property_keys(&attributes)? {
            if key.as_string().is_none() { continue; }
            if !self.default_get_own_property(object, &key)?.is_some_and(|p| p.enumerable) { continue; }
            let value = self.get_prop(&attributes, key.clone())?;
            let JsValue::Str(value)=value else { return Err(Throw::type_error("import attribute values must be strings")); };
            result.push((key.as_string().unwrap().clone(),value));
        }
        Ok(result)
    }

    fn run_module_import_job(&mut self, job: ModuleImportJob) -> Result<(), Throw> {
        self.binding_with_realm(job.realm, |i| {
            let result=match job.phase_hook {
                Some(hook)=>hook(i,&job.request,&job.referrer),
                None=>job.hook.unwrap()(i,&job.request.specifier.to_utf8_lossy(),&job.referrer),
            };
            match result {
                Ok(namespace) => vm::async_complete(i, &job.promise, namespace),
                Err(error) => settle_promise(&job.promise, PromiseFate::Rejected(i.normalize_throw(error).0), i),
            }
        })
    }
}

#[cfg(test)]
mod phase_import_tests {
    use super::*;
    #[test]
    fn every_phase_evaluates_arguments_synchronously_and_rejects_conversion_attributes_in_order() {
        for phase in ["source","defer"] {
            for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
                let mut i=Interpreter::new();
                i.run_script_with_backend(&format!("var log=[],marker={{}},caught=false,rejected;try{{import.{phase}('x',(()=>{{throw marker}})())}}catch(e){{caught=e===marker}};import.{phase}({{toString(){{log.push('string');return 'x'}}}},{{get with(){{log.push('with');return {{get a(){{log.push('a');return 'v'}},get b(){{log.push('b');throw marker}}}}}}}}).catch(e=>rejected=e===marker);"),backend).unwrap();
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("caught&&rejected&&log.join(',')==='string,with,a,b'").unwrap(),JsValue::Bool(true));
            }
        }
    }
    #[test]
    fn phase_hook_receives_lossless_sorted_attributes_after_all_validation_and_adopts_pending_result() {
        let mut i=Interpreter::new_with_external_scheduler(64).unwrap();
        let calls=Rc::new(RefCell::new(Vec::new()));let seen=calls.clone();
        i.binding_set_phase_module_loader_hook(Rc::new(move |i,request,_| {
            seen.borrow_mut().push(request.clone());
            i.run_script("Promise.resolve({value:7})")
        }));
        i.run_script("var done;import.defer('\\uD800',{with:{z:'last',a:'first'}}).then(ns=>done=ns.value)").unwrap();
        assert!(calls.borrow().is_empty());
        loop {let requests=i.take_scheduling_requests();if requests.is_empty(){break;}
            for request in requests {let SchedulingRequest::Microtask {job_token}=request else {panic!("unexpected timer")};
                i.invoke_one_job(job_token).unwrap();}}
        assert_eq!(i.run_script("done").unwrap(),JsValue::num(7.0));
        let seen=calls.borrow();assert_eq!(seen[0].phase,crate::module::ImportPhase::Defer);
        assert_eq!(seen[0].specifier.units(),&[0xd800]);assert_eq!(seen[0].attributes[0].0.to_utf8_lossy(),"a");
    }
}

#[cfg(test)]
#[path = "script_utf16_entry_tests.rs"]
mod script_utf16_entry_tests;
