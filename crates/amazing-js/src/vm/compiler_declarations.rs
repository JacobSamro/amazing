// Portions ported from lucid-softworks/lumen (crates/lumen/src/interpreter.rs,
// collect_annexb_funcs; crates/lumen/src/eval.rs, declare_block_lexicals)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
use super::*;

fn declaration(mut statement: &Stmt) -> Option<&ast::FunctionItem> {
    while let StmtKind::Labeled { body, .. } = &statement.kind { statement = body; }
    match &statement.kind { StmtKind::Function(function) => Some(function), _ => None }
}

impl Compiler {
    pub(super) fn annex_declarations(&mut self, body: &[Stmt], params: &[ast::Param]) {
        if self.strict { return; }
        let mut blocked = super::super::super::block_lexical_names(body);
        blocked.extend(params.iter().flat_map(|p| super::super::super::binding_names(&p.binding)));
        for statement in body { self.annex_statement(statement, false, &mut blocked); }
    }

    fn annex_body(&mut self, body: &[Stmt], blocked: &mut Vec<String>) {
        let old = blocked.len();
        blocked.extend(super::super::super::block_lexical_names(body));
        for statement in body {
            if declaration(statement).is_some() { self.annex_statement(statement, true, blocked); }
        }
        // Direct block functions are lexical barriers for deeper occurrences,
        // but not for themselves or their same-block duplicates.
        blocked.extend(body.iter().filter_map(declaration).filter_map(|f| f.name.as_ref().map(|n| n.name.clone())));
        for statement in body {
            if declaration(statement).is_none() { self.annex_statement(statement, true, blocked); }
        }
        blocked.truncate(old);
    }

    fn annex_statement(&mut self, statement: &Stmt, nested: bool, blocked: &mut Vec<String>) {
        match &statement.kind {
            StmtKind::Function(function) if nested && !function.is_generator && !function.is_async => {
                if let Some(name) = &function.name {
                    if !blocked.contains(&name.name) {
                        // Addresses are used only during compilation; Programs retain
                        // ordinary names and instructions, never these temporary tokens.
                        self.annex_b.insert(function.as_ref() as *const ast::FunctionItem as usize);
                        if !self.program.hoisted.contains(&name.name) { self.program.hoisted.push(name.name.clone()); }
                    }
                }
            }
            StmtKind::Block(block) => self.annex_body(&block.body, blocked),
            StmtKind::If { then, els, .. } => {
                self.annex_statement(then, true, blocked);
                if let Some(els) = els { self.annex_statement(els, true, blocked); }
            }
            StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. }
            | StmtKind::Labeled { body, .. } | StmtKind::With { body, .. } => self.annex_statement(body, true, blocked),
            StmtKind::For { init, body, .. } => {
                let old = blocked.len();
                if let Some(ast::ForInit::Decl(d)) = init {
                    if d.kind != VarKind::Var { blocked.extend(d.decls.iter().flat_map(|d| super::super::super::binding_names(&d.binding))); }
                }
                self.annex_statement(body, true, blocked);
                blocked.truncate(old);
            }
            StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } | StmtKind::ForAwaitOf { left, body, .. } => {
                let old = blocked.len();
                if let ast::ForLeft::Decl(d) = left {
                    if d.kind != VarKind::Var { blocked.extend(d.decls.iter().flat_map(|d| super::super::super::binding_names(&d.binding))); }
                }
                self.annex_statement(body, true, blocked);
                blocked.truncate(old);
            }
            StmtKind::Switch { cases, .. } => {
                let old = blocked.len();
                for case in cases { blocked.extend(super::super::super::block_lexical_names(&case.body)); }
                for case in cases { for stmt in &case.body { self.annex_statement(stmt, true, blocked); } }
                blocked.truncate(old);
            }
            StmtKind::Try(statement) => {
                self.annex_body(&statement.block.body, blocked);
                if let Some(catch) = &statement.catch {
                    let old = blocked.len();
                    if let Some(binding) = &catch.binding {
                        if !matches!(binding, Binding::Ident(_)) { blocked.extend(super::super::super::binding_names(binding)); }
                    }
                    self.annex_body(&catch.body.body, blocked);
                    blocked.truncate(old);
                }
                if let Some(finally) = &statement.finally { self.annex_body(&finally.body, blocked); }
            }
            _ => {}
        }
    }

    pub(super) fn instantiate_block(&mut self, body: &[Stmt]) {
        if !self.native_declarations { return; }
        let mut functions = Vec::new();
        for statement in body {
            match &statement.kind {
                StmtKind::Var(d) if d.kind != VarKind::Var => {
                    for item in &d.decls { self.reserve_pattern(&item.binding, d.kind == VarKind::Let); }
                }
                StmtKind::Class(class) => {
                    if let Some(name) = &class.name { self.emit(Instruction::ReserveBinding(name.name.clone(), true)); }
                }
                _ => {}
            }
            if let Some(function) = declaration(statement) {
                self.block_function_nodes.insert(function as *const ast::FunctionItem as usize);
                functions.push(Rc::new(function.clone()));
            }
        }
        if !functions.is_empty() { self.emit(Instruction::InitBlockFunctions(functions)); }
    }

    pub(super) fn if_branch(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        if self.native_declarations && matches!(statement.kind, StmtKind::Function(_)) {
            self.emit(Instruction::EnterScope);
            self.instantiate_block(std::slice::from_ref(statement));
            self.statement(statement)?;
            self.emit(Instruction::LeaveScope);
            Ok(())
        } else { self.statement(statement) }
    }
}
