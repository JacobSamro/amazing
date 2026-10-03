// StatementList / loop UpdateEmpty adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum CompletionBoundary {
    Saved(Register, Register),
    Undefined,
}

impl Compiler {
    pub(super) fn completion_pair(&mut self, span: Span) -> Result<(Register, Register), CompileError> {
        Ok((self.register(span)?, self.register(span)?))
    }

    pub(super) fn completion_statement(&mut self, statement: &Stmt, pair: (Register, Register)) -> Result<(), CompileError> {
        self.emit(Instruction::SaveCompletion(pair.0, pair.1));
        self.completion_stack.push(CompletionBoundary::Saved(pair.0, pair.1));
        let result = self.statement(statement);
        self.completion_stack.pop();
        self.emit(Instruction::UpdateEmptyCompletion(pair.0, pair.1));
        result
    }

    pub(super) fn loop_statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        let pair = self.completion_pair(statement.span)?;
        self.completion_statement(statement, pair)
    }

    pub(super) fn unwind_completions(&mut self, target_depth: usize) {
        let boundaries: Vec<_> = self.completion_stack[target_depth..].iter().rev().copied().collect();
        for boundary in boundaries {
            match boundary {
                CompletionBoundary::Saved(value, present) => { self.emit(Instruction::UpdateEmptyCompletion(value, present)); }
                CompletionBoundary::Undefined => { self.emit(Instruction::EnsureCompletionValue); }
            }
        }
    }
}
