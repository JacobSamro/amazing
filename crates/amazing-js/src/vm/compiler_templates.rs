// Portions ported from boa-dev/boa
// (core/engine/src/bytecompiler/expression/mod.rs, TaggedTemplate lowering)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Amazing retains the tag/reference pair in frame registers and delegates
// stable-site per-realm template construction to its shared semantic helper.
use super::*;

impl Compiler {
    pub(super) fn tagged_template(&mut self, dst: Register, tag: &Expr,
        template: &ast::TemplateLit, span: Span) -> Result<(), CompileError> {
        // GetValue of the tag precedes GetTemplateObject and every substitution.
        // A grouped property keeps its receiver; sequence/conditional values
        // discard it. The captured pair survives all substitution suspension.
        let (callee, receiver) = self.chain_reference(tag, false)?;
        let site = self.register(span)?;
        self.emit(Instruction::TemplateObject(site, Rc::new(template.clone())));
        let mut arguments = Vec::with_capacity(template.exprs.len() + 1);
        arguments.push(CallArgument::Value(site));
        for substitution in &template.exprs {
            arguments.push(CallArgument::Value(self.expression(substitution)?));
        }
        // Tagged substitutions retain their values without string coercion.
        // An eval tag is an ordinary call, never the direct-eval operation.
        self.emit(Instruction::Call(dst, callee, receiver, arguments));
        Ok(())
    }
}
