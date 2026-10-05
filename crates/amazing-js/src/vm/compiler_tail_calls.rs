// Proper-tail expression traversal adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs::eval_return_expr) @
// f6c9860d44fa52249cfd647122edea184d32c863, MIT,
// Copyright (c) 2026 Lucid Softworks limited.
use super::*;

impl Compiler {
    pub(super) fn proper_tail_body(&self) -> bool {
        self.program.is_function
            && self.strict
            && !self.is_async
            && !self.is_generator
            && self.tail_calls_allowed
    }

    // Every branch emits its own terminal completion. Nested expressions keep
    // ordinary Call operations, so only the actual final call is replaced.
    pub(super) fn tail_return(&mut self, expression: &Expr) -> Result<(), CompileError> {
        self.enter(expression.span)?;
        let result = self.tail_return_inner(expression);
        self.depth -= 1;
        result
    }

    fn tail_return_inner(&mut self, expression: &Expr) -> Result<(), CompileError> {
        match &expression.kind {
            ExprKind::Paren(inner) => self.tail_return(inner),
            ExprKind::Sequence(expressions) if !expressions.is_empty() => {
                for expression in &expressions[..expressions.len() - 1] {
                    self.expression(expression)?;
                }
                self.tail_return(expressions.last().unwrap())
            }
            ExprKind::Conditional {
                test,
                consequent,
                alternate,
            } => {
                let condition = self.expression(test)?;
                let alternate_jump = self.emit(Instruction::Branch(condition, Condition::Falsy, 0));
                self.tail_return(consequent)?;
                self.patch(alternate_jump);
                self.tail_return(alternate)
            }
            ExprKind::Logical { op, left, right } => {
                let value = self.expression(left)?;
                let condition = match op {
                    ast::LogicalOp::And => Condition::Truthy,
                    ast::LogicalOp::Or => Condition::Falsy,
                    ast::LogicalOp::Coalesce => Condition::Nullish,
                };
                let take_right = self.emit(Instruction::Branch(value, condition, 0));
                self.emit(Instruction::Return(value));
                self.patch(take_right);
                self.tail_return(right)
            }
            ExprKind::Call { callee, .. } if !matches!(callee.kind, ExprKind::Super) => {
                let (value, _) = self.tail_chain_reference(expression)?;
                // The optional short-circuit and actual intrinsic direct eval
                // paths finish as ordinary returns, without evaluating twice.
                self.emit(Instruction::Return(value));
                Ok(())
            }
            ExprKind::TaggedTemplate { tag, template } => {
                let (callee, receiver) = self.chain_reference(tag, false)?;
                let site = self.register(expression.span)?;
                self.emit(Instruction::TemplateObject(site, Rc::new(template.clone())));
                let mut arguments = vec![CallArgument::Value(site)];
                for expression in &template.exprs {
                    arguments.push(CallArgument::Value(self.expression(expression)?));
                }
                self.emit(Instruction::TailCall(callee, receiver, arguments, false));
                Ok(())
            }
            _ => {
                let value = self.expression(expression)?;
                self.emit(Instruction::Return(value));
                Ok(())
            }
        }
    }
}
