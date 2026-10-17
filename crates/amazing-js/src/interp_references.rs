// Portions ported/adapted from lucid-softworks/lumen (crates/lumen/src/eval.rs,
// Reference/GetValue and assignment-pattern evaluation)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Environment handles and raw property references use Amazing's value model.

impl Interpreter {
    fn require_reference_base(&self, base: &JsValue) -> Result<(), Throw> {
        if base.is_null() || base.is_undefined() {
            Err(Throw::type_error("cannot access a property on nullish value"))
        } else { Ok(()) }
    }

    fn to_numeric_value(&mut self, value: JsValue) -> Result<JsValue, Throw> {
        let primitive = self.primitive_number_hint(value)?;
        if matches!(primitive, JsValue::BigInt(_)) { Ok(primitive) }
        else { self.to_number(&primitive).map(JsValue::num) }
    }

    // Capture without GetValue: neither property-key coercion nor a getter may
    // run until the consuming GetValue/PutValue operation.
    fn capture_reference(&mut self, target: &ast::Expr, env: &EnvHandle) -> Result<Reference, Throw> {
        match &target.kind {
            ExprKind::Paren(inner) => self.capture_reference(inner, env),
            ExprKind::Ident(identifier) => self.name_reference(env, &identifier.name),
            ExprKind::Call { optional: false, .. } => {
                self.eval_expr(target, env)?;
                Err(Throw::reference_error("function call is not an assignment reference"))
            }
            ExprKind::Member { obj, prop, optional: false } => {
                let super_reference = matches!(obj.kind, ExprKind::Super);
                if super_reference {
                    let receiver = env.resolve_this_checked()?;
                    let raw = match prop {
                        MemberProp::Computed(expression) => Some(self.eval_expr(expression, env)?),
                        _ => None,
                    };
                    let home = self.frames.last().and_then(|frame| frame.home.clone())
                        .ok_or_else(|| Throw::syntax_error("super used outside a method"))?;
                    let base = home.borrow().proto.clone().map(JsValue::Object).unwrap_or(JsValue::Null);
                    return match (prop, raw) {
                        (MemberProp::Ident(identifier), _) => Ok(Reference::SuperProperty(base, PropertyKey::String(identifier.key.clone()), receiver)),
                        (MemberProp::Computed(_), Some(raw)) => Ok(Reference::RawSuperProperty(base, raw, receiver)),
                        _ => Err(Throw::syntax_error("invalid super property")),
                    };
                }
                let base = self.eval_expr(obj, env)?;
                let receiver = base.clone();
                match prop {
                    MemberProp::Private(identifier) => {
                        let name = env.resolve_private(&identifier.name).ok_or_else(||
                            Throw::syntax_error(format!("private name {} is not defined", identifier.name)))?;
                        Ok(Reference::Private(receiver, name))
                    }
                    MemberProp::Ident(identifier) => Ok(Reference::Property(base, PropertyKey::String(identifier.key.clone()))),
                    MemberProp::Computed(expression) => {
                        let raw = self.eval_expr(expression, env)?;
                        Ok(Reference::RawProperty(base, raw))
                    }
                }
            }
            _ => Err(Throw::reference_error("invalid assignment target")),
        }
    }

    fn coerce_reference_key(&mut self, reference: &mut Reference) -> Result<(), Throw> {
        // GetValue updates [[ReferencedName]] once; a compound assignment's
        // subsequent PutValue must reuse that exact key.
        match reference {
            Reference::RawProperty(base, raw) => {
                self.require_reference_base(base)?;
                let key = self.to_property_key(raw)?;
                *reference = Reference::Property(base.clone(), key);
            }
            Reference::RawSuperProperty(base, raw, receiver) => {
                self.require_reference_base(base)?;
                let key = self.to_property_key(raw)?;
                *reference = Reference::SuperProperty(base.clone(), key, receiver.clone());
            }
            _ => {}
        }
        Ok(())
    }

    fn reference_value(&mut self, reference: &mut Reference) -> Result<JsValue, Throw> {
        self.coerce_reference_key(reference)?;
        match reference {
            Reference::Cell(_, cell, _) => cell.read(),
            Reference::Binding(name, env) => {
                if let Some(global) = env.global_binding_object(name) {
                    return self.get_prop(&JsValue::Object(global), name.as_str());
                }
                env.local_cell(name).ok_or_else(|| Throw::reference_error(format!("{name} is not defined")))?.read()
            }
            Reference::ObjectBinding(object, name, _) => {
                let key = PropertyKey::from(name.as_str());
                if !self.default_has_property(object, &key)? {
                    if self.strict_eval_context { return Err(Throw::reference_error(format!("{name} is not defined"))); }
                    return Ok(JsValue::Undefined);
                }
                self.get_prop(&JsValue::Object(object.clone()), &key)
            }
            Reference::Unresolvable(name, _) => Err(Throw::reference_error(format!("{name} is not defined"))),
            Reference::Property(base, key) => { self.require_reference_base(base)?; self.get_prop(base, &*key) }
            Reference::SuperProperty(base, key, receiver) => {
                self.require_reference_base(base)?;
                self.get_prop_receiver(base, key, receiver.clone())
            }
            Reference::Private(base, name) => private_field_get(self, base, name),
            Reference::RawProperty(_, _) | Reference::RawSuperProperty(_, _, _) => unreachable!(),
        }
    }

    fn prepare_assignment_pattern(&mut self, target: &ast::Expr, env: &EnvHandle) -> Result<Option<Reference>, Throw> {
        match &target.kind {
            ExprKind::Paren(inner) | ExprKind::Spread(inner) => self.prepare_assignment_pattern(inner, env),
            ExprKind::Assign { op: ast::AssignOp::Assign, target, .. } => self.prepare_assignment_pattern(target, env),
            ExprKind::Array(_) | ExprKind::Object(_) => Ok(None),
            _ => self.capture_reference(target, env).map(Some),
        }
    }

    fn assign_prepared_pattern(&mut self, target: &ast::Expr, value: &JsValue, reference: Option<Reference>, env: &EnvHandle) -> Result<(), Throw> {
        match &target.kind {
            ExprKind::Paren(inner) | ExprKind::Spread(inner) => self.assign_prepared_pattern(inner, value, reference, env),
            ExprKind::Assign { op: ast::AssignOp::Assign, target, value: default } => {
                let mut assigned = value.clone();
                if assigned.is_undefined() {
                    assigned = if let Some(name) = event_handler::identifier_name(target) {
                        let mut value = self.eval_named_class_expr(default, env, &name.into())?;
                        self.named_evaluation(default, &mut value, name);
                        value
                    } else { self.eval_expr(default, env)? };
                }
                self.assign_prepared_pattern(target, &assigned, reference, env)
            }
            _ => if let Some(reference) = reference { self.put_reference(reference, value.clone(), env) }
                else { self.assign_pattern_expr(target, value, env) },
        }
    }
}

#[cfg(test)]
mod reference_order_tests {
    use super::*;
    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let result = Interpreter::new().run_script_with_backend(source, backend).unwrap_or_else(|error| {
                let detail = if let JsValue::Object(object) = &error.0 { object.borrow().get_own("message") } else { Some(error.0.clone()) };
                panic!("{backend:?}: {detail:?}");
            });
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn computed_write_defers_conversion_and_nullish_failure_until_rhs() {
        both(r#"
            var log=[], target={}, key={toString(){log.push('key');return 'x';}};
            target[key]=(log.push('rhs'), 7);
            var failure=false;
            try { null[key]=(log.push('null-rhs'),8); } catch(e){failure=e instanceof TypeError;}
            log.join(',')==='rhs,key,null-rhs' && target.x===7 && failure;
        "#);
    }
    #[test]
    fn destructuring_captures_target_before_get_default_and_rest() {
        both(r#"
            var log=[], target={}, key={toString(){log.push('key');return 'x';}};
            function receiver(){log.push('target');return target;}
            var source={get value(){log.push('get');return undefined;}};
            ({value:receiver()[key]=(log.push('default'),9)}=source);
            var first=log.join(',')==='target,get,default,key' && target.x===9;
            log=[];
            var iterable={ [Symbol.iterator](){return {next(){log.push('step');return {done:true};}};} };
            [receiver()[key]=(log.push('default'),10)] = iterable;
            var second=log.join(',')==='target,step,default,key' && target.x===10;
            log=[];
            ({...receiver()[key]}={get x(){log.push('copy');return 11;}});
            first && second && log.join(',')==='target,copy,key' && target.x.x===11;
        "#);
    }
    #[test]
    fn captured_with_binding_and_unresolvable_reference_survive_rhs() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut interpreter = Interpreter::new();
            // With is interpreted during closure creation. The captured writer
            // has an ordinary assignment body that can execute in either backend.
            interpreter.run_script_with_backend(r#"
                var outer=1, scope={outer:2};
                var writer;
                with(scope){writer=function(){outer=(delete scope.outer,3);};}
            "#, ExecutionBackend::TreeWalk).unwrap();
            let result = interpreter.run_script_with_backend(r#"
                writer();
                function strictMissing(){'use strict'; var rhs=false; try { ghost=(rhs=true,4); }catch(e){return rhs && e instanceof ReferenceError;} }
                var missing=false;try{absent++;}catch(e){missing=e instanceof ReferenceError;}
                var numeric='12', previous=numeric++, boxed={valueOf(){return 9;}}, old=boxed++;
                outer===1 && scope.outer===3 && strictMissing() && missing && previous===12 && numeric===13 && old===9 && boxed===10;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn rhs_eval_cannot_redirect_an_already_captured_declarative_reference() {
        both(r#"
            var x=1;
            function f(){x=eval('var x=2;3');return x;}
            var inner=f();
            x===3 && inner===2;
        "#);
    }
    #[test]
    fn abrupt_target_capture_closes_iterator_before_any_step() {
        both(r#"
            var log=[], marker={};
            var iterable={ [Symbol.iterator](){return {next(){log.push('step');return {value:1,done:false};},return(){log.push('close');return {};}};} };
            function target(){log.push('target');throw marker;}
            var caught=false;try{[target().x]=iterable;}catch(e){caught=e===marker;}
            caught && log.join(',')==='target,close';
        "#);
    }
}

#[cfg(test)]
mod reference_receiver_tests {
    use super::*;
    #[test]
    fn primitive_setters_receive_original_value_and_sloppy_failed_writes_are_ignored() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let result = Interpreter::new().run_script_with_backend(r#"
                var receiver, count=0;
                Object.defineProperty(Number.prototype,'sink',{set:function(v){'use strict';receiver=this;count=v;},configurable:true});
                (5).sink=3; (5).newProperty=7;
                var strict=false;try{(function(){'use strict';(5).newProperty=9;})();}catch(e){strict=e instanceof TypeError;}
                receiver===5 && count===3 && strict;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn numeric_operators_finish_left_conversion_before_right_conversion() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let result = Interpreter::new().run_script_with_backend(r#"
                var operations=[(a,b)=>a-b,(a,b)=>a*b,(a,b)=>a/b,(a,b)=>a%b,(a,b)=>a**b,
                    (a,b)=>a<<b,(a,b)=>a>>b,(a,b)=>a>>>b,(a,b)=>a&b,(a,b)=>a|b,(a,b)=>a^b];
                var calls=0, right={valueOf(){calls++;throw new Error('right');}};
                operations.every(function(operation){try{operation(Symbol(),right);}catch(e){return e instanceof TypeError;}}) && calls===0;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn super_computed_expression_precedes_base_capture_and_rhs_precedes_nullish_failure() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let result = Interpreter::new().run_script_with_backend(r#"
                var oldBase={x:1}, newBase={x:2}, log=[];
                var home={__proto__:oldBase, m(){return super[(Object.setPrototypeOf(home,newBase),'x')]++;},
                    n(){try {super[(log.push('expr'),'x')]=(log.push('rhs'),3);}catch(e){return e instanceof TypeError;}}};
                var old=home.m();Object.setPrototypeOf(home,null);
                old===2 && home.x===3 && home.n() && log.join(',')==='expr,rhs';
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
}
