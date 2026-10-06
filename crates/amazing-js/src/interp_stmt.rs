// Proper-tail outcome/redispatch adaptation from lucid-softworks/lumen
// (crates/lumen/src/{eval,interpreter}.rs and interpreter/call_entry.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Function body lexical reservation/closure-environment adaptation reuses
// lucid-softworks/lumen (crates/lumen/src/{eval,interpreter}.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// ForIn head initializer ordering adapted from boa-dev/boa
// (core/engine/src/bytecompiler/statement/loop.rs::compile_for_in_loop)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Live GetSuperConstructor/SuperCall and original-constructor instance elements
// adapted from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// GetTemplateObject registry/cache algorithm adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Stable parse-site tokens replace upstream pointer identity; UTF-16 values and realms remain Amazing-owned.
// Synchronous ForIn/OfBodyEvaluation adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Portions ported from boa-dev/boa (core/engine/src/builtins/error/{mod,aggregate,type}.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Error-family allocation, metadata and iterable algorithms adapted to Amazing.
// Async-generator ReturnStatement Await adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// StatementList/try completion portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Normal IteratorClose GetMethod/completion portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Assignment-pattern reference ordering portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
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


fn annex_b_eligible_set<R>(i: &Interpreter, f: impl FnOnce(&mut HashSet<usize>) -> R) -> R {
    f(&mut i.annex_b_eligible.borrow_mut())
}

fn is_annex_b_eligible(i: &Interpreter, node: *const ast::FunctionItem) -> bool {
    i.annex_b_eligible.borrow().contains(&(node as usize))
}

/// Peels through zero or more `LabelIdentifier :` wrappers (Annex B.3.2 allows a
/// LabelledStatement's body to be a FunctionDeclaration in non-strict code -- see
/// `parser_statements.rs`'s own `is_labeled_function`) to find the FunctionDeclaration
/// itself, or `None` if this statement isn't (possibly labeled) one.
///
/// `StmtKind::Block`'s block-entry pre-loop (BlockDeclarationInstantiation) uses this
/// so a labeled function declaration is lexically (re-)bound at block entry exactly
/// like an unlabeled one -- without it, a later labeled redeclaration's own statement
/// executes its Annex B.3.3 sync step (in the `StmtKind::Function`/`StmtKind::Labeled`
/// exec arms) reading `env.local_cell`, but that cell was never updated to the labeled
/// declaration's own closure, so both the block's own reads and the promoted outer
/// `var` stay stuck on whichever earlier, unlabeled declaration last ran the pre-loop.
fn labeled_function_decl(stmt: &ast::Stmt) -> Option<&ast::FunctionItem> {
    match &stmt.kind {
        StmtKind::Function(f) => Some(f.as_ref()),
        StmtKind::Labeled { body, .. } => labeled_function_decl(body),
        _ => None,
    }
}

impl Interpreter {
    fn exec_stmt(&mut self, s: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        self.completion = None;
        let result = self.exec_stmt_inner(s, env);
        // These statements apply UpdateEmpty(result, undefined), including
        // abrupt break/continue. A Block and Label preserve genuine Empty.
        if matches!(&s.kind, StmtKind::If { .. } | StmtKind::With { .. } | StmtKind::Try(_)
            | StmtKind::While { .. } | StmtKind::DoWhile { .. } | StmtKind::For { .. }
            | StmtKind::ForIn { .. } | StmtKind::ForOf { .. } | StmtKind::ForAwaitOf { .. }
            | StmtKind::Switch { .. }) && self.completion.is_none() {
            self.completion = Some(JsValue::Undefined);
        }
        result
    }

    fn exec_loop_body(&mut self, body: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        let value = self.completion.clone();
        let result = self.exec_stmt(body, env);
        let result = match result {
            Err(_) if has_pending_return() => Ok(Flow::Return(take_pending_return().unwrap())),
            other => other,
        };
        if self.completion.is_none() { self.completion = value; }
        result
    }

    fn exec_stmt_inner(&mut self, s: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        if let Some(hook) = self.debug_hook.clone() {
            hook.borrow_mut().on_statement(&self.current_script_id, &self.current_script_url, s.span.start.line, self.depth);
        }
        match &s.kind {
            StmtKind::Empty | StmtKind::Debugger => Ok(Flow::Normal),
            StmtKind::Expression(e) => {
                self.completion = Some(self.eval_expr(e, env)?);
                Ok(Flow::Normal)
            }
            StmtKind::Var(d) => {
                for x in &d.decls {
                    if d.kind == ast::VarKind::Var && x.init.is_none() {
                        continue;
                    }
                    let reference = if d.kind == ast::VarKind::Var && uses_object_var_assignment(env) {
                        match &x.binding {
                            Binding::Ident(name) => Some(self.capture_reference(&ast::Expr::new(ExprKind::Ident(name.clone()), name.span), env)?),
                            _ => None,
                        }
                    } else { None };
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
                    if let Some(reference) = reference { self.put_reference(reference, v, env)?; }
                    else { self.bind_pattern(&x.binding, v, d.kind, env)?; }
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
                        // `scope.local_cell` alone only proves *some* sibling declaration of
                        // this name was eligible somewhere in this function -- not that *this*
                        // occurrence was (see `ANNEX_B_ELIGIBLE`'s doc comment above). Without
                        // the second check, a later individually-blocked same-named declaration
                        // (shadowed by an intervening let/const at its own nesting depth) would
                        // still stomp an earlier legitimately-hoisted one on every execution.
                        //
                        // Instantiation owns this occurrence's formal-name exclusions:
                        // ordinary FDI applies them, while eval uses its own collector.
                        // Execution consumes the original node key without reclassifying it.
                        let promoted = scope.local_cell(&n.name).is_some_and(|c| !c.is_lexical())
                            && is_annex_b_eligible(self, f.as_ref() as *const ast::FunctionItem);
                        if promoted {
                            if let Some(local) = env.local_cell(&n.name) {
                                if let Ok(v) = local.read() {
                                    self.copy_annex_binding(&scope, &n.name, v)?;
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
            StmtKind::Block(b) => self.exec_lexical_block(b, env),
            StmtKind::If { cond, then, els } => {
                self.completion = Some(JsValue::Undefined);
                if self.eval_expr(cond, env)?.to_boolean() {
                    self.exec_if_branch(then, env)
                } else if let Some(e) = els {
                    self.exec_if_branch(e, env)
                } else {
                    Ok(Flow::Normal)
                }
            }
            StmtKind::While { test, body } => {
                self.completion = Some(JsValue::Undefined);
                self.loop_while(test, body, env, None)
            }
            StmtKind::With { object, body } => {
                // 14.11.2 Runtime Semantics: Evaluation. RequireObjectCoercible
                // then ToObject on the head expression (throwing for
                // null/undefined, unlike Object(...)'s own boxing-only
                // behavior -- `native_object` alone would return `undefined`
                // unchanged instead), then run the body in a fresh
                // object-environment record so `scoped_reference` (used by
                // every identifier read/write/call/delete already) sees it.
                let value = self.eval_expr(object, env)?;
                if value.is_null() || value.is_undefined() {
                    return Err(Throw::type_error(
                        "cannot convert null or undefined to object in with statement",
                    ));
                }
                let bound = native_object(self, JsValue::Undefined, vec![value])?;
                let JsValue::Object(bound) = bound else {
                    return Err(Throw::type_error("with statement requires an object"));
                };
                let with_env = EnvHandle::new_object(bound, env.clone());
                self.exec_stmt(body, &with_env)
            }
            StmtKind::DoWhile { body, test } => {
                self.completion = Some(JsValue::Undefined);
                self.loop_do_while(body, test, env, None)
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                self.completion = Some(JsValue::Undefined);
                self.loop_for(
                    init.as_ref(),
                    test.as_ref(),
                    update.as_ref(),
                    body,
                    env,
                    None,
                )
            }
            StmtKind::ForIn { left, right, body } => self.exec_for_in(left, right, body, env, &[]),
            StmtKind::ForOf { left, right, body } => {
                self.completion = Some(JsValue::Undefined);
                let v = self.eval_for_head_right(left, right, env)?;
                self.exec_for_of(left, &v, body, env)
            }
            StmtKind::ForAwaitOf { left, right, body } => {
                self.completion = Some(JsValue::Undefined);
                let iterable = self.eval_for_head_right(left, right, env)?;
                self.exec_for_await_of(left, &iterable, body, env)
            }
            StmtKind::Switch { disc, cases } => {
                self.completion = Some(JsValue::Undefined);
                // 14.12's CaseBlockEvaluation: the discriminant is evaluated
                // in the *outer* environment, but every case's test
                // expression and body run in ONE fresh Block environment
                // shared by the whole switch (not per-case, and not the
                // enclosing one) -- e.g. a `let` in one case and another
                // `let` of the same name after the switch must not conflict
                // (JSVM-10: this switch previously ran everything straight
                // in the caller's own `env`, so they did).
                let d = self.eval_expr(disc, env)?;
                let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                // Mirrors `StmtKind::Block`'s own Annex-B function pre-pass,
                // over every case's body concatenated in source order (one
                // shared scope, so a later case's function still shadows an
                // earlier same-named one at switch entry, per spec).
                for case in cases {
                    for stmt in &case.body {
                        if let StmtKind::Function(f) = &stmt.kind {
                            if let Some(n) = &f.name {
                                let v = self.make_function(f, &e);
                                if e.local_cell(&n.name).is_some() {
                                    // Annex B.3.3.4, same as `StmtKind::Block` above: a later
                                    // same-named FunctionDeclaration (even in a different case)
                                    // silently re-initializes rather than re-declaring.
                                    let _ = e.assign(&n.name, v);
                                } else {
                                    e.declare_lexical(&n.name, v, true)?;
                                }
                            }
                        }
                    }
                }
                let mut start = None;
                let mut def = None;
                for (n, c) in cases.iter().enumerate() {
                    if let Some(t) = &c.test {
                        let tv = self.eval_expr(t, &e)?;
                        if self.strict_eq(&d, &tv) {
                            start = Some(n);
                            break;
                        }
                    } else {
                        def = Some(n);
                    }
                }
                let start = start.or(def);
                // A CaseBlock is a disposal boundary only when some clause directly declares a
                // `using` / `await using` resource, matching StmtKind::Block's own check. The
                // shared exec_switch_cases loop (below) carries JSVM-10's labeled-break fix: an
                // UNLABELED break is swallowed here, a LABELED break keeps propagating upward.
                if !cases.iter().any(|c| c.body.iter().any(stmt_declares_resource)) {
                    return self.exec_switch_cases(cases, start, &e);
                }
                self.using_stack.push(Vec::new());
                let result = self.exec_switch_cases(cases, start, &e);
                let frame = self.using_stack.pop().unwrap_or_default();
                self.dispose_frame(frame, result)
            }
            StmtKind::Break { label: l } => Ok(Flow::Break(l.as_ref().map(|x| x.name.clone()))),
            StmtKind::Continue { label: l } => {
                Ok(Flow::Continue(l.as_ref().map(|x| x.name.clone())))
            }
            StmtKind::Labeled { label, body } => {
                // Every directly attached label belongs to this iteration statement.
                // Keep the full set so an outer alias can be a continue target.
                let mut labels = vec![label.name.clone()];
                let mut iteration = body.as_ref();
                while let StmtKind::Labeled { label, body } = &iteration.kind {
                    labels.push(label.name.clone());
                    iteration = body.as_ref();
                }
                if let StmtKind::ForIn { left, right, body } = &iteration.kind {
                    return self.exec_for_in(left, right, body, env, &labels);
                }
                match &body.kind {
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
                }
            },
            StmtKind::Return(e) => {
                if self.tail_calls_allowed
                    && !self.using_stack[self.tail_resource_base..].iter().any(|frame| !frame.is_empty())
                {
                    if let Some(expression) = e {
                        return match self.eval_tail_expression(expression, env)? {
                            CallOutcome::Value(value) => Ok(Flow::Return(value)),
                            CallOutcome::Tail(record) => Ok(Flow::Tail(record)),
                        };
                    }
                }
                let value = match e {
                    Some(expression) => {
                        let value = self.eval_expr(expression, env)?;
                        coroutine_return_value(self, value)?
                    }
                    None => JsValue::Undefined,
                };
                Ok(Flow::Return(value))
            }
            StmtKind::Throw(e) => Err(Throw(self.eval_expr(e, env)?)),
            StmtKind::Try(t) => {
                self.completion = Some(JsValue::Undefined);
                self.exec_try(t, env)
            }
        }
    }

    /// CaseBlockEvaluation's clause-selection loop (14.12.3), factored out so both the plain and
    /// the disposal-boundary path in StmtKind::Switch above can share it. `start` is the already-
    /// resolved index of the matched (or, failing that, default) clause to begin running from --
    /// `None` when nothing matched and there was no default clause, in which case nothing runs.
    fn exec_switch_cases(&mut self, cases: &[ast::SwitchCase], start: Option<usize>, env: &EnvHandle) -> Result<Flow, Throw> {
        let mut i = start;
        while let Some(n) = i {
            let previous = self.completion.clone();
            let result = self.exec_stmts(&cases[n].body, env);
            if self.completion.is_none() { self.completion = previous; }
            match result? {
                Flow::Normal => {
                    i = (n + 1 < cases.len()).then_some(n + 1);
                }
                // An UNLABELED break targets the nearest enclosing switch/loop -- this
                // switch, here -- and is swallowed. A LABELED break (JSVM-10 fix: this
                // previously swallowed it unconditionally too, so `outer: for (...) {
                // switch (x) { case 1: break outer; } }` silently did nothing instead of
                // breaking the outer loop) must keep propagating upward exactly like any
                // other Flow -- only the matching `Labeled` wrapper may swallow it.
                Flow::Break(None) => break,
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
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
                    // FDI/eval instantiation has already classified this exact node;
                    // caller metadata must not reinterpret the collector's decision.
                    if scope.local_cell(&n.name).is_some_and(|c| !c.is_lexical())
                        && is_annex_b_eligible(self, f.as_ref() as *const ast::FunctionItem) {
                        let implicit = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                        let v = self.make_function(f, &implicit);
                        let _ = implicit.declare_lexical(&n.name, v.clone(), true);
                        self.copy_annex_binding(&scope, &n.name, v)?;
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
        // Eval's Script body has no formal parameters. Its native frame is
        // temporarily removed, so do not mistake the lexical caller for FDI.
        // A function called by eval adds its own frame and keeps the FDI rule.
        if self.eval_grammar_override.is_some_and(|(depth, _)| self.frames.len() <= depth) {
            return Vec::new();
        }
        self.frames
            .last()
            .and_then(|frame| frame.own_function.as_ref())
            .and_then(|obj| obj.borrow().func.as_ref().map(|f| f.params.clone()))
            .map(|params| params.iter().flat_map(|p| binding_names(&p.binding)).collect())
            .unwrap_or_default()
    }

    fn exec_stmts(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<Flow, Throw> {
        let mut value = None;
        self.completion = None;
        let tail_calls_allowed = self.tail_calls_allowed;
        let result = (|| {
        for statement in body {
            let result = self.exec_stmt(statement, env);
            let result = match result {
                Err(_) if has_pending_return() => Ok(Flow::Return(take_pending_return().unwrap())),
                other => other,
            };
            if stmt_declares_resource(statement) { self.tail_calls_allowed = false; }
            if self.completion.is_none() { self.completion = value.clone(); }
            value = self.completion.clone();
            match result? {
                Flow::Normal => {}
                abrupt => return Ok(abrupt),
            }
        }
        Ok(Flow::Normal)
        })();
        self.tail_calls_allowed = tail_calls_allowed;
        result
    }

    /// Lexical block evaluation shared by ordinary and try/catch/finally blocks.
    /// Each body's resources must finish before its completion enters the next
    /// catch/finally clause, and must not register in an enclosing scope's frame.
    fn exec_stmts_with_disposal(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<Flow, Throw> {
        if !body.iter().any(stmt_declares_resource) {
            return self.exec_stmts(body, env);
        }
        self.using_stack.push(Vec::new());
        let result = self.exec_stmts(body, env);
        let frame = self.using_stack.pop().unwrap_or_default();
        self.dispose_frame(frame, result)
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
        let protected = self.completion.clone();
        // The expression-unwind marker has reached a statement boundary.
        // Clear it before calling disposers, whose throws must replace Return
        // rather than be suppressed with the sentinel or mistaken for Return.
        let mut completion = match result {
            Err(_) if has_pending_return() => Ok(Flow::Return(take_pending_return().unwrap())),
            other => other,
        };
        for d in frame.into_iter().rev() {
            let disposed = self
                .call_value(&d.method, d.value.clone(), Vec::new())
                .and_then(|v| if d.method_is_async {
                    coroutine_return_await(self, v.clone())
                        .or_else(|| coroutine_await(self, v.clone()))
                        .unwrap_or_else(|| self.await_value(v))
                } else { Ok(v) });
            if let Err(new_err) = disposed {
                completion = match completion {
                    Err(prev) => Err(Throw(self.make_suppressed_error(new_err.0, prev.0))),
                    Ok(_) => Err(new_err),
                };
            }
        }
        self.completion = protected;
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
            Job::ModuleImport(job) => self.run_module_import_job(job),
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
        self.completion = Some(JsValue::Undefined);
        while self.eval_expr(test, env)?.to_boolean() {
            match self.exec_loop_body(body, env)? {
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
        self.completion = Some(JsValue::Undefined);
        loop {
            match self.exec_loop_body(body, env)? {
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(None) | Flow::Normal => {}
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => {}
                Flow::Continue(name) => return Ok(Flow::Continue(name)),
                Flow::Break(name) => return Ok(Flow::Break(name)),
                Flow::Return(value) => return Ok(Flow::Return(value)),
                Flow::Tail(record) => return Ok(Flow::Tail(record)),
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
        let saved = std::mem::replace(&mut self.tail_calls_allowed, false);
        let result = self.loop_for_inner(init, test, update, body, env, label);
        self.tail_calls_allowed = saved;
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
        self.completion = Some(JsValue::Undefined);
        let loop_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        if let Some(init) = init {
            match init {
                ForInit::Decl(decl) => {
                    let fake = ast::Stmt::new(StmtKind::Var((**decl).clone()), body.span.clone());
                    self.exec_stmt(&fake, &loop_env)?;
                    self.completion = Some(JsValue::Undefined);
                }
                ForInit::Expr(expression) => {
                    self.eval_expr(expression, &loop_env)?;
                }
            }
        }
        // JS-19 (js-vm cross-lane report, 2026-09-27): 14.7.4.2 step 8 sets perIterationLets to
        // the EMPTY list when the head's LexicalDeclaration isConst -- a `const` (or `using`/
        // `await using`, which the parser also represents as VarKind::Const, see
        // parser_statements.rs) for-head binding gets NO per-iteration copy at all. It keeps
        // living in the one `loopEnv` binding created once, above, via the ordinary `const`
        // path of executing that declaration statement (CreateImmutableBinding) -- test, body,
        // and update all read/write that SAME binding directly. This was previously computing
        // per_iteration_names for const too and copying it through fresh state/iteration/update
        // environments every iteration (correct only for `let`), which broke in two opposite
        // ways depending on how one first tried to fix it: with the copies left mutable,
        // `for (const c = 1; ; c++) {}` never threw and looped forever; naively making the
        // copies immutable instead made the ALWAYS-RUNS internal copy-back below
        // (`state_env.assign`, step 9.e of CreatePerIterationEnvironment, which is not a user
        // assignment and must never itself check constness) throw on every `for (const [x] =
        // ...; ...) {}`, even when nothing ever reassigns `x`. Treating const's
        // perIterationLets as empty -- matching the spec text -- avoids both: there is no
        // separate copy for the internal step to touch, and the update clause's own assignment
        // still throws because `loopEnv`'s binding is genuinely immutable.
        let per_iteration_names = match init {
            Some(ForInit::Decl(decl)) if !matches!(decl.kind, ast::VarKind::Var | ast::VarKind::Const) => decl
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
            match self.exec_loop_body(body, &iteration_env)? {
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if label == Some(name.as_str()) => break,
                Flow::Continue(None) => {}
                Flow::Continue(Some(name)) if label == Some(name.as_str()) => {}
                Flow::Normal => {}
                Flow::Continue(name) => return Ok(Flow::Continue(name)),
                Flow::Break(name) => return Ok(Flow::Break(name)),
                Flow::Return(value) => return Ok(Flow::Return(value)),
                Flow::Tail(record) => return Ok(Flow::Tail(record)),
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
    fn exec_for_in(&mut self, left: &ForLeft, right: &ast::Expr, body: &ast::Stmt,
        env: &EnvHandle, labels: &[String]) -> Result<Flow, Throw>
    {
        self.completion = Some(JsValue::Undefined);
        self.eval_for_in_initializer(left, env)?;
        let v = self.eval_for_head_right(left, right, env)?;
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
            match self.exec_loop_body(body, &iteration_env)? {
                Flow::Normal | Flow::Continue(None) => {}
                Flow::Continue(Some(name)) if labels.contains(&name) => {}
                Flow::Break(None) => break,
                Flow::Break(Some(name)) if labels.contains(&name) => break,
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }

    // Annex B ForIn head evaluation resolves the var binding before evaluating
    // its initializer, and stores once before the right-hand expression. A With
    // environment can change during that initializer; retain the actual reference.
    fn eval_for_in_initializer(&mut self, left: &ForLeft, env: &EnvHandle) -> Result<(), Throw> {
        let ForLeft::Decl(declaration) = left else { return Ok(()); };
        if declaration.kind != ast::VarKind::Var { return Ok(()); }
        let Some(item) = declaration.decls.first() else { return Ok(()); };
        let (Binding::Ident(name), Some(initializer)) = (&item.binding, &item.init) else { return Ok(()); };
        let target = ast::Expr::new(ExprKind::Ident(name.clone()), name.span);
        let reference = self.capture_reference(&target, env)?;
        let mut value = self.eval_named_class_expr(initializer, env, &Self::class_binding_name(&name.name))?;
        self.named_evaluation(initializer, &mut value, &name.name);
        self.put_reference(reference, value, env)
    }

    /// ForIn/OfHeadEvaluation (14.7.5.1): for a `let`/`const`/`using`/`await using` ForDeclaration,
    /// the loop's bound names already exist -- UNINITIALIZED, i.e. in TDZ -- in the environment
    /// `expr` (the head's right-hand side) is evaluated in, so a self-referencing right side
    /// (`for (let x of [x])`, `for (using x of [x])`) throws ReferenceError instead of silently
    /// reading an outer same-named binding. This temporary environment is deliberately discarded
    /// right after: reusing it as the loop's own per-iteration environment (what
    /// `for_iteration_env` creates fresh every time round) would break the fresh-binding-per-
    /// iteration semantics a closure captured in the body relies on. A `var`/plain assignment
    /// target skips this entirely, matching plain `ForBinding`'s evaluation order.
    fn eval_for_head_right(&mut self, left: &ForLeft, right: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        let ForLeft::Decl(d) = left else { return self.eval_expr(right, env); };
        if d.kind == ast::VarKind::Var { return self.eval_expr(right, env); }
        let head_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        for declarator in &d.decls {
            for name in binding_names(&declarator.binding) {
                head_env.declare_uninitialized(&name, d.kind == ast::VarKind::Let)?;
            }
        }
        self.eval_expr(right, &head_env)
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
    fn reserve_body_lexicals(&self, body: &[ast::Stmt], e: &EnvHandle) -> Result<(), Throw> {
        for statement in body {
            match &statement.kind {
                StmtKind::Var(declaration) if declaration.kind != ast::VarKind::Var => {
                    for item in &declaration.decls {
                        for name in binding_names(&item.binding) { e.declare_uninitialized(&name, declaration.kind == ast::VarKind::Let)?; }
                    }
                }
                StmtKind::Class(class) => { if let Some(name) = &class.name { e.declare_uninitialized(&name.name, true)?; } }
                _ => {}
            }
        }
        Ok(())
    }

    fn prepare_function_lexical_environment(&self, body: &[ast::Stmt], variable_env: &EnvHandle, strict: bool) -> Result<EnvHandle, Throw> {
        // FDI steps 31–35: declarations are reserved before any body
        // statement, while sloppy lexical bindings remain outside varEnv.
        let lexical_env = if strict { variable_env.clone() }
            else { EnvHandle::new(EnvKind::Block, Some(variable_env.clone())) };
        self.reserve_body_lexicals(body, &lexical_env)?;
        Ok(lexical_env)
    }

    fn exec_lexical_block(&mut self, b: &ast::Block, env: &EnvHandle) -> Result<Flow, Throw> {
        self.completion = None;
        let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        self.reserve_body_lexicals(&b.body, &e)?;
        // BlockDeclarationInstantiation's own function-hoisting: every direct
        // FunctionDeclaration child of this block is lexically bound, to a fresh
        // closure, at block entry -- independent of Annex B (this happens in strict
        // mode too; it's what makes `f` resolve inside the block at all, and what
        // Annex B.3.3's sync step above reads from).
        for stmt in &b.body {
            if let Some(f) = labeled_function_decl(stmt) {
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
        self.exec_stmts_with_disposal(&b.body, &e)
    }

    fn exec_try(&mut self, t: &ast::TryStmt, env: &EnvHandle) -> Result<Flow, Throw> {
        let tail_calls_allowed = std::mem::replace(&mut self.tail_calls_allowed, false);
        let result = self.exec_lexical_block(&t.block, env);
        self.tail_calls_allowed = tail_calls_allowed;
        let mut out = match result {
            Ok(x) => Ok(x),
            // A coroutine-backed generator's `.return(v)` completion at a suspended `yield`
            // inside this try block is smuggled through this same `Err` channel (see
            // interp_coroutine.rs's module doc) so it unwinds through arbitrarily nested
            // expressions/statements -- but unlike a real JS throw, it must bypass `catch`
            // entirely (only `finally`, below, still runs), the same way a real `return`
            // statement would. `has_pending_return()` distinguishes the two.
            Err(th) if has_pending_return() => Err(th),
            Err(th) => {
                let th = self.normalize_throw(th);
                if let Some(c) = &t.catch {
                    let ce = EnvHandle::new(if c.binding.is_some() { EnvKind::Catch } else { EnvKind::Block }, Some(env.clone()));
                    let saved = std::mem::replace(&mut self.tail_calls_allowed,
                        tail_calls_allowed && t.finally.is_none());
                    let result = (|| {
                        if let Some(binding) = &c.binding {
                            self.bind_pattern(binding, th.0, ast::VarKind::Let, &ce)?;
                        }
                        self.exec_lexical_block(&c.body, &ce)
                    })();
                    self.tail_calls_allowed = saved;
                    result
                } else {
                    Err(th)
                }
            }
        };
        let protected = self.completion.clone();
        if let Some(f) = &t.finally {
            let result = self.exec_lexical_block(f, env);
            match result {
                Ok(Flow::Normal) => self.completion = protected,
                Ok(x) => out = Ok(x),
                Err(e) => out = Err(e),
            }
        }
        out
    }

    pub fn eval_expr(&mut self, e: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        if optional_chain_link(e) {
            let reference = self.eval_chain_reference(e, env)?;
            return self.chain_value(reference);
        }
        match &e.kind {
            ExprKind::PrivateName(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            ExprKind::RegExp(regex) => self.make_regexp(regex.clone()),
            ExprKind::Num(n) => self.numeric(n),
            ExprKind::BigInt(digits) => BinaryBigInt::parse_radix(digits, 10)
                .map(Self::bigint_value)
                .ok_or_else(|| Throw::syntax_error("invalid BigInt literal")),
            ExprKind::TaggedTemplate { tag, template } => {
                // 13.3.11.1 Runtime Semantics: Evaluation. `resolve_tag_callee`
                // preserves the MemberExpression/Ident `this` binding exactly
                // like an ordinary call's callee (`call_expr`'s own logic);
                // parentheses retain a property reference while plain values
                // and call results use `this` undefined.
                let (f, this) = self.resolve_tag_callee(tag, env)?;
                let template_object = self.get_template_object(template)?;
                let mut argv = Vec::with_capacity(1 + template.exprs.len());
                argv.push(template_object);
                for x in &template.exprs {
                    argv.push(self.eval_expr(x, env)?);
                }
                self.call_value(&f, this, argv)
            }
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
            ExprKind::DynamicImport(args) => {
                self.dynamic_import(args, env)
            }
            ExprKind::DynamicImportPhase { phase,args } => self.dynamic_import_phase(args,env,
                if phase=="source" {crate::module::ImportPhase::Source} else {crate::module::ImportPhase::Defer}),
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
                            self.copy_data_properties(&o, &src, &[])?;
                        }
                        ast::ObjElem::Prop(p) => {
                            let k = match &p.key_utf16 {
                                Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
                                None => self.prop_key(&p.key, env)?,
                            };
                            if let ast::PropValue::Method(method) = &p.value {
                                if matches!(method.kind, ast::MethodKind::Get | ast::MethodKind::Set) {
                                    let function_value = self.object_method_value(method, env, &o, &k);
                                    let JsValue::Object(object) = &o else { unreachable!() };
                                    let mut property = object.borrow().own_prop(&k).cloned()
                                        .filter(|property| property.is_accessor())
                                        .unwrap_or(Property {
                                            accessor: false,
                                            value: JsValue::Undefined, writable: false,
                                            enumerable: true, configurable: true,
                                            getter: None, setter: None,
                                        });
                                    property.accessor = true;
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
                                        // Keep a lone-surrogate property key exact.
                                        self.eval_named_class_expr(x, env, &Self::property_function_name(&k))?
                                    };
                                    if !is_proto_setter {
                                        self.named_evaluation_utf16(x, &mut val, &Self::property_function_name(&k));
                                    }
                                    val
                                }
                                ast::PropValue::CoverInitializedName { .. } => {
                                    return Err(Throw::syntax_error("cover initialized name outside assignment pattern"));
                                }
                                ast::PropValue::Method(m) => self.object_method_value(m, env, &o, &k),
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
                    named_env.declare_function_name(&name.name, value.clone())?;
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
            ExprKind::Member { .. } => {
                // All property reads consume the same reference path. The raw
                // computed expression runs before GetValue checks the base;
                // nullish rejection precedes ToPropertyKey and property Get.
                let reference = self.eval_chain_reference(e, env)?;
                self.chain_value(reference)
            }
            ExprKind::Yield { argument, delegate } => {
                let value = match argument {
                    Some(a) => self.eval_expr(a, env)?,
                    None => JsValue::Undefined,
                };
                eval_yield(self, value, *delegate)
            }
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
            BinaryBigInt::parse_radix(&raw, n.radix as u32)
                .map(|integer| integer.to_f64())
                .ok_or_else(|| Throw::syntax_error("invalid numeric literal"))?
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
            ast::UnOp::Delete if optional_chain_link(x) => self.delete_chain(x, env),
            ast::UnOp::Delete => match &x.kind {
                ExprKind::Paren(inner) => self.unary(op, inner, env),
                ExprKind::Ident(name) => self.delete_scoped_name(env, &name.name),
                ExprKind::Member { obj, prop, .. } if matches!(prop, MemberProp::Private(_)) => {
                    // A private member's `delete` target is an early SyntaxError
                    // (§13.5.1.1); the parser does not reject it yet, so guard here.
                    let _ = self.eval_expr(obj, env)?;
                    Err(Throw::syntax_error("private members cannot be deleted"))
                }
                ExprKind::Member { .. } => self.delete_chain(x, env),
                _ => {
                    self.eval_expr(x, env)?;
                    Ok(JsValue::Bool(true))
                }
            },
            ast::UnOp::Await => {
                let value = self.eval_expr(x, env)?;
                // A coroutine-backed async function body suspends for real (interp_coroutine.rs).
                if let Some(result) = coroutine_await(self, value.clone()) {
                    return result;
                }
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
                // Unary minus is ToNumeric(expr) (13.5.6) -- ToPrimitive with the number
                // hint, not default, per 7.1.3.
                let primitive = self.primitive_number_hint(value)?;
                if let JsValue::BigInt(value) = &primitive { Ok(Self::bigint_value(value.negated())) }
                else { Ok(JsValue::num(-self.to_number(&primitive)?)) }
            },
            ast::UnOp::Pos => Ok(JsValue::num(self.to_number(&value)?)),
            ast::UnOp::Not => Ok(JsValue::Bool(!value.to_boolean())),
            ast::UnOp::BitNot => {
                // Bitwise NOT is also ToNumeric(expr) (13.5.8) -- number hint.
                let primitive = self.primitive_number_hint(value)?;
                if let JsValue::BigInt(value) = &primitive { Ok(Self::bigint_value(value.bit_not())) }
                else { Ok(JsValue::num((!self.to_int32(&primitive)?) as f64)) }
            },
            ast::UnOp::Typeof => Ok(JsValue::str(self.typeof_value(&value))),
            ast::UnOp::Void => Ok(JsValue::Undefined),
            ast::UnOp::Delete | ast::UnOp::Await => Err(Throw::type_error("invalid unary value operation")),
        }
    }
    fn update_value(&mut self, op: ast::UpdateOp, old: JsValue) -> Result<JsValue, Throw> {
        // ++/-- are ToNumeric(oldValue) (13.4.x) -- number hint, not default.
        let primitive = self.primitive_number_hint(old)?;
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
        // 13.4.2/13.4.3: postfix ++/-- return ToNumeric(oldValue), not the raw
        // pre-conversion operand -- observable when the operand is an object with a
        // custom valueOf/@@toPrimitive. Convert once here; update_value() re-deriving
        // it from an already-primitive value is a no-op re-entry, not a second
        // ToPrimitive call, so this does not double the conversion's side effects.
        let old_numeric = self.to_numeric_value(old)?;
        let next = self.update_value(op, old_numeric.clone())?;
        self.put_reference(reference, next.clone(), env)?;
        Ok(if prefix { next } else { old_numeric })
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
                // Every numeric/bitwise operator other than `+` is
                // ApplyStringOrNumericBinaryOperator's ToNumeric(lval)/ToNumeric(rval)
                // (13.15.3) -- ToPrimitive with the number hint, not default.
                let x = self.to_numeric_value(a)?;
                let y = self.to_numeric_value(b)?;
                if matches!(x, JsValue::BigInt(_)) || matches!(y, JsValue::BigInt(_)) {
                    return self.bigint_operator(op, x, y);
                }
                Ok(JsValue::num(match op {
                    Sub => self.to_number(&x)? - self.to_number(&y)?,
                    Mul => self.to_number(&x)? * self.to_number(&y)?,
                    Div => self.to_number(&x)? / self.to_number(&y)?,
                    Rem => self.to_number(&x)? % self.to_number(&y)?,
                    Pow => crate::value::number_exponentiate(self.to_number(&x)?, self.to_number(&y)?),
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
                // IsLessThan(x, y, LeftFirst) (7.2.13) calls ToPrimitive on each operand
                // with the number hint, not default.
                let x = self.primitive_number_hint(a)?;
                let y = self.primitive_number_hint(b)?;
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
        let inferred_name = match &t.kind { ExprKind::Ident(identifier) => Some(identifier.name.as_str()), _ => None };
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
        let (mut reference, old) = if op == ast::AssignOp::Assign {
            if let Some(name) = event_handler::identifier_name(t) {
                (self.name_reference(e, name)?, JsValue::Undefined)
            } else { (self.capture_reference(t, e)?, JsValue::Undefined) }
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
            let mut v = match inferred_name {
                Some(name) => self.eval_named_class_expr(r, e, &name.into())?,
                None => self.eval_expr(r, e)?,
            };
            if let Some(name) = inferred_name {
                self.named_evaluation(r, &mut v, name);
            }
            self.coerce_reference_key(&mut reference)?;
            self.put_reference(reference, v.clone(), e)?;
            return Ok(v);
        }
        if matches!(&reference, Reference::Unresolvable(_, _)) {
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
        let mut v = match inferred_name {
            Some(name) => self.eval_named_class_expr(r, e, &name.into())?,
            None => self.eval_expr(r, e)?,
        };
        if let Some(name) = inferred_name {
            self.named_evaluation(r, &mut v, name);
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

    /// SetFunctionName for a still-anonymous ECMAScript callable. Preserve the
    /// existing string entry point; observable names use lossless UTF-16 units.
    fn infer_function_name(&self, value: &mut JsValue, name: &str) {
        self.infer_function_name_utf16(value, &name.into());
    }

    fn infer_function_name_utf16(&self, value: &mut JsValue, name: &crate::value::JsString) {
        if let JsValue::Object(object) = value {
            let mut object = object.borrow_mut();
            // Class names are installed before static elements by NamedEvaluation.
            if object.class_constructor { return; }
            if let Some(function) = &mut object.func {
                // Native closures have explicit spec-defined names, including
                // the empty name. Passing one through an assignment cannot
                // turn it into an anonymous ECMAScript function definition.
                if matches!(function.body, FunctionBody::Native(_)) { return; }
                if function.name.is_empty() {
                    function.name = name.to_utf8_lossy();
                    object.define("name", Property {
                        accessor: false,
                        value: JsValue::str(name.clone()), writable: false, enumerable: false,
                        configurable: true, getter: None, setter: None,
                    }).unwrap();
                }
            }
        }
    }

    /// NamedEvaluation applies only when the source is an anonymous function
    /// definition. Parentheses preserve it; comma expressions and existing values do not.
    fn named_evaluation(&self, source: &ast::Expr, value: &mut JsValue, name: &str) {
        if is_anonymous_function_definition(source) { self.infer_function_name(value, name); }
    }

    fn named_evaluation_utf16(&self, source: &ast::Expr, value: &mut JsValue, name: &crate::value::JsString) {
        if is_anonymous_function_definition(source) {
            self.infer_function_name_utf16(value, name);
        }
    }

    // testharness.js exports its API by assigning properties of global_scope.
    // Unresolved lexical names must therefore consult the global object record.
    fn identifier_value(&mut self, env: &EnvHandle, name: &str) -> Result<Option<JsValue>, Throw> {
        let Some(mut reference) = self.scoped_reference(env, name)? else { return Ok(None); };
        self.reference_value(&mut reference).map(Some)
    }

    fn resolve_reference(&mut self, target: &ast::Expr, env: &EnvHandle) -> Result<(Reference, JsValue), Throw> {
        let mut reference = self.capture_reference(target, env)?;
        let value = self.reference_value(&mut reference)?;
        Ok((reference, value))
    }

    fn put_reference(
        &mut self,
        reference: Reference,
        value: JsValue,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match reference {
            Reference::Binding(name, binding_env) => {
                let env = &binding_env;
                if env.kind() != EnvKind::Global {
                    return env.set_local_binding(&name, value, self.strict_eval_context);
                }
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
                if !env.assign_with_strict(&name, value.clone(), self.strict_eval_context)? {
                    let global = JsValue::Object(self.global_obj.clone());
                    self.put_reference(Reference::Property(global, name.into()), value, env)?;
                }
                Ok(())
            }
            Reference::Unresolvable(name, global) => {
                if self.strict_eval_context { return Err(Throw::reference_error(format!("{name} is not defined"))); }
                self.put_reference(Reference::Property(JsValue::Object(global), name.into()), value, env)
            }
            Reference::ObjectBinding(object, name, _) => {
                let key = PropertyKey::from(name.as_str());
                let exists = self.default_has_property(&object, &key)?;
                if self.strict_eval_context && !exists {
                    return Err(Throw::reference_error(format!("{name} is not defined")));
                }
                self.put_reference(Reference::Property(JsValue::Object(object), key), value, env)
            }
            Reference::RawProperty(base, raw) => {
                self.require_reference_base(&base)?;
                let key = self.to_property_key(&raw)?;
                self.put_reference(Reference::Property(base, key), value, env)
            }
            Reference::RawSuperProperty(base, raw, receiver) => {
                self.require_reference_base(&base)?;
                let key = self.to_property_key(&raw)?;
                self.put_reference(Reference::SuperProperty(base, key, receiver), value, env)
            }
            Reference::Property(base, key) => {
                self.require_reference_base(&base)?;
                let boxed = if base.is_object() { base.clone() }
                    else { native_object(self, JsValue::Undefined, vec![base.clone()])? };
                let JsValue::Object(object) = &boxed else { unreachable!() };
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
            Reference::SuperProperty(base, key, receiver) => {
                let JsValue::Object(object) = base else { return Err(Throw::type_error("nullish super prototype")); };
                if self.set_with_receiver(&object, &key, value, receiver)? || !self.strict_eval_context { Ok(()) }
                else { Err(Throw::type_error("cannot assign to super property")) }
            }
        }
    }
    fn assign_target(&mut self, t: &ast::Expr, v: JsValue, e: &EnvHandle) -> Result<(), Throw> {
        let reference = self.capture_reference(t, e)?;
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
            ExprKind::Assign { op: ast::AssignOp::Assign, .. } => {
                let reference = self.prepare_assignment_pattern(target, env)?;
                self.assign_prepared_pattern(target, value, reference, env)
            }
            ExprKind::Array(elements) => {
                // ArrayAssignmentPattern (8.6.3 DestructuringAssignmentEvaluation): pull only as
                // many values as the pattern declares -- an elision still steps the iterator, a
                // trailing rest drains the rest (and thereby exhausts it, so no close needed) --
                // and IteratorClose whatever's left over once the list is done, exactly like
                // `bind_pattern_mode`'s `Binding::Array` arm's own `PatternIter`. The previous
                // `self.iterate(value)?` eagerly drained the WHOLE iterator up front regardless
                // of how many elements the pattern actually bound, so `[x] = iterable` never
                // closed a not-yet-exhausted iterator and any iterator that legitimately never
                // signals `done` (a common conformance-test shape) looped until `iterate`'s
                // hard-coded step bound gave up.
                let mut iter = PatternIter::new(self, value)?;
                let result = (|| -> Result<(), Throw> {
                    for element in elements {
                        match element {
                            Some(element) if matches!(&element.kind, ExprKind::Spread(_)) => {
                                let ExprKind::Spread(inner) = &element.kind else { unreachable!() };
                                let reference = self.prepare_assignment_pattern(inner, env)?;
                                let items = iter.collect_rest(self)?;
                                let rest = array_values(self, items)?;
                                self.assign_prepared_pattern(inner, &rest, reference, env)?;
                            }
                            Some(element) => {
                                let reference = self.prepare_assignment_pattern(element, env)?;
                                let item = iter.step(self)?.unwrap_or(JsValue::Undefined);
                                self.assign_prepared_pattern(element, &item, reference, env)?;
                            }
                            None => {
                                iter.skip(self)?;
                            }
                        }
                    }
                    Ok(())
                })();
                let closed = iter.close(self);
                // See `bind_pattern_mode`'s `Binding::Array` arm's identical combination.
                result.and(closed)
            }
            ExprKind::Object(elements) => {
                if value.is_null() || value.is_undefined() {
                    return Err(Throw::type_error("cannot destructure nullish value"));
                }
                let mut used = Vec::new();
                for element in elements {
                    match element {
                        ast::ObjElem::Spread(inner) => {

                            // `parse_object_element` wraps a `{...x}` rest target as
                            // `ObjElem::Spread(Expr::Spread(x))` (its own generic spread-parsing
                            // path, shared with array/call-argument spreads), not as the bare
                            // target directly -- unwrap the same way `parser.rs`'s own
                            // `ensure_assignable` already does, or this recurses on the
                            // `ExprKind::Spread` node itself, which no arm below matches, and
                            // falls through to `assign_target`'s "invalid assignment target".
                            let target = match &inner.kind {
                                ExprKind::Spread(t) => t.as_ref(),
                                _ => inner.as_ref(),
                            };
                            let reference = self.prepare_assignment_pattern(target, env)?;
                            let rest = self.object_rest(value, &used)?;
                            self.assign_prepared_pattern(target, &rest, reference, env)?;
                        }
                        ast::ObjElem::Prop(property) => {
                            let key = match &property.key_utf16 { Some(units) => PropertyKey::from(crate::value::JsString::from_units(units.clone())), None => self.prop_key(&property.key, env)? };
                            used.push(key.clone());
                            let target = match &property.value {
                                ast::PropValue::Shorthand(identifier) => ast::Expr::new(ExprKind::Ident(identifier.clone()), identifier.span),
                                ast::PropValue::Init(expression) => expression.clone(),
                                ast::PropValue::CoverInitializedName { binding, default } => ast::Expr::new(
                                    ExprKind::Assign { op: ast::AssignOp::Assign,
                                        target: Box::new(ast::Expr::new(ExprKind::Ident(binding.clone()), binding.span)),
                                        value: Box::new(default.clone()) }, binding.span),
                                ast::PropValue::Method(_) => return Err(Throw::reference_error("method is not an assignment target")),
                            };
                            let reference = self.prepare_assignment_pattern(&target, env)?;
                            let item = self.get_prop(value, &key)?;
                            self.assign_prepared_pattern(&target, &item, reference, env)?;
                        }
                    }
                }
                Ok(())
            }
            _ => self.assign_target(target, value.clone(), env),
        }
    }
    /// Original FunctionEnvironment function retained across lexical arrows.
    fn super_call_function(&self) -> Result<Obj, Throw> {
        match self.frames.last().and_then(|frame| frame.super_constructor.clone()) {
            Some(JsValue::Object(function)) => Ok(function),
            _ => Err(Throw::syntax_error("super() used outside a derived constructor")),
        }
    }

    /// GetSuperConstructor snapshots F.[[Prototype]] before argument evaluation.
    /// F is an ordinary ECMAScript function; its nullable prototype need not
    /// be a constructor until the subsequent SuperCall has evaluated argv.
    fn live_super_constructor(&self) -> Result<JsValue, Throw> {
        let function = self.super_call_function()?;
        let prototype = function.borrow().proto.clone();
        Ok(prototype.map_or(JsValue::Null, JsValue::Object))
    }

    fn bind_super_this(&mut self, environment: &EnvHandle, result: &JsValue, function: &Obj) -> Result<(), Throw> {
        // Construct ran already: a second super() still runs its constructor
        // before BindThisValue rejects, and fields run only after successful bind.
        environment.initialize_this(result.clone())?;
        if let Some(frame) = self.frames.last_mut() {
            frame.this = result.clone();
        }
        let elements = function.borrow().class_elements.clone();
        if let Some(elements) = elements {
            initialize_instance_elements(self, result, &elements)?;
        }
        Ok(())
    }

    fn call_expr(
        &mut self,
        c: &ast::Expr,
        args: &[ast::Expr],
        optional: bool,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        if matches!(&c.kind, ExprKind::Super) {
            let new_target = self.current_new_target().ok_or_else(|| Throw::type_error("derived constructor requires new"))?;
            let function = self.super_call_function()?;
            let constructor = self.live_super_constructor()?;
            let argv = self.eval_args(args, e)?;
            let result = self.construct_with_new_target(&constructor, argv, new_target)?;
            self.bind_super_this(e, &result, &function)?;
            return Ok(result);
        }
        let Some((f, this)) = self.chain_callee(c, e)? else { return Ok(JsValue::Undefined); };
        if optional && (f.is_null() || f.is_undefined()) {
            return Ok(JsValue::Undefined);
        }
        self.invoke_expression_call(c, args, optional, f, this, e)
    }

    fn invoke_expression_call(&mut self, c: &ast::Expr, args: &[ast::Expr], optional: bool, f: JsValue, this: JsValue, e: &EnvHandle) -> Result<JsValue, Throw> {
        let argv = self.eval_args(args, e)?;
        let direct = !optional && event_handler::identifier_name(c) == Some("eval");
        self.invoke_evaluated_expression_call(direct, f, this, argv, e)
    }

    fn invoke_evaluated_expression_call(&mut self, direct: bool, f: JsValue, this: JsValue, argv: Vec<JsValue>, e: &EnvHandle) -> Result<JsValue, Throw> {
        let previous_direct_eval = self.direct_eval_call;
        let previous_direct_eval_env = self.direct_eval_env.take();
        self.direct_eval_call = direct && self.is_intrinsic_eval(&f);
        self.direct_eval_env = self.direct_eval_call.then(|| e.clone());
        let result = self.call_value(&f, this, argv);
        self.direct_eval_call = previous_direct_eval;
        self.direct_eval_env = previous_direct_eval_env;
        result
    }
    fn delete_member_value(&mut self, o: JsValue, k: PropertyKey) -> Result<JsValue, Throw> {
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
            if let Some(result)=self.namespace_delete(&o,&k) {
                let deleted=result?;
                if !deleted&&self.strict_eval_context {return Err(Throw::type_error("module namespace export cannot be deleted"));}
                return Ok(JsValue::Bool(deleted));
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
    /// Tags preserve property references, including parenthesized optional chains.
    fn resolve_tag_callee(&mut self, c: &ast::Expr, e: &EnvHandle) -> Result<(JsValue, JsValue), Throw> {
        Ok(self.chain_callee(c, e)?.unwrap_or((JsValue::Undefined, JsValue::Undefined)))
    }

    /// 13.2.8.4 GetTemplateObject: builds, once per template literal site per
    /// realm (mirroring the spec's per-Realm [[TemplateMap]]), the frozen
    /// cooked-strings array with a frozen `raw` array of the same site's raw
    /// spellings. The parsed site's stable token survives AST/body clones;
    /// the owning interpreter retains one frozen object per realm and site.
    pub(crate) fn get_template_object(&mut self, t: &ast::TemplateLit) -> Result<JsValue, Throw> {
        let key = (self.binding_current_realm(), t.site.key());
        if let Some(cached) = self.template_objects.get(&key).cloned() {
            return Ok(cached);
        }
        let raw: Vec<JsValue> = t.raw_quasis_utf16.iter()
            .map(|units| JsValue::str_units(units.clone())).collect();
        let cooked: Vec<JsValue> = t.quasis_utf16.iter().zip(t.cooked_valid.iter())
            .map(|(units, valid)| if *valid { JsValue::str_units(units.clone()) } else { JsValue::Undefined })
            .collect();

        let mut raw_obj = JsObject::array();
        raw_obj.proto = Some(self.array_proto.clone());
        let raw_array = JsValue::object(raw_obj);
        array_create_data_properties(self, &raw_array, raw)?;
        let raw_array = object_integrity(self, vec![raw_array], true)?;

        let mut cooked_obj = JsObject::array();
        cooked_obj.proto = Some(self.array_proto.clone());
        let template_object = JsValue::object(cooked_obj);
        array_create_data_properties(self, &template_object, cooked)?;
        let JsValue::Object(template_obj) = &template_object else {
            unreachable!("array literal construction always yields an object")
        };
        let _ = template_obj.borrow_mut().define("raw", Property {
            accessor: false,
            value: raw_array,
            writable: false,
            enumerable: false,
            configurable: false,
            getter: None,
            setter: None,
        });
        let template_object = object_integrity(self, vec![template_object], true)?;
        self.template_objects.insert(key, template_object.clone());
        Ok(template_object)
    }

    fn prop_key(&mut self, p: &PropKey, e: &EnvHandle) -> Result<PropertyKey, Throw> {
        match p {
            PropKey::Ident(i) => Ok(i.name.clone().into()),
            PropKey::Private(_) => Err(Throw::type_error(
                "private names are not yet supported by the runtime",
            )),
            PropKey::Str(s) => Ok(s.clone().into()),
            PropKey::Num(n) => {
                let value = self.numeric(n)?;
                self.to_property_key(&value)
            }
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
    ) -> Result<CallOutcome, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("value is not callable")),
        };
        if matches!(o.borrow().exotic, Exotic::Proxy(_)) {
            return match new_target {
                Some(nt) => self.proxy_construct(&o, args, nt).map(CallOutcome::Value),
                None => self.proxy_call_forward(&o, this, args).map(CallOutcome::Tail),
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
        if o.borrow().class_constructor && new_target.is_none() && !is_default_class_constructor(&o) {
            return Err(Throw::type_error("class constructor requires new"));
        }
        let lexical_super_constructor = o.borrow().lexical_super_constructor.clone();
        let lexical_new_target = o.borrow().lexical_new_target.clone();
        let async_body = self.async_body_code(&o, &fun, strict_function);
        let bytecode_async = fun.is_async && !fun.is_generator
            && (async_body.is_some() || matches!(&fun.body, FunctionBody::Bytecode(_)));
        let async_generator = fun.is_async && fun.is_generator;
        // A plain async function the VM did not compile runs on a coroutine (JSVM-9), whose
        // promise the body branch below returns directly.
        let thread_async = fun.is_async && !fun.is_generator && async_body.is_none()
            && matches!(&fun.body, FunctionBody::Block(_) | FunctionBody::Expr(_));
        let thread_started = std::cell::Cell::new(false);
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Throw::range_error("maximum call stack size exceeded"));
        }
        self.depth += 1;
        self.frames.push(CallFrame {
            context_id: CallFrame::next_context_id(),
            this: this.clone(),
            home: fun.home.clone(),
            // Retain the original FunctionEnvironment function, not its
            // definition-time prototype. Lexical arrows keep this same F.
            super_constructor: derived_constructor.as_ref().map(|_| JsValue::Object(o.clone())).or(lexical_super_constructor),
            new_target: new_target.clone().or(lexical_new_target.clone()),
            own_function: Some(o.clone()),
        });
        let previous_completion = self.completion.clone();
        let previous_strict = self.strict_eval_context;
        if interpreted || is_default_class_constructor(&o) { self.strict_eval_context = strict_function; }
        self.completion = None;
        let previous_tail_calls = std::mem::replace(&mut self.tail_calls_allowed, false);
        let previous_tail_resource_base =
            std::mem::replace(&mut self.tail_resource_base, self.using_stack.len());
        let r = (|| -> Result<CallOutcome, Throw> {
            match fun.body {
                FunctionBody::Bytecode(code) => {
                    // The generator's own object, not a pre-read prototype: a
                    // parameter default can replace `functionObject.prototype`
                    // during FunctionDeclarationInstantiation, and per spec
                    // (OrdinaryCreateFromConstructor) that later value is what
                    // the created generator's `[[Prototype]]` must observe, not
                    // whatever "prototype" held before parameters were bound.
                    // `call` reads it fresh after binding parameters.
                    code.call_outcome(
                        self, fun.env.clone(), fun.is_arrow, this, args,
                        fun.is_generator.then(|| o.clone()),
                        derived_constructor.clone(), new_target.clone().or(lexical_new_target.clone()), caller_realm,
                    )
                }
                FunctionBody::Native(n) => {
                    self.native_stack.push(o.clone());
                    let result = match self.native_tail_forward(n, &this, &args) {
                        Some(result) => result.map(|record|
                            CallOutcome::Tail(record.forwarded(self.binding_current_realm(), false))),
                        None => n(self, this, args).map(CallOutcome::Value),
                    };
                    self.native_stack.pop();
                    result
                }
                FunctionBody::HostNative(id, policy, realm) => {
                    if realm != self.host_realm {
                        return Err(Throw::type_error("host function belongs to another interpreter"));
                    }
                    let constructing = new_target.is_some();
                    if (constructing && policy == HostCallPolicy::CallOnly)
                        || (!constructing && matches!(policy, HostCallPolicy::ConstructOnly | HostCallPolicy::ConstructOnlyAllocates))
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
                    result.map(CallOutcome::Value)
                }
                FunctionBody::Expr(x) => {
                    let e = EnvHandle::new(if fun.is_arrow { EnvKind::Arrow } else { EnvKind::Function }, Some(fun.env.clone()));
                    if !fun.is_arrow { e.bind_this(this); }
                    let e = self.prepare_function_parameter_environment(&e, &fun.params, strict_function);
                    self.bind_params(&e, &fun.params, args, fun.is_arrow, strict_function)?;
                    let e = self.prepare_function_body_environment(&e, &fun.params, &[], &[])?;
                    if let Some(code) = &async_body { return code.call_async_with_env(self, e).map(CallOutcome::Value); }
                    let e = self.prepare_function_lexical_environment(&[], &e, strict_function)?;
                    if thread_async {
                        if let Some(promise) = self.start_thread_async_function(FunctionBody::Expr(x.clone()), e.clone(), strict_function) {
                            thread_started.set(true);
                            return Ok(promise.into());
                        }
                    }
                    self.tail_calls_allowed = strict_function && !fun.is_async && !fun.is_generator;
                    if self.tail_calls_allowed { self.eval_tail_expression(&x, &e) }
                    else { self.eval_expr(&x, &e).map(CallOutcome::Value) }
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
                            let parent = self.live_super_constructor()?;
                            let bound = self.construct_with_new_target(&parent, args.clone(), new_target.clone().expect("newTarget"))?;
                            self.bind_super_this(&e, &bound, &o)?;
                        }
                    } else {
                        if !fun.is_arrow { e.bind_this(this.clone()); }
                        if new_target.is_some() {
                            if let Some(elements) = o.borrow().class_elements.clone() {
                                initialize_instance_elements(self, &this, &elements)?;
                            }
                        }
                    }
                    let e = self.prepare_function_parameter_environment(&e, &fun.params, strict_function);
                    self.bind_params(&e, &fun.params, args, fun.is_arrow || !Self::function_body_needs_arguments(&fun.params, &b.body), strict_function)?;
                    let e = self.prepare_ast_function_body_environment(&e, &fun.params, &b.body)?;
                    // Generators/async generators: FunctionDeclarationInstantiation (hoisting)
                    // still runs eagerly, synchronously, right here -- only the body's statements
                    // are deferred, to a pooled coroutine thread that the first `.next()` drives
                    // (JSVM-8; see interp_coroutine.rs). `new_target` is set only when called via
                    // `new`, which generator functions never support (25.5.1/27.3.1: no
                    // [[Construct]]).
                    if fun.is_generator {
                        if new_target.is_some() {
                            return Err(Throw::type_error("generator functions are not constructors"));
                        }
                        let e = self.hoist_function_body(&b.body, &e, strict_function)?;
                        return Ok(self.start_thread_generator(&o, b, e, fun.is_async, strict_function).into());
                    }
                    if let Some(code) = &async_body { return code.call_async_with_env(self, e).map(CallOutcome::Value); }
                    // Hoist, coroutine execution and allocation-failure fallback
                    // must use identical declaration nodes: deep Block clones
                    // invalidate the per-occurrence Annex B eligibility keys.
                    let body = Rc::new(FunctionBody::Block(b));
                    let FunctionBody::Block(b) = body.as_ref() else { unreachable!() };
                    let e = self.hoist_function_body(&b.body, &e, strict_function)?;
                    if thread_async {
                        if let Some(promise) = self.start_thread_async_function(body.clone(), e.clone(), strict_function) {
                            thread_started.set(true);
                            return Ok(promise.into());
                        }
                    }
                    // A function body is a disposal boundary in its own right (spec: ordinary
                    // FunctionDeclarationInstantiation's disposal capability), independent of any
                    // nested block's own boundary.
                    self.tail_calls_allowed = strict_function && !fun.is_async && !fun.is_generator;
                    let has_using = b.body.iter().any(stmt_declares_resource);
                    let completion = if has_using {
                        self.using_stack.push(Vec::new());
                        let result = self.exec_stmts(&b.body, &e);
                        let frame = self.using_stack.pop().unwrap_or_default();
                        self.dispose_frame(frame, result)?
                    } else {
                        self.exec_stmts(&b.body, &e)?
                    };
                    match completion {
                        Flow::Tail(record) => Ok(CallOutcome::Tail(if derived {
                            record.derived_result(e, caller_realm)
                        } else { record })),
                        completion if derived => self.complete_derived_constructor(completion, &e, caller_realm).map(CallOutcome::Value),
                        Flow::Return(value) => Ok(value.into()),
                        _ => Ok(JsValue::Undefined.into()),
                    }
                }
            }
        })();
        self.tail_calls_allowed = previous_tail_calls;
        self.tail_resource_base = previous_tail_resource_base;
        self.completion = previous_completion;
        self.strict_eval_context = previous_strict;
        self.frames.pop();
        self.depth -= 1;
        let r = match r {
            Ok(CallOutcome::Tail(record)) => return Ok(CallOutcome::Tail(record)),
            Ok(CallOutcome::Value(value)) => Ok(value),
            Err(error) => Err(error),
        };
        let result = if async_generator {
            // Parameter binding runs when an async generator is called. An
            // abrupt binding completion is synchronous, before any iterator
            // result promise exists.
            r
        } else if bytecode_async {
            match r {
                Ok(value) => Ok(value),
                Err(error) => Ok(new_promise(self, PromiseFate::Rejected(self.normalize_throw(error).0))),
            }
        } else if thread_started.get() {
            // The coroutine path already returned the function's own promise. (Its synchronous
            // fallback, when no coroutine could be spawned, falls through to the branch below.)
            r
        } else if fun.is_async && !fun.is_generator {
            // An async *generator* call returns the AsyncGenerator instance itself, synchronously
            // and directly (spec 27.4.3.2 EvaluateAsyncGeneratorBody's completion is the generator
            // object, not a promise) -- the async part shows up per-call, in `next`/`return`/
            // `throw` each returning their own promise (see interp_coroutine.rs), not by wrapping
            // the whole `g()` call the way a plain (non-generator) async function's call is here.
            match r {
                Ok(value) => Ok(new_promise(self, PromiseFate::Fulfilled(value))),
                Err(error) => Ok(new_promise(self, PromiseFate::Rejected(self.normalize_throw(error).0))),
            }
        } else {
            r
        };
        result.map(CallOutcome::Value)
    }

    fn bind_params(
        &mut self,
        e: &EnvHandle,
        ps: &[ast::Param],
        args: Vec<JsValue>,
        is_arrow: bool,
        strict: bool,
    ) -> Result<(), Throw> {
        // Parameters are mutable function bindings, not lexical declarations:
        // body var declarations reuse them and sloppy duplicate names overwrite.
        // Allocate locally before binding patterns so assignment cannot escape
        // into a same-named binding in the enclosing environment.
        let mut names = std::collections::HashSet::new();
        let has_duplicates = ps.iter().flat_map(|p| binding_names(&p.binding)).any(|name| !names.insert(name));
        names.clear();
        for p in ps {
            for name in binding_names(&p.binding) {
                if names.insert(name.clone()) {
                    e.declare_uninitialized(&name, true)?;
                    if has_duplicates { e.initialize_parameter(&name, JsValue::Undefined)?; }
                }
            }
        }
        // FunctionDeclarationInstantiation (10.2.11) creates and binds `arguments` (steps 18-22)
        // BEFORE IteratorBindingInitialization runs over the formal parameters (step 23), so a
        // parameter default that reads `arguments` (`function f(a = arguments) {}`) sees a real
        // arguments object, not a ReferenceError -- it must be bound here, before the defaults
        // loop below, not after it.
        if !is_arrow && !names.contains("arguments") {
            let a = self.create_arguments_object(e, ps, &args, strict);
            if strict {
                e.declare_lexical("arguments", a, false)?;
                e.finish_parameter_initialization("arguments");
            } else if e.kind() == EnvKind::Parameter { e.declare_lexical("arguments", a, true)?; }
            else { e.declare_var("arguments", a)?; }
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
        Ok(())
    }
    fn construct(&mut self, f: &JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        self.construct_with_new_target(f, args, f.clone())
    }
    #[inline(never)]
    fn construct_with_new_target(&mut self, f: &JsValue, args: Vec<JsValue>, new_target: JsValue) -> Result<JsValue, Throw> {
        self.admit_synchronous_call()?;
        self.construct_with_new_target_admitted(f, args, new_target)
    }

    #[inline(never)]
    fn construct_with_new_target_admitted(&mut self, f: &JsValue, args: Vec<JsValue>, new_target: JsValue) -> Result<JsValue, Throw> {
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
            if std::ptr::fn_addr_eq(*native, bigint_constructor as crate::value::NativeFn)
                || std::ptr::fn_addr_eq(*native, native_symbol as crate::value::NativeFn)) {
            // Both have [[Construct]], but reject NewTarget before any
            // prototype lookup or instance allocation.
            return Err(Throw::type_error("BigInt and Symbol cannot be invoked with new"));
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
        // A default class builtin owns allocation and instance initialization.
        // Do not pre-read NewTarget.prototype or overwrite an existing parent result.
        if is_default_class_constructor(&o) {
            return self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target));
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
        // Binary stores/DataView own the prescribed coercions before allocation.
        if matches!(&fun.body, FunctionBody::Native(native) if binary_owns_constructor_allocation(*native)) {
            return self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target));
        }
        // Error-family constructors own OrdinaryCreateFromConstructor before
        // message/cause/iterable evaluation. Generic allocation must not read
        // newTarget.prototype first or overwrite the returned Error's prototype.
        if matches!(&fun.body, FunctionBody::Native(native) if runtime_is_error_constructor(*native)) {
            return self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target));
        }
        // The intrinsic Promise algorithm owns allocation: its IsCallable
        // executor check precedes GetPrototypeFromConstructor. Dispatch by
        // native identity, never the mutable global binding or function name.
        if matches!(&fun.body, FunctionBody::Native(native)
            if std::ptr::fn_addr_eq(*native, native_promise as crate::value::NativeFn)) {
            return self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target));
        }
        // Map and Set own prototype allocation before their cached adder Get.
        if matches!(&fun.body, FunctionBody::Native(native)
            if std::ptr::fn_addr_eq(*native, std_map_constructor as crate::value::NativeFn)
                || std::ptr::fn_addr_eq(*native, std_set_constructor as crate::value::NativeFn)) {
            return self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target));
        }
        // CreateDynamicFunction owns coercion and all three parse goals before
        // NewTarget.prototype. Generic allocation would reverse that order.
        if matches!(&fun.body, FunctionBody::Native(native)
            if runtime_is_dynamic_function_constructor(*native)) {
            return self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target));
        }
        if matches!(fun.body, FunctionBody::HostNative(_, HostCallPolicy::ConstructOnlyAllocates, _)) {
            let result = self.call_value_with_new_target(f, JsValue::Undefined, args, Some(new_target))?;
            return if result.is_object() { Ok(result) }
                else { Err(Throw::type_error("allocating host constructor returned a non-object")) };
        }
        let proto = match self.get_prop(&new_target, "prototype")? {
            JsValue::Object(p) => p,
            _ => self.constructor_fallback_prototype(f, &new_target)?,
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
        if self.callable_vm_enabled() && (f.is_generator || vm::ordinary_functions_enabled()) {
            if let Ok(code) = vm::compile_function_item(f, f.body.span, self.strict_eval_context) {
                return code.instantiate(self, e);
            }
        }
        let mut o = JsObject::function(JsFunction {
            source: f.source.clone(),
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
                accessor: false,
                value: JsValue::Object(p.clone()), writable: true, enumerable: false,
                configurable: false, getter: None, setter: None,
            }).unwrap();
            h.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(p);
        }
        // JS-19 fix-before-push (2026-09-27): a plain sloppy function declaration/expression
        // gets its own, non-poisoned "caller" accessor here, shadowing the inherited
        // %Function.prototype% poison pill -- see `native_legacy_function_caller_getter`'s doc
        // comment in interp_builtins.rs for why (the "legacy function.caller" feature,
        // test262 `features: [caller]`). Async and generator functions never qualify (matching
        // 9.2.7 AddRestrictedFunctionProperties' own gating), nor do strict ones, which keep
        // the inherited always-throwing accessor.
        if !f.is_async && !f.is_generator && !h.borrow().strict_function {
            let getter = symbol_method(self, "", native_legacy_function_caller_getter);
            let setter = symbol_method(self, "", native_legacy_function_caller_setter);
            h.borrow_mut().define("caller", Property {
                accessor: true,
                value: JsValue::Undefined, writable: false, enumerable: false,
                configurable: true, getter: Some(getter), setter: Some(setter),
            }).unwrap();
        }
        JsValue::Object(h)
    }
    fn make_arrow(&self, a: &ast::ArrowFunc, e: &EnvHandle) -> JsValue {
        if self.callable_vm_enabled() && vm::ordinary_functions_enabled() {
            let span = match &a.body {
                ast::ArrowBody::Expr(expression) => expression.span,
                ast::ArrowBody::Block(block) => block.span,
            };
            if let Ok(code) = vm::compile_arrow_item(a, span, self.strict_eval_context) {
                return code.instantiate(self, e);
            }
        }
        let body = match &a.body {
            ast::ArrowBody::Expr(x) => FunctionBody::Expr((**x).clone()),
            ast::ArrowBody::Block(b) => FunctionBody::Block(b.clone()),
        };
        let mut o = JsObject::function(JsFunction {
            source: a.source.clone(),
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
        let grammar = self.lexical_eval_grammar();
        o.lexical_new_target_allowed = grammar.allow_new_target;
        o.lexical_class_initializer = grammar.class_initializer;
        let handle = Obj(Rc::new(RefCell::new(o)));
        if let Some(frame) = self.frames.last() {
            let mut arrow = handle.borrow_mut();
            arrow.func.as_mut().unwrap().home = grammar.allow_super_property.then(||frame.home.clone()).flatten();
            arrow.lexical_super_constructor = grammar.allow_super_call.then(||frame.super_constructor.clone()).flatten();
            arrow.lexical_new_target = grammar.allow_new_target.then(||frame.new_target.clone()).flatten();
        }
        JsValue::Object(handle)
    }
    fn method_value(&self, m: &ast::ClassMethod, e: &EnvHandle, class_method: bool) -> JsValue {
        if self.callable_vm_enabled() {
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
            source: m.source.clone(),
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
        // Sync/async generator methods get their own per-method `.prototype` too (15.5.1
        // GeneratorMethod's OrdinaryObjectCreate(%GeneratorFunction.prototype.prototype%)),
        // exactly like a generator function declaration/expression via `make_function` --
        // a coroutine-backed generator instance's proto chain (`start_thread_generator`)
        // depends on this to reach the shared `next`/`return`/`throw` methods through the
        // spec-correct object rather than only through its own fallback default.
        if m.is_generator {
            let p = Obj(Rc::new(RefCell::new({
                let mut x = JsObject::ordinary();
                x.proto = Some(if m.is_async { self.async_generator_proto.clone() } else { self.generator_proto.clone() });
                x
            })));
            handle.borrow_mut().define("prototype", Property {
                accessor: false,
                value: JsValue::Object(p.clone()), writable: true, enumerable: false,
                configurable: false, getter: None, setter: None,
            }).unwrap();
            handle.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(p);
        }
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
            accessor: false,
            value: JsValue::Undefined, writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
        if matches!(kind, ast::MethodKind::Get | ast::MethodKind::Set) {
            descriptor.writable = false;
            descriptor.accessor = true;
            if kind == ast::MethodKind::Get { descriptor.getter = Some(value); }
            else { descriptor.setter = Some(value); }
        } else {
            descriptor = Property {
                accessor: false,
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
    fn name_class_method(method: &mut JsValue, base_name: &crate::value::JsString, kind: ast::MethodKind, params: &[ast::Param]) {
        let JsValue::Object(function) = method else { return };
        let mut units: Vec<u16> = match kind {
            ast::MethodKind::Get => "get ".encode_utf16().collect(),
            ast::MethodKind::Set => "set ".encode_utf16().collect(),
            _ => Vec::new(),
        };
        units.extend_from_slice(base_name.units());
        let name = crate::value::JsString::from_units(units);
        let length = params.iter().take_while(|param| !param.rest && param.default.is_none()).count();
        let mut function = function.borrow_mut();
        function.func.as_mut().unwrap().name = name.to_utf8_lossy();
        function.define("name", Property {
            accessor: false,
            value: JsValue::str(name), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).expect("method name property");
        function.define("length", Property {
            accessor: false,
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
                accessor: true,
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
                accessor: true,
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
                accessor: true,
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
                accessor: true,
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
                accessor: true,
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
    /// Whether `v`'s own `Symbol.iterator` is still this realm's *original*
    /// `%Array.prototype%[Symbol.iterator]` (`self.original_array_iterator`, a fixed snapshot
    /// from install time -- see its own doc comment), not merely whatever `array_proto` currently
    /// holds under that key. Comparing against the live value would always agree with itself once
    /// `Array.prototype[Symbol.iterator]` is globally reassigned (both sides read the same
    /// replacement), so every fast path guarded by this check would keep bypassing the override.
    fn has_original_array_iterator(&mut self, v: &JsValue) -> Result<bool, Throw> {
        let method = self.get_prop(v, WellKnownSymbol::Iterator.symbol())?;
        Ok(matches!(
            (&method, &self.original_array_iterator),
            (JsValue::Object(a), JsValue::Object(b)) if Rc::ptr_eq(&a.0, &b.0)
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
                        let _ = self.close_iterator(v);
                        return Err(error);
                    }
                };
                let result_value = match result {
                    JsValue::Object(_) => result,
                    _ => {
                        let _ = self.close_iterator(v);
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
                        let _ = self.close_iterator(v);
                        return Err(error);
                    }
                };
                if done {
                    return Ok(values);
                }
                match self.get_prop(&result_value, "value") {
                    Ok(value) => values.push(value),
                    Err(error) => {
                        let _ = self.close_iterator(v);
                        return Err(error);
                    }
                }
            }
            let _ = self.close_iterator(v);
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
    /// 7.4.6 IteratorClose, minus the completion-type combination step: every call site already
    /// knows whether it's closing for an abrupt completion (a `next()`/body throw, or a
    /// synthesized "iterator result is not an object" error) -- there, per spec step 2, this
    /// call's own outcome is entirely ignored and the caller keeps propagating its original
    /// error (`let _ = self.close_iterator(...)`) -- or a normal one (a `break`, an unlabeled
    /// `Flow::Return`, or a labeled `break`/`continue` to an outer construct), where THIS call's
    /// `Err` (an uncallable-but-present `return` throwing, or a non-Object result) becomes the
    /// new completion, replacing the original (`self.close_iterator(...)?`). Mirrors
    /// `close_async_iterator`'s already-correct treatment for `for await`.
    fn close_iterator(&mut self, iterator: &JsValue) -> Result<(), Throw> {
        let return_method = self.get_prop(iterator, "return")?;
        if matches!(return_method, JsValue::Undefined | JsValue::Null) {
            return Ok(());
        }
        if callable_function(&return_method).is_none() {
            return Err(Throw::type_error("iterator's return method is not callable"));
        }
        match self.call_value(&return_method, iterator.clone(), Vec::new())? {
            JsValue::Object(_) => Ok(()),
            _ => Err(Throw::type_error("iterator's return method did not return an object")),
        }
    }

    fn exec_for_of(
        &mut self,
        left: &ForLeft,
        iterable: &JsValue,
        body: &ast::Stmt,
        env: &EnvHandle,
    ) -> Result<Flow, Throw> {
        let saved = std::mem::replace(&mut self.tail_calls_allowed, false);
        let result = (|| {
        let dispose = for_of_dispose_kind(left);
        // ForIn/OfBodyEvaluation uses GetIterator's actual cached next method.
        // Step abrupts precede binding/body execution and never IteratorClose.
        let mut iterator = PatternIter::new(self, iterable)?;
        while let Some(value) = iterator.step(self)? {
            let iteration_env = for_iteration_env(left, env);
            match self.for_of_iteration_body(left, value, body, &iteration_env, dispose) {
                Ok(Flow::Normal | Flow::Continue(None)) => {}
                // A normal completion (unlike the `Err` arm below): IteratorClose's own
                // failure -- an uncallable `return` that itself throws, or a non-Object
                // result -- REPLACES this break/flow as the new completion (7.4.6 step 3/4),
                // rather than being discarded the way it is when we're already unwinding an
                // abrupt one.
                Ok(Flow::Break(None)) => {
                    iterator.close(self)?;
                    break;
                }
                Ok(flow) => {
                    iterator.close(self)?;
                    return Ok(flow);
                }
                Err(error) => {
                    let _ = iterator.close(self);
                    return Err(error);
                }
            }
        }
        Ok(Flow::Normal)

        })();
        self.tail_calls_allowed = saved;
        result
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
            return self.exec_loop_body(body, iteration_env);
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
        self.exec_loop_body(body, iteration_env)
    }
}

#[cfg(test)]
#[path = "interp_reference_read_tests.rs"]
mod reference_read_tests;
