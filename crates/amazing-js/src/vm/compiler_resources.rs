//! Disposal uses the VM's existing abrupt-completion/finally machinery. The
//! resource stack is owned by each heap frame, including across pending awaits.
use super::*;

impl Compiler {
    pub(super) fn for_iteration(&mut self, left: &ast::ForLeft, value: Register, body: &Stmt, span: Span) -> Result<(), CompileError> {
        let resources = matches!(left, ast::ForLeft::Decl(d) if d.resource_kind.is_some()).then(|| self.begin_resources());
        self.bind_for_native(left, value, span)?;
        self.loop_statement(body)?;
        self.emit(Instruction::LeaveScope);
        if let Some((slot, handler)) = resources { self.end_resources(slot, handler, span)?; }
        Ok(())
    }
    pub(super) fn begin_resources(&mut self) -> (usize, usize) {
        let slot = self.program.resource_slots;
        self.program.resource_slots += 1;
        self.emit(Instruction::ResourceStart(slot));
        let handler = self.emit(Instruction::PushTry { catch: None, finally: None, end: 0 });
        self.resource_stack.push(slot);
        (slot, handler)
    }

    pub(super) fn end_resources(&mut self, slot: usize, handler: usize, span: Span) -> Result<(), CompileError> {
        self.resource_stack.pop();
        self.emit(Instruction::TryNormal);
        let finally = self.program.code.len();
        self.emit(Instruction::ResourceBeginDispose(slot));
        let start = self.program.code.len();
        let value = self.register(span)?;
        let awaited = self.register(span)?;
        let done = self.register(span)?;
        self.emit(Instruction::ResourceNext(value, awaited, done, slot));
        let exit = self.emit(Instruction::Branch(done, Condition::Truthy, 0));
        if self.is_async {
            let sync = self.emit(Instruction::Branch(awaited, Condition::Falsy, 0));
            let awaiting = self.emit(Instruction::PushTry { catch: None, finally: None, end: 0 });
            let result = self.register(span)?;
            self.emit(Instruction::Await(result, value));
            self.emit(Instruction::TryNormal);
            let catch = self.program.code.len();
            let error = self.register(span)?;
            self.emit(Instruction::EnterCatchValue(error));
            self.emit(Instruction::ResourceError(slot, error));
            self.emit(Instruction::LeaveScope);
            self.emit(Instruction::TryNormal);
            let end = self.program.code.len();
            self.program.code[awaiting] = Instruction::PushTry { catch: Some(catch), finally: None, end };
            self.patch(sync);
        }
        self.emit(Instruction::Jump(start));
        self.patch(exit);
        self.emit(Instruction::ResourceFinish(slot));
        self.emit(Instruction::EndFinally);
        let end = self.program.code.len();
        self.program.code[handler] = Instruction::PushTry { catch: None, finally: Some(finally), end };
        Ok(())
    }

    pub(super) fn statements(&mut self, statements: &[Stmt], span: Span) -> Result<(), CompileError> {
        let resources = statements.iter().any(|s| matches!(&s.kind, StmtKind::Var(d) if d.resource_kind.is_some()))
            .then(|| self.begin_resources());
        self.emit(Instruction::CompleteEmpty);
        let pair = self.completion_pair(span)?;
        for statement in statements { self.completion_statement(statement, pair)?; }
        if let Some((slot, handler)) = resources { self.end_resources(slot, handler, span)?; }
        Ok(())
    }
}
