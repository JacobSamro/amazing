// Portions ported from boa-dev/boa (core/engine/src/builtins/eval/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// PerformEval lexical flags, fresh lexical scope and saved-context restoration
// adapted to Amazing's parser context, EnvHandle and native callable frames.

#[derive(Clone, Copy, Default)]
struct EvalGrammar {
    allow_new_target: bool,
    allow_super_property: bool,
    allow_super_call: bool,
    class_initializer: bool,
}

#[cfg(test)]
mod eval_context_runtime_tests {
    use super::*;

    fn check(sources: &[&str]) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in sources {
                let mut i = Interpreter::new();
                let result = i.run_script_with_backend(source, backend)
                    .unwrap_or_else(|error|panic!("{backend:?}: {source}: {error:?}"));
                assert_eq!(result, JsValue::Bool(true), "{backend:?}: {source}");
                assert!(i.frames.is_empty(), "eval restored the native/caller frame stack");
                assert!(i.eval_grammar_override.is_none());
            }
        }
    }

    #[test]
    fn direct_eval_inherits_function_method_constructor_and_private_contexts() {
        check(&[
            "function F(){return eval('new.target')} F()===undefined && new F()===F",
            "function F(){return ()=>eval('new.target')} var a=F();a()===undefined && (new F())()===F",
            "class A{get x(){return this.y}}class B extends A{m(){return eval('super.x')}}var b=new B();b.y=7;b.m()===7",
            "class A{}class B extends A{constructor(){eval('super()');this.x=7}}new B().x===7",
            "class A{#x=3;m(){return eval('this.#x')}}new A().m()===3",
            "class A{#x=4;m(){return function(){return eval('this.#x')}.call(this)}}new A().m()===4",
            "class A{#x=5;m(){return ()=>eval('this.#x')}}new A().m()()===5",
            "class A{get x(){return this.y}}class B extends A{m(){return eval('()=>eval(\"super.x\")')}}var b=new B();b.y=9;b.m()()===9",
            "class A{x=eval('new.target');static{this.ok=eval('new.target')===undefined}}new A().x===undefined && A.ok",
            "function outer(){return inner()}function inner(){try{eval('throw 1')}catch(e){}return inner.caller===outer && eval('inner.caller')===outer}outer()",
        ]);
    }

    #[test]
    fn eval_grammar_resets_for_indirect_eval_and_ordinary_functions() {
        check(&[
            "var f=()=>eval('new.target'),ok=false;try{f()}catch(e){ok=e instanceof SyntaxError}ok",
            "class A{f=()=>eval('arguments')}var ok=false;try{new A().f()}catch(e){ok=e instanceof SyntaxError}ok",
            "class A{static f=()=>eval('arguments')}var ok=false;try{A.f()}catch(e){ok=e instanceof SyntaxError}ok",
            "class A{f=(()=>()=>eval('arguments'))()}var ok=false;try{new A().f()}catch(e){ok=e instanceof SyntaxError}ok",
            "class A{f=function(){return eval('arguments.length')}}new A().f(1,2)===2",
            "class A{f=eval('(function(){return ()=>eval(\"arguments.length\")})()')}new A().f()===0",
            "class A{m(){try{return (0,eval)('super.x')}catch(e){return e instanceof SyntaxError}}}new A().m()",
            "class A{}class B extends A{m(){return (0,eval)('()=>eval(\"super.x\")')}}var f=new B().m(),ok=false;try{f()}catch(e){ok=e instanceof SyntaxError}ok",
            "function F(){return (0,eval)('()=>eval(\"new.target\")')}var f=new F(),ok=false;try{f()}catch(e){ok=e instanceof SyntaxError}ok",
            "class A{#x=1;m(){try{return (0,eval)('this.#x')}catch(e){return e instanceof SyntaxError}}}new A().m()",
            "function f(){var n=0;for(var s of ['return 1','yield 1','await 1']){try{eval(s)}catch(e){if(e instanceof SyntaxError)n++}}return n===3}f()",
        ]);
    }

    #[test]
    fn eval_lexical_bindings_are_fresh_tdz_checked_and_retained_by_closures() {
        check(&[
            "function f(){let x=1;eval('let x=2');return x===1}f()",
            "var x=1;eval('let x=2');x===1",
            "function f(){let x=1;try{eval('typeof x;let x=2')}catch(e){return e instanceof ReferenceError}return false}f()",
            "eval('let [x,y]=[1,2];()=>x+y')()===3",
            "typeof eval('class X{};()=>X')()==='function'",
            "eval('\"use strict\";var x=2');typeof x==='undefined'",
            "function f(){eval('var x=2');return x===2}f()",
            "function f(){eval('\"use strict\";var x=2');return typeof x==='undefined'}f()",
            "var x={toString(){throw 1}};eval(x)===x",
            "(0,eval)('let x=2');typeof x==='undefined'",
            "eval('{function f(){return 3}}');f()===3 && delete globalThis.f",
            "function f(p){eval('{function p(){return 7}}');return p()===7}f(1)",
            "var after;function f(){eval('{function g(){return 4}}after=g;var g=123')}f();after()===4",
            "eval('function f(p){{function p(){return 9}}return p===1}');f(1)",
        ]);
    }

    #[test]
    fn ast_fallback_arrows_retain_lexical_eval_grammar() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            // Scoped declarations deliberately select the whole callable AST
            // fallback. Seed through TreeWalk; exercise invocation in both modes.
            i.run_script("function F(){return ()=>{if(false){function unused(){}}return eval('new.target')}};class A{f=()=>{if(false){function unused(){}}return eval('arguments')}}").unwrap();
            assert_eq!(i.run_script_with_backend(
                "var f=new F();var ok=f()===F;try{new A().f();ok=false}catch(e){ok=ok&&e instanceof SyntaxError}ok", backend,
            ).unwrap(), JsValue::Bool(true));
        }
    }
}

impl Interpreter {
    fn lexical_eval_grammar(&self) -> EvalGrammar {
        let active = self.frames.iter().enumerate().rev().find(|(_, frame)| {
            let Some(function) = &frame.own_function else { return true; };
            function.borrow().func.as_ref().is_some_and(|function|
                !matches!(function.body, FunctionBody::Native(_) | FunctionBody::HostNative(..)))
        });
        if let Some((depth, grammar)) = self.eval_grammar_override {
            if active.is_none_or(|(index, _)| index < depth) { return grammar; }
        }
        let Some((_, frame)) = active else { return EvalGrammar::default(); };
        let (allow_new_target, class_initializer) = if let Some(object) = &frame.own_function {
            let object = object.borrow();
            let arrow = object.func.as_ref().is_some_and(|function|function.is_arrow);
            (!arrow || object.lexical_new_target_allowed, arrow && object.lexical_class_initializer)
        } else { (true, true) };
        EvalGrammar {
            allow_new_target,
            allow_super_property: frame.home.is_some(),
            allow_super_call: frame.super_constructor.is_some(),
            class_initializer,
        }
    }

    fn perform_eval(&mut self, input: JsValue) -> Result<JsValue, Throw> {
        // Non-String values are returned unchanged, before touching context.
        let JsValue::Str(source) = input else { return Ok(input); };
        let direct = self.direct_eval_call;
        let global = self.global_with_active_source_context();
        let caller = if direct { self.direct_eval_env.clone().unwrap_or_else(||global.clone()) }
            else { global };
        let grammar = if direct { self.lexical_eval_grammar() } else { EvalGrammar::default() };
        let context = crate::parser::EvalContext {
            strict: direct && self.strict_eval_context,
            allow_new_target: grammar.allow_new_target,
            allow_super_property: grammar.allow_super_property,
            allow_super_call: grammar.allow_super_call,
            class_initializer: grammar.class_initializer,
            visible_private_names: if direct { caller.visible_private_names() } else { Vec::new() },
        };
        let program = crate::parser::parse_eval_utf16(source.units(), &context)
            .map_err(|error|Throw::syntax_error(error.to_string()))?;
        let strict = context.strict || has_use_strict_directive(&program.body);
        let env = EnvHandle::new(if strict { EnvKind::Var } else { EnvKind::Block }, Some(caller));
        self.direct_eval_env = Some(env);
        let previous_eval = self.in_eval;
        let previous_strict = self.strict_eval_context;
        let previous_direct = self.direct_eval_call;
        // Pop the entire native eval frame, preserving its invocation token.
        // Super and caller introspection then see the actual lexical caller.
        let native_frame = self.frames.pop();
        let previous_grammar = self.eval_grammar_override.replace((self.frames.len(), grammar));
        self.strict_eval_context = strict;
        self.direct_eval_call = false;
        self.in_eval = true;
        let result = self.run_program(&program);
        self.in_eval = previous_eval;
        self.strict_eval_context = previous_strict;
        self.direct_eval_call = previous_direct;
        self.eval_grammar_override = previous_grammar;
        if let Some(frame) = native_frame { self.frames.push(frame); }
        result
    }
}
