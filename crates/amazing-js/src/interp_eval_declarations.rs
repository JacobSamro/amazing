// EvalDeclarationInstantiation (§19.2.1.3), including the browser Catch
// record exception and Annex B block-function replacement. Script and FDI
// collectors retain their independent declaration/provenance protocols.
impl Interpreter {
    fn eval_name_conflicts(&self, env: &EnvHandle, scope: &EnvHandle, name: &str) -> bool {
        let mut cursor = env.clone();
        while !Rc::ptr_eq(&cursor.0, &scope.0) {
            let frame = cursor.0.borrow();
            // Object records are transparent here: do not perform HasBinding,
            // unscopables or property operations. Only genuine CatchParameter
            // records get Annex B's browser exception; their body Blocks do not.
            if frame.binding_object.is_none() && frame.kind != EnvKind::Catch
                && cursor.local_cell(name).is_some() { return true; }
            let parent = frame.parent.clone().expect("eval lexical frame has a variable scope");
            drop(frame);
            cursor = parent;
        }
        scope.kind() == EnvKind::Global && scope.has_global_lexical(name)
    }

    fn can_declare_eval_global_var(&mut self, object: &Obj, name: &str) -> Result<bool, Throw> {
        if self.default_get_own_property(object, &PropertyKey::from(name))?.is_some() { return Ok(true); }
        self.default_is_extensible(object)
    }

    fn instantiate_eval_declarations(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<(), Throw> {
        // This structural collector separates direct VarScoped function
        // declarations from nested Annex B candidates and retains original
        // candidate addresses. No generic Script/FDI collector is changed.
        let mut plan = GlobalDeclarationPlan::new(body, self.strict_eval_context);
        plan.variables = eval_var_occurrences(body);
        let scope = env.var_scope();
        let global = scope.kind() == EnvKind::Global;
        let object = self.global_obj.clone();
        if !self.strict_eval_context {
            for name in plan.variables.iter().chain(plan.functions.iter().filter_map(|f| f.name.as_ref().map(|n| &n.name))) {
                if self.eval_name_conflicts(env, &scope, name) {
                    return Err(Throw::syntax_error(format!("identifier '{name}' has already been declared")));
                }
            }
        }
        let mut function_names = HashSet::new();
        let mut functions = Vec::new();
        for function in plan.functions.iter().rev() {
            let Some(name) = &function.name else { continue; };
            if !function_names.insert(name.name.clone()) { continue; }
            if global && !self.can_declare_global_function(&object, &name.name)? {
                return Err(Throw::type_error(format!("cannot declare global function '{}'", name.name)));
            }
            functions.push(function);
        }
        functions.reverse();
        let mut variables = Vec::new();
        for name in &plan.variables {
            if function_names.contains(name) { continue; }
            if global && !self.can_declare_eval_global_var(&object, name)? {
                return Err(Throw::type_error(format!("cannot declare global variable '{name}'")));
            }
            if !variables.contains(name) { variables.push(name.clone()); }
        }
        let mut annex_keys = HashSet::new();
        let mut declared_names = function_names.clone();
        declared_names.extend(variables.iter().cloned());
        for (key, name, eligible) in &plan.annex {
            if !eligible || plan.lexicals.iter().any(|(lexical, _)| lexical == name)
                || self.eval_name_conflicts(env, &scope, name) { continue; }
            // Annex B introduces a var binding, not an ordinary function
            // declaration. An ineligible extension is skipped, not rejected.
            if global && !self.can_declare_eval_global_var(&object, name)? { continue; }
            annex_keys.insert(*key);
            if declared_names.insert(name.clone()) {
                self.create_eval_binding(env, name, None)?;
            }
        }
        // Ordinary declaration rejection precedes old-scope mutation. Exotic
        // global internal methods remain observable and may be abrupt/reentrant.
        for (name, mutable) in &plan.lexicals { env.declare_uninitialized(name, *mutable)?; }
        for function in functions {
            let name = &function.name.as_ref().unwrap().name;
            let value = self.make_function(function, env);
            self.create_eval_binding(env, name, Some(value))?;
        }
        for name in variables { self.create_eval_binding(env, &name, None)?; }
        let mut eligible = self.annex_b_eligible.borrow_mut();
        for (key, _, _) in &plan.annex { eligible.remove(key); }
        eligible.extend(annex_keys);
        Ok(())
    }

    fn eval_global_descriptor(&mut self, object: &Obj, name: &str, value: JsValue, full: bool) -> Result<(), Throw> {
        let mut descriptor = JsObject::ordinary();
        descriptor.define("value", Property::data(value)).unwrap();
        if full {
            for attribute in ["writable", "enumerable", "configurable"] {
                descriptor.define(attribute, Property::data(JsValue::Bool(true))).unwrap();
            }
        }
        if !self.define_own_property(object, &PropertyKey::from(name), &JsValue::object(descriptor))? {
            return Err(Throw::type_error(format!("cannot declare eval global binding '{name}'")));
        }
        Ok(())
    }

    fn create_eval_binding(&mut self, env: &EnvHandle, name: &str, function: Option<JsValue>) -> Result<(), Throw> {
        let scope = env.var_scope();

        if scope.kind() != EnvKind::Global {
            if !self.strict_eval_context { return scope.declare_sloppy_eval_var(name, function); }
            if let Some(value) = function { return scope.declare_var(name, value); }
            return scope.hoist_var(name);
        }
        // Always use the actual active realm global object, never its public
        // stable WindowProxy. Eval names do not enter Script var-name history.
        let object = self.global_obj.clone();
        if let Some(value) = function {
            let full = self.default_get_own_property(&object, &PropertyKey::from(name))?
                .is_none_or(|property| property.configurable);
            self.eval_global_descriptor(&object, name, value.clone(), full)?;
            // CreateGlobalFunctionBinding requires Set(..., false) after
            // DefinePropertyOrThrow; a false Set result is not an abrupt throw.
            self.set_with_receiver(&object, &PropertyKey::from(name), value.clone(), JsValue::Object(object.clone()))?;
            scope.install_script_var_cell(name, value);
        } else {
            let absent = self.default_get_own_property(&object, &PropertyKey::from(name))?.is_none();
            // IsExtensible is a separate observable operation even if own.
            let extensible = self.default_is_extensible(&object)?;
            if absent && extensible {
                self.eval_global_descriptor(&object, name, JsValue::Undefined, true)?;
                self.set_with_receiver(&object, &PropertyKey::from(name), JsValue::Undefined, JsValue::Object(object.clone()))?;
            }
            scope.install_script_var_cell(name, JsValue::Undefined);

        }
        Ok(())
    }

    fn is_intrinsic_eval(&self, value: &JsValue) -> bool {
        let JsValue::Object(object) = value else { return false; };
        let object = object.borrow();
        object.realm == Some(self.host_realm) && object.func.as_ref().is_some_and(|function|
            matches!(&function.body, FunctionBody::Native(native)
                if std::ptr::fn_addr_eq(*native, native_eval as crate::value::NativeFn)))
    }
}

// Preserve every VarScoped name occurrence: CanDeclareGlobalVar is observable
// for each declaration, even when initialization later deduplicates names.
fn eval_var_occurrences(body: &[ast::Stmt]) -> Vec<String> {
    fn decl(d: &ast::VarDecl, names: &mut Vec<String>) {
        if d.kind == ast::VarKind::Var {
            for entry in &d.decls { names.extend(binding_names(&entry.binding)); }
        }
    }
    fn walk(s: &ast::Stmt, names: &mut Vec<String>) {
        match &s.kind {
            StmtKind::Var(d) => decl(d, names),
            StmtKind::Block(b) => { for s in &b.body { walk(s, names); } }
            StmtKind::If { then, els, .. } => { walk(then, names); if let Some(s) = els { walk(s, names); } }
            StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. }
            | StmtKind::With { body, .. } | StmtKind::Labeled { body, .. } => walk(body, names),
            StmtKind::For { init, body, .. } => {
                if let Some(ForInit::Decl(d)) = init { decl(d, names); }
                walk(body, names);
            }
            StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } | StmtKind::ForAwaitOf { left, body, .. } => {
                if let ForLeft::Decl(d) = left { decl(d, names); }
                walk(body, names);
            }
            StmtKind::Try(t) => {
                for s in &t.block.body { walk(s, names); }
                if let Some(c) = &t.catch { for s in &c.body.body { walk(s, names); } }
                if let Some(f) = &t.finally { for s in &f.body { walk(s, names); } }
            }
            StmtKind::Switch { cases, .. } => { for c in cases { for s in &c.body { walk(s, names); } } }
            _ => {}
        }
    }
    let mut names = Vec::new();
    for statement in body { walk(statement, &mut names); }
    names
}

fn collect_eval_declarations(body: &[ast::Stmt], functions: &mut Vec<String>, variables: &mut Vec<String>) {
    fn declaration(declaration: &ast::VarDecl, variables: &mut Vec<String>) {
        if declaration.kind == ast::VarKind::Var {
            for entry in &declaration.decls { variables.extend(binding_names(&entry.binding)); }
        }
    }
    for statement in body {
        match &statement.kind {
            StmtKind::Var(d) => declaration(d, variables),
            StmtKind::Function(f) => { if let Some(name) = &f.name { functions.push(name.name.clone()); } }
            StmtKind::Block(block) => collect_eval_declarations(&block.body, functions, variables),
            StmtKind::If { then, els, .. } => {
                collect_eval_declarations(std::slice::from_ref(then), functions, variables);
                if let Some(els) = els { collect_eval_declarations(std::slice::from_ref(els), functions, variables); }
            }
            StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. } | StmtKind::Labeled { body, .. } =>
                collect_eval_declarations(std::slice::from_ref(body), functions, variables),
            StmtKind::For { init, body, .. } => {
                if let Some(ForInit::Decl(d)) = init { declaration(d, variables); }
                collect_eval_declarations(std::slice::from_ref(body), functions, variables);
            }
            StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } | StmtKind::ForAwaitOf { left, body, .. } => {
                if let ForLeft::Decl(d) = left { declaration(d, variables); }
                collect_eval_declarations(std::slice::from_ref(body), functions, variables);
            }
            StmtKind::Try(statement) => {
                collect_eval_declarations(&statement.block.body, functions, variables);
                if let Some(catch) = &statement.catch { collect_eval_declarations(&catch.body.body, functions, variables); }
                if let Some(finally) = &statement.finally { collect_eval_declarations(&finally.body, functions, variables); }
            }
            StmtKind::Switch { cases, .. } => {
                for case in cases { collect_eval_declarations(&case.body, functions, variables); }
            }
            _ => {}
        }
    }
}
