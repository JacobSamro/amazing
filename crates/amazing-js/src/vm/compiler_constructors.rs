// Live SuperCall lowering adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
use super::*;

impl Compiler {
    pub(super) fn super_constructor_call(&mut self, dst: Register, args: &[ast::Expr], span: Span)
        -> Result<(), CompileError>
    {
        // GetSuperConstructor precedes arguments. Root the nullable selected
        // value even when argv mutates the original constructor's prototype.
        let constructor = self.register(span)?;
        self.emit(Instruction::LoadSuperConstructor(constructor));
        let arguments = self.call_arguments(args)?;
        self.emit(Instruction::SuperCall(dst, constructor, arguments));
        Ok(())
    }
}
