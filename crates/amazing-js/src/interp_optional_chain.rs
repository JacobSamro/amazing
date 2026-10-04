// Super Reference/GetValue portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Portions ported from boa-dev/boa (core/engine/src/bytecompiler/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Optional-chain control flow, receiver preservation and final-link deletion
// are adapted from compile_optional_preserve_this / compile_optional_delete
// to Amazing's tree walker and existing reference/property gateways.

enum ChainEvaluation {
    ShortCircuit,
    Value(JsValue),
    Property { base: JsValue, receiver: JsValue, key: PropertyKey, super_reference: bool },
    Private { receiver: JsValue, name: crate::value::PrivateName },
}

/// Parentheses terminate propagation without discarding a property's receiver.
fn optional_chain_link(expression: &ast::Expr) -> bool {
    match &expression.kind {
        ExprKind::Member { obj, optional, .. } => *optional || optional_chain_link(obj),
        ExprKind::Call { callee, optional, .. } => *optional || optional_chain_link(callee),
        _ => false,
    }
}

impl Interpreter {
    fn chain_value(&mut self, reference: ChainEvaluation) -> Result<JsValue, Throw> {
        match reference {
            ChainEvaluation::ShortCircuit => Ok(JsValue::Undefined),
            ChainEvaluation::Value(value) => Ok(value),
            ChainEvaluation::Property { base, receiver, key, .. } => self.get_prop_receiver(&base, &key, receiver),
            ChainEvaluation::Private { receiver, name } => private_field_get(self, &receiver, &name),
        }
    }

    fn chain_callee(&mut self, callee: &ast::Expr, env: &EnvHandle)
        -> Result<Option<(JsValue, JsValue)>, Throw>
    {
        if let Some(name) = event_handler::identifier_name(callee) {
            return self.scoped_call(env, name).map(Some);
        }
        let reference = self.eval_chain_reference(callee, env)?;
        let receiver = match &reference {
            ChainEvaluation::ShortCircuit => return Ok(None),
            ChainEvaluation::Property { receiver, .. } | ChainEvaluation::Private { receiver, .. } => receiver.clone(),
            ChainEvaluation::Value(_) => JsValue::Undefined,
        };
        Ok(Some((self.chain_value(reference)?, receiver)))
    }

    fn eval_chain_reference(&mut self, expression: &ast::Expr, env: &EnvHandle)
        -> Result<ChainEvaluation, Throw>
    {
        match &expression.kind {
            ExprKind::Paren(inner) => {
                let reference = self.eval_chain_reference(inner, env)?;
                Ok(match reference {
                    ChainEvaluation::ShortCircuit => ChainEvaluation::Value(JsValue::Undefined),
                    reference => reference,
                })
            }
            ExprKind::Member { obj, optional: false, .. } if matches!(obj.kind, ExprKind::Super) => {
                // SuperProperty evaluation captures actual this and the raw
                // computed expression before reading the current HomeObject's
                // prototype. GetValue then checks the base before ToPropertyKey.
                let mut reference = self.capture_reference(expression, env)?;
                self.coerce_reference_key(&mut reference)?;
                let Reference::SuperProperty(base, key, receiver) = reference else {
                    unreachable!("super member capture yields a super reference")
                };
                self.require_reference_base(&base)?;
                Ok(ChainEvaluation::Property { base, receiver, key, super_reference: true })
            }
            ExprKind::Member { obj, prop, optional } => {
                let reference = self.eval_chain_reference(obj, env)?;
                if matches!(reference, ChainEvaluation::ShortCircuit) { return Ok(reference); }
                let base = self.chain_value(reference)?;
                if *optional && (base.is_null() || base.is_undefined()) {
                    return Ok(ChainEvaluation::ShortCircuit);
                }
                let receiver = if matches!(obj.kind, ExprKind::Super) { env.resolve_this_checked()? } else { base.clone() };
                if let MemberProp::Private(identifier) = prop {
                    let name = env.resolve_private(&identifier.name).ok_or_else(|| {
                        Throw::syntax_error(format!("private name #{} is not defined", identifier.name))
                    })?;
                    return Ok(ChainEvaluation::Private { receiver, name });
                }
                let key_value = match prop {
                    MemberProp::Computed(key) => Some(self.eval_expr(key, env)?),
                    _ => None,
                };
                if base.is_null() || base.is_undefined() {
                    return Err(Throw::type_error("cannot access a property on nullish value"));
                }
                let key = match (prop, key_value) {
                    (MemberProp::Ident(name), _) => name.name.clone().into(),
                    (MemberProp::Computed(_), Some(value)) => self.to_property_key(&value)?,
                    _ => unreachable!(),
                };
                Ok(ChainEvaluation::Property { base, receiver, key, super_reference: matches!(obj.kind, ExprKind::Super) })
            }
            ExprKind::Call { callee, args, optional } => {
                if matches!(callee.kind, ExprKind::Super) {
                    return self.call_expr(callee, args, *optional, env).map(ChainEvaluation::Value);
                }
                let Some((function, receiver)) = self.chain_callee(callee, env)? else {
                    return Ok(ChainEvaluation::ShortCircuit);
                };
                if *optional && (function.is_null() || function.is_undefined()) {
                    return Ok(ChainEvaluation::ShortCircuit);
                }
                self.invoke_expression_call(callee, args, *optional, function, receiver, env)
                    .map(ChainEvaluation::Value)
            }
            _ => self.eval_expr(expression, env).map(ChainEvaluation::Value),
        }
    }

    fn delete_chain(&mut self, expression: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match self.eval_chain_reference(expression, env)? {
            ChainEvaluation::Property { super_reference: true, .. } => Err(Throw::reference_error("cannot delete super property")),
            ChainEvaluation::Property { base, key, .. } => self.delete_member_value(base, key),
            ChainEvaluation::Private { .. } => Err(Throw::syntax_error("private members cannot be deleted")),
            ChainEvaluation::ShortCircuit | ChainEvaluation::Value(_) => Ok(JsValue::Bool(true)),
        }
    }
}

#[cfg(test)]
mod optional_chain_tests {
    use super::*;

    #[test]
    fn short_circuit_spans_only_ungrouped_links_and_skips_all_work() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var calls=0,missing=null;
                function work(){calls++;return 'x';}
                var a=missing?.a[work()]().b, b=missing?.[work()].x(work());
                var c=(()=>undefined)?.()?.x.y;
                var grouped=false,unprotected=false;
                try{(missing?.a).b;}catch(e){grouped=e instanceof TypeError;}
                try{missing.a?.();}catch(e){unprotected=e instanceof TypeError;}
                calls===0 && a===undefined && b===undefined && c===undefined && grouped && unprotected;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn calls_keep_receivers_and_delete_avoids_getting_the_final_property() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var o={x:7,m(){return this.x;}, get doomed(){throw new Error('getter');}};
                var a=o?.m(),b=o.m?.(),c=(o?.m)(),d=(o?.m)?.(),tag=(o?.m)`x`;
                var calls=0,missing=null;
                var deleted=delete missing?.a[(calls++, 'x')].b, grouped=false;
                try{delete (missing?.a).b;}catch(e){grouped=e instanceof TypeError;}
                var real=delete o?.doomed;
                a===7 && b===7 && c===7 && d===7 && tag===7 && deleted && grouped && real && calls===0 && !('doomed' in o);
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
}
