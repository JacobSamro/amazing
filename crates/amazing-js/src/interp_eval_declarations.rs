// Eval declaration checks precede every binding mutation (§19.2.1.3).
impl Interpreter {
    fn validate_eval_global_declarations(&self, body: &[ast::Stmt], env: &EnvHandle) -> Result<(), Throw> {
        let scope = env.var_scope();
        let mut functions = Vec::new();
        let mut variables = Vec::new();
        collect_eval_declarations(body, &mut functions, &mut variables);
        // Parameter expressions keep formals and arguments in a lexical frame
        // outside eval's variable environment. Check these bindings before
        // creating any vars; ordinary block/catch declaration rules keep their
        // existing handling, including Annex B's simple catch exception.
        for name in functions.iter().chain(variables.iter()) {
            let mut cursor = env.clone();
            while !Rc::ptr_eq(&cursor.0, &scope.0) {
                if cursor.kind() == EnvKind::Parameter && cursor.has_local_lexical(name) {
                    return Err(Throw::syntax_error(format!("identifier '{name}' has already been declared")));
                }
                let parent = cursor.0.borrow().parent.clone().expect("lexical frame has a variable scope");
                cursor = parent;
            }
        }
        if scope.kind() != EnvKind::Global { return Ok(()); }
        let global = self.global_obj.borrow();
        for name in functions.iter().chain(variables.iter()) {
            if scope.has_local_lexical(name) {
                return Err(Throw::syntax_error(format!("identifier '{name}' has already been declared")));
            }
        }
        for name in &functions {
            let allowed = match global.own_prop(name.as_str()) {
                None => global.extensible,
                Some(property) => property.configurable || (property.writable && property.enumerable
                    && property.is_data()),
            };
            if !allowed { return Err(Throw::type_error("cannot declare global function")); }
        }
        for name in &variables {
            if !global.extensible && global.own_prop(name.as_str()).is_none() {
                return Err(Throw::type_error("cannot declare global variable"));
            }
        }
        Ok(())
    }

    fn create_eval_binding(&self, env: &EnvHandle, name: &str, function: Option<JsValue>) -> Result<(), Throw> {
        let scope = env.var_scope();
        let new_global = scope.kind() == EnvKind::Global && self.global_obj.borrow().own_prop(name).is_none();
        if new_global {
            self.global_obj.borrow_mut().define(name, Property::data(JsValue::Undefined))
                .map_err(|_| Throw::type_error("cannot create eval global binding"))?;
        }
        if let Some(value) = function { scope.declare_var(name, value)?; }
        else { scope.hoist_var(name)?; }
        if scope.kind() == EnvKind::Global { scope.register_script_var(name); }
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
