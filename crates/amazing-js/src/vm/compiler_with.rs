//! Object-environment var initialization preserves the resolved reference
//! across initializer, iterator, property getter and default-expression calls.
use super::*;

impl Compiler {
    pub(super) fn with_binding_reference(&mut self, binding: &Binding) -> Option<usize> {
        match binding {
            Binding::Ident(name) => {
                let slot = self.program.references;
                self.program.references += 1;
                self.emit(Instruction::ResolveWriteName(slot, name.name.clone()));
                Some(slot)
            }
            Binding::Default(inner, _) | Binding::Rest(inner) => self.with_binding_reference(inner),
            _ => None,
        }
    }

    pub(super) fn with_binding_prepared(&mut self, binding: &Binding, value: Register, reference: Option<usize>, span: Span) -> Result<(), CompileError> {
        self.enter(span)?;
        let result = self.with_binding_inner(binding, value, reference, span);
        self.depth -= 1;
        result
    }

    fn with_binding_inner(&mut self, binding: &Binding, value: Register, reference: Option<usize>, span: Span) -> Result<(), CompileError> {
        match binding {
            Binding::Ident(_) => { self.emit(Instruction::StorePatternReference(reference.expect("prepared with binding"), value)); }
            Binding::Rest(inner) => self.with_binding_prepared(inner, value, reference, span)?,
            Binding::Default(inner, expression) => {
                let name = match inner.as_ref() { Binding::Ident(name) => Some(name.name.as_str()), _ => None };
                let value = self.pattern_default(value, expression, name)?;
                self.with_binding_prepared(inner, value, reference, span)?;
            }
            Binding::Array(elements) => {
                let (slot, handler) = self.pattern_start(value);
                for element in elements {
                    if let Some(element) = element {
                        let reference = self.with_binding_reference(element);
                        let item = self.register(span)?;
                        if matches!(element.as_ref(), Binding::Rest(_)) {
                            self.emit(Instruction::PatternRest(item, slot));
                        } else { self.emit(Instruction::PatternStep(item, slot)); }
                        self.with_binding_prepared(element, item, reference, span)?;
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
                            let reference = self.with_binding_reference(inner);
                            self.emit(Instruction::ObjectRest(item, value, keys.clone()));
                            self.with_binding_prepared(inner, item, reference, span)?;
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
                    let reference = self.with_binding_reference(inner);
                    self.emit(Instruction::GetMember(item, value, key));
                    self.with_binding_prepared(inner, item, reference, span)?;
                }
            }
        }
        Ok(())
    }
}
