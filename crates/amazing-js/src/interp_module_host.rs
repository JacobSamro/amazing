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
/// Synchronous like the rest of this engine's module support (see `module.rs`): there is no true
/// concurrent I/O modeled underneath, so any fetching/parsing the driver still needs to do must
/// already be complete by the time this returns. Either outcome -- success or the `Err` string --
/// is turned into an already-settled Promise by `dynamic_import` below; a loader failure must
/// reject the returned promise, never throw synchronously into the caller's frame (13.3.10.1
/// steps 3-7 run resolution and FinishDynamicImport entirely through promise jobs).
pub type ModuleLoaderHook = dyn Fn(&mut Interpreter, &str, &str) -> Result<JsValue, String>;

impl Interpreter {
    /// Install the driver's loader. `crates/amazing`'s HTML/script-execution driver is the
    /// expected caller; see `questions.md`'s JS-18 item-2 entry for the wiring it still needs on
    /// that side (an `amazing-html`/`dom-bindings`-owned module registry keyed the same way this
    /// hook's `referrer` is).
    pub fn binding_set_module_loader_hook(&mut self, hook: Rc<ModuleLoaderHook>) {
        self.module_loader = Some(hook);
    }

    /// `ExprKind::DynamicImport`'s evaluation: never fails synchronously (see `ModuleLoaderHook`'s
    /// doc comment) -- a missing host hook, a `ToString(specifier)` failure, or the hook's own
    /// `Err` are all reported as a rejected promise, matching `import()`'s spec-mandated
    /// Promise-returning shape.
    fn dynamic_import(&mut self, specifier: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        let value = self.eval_expr(specifier, env)?;
        let referrer = env.resolve("\0module.id");
        let referrer = match &referrer {
            Some(JsValue::Str(s)) => s.to_string(),
            _ => String::new(),
        };
        let specifier_string = match self.to_js_string(&value) {
            Ok(s) => s.to_string(),
            Err(thrown) => return Ok(new_promise(self, PromiseFate::Rejected(thrown.0))),
        };
        let outcome = match self.module_loader.clone() {
            Some(hook) => hook(self, &specifier_string, &referrer),
            None => Err("dynamic import is not supported by this host".to_owned()),
        };
        Ok(match outcome {
            Ok(namespace) => new_promise(self, PromiseFate::Fulfilled(namespace)),
            Err(message) => new_promise(self, PromiseFate::Rejected(Throw::type_error(message).0)),
        })
    }
}
