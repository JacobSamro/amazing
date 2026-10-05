// Proper-tail outcome/redispatch adaptation from lucid-softworks/lumen
// (crates/lumen/src/{eval,interpreter}.rs and interpreter/call_entry.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Super-reference read portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, super reference and member/index paths)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Portions ported from boa-dev/boa (core/engine/src/bytecompiler/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: optional-chain reference
// preservation, link short-circuiting, final-link deletion and whole call
// lowering (::call): grouped receivers and contextual eval after arguments.
use super::*;

enum Link<'a> {
    Member(&'a ast::MemberProp, bool),
    Call(&'a [Expr], bool, bool),
}

pub(super) fn is_chain_expression(expression: &Expr) -> bool {
    in_optional_chain(expression) || matches!(&expression.kind,
        ExprKind::Unary { op: UnOp::Delete, operand } if in_optional_chain(operand))
}

// Only reference-preserving read/call spines qualify. Arguments and other
// value expressions do not inherit the callee's Reference; assignments,
// updates, deletion and bare super construction retain their own consumers.
pub(super) fn is_super_read(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Paren(inner) => is_super_read(inner),
        ExprKind::Member { obj, .. } => matches!(obj.kind, ExprKind::Super) || is_super_read(obj),
        ExprKind::Call { callee, .. } => is_super_read(callee),
        _ => false,
    }
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
    pub(super) fn chain_reference(&mut self, expression: &Expr, delete: bool) -> Result<(Register, Register), CompileError> {
        self.enter(expression.span)?;
        let result = self.chain_reference_inner(expression, delete, None, false);
        self.depth -= 1;
        result
    }

    pub(super) fn tail_chain_reference(&mut self, expression: &Expr) -> Result<(Register, Register), CompileError> {
        self.enter(expression.span)?;
        let result = self.chain_reference_inner(expression, false, None, true);
        self.depth -= 1;
        result
    }

    pub(super) fn heritage_reference(&mut self, expression: &Expr, slot: usize) -> Result<Register, CompileError> {
        self.enter(expression.span)?;
        let result = self.chain_reference_inner(expression, false, Some(slot), false).map(|pair| pair.0);
        self.depth -= 1;
        result
    }

    fn chain_reference_inner(&mut self, expression: &Expr, delete: bool, capture: Option<usize>, tail: bool) -> Result<(Register, Register), CompileError> {
        if let ExprKind::Paren(inner) = &expression.kind {
            if let Some(slot) = capture { return self.heritage_reference(inner, slot).map(|value| (value, 0)); }
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
                // Super construction retains its own lowering. A direct eval
                // candidate retains its captured value until arguments finish.
                ExprKind::Call { callee, args, optional }
                    if *optional || !matches!(&callee.kind, ExprKind::Super) => {
                    let direct = !*optional && super::super::super::event_handler::identifier_name(callee) == Some("eval");
                    links.push(Link::Call(args, *optional, direct));
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
                // A Super Reference captures actual this before evaluating a
                // raw computed key. Its HomeObject prototype is read only
                // after that expression, including when it suspends.
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
            let optional = match link { Link::Member(_, optional) | Link::Call(_, optional, _) => *optional };
            if optional { jumps.push(self.emit(Instruction::Branch(value, Condition::Nullish, 0))); }
            match link {
                Link::Member(prop, _) => {
                    let object = self.register(expression.span)?;
                    let raw_super_key = if super_base {
                        let raw = match prop {
                            ast::MemberProp::Computed(key) => Some(self.expression(key)?),
                            _ => None,
                        };
                        self.emit(Instruction::LoadSuperHome(object));
                        raw
                    } else {
                        self.emit(Instruction::Move(object, value));
                        None
                    };
                    if !super_base { self.emit(Instruction::Move(receiver, object)); }
                    if delete && index + 1 == links.len() {
                        if super_base { return Err(self.error(expression.span, "delete super member")); }
                        let key = self.member_key(object, prop, true)?;
                        self.emit(Instruction::DeleteMember(value, object, key));
                        deleted = true;
                    } else if let Some(slot) = capture.filter(|_| index + 1 == links.len()) {
                        if super_base { return Err(self.error(expression.span, "class heritage super reference")); }
                        if let ast::MemberProp::Private(name) = prop {
                            self.emit(Instruction::ResolvePrivateWrite(slot, object, name.name.clone()));
                        } else {
                            let key = self.member_key(object, prop, true)?;
                            self.emit(Instruction::ResolveMemberWrite(slot, object, key));
                        }
                        self.emit(Instruction::Move(value, 0));
                    } else if let ast::MemberProp::Private(name) = prop {
                        self.emit(Instruction::GetPrivateField(value, object, name.name.clone()));
                    } else {
                        let key = if let Some(raw) = raw_super_key {
                            self.emit(Instruction::RequireObjectCoercible(object));
                            self.emit(Instruction::ToKey(raw));
                            KeyOperand::Dynamic(raw)
                        } else { self.member_key(object, prop, true)? };
                        self.emit(Instruction::GetMemberReceiver(value, object, key, receiver));
                    }
                }
                Link::Call(args, _, direct) => {
                    let arguments = self.call_arguments(args)?;
                    self.emit(if tail && index + 1 == links.len() {
                        Instruction::TailCall(value, receiver, arguments, *direct)
                    } else if *direct { Instruction::CallEval(value, value, receiver, arguments) }
                    else { Instruction::Call(value, value, receiver, arguments) });
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
