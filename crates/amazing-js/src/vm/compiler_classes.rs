// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// ClassDefinitionEvaluation source-order phases adapted to rooted VM slots.
use super::*;
use super::super::ClassHeritageCode;

impl Compiler {
    pub(super) fn suspended_class(&mut self, class: &ast::ClassItem, span: Span,
        name: Option<KeyOperand>) -> Result<Register, CompileError> {
        let previous = self.strict;
        self.strict = true;
        let result = self.suspended_class_inner(class, span, name);
        self.strict = previous;
        result
    }

    fn suspended_class_inner(&mut self, class: &ast::ClassItem, span: Span,
        name: Option<KeyOperand>) -> Result<Register, CompileError> {
        let slot = self.program.class_slots;
        self.program.class_slots += 1;
        self.emit(Instruction::ClassBegin(slot, Rc::new(class.clone()), name));
        self.scope_depth += 1;
        for decorator in &class.decorators {
            let (value, receiver) = self.chain_reference(decorator, false)?;
            self.emit(Instruction::ClassDecorator(slot, None, value, receiver));
        }
        let reference = class.super_class.as_ref().map(|_| {
            let reference = self.program.references;
            self.program.references += 1;
            reference
        });
        self.emit(Instruction::ClassEnterHeritage(slot, reference));
        let heritage = class.super_class.as_ref().map(|expression|
            self.class_heritage(expression, reference.expect("heritage slot"))).transpose()?;
        self.emit(Instruction::ClassPrepareElements(slot, heritage));
        for (index, member) in class.members.iter().enumerate() {
            let (decorators, key) = match member {
                ast::ClassMember::Method(method) if method.kind != ast::MethodKind::Constructor =>
                    (method.decorators.as_slice(), Some(&method.key)),
                ast::ClassMember::Field(field) => (field.decorators.as_slice(), Some(&field.key)),
                _ => (&[][..], None),
            };
            for decorator in decorators {
                let (value, receiver) = self.chain_reference(decorator, false)?;
                self.emit(Instruction::ClassDecorator(slot, Some(index), value, receiver));
            }
            let key = match key {
                Some(ast::PropKey::Computed(expression)) => {
                    let value = self.expression(expression)?;
                    self.emit(Instruction::ToKey(value));
                    Some(KeyOperand::Dynamic(value))
                }
                _ => None,
            };
            self.emit(Instruction::ClassAppendElement(slot, index, key));
        }
        let value = self.register(span)?;
        self.emit(Instruction::ClassFinish(value, slot));
        self.scope_depth -= 1;
        Ok(value)
    }

    fn class_heritage(&mut self, expression: &Expr, slot: usize) -> Result<ClassHeritageCode, CompileError> {
        self.enter(expression.span)?;
        let result = self.class_heritage_inner(expression, slot);
        self.depth -= 1;
        result
    }

    fn class_heritage_inner(&mut self, expression: &Expr, slot: usize) -> Result<ClassHeritageCode, CompileError> {
        if let ExprKind::Paren(inner) = &expression.kind { return self.class_heritage(inner, slot); }
        match &expression.kind {
            ExprKind::Ident(name) => {
                self.emit(Instruction::ResolveWriteName(slot, name.name.clone()));
                Ok(ClassHeritageCode::Reference(slot, 0))
            }
            ExprKind::Member { obj, prop, optional: false } if matches!(obj.kind, ExprKind::Super) => {
                let base = self.register(expression.span)?;
                let receiver = self.register(expression.span)?;
                self.emit(Instruction::LoadSuperHome(base));
                self.emit(Instruction::LoadThis(receiver));
                let key = self.member_key(base, prop, true)?;
                Ok(ClassHeritageCode::Super(base, key, receiver))
            }
            ExprKind::Member { .. } => {
                let fallback = self.heritage_reference(expression, slot)?;
                Ok(ClassHeritageCode::Reference(slot, fallback))
            }
            _ => self.expression(expression).map(ClassHeritageCode::Value),
        }
    }
}
