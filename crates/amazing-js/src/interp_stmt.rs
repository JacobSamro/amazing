// Statement evaluation: exec_stmt/exec_stmts and everything they call for control flow --
// loops (while/do-while/for/for-in/for-of), try/catch/finally, switch, and job-queue draining --
// plus explicit resource management's using/await-using disposal machinery (using_stack,
// add_disposable, dispose_method, dispose_frame, await_value). Split out of interp.rs (JSRT-18)
// to keep it under the 4,000-line-per-file limit; no behavior change.
//
// The disposal machinery here is ported and adapted from lucid-softworks/lumen
// (crates/lumen/src/{eval,interpreter}.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.


/// A captured `using`/`await using` resource: the value plus its resolved dispose method.
/// `method_is_async` records whether `method` came from a real `@@asyncDispose` lookup (its
/// result is awaited at disposal time) as opposed to the `@@dispose` fallback used inside an
/// `await using` declaration when no `@@asyncDispose` exists (called but not awaited).
/// Ported from lucid-softworks/lumen (crates/lumen/src/interpreter.rs) @ f6c9860d, MIT.
struct Disposable {
    value: JsValue,
    method: JsValue,
    method_is_async: bool,
}

/// Whether a statement directly in a block/for-head is a `using`/`await using` declaration (so
/// that scope is a disposal boundary). Does not recurse — disposal scopes are per lexical block.
fn stmt_declares_resource(s: &ast::Stmt) -> bool {
    matches!(&s.kind, StmtKind::Var(d) if d.resource_kind.is_some())
}

/// Whether a `for-of` head (`for (using x of ...)` / `for (await using x of ...)`) declares a
/// per-iteration disposal resource, and if so whether it is the `await using` form.
fn for_of_dispose_kind(left: &ForLeft) -> Option<bool> {
    match left {
        ForLeft::Decl(d) => d.resource_kind.map(|k| k == ast::ResourceKind::AwaitUsing),
        ForLeft::Target(_) => None,
    }
}

/// A block body's own top-level lexically-declared names (`let`/`const`/`using`/`await using`,
/// plus named classes) -- used by Annex B.3.3 to decide whether a nested block-function's legacy
/// `var` promotion would collide with an intervening lexical declaration of the same name.
/// Adapted from lucid-softworks/lumen (crates/lumen/src/interpreter.rs, block_lexical_names) @
/// f6c9860d, MIT.
fn block_lexical_names(body: &[ast::Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    for s in body {
        match &s.kind {
            StmtKind::Var(d) if d.kind != ast::VarKind::Var => {
                for x in &d.decls {
                    out.extend(binding_names(&x.binding));
                }
            }
            StmtKind::Class(c) => {
                if let Some(n) = &c.name {
                    out.push(n.name.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// Push newly-shadowing names onto Annex B.3.3's `blocked` list (dedup'd); returns how many were
/// actually pushed, so the caller can truncate back to the enclosing scope's set once its own
/// nested block/loop/if body has been walked.
fn push_blocked(blocked: &mut Vec<String>, added: Vec<String>) -> usize {
    let mut pushed = 0;
    for name in added {
        if !blocked.iter().any(|b| b == &name) {
            blocked.push(name);
            pushed += 1;
        }
    }
    pushed
}

impl Interpreter {
    fn exec_stmt(&mut self, s: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        if let Some(hook) = self.debug_hook.clone() {
            hook.borrow_mut().on_statement(&self.current_script_id, &self.current_script_url, s.span.start.line, self.depth);
        }
        match &s.kind {
            StmtKind::Empty | StmtKind::Debugger => Ok(Flow::Normal),
            StmtKind::Expression(e) => {
                self.completion = self.eval_expr(e, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::Var(d) => {
                for x in &d.decls {
                    if d.kind == ast::VarKind::Var && x.init.is_none() {
                        continue;
                    }
                    let mut v = x
                        .init
                        .as_ref()
                        .map(|e| match &x.binding {
                            Binding::Ident(name) => self.eval_named_class_expr(e, env, &Self::class_binding_name(&name.name)),
                            _ => self.eval_expr(e, env),
                        })
                        .transpose()?
                        .unwrap_or(JsValue::Undefined);
                    if d.kind == ast::VarKind::Const && x.init.is_none() {
                        return Err(Throw::syntax_error("const requires an initializer"));
                    }
                    // VariableDeclaration/LexicalBinding NamedEvaluation (spec 8.6.3/14.3.1.2):
                    // only a bare BindingIdentifier target, never a destructuring pattern.
                    // `export default <expr>` desugars (module.rs) into `const \0module.default
                    // = expr`, so its synthetic internal binding name is mapped to the spec's
                    // real name argument, "default", rather than used literally.
                    if let (Some(init), Binding::Ident(identifier)) = (&x.init, &x.binding) {
                        let name = if identifier.name == "\0module.default" { "default" } else { &identifier.name };
                        self.named_evaluation(init, &mut v, name);
                    }
                    if let Some(kind) = d.resource_kind {
                        self.add_disposable(&v, kind == ast::ResourceKind::AwaitUsing)?;
                    }
                    self.bind_pattern(&x.binding, v, d.kind, env)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::Function(f) => {
                // Annex B.3.3 web-compat sync: evaluating a sloppy block-position plain
                // function declaration copies the block's own (just-hoisted, possibly
                // reassigned-by-now) binding into whatever legacy `var` binding `hoist_inner`
                // created for it in the enclosing scope -- a no-op if that name was blocked by
                // an intervening lexical declaration (no such outer binding exists) or we're in
                // strict mode (Annex B never applies). Adapted from lucid-softworks/lumen
                // (crates/lumen/src/eval.rs, annexb_fn_sync_eval) @ f6c9860d, MIT.
                //
                // This arm assumes `env` is the exact Block a StmtKind::Block block-entry
                // pre-pass already lexically bound this declaration into (`env.local_cell`
                // below is that per-block-entry binding, not some unrelated same-named
                // ambient var). A *bare*, braceless if/else-position FunctionDeclaration
                // (Annex B.3.4) never goes through this arm at all -- StmtKind::If's own
                // `exec_if_branch` handles that position directly, since there is no block
                // of its own here to have run a pre-pass, and reusing whatever `env` happens
                // to already have bound under this name (e.g. an unrelated outer declaration
                // of the same name elsewhere in the program) would read the wrong value.
                if !self.strict_eval_context && !f.is_async && !f.is_generator {
                    if let Some(n) = &f.name {
                        let scope = env.var_scope();
                        // A Global-kind scope's non-lexical bindings are read through the global
                        // object's own property (Env::resolve_raw), not the cell's `.value` --
                        // `scope.assign` (Env::assign_raw) already knows to write through there
                        // too, so use it for the actual mutation rather than the cell directly.
                        let promoted = scope.local_cell(&n.name).is_some_and(|c| !c.is_lexical());
                        if promoted {
                            if let Some(local) = env.local_cell(&n.name) {
                                if let Ok(v) = local.read() {
                                    let _ = scope.assign(&n.name, v);
                                }
                            }
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Class(c) => {
                if let Some(n) = &c.name {
                    let v = self.eval_class(c, env)?;
                    env.declare_lexical(&n.name, v, true)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::Block(b) => {
                self.completion = JsValue::Undefined;
                let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                // BlockDeclarationInstantiation's own function-hoisting: every direct
                // FunctionDeclaration child of this block is lexically bound, to a fresh
                // closure, at block entry -- independent of Annex B (this happens in strict
                // mode too; it's what makes `f` resolve inside the block at all, and what
                // Annex B.3.3's sync step above reads from).
                for stmt in &b.body {
                    if let StmtKind::Function(f) = &stmt.kind {
                        if let Some(n) = &f.name {
                            let v = self.make_function(f, &e);
                            if e.local_cell(&n.name).is_some() {
                                // Annex B.3.3.4: non-strict code may give the same name more
                                // than one FunctionDeclaration directly in one block (the
                                // duplicate-LexicallyDeclaredNames early error is relaxed only
                                // when every duplicate is a FunctionDeclaration).
                                // BlockDeclarationInstantiation still runs InitializeBinding once
                                // per declaration in source order, so the last one silently wins
                                // rather than re-declaring into a cell that already exists.
                                let _ = e.assign(&n.name, v);
                            } else {
                                e.declare_lexical(&n.name, v, true)?;
                            }
                        }
                    }
                }
                // A block is a disposal boundary only when it directly declares a `using` /
                // `await using` resource (spec: BlockDeclarationInstantiation's disposal capability).
                if !b.body.iter().any(stmt_declares_resource) {
                    return self.exec_stmts(&b.body, &e);
                }
                self.using_stack.push(Vec::new());
                let result = self.exec_stmts(&b.body, &e);
                let frame = self.using_stack.pop().unwrap_or_default();
                self.dispose_frame(frame, result)
            }
            StmtKind::If { cond, then, els } => {
                self.completion = JsValue::Undefined;
                if self.eval_expr(cond, env)?.to_boolean() {
                    self.exec_if_branch(then, env)
                } else if let Some(e) = els {
                    self.exec_if_branch(e, env)
                } else {
                    Ok(Flow::Normal)
                }
            }
            StmtKind::While { test, body } => {
                self.completion = JsValue::Undefined;
                self.loop_while(test, body, env, None)
            }
            StmtKind::With { .. } => Err(Throw::type_error("with statement is not implemented")),
            StmtKind::DoWhile { body, test } => {
                self.completion = JsValue::Undefined;
                self.loop_do_while(body, test, env, None)
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                self.completion = JsValue::Undefined;
                self.loop_for(
                    init.as_ref(),
                    test.as_ref(),
                    update.as_ref(),
                    body,
                    env,
                    None,
                )
            }
            StmtKind::ForIn { left, right, body } => {
                self.completion = JsValue::Undefined;
                let v = self.eval_expr(right, env)?;
                let object = match v {
                    JsValue::Null | JsValue::Undefined => None,
                    other => match array_receiver(self, other)? {
                        JsValue::Object(object) => Some(object),
                        _ => None,
                    },
                };
                let mut iterator = ForInState {
                    object,
                    object_was_visited: false,
                    visited_keys: HashSet::new(),
                    remaining_keys: VecDeque::new(),
                };
                while let Some(k) = self.next_for_in_key(&mut iterator)? {
                    let iteration_env = for_iteration_env(left, env);
                    self.bind_for(left, JsValue::str(k), &iteration_env)?;
                    match self.exec_stmt(body, &iteration_env)? {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Break(None) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForOf { left, right, body } => {
                self.completion = JsValue::Undefined;
                let v = self.eval_expr(right, env)?;
                self.exec_for_of(left, &v, body, env)
            }
            StmtKind::ForAwaitOf { left, right, body } => {
                self.completion = JsValue::Undefined;
                let iterable = self.eval_expr(right, env)?;
                self.exec_for_await_of(left, &iterable, body, env)
            }
            StmtKind::Switch { disc, cases } => {
                self.completion = JsValue::Undefined;
                let d = self.eval_expr(disc, env)?;
                let mut start = None;
                let mut def = None;
                for (n, c) in cases.iter().enumerate() {
                    if let Some(t) = &c.test {
                        let tv = self.eval_expr(t, env)?;
                        if self.strict_eq(&d, &tv) {
                            start = Some(n);
                            break;
                        }
                    } else {
                        def = Some(n);
                    }
                }
                let mut i = start.or(def);
                while let Some(n) = i {
                    match self.exec_stmts(&cases[n].body, env)? {
                        Flow::Normal => {
                            i = (n + 1 < cases.len()).then_some(n + 1);
                        }
                        Flow::Break(_) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Break { label: l } => Ok(Flow::Break(l.as_ref().map(|x| x.name.clone()))),
            StmtKind::Continue { label: l } => {
                Ok(Flow::Continue(l.as_ref().map(|x| x.name.clone())))
            }
            StmtKind::Labeled { label, body } => match &body.kind {
                StmtKind::While { test, body } => {
                    self.loop_while(test, body, env, Some(&label.name))
                }
                StmtKind::DoWhile { body, test } => {
                    self.loop_do_while(body, test, env, Some(&label.name))
                }
                StmtKind::For {
                    init,
                    test,
                    update,
                    body,
                } => self.loop_for(
                    init.as_ref(),
                    test.as_ref(),
                    update.as_ref(),
                    body,
                    env,
                    Some(&label.name),
                ),
                _ => match self.exec_stmt(body, env)? {
                    Flow::Break(Some(x)) if x == label.name => Ok(Flow::Normal),
                    x => Ok(x),
                },
            },
            StmtKind::Return(e) => Ok(Flow::Return(
                e.as_ref()
                    .map(|x| self.eval_expr(x, env))
                    .transpose()?
                    .unwrap_or(JsValue::Undefined),
            )),
            StmtKind::Throw(e) => Err(Throw(self.eval_expr(e, env)?)),
            StmtKind::Try(t) => {
                self.completion = JsValue::Undefined;
                self.exec_try(t, env)
            }
        }
    }

    /// Executes one IfStatement branch (spec 13.6.7 / Annex B.3.4): a *bare*,
    /// braceless `if (x) function f(){}` / `... else function f(){}` acts as
    /// if that FunctionDeclaration were the sole statement of a fresh,
    /// implicit Block -- its closure is created and bound there, then synced
    /// out to whatever legacy `var` placeholder `hoist_inner` reserved for it
    /// in the enclosing var scope. This must be handled here, at the If
    /// itself, rather than by falling through to the generic
    /// `StmtKind::Function` arm above: that arm's `env.local_cell` lookup
    /// assumes `env` is a Block whose own pre-pass already bound this exact
    /// occurrence, which is never true for a bare if/else branch -- `env`
    /// here is just the ambient enclosing scope, and if some unrelated
    /// same-named declaration already lives there (e.g. another top-level
    /// `function f(){}` elsewhere in the same program), that arm would
    /// silently read and re-sync THAT value instead of creating this
    /// branch's own closure. Every other bare-statement position (while,
    /// for, labeled, ...) does not permit a FunctionDeclaration at all, so
    /// this special case is scoped to If alone. Adapted from
    /// lucid-softworks/lumen (crates/lumen/src/eval.rs, annexb_fn_sync_eval)
    /// @ f6c9860d, MIT.
    fn exec_if_branch(&mut self, branch: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        if let StmtKind::Function(f) = &branch.kind {
            if !self.strict_eval_context && !f.is_async && !f.is_generator {
                if let Some(n) = &f.name {
                    let scope = env.var_scope();
                    // B.3.3.1's own exception: the whole extension is skipped, not merely a
                    // no-op sync, when F collides with a parameter name of the immediately
                    // enclosing function (var_scope's own cell IS that parameter's cell here,
                    // indistinguishable by identity/is_lexical alone from a genuine hoist_inner
                    // placeholder -- `current_function_param_names` checks the AST directly).
                    let blocked_by_param = self.current_function_param_names().iter().any(|p| p == &n.name);
                    if !blocked_by_param && scope.local_cell(&n.name).is_some_and(|c| !c.is_lexical()) {
                        let implicit = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                        let v = self.make_function(f, &implicit);
                        let _ = implicit.declare_lexical(&n.name, v.clone(), true);
                        let _ = scope.assign(&n.name, v);
                        return Ok(Flow::Normal);
                    }
                }
            }
        }
        self.exec_stmt(branch, env)
    }

    /// Names bound by the currently executing (innermost) call's own formal
    /// parameter list, per the AST -- `[]` for top-level/global/eval code
    /// with no enclosing function call at all. Used only by Annex B.3.3.1's
    /// parameter-collision exception above; a destructuring parameter's own
    /// nested names count too, via `binding_names`.
    fn current_function_param_names(&self) -> Vec<String> {
        self.frames
            .last()
            .and_then(|frame| frame.own_function.as_ref())
            .and_then(|obj| obj.borrow().func.as_ref().map(|f| f.params.clone()))
            .map(|params| params.iter().flat_map(|p| binding_names(&p.binding)).collect())
            .unwrap_or_default()
    }

    fn exec_stmts(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<Flow, Throw> {
        for s in body {
            match self.exec_stmt(s, env)? {
                Flow::Normal => {}
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }

    /// GetDisposeMethod (explicit resource management, spec 27.3.1): `@@asyncDispose` (falling
    /// back to `@@dispose`) for `await using`, else plain `@@dispose`. Returns the resolved
    /// method plus whether it came from a real `@@asyncDispose` lookup (as opposed to the
    /// `@@dispose` fallback), which decides whether disposal awaits its result.
    /// Ported from lucid-softworks/lumen (crates/lumen/src/eval.rs) @ f6c9860d, MIT.
    fn dispose_method(&mut self, value: &JsValue, is_async: bool) -> Result<(JsValue, bool), Throw> {
        let mut method = JsValue::Undefined;
        let mut from_async = false;
        if is_async {
            method = self.get_prop(value, WellKnownSymbol::AsyncDispose.symbol())?;
            from_async = self.binding_is_callable(&method);
        }
        if !self.binding_is_callable(&method) {
            method = self.get_prop(value, WellKnownSymbol::Dispose.symbol())?;
        }
        if !self.binding_is_callable(&method) {
            return Err(Throw::type_error("value is not disposable"));
        }
        Ok((method, from_async))
    }

    /// AddDisposableResource (spec 27.3.3.2): capture `value`'s dispose method now, at `using`/
    /// `await using` binding time (so a later TypeError from a non-disposable value surfaces at
    /// the declaration, not at scope exit). `null`/`undefined` resources are simply skipped: no
    /// dispose call is ever made for them.
    /// Ported from lucid-softworks/lumen (crates/lumen/src/eval.rs) @ f6c9860d, MIT.
    fn add_disposable(&mut self, value: &JsValue, is_async: bool) -> Result<(), Throw> {
        if matches!(value, JsValue::Null | JsValue::Undefined) {
            return Ok(());
        }
        let (method, method_is_async) = self.dispose_method(value, is_async)?;
        if self.using_stack.is_empty() {
            self.using_stack.push(Vec::new());
        }
        self.using_stack.last_mut().unwrap().push(Disposable {
            value: value.clone(),
            method,
            method_is_async,
        });
        Ok(())
    }

    /// DisposeResources (spec 27.3.3.4): run a disposal frame's resources in reverse declaration
    /// order against `result`, the enclosing scope's own completion. A disposal that throws
    /// either becomes the completion (when nothing was already pending) or is folded into a
    /// `SuppressedError` chain, with the previously-pending error as `suppressed` — so a single
    /// disposal error, with the scope otherwise completing normally, propagates as-is.
    /// Ported from lucid-softworks/lumen (crates/lumen/src/eval.rs) @ f6c9860d, MIT.
    fn dispose_frame(&mut self, frame: Vec<Disposable>, result: Result<Flow, Throw>) -> Result<Flow, Throw> {
        let mut completion = result;
        for d in frame.into_iter().rev() {
            let disposed = self
                .call_value(&d.method, d.value.clone(), Vec::new())
                .and_then(|v| if d.method_is_async { self.await_value(v) } else { Ok(v) });
            if let Err(new_err) = disposed {
                completion = match completion {
                    Err(prev) => Err(Throw(self.make_suppressed_error(new_err.0, prev.0))),
                    Ok(_) => Err(new_err),
                };
            }
        }
        completion
    }

    /// Build a `SuppressedError` with the same `error`/`suppressed` own-data-property shape as
    /// the `SuppressedError` constructor (see `native_suppressed_error`), for disposal-time error
    /// aggregation that isn't going through a `new SuppressedError(...)` call.
    fn make_suppressed_error(&mut self, error: JsValue, suppressed: JsValue) -> JsValue {
        let mut object = JsObject::ordinary();
        object.exotic = Exotic::Error;
        object.proto = self.intrinsic_constructors.get("SuppressedError").map(|(_, proto)| proto.clone());
        object.define("error", symbol_data(error, true, true)).unwrap();
        object.define("suppressed", symbol_data(suppressed, true, true)).unwrap();
        JsValue::Object(Obj(Rc::new(RefCell::new(object))))
    }

    /// Await(value) (spec 27.7.5.3), factored out of `UnOp::Await` so disposal can reuse it. Only
    /// unwraps an already-settled promise; a still-pending one degrades to `undefined` since this
    /// tree-walking interpreter has no coroutine/suspension machinery to genuinely park here.
    fn await_value(&mut self, value: JsValue) -> Result<JsValue, Throw> {
        match value {
            JsValue::Object(o) => {
                let exotic = o.borrow().exotic.clone();
                match exotic {
                    Exotic::Promise(PromiseState { state: PromiseFate::Fulfilled(value), .. }) => Ok(value),
                    Exotic::Promise(PromiseState { state: PromiseFate::Rejected(value), .. }) => Err(Throw(value)),
                    Exotic::Promise(PromiseState { state: PromiseFate::Pending, .. }) => Ok(JsValue::Undefined),
                    _ => Ok(JsValue::Object(o)),
                }
            }
            other => Ok(other),
        }
    }

    fn drain_jobs(&mut self) -> Result<(), Throw> {
        if self.depth != 0 { return Ok(()); }
        while let Some(job) = self.jobs.pop_front() {
            self.run_job(job)?;
        }
        Ok(())
    }

    fn run_job(&mut self, job: Job) -> Result<(), Throw> {
        match job {
            Job::Promise(job) => run_promise_reaction(job.reaction, job.fate, self),
            Job::Microtask(callback) => {
                self.call_value(&callback, JsValue::Undefined, Vec::new())?;
                Ok(())
            }
        }
    }
    fn loop_while(
        &mut self,
        test: &ast::Expr,
        body: &ast::Stmt,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        while self.eval_expr(test, env)?.to_boolean() {
            match self.exec_stmt(body, env)? {
                Flow::Normal | Flow::Continue(None) => {}
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => continue,
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }

    fn loop_do_while(
        &mut self,
        body: &ast::Stmt,
        test: &ast::Expr,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        loop {
            match self.exec_stmt(body, env)? {
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(None) | Flow::Normal => {}
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => {}
                Flow::Continue(name) => return Ok(Flow::Continue(name)),
                Flow::Break(name) => return Ok(Flow::Break(name)),
                Flow::Return(value) => return Ok(Flow::Return(value)),
            }
            if !self.eval_expr(test, env)?.to_boolean() {
                break;
            }
        }
        Ok(Flow::Normal)
    }

    fn loop_for(
        &mut self,
        init: Option<&ForInit>,
        test: Option<&ast::Expr>,
        update: Option<&ast::Expr>,
        body: &ast::Stmt,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        // A `for (using x = r; ...)` / `for (await using x = r; ...)` head is a disposal
        // boundary: the head's resource(s) are disposed once, when the whole loop exits (not
        // per-iteration — contrast `for (using x of iterable)`, handled in `exec_for_of`).
        let has_using = matches!(init, Some(ForInit::Decl(decl)) if decl.resource_kind.is_some());
        if !has_using {
            return self.loop_for_inner(init, test, update, body, env, label);
        }
        self.using_stack.push(Vec::new());
        let result = self.loop_for_inner(init, test, update, body, env, label);
        let frame = self.using_stack.pop().unwrap_or_default();
        self.dispose_frame(frame, result)
    }

    fn loop_for_inner(
        &mut self,
        init: Option<&ForInit>,
        test: Option<&ast::Expr>,
        update: Option<&ast::Expr>,
        body: &ast::Stmt,
        env: &EnvHandle,
        label: Option<&str>,
    ) -> Result<Flow, Throw> {
        let loop_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        if let Some(init) = init {
            match init {
                ForInit::Decl(decl) => {
                    let fake = ast::Stmt::new(StmtKind::Var((**decl).clone()), body.span.clone());
                    self.exec_stmt(&fake, &loop_env)?;
                }
                ForInit::Expr(expression) => {
                    self.eval_expr(expression, &loop_env)?;
                }
            }
        }
        let per_iteration_names = match init {
            Some(ForInit::Decl(decl)) if decl.kind != ast::VarKind::Var => decl
                .decls
                .iter()
                .flat_map(|declarator| binding_names(&declarator.binding))
                .collect(),
            _ => Vec::new(),
        };
        let state_env = if per_iteration_names.is_empty() {
            loop_env.clone()
        } else {
            let state = EnvHandle::new(EnvKind::Block, Some(env.clone()));
            for name in &per_iteration_names {
                let value = loop_env.resolve(name).unwrap_or(JsValue::Undefined);
                state.declare_lexical(name, value, true)?;
            }
            state
        };
        loop {
            let iteration_env = if per_iteration_names.is_empty() {
                loop_env.clone()
            } else {
                let iteration = EnvHandle::new(EnvKind::Block, Some(state_env.clone()));
                for name in &per_iteration_names {
                    let value = state_env.resolve(name).unwrap_or(JsValue::Undefined);
                    iteration.declare_lexical(name, value, true)?;
                }
                iteration
            };
            if let Some(test) = test {
                if !self.eval_expr(test, &iteration_env)?.to_boolean() {
                    break;
                }
            }
            match self.exec_stmt(body, &iteration_env)? {
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(None) => {}
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => {}
                Flow::Normal => {}
                Flow::Continue(name) => return Ok(Flow::Continue(name)),
                Flow::Break(name) => return Ok(Flow::Break(name)),
                Flow::Return(value) => return Ok(Flow::Return(value)),
            }
            if per_iteration_names.is_empty() {
                if let Some(update) = update {
                    self.eval_expr(update, &loop_env)?;
                }
            } else {
                let update_env = EnvHandle::new(EnvKind::Block, Some(state_env.clone()));
                for name in &per_iteration_names {
                    let value = iteration_env.resolve(name).unwrap_or(JsValue::Undefined);
                    update_env.declare_lexical(name, value, true)?;
                }
                if let Some(update) = update {
                    self.eval_expr(update, &update_env)?;
                }
                for name in &per_iteration_names {
                    let value = update_env.resolve(name).unwrap_or(JsValue::Undefined);
                    state_env.assign(name, value)?;
                }
            }
        }
        Ok(Flow::Normal)
    }
    fn bind_for(&mut self, left: &ForLeft, value: JsValue, env: &EnvHandle) -> Result<(), Throw> {
        match left {
            ForLeft::Target(t) if is_destructuring_expr(t) => {
                self.assign_pattern_expr(t, &value, env)
            }
            ForLeft::Target(t) => self.assign_target(t, value, env),
            ForLeft::Decl(d) => self.bind_pattern(
                &d.decls
                    .first()
                    .ok_or_else(|| Throw::syntax_error("missing loop binding"))?
                    .binding,
                value,
                d.kind,
                env,
            ),
        }
    }
    fn exec_try(&mut self, t: &ast::TryStmt, env: &EnvHandle) -> Result<Flow, Throw> {
        let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        self.hoist(&t.block.body, env)?;
        let mut out = match self.exec_stmts(&t.block.body, &e) {
            Ok(x) => Ok(x),
            Err(th) => {
                let th = self.normalize_throw(th);
                if let Some(c) = &t.catch {
                    let ce = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                    if let Some(binding) = &c.binding {
                        self.bind_pattern(binding, th.0, ast::VarKind::Let, &ce)?;
                    }
                    self.hoist(&c.body.body, env)?;
                    self.exec_stmts(&c.body.body, &ce)
                } else {
                    Err(th)
                }
            }
        };
        if let Some(f) = &t.finally {
            let fe = EnvHandle::new(EnvKind::Block, Some(env.clone()));
            self.hoist(&f.body, env)?;
            match self.exec_stmts(&f.body, &fe) {
                Ok(Flow::Normal) => {}
                Ok(x) => out = Ok(x),
                Err(e) => out = Err(e),
            }
        }
        out
    }

    pub fn eval_expr(&mut self, e: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match &e.kind {
            ExprKind::PrivateName(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            ExprKind::RegExp(regex) => self.make_regexp(regex.clone()),
            ExprKind::Num(n) => self.numeric(n),
            ExprKind::BigInt(digits) => BinaryBigInt::parse_radix(digits, 10)
                .map(Self::bigint_value)
                .ok_or_else(|| Throw::syntax_error("invalid BigInt literal")),
            ExprKind::TaggedTemplate { .. } => Err(Throw::type_error("tagged templates are not implemented")),
            ExprKind::Str(s) => Ok(match &e.string_utf16 {
                Some(units) => JsValue::str_units(units.clone()),
                None => JsValue::str(s.as_str()),
            }),
            ExprKind::Bool(b) => Ok(JsValue::Bool(*b)),
            ExprKind::Null => Ok(JsValue::Null),
            ExprKind::Ident(i) => self
                .identifier_value(env, &i.name)?
                .ok_or_else(|| Throw::reference_error(format!("{} is not defined", i.name))),
            ExprKind::This => env.resolve_this_checked(),
            ExprKind::NewTarget => Ok(self.current_new_target().unwrap_or(JsValue::Undefined)),
            ExprKind::DynamicImport(_) => Err(Throw::type_error("dynamic import is not implemented")),
            ExprKind::DynamicImportPhase { .. } => Err(Throw::type_error("dynamic import phase is not implemented")),
            ExprKind::ImportMeta => env.resolve("\0import.meta")
                .ok_or_else(|| Throw::type_error("import.meta is only available in modules")),
            ExprKind::Super => {
                let home = self
                    .frames
                    .last()
                    .and_then(|frame| frame.home.clone())
                    .ok_or_else(|| Throw::syntax_error("super used outside a method"))?;
                let prototype = home
                    .borrow()
                    .proto
                    .clone()
                    .ok_or_else(|| Throw::type_error("super prototype is null"))?;
                Ok(JsValue::Object(prototype))
            }
            ExprKind::Template(t) => {
                let mut s = Vec::<u16>::new();
                for (i, q) in t.quasis.iter().enumerate() {
                    if let Some(units) = t.quasis_utf16.get(i) {
                        s.extend_from_slice(units);
                    } else {
                        s.extend(q.encode_utf16());
                    }
                    if let Some(x) = t.exprs.get(i) {
                        let v = self.eval_expr(x, env)?;
                        s.extend_from_slice(self.to_js_string(&v)?.units());
                    }
                }
                Ok(JsValue::str_units(s))
            }
            ExprKind::Array(xs) => {
                let a = JsValue::object({
                    let mut o = JsObject::array();
                    o.proto = Some(self.array_proto.clone());
                    o
                });
                let mut n = 0usize;
                for x in xs {
                    match x {
                        None => n += 1,
                        Some(x) if matches!(x.kind, ExprKind::Spread(_)) => {
                            if let ExprKind::Spread(y) = &x.kind {
                                let src = self.eval_expr(y, env)?;
                                let vals = self.iterate(&src)?;
                                for v in vals {
                                    self.set_prop(&a, &n.to_string(), v)?;
                                    n += 1;
                                }
                            }
                        }
                        Some(x) => {
                            let v = self.eval_expr(x, env)?;
                            self.set_prop(&a, &n.to_string(), v)?;
                            n += 1;
                        }
                    }
                }
                self.set_prop(&a, "length", JsValue::num(n as f64))?;
                Ok(a)
            }
            ExprKind::Object(xs) => {
                let o = self.object_value();
                for x in xs {
                    match x {
                        ast::ObjElem::Spread(y) => {
                            let src = self.eval_expr(y, env)?;
                            for k in self.enumerable_property_keys(&src)? {
                                let v = self.get_prop(&src, &k)?;
                                self.set_prop(&o, &k, v)?;
                            }
                        }
                        ast::ObjElem::Prop(p) => {
                            let k = match &p.key_utf16 {
                                Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
                                None => self.prop_key(&p.key, env)?,
                            };
                            if let ast::PropValue::Method(method) = &p.value {
                                if matches!(method.kind, ast::MethodKind::Get | ast::MethodKind::Set) {
                                    let function_value = self.method_value(method, env, false);
                                    let JsValue::Object(object) = &o else { unreachable!() };
                                    let mut property = object.borrow().own_prop(&k).cloned()
                                        .filter(|property| property.getter.is_some() || property.setter.is_some())
                                        .unwrap_or(Property {
                                            value: JsValue::Undefined, writable: false,
                                            enumerable: true, configurable: true,
                                            getter: None, setter: None,
                                        });
                                    if method.kind == ast::MethodKind::Get { property.getter = Some(function_value); }
                                    else { property.setter = Some(function_value); }
                                    object.borrow_mut().define(&k, property)
                                        .map_err(|_| Throw::type_error("could not define object accessor"))?;
                                    continue;
                                }
                            }
                            let v = match &p.value {
                                ast::PropValue::Shorthand(i) => self.identifier_value(env, &i.name)?
                                    .ok_or_else(|| Throw::reference_error(i.name.clone()))?,
                                ast::PropValue::Init(x) => {
                                    // PropertyDefinitionEvaluation (spec 13.2.5.5) skips NamedEvaluation
                                    // entirely for a non-computed `__proto__` key ("isProtoSetter" is
                                    // true): an anonymous `{ __proto__: function(){} }` value must NOT
                                    // be named "__proto__" -- it becomes the object's [[Prototype]]
                                    // below, not an own property, and confirmedly keeps whatever name
                                    // (usually none) it already had.
                                    let is_proto_setter = !matches!(&p.key, PropKey::Computed(_))
                                        && k.as_string().is_some_and(|name| *name == "__proto__");
                                    let mut val = if is_proto_setter {
                                        self.eval_expr(x, env)?
                                    } else {
                                        // Same class-self-reference-before-eval need as every other
                                        // NamedEvaluation call site (see the comment on `assign`).
                                        // `Self::property_function_name` (not the lossy
                                        // `property_key_as_fn_name` used below for the ordinary
                                        // function case) keeps a lone-surrogate string key exact.
                                        self.eval_named_class_expr(x, env, &Self::property_function_name(&k))?
                                    };
                                    if !is_proto_setter {
                                        self.named_evaluation(x, &mut val, &property_key_as_fn_name(&k));
                                    }
                                    val
                                }
                                ast::PropValue::CoverInitializedName { .. } => {
                                    return Err(Throw::syntax_error("cover initialized name outside assignment pattern"));
                                }
                                ast::PropValue::Method(m) => self.method_value(m, env, false),
                            };
                            // A non-computed colon definition of __proto__ sets
                            // [[Prototype]] rather than creating an own property.
                            if matches!(&p.value, ast::PropValue::Init(_))
                                && !matches!(&p.key, PropKey::Computed(_))
                                && k.as_string().is_some_and(|name| *name == "__proto__")
                            {
                                let JsValue::Object(object) = &o else { unreachable!() };
                                match v {
                                    JsValue::Object(prototype) => object.borrow_mut().proto = Some(prototype),
                                    JsValue::Null => object.borrow_mut().proto = None,
                                    _ => {}
                                }
                                continue;
                            }
                            // Object-literal PropertyDefinition uses CreateDataProperty,
                            // which replaces a preceding accessor of the same name.
                            let JsValue::Object(object) = &o else { unreachable!() };
                            object.borrow_mut().define(&k, Property::data(v))
                                .map_err(|_| Throw::type_error("could not define object property"))?;
                        }
                    }
                }
                Ok(o)
            }
            ExprKind::Spread(x) => self.eval_expr(x, env),
            ExprKind::Function(f) => {
                if let Some(name) = &f.name {
                    let named_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                    let value = self.make_function(f, &named_env);
                    named_env.declare_lexical(&name.name, value.clone(), false)?;
                    Ok(value)
                } else {
                    Ok(self.make_function(f, env))
                }
            }
            ExprKind::Arrow(a) => Ok(self.make_arrow(a, env)),
            ExprKind::Class(c) => self.eval_class(c, env),
            ExprKind::Paren(x) => self.eval_expr(x, env),
            ExprKind::Sequence(xs) => {
                let mut v = JsValue::Undefined;
                for x in xs {
                    v = self.eval_expr(x, env)?;
                }
                Ok(v)
            }
            ExprKind::Unary { op, operand } => self.unary(*op, operand, env),
            ExprKind::Update { op, prefix, target } => self.update(*op, *prefix, target, env),
            ExprKind::Binary { op, left, right } => {
                if *op == ast::BinOp::Add {
                    return self.eval_add_chain(e, env);
                }
                if *op == ast::BinOp::In {
                    if let ExprKind::PrivateName(identifier) = &left.kind {
                        let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                            Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                        })?;
                        let base = self.eval_expr(right, env)?;
                        return private_field_has(&base, &name).map(JsValue::Bool);
                    }
                }
                let a = self.eval_expr(left, env)?;
                let b = self.eval_expr(right, env)?;
                self.binary(*op, a, b)
            }
            ExprKind::Logical { op, left, right } => {
                let a = self.eval_expr(left, env)?;
                let take = match op {
                    ast::LogicalOp::And => a.to_boolean(),
                    ast::LogicalOp::Or => !a.to_boolean(),
                    ast::LogicalOp::Coalesce => a.is_null() || a.is_undefined(),
                };
                if take {
                    self.eval_expr(right, env)
                } else {
                    Ok(a)
                }
            }
            ExprKind::Assign { op, target, value } => self.assign(*op, target, value, env),
            ExprKind::Conditional {
                test,
                consequent,
                alternate,
            } => {
                if self.eval_expr(test, env)?.to_boolean() {
                    self.eval_expr(consequent, env)
                } else {
                    self.eval_expr(alternate, env)
                }
            }
            ExprKind::Call {
                callee,
                args,
                optional,
            } => self.call_expr(callee, args, *optional, env),
            ExprKind::New { callee, args } => {
                let f = self.eval_expr(callee, env)?;
                let a = self.eval_args(args, env)?;
                self.construct(&f, a)
            }
            ExprKind::Member {
                obj,
                prop,
                optional,
            } => {
                let o = self.eval_expr(obj, env)?;
                if *optional && (o.is_null() || o.is_undefined()) {
                    Ok(JsValue::Undefined)
                } else if let MemberProp::Private(identifier) = prop {
                    let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                        Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                    })?;
                    private_field_get(self, &o, &name)
                } else {
                    let k = self.member_key(prop, env)?;
                    self.get_prop(&o, &k)
                }
            }
            ExprKind::Yield { .. } => Err(Throw::type_error("generators are not implemented")),
        }
    }

    fn eval_add_chain(&mut self, expression: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        let mut rights = Vec::new();
        let mut leftmost = expression;
        while let ExprKind::Binary { op: ast::BinOp::Add, left, right } = &leftmost.kind {
            rights.push(right.as_ref());
            leftmost = left;
        }
        let mut value = self.eval_expr(leftmost, env)?;
        while let Some(right) = rights.pop() {
            let next = self.eval_expr(right, env)?;
            value = self.binary(ast::BinOp::Add, value, next)?;
        }
        Ok(value)
    }

    fn numeric(&self, n: &crate::tokens::Numeric) -> Result<JsValue, Throw> {
        if n.bigint {
            let digits: String = n.raw.chars().filter(|character| *character != '_').collect();
            let value = BinaryBigInt::parse_radix(&digits, n.radix as u32)
                .ok_or_else(|| Throw::syntax_error("invalid BigInt literal"))?;
            return Ok(Self::bigint_value(value));
        }
        let raw: String = n.raw.chars().filter(|c| *c != '_').collect();
        let v = if n.radix == 10 {
            raw.parse().unwrap_or(f64::NAN)
        } else {
            u128::from_str_radix(&raw, n.radix as u32)
                .map(|x| x as f64)
                .unwrap_or(f64::NAN)
        };
        Ok(JsValue::num(v))
    }
    fn unary(&mut self, op: ast::UnOp, x: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match op {
            ast::UnOp::Typeof => {
                match &x.kind {
                    ExprKind::Ident(i) => {
                        return Ok(JsValue::str(
                            self.identifier_value(env, &i.name)?
                                .map(|v| self.typeof_value(&v))
                                .unwrap_or("undefined"),
                        ));
                    }
                    ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Ident(_)) => {
                        if let ExprKind::Ident(i) = &inner.kind {
                            return Ok(JsValue::str(
                                self.identifier_value(env, &i.name)?
                                    .map(|v| self.typeof_value(&v))
                                    .unwrap_or("undefined"),
                            ));
                        }
                    }
                    _ => {}
                }
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::str(self.typeof_value(&v)))
            }
            ast::UnOp::Void => {
                let value = self.eval_expr(x, env)?;
                self.unary_value(op, value)
            }
            ast::UnOp::Delete => match &x.kind {
                ExprKind::Paren(inner) => self.unary(op, inner, env),
                ExprKind::Ident(name) => self.delete_scoped_name(env, &name.name),
                ExprKind::Member { obj, prop, .. } if matches!(prop, MemberProp::Private(_)) => {
                    // A private member's `delete` target is an early SyntaxError
                    // (§13.5.1.1); the parser does not reject it yet, so guard here.
                    let _ = self.eval_expr(obj, env)?;
                    Err(Throw::syntax_error("private members cannot be deleted"))
                }
                ExprKind::Member { obj, prop, .. } => {
                    let o = self.eval_expr(obj, env)?;
                    let k = self.member_key(prop, env)?;
                    if let JsValue::Object(o) = o {
                        if let Some(result) = self.window_proxy_delete(&o, &k) {
                            let deleted = result?;
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete WindowProxy property")); }
                            return Ok(JsValue::Bool(deleted));
                        }
                        if let Some(result) = self.proxy_delete(&o, &k) {
                            let deleted = result?;
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete proxy property")); }
                            return Ok(JsValue::Bool(deleted));
                        }
                        if matches!(o.borrow().exotic, Exotic::ModuleNamespace(_)) {
                            if o.borrow().own_prop(&k).is_some() {
                                return Err(Throw::type_error("module namespace export cannot be deleted"));
                            }
                            return Ok(JsValue::Bool(true));
                        }
                        if let Some(deleted) = binary_delete_index(&o, &k) {
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete typed array index")); }
                            return Ok(JsValue::Bool(deleted));
                        }
                        if let Some(deleted) = match k.as_string() {
                            Some(name) => self.legacy_delete(&o, name)?,
                            None => None,
                        } {
                            Ok(JsValue::Bool(deleted))
                        } else {
                            let deleted = o.borrow_mut().delete_own(&k);
                            if !deleted && self.strict_eval_context { return Err(Throw::type_error("cannot delete non-configurable property")); }
                            Ok(JsValue::Bool(deleted))
                        }
                    } else {
                        Ok(JsValue::Bool(true))
                    }
                }
                _ => {
                    self.eval_expr(x, env)?;
                    Ok(JsValue::Bool(true))
                }
            },
            ast::UnOp::Await => {
                let value = self.eval_expr(x, env)?;
                self.await_value(value)
            }
            ast::UnOp::Not => {
                let value = self.eval_expr(x, env)?;
                self.unary_value(op, value)
            }
            ast::UnOp::BitNot => {
                let v = self.eval_expr(x, env)?;
                self.unary_value(op, v)
            }
            ast::UnOp::Neg => {
                let v = self.eval_expr(x, env)?;
                self.unary_value(op, v)
            }
            ast::UnOp::Pos => {
                let v = self.eval_expr(x, env)?;
                self.unary_value(op, v)
            }
        }
    }
    fn unary_value(&mut self, op: ast::UnOp, value: JsValue) -> Result<JsValue, Throw> {
        match op {
            ast::UnOp::Neg => {
                let primitive = self.primitive(value)?;
                if let JsValue::BigInt(value) = &primitive { Ok(Self::bigint_value(value.negated())) }
                else { Ok(JsValue::num(-self.to_number(&primitive)?)) }
            },
            ast::UnOp::Pos => Ok(JsValue::num(self.to_number(&value)?)),
            ast::UnOp::Not => Ok(JsValue::Bool(!value.to_boolean())),
            ast::UnOp::BitNot => {
                let primitive = self.primitive(value)?;
                if let JsValue::BigInt(value) = &primitive { Ok(Self::bigint_value(value.bit_not())) }
                else { Ok(JsValue::num((!self.to_int32(&primitive)?) as f64)) }
            },
            ast::UnOp::Typeof => Ok(JsValue::str(self.typeof_value(&value))),
            ast::UnOp::Void => Ok(JsValue::Undefined),
            ast::UnOp::Delete | ast::UnOp::Await => Err(Throw::type_error("invalid unary value operation")),
        }
    }
    fn update_value(&mut self, op: ast::UpdateOp, old: JsValue) -> Result<JsValue, Throw> {
        let primitive = self.primitive(old)?;
        if let JsValue::BigInt(value) = primitive {
            Ok(Self::bigint_value(if op == ast::UpdateOp::Inc { value.add(&BinaryBigInt::one()) }
                else { value.sub(&BinaryBigInt::one()) }))
        } else {
            let delta = if op == ast::UpdateOp::Inc { 1.0 } else { -1.0 };
            Ok(JsValue::num(self.to_number(&primitive)? + delta))
        }
    }
    fn update(
        &mut self,
        op: ast::UpdateOp,
        prefix: bool,
        x: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        let (reference, old) = self.resolve_reference(x, env)?;
        let next = self.update_value(op, old.clone())?;
        self.put_reference(reference, next.clone(), env)?;
        Ok(if prefix { next } else { old })
    }
    fn binary(&mut self, op: ast::BinOp, a: JsValue, b: JsValue) -> Result<JsValue, Throw> {
        use ast::BinOp::*;
        match op {
            Add => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                if x.is_str() || y.is_str() {
                    let left = self.to_js_string(&x)?;
                    let right = self.to_js_string(&y)?;
                    let mut units = Vec::with_capacity(left.len() + right.len());
                    units.extend_from_slice(left.units());
                    units.extend_from_slice(right.units());
                    Ok(JsValue::str_units(units))
                } else if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    self.bigint_operator(op, x, y)
                } else {
                    Ok(JsValue::num(self.to_number(&x)? + self.to_number(&y)?))
                }
            }
            Sub | Mul | Div | Rem | Pow | Shl | Shr | UShr | BitAnd | BitOr | BitXor => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    return self.bigint_operator(op, x, y);
                }
                Ok(JsValue::num(match op {
                    Sub => self.to_number(&x)? - self.to_number(&y)?,
                    Mul => self.to_number(&x)? * self.to_number(&y)?,
                    Div => self.to_number(&x)? / self.to_number(&y)?,
                    Rem => self.to_number(&x)? % self.to_number(&y)?,
                    Pow => self.to_number(&x)?.powf(self.to_number(&y)?),
                    Shl | Shr | UShr => {
                        let left = self.to_int32(&x)?;
                        let shift = self.to_uint32(&y)? & 31;
                        match op { Shl => left.wrapping_shl(shift) as f64,
                            Shr => (left >> shift) as f64,
                            _ => (left as u32).wrapping_shr(shift) as f64 }
                    }
                    BitAnd | BitOr | BitXor => {
                        let left = self.to_int32(&x)?;
                        let right = self.to_int32(&y)?;
                        match op { BitAnd => (left & right) as f64,
                            BitOr => (left | right) as f64,
                            _ => (left ^ right) as f64 }
                    }
                    _ => unreachable!(),
                }))
            }
            Lt | Gt | Le | Ge => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                let c = if x.is_str() && y.is_str() {
                    Some(x.as_str().unwrap().cmp(y.as_str().unwrap()))
                } else if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    self.compare_bigint_primitives(&x, &y)?
                } else {
                    self.to_number(&x)?
                        .partial_cmp(&self.to_number(&y)?)
                };
                Ok(JsValue::Bool(match op {
                    Lt => c.is_some_and(|order| order.is_lt()),
                    Gt => c.is_some_and(|order| order.is_gt()),
                    Le => c.is_some_and(|order| order.is_le()),
                    Ge => c.is_some_and(|order| order.is_ge()),
                    _ => false,
                }))
            }
            EqEq | NotEq => {
                let e = self.loose_eq(&a, &b)?;
                Ok(JsValue::Bool(if op == EqEq { e } else { !e }))
            }
            StrictEq | StrictNeq => {
                let e = self.strict_eq(&a, &b);
                Ok(JsValue::Bool(if op == StrictEq { e } else { !e }))
            }
            InstanceOf => self.instanceof(&a, &b),
            In => {
                let k = self.to_property_key(&a)?;
                match b {
                    JsValue::Object(o) => {
                        if let Some(result) = self.window_proxy_has(&o, &k) { return Ok(JsValue::Bool(result?)); }
                        if let Some(result) = self.proxy_has(&o, &k) { return Ok(JsValue::Bool(result?)); }
                        Ok(JsValue::Bool(match k.as_string() {
                        _ if binary_index_descriptor(&o, &k).is_some() => binary_index_descriptor(&o, &k).flatten().is_some(),
                        Some(name) => self.legacy_has_property(&o, name)?,
                        None => o.borrow().has_property(&k),
                    })) },
                    _ => Err(Throw::type_error("right operand of in is not an object")),
                }
            }
        }
    }
    fn assign(
        &mut self,
        op: ast::AssignOp,
        t: &ast::Expr,
        r: &ast::Expr,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        if op == ast::AssignOp::Assign {
            if let ExprKind::Member { obj, prop: MemberProp::Private(identifier), .. } = &t.kind {
                let receiver = self.eval_expr(obj, e)?;
                let name = e.resolve_private(&identifier.name).ok_or_else(|| {
                    Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                })?;
                let value = self.eval_expr(r, e)?;
                private_field_set(self, &receiver, &name, value.clone())?;
                return Ok(value);
            }
        }
        if op == ast::AssignOp::Assign && is_destructuring_expr(t) {
            let value = self.eval_expr(r, e)?;
            self.assign_pattern_expr(t, &value, e)?;
            return Ok(value);
        }
        let (reference, old) = if op == ast::AssignOp::Assign {
            if let Some(name) = event_handler::identifier_name(t) {
                (self.name_reference(e, name)?, JsValue::Undefined)
            } else { self.resolve_reference(t, e)? }
        } else { self.resolve_reference(t, e)? };
        if op == ast::AssignOp::Assign {
            // A named class expression's own self-reference binding needs the name available
            // *before* the class body runs (computed keys/static blocks may already observe it),
            // which the post-hoc `infer_function_name` rename below cannot retroactively supply.
            // Unlike that unconditional legacy heuristic, this one really does need the strict
            // spec restriction (13.15.2's IsIdentifierRef check on the *source* target, not
            // `reference`'s resolved kind: a global binding and `obj.prop` can both resolve to
            // `Reference::Property`, but only the former is an IdentifierRef) -- `holder.C=class
            // {}` must stay unnamed while `C=class{}` (even at global scope) must not.
            let mut v = match event_handler::identifier_name(t) {
                Some(name) => self.eval_named_class_expr(r, e, &name.into())?,
                None => self.eval_expr(r, e)?,
            };
            // Plain assignment intentionally keeps the pre-existing, value-based heuristic
            // (rename whatever nameless function/class the RHS produced -- no source-expression
            // check, no IsIdentifierRef/Reference::Binding restriction) rather than switching to
            // precise NamedEvaluation here: `amazing-js/src/vm` (js-vm-owned) mirrors this exact
            // unconditional heuristic for its own bytecode `=` path, and
            // `assignment_preserves_legacy_function_name_inference` differentially checks both
            // backends agree. A global-scope target resolves to `Reference::Property` (via
            // `resolve_reference`'s `global_binding_object` special case), not
            // `Reference::Binding` -- an earlier attempt to restrict this call to
            // `Reference::Binding` only regressed every top-level `name = () => {}` (caught via
            // language/expressions/assignment/fn-name-*.js), so this stays unconditional on the
            // reference kind, matching the name both backends already agree on. A symbol-keyed
            // member target with no description has no derivable name at all (`reference_name`
            // returns `None`, exactly as `amazing-js/src/vm` treats it) -- skipped, not "".
            // (A no-op for a value `eval_named_class_expr` already named above, since
            // `infer_function_name` only renames a still-nameless function/class -- and classes
            // never reach this heuristic for a non-identifier target anyway, matching spec.)
            if let Some(name) = reference_name(&reference) {
                self.infer_function_name(&mut v, &name);
            }
            self.put_reference(reference, v.clone(), e)?;
            return Ok(v);
        }
        if matches!(&reference, Reference::Binding(name) if !e.has(name)) {
            return Err(Throw::reference_error(
                "assignment to an unresolved binding",
            ));
        }
        if op == ast::AssignOp::And && !old.to_boolean() {
            return Ok(old);
        }
        if op == ast::AssignOp::Or && old.to_boolean() {
            return Ok(old);
        }
        if op == ast::AssignOp::Nullish && !old.is_null() && !old.is_undefined() {
            return Ok(old);
        }
        // Same class-self-reference-before-eval need, and same strict IsIdentifierRef gate
        // (13.15.4's LogicalAssignmentEvaluation goes through the same NamedEvaluation
        // definition as plain `=`), as `assign`'s AssignOp::Assign arm above.
        let mut v = match event_handler::identifier_name(t) {
            Some(name) => self.eval_named_class_expr(r, e, &name.into())?,
            None => self.eval_expr(r, e)?,
        };
        // Same legacy, unconditional value-based heuristic as plain `=` (see the comment
        // there) applies to the logical-assignment operators.
        if let Some(name) = reference_name(&reference) {
            self.infer_function_name(&mut v, &name);
        }
        if matches!(
            op,
            ast::AssignOp::And | ast::AssignOp::Or | ast::AssignOp::Nullish
        ) {
            self.put_reference(reference, v.clone(), e)?;
            return Ok(v);
        }
        let bop = match op {
            ast::AssignOp::Add => ast::BinOp::Add,
            ast::AssignOp::Sub => ast::BinOp::Sub,
            ast::AssignOp::Mul => ast::BinOp::Mul,
            ast::AssignOp::Div => ast::BinOp::Div,
            ast::AssignOp::Rem => ast::BinOp::Rem,
            ast::AssignOp::Pow => ast::BinOp::Pow,
            ast::AssignOp::Shl => ast::BinOp::Shl,
            ast::AssignOp::Shr => ast::BinOp::Shr,
            ast::AssignOp::UShr => ast::BinOp::UShr,
            ast::AssignOp::BitAnd => ast::BinOp::BitAnd,
            ast::AssignOp::BitOr => ast::BinOp::BitOr,
            ast::AssignOp::BitXor => ast::BinOp::BitXor,
            _ => ast::BinOp::Add,
        };
        let out = self.binary(bop, old, v)?;
        self.put_reference(reference, out.clone(), e)?;
        Ok(out)
    }

    /// Renames `value` if it's a still-nameless function/class object -- the actual rename
    /// step underneath `named_evaluation` below. Kept at this exact 2-argument signature and
    /// unconditional (no source-expression check): `amazing-js/src/vm` (js-vm-owned) has its
    /// own, independent assignment-naming call site that already depends on this signature.
    fn infer_function_name(&self, value: &mut JsValue, name: &str) {
        if let JsValue::Object(object) = value {
            let mut object = object.borrow_mut();
            // Class names are installed before static elements by NamedEvaluation.
            if object.class_constructor { return; }
            if let Some(function) = &mut object.func {
                if function.name.is_empty() {
                    function.name = name.to_string();
                    object.define("name", Property {
                        value: JsValue::str(name), writable: false, enumerable: false,
                        configurable: true, getter: None, setter: None,
                    }).unwrap();
                }
            }
        }
    }

    /// NamedEvaluation (spec 8.6.3, and its many call sites: LexicalBinding/VariableDeclaration
    /// initializers, AssignmentExpression incl. `&&=`/`||=`/`??=`, destructuring/parameter
    /// defaults, object-literal properties, class fields, `export default`) for the tree-walker
    /// (JSRT-19): only renames when `source` is itself an anonymous function/arrow/class
    /// definition -- `is_anonymous_function_definition` is what makes `x = (0, function(){})`
    /// correctly stay unnamed while `x = (function(){})` gets named. This is deliberately a
    /// separate method from `infer_function_name` (rather than adding the check there) so
    /// `amazing-js/src/vm`'s own, unrelated call site is untouched.
    fn named_evaluation(&self, source: &ast::Expr, value: &mut JsValue, name: &str) {
        if is_anonymous_function_definition(source) {
            self.infer_function_name(value, name);
        }
    }

    // testharness.js exports its API by assigning properties of global_scope.
    // Unresolved lexical names must therefore consult the global object record.
    fn identifier_value(&mut self, env: &EnvHandle, name: &str) -> Result<Option<JsValue>, Throw> {
        match self.scoped_reference(env, name)? {
            None => {
                // Nothing lexical, no object-environment binding, and the raw
                // check in `scoped_reference`'s own global fallback missed it
                // either -- but that raw check doesn't dispatch Proxy `has`
                // traps. Give the real global object one more look through
                // the full `[[HasProperty]]` algorithm before giving up.
                let exists = self.default_has_property(&self.global_obj.clone(), &PropertyKey::from(name))?;
                if exists {
                    let global = JsValue::Object(self.global_obj.clone());
                    self.get_prop(&global, name).map(Some)
                } else {
                    Ok(None)
                }
            }
            Some(Reference::Property(base, key)) => self.get_prop(&base, &key).map(Some),
            Some(_) => {
                if env.is_uninitialized(name) {
                    return Err(Throw::reference_error(format!("{name} is uninitialized")));
                }
                if let Some(global) = env.global_binding_object(name) {
                    return self.get_prop(&JsValue::Object(global), name).map(Some);
                }
                Ok(env.resolve(name))
            }
        }
    }

    fn resolve_reference(
        &mut self,
        target: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<(Reference, JsValue), Throw> {
        match &target.kind {
            ExprKind::Ident(identifier) => {
                let name = &identifier.name;
                match self.scoped_reference(env, name)? {
                    Some(Reference::Property(base, key)) => {
                        let value = self.get_prop(&base, &key)?;
                        Ok((Reference::Property(base, key), value))
                    }
                    Some(reference) => {
                        if env.is_uninitialized(name) {
                            return Err(Throw::reference_error(format!("{name} is uninitialized")));
                        }
                        if let Some(global) = env.global_binding_object(name) {
                            let global = JsValue::Object(global);
                            let value = self.get_prop(&global, name)?;
                            return Ok((Reference::Property(global, name.clone().into()), value));
                        }
                        let value = env.resolve(name).unwrap_or(JsValue::Undefined);
                        Ok((reference, value))
                    }
                    None => {
                        // Same Proxy-aware global fallback as `identifier_value`:
                        // a raw has-property miss everywhere else doesn't mean
                        // the real global object's `[[HasProperty]]` (which may
                        // dispatch through a Proxy) would also miss.
                        let exists = self.default_has_property(
                            &self.global_obj.clone(), &PropertyKey::from(name.as_str()),
                        )?;
                        if exists {
                            let global = JsValue::Object(self.global_obj.clone());
                            let value = self.get_prop(&global, name)?;
                            Ok((Reference::Property(global, name.clone().into()), value))
                        } else {
                            Ok((Reference::Binding(name.clone()), JsValue::Undefined))
                        }
                    }
                }
            }
            ExprKind::Member { obj, prop, .. } => {
                let base = self.eval_expr(obj, env)?;
                if let MemberProp::Private(identifier) = prop {
                    let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                        Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                    })?;
                    let value = private_field_get(self, &base, &name)?;
                    return Ok((Reference::Private(base, name), value));
                }
                let key = match prop {
                    MemberProp::Ident(identifier) => identifier.name.clone().into(),
                    MemberProp::Private(_) => unreachable!("handled above"),
                    MemberProp::Computed(expression) => {
                        let property_value = self.eval_expr(expression, env)?;
                        if base.is_null() || base.is_undefined() {
                            return Err(Throw::type_error(
                                "cannot access a property on nullish value",
                            ));
                        }
                        self.to_property_key(&property_value)?
                    }
                };
                if base.is_null() || base.is_undefined() {
                    return Err(Throw::type_error(
                        "cannot access a property on nullish value",
                    ));
                }
                let value = self.get_prop(&base, &key)?;
                Ok((Reference::Property(base, key), value))
            }
            ExprKind::Paren(inner) => self.resolve_reference(inner, env),
            _ => Err(Throw::reference_error("invalid assignment target")),
        }
    }

    fn put_reference(
        &mut self,
        reference: Reference,
        value: JsValue,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match reference {
            Reference::Binding(name) => {
                if name == "location" && env.shares_binding_frame(&self.global_env, &name) {
                    let window = JsValue::Object(self.global_obj.clone());
                    return self.set_prop(&window, "location", value);
                }
                // A `var`-like global binding (declared or built in, e.g.
                // `undefined`/`NaN`/`Infinity`) is mirrored onto the global
                // object; Env::assign_raw applies the write when the mirrored
                // property is writable but otherwise silently no-ops it while
                // still reporting success, so a non-writable global never
                // throws even in strict mode. Route through the object's own
                // [[Set]] (set_with_receiver), which already has correct
                // per-property writable/accessor semantics, and apply PutValue's
                // strict/sloppy handling of an unsuccessful [[Set]] exactly as
                // Reference::Property does below.
                if let Some(global) = env.global_binding_object(&name) {
                    let key = PropertyKey::from(name.as_str());
                    let receiver = JsValue::Object(global.clone());
                    if self.set_with_receiver(&global, &key, value.clone(), receiver)? {
                        // Keep the env-level binding cell's mirror in sync;
                        // harmless no-op re-write since we just confirmed it
                        // succeeded (reads of a global non-lexical binding go
                        // through the object property directly, not this
                        // cell, but other bindings still fall back to it).
                        let _ = env.assign(&name, value);
                        return Ok(());
                    } else if !self.strict_eval_context {
                        // PutValue ignores an unsuccessful [[Set]] for sloppy references.
                        return Ok(());
                    } else {
                        return Err(Throw::type_error(format!("Cannot assign to read only property '{name}'")));
                    }
                }
                if !env.assign(&name, value.clone())? {
                    let global = JsValue::Object(self.global_obj.clone());
                    self.put_reference(Reference::Property(global, name.into()), value, env)?;
                }
                Ok(())
            }
            Reference::Property(base, key) => {
                let JsValue::Object(object) = &base else {
                    return self.set_prop(&base, &key, value);
                };
                if self.set_with_receiver(object, &key, value, base.clone())? {
                    Ok(())
                } else if !self.strict_eval_context {
                    // PutValue ignores an unsuccessful [[Set]] for sloppy references.
                    Ok(())
                } else {
                    Err(Throw::type_error("cannot assign to property"))
                }
            }
            Reference::Private(base, name) => private_field_set(self, &base, &name, value),
        }
    }
    fn assign_target(&mut self, t: &ast::Expr, v: JsValue, e: &EnvHandle) -> Result<(), Throw> {
        let reference = if let Some(name) = event_handler::identifier_name(t) {
            self.name_reference(e, name)?
        } else { self.resolve_reference(t, e)?.0 };
        self.put_reference(reference, v, e)
    }

    fn assign_pattern_expr(
        &mut self,
        target: &ast::Expr,
        value: &JsValue,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match &target.kind {
            ExprKind::Paren(inner) => self.assign_pattern_expr(inner, value, env),
            ExprKind::Assign {
                op: ast::AssignOp::Assign,
                target,
                value: default,
            } => {
                if value.is_undefined() {
                    let mut fallback = match &target.kind {
                        ExprKind::Ident(identifier) =>
                            self.eval_named_class_expr(default, env, &identifier.name.clone().into())?,
                        _ => self.eval_expr(default, env)?,
                    };
                    if let ExprKind::Ident(identifier) = &target.kind {
                        self.named_evaluation(default, &mut fallback, &identifier.name);
                    }
                    self.assign_pattern_expr(target, &fallback, env)
                } else {
                    self.assign_pattern_expr(target, value, env)
                }
            }
            ExprKind::Array(elements) => {
                let items = self.iterate(value)?;
                let mut index = 0usize;
                for element in elements.iter().flatten() {
                    if let ExprKind::Spread(inner) = &element.kind {
                        let rest = array_values(self, items[index..].to_vec())?;
                        self.assign_pattern_expr(inner, &rest, env)?;
                        index = items.len();
                    } else {
                        let item = items.get(index).cloned().unwrap_or(JsValue::Undefined);
                        self.assign_pattern_expr(element, &item, env)?;
                        index += 1;
                    }
                }
                Ok(())
            }
            ExprKind::Object(elements) => {
                if value.is_null() || value.is_undefined() {
                    return Err(Throw::type_error("cannot destructure nullish value"));
                }
                let mut used = Vec::new();
                for element in elements {
                    match element {
                        ast::ObjElem::Spread(inner) => {
                            let rest = self.object_rest(value, &used)?;
                            self.assign_pattern_expr(inner, &rest, env)?;
                        }
                        ast::ObjElem::Prop(property) => {
                            let key = self.prop_key(&property.key, env)?;
                            used.push(key.clone());
                            let item = self.get_prop(value, &key)?;
                            match &property.value {
                                ast::PropValue::Shorthand(identifier) => {
                                    let reference = ast::Expr::new(
                                        ExprKind::Ident(identifier.clone()),
                                        identifier.span,
                                    );
                                    self.assign_pattern_expr(&reference, &item, env)?;
                                }
                                ast::PropValue::Init(expression) => {
                                    self.assign_pattern_expr(expression, &item, env)?;
                                }
                                ast::PropValue::CoverInitializedName { binding, default } => {
                                    let assigned = if matches!(&item, JsValue::Undefined) {
                                        let mut v = self.eval_named_class_expr(default, env, &binding.name.clone().into())?;
                                        self.named_evaluation(default, &mut v, &binding.name);
                                        v
                                    } else {
                                        item
                                    };
                                    let reference = ast::Expr::new(
                                        ExprKind::Ident(binding.clone()), binding.span,
                                    );
                                    self.assign_pattern_expr(&reference, &assigned, env)?;
                                }
                                ast::PropValue::Method(_) => {
                                    return Err(Throw::reference_error(
                                        "method is not an assignment target",
                                    ));
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            _ => self.assign_target(target, value.clone(), env),
        }
    }
    fn call_expr(
        &mut self,
        c: &ast::Expr,
        args: &[ast::Expr],
        optional: bool,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        if matches!(&c.kind, ExprKind::Super) {
            let frame = self.frames.last().ok_or_else(|| Throw::syntax_error("super() used outside a constructor"))?;
            let constructor = frame.super_constructor.clone().ok_or_else(|| Throw::syntax_error("super() used outside a derived constructor"))?;
            let new_target = frame.new_target.clone().ok_or_else(|| Throw::type_error("derived constructor requires new"))?;
            let own_function = frame.own_function.clone();
            let argv = self.eval_args(args, e)?;
            let result = self.construct_with_new_target(&constructor, argv, new_target)?;
            e.initialize_this(result.clone())?;
            if let Some(frame) = self.frames.last_mut() {
                frame.this = result.clone();
            }
            if let Some(own_function) = own_function {
                if let Some(elements) = own_function.borrow().class_elements.clone() {
                    initialize_instance_elements(self, &result, &elements)?;
                }
            }
            return Ok(result);
        }
        let (f, this) = if let ExprKind::Member {
            obj,
            prop,
            optional: m,
        } = &c.kind
        {
            let o = self.eval_expr(obj, e)?;
            if (optional || *m) && (o.is_null() || o.is_undefined()) {
                return Ok(JsValue::Undefined);
            }
            let receiver = if matches!(&obj.kind, ExprKind::Super) {
                e.resolve_this_checked()?
            } else {
                o.clone()
            };
            let function = if let MemberProp::Private(identifier) = prop {
                let name = e.resolve_private(&identifier.name).ok_or_else(|| {
                    Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                })?;
                private_field_get(self, &receiver, &name)?
            } else {
                let k = self.member_key(prop, e)?;
                self.get_prop(&o, &k)?
            };
            (function, receiver)
        } else if let Some(name) = event_handler::identifier_name(c) {
            self.scoped_call(e, name)?
        } else {
            (self.eval_expr(c, e)?, JsValue::Undefined)
        };
        if optional && (f.is_null() || f.is_undefined()) {
            return Ok(JsValue::Undefined);
        }
        let argv = self.eval_args(args, e)?;
        let previous_direct_eval = self.direct_eval_call;
        let previous_direct_eval_env = self.direct_eval_env.take();
        self.direct_eval_call = !optional && matches!(&c.kind, ExprKind::Ident(name) if name.name == "eval") && self.is_intrinsic_eval(&f);
        self.direct_eval_env = self.direct_eval_call.then(|| e.clone());
        let result = self.call_value(&f, this, argv);
        self.direct_eval_call = previous_direct_eval;
        self.direct_eval_env = previous_direct_eval_env;
        result
    }
    fn eval_args(&mut self, xs: &[ast::Expr], e: &EnvHandle) -> Result<Vec<JsValue>, Throw> {
        let mut out = Vec::new();
        for x in xs {
            if let ExprKind::Spread(y) = &x.kind {
                let v = self.eval_expr(y, e)?;
                out.extend(self.iterate(&v)?)
            } else {
                out.push(self.eval_expr(x, e)?);
            }
        }
        Ok(out)
    }
    fn member_key(&mut self, p: &MemberProp, e: &EnvHandle) -> Result<PropertyKey, Throw> {
        match p {
            MemberProp::Ident(i) => Ok(i.name.clone().into()),
            MemberProp::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            MemberProp::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }
    fn prop_key(&mut self, p: &PropKey, e: &EnvHandle) -> Result<PropertyKey, Throw> {
        match p {
            PropKey::Ident(i) => Ok(i.name.clone().into()),
            PropKey::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            PropKey::Str(s) => Ok(s.clone().into()),
            PropKey::Num(n) => Ok(number_to_string(self.numeric(n)?.to_number()?).into()),
            PropKey::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }

    pub fn call_value(
        &mut self,
        f: &JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    ) -> Result<JsValue, Throw> {
        self.call_value_with_new_target(f, this, args, None).map_err(|th| self.normalize_throw(th))
    }

    fn call_value_with_new_target_inner(
        &mut self,
        f: &JsValue,
        this: JsValue,
        args: Vec<JsValue>,
        new_target: Option<JsValue>,
        caller_realm: RealmHandle,
    ) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("value is not callable")),
        };
        if matches!(o.borrow().exotic, Exotic::Proxy(_)) {
            return match new_target {
                Some(nt) => self.proxy_construct(&o, args, nt),
                None => self.proxy_call(&o, this, args),
            };
        }
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("value is not callable"))?;
        let strict_function = o.borrow().strict_function;
        let interpreted = matches!(&fun.body, FunctionBody::Block(_) | FunctionBody::Expr(_) | FunctionBody::Bytecode(_));
        let this = if interpreted && !fun.is_arrow && !strict_function {
            match this {
                JsValue::Null | JsValue::Undefined => self.global_env.global_scope().resolve_this(),
                JsValue::Object(_) => this,
                primitive => native_object(self, JsValue::Undefined, vec![primitive])?,
            }
        } else { this };
        let derived_constructor = o.borrow().derived_constructor.clone();
        if o.borrow().class_constructor && new_target.is_none() {
            return Err(Throw::type_error("class constructor requires new"));
        }
        let lexical_super_constructor = o.borrow().lexical_super_constructor.clone();
        let lexical_new_target = o.borrow().lexical_new_target.clone();
        let async_body = self.async_body_code(&o, &fun, strict_function);
        let bytecode_async = fun.is_async && !fun.is_generator
            && (async_body.is_some() || matches!(&fun.body, FunctionBody::Bytecode(_)));
        let async_generator = fun.is_async && fun.is_generator;
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Throw::range_error("maximum call stack size exceeded"));
        }
        self.depth += 1;
        self.frames.push(CallFrame {
            this: this.clone(),
            home: fun.home.clone(),
            super_constructor: derived_constructor.as_ref().map(|meta| meta.parent.clone()).or(lexical_super_constructor),
            new_target: new_target.clone().or(lexical_new_target.clone()),
            own_function: Some(o.clone()),
        });
        let previous_completion = self.completion.clone();
        let previous_strict = self.strict_eval_context;
        if interpreted { self.strict_eval_context = strict_function; }
        self.completion = JsValue::Undefined;
        let r = (|| -> Result<JsValue, Throw> {
            match fun.body {
                FunctionBody::Bytecode(code) => {
                    // The generator's own object, not a pre-read prototype: a
                    // parameter default can replace `functionObject.prototype`
                    // during FunctionDeclarationInstantiation, and per spec
                    // (OrdinaryCreateFromConstructor) that later value is what
                    // the created generator's `[[Prototype]]` must observe, not
                    // whatever "prototype" held before parameters were bound.
                    // `call` reads it fresh after binding parameters.
                    code.call(
                        self, fun.env.clone(), fun.is_arrow, this, args,
                        fun.is_generator.then(|| o.clone()),
                        derived_constructor.clone(), new_target.clone().or(lexical_new_target.clone()),
                    )
                }
                FunctionBody::Native(n) => {
                    self.native_stack.push(o.clone());
                    let result = n(self, this, args);
                    self.native_stack.pop();
                    result
                }
                FunctionBody::HostNative(id, policy, realm) => {
                    if realm != self.host_realm {
                        return Err(Throw::type_error("host function belongs to another interpreter"));
                    }
                    let constructing = new_target.is_some();
                    if (constructing && policy == HostCallPolicy::CallOnly)
                        || (!constructing && policy == HostCallPolicy::ConstructOnly)
                    {
                        return Err(Throw::type_error(if constructing {
                            "host function is not a constructor"
                        } else {
                            "host constructor requires new"
                        }));
                    }
                    let dispatch = self.host_native_dispatch.clone().ok_or_else(|| {
                        Throw::type_error("host function dispatcher is not installed")
                    })?;
                    let result = dispatch(self, id, this, args, new_target);
                    self.flush_native_microtasks()?;
                    result
                }
                FunctionBody::Expr(x) => {
                    let e = EnvHandle::new(if fun.is_arrow { EnvKind::Arrow } else { EnvKind::Function }, Some(fun.env.clone()));
                    if !fun.is_arrow { e.bind_this(this); }
                    self.bind_params(&e, &fun.params, args, fun.is_arrow)?;
                    if let Some(code) = &async_body { return code.call_async_with_env(self, e); }
                    self.eval_expr(&x, &e)
                }
                FunctionBody::Block(b) => {
                    let e = EnvHandle::new(if fun.is_arrow { EnvKind::Arrow } else { EnvKind::Function }, Some(fun.env.clone()));
                    let derived = derived_constructor.is_some();
                    if derived {
                        if new_target.is_none() {
                            return Err(Throw::type_error("derived constructor requires new"));
                        }
                        e.mark_this_uninitialized();
                        if derived_constructor.as_ref().is_some_and(|meta| meta.implicit) {
                            let parent = &derived_constructor.as_ref().expect("derived parent").parent;
                            let bound = self.construct_with_new_target(parent, args.clone(), new_target.clone().expect("newTarget"))?;
                            e.initialize_this(bound.clone())?;
                            if let Some(frame) = self.frames.last_mut() {
                                frame.this = bound.clone();
                            }
                            if let Some(elements) = o.borrow().class_elements.clone() {
                                initialize_instance_elements(self, &bound, &elements)?;
                            }
                        }
                    } else {
                        if !fun.is_arrow { e.bind_this(this.clone()); }
                        if new_target.is_some() {
                            if let Some(elements) = o.borrow().class_elements.clone() {
                                initialize_instance_elements(self, &this, &elements)?;
                            }
                        }
                    }
                    self.bind_params(&e, &fun.params, args, fun.is_arrow)?;
                    if let Some(code) = &async_body { return code.call_async_with_env(self, e); }
                    self.hoist(&b.body, &e)?;
                    // A function body is a disposal boundary in its own right (spec: ordinary
                    // FunctionDeclarationInstantiation's disposal capability), independent of any
                    // nested block's own boundary.
                    let has_using = b.body.iter().any(stmt_declares_resource);
                    let completion = if has_using {
                        self.using_stack.push(Vec::new());
                        let result = self.exec_stmts(&b.body, &e);
                        let frame = self.using_stack.pop().unwrap_or_default();
                        self.dispose_frame(frame, result)?
                    } else {
                        self.exec_stmts(&b.body, &e)?
                    };
                    if derived {
                        self.complete_derived_constructor(completion, &e, caller_realm)
                    } else {
                        match completion {
                            Flow::Return(value) => Ok(value),
                            _ => Ok(JsValue::Undefined),
                        }
                    }
                }
            }
        })();
        self.completion = previous_completion;
        self.strict_eval_context = previous_strict;
        self.frames.pop();
        self.depth -= 1;
        if async_generator {
            // Parameter binding runs when an async generator is called. An
            // abrupt binding completion is synchronous, before any iterator
            // result promise exists.
            r
        } else if bytecode_async {
            match r {
                Ok(value) => Ok(value),
                Err(Throw(value)) => Ok(new_promise(self, PromiseFate::Rejected(value))),
            }
        } else if fun.is_async {
            match r {
                Ok(value) => Ok(new_promise(self, PromiseFate::Fulfilled(value))),
                Err(Throw(value)) => Ok(new_promise(self, PromiseFate::Rejected(value))),
            }
        } else {
            r
        }
    }
    fn bind_params(
        &mut self,
        e: &EnvHandle,
        ps: &[ast::Param],
        args: Vec<JsValue>,
        is_arrow: bool,
    ) -> Result<(), Throw> {
        // Parameters are mutable function bindings, not lexical declarations:
        // body var declarations reuse them and sloppy duplicate names overwrite.
        // Allocate locally before binding patterns so assignment cannot escape
        // into a same-named binding in the enclosing environment.
        let mut names = std::collections::HashSet::new();
        for p in ps {
            for name in binding_names(&p.binding) {
                if names.insert(name.clone()) {
                    e.declare_uninitialized(&name, true)?;
                }
            }
        }
        for (i, p) in ps.iter().enumerate() {
            let v = if p.rest {
                array_values(self, args.iter().skip(i).cloned().collect())?
            } else {
                args.get(i).cloned().unwrap_or(JsValue::Undefined)
            };
            let v = if v.is_undefined() {
                if let Some(default) = &p.default {
                    let mut d = match &p.binding {
                        Binding::Ident(identifier) =>
                            self.eval_named_class_expr(default, e, &identifier.name.clone().into())?,
                        _ => self.eval_expr(default, e)?,
                    };
                    if let Binding::Ident(identifier) = &p.binding {
                        self.named_evaluation(default, &mut d, &identifier.name);
                    }
                    d
                } else { v }
            } else { v };
            self.bind_pattern_mode(&p.binding, v, None, e)?;
        }
        for name in &names { e.finish_parameter_initialization(name); }
        if is_arrow || names.contains("arguments") { return Ok(()); }
        let argument_count = args.len();
        let a = self.object_value();
        for (i, v) in args.into_iter().enumerate() {
            self.set_prop(&a, &i.to_string(), v)?;
        }
        if let JsValue::Object(object) = &a {
            object.borrow_mut().define("length", Property {
                value: JsValue::num(argument_count as f64), writable: true,
                enumerable: false, configurable: true, getter: None, setter: None,
            }).unwrap();
            if let Some(values) = self.array_proto.borrow().get_own("values") {
                object.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), Property {
                    value: values, writable: true, enumerable: false, configurable: true,
                    getter: None, setter: None,
                }).unwrap();
            }
        }
        e.declare_var("arguments", a)?;
        Ok(())
    }
    fn construct(&mut self, f: &JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        self.construct_with_new_target(f, args, f.clone())
    }
    fn construct_with_new_target(&mut self, f: &JsValue, args: Vec<JsValue>, new_target: JsValue) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("constructor is not an object")),
        };
        if matches!(o.borrow().exotic, Exotic::Proxy(_)) {
            return self.proxy_construct(&o, args, new_target);
        }
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("not a constructor"))?;
        if matches!(&fun.body, FunctionBody::Native(native)
            if std::ptr::fn_addr_eq(*native, bigint_constructor as crate::value::NativeFn)) {
            return Err(Throw::type_error("BigInt is not a constructor"));
        }
        if fun.is_arrow {
            return Err(Throw::type_error("arrow is not a constructor"));
        }
        if !o.borrow().constructible {
            return Err(Throw::type_error("function is not a constructor"));
        }
        if Self::is_bound_function(f) {
            return self.invoke_bound_function(f, args, Some(new_target));
        }
        let derived = o.borrow().derived_constructor.is_some();
        if derived {
            let result = self.call_value_with_new_target(
                f,
                JsValue::Undefined,
                args,
                Some(new_target),
            )?;
            return if result.is_object() {
                Ok(result)
            } else {
                Err(Throw::type_error("derived constructor returned a non-object"))
            };
        }
        if !new_target.is_object() { return Err(Throw::type_error("newTarget is not an object")); }
        let proto = match self.get_prop(&new_target, "prototype")? {
            JsValue::Object(p) => p,
            _ => self.constructor_fallback_prototype(f, &new_target),
        };
        let obj = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(proto.clone());
            x.constructed = true;
            x
        })));
        let v = self.call_value_with_new_target(
            f,
            JsValue::Object(obj.clone()),
            args,
            Some(new_target.clone()),
        )?;
        if v.is_object() {
            // Native and host constructors may return an existing object. A
            // subclass still supplies its prototype through newTarget; in
            // particular, a canonical DOM wrapper must retain its identity.
            if matches!(fun.body, FunctionBody::Native(_) | FunctionBody::HostNative(..)) && new_target != *f {
                if let JsValue::Object(result) = &v {
                    result.borrow_mut().proto = Some(proto);
                }
            }
            Ok(v)
        } else {
            Ok(JsValue::Object(obj))
        }
    }
    fn make_function(&self, f: &ast::FunctionItem, e: &EnvHandle) -> JsValue {
        if f.is_generator {
            if let Ok(code) = vm::compile_function_item(f, f.body.span, self.strict_eval_context) {
                return code.instantiate(self, e);
            }
        }
        let mut o = JsObject::function(JsFunction {
            name: f.name.as_ref().map(|x| x.name.clone()).unwrap_or_default(),
            params: f.params.clone(),
            body: FunctionBody::Block(f.body.clone()),
            is_arrow: false,
            is_async: f.is_async,
            is_generator: f.is_generator,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(if f.is_async && f.is_generator {
            self.async_generator_function_proto.clone()
        } else if f.is_async {
            self.async_function_proto.clone()
        } else if f.is_generator {
            self.generator_function_proto.clone()
        } else {
            self.function_proto.clone()
        });
        o.strict_function = self.strict_eval_context || has_use_strict_directive(&f.body.body);
        // Async and generator function instances have no [[Construct]]
        // internal method (25.5.1/27.3.1/27.4.1): only a plain function is
        // constructible.
        o.constructible = !(f.is_async || f.is_generator);
        let h = Obj(Rc::new(RefCell::new(o)));
        // A plain (non-generator) async function has no "prototype"
        // property at all (25.5.3 Async Function Instances): [[Construct]]
        // is unreachable and there's nowhere to stash new.target-derived
        // instance state. Sync and async generators keep their own
        // `.prototype` (that's where a constructed generator's state would
        // live), matching the VM's equivalent narrowed exclusion.
        if !(f.is_async && !f.is_generator) {
            let p = Obj(Rc::new(RefCell::new({
                let mut x = JsObject::ordinary();
                // A (sync or async) generator's own `.prototype` inherits
                // from the shared %[Async]GeneratorPrototype%, not
                // Object.prototype (GeneratorFunction/instance-prototype.js).
                x.proto = Some(if f.is_generator {
                    if f.is_async { self.async_generator_proto.clone() } else { self.generator_proto.clone() }
                } else {
                    self.object_proto.clone()
                });
                x
            })));
            if !f.is_generator {
                p.borrow_mut().define("constructor", symbol_data(JsValue::Object(h.clone()), true, true)).unwrap();
            }
            // A function's own "prototype" is {writable:true, enumerable:
            // false, configurable:false} (20.2.4.3 for ordinary functions;
            // 27.5.3/27.4.3 identically for generators) -- `set_own` would
            // give it Property::data's ordinary-property defaults
            // (enumerable:true, configurable:true), which is wrong for both
            // kinds, not just generators.
            h.borrow_mut().define("prototype", Property {
                value: JsValue::Object(p.clone()), writable: true, enumerable: false,
                configurable: false, getter: None, setter: None,
            }).unwrap();
            h.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(p);
        }
        JsValue::Object(h)
    }
    fn make_arrow(&self, a: &ast::ArrowFunc, e: &EnvHandle) -> JsValue {
        let body = match &a.body {
            ast::ArrowBody::Expr(x) => FunctionBody::Expr((**x).clone()),
            ast::ArrowBody::Block(b) => FunctionBody::Block(b.clone()),
        };
        let mut o = JsObject::function(JsFunction {
            name: "".into(),
            params: a.params.clone(),
            body,
            is_arrow: true,
            is_async: a.is_async,
            is_generator: false,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        // An arrow has no `[[Prototype]]` special-case of its own: it is
        // %AsyncFunction.prototype% when async, matching `make_function`
        // and the VM's `instantiate` (arrows are never generators, so that
        // combination does not apply here). This had been hardcoded to
        // `function_proto` for every arrow, sync or async.
        o.proto = Some(if a.is_async {
            self.async_function_proto.clone()
        } else {
            self.function_proto.clone()
        });
        o.strict_function = self.strict_eval_context || matches!(&a.body, ast::ArrowBody::Block(block) if has_use_strict_directive(&block.body));
        let handle = Obj(Rc::new(RefCell::new(o)));
        if let Some(frame) = self.frames.last() {
            let mut arrow = handle.borrow_mut();
            arrow.lexical_super_constructor = frame.super_constructor.clone();
            arrow.lexical_new_target = frame.new_target.clone();
        }
        JsValue::Object(handle)
    }
    fn method_value(&self, m: &ast::ClassMethod, e: &EnvHandle, class_method: bool) -> JsValue {
        if m.kind != ast::MethodKind::Constructor {
            if let Ok(code) = vm::compile_class_method(m, class_method || self.strict_eval_context) {
                let value = code.instantiate(self, e);
                if let JsValue::Object(function) = &value {
                    let mut function = function.borrow_mut();
                    function.constructible = false;
                    function.strict_function = class_method
                        || self.strict_eval_context
                        || has_use_strict_directive(&m.body.body);
                    function.proto = Some(if m.is_async && m.is_generator {
                        self.async_generator_function_proto.clone()
                    } else if m.is_async {
                        self.async_function_proto.clone()
                    } else if m.is_generator {
                        self.generator_function_proto.clone()
                    } else {
                        self.function_proto.clone()
                    });
                    if !m.is_generator {
                        function.delete_own("prototype");
                    }
                }
                return value;
            }
        }
        let mut o = JsObject::function(JsFunction {
            name: "".into(),
            params: m.params.clone(),
            body: FunctionBody::Block(m.body.clone()),
            is_arrow: false,
            is_async: m.is_async,
            is_generator: m.is_generator,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(if m.is_async && m.is_generator {
            self.async_generator_function_proto.clone()
        } else if m.is_async {
            self.async_function_proto.clone()
        } else if m.is_generator {
            self.generator_function_proto.clone()
        } else {
            self.function_proto.clone()
        });
        o.strict_function = class_method || self.strict_eval_context || has_use_strict_directive(&m.body.body);
        let handle = Obj(Rc::new(RefCell::new(o)));
        JsValue::Object(handle)
    }
    fn eval_class(&mut self, c: &ast::ClassItem, e: &EnvHandle) -> Result<JsValue, Throw> {
        self.eval_class_named(c, e, None)
    }

    // NamedEvaluation (§15.7.14 ClassTail) supplies an anonymous class's name
    // before static work runs. It does not create a class-local lexical
    // binding for the inferred name (only an explicit class name gets one).
    fn eval_named_class_expr(&mut self, expression: &ast::Expr, env: &EnvHandle,
        name: &crate::value::JsString) -> Result<JsValue, Throw>
    {
        match &expression.kind {
            ExprKind::Paren(inner) => self.eval_named_class_expr(inner, env, name),
            ExprKind::Class(class) if class.name.is_none() =>
                self.eval_class_named(class, env, Some(name)),
            _ => self.eval_expr(expression, env),
        }
    }

    // Shared by the VM's MakeClass instruction, which defines each compiled
    // class method the same way the tree-walk evaluator does: a get/set pair
    // sharing one accessor descriptor, everything else a plain data method.
    fn define_class_method(&mut self, target: &JsValue, key: &PropertyKey,
        kind: ast::MethodKind, value: JsValue) -> Result<(), Throw>
    {
        let JsValue::Object(object) = target else { unreachable!() };
        let mut descriptor = object.borrow().own_prop(key).cloned().unwrap_or(Property {
            value: JsValue::Undefined, writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
        if matches!(kind, ast::MethodKind::Get | ast::MethodKind::Set) {
            descriptor.writable = false;
            if kind == ast::MethodKind::Get { descriptor.getter = Some(value); }
            else { descriptor.setter = Some(value); }
        } else {
            descriptor = Property {
                value, writable: true, enumerable: false, configurable: true,
                getter: None, setter: None,
            };
        }
        object.borrow_mut().define(key, descriptor)
            .map_err(|_| Throw::type_error("could not define class method"))
    }

    fn class_binding_name(name: &str) -> crate::value::JsString {
        // The module front end lowers anonymous default exports into this slot.
        if name == "\0module.default" { "default".into() } else { name.into() }
    }

    fn property_function_name(key: &PropertyKey) -> crate::value::JsString {
        match key {
            PropertyKey::String(name) => name.clone(),
            PropertyKey::Symbol(symbol) => match symbol.description() {
                Some(description) => {
                    let mut units = vec![b'[' as u16];
                    units.extend_from_slice(description.units());
                    units.push(b']' as u16);
                    crate::value::JsString::from_units(units)
                }
                None => crate::value::JsString::from(""),
            },
        }
    }

    // SetFunctionName (§10.2.9) plus the length recomputation a class method
    // needs on top of it: an accessor's own `name` is "get "/"set " prefixed,
    // and `length` counts the method's own leading required parameters
    // rather than the constructor-style default `method_value` gave it.
    fn name_class_method(method: &mut JsValue, base_name: &str, kind: ast::MethodKind, params: &[ast::Param]) {
        let JsValue::Object(function) = method else { return };
        let name = match kind {
            ast::MethodKind::Get => format!("get {base_name}"),
            ast::MethodKind::Set => format!("set {base_name}"),
            _ => base_name.to_string(),
        };
        let length = params.iter().take_while(|param| !param.rest && param.default.is_none()).count();
        let mut function = function.borrow_mut();
        function.func.as_mut().unwrap().name = name.clone();
        function.define("name", Property {
            value: JsValue::str(name), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).expect("method name property");
        function.define("length", Property {
            value: JsValue::num(length as f64), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).expect("method length property");
    }

    fn eval_class_named(&mut self, c: &ast::ClassItem, e: &EnvHandle,
        inferred_name: Option<&crate::value::JsString>) -> Result<JsValue, Throw>
    {
        let previous_strict = self.strict_eval_context;
        self.strict_eval_context = true;
        let result = self.eval_class_inner(c, e, inferred_name);
        self.strict_eval_context = previous_strict;
        result
    }

    fn eval_class_inner(&mut self, c: &ast::ClassItem, e: &EnvHandle,
        inferred_name: Option<&crate::value::JsString>) -> Result<JsValue, Throw> {
        let ctor = c.members.iter().find_map(|m| {
            if let ast::ClassMember::Method(x) = m {
                (x.kind == ast::MethodKind::Constructor).then_some(x)
            } else {
                None
            }
        });
        // NewPrivateEnvironment (§9.2): every method, field initializer, and
        // static block of this class captures `class_env`, not the raw outer
        // scope, so `#name` inside any of them resolves through it.
        let class_env = EnvHandle::new(EnvKind::Block, Some(e.clone()));
        class_env.set_class_private(collect_private_names(&c.members));
        let f = if let Some(x) = ctor {
            self.method_value(x, &class_env, true)
        } else {
            self.make_function(
                &ast::FunctionItem {
                    name: c.name.clone(),
                    params: Vec::new(),
                    body: ast::Block {
                        body: Vec::new(),
                        span: crate::tokens::Span::new(
                            crate::tokens::SourceLocation::new(1, 1),
                            crate::tokens::SourceLocation::new(1, 1),
                        ),
                    },
                    is_async: false,
                    is_generator: false,
                },
                &class_env,
            )
        };
        if let JsValue::Object(o) = &f {
            o.borrow_mut().class_constructor = true;
            if o.borrow().get_own("prototype").is_none() {
                let prototype = Obj(Rc::new(RefCell::new({
                    let mut object = JsObject::ordinary();
                    object.proto = Some(self.object_proto.clone());
                    object
                })));
                o.borrow_mut()
                    .set_own("prototype", JsValue::Object(prototype.clone()));
                o.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(prototype);
            }
            let prototype = o.borrow().get_own("prototype");
            if let Some(JsValue::Object(p)) = prototype {
                p.borrow_mut().define("constructor", symbol_data(f.clone(), true, true)).unwrap();
                if let Some(super_class) = &c.super_class {
                    let super_value = self.eval_expr(super_class, &class_env)?;
                    let super_prototype = match &super_value {
                        JsValue::Object(super_constructor) => match super_constructor
                            .borrow()
                            .get_own("prototype")
                        {
                            Some(JsValue::Object(prototype)) => prototype,
                            _ => {
                                return Err(Throw::type_error("super constructor has no prototype"))
                            }
                        },
                        _ => return Err(Throw::type_error("super class is not a constructor")),
                    };
                    p.borrow_mut().proto = Some(super_prototype);
                    o.borrow_mut().proto = match &super_value {
                        JsValue::Object(parent) => Some(parent.clone()),
                        _ => unreachable!(),
                    };
                    o.borrow_mut().derived_constructor = Some(DerivedConstructor {
                        parent: super_value,
                        implicit: ctor.is_none(),
                    });
                }
                let mut class_function = o.borrow_mut();
                if let Some(function) = &mut class_function.func {
                    function.home = Some(p.clone());
                }
                drop(class_function);
                // An explicit class name gets its own class-local lexical
                // binding (so a method/field/static block can reference the
                // class by name even before any outer binding is initialized);
                // NamedEvaluation's inferred name never creates one.
                if let Some(name) = &c.name {
                    class_env.declare_lexical(&name.name, f.clone(), false)?;
                }
                let class_name = c.name.as_ref().map(|name| crate::value::JsString::from(name.name.clone()))
                    .or_else(|| inferred_name.cloned()).unwrap_or_else(|| crate::value::JsString::from(""));
                o.borrow_mut().func.as_mut().unwrap().name = class_name.to_utf8_lossy();
                o.borrow_mut().define("name", Property {
                    value: JsValue::str(class_name), writable: false, enumerable: false,
                    configurable: true, getter: None, setter: None,
                }).expect("class name property");
                let mut instance_elements: Vec<InstanceElement> = Vec::new();
                let mut static_elements: Vec<StaticElement> = Vec::new();
                for m in &c.members {
                    match m {
                        ast::ClassMember::Method(x) if x.kind != ast::MethodKind::Constructor => {
                            let target_home = if x.is_static { o.clone() } else { p.clone() };
                            let mut method = self.method_value(x, &class_env, true);
                            if let JsValue::Object(method_object) = &method {
                                method_object.borrow_mut().func.as_mut().unwrap().home = Some(target_home.clone());
                            }
                            if let ast::PropKey::Private(identifier) = &x.key {
                                // Private identifier text already includes the leading '#'.
                                Self::name_class_method(&mut method, &identifier.name, x.kind, &x.params);
                                let name = class_env
                                    .resolve_private(&identifier.name)
                                    .expect("collect_private_names recorded every private key");
                                let is_getter = x.kind == ast::MethodKind::Get;
                                let is_accessor = matches!(x.kind, ast::MethodKind::Get | ast::MethodKind::Set);
                                if x.is_static {
                                    upsert_static_private_accessor_or_method(
                                        &mut static_elements, name, is_accessor, is_getter, method,
                                    );
                                } else {
                                    upsert_instance_private_accessor_or_method(
                                        &mut instance_elements, name, is_accessor, is_getter, method,
                                    );
                                }
                                continue;
                            }
                            let k = match &x.key_utf16 {
                                Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
                                None => self.prop_key(&x.key, &class_env)?,
                            };
                            let key_name = match &k {
                                PropertyKey::String(s) => s.to_utf8_lossy(),
                                PropertyKey::Symbol(symbol) => symbol.descriptive_string(),
                            };
                            Self::name_class_method(&mut method, &key_name, x.kind, &x.params);
                            let target = if x.is_static { &o } else { &p };
                            if matches!(x.kind, ast::MethodKind::Get | ast::MethodKind::Set) {
                                let mut property =
                                    target.borrow().own_prop(&k).cloned().unwrap_or(Property {
                                        value: JsValue::Undefined,
                                        writable: false,
                                        enumerable: false,
                                        configurable: true,
                                        getter: None,
                                        setter: None,
                                    });
                                if x.kind == ast::MethodKind::Get {
                                    property.getter = Some(method);
                                } else {
                                    property.setter = Some(method);
                                }
                                target.borrow_mut().define(&k, property).map_err(|_| {
                                    Throw::type_error("could not define class accessor")
                                })?;
                            } else {
                                target.borrow_mut().define(k, symbol_data(method, true, true)).unwrap();
                            }
                        }
                        ast::ClassMember::Method(_) => {}
                        ast::ClassMember::Field(field) => {
                            let key = if let ast::PropKey::Private(identifier) = &field.key {
                                FieldKey::Private(
                                    class_env
                                        .resolve_private(&identifier.name)
                                        .expect("collect_private_names recorded every private key"),
                                )
                            } else {
                                FieldKey::Public(self.prop_key(&field.key, &class_env)?)
                            };
                            let init = field.init.clone();
                            if field.is_static {
                                static_elements.push(StaticElement::Field { key, init });
                            } else {
                                instance_elements.push(InstanceElement::Field { key, init });
                            }
                        }
                        ast::ClassMember::StaticBlock(block) => {
                            static_elements.push(StaticElement::Block(block.clone()));
                        }
                    }
                }
                if !instance_elements.is_empty() {
                    o.borrow_mut().class_elements = Some(Rc::new(ClassElements {
                        instance: instance_elements,
                        class_env: class_env.clone(),
                        home: p.clone(),
                    }));
                }
                run_static_elements(self, &f, o.clone(), &class_env, &static_elements)?;
            }
        }
        Ok(f)
    }

    fn object_value(&self) -> JsValue {
        let mut o = JsObject::ordinary();
        o.proto = Some(self.object_proto.clone());
        JsValue::object(o)
    }

    fn make_dom_document(&self) -> JsValue {
        let mut document = JsObject::ordinary();
        document.proto = Some(self.object_proto.clone());
        document.set_own("[[DomNode]]", JsValue::num(0.0));
        document.set_own(
            "querySelector",
            Self::native(
                "querySelector",
                native_dom_query_selector,
                &self.function_proto,
            ),
        );
        document.set_own(
            "querySelectorAll",
            Self::native(
                "querySelectorAll",
                native_dom_query_selector_all,
                &self.function_proto,
            ),
        );
        document.set_own(
            "getElementById",
            Self::native(
                "getElementById",
                native_dom_get_element_by_id,
                &self.function_proto,
            ),
        );
        document.set_own(
            "getElementsByTagName",
            Self::native(
                "getElementsByTagName",
                native_dom_get_elements_by_tag,
                &self.function_proto,
            ),
        );
        document.set_own(
            "getElementsByClassName",
            Self::native(
                "getElementsByClassName",
                native_dom_get_elements_by_class,
                &self.function_proto,
            ),
        );
        document.set_own(
            "createElement",
            Self::native(
                "createElement",
                native_dom_create_element,
                &self.function_proto,
            ),
        );
        document.set_own(
            "createTextNode",
            Self::native(
                "createTextNode",
                native_dom_create_text_node,
                &self.function_proto,
            ),
        );
        document.set_own(
            "addEventListener",
            Self::native(
                "addEventListener",
                native_dom_add_event_listener,
                &self.function_proto,
            ),
        );
        document.set_own(
            "removeEventListener",
            Self::native(
                "removeEventListener",
                native_dom_remove_event_listener,
                &self.function_proto,
            ),
        );
        let title_getter = Self::native("get title", native_dom_title_get, &self.function_proto);
        let title_setter = Self::native("set title", native_dom_title_set, &self.function_proto);
        let _ = document.define(
            "title",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(title_getter),
                setter: Some(title_setter),
            },
        );
        JsValue::Object(Obj(Rc::new(RefCell::new(document))))
    }

    fn make_dom_element(&self, id: u32) -> JsValue {
        let mut element = JsObject::ordinary();
        element.proto = Some(self.object_proto.clone());
        element.set_own("[[DomNode]]", JsValue::num(id as f64));
        let getter = Self::native("get textContent", native_dom_text_get, &self.function_proto);
        let setter = Self::native("set textContent", native_dom_text_set, &self.function_proto);
        let _ = element.define(
            "textContent",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(getter),
                setter: Some(setter),
            },
        );
        let inner_getter = Self::native("get innerText", native_dom_text_get, &self.function_proto);
        let inner_setter = Self::native("set innerText", native_dom_text_set, &self.function_proto);
        let _ = element.define(
            "innerText",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(inner_getter),
                setter: Some(inner_setter),
            },
        );
        let inner_getter = Self::native(
            "get innerHTML",
            native_dom_inner_html_get,
            &self.function_proto,
        );
        let inner_setter = Self::native(
            "set innerHTML",
            native_dom_inner_html_set,
            &self.function_proto,
        );
        let _ = element.define(
            "innerHTML",
            Property {
                value: JsValue::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(inner_getter),
                setter: Some(inner_setter),
            },
        );
        element.set_own(
            "getAttribute",
            Self::native(
                "getAttribute",
                native_dom_get_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "setAttribute",
            Self::native(
                "setAttribute",
                native_dom_set_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "hasAttribute",
            Self::native(
                "hasAttribute",
                native_dom_has_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "removeAttribute",
            Self::native(
                "removeAttribute",
                native_dom_remove_attribute,
                &self.function_proto,
            ),
        );
        element.set_own(
            "append",
            Self::native("append", native_dom_append, &self.function_proto),
        );
        element.set_own(
            "appendChild",
            Self::native("appendChild", native_dom_append_child, &self.function_proto),
        );
        element.set_own(
            "remove",
            Self::native("remove", native_dom_remove, &self.function_proto),
        );
        element.set_own(
            "matches",
            Self::native("matches", native_dom_matches, &self.function_proto),
        );
        element.set_own(
            "closest",
            Self::native("closest", native_dom_closest, &self.function_proto),
        );
        element.set_own(
            "addEventListener",
            Self::native(
                "addEventListener",
                native_dom_add_event_listener,
                &self.function_proto,
            ),
        );
        element.set_own(
            "removeEventListener",
            Self::native(
                "removeEventListener",
                native_dom_remove_event_listener,
                &self.function_proto,
            ),
        );
        element.set_own(
            "click",
            Self::native("click", native_dom_click, &self.function_proto),
        );
        let mut style = JsObject::ordinary();
        style.proto = Some(self.object_proto.clone());
        style.set_own("[[DomStyleNode]]", JsValue::num(id as f64));
        element.set_own("style", JsValue::Object(Obj(Rc::new(RefCell::new(style)))));
        let mut dataset = JsObject::ordinary();
        dataset.proto = Some(self.object_proto.clone());
        dataset.set_own("[[DomDatasetNode]]", JsValue::num(id as f64));
        element.set_own(
            "dataset",
            JsValue::Object(Obj(Rc::new(RefCell::new(dataset)))),
        );
        let mut class_list = JsObject::ordinary();
        class_list.proto = Some(self.object_proto.clone());
        class_list.set_own("[[DomClassListNode]]", JsValue::num(id as f64));
        class_list.set_own(
            "contains",
            Self::native("contains", native_class_list_contains, &self.function_proto),
        );
        class_list.set_own(
            "add",
            Self::native("add", native_class_list_add, &self.function_proto),
        );
        class_list.set_own(
            "remove",
            Self::native("remove", native_class_list_remove, &self.function_proto),
        );
        class_list.set_own(
            "toggle",
            Self::native("toggle", native_class_list_toggle, &self.function_proto),
        );
        class_list.set_own(
            "toString",
            Self::native(
                "toString",
                native_class_list_to_string,
                &self.function_proto,
            ),
        );
        element.set_own(
            "classList",
            JsValue::Object(Obj(Rc::new(RefCell::new(class_list)))),
        );
        let onclick_getter =
            Self::native("get onclick", native_dom_onclick_get, &self.function_proto);
        let onclick_setter =
            Self::native("set onclick", native_dom_onclick_set, &self.function_proto);
        let _ = element.define(
            "onclick",
            Property {
                value: JsValue::Null,
                writable: false,
                enumerable: true,
                configurable: true,
                getter: Some(onclick_getter),
                setter: Some(onclick_setter),
            },
        );
        JsValue::Object(Obj(Rc::new(RefCell::new(element))))
    }

    fn dom_style_id(object: &Obj) -> Option<u32> {
        match object.borrow().get_own("[[DomStyleNode]]") {
            Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                Some(n as u32)
            }
            _ => None,
        }
    }

    fn dom_dataset_id(object: &Obj) -> Option<u32> {
        match object.borrow().get_own("[[DomDatasetNode]]") {
            Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                Some(n as u32)
            }
            _ => None,
        }
    }

    fn dom_class_list_id(object: &Obj) -> Option<u32> {
        match object.borrow().get_own("[[DomClassListNode]]") {
            Some(JsValue::Num(n)) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                Some(n as u32)
            }
            _ => None,
        }
    }

    fn dataset_attribute_name(property: &str) -> String {
        let mut name = String::from("data-");
        for ch in property.chars() {
            if ch.is_ascii_uppercase() {
                name.push('-');
                name.push(ch.to_ascii_lowercase());
            } else {
                name.push(ch);
            }
        }
        name
    }

    fn array_length(&self, v: &JsValue) -> usize {
        if let JsValue::Object(o) = v { if let Some(BinaryState::View(view)) = binary_state(o) { if view.kind.is_some() { return binary_length(&view); } } }
        match v {
            JsValue::Object(o) => match o.borrow().get_own("length") {
                Some(JsValue::Num(n)) if n >= 0.0 => n as usize,
                _ => 0,
            },
            _ => 0,
        }
    }
    /// True when `v` is an array whose `[Symbol.iterator]` still resolves to
    /// the intrinsic `Array.prototype[Symbol.iterator]` (§22.1.3.34, aliased
    /// to `Array.prototype.values`), so the O(1) index-based fast path below
    /// is spec-equivalent to walking the general iterator protocol. A
    /// subclass override or a deleted `Array.prototype[Symbol.iterator]`
    /// must fall through to the slow path instead, so a missing/non-callable
    /// method still throws per GetIterator (§7.4.2).
    fn has_original_array_iterator(&mut self, v: &JsValue) -> Result<bool, Throw> {
        let method = self.get_prop(v, WellKnownSymbol::Iterator.symbol())?;
        let canonical = self.array_proto.borrow().get_own(WellKnownSymbol::Iterator.symbol());
        Ok(matches!(
            (&method, &canonical),
            (JsValue::Object(a), Some(JsValue::Object(b))) if Rc::ptr_eq(&a.0, &b.0)
        ))
    }
    fn iterate(&mut self, v: &JsValue) -> Result<Vec<JsValue>, Throw> {
        if let JsValue::Object(object) = v {
            if matches!(object.borrow().exotic, Exotic::Array) && self.has_original_array_iterator(v)? {
                let n = match object.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n as usize,
                    _ => 0,
                };
                return Ok((0..n)
                    .map(|n| self.get_prop(v, &n.to_string()))
                    .collect::<Result<_, _>>()?);
            }
            let iterator_method = self.get_prop(v, WellKnownSymbol::Iterator.symbol())?;
            if callable_function(&iterator_method).is_some() {
                let iterator = self.call_value(&iterator_method, v.clone(), Vec::new())?;
                let same = matches!((v, &iterator), (JsValue::Object(left), JsValue::Object(right)) if Rc::ptr_eq(&left.0, &right.0));
                if !same {
                    return self.iterate(&iterator);
                }
            }
            let next = self.get_prop(v, "next")?;
            if !callable_function(&next).is_some() {
                return Err(Throw::type_error("value is not iterable"));
            }
            let mut values = Vec::new();
            // This eager collector is a temporary bridge until iterator
            // consumers advance lazily. Bound infinite iterators well below
            // the conformance runner's per-file deadline.
            for _ in 0..100_000usize {
                let result = match self.call_value(&next, v.clone(), Vec::new()) {
                    Ok(result) => result,
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                };
                let result_value = match result {
                    JsValue::Object(_) => result,
                    _ => {
                        self.close_iterator(v);
                        return Err(Throw::type_error("iterator result is not an object"));
                    }
                };
                // IteratorComplete/IteratorValue (§7.4.6/§7.4.7) go through
                // the full [[Get]], not a raw own-property probe, so a
                // `done`/`value` accessor that throws is observed instead of
                // silently reading as absent (which would loop forever on an
                // iterator whose result objects define `value` as a getter).
                let done = match self.get_prop(&result_value, "done") {
                    Ok(value) => value.to_boolean(),
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                };
                if done {
                    return Ok(values);
                }
                match self.get_prop(&result_value, "value") {
                    Ok(value) => values.push(value),
                    Err(error) => {
                        self.close_iterator(v);
                        return Err(error);
                    }
                }
            }
            self.close_iterator(v);
            return Err(Throw::range_error("iterator exceeded the iteration limit"));
        }
        match v {
            JsValue::Str(s) => {
                let mut values = Vec::with_capacity(s.len());
                let mut at = 0;
                while at < s.len() {
                    let first = s.units()[at];
                    if (0xd800..=0xdbff).contains(&first)
                        && s.units().get(at + 1).is_some_and(|next| (0xdc00..=0xdfff).contains(next))
                    {
                        values.push(JsValue::str_units(s.units()[at..at + 2].to_vec()));
                        at += 2;
                    } else {
                        values.push(JsValue::str_units(vec![first]));
                        at += 1;
                    }
                }
                Ok(values)
            }
            _ => Err(Throw::type_error("value is not iterable")),
        }
    }
    fn close_iterator(&mut self, iterator: &JsValue) {
        let Ok(return_method) = self.get_prop(iterator, "return") else {
            return;
        };
        if callable_function(&return_method).is_some() {
            let _ = self.call_value(&return_method, iterator.clone(), Vec::new());
        }
    }

    fn exec_for_of(
        &mut self,
        left: &ForLeft,
        iterable: &JsValue,
        body: &ast::Stmt,
        env: &EnvHandle,
    ) -> Result<Flow, Throw> {
        let dispose = for_of_dispose_kind(left);
        let custom_iterator = self.custom_iterator(iterable)?;
        let Some(iterator) = custom_iterator else {
            for value in self.iterate(iterable)? {
                let iteration_env = for_iteration_env(left, env);
                match self.for_of_iteration_body(left, value, body, &iteration_env, dispose)? {
                    Flow::Normal | Flow::Continue(None) => {}
                    Flow::Break(None) => break,
                    flow => return Ok(flow),
                }
            }
            return Ok(Flow::Normal);
        };
        loop {
            let next = self.get_prop(&iterator, "next")?;
            let result = match self.call_value(&next, iterator.clone(), Vec::new()) {
                Ok(result) => result,
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            };
            let result_value = match result {
                JsValue::Object(_) => result,
                _ => {
                    self.close_iterator(&iterator);
                    return Err(Throw::type_error("iterator result is not an object"));
                }
            };
            let done = match self.get_prop(&result_value, "done") {
                Ok(value) => value.to_boolean(),
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            };
            if done {
                break;
            }
            let value = match self.get_prop(&result_value, "value") {
                Ok(value) => value,
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            };
            let iteration_env = for_iteration_env(left, env);
            match self.for_of_iteration_body(left, value, body, &iteration_env, dispose) {
                Ok(Flow::Normal | Flow::Continue(None)) => {}
                Ok(Flow::Break(None)) => {
                    self.close_iterator(&iterator);
                    break;
                }
                Ok(flow) => {
                    self.close_iterator(&iterator);
                    return Ok(flow);
                }
                Err(error) => {
                    self.close_iterator(&iterator);
                    return Err(error);
                }
            }
        }
        Ok(Flow::Normal)
    }

    /// Run one `for-of` iteration: bind the element and execute the body. `dispose` is
    /// `Some(is_async)` when `left` is a `using`/`await using` declaration, in which case the
    /// element is a per-iteration disposal resource — disposed (awaited for `await using`) at
    /// the end of that same iteration, before the loop's next `next()` call.
    /// Ported from lucid-softworks/lumen (crates/lumen/src/eval.rs) @ f6c9860d, MIT.
    fn for_of_iteration_body(
        &mut self,
        left: &ForLeft,
        value: JsValue,
        body: &ast::Stmt,
        iteration_env: &EnvHandle,
        dispose: Option<bool>,
    ) -> Result<Flow, Throw> {
        let Some(is_async) = dispose else {
            self.bind_for(left, value, iteration_env)?;
            return self.exec_stmt(body, iteration_env);
        };
        self.using_stack.push(Vec::new());
        let step = self.for_of_iteration_step(left, value, body, iteration_env, is_async);
        let frame = self.using_stack.pop().unwrap_or_default();
        self.dispose_frame(frame, step)
    }

    fn for_of_iteration_step(
        &mut self,
        left: &ForLeft,
        value: JsValue,
        body: &ast::Stmt,
        iteration_env: &EnvHandle,
        is_async: bool,
    ) -> Result<Flow, Throw> {
        self.add_disposable(&value, is_async)?;
        self.bind_for(left, value, iteration_env)?;
        self.exec_stmt(body, iteration_env)
    }

    fn custom_iterator(&mut self, value: &JsValue) -> Result<Option<JsValue>, Throw> {
        let JsValue::Object(object) = value else {
            return Ok(None);
        };
        if matches!(object.borrow().exotic, Exotic::Array) && self.has_original_array_iterator(value)? {
            return Ok(None);
        }
        let method = self.get_prop(value, WellKnownSymbol::Iterator.symbol())?;
        if callable_function(&method).is_some() {
            let iterator = self.call_value(&method, value.clone(), Vec::new())?;
            let next = self.get_prop(&iterator, "next")?;
            if callable_function(&next).is_none() {
                return Err(Throw::type_error(
                    "iterator method did not return an iterator",
                ));
            }
            return Ok(Some(iterator));
        }
        let next = self.get_prop(value, "next")?;
        if callable_function(&next).is_some() {
            Ok(Some(value.clone()))
        } else {
            Ok(None)
        }
    }
}
