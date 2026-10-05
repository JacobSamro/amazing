// Proper-tail return-expression and redispatch algorithms ported and adapted
// from lucid-softworks/lumen (crates/lumen/src/{eval,interpreter}.rs and
// interpreter/call_entry.rs) @ f6c9860d44fa52249cfd647122edea184d32c863,
// MIT, Copyright (c) 2026 Lucid Softworks limited.
// Amazing carries an explicit completion, never a pending global value marker.

/// The strong values needed by the next call; no caller registers or frame are
/// retained. Native graph tracing must count each real slot, including aliases.
pub(crate) struct TailCallRecord {
    pub(crate) callee: JsValue,
    pub(crate) receiver: JsValue,
    pub(crate) arguments: Vec<JsValue>,
    constructor: Option<TailConstructorResult>,
    forwarding_realm: Option<RealmHandle>,
    transparent_forward: bool,
}

pub(crate) enum CallOutcome {
    Value(JsValue),
    Tail(TailCallRecord),
}

// A derived [[Construct]] still checks its *final* return value after the
// tail-called [[Call]]. This one original result obligation does not retain
// its execution frame, and cannot accumulate with each tail redispatch.
struct TailConstructorResult {
    environment: EnvHandle,
    caller_realm: RealmHandle,
}

impl TailCallRecord {
    /// Collector/adoption inventory: aliases count once for *each physical
    /// slot*, not once per object identity. The record owns no other handles.
    #[allow(dead_code)]
    pub(crate) fn visit_strong_slots(
        &self,
        mut value: impl FnMut(&JsValue),
        mut environment: impl FnMut(&EnvHandle),
    ) {
        value(&self.callee);
        value(&self.receiver);
        for argument in &self.arguments {
            value(argument);
        }
        if let Some(policy) = &self.constructor {
            environment(&policy.environment);
        }
    }

    pub(crate) fn new(
        callee: JsValue,
        receiver: JsValue,
        arguments: Vec<JsValue>,
    ) -> Result<Self, Throw> {
        // EvaluateCall checks IsCallable after ArgumentListEvaluation and
        // before PrepareForTailCall. Class call rejection remains [[Call]].
        if !matches!(&callee, JsValue::Object(object) if object.borrow().func.is_some()) {
            return Err(Throw::type_error("value is not callable"));
        }
        Ok(Self {
            callee,
            receiver,
            arguments,
            constructor: None,
            forwarding_realm: None,
            transparent_forward: false,
        })
    }

    // Forwarding adapters have no retained execution frame, but the next
    // genuine call still observes the adapter's caller realm as incumbent.
    fn forwarded(mut self, realm: RealmHandle, transparent: bool) -> Self {
        self.forwarding_realm = Some(realm);
        self.transparent_forward = transparent;
        self
    }

    pub(crate) fn derived_result(
        mut self,
        environment: EnvHandle,
        caller_realm: RealmHandle,
    ) -> Self {
        self.constructor = Some(TailConstructorResult {
            environment,
            caller_realm,
        });
        self
    }
}

impl From<JsValue> for CallOutcome {
    fn from(value: JsValue) -> Self {
        Self::Value(value)
    }
}

impl Interpreter {
    fn finish_detached_tail_call(&mut self, record: TailCallRecord) -> Result<JsValue, Throw> {
        let TailCallRecord {
            callee,
            receiver,
            arguments,
            constructor,
            ..
        } = record;
        let value = self.call_value(&callee, receiver, arguments)?;
        if let Some(policy) = constructor {
            self.complete_derived_constructor(
                Flow::Return(value),
                &policy.environment,
                policy.caller_realm,
            )
        } else {
            Ok(value)
        }
    }

    /// Only called for an eligible ReturnStatement/ConciseBody. All other
    /// expression children evaluate normally, and retain their own context.
    fn eval_tail_expression(
        &mut self,
        expression: &ast::Expr,
        environment: &EnvHandle,
    ) -> Result<CallOutcome, Throw> {
        match &expression.kind {
            ExprKind::Paren(inner) => self.eval_tail_expression(inner, environment),
            ExprKind::Sequence(expressions) if !expressions.is_empty() => {
                for expression in &expressions[..expressions.len() - 1] {
                    self.eval_expr(expression, environment)?;
                }
                self.eval_tail_expression(expressions.last().unwrap(), environment)
            }
            ExprKind::Conditional {
                test,
                consequent,
                alternate,
            } => {
                let condition = self.eval_expr(test, environment)?.to_boolean();
                self.eval_tail_expression(
                    if condition { consequent } else { alternate },
                    environment,
                )
            }
            ExprKind::Logical { op, left, right } => {
                let value = self.eval_expr(left, environment)?;
                let take_right = match op {
                    ast::LogicalOp::And => value.to_boolean(),
                    ast::LogicalOp::Or => !value.to_boolean(),
                    ast::LogicalOp::Coalesce => value.is_null() || value.is_undefined(),
                };
                if take_right {
                    self.eval_tail_expression(right, environment)
                } else {
                    Ok(value.into())
                }
            }
            ExprKind::Call {
                callee,
                args,
                optional,
            } if !matches!(callee.kind, ExprKind::Super) => {
                let Some((function, receiver)) = self.chain_callee(callee, environment)? else {
                    return Ok(JsValue::Undefined.into());
                };
                if *optional && (function.is_null() || function.is_undefined()) {
                    return Ok(JsValue::Undefined.into());
                }
                let arguments = self.eval_args(args, environment)?;
                let direct = !optional && event_handler::identifier_name(callee) == Some("eval");
                if direct && self.is_intrinsic_eval(&function) {
                    return self
                        .invoke_evaluated_expression_call(
                            true,
                            function,
                            receiver,
                            arguments,
                            environment,
                        )
                        .map(CallOutcome::Value);
                }
                TailCallRecord::new(function, receiver, arguments).map(CallOutcome::Tail)
            }
            ExprKind::TaggedTemplate { tag, template } => {
                let (function, receiver) = self.resolve_tag_callee(tag, environment)?;
                let object = self.get_template_object(template)?;
                let mut arguments = Vec::with_capacity(template.exprs.len() + 1);
                arguments.push(object);
                for expression in &template.exprs {
                    arguments.push(self.eval_expr(expression, environment)?);
                }
                TailCallRecord::new(function, receiver, arguments).map(CallOutcome::Tail)
            }
            _ => self
                .eval_expr(expression, environment)
                .map(CallOutcome::Value),
        }
    }

    /// The genuine call/apply builtins perform their argument protocol in their
    /// own realm, then discard that activation before calling the target.
    fn native_tail_forward(
        &mut self,
        native: crate::value::NativeFn,
        receiver: &JsValue,
        arguments: &[JsValue],
    ) -> Option<Result<TailCallRecord, Throw>> {
        if std::ptr::fn_addr_eq(native, native_function_call as crate::value::NativeFn) {
            return Some((|| {
                if callable_function(receiver).is_none() {
                    return Err(Throw::type_error(
                        "Function.prototype.call receiver is not callable",
                    ));
                }
                TailCallRecord::new(
                    receiver.clone(),
                    arguments.first().cloned().unwrap_or(JsValue::Undefined),
                    arguments.get(1..).unwrap_or(&[]).to_vec(),
                )
            })());
        }
        if std::ptr::fn_addr_eq(native, native_function_apply as crate::value::NativeFn) {
            return Some((|| {
                if callable_function(receiver).is_none() {
                    return Err(Throw::type_error(
                        "Function.prototype.apply receiver is not callable",
                    ));
                }
                let list = match arguments.get(1) {
                    None | Some(JsValue::Undefined) | Some(JsValue::Null) => Vec::new(),
                    Some(value) => self.create_list_from_array_like(value)?,
                };
                TailCallRecord::new(
                    receiver.clone(),
                    arguments.first().cloned().unwrap_or(JsValue::Undefined),
                    list,
                )
            })());
        }
        if std::ptr::fn_addr_eq(native, std_reflect_apply as crate::value::NativeFn) {
            return Some((|| {
                let target = arguments.first().cloned().unwrap_or(JsValue::Undefined);
                if !matches!(&target, JsValue::Object(object) if object.borrow().func.is_some()) {
                    return Err(Throw::type_error("Reflect.apply target is not callable"));
                }
                let list = self.create_list_from_array_like(
                    &arguments.get(2).cloned().unwrap_or(JsValue::Undefined),
                )?;
                TailCallRecord::new(
                    target,
                    arguments.get(1).cloned().unwrap_or(JsValue::Undefined),
                    list,
                )
            })());
        }
        None
    }

    fn proxy_call_forward(
        &mut self,
        object: &Obj,
        receiver: JsValue,
        arguments: Vec<JsValue>,
    ) -> Result<TailCallRecord, Throw> {
        let (target, handler) = self.proxy_parts(object).expect("Proxy call")?;
        if !matches!(&target, JsValue::Object(object) if object.borrow().func.is_some()) {
            return Err(Throw::type_error("proxy target is not callable"));
        }
        match self.proxy_trap(&handler, "apply")? {
            Some(trap) => {
                let list = array_values(self, arguments)?;
                TailCallRecord::new(trap, handler, vec![target, receiver, list])
            }
            None => TailCallRecord::new(target, receiver, arguments),
        }
    }
}
