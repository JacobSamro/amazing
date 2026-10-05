// Super pattern capture adapted from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// ForIn head initializer ordering adapted from boa-dev/boa
// (core/engine/src/bytecompiler/statement/loop.rs::compile_for_in_loop)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
//! Destructuring is lowered to ordinary resumable instructions. Iterator state
//! belongs to the VM frame, so defaults and computed targets may yield or await.
use super::*;

pub(super) fn pattern_target(mut target: &Expr) -> bool {
    while let ExprKind::Paren(inner) = &target.kind { target = inner; }
    matches!(target.kind, ExprKind::Array(_) | ExprKind::Object(_))
}

impl Compiler {
    pub(super) fn reserve_pattern(&mut self, binding: &Binding, mutable: bool) {
        for name in super::super::super::binding_names(binding) {
            self.emit(Instruction::ReserveBinding(name, mutable));
        }
    }

    pub(super) fn for_in_initializer(&mut self, left: &ast::ForLeft) -> Result<(), CompileError> {
        let ast::ForLeft::Decl(declaration) = left else { return Ok(()); };
        if declaration.kind != VarKind::Var { return Ok(()); }
        let Some(item) = declaration.decls.first() else { return Ok(()); };
        let (Binding::Ident(name), Some(initializer)) = (&item.binding, &item.init) else { return Ok(()); };
        let slot = self.program.references;
        self.program.references += 1;
        self.emit(Instruction::ResolveWriteName(slot, name.name.clone()));
        let value = self.named_class_expression(initializer, KeyOperand::Static(name.name.as_str().into()))?;
        self.emit(Instruction::NamedEvaluation(value, Rc::new(initializer.clone()), name.name.clone()));
        self.emit(Instruction::StoreReference(slot, value));
        Ok(())
    }

    pub(super) fn for_head_right(&mut self, left: &ast::ForLeft, right: &Expr) -> Result<Register, CompileError> {
        let lexical = match left { ast::ForLeft::Decl(d) if d.kind != VarKind::Var => Some(d), _ => None };
        if let Some(declaration) = lexical {
            self.emit(Instruction::EnterScope);
            for item in &declaration.decls { self.reserve_pattern(&item.binding, declaration.kind == VarKind::Let); }
        }
        let value = self.expression(right)?;
        if lexical.is_some() { self.emit(Instruction::LeaveScope); }
        Ok(value)
    }
    pub(super) fn hoist_pattern(&mut self, binding: &Binding) {
        match binding {
            Binding::Ident(name) => self.program.hoisted.push(name.name.clone()),
            Binding::Array(elements) => for element in elements.iter().flatten() { self.hoist_pattern(element); },
            Binding::Object(properties) => for (_, value) in properties { self.hoist_pattern(value); },
            Binding::Rest(inner) | Binding::Default(inner, _) => self.hoist_pattern(inner),
        }
    }

    pub(super) fn pattern_start(&mut self, value: Register) -> (usize, usize) {
        let slot = self.program.async_iterator_slots;
        self.program.async_iterator_slots += 1;
        self.emit(Instruction::PatternStart(slot, value));
        let handler = self.emit(Instruction::PushTry { catch: None, finally: None, end: 0 });
        (slot, handler)
    }

    pub(super) fn pattern_end(&mut self, slot: usize, handler: usize) {
        self.emit(Instruction::TryNormal);
        let finally = self.program.code.len();
        self.emit(Instruction::PatternClose(slot));
        self.emit(Instruction::EndFinally);
        let end = self.program.code.len();
        self.program.code[handler] = Instruction::PushTry { catch: None, finally: Some(finally), end };
    }

    pub(super) fn pattern_default(&mut self, source: Register, expression: &Expr, name: Option<&str>) -> Result<Register, CompileError> {
        let value = self.register(expression.span)?;
        self.emit(Instruction::Move(value, source));
        let branch = self.emit(Instruction::Branch(source, Condition::Undefined, 0));
        let skip = self.emit(Instruction::Jump(0));
        self.patch(branch);
        let fallback = match name {
            Some(name) => self.named_class_expression(expression, KeyOperand::Static(name.into()))?,
            None => self.expression(expression)?,
        };
        if let Some(name) = name { self.emit(Instruction::NamedEvaluation(fallback, Rc::new(expression.clone()), name.to_owned())); }
        self.emit(Instruction::Move(value, fallback));
        self.patch(skip);
        Ok(value)
    }

    pub(super) fn binding_pattern(&mut self, binding: &Binding, value: Register, kind: VarKind, span: Span) -> Result<(), CompileError> {
        self.enter(span)?;
        let result = self.binding_pattern_inner(binding, value, kind, span);
        self.depth -= 1;
        result
    }

    fn binding_pattern_inner(&mut self, binding: &Binding, value: Register, kind: VarKind, span: Span) -> Result<(), CompileError> {
        match binding {
            Binding::Ident(name) => { self.emit(Instruction::Declare(name.name.clone(), value, kind)); }
            Binding::Rest(inner) => self.binding_pattern(inner, value, kind, span)?,
            Binding::Default(inner, expression) => {
                let name = match inner.as_ref() { Binding::Ident(name) => Some(name.name.as_str()), _ => None };
                let value = self.pattern_default(value, expression, name)?;
                self.binding_pattern(inner, value, kind, span)?;
            }
            Binding::Array(elements) => {
                let (slot, handler) = self.pattern_start(value);
                for element in elements {
                    let item = self.register(span)?;
                    if let Some(element) = element {
                        if let Binding::Rest(inner) = element.as_ref() {
                            self.emit(Instruction::PatternRest(item, slot));
                            self.binding_pattern(inner, item, kind, span)?;
                        } else {
                            self.emit(Instruction::PatternStep(item, slot));
                            self.binding_pattern(element, item, kind, span)?;
                        }
                    } else { self.emit(Instruction::PatternSkip(slot)); }
                }
                self.pattern_end(slot, handler);
            }
            Binding::Object(properties) => {
                self.emit(Instruction::RequireObjectCoercible(value));
                let mut keys = Vec::new();
                for (key, inner) in properties {
                    let item = self.register(span)?;
                    let key = match key {
                        ast::BindingKey::Static(name) if name == "..." => {
                            self.emit(Instruction::ObjectRest(item, value, keys.clone()));
                            self.binding_pattern(inner, item, kind, span)?;
                            continue;
                        }
                        ast::BindingKey::Static(name) => KeyOperand::Static(name.as_str().into()),
                        ast::BindingKey::Computed(expression) => {
                            let key = self.expression(expression)?;
                            self.emit(Instruction::ToKey(key));
                            KeyOperand::Dynamic(key)
                        }
                    };
                    keys.push(key.clone());
                    self.emit(Instruction::GetMember(item, value, key));
                    self.binding_pattern(inner, item, kind, span)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn bind_for_native(&mut self, left: &ast::ForLeft, value: Register, span: Span) -> Result<(), CompileError> {
        if let ast::ForLeft::Decl(declaration) = left {
            if declaration.kind == VarKind::Var {
                for item in &declaration.decls { self.hoist_pattern(&item.binding); }
            }
        }
        // Plain identifiers can reuse the existing single-step binder. Every
        // pattern uses the same native lowering, whether or not it suspends.
        match left {
            ast::ForLeft::Decl(declaration) if declaration.kind == VarKind::Var && (self.with_depth != 0 || self.program.global_declarations.is_some()) => {
                self.emit(Instruction::EnterScope);
                for item in &declaration.decls {
                    let reference = self.with_binding_reference(&item.binding);
                    self.with_binding_prepared(&item.binding, value, reference, span)?;
                }
            }
            ast::ForLeft::Decl(declaration) if declaration.resource_kind.is_some() || declaration.decls.iter().any(|d| !matches!(d.binding, Binding::Ident(_))) => {
                self.emit(Instruction::EnterScope);
                if declaration.kind != VarKind::Var {
                    for item in &declaration.decls { self.reserve_pattern(&item.binding, declaration.kind == VarKind::Let); }
                }
                if let Some(kind) = declaration.resource_kind {
                    let slot = *self.resource_stack.last().expect("iteration resource scope");
                    self.emit(Instruction::ResourceCapture(slot, value, kind == ast::ResourceKind::AwaitUsing));
                }
                for declaration_item in &declaration.decls {
                    self.binding_pattern(&declaration_item.binding, value, declaration.kind, span)?;
                }
            }
            ast::ForLeft::Target(target) if pattern_target(target) || may_suspend(target) => {
                self.emit(Instruction::EnterScope);
                self.assignment_pattern(target, value)?;
            }
            _ => { self.emit(Instruction::BindFor(left.clone(), value)); }
        }
        Ok(())
    }

    pub(super) fn assignment_pattern(&mut self, target: &Expr, value: Register) -> Result<(), CompileError> {
        self.enter(target.span)?;
        let result = self.assignment_pattern_inner(target, value);
        self.depth -= 1;
        result
    }

    fn pattern_reference(&mut self, target: &Expr) -> Result<Option<usize>, CompileError> {
        self.enter(target.span)?;
        let result = (|| match &target.kind {
            ExprKind::Paren(inner) | ExprKind::Spread(inner) => self.pattern_reference(inner),
            ExprKind::Assign { op: AssignOp::Assign, target, .. } => self.pattern_reference(target),
            ExprKind::Array(_) | ExprKind::Object(_) => Ok(None),
            _ => {
                let slot = self.program.references;
                self.program.references += 1;
                if let Some(name) = super::super::super::event_handler::identifier_name(target) {
                    self.emit(Instruction::ResolveWriteName(slot, name.to_owned()));
                    Ok(Some(slot))
                } else if let Some((object, name)) = private_target(target) {
                    let object = self.expression(object)?;
                    self.emit(Instruction::ResolvePrivateWrite(slot, object, name.name.clone()));
                    Ok(Some(slot))
                } else if let ExprKind::Member { obj, prop, optional: false } = &target.kind {
                    if matches!(obj.kind, ExprKind::Super) {
                        // GetThisBinding precedes the raw key. GetSuperBase is
                        // live after key evaluation, and PutValue owns coercion.
                        let receiver = self.register(obj.span)?;
                        self.emit(Instruction::LoadThis(receiver));
                        let raw = match prop {
                            ast::MemberProp::Computed(key) => Some(self.expression(key)?),
                            _ => None,
                        };
                        let base = self.register(obj.span)?;
                        self.emit(Instruction::LoadSuperBaseNullable(base));
                        match (prop, raw) {
                            (ast::MemberProp::Computed(_), Some(key)) => self.emit(Instruction::ResolveRawSuperMemberWrite(slot, base, key, receiver)),
                            (ast::MemberProp::Ident(name), _) => self.emit(Instruction::ResolveSuperMemberWrite(slot, base, KeyOperand::Static(name.name.as_str().into()), receiver)),
                            _ => return Err(self.error(target.span, "invalid super property")),
                        };
                    } else {
                        let object = self.expression(obj)?;
                        self.write_member_reference(slot, object, prop)?;
                    }
                    Ok(Some(slot))
                } else if matches!(target.kind, ExprKind::Call { optional: false, .. }) {
                    self.expression(target)?;
                    self.emit(Instruction::RejectCallTarget);
                    Ok(Some(slot))
                } else { Err(self.error(target.span, "assignment target")) }
            }
        })();
        self.depth -= 1;
        result
    }

    fn assign_prepared(&mut self, target: &Expr, value: Register, reference: Option<usize>) -> Result<(), CompileError> {
        self.enter(target.span)?;
        let result = (|| match &target.kind {
            ExprKind::Paren(inner) | ExprKind::Spread(inner) => self.assign_prepared(inner, value, reference),
            ExprKind::Assign { op: AssignOp::Assign, target, value: default } => {
                let name = super::super::super::event_handler::identifier_name(target);
                let value = self.pattern_default(value, default, name)?;
                self.assign_prepared(target, value, reference)
            }
            _ => match reference {
                Some(slot) => { self.emit(Instruction::StorePatternReference(slot, value)); Ok(()) }
                None => self.assignment_pattern(target, value),
            }
        })();
        self.depth -= 1;
        result
    }

    fn assignment_pattern_inner(&mut self, target: &Expr, value: Register) -> Result<(), CompileError> {
        match &target.kind {
            ExprKind::Paren(inner) | ExprKind::Spread(inner) => self.assignment_pattern(inner, value)?,
            ExprKind::Assign { op: AssignOp::Assign, target, value: default } => {
                let name = super::super::super::event_handler::identifier_name(target);
                let value = self.pattern_default(value, default, name)?;
                self.assignment_pattern(target, value)?;
            }
            ExprKind::Array(elements) => {
                let (slot, handler) = self.pattern_start(value);
                for element in elements {
                    let item = self.register(target.span)?;
                    if let Some(element) = element {
                        if let ExprKind::Spread(inner) = &element.kind {
                            let reference = self.pattern_reference(inner)?;
                            self.emit(Instruction::PatternRest(item, slot));
                            self.assign_prepared(inner, item, reference)?;
                        } else {
                            let reference = self.pattern_reference(element)?;
                            self.emit(Instruction::PatternStep(item, slot));
                            self.assign_prepared(element, item, reference)?;
                        }
                    } else { self.emit(Instruction::PatternSkip(slot)); }
                }
                self.pattern_end(slot, handler);
            }
            ExprKind::Object(elements) => {
                self.emit(Instruction::RequireObjectCoercible(value));
                let mut keys = Vec::new();
                for element in elements {
                    let item = self.register(target.span)?;
                    match element {
                        ast::ObjElem::Spread(inner) => {
                            let reference = self.pattern_reference(inner)?;
                            self.emit(Instruction::ObjectRest(item, value, keys.clone()));
                            self.assign_prepared(inner, item, reference)?;
                        }
                        ast::ObjElem::Prop(property) => {
                            let key = self.object_key(&property.key, property.key_utf16.as_deref(), property.span)?;
                            keys.push(key.clone());
                            let inner = match &property.value {
                                ast::PropValue::Init(inner) => inner.clone(),
                                ast::PropValue::Shorthand(name) => Expr::new(ExprKind::Ident(name.clone()), name.span),
                                ast::PropValue::CoverInitializedName { binding, default } => {
                                    Expr::new(ExprKind::Assign { op: AssignOp::Assign,
                                        target: Box::new(Expr::new(ExprKind::Ident(binding.clone()), binding.span)),
                                        value: Box::new(default.clone()) }, property.span)
                                }
                                _ => return Err(self.error(property.span, "assignment pattern element")),
                            };
                            let reference = self.pattern_reference(&inner)?;
                            self.emit(Instruction::GetMember(item, value, key));
                            self.assign_prepared(&inner, item, reference)?;
                        }
                    }
                }
            }
            _ => {
                let reference = self.pattern_reference(target)?;
                self.assign_prepared(target, value, reference)?;
            }
        }
        Ok(())
    }
}
