// Portions ported from boa-dev/boa (core/engine/src/bytecompiler/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: optional-chain reference
// preservation, link short-circuiting and final-link deletion.
use super::*;

enum Link<'a> {
    Member(&'a ast::MemberProp, bool),
    Call(&'a [Expr], bool),
}

pub(super) fn is_chain_expression(expression: &Expr) -> bool {
    in_optional_chain(expression) || matches!(&expression.kind,
        ExprKind::Unary { op: UnOp::Delete, operand } if in_optional_chain(operand))
}

impl Compiler {
    pub(super) fn suspended_chain(&mut self, expression: &Expr) -> Result<Register, CompileError> {
        match &expression.kind {
            ExprKind::Unary { op: UnOp::Delete, operand } => self.chain_reference(operand, true).map(|pair| pair.0),
            _ => self.chain_reference(expression, false).map(|pair| pair.0),
        }
    }

    // A pair retains the Reference receiver across a ParenthesizedExpression.
    // Each grouped expression has its own jump list: grouping ends propagation
    // of the short circuit, but does not turn a property Reference into a value.
    fn chain_reference(&mut self, expression: &Expr, delete: bool) -> Result<(Register, Register), CompileError> {
        self.enter(expression.span)?;
        let result = self.chain_reference_inner(expression, delete);
        self.depth -= 1;
        result
    }

    fn chain_reference_inner(&mut self, expression: &Expr, delete: bool) -> Result<(Register, Register), CompileError> {
        if let ExprKind::Paren(inner) = &expression.kind {
            return self.chain_reference(inner, delete);
        }
        let mut links = Vec::new();
        let mut base = expression;
        loop {
            match &base.kind {
                ExprKind::Member { obj, prop, optional } => {
                    links.push(Link::Member(prop, *optional));
                    base = obj;
                }
                // Direct eval and super calls keep their existing contextual
                // lowering. Optional eval calls are ordinary indirect calls.
                ExprKind::Call { callee, args, optional }
                    if *optional || (!matches!(&callee.kind, ExprKind::Super)
                        && !matches!(&callee.kind, ExprKind::Ident(name) if name.name == "eval")) => {
                    links.push(Link::Call(args, *optional));
                    base = callee;
                }
                _ => break,
            }
            if links.len() > MAX_DEPTH { return Err(self.error(expression.span, RESOURCE_LIMIT)); }
        }
        links.reverse();
        let value = self.register(expression.span)?;
        let receiver = self.register(expression.span)?;
        let mut super_base = matches!(&base.kind, ExprKind::Super);
        match &base.kind {
            ExprKind::Ident(name) => { self.emit(Instruction::LoadCallName(value, receiver, name.name.clone())); }
            ExprKind::Paren(inner) => {
                let (inner_value, inner_receiver) = self.chain_reference(inner, false)?;
                self.emit(Instruction::Move(value, inner_value));
                self.emit(Instruction::Move(receiver, inner_receiver));
            }
            ExprKind::Super => {
                self.emit(Instruction::LoadSuperHome(value));
                self.emit(Instruction::LoadThis(receiver));
            }
            _ => {
                let source = self.expression(base)?;
                self.emit(Instruction::Move(value, source));
                self.emit(Instruction::Move(receiver, 0));
            }
        }
        let mut jumps = Vec::new();
        let mut deleted = false;
        for (index, link) in links.iter().enumerate() {
            let optional = match link { Link::Member(_, optional) | Link::Call(_, optional) => *optional };
            if optional { jumps.push(self.emit(Instruction::Branch(value, Condition::Nullish, 0))); }
            match link {
                Link::Member(prop, _) => {
                    let object = self.register(expression.span)?;
                    self.emit(Instruction::Move(object, value));
                    if !super_base { self.emit(Instruction::Move(receiver, object)); }
                    if delete && index + 1 == links.len() {
                        if super_base { return Err(self.error(expression.span, "delete super member")); }
                        let key = self.member_key(object, prop, true)?;
                        self.emit(Instruction::DeleteMember(value, object, key));
                        deleted = true;
                    } else if let ast::MemberProp::Private(name) = prop {
                        self.emit(Instruction::GetPrivateField(value, object, name.name.clone()));
                    } else {
                        let key = self.member_key(object, prop, true)?;
                        self.emit(Instruction::GetMemberReceiver(value, object, key, receiver));
                    }
                }
                Link::Call(args, _) => {
                    let arguments = self.call_arguments(args)?;
                    self.emit(Instruction::Call(value, value, receiver, arguments));
                    self.emit(Instruction::Move(receiver, 0));
                }
            }
            super_base = false;
        }
        if delete && !deleted { self.emit(Instruction::Constant(value, JsValue::Bool(true))); }
        if !jumps.is_empty() {
            let normal = self.emit(Instruction::Jump(0));
            for jump in jumps { self.patch(jump); }
            self.emit(Instruction::Constant(value, if delete { JsValue::Bool(true) } else { JsValue::Undefined }));
            self.emit(Instruction::Move(receiver, 0));
            self.patch(normal);
        }
        Ok((value, receiver))
    }
}
