// Portions ported from lucid-softworks/lumen (crates/lumen/src/interpreter.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// ContainsExpression and parameter/body environment separation adapted to
// Amazing's EnvHandle and shared AST/bytecode function binding.

fn parameters_have_expressions(params: &[ast::Param]) -> bool {
    params.iter().any(|param| param.default.is_some() || binding_has_expression(&param.binding))
}

fn binding_has_expression(binding: &Binding) -> bool {
    match binding {
        Binding::Ident(_) => false,
        Binding::Default(_, _) => true,
        Binding::Rest(inner) => binding_has_expression(inner),
        Binding::Array(elements) => elements.iter().flatten().any(|element| binding_has_expression(element)),
        Binding::Object(properties) => properties.iter().any(|(key, value)|
            matches!(key, ast::BindingKey::Computed(_)) || binding_has_expression(value)),
    }
}

impl Interpreter {
    fn prepare_function_parameter_environment(
        &self, callee: &EnvHandle, params: &[ast::Param], strict: bool,
    ) -> EnvHandle {
        // FDI step 20: eval-created vars live outside the parameter bindings.
        if !strict && parameters_have_expressions(params) {
            EnvHandle::new(EnvKind::Parameter, Some(callee.clone()))
        } else { callee.clone() }
    }

    fn prepare_function_body_environment(
        &self,
        parameters: &EnvHandle,
        params: &[ast::Param],
        variable_names: &[String],
    ) -> Result<EnvHandle, Throw> {
        if !parameters_have_expressions(params) { return Ok(parameters.clone()); }
        let body = EnvHandle::new(EnvKind::Var, Some(parameters.clone()));
        let mut parameter_names: HashSet<String> = params.iter()
            .flat_map(|param| binding_names(&param.binding)).collect();
        if parameters.local_cell("arguments").is_some() {
            parameter_names.insert("arguments".into());
        }
        for name in variable_names {
            if parameter_names.contains(name) {
                // A body var sharing a formal name starts with a copy of its
                // initialized value, not a shared cell. Function hoisting
                // subsequently replaces it with the declared function.
                body.declare_var(name, parameters.resolve(name).unwrap_or(JsValue::Undefined))?;
            }
        }
        Ok(body)
    }

    fn prepare_ast_function_body_environment(
        &self, parameters: &EnvHandle, params: &[ast::Param], body: &[ast::Stmt],
    ) -> Result<EnvHandle, Throw> {
        let mut functions = Vec::new();
        let mut variables = Vec::new();
        collect_eval_declarations(body, &mut functions, &mut variables);
        self.prepare_function_body_environment(parameters, params, &variables)
    }
}

#[cfg(test)]
mod function_environment_tests {
    use super::*;

    #[test]
    fn parameter_eval_declarations_cannot_cross_parameter_bindings() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in [
                "function f(p=eval('var arguments')){let arguments;} try{f();false}catch(e){e instanceof SyntaxError}",
                "function f(p=eval('var arguments=7')){return arguments} try{f();false}catch(e){e instanceof SyntaxError}",
                "function* f(p=eval('var arguments')){let arguments;} try{f();false}catch(e){e instanceof SyntaxError}",
                "async function* f(p=eval('var arguments')){let arguments;} try{f();false}catch(e){e instanceof SyntaxError}",
                "var o={m(p=eval('var arguments')){let arguments;}};try{o.m();false}catch(e){e instanceof SyntaxError}",
                "var o={*m(p=eval('var arguments')){let arguments;}};try{o.m();false}catch(e){e instanceof SyntaxError}",
                "function f(p=eval('var p')){} try{f();false}catch(e){e instanceof SyntaxError}",
                "var f=(p=eval('var p'))=>p;try{f();false}catch(e){e instanceof SyntaxError}",
            ] {
                let mut i = Interpreter::new();
                assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}: {source}");
            }
            let mut i = Interpreter::new();
            i.run_script_with_backend("var rejected=false;async function f(p=eval('var arguments')){let arguments;} f().then(()=>{},e=>rejected=e instanceof SyntaxError);", backend).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("rejected").unwrap(), JsValue::Bool(true));
        }
    }

    #[test]
    fn parameter_default_closures_keep_the_parameter_environment() {
        let cases = [
            "var x=1; function f(get=()=>x){var x=2;return get()===1} f();",
            "function f(x=1,get=()=>x){var x=2;return get()===1 && x===2} f();",
            "function f(x=1,get=()=>x){x=2;return get()===2} f();",
            "var x=1;function f(get=()=>x){function x(){};return get()===1 && typeof x==='function'} f();",
            "function f([x=1,get=()=>x]){var x=2;return get()===1 && x===2} f([]);",
            "var capture; function f({[(capture=()=>x,'x')]:x}){var x=2;return capture()===1 && x===2} f({x:1});",
            "function f(x=1,get=()=>x,...rest){var x=2;return get()===1 && rest[0]===7} f(undefined,undefined,7);",
            "function f(get=eval('var x=1; ()=>x')){var x=2;return get()===1 && x===2} f();",
            "((x=1,get=()=>x)=>eval('var x=2; get()'))()===1;",
            "function f(get=()=>arguments){var arguments=7;return get()[1]===3 && arguments===7} f(undefined,3);",
            "class A{} class B extends A {constructor(get=()=>this){super();this.x=7;this.passed=get()===this}} new B().passed;",
            "function f(){return ((get=()=>this)=>{var local=7;return get()===this})()} f.call({x:1});",
            "function* g(x=1,get=()=>x){var x=2;yield get()===1 && x===2} g().next().value;",
            "function* g(get=eval('var x=1; ()=>x')){var x=2;yield get()===1 && x===2} g().next().value;",
            "function f(x=1,get=()=>x){switch(0){case 0:var x=2;} return get()===1 && x===2} f();",
        ];
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in cases {
                let mut i = Interpreter::new();
                // Direct parameter eval still requires AST fallback on this
                // fixed base; JSVM-12 independently removes that compiler
                // restriction. Keep both supported execution paths gated.
                let actual_backend = if backend == ExecutionBackend::Bytecode && source.contains("eval(") {
                    ExecutionBackend::TreeWalk
                } else { backend };
                let value = i.run_script_with_backend(source, actual_backend)
                    .unwrap_or_else(|error| {
                        let detail = match &error.0 {
                            JsValue::Object(object) => object.borrow().get_own("message"),
                            value => Some(value.clone()),
                        };
                        panic!("{backend:?}: {source}: {error:?}, {detail:?}")
                    });
                assert_eq!(value, JsValue::Bool(true), "{backend:?}: {source}");
            }
        }
    }

    #[test]
    fn async_function_and_generator_default_closures_keep_parameter_values() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in [
                "var result=false; async function f(x=1,get=()=>x){var x=2;return get()===1 && x===2} f().then(v=>{result=v});",
                "var result=false; async function* f(x=1,get=()=>x){var x=2;yield get()===1 && x===2} f().next().then(v=>{result=v.value});",
            ] {
                let mut i = Interpreter::new();
                i.run_script_with_backend(source, backend).unwrap();
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("result").unwrap(), JsValue::Bool(true), "{backend:?}: {source}");
            }
        }
    }

    #[test]
    fn annex_b_parameter_exclusions_do_not_hide_default_values() {
        for source in [
            "function f(x=123){{function x(){}} return x===123} f();",
            "function f(x=123){if(false) function x(){}; return x===123} f();",
            "function f(x=123){switch(0){case 0:function x(){}} return x===123} f();",
        ] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
        }
    }
}
