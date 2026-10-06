// Portions adapted from lucid-softworks/lumen (crates/lumen/src/eval.rs,
// eval_delete/delete_ident) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Reference and environment operations use Amazing's existing semantic gateways.
use super::*;

impl Compiler {
    pub(super) fn delete_expression(&mut self, operand: &Expr) -> Result<Register, CompileError> {
        let mut target = operand;
        while let ExprKind::Paren(inner) = &target.kind { target = inner; }
        match &target.kind {
            ExprKind::Member { obj, prop, optional: false } if matches!(obj.kind, ExprKind::Super) => {
                // A raw Super Reference is rejected before ToPropertyKey and
                // ToObject. GetThisBinding and the computed expression remain
                // observable, including suspension and abrupt completion.
                let value = self.register(operand.span)?;
                self.emit(Instruction::LoadThis(value));
                if let ast::MemberProp::Computed(key) = prop { self.expression(key)?; }
                self.emit(Instruction::RejectSuperDelete);
                Ok(value)
            }
            ExprKind::Ident(name) => {
                let value = self.register(operand.span)?;
                self.emit(Instruction::DeleteName(value, name.name.clone()));
                Ok(value)
            }
            ExprKind::Member { prop: ast::MemberProp::Private(_), .. } => {
                // Private Delete early errors remain the parser-owned boundary.
                Err(self.error(operand.span, "delete private member"))
            }
            ExprKind::Member { .. } | ExprKind::Call { .. } => {
                // Final calls are values: execute the full rooted chain, then
                // yield true. Reuse its actual receiver/optional/direct-eval
                // semantics instead of an eager whole-expression adapter.
                self.chain_reference(target, true).map(|pair| pair.0)
            }
            _ => {
                // Non-reference operands still perform their complete evaluation
                // (including getters and suspension), then delete to true.
                self.expression(operand)?;
                let value = self.register(operand.span)?;
                self.emit(Instruction::Constant(value, JsValue::Bool(true)));
                Ok(value)
            }
        }
    }
}
