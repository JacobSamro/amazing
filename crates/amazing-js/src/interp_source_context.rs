fn environment_source_context(env: &EnvHandle) -> Option<Rc<str>> {
    let mut current = env.clone();
    loop {
        if current.kind() == EnvKind::Module {
            if let Some(JsValue::Str(id)) = current.resolve("\0module.id") {
                return Some(Rc::from(id.to_utf8_lossy()));
            }
        }
        let parent = current.0.borrow().parent.clone();
        match parent { Some(parent) => current = parent, None => break }
    }
    env.source_context().map(Rc::from)
}

impl Interpreter {
    fn active_source_context(&self) -> Option<Rc<str>> {
        // Builtins have no ScriptOrModule. Interpreted callbacks and resumed
        // VM frames retain their original function environment's locator.
        for frame in self.frames.iter().rev() {
            let Some(object) = &frame.own_function else { continue; };
            let object = object.borrow();
            let Some(function) = &object.func else { continue; };
            if matches!(&function.body, FunctionBody::Native(_) | FunctionBody::HostNative(..)) { continue; }
            if let Some(source) = environment_source_context(&function.env) { return Some(source); }
        }
        match &self.active_source_override {
            Some(source) => source.clone(),
            None => environment_source_context(&self.global_env),
        }
    }

    fn global_with_active_source_context(&self) -> EnvHandle {
        let global = self.global_env.global_scope();
        match self.active_source_context() {
            Some(source) => global.with_source_context(&source),
            // Explicitly unlocated compilation must not copy an ambient
            // checkpoint script's sidecar from this same global frame.
            None => global.with_source_context(""),
        }
    }
}

#[cfg(test)]
mod dynamic_source_context_tests {
    use super::*;

    fn record_imports(i: &mut Interpreter) -> Rc<RefCell<Vec<(String, String)>>> {
        let origins = Rc::new(RefCell::new(Vec::new()));
        let recorded = origins.clone();
        i.binding_set_typed_module_loader_hook(Rc::new(move |_, specifier, referrer| {
            recorded.borrow_mut().push((specifier.to_string(), referrer.to_string()));
            Ok(JsValue::Undefined)
        }));
        origins
    }

    #[test]
    fn dynamic_function_and_eval_capture_active_script_without_local_bindings() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let origins = record_imports(&mut i);
            i.run_script_with_source_context_and_backend(r#"
                var createLater=()=>[
                    Function("return import('./ordinary.js')"),
                    Object.getPrototypeOf(async function(){}).constructor("return import('./async.js')"),
                    Object.getPrototypeOf(function*(){}).constructor("yield import('./generator.js')"),
                    Object.getPrototypeOf(async function*(){}).constructor("yield import('./async-generator.js')")
                ];
                function* createThroughResume(){yield Function("return import('./resumed.js')")}
                var resumedFactory=createThroughResume();
                var indirectLater=()=>{let privateLocal=7;return (0,eval)("typeof privateLocal==='undefined' && import('./indirect.js')")};
                var resolveLater;
                new Promise(resolve=>resolveLater=resolve).then(eval);
            "#, "https://example.test/a/origin.js", backend).unwrap();
            // Keep this fixture on the AST path, then invoke from either backend.
            i.force_ast_callables = true;
            i.run_script_with_source_context_and_backend(r#"
                function* createThroughAstResume(){with({}){};yield Function("return import('./ast-resumed.js')")}
                var astResumedFactory=createThroughAstResume();
            "#, "https://example.test/a/origin.js", ExecutionBackend::TreeWalk).unwrap();
            i.force_ast_callables = false;
            i.run_script_with_source_context_and_backend(r#"
                var created=createLater();
                created[0]();created[1]();created[2]().next();created[3]().next();
                resumedFactory.next().value();astResumedFactory.next().value();indirectLater();
                resolveLater("import('./promise-eval.js')");
            "#, "https://example.test/b/caller.js", backend).unwrap();
            i.drain_microtasks().unwrap();
            let mut actual = origins.borrow().clone();
            actual.sort();
            let mut expected: Vec<_> = ["ordinary", "async", "generator", "async-generator",
                "resumed", "ast-resumed", "indirect", "promise-eval"].into_iter().map(|name|
                    (format!("./{name}.js"), "https://example.test/a/origin.js".to_string())).collect();
            expected.sort();
            assert_eq!(actual, expected, "{backend:?}");
            assert!(i.global_env.source_context().is_none());
        }
    }

    #[test]
    fn module_and_foreign_dynamic_functions_keep_global_realm_bindings() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let origins = record_imports(&mut i);
            let child = i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();
            let foreign = i.binding_eval_in_realm(child, "globalThis.marker='foreign';Function").unwrap();
            i.global_env.declare_var("ForeignFunction", foreign).unwrap();
            i.run_script_with_source_context_and_backend(
                "var foreignLater=ForeignFunction(\"import('./foreign.js');return globalThis.marker\");",
                "https://example.test/creator.js", backend,
            ).unwrap();
            assert_eq!(i.run_script("foreignLater()").unwrap(), JsValue::str("foreign"));
            let global = i.global_env.clone();
            let module = EnvHandle::new(EnvKind::Module, Some(global.clone()));
            module.declare_var("\0module.id", JsValue::str("https://example.test/module.js")).unwrap();
            module.declare_lexical("moduleLocal", JsValue::num(7.0), true).unwrap();
            i.global_env = module;
            assert_eq!(i.run_script_with_backend(
                "Function(\"import('./module-created.js');return typeof moduleLocal\")()", backend,
            ).unwrap(), JsValue::str("undefined"));
            i.global_env = global;
            i.drain_microtasks().unwrap();
            assert_eq!(&*origins.borrow(), &[
                ("./foreign.js".to_string(), "https://example.test/creator.js".to_string()),
                ("./module-created.js".to_string(), "https://example.test/module.js".to_string()),
            ], "{backend:?}");
        }
    }

    #[test]
    fn unlocated_builtin_job_does_not_inherit_the_checkpoint_script() {
        let mut i = Interpreter::new();
        let origins = record_imports(&mut i);
        i.run_script("var finish;new Promise(resolve=>finish=resolve).then(eval);").unwrap();
        i.run_script_with_source_context("finish(\"import('./unlocated.js')\")", "https://example.test/checkpoint.js").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(&*origins.borrow(), &[("./unlocated.js".to_string(), String::new())]);
        assert!(i.active_source_override.is_none());
        assert!(i.global_env.source_context().is_none());
    }
}
