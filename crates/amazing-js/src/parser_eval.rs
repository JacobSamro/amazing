//! Script-goal parsing with PerformEval's inherited lexical permissions.
use super::*;

/// Context inherited by direct eval from its lexical caller. Indirect eval uses
/// the default context. Return, Yield and Await remain unavailable at the eval
/// program's top level, even when its caller is a function, generator or async
/// function. Private names include their leading `#` and cover all visible
/// private environments; nested class declarations may shadow them normally.
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    pub strict: bool,
    pub allow_new_target: bool,
    pub allow_super_property: bool,
    pub allow_super_call: bool,
    pub class_initializer: bool,
    pub visible_private_names: Vec<String>,
}

/// Parse eval's Script goal with the supplied lexical context. Ordinary
/// functions parsed inside eval reset super/initializer permissions, while
/// arrows inherit them. Strictness and private-name visibility follow existing
/// function and nested-class parsing rules.
pub fn parse_eval(src: &str, context: &EvalContext) -> Result<Program, SyntaxError> {
    parse_eval_utf16(&src.encode_utf16().collect::<Vec<_>>(), context)
}

/// Parse eval from exact UTF-16 source with the same inherited grammar context.
pub fn parse_eval_utf16(src: &[u16], context: &EvalContext) -> Result<Program, SyntaxError> {
    let mut parser = Parser::new_utf16(src);
    parser.strict_script = context.strict;
    parser.in_new_target = usize::from(context.allow_new_target);
    parser.super_property_allowed = context.allow_super_property;
    parser.super_call_allowed = context.allow_super_call;
    parser.in_class_field_initializer = usize::from(context.class_initializer);
    if !context.visible_private_names.is_empty() {
        let declared = context
            .visible_private_names
            .iter()
            .map(|name| (name.clone(), (4, false)))
            .collect();
        parser.private_scopes.push(PrivateScope {
            declared,
            uses: Vec::new(),
        });
    }
    let program = parser.parse_program()?;
    if let Some(scope) = parser.private_scopes.pop() {
        for use_name in scope.uses {
            if !scope.declared.contains_key(&use_name.name) {
                return Err(parser.err_at(
                    use_name.span.start,
                    format!("private name {} is not declared", use_name.name),
                ));
            }
        }
    }
    Ok(program)
}

impl Parser {
    pub(super) fn validate_arguments_reference(&self, id: &Ident) -> Result<(), SyntaxError> {
        if id.name == "arguments"
            && (self.in_class_field_initializer > 0 || self.in_static_block > 0)
        {
            return Err(self.err_at(
                id.span.start,
                "arguments is forbidden in a class field initializer",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_remains_script_goal_while_inheriting_super_and_new_target_permissions() {
        let context = EvalContext {
            allow_new_target: true,
            allow_super_property: true,
            allow_super_call: true,
            ..EvalContext::default()
        };
        for source in [
            "new.target",
            "super.x",
            "super[key]",
            "super()",
            "()=>super.x",
            "()=>new.target",
            "()=>super()",
        ] {
            assert!(parse_eval(source, &context).is_ok(), "{source}");
            assert!(
                parse_eval(source, &EvalContext::default()).is_err(),
                "{source}"
            );
        }
        for source in [
            "return 1",
            "yield 1",
            "await 1",
            "import value from 'x'",
            "export const value=1",
            "break",
            "continue",
            "function f(){super.x}",
            "function f(){super()}",
            "function f(x=super.x){}",
            "function f(x=super()){}",
            "({method(x=super()){}})",
            "class Inner { method(x=super()){} }",
        ] {
            assert!(parse_eval(source, &context).is_err(), "{source}");
        }
        assert!(parse_eval(
            "class Derived extends Base { constructor(value=super()){ } }",
            &EvalContext::default()
        )
        .is_ok());
        let property_only = EvalContext {
            allow_super_property: true,
            ..EvalContext::default()
        };
        assert!(parse_eval("super.x", &property_only).is_ok());
        assert!(parse_eval("super()", &property_only).is_err());
        assert!(parse_eval("new.target", &property_only).is_err());
        assert!(parse_eval("function f(){return new.target}", &EvalContext::default()).is_ok());
    }

    #[test]
    fn eval_private_names_are_visible_through_functions_and_nested_class_scopes() {
        let context = EvalContext {
            visible_private_names: vec!["#hidden".into()],
            ..EvalContext::default()
        };
        for source in [
            "object.#hidden",
            "#hidden in object",
            "function f(object){return object.#hidden}",
            "class Inner { #own; read(object){return object.#hidden+this.#own} }",
            "class Inner { #hidden; read(){return this.#hidden} }",
            "class Inner { #hidden; read(){return this.#hidden} };object.#hidden",
        ] {
            assert!(parse_eval(source, &context).is_ok(), "{source}");
        }
        for source in [
            "object.#unknown",
            "class Inner { read(object){return object.#unknown} }",
            "class Inner { #local; };object.#local",
        ] {
            assert!(parse_eval(source, &context).is_err(), "{source}");
        }
        assert!(parse_eval("object.#hidden", &EvalContext::default()).is_err());
        assert!(parse_eval(
            "class Inner { #hidden; };object.#hidden",
            &EvalContext::default()
        )
        .is_err());
    }

    #[test]
    fn initializer_arguments_restriction_includes_shorthand_but_resets_in_ordinary_functions() {
        let context = EvalContext {
            strict: true,
            class_initializer: true,
            ..EvalContext::default()
        };
        for source in [
            "arguments",
            "()=>arguments",
            "({arguments})",
            "()=>({arguments})",
            "({[arguments]:1})",
            "()=>{let arguments;}",
        ] {
            assert!(parse_eval(source, &context).is_err(), "{source}");
        }
        for source in [
            "function f(){return arguments}",
            "function f(x=arguments){return x}",
            "({method(x=arguments){return x}})",
            "class Inner { method(x=arguments){return x} }",
            "({method(){return arguments}})",
            "({arguments:1})",
            "({arguments(){return arguments}})",
        ] {
            assert!(parse_eval(source, &context).is_ok(), "{source}");
        }
        assert!(parse_eval("({arguments})", &EvalContext::default()).is_ok());
        assert!(parse_eval("with(object){}", &context).is_err());
        assert!(parse_eval("with(object){}", &EvalContext::default()).is_ok());
        for source in [
            "class C { field=({arguments}); }",
            "class C { static { ({arguments}); } }",
        ] {
            assert!(parse_script(source).is_err(), "{source}");
        }
    }
}
