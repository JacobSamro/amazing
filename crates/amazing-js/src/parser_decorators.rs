// Portions ported from lucid-softworks/lumen (crates/lumen/src/parser.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//! Decorator grammar, using the enclosing expression's Yield/Await parameters.
use super::*;

impl Parser {
    pub(super) fn parse_decorators(&mut self) -> Result<Vec<Expr>, SyntaxError> {
        let mut decorators = Vec::new();
        while self.eat_punct(Punct::At) {
            decorators.push(self.parse_decorator()?);
        }
        Ok(decorators)
    }

    fn parse_decorator(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        if self.eat_punct(Punct::LParen) {
            let saved = std::mem::replace(&mut self.no_in, false);
            let expression = self.parse_expression();
            self.no_in = saved;
            let expression = expression?;
            self.expect_punct(Punct::RParen)?;
            return Ok(Expr::new(ExprKind::Paren(Box::new(expression)), self.span_from(start)));
        }
        let identifier = self.expect_ident()?;
        self.validate_arguments_reference(&identifier)?;
        let mut expression = Expr::new(ExprKind::Ident(identifier), self.span_from(start));
        while self.eat_punct(Punct::Dot) {
            let property = if let TokenKind::PrivateIdent(name) = self.cur().kind.clone() {
                let token = self.bump();
                let identifier = Ident::new(name, token.span);
                self.use_private(identifier.clone())?;
                MemberProp::Private(identifier)
            } else { MemberProp::Ident(self.prop_name()?) };
            expression = Expr::new(ExprKind::Member {
                obj: Box::new(expression), prop: property, optional: false,
            }, self.span_from(start));
        }
        if self.at_punct(Punct::LParen) {
            let args = self.parse_args()?;
            expression = Expr::new(ExprKind::Call {
                callee: Box::new(expression), args, optional: false,
            }, self.span_from(start));
        }
        Ok(expression)
    }

    pub(super) fn parse_decorated_module_item(&mut self, exports: &mut HashSet<String>) -> Result<ModuleItem, SyntaxError> {
        let start = self.cur_loc();
        let source_start = self.pos;
        let decorators = self.parse_decorators()?;
        if self.at_kw(crate::tokens::Keyword::Export) {
            let mut item = self.parse_export_declaration(exports)?;
            let statement = match &mut item {
                ModuleItem::ExportNamed { declaration: Some(statement), .. }
                | ModuleItem::ExportDefaultDeclaration(statement) => statement,
                _ => return Err(self.err_at(start, "decorators before export require a class declaration")),
            };
            let StmtKind::Class(class) = &mut statement.kind else {
                return Err(self.err_at(start, "decorators before export require a class declaration"));
            };
            if !class.decorators.is_empty() {
                return Err(self.err_at(start, "class decorators cannot occur both before and after export"));
            }
            class.decorators = decorators;
            statement.span.start = start;
            Ok(item)
        } else {
            let mut class = self.parse_class_with_decorators(decorators)?;
            class.source = self.source_from(source_start);
            if class.name.is_none() { return Err(self.err_at(start, "class declaration requires a name")); }
            Ok(ModuleItem::Statement(Stmt::new(StmtKind::Class(Box::new(class)), self.span_from(start))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decorator_grammar_preserves_lists_and_restricted_member_calls() {
        for source in ["@one @object.decorator(1) class C { @two method(){} @three field; @four accessor value; }", "let C = @(one || two) class {};", "class Outer { #decorator; method(){ return @(this.#decorator) class {}; } }", "@await() class C {}"] {
            assert!(parse_script(source).is_ok(), "{source}");
        }
        let program = parse_script("@one @object.decorator(1) class C { @two method(){} @three field; }").unwrap();
        let StmtKind::Class(class) = &program.body[0].kind else { panic!("class") };
        assert_eq!(class.decorators.len(), 2);
        assert!(matches!(&class.members[0], ClassMember::Method(method) if method.decorators.len() == 1));
        assert!(matches!(&class.members[1], ClassMember::Field(field) if field.decorators.len() == 1));
    }
    #[test]
    fn decorator_grammar_rejects_non_decorator_expressions_and_illegal_elements() {
        for source in ["@this.decorator class C{}", "@obj[key] class C{}", "@fn()() class C{}", "@obj?.dec class C{}", "@new F class C{}", "@dec function f(){}", "class C { @dec constructor(){} }", "class C { @dec static {} }", "class C { @dec ; }", "function* f(){ @yield class C{} }", "async function f(){ @await class C{} }", "@dec class {}"] {
            assert!(parse_script(source).is_err(), "{source}");
        }
        assert!(parse_script("class C { @dec static constructor(){} }").is_ok());
    }
    #[test]
    fn exported_decorators_have_one_permitted_side_of_export() {
        for source in ["@dec export class C{}", "export @dec class C{}", "@dec export default class {}", "export default @dec class {}", "@dec export default class C {}"] {
            assert!(parse_module(source).is_ok(), "{source}");
        }
        for source in ["@a export @b class C{}", "@a export default @b class {}", "@a export function f(){}", "@a export {a}", "export @a class {}"] {
            assert!(parse_module(source).is_err(), "{source}");
        }
    }
}
