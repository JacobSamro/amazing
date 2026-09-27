// Compile async AST bodies independently of the surrounding script. Parameters
// remain bound by the interpreter, so defaults, patterns and arguments retain
// their existing environment. Compilation either succeeds before body execution
// or leaves the complete body on the AST path; there is no mid-body fallback.
impl Interpreter {
    fn async_body_code(&self, object: &Obj, function: &JsFunction, strict: bool) -> Option<vm::FunctionCode> {
        if !function.is_async || function.is_generator {
            return None;
        }
        if let Some(cached) = &object.borrow().async_body_code {
            return cached.clone().ok();
        }
        let body = match &function.body {
            FunctionBody::Block(body) => body.clone(),
            FunctionBody::Expr(expression) => ast::Block {
                body: vec![ast::Stmt::new(ast::StmtKind::Return(Some(expression.clone())), expression.span)],
                span: expression.span,
            },
            _ => return None,
        };
        let compiled = vm::compile_async_body(&body, strict);
        jsvm10_record("async", compiled.as_ref().map(|_| ()));
        object.borrow_mut().async_body_code = Some(compiled.clone());
        compiled.ok()
    }
}
